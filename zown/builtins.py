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

from .lexer import INT_MAX, INT_MIN
from .errors import (
    BOUNDS, DIV_ZERO, OVERFLOW, Pos, TEST_FAIL, TYPE_MISMATCH, UNSUPPORTED, ZownError,
)
from .blake3 import blake3_hex
from .lanes import b_hw, b_lc, b_pm, b_pn, b_rn, b_sg, b_ws
from .mesh import ula
from .buf import b_bf, b_bg, b_bp, release
from .store import (
    b_ge, b_gh, b_gl, b_gs, b_gv, b_la, b_lp, b_lr, b_mg, b_sb, b_sl, b_so,
    b_tc, b_tq, b_tu,
)
from .crypt import b_ad, b_ae, b_es, b_ev, b_xk
from .hostio import b_ac, b_ag, b_en, b_fr, b_fw, b_gd, b_ml, b_mm, b_mo, b_tm
from .tele import b_ch, b_df, b_lg, b_nt, b_ob, b_rm, b_se, b_sp, b_td, b_vr, b_wn
from .ui import b_an, b_bh, b_bt, b_cm, b_co, b_gp, b_ky, b_ly, b_pk, b_rw, b_tx
from .coll import (
    b_bm, b_bv, b_gt, b_hs, b_ls, b_mp, b_po, b_pq, b_ps, b_pt, b_px, b_qu,
    b_rg, b_sc, b_sk, b_te, b_ti,
)


def _need_str(vm, op: str, pos: Pos) -> str:
    v = vm.pop(op, pos)
    if not isinstance(v, str):
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a string", op=op, pos=pos)
    return v


def _need_num(vm, op: str, pos: Pos):
    return vm.pop_num(op, pos)


# --- string words --------------------------------------------------------------
def b_head(vm, pos: Pos) -> None:
    # str hd -> the first character. Empty is BOUNDS. One code point, not a byte.
    from .errors import BOUNDS
    text = _need_str(vm, "hd", pos)
    if text == "":
        raise vm.err(BOUNDS, "`hd` needs a character but the string is empty",
                     op="hd", pos=pos, hint="check the length with ln first")
    vm.push(text[0])


def b_tail(vm, pos: Pos) -> None:
    # str tl -> every character after the first. Empty stays empty.
    text = _need_str(vm, "tl", pos)
    vm.push(text[1:])


def b_len(vm, pos: Pos) -> None:
    from .buf import Buf
    from .vm import Block
    v = vm.pop("ln", pos)
    if isinstance(v, str):
        vm.push(len(v))
    elif isinstance(v, Block):
        vm.push(len(v.nodes))
    elif isinstance(v, Buf):
        data = v.alloc.data
        if data is None:
            raise vm.err(BOUNDS, "`ln` buffer was freed", op="ln", pos=pos,
                         hint="the exclusive owner was dropped with `,`")
        n = len(data)
        release(v)
        vm.push(n)
    else:
        from .coll import Coll
        if isinstance(v, Coll):
            if v.store.freed:
                raise vm.err(BOUNDS, "`ln` collection was freed", op="ln", pos=pos,
                             hint="the exclusive owner was dropped with `,`")
            n = v.store.count()
            release(v)
            vm.push(n)
        else:
            raise vm.err(TYPE_MISMATCH, "`ln` expected a string, block, buf, or collection",
                         op="ln", pos=pos)


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
    v = vm.pop("pr", pos)
    vm.out.write(_as_str(v))
    release(v)


# --- math words ----------------------------------------------------------------
# Numeric model (SPEC §11): int is a checked 64-bit integer (OVERFLOW, never a
# silent wrap), big is exact and unbounded, float is IEEE-754 binary64 (inf/nan
# are values; converting them to an integer traps). Every helper below states
# which of the three it produces.
def b_abs(vm, pos: Pos) -> None:
    from .vm import Big
    v = _need_num(vm, "ab", pos)
    if isinstance(v, Big):
        vm.push(Big(abs(v.v)))
    elif isinstance(v, float):
        vm.push(abs(v))
    else:
        vm.push(vm.int_result("ab", pos, abs(v)))  # abs(INT_MIN) does not fit


def _std_word(vm, name: str, pos: Pos) -> None:
    """Run the block `std/core.zn` registered. The host no longer has a second copy."""
    blk = vm.words.get(name)
    if blk is None:
        raise vm.err(UNSUPPORTED, f"`{name}` is a std word and std/core.zn did not define it",
                     op=name, pos=pos)
    vm.run_word(name, blk, pos)


