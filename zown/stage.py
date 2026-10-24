"""Run the Zown lexer and compare it with the host lexer (SPEC §28)."""

from __future__ import annotations

import os

from .lexer import token_dump
from .vm import VM


class StageMismatch(Exception):
    def __init__(self, msg: str) -> None:
        super().__init__(msg)
        self.msg = msg


def lex_path() -> str:
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "std", "lex.zn")


def quote(src: str) -> str:
    """A Zown string literal that evaluates to `src`."""
    out = ["$"]
    for ch in src:
        if ch == "\\":
            out.append("\\\\")
        elif ch == "$":
            out.append("\\$")
        elif ch == "\n":
            out.append("\\n")
        elif ch == "\r":
            out.append("\\r")
        elif ch == "\t":
            out.append("\\t")
        elif ch == "\0":
            out.append("\\0")
        else:
            out.append(ch)
    out.append("$")
    return "".join(out)


def program(src: str, lexer: str | None = None) -> str:
    body = lexer if lexer is not None else open(lex_path(), encoding="utf-8").read()
    return quote(src) + " :z\n" + body + "\nlx\n"


def run_lexer(src: str, out=None) -> VM:
    vm = VM(file="<stage>", out=out)
    vm.run_src(program(src))
    return vm


def zown_dump(src: str) -> str:
    import io
    buf = io.StringIO()
    run_lexer(src, out=buf)
    return buf.getvalue()


def stage_file(path: str) -> str:
    """Token dump of `path`. Raises ZownError if the host or the Zown lexer fails.

    The two dumps must match; a mismatch is a ZownError so the CLI can print it.
    """
    with open(path, encoding="utf-8") as fh:
        src = fh.read()
    host = token_dump(src)
    got = zown_dump(src)
    if got != host:
        raise StageMismatch(_diff(host, got))
    return got


def _diff(host: str, got: str) -> str:
    hl, gl = host.splitlines(), got.splitlines()
    n = min(len(hl), len(gl))
    for i in range(n):
        if hl[i] != gl[i]:
            return f"line {i + 1}\n  host {hl[i]}\n  zown {gl[i]}"
    if len(hl) != len(gl):
        return f"host has {len(hl)} lines, zown has {len(gl)}"
    return "dumps differ"
