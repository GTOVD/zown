"""ChaCha20-Poly1305 as AEAD, IETF construction (RFC 8439).

A key is 32 bytes and a nonce is 12 bytes. The seal is the ciphertext
with the 16-byte tag appended. This is the real algorithm, not a
constant-time implementation.
"""

from __future__ import annotations

_P130 = (1 << 130) - 5


def _rotl(x: int, n: int) -> int:
    return ((x << n) | (x >> (32 - n))) & 0xFFFFFFFF


def _quarter(s: list[int], a: int, b: int, c: int, d: int) -> None:
    s[a] = (s[a] + s[b]) & 0xFFFFFFFF
    s[d] = _rotl(s[d] ^ s[a], 16)
    s[c] = (s[c] + s[d]) & 0xFFFFFFFF
    s[b] = _rotl(s[b] ^ s[c], 12)
    s[a] = (s[a] + s[b]) & 0xFFFFFFFF
    s[d] = _rotl(s[d] ^ s[a], 8)
    s[c] = (s[c] + s[d]) & 0xFFFFFFFF
    s[b] = _rotl(s[b] ^ s[c], 7)


def chacha20_block(key: bytes, nonce: bytes, counter: int) -> bytes:
    """One 64-byte ChaCha20 block. `nonce` is 12 bytes. `counter` is the 32-bit block count."""
    const = b"expand 32-byte k"
    state = [int.from_bytes(const[i:i + 4], "little") for i in range(0, 16, 4)]
    state += [int.from_bytes(key[i:i + 4], "little") for i in range(0, 32, 4)]
    state.append(counter & 0xFFFFFFFF)
    state += [int.from_bytes(nonce[i:i + 4], "little") for i in range(0, 12, 4)]
    w = state[:]
    for _ in range(10):
        _quarter(w, 0, 4, 8, 12)
        _quarter(w, 1, 5, 9, 13)
        _quarter(w, 2, 6, 10, 14)
        _quarter(w, 3, 7, 11, 15)
        _quarter(w, 0, 5, 10, 15)
        _quarter(w, 1, 6, 11, 12)
        _quarter(w, 2, 7, 8, 13)
        _quarter(w, 3, 4, 9, 14)
    out = bytearray()
    for i in range(16):
        out += ((w[i] + state[i]) & 0xFFFFFFFF).to_bytes(4, "little")
    return bytes(out)


def chacha20_xor(key: bytes, nonce: bytes, counter: int, data: bytes) -> bytes:
    out = bytearray(len(data))
    offset = 0
    block = counter
    while offset < len(data):
        stream = chacha20_block(key, nonce, block)
        take = min(64, len(data) - offset)
        for i in range(take):
            out[offset + i] = data[offset + i] ^ stream[i]
        offset += take
        block = (block + 1) & 0xFFFFFFFF
    return bytes(out)


def _clamp_r(key: bytes) -> int:
    r = bytearray(key[:16])
    r[3] &= 15
    r[7] &= 15
    r[11] &= 15
    r[15] &= 15
    r[4] &= 252
    r[8] &= 252
    r[12] &= 252
    return int.from_bytes(r, "little")


def poly1305(key: bytes, msg: bytes) -> bytes:
    """16-byte Poly1305 tag. `key` is 32 bytes."""
    r = _clamp_r(key)
    s = int.from_bytes(key[16:32], "little")
    h = 0
    i = 0
    while i < len(msg):
        chunk = msg[i:i + 16]
        n = int.from_bytes(chunk, "little") + (1 << (8 * len(chunk)))
        h = (h + n) * r % _P130
        i += 16
    tag = (h + s) & ((1 << 128) - 1)
    return tag.to_bytes(16, "little")


def _pad16(n: int) -> bytes:
    if n % 16 == 0:
        return b""
    return b"\0" * (16 - (n % 16))


def seal(key: bytes, nonce: bytes, plain: bytes, aad: bytes) -> bytes:
    otk = chacha20_block(key, nonce, 0)[:32]
    ct = chacha20_xor(key, nonce, 1, plain)
    mac = aad + _pad16(len(aad)) + ct + _pad16(len(ct))
    mac += len(aad).to_bytes(8, "little") + len(ct).to_bytes(8, "little")
    return ct + poly1305(otk, mac)


def open_box(key: bytes, nonce: bytes, boxed: bytes, aad: bytes) -> bytes | None:
    """Plaintext, or None when the tag does not match."""
    if len(boxed) < 16:
        return None
    ct, tag = boxed[:-16], boxed[-16:]
    otk = chacha20_block(key, nonce, 0)[:32]
    mac = aad + _pad16(len(aad)) + ct + _pad16(len(ct))
    mac += len(aad).to_bytes(8, "little") + len(ct).to_bytes(8, "little")
    if poly1305(otk, mac) != tag:
        return None
    return chacha20_xor(key, nonce, 1, ct)
