"""Zown stack virtual machine (reference interpreter).

Execution model
---------------
Zown is a stack-based (concatenative) language. Most values live on a single
operand stack rather than in named variables; this keeps programs dense because
the model rarely needs identifiers at all.

Naming rules (v0.1):
    :name   pops the top value and binds `name` -> that value.
    name    pushes the value bound to `name`. If `name` is not a user binding
            but is a builtin word, the builtin executes instead.
    @       pops a block and executes it.

Blocks `[ ... ]` are first-class values (quotations). Control flow is built from
blocks + operators:
    cond [then] [else] ?    -> select: leaves `then` if cond is truthy else `else`
    [cond] [body] ;         -> while: run body while cond evaluates truthy
    subject [arms] ??       -> pattern match (v0.2): first [pattern][body] arm wins

v0.2 additions (SPEC Part II): capability tokens (`c) with `gr`/`rq`/`hv`,
fixed-width integer tags (i8..u128) with `wr`/`st`/`ck`, and SIMD vectors
(f4 d2 i4 b16). Static checks that run before execution live in checker.py.
"""

from __future__ import annotations

import sys
from dataclasses import dataclass
from typing import Any, Callable

from .errors import (
    BOUNDS,
    DIV_ZERO,
    NAME_UNRESOLVED,
    NOT_CALLABLE,
    OVERFLOW,
    STACK_OVERFLOW,
    STACK_UNDERFLOW,
    TYPE_MISMATCH,
    UB_TRAP,
    Pos,
    ZownError,
)
from .lexer import INT_MAX, INT_MIN
from .parser import Node, parse

# Nested block invocations allowed before a structured STACK_OVERFLOW (SPEC §19).
# Implementation-defined; the oracle keeps it modest so unbounded recursion is
# always a `.zerr`, never a host-language crash. Native runtimes set their own.
MAX_INVOKE_DEPTH = 128


@dataclass
class Block:
    """A first-class quotation: a sequence of parsed nodes."""

    nodes: list[Node]

    def __repr__(self) -> str:  # pragma: no cover
        return f"Block(len={len(self.nodes)})"


@dataclass
class Big:
    """An arbitrary-precision integer (v0.2; SPEC Part II §11, decision D1).

    The default `int` is a *checked* 64-bit integer: an int-typed result outside
    [INT_MIN, INT_MAX] is a structured OVERFLOW, never a silent wrap. `big` is the
    explicit opt-in to unbounded precision — `n b` or `$digits$ b` — and any
    arithmetic touching a big stays big (and exact). Mixing with a float gives a
    float (IEEE-754), the same promotion rule as int.
    """

    v: int

    def __repr__(self) -> str:  # pragma: no cover
        return f"Big({self.v})"


@dataclass
class Res:
    """A result value (v0.2; SPEC Part II §16.1, decision D3): `ok(payload)` or
    `err(code)`.

    `[ body ] at` produces one: the body's single value wrapped as ok, or the
    `.zerr` code of the failure it raised wrapped as err. The payload is
    reachable **only** through `??` with an `[ok]` / `[err]` arm — every other
    operator sees an opaque value — so the error path can never be skipped by
    accident. This is the in-language half of "errors are instructions".
    """

    ok: bool
    val: Any  # the value (ok) or the recovery code string (err)

    def __repr__(self) -> str:  # pragma: no cover
        return f"{'ok' if self.ok else 'err'}({self.val!r})"


@dataclass
class Cap:
    """A capability token (v0.2; SPEC Part II §12).

    A `Cap` *names* an authority (e.g. `s = net-send). Holding the token on the
    stack is not the same as being *granted* the authority: a program starts with
    zero grants and a capability must be brought into scope with `gr` before `rq`
    will accept it. This is "zero authority by default" made concrete.
    """

    name: str

    def __repr__(self) -> str:  # pragma: no cover
        return f"`{self.name}"


