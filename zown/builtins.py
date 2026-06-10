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


# --- capability words (v0.2; SPEC Part II §12) ---------------------------------
# Zero authority by default: a program holds no capabilities until a `gr` block
# brings one into scope. `rq` asserts a capability is held (else CAP_DENIED); `hv`
# is the non-fatal test. Privileged stdlib ops (net, disk, ...) will gate on `rq`.
def b_grant(vm, pos: Pos) -> None:
    # `cap [body] gr  -> run body with `cap granted, then restore prior authority.
    blk = vm.pop_block("gr", pos)
    cap = vm.pop_cap("gr", pos)
    had = cap.name in vm.caps
    vm.caps.add(cap.name)
    try:
        vm.invoke(blk)
    finally:
        if not had:
            vm.caps.discard(cap.name)


def b_require(vm, pos: Pos) -> None:
    # `cap rq  -> assert the capability is granted; structured CAP_DENIED if not.
    from .errors import CAP_DENIED
    cap = vm.pop_cap("rq", pos)
    if cap.name not in vm.caps:
        raise vm.err(
            CAP_DENIED,
            f"operation requires capability `{cap.name}; not granted",
            op="rq", pos=pos, kind="sec", cap=cap.name,
            hint=f"grant it: ``{cap.name} [ ... ] gr`, or run inside a holder",
        )


def b_have(vm, pos: Pos) -> None:
    # `cap hv  -> push 1 if the capability is granted, else 0 (non-fatal probe).
    cap = vm.pop_cap("hv", pos)
    vm.push(1 if cap.name in vm.caps else 0)


# --- fixed-width integers (v0.2; SPEC Part II §11) -----------------------------
# The default Zown int is arbitrary precision. A width tag (i8..u128) plus a
# policy word makes overflow explicit: `wr` wraps (two's complement), `st`
# saturates to the width's range, `ck` traps with OVERFLOW if the value won't fit.
_WIDTHS = [
    ("i8", True, 8), ("i16", True, 16), ("i32", True, 32), ("i64", True, 64),
    ("i128", True, 128),
    ("u8", False, 8), ("u16", False, 16), ("u32", False, 32), ("u64", False, 64),
    ("u128", False, 128),
]


def _make_width(signed: bool, bits: int):
    def handler(vm, pos: Pos) -> None:
        from .vm import WidthTag
        vm.push(WidthTag(signed, bits))
    return handler


def _pop_width(vm, op: str, pos: Pos):
    from .vm import WidthTag
    w = vm.pop(op, pos)
    if not isinstance(w, WidthTag):
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a width tag (i8..u128) on top",
                     op=op, pos=pos, hint="push a width like u8 before the policy word")
    return w


def _pop_intish(vm, op: str, pos: Pos) -> int:
    v = vm.pop(op, pos)
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected an integer, got {type_name(v)}",
                     op=op, pos=pos)
    if isinstance(v, float):
        if not v.is_integer():
            raise vm.err(TYPE_MISMATCH, f"`{op}` expected an integer, got a fractional float",
                         op=op, pos=pos, hint="round/floor first, or use a float op")
        v = int(v)
    return v


def _wrap_int(n: int, signed: bool, bits: int) -> int:
    """Reduce n into a `bits`-wide integer, two's complement if signed."""
    mod = 1 << bits
    m = n % mod
    if signed and m >= (1 << (bits - 1)):
        m -= mod
    return m


def b_wrap(vm, pos: Pos) -> None:
    # n width wr  -> n reduced into width modulo 2**bits (two's complement signed).
    w = _pop_width(vm, "wr", pos)
    n = _pop_intish(vm, "wr", pos)
    vm.push(_wrap_int(n, w.signed, w.bits))


def b_sat(vm, pos: Pos) -> None:
    # n width st  -> n clamped to the width's [lo..hi].
    w = _pop_width(vm, "st", pos)
    n = _pop_intish(vm, "st", pos)
    vm.push(w.lo if n < w.lo else w.hi if n > w.hi else n)


def b_chk(vm, pos: Pos) -> None:
    # n width ck  -> n unchanged if it fits the width, else a structured OVERFLOW.
    from .errors import OVERFLOW
    w = _pop_width(vm, "ck", pos)
    n = _pop_intish(vm, "ck", pos)
    if n < w.lo or n > w.hi:
        raise vm.err(OVERFLOW, f"{n} does not fit {w.name} [{w.lo}..{w.hi}]",
                     op="ck", pos=pos,
                     hint="use `wr` to wrap, `st` to saturate, or widen the type")
    vm.push(n)


# --- SIMD vectors (v0.2; SPEC Part II §11) -------------------------------------
# A constructor word evaluates a block to produce its lanes: `[1 2 3 4] f4`. Lane
# count and type are fixed by the word. Integer lanes wrap to their width (the
# same no-silent-overflow rule). Elementwise ops (vadd/vsub/vmul) require matching
# vector types. (Ops are word-form, not `v+`: the lexer would split `v` and `+`.)
_VECS = [
    # name, count, is_float, signed, bits
    ("f4", 4, True, True, 32),
    ("d2", 2, True, True, 64),
    ("i4", 4, False, True, 32),
    ("b16", 16, False, False, 8),
]


