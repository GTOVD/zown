#!/usr/bin/env python3
"""Differential AST parity: Python reference vs native `zownc`.

For every valid `.zn` program (conformance cases + examples), parse it with both
frontends and assert the ASTs are structurally identical. This is the M3 gate in
docs/PLAN.md: the Rust frontend must agree with the oracle before we build the VM
and backends on top of it.

Usage:
    python3 conformance/ast_parity.py
"""

from __future__ import annotations

import glob
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ZOWNC = os.path.join(ROOT, "zownc", "target", "debug", "zownc")
PY = sys.executable

GREEN, RED, RST = "\033[32m", "\033[31m", "\033[0m"


def py_ast(path: str):
    out = subprocess.run(
        [PY, os.path.join(ROOT, "bin", "zown"), "ast", path],
        capture_output=True, text=True,
    )
    if out.returncode != 0:
        raise RuntimeError(f"python ast failed: {out.stderr.strip()}")
    return json.loads(out.stdout)


def rust_ast(path: str):
    out = subprocess.run([ZOWNC, "ast", path], capture_output=True, text=True)
    if out.returncode != 0:
        raise RuntimeError(f"zownc ast failed: {out.stderr.strip()}")
    return json.loads(out.stdout)


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
            p, r = py_ast(path), rust_ast(path)
        except RuntimeError as e:
            print(f"{RED}FAIL{RST} {rel}: {e}")
            fail += 1
            continue
        if p == r:
            print(f"{GREEN}ok{RST}   {rel}")
            ok += 1
        else:
            print(f"{RED}FAIL{RST} {rel}\n  py  ={json.dumps(p)}\n  rust={json.dumps(r)}")
            fail += 1

    print(f"\n{ok} parity, {fail} diff")
    return 1 if fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