@dataclass
class WidthTag:
    """A fixed-width integer type tag (v0.2; SPEC Part II §11).

    The default Zown integer is arbitrary precision. A `WidthTag` (pushed by words
    `i8 i16 i32 i64 i128 u8 u16 u32 u64 u128`) names a bounded representation; the
    policy words `wr`/`st`/`ck` then reduce a value into that width. Overflow is
    therefore *never silent* — it is always wrap, saturate, or a checked trap.
    """

    signed: bool
    bits: int

    @property
    def name(self) -> str:
        return f"{'i' if self.signed else 'u'}{self.bits}"

    @property
    def lo(self) -> int:
        return -(1 << (self.bits - 1)) if self.signed else 0

    @property
    def hi(self) -> int:
        return (1 << (self.bits - 1)) - 1 if self.signed else (1 << self.bits) - 1

    @property
    def fits_int(self) -> bool:
        """Does the width's *entire* range fit the checked 64-bit `int`? A policy
        word (`wr`/`st`/`ck`) yields an `int` for such widths (i8..i64, u8..u32)
        and a `big` otherwise (u64, i128, u128) — the type is decided by the
        width, not by the value, so it is static."""
        return self.lo >= INT_MIN and self.hi <= INT_MAX

    def __repr__(self) -> str:  # pragma: no cover
        return self.name


@dataclass
class Vec:
    """A fixed-lane SIMD vector (v0.2; SPEC Part II §11).

    Lane count and lane type are fixed by the constructor word (`f4` = f32x4,
    `d2` = f64x2, `i4` = i32x4, `b16` = u8x16). Elementwise ops (`vadd`/`vsub`/
    `vmul`) require matching vector types; integer lanes wrap to their width, the
    same "no silent overflow" rule as scalar fixed-width ints.

    Note: the oracle stores float lanes as Python floats (f64). It does not model
    f32 rounding — that precision detail is deferred to the native backend (M9);
    here the observable semantics are lane count, type compatibility, and wrap.
    """

    name: str          # "f4" | "d2" | "i4" | "b16"
    count: int
    is_float: bool
    signed: bool       # meaningful only for integer lanes
    bits: int          # 32/64 (float label) or integer lane width
    lanes: list

    def __repr__(self) -> str:  # pragma: no cover
        return f"{self.name}({len(self.lanes)})"


def truthy(v: Any) -> bool:
    if isinstance(v, Block):
        return len(v.nodes) > 0
    if isinstance(v, str):
        return len(v) > 0
    if isinstance(v, (Cap, WidthTag)):
        return True  # a token is always a "present" value
    if isinstance(v, Vec):
        return len(v.lanes) > 0
    if isinstance(v, Big):
        return v.v != 0
    if isinstance(v, Res):
        return v.ok  # a result is truthy iff it is ok (its payload stays wrapped)
    return bool(v)


# --- numeric model helpers (SPEC §1/§11) ----------------------------------------
def is_num(v: Any) -> bool:
    """int (checked i64), float (IEEE-754), or big — never bool."""
    return not isinstance(v, bool) and isinstance(v, (int, float, Big))


def ival(v: Any) -> Any:
    """Unwrap a big to its Python int; other values pass through."""
    return v.v if isinstance(v, Big) else v


def fits_int(n: int) -> bool:
    return INT_MIN <= n <= INT_MAX


