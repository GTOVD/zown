#!/usr/bin/env python3
"""Zown conformance runner.

Language-agnostic golden tests that define v0.1 observable behavior. Each case is
a `.zn` source plus a golden expectation:

  cases/<name>.zn   -> cases/<name>.out    (expected stdout)
  errors/<name>.zn  -> errors/<name>.code  (expected "<CODE>" or "<CODE> <op>")

The Python reference VM is the oracle. The same `.zn` files will later be run
through the native `zownc` toolchain to prove parity (see docs/PLAN.md M4).

Usage:
    python3 conformance/run.py            # check against goldens
    python3 conformance/run.py --bless    # (re)generate goldens from the oracle
    python3 conformance/run.py --list     # list cases
"""

from __future__ import annotations

import io
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

from zown.errors import ZownError  # noqa: E402
from zown.vm import VM  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
CASES = os.path.join(HERE, "cases")
ERRORS = os.path.join(HERE, "errors")

GREEN, RED, DIM, RST = "\033[32m", "\033[31m", "\033[2m", "\033[0m"


def _run(src: str) -> str:
    buf = io.StringIO()
    VM(file="<conformance>", out=buf).run_src(src)
    return buf.getvalue()


def _zn_files(d: str) -> list[str]:
    if not os.path.isdir(d):
        return []
    return sorted(f for f in os.listdir(d) if f.endswith(".zn"))


def check_cases(bless: bool) -> tuple[int, int]:
    ok = fail = 0
    for fn in _zn_files(CASES):
        name = fn[:-3]
        src = open(os.path.join(CASES, fn), encoding="utf-8").read()
        out_path = os.path.join(CASES, name + ".out")
        try:
            got = _run(src)
        except ZownError as e:
            print(f"{RED}ERROR{RST} {name}: case raised {e.code}: {e.msg}")
            fail += 1
            continue
        if bless:
            open(out_path, "w", encoding="utf-8").write(got)
            print(f"{DIM}blessed{RST} {name}.out")
            ok += 1
            continue
        if not os.path.exists(out_path):
            print(f"{RED}MISSING{RST} {name}.out (run --bless)")
            fail += 1
            continue
        want = open(out_path, encoding="utf-8").read()
        if got == want:
            print(f"{GREEN}ok{RST}   {name}")
            ok += 1
        else:
            print(f"{RED}FAIL{RST} {name}\n  want={want!r}\n  got ={got!r}")
            fail += 1
    return ok, fail


def check_errors(bless: bool) -> tuple[int, int]:
    ok = fail = 0
    for fn in _zn_files(ERRORS):
        name = fn[:-3]
        src = open(os.path.join(ERRORS, fn), encoding="utf-8").read()
        code_path = os.path.join(ERRORS, name + ".code")
        try:
            out = _run(src)
            print(f"{RED}FAIL{RST} {name}: expected an error, got output {out!r}")
            fail += 1
            continue
        except ZownError as e:
            got = e.code if not e.op else f"{e.code} {e.op}"
        if bless:
            open(code_path, "w", encoding="utf-8").write(got + "\n")
            print(f"{DIM}blessed{RST} {name}.code -> {got}")
            ok += 1
            continue
        if not os.path.exists(code_path):
            print(f"{RED}MISSING{RST} {name}.code (run --bless)")
            fail += 1
            continue
        want = open(code_path, encoding="utf-8").read().strip()
        if got == want:
            print(f"{GREEN}ok{RST}   {name} [{got}]")
            ok += 1
        else:
            print(f"{RED}FAIL{RST} {name}: want {want!r}, got {got!r}")
            fail += 1
    return ok, fail


def main(argv: list[str]) -> int:
    bless = "--bless" in argv
    if "--list" in argv:
        for fn in _zn_files(CASES):
            print("case  ", fn)
        for fn in _zn_files(ERRORS):
            print("error ", fn)
        return 0
    print("== cases ==")
    co, cf = check_cases(bless)
    print("== errors ==")
    eo, ef = check_errors(bless)
    total_ok, total_fail = co + eo, cf + ef
    print(f"\n{total_ok} passed, {total_fail} failed")
    return 1 if total_fail else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
