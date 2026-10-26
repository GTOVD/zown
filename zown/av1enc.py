"""AV1 still encode for the profile `av` decodes (SPEC §23).

`encode` writes one 16×16 4:2:0 reduced still and nothing else. The frame is
lossless: 4×4 Walsh-Hadamard, qindex 0. `decode` of the result is the same
I420 bytes. A buffer that is not 384 bytes is UNSUPPORTED. An empty buffer
is BOUNDS.
"""

from __future__ import annotations

import copy

from .av1 import (
    DC, LO_CTX, MODE, NONE, PART_N, SCAN, SKIP_CTX, TX_4, _bad,
)
from .av1data import COEF, KFYM
from .errors import BOUNDS, UNSUPPORTED

MASK64 = (1 << 64) - 1


class MsacEnc:
    """Daala range encoder. Bytes match the decoder in `av1.Msac`."""

    def __init__(self) -> None:
        self.buf = bytearray()
        self.offs = 0
        self.low = 0
        self.rng = 0x8000
        self.cnt = -9

    def _carry(self, offs: int) -> None:
        carry = 1
        while carry:
            if offs < 0:
                break
            total = self.buf[offs] + 1
            self.buf[offs] = total & 0xFF
            carry = total >> 8
            offs -= 1

    def _write(self, output: int, carry: int, num: int) -> None:
        shift = (8 - num) << 3
        reg = (output << shift) & MASK64
        raw = reg.to_bytes(8, "big")
        end = self.offs + 8
        if len(self.buf) < end:
            self.buf.extend(b"\x00" * (end - len(self.buf)))
        self.buf[self.offs:end] = raw
        if carry:
            self._carry(self.offs - 1)
        self.offs += num

    def _norm(self, low: int, rng: int) -> None:
        c = self.cnt
        d = 16 - rng.bit_length()
        s = c + d
        if s >= 40:
            num = (s >> 3) + 1
            c = c + 24 - (num << 3)
            output = low >> c
            low &= (1 << c) - 1
            mask = 1 << (num << 3)
            carry = output & mask
            output &= mask - 1
            self._write(output, carry, num)
            s = c + d - 24
        self.low = (low << d) & MASK64
        self.rng = rng << d
        self.cnt = s

    def bool(self, bit: int, f: int) -> None:
        r = self.rng
        v = ((r >> 8) * (f >> 6) >> 1) + 4
        low = self.low
        if bit:
            low += r - v
            r = v
        else:
            r -= v
        self._norm(low, r)

    def equi(self, bit: int) -> None:
        self.bool(bit, 16384)

    def bools(self, n: int, value: int) -> None:
        for i in range(n - 1, -1, -1):
            self.equi((value >> i) & 1)

    def adapt_bool(self, cdf: list[int], bit: int) -> None:
        self.bool(bit, cdf[0])
        count = cdf[1]
        rate = 4 + (count >> 4)
        if bit:
            cdf[0] += (32768 - cdf[0]) >> rate
        else:
            cdf[0] -= cdf[0] >> rate
        cdf[1] = count + (count < 32)

    def symbol(self, cdf: list[int], n: int, s: int) -> None:
        fl = 32768 if s == 0 else cdf[s - 1]
        fh = cdf[s]
        r = self.rng
        low = self.low
        if fl < 32768:
            u = ((r >> 8) * (fl >> 6) >> 1) + 4 * (n - (s - 1))
            v = ((r >> 8) * (fh >> 6) >> 1) + 4 * (n - s)
            low += r - u
            r = u - v
        else:
            r -= ((r >> 8) * (fh >> 6) >> 1) + 4 * (n - s)
        self._norm(low, r)
        count = cdf[n]
        rate = 4 + (count >> 4) + (n > 2)
        for i in range(s):
            cdf[i] += (32768 - cdf[i]) >> rate
        for i in range(s, n):
            cdf[i] -= cdf[i] >> rate
        cdf[n] = count + (count < 32)

    def hi_tok(self, cdf: list[int], tok: int) -> None:
        if tok < 6:
            self.symbol(cdf, 3, tok - 3)
            return
        self.symbol(cdf, 3, 3)
        if tok < 9:
            self.symbol(cdf, 3, tok - 6)
            return
        self.symbol(cdf, 3, 3)
        if tok < 12:
            self.symbol(cdf, 3, tok - 9)
            return
        self.symbol(cdf, 3, 3)
        self.symbol(cdf, 3, tok - 12)

    def golomb(self, value: int) -> None:
        length = (value + 1).bit_length() - 1
        for _ in range(length):
            self.equi(0)
        self.equi(1)
        body = (value + 1) & ((1 << length) - 1) if length else 0
        for i in range(length - 1, -1, -1):
            self.equi((body >> i) & 1)

    def done(self) -> bytes:
        low = self.low
        c = self.cnt
        s = 10 + c
        m = 0x3FFF
        e = ((low + m) & ~m) | (m + 1)
        if s > 0:
            n = (1 << (c + 16)) - 1
            while s > 0:
                val = (e >> (c + 16)) & 0xFFFF
                if len(self.buf) <= self.offs:
                    self.buf.append(0)
                self.buf[self.offs] = val & 0xFF
                if val & 0x100:
                    self._carry(self.offs - 1)
                self.offs += 1
                e &= n
                s -= 8
                c -= 8
                n >>= 8
        return bytes(self.buf[:self.offs])