def b_max(vm, pos: Pos) -> None:
    _std_word(vm, "mx", pos)


def b_min(vm, pos: Pos) -> None:
    _std_word(vm, "mn", pos)


def _no_real(vm, op: str, pos: Pos, what: str):
    from .errors import UB_TRAP
    return vm.err(UB_TRAP, f"`{op}`: {what} has no real result", op=op, pos=pos,
                  hint="guard the operand's sign first; Zown traps instead of producing nan")


def b_sqrt(vm, pos: Pos) -> None:
    from .vm import Big, _num_result
    v = _need_num(vm, "sq", pos)
    if isinstance(v, Big):
        if v.v < 0:
            raise _no_real(vm, "sq", pos, "the square root of a negative number")
        r = math.isqrt(v.v)
        if r * r == v.v:
            vm.push(Big(r))  # exact -> stays big
            return
        vm.push(math.sqrt(vm.to_float("sq", pos, v)))  # inexact -> float
        return
    if v < 0:  # int or float (nan compares false and falls through to math.sqrt -> nan)
        raise _no_real(vm, "sq", pos, "the square root of a negative number")
    vm.push(_num_result(math.sqrt(v)))


def b_pow(vm, pos: Pos) -> None:
    from .vm import Big, _num_result, ival
    e = _need_num(vm, "pw", pos)
    a = _need_num(vm, "pw", pos)
    if isinstance(a, float) or isinstance(e, float):
        fa, fe = vm.to_float("pw", pos, a), vm.to_float("pw", pos, e)
        if fa < 0 and not fe.is_integer():
            raise _no_real(vm, "pw", pos, "a negative base with a fractional exponent")
        try:
            r = fa ** fe
        except OverflowError:  # IEEE: overflow is ±inf (sign: negative base, odd exponent)
            r = -math.inf if (fa < 0 and fe % 2 == 1) else math.inf
        except ZeroDivisionError:
            raise vm.err(DIV_ZERO, "`pw`: zero to a negative power", op="pw", pos=pos)
        vm.push(_num_result(r))
        return
    av, ev = ival(a), ival(e)
    if ev < 0:  # integer base, negative exponent -> float (0 ** -n is a division by zero)
        if av == 0:
            raise vm.err(DIV_ZERO, "`pw`: zero to a negative power", op="pw", pos=pos)
        try:
            vm.push(_num_result(vm.to_float("pw", pos, a) ** ev))
        except OverflowError:
            vm.push(0.0)  # 1/huge underflows to 0.0
        return
    if isinstance(a, Big) or isinstance(e, Big):
        vm.push(Big(av ** ev))  # exact and unbounded
        return
    # int ** int must fit the checked int; refuse early rather than compute a giant.
    if abs(av) >= 2 and ev > 63:
        raise vm.err(OVERFLOW, f"`pw` result {av}**{ev} does not fit int", op="pw", pos=pos,
                     hint="convert the base with `b` for arbitrary-precision big math")
    vm.push(vm.int_result("pw", pos, av ** ev))


def _to_int_word(op: str, fn):
    """fl / ce / rd: ints and bigs are already integral; a float converts through
    the trap-on-nan / OVERFLOW-on-inf rule (SPEC §19)."""
    def handler(vm, pos: Pos) -> None:
        v = _need_num(vm, op, pos)
        if isinstance(v, float):
            if not math.isfinite(v):
                vm.int_from_float(op, pos, v)  # raises UB_TRAP (nan) / OVERFLOW (inf)
            vm.push(vm.int_result(op, pos, fn(v)))  # a finite float rounds to a Python int
            return
        vm.push(v)
    return handler


b_floor = _to_int_word("fl", math.floor)
b_ceil = _to_int_word("ce", math.ceil)
b_round = _to_int_word("rd", round)  # banker's rounding, as the oracle always did


# --- conversion words ----------------------------------------------------------
def b_to_str(vm, pos: Pos) -> None:
    from .vm import _as_str
    v = vm.pop("s", pos)
    vm.push(_as_str(v))
    release(v)


def b_cat(vm, pos: Pos) -> None:
    # a b ct  -> concatenate two strings. `+` is numeric add and does not do this.
    from .vm import type_name
    b = vm.pop("ct", pos)
    a = vm.pop("ct", pos)
    if not isinstance(a, str) or not isinstance(b, str):
        raise vm.err(TYPE_MISMATCH,
                     f"`ct` expected two strings, got {type_name(a)} and {type_name(b)}",
                     op="ct", pos=pos, hint="convert with `s` first")
    vm.push(a + b)


