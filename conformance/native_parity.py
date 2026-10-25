#!/usr/bin/env python3
"""Differential native parity: `zownc native` vs the conformance goldens.

The Cranelift backend compiles each IR block to a native function and runs it
in-process. Int `+ - * _` and comparisons are inlined; other values and words
go through the reference VM, including `??`, `ty`, `gr`, and the vector
constructors. v0.1 and v0.2 must match. A program the backend refuses is a
skip, and the runner fails if the matching set shrinks.

Usage:
    python3 conformance/native_parity.py   # requires: (cd zownc && cargo build)
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
# v0.1 (20) + v0.2 (116). Drops if a program starts getting refused or diverges.
MIN_PASS = 136

GREEN, RED, RST = "\033[32m", "\033[31m", "\033[0m"


class Skip(Exception):
    def __init__(self, why: str) -> None:
        self.why = why


def _skipped(stderr: str) -> str | None:
    for line in stderr.splitlines():
        if line.startswith("zownc native: UNSUPPORTED:"):
            return line.split("UNSUPPORTED:", 1)[1].strip()
    return None


def _env() -> dict[str, str]:
    env = os.environ.copy()
    env["ZOWN_TICK"] = "1"
    env["ZOWN_WALL"] = "0"
    return env


def run_case(path: str) -> str:
    out = subprocess.run(
        [ZOWNC, "native", "--unchecked", path],
        capture_output=True, text=True, env=_env(), cwd=ROOT,
    )
    why = _skipped(out.stderr)
    if why:
        raise Skip(why)
    if out.returncode != 0:
        raise RuntimeError(
            f"exit {out.returncode}\nstdout={out.stdout!r}\nstderr={out.stderr!r}"
        )
    return out.stdout


def run_error(path: str) -> str:
    out = subprocess.run(
        [ZOWNC, "native", "--unchecked", "--zerr", path],
        capture_output=True, text=True, env=_env(), cwd=ROOT,
    )
    why = _skipped(out.stderr)
    if why:
        raise Skip(why)
    if out.returncode == 0:
        raise RuntimeError(f"expected an error, exited 0 with stdout {out.stdout!r}")
    pkt = json.loads(out.stderr)
    return pkt["code"] if not pkt.get("op") else f"{pkt['code']} {pkt['op']}"


def check_cases(directory: str) -> tuple[int, int, int]:
    ok = fail = skip = 0
    for zn in sorted(glob.glob(os.path.join(directory, "*.zn"))):
        name = os.path.basename(zn)[:-3]
        want = open(os.path.join(directory, name + ".out"), encoding="utf-8").read()
        try:
            got = run_case(zn)
        except Skip as e:
            print(f"skip {name} ({e.why})")
            skip += 1
            continue
        except RuntimeError as e:
            print(f"{RED}FAIL{RST} {name}: {e}")
            fail += 1
            continue
        if got == want:
            print(f"{GREEN}ok{RST}   {name}")
            ok += 1
        else:
            print(f"{RED}FAIL{RST} {name}\n  want={want!r}\n  got ={got!r}")
            fail += 1
    return ok, fail, skip


def check_errors(directory: str) -> tuple[int, int, int]:
    ok = fail = skip = 0
    for zn in sorted(glob.glob(os.path.join(directory, "*.zn"))):
        name = os.path.basename(zn)[:-3]
        want = open(os.path.join(directory, name + ".code"), encoding="utf-8").read().strip()
        try:
            got = run_error(zn)
        except Skip as e:
            print(f"skip {name} ({e.why})")
            skip += 1
            continue
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
    return ok, fail, skip


def main() -> int:
    if not os.path.exists(ZOWNC):
        print(f"{RED}error{RST}: {ZOWNC} not found. Build it: (cd zownc && cargo build)")
        return 2

    ok = fail = skip = 0
    for label, cases, errors in (
        ("v0.1", CASES, ERRORS),
        ("v0.2", CASES_V2, ERRORS_V2),
    ):
        print(f"== {label} cases (stdout) ==")
        a, b, c = check_cases(cases)
        ok += a
        fail += b
        skip += c
        print(f"== {label} errors (recovery code) ==")
        a, b, c = check_errors(errors)
        ok += a
        fail += b
        skip += c

    print(f"\n{ok} parity, {skip} skipped, {fail} diff")
    if ok < MIN_PASS:
        print(f"{RED}error{RST}: expected at least {MIN_PASS} matching programs, got {ok}")
        return 1
    return 1 if fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
