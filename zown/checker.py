"""Static checker (M8a) — the safety core's first pass.

The checker walks the parsed AST and emits structured `.zerr` diagnostics
*before* the program runs. It is the hook every later static guarantee plugs
into (capability flow in M8b; ownership/result types in M8c). This first slice
ships two sound, no-false-positive checks:

  * **Name resolution.** A `name` node that is never bound anywhere in the
    program (`:name`) and is not a builtin can never resolve at runtime — Zown's
    environment is a single global map, so a name absent from every binding site
    is *definitely* unresolved. We surface it as a static `NAME_UNRESOLVED`
    instead of a run-time surprise on the unlucky path.

  * **`??` arm shape.** When an arms block literally precedes `??`
    (`[ ... ] ??`), we validate it statically: pairs of `[pattern] [body]`, each
    pattern a type name / literal / `_`. Dynamically-built arms are skipped
    (the checker only asserts what it can prove).

Patterns are introspected, never executed, so the type-name tokens inside a
pattern (e.g. `int`) are *not* treated as names to resolve.
"""

from __future__ import annotations

from .builtins import BUILTINS
from .errors import BAD_PATTERN, NAME_UNRESOLVED, ZownError
from .parser import parse

# Pattern tokens recognized by `??` (mirrors vm._PATTERN_TYPES).
_PATTERN_TYPES = {"int", "float", "str", "bool", "block", "cap", "width", "vec"}


class Checker:
    def __init__(self, file: str | None = None):
        self.file = file
        self.diags: list[ZownError] = []
        self.bound: set[str] = set()

    def check(self, nodes: list) -> list[ZownError]:
        self._collect_binds(nodes)
        self._walk(nodes)
        return self.diags

    def _diag(self, code: str, msg: str, pos, op: str | None = None, hint: str = "") -> None:
        self.diags.append(ZownError(code=code, msg=msg, kind="check", pos=pos,
                                    op=op, hint=hint, file=self.file))

    def _collect_binds(self, nodes: list) -> None:
        for n in nodes:
            if n[0] == "bind":
                self.bound.add(n[1])
            elif n[0] == "blk":
                self._collect_binds(n[1])

    def _walk(self, nodes: list) -> None:
        for i, n in enumerate(nodes):
            tag = n[0]
            if tag == "name":
                name = n[1]
                if name not in self.bound and name not in BUILTINS:
                    self._diag(NAME_UNRESOLVED,
                               f"`{name}` is never bound and is not a builtin",
                               n[2], op=name,
                               hint=f"bind it with :{name}, or check the stdlib word list")
            elif tag == "blk":
                nxt = nodes[i + 1] if i + 1 < len(nodes) else None
                if nxt is not None and nxt[0] == "op" and nxt[1] == "??":
                    self._check_match_arms(n)  # validates patterns + walks bodies
                else:
                    self._walk(n[1])

    def _check_match_arms(self, blk) -> None:
        children, pos = blk[1], blk[2]
        if len(children) % 2 != 0:
            self._diag(BAD_PATTERN,
                       f"match arms are [pattern][body] pairs; got {len(children)} blocks",
                       pos, op="??")
            return
        for j in range(0, len(children), 2):
            pat, body = children[j], children[j + 1]
            if pat[0] != "blk" or body[0] != "blk":
                self._diag(BAD_PATTERN, "each match arm is [pattern] [body], both blocks",
                           pos, op="??")
                continue
            self._check_pattern(pat)
            self._walk(body[1])  # bodies are ordinary code; patterns are not

    def _check_pattern(self, pat) -> None:
        nodes, pos = pat[1], pat[2]
        if len(nodes) != 1:
            self._diag(BAD_PATTERN,
                       f"a pattern holds exactly one token, got {len(nodes)}",
                       pos, op="??", hint="use [int], [42], [$txt$], or [_]")
            return
        n = nodes[0]
        tag = n[0]
        if tag == "op" and n[1] == "_":
            return
        if tag in ("int", "float", "str"):
            return
        if tag == "name" and n[1] in _PATTERN_TYPES:
            return
        label = n[1] if len(n) > 1 else tag
        self._diag(BAD_PATTERN, f"unrecognized pattern `{label}`", pos, op="??",
                   hint="a pattern is a type name, a literal, or _ (default)")


def check_src(src: str, file: str | None = None) -> list[ZownError]:
    """Parse and statically check `src`. Parse/lex errors propagate as ZownError;
    static findings are returned as a list (empty == clean)."""
    nodes = parse(src, file)
    return Checker(file).check(nodes)
