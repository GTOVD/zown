#!/usr/bin/env python3
"""The Zown lexer, compiled natively, matches the host token dump.

Stage-0 is `zownc native`. It runs `zown/std/lex.zn` on a source string.
The fixed point is that lexer's own source: the dump equals `token_dump`.

Usage:
    python3 conformance/stage_parity.py
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

from zown.lexer import token_dump  # noqa: E402
from zown.stage import program  # noqa: E402

ZOWNC = os.path.join(ROOT, "zownc", "target", "debug",
                     "zownc.exe" if os.name == "nt" else "zownc")
FILES = [
    os.path.join(ROOT, "zown", "std", "lex.zn"),
    os.path.join(ROOT, "examples", "hello.zn"),
    os.path.join(ROOT, "examples", "graph.zn"),
    os.path.join(ROOT, "conformance", "cases_v2", "intent.zn"),
]

GREEN, RED, RST = "\033[32m", "\033[31m", "\033[0m"


def native_dump(src: str) -> str:
    prog = program(src)
    fd, path = tempfile.mkstemp(suffix=".zn")
    os.close(fd)
    try:
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(prog)
        out = subprocess.run(
            [ZOWNC, "native", "--unchecked", path],
            capture_output=True, text=True,
        )
    finally:
        os.remove(path)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip() or out.stdout)
    return out.stdout


def main() -> int:
    if not os.path.exists(ZOWNC):
        print(f"{RED}error{RST}: {ZOWNC} not found. Build it: (cd zownc && cargo build)")
        return 2
    fail = 0
    for path in FILES:
        name = os.path.relpath(path, ROOT)
        src = open(path, encoding="utf-8").read()
        host = token_dump(src)
        try:
            got = native_dump(src)
        except RuntimeError as e:
            print(f"{RED}FAIL{RST} {name}: {e}")
            fail += 1
            continue
        if got == host:
            print(f"{GREEN}ok{RST}   {name}")
        else:
            print(f"{RED}FAIL{RST} {name}")
            fail += 1
    print(f"\n{len(FILES) - fail} parity, {fail} diff")
    return 1 if fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
