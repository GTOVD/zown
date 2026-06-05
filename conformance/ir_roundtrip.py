#!/usr/bin/env python3
"""IR lossless round-trip gate (M5).

For every program in the corpus, verify that lowering the AST to IR and rebuilding
it (`zownc irast`) reproduces the AST exactly (`zownc ast`). If lowering is a
lossless, structural representation of the program -- and the AST frontend already
matches the oracle (M3) -- then the IR faithfully represents the program, which is
the property the WASM/native backends rely on.

Usage:
    python3 conformance/ir_roundtrip.py    # requires: (cd zownc && cargo build)
"""

from __future__ import annotations

import glob
import os
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ZOWNC = os.path.join(ROOT, "zownc", "target", "debug", "zownc")

GREEN, RED, RST = "\033[32m", "\033[31m", "\033[0m"


def out(cmd_path: str, path: str) -> str:
    r = subprocess.run([ZOWNC, cmd_path, path], capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError(r.stderr.strip())
    return r.stdout


def main() -> int:
    if not os.path.exists(ZOWNC):
        print(f"{RED}error{RST}: {ZOWNC} not found. Build it: (cd zownc && cargo build)")
        return 2

    files = sorted(
        glob.glob(os.path.join(ROOT, "conformance", "cases", "*.zn"))
        + glob.glob(os.path.join(ROOT, "examples", "*.zn"))
    )
    ok = fail = 0
    for path in files:
        rel = os.path.relpath(path, ROOT)
        try:
            ast = out("ast", path)
            irast = out("irast", path)
        except RuntimeError as e:
            print(f"{RED}FAIL{RST} {rel}: {e}")
            fail += 1
            continue
        if ast == irast:
            print(f"{GREEN}ok{RST}   {rel}")
            ok += 1
        else:
            print(f"{RED}FAIL{RST} {rel}: IR round-trip changed the AST")
            fail += 1

    print(f"\n{ok} lossless, {fail} lossy")
    return 1 if fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
