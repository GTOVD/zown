"""Zown parser: token stream -> nested node program.

The grammar is intentionally tiny. A program is a sequence of nodes; a block
`[ ... ]` is itself a sequence of nodes that gets pushed onto the stack as a
first-class value.

Node forms (all tuples, first element is the tag):
    ("int",   int)
    ("float", float)
    ("str",   str)
    ("blk",   [node, ...])
    ("name",  str)        # push the value bound to this name (or run a builtin)
    ("bind",  str)        # pop a value and bind it to this name
    ("cap",   str)        # push a capability token `name (v0.2; SPEC Part II §12)
    ("op",    str)        # an operator symbol
Each node also carries a trailing Pos for diagnostics: (..., pos).
"""

from __future__ import annotations

from .errors import Pos, REPAIR_SYNTAX, ZownError
from .lexer import (
    T_BIND,
    T_CAP,
    T_EOF,
    T_FLOAT,
    T_IDENT,
    T_INT,
    T_LBRACK,
    T_OP,
    T_RBRACK,
    T_STR,
    Token,
    lex,
)

Node = tuple


class Parser:
    def __init__(self, tokens: list[Token], file: str | None = None):
        self.toks = tokens
        self.i = 0
        self.file = file

    def _peek(self) -> Token:
        return self.toks[self.i]

    def _advance(self) -> Token:
        t = self.toks[self.i]
        self.i += 1
        return t

    def parse(self) -> list[Node]:
        nodes = self._sequence(top=True)
        return nodes

    def _sequence(self, top: bool) -> list[Node]:
        nodes: list[Node] = []
        while True:
            t = self._peek()
            if t.kind == T_EOF:
                if not top:
                    raise ZownError(
                        code=REPAIR_SYNTAX,
                        msg="unclosed block: missing `]`",
                        kind="parse",
                        pos=t.pos,
                        hint="every `[` needs a matching `]`",
                        file=self.file,
                    )
                return nodes
            if t.kind == T_RBRACK:
                if top:
                    raise ZownError(
                        code=REPAIR_SYNTAX,
                        msg="unexpected `]` with no matching `[`",
                        kind="parse",
                        pos=t.pos,
                        hint="remove the stray `]` or add a matching `[`",
                        file=self.file,
                    )
                self._advance()
                return nodes
            nodes.append(self._node())

    def _node(self) -> Node:
        t = self._advance()
        if t.kind == T_INT:
            return ("int", t.value, t.pos)
        if t.kind == T_FLOAT:
            return ("float", t.value, t.pos)
        if t.kind == T_STR:
            return ("str", t.value, t.pos)
        if t.kind == T_IDENT:
            return ("name", t.value, t.pos)
        if t.kind == T_BIND:
            return ("bind", t.value, t.pos)
        if t.kind == T_CAP:
            return ("cap", t.value, t.pos)
        if t.kind == T_OP:
            return ("op", t.value, t.pos)
        if t.kind == T_LBRACK:
            inner = self._sequence(top=False)
            return ("blk", inner, t.pos)
        # T_RBRACK / T_EOF handled by caller; anything else is a bug
        raise ZownError(
            code=REPAIR_SYNTAX,
            msg=f"unexpected token {t.kind}",
            kind="parse",
            pos=t.pos,
            file=self.file,
        )


def parse(src: str, file: str | None = None) -> list[Node]:
    return Parser(lex(src, file), file).parse()