class _Bits:
    def __init__(self) -> None:
        self.bits: list[int] = []

    def u(self, n: int, v: int) -> None:
        for i in range(n - 1, -1, -1):
            self.bits.append((v >> i) & 1)

    def align(self) -> None:
        while len(self.bits) & 7:
            self.bits.append(0)

    def trailing(self) -> None:
        self.bits.append(1)
        while len(self.bits) & 7:
            self.bits.append(0)

    def payload(self) -> bytes:
        out = bytearray()
        acc = 0
        k = 0
        for b in self.bits:
            acc = (acc << 1) | b
            k += 1
            if k == 8:
                out.append(acc)
                acc = 0
                k = 0
        return bytes(out)


def _leb(n: int) -> bytes:
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def _obu(typ: int, payload: bytes) -> bytes:
    return bytes([(typ << 3) | 0x02]) + _leb(len(payload)) + payload


def _seq() -> bytes:
    b = _Bits()
    b.u(3, 0)  # profile
    b.u(1, 1)  # still
    b.u(1, 1)  # reduced
    b.u(5, 0)  # level
    b.u(4, 3)  # width bits minus one
    b.u(4, 3)
    b.u(4, 15)  # 16 pixels
    b.u(4, 15)
    b.u(1, 0)  # 128-sb
    b.u(1, 0)  # filter intra
    b.u(1, 0)  # edge filter
    b.u(1, 0)
    b.u(1, 0)
    b.u(1, 0)  # superres, cdef, restoration
    b.u(1, 0)  # high bit depth
    b.u(1, 0)  # mono
    b.u(1, 0)  # color description
    b.u(1, 1)  # color range
    b.u(2, 0)  # chroma sample position
    b.u(1, 0)  # separate uv delta
    b.u(1, 0)  # film grain
    b.trailing()
    return b.payload()


def _frame_header() -> bytes:
    b = _Bits()
    b.u(1, 0)  # cdf update stays on
    b.u(1, 0)  # screen
    b.u(1, 0)  # render size
    b.u(1, 1)  # uniform tiles
    b.u(8, 0)  # qindex 0, lossless
    b.u(1, 0)
    b.u(1, 0)
    b.u(1, 0)  # no delta q
    b.u(1, 0)  # qmatrix
    b.u(1, 0)  # segmentation
    b.u(1, 0)  # reduced tx set
    b.align()
    return b.payload()


def _fwht(raster: list[int]) -> list[int]:
    """Forward 4×4 Walsh-Hadamard. The result is what the decoder stores."""
    mid = [0] * 16
    for i in range(4):
        a = raster[0 * 4 + i]
        b = raster[1 * 4 + i]
        c = raster[2 * 4 + i]
        d = raster[3 * 4 + i]
        a += b
        d -= c
        e = (a - d) >> 1
        b = e - b
        c = e - c
        a -= c
        d += b
        mid[i * 4 + 0] = a
        mid[i * 4 + 1] = c
        mid[i * 4 + 2] = d
        mid[i * 4 + 3] = b
    out = [0] * 16
    for i in range(4):
        a = mid[4 * 0 + i]
        b = mid[4 * 1 + i]
        c = mid[4 * 2 + i]
        d = mid[4 * 3 + i]
        a += b
        d -= c
        e = (a - d) >> 1
        b = e - b
        c = e - c
        a -= c
        d += b
        out[4 * 0 + i] = a * 4
        out[4 * 1 + i] = c * 4
        out[4 * 2 + i] = d * 4
        out[4 * 3 + i] = b * 4
    return out