class VM:
    def __init__(self, file: str | None = None, out=None):
        self.stack: list[Any] = []
        self.env: dict[str, Any] = {}
        # Granted capabilities. Empty by default: a program has zero authority
        # until a `gr` block brings a capability into scope (SPEC Part II §12).
        self.caps: set[str] = set()
        self.depth = 0  # current block-invocation depth (see MAX_INVOKE_DEPTH)
        self.file = file
        self.out = out if out is not None else sys.stdout
        # builtins are wired up lazily to avoid an import cycle
        from .builtins import BUILTINS

        self.builtins: dict[str, Callable[["VM", Pos], None]] = BUILTINS

    # --- stack helpers ---------------------------------------------------------
    def push(self, v: Any) -> None:
        self.stack.append(v)

    def pop(self, op: str, pos: Pos | None = None) -> Any:
        if not self.stack:
            raise self.err(STACK_UNDERFLOW, f"`{op}` needs a value but the stack is empty",
                           op=op, pos=pos, hint="push a value before this operator")
        return self.stack.pop()

    def pop_num(self, op: str, pos: Pos | None = None) -> Any:
        """Pop an int, float, or big (TYPE_MISMATCH otherwise)."""
        v = self.pop(op, pos)
        if not is_num(v):
            raise self.err(TYPE_MISMATCH, f"`{op}` expected a number, got {type_name(v)}",
                           op=op, pos=pos, hint="this operator only works on numbers")
        return v

    # --- numeric results (SPEC §11: overflow is never silent) -------------------
    def int_result(self, op: str, pos: Pos | None, n: int) -> int:
        """An int-typed result must fit the checked 64-bit int, else OVERFLOW."""
        if not fits_int(n):
            raise self.err(OVERFLOW, f"`{op}` result {n} does not fit int "
                           f"[{INT_MIN}..{INT_MAX}]", op=op, pos=pos,
                           hint="convert an operand with `b` for arbitrary-precision big math")
        return n

    def to_float(self, op: str, pos: Pos | None, v: Any) -> float:
        """Promote a number to float; a big beyond the float range is OVERFLOW."""
        if isinstance(v, Big):
            try:
                return float(v.v)
            except OverflowError:
                raise self.err(OVERFLOW, f"`{op}`: big value is too large for a float",
                               op=op, pos=pos, hint="keep the computation in big (no float operand)")
        return float(v)

    def int_from_float(self, op: str, pos: Pos | None, f: float) -> int:
        """Convert a whole float to int: nan is a UB_TRAP, ±inf an OVERFLOW, and a
        finite value beyond the int range an OVERFLOW (never a garbage int)."""
        if f != f:
            raise self.err(UB_TRAP, f"`{op}` cannot convert nan to an integer", op=op, pos=pos,
                           hint="guard with a comparison: nan is not equal to itself")
        if f in (float("inf"), float("-inf")):
            raise self.err(OVERFLOW, f"`{op}` cannot convert {f} to an integer", op=op, pos=pos)
        return self.int_result(op, pos, int(f))

    def pop_block(self, op: str, pos: Pos | None = None) -> Block:
        v = self.pop(op, pos)
        if not isinstance(v, Block):
            raise self.err(NOT_CALLABLE, f"`{op}` expected a block, got {type_name(v)}",
                           op=op, pos=pos, hint="wrap the code in [ ... ] to make a block")
        return v

    def pop_cap(self, op: str, pos: Pos | None = None) -> Cap:
        v = self.pop(op, pos)
        if not isinstance(v, Cap):
            raise self.err(TYPE_MISMATCH, f"`{op}` expected a capability `name, got {type_name(v)}",
                           op=op, pos=pos, hint="write a capability token like `s (net-send)")
        return v

    def err(self, code: str, msg: str, op: str | None = None, pos: Pos | None = None,
            hint: str = "", kind: str = "run", cap: str | None = None) -> ZownError:
        return ZownError(code=code, msg=msg, kind=kind, op=op, pos=pos,
                         stack=list(self.stack), hint=hint, file=self.file, cap=cap)

    # --- execution -------------------------------------------------------------
    def run_src(self, src: str) -> None:
        self.run(parse(src, self.file))

    def run(self, nodes: list[Node]) -> None:
        for node in nodes:
            self.exec_node(node)

    def exec_node(self, node: Node) -> None:
        tag = node[0]
        if tag == "int" or tag == "float" or tag == "str":
            self.push(node[1])
        elif tag == "blk":
            self.push(Block(node[1]))
        elif tag == "name":
            self._exec_name(node[1], node[2])
        elif tag == "bind":
            self.env[node[1]] = self.pop(f":{node[1]}", node[2])
        elif tag == "move":
            self._exec_move(node[1], node[2])
        elif tag == "cap":
            self.push(Cap(node[1]))
        elif tag == "op":
            self._exec_op(node[1], node[2])
        else:  # pragma: no cover
            raise self.err("UNSUPPORTED", f"unknown node {tag}")

    def _exec_name(self, name: str, pos: Pos) -> None:
        if name in self.env:
            self.push(self.env[name])
            return
        fn = self.builtins.get(name)
        if fn is not None:
            fn(self, pos)
            return
        raise self.err(NAME_UNRESOLVED, f"`{name}` is not bound and is not a builtin",
                       op=name, pos=pos,
                       hint="bind it with `:%s` or check the stdlib word list" % name)

    def _exec_move(self, name: str, pos: Pos) -> None:
        # 'name: the binding's value moves onto the stack and the name is unbound
        # (SPEC §20). Builtins are not bindings, so they cannot be moved.
        if name in self.env:
            self.push(self.env.pop(name))
            return
        raise self.err(NAME_UNRESOLVED, f"`'{name}`: {name} is not bound; a move needs a live binding",
                       op=f"'{name}", pos=pos,
                       hint=f"was {name} already moved out? rebind it with :{name} first")

    def invoke(self, blk: Block, op: str | None = None, pos: Pos | None = None) -> None:
        """Run a block. `op`/`pos` name the invoking site for diagnostics."""
        if self.depth >= MAX_INVOKE_DEPTH:
            raise self.err(STACK_OVERFLOW,
                           f"block invocations nested deeper than the limit ({MAX_INVOKE_DEPTH})",
                           op=op, pos=pos,
                           hint="unbounded recursion? give it a base case, or iterate with `;`")
        self.depth += 1
        try:
            self.run(blk.nodes)
        finally:
            self.depth -= 1

    # --- operators -------------------------------------------------------------
    def _exec_op(self, op: str, pos: Pos) -> None:
        handler = _OPS.get(op)
        if handler is None:  # pragma: no cover - lexer should prevent this
            raise self.err("UNSUPPORTED", f"operator `{op}` is not implemented",
                           op=op, pos=pos)
        handler(self, pos)


