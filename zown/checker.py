"""Static checker (M8) — the safety core's compile-time pass.

The checker walks the parsed AST and emits structured `.zerr` diagnostics
*before* the program runs. It is the hook every static guarantee plugs into
(M8a name resolution + match shape, M8b capability flow; result/ownership types
follow in M8c).

Soundness contract
------------------
Every diagnostic means: **if this node executes, it definitely faults**. The
checker never guesses about *whether* a node runs (dead branches are still
checked, exactly like a type error in an unreachable arm), and it never reports
anything that could succeed on some execution. Zero false positives is the
invariant; the conformance corpus (every runnable program must check clean)
guards it. Where the checker cannot prove a fault it stays silent — a runtime
`.zerr` still catches the rest.

Checks
------
* **Name resolution (M8a).** A `name` node never bound anywhere (`:name`) and
  not a builtin can never resolve: Zown's environment is a single global map,
  so a name absent from every binding site is *definitely* unresolved →
  static `NAME_UNRESOLVED`.

* **`??` arm shape (M8a).** An arms block literally before `??` is validated:
  `[pattern] [body]` pairs, each pattern a type name / literal / `_`. Patterns
  are introspected, never resolved as names. Dynamically-built arms are skipped.

* **Capability flow (M8b).** `` `c rq `` is a static `CAP_DENIED` when the
  checker can prove `c` is not granted at that point. Grants have *dynamic*
  extent (`gr` holds a capability only while it invokes its block), so this is
  a flow question, not a lexical one. Two facts are provable:

  1. *Exact authority.* Top-level code runs with **exactly zero** grants. A
     block literal that is **anchored** — consumed inline by `@`, `;`, `? @`,
     `??`, a SIMD constructor, or `gr` — runs with exactly the enclosing
     authority (plus `c` for `` `c [ ... ] gr ``). Inside such blocks the
     granted set is still exact, so `c ∉ set` is a definite denial.
  2. *Never granted.* Capability values are created only by `` ` `` literals
     and `gr` needs one on the stack. A literal immediately consumed by `rq`
     or `hv` can never reach `gr`; if no *free* `` `c `` literal exists
     anywhere in the program, `c` can never be granted, so every `` `c rq `` is
     denied — even inside blocks whose invocation context is unknown.

  Anything else — a block bound to a name, left on the stack, selected by `?`
  without an immediate `@`, or granted a capability that arrived through a
  name — makes the authority *unknown*; inside it only rule 2 applies. This
  is what keeps `[ `s rq ] :send   `s [ send @ ] gr` clean.
"""

from __future__ import annotations

from .builtins import BUILTINS
from .errors import BAD_PATTERN, CAP_DENIED, NAME_UNRESOLVED, ZownError
from .vm import _PATTERN_TYPES  # one vocabulary for static and runtime `??` patterns

# Builtin words that invoke a block literal *inline*, in the caller's context.
_VEC_CTORS = {"f4", "d2", "i4", "b16"}

# `known` authority: a frozenset of granted capability names when the set is
# exact, or UNKNOWN when the block may be invoked from any context.
UNKNOWN = None


