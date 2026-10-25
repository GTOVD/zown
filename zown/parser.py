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
    ("move",  str)        # push the value bound to this name and unbind it (v0.2; §20)
    ("cap",   str)        # push a capability token `name (v0.2; SPEC Part II §12)
    ("op",    str)        # an operator symbol
Each node also carries a trailing Pos for diagnostics: (..., pos).
"""

from __future__ import annotations

from .errors import Pos, REPAIR_SYNTAX, ZownError
from .lexer import (
    T_BIND,
    T_CAP,
    T_COMMENT,
    T_EOF,
    T_FLOAT,
    T_IDENT,
    T_INT,
    T_LBRACK,
    T_MOVE,
    T_OP,
    T_RBRACK,
    T_STR,
    Token,
    lex,
)

Node = tuple

# Block nesting allowed before a structured REPAIR_SYNTAX (SPEC §19): the parser
# is recursive, and a pathological `[[[[…]]]]` must be a `.zerr`, not a crash.
MAX_NESTING = 256


class Parser:
    def __init__(self, tokens: list[Token], file: str | None = None):
        self.toks = tokens
        self.i = 0
        self.file = file
        self.depth = 0

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
        if t.kind == T_MOVE:
            return ("move", t.value, t.pos)
        if t.kind == T_CAP:
            return ("cap", t.value, t.pos)
        if t.kind == T_OP:
            return ("op", t.value, t.pos)
        if t.kind == T_LBRACK:
            if self.depth >= MAX_NESTING:
                raise ZownError(
                    code=REPAIR_SYNTAX,
                    msg=f"blocks nested deeper than the limit ({MAX_NESTING})",
                    kind="parse", pos=t.pos, file=self.file,
                    hint="flatten the structure; bind inner blocks to names",
                )
            self.depth += 1
            try:
                inner = self._sequence(top=False)
            finally:
                self.depth -= 1
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


# Leaf tag in the canonical dump → token kind that produced it. `blk` / `end`
# are the block markers and are not tags in this map.
_AST_KIND = {
    "int": T_INT,
    "float": T_FLOAT,
    "str": T_STR,
    "name": T_IDENT,
    "bind": T_BIND,
    "move": T_MOVE,
    "cap": T_CAP,
    "op": T_OP,
}


def ast_dump(src: str, file: str | None = None) -> str:
    """Canonical AST text. `std/parse.zn` prints this same text.

    A block is a `blk` line, the nodes inside indented two spaces, and an
    `end` line back at the block's indent. A leaf is the tag, one space, and
    the raw lexeme: `int 007`, `float 1.50`, `str $a\\n$`. The lexeme is the
    source text, not the value stored on the node.
    """
    nodes = parse(src, file)
    tokens = [t for t in lex(src, file) if t.kind not in (T_EOF, T_COMMENT)]
    lines: list[str] = []
    end = _emit_ast(nodes, tokens, 0, 0, lines, src)
    if end != len(tokens):
        raise RuntimeError("ast dump did not consume the token stream")
    if not lines:
        return ""
    return "\n".join(lines) + "\n"


def _emit_ast(nodes: list[Node], tokens: list[Token], i: int, depth: int,
              lines: list[str], src: str) -> int:
    for node in nodes:
        if i >= len(tokens):
            raise RuntimeError("ast dump ran out of tokens")
        tok = tokens[i]
        ind = "  " * depth
        tag = node[0]
        if tag == "blk":
            if tok.kind != T_LBRACK:
                raise RuntimeError(f"expected `[`, got {tok.kind}")
            lines.append(f"{ind}blk")
            i = _emit_ast(node[1], tokens, i + 1, depth + 1, lines, src)
            if i >= len(tokens) or tokens[i].kind != T_RBRACK:
                got = tokens[i].kind if i < len(tokens) else "end of tokens"
                raise RuntimeError(f"expected `]`, got {got}")
            lines.append(f"{ind}end")
            i += 1
            continue
        expect = _AST_KIND.get(tag)
        if tok.kind != expect:
            raise RuntimeError(f"expected {expect}, got {tok.kind}")
        raw = src[tok.span[0]:tok.span[0] + tok.span[1]]
        lines.append(f"{ind}{tag} {raw}")
        i += 1
    return i
