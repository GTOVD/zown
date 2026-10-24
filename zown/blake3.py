"""Unkeyed BLAKE3 (32-byte digest).

This is the regular hash, not the keyed hash and not KDF. It follows the
BLAKE3 reference implementation: 7-round compression, 1024-byte chunks, and
the CV stack that parents subtrees as chunks complete. Test vectors live in
`tests/test_zown.py`.
"""

from __future__ import annotations

OUT_LEN = 32
BLOCK_LEN = 64
CHUNK_LEN = 1024

CHUNK_START = 1 << 0
CHUNK_END = 1 << 1
PARENT = 1 << 2
ROOT = 1 << 3

IV = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
]

MSG_PERMUTATION = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8]


def _rotr(x: int, n: int) -> int:
    return ((x >> n) | (x << (32 - n))) & 0xFFFFFFFF


def _g(state: list[int], a: int, b: int, c: int, d: int, mx: int, my: int) -> None:
    state[a] = (state[a] + state[b] + mx) & 0xFFFFFFFF
    state[d] = _rotr(state[d] ^ state[a], 16)
    state[c] = (state[c] + state[d]) & 0xFFFFFFFF
    state[b] = _rotr(state[b] ^ state[c], 12)
    state[a] = (state[a] + state[b] + my) & 0xFFFFFFFF
    state[d] = _rotr(state[d] ^ state[a], 8)
    state[c] = (state[c] + state[d]) & 0xFFFFFFFF
    state[b] = _rotr(state[b] ^ state[c], 7)


def _compress(cv: list[int], block: list[int], counter: int, block_len: int, flags: int) -> list[int]:
    state = [
        cv[0], cv[1], cv[2], cv[3], cv[4], cv[5], cv[6], cv[7],
        IV[0], IV[1], IV[2], IV[3],
        counter & 0xFFFFFFFF,
        (counter >> 32) & 0xFFFFFFFF,
        block_len & 0xFFFFFFFF,
        flags & 0xFFFFFFFF,
    ]
    m = block[:]
    for _ in range(7):
        _g(state, 0, 4, 8, 12, m[0], m[1])
        _g(state, 1, 5, 9, 13, m[2], m[3])
        _g(state, 2, 6, 10, 14, m[4], m[5])
        _g(state, 3, 7, 11, 15, m[6], m[7])
        _g(state, 0, 5, 10, 15, m[8], m[9])
        _g(state, 1, 6, 11, 12, m[10], m[11])
        _g(state, 2, 7, 8, 13, m[12], m[13])
        _g(state, 3, 4, 9, 14, m[14], m[15])
        m = [m[i] for i in MSG_PERMUTATION]
    for i in range(8):
        state[i] ^= state[i + 8]
        state[i + 8] ^= cv[i]
    return state


def _words(block: bytes) -> list[int]:
    padded = block.ljust(BLOCK_LEN, b"\0")
    return [int.from_bytes(padded[i:i + 4], "little") for i in range(0, BLOCK_LEN, 4)]


def _le_bytes(words: list[int], n: int) -> bytes:
    raw = b"".join(w.to_bytes(4, "little") for w in words)
    return raw[:n]


class _Output:
    def __init__(self, cv: list[int], block: list[int], counter: int, block_len: int, flags: int):
        self.cv = cv
        self.block = block
        self.counter = counter
        self.block_len = block_len
        self.flags = flags

    def chaining_value(self) -> list[int]:
        return _compress(self.cv, self.block, self.counter, self.block_len, self.flags)[:8]

    def root_bytes(self, length: int = OUT_LEN) -> bytes:
        out = bytearray()
        counter = 0
        while len(out) < length:
            words = _compress(
                self.cv, self.block, counter, self.block_len, self.flags | ROOT,
            )
            out.extend(_le_bytes(words, 64))
            counter += 1
        return bytes(out[:length])


class _Chunk:
    def __init__(self, key: list[int], counter: int, flags: int):
        self.cv = key[:]
        self.counter = counter
        self.block = bytearray(BLOCK_LEN)
        self.block_len = 0
        self.blocks_compressed = 0
        self.flags = flags

    def length(self) -> int:
        return BLOCK_LEN * self.blocks_compressed + self.block_len

    def _start_flag(self) -> int:
        return CHUNK_START if self.blocks_compressed == 0 else 0

    def update(self, data: bytes) -> None:
        offset = 0
        while offset < len(data):
            if self.block_len == BLOCK_LEN:
                words = _words(bytes(self.block))
                self.cv = _compress(
                    self.cv, words, self.counter, BLOCK_LEN, self.flags | self._start_flag(),
                )[:8]
                self.blocks_compressed += 1
                self.block = bytearray(BLOCK_LEN)
                self.block_len = 0
            take = min(BLOCK_LEN - self.block_len, len(data) - offset)
            self.block[self.block_len:self.block_len + take] = data[offset:offset + take]
            self.block_len += take
            offset += take

    def output(self) -> _Output:
        return _Output(
            self.cv[:],
            _words(bytes(self.block[:self.block_len])),
            self.counter,
            self.block_len,
            self.flags | self._start_flag() | CHUNK_END,
        )


def _parent(left: list[int], right: list[int], key: list[int], flags: int) -> _Output:
    return _Output(key[:], left + right, 0, BLOCK_LEN, PARENT | flags)


class Hasher:
    def __init__(self) -> None:
        self.key = IV[:]
        self.flags = 0
        self.chunk = _Chunk(self.key, 0, self.flags)
        self.stack: list[list[int]] = []

    def update(self, data: bytes) -> None:
        offset = 0
        while offset < len(data):
            if self.chunk.length() == CHUNK_LEN:
                cv = self.chunk.output().chaining_value()
                total = self.chunk.counter + 1
                self._push(cv, total)
                self.chunk = _Chunk(self.key, total, self.flags)
            take = min(CHUNK_LEN - self.chunk.length(), len(data) - offset)
            self.chunk.update(data[offset:offset + take])
            offset += take

    def _push(self, cv: list[int], total: int) -> None:
        while total & 1 == 0:
            cv = _parent(self.stack.pop(), cv, self.key, self.flags).chaining_value()
            total >>= 1
        self.stack.append(cv)

    def digest(self) -> bytes:
        output = self.chunk.output()
        for parent in reversed(self.stack):
            output = _parent(parent, output.chaining_value(), self.key, self.flags)
        return output.root_bytes()


def blake3(data: bytes) -> bytes:
    """32-byte BLAKE3 digest of `data`."""
    hasher = Hasher()
    hasher.update(data)
    return hasher.digest()


def blake3_hex(data: bytes) -> str:
    return blake3(data).hex()
