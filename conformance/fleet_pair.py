#!/usr/bin/env python3
"""Two peers share a trace id and a want (M29).

The client sets the trace, declares a want, observes a different value, and
sends that value. The server receives the trace and the want with the frame,
observes the same value, and `df` prints the same drift. Both logs name the
trace id.

Usage:
    python3 conformance/fleet_pair.py
"""

from __future__ import annotations

import os
import socket
import subprocess
import sys
import tempfile
import textwrap

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ZOWNC = os.path.join(ROOT, "zownc", "target", "debug",
                     "zownc.exe" if os.name == "nt" else "zownc")
ZOWN = os.path.join(ROOT, "bin", "zown")

SERVER = textwrap.dedent("""\
`r [ `t [
  $fleet$ $get$ 0 ag n ~l :s
  s ~r :v
  $go$ v ob
  df .
  lg .
] gr ] gr
""")
CLIENT = textwrap.dedent("""\
`t [
  $run1$ td
  $go$ $1$ wn
  $go$ $0$ ob
  $fleet$ $get$ ~c :s
  s $0$ ~s
  df .
  lg .
] gr
""")

GREEN, RED, RST = "\033[32m", "\033[31m", "\033[0m"


def free_port() -> str:
    sock = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
    sock.bind(("::1", 0))
    port = str(sock.getsockname()[1])
    sock.close()
    return port


def write_src(directory: str, name: str, src: str) -> str:
    path = os.path.join(directory, name)
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(src)
    return path


def launch(cmd: list[str], env: dict[str, str]) -> subprocess.Popen[str]:
    return subprocess.Popen(
        cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env
    )


def finish(proc: subprocess.Popen[str]) -> tuple[str, str, int]:
    try:
        out, err = proc.communicate(timeout=8)
    except subprocess.TimeoutExpired:
        proc.kill()
        out, err = proc.communicate()
        err = (err or "") + "\ntimed out"
    return out, err, proc.returncode if proc.returncode is not None else 1


def run_pair(prefix: list[str], label: str) -> None:
    work = tempfile.mkdtemp()
    env = os.environ.copy()
    env["ZOWN_MESH"] = os.path.join(work, "mesh")
    env["ZOWN_TICK"] = "1"
    env["ZOWN_WALL"] = "0"
    os.makedirs(env["ZOWN_MESH"])
    port = free_port()
    server = write_src(work, "server.zn", SERVER)
    client = write_src(work, "client.zn", CLIENT)
    srv = launch(prefix + [server, "--", port], env)
    cli = launch(prefix + [client], env)
    sout, serr, scode = finish(srv)
    cout, cerr, ccode = finish(cli)
    if scode != 0 or ccode != 0 or "go 1 0\n" not in sout or "go 1 0\n" not in cout:
        raise RuntimeError(
            f"{label} drift\n server {scode} {sout!r} {serr!r}\n client {ccode} {cout!r} {cerr!r}"
        )
    if "run1" not in sout or "run1" not in cout:
        raise RuntimeError(
            f"{label} trace\n server {sout!r}\n client {cout!r}"
        )


def main() -> int:
    if not os.path.exists(ZOWNC):
        print(f"{RED}error{RST}: {ZOWNC} not found. Build it: (cd zownc && cargo build)")
        return 2
    try:
        run_pair([sys.executable, ZOWN, "run"], "oracle")
        print(f"{GREEN}ok{RST}   oracle")
        run_pair([ZOWNC, "run"], "zownc")
        print(f"{GREEN}ok{RST}   zownc")
    except RuntimeError as exc:
        print(f"{RED}FAIL{RST} {exc}")
        return 1
    print("\n2 peers, 0 failed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
