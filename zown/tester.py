"""`zown test` — run Zown programs against goldens, or as pass/fail scripts.

A directory (or one file) is scanned for `*.zn`, top level only:

- `name.code`  the program must trap with that recovery code (`CODE` or `CODE op`)
- `name.out`   the program must succeed and print exactly that text
- neither      the program must succeed; stdout is not compared

The VM is ungated, matching the conformance oracle. `ts` and `pp` are how a
program fails itself without a golden file.
"""

from __future__ import annotations

import io
import os
import sys

from .errors import ZownError
from .vm import VM

GREEN, RED, RST = "\033[32m", "\033[31m", "\033[0m"


def _zn_files(directory: str) -> list[str]:
    return sorted(f for f in os.listdir(directory) if f.endswith(".zn"))


def _run(src: str) -> str:
    buf = io.StringIO()
    VM(file="<test>", out=buf).run_src(src)
    return buf.getvalue()


def _one(path: str) -> tuple[bool, str]:
    """Return (passed, printable line)."""
    name = os.path.basename(path)
    stem, _ = os.path.splitext(path)
    src = open(path, encoding="utf-8").read()
    code_path = stem + ".code"
    out_path = stem + ".out"
    try:
        got = _run(src)
    except ZownError as e:
        got_code = e.code if not e.op else f"{e.code} {e.op}"
        if os.path.exists(code_path):
            want = open(code_path, encoding="utf-8").read().strip()
            if got_code == want:
                return True, f"{GREEN}ok{RST}   {name} [{got_code}]"
            return False, f"{RED}FAIL{RST} {name}: want {want!r}, got {got_code!r}"
        return False, f"{RED}FAIL{RST} {name}: {got_code}: {e.msg}"
    if os.path.exists(code_path):
        want = open(code_path, encoding="utf-8").read().strip()
        return False, f"{RED}FAIL{RST} {name}: expected {want}, it succeeded"
    if os.path.exists(out_path):
        want = open(out_path, encoding="utf-8").read()
        if got == want:
            return True, f"{GREEN}ok{RST}   {name}"
        return False, f"{RED}FAIL{RST} {name}\n  want={want!r}\n  got ={got!r}"
    return True, f"{GREEN}ok{RST}   {name}"


def run_tests(path: str, write=None) -> int:
    """Run `path` (a file or a directory). Return 0 when every program passes."""
    out = write if write is not None else sys.stdout
    if os.path.isfile(path):
        files = [path]
    elif os.path.isdir(path):
        files = [os.path.join(path, name) for name in _zn_files(path)]
        if not files:
            out.write(f"no .zn programs in {path}\n")
            return 1
    else:
        out.write(f"zown test: {path} does not exist\n")
        return 1
    failed = 0
    for fn in files:
        ok, line = _one(fn)
        out.write(line + "\n")
        if not ok:
            failed += 1
    out.write(f"\n{len(files) - failed} passed, {failed} failed\n")
    return 1 if failed else 0