def type_name(v: Any) -> str:
    if isinstance(v, bool):
        return "bool"
    if isinstance(v, Block):
        return "block"
    if isinstance(v, Cap):
        return "cap"
    if isinstance(v, WidthTag):
        return "width"
    if isinstance(v, Vec):
        return "vec"
    if isinstance(v, Big):
        return "big"
    if isinstance(v, Res):
        return "res"
    if isinstance(v, int):
        return "int"
    if isinstance(v, float):
        return "float"
    if isinstance(v, str):
        return "str"
    return type(v).__name__


# --- operator implementations --------------------------------------------------
def _num_result(v: Any) -> Any:
    """Collapse a whole float produced by numeric math back to int — only when it
    fits the checked int; larger whole floats (and inf/nan) stay floats."""
    if isinstance(v, float) and v.is_integer() and fits_int(int(v)):
        return int(v)
    return v


def _numeric(vm: VM, op: str, pos: Pos, a: Any, b: Any, fn) -> Any:
    """Binary arithmetic with Zown's promotion rule (SPEC §11):
       int ∘ int   -> int   (checked: OVERFLOW if the result leaves the int range)
       big involved -> big  (exact, unbounded) — unless a float is involved
       float involved -> float (IEEE-754 binary64; inf/nan are values)"""
    _require_nums(vm, op, pos, a, b)
    if isinstance(a, float) or isinstance(b, float):
        return fn(vm.to_float(op, pos, a), vm.to_float(op, pos, b))
    r = fn(ival(a), ival(b))
    if isinstance(a, Big) or isinstance(b, Big):
        return Big(r)
    return vm.int_result(op, pos, r)