def b_repeat(vm, pos: Pos) -> None:
    # str count rp  -> the string repeated `count` times. A count <= 0 yields "".
    # `*` is numeric multiply and does not do this.
    from .vm import type_name
    n = vm.pop("rp", pos)
    s = vm.pop("rp", pos)
    if not isinstance(s, str) or isinstance(n, bool) or not isinstance(n, int):
        raise vm.err(TYPE_MISMATCH,
                     f"`rp` expected a string and an int, got {type_name(s)} and {type_name(n)}",
                     op="rp", pos=pos, hint="the count is a plain int; negate with `_`")
    vm.push(s * n if n > 0 else "")


def _parse_int_text(vm, op: str, pos: Pos, s: str):
    """Parse decimal digits; returns a Python int or raises TYPE_MISMATCH."""
    try:
        return int(s)
    except ValueError:
        raise vm.err(TYPE_MISMATCH, f"`{op}` cannot parse {s!r} as a number", op=op, pos=pos)


def b_to_num(vm, pos: Pos) -> None:
    from .vm import is_num
    v = vm.pop("n", pos)
    if is_num(v):
        vm.push(v)
        return
    if isinstance(v, str):
        s = v.strip()
        if "." not in s and "e" not in s.lower() and s.lower() not in ("inf", "-inf", "nan"):
            n = _parse_int_text(vm, "n", pos, s)
            if not (-(1 << 63) <= n < (1 << 63)):
                raise vm.err(OVERFLOW, f"`n`: {s!r} does not fit int", op="n", pos=pos,
                             hint="use `b` to parse it as an arbitrary-precision big")
            vm.push(n)
            return
        try:
            vm.push(float(s))  # IEEE spellings `inf`/`nan` parse; huge text -> inf
            return
        except ValueError:
            raise vm.err(TYPE_MISMATCH, f"`n` cannot parse {v!r} as a number", op="n", pos=pos)
    raise vm.err(TYPE_MISMATCH, "`n` expected a string or number", op="n", pos=pos)


def b_big(vm, pos: Pos) -> None:
    # v b  -> the arbitrary-precision big of an int, a whole float, a big (no-op),
    #         or a string of digits (the way to write a literal beyond the int range).
    from .vm import Big, type_name
    v = vm.pop("b", pos)
    if isinstance(v, Big):
        vm.push(v)
    elif isinstance(v, bool):
        raise vm.err(TYPE_MISMATCH, "`b` expected an int, whole float, or digit string",
                     op="b", pos=pos)
    elif isinstance(v, int):
        vm.push(Big(v))
    elif isinstance(v, float):
        if not v.is_integer():  # also rejects inf/nan (is_integer() is False for them)
            raise vm.err(TYPE_MISMATCH, "`b` needs a whole number, got a fractional float",
                         op="b", pos=pos, hint="round/floor first")
        vm.push(Big(int(v)))
    elif isinstance(v, str):
        vm.push(Big(_parse_int_text(vm, "b", pos, v.strip())))
    else:
        raise vm.err(TYPE_MISMATCH, f"`b` expected a number or digit string, got {type_name(v)}",
                     op="b", pos=pos)


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
    _std_word(vm, "cl", pos)


def _word_name(s: str) -> bool:
    return s.isascii() and len(s) >= 1 and s[0].isalpha() and s.isalnum()


def b_wd(vm, pos: Pos) -> None:
    # [body] $name$ wd  -> register an auto-running word.
    name = vm.pop("wd", pos)
    if not isinstance(name, str) or not _word_name(name):
        raise vm.err(TYPE_MISMATCH, "`wd` expected a word name (a letter, then letters or digits)",
                     op="wd", pos=pos, hint="write the name as a string: [ ... ] $mx$ wd")
    blk = vm.pop_block("wd", pos)
    vm.words[name] = blk


def _finish_test(vm, op: str, pos: Pos, depth: int) -> None:
    from .vm import truthy
    left = len(vm.stack) - depth
    if left != 1:
        raise vm.err(BOUNDS, f"`{op}` body must leave exactly one value, left {left}",
                     op=op, pos=pos, hint="the body pushes 1 to pass")
    if not truthy(vm.pop(op, pos)):
        raise vm.err(TEST_FAIL, f"`{op}` body left a false value", op=op, pos=pos,
                     hint="a passing test leaves 1")


def b_ts(vm, pos: Pos) -> None:
    blk = vm.pop_block("ts", pos)
    depth = len(vm.stack)
    vm.invoke(blk, "ts", pos)
    _finish_test(vm, "ts", pos, depth)


