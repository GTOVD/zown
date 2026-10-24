"""Cooperative fast lanes (SPEC §22).

The oracle does not start OS real-time threads. It runs a tick scheduler both
runtimes can reproduce: every tick runs each registered lane once, then at most
one compute job. A queue of jobs cannot skip a lane. `ZOWN_CPUS` pins the
topology so a test does not depend on the host.
"""

from __future__ import annotations

import os
from collections import deque
from typing import Any

from .errors import BOUNDS, TYPE_MISMATCH, Pos

PIPE_CAP = 8
LANE_ORDER = ("m", "n")


def cpu_count(override: int | None = None) -> int:
    """Host CPUs, unless `override` or `ZOWN_CPUS` is an integer >= 1."""
    if override is not None and override >= 1:
        return override
    raw = os.environ.get("ZOWN_CPUS")
    if raw is not None and raw.strip():
        try:
            n = int(raw, 10)
        except ValueError:
            n = 0
        if n >= 1:
            return n
    n = os.cpu_count() or 1
    return n if n >= 1 else 1


def room_for(cpus: int) -> int:
    """Real-time slots the machine can pin. One CPU stays a worker when it can."""
    return max(cpus - 1, 1)


def grant_for(cpus: int, requested: int | None) -> int:
    """Slots actually pinned. `None` is the default request, min(2, room)."""
    room = room_for(cpus)
    if requested is None:
        return min(2, room)
    if requested < 0:
        return 0
    return min(requested, room)


def workers_for(cpus: int, granted: int) -> int:
    return max(cpus - granted, 1)


def tune_report(requested: int | None = None, override: int | None = None) -> str:
    """Text `zown tune` prints. `pinned` when at least one slot is granted."""
    cpus = cpu_count(override)
    granted = grant_for(cpus, requested)
    workers = workers_for(cpus, granted)
    place = "pinned" if granted else "fibers"
    return f"cpus {cpus}\ngranted {granted}\nworkers {workers}\n{place}\n"


class Lanes:
    """Scheduler state hanging off one VM. Lanes and jobs share its stack."""

    def __init__(self) -> None:
        self.cpus_override: int | None = None
        self.requested: int | None = None  # None -> default grant
        self.bodies: dict[str, Any] = {}
        self.pipes: dict[str, deque] = {name: deque() for name in LANE_ORDER}
        self.jobs: list[deque] = []
        self.steals = 0
        self.trace: list[str] = []
        self.lane: str | None = None
        self.in_tick = False
        self.rr = 0

    def cpus(self) -> int:
        return cpu_count(self.cpus_override)

    def granted(self) -> int:
        return grant_for(self.cpus(), self.requested)

    def ensure_workers(self) -> None:
        """Match the deque count to the worker count. Extra jobs fold onto the last."""
        n = workers_for(self.cpus(), self.granted())
        while len(self.jobs) < n:
            self.jobs.append(deque())
        while len(self.jobs) > n:
            extra = self.jobs.pop()
            self.jobs[-1].extend(extra)


def _pop_int(vm, op: str, pos: Pos) -> int:
    from .vm import type_name
    v = vm.pop(op, pos)
    if type(v) is not int:
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected an int, got {type_name(v)}",
                     op=op, pos=pos, hint="the count is a plain int")
    return v


def _lane_order(lanes: Lanes) -> list[str]:
    """Pinned lanes first, then the ones that did not fit, as fibers.

    Both classes run every tick. With the fixed order m then n this is the
    registered lanes in that order; the split is the pin set a threaded host
    would use. A lane with no body does not take a slot.
    """
    lanes.ensure_workers()
    live = [name for name in LANE_ORDER if name in lanes.bodies]
    granted = lanes.granted()
    return live[:granted] + live[granted:]


def _take_job(lanes: Lanes):
    """One compute slice. The owner pops the left; a thief pops the right."""
    lanes.ensure_workers()
    n = len(lanes.jobs)
    if n == 0:
        return None
    idx = lanes.rr % n
    lanes.rr += 1
    local = lanes.jobs[idx]
    if local:
        return local.popleft()
    for step in range(1, n):
        other = lanes.jobs[(idx + step) % n]
        if other:
            lanes.steals += 1
            return other.pop()
    return None


