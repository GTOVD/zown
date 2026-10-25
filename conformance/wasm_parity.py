#!/usr/bin/env python3
"""WASM backend parity (M6, extended in M22) vs the goldens.

Compiles each conformance program with `zownc build` to both WebAssembly text
(`.wat`) and binary (`.wasm`), runs each under `wasmtime`, and diffs it against
the oracle golden. A success case counts only when both formats match stdout.
An error case counts when both formats exit nonzero and the stderr `.zerr`
carries the golden code and op.

Anything the backend cannot compile yet is a SKIP with the compiler's reason.
Host words this slice does not lower (files, buffers, crypto, lanes, the mesh)
stay skipped on purpose. If `wasmtime` itself is missing, the run records that
skip and does not pretend the corpus passed under a runtime.

Usage:
    python3 conformance/wasm_parity.py    # requires: cargo build; wasmtime if present
"""

from __future__ import annotations

import glob
import json
import os
import shutil
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ZOWNC = os.path.join(ROOT, "zownc", "target", "debug",
                     "zownc.exe" if os.name == "nt" else "zownc")
CASE_DIRS = [
    os.path.join(ROOT, "conformance", "cases"),
    os.path.join(ROOT, "conformance", "cases_v2"),
]
ERROR_DIRS = [
    os.path.join(ROOT, "conformance", "errors"),
    os.path.join(ROOT, "conformance", "errors_v2"),
]

GREEN, RED, YEL = "\033[32m", "\033[31m", "\033[33m"
RST = "\033[0m"


def find_wasmtime() -> str | None:
    w = shutil.which("wasmtime")
    if w:
        return w
    cand = os.path.expanduser("~/.wasmtime/bin/wasmtime")
    return cand if os.path.exists(cand) else None


def build_reason(stderr: str) -> str:
    text = stderr.strip().replace("zownc build: ", "")
    return text.splitlines()[0] if text else "unsupported"


def packet_matches(stderr: str, golden: str) -> bool:
    """True when a `.zerr` on stderr carries the golden `CODE` or `CODE op`."""
    parts = golden.split(" ", 1)
    code = parts[0]
    op = parts[1] if len(parts) == 2 else None
    for line in stderr.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            pkt = json.loads(line)
        except json.JSONDecodeError:
            continue
        if pkt.get("code") != code:
            continue
        if op is not None and pkt.get("op") != op:
            continue
        return True
    return False


def run_both(wasmtime: str, src: str) -> tuple[str | None, list[subprocess.CompletedProcess[str]]]:
    """Build and run `.wat` and `.wasm`. A string result is a skip reason."""
    runs: list[subprocess.CompletedProcess[str]] = []
    for ext in (".wat", ".wasm"):
        with tempfile.NamedTemporaryFile(suffix=ext, delete=False) as tf:
            out = tf.name
        build = subprocess.run([ZOWNC, "build", src, "-o", out], capture_output=True, text=True)
        if build.returncode != 0:
            os.unlink(out)
            return build_reason(build.stderr), runs
        run = subprocess.run([wasmtime, "run", out], capture_output=True, text=True)
        os.unlink(out)
        runs.append(run)
    return None, runs


def main() -> int:
    if not os.path.exists(ZOWNC):
        print(f"{RED}error{RST}: build zownc first: (cd zownc && cargo build)")
        return 2
    wasmtime = find_wasmtime()
    if not wasmtime:
        print(f"{YEL}skip{RST} wasm_parity  (wasmtime not found on PATH or ~/.wasmtime/bin)")
        return 0

    ok = fail = skip = 0

    def skipped(name: str, reason: str) -> None:
        nonlocal skip
        print(f"{YEL}skip{RST} {name}  ({reason})")
        skip += 1

    for folder in CASE_DIRS:
        for zn in sorted(glob.glob(os.path.join(folder, "*.zn"))):
            name = os.path.basename(zn)[:-3]
            want = open(os.path.join(folder, name + ".out"), encoding="utf-8").read()
            reason, runs = run_both(wasmtime, zn)
            if reason is not None:
                skipped(name, reason)
                continue
            bad = None
            for ext, run in zip((".wat", ".wasm"), runs):
                if run.returncode != 0 or run.stdout != want:
                    bad = (ext, run)
                    break
            if bad:
                ext, run = bad
                print(f"{RED}FAIL{RST} {name} ({ext}) exit={run.returncode}\n"
                      f"  want={want!r}\n  got ={run.stdout!r}\n  err ={run.stderr.strip()[:300]}")
                fail += 1
            else:
                print(f"{GREEN}ok{RST}   {name}  (.wat + .wasm under wasmtime)")
                ok += 1

    for folder in ERROR_DIRS:
        for zn in sorted(glob.glob(os.path.join(folder, "*.zn"))):
            name = os.path.basename(zn)[:-3]
            code_path = os.path.join(folder, name + ".code")
            if not os.path.exists(code_path):
                continue
            golden = open(code_path, encoding="utf-8").read().strip()
            reason, runs = run_both(wasmtime, zn)
            if reason is not None:
                skipped(name, reason)
                continue
            bad = None
            for ext, run in zip((".wat", ".wasm"), runs):
                if run.returncode == 0 or not packet_matches(run.stderr, golden):
                    bad = (ext, run)
                    break
            if bad:
                ext, run = bad
                print(f"{RED}FAIL{RST} {name} ({ext}) exit={run.returncode} want={golden}\n"
                      f"  err={run.stderr.strip()[:300]}")
                fail += 1
            else:
                print(f"{GREEN}ok{RST}   {name}  (.zerr {golden})")
                ok += 1

    print(f"\n{ok} wasm-parity, {fail} fail, {skip} skip")
    return 1 if fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
