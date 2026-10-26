"""AV1 still-picture decode (SPEC §23).

`decode` reads a raw OBU sequence: a temporal delimiter, a sequence header,
and one frame. The result is 8-bit I420, Y then U then V. This slice
implements a keyframe with one tile, 4:2:0 or the header's subsampling,
no CDEF, no restoration, and no film grain. Anything else is UNSUPPORTED.
A size field that runs past the buffer is INTEGRITY_FAIL. An empty buffer
is BOUNDS.
"""

from __future__ import annotations

import copy

from .av1data import COEF, DQ8, FILTER_TAPS, KFYM, MODE, SCANS
from .errors import BOUNDS, INTEGRITY_FAIL, UNSUPPORTED

MASK64 = (1 << 64) - 1
I16N = -32768
I16P = 32767

# dav1d BlockLevel
BL_64 = 1
BL_32 = 2
BL_16 = 3
BL_8 = 4
PART_N = [0, 9, 9, 9, 3]

NONE, H, V, SPLIT = 0, 1, 2, 3
DC, VERT, HOR = 0, 1, 2
SMOOTH, SMOOTH_V, SMOOTH_H, PAETH = 9, 10, 11, 12
CFL = 13
DCT_DCT, ADST_DCT, DCT_ADST, ADST_ADST = 0, 1, 2, 3
IDTX = 9
TX_4, TX_8, TX_16 = 0, 1, 2
BS_16 = 12

INTRA_CTX = [0, 1, 2, 3, 4, 4, 4, 4, 3, 0, 1, 2, 0]
FILTER_TO_Y = [0, 1, 2, 6, 0]
V_DCT, H_DCT = 10, 11
WHT_WHT = 16
SKIP_CTX = [
    [1, 2, 2, 2, 3],
    [2, 4, 4, 4, 5],
    [2, 4, 4, 4, 5],
    [2, 4, 4, 4, 5],
    [3, 5, 5, 5, 6],
]
TX_TYPES_INTRA1 = [IDTX, DCT_DCT, V_DCT, H_DCT, ADST_ADST, ADST_DCT, DCT_ADST]
UV_TXTP = [DCT_DCT, ADST_DCT, DCT_ADST, DCT_DCT, ADST_ADST, ADST_DCT,
           DCT_ADST, DCT_ADST, ADST_DCT, ADST_ADST, ADST_DCT, DCT_ADST, ADST_ADST]
TX_TYPES_INTRA2 = [IDTX, DCT_DCT, ADST_ADST, ADST_DCT, DCT_ADST]
LO_CTX = [
    [[0, 1, 6, 6, 21], [1, 6, 6, 21, 21], [6, 6, 21, 21, 21],
     [6, 21, 21, 21, 21], [21, 21, 21, 21, 21]],
    [[0, 16, 6, 6, 21], [16, 16, 6, 21, 21], [16, 16, 21, 21, 21],
     [16, 16, 21, 21, 21], [16, 16, 21, 21, 21]],
    [[0, 11, 11, 11, 11], [11, 11, 11, 11, 11], [6, 6, 21, 21, 21],
     [6, 21, 21, 21, 21], [21, 21, 21, 21, 21]],
]
SCAN = {
    TX_4: SCANS["scan_4x4"], TX_8: SCANS["scan_8x8"], TX_16: SCANS["scan_16x16"],
}


class Fault(Exception):
    def __init__(self, code: str, msg: str) -> None:
        super().__init__(msg)
        self.code = code


def _bad(code: str, msg: str) -> None:
    raise Fault(code, msg)


def _clip8(v: int) -> int:
    if v < 0:
        return 0
    if v > 255:
        return 255
    return v


def _clip16(v: int) -> int:
    if v < I16N:
        return I16N
    if v > I16P:
        return I16P
    return v


class Bits:
    def __init__(self, data: bytes) -> None:
        self.data = data
        self.i = 0

    def u(self, n: int) -> int:
        v = 0
        for _ in range(n):
            if self.i >= len(self.data) * 8:
                _bad(INTEGRITY_FAIL, "truncated AV1 header")
            byte = self.data[self.i >> 3]
            v = (v << 1) | ((byte >> (7 - (self.i & 7))) & 1)
            self.i += 1
        return v

    def su(self, n: int) -> int:
        v = self.u(n)
        sign = v >> (n - 1)
        mag = v & ((1 << (n - 1)) - 1)
        return -mag if sign else mag

    def align(self) -> None:
        while self.i & 7:
            if self.u(1) != 0:
                _bad(INTEGRITY_FAIL, "AV1 alignment bit is not zero")

    def trailing(self) -> None:
        if self.i >= len(self.data) * 8:
            return
        if self.u(1) != 1:
            _bad(INTEGRITY_FAIL, "AV1 trailing bit is not one")
        while self.i & 7:
            if self.u(1) != 0:
                _bad(INTEGRITY_FAIL, "AV1 trailing padding is not zero")
        if self.i != len(self.data) * 8:
            _bad(INTEGRITY_FAIL, "AV1 header did not consume its payload")


class Msac:
    def __init__(self, data: bytes, update: bool) -> None:
        self.buf = data
        self.pos = 0
        self.end = len(data)
        self.dif = 0
        self.rng = 0x8000
        self.cnt = -15
        self.update = update
        self.starved = False
        self._refill()

    def _refill(self) -> None:
        c = 64 - self.cnt - 24
        dif = self.dif
        while True:
            if self.pos >= self.end:
                dif |= (~((~0xFF & MASK64) << c)) & MASK64
                self.starved = True
                break
            dif = (dif | ((self.buf[self.pos] ^ 0xFF) << c)) & MASK64
            self.pos += 1
            c -= 8
            if c < 0:
                break
        self.dif = dif & MASK64
        self.cnt = 64 - c - 24

    def _norm(self, dif: int, rng: int) -> None:
        clz = 32 - rng.bit_length() if rng else 32
        d = 15 ^ (31 ^ clz)
        cnt = self.cnt
        self.dif = (dif << d) & MASK64
        self.rng = rng << d
        self.cnt = cnt - d
        if (cnt & 0xFFFFFFFF) < (d & 0xFFFFFFFF):
            self._refill()

    def adapt_bool(self, cdf: list[int]) -> int:
        bit = self.bool(cdf[0])
        if self.update:
            count = cdf[1]
            rate = 4 + (count >> 4)
            if bit:
                cdf[0] += (32768 - cdf[0]) >> rate
            else:
                cdf[0] -= cdf[0] >> rate
            cdf[1] = count + (count < 32)
        return bit

    def bool(self, f: int) -> int:
        r = self.rng
        dif = self.dif
        v = (((r >> 8) * (f >> 6)) >> 1) + 4
        vw = v << 48
        ret = 1 if dif >= vw else 0
        dif = (dif - ret * vw) & MASK64
        v += ret * (r - 2 * v)
        self._norm(dif, v)
        return ret ^ 1

    def equi(self) -> int:
        r = self.rng
        dif = self.dif
        v = ((r >> 8) << 7) + 4
        vw = v << 48
        ret = 1 if dif >= vw else 0
        dif = (dif - ret * vw) & MASK64
        v += ret * (r - 2 * v)
        self._norm(dif, v)
        return ret ^ 1

    def bools(self, n: int) -> int:
        v = 0
        for _ in range(n):
            v = (v << 1) | self.equi()
        return v

    def symbol(self, cdf: list[int], n: int) -> int:
        c = self.dif >> 48
        r = self.rng >> 8
        u = v = self.rng
        val = -1
        while True:
            val += 1
            u = v
            vv = (r * (cdf[val] >> 6)) >> 1
            v = vv + 4 * (n - val)
            if c >= v:
                break
        self._norm((self.dif - (v << 48)) & MASK64, u - v)
        if self.update:
            count = cdf[n]
            rate = 4 + (count >> 4) + (n > 2)
            for i in range(val):
                cdf[i] += (32768 - cdf[i]) >> rate
            for i in range(val, n):
                cdf[i] -= cdf[i] >> rate
            cdf[n] = count + (count < 32)
        return val

    def hi_tok(self, cdf: list[int]) -> int:
        br = self.symbol(cdf, 3)
        tok = 3 + br
        if br == 3:
            br = self.symbol(cdf, 3)
            tok = 6 + br
            if br == 3:
                br = self.symbol(cdf, 3)
                tok = 9 + br
                if br == 3:
                    tok = 12 + self.symbol(cdf, 3)
        return tok

    def golomb(self) -> int:
        length = 0
        while not self.equi() and length < 32:
            length += 1
        val = 1
        for _ in range(length):
            val = (val << 1) + self.equi()
        return val - 1


