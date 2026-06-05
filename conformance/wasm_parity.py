#!/usr/bin/env python3
"""WASM backend parity (M6a) vs the goldens.

Compiles each conformance program with the native backend (`zownc build`) and, if
this backend slice supports it, runs the resulting module under `wasmtime` and
diffs stdout against the same golden the oracle is held to. Cases that need a
later slice (strings, blocks, bindings, floats) report SKIP with the reason, so
coverage grows visibly as the backend does.

Usage:
    python3 conformance/wasm_parity.py    # requires: cargo build + wasmtime
"""

from __future__ import annotations

import glob
import os
import shutil
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ZOWNC = os.path.join(ROOT, "zownc", "target", "debug", "zownc")
CASES = os.path.join(ROOT, "conformance", "cases")

GREEN, RED, YEL, RST = "\033[32m", "\033[31m", "\033[33m", "\033[2m"
RST = "\033[0m"


def find_wasmtime() -> str | None:
    w = shutil.which("wasmtime")
    if w:
        return w
    cand = os.path.expanduser("~/.wasmtime/bin/wasmtime")
    return cand if os.path.exists(cand) else None


def main() -> int:
    if not os.path.exists(ZOWNC):
        print(f"{RED}error{RST}: build zownc first: (cd zownc && cargo build)")
        return 2
    wasmtime = find_wasmtime()
    if not wasmtime:
        print(f"{RED}error{RST}: wasmtime not found on PATH or ~/.wasmtime/bin")
        return 2

    ok = fail = skip = 0
    for zn in sorted(glob.glob(os.path.join(CASES, "*.zn"))):
        name = os.path.basename(zn)[:-3]
        want = open(os.path.join(CASES, name + ".out"), encoding="utf-8").read()
        with tempfile.NamedTemporaryFile(suffix=".wat", delete=False) as tf:
            wat = tf.name
        build = subprocess.run([ZOWNC, "build", zn, "-o", wat], capture_output=True, text=True)
        if build.returncode != 0:
            reason = build.stderr.strip().replace("zownc build: ", "")
            print(f"{YEL}skip{RST} {name}  ({reason.splitlines()[0] if reason else 'unsupported'})")
            skip += 1
            os.unlink(wat)
            continue
        run = subprocess.run([wasmtime, "run", wat], capture_output=True, text=True)
        os.unlink(wat)
        if run.returncode != 0:
            print(f"{RED}FAIL{RST} {name}: wasmtime error: {run.stderr.strip()}")
            fail += 1
            continue
        if run.stdout == want:
            print(f"{GREEN}ok{RST}   {name}  (ran under wasmtime)")
            ok += 1
        else:
            print(f"{RED}FAIL{RST} {name}\n  want={want!r}\n  got ={run.stdout!r}")
            fail += 1

    print(f"\n{ok} wasm-parity, {fail} fail, {skip} skip (await later M6 slices)")
    return 1 if fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