def _coerce_lane(vm, name: str, pos: Pos, x, is_float: bool, signed: bool, bits: int):
    if isinstance(x, bool) or not isinstance(x, (int, float)):
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`{name}` lanes must be numbers, got {type_name(x)}",
                     op=name, pos=pos)
    if is_float:
        return float(x)
    if isinstance(x, float):
        if not x.is_integer():
            raise vm.err(TYPE_MISMATCH, f"`{name}` integer lanes need whole numbers",
                         op=name, pos=pos)
        x = int(x)
    return _wrap_int(x, signed, bits)


def _make_vec(name: str, count: int, is_float: bool, signed: bool, bits: int):
    def handler(vm, pos: Pos) -> None:
        from .vm import Vec
        from .errors import BOUNDS
        blk = vm.pop_block(name, pos)
        saved = vm.stack
        vm.stack = []
        try:
            vm.invoke(blk)
            lanes = vm.stack
        finally:
            vm.stack = saved
        if len(lanes) != count:
            raise vm.err(BOUNDS, f"`{name}` needs {count} lanes, got {len(lanes)}",
                         op=name, pos=pos, hint=f"the block must leave exactly {count} numbers")
        lanes = [_coerce_lane(vm, name, pos, x, is_float, signed, bits) for x in lanes]
        vm.push(Vec(name, count, is_float, signed, bits, lanes))
    return handler


def _pop_vec(vm, op: str, pos: Pos):
    from .vm import Vec
    v = vm.pop(op, pos)
    if not isinstance(v, Vec):
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a vector, got {type_name(v)}",
                     op=op, pos=pos)
    return v


def _make_vbinop(op: str, fn):
    def handler(vm, pos: Pos) -> None:
        from .vm import Vec
        b = _pop_vec(vm, op, pos)
        a = _pop_vec(vm, op, pos)
        if a.name != b.name:
            raise vm.err(TYPE_MISMATCH,
                         f"`{op}` needs matching vector types, got {a.name} and {b.name}",
                         op=op, pos=pos)
        out = []
        for x, y in zip(a.lanes, b.lanes):
            r = fn(x, y)
            if not a.is_float:
                r = _wrap_int(int(r), a.signed, a.bits)
            out.append(r)
        vm.push(Vec(a.name, a.count, a.is_float, a.signed, a.bits, out))
    return handler


def b_vsum(vm, pos: Pos) -> None:
    # vec vsum  -> horizontal sum to a scalar (exact; lane width is not re-applied).
    v = _pop_vec(vm, "vsum", pos)
    total = sum(v.lanes)
    from .vm import _num_result
    vm.push(_num_result(total) if v.is_float else total)


def b_vat(vm, pos: Pos) -> None:
    # vec idx vat  -> the lane at idx (BOUNDS if out of range).
    from .errors import BOUNDS
    idx = _pop_intish(vm, "vat", pos)
    v = _pop_vec(vm, "vat", pos)
    if idx < 0 or idx >= len(v.lanes):
        raise vm.err(BOUNDS, f"lane {idx} is outside {v.name} [0..{len(v.lanes) - 1}]",
                     op="vat", pos=pos)
    vm.push(v.lanes[idx])


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
    "gr": (b_grant, "grant", "`cap [body] gr -> run body with `cap granted, then restore"),
    "rq": (b_require, "require", "`cap rq -> assert capability is granted (CAP_DENIED if not)"),
    "hv": (b_have, "have", "`cap hv -> push 1 if capability is granted, else 0"),
    "wr": (b_wrap, "wrap", "n width wr -> n wrapped into width (two's complement)"),
    "st": (b_sat, "saturate", "n width st -> n clamped to width's [min..max]"),
    "ck": (b_chk, "check", "n width ck -> n if it fits width, else OVERFLOW"),
}

WORDS["vadd"] = (_make_vbinop("vadd", lambda x, y: x + y), "vec_add", "elementwise add two matching vectors")
WORDS["vsub"] = (_make_vbinop("vsub", lambda x, y: x - y), "vec_sub", "elementwise subtract two matching vectors")
WORDS["vmul"] = (_make_vbinop("vmul", lambda x, y: x * y), "vec_mul", "elementwise multiply two matching vectors")
WORDS["vsum"] = (b_vsum, "vec_sum", "horizontal sum of a vector's lanes to a scalar")
WORDS["vat"] = (b_vat, "vec_at", "vec idx vat -> the lane at idx (BOUNDS if out of range)")

# fixed-width integer type tags (i8..u128): each pushes a WidthTag value.
for _name, _signed, _bits in _WIDTHS:
    _kind = "signed" if _signed else "unsigned"
    _alias = ("int" if _signed else "uint") + str(_bits)
    WORDS[_name] = (
        _make_width(_signed, _bits),
        _alias,
        f"width tag: {_kind} {_bits}-bit integer",
    )

# SIMD vector constructor words: `[ ...lanes ] f4` etc.
for _vname, _count, _isf, _vsigned, _vbits in _VECS:
    _lane = ("f" if _isf else ("i" if _vsigned else "u")) + str(_vbits)
    WORDS[_vname] = (
        _make_vec(_vname, _count, _isf, _vsigned, _vbits),
        f"{_lane}x{_count}",
        f"build a {_lane} x{_count} SIMD vector from a block of {_count} lanes",
    )

BUILTINS: dict[str, Callable[[Any, Pos], None]] = {k: v[0] for k, v in WORDS.items()}