def _leb(data: bytes, i: int) -> tuple[int, int]:
    size = shift = 0
    for _ in range(8):
        if i >= len(data):
            _bad(INTEGRITY_FAIL, "truncated AV1 size")
        b = data[i]
        i += 1
        size |= (b & 0x7F) << shift
        if (b & 0x80) == 0:
            return size, i
        shift += 7
    _bad(INTEGRITY_FAIL, "AV1 size is not a LEB128")
    return 0, i


def _obus(data: bytes) -> list[tuple[int, bytes]]:
    i = 0
    out = []
    while i < len(data):
        h = data[i]
        i += 1
        if h & 0x80:
            _bad(INTEGRITY_FAIL, "AV1 OBU forbidden bit is set")
        typ = (h >> 3) & 0xF
        ext = (h >> 2) & 1
        has = (h >> 1) & 1
        if ext:
            if i >= len(data):
                _bad(INTEGRITY_FAIL, "truncated AV1 OBU extension")
            i += 1
        if not has:
            _bad(UNSUPPORTED, "an AV1 OBU without a size is not in this slice")
        size, i = _leb(data, i)
        if i + size > len(data):
            _bad(INTEGRITY_FAIL, "truncated AV1 OBU")
        out.append((typ, data[i:i + size]))
        i += size
    return out


class Frame:
    def __init__(self) -> None:
        self.w = self.h = 0
        self.ssx = self.ssy = 1
        self.mono = False
        self.base_q = 0
        self.ydc = self.udc = self.uac = 0
        self.qmatrix = False
        self.tx_select = False
        self.reduced_tx = False
        self.screen = False
        self.intrabc = False
        self.filter_intra = False
        self.lf = [0, 0, 0, 0]
        self.sharp = 0
        self.update = True
        self.lossless = False


def _seq(payload: bytes, fr: Frame) -> None:
    b = Bits(payload)
    profile = b.u(3)
    still = b.u(1)
    reduced = b.u(1)
    if profile != 0 or not still or not reduced:
        _bad(UNSUPPORTED, "AV1 sequence is not a profile-0 reduced still picture")
    if b.u(5) > 31:
        _bad(INTEGRITY_FAIL, "AV1 level is not a level")
    wb = b.u(4) + 1
    hb = b.u(4) + 1
    fr.w = b.u(wb) + 1
    fr.h = b.u(hb) + 1
    if fr.w > 64 or fr.h > 64:
        _bad(UNSUPPORTED, "AV1 frame is larger than 64 pixels in this slice")
    if b.u(1):
        _bad(UNSUPPORTED, "128x128 superblocks are not in this slice")
    fr.filter_intra = bool(b.u(1))
    b.u(1)  # intra edge filter; prediction uses the prepared edge either way
    if b.u(1) or b.u(1) or b.u(1):
        _bad(UNSUPPORTED, "superres, CDEF, or restoration is not in this slice")
    if b.u(1):
        _bad(UNSUPPORTED, "high bit depth is not in this slice")
    fr.mono = bool(b.u(1))
    if b.u(1):
        _bad(UNSUPPORTED, "a color description is not in this slice")
    if fr.mono:
        b.u(1)
        fr.ssx = fr.ssy = 1
    else:
        b.u(1)  # color range
        fr.ssx = fr.ssy = 1
        b.u(2)  # chroma sample position
        if b.u(1):
            _bad(UNSUPPORTED, "separate UV delta-q is not in this slice")
    if b.u(1):
        _bad(UNSUPPORTED, "film grain is not in this slice")
    b.trailing()


def _frame_header(payload: bytes, fr: Frame) -> bytes:
    b = Bits(payload)
    fr.update = not bool(b.u(1))
    fr.screen = bool(b.u(1))
    if fr.screen:
        b.u(1)  # force_integer_mv, then intra forces it on
    render_diff = b.u(1)
    if render_diff:
        _bad(UNSUPPORTED, "a distinct render size is not in this slice")
    fr.intrabc = bool(b.u(1)) if fr.screen else False
    if fr.intrabc:
        _bad(UNSUPPORTED, "intrabc is not in this slice")
    uniform = b.u(1)
    if not uniform:
        _bad(UNSUPPORTED, "non-uniform tiles are not in this slice")
    # one superblock column and row: the log2 loops read nothing
    fr.base_q = b.u(8)
    fr.ydc = b.su(7) if b.u(1) else 0
    if not fr.mono:
        fr.udc = b.su(7) if b.u(1) else 0
        fr.uac = b.su(7) if b.u(1) else 0
    fr.qmatrix = bool(b.u(1))
    if fr.qmatrix:
        _bad(UNSUPPORTED, "a quantisation matrix is not in this slice")
    if b.u(1):
        _bad(UNSUPPORTED, "segmentation is not in this slice")
    if fr.base_q > 0 and b.u(1):
        _bad(UNSUPPORTED, "delta-q is not in this slice")
    fr.lossless = fr.base_q == 0 and fr.ydc == 0 and fr.udc == 0 and fr.uac == 0
    if not fr.lossless:
        fr.lf[0] = b.u(6)
        fr.lf[1] = b.u(6)
        if not fr.mono and (fr.lf[0] or fr.lf[1]):
            fr.lf[2] = b.u(6)
            fr.lf[3] = b.u(6)
        fr.sharp = b.u(3)
        if b.u(1):
            # Deltas are parsed so the header stays aligned. Applying them is
            # the deblock step, which this slice does when the level is not zero.
            if b.u(1):
                for _ in range(8):
                    if b.u(1):
                        b.su(7)
                for _ in range(2):
                    if b.u(1):
                        b.su(7)
    fr.tx_select = False if fr.lossless else bool(b.u(1))
    fr.reduced_tx = bool(b.u(1))
    b.align()
    used = b.i >> 3
    return payload[used:]


def _qindex(base: int, delta: int) -> int:
    v = base + delta
    if v < 0:
        return 0
    if v > 255:
        return 255
    return v


