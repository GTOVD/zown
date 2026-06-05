"""Zown standard library: token-dense builtin words.

Builtins are auto-executing words (1-2 chars). They are looked up only when a
bare identifier is *not* a user binding, so any of these can be shadowed by the
program. Each entry below is the canonical, minimal name for a capability that
in legacy languages would be a verbose method call like `.length` or `.trim()`.

The companion alias/description for every word lives in WORDS, which the manifest
generator uses so an AI or human can trace `tr` -> `trim` with a one-line intent.
"""

from __future__ import annotations

import math
from typing import Any, Callable

from .errors import Pos, TYPE_MISMATCH, ZownError


def _need_str(vm, op: str, pos: Pos) -> str:
    v = vm.pop(op, pos)
    if not isinstance(v, str):
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a string", op=op, pos=pos)
    return v


def _need_num(vm, op: str, pos: Pos):
    return vm.pop_num(op, pos)


# --- string words --------------------------------------------------------------
def b_len(vm, pos: Pos) -> None:
    from .vm import Block
    v = vm.pop("ln", pos)
    if isinstance(v, str):
        vm.push(len(v))
    elif isinstance(v, Block):
        vm.push(len(v.nodes))
    else:
        raise vm.err(TYPE_MISMATCH, "`ln` expected a string or block", op="ln", pos=pos)


def b_trim(vm, pos: Pos) -> None:
    vm.push(_need_str(vm, "tr", pos).strip())


def b_upper(vm, pos: Pos) -> None:
    vm.push(_need_str(vm, "up", pos).upper())


def b_lower(vm, pos: Pos) -> None:
    vm.push(_need_str(vm, "lo", pos).lower())


def b_rev(vm, pos: Pos) -> None:
    vm.push(_need_str(vm, "rv", pos)[::-1])


def b_print_raw(vm, pos: Pos) -> None:
    from .vm import _as_str
    vm.out.write(_as_str(vm.pop("pr", pos)))


# --- math words ----------------------------------------------------------------
def b_abs(vm, pos: Pos) -> None:
    vm.push(abs(_need_num(vm, "ab", pos)))


def b_max(vm, pos: Pos) -> None:
    b = _need_num(vm, "mx", pos)
    a = _need_num(vm, "mx", pos)
    vm.push(max(a, b))


def b_min(vm, pos: Pos) -> None:
    b = _need_num(vm, "mn", pos)
    a = _need_num(vm, "mn", pos)
    vm.push(min(a, b))


def b_sqrt(vm, pos: Pos) -> None:
    from .vm import _num_result
    vm.push(_num_result(math.sqrt(_need_num(vm, "sq", pos))))


def b_pow(vm, pos: Pos) -> None:
    from .vm import _num_result
    e = _need_num(vm, "pw", pos)
    a = _need_num(vm, "pw", pos)
    vm.push(_num_result(a ** e))


def b_floor(vm, pos: Pos) -> None:
    vm.push(math.floor(_need_num(vm, "fl", pos)))


def b_ceil(vm, pos: Pos) -> None:
    vm.push(math.ceil(_need_num(vm, "ce", pos)))


def b_round(vm, pos: Pos) -> None:
    vm.push(round(_need_num(vm, "rd", pos)))


# --- conversion words ----------------------------------------------------------
def b_to_str(vm, pos: Pos) -> None:
    from .vm import _as_str
    vm.push(_as_str(vm.pop("s", pos)))


def b_to_num(vm, pos: Pos) -> None:
    v = vm.pop("n", pos)
    if isinstance(v, (int, float)):
        vm.push(v)
        return
    if isinstance(v, str):
        s = v.strip()
        try:
            vm.push(int(s) if ("." not in s and "e" not in s.lower()) else float(s))
            return
        except ValueError:
            raise vm.err(TYPE_MISMATCH, f"`n` cannot parse {v!r} as a number", op="n", pos=pos)
    raise vm.err(TYPE_MISMATCH, "`n` expected a string or number", op="n", pos=pos)


# --- stack words ---------------------------------------------------------------
def b_depth(vm, pos: Pos) -> None:
    vm.push(len(vm.stack))


def b_rot(vm, pos: Pos) -> None:
    if len(vm.stack) < 3:
        from .errors import STACK_UNDERFLOW
        raise vm.err(STACK_UNDERFLOW, "`rt` (rot) needs three values", op="rt", pos=pos)
    a = vm.stack.pop(-3)
    vm.stack.append(a)


def b_clear(vm, pos: Pos) -> None:
    vm.stack.clear()


# word -> (handler, alias, description)
WORDS: dict[str, tuple[Callable[[Any, Pos], None], str, str]] = {
    "ln": (b_len, "length", "length of a string (chars) or block (nodes)"),
    "tr": (b_trim, "trim", "strip leading/trailing whitespace from a string"),
    "up": (b_upper, "upper", "uppercase a string"),
    "lo": (b_lower, "lower", "lowercase a string"),
    "rv": (b_rev, "reverse", "reverse a string"),
    "pr": (b_print_raw, "print_raw", "print top value with no trailing newline"),
    "ab": (b_abs, "abs", "absolute value of a number"),
    "mx": (b_max, "max", "max of the top two numbers"),
    "mn": (b_min, "min", "min of the top two numbers"),
    "sq": (b_sqrt, "sqrt", "square root of a number"),
    "pw": (b_pow, "pow", "base exponent -> base**exponent"),
    "fl": (b_floor, "floor", "floor of a number"),
    "ce": (b_ceil, "ceil", "ceiling of a number"),
    "rd": (b_round, "round", "round a number to nearest int"),
    "s": (b_to_str, "to_str", "convert top value to its string form"),
    "n": (b_to_num, "to_num", "parse a string into a number"),
    "dp": (b_depth, "depth", "push the current stack depth"),
    "rt": (b_rot, "rot", "rotate the top three values (a b c -> b c a)"),
    "clr": (b_clear, "clear", "clear the entire stack"),
}

BUILTINS: dict[str, Callable[[Any, Pos], None]] = {k: v[0] for k, v in WORDS.items()}
