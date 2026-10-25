"""Static checker (M8) — the safety core's compile-time pass.

The checker walks the parsed AST and emits structured `.zerr` diagnostics
*before* the program runs. It is the hook every static guarantee plugs into
(M8a name resolution + match shape, M8b capability flow; result/ownership types
follow in M8c).

Soundness contract
------------------
A diagnostic means one of two things, and nothing weaker:

* **A definite unhandled fault** — *if this node executes, it raises a `.zerr`
  that nothing catches.* The checker never guesses about *whether* a node runs
  (dead branches are still checked, exactly like a type error in an unreachable
  arm), and it never reports anything that could succeed on some execution.
  Inside a `ty` body (SPEC §16.1) a fault is the intended `err` path, so
  fault predictions (capability flow) are suppressed there.
* **A violation of a mandatory static rule** the SPEC declares — today: a name
  bound nowhere (well-formedness), malformed `??` arms (well-formedness), and a
  `??` over a result that does not handle both `ok` and `err` (exhaustiveness,
  §16.1). These are reported even inside `ty`: no execution can make them right.

Zero false positives is the invariant; the conformance corpus (every runnable
program must check clean) guards it. Where the checker cannot prove either kind
of finding it stays silent — a runtime `.zerr` still catches the rest.

Checks
------
* **Name resolution (M8a).** A `name` node never bound anywhere (`:name`) and
  not a builtin can never resolve: Zown's environment is a single global map,
  so a name absent from every binding site is *definitely* unresolved →
  static `NAME_UNRESOLVED`.

* **`??` arm shape (M8a).** An arms block literally before `??` is validated:
  `[pattern] [body]` pairs, each pattern a type name / literal / `any`. Patterns
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
  is what keeps `[ `s rq ] :send   `s [ send @ ] gr` clean. Because an escaped
  block may be run under `ty` (which catches the fault), rule 2 is disabled
  altogether in programs that use `ty`.

* **Result exhaustiveness (M8c-ii).** For the literal shape `[ … ] ty [ arms ]
  ??` the subject is known to be a result, so the arms must cover both `ok`
  and `err` — via `[ok]`+`[err]`, `[res]`, or a default `[any]` — else a static
  `NO_MATCH`. A `??` whose subject is not syntactically known to be a result is
  not judged.

* **Use after move (M8c-iv).** `'x` moves a binding out (SPEC §20): afterwards
  `x` is unbound until rebound. Within one node sequence the checker tracks the
  names moved so far; a later `x` (unless `x` is also a builtin, which the
  lookup would fall back to) or a second `'x` is a definite `NAME_UNRESOLVED`.
  A `:x` clears the mark; any node that can run arbitrary code (`@ ; ??`, `gr`,
  `ty`, SIMD constructors) clears *all* marks, since it may have rebound
  anything. Blocks that run immediately at their position (`@`, the `;`
  condition, `? @` branches, `??` bodies, `gr`, `ty`, constructors) inherit the
  marks; the `;` body and escaped blocks start clean. A move of a name that is
  bound nowhere is a well-formedness error (builtins cannot be moved).
"""

from __future__ import annotations

from .builtins import BUILTINS
from .errors import BAD_PATTERN, CAP_DENIED, NAME_UNRESOLVED, NO_MATCH, TYPE_MISMATCH, ZownError
from .vm import _PATTERN_TYPES, _RES_PATTERNS  # one vocabulary, static and runtime