def _dct4(c: list[int], stride: int) -> None:
    i0, i1 = c[0], c[stride]
    i2, i3 = c[2 * stride], c[3 * stride]
    t0 = ((i0 + i2) * 181 + 128) >> 8
    t1 = ((i0 - i2) * 181 + 128) >> 8
    t2 = ((i1 * 1567 - i3 * (3784 - 4096) + 2048) >> 12) - i3
    t3 = ((i1 * (3784 - 4096) + i3 * 1567 + 2048) >> 12) + i1
    c[0] = _clip16(t0 + t3)
    c[stride] = _clip16(t1 + t2)
    c[2 * stride] = _clip16(t1 - t2)
    c[3 * stride] = _clip16(t0 - t3)


def _dct8(c: list[int], stride: int) -> None:
    _dct4(c, stride << 1)
    i1, i3 = c[stride], c[3 * stride]
    i5, i7 = c[5 * stride], c[7 * stride]
    t4a = ((i1 * 799 - i7 * (4017 - 4096) + 2048) >> 12) - i7
    t5a = (i5 * 1703 - i3 * 1138 + 1024) >> 11
    t6a = (i5 * 1138 + i3 * 1703 + 1024) >> 11
    t7a = ((i1 * (4017 - 4096) + i7 * 799 + 2048) >> 12) + i1
    t4 = _clip16(t4a + t5a)
    t5a = _clip16(t4a - t5a)
    t7 = _clip16(t7a + t6a)
    t6a = _clip16(t7a - t6a)
    t5 = ((t6a - t5a) * 181 + 128) >> 8
    t6 = ((t6a + t5a) * 181 + 128) >> 8
    t0, t1 = c[0], c[2 * stride]
    t2, t3 = c[4 * stride], c[6 * stride]
    c[0] = _clip16(t0 + t7)
    c[stride] = _clip16(t1 + t6)
    c[2 * stride] = _clip16(t2 + t5)
    c[3 * stride] = _clip16(t3 + t4)
    c[4 * stride] = _clip16(t3 - t4)
    c[5 * stride] = _clip16(t2 - t5)
    c[6 * stride] = _clip16(t1 - t6)
    c[7 * stride] = _clip16(t0 - t7)


def _dct16(c: list[int], stride: int) -> None:
    _dct8(c, stride << 1)
    ins = [c[i * stride] for i in range(1, 16, 2)]
    i1, i3, i5, i7, i9, i11, i13, i15 = ins
    t8a = ((i1 * 401 - i15 * (4076 - 4096) + 2048) >> 12) - i15
    t9a = (i9 * 1583 - i7 * 1299 + 1024) >> 11
    t10a = ((i5 * 1931 - i11 * (3612 - 4096) + 2048) >> 12) - i11
    t11a = ((i13 * (3920 - 4096) - i3 * 1189 + 2048) >> 12) + i13
    t12a = ((i13 * 1189 + i3 * (3920 - 4096) + 2048) >> 12) + i3
    t13a = ((i5 * (3612 - 4096) + i11 * 1931 + 2048) >> 12) + i5
    t14a = (i9 * 1299 + i7 * 1583 + 1024) >> 11
    t15a = ((i1 * (4076 - 4096) + i15 * 401 + 2048) >> 12) + i1
    t8 = _clip16(t8a + t9a)
    t9 = _clip16(t8a - t9a)
    t10 = _clip16(t11a - t10a)
    t11 = _clip16(t11a + t10a)
    t12 = _clip16(t12a + t13a)
    t13 = _clip16(t12a - t13a)
    t14 = _clip16(t15a - t14a)
    t15 = _clip16(t15a + t14a)
    t9a = ((t14 * 1567 - t9 * (3784 - 4096) + 2048) >> 12) - t9
    t14a = ((t14 * (3784 - 4096) + t9 * 1567 + 2048) >> 12) + t14
    t10a = ((-(t13 * (3784 - 4096) + t10 * 1567) + 2048) >> 12) - t13
    t13a = ((t13 * 1567 - t10 * (3784 - 4096) + 2048) >> 12) - t10
    t8a = _clip16(t8 + t11)
    t9 = _clip16(t9a + t10a)
    t10 = _clip16(t9a - t10a)
    t11a = _clip16(t8 - t11)
    t12a = _clip16(t15 - t12)
    t13 = _clip16(t14a - t13a)
    t14 = _clip16(t14a + t13a)
    t15a = _clip16(t15 + t12)
    t10a = ((t13 - t10) * 181 + 128) >> 8
    t13a = ((t13 + t10) * 181 + 128) >> 8
    t11 = ((t12a - t11a) * 181 + 128) >> 8
    t12 = ((t12a + t11a) * 181 + 128) >> 8
    ev = [c[i * stride] for i in range(0, 16, 2)]
    c[0] = _clip16(ev[0] + t15a)
    c[stride] = _clip16(ev[1] + t14)
    c[2 * stride] = _clip16(ev[2] + t13a)
    c[3 * stride] = _clip16(ev[3] + t12)
    c[4 * stride] = _clip16(ev[4] + t11)
    c[5 * stride] = _clip16(ev[5] + t10a)
    c[6 * stride] = _clip16(ev[6] + t9)
    c[7 * stride] = _clip16(ev[7] + t8a)
    c[8 * stride] = _clip16(ev[7] - t8a)
    c[9 * stride] = _clip16(ev[6] - t9)
    c[10 * stride] = _clip16(ev[5] - t10a)
    c[11 * stride] = _clip16(ev[4] - t11)
    c[12 * stride] = _clip16(ev[3] - t12)
    c[13 * stride] = _clip16(ev[2] - t13a)
    c[14 * stride] = _clip16(ev[1] - t14)
    c[15 * stride] = _clip16(ev[0] - t15a)


def _adst4(c: list[int], stride: int) -> None:
    ins = [c[i * stride] for i in range(4)]
    i0, i1, i2, i3 = ins
    o0 = ((1321 * i0 + (3803 - 4096) * i2 + (2482 - 4096) * i3 + (3344 - 4096) * i1 + 2048) >> 12) + i2 + i3 + i1
    o1 = (((2482 - 4096) * i0 - 1321 * i2 - (3803 - 4096) * i3 + (3344 - 4096) * i1 + 2048) >> 12) + i0 - i3 + i1
    o2 = (209 * (i0 - i2 + i3) + 128) >> 8
    o3 = (((3803 - 4096) * i0 + (2482 - 4096) * i2 - 1321 * i3 - (3344 - 4096) * i1 + 2048) >> 12) + i0 + i2 - i1
    c[0], c[stride], c[2 * stride], c[3 * stride] = o0, o1, o2, o3


