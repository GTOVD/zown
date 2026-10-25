#!/usr/bin/env python3
"""Two Zown peers exchange one frame on `[::1]` (M25).

The oracle and `zownc run` each play both roles. The listener registers the
name `echo` with `~l`. The other peer dials it with `~c`. A third run is the
`~w` relay. Nothing leaves the machine.

Usage:
    python3 conformance/stream_pair.py
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
  $echo$ $echo$ 0 ag n ~l :s
  s ~r .
  $pong$ ~s
] gr ] gr
""")
CLIENT = textwrap.dedent("""\
`t [ `r [
  $echo$ $echo$ ~c :s
  s $ping$ ~s
  ~r .
] gr ] gr
""")
RELAY = "`r [ `t [ 0 ag n ~w ] gr ] gr\n"
LEFT = textwrap.dedent("""\
`t [ `r [
  $::1$ 0 ag n ~o :s
  s $ping$ ~s
  ~r .
] gr ] gr
""")
RIGHT = textwrap.dedent("""\
`t [ `r [
  $::1$ 0 ag n ~o :s
  s ~r .
  $pong$ ~s
] gr ] gr
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
    return subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)


def finish(proc: subprocess.Popen[str], label: str) -> tuple[str, str, int]:
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
    sout, serr, scode = finish(srv, "server")
    cout, cerr, ccode = finish(cli, "client")
    if scode != 0 or ccode != 0 or sout != "ping\n" or cout != "pong\n":
        raise RuntimeError(
            f"{label} pair\n server {scode} {sout!r} {serr!r}\n client {ccode} {cout!r} {cerr!r}"
        )
    rport = free_port()
    relay = write_src(work, "relay.zn", RELAY)
    left = write_src(work, "left.zn", LEFT)
    right = write_src(work, "right.zn", RIGHT)
    rel = launch(prefix + [relay, "--", rport], env)
    import time
    time.sleep(0.1)
    a = launch(prefix + [left, "--", rport], env)
    time.sleep(0.05)
    b = launch(prefix + [right, "--", rport], env)
    aout, aerr, acode = finish(a, "left")
    bout, berr, bcode = finish(b, "right")
    rout, rerr, rcode = finish(rel, "relay")
    if acode != 0 or bcode != 0 or rcode != 0 or aout != "pong\n" or bout != "ping\n":
        raise RuntimeError(
            f"{label} relay\n left {acode} {aout!r} {aerr!r}\n"
            f" right {bcode} {bout!r} {berr!r}\n relay {rcode} {rout!r} {rerr!r}"
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
