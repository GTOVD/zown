"""Canonical formatter.

Whitespace between tokens becomes one space. A newline that separated tokens
stays a newline, and blank lines go away. Comments stay on their own line when
they were alone, or at the end of the line when they shared one with code.
The result is idempotent: formatting it again changes nothing.
"""

from __future__ import annotations

from .lexer import T_COMMENT, T_EOF, Lexer


def format_source(src: str) -> str:
    """Return the canonical form of `src`. Lex errors propagate."""
    toks = [t for t in Lexer(src, keep_comments=True).tokens() if t.kind != T_EOF]
    lines: list[str] = []
    buf: list[str] = []
    cur: int | None = None

    def flush() -> None:
        if buf:
            lines.append(" ".join(buf))
            buf.clear()

    for t in toks:
        lexeme = src[t.span[0]:t.span[0] + t.span[1]]
        if t.kind == T_COMMENT:
            if buf and cur == t.pos.line:
                lines.append(" ".join(buf) + " " + lexeme)
                buf.clear()
            else:
                flush()
                lines.append(lexeme)
            cur = None
            continue
        if cur is None:
            cur = t.pos.line
        elif t.pos.line != cur:
            flush()
            cur = t.pos.line
        buf.append(lexeme)
    flush()
    if not lines:
        return "\n" if src.strip() == "" else ""
    return "\n".join(lines) + "\n"