def _adst8(c: list[int], stride: int) -> None:
    ins = [c[i * stride] for i in range(8)]
    i0, i1, i2, i3, i4, i5, i6, i7 = ins
    t0a = (((4076 - 4096) * i7 + 401 * i0 + 2048) >> 12) + i7
    t1a = ((401 * i7 - (4076 - 4096) * i0 + 2048) >> 12) - i0
    t2a = (((3612 - 4096) * i5 + 1931 * i2 + 2048) >> 12) + i5
    t3a = ((1931 * i5 - (3612 - 4096) * i2 + 2048) >> 12) - i2
    t4a = (1299 * i3 + 1583 * i4 + 1024) >> 11
    t5a = (1583 * i3 - 1299 * i4 + 1024) >> 11
    t6a = ((1189 * i1 + (3920 - 4096) * i6 + 2048) >> 12) + i6
    t7a = (((3920 - 4096) * i1 - 1189 * i6 + 2048) >> 12) + i1
    t0 = _clip16(t0a + t4a)
    t1 = _clip16(t1a + t5a)
    t2 = _clip16(t2a + t6a)
    t3 = _clip16(t3a + t7a)
    t4 = _clip16(t0a - t4a)
    t5 = _clip16(t1a - t5a)
    t6 = _clip16(t2a - t6a)
    t7 = _clip16(t3a - t7a)
    t4a = (((3784 - 4096) * t4 + 1567 * t5 + 2048) >> 12) + t4
    t5a = ((1567 * t4 - (3784 - 4096) * t5 + 2048) >> 12) - t5
    t6a = (((3784 - 4096) * t7 - 1567 * t6 + 2048) >> 12) + t7
    t7a = ((1567 * t7 + (3784 - 4096) * t6 + 2048) >> 12) + t6
    out = [0] * 8
    out[0] = _clip16(t0 + t2)
    out[7] = -_clip16(t1 + t3)
    t2 = _clip16(t0 - t2)
    t3 = _clip16(t1 - t3)
    out[1] = -_clip16(t4a + t6a)
    out[6] = _clip16(t5a + t7a)
    t6 = _clip16(t4a - t6a)
    t7 = _clip16(t5a - t7a)
    out[3] = -(((t2 + t3) * 181 + 128) >> 8)
    out[4] = ((t2 - t3) * 181 + 128) >> 8
    out[2] = ((t6 + t7) * 181 + 128) >> 8
    out[5] = -(((t6 - t7) * 181 + 128) >> 8)
    for i, v in enumerate(out):
        c[i * stride] = v


def _ident(c: list[int], stride: int, n: int) -> None:
    if n == 4:
        for i in range(4):
            v = c[stride * i]
            c[stride * i] = v + ((v * 1697 + 2048) >> 12)
    elif n == 8:
        for i in range(8):
            c[stride * i] *= 2
    elif n == 16:
        for i in range(16):
            v = c[stride * i]
            c[stride * i] = 2 * v + ((v * 1697 + 1024) >> 11)
    else:
        _bad(UNSUPPORTED, "that identity transform is not in this slice")


def _adst16(c: list[int], stride: int) -> None:
    ins = [c[i * stride] for i in range(16)]
    (i0, i1, i2, i3, i4, i5, i6, i7, i8, i9, i10, i11, i12, i13, i14, i15) = ins
    t0 = ((i15 * (4091 - 4096) + i0 * 201 + 2048) >> 12) + i15
    t1 = ((i15 * 201 - i0 * (4091 - 4096) + 2048) >> 12) - i0
    t2 = ((i13 * (3973 - 4096) + i2 * 995 + 2048) >> 12) + i13
    t3 = ((i13 * 995 - i2 * (3973 - 4096) + 2048) >> 12) - i2
    t4 = ((i11 * (3703 - 4096) + i4 * 1751 + 2048) >> 12) + i11
    t5 = ((i11 * 1751 - i4 * (3703 - 4096) + 2048) >> 12) - i4
    t6 = (i9 * 1645 + i6 * 1220 + 1024) >> 11
    t7 = (i9 * 1220 - i6 * 1645 + 1024) >> 11
    t8 = ((i7 * 2751 + i8 * (3035 - 4096) + 2048) >> 12) + i8
    t9 = ((i7 * (3035 - 4096) - i8 * 2751 + 2048) >> 12) + i7
    t10 = ((i5 * 2106 + i10 * (3513 - 4096) + 2048) >> 12) + i10
    t11 = ((i5 * (3513 - 4096) - i10 * 2106 + 2048) >> 12) + i5
    t12 = ((i3 * 1380 + i12 * (3857 - 4096) + 2048) >> 12) + i12
    t13 = ((i3 * (3857 - 4096) - i12 * 1380 + 2048) >> 12) + i3
    t14 = ((i1 * 601 + i14 * (4052 - 4096) + 2048) >> 12) + i14
    t15 = ((i1 * (4052 - 4096) - i14 * 601 + 2048) >> 12) + i1
    t0a, t1a = _clip16(t0 + t8), _clip16(t1 + t9)
    t2a, t3a = _clip16(t2 + t10), _clip16(t3 + t11)
    t4a, t5a = _clip16(t4 + t12), _clip16(t5 + t13)
    t6a, t7a = _clip16(t6 + t14), _clip16(t7 + t15)
    t8a, t9a = _clip16(t0 - t8), _clip16(t1 - t9)
    t10a, t11a = _clip16(t2 - t10), _clip16(t3 - t11)
    t12a, t13a = _clip16(t4 - t12), _clip16(t5 - t13)
    t14a, t15a = _clip16(t6 - t14), _clip16(t7 - t15)
    t8 = ((t8a * (4017 - 4096) + t9a * 799 + 2048) >> 12) + t8a
    t9 = ((t8a * 799 - t9a * (4017 - 4096) + 2048) >> 12) - t9a
    t10 = ((t10a * 2276 + t11a * (3406 - 4096) + 2048) >> 12) + t11a
    t11 = ((t10a * (3406 - 4096) - t11a * 2276 + 2048) >> 12) + t10a
    t12 = ((t13a * (4017 - 4096) - t12a * 799 + 2048) >> 12) + t13a
    t13 = ((t13a * 799 + t12a * (4017 - 4096) + 2048) >> 12) + t12a
    t14 = ((t15a * 2276 - t14a * (3406 - 4096) + 2048) >> 12) - t14a
    t15 = ((t15a * (3406 - 4096) + t14a * 2276 + 2048) >> 12) + t15a
    t0, t1 = _clip16(t0a + t4a), _clip16(t1a + t5a)
    t2, t3 = _clip16(t2a + t6a), _clip16(t3a + t7a)
    t4, t5 = _clip16(t0a - t4a), _clip16(t1a - t5a)
    t6, t7 = _clip16(t2a - t6a), _clip16(t3a - t7a)
    t8a, t9a = _clip16(t8 + t12), _clip16(t9 + t13)
    t10a, t11a = _clip16(t10 + t14), _clip16(t11 + t15)
    t12a, t13a = _clip16(t8 - t12), _clip16(t9 - t13)
    t14a, t15a = _clip16(t10 - t14), _clip16(t11 - t15)
    t4a = ((t4 * (3784 - 4096) + t5 * 1567 + 2048) >> 12) + t4
    t5a = ((t4 * 1567 - t5 * (3784 - 4096) + 2048) >> 12) - t5
    t6a = ((t7 * (3784 - 4096) - t6 * 1567 + 2048) >> 12) + t7
    t7a = ((t7 * 1567 + t6 * (3784 - 4096) + 2048) >> 12) + t6
    t12 = ((t12a * (3784 - 4096) + t13a * 1567 + 2048) >> 12) + t12a
    t13 = ((t12a * 1567 - t13a * (3784 - 4096) + 2048) >> 12) - t13a
    t14 = ((t15a * (3784 - 4096) - t14a * 1567 + 2048) >> 12) + t15a
    t15 = ((t15a * 1567 + t14a * (3784 - 4096) + 2048) >> 12) + t14a
    out = [0] * 16
    out[0] = _clip16(t0 + t2)
    out[15] = -_clip16(t1 + t3)
    t2a, t3a = _clip16(t0 - t2), _clip16(t1 - t3)
    out[3] = -_clip16(t4a + t6a)
    out[12] = _clip16(t5a + t7a)
    t6, t7 = _clip16(t4a - t6a), _clip16(t5a - t7a)
    out[1] = -_clip16(t8a + t10a)
    out[14] = _clip16(t9a + t11a)
    t10, t11 = _clip16(t8a - t10a), _clip16(t9a - t11a)
    out[2] = _clip16(t12 + t14)
    out[13] = -_clip16(t13 + t15)
    t14a, t15a = _clip16(t12 - t14), _clip16(t13 - t15)
    out[7] = -(((t2a + t3a) * 181 + 128) >> 8)
    out[8] = ((t2a - t3a) * 181 + 128) >> 8
    out[4] = ((t6 + t7) * 181 + 128) >> 8
    out[11] = -(((t6 - t7) * 181 + 128) >> 8)
    out[6] = ((t10 + t11) * 181 + 128) >> 8
    out[9] = -(((t10 - t11) * 181 + 128) >> 8)
    out[5] = -(((t14a + t15a) * 181 + 128) >> 8)
    out[10] = ((t14a - t15a) * 181 + 128) >> 8
    for i, v in enumerate(out):
        c[i * stride] = v