def _run_lane(vm, name: str, pos: Pos) -> None:
    lanes = vm.lanes
    blk = lanes.bodies[name]
    prev = lanes.lane
    lanes.lane = name
    try:
        vm.invoke(blk, f"~{name}", pos)
    finally:
        lanes.lane = prev
    lanes.trace.append(name)


def _one_tick(vm, pos: Pos) -> None:
    lanes = vm.lanes
    for name in _lane_order(lanes):
        _run_lane(vm, name, pos)
    job = _take_job(lanes)
    if job is None:
        return
    vm.invoke(job, "^", pos)
    lanes.trace.append("c")


def op_pipe(vm, pos: Pos) -> None:
    """`|` — send the top value into the running lane's pipe."""
    lane = vm.lanes.lane
    if lane is None:
        raise vm.err(TYPE_MISMATCH, "`|` sends only inside a running lane",
                     op="|", pos=pos,
                     hint="register a lane with ~m or ~n; pull from outside with pm or pn")
    v = vm.pop("|", pos)
    pipe = vm.lanes.pipes[lane]
    if len(pipe) >= PIPE_CAP:
        raise vm.err(BOUNDS, f"lane {lane} pipe is full ({PIPE_CAP})",
                     op="|", pos=pos, hint="pull with pm or pn before the next send")
    pipe.append(v)


def op_spawn(vm, pos: Pos) -> None:
    """`^` — queue a block on worker 0."""
    blk = vm.pop_block("^", pos)
    vm.lanes.ensure_workers()
    vm.lanes.jobs[0].append(blk)


def op_tilde(vm, op: str, pos: Pos) -> None:
    """`~m` / `~n` register a lane. `~p` / `~f` publish and fetch a bundle."""
    name = op[1:]
    if name in LANE_ORDER:
        blk = vm.pop_block(op, pos)
        vm.lanes.bodies[name] = blk
        return
    if name == "p":
        from .mesh import publish
        publish(vm, pos)
        return
    if name == "f":
        from .mesh import fetch
        fetch(vm, pos)
        return
    raise vm.err("UNSUPPORTED", f"operator `{op}` is not implemented",
                 op=op, pos=pos,
                 hint="~m and ~n are lanes; ~p publishes a bundle and ~f fetches it")


def b_hw(vm, pos: Pos) -> None:
    vm.push(vm.lanes.cpus())


def b_lc(vm, pos: Pos) -> None:
    n = _pop_int(vm, "lc", pos)
    if n < 0:
        raise vm.err(BOUNDS, "`lc` requested slots must be >= 0", op="lc", pos=pos,
                     hint="ask for 0 to run every lane as a fiber")
    vm.lanes.requested = n
    vm.lanes.ensure_workers()
    vm.push(vm.lanes.granted())


def b_rn(vm, pos: Pos) -> None:
    if vm.lanes.in_tick:
        raise vm.err(TYPE_MISMATCH, "`rn` cannot re-enter while a tick is running",
                     op="rn", pos=pos,
                     hint="a lane or a job runs to the end of its block; start ticks from outside")
    k = _pop_int(vm, "rn", pos)
    if k < 0:
        raise vm.err(BOUNDS, "`rn` tick count must be >= 0", op="rn", pos=pos)
    vm.lanes.in_tick = True
    try:
        for _ in range(k):
            _one_tick(vm, pos)
    finally:
        vm.lanes.in_tick = False


def _pull(vm, lane: str, word: str, pos: Pos) -> None:
    pipe = vm.lanes.pipes[lane]
    if not pipe:
        raise vm.err(BOUNDS, f"lane {lane} pipe is empty", op=word, pos=pos,
                     hint="a lane sends with `|` inside its body; the pipe holds 8 values")
    vm.push(pipe.popleft())


def b_pm(vm, pos: Pos) -> None:
    _pull(vm, "m", "pm", pos)


def b_pn(vm, pos: Pos) -> None:
    _pull(vm, "n", "pn", pos)


def b_ws(vm, pos: Pos) -> None:
    vm.push(vm.lanes.steals)


def b_sg(vm, pos: Pos) -> None:
    vm.push("".join(vm.lanes.trace))
