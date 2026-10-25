"""Ed25519 (RFC 8032) and X25519 (RFC 7748).

Signatures and shared secrets match the published test vectors. The
arithmetic is the real curve arithmetic. It is not constant-time.
"""

from __future__ import annotations

import hashlib

P = 2**255 - 19
L = 2**252 + 27742317777372353535851937790883648493
D = (-121665 * pow(121666, -1, P)) % P
I = pow(2, (P - 1) // 4, P)  # sqrt(-1)
A24 = 121665


def _sha512(data: bytes) -> bytes:
    return hashlib.sha512(data).digest()


def _inv(x: int) -> int:
    return pow(x, P - 2, P)


def _recover_x(y: int, sign: int) -> int | None:
    if y >= P:
        return None
    y2 = y * y % P
    xx = (y2 - 1) * _inv(D * y2 + 1) % P
    x = pow(xx, (P + 3) // 8, P)
    if (x * x - xx) % P != 0:
        x = x * I % P
    if (x * x - xx) % P != 0:
        return None
    if (x & 1) != sign:
        x = P - x
    return x


def _encode(x: int, y: int) -> bytes:
    bits = y | ((x & 1) << 255)
    return bits.to_bytes(32, "little")


def _decode(raw: bytes) -> tuple[int, int] | None:
    if len(raw) != 32:
        return None
    y = int.from_bytes(raw, "little") & ((1 << 255) - 1)
    sign = raw[31] >> 7
    x = _recover_x(y, sign)
    if x is None:
        return None
    return x, y


class _Pt:
    __slots__ = ("x", "y", "z", "t")

    def __init__(self, x: int, y: int, z: int, t: int) -> None:
        self.x, self.y, self.z, self.t = x, y, z, t


def _affine(x: int, y: int) -> _Pt:
    return _Pt(x, y, 1, x * y % P)


def _identity() -> _Pt:
    return _Pt(0, 1, 1, 0)


def _add(p: _Pt, q: _Pt) -> _Pt:
    a = (p.y - p.x) * (q.y - q.x) % P
    b = (p.y + p.x) * (q.y + q.x) % P
    c = (2 * D) * p.t * q.t % P
    d = 2 * p.z * q.z % P
    e = (b - a) % P
    f = (d - c) % P
    g = (d + c) % P
    h = (b + a) % P
    return _Pt(e * f % P, g * h % P, f * g % P, e * h % P)


def _dbl(p: _Pt) -> _Pt:
    a = p.x * p.x % P
    b = p.y * p.y % P
    c = 2 * p.z * p.z % P
    h = (a + b) % P
    e = (h - (p.x + p.y) * (p.x + p.y)) % P
    g = (a - b) % P
    f = (c + g) % P
    return _Pt(e * f % P, g * h % P, f * g % P, e * h % P)


def _encode_pt(p: _Pt) -> bytes:
    zinv = _inv(p.z)
    return _encode(p.x * zinv % P, p.y * zinv % P)


def _scalarmult(p: _Pt, s: int) -> _Pt:
    r = _identity()
    q = p
    while s:
        if s & 1:
            r = _add(r, q)
        q = _dbl(q)
        s >>= 1
    return r


_BY = 4 * _inv(5) % P
_BX = _recover_x(_BY, 0)
assert _BX is not None
_B = _affine(_BX, _BY)


def _clamp_ed(h0: bytes) -> int:
    a = bytearray(h0[:32])
    a[0] &= 248
    a[31] &= 63
    a[31] |= 64
    return int.from_bytes(a, "little")


def public_key(seed: bytes) -> bytes:
    a = _clamp_ed(_sha512(seed))
    return _encode_pt(_scalarmult(_B, a))


def sign(seed: bytes, message: bytes) -> bytes:
    digest = _sha512(seed)
    a = _clamp_ed(digest)
    prefix = digest[32:]
    pub = _encode_pt(_scalarmult(_B, a))
    r = int.from_bytes(_sha512(prefix + message), "little") % L
    big_r = _encode_pt(_scalarmult(_B, r))
    k = int.from_bytes(_sha512(big_r + pub + message), "little") % L
    s = (r + k * a) % L
    return big_r + s.to_bytes(32, "little")


def verify(public: bytes, message: bytes, signature: bytes) -> bool:
    if len(public) != 32 or len(signature) != 64:
        return False
    big_r, s_bytes = signature[:32], signature[32:]
    s = int.from_bytes(s_bytes, "little")
    if s >= L:
        return False
    a = _decode(public)
    r = _decode(big_r)
    if a is None or r is None:
        return False
    k = int.from_bytes(_sha512(big_r + public + message), "little") % L
    left = _encode_pt(_scalarmult(_B, s))
    right = _encode_pt(_add(_affine(*r), _scalarmult(_affine(*a), k)))
    return left == right


def x25519(scalar: bytes, point: bytes) -> bytes:
    k = bytearray(scalar)
    k[0] &= 248
    k[31] &= 127
    k[31] |= 64
    x1 = int.from_bytes(point, "little") % P
    x2, z2 = 1, 0
    x3, z3 = x1, 1
    swap = 0
    for t in range(254, -1, -1):
        kt = (k[t // 8] >> (t % 8)) & 1
        swap ^= kt
        if swap:
            x2, x3 = x3, x2
            z2, z3 = z3, z2
        swap = kt
        a = (x2 + z2) % P
        aa = a * a % P
        b = (x2 - z2) % P
        bb = b * b % P
        e = (aa - bb) % P
        c = (x3 + z3) % P
        d = (x3 - z3) % P
        da = d * a % P
        cb = c * b % P
        x3 = (da + cb) ** 2 % P
        z3 = x1 * (da - cb) ** 2 % P
        x2 = aa * bb % P
        z2 = e * (aa + A24 * e) % P
    if swap:
        x2, z2 = x3, z3
    return ((x2 * _inv(z2)) % P).to_bytes(32, "little")
