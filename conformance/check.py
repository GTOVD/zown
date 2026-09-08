#!/usr/bin/env python3
"""Zown static-checker conformance runner (oracle-only).

`zown check` (PLAN M8) reports faults *before* a program runs. Its findings are
observable behavior, so they are pinned as language-agnostic goldens exactly like
runtime output: when the checker is ported to Rust (`zownc check`, PLAN M8d) it
must reproduce these files.

Layout:
  check/<name>.zn    -> check/<name>.diag

A `.diag` file holds one line per diagnostic, in source order:
  <CODE> <op>            e.g.  NAME_UNRESOLVED zz
  <CODE> <op> <cap>      e.g.  CAP_DENIED rq s     (security packets carry the cap)
or the single word `ok` when the program must check clean. Clean cases matter as
much as faulting ones: they pin the checker's *soundness* (no false positives).

Usage:
    python3 conformance/check.py            # check against goldens
    python3 conformance/check.py --bless    # (re)generate goldens from the oracle
    python3 conformance/check.py --list     # list cases
"""

from __future__ import annotations

import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

from zown.checker import check_src  # noqa: E402
from zown.errors import ZownError  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
CASES = os.path.join(HERE, "check")

GREEN, RED, DIM, RST = "\033[32m", "\033[31m", "\033[2m", "\033[0m"


def _render(src: str, name: str) -> str:
    """Run the checker and render its findings in the golden format."""
    diags = check_src(src, name)
    if not diags:
        return "ok\n"
    lines = []
    for d in diags:
        line = d.code if not d.op else f"{d.code} {d.op}"
        if d.cap is not None:
            line += f" {d.cap}"
        lines.append(line)
    return "\n".join(lines) + "\n"


def _zn_files() -> list[str]:
    if not os.path.isdir(CASES):
        return []
    return sorted(f for f in os.listdir(CASES) if f.endswith(".zn"))


def main(argv: list[str]) -> int:
    bless = "--bless" in argv
    if "--list" in argv:
        for fn in _zn_files():
            print("check ", fn)
        return 0
    ok = fail = 0
    print("== static checker (oracle-only) ==")
    for fn in _zn_files():
        name = fn[:-3]
        src = open(os.path.join(CASES, fn), encoding="utf-8").read()
        diag_path = os.path.join(CASES, name + ".diag")
        try:
            got = _render(src, fn)
        except ZownError as e:  # a lex/parse error is not a checker finding
            print(f"{RED}ERROR{RST} {name}: case failed to parse: {e.code}: {e.msg}")
            fail += 1
            continue
        if bless:
            open(diag_path, "w", encoding="utf-8", newline="\n").write(got)
            print(f"{DIM}blessed{RST} {name}.diag -> {got.strip().replace(chr(10), ' | ')}")
            ok += 1
            continue
        if not os.path.exists(diag_path):
            print(f"{RED}MISSING{RST} {name}.diag (run --bless)")
            fail += 1
            continue
        want = open(diag_path, encoding="utf-8").read()
        if got == want:
            print(f"{GREEN}ok{RST}   {name} [{want.strip().replace(chr(10), ' | ')}]")
            ok += 1
        else:
            print(f"{RED}FAIL{RST} {name}\n  want={want!r}\n  got ={got!r}")
            fail += 1
    print(f"\n{ok} passed, {fail} failed")
    return 1 if fail else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
