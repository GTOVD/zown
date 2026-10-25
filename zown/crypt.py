"""Crypto words (SPEC §23). `ae`/`ad` are ChaCha20-Poly1305, `es`/`ev` are
Ed25519, `xk` is X25519. Every key and message is a `buf`. A bare name
passes a borrow, so the binding keeps the exclusive value.
"""

from __future__ import annotations

from .aead import open_box, seal
from .buf import BUF_MAX, Alloc, Buf, release
from .edwards import sign, verify, x25519
from .errors import BOUNDS, INTEGRITY_FAIL, SIG_INVALID, TYPE_MISMATCH, Pos


def _take(vm, op: str, pos: Pos) -> bytes:
    from .vm import type_name
    v = vm.pop(op, pos)
    if not isinstance(v, Buf):
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a buf, got {type_name(v)}",
                     op=op, pos=pos, hint="keys and messages are bufs")
    if v.alloc.data is None:
        raise vm.err(BOUNDS, f"`{op}` buffer was freed",
                     op=op, pos=pos, hint="the exclusive owner was dropped with `,`")
    data = bytes(v.alloc.data)
    release(v)
    return data


def _push(vm, data: bytes) -> None:
    alloc = Alloc(len(data))
    if data and alloc.data is not None:
        alloc.data[:] = data
    vm.push(Buf(alloc, True))


def _len(vm, op: str, pos: Pos, data: bytes, n: int, what: str) -> None:
    if len(data) != n:
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected {what} of {n} bytes",
                     op=op, pos=pos)


def _sec(vm, code: str, op: str, pos: Pos, msg: str, hint: str):
    raise vm.err(code, msg, op=op, pos=pos, kind="sec", hint=hint)


def b_ae(vm, pos: Pos) -> None:
    # key nonce plain aad ae -> ciphertext || tag
    aad = _take(vm, "ae", pos)
    plain = _take(vm, "ae", pos)
    nonce = _take(vm, "ae", pos)
    key = _take(vm, "ae", pos)
    _len(vm, "ae", pos, key, 32, "a key")
    _len(vm, "ae", pos, nonce, 12, "a nonce")
    if len(plain) + 16 > BUF_MAX:
        raise vm.err(BOUNDS, f"`ae` seal is larger than {BUF_MAX} bytes", op="ae", pos=pos)
    _push(vm, seal(key, nonce, plain, aad))


def b_ad(vm, pos: Pos) -> None:
    # key nonce boxed aad ad -> plaintext, or INTEGRITY_FAIL
    aad = _take(vm, "ad", pos)
    boxed = _take(vm, "ad", pos)
    nonce = _take(vm, "ad", pos)
    key = _take(vm, "ad", pos)
    _len(vm, "ad", pos, key, 32, "a key")
    _len(vm, "ad", pos, nonce, 12, "a nonce")
    if len(boxed) < 16:
        raise vm.err(TYPE_MISMATCH, "`ad` expected a seal of at least 16 bytes",
                     op="ad", pos=pos)
    plain = open_box(key, nonce, boxed, aad)
    if plain is None:
        _sec(vm, INTEGRITY_FAIL, "ad", pos, "`ad` tag does not match",
             "the ciphertext, aad, key, or nonce changed")
    _push(vm, plain)


def b_es(vm, pos: Pos) -> None:
    # seed message es -> 64-byte signature
    message = _take(vm, "es", pos)
    seed = _take(vm, "es", pos)
    _len(vm, "es", pos, seed, 32, "a seed")
    _push(vm, sign(seed, message))


def b_ev(vm, pos: Pos) -> None:
    # public message sig ev -> 1, or SIG_INVALID
    sig = _take(vm, "ev", pos)
    message = _take(vm, "ev", pos)
    public = _take(vm, "ev", pos)
    _len(vm, "ev", pos, public, 32, "a public key")
    _len(vm, "ev", pos, sig, 64, "a signature")
    if not verify(public, message, sig):
        _sec(vm, SIG_INVALID, "ev", pos, "`ev` signature is not valid",
             "the signature, message, or public key does not match")
    vm.push(1)


def b_xk(vm, pos: Pos) -> None:
    # scalar point xk -> 32-byte shared secret
    point = _take(vm, "xk", pos)
    scalar = _take(vm, "xk", pos)
    _len(vm, "xk", pos, scalar, 32, "a scalar")
    _len(vm, "xk", pos, point, 32, "a point")
    _push(vm, x25519(scalar, point))
