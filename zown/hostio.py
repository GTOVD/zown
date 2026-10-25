"""Host I/O behind capabilities (SPEC §23).

Files need `` `d `` and a path root from `gd`. A resolved path outside every
live root is `CAP_DENIED`. Env lookup needs `` `v ``. The clock needs `` `c ``.
`ZOWN_TICK` and `ZOWN_WALL` pin the two numbers the goldens print. Arguments
are the program's own vector, empty unless the host passed some after `--`.
"""

from __future__ import annotations

import os
import time

from .buf import BUF_MAX, Alloc, Buf
from .errors import BOUNDS, CAP_DENIED, OVERFLOW, TYPE_MISMATCH, Pos
from .lexer import INT_MAX, INT_MIN

_LINK_HOPS = 40


class PathError(Exception):
    def __init__(self, kind: str) -> None:
        self.kind = kind


def resolve(path: str) -> str:
    """Absolute path with `.`, `..`, and symlinks removed.

    A missing final component stays in the path. A symlink is followed, so a
    link that leaves a granted directory does not stay inside it. More than
    40 links is a loop.
    """
    if path == "" or "\0" in path:
        raise PathError("type")
    if path.startswith("/"):
        pending = [p for p in path.split("/") if p and p != "."]
    else:
        pending = [p for p in os.getcwd().split("/") if p and p != "."]
        pending.extend(p for p in path.split("/") if p and p != ".")
    out: list[str] = []
    hops = 0
    i = 0
    while i < len(pending):
        part = pending[i]
        i += 1
        if part == "..":
            if out:
                out.pop()
            continue
        cur = "/" + "/".join(out + [part])
        if os.path.islink(cur):
            hops += 1
            if hops > _LINK_HOPS:
                raise PathError("bounds")
            target = os.readlink(cur)
            extra = [p for p in target.split("/") if p and p != "."]
            if target.startswith("/"):
                out = []
            pending = extra + pending[i:]
            i = 0
            continue
        out.append(part)
    if not out:
        return "/"
    return "/" + "/".join(out)


def allowed(root: str, candidate: str) -> bool:
    """`candidate` is `root`, or a descendant when `root` is not an existing file."""
    if candidate == root:
        return True
    if os.path.isfile(root):
        return False
    prefix = root if root.endswith("/") else root + "/"
    return candidate.startswith(prefix)


def _pop_str(vm, op: str, pos: Pos) -> str:
    v = vm.pop(op, pos)
    if not isinstance(v, str):
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a string, got {type_name(v)}",
                     op=op, pos=pos)
    return v


def _must_resolve(vm, op: str, path: str, pos: Pos) -> str:
    try:
        return resolve(path)
    except PathError as e:
        if e.kind == "type":
            raise vm.err(TYPE_MISMATCH, f"`{op}` expected a path string",
                         op=op, pos=pos, hint="the path is a non-empty string with no NUL")
        raise vm.err(BOUNDS, f"`{op}` path has too many symlinks",
                     op=op, pos=pos, hint="a symlink loop cannot be resolved")


def _deny(vm, op: str, cap: str, pos: Pos, msg: str, hint: str):
    raise vm.err(CAP_DENIED, msg, op=op, pos=pos, kind="sec", cap=cap, hint=hint)


def _under_disk(vm, op: str, path: str, pos: Pos) -> str:
    if "d" not in vm.caps or not vm.roots:
        _deny(vm, op, "d", pos,
              f"`{op}` needs `d and a granted path",
              "grant a directory: $path$ [ ... ] gd")
    candidate = _must_resolve(vm, op, path, pos)
    for root in vm.roots:
        if allowed(root, candidate):
            return candidate
    _deny(vm, op, "d", pos,
          f"`{op}` path is outside the granted root",
          "the grant is a directory or one file; `..` and symlinks are resolved first")


def b_gd(vm, pos: Pos) -> None:
    # path [body] gd  -> run body with `d granted and this path as a root.
    blk = vm.pop_block("gd", pos)
    raw = _pop_str(vm, "gd", pos)
    root = _must_resolve(vm, "gd", raw, pos)
    vm.roots.append(root)
    had = "d" in vm.caps
    vm.caps.add("d")
    try:
        vm.invoke(blk, "gd", pos)
    finally:
        vm.roots.pop()
        if not had:
            vm.caps.discard("d")


def _buf_bytes(vm, buf, op: str, pos: Pos) -> bytes:
    from .vm import type_name
    if not isinstance(buf, Buf):
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a buf, got {type_name(buf)}",
                     op=op, pos=pos, hint="read with fr, or allocate with bf")
    if buf.alloc.data is None:
        raise vm.err(BOUNDS, f"`{op}` buffer was freed",
                     op=op, pos=pos, hint="the exclusive owner was dropped with `,`")
    return bytes(buf.alloc.data)