def b_pp(vm, pos: Pos) -> None:
    blk = vm.pop_block("pp", pos)
    k = vm.pop("pp", pos)
    if type(k) is not int:
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`pp` expected an int, got {type_name(k)}",
                     op="pp", pos=pos, hint="the count is a plain int")
    if k < 0:
        raise vm.err(BOUNDS, "`pp` repeat count must be >= 0", op="pp", pos=pos)
    for _ in range(k):
        depth = len(vm.stack)
        vm.invoke(blk, "pp", pos)
        _finish_test(vm, "pp", pos, depth)


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
        vm.invoke(blk, "gr", pos)
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


# --- result values (v0.2; SPEC Part II §16.1) ----------------------------------
def b_attempt(vm, pos: Pos) -> None:
    # [ body ] ty  -> ok(value) if body ran and left exactly one value on an
    #                 isolated stack; err(code) if it raised any .zerr.
    # The body runs on an isolated stack (like the SIMD constructors) so `ty` has
    # the fixed stack effect ( blk -- res ) on both paths; pass inputs through
    # bindings (5 :x [ x 2 / ] ty). Output, bindings, and grants made by the body
    # are not undone — only the stack is isolated. `at` is lane index, not this.
    from .errors import BOUNDS
    from .vm import Res
    blk = vm.pop_block("ty", pos)
    saved = vm.stack
    vm.stack = []
    try:
        try:
            vm.invoke(blk, "ty", pos)
        except ZownError as e:
            vm.stack = saved
            vm.push(Res(False, e.code))
            return
        left = vm.stack
    finally:
        vm.stack = saved
    if len(left) != 1:
        raise vm.err(BOUNDS, f"`ty` body must leave exactly one value, left {len(left)}",
                     op="ty", pos=pos,
                     hint="bind inputs first (5 :x) and leave the single result on the stack")
    vm.push(Res(True, left[0]))


# --- fixed-width integers (v0.2; SPEC Part II §11) -----------------------------
# The default Zown int is a checked 64-bit. A width tag plus a policy word makes
# overflow explicit: `wr` wraps (two's complement), `st` saturates to the width's
# range, `ck` traps with OVERFLOW if the value won't fit. The suffix is the bit
# width. 128 does not fit in two digits, so those two words are `ix` and `ux`.
_WIDTHS = [
    ("i8", True, 8), ("i16", True, 16), ("i32", True, 32), ("i64", True, 64),
    ("ix", True, 128),
    ("u8", False, 8), ("u16", False, 16), ("u32", False, 32), ("u64", False, 64),
    ("ux", False, 128),
]


def _make_width(signed: bool, bits: int):
    def handler(vm, pos: Pos) -> None:
        from .vm import WidthTag
        vm.push(WidthTag(signed, bits))
    return handler


def b_iz(vm, pos: Pos) -> None:
    from .vm import WidthTag
    vm.push(WidthTag(True, 64, True))


def b_uz(vm, pos: Pos) -> None:
    from .vm import WidthTag
    vm.push(WidthTag(False, 64, True))


def b_dc(vm, pos: Pos) -> None:
    from .exact import Dec, parse_dec
    from .vm import Big, type_name
    v = vm.pop("dc", pos)
    if isinstance(v, str):
        vm.push(parse_dec(vm, "dc", pos, v))
        return
    if isinstance(v, bool):
        raise vm.err(TYPE_MISMATCH, "`dc` expected an int, a big, or a decimal string, got bool",
                     op="dc", pos=pos)
    if isinstance(v, int):
        vm.push(Dec(v, 0))
        return
    if isinstance(v, Big):
        if v.v < INT_MIN or v.v > INT_MAX:
            raise vm.err(OVERFLOW, f"`dc` coefficient {v.v} does not fit int", op="dc", pos=pos)
        vm.push(Dec(int(v.v), 0))
        return
    raise vm.err(TYPE_MISMATCH, f"`dc` expected an int, a big, or a decimal string, got {type_name(v)}",
                 op="dc", pos=pos, hint="a float is not a decimal")


def b_cx(vm, pos: Pos) -> None:
    from .exact import Cx
    from .vm import type_name
    im = vm.pop("cx", pos)
    re = vm.pop("cx", pos)
    for n in (re, im):
        if isinstance(n, bool) or not isinstance(n, int):
            raise vm.err(TYPE_MISMATCH, f"`cx` expected an int, got {type_name(n)}",
                         op="cx", pos=pos, hint="cx is a Gaussian integer, not a float")
    vm.push(Cx(re, im))