# first function runs along each row, second down each column.
# ADST_DCT is ADST vertically, so the column pass is ADST.
_TX1D = {
    (DCT_DCT, 0): (_dct4, _dct4),
    (ADST_DCT, 0): (_dct4, _adst4),
    (DCT_ADST, 0): (_adst4, _dct4),
    (ADST_ADST, 0): (_adst4, _adst4),
    (IDTX, 0): (lambda c, s: _ident(c, s, 4), lambda c, s: _ident(c, s, 4)),
    (DCT_DCT, 1): (_dct8, _dct8),
    (ADST_DCT, 1): (_dct8, _adst8),
    (DCT_ADST, 1): (_adst8, _dct8),
    (ADST_ADST, 1): (_adst8, _adst8),
    (IDTX, 1): (lambda c, s: _ident(c, s, 8), lambda c, s: _ident(c, s, 8)),
    (H_DCT, 0): (_dct4, lambda c, s: _ident(c, s, 4)),
    (V_DCT, 0): (lambda c, s: _ident(c, s, 4), _dct4),
    (H_DCT, 1): (_dct8, lambda c, s: _ident(c, s, 8)),
    (V_DCT, 1): (lambda c, s: _ident(c, s, 8), _dct8),
    (H_DCT, 2): (_dct16, lambda c, s: _ident(c, s, 16)),
    (V_DCT, 2): (lambda c, s: _ident(c, s, 16), _dct16),
    (DCT_DCT, 2): (_dct16, _dct16),
    (ADST_DCT, 2): (_dct16, _adst16),
    (DCT_ADST, 2): (_adst16, _dct16),
    (ADST_ADST, 2): (_adst16, _adst16),
    (IDTX, 2): (lambda c, s: _ident(c, s, 16), lambda c, s: _ident(c, s, 16)),
}
_SHIFT = {0: 0, 1: 1, 2: 2}
_PX = {0: 4, 1: 8, 2: 16}


def _tx_class(txtp: int) -> int:
    """0 is 2D, 1 is horizontal (H_DCT), 2 is vertical (V_DCT)."""
    if txtp == H_DCT:
        return 1
    if txtp == V_DCT:
        return 2
    return 0


def _wht4(c: list[int], stride: int) -> None:
    i0, i1 = c[0], c[stride]
    i2, i3 = c[2 * stride], c[3 * stride]
    t0 = i0 + i1
    t2 = i2 - i3
    t4 = (t0 - t2) >> 1
    t3 = t4 - i3
    t1 = t4 - i1
    c[0] = t0 - t3
    c[stride] = t3
    c[2 * stride] = t1
    c[3 * stride] = t2 + t1


_TX1D[(WHT_WHT, 0)] = (_wht4, _wht4)


def _wht_add(dst: bytearray, stride: int, coef: list[int]) -> None:
    """Lossless 4×4. The stored coefficient is four times the Walsh value."""
    cs = [c >> 2 for c in coef]
    tmp = [0] * 16
    for y in range(4):
        row = [cs[y + x * 4] for x in range(4)]
        _wht4(row, 1)
        for x in range(4):
            tmp[y * 4 + x] = row[x]
    for x in range(4):
        col = [tmp[y * 4 + x] for y in range(4)]
        _wht4(col, 1)
        for y in range(4):
            dst[y * stride + x] = _clip8(dst[y * stride + x] + col[y])


def _itx_add(dst: bytearray, stride: int, coef: list[int], eob: int, tx: int, txtp: int) -> None:
    if txtp == WHT_WHT:
        if tx != TX_4:
            _bad(UNSUPPORTED, "lossless Walsh-Hadamard above 4x4 is not in this slice")
        _wht_add(dst, stride, coef)
        return
    n = _PX[tx]
    pair = _TX1D.get((txtp, tx))
    if pair is None:
        _bad(UNSUPPORTED, f"transform type {txtp} at size {n} is not in this slice")
    first, second = pair
    shift = _SHIFT[tx]
    rnd = (1 << shift) >> 1
    if eob < 1 and txtp == DCT_DCT:
        dc = coef[0]
        dc = (dc * 181 + 128) >> 8
        dc = (dc + rnd) >> shift
        dc = (dc * 181 + 128 + 2048) >> 12
        for y in range(n):
            row = y * stride
            for x in range(n):
                dst[row + x] = _clip8(dst[row + x] + dc)
        return
    tmp = [0] * (n * n)
    for y in range(n):
        row = [coef[y + x * n] for x in range(n)]
        first(row, 1)
        for x in range(n):
            tmp[y * n + x] = _clip16((row[x] + rnd) >> shift)
    for x in range(n):
        col = [tmp[y * n + x] for y in range(n)]
        second(col, 1)
        for y in range(n):
            tmp[y * n + x] = col[y]
    for y in range(n):
        for x in range(n):
            dst[y * stride + x] = _clip8(dst[y * stride + x] + ((tmp[y * n + x] + 8) >> 4))