def _dc_pred(dst: bytearray, stride: int, x: int, y: int) -> int:
    n = 4
    if x == 0 and y == 0:
        return 128
    if y == 0:
        s = n >> 1
        for i in range(n):
            s += dst[(y + i) * stride + (x - 1)]
        return s >> (n.bit_length() - 1)
    if x == 0:
        s = n >> 1
        for i in range(n):
            s += dst[(y - 1) * stride + (x + i)]
        return s >> (n.bit_length() - 1)
    s = n
    for i in range(n):
        s += dst[(y - 1) * stride + x + i]
        s += dst[(y + i) * stride + (x - 1)]
    return s >> n.bit_length()


def _skip_ctx(above: list[int], left: list[int], x4: int, y4: int) -> int:
    la = min(above[x4] & 0x3F, 4)
    ll = min(left[y4] & 0x3F, 4)
    return SKIP_CTX[la][ll]


def _dc_sign_ctx(above: list[int], left: list[int], x4: int, y4: int) -> int:
    s = (above[x4] >> 6) + (left[y4] >> 6) - 2
    return (s != 0) + (s > 0)


def _encode_eob(ms: MsacEnc, coef: dict, ctx: int, ch: int, eob: int) -> None:
    cdf = coef["eob_bin_16"][ch][0]
    if eob <= 1:
        ms.symbol(cdf, 4, eob)
        return
    k = eob.bit_length() - 2
    eob_bin = k + 2
    ms.symbol(cdf, 4, eob_bin)
    # (hi|2)<<k is the top. The upper half starts at 3<<k.
    hi = 1 if eob >= (3 << k) else 0
    ms.adapt_bool(coef["eob_hi_bit"][ctx][ch][eob_bin], hi)
    if k:
        ms.bools(k, eob & ((1 << k) - 1))


def _br_for(tok: int, base: int, mag: int) -> int:
    if tok < 3:
        return base
    mag &= 63
    return base + (6 if mag > 12 else (mag + 1) >> 1)


def _encode_base(ms: MsacEnc, coef: dict, ctx: int, ch: int, base_cdf: list[int],
                 br_at: int, tok: int) -> int:
    """Write one base/hi token. Golomb follows the sign, so it stops at 15."""
    if tok < 3:
        ms.symbol(base_cdf, 3, tok)
        return tok
    ms.symbol(base_cdf, 3, 3)
    shown = 15 if tok >= 15 else tok
    ms.hi_tok(coef["br_tok"][min(ctx, 3)][ch][br_at], shown)
    return shown


def _encode_eob_tok(ms: MsacEnc, coef: dict, ctx: int, ch: int, eob_cdf: list[int],
                    br_at: int, tok: int) -> int:
    et = 2 if tok >= 3 else tok - 1
    ms.symbol(eob_cdf, 2, et)
    if et < 2:
        return tok
    shown = 15 if tok >= 15 else tok
    ms.hi_tok(coef["br_tok"][min(ctx, 3)][ch][br_at], shown)
    return shown


