#!/usr/bin/env python3
"""Differential VM parity: native `zownc run` vs the golden expectations.

Runs every conformance program through the native toolchain and checks it against
the same goldens the Python oracle is held to. v0.1 lives in `cases/` and
`errors/`; v0.2 (checked int, big, widths, vectors, capabilities, `??`, `at`,
moves) lives in `cases_v2/` and `errors_v2/`. `zownc run` is ungated, matching
`v2.py`, which drives the oracle VM directly.

Usage:
    python3 conformance/vm_parity.py     # requires: (cd zownc && cargo build)
"""

from __future__ import annotations

import glob
import json
import os
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ZOWNC = os.path.join(ROOT, "zownc", "target", "debug",
                     "zownc.exe" if os.name == "nt" else "zownc")
CASES = os.path.join(ROOT, "conformance", "cases")
ERRORS = os.path.join(ROOT, "conformance", "errors")
CASES_V2 = os.path.join(ROOT, "conformance", "cases_v2")
ERRORS_V2 = os.path.join(ROOT, "conformance", "errors_v2")

GREEN, RED, RST = "\033[32m", "\033[31m", "\033[0m"


def run_case(path: str) -> str:
    out = subprocess.run([ZOWNC, "run", "--unchecked", path], capture_output=True, text=True)
    return out.stdout


def run_error(path: str) -> str:
    out = subprocess.run(
        [ZOWNC, "run", "--unchecked", "--zerr", path], capture_output=True, text=True
    )
    if out.returncode == 0:
        raise RuntimeError(f"expected an error, exited 0 with stdout {out.stdout!r}")
    pkt = json.loads(out.stderr)
    return pkt["code"] if not pkt.get("op") else f"{pkt['code']} {pkt['op']}"


def check_cases(directory: str) -> tuple[int, int]:
    ok = fail = 0
    for zn in sorted(glob.glob(os.path.join(directory, "*.zn"))):
        name = os.path.basename(zn)[:-3]
        want = open(os.path.join(directory, name + ".out"), encoding="utf-8").read()
        got = run_case(zn)
        if got == want:
            print(f"{GREEN}ok{RST}   {name}")
            ok += 1
        else:
            print(f"{RED}FAIL{RST} {name}\n  want={want!r}\n  got ={got!r}")
            fail += 1
    return ok, fail


def check_errors(directory: str) -> tuple[int, int]:
    ok = fail = 0
    for zn in sorted(glob.glob(os.path.join(directory, "*.zn"))):
        name = os.path.basename(zn)[:-3]
        want = open(os.path.join(directory, name + ".code"), encoding="utf-8").read().strip()
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
    return ok, fail


def main() -> int:
    if not os.path.exists(ZOWNC):
        print(f"{RED}error{RST}: {ZOWNC} not found. Build it: (cd zownc && cargo build)")
        return 2

    ok = fail = 0
    for label, cases, errors in (
        ("v0.1", CASES, ERRORS),
        ("v0.2", CASES_V2, ERRORS_V2),
    ):
        print(f"== {label} cases (stdout) ==")
        a, b = check_cases(cases)
        ok += a
        fail += b
        print(f"== {label} errors (recovery code) ==")
        a, b = check_errors(errors)
        ok += a
        fail += b

    print(f"\n{ok} parity, {fail} diff")
    return 1 if fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
