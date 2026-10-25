"""Run a Zown frontend and compare it with the host (SPEC §28)."""

from __future__ import annotations

import os

from .lexer import token_dump
from .parser import ast_dump
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


def parse_path() -> str:
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "std", "parse.zn")


def parse_program(src: str, parser: str | None = None) -> str:
    body = parser if parser is not None else open(parse_path(), encoding="utf-8").read()
    return quote(token_dump(src)) + " :d\n" + body + "\npx\n"


def zown_ast(src: str) -> str:
    import io
    buf = io.StringIO()
    vm = VM(file="<stage>", out=buf)
    vm.run_src(parse_program(src))
    return buf.getvalue()


def stage_parse(path: str) -> str:
    """Canonical AST of `path`. The host parser and `std/parse.zn` must match.

    A host lex or parse error propagates. A dump mismatch is a StageMismatch.
    """
    with open(path, encoding="utf-8") as fh:
        src = fh.read()
    host = ast_dump(src, path)
    got = zown_ast(src)
    if got != host:
        raise StageMismatch(_diff(host, got))
    return got


def lower_path() -> str:
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "std", "lower.zn")


def lower_program(src: str, lower: str | None = None, file: str | None = None) -> str:
    """Bind the canonical AST of `src` to `a` and append `lw`."""
    body = lower if lower is not None else open(lower_path(), encoding="utf-8").read()
    return quote(ast_dump(src, file)) + " :a\n" + body + "\nlw\n"


def zown_ir(src: str, file: str | None = None) -> str:
    """IR text of `src`, printed by `std/lower.zn`."""
    import io
    buf = io.StringIO()
    vm = VM(file="<stage>", out=buf)
    vm.run_src(lower_program(src, file=file))
    return buf.getvalue()


def stage_lower(path: str) -> str:
    """IR text of `path`. The same text `zownc ir` prints; parity is stage2_parity."""
    with open(path, encoding="utf-8") as fh:
        src = fh.read()
    return zown_ir(src, path)


def emit_path() -> str:
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "std", "emit.zn")


def emit_program(src: str, emitter: str | None = None) -> str:
    """Bind a print-subset program to `z` and append `ex`."""
    body = emitter if emitter is not None else open(emit_path(), encoding="utf-8").read()
    return quote(src) + " :z\n" + body + "\nex\n"


def image_from_hex(text: str) -> bytes:
    """512 bytes from the lowercase hex `ex` prints."""
    raw = "".join(text.split())
    if len(raw) != 1024 or any(c not in "0123456789abcdef" for c in raw):
        raise StageMismatch(
            f"emitter printed {len(raw)} hex digits, want 1024 lowercase digits"
        )
    return bytes.fromhex(raw)


def zown_image(src: str) -> bytes:
    """Boot sector of a print-subset program, built by `std/emit.zn`."""
    import io
    buf = io.StringIO()
    vm = VM(file="<stage>", out=buf)
    vm.run_src(emit_program(src))
    return image_from_hex(buf.getvalue())


def stage_emit(path: str) -> bytes:
    """512-byte boot sector of `path`. The source is the print subset in SPEC §29."""
    with open(path, encoding="utf-8") as fh:
        src = fh.read()
    return zown_image(src)


def _diff(host: str, got: str) -> str:
    hl, gl = host.splitlines(), got.splitlines()
    n = min(len(hl), len(gl))
    for i in range(n):
        if hl[i] != gl[i]:
            return f"line {i + 1}\n  host {hl[i]}\n  zown {gl[i]}"
    if len(hl) != len(gl):
        return f"host has {len(hl)} lines, zown has {len(gl)}"
    return "dumps differ"