def b_fr(vm, pos: Pos) -> None:
    # path fr -> an exclusive buf of the file's bytes.
    path = _pop_str(vm, "fr", pos)
    src = _under_disk(vm, "fr", path, pos)
    if os.path.isdir(src):
        raise vm.err(TYPE_MISMATCH, "`fr` path is a directory",
                     op="fr", pos=pos, hint="fr reads one file")
    if not os.path.isfile(src):
        raise vm.err(BOUNDS, "`fr` file is not there",
                     op="fr", pos=pos, hint="the path is under the grant, and the file exists")
    try:
        size = os.path.getsize(src)
    except OSError:
        raise vm.err(BOUNDS, "`fr` file is not there",
                     op="fr", pos=pos, hint="the path is under the grant, and the file exists")
    if size > BUF_MAX:
        raise vm.err(BOUNDS, f"`fr` file is larger than {BUF_MAX} bytes",
                     op="fr", pos=pos, hint="the buffer budget is 1048576 bytes")
    try:
        with open(src, "rb") as fh:
            data = fh.read(BUF_MAX + 1)
    except OSError:
        raise vm.err(BOUNDS, "`fr` the host refused the read",
                     op="fr", pos=pos)
    if len(data) > BUF_MAX:
        raise vm.err(BOUNDS, f"`fr` file is larger than {BUF_MAX} bytes",
                     op="fr", pos=pos, hint="the buffer budget is 1048576 bytes")
    alloc = Alloc(len(data))
    if data and alloc.data is not None:
        alloc.data[:] = data
    vm.push(Buf(alloc, True))


def b_fw(vm, pos: Pos) -> None:
    # buf path fw -> write every byte. The buf stays. A borrow may be the source.
    from .errors import STACK_UNDERFLOW
    path = _pop_str(vm, "fw", pos)
    if not vm.stack:
        raise vm.err(STACK_UNDERFLOW, "`fw` needs a buf under the path",
                     op="fw", pos=pos, hint="the stack is buf, then path")
    data = _buf_bytes(vm, vm.stack[-1], "fw", pos)
    dest = _under_disk(vm, "fw", path, pos)
    if os.path.isdir(dest):
        raise vm.err(TYPE_MISMATCH, "`fw` path is a directory",
                     op="fw", pos=pos, hint="fw writes one file")
    parent = os.path.dirname(dest)
    if parent and not os.path.isdir(parent):
        raise vm.err(BOUNDS, "`fw` the host refused the write",
                     op="fw", pos=pos, hint="the parent directory has to exist")
    try:
        with open(dest, "wb") as fh:
            fh.write(data)
    except OSError:
        raise vm.err(BOUNDS, "`fw` the host refused the write",
                     op="fw", pos=pos)


def b_ac(vm, pos: Pos) -> None:
    # ac -> how many arguments the host passed. The toolchain's own argv is not here.
    n = len(vm.args)
    if n > INT_MAX:
        raise vm.err(OVERFLOW, "`ac` argument count does not fit int", op="ac", pos=pos)
    vm.push(n)


def b_ag(vm, pos: Pos) -> None:
    # idx ag -> that argument. Outside 0..count is BOUNDS.
    v = vm.pop("ag", pos)
    if isinstance(v, bool) or not isinstance(v, int):
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`ag` expected an int, got {type_name(v)}",
                     op="ag", pos=pos)
    if v < 0 or v >= len(vm.args):
        raise vm.err(BOUNDS, "`ag` index is outside the argument vector",
                     op="ag", pos=pos, hint="ac pushes the count")
    vm.push(vm.args[v])


def b_en(vm, pos: Pos) -> None:
    # name en -> that variable's value. Unset is BOUNDS. Not a dump of the environment.
    if "v" not in vm.caps:
        _deny(vm, "en", "v", pos,
              "`en` needs `v",
              "grant it: `v [ ... ] gr")
    name = _pop_str(vm, "en", pos)
    if name == "" or "\0" in name:
        raise vm.err(TYPE_MISMATCH, "`en` expected a variable name",
                     op="en", pos=pos, hint="the name is a non-empty string with no NUL")
    if name not in os.environ:
        raise vm.err(BOUNDS, "`en` that variable is not set",
                     op="en", pos=pos, hint="en looks up one name; it does not list the environment")
    vm.push(os.environ[name])


def _pinned(name: str) -> int | None | str:
    """An integer pin, None when unset or not an integer, or 'overflow'."""
    raw = os.environ.get(name)
    if raw is None or not raw.strip():
        return None
    try:
        n = int(raw.strip(), 10)
    except ValueError:
        return None
    if n < INT_MIN or n > INT_MAX:
        return "overflow"
    return n


def _clock(vm, pos: Pos, name: str, real: int) -> int:
    pin = _pinned(name)
    if pin == "overflow":
        raise vm.err(OVERFLOW, "`tm` pin does not fit int", op="tm", pos=pos)
    if pin is None:
        if real < INT_MIN or real > INT_MAX:
            raise vm.err(OVERFLOW, "`tm` result does not fit int", op="tm", pos=pos)
        return real
    return pin


def b_tm(vm, pos: Pos) -> None:
    # tm -> monotonic tick, then wall unix seconds. Both pins are host env.
    if "c" not in vm.caps:
        _deny(vm, "tm", "c", pos,
              "`tm` needs `c",
              "grant it: `c [ ... ] gr")
    tick = _clock(vm, pos, "ZOWN_TICK", time.monotonic_ns())
    wall = _clock(vm, pos, "ZOWN_WALL", int(time.time()))
    vm.push(tick)
    vm.push(wall)
