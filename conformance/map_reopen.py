#!/usr/bin/env python3
"""A synced snapshot survives the process that wrote it (M26).

One process fsyncs the graph with `mm` and exits. The next process `ml`s
that file, prints `gs`, and follows one edge. The oracle and `zownc run`
must print the same text.

Usage:
    python3 conformance/map_reopen.py
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile
import textwrap

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ZOWNC = os.path.join(ROOT, "zownc", "target", "debug",
                     "zownc.exe" if os.name == "nt" else "zownc")
ZOWN = os.path.join(ROOT, "bin", "zown")

WRITE = textwrap.dedent("""\
$a$ gv
$b$ gv
$a$ $knows$ $b$ ge
0 ag :d
d [
  d $/g.txt$ ct mm
] gd
""")
READ = textwrap.dedent("""\
0 ag :d
d [
  d $/g.txt$ ct ml
] gd
gs .
$a$ » $knows$ → « .
""")
WANT = "v a\nv b\ne a knows b\n\nb\n"

GREEN, RED, RST = "\033[32m", "\033[31m", "\033[0m"


def run(prefix: list[str], src: str, directory: str) -> str:
    fd, path = tempfile.mkstemp(suffix=".zn")
    os.close(fd)
    try:
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(src)
        proc = subprocess.run(
            prefix + [path, "--", directory],
            capture_output=True, text=True,
        )
    finally:
        os.remove(path)
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout)
    return proc.stdout


def once(prefix: list[str], label: str) -> None:
    work = tempfile.mkdtemp()
    run(prefix, WRITE, work)
    got = run(prefix, READ, work)
    if got != WANT:
        raise RuntimeError(f"{label}\n want={WANT!r}\n got ={got!r}")


def main() -> int:
    if not os.path.exists(ZOWNC):
        print(f"{RED}error{RST}: {ZOWNC} not found. Build it: (cd zownc && cargo build)")
        return 2
    try:
        once([sys.executable, ZOWN, "run"], "oracle")
        print(f"{GREEN}ok{RST}   oracle")
        once([ZOWNC, "run"], "zownc")
        print(f"{GREEN}ok{RST}   zownc")
    except RuntimeError as exc:
        print(f"{RED}FAIL{RST} {exc}")
        return 1
    print("\n2 reopened, 0 failed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