def _pop_width(vm, op: str, pos: Pos):
    from .vm import WidthTag
    w = vm.pop(op, pos)
    if not isinstance(w, WidthTag):
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a width tag (i8..ix, u8..ux) on top",
                     op=op, pos=pos, hint="push a width like u8 before the policy word")
    return w


def _pop_intish(vm, op: str, pos: Pos) -> int:
    """Pop an integral value (int, big, or whole float) as a Python int."""
    from .vm import Big, is_num, type_name
    v = vm.pop(op, pos)
    if not is_num(v):
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected an integer, got {type_name(v)}",
                     op=op, pos=pos)
    if isinstance(v, Big):
        return v.v
    if isinstance(v, float):
        if not v.is_integer():  # fractional, inf, and nan all fail here
            raise vm.err(TYPE_MISMATCH, f"`{op}` expected an integer, got a fractional float",
                         op=op, pos=pos, hint="round/floor first, or use a float op")
        v = int(v)
    return v


def _typed(w, n: int):
    """The result of a policy word has the width's static type: `int` when the
    width's whole range fits the checked int, else `big` (SPEC §11)."""
    from .vm import Big
    return n if w.fits_int else Big(n)


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
    vm.push(_typed(w, _wrap_int(n, w.signed, w.bits)))


def b_sat(vm, pos: Pos) -> None:
    # n width st  -> n clamped to the width's [lo..hi].
    w = _pop_width(vm, "st", pos)
    n = _pop_intish(vm, "st", pos)
    vm.push(_typed(w, w.lo if n < w.lo else w.hi if n > w.hi else n))


def b_chk(vm, pos: Pos) -> None:
    # n width ck  -> n unchanged if it fits the width, else a structured OVERFLOW.
    w = _pop_width(vm, "ck", pos)
    n = _pop_intish(vm, "ck", pos)
    if n < w.lo or n > w.hi:
        raise vm.err(OVERFLOW, f"{n} does not fit {w.name} [{w.lo}..{w.hi}]",
                     op="ck", pos=pos,
                     hint="use `wr` to wrap, `st` to saturate, or widen the type")
    vm.push(_typed(w, n))


# --- SIMD vectors (v0.2; SPEC Part II §11) -------------------------------------
# A constructor word evaluates a block to produce its lanes: `[1 2 3 4] vi`. Lane
# count and type are fixed by the word. Integer lanes wrap to their width (the
# same no-silent-overflow rule). Every vector word starts with `v` and is two
# characters: constructors `vf vd vi vu`, elementwise `va vs vm`. A digit after
# `i` or `u` is a scalar bit width, so a vector is never spelled `i4` or `b16`.
_VECS = [
    # name, count, is_float, signed, bits
    ("vf", 4, True, True, 32),   # f32 x4
    ("vd", 2, True, True, 64),   # f64 x2
    ("vi", 4, False, True, 32),  # i32 x4
    ("vu", 16, False, False, 8), # u8 x16
]


def _coerce_lane(vm, name: str, pos: Pos, x, is_float: bool, signed: bool, bits: int):
    from .vm import Big, is_num, type_name
    if not is_num(x):
        raise vm.err(TYPE_MISMATCH, f"`{name}` lanes must be numbers, got {type_name(x)}",
                     op=name, pos=pos)
    if isinstance(x, Big):
        x = x.v  # a big lane is reduced to the lane width like any integer
    if is_float:
        return vm.to_float(name, pos, x)
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
            vm.invoke(blk, name, pos)
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


def b_h3(vm, pos: Pos) -> None:
    text = vm.pop("h3", pos)
    if not isinstance(text, str):
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`h3` expected a string, got {type_name(text)}",
                     op="h3", pos=pos, hint="BLAKE3 hashes the string's UTF-8 bytes")
    vm.push(blake3_hex(text.encode("utf-8")))


def b_ni(vm, pos: Pos) -> None:
    vm.push(vm.node_id.hex())


def b_ua(vm, pos: Pos) -> None:
    vm.push(ula(vm.node_id))


def b_vsum(vm, pos: Pos) -> None:
    # vec vh  -> horizontal sum to a scalar (exact; lane width is not re-applied).
    v = _pop_vec(vm, "vh", pos)
    total = sum(v.lanes)
    from .vm import _num_result
    vm.push(_num_result(total) if v.is_float else total)


