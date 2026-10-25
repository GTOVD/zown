"""Decimal and complex values (SPEC §11).

`dc` is a checked decimal: an `int` coefficient and a scale in `0..18`.
`cx` is a Gaussian integer, two checked `int` components. Neither is a
`float` and neither is a `big`. An operation whose coefficient or component
does not fit `int` is `OVERFLOW`.
"""

from __future__ import annotations

from dataclasses import dataclass

from .errors import OVERFLOW, TYPE_MISMATCH
from .lexer import INT_MAX, INT_MIN

SCALE_MAX = 18


@dataclass(frozen=True)
class Dec:
    coeff: int
    scale: int

    def text(self) -> str:
        sign = "-" if self.coeff < 0 else ""
        digits = str(abs(self.coeff))
        if self.scale == 0:
            return sign + digits
        if len(digits) <= self.scale:
            digits = "0" * (self.scale - len(digits) + 1) + digits
        cut = len(digits) - self.scale
        return f"{sign}{digits[:cut]}.{digits[cut:]}"

    def __repr__(self) -> str:  # pragma: no cover
        return self.text()


@dataclass(frozen=True)
class Cx:
    re: int
    im: int

    def text(self) -> str:
        return f"cx({self.re} {self.im})"

    def __repr__(self) -> str:  # pragma: no cover
        return self.text()


def _fit(vm, op: str, pos, n: int) -> int:
    if n < INT_MIN or n > INT_MAX:
        raise vm.err(OVERFLOW, f"`{op}` result {n} does not fit int", op=op, pos=pos,
                     hint="dc and cx stay inside the checked int; they are not big")
    return n


def _pow10(k: int) -> int:
    p = 1
    for _ in range(k):
        p *= 10
    return p


def _align(vm, op: str, pos, a: Dec, b: Dec) -> tuple[Dec, Dec]:
    if a.scale == b.scale:
        return a, b
    if a.scale < b.scale:
        small, big, flip = a, b, False
    else:
        small, big, flip = b, a, True
    grown = _fit(vm, op, pos, small.coeff * _pow10(big.scale - small.scale))
    lifted = Dec(grown, big.scale)
    return (lifted, big) if not flip else (big, lifted)


def dec_eq(a: Dec, b: Dec) -> bool:
    scale = max(a.scale, b.scale)
    return a.coeff * _pow10(scale - a.scale) == b.coeff * _pow10(scale - b.scale)


def dec_lt(a: Dec, b: Dec) -> bool:
    scale = max(a.scale, b.scale)
    return a.coeff * _pow10(scale - a.scale) < b.coeff * _pow10(scale - b.scale)


def dec_add(vm, op: str, pos, a: Dec, b: Dec, sub: bool) -> Dec:
    a, b = _align(vm, op, pos, a, b)
    coeff = a.coeff - b.coeff if sub else a.coeff + b.coeff
    return Dec(_fit(vm, op, pos, coeff), a.scale)


def dec_mul(vm, op: str, pos, a: Dec, b: Dec) -> Dec:
    scale = a.scale + b.scale
    if scale > SCALE_MAX:
        raise vm.err(OVERFLOW, f"`{op}` decimal scale {scale} is above {SCALE_MAX}",
                     op=op, pos=pos)
    return Dec(_fit(vm, op, pos, a.coeff * b.coeff), scale)


def dec_neg(vm, op: str, pos, a: Dec) -> Dec:
    return Dec(_fit(vm, op, pos, -a.coeff), a.scale)


def cx_add(vm, op: str, pos, a: Cx, b: Cx, sub: bool) -> Cx:
    if sub:
        return Cx(_fit(vm, op, pos, a.re - b.re), _fit(vm, op, pos, a.im - b.im))
    return Cx(_fit(vm, op, pos, a.re + b.re), _fit(vm, op, pos, a.im + b.im))


def cx_mul(vm, op: str, pos, a: Cx, b: Cx) -> Cx:
    re = _fit(vm, op, pos, a.re * b.re - a.im * b.im)
    im = _fit(vm, op, pos, a.re * b.im + a.im * b.re)
    return Cx(re, im)


def cx_neg(vm, op: str, pos, a: Cx) -> Cx:
    return Cx(_fit(vm, op, pos, -a.re), _fit(vm, op, pos, -a.im))


def _bad_dec(vm, op: str, pos, text: str):
    raise vm.err(TYPE_MISMATCH, f"`{op}` expected a decimal literal, got {text!r}",
                 op=op, pos=pos)


def parse_dec(vm, op: str, pos, text: str) -> Dec:
    """`$1.50$` or `$-2$`. A digit is required on both sides of a dot."""
    if not text or "\n" in text or text.count(".") > 1 or text.count("-") > 1:
        _bad_dec(vm, op, pos, text)
    neg = text[0] == "-"
    body = text[1:] if neg else text
    if not body or body[0] == "." or body[-1] == "." or "-" in body:
        _bad_dec(vm, op, pos, text)
    if "." in body:
        whole, frac = body.split(".", 1)
        if not whole.isdigit() or not frac.isdigit():
            _bad_dec(vm, op, pos, text)
        scale = len(frac)
        digits = whole + frac
    else:
        if not body.isdigit():
            _bad_dec(vm, op, pos, text)
        scale = 0
        digits = body
    if scale > SCALE_MAX:
        raise vm.err(OVERFLOW, f"`{op}` decimal scale {scale} is above {SCALE_MAX}",
                     op=op, pos=pos)
    coeff = int(digits)
    if neg:
        coeff = -coeff
    return Dec(_fit(vm, op, pos, coeff), scale)
