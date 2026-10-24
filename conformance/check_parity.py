#!/usr/bin/env python3
"""Differential checker parity: `zownc check` vs `conformance/check/*.diag`.

The golden line format is the oracle's (`check.py`). Exit 0 means clean (`ok`),
exit 2 means the checker rejected the program.

Usage:
    python3 conformance/check_parity.py    # requires: (cd zownc && cargo build)
"""

from __future__ import annotations

import os
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ZOWNC = os.path.join(ROOT, "zownc", "target", "debug",
                     "zownc.exe" if os.name == "nt" else "zownc")
CASES = os.path.join(ROOT, "conformance", "check")

GREEN, RED, RST = "\033[32m", "\033[31m", "\033[0m"


def main() -> int:
    if not os.path.exists(ZOWNC):
        print(f"{RED}error{RST}: {ZOWNC} not found. Build it: (cd zownc && cargo build)")
        return 2
    ok = fail = 0
    names = sorted(f[:-3] for f in os.listdir(CASES) if f.endswith(".zn"))
    for name in names:
        src = os.path.join(CASES, name + ".zn")
        want = open(os.path.join(CASES, name + ".diag"), encoding="utf-8").read().replace("\r\n", "\n")
        out = subprocess.run([ZOWNC, "check", src], capture_output=True, text=True)
        got = out.stdout.replace("\r\n", "\n")
        expect = 0 if want == "ok\n" else 2
        if got == want and out.returncode == expect:
            print(f"{GREEN}ok{RST}   {name} [{want.strip().replace(chr(10), ' | ')}]")
            ok += 1
        else:
            print(
                f"{RED}FAIL{RST} {name}: exit {out.returncode} (want {expect})\n"
                f"  want={want!r}\n  got ={got!r}\n  err ={out.stderr!r}"
            )
            fail += 1
    print(f"\n{ok} parity, {fail} diff")
    return 1 if fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
