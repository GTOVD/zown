#!/usr/bin/env python3
"""WASM backend parity (M6) vs the goldens.

Compiles each conformance program with the native backend (`zownc build`) to both
WebAssembly text (`.wat`) and binary (`.wasm`), runs each under `wasmtime`, and
diffs stdout against the same golden the oracle is held to. A case only counts as
`ok` when *both* formats match. Anything the backend can't compile yet reports
SKIP with the reason, so coverage grows visibly as the backend does.

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

        case_failed = False
        for ext in (".wat", ".wasm"):
            with tempfile.NamedTemporaryFile(suffix=ext, delete=False) as tf:
                out = tf.name
            build = subprocess.run([ZOWNC, "build", zn, "-o", out], capture_output=True, text=True)
            if build.returncode != 0:
                # Unsupported construct: report once and stop on this case.
                reason = build.stderr.strip().replace("zownc build: ", "")
                print(f"{YEL}skip{RST} {name}  ({reason.splitlines()[0] if reason else 'unsupported'})")
                skip += 1
                os.unlink(out)
                case_failed = None  # signal "skipped", not failed
                break
            run = subprocess.run([wasmtime, "run", out], capture_output=True, text=True)
            os.unlink(out)
            if run.returncode != 0:
                print(f"{RED}FAIL{RST} {name} ({ext}): wasmtime error: {run.stderr.strip()}")
                case_failed = True
                break
            if run.stdout != want:
                print(f"{RED}FAIL{RST} {name} ({ext})\n  want={want!r}\n  got ={run.stdout!r}")
                case_failed = True
                break

        if case_failed is None:
            continue
        if case_failed:
            fail += 1
        else:
            print(f"{GREEN}ok{RST}   {name}  (.wat + .wasm under wasmtime)")
            ok += 1

    print(f"\n{ok} wasm-parity, {fail} fail, {skip} skip")
    return 1 if fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