def op_add(vm: VM, pos: Pos) -> None:
    b = vm.pop("+", pos)
    a = vm.pop("+", pos)
    if isinstance(a, str) or isinstance(b, str):
        vm.push(_as_str(a) + _as_str(b))  # `+` concatenates when either side is text
        return
    vm.push(_numeric(vm, "+", pos, a, b, lambda x, y: x + y))


def op_sub(vm: VM, pos: Pos) -> None:
    b = vm.pop_num("-", pos)
    a = vm.pop_num("-", pos)
    vm.push(_numeric(vm, "-", pos, a, b, lambda x, y: x - y))


def op_mul(vm: VM, pos: Pos) -> None:
    b = vm.pop("*", pos)
    a = vm.pop("*", pos)
    if isinstance(a, str) and isinstance(b, int) and not isinstance(b, bool):
        vm.push(a * b)  # str repeat: $ab$ 3 *  (a count <= 0 yields "" — defined)
        return
    vm.push(_numeric(vm, "*", pos, a, b, lambda x, y: x * y))


def _is_zero(v: Any) -> bool:
    return ival(v) == 0


def op_div(vm: VM, pos: Pos) -> None:
    b = vm.pop_num("/", pos)
    a = vm.pop_num("/", pos)
    if _is_zero(b):
        raise vm.err(DIV_ZERO, "division by zero", op="/", pos=pos,
                     hint="guard the denominator with `=0 ==` before dividing")
    if isinstance(a, float) or isinstance(b, float):
        vm.push(_num_result(vm.to_float("/", pos, a) / vm.to_float("/", pos, b)))
        return
    av, bv = ival(a), ival(b)
    if av % bv == 0:  # exact quotient stays integral (big if either side is big)
        q = av // bv
        vm.push(Big(q) if isinstance(a, Big) or isinstance(b, Big) else vm.int_result("/", pos, q))
        return
    try:
        vm.push(av / bv)  # true division -> float (correctly rounded, even for big)
    except OverflowError:
        raise vm.err(OVERFLOW, "`/`: quotient is too large for a float", op="/", pos=pos)


def op_mod(vm: VM, pos: Pos) -> None:
    b = vm.pop_num("%", pos)
    a = vm.pop_num("%", pos)
    if _is_zero(b):
        raise vm.err(DIV_ZERO, "modulo by zero", op="%", pos=pos)
    # Python semantics: the result takes the sign of the divisor (defined, SPEC §19).
    vm.push(_numeric(vm, "%", pos, a, b, lambda x, y: x % y))


def _cmp(vm: VM, op: str, pos: Pos, fn) -> None:
    b = vm.pop(op, pos)
    a = vm.pop(op, pos)
    try:
        vm.push(1 if fn(ival(a), ival(b)) else 0)  # big compares by value
    except TypeError:
        raise vm.err(TYPE_MISMATCH, f"`{op}` cannot compare {type_name(a)} and {type_name(b)}",
                     op=op, pos=pos)


def op_eq(vm: VM, pos: Pos) -> None:
    _cmp(vm, "==", pos, lambda a, b: a == b)


def op_ne(vm: VM, pos: Pos) -> None:
    _cmp(vm, "!=", pos, lambda a, b: a != b)


def op_lt(vm: VM, pos: Pos) -> None:
    _cmp(vm, "<", pos, lambda a, b: a < b)


def op_gt(vm: VM, pos: Pos) -> None:
    _cmp(vm, ">", pos, lambda a, b: a > b)


def op_le(vm: VM, pos: Pos) -> None:
    _cmp(vm, "<=", pos, lambda a, b: a <= b)


def op_ge(vm: VM, pos: Pos) -> None:
    _cmp(vm, ">=", pos, lambda a, b: a >= b)


def op_and(vm: VM, pos: Pos) -> None:
    b = vm.pop("&&", pos)
    a = vm.pop("&&", pos)
    vm.push(1 if (truthy(a) and truthy(b)) else 0)