def _encode_block(ms: MsacEnc, coef: dict, above: list[int], left: list[int],
                  small: list[int], x4: int, y4: int, plane: int) -> None:
    """`small[row + col*4]` is the Walsh value. A zero block is skipped."""
    ctx = TX_4
    ch = 1 if plane else 0
    sctx = 7 if plane else _skip_ctx(above, left, x4, y4)
    scan = SCAN[TX_4]
    # `small` holds the dequantized coefficient. The token is that value / 4.
    scaled = [v // 4 for v in small]
    eob = -1
    for i in range(15, -1, -1):
        if scaled[scan[i]]:
            eob = i
            break
    if eob < 0:
        ms.adapt_bool(coef["skip"][ctx][sctx], 1)
        above[x4] = 0x40
        left[y4] = 0x40
        return
    ms.adapt_bool(coef["skip"][ctx][sctx], 0)
    _encode_eob(ms, coef, ctx, ch, eob)
    n = 4
    stride_l = n
    levels = [0] * (stride_l * (n + 2))

    def pos(i: int) -> tuple[int, int]:
        rc = scan[i]
        return rc // n, rc % n

    def store(lx: int, ly: int, tok: int) -> None:
        if tok >= 3:
            levels[lx * stride_l + ly] = tok + (3 << 6)
        else:
            levels[lx * stride_l + ly] = tok * 0x41

    def lo_ctx(lx: int, ly: int) -> tuple[int, int]:
        def at(dx: int, dy: int) -> int:
            return levels[(lx + dx) * stride_l + (ly + dy)]
        mag = at(0, 1) + at(1, 0) + at(1, 1)
        hi = mag
        mag += at(0, 2) + at(2, 0)
        off = LO_CTX[0][min(ly, 4)][min(lx, 4)]
        return off + (4 if mag > 512 else (mag + 64) >> 7), hi

    def hi_base(lx: int, ly: int) -> int:
        return 14 if (lx or ly) > 1 else 7

    acs: list[int] = []
    if eob:
        lx, ly = pos(eob)
        eob_ctx = 1 + (eob > 2) + (eob > 4)
        tok = abs(scaled[scan[eob]])
        shown = _encode_eob_tok(ms, coef, ctx, ch, coef["eob_base_tok"][ctx][ch][eob_ctx],
                                hi_base(lx, ly), tok)
        store(lx, ly, shown)
        acs.append(eob)
        for i in range(eob - 1, 0, -1):
            lx, ly = pos(i)
            cctx, mag = lo_ctx(lx, ly)
            tok = abs(scaled[scan[i]])
            shown = _encode_base(ms, coef, ctx, ch, coef["base_tok"][ctx][ch][cctx],
                                 _br_for(tok, hi_base(lx, ly), mag), tok)
            store(lx, ly, shown)
            if tok:
                acs.append(i)
        dc_tok = abs(scaled[scan[0]])
        _encode_base(ms, coef, ctx, ch, coef["base_tok"][ctx][ch][0],
                     _dc_hi(levels, stride_l), dc_tok)
    else:
        dc_tok = abs(scaled[scan[0]])
        _encode_eob_tok(ms, coef, ctx, ch, coef["eob_base_tok"][ctx][ch][0], 0, dc_tok)
    cul = 0
    neg = 0
    if dc_tok:
        neg = 1 if scaled[scan[0]] < 0 else 0
        ms.adapt_bool(coef["dc_sign"][ch][_dc_sign_ctx(above, left, x4, y4)], neg)
        if dc_tok >= 15:
            ms.golomb(dc_tok - 15)
        cul += dc_tok
    for i in sorted(acs):
        ms.equi(1 if scaled[scan[i]] < 0 else 0)
        tok = abs(scaled[scan[i]])
        if tok >= 15:
            ms.golomb(tok - 15)
        cul += tok
    if dc_tok == 0:
        sign_level = 64
    else:
        sign_level = (neg - 1) & 128
    above[x4] = min(cul, 63) | sign_level
    left[y4] = above[x4]


def _dc_hi(levels: list[int], stride: int) -> int:
    mag = (levels[1] + levels[stride] + levels[stride + 1]) & 63
    return 6 if mag > 12 else (mag + 1) >> 1


def encode(data: bytes) -> bytes:
    """16×16 4:2:0 I420 to a raw OBU sequence. Lossless for this size."""
    if not data:
        _bad(BOUNDS, "empty AV1 frame")
    if len(data) != 384:
        _bad(UNSUPPORTED, "this slice encodes a 16x16 4:2:0 frame")
    y = bytearray(data[:256])
    u = bytearray(data[256:320])
    v = bytearray(data[320:384])
    rec_y = bytearray(256)
    rec_u = bytearray([128]) * 64
    rec_v = bytearray([128]) * 64
    ms = MsacEnc()
    mode = copy.deepcopy(MODE)
    coef = copy.deepcopy(COEF[0])
    kfym = copy.deepcopy(KFYM)
    # One 16×16 partition: the 64 and 32 splits are implicit.
    ms.symbol(mode["partition"][3][0], PART_N[3], NONE)
    ms.adapt_bool(mode["skip"][0], 0)
    ms.symbol(kfym[0][0], 12, DC)
    ms.symbol(mode["uv_mode"][1][DC], 13, DC)
    above_y = [0x40] * 64
    left_y = [0x40] * 64
    above_c = [0x40] * 64
    left_c = [0x40] * 64

    def plane_blocks(src: bytearray, rec: bytearray, stride: int, n: int,
                     above: list[int], left: list[int], plane: int) -> None:
        for by in range(0, n, 4):
            for bx in range(0, n, 4):
                pred = _dc_pred(rec, stride, bx, by)
                raster = []
                for row in range(4):
                    for col in range(4):
                        raster.append(src[(by + row) * stride + bx + col] - pred)
                small = _fwht(raster)
                _encode_block(ms, coef, above, left, small, bx >> 2, by >> 2, plane)
                for row in range(4):
                    for col in range(4):
                        rec[(by + row) * stride + bx + col] = src[(by + row) * stride + bx + col]

    plane_blocks(y, rec_y, 16, 16, above_y, left_y, 0)
    plane_blocks(u, rec_u, 8, 8, above_c, left_c, 1)
    plane_blocks(v, rec_v, 8, 8, above_c, left_c, 2)
    tile = ms.done()
    payload = _frame_header() + tile
    return _obu(2, b"") + _obu(1, _seq()) + _obu(6, payload)
