"""Zown lexer.

A two-state machine: MODE_CODE and MODE_LITERAL. Code is read as self-delimiting
symbol tokens via a direct dispatch on the byte; strings are bounded by `$` and
swept as raw data. This mirrors the SIMD/zero-copy design described in the notes;
this reference build is a straightforward scalar version of the same state machine
(spans are recorded as (offset,length) so a future zero-copy backend can drop in).
"""

from __future__ import annotations

from dataclasses import dataclass

from .errors import Pos, REPAIR_SYNTAX, ZownError

# --- token kinds ---------------------------------------------------------------
T_INT = "INT"
T_FLOAT = "FLOAT"
T_STR = "STR"
T_IDENT = "IDENT"
T_BIND = "BIND"        # :name  -> value of `name` is the binding target
T_LBRACK = "LBRACK"    # [
T_RBRACK = "RBRACK"    # ]
T_OP = "OP"            # any operator symbol
T_EOF = "EOF"

# Two-character operators (checked before single-char).
TWO_CHAR_OPS = {"==", "!=", "<=", ">=", "&&", "||"}
# Single-character operators.
ONE_CHAR_OPS = set("@.,=\\&+-*/%<>!?;_")

_IDENT_START = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ")
_IDENT_CONT = _IDENT_START | set("0123456789")
_DIGITS = set("0123456789")
_WS = set(" \t\r\n")

_ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "\\": "\\", "$": "$", "0": "\0"}


@dataclass
class Token:
    kind: str
    value: object
    pos: Pos
    # raw source span for zero-copy / tooling
    span: tuple[int, int]  # (offset, length)

    def __repr__(self) -> str:  # pragma: no cover - debug aid
        return f"Token({self.kind}, {self.value!r}@{self.pos.line}:{self.pos.col})"


class Lexer:
    def __init__(self, src: str, file: str | None = None):
        self.src = src
        self.file = file
        self.i = 0
        self.line = 1
        self.col = 1

    # --- low level cursor ------------------------------------------------------
    def _peek(self, k: int = 0) -> str:
        j = self.i + k
        return self.src[j] if j < len(self.src) else ""

    def _advance(self) -> str:
        c = self.src[self.i]
        self.i += 1
        if c == "\n":
            self.line += 1
            self.col = 1
        else:
            self.col += 1
        return c

    def _pos(self) -> Pos:
        return Pos(self.line, self.col, self.i)

    def _err(self, code: str, msg: str, hint: str = "") -> ZownError:
        return ZownError(code=code, msg=msg, kind="lex", pos=self._pos(), hint=hint, file=self.file)

    # --- main loop -------------------------------------------------------------
    def tokens(self) -> list[Token]:
        out: list[Token] = []
        while self.i < len(self.src):
            c = self._peek()
            if c in _WS:
                self._advance()
                continue
            if c == "#":  # line comment to end of line
                while self.i < len(self.src) and self._peek() != "\n":
                    self._advance()
                continue
            if c == "$":
                out.append(self._read_string())
                continue
            if c == "[":
                p, start = self._pos(), self.i
                self._advance()
                out.append(Token(T_LBRACK, "[", p, (start, 1)))
                continue
            if c == "]":
                p, start = self._pos(), self.i
                self._advance()
                out.append(Token(T_RBRACK, "]", p, (start, 1)))
                continue
            if c == ":":
                out.append(self._read_bind())
                continue
            if c in _DIGITS:
                out.append(self._read_number())
                continue
            if c in _IDENT_START:
                out.append(self._read_ident())
                continue
            tok = self._read_op()
            if tok is not None:
                out.append(tok)
                continue
            raise self._err(
                REPAIR_SYNTAX,
                f"unexpected character {c!r}",
                hint="character is not a Zown operator, literal, or identifier start",
            )
        out.append(Token(T_EOF, None, self._pos(), (self.i, 0)))
        return out

    # --- readers ---------------------------------------------------------------
    def _read_string(self) -> Token:
        p, start = self._pos(), self.i
        self._advance()  # opening $
        buf: list[str] = []
        while True:
            if self.i >= len(self.src):
                raise self._err(
                    REPAIR_SYNTAX,
                    "unterminated string literal",
                    hint="add a closing `$`; strings are bounded by $...$",
                )
            c = self._advance()
            if c == "\\":
                nxt = self._advance() if self.i < len(self.src) else ""
                buf.append(_ESCAPES.get(nxt, nxt))
                continue
            if c == "$":
                break
            buf.append(c)
        length = self.i - start
        return Token(T_STR, "".join(buf), p, (start, length))

    def _read_bind(self) -> Token:
        p, start = self._pos(), self.i
        self._advance()  # the ':'
        if self._peek() not in _IDENT_START:
            raise self._err(
                REPAIR_SYNTAX,
                "expected an identifier after `:`",
                hint="binding form is `:name`, e.g. `5:x` binds 5 to x",
            )
        name_chars: list[str] = []
        while self._peek() in _IDENT_CONT:
            name_chars.append(self._advance())
        name = "".join(name_chars)
        return Token(T_BIND, name, p, (start, self.i - start))

    def _read_number(self) -> Token:
        p, start = self._pos(), self.i
        while self._peek() in _DIGITS:
            self._advance()
        is_float = False
        # `.` is the print op; only consume it as a decimal point if a digit follows.
        if self._peek() == "." and self._peek(1) in _DIGITS:
            is_float = True
            self._advance()  # '.'
            while self._peek() in _DIGITS:
                self._advance()
        text = self.src[start:self.i]
        span = (start, self.i - start)
        if is_float:
            return Token(T_FLOAT, float(text), p, span)
        return Token(T_INT, int(text), p, span)

    def _read_ident(self) -> Token:
        p, start = self._pos(), self.i
        while self._peek() in _IDENT_CONT:
            self._advance()
        return Token(T_IDENT, self.src[start:self.i], p, (start, self.i - start))

    def _read_op(self) -> Token | None:
        p, start = self._pos(), self.i
        two = self.src[self.i:self.i + 2]
        if two in TWO_CHAR_OPS:
            self._advance()
            self._advance()
            return Token(T_OP, two, p, (start, 2))
        c = self._peek()
        if c == "|":  # lone '|' is reserved (atomic pipe, future); '||' handled above
            raise self._err(
                REPAIR_SYNTAX,
                "lone `|` is reserved",
                hint="use `||` for logical or; `|` is reserved for the future atomic-pipe operator",
            )
        if c in ONE_CHAR_OPS:
            self._advance()
            return Token(T_OP, c, p, (start, 1))
        return None


def lex(src: str, file: str | None = None) -> list[Token]:
    return Lexer(src, file).tokens()