class Checker:
    def __init__(self, file: str | None = None):
        self.file = file
        self.diags: list[ZownError] = []
        self.bound: set[str] = set()
        # Capability names that could ever be granted (rule 2). Filled by
        # _collect_grantable; a name absent here is never grantable.
        self.grantable: set[str] = set()

    # --- entry -------------------------------------------------------------------
    def check(self, nodes: list) -> list[ZownError]:
        self._collect_binds(nodes)
        self._collect_grantable(nodes)
        self._walk(nodes, frozenset())  # top level: exactly zero authority
        return self.diags

    def _diag(self, code: str, msg: str, pos, op: str | None = None, hint: str = "",
              cap: str | None = None) -> None:
        self.diags.append(ZownError(code=code, msg=msg, kind="check", pos=pos,
                                    op=op, hint=hint, file=self.file, cap=cap))

    # --- helpers -----------------------------------------------------------------
    def _is_word(self, node, word: str) -> bool:
        """`node` is the builtin `word` (a bare name not shadowed by a binding)."""
        return (node is not None and node[0] == "name" and node[1] == word
                and word not in self.bound)

    @staticmethod
    def _is_op(node, sym: str) -> bool:
        return node is not None and node[0] == "op" and node[1] == sym

    @staticmethod
    def _at(nodes: list, i: int):
        return nodes[i] if 0 <= i < len(nodes) else None

    # --- pre-passes --------------------------------------------------------------
    def _collect_binds(self, nodes: list) -> None:
        for n in nodes:
            if n[0] == "bind":
                self.bound.add(n[1])
            elif n[0] == "blk":
                self._collect_binds(n[1])

    def _collect_grantable(self, nodes: list) -> None:
        """Rule 2 pre-pass: every `` `c `` literal not immediately consumed by
        `rq`/`hv` is *free* — it may flow to `gr`. If the grant word itself is
        shadowed by a user binding, nothing can ever be granted."""
        if "gr" in self.bound:
            return
        self._scan_free_caps(nodes)

    def _scan_free_caps(self, nodes: list) -> None:
        for i, n in enumerate(nodes):
            if n[0] == "cap":
                nxt = self._at(nodes, i + 1)
                if not (self._is_word(nxt, "rq") or self._is_word(nxt, "hv")):
                    self.grantable.add(n[1])
            elif n[0] == "blk":
                self._scan_free_caps(n[1])

    # --- main walk ---------------------------------------------------------------
    def _walk(self, nodes: list, known) -> None:
        for i, n in enumerate(nodes):
            tag = n[0]
            if tag == "name":
                name = n[1]
                if name not in self.bound and name not in BUILTINS:
                    self._diag(NAME_UNRESOLVED,
                               f"`{name}` is never bound and is not a builtin",
                               n[2], op=name,
                               hint=f"bind it with :{name}, or check the stdlib word list")
            elif tag == "cap":
                nxt = self._at(nodes, i + 1)
                if self._is_word(nxt, "rq"):
                    self._check_require(n[1], nxt[2], known)
            elif tag == "blk":
                self._walk_block(nodes, i, known)

    def _walk_block(self, nodes: list, i: int, known) -> None:
        """Decide how the block literal at `nodes[i]` is consumed, then walk its
        body with the authority it will *definitely* run under."""
        blk = nodes[i]
        prev, nxt, nxt2, nxt3 = (self._at(nodes, i - 1), self._at(nodes, i + 1),
                                 self._at(nodes, i + 2), self._at(nodes, i + 3))

        if self._is_op(nxt, "??"):
            self._check_match_arms(blk, known)  # validates patterns + walks bodies
            return

        if self._is_op(nxt, "@"):                                   # [b] @
            inner = known
        elif self._is_op(nxt, ";"):                                 # [c] [b] ;  (body)
            inner = known
        elif nxt is not None and nxt[0] == "blk" and self._is_op(nxt2, ";"):
            inner = known                                           # [c] [b] ;  (cond)
        elif self._is_op(nxt, "?") and self._is_op(nxt2, "@"):      # x [t] [e] ? @  (else)
            inner = known
        elif (nxt is not None and nxt[0] == "blk"
              and self._is_op(nxt2, "?") and self._is_op(nxt3, "@")):
            inner = known                                           # x [t] [e] ? @  (then)
        elif self._is_word(nxt, "gr"):                              # `c [b] gr
            if prev is not None and prev[0] == "cap" and known is not UNKNOWN:
                inner = known | {prev[1]}
            else:
                inner = UNKNOWN  # the granted capability came through a name
        elif nxt is not None and nxt[0] == "name" and nxt[1] in _VEC_CTORS \
                and nxt[1] not in self.bound:                       # [lanes] i4
            inner = known
        else:
            inner = UNKNOWN  # escapes: bound, left on the stack, or unknown consumer
        self._walk(blk[1], inner)

    # --- capability flow (M8b) ---------------------------------------------------
    def _check_require(self, cap: str, pos, known) -> None:
        if known is not UNKNOWN and cap not in known:
            held = " ".join(f"`{c}" for c in sorted(known)) or "nothing"
            self._diag(CAP_DENIED,
                       f"`{cap} is required here but the authority in scope is exactly "
                       f"{held}; this `rq` can never succeed",
                       pos, op="rq", cap=cap,
                       hint=f"run it inside a grant: `{cap} [ ... ] gr")
        elif cap not in self.grantable:
            self._diag(CAP_DENIED,
                       f"`{cap} is required but never granted anywhere in the program",
                       pos, op="rq", cap=cap,
                       hint=f"add a grant scope `{cap} [ ... ] gr around the caller")

    # --- match arms (M8a) --------------------------------------------------------
    def _check_match_arms(self, blk, known) -> None:
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
            self._walk(body[1], known)  # bodies run inline; patterns are not code

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
    from .parser import parse
    nodes = parse(src, file)
    return Checker(file).check(nodes)