def b_at(vm, pos: Pos) -> None:
    # vec idx at  -> the lane at idx (BOUNDS if out of range).
    from .errors import BOUNDS
    idx = _pop_intish(vm, "at", pos)
    v = _pop_vec(vm, "at", pos)
    if idx < 0 or idx >= len(v.lanes):
        raise vm.err(BOUNDS, f"lane {idx} is outside {v.name} [0..{len(v.lanes) - 1}]",
                     op="at", pos=pos)
    vm.push(v.lanes[idx])


# word -> (handler, alias, description)
WORDS: dict[str, tuple[Callable[[Any, Pos], None], str, str]] = {
    "bf": (b_bf, "buf", "n bf -> an exclusive buffer of n zero bytes"),
    "bg": (b_bg, "buf_get", "buf idx bg -> the byte at idx; the buf stays"),
    "bp": (b_bp, "buf_put", "buf idx byte bp -> store byte; exclusive buf only; the buf stays"),
    "hd": (b_head, "head", "str hd -> the first character (BOUNDS if the string is empty)"),
    "tl": (b_tail, "tail", "str tl -> the string after the first character (empty stays empty)"),
    "ln": (b_len, "length", "length of a string (chars) or block (nodes)"),
    "tr": (b_trim, "trim", "strip leading/trailing whitespace from a string"),
    "up": (b_upper, "upper", "uppercase a string"),
    "lo": (b_lower, "lower", "lowercase a string"),
    "rv": (b_rev, "reverse", "reverse a string"),
    "pr": (b_print_raw, "print_raw", "print top value with no trailing newline"),
    "ab": (b_abs, "abs", "absolute value of a number"),
    "mx": (b_max, "max", "max of the top two numbers (defined in std/core.zn)"),
    "mn": (b_min, "min", "min of the top two numbers (defined in std/core.zn)"),
    "sq": (b_sqrt, "sqrt", "square root of a number"),
    "pw": (b_pow, "pow", "base exponent -> base**exponent"),
    "fl": (b_floor, "floor", "floor of a number"),
    "ce": (b_ceil, "ceil", "ceiling of a number"),
    "rd": (b_round, "round", "round a number to nearest int"),
    "s": (b_to_str, "to_str", "convert top value to its string form"),
    "n": (b_to_num, "to_num", "parse a string into a number (int must fit i64; else use b)"),
    "b": (b_big, "big", "v b -> arbitrary-precision big from an int, whole float, or digit string"),
    "dp": (b_depth, "depth", "push the current stack depth"),
    "rt": (b_rot, "rot", "rotate the top three values (a b c -> b c a)"),
    "cl": (b_clear, "clear", "clear the entire stack (defined in std/core.zn)"),
    "wd": (b_wd, "word", "[body] $name$ wd -> register body as an auto-running word"),
    "ts": (b_ts, "test", "[body] ts -> the body must leave exactly one truthy value"),
    "pp": (b_pp, "prop", "k [body] pp -> run body k times; each run leaves one truthy value"),
    "h3": (b_h3, "blake3", "str h3 -> 64 hex digits of BLAKE3(UTF-8)"),
    "ni": (b_ni, "node_id", "push this VM's 32-byte node id as 64 hex digits"),
    "ua": (b_ua, "node_addr", "push this VM's IPv6 ULA (fd00::/8 from the node id)"),
    "co": (b_co, "color", "r g b co -> a color; each channel is an int in 0..255"),
    "tx": (b_tx, "text", "$text$ size color tx -> a text node (width is characters times size)"),
    "bt": (b_bt, "button", "$label$ size color bt -> a button node (role button, padded label)"),
    "bh": (b_bh, "button_handler", "$label$ size color [body] bh -> a button that runs body when it is hit"),
    "pk": (b_pk, "pointer", "node x y pk -> run the deepest button handler that contains the point"),
    "ky": (b_ky, "key", "node $key$ ky -> run the handler of the button whose label is that key"),
    "an": (b_an, "animate", "node n an -> add n to the node width; n is a tick count"),
    "gp": (b_gp, "gpu", "w h [shader] gp -> buf of a rectangle in the shader's color"),
    "rw": (b_rw, "row", "color gap pad [nodes] rw -> a row box, children start-aligned"),
    "cm": (b_cm, "column", "color gap pad [nodes] cm -> a column box, children start-aligned"),
    "ly": (b_ly, "layout", "node ly -> the preorder layout dump"),
    "gv": (b_gv, "graph_vert", "name gv -> ensure a vertex; a duplicate is a no-op"),
    "ge": (b_ge, "graph_edge", "src label dst ge -> ensure an edge (and its vertices)"),
    "gs": (b_gs, "graph_snap", "push the canonical snapshot text"),
    "gl": (b_gl, "graph_load", "snapshot gl -> replace the graph from that text"),
    "gh": (b_gh, "graph_head", "push BLAKE3 of the snapshot bytes"),
    "sb": (b_sb, "sync_bundle", "push the graph sealed as a zown-sync bundle"),
    "so": (b_so, "sync_open", "bundle so -> verify the seal, replace the graph, push the head"),
    "sl": (b_sl, "sync_sign", "seed sl -> the sealed bundle plus an Ed25519 trailer"),
    "la": (b_la, "log_append", "append this bundle's digest to the transparency log and push the new size"),
    "lp": (b_lp, "log_proof", "push an inclusion-proof line for this bundle, or BOUNDS if it was not logged"),
    "lr": (b_lr, "log_root", "push the log root as 64 hex digits, or \"\" when the log is empty"),
    "mg": (b_mg, "merge", "snapshot mg -> union it into the graph, both edges kept, sorted"),
    "tc": (b_tc, "table_col", "name tc -> ensure a column; a duplicate name stays put"),
    "tu": (b_tu, "table_row", "cells tu -> append one row, one single-word cell per column"),
    "tq": (b_tq, "table_scan", "name tq -> push that column, one cell per line"),
    "mm": (b_mm, "map_write", "path mm -> fsync the snapshot at path; needs `d"),
    "mo": (b_mo, "map_open", "path mo -> exclusive buf of that snapshot file; needs `d"),
    "ml": (b_ml, "map_load", "path ml -> replace the graph from that snapshot file; needs `d"),
    "ch": (b_ch, "content", "64-hex ch -> tag later log lines with that content hash"),
    "vr": (b_vr, "version", "word vr -> tag later log lines with that version"),
    "td": (b_td, "trace", "word td -> tag later log lines with that trace id"),
    "sp": (b_sp, "span", "name sp -> open a span"),
    "se": (b_se, "span_end", "se -> close the innermost span"),
    "nt": (b_nt, "note", "text nt -> append a one-line note"),
    "wn": (b_wn, "want", "name value wn -> declare the desired word for name"),
    "ob": (b_ob, "observe", "name value ob -> record the actual word; log meet or drift"),
    "df": (b_df, "drift", "push the current drift report (ok when every want is met)"),
    "rm": (b_rm, "remedy", "code note rm -> remember a note for the next fault with that code"),
    "lg": (b_lg, "log", "push the telemetry log"),
    "gd": (b_gd, "grant_disk", "path [body] gd -> run body with `d and this path as a root"),
    "fr": (b_fr, "file_read", "path fr -> exclusive buf of the file bytes; path must be under a gd root"),
    "fw": (b_fw, "file_write", "buf path fw -> write every byte; the buf stays; path must be under a gd root"),
    "ac": (b_ac, "argc", "ac -> how many program arguments the host passed"),
    "ag": (b_ag, "arg", "idx ag -> that program argument (BOUNDS outside 0..count)"),
    "en": (b_en, "env", "name en -> that variable; unset is BOUNDS; needs `v"),
    "tm": (b_tm, "time", "tm -> monotonic tick, then wall seconds; needs `c; ZOWN_TICK and ZOWN_WALL pin them"),
    "ae": (b_ae, "aead_seal", "key nonce plain aad ae -> buf of ciphertext || 16-byte ChaCha20-Poly1305 tag"),
    "ad": (b_ad, "aead_open", "key nonce boxed aad ad -> plaintext buf; a bad tag is INTEGRITY_FAIL"),
    "es": (b_es, "ed_sign", "seed message es -> 64-byte Ed25519 signature"),
    "ev": (b_ev, "ed_verify", "public message sig ev -> 1; a bad signature is SIG_INVALID"),
    "xk": (b_xk, "x25519", "scalar point xk -> 32-byte X25519 shared secret"),
    "ct": (b_cat, "concat", "concatenate two strings (a b ct)"),
    "rp": (b_repeat, "repeat", "str count rp -> the string repeated count times (count <= 0 yields \"\")"),
    "hw": (b_hw, "cpus", "push the CPU count (ZOWN_CPUS overrides the host)"),
    "lc": (b_lc, "lanes", "n lc -> pin min(n, max(cpus-1, 1)) real-time slots and push the grant"),
    "rn": (b_rn, "run_ticks", "k rn -> run k scheduler ticks (lanes, then at most one job)"),
    "pm": (b_pm, "pull_in", "pull one value from the input lane pipe (empty is BOUNDS)"),
    "pn": (b_pn, "pull_net", "pull one value from the net lane pipe (empty is BOUNDS)"),
    "ws": (b_ws, "steals", "push how many times a compute worker stole a job"),
    "sg": (b_sg, "trace", "push the lane trace (m, n, and c for a compute slice)"),
    "ty": (b_attempt, "attempt", "[body] ty -> ok(value) or err(code); unwrap with ?? [ok]/[err] arms"),
    "at": (b_at, "at", "vec idx at -> the lane at idx (BOUNDS if out of range)"),
    "gr": (b_grant, "grant", "`cap [body] gr -> run body with `cap granted, then restore"),
    "rq": (b_require, "require", "`cap rq -> assert capability is granted (CAP_DENIED if not)"),
    "hv": (b_have, "have", "`cap hv -> push 1 if capability is granted, else 0"),
    "wr": (b_wrap, "wrap", "n width wr -> n wrapped into width (two's complement)"),
    "st": (b_sat, "saturate", "n width st -> n clamped to width's [min..max]"),
    "ck": (b_chk, "check", "n width ck -> n if it fits width, else OVERFLOW"),
    "va": (_make_vbinop("va", lambda x, y: x + y), "vec_add", "elementwise add two matching vectors"),
    "vs": (_make_vbinop("vs", lambda x, y: x - y), "vec_sub", "elementwise subtract two matching vectors"),
    "vm": (_make_vbinop("vm", lambda x, y: x * y), "vec_mul", "elementwise multiply two matching vectors"),
    "vh": (b_vsum, "vec_sum", "horizontal sum of a vector's lanes to a scalar"),
    "iz": (b_iz, "isize", "width tag: signed pointer-sized integer (64 bits)"),
    "uz": (b_uz, "usize", "width tag: unsigned pointer-sized integer (64 bits)"),
    "dc": (b_dc, "decimal", "int, fitting big, or $1.50$ dc -> a checked decimal"),
    "cx": (b_cx, "complex", "re im cx -> a Gaussian integer"),
    "ls": (b_ls, "list", "ls -> an empty exclusive list"),
    "mp": (b_mp, "map", "mp -> an empty exclusive map"),
    "sc": (b_sc, "set", "sc -> an empty exclusive set"),
    "qu": (b_qu, "queue", "qu -> an empty exclusive queue"),
    "rg": (b_rg, "ring", "n rg -> an exclusive ring of capacity n"),
    "te": (b_te, "btree", "te -> an empty exclusive B-tree"),
    "ti": (b_ti, "trie", "ti -> an empty exclusive trie"),
    "bm": (b_bm, "bloom", "n bm -> an exclusive bloom filter of n bits"),
    "pq": (b_pq, "pqueue", "pq -> an empty exclusive priority queue"),
    "bv": (b_bv, "bitvec", "n bv -> an exclusive bit vector of n zero bits"),
    "ps": (b_ps, "push", "coll v ps -> insert v; the collection stays"),
    "po": (b_po, "pop", "coll po -> remove the next value and push it"),
    "gt": (b_gt, "get", "coll idx-or-key gt -> that element; the collection stays"),
    "pt": (b_pt, "put", "coll key val pt, or trie key pt, or bits idx bit pt"),
    "hs": (b_hs, "has", "coll v hs -> 1 if present, else 0"),
    "sk": (b_sk, "scan", "coll sk -> the canonical text; the collection stays"),
    "px": (b_px, "prefix", "trie prefix px -> keys with that prefix, one per line"),
}

# fixed-width integer type tags (i8..ix, u8..ux): each pushes a WidthTag value.
for _name, _signed, _bits in _WIDTHS:
    _kind = "signed" if _signed else "unsigned"
    _alias = ("int" if _signed else "uint") + str(_bits)
    WORDS[_name] = (
        _make_width(_signed, _bits),
        _alias,
        f"width tag: {_kind} {_bits}-bit integer",
    )

# SIMD vector constructor words: `[ ...lanes ] vf` etc.
for _vname, _count, _isf, _vsigned, _vbits in _VECS:
    _lane = ("f" if _isf else ("i" if _vsigned else "u")) + str(_vbits)
    WORDS[_vname] = (
        _make_vec(_vname, _count, _isf, _vsigned, _vbits),
        f"{_lane}x{_count}",
        f"build a {_lane} x{_count} SIMD vector from a block of {_count} lanes",
    )

BUILTINS: dict[str, Callable[[Any, Pos], None]] = {k: v[0] for k, v in WORDS.items()}