def op_or(vm: VM, pos: Pos) -> None:
    b = vm.pop("||", pos)
    a = vm.pop("||", pos)
    vm.push(1 if (truthy(a) or truthy(b)) else 0)


def op_not(vm: VM, pos: Pos) -> None:
    a = vm.pop("!", pos)
    vm.push(0 if truthy(a) else 1)


def op_neg(vm: VM, pos: Pos) -> None:
    a = vm.pop_num("_", pos)
    if isinstance(a, Big):
        vm.push(Big(-a.v))
    elif isinstance(a, float):
        vm.push(-a)
    else:
        vm.push(vm.int_result("_", pos, -a))  # -INT_MIN does not fit: OVERFLOW


def op_dup(vm: VM, pos: Pos) -> None:
    if not vm.stack:
        raise vm.err(STACK_UNDERFLOW, "`=` (dup) needs a value", op="=", pos=pos)
    vm.push(vm.stack[-1])


def op_drop(vm: VM, pos: Pos) -> None:
    vm.pop(",", pos)


def op_swap(vm: VM, pos: Pos) -> None:
    b = vm.pop("\\", pos)
    a = vm.pop("\\", pos)
    vm.push(b)
    vm.push(a)


def op_over(vm: VM, pos: Pos) -> None:
    if len(vm.stack) < 2:
        raise vm.err(STACK_UNDERFLOW, "`&` (over) needs two values", op="&", pos=pos)
    vm.push(vm.stack[-2])


def op_print(vm: VM, pos: Pos) -> None:
    v = vm.pop(".", pos)
    vm.out.write(_as_str(v))
    vm.out.write("\n")


def op_invoke(vm: VM, pos: Pos) -> None:
    blk = vm.pop_block("@", pos)
    vm.invoke(blk, "@", pos)


def op_select(vm: VM, pos: Pos) -> None:
    # cond [then] [else] ?   ->  pushes the chosen block (does not run it)
    els = vm.pop_block("?", pos)
    then = vm.pop_block("?", pos)
    cond = vm.pop("?", pos)
    vm.push(then if truthy(cond) else els)


def op_while(vm: VM, pos: Pos) -> None:
    # [cond] [body] ;
    body = vm.pop_block(";", pos)
    cond = vm.pop_block(";", pos)
    while True:
        vm.invoke(cond, ";", pos)
        if not truthy(vm.pop(";", pos)):
            break
        vm.invoke(body, ";", pos)


# Pattern kinds a `??` arm may use (SPEC Part II §16). Patterns are *introspected*
# (read from the AST), not executed: a type name, a literal, `_` (default), or a
# result destructurer (`ok` / `err`, §16.1). The checker imports these sets so
# static and runtime pattern vocabularies agree.
_PATTERN_TYPES = {"int", "float", "str", "bool", "block", "cap", "width", "vec", "big", "res"}
_RES_PATTERNS = {"ok", "err"}


def _pat_desc(n: Node) -> str:
    tag = n[0]
    if tag in ("int", "float", "str", "name", "op"):
        return str(n[1])
    return tag


def _pattern_matches(vm: VM, subject: Any, pat_nodes: list, pos: Pos) -> tuple[bool, Any]:
    """Does the pattern match the subject? Returns (matched, value_for_body): the
    body receives the subject itself, or the *unwrapped* payload for `ok`/`err`."""
    from .errors import BAD_PATTERN
    if len(pat_nodes) != 1:
        raise vm.err(BAD_PATTERN,
                     f"a pattern holds exactly one token, got {len(pat_nodes)}",
                     op="??", pos=pos, hint="use [int], [42], [$txt$], [ok], or [_] (default)")
    n = pat_nodes[0]
    tag = n[0]
    if tag == "op" and n[1] == "_":
        return True, subject  # wildcard / default arm
    if tag in ("int", "float", "str"):
        return ival(subject) == n[1], subject  # literal patterns compare by value (big too)
    if tag == "name" and n[1] in _RES_PATTERNS:
        if isinstance(subject, Res) and subject.ok == (n[1] == "ok"):
            return True, subject.val  # the only way to reach a result's payload
        return False, subject
    if tag == "name" and n[1] in _PATTERN_TYPES:
        return type_name(subject) == n[1], subject
    raise vm.err(BAD_PATTERN, f"unrecognized pattern `{_pat_desc(n)}`",
                 op="??", pos=pos,
                 hint="a pattern is a type name, a literal, ok/err, or _ (default)")


