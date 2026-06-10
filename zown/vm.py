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
    n [body] tm             -> run body n times (index pushed each iteration)
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
    STACK_UNDERFLOW,
    TYPE_MISMATCH,
    Pos,
    ZownError,
)
from .parser import Node, parse


@dataclass
class Block:
    """A first-class quotation: a sequence of parsed nodes."""

    nodes: list[Node]

    def __repr__(self) -> str:  # pragma: no cover
        return f"Block(len={len(self.nodes)})"


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
    return bool(v)


class VM:
    def __init__(self, file: str | None = None, out=None):
        self.stack: list[Any] = []
        self.env: dict[str, Any] = {}
        # Granted capabilities. Empty by default: a program has zero authority
        # until a `gr` block brings a capability into scope (SPEC Part II §12).
        self.caps: set[str] = set()
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

    def pop_num(self, op: str, pos: Pos | None = None) -> float | int:
        v = self.pop(op, pos)
        if isinstance(v, bool) or not isinstance(v, (int, float)):
            raise self.err(TYPE_MISMATCH, f"`{op}` expected a number, got {type_name(v)}",
                           op=op, pos=pos, hint="this operator only works on numbers")
        return v

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

    def invoke(self, blk: Block) -> None:
        self.run(blk.nodes)

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
    if isinstance(v, int):
        return "int"
    if isinstance(v, float):
        return "float"
    if isinstance(v, str):
        return "str"
    return type(v).__name__


# --- operator implementations --------------------------------------------------
def _num_result(v: float) -> Any:
    """Collapse whole floats produced by int math back to int."""
    if isinstance(v, float) and v.is_integer():
        return int(v)
    return v


def op_add(vm: VM, pos: Pos) -> None:
    b = vm.pop("+", pos)
    a = vm.pop("+", pos)
    if isinstance(a, str) or isinstance(b, str):
        vm.push(_as_str(a) + _as_str(b))  # `+` concatenates when either side is text
        return
    _require_nums(vm, "+", pos, a, b)
    vm.push(a + b)


def op_sub(vm: VM, pos: Pos) -> None:
    b = vm.pop_num("-", pos)
    a = vm.pop_num("-", pos)
    vm.push(a - b)


def op_mul(vm: VM, pos: Pos) -> None:
    b = vm.pop("*", pos)
    a = vm.pop("*", pos)
    if isinstance(a, str) and isinstance(b, int):  # str repeat: $ab$ 3 *
        vm.push(a * b)
        return
    _require_nums(vm, "*", pos, a, b)
    vm.push(a * b)


def op_div(vm: VM, pos: Pos) -> None:
    b = vm.pop_num("/", pos)
    a = vm.pop_num("/", pos)
    if b == 0:
        raise vm.err(DIV_ZERO, "division by zero", op="/", pos=pos,
                     hint="guard the denominator with `=0 ==` before dividing")
    vm.push(_num_result(a / b))


def op_mod(vm: VM, pos: Pos) -> None:
    b = vm.pop_num("%", pos)
    a = vm.pop_num("%", pos)
    if b == 0:
        raise vm.err(DIV_ZERO, "modulo by zero", op="%", pos=pos)
    vm.push(a % b)


def _cmp(vm: VM, op: str, pos: Pos, fn) -> None:
    b = vm.pop(op, pos)
    a = vm.pop(op, pos)
    try:
        vm.push(1 if fn(a, b) else 0)
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
    vm.push(-a)


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
    vm.invoke(blk)


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
        vm.invoke(cond)
        if not truthy(vm.pop(";", pos)):
            break
        vm.invoke(body)


_OPS: dict[str, Callable[[VM, Pos], None]] = {
    "+": op_add, "-": op_sub, "*": op_mul, "/": op_div, "%": op_mod,
    "==": op_eq, "!=": op_ne, "<": op_lt, ">": op_gt, "<=": op_le, ">=": op_ge,
    "&&": op_and, "||": op_or, "!": op_not, "_": op_neg,
    "=": op_dup, ",": op_drop, "\\": op_swap, "&": op_over,
    ".": op_print, "@": op_invoke, "?": op_select, ";": op_while,
}


def _as_str(v: Any) -> str:
    if isinstance(v, bool):
        return "1" if v else "0"
    if isinstance(v, float):
        return str(int(v)) if v.is_integer() else str(v)
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
        if isinstance(v, bool) or not isinstance(v, (int, float)):
            raise vm.err(TYPE_MISMATCH, f"`{op}` expected numbers, got {type_name(v)}",
                         op=op, pos=pos)


def run_source(src: str, file: str | None = None, out=None) -> VM:
    vm = VM(file=file, out=out)
    vm.run_src(src)
    return vm
