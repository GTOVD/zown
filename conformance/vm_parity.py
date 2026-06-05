#!/usr/bin/env python3
"""Differential VM parity: native `zownc run` vs the golden expectations.

Runs every conformance program through the native toolchain and checks it against
the same goldens the Python oracle is held to (cases/*.out, errors/*.code). This
is the M4 gate in docs/PLAN.md: when this is green, today's language is fully
reimplemented in Rust.

Usage:
    python3 conformance/vm_parity.py     # requires: (cd zownc && cargo build)
"""

from __future__ import annotations

import glob
import json
import os
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ZOWNC = os.path.join(ROOT, "zownc", "target", "debug", "zownc")
CASES = os.path.join(ROOT, "conformance", "cases")
ERRORS = os.path.join(ROOT, "conformance", "errors")

GREEN, RED, RST = "\033[32m", "\033[31m", "\033[0m"


def run_case(path: str) -> str:
    out = subprocess.run([ZOWNC, "run", path], capture_output=True, text=True)
    return out.stdout


def run_error(path: str) -> str:
    out = subprocess.run([ZOWNC, "run", "--zerr", path], capture_output=True, text=True)
    if out.returncode == 0:
        raise RuntimeError(f"expected an error, exited 0 with stdout {out.stdout!r}")
    pkt = json.loads(out.stderr)
    return pkt["code"] if not pkt.get("op") else f"{pkt['code']} {pkt['op']}"


def main() -> int:
    if not os.path.exists(ZOWNC):
        print(f"{RED}error{RST}: {ZOWNC} not found. Build it: (cd zownc && cargo build)")
        return 2

    ok = fail = 0

    print("== cases (stdout) ==")
    for zn in sorted(glob.glob(os.path.join(CASES, "*.zn"))):
        name = os.path.basename(zn)[:-3]
        want = open(os.path.join(CASES, name + ".out"), encoding="utf-8").read()
        got = run_case(zn)
        if got == want:
            print(f"{GREEN}ok{RST}   {name}")
            ok += 1
        else:
            print(f"{RED}FAIL{RST} {name}\n  want={want!r}\n  got ={got!r}")
            fail += 1

    print("== errors (recovery code) ==")
    for zn in sorted(glob.glob(os.path.join(ERRORS, "*.zn"))):
        name = os.path.basename(zn)[:-3]
        want = open(os.path.join(ERRORS, name + ".code"), encoding="utf-8").read().strip()
        try:
            got = run_error(zn)
        except (RuntimeError, json.JSONDecodeError) as e:
            print(f"{RED}FAIL{RST} {name}: {e}")
            fail += 1
            continue
        if got == want:
            print(f"{GREEN}ok{RST}   {name} [{got}]")
            ok += 1
        else:
            print(f"{RED}FAIL{RST} {name}: want {want!r}, got {got!r}")
            fail += 1

    print(f"\n{ok} parity, {fail} diff")
    return 1 if fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