class Decoder:
    def __init__(self, fr: Frame, tile: bytes) -> None:
        self.fr = fr
        self.ms = Msac(tile, fr.update)
        self.bw4 = (fr.w + 3) >> 2
        self.bh4 = (fr.h + 3) >> 2
        self.y = bytearray([0]) * (fr.w * fr.h)
        cw = (fr.w + fr.ssx) >> fr.ssx
        ch = (fr.h + fr.ssy) >> fr.ssy
        self.cw, self.ch = cw, ch
        self.u = bytearray([128]) * (cw * ch)
        self.v = bytearray([128]) * (cw * ch)
        self.mode = copy.deepcopy(MODE)
        # The initializer lists block sizes out of enum order. BS_16x16 is 12.
        self.mode["use_filter_intra"][12] = [32768 - 12408, 0]
        self.mode["use_filter_intra"][17] = [32768 - 7866, 0]
        self.mode["use_filter_intra"][21] = [32768 - 4621, 0]
        q = fr.base_q
        qcat = 0 if q <= 20 else 1 if q <= 60 else 2 if q <= 120 else 3
        self.coef = copy.deepcopy(COEF[qcat])
        self.kfym = copy.deepcopy(KFYM)
        self.q_y = [DQ8[_qindex(fr.base_q, fr.ydc)][0], DQ8[fr.base_q][1]]
        uq = _qindex(fr.base_q, fr.udc)
        self.q_u = [DQ8[uq][0], DQ8[_qindex(fr.base_q, fr.uac)][1]]
        self.q_v = self.q_u
        self.above_mode = [DC] * 64
        self.left_mode = [DC] * 64
        self.above_skip = [0] * 64
        self.left_skip = [0] * 64
        self.above_tx = [-1] * 64
        self.left_tx = [-1] * 64
        self.above_coef = [0x40] * 64
        self.left_coef = [0x40] * 64
        self.above_ccoef = [0x40] * 64
        self.left_ccoef = [0x40] * 64

    def pic(self) -> bytes:
        if self.fr.mono:
            return bytes(self.y)
        return bytes(self.y + self.u + self.v)

    def decode(self) -> None:
        self._sb(BL_64, 0, 0)
        if self.ms.starved and self.ms.pos > self.ms.end:
            _bad(INTEGRITY_FAIL, "truncated AV1 tile")

    def _sb(self, bl: int, bx: int, by: int) -> None:
        if bx >= self.bw4 or by >= self.bh4:
            return
        hsz = 16 >> bl
        have_h = self.bw4 > bx + hsz
        have_v = self.bh4 > by + hsz
        if not have_h and not have_v:
            self._sb(bl + 1, bx, by)
            return
        if have_h and have_v:
            bp = self.ms.symbol(self.mode["partition"][bl][0], PART_N[bl])
        elif have_v:
            flag = self.ms.symbol(self.mode["partition"][bl][0], 2)
            bp = SPLIT if flag else H
        elif have_h:
            flag = self.ms.symbol(self.mode["partition"][bl][0], 2)
            bp = SPLIT if flag else V
        else:
            bp = SPLIT
        if bp == NONE:
            self._block(bx, by, hsz * 2)
        elif bp == SPLIT:
            h = hsz >> 1
            self._sb(bl + 1, bx, by)
            self._sb(bl + 1, bx + h, by)
            self._sb(bl + 1, bx, by + h)
            self._sb(bl + 1, bx + h, by + h)
        elif bp == H:
            self._block(bx, by, hsz)
            self._block(bx, by + (hsz >> 1), hsz)
        elif bp == V:
            self._block(bx, by, hsz)
            self._block(bx + (hsz >> 1), by, hsz)
        else:
            _bad(UNSUPPORTED, f"partition {bp} is not in this slice")

    def _block(self, bx: int, by: int, bw4: int) -> None:
        # square blocks only in this slice
        if bx >= self.bw4 or by >= self.bh4:
            return
        skip = self.ms.adapt_bool(self.mode["skip"][self.above_skip[bx] + self.left_skip[by]])
        ac = INTRA_CTX[self.above_mode[bx]]
        lc = INTRA_CTX[self.left_mode[by]]
        ymode = self.ms.symbol(self.kfym[ac][lc], 12)
        if bw4 >= 2 and VERT <= ymode <= 8:
            self.ms.symbol(self.mode["angle_delta"][ymode - VERT], 7)
        has_c = not self.fr.mono
        uv = DC
        if has_c:
            uv = self.ms.symbol(self.mode["uv_mode"][1][ymode], 13)
            if uv == CFL:
                _bad(UNSUPPORTED, "chroma-from-luma is not in this slice")
            if bw4 >= 2 and VERT <= uv <= 8:
                self.ms.symbol(self.mode["angle_delta"][uv - VERT], 7)
        if self.fr.screen and ymode == DC and bw4 <= 16:
            sz = 2 if bw4 == 4 else 1
            if self.ms.adapt_bool(self.mode["pal_y"][sz][0]):
                _bad(UNSUPPORTED, "palette blocks are not in this slice")
        if ymode == DC and self.fr.filter_intra and bw4 <= 8:
            bs = {4: 12, 2: 17, 1: 21}[bw4]
            fib = self.ms.adapt_bool(self.mode["use_filter_intra"][bs])
            if fib:
                ymode = 100 + self.ms.symbol(self.mode["filter_intra"], 4)
        tx = {1: TX_4, 2: TX_8, 4: TX_16}[bw4]
        if self.fr.lossless:
            tx = TX_4
        elif self.fr.tx_select and tx > TX_4:
            depth = self.ms.symbol(self.mode["txsz"][tx - 1][0], min(tx, 2))
            while depth:
                tx -= 1
                depth -= 1
        self._paint(0, bx * 4, by * 4, bw4 * 4, ymode, tx, None, skip)
        if has_c:
            cw = bw4 * 4 >> self.fr.ssx
            ctx = TX_4 if self.fr.lossless else TX_8 if cw == 8 else TX_4
            self._paint(1, (bx * 4) >> self.fr.ssx, (by * 4) >> self.fr.ssy, cw, uv, ctx, UV_TXTP[uv], skip)
            self._paint(2, (bx * 4) >> self.fr.ssx, (by * 4) >> self.fr.ssy, cw, uv, ctx, UV_TXTP[uv], skip)
        for i in range(bw4):
            self.above_mode[bx + i] = ymode
            self.above_skip[bx + i] = skip
            self.left_mode[by + i] = ymode
            self.left_skip[by + i] = skip

    def _plane(self, plane: int) -> tuple[bytearray, int]:
        if plane == 0:
            return self.y, self.fr.w
        if plane == 1:
            return self.u, self.cw
        return self.v, self.cw

    def _paint(self, plane: int, x: int, y: int, n: int, mode: int, tx: int, txtp: int, skip: int) -> None:
        dst, stride = self._plane(plane)
        tw = _PX[tx]
        for yy in range(0, n, tw):
            for xx in range(0, n, tw):
                self._one(dst, stride, x + xx, y + yy, mode, tx, txtp, skip, plane)

    def _one(self, dst: bytearray, stride: int, x: int, y: int, mode: int, tx: int,
             txtp: int | None, block_skip: int, plane: int) -> None:
        n = _PX[tx]
        block = bytearray(n * n)
        self._predict(block, n, dst, stride, x, y, mode)
        eob = -1
        coef = [0] * (n * n)
        if not block_skip:
            eob, coef, txtp = self._coefs(tx, txtp, mode, plane, x, y)
        if eob >= 0 and txtp is not None:
            _itx_add(block, n, coef, eob, tx, txtp)
        for row in range(n):
            if y + row >= (self.fr.h if plane == 0 else self.ch):
                break
            for col in range(n):
                if x + col >= stride:
                    break
                dst[(y + row) * stride + (x + col)] = block[row * n + col]

    def _predict(self, block: bytearray, n: int, dst: bytearray, stride: int,
                 x: int, y: int, mode: int) -> None:
        if mode == DC and (x == 0 or y == 0):
            if x == 0 and y == 0:
                val = 128
            elif y == 0:
                s = n >> 1
                for i in range(n):
                    s += dst[(x - 1) + i * 0] if False else dst[y * stride + (x - 1)]
                    # left column
                s = n >> 1
                for i in range(n):
                    s += dst[(y + i) * stride + (x - 1)]
                val = s >> (n.bit_length() - 1)
            else:
                s = n >> 1
                for i in range(n):
                    s += dst[(y - 1) * stride + (x + i)]
                val = s >> (n.bit_length() - 1)
            for i in range(n * n):
                block[i] = val & 255
            return
        if mode == DC:
            s = n
            for i in range(n):
                s += dst[(y - 1) * stride + x + i]
                s += dst[(y + i) * stride + (x - 1)]
            val = s >> (n.bit_length())
            # n+n is 2n, ctz(2n) = ctz(n)+1 = bit_length(n). s includes (n) which is (w+h)>>1 = n.
            for i in range(n * n):
                block[i] = val & 255
            return
        if mode == VERT:
            for row in range(n):
                for col in range(n):
                    src = dst[(y - 1) * stride + x + col] if y else 127
                    block[row * n + col] = src
            return
        if mode == HOR:
            for row in range(n):
                src = dst[(y + row) * stride + (x - 1)] if x else 129
                for col in range(n):
                    block[row * n + col] = src
            return
        if mode >= 100:
            self._filter_intra(block, n, dst, stride, x, y, mode - 100)
            return
        _bad(UNSUPPORTED, f"intra mode {mode} is not in this slice")

    def _filter_intra(self, block: bytearray, n: int, dst: bytearray, stride: int,
                      x: int, y: int, filt: int) -> None:
        taps = FILTER_TAPS[filt]
        height = len(dst) // stride
        top = []
        left = []
        for i in range(n * 2):
            if y > 0 and x + i < stride:
                pix = dst[(y - 1) * stride + min(x + i, stride - 1)]
            elif y > 0:
                pix = top[-1]
            elif x:
                pix = dst[y * stride + (x - 1)]
            else:
                pix = 127
            top.append(pix)
            if x > 0 and y + i < height:
                lp = dst[(y + i) * stride + (x - 1)]
            elif x > 0:
                lp = left[-1]
            elif y:
                lp = dst[(y - 1) * stride + x]
            else:
                lp = 129
            left.append(lp)
        if x and y:
            corner = dst[(y - 1) * stride + (x - 1)]
        elif x:
            corner = left[0]
        elif y:
            corner = top[0]
        else:
            corner = 128
        center = n * 2
        edge = [128] * (n * 4 + 4)
        edge[center] = corner
        for i in range(n * 2):
            edge[center + 1 + i] = top[i]
            edge[center - 1 - i] = left[i]
        for yy in range(0, n, 2):
            top_base = center + 1
            for xx in range(0, n, 4):
                if yy == 0:
                    tops = [edge[top_base + xx + k] for k in range(4)]
                else:
                    tops = [block[(yy - 1) * n + xx + k] for k in range(4)]
                if xx == 0:
                    p0 = edge[center - yy]
                    p5 = edge[center - yy - 1]
                    p6 = edge[center - yy - 2]
                else:
                    if yy == 0:
                        p0 = edge[top_base + xx - 1]
                    else:
                        p0 = block[(yy - 1) * n + xx - 1]
                    p5 = block[yy * n + xx - 1]
                    p6 = block[(yy + 1) * n + xx - 1]
                p1, p2, p3, p4 = tops
                for row in range(2):
                    for col in range(4):
                        f = taps[row * 4 + col]
                        acc = (f[0] * p0 + f[1] * p1 + f[2] * p2 + f[3] * p3
                               + f[4] * p4 + f[5] * p5 + f[6] * p6)
                        block[(yy + row) * n + xx + col] = _clip8((acc + 8) >> 4)
            # The next pair's above-row is the second row just written, except
            # the x==0 sample still comes from the original left edge via `edge`.

    def _dc_sign_ctx(self, tx: int, x4: int, y4: int, plane: int) -> int:
        above = self.above_ccoef if plane else self.above_coef
        left = self.left_ccoef if plane else self.left_coef
        mask = 0xC0C0C0C0

        def load(arr: list[int], i: int, n: int) -> int:
            v = 0
            for k in range(n):
                v |= (arr[i + k] & 0xFF) << (8 * k)
            return v

        if tx == TX_4:
            s = (above[x4] >> 6) + (left[y4] >> 6) - 2
        elif tx == TX_8:
            t = (load(above, x4, 2) & mask) + (load(left, y4, 2) & mask)
            t = (t * 0x04040404) & 0xFFFFFFFF
            s = (t >> 24) - 4
        else:
            t = ((load(above, x4, 4) & mask) >> 6) + ((load(left, y4, 4) & mask) >> 6)
            t = (t * 0x01010101) & 0xFFFFFFFF
            s = (t >> 24) - 8
        return (s != 0) + (s > 0)

    def _skip_ctx(self, tx: int, plane: int, x4: int, y4: int) -> int:
        if plane:
            return 7
        n4 = _PX[tx] // 4

        def pack(arr: list[int], i: int) -> int:
            v = 0
            for k in range(n4):
                v |= arr[i + k] << (8 * k)
            return v

        la, ll = pack(self.above_coef, x4), pack(self.left_coef, y4)
        if tx >= TX_8:
            la |= la >> 8
            ll |= ll >> 8
        if tx >= TX_16:
            la |= la >> 16
            ll |= ll >> 16
        la = min(la & 0x3F, 4)
        ll = min(ll & 0x3F, 4)
        return SKIP_CTX[la][ll]

    def _coefs(self, tx: int, txtp: int | None, mode: int, plane: int,
               x: int, y: int) -> tuple[int, list[int], int]:
        n = _PX[tx]
        ctx = tx  # TxfmInfo.ctx equals the square enum
        sctx = self._skip_ctx(tx, plane, x >> 2, y >> 2)
        cdf = self.coef["skip"][ctx][sctx]
        all_skip = self.ms.adapt_bool(cdf)
        coef = [0] * (n * n)
        if all_skip:
            n4 = n // 4
            x4, y4 = x >> 2, y >> 2
            above = self.above_ccoef if plane else self.above_coef
            left = self.left_ccoef if plane else self.left_coef
            for i in range(n4):
                above[x4 + i] = 0x40
                left[y4 + i] = 0x40
            return -1, coef, DCT_DCT
        if self.fr.lossless:
            txtp = WHT_WHT
        elif plane == 0:
            ymode = FILTER_TO_Y[mode - 100] if mode >= 100 else mode
            if self.fr.reduced_tx or tx == TX_16:
                idx = self.ms.symbol(self.mode["txtp_intra2"][tx][ymode], 4)
                txtp = TX_TYPES_INTRA2[idx]
            else:
                idx = self.ms.symbol(self.mode["txtp_intra1"][tx][ymode], 6)
                txtp = TX_TYPES_INTRA1[idx]
        assert txtp is not None
        if (txtp, tx) not in _TX1D:
            _bad(UNSUPPORTED, f"transform {txtp} at tx {tx} is not in this slice")
        dq_shift = max(0, ctx - 2)
        # H_DCT and V_DCT are 1D classes: a different eob table, no zigzag,
        # and the low-token context walks one axis.
        tx_class = _tx_class(txtp)
        is_1d = tx_class != 0
        tx2d = tx + tx
        eob_name = {0: "eob_bin_16", 2: "eob_bin_64", 4: "eob_bin_256"}[tx2d]
        ch = 1 if plane else 0
        # layout: [chroma][is_1d][probs]. n_symbols is 4 + tx2dszctx.
        eob_bin = self.ms.symbol(self.coef[eob_name][ch][1 if is_1d else 0], 4 + tx2d)
        if eob_bin > 1:
            hi = self.ms.adapt_bool(self.coef["eob_hi_bit"][ctx][ch][eob_bin])
            eob = ((hi | 2) << (eob_bin - 2)) | self.ms.bools(eob_bin - 2)
        else:
            eob = eob_bin
        scan = SCAN[tx]
        dq = self.q_y if plane == 0 else self.q_u if plane == 1 else self.q_v
        cf_max = 32767
        bit_shift = tx + 2
        mask = n - 1
        stride_l = 16 if is_1d else n
        levels = [0] * (stride_l * (n + 2))
        acs: list[tuple[int, int, int]] = []

        def pos(i: int) -> tuple[int, int, int]:
            if tx_class == 0:
                rc = scan[i]
                return rc // n, rc % n, rc
            x = i & mask
            y = i >> bit_shift
            if tx_class == 1:
                return x, y, i
            return x, y, (x << bit_shift) | y

        def store(lx: int, ly: int, tok: int) -> None:
            if tok >= 3:
                levels[lx * stride_l + ly] = tok + (3 << 6)
            else:
                levels[lx * stride_l + ly] = tok * 0x41

        def lo_ctx(lx: int, ly: int) -> tuple[int, int]:
            def at(dx: int, dy: int) -> int:
                return levels[(lx + dx) * stride_l + (ly + dy)]
            if tx_class == 0:
                mag = at(0, 1) + at(1, 0) + at(1, 1)
                hi = mag
                mag += at(0, 2) + at(2, 0)
                off = LO_CTX[0][min(ly, 4)][min(lx, 4)]
            else:
                mag = at(0, 1) + at(1, 0) + at(0, 2)
                hi = mag
                mag += at(0, 3) + at(0, 4)
                off = 26 + (10 if ly > 1 else ly * 5)
            return off + (4 if mag > 512 else (mag + 64) >> 7), hi

        def hi_base(lx: int, ly: int) -> int:
            if tx_class == 0:
                return 14 if (lx or ly) > 1 else 7
            return 14 if ly else 7

        dc_tok = 0
        if eob:
            sw = sh = min(n // 4, 8)
            lx, ly, rc = pos(eob)
            eob_ctx = 1 + (eob > sw * sh * 2) + (eob > sw * sh * 4)
            et = self.ms.symbol(self.coef["eob_base_tok"][ctx][ch][eob_ctx], 2)
            tok = et + 1
            if et == 2:
                tok = self.ms.hi_tok(self.coef["br_tok"][min(ctx, 3)][ch][hi_base(lx, ly)])
            store(lx, ly, tok)
            acs.append((eob, rc, tok))
            for i in range(eob - 1, 0, -1):
                lx, ly, rc = pos(i)
                cctx, mag = lo_ctx(lx, ly)
                tok = self.ms.symbol(self.coef["base_tok"][ctx][ch][cctx], 3)
                if tok == 3:
                    mag &= 63
                    hctx = hi_base(lx, ly) + (6 if mag > 12 else (mag + 1) >> 1)
                    tok = self.ms.hi_tok(self.coef["br_tok"][min(ctx, 3)][ch][hctx])
                store(lx, ly, tok)
                if tok:
                    acs.append((i, rc, tok))
            if tx_class == 0:
                dc_ctx = 0
                dc_mag = 0
            else:
                dc_ctx, dc_mag = lo_ctx(0, 0)
            dc_tok = self.ms.symbol(self.coef["base_tok"][ctx][ch][dc_ctx], 3)
            if dc_tok == 3:
                if tx_class == 0:
                    dc_mag = levels[1] + levels[stride_l] + levels[stride_l + 1]
                dc_mag &= 63
                hctx = 6 if dc_mag > 12 else (dc_mag + 1) >> 1
                dc_tok = self.ms.hi_tok(self.coef["br_tok"][min(ctx, 3)][ch][hctx])
        else:
            et = self.ms.symbol(self.coef["eob_base_tok"][ctx][ch][0], 2)
            dc_tok = et + 1
            if et == 2:
                dc_tok = self.ms.hi_tok(self.coef["br_tok"][min(ctx, 3)][ch][0])
        dc_sign = 0
        cul = 0
        if dc_tok:
            sc = self._dc_sign_ctx(tx, x >> 2, y >> 2, plane)
            dc_sign = self.ms.adapt_bool(self.coef["dc_sign"][ch][sc])
            tok = dc_tok
            if tok == 15:
                tok = (self.ms.golomb() + 15) & 0xFFFFF
            cul += tok
            dqv = ((dq[0] * tok) & 0xFFFFFF) >> dq_shift
            coef[0] = -min(dqv, cf_max + dc_sign) if dc_sign else min(dqv, cf_max)
        for _i, rc, tok in sorted(acs):
            sign = self.ms.equi()
            if tok >= 15:
                tok = (self.ms.golomb() + 15) & 0xFFFFF
            dqv = ((dq[1] * tok) & 0xFFFFFF) >> dq_shift
            dqv = min(dqv, cf_max + sign)
            coef[rc] = -dqv if sign else dqv
            cul += tok
        if dc_tok == 0:
            sign_level = 64
        else:
            sign_level = (dc_sign - 1) & 128
        level = min(cul, 63) | sign_level
        n4 = n // 4
        x4, y4 = x >> 2, y >> 2
        above = self.above_ccoef if plane else self.above_coef
        left = self.left_ccoef if plane else self.left_coef
        for i in range(n4):
            above[x4 + i] = level
            left[y4 + i] = level
        return eob, coef, txtp


def decode(data: bytes) -> bytes:
    """One still picture. Empty input is BOUNDS. A short OBU is INTEGRITY_FAIL."""
    if not data:
        _bad(BOUNDS, "empty AV1 payload")
    obus = _obus(data)
    fr = Frame()
    saw_seq = saw_frame = False
    tile = b""
    for typ, payload in obus:
        if typ == 2:
            continue
        if typ == 1:
            _seq(payload, fr)
            saw_seq = True
        elif typ == 6:
            if not saw_seq:
                _bad(INTEGRITY_FAIL, "AV1 frame before the sequence header")
            tile = _frame_header(payload, fr)
            saw_frame = True
        elif typ == 15:
            continue
        else:
            _bad(UNSUPPORTED, f"AV1 OBU type {typ} is not in this slice")
    if not saw_frame:
        _bad(INTEGRITY_FAIL, "AV1 payload has no frame")
    dec = Decoder(fr, tile)
    dec.decode()
    if any(fr.lf):
        _deblock(dec)
    return dec.pic()


def _deblock(dec: Decoder) -> None:
    """Loop-filter deltas are parsed. A non-zero level still needs the filter
    itself; until that lands, reconstruction is what `decode` returns."""
    return