# Builtin words that invoke a block literal *inline*, in the caller's context.
_VEC_CTORS = {"vf", "vd", "vi", "vu"}
_BOX_CTORS = {"rw", "cm"}
# Nodes that may run arbitrary code — and therefore rebind any name.
_INVOKING_OPS = {"@", ";", "??"}
_INVOKING_WORDS = {"gr", "ty", "gd", "gp"} | _VEC_CTORS | _BOX_CTORS
# Host words that require a capability the way `rq` does (SPEC §23).
_HOST_CAP = {"fr": "d", "fw": "d", "mm": "d", "mo": "d", "ml": "d", "en": "v", "tm": "c"}
_NET_CAP = {"~o": "t", "~s": "t", "~c": "t", "~l": "r", "~r": "r"}
_HOST_HINT = {
    "fr": "grant a path: $path$ [ ... ] gd",
    "fw": "grant a path: $path$ [ ... ] gd",
    "mm": "grant a path: $path$ [ ... ] gd",
    "mo": "grant a path: $path$ [ ... ] gd",
    "ml": "grant a path: $path$ [ ... ] gd",
    "en": "grant it: `v [ ... ] gr",
    "tm": "grant it: `c [ ... ] gr",
    "~o": "grant it: `t [ ... ] gr",
    "~s": "grant it: `t [ ... ] gr",
    "~c": "grant it: `t [ ... ] gr",
    "~l": "grant it: `r [ ... ] gr",
    "~r": "grant it: `r [ ... ] gr",
    "~w": "grant it: `r [ ... ] gr and `t [ ... ] gr",
}

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
        # Literal name → protocols a `~l` in this program offers.
        self.offers: dict[str, set[str]] = {}
        # Does the program use `ty`? Then an escaped block may run under it (its
        # faults caught), so the "never granted" rule cannot be applied to them.
        self.uses_at = False

    # --- entry -------------------------------------------------------------------
    def check(self, nodes: list) -> list[ZownError]:
        self._collect_binds(nodes)
        self._collect_grantable(nodes)
        self.uses_at = self._uses_word(nodes, "ty")
        self._collect_offers(nodes)
        self._walk(nodes, frozenset(), in_at=False)  # top level: exactly zero authority
        return self.diags

    def _collect_offers(self, nodes: list) -> None:
        """Literal `$name$ $proto$ <port> ~l` offers that protocol for the name."""
        for i, n in enumerate(nodes):
            if n[0] == "blk":
                self._collect_offers(n[1])
            elif n[0] == "op" and n[1] == "~l" and i >= 3:
                name, proto = nodes[i - 3], nodes[i - 2]
                if name[0] == "str" and proto[0] == "str":
                    self.offers.setdefault(name[1], set()).add(proto[1])

    def _uses_word(self, nodes: list, word: str) -> bool:
        for n in nodes:
            if self._is_word(n, word):
                return True
            if n[0] == "blk" and self._uses_word(n[1], word):
                return True
        return False

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
        if "gr" not in self.bound:
            self._scan_free_caps(nodes)
        if "gd" not in self.bound and self._mentions(nodes, "gd"):
            self.grantable.add("d")

    def _mentions(self, nodes: list, word: str) -> bool:
        for n in nodes:
            if self._is_word(n, word):
                return True
            if n[0] == "blk" and self._mentions(n[1], word):
                return True
        return False

    def _scan_free_caps(self, nodes: list) -> None:
        for i, n in enumerate(nodes):
            if n[0] == "cap":
                nxt = self._at(nodes, i + 1)
                if not (self._is_word(nxt, "rq") or self._is_word(nxt, "hv")):
                    self.grantable.add(n[1])
            elif n[0] == "blk":
                self._scan_free_caps(n[1])

    # --- main walk ---------------------------------------------------------------
    def _walk(self, nodes: list, known, in_at: bool, moved=None) -> None:
        """`known`: exact granted set or UNKNOWN. `in_at`: are we (dynamically)
        inside a `ty` body, where faults are caught and become `err`? `moved`:
        names moved out (`'x`) earlier on this straight-line path and not rebound."""
        moved = set() if moved is None else set(moved)  # this sequence owns its copy
        for i, n in enumerate(nodes):
            tag = n[0]
            if tag == "name":
                name = n[1]
                if name not in self.bound and name not in BUILTINS:
                    self._diag(NAME_UNRESOLVED,
                               f"`{name}` is never bound and is not a builtin",
                               n[2], op=name,
                               hint=f"bind it with :{name}, or check the stdlib word list")
                elif name in moved and name not in BUILTINS and not in_at:
                    self._diag(NAME_UNRESOLVED,
                               f"`{name}` was moved out by '{name} above and not rebound",
                               n[2], op=name,
                               hint=f"a bare name is a borrow; move it last, or rebind with :{name}")
                if name not in self.bound and not in_at and name in _HOST_CAP:
                    self._check_require(_HOST_CAP[name], n[2], known, name)
            elif tag == "move":
                name = n[1]
                if name not in self.bound:
                    self._diag(NAME_UNRESOLVED,
                               f"`'{name}`: {name} is never bound (builtins cannot be moved)",
                               n[2], op=f"'{name}", hint=f"bind it first: value :{name}")
                elif name in moved and not in_at:
                    self._diag(NAME_UNRESOLVED,
                               f"`'{name}`: {name} was already moved out above",
                               n[2], op=f"'{name}",
                               hint=f"rebind it with :{name} before moving it again")
                moved.add(name)
            elif tag == "bind":
                moved.discard(n[1])
            elif tag == "cap":
                nxt = self._at(nodes, i + 1)
                if self._is_word(nxt, "rq") and not in_at:
                    self._check_require(n[1], nxt[2], known, "rq")
            elif tag == "op":
                sym = n[1]
                if not in_at and sym in _NET_CAP:
                    self._check_require(_NET_CAP[sym], n[2], known, sym)
                elif not in_at and sym == "~w":
                    self._check_require("r", n[2], known, sym)
                    self._check_require("t", n[2], known, sym)
                if sym == "~c" and i >= 2 and nodes[i - 1][0] == "str" and nodes[i - 2][0] == "str":
                    name, proto = nodes[i - 2][1], nodes[i - 1][1]
                    offered = self.offers.get(name)
                    if offered is not None and proto not in offered:
                        self._diag(
                            TYPE_MISMATCH,
                            f"`~c` protocol {proto} does not match the {name} listener",
                            n[2], op="~c",
                            hint="the listener's protocol literal and this one differ",
                        )
            elif tag == "blk":
                self._walk_block(nodes, i, known, in_at, moved)
            # Anything that runs code may have rebound any name: forget the marks.
            if (tag == "op" and n[1] in _INVOKING_OPS) or \
                    (tag == "name" and n[1] in _INVOKING_WORDS and n[1] not in self.bound):
                moved.clear()

    def _walk_block(self, nodes: list, i: int, known, in_at: bool, moved: set) -> None:
        """Decide how the block literal at `nodes[i]` is consumed, then walk its
        body with the authority — and the moved-out names — it will *definitely*
        run under. Blocks that run immediately at this position inherit `moved`;
        the `;` body (its condition may rebind) and escaped blocks start clean."""
        blk = nodes[i]
        prev, prev2 = self._at(nodes, i - 1), self._at(nodes, i - 2)
        nxt, nxt2, nxt3 = self._at(nodes, i + 1), self._at(nodes, i + 2), self._at(nodes, i + 3)

        if self._is_op(nxt, "??"):
            # `[ body ] at [ arms ] ??` — the subject is syntactically a result.
            subject_is_res = (self._is_word(prev, "ty") and prev2 is not None
                              and prev2[0] == "blk")
            self._check_match_arms(blk, known, in_at, subject_is_res, moved)
            return

        inherit = moved
        if self._is_op(nxt, "@"):                                   # [b] @
            inner = known
        elif self._is_op(nxt, ";"):                                 # [c] [b] ;  (body)
            inner, inherit = known, None  # the condition ran first and may have rebound
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
        elif self._is_word(nxt, "gd"):                              # path [b] gd
            inner = known | {"d"} if known is not UNKNOWN else UNKNOWN
        elif self._is_word(nxt, "ty"):                              # [b] ty
            inner, in_at = known, True  # runs inline, but its faults are caught
        elif nxt is not None and nxt[0] == "name" and nxt[1] in _VEC_CTORS | _BOX_CTORS | {"gp"} \
                and nxt[1] not in self.bound:                       # [lanes] vi, [nodes] rw/cm, or [shader] gp
            inner = known
        else:
            inner, inherit = UNKNOWN, None  # escapes: bound, on the stack, or unknown consumer
        self._walk(blk[1], inner, in_at, inherit)

    # --- capability flow (M8b) ---------------------------------------------------
    def _check_require(self, cap: str, pos, known, op: str = "rq") -> None:
        if known is not UNKNOWN and cap not in known:
            held = " ".join(f"`{c}" for c in sorted(known)) or "nothing"
            if op == "rq":
                msg = (f"`{cap} is required here but the authority in scope is exactly "
                       f"{held}; this `rq` can never succeed")
                hint = f"run it inside a grant: `{cap} [ ... ] gr"
            else:
                msg = f"`{op}` needs `{cap}, and the authority in scope is exactly {held}"
                hint = _HOST_HINT[op]
            self._diag(CAP_DENIED, msg, pos, op=op, cap=cap, hint=hint)
        elif cap not in self.grantable and not self.uses_at:
            if op == "rq":
                msg = f"`{cap} is required but never granted anywhere in the program"
                hint = f"add a grant scope `{cap} [ ... ] gr around the caller"
            else:
                msg = f"`{op}` needs `{cap}, which is never granted anywhere in the program"
                hint = _HOST_HINT[op]
            self._diag(CAP_DENIED, msg, pos, op=op, cap=cap, hint=hint)

    # --- match arms (M8a) + result exhaustiveness (M8c-ii) -----------------------
    def _check_match_arms(self, blk, known, in_at: bool, subject_is_res: bool,
                          moved: set | None = None) -> None:
        children, pos = blk[1], blk[2]
        if len(children) % 2 != 0:
            self._diag(BAD_PATTERN,
                       f"match arms are [pattern][body] pairs; got {len(children)} blocks",
                       pos, op="??")
            return
        seen: set[str] = set()
        for j in range(0, len(children), 2):
            pat, body = children[j], children[j + 1]
            if pat[0] != "blk" or body[0] != "blk":
                self._diag(BAD_PATTERN, "each match arm is [pattern] [body], both blocks",
                           pos, op="??")
                continue
            label = self._check_pattern(pat)
            if label is not None:
                seen.add(label)
            self._walk(body[1], known, in_at, moved)  # bodies run inline; patterns are not code
        if subject_is_res and not ("any" in seen or "res" in seen or {"ok", "err"} <= seen):
            missing = " and ".join(sorted({"ok", "err"} - seen))
            self._diag(NO_MATCH,
                       f"a result must be handled exhaustively: no arm for {missing}",
                       pos, op="??",
                       hint="add [err] [ ... ] (and/or [ok] [ ... ]), or a default arm [any] [ ... ]")

    def _check_pattern(self, pat) -> str | None:
        """Validate one pattern block; return its label (`any`, a type name, `ok`,
        `err`, or `lit` for a literal) or None if malformed."""
        nodes, pos = pat[1], pat[2]
        if len(nodes) != 1:
            self._diag(BAD_PATTERN,
                       f"a pattern holds exactly one token, got {len(nodes)}",
                       pos, op="??", hint="use [int], [42], [$txt$], [ok], or [any]")
            return None
        n = nodes[0]
        tag = n[0]
        if tag == "name" and n[1] == "any":
            return "any"
        if tag in ("int", "float", "str"):
            return "lit"
        if tag == "name" and (n[1] in _PATTERN_TYPES or n[1] in _RES_PATTERNS):
            return n[1]
        label = n[1] if len(n) > 1 else tag
        self._diag(BAD_PATTERN, f"unrecognized pattern `{label}`", pos, op="??",
                   hint="a pattern is a type name, a literal, ok/err, or any (default)")
        return None


def check_src(src: str, file: str | None = None) -> list[ZownError]:
    """Parse and statically check `src`. Parse/lex errors propagate as ZownError;
    static findings are returned as a list (empty == clean)."""
    from .parser import parse
    nodes = parse(src, file)
    return Checker(file).check(nodes)