def op_match(vm: VM, pos: Pos) -> None:
    # subject [ [pat][body] [pat][body] ... ] ??
    # First arm whose pattern matches runs; its body gets the subject on the stack
    # (or the unwrapped payload, for an ok/err arm over a result).
    from .errors import BAD_PATTERN, NO_MATCH
    arms = vm.pop_block("??", pos)
    subject = vm.pop("??", pos)
    nodes = arms.nodes
    if len(nodes) % 2 != 0:
        raise vm.err(BAD_PATTERN,
                     f"match arms are [pattern][body] pairs; got {len(nodes)} blocks",
                     op="??", pos=pos)
    for i in range(0, len(nodes), 2):
        pat, body = nodes[i], nodes[i + 1]
        if pat[0] != "blk" or body[0] != "blk":
            raise vm.err(BAD_PATTERN, "each arm is [pattern] [body], both blocks",
                         op="??", pos=pos)
        matched, value = _pattern_matches(vm, subject, pat[1], pos)
        if matched:
            vm.push(value)  # the matched value is available to the body
            vm.invoke(Block(body[1]), "??", pos)
            return
    what = ("err result" if isinstance(subject, Res) and not subject.ok
            else "ok result" if isinstance(subject, Res) else f"{type_name(subject)} subject")
    raise vm.err(NO_MATCH, f"no arm matched the {what}", op="??", pos=pos,
                 hint="handle it: [err] [ ... ], or add a default arm [_] [ ... ]"
                 if isinstance(subject, Res) else "add a default arm: [_] [ ... ]")


_OPS: dict[str, Callable[[VM, Pos], None]] = {
    "+": op_add, "-": op_sub, "*": op_mul, "/": op_div, "%": op_mod,
    "==": op_eq, "!=": op_ne, "<": op_lt, ">": op_gt, "<=": op_le, ">=": op_ge,
    "&&": op_and, "||": op_or, "!": op_not, "_": op_neg,
    "=": op_dup, ",": op_drop, "\\": op_swap, "&": op_over,
    ".": op_print, "@": op_invoke, "?": op_select, ";": op_while,
    "??": op_match,
}


def _as_str(v: Any) -> str:
    if isinstance(v, bool):
        return "1" if v else "0"
    if isinstance(v, float):
        # Whole floats that fit the int range display as integers (`7.0` -> `7`);
        # larger ones keep float notation (`1e+30`); inf/-inf/nan print as such.
        if v.is_integer() and fits_int(int(v)):
            return str(int(v))
        return str(v)
    if isinstance(v, Big):
        return str(v.v)
    if isinstance(v, Res):
        return f"{'ok' if v.ok else 'err'}({_as_str(v.val)})"
    if isinstance(v, Block):
        return f"[blk:{len(v.nodes)}]"
    if isinstance(v, Cap):
        return f"`{v.name}"
    if isinstance(v, WidthTag):
        return v.name
    if isinstance(v, Vec):
        return f"{v.name}({' '.join(_as_str(x) for x in v.lanes)})"
    return str(v)


def _require_nums(vm: VM, op: str, pos: Pos, *vals: Any) -> None:
    for v in vals:
        if not is_num(v):
            raise vm.err(TYPE_MISMATCH, f"`{op}` expected numbers, got {type_name(v)}",
                         op=op, pos=pos)


def run_source(src: str, file: str | None = None, out=None) -> VM:
    vm = VM(file=file, out=out)
    vm.run_src(src)
    return vm
