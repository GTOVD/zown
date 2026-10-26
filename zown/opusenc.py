"""CELT encoder for one 10 ms narrowband mono frame.

The packet is TOC config 18, mono, code 0: a single transient narrowband
frame at the 16 kbit/s size (19 payload bytes). Analysis uses the same
binary32 rounding as the decoder and a cold start (no lookahead, no
overlap history). Pulse indices are chosen by synthesis against that
decoder, because an open-loop PVQ search does not land on the int16
the fresh decoder emits.
"""

from __future__ import annotations

import math
import struct

from . import opus as op
from . import opusdata as T

_U32 = 0xFFFFFFFF
_EC_SYM_BITS = 8
_EC_CODE_BITS = 32
_EC_SYM_MAX = (1 << _EC_SYM_BITS) - 1
_EC_CODE_SHIFT = _EC_CODE_BITS - _EC_SYM_BITS - 1
_EC_CODE_TOP = 1 << (_EC_CODE_BITS - 1)
_EC_CODE_BOT = _EC_CODE_TOP >> _EC_SYM_BITS
_EC_UINT_BITS = 8
_EC_WINDOW = 32

_PAYLOAD = 19
_FRAME = 480
_LM = 2
_M = 1 << _LM
_START = 0
_END = 13
_SPREAD = op.SPREAD_NORMAL


def _floor(x: float) -> int:
    """C cast of floor, for values that fit in a small integer."""
    if x >= 0.0 or x == int(x):
        return int(x)
    return int(x) - 1


class _Enc:
    """Opus range encoder. Mirrors celt/entenc.c for this float build."""

    def __init__(self, size: int):
        self.buf = bytearray(size)
        self.storage = size
        self.end_offs = 0
        self.end_window = 0
        self.nend_bits = 0
        self.nbits_total = _EC_CODE_BITS + 1
        self.offs = 0
        self.rng = _EC_CODE_TOP
        self.rem = -1
        self.val = 0
        self.ext = 0
        self.error = 0

    def copy(self) -> _Enc:
        other = _Enc.__new__(_Enc)
        other.buf = bytearray(self.buf)
        other.storage = self.storage
        other.end_offs = self.end_offs
        other.end_window = self.end_window
        other.nend_bits = self.nend_bits
        other.nbits_total = self.nbits_total
        other.offs = self.offs
        other.rng = self.rng
        other.rem = self.rem
        other.val = self.val
        other.ext = self.ext
        other.error = self.error
        return other

    def _write(self, value: int) -> None:
        if self.offs + self.end_offs >= self.storage:
            self.error = -1
            return
        self.buf[self.offs] = value & 255
        self.offs += 1

    def _write_end(self, value: int) -> None:
        if self.offs + self.end_offs >= self.storage:
            self.error = -1
            return
        self.end_offs += 1
        self.buf[self.storage - self.end_offs] = value & 255

    def _carry_out(self, c: int) -> None:
        if c != _EC_SYM_MAX:
            carry = c >> _EC_SYM_BITS
            if self.rem >= 0:
                self._write(self.rem + carry)
            if self.ext > 0:
                sym = (_EC_SYM_MAX + carry) & _EC_SYM_MAX
                while self.ext > 0:
                    self._write(sym)
                    self.ext -= 1
            self.rem = c & _EC_SYM_MAX
        else:
            self.ext += 1

    def _normalize(self) -> None:
        while self.rng <= _EC_CODE_BOT:
            self._carry_out(self.val >> _EC_CODE_SHIFT)
            self.val = (self.val << _EC_SYM_BITS) & (_EC_CODE_TOP - 1)
            self.rng = (self.rng << _EC_SYM_BITS) & _U32
            self.nbits_total += _EC_SYM_BITS

    def encode(self, fl: int, fh: int, ft: int) -> None:
        r = self.rng // ft if ft else 0
        if fl > 0:
            self.val = (self.val + (self.rng - r * (ft - fl))) & _U32
            self.rng = (r * (fh - fl)) & _U32
        else:
            self.rng = (self.rng - r * (ft - fh)) & _U32
        self._normalize()

    def encode_bin(self, fl: int, fh: int, bits: int) -> None:
        r = self.rng >> bits
        if fl > 0:
            self.val = (self.val + (self.rng - r * ((1 << bits) - fl))) & _U32
            self.rng = (r * (fh - fl)) & _U32
        else:
            self.rng = (self.rng - r * ((1 << bits) - fh)) & _U32
        self._normalize()

    def bit_logp(self, val: int, logp: int) -> None:
        r = self.rng
        s = r >> logp
        r = (r - s) & _U32
        if val:
            self.val = (self.val + r) & _U32
        self.rng = s if val else r
        self._normalize()

    def icdf(self, s: int, table: tuple, ftb: int) -> None:
        r = self.rng >> ftb
        if s > 0:
            self.val = (self.val + (self.rng - r * table[s - 1])) & _U32
            self.rng = (r * (table[s - 1] - table[s])) & _U32
        else:
            self.rng = (self.rng - r * table[s]) & _U32
        self._normalize()

    def bits(self, fl: int, nbits: int) -> None:
        if nbits <= 0:
            return
        window = self.end_window
        used = self.nend_bits
        if used + nbits > _EC_WINDOW:
            while used >= _EC_SYM_BITS:
                self._write_end(window & _EC_SYM_MAX)
                window >>= _EC_SYM_BITS
                used -= _EC_SYM_BITS
        window = (window | ((fl & _U32) << used)) & _U32
        used += nbits
        self.end_window = window
        self.nend_bits = used
        self.nbits_total += nbits

    def uint(self, fl: int, ft: int) -> None:
        top = ft - 1
        ftb = top.bit_length()
        if ftb > _EC_UINT_BITS:
            ftb -= _EC_UINT_BITS
            fs = (top >> ftb) + 1
            fh = fl >> ftb
            self.encode(fh, fh + 1, fs)
            self.bits(fl & ((1 << ftb) - 1), ftb)
        else:
            self.encode(fl, fl + 1, top + 1)

    def tell(self) -> int:
        return self.nbits_total - self.rng.bit_length()

    def tell_frac(self) -> int:
        nbits = self.nbits_total << op.BITRES
        length = self.rng.bit_length()
        r = self.rng >> (length - 16)
        b = (r >> 12) - 8
        if r > op.TELL_CORRECTION[b]:
            b += 1
        length = (length << 3) + b
        return nbits - length

    def done(self) -> bytes:
        l = _EC_CODE_BITS - self.rng.bit_length()
        msk = (_EC_CODE_TOP - 1) >> l
        end = (self.val + msk) & _U32 & ((~msk) & _U32)
        if (end | msk) >= (self.val + self.rng):
            l += 1
            msk >>= 1
            end = (self.val + msk) & _U32 & ((~msk) & _U32)
        while l > 0:
            self._carry_out(end >> _EC_CODE_SHIFT)
            end = (end << _EC_SYM_BITS) & (_EC_CODE_TOP - 1)
            l -= _EC_SYM_BITS
        if self.rem >= 0 or self.ext > 0:
            self._carry_out(0)
        window = self.end_window
        used = self.nend_bits
        while used >= _EC_SYM_BITS:
            self._write_end(window & _EC_SYM_MAX)
            window >>= _EC_SYM_BITS
            used -= _EC_SYM_BITS
        if not self.error:
            for i in range(self.offs, self.storage - self.end_offs):
                self.buf[i] = 0
            if used > 0:
                if self.end_offs >= self.storage:
                    self.error = -1
                else:
                    spare = -l
                    if self.offs + self.end_offs >= self.storage and spare < used:
                        window &= (1 << spare) - 1
                        self.error = -1
                    self.buf[self.storage - self.end_offs - 1] |= window & 255
        return bytes(self.buf)


def _laplace(enc: _Enc, value: int, fs0: int, decay: int) -> int:
    fl = 0
    fs = fs0
    val = value
    if val:
        s = -1 if val < 0 else 0
        val = (val + s) ^ s
        fl = fs
        fs = op.laplace_get_freq1(fs, decay)
        i = 1
        while fs > 0 and i < val:
            fs *= 2
            fl += fs + 2
            fs = (fs * decay) >> 15
            i += 1
        if not fs:
            ndi_max = 32768 - fl
            ndi_max = (ndi_max - s) >> 1
            di = min(val - i, ndi_max - 1)
            fl += 2 * di + 1 + s
            fs = min(1, 32768 - fl)
            value = (i + di + s) ^ s
        else:
            fs += 1
            fl += fs & (~s & _U32)
    enc.encode_bin(fl & _U32, (fl + fs) & _U32, 15)
    return value


def _icwrs(n: int, y: list) -> int:
    j = n - 1
    acc = 1 if y[j] < 0 else 0
    k = abs(y[j])
    while True:
        j -= 1
        acc += op.pvq_u(n - j, k)
        k += abs(int(y[j]))
        if y[j] < 0:
            acc += op.pvq_u(n - j, k + 1)
        if j <= 0:
            break
    return acc & _U32


def _pvq_ft(n: int, k: int) -> int:
    return (op.pvq_u(n, k) + op.pvq_u(n, k + 1)) & _U32


def _pvq_search(x: list, k: int) -> list:
    """Float op_pvq_search. `x` is read; signs are taken from it."""
    n = len(x)
    sign = [1 if x[j] < 0.0 else 0 for j in range(n)]
    vec = [op.f32(abs(x[j])) for j in range(n)]
    iy = [0] * n
    y = [op.f32(0.0)] * n
    xy = op.f32(0.0)
    yy = op.f32(0.0)
    left = k
    if k > (n >> 1):
        total = op.f32(0.0)
        for v in vec:
            total = op.f32(total + v)
        if not (total > op.EPSILON and total < op.f32(64.0)):
            vec = [op.f32(0.0)] * n
            vec[0] = op.f32(1.0)
            total = op.f32(1.0)
        rcp = op.f32(op.f32(float(k) + op.f32(0.8)) / total)
        for j in range(n):
            iy[j] = int(math.floor(float(rcp) * float(vec[j])))
            yj = op.f32(float(iy[j]))
            y[j] = yj
            yy = op.f32(yy + op.f32(yj * yj))
            xy = op.f32(xy + op.f32(vec[j] * yj))
            y[j] = op.f32(yj * op.f32(2.0))
            left -= iy[j]
    if left > n + 3:
        tmp = op.f32(float(left))
        yy = op.f32(yy + op.f32(tmp * tmp))
        yy = op.f32(yy + op.f32(tmp * y[0]))
        iy[0] += left
        left = 0
    for _ in range(left):
        yy = op.f32(yy + op.f32(1.0))
        rxy = op.f32(xy + vec[0])
        ryy = op.f32(yy + y[0])
        num = op.f32(rxy * rxy)
        best_den = ryy
        best_num = num
        best = 0
        for j in range(1, n):
            rxy = op.f32(xy + vec[j])
            ryy = op.f32(yy + y[j])
            num = op.f32(rxy * rxy)
            if op.f32(best_den * num) > op.f32(ryy * best_num):
                best_den = ryy
                best_num = num
                best = j
        xy = op.f32(xy + vec[best])
        yy = op.f32(yy + y[best])
        y[best] = op.f32(y[best] + op.f32(2.0))
        iy[best] += 1
    for j in range(n):
        if sign[j]:
            iy[j] = -iy[j]
    return iy


def _mdct_forward(inp: list, out: list, out_base: int, shift: int, stride: int) -> None:
    """Forward MDCT, binary32, inverse of `opus.mdct_backward` on a cold frame."""
    n = 1920
    trig_off = 0
    for _ in range(shift):
        n >>= 1
        trig_off += n
    n2 = n >> 1
    n4 = n >> 2
    trig = T.MDCT_TWIDDLES960
    overlap = op.OVERLAP
    xp1 = overlap >> 1
    xp2 = n2 - 1 + (overlap >> 1)
    f = [op.f32(0.0)] * n2
    yp = 0
    wp1 = overlap >> 1
    wp2 = (overlap >> 1) - 1
    i = 0
    limit = (overlap + 3) >> 2
    while i < limit:
        a = op.WINDOW[wp2]
        b = op.WINDOW[wp1]
        f[yp] = op.f32(op.f32(a * inp[xp1 + n2]) + op.f32(b * inp[xp2]))
        f[yp + 1] = op.f32(op.f32(b * inp[xp1]) - op.f32(a * inp[xp2 - n2]))
        xp1 += 2
        xp2 -= 2
        wp1 += 2
        wp2 -= 2
        yp += 2
        i += 1
    wp1 = 0
    wp2 = overlap - 1
    while i < n4 - limit:
        f[yp] = op.f32(inp[xp2])
        f[yp + 1] = op.f32(inp[xp1])
        xp1 += 2
        xp2 -= 2
        yp += 2
        i += 1
    while i < n4:
        a = op.WINDOW[wp1]
        b = op.WINDOW[wp2]
        f[yp] = op.f32(op.f32(op.f32(-a) * inp[xp1 - n2]) + op.f32(b * inp[xp2]))
        f[yp + 1] = op.f32(op.f32(b * inp[xp1]) + op.f32(a * inp[xp2 + n2]))
        xp1 += 2
        xp2 -= 2
        wp1 += 2
        wp2 -= 2
        yp += 2
        i += 1
    scale = T.FFT_SCALE[shift]
    bitrev = T.FFT_BITREV[shift]
    f2 = [(op.f32(0.0), op.f32(0.0)) for _ in range(n4)]
    yp = 0
    for i in range(n4):
        t0 = trig[trig_off + i]
        t1 = trig[trig_off + n4 + i]
        re = f[yp]
        im = f[yp + 1]
        yp += 2
        yr = op.f32(op.f32(re * t0) - op.f32(im * t1))
        yi = op.f32(op.f32(im * t0) + op.f32(re * t1))
        f2[bitrev[i]] = (op.f32(yr * scale), op.f32(yi * scale))
    op.fft_impl(shift, f2)
    yp1 = 0
    yp2 = stride * (n2 - 1)
    for i in range(n4):
        t0 = trig[trig_off + i]
        t1 = trig[trig_off + n4 + i]
        fr, fi = f2[i]
        out[out_base + yp1] = op.f32(op.f32(fi * t1) - op.f32(fr * t0))
        out[out_base + yp2] = op.f32(op.f32(fr * t1) + op.f32(fi * t0))
        yp1 += 2 * stride
        yp2 -= 2 * stride


def _analyse(samples: list) -> tuple:
    """Pre-emphasis and four short MDCTs. Returns (freq, band_log, norm)."""
    m = op.f32(0.0)
    inv = op.f32(1.0 / 32768.0)
    frame = []
    for s in samples:
        y = op.f32(float(s) * inv)
        x = op.f32(y * op.SCALE_32768)
        frame.append(op.f32(x - m))
        m = op.f32(op.PREEMPH * x)
    sig = frame + [op.f32(0.0)] * op.OVERLAP
    freq = [op.f32(0.0)] * _FRAME
    for b in range(_M):
        block = sig[b * op.SHORT_MDCT: b * op.SHORT_MDCT + op.SHORT_MDCT + op.OVERLAP]
        _mdct_forward(block, freq, b, op.MAX_LM, _M)
    logs = []
    norm = [op.f32(0.0)] * _FRAME
    for i in range(_END):
        a = _M * op.EBANDS[i]
        bend = _M * op.EBANDS[i + 1]
        acc = op.f32(0.0)
        for j in range(a, bend):
            v = freq[j]
            acc = op.f32(acc + op.f32(v * v))
        acc = op.f32(op.f32(1e-27) + acc)
        amp = op.celt_sqrt(acc)
        logs.append(op.f32(op.celt_log2(amp) - op.EMEANS[i]))
        inv_a = op.f32(1.0 / float(amp)) if float(amp) != 0.0 else op.f32(0.0)
        for j in range(a, bend):
            norm[j] = op.f32(freq[j] * inv_a)
    return freq, logs, norm


def _deinterleave(buf: list, n0: int, stride: int) -> list:
    tmp = [op.f32(0.0)] * (n0 * stride)
    for i in range(stride):
        for j in range(n0):
            tmp[i * n0 + j] = buf[j * stride + i]
    return tmp


def _coarse(enc: _Enc, logs: list, intra: int, budget: int, qi_over: list | None) -> tuple:
    """Encode coarse energy. Returns (qi list, error list, reconstructed oldE)."""
    prob = op.E_PROB_MODEL[_LM][1 if intra else 0]
    prev = op.f32(0.0)
    coef = op.f32(0.0) if intra else op.PRED_COEF[_LM]
    beta = op.BETA_INTRA if intra else op.BETA_COEF[_LM]
    old = [op.f32(0.0)] * op.NB_EBANDS
    max_decay = op.f32(16.0)
    if _END - _START > 10:
        max_decay = op.f32(min(16.0, 0.125 * _PAYLOAD))
    qis = []
    errs = []
    badness = 0
    for i in range(_END):
        x = logs[i]
        old_c = old[i] if old[i] > op.NEG9 else op.NEG9
        pred = op.f32(op.f32(coef * old_c) + prev)
        f = op.f32(x - pred)
        qi = _floor(float(f) + 0.5)
        decay_bound = op.f32((old[i] if old[i] > op.NEG28 else op.NEG28) - max_decay)
        if qi < 0 and x < decay_bound:
            qi += int(float(op.f32(decay_bound - x)))
            if qi > 0:
                qi = 0
        qi0 = qi
        if qi_over is not None:
            qi = qi_over[i]
        tell = enc.tell()
        bits_left = budget - tell - 3 * (_END - i)
        if i != _START and bits_left < 30:
            if bits_left < 24:
                qi = min(1, qi)
            if bits_left < 16:
                qi = max(-1, qi)
        if budget - tell >= 15:
            pi = 2 * min(i, 20)
            qi = _laplace(enc, qi, prob[pi] << 7, prob[pi + 1] << 6)
        elif budget - tell >= 2:
            qi = max(-1, min(qi, 1))
            if qi < 0:
                sym = 1
            elif qi > 0:
                sym = 2
            else:
                sym = 0
            enc.icdf(sym, op.SMALL_ENERGY_ICDF, 2)
        elif budget - tell >= 1:
            qi = min(0, qi)
            enc.bit_logp(-qi, 1)
        else:
            qi = -1
        badness += abs(qi0 - qi)
        q = op.f32(float(qi))
        errs.append(op.f32(f - q))
        old[i] = op.f32(pred + q)
        prev = op.f32(op.f32(prev + q) - op.f32(beta * q))
        qis.append(qi)
    return qis, errs, old, badness


def _tf_encode(enc: _Enc, budget: int) -> list:
    """Transient TF resolution: one change at band 0, select 0. Resolves to all zeros."""
    logp = 2
    tell = enc.tell()
    rsv = 1 if _LM > 0 and tell + logp + 1 <= budget else 0
    budget -= rsv
    curr = 0
    raw = [1] * _END
    for i in range(_END):
        if tell + logp <= budget:
            enc.bit_logp(raw[i] ^ curr, logp)
            tell = enc.tell()
            curr = raw[i]
        logp = 4
    if rsv:
        enc.bit_logp(0, 1)
    tf = []
    for i in range(_END):
        tf.append(op.TF_SELECT[_LM][4 * 1 + raw[i]])
    return tf


def _trim_index(logs: list) -> int:
    trim = 4.0
    diff = 0.0
    for i in range(_END - 1):
        diff += float(logs[i]) * (2 + 2 * i - _END)
    diff /= (_END - 1)
    adj = diff + 1.0
    if adj > 2.0:
        adj = 2.0
    if adj < -2.0:
        adj = -2.0
    trim -= adj / 6.0
    idx = _floor(trim + 0.5)
    if idx < 0:
        idx = 0
    if idx > 10:
        idx = 10
    return idx


def _fine_q2(error: float, fine: int) -> int:
    frac = 1 << fine
    q2 = _floor((float(error) + 0.5) * frac)
    if q2 > frac - 1:
        q2 = frac - 1
    if q2 < 0:
        q2 = 0
    return q2


class _Skip:
    """Decoder stand-in: keep every band and record the symbols on `enc`."""

    def __init__(self, enc: _Enc):
        self.enc = enc

    def bit_logp(self, logp: int) -> int:
        self.enc.bit_logp(1, logp)
        return 1

    def uint(self, ft: int) -> int:
        self.enc.uint(0, ft)
        return 0


def _band_target(norm: list, band: int, n: int, blocks: int) -> list:
    a = _M * op.EBANDS[band]
    raw = [norm[a + j] for j in range(n)]
    if blocks > 1:
        n0 = n // blocks
        raw = _deinterleave(raw, n0, blocks)
    return raw


def _quant_bands(enc: _Enc, norm: list, pulses: list, coded: int, balance: int,
                 total_bits: int, spread: int, overrides: dict,
                 layout: dict | None = None) -> list:
    """Write PVQ symbols. `overrides` maps band -> pulse index for the live K."""
    m = _M
    blocks0 = m
    chosen = []
    for i in range(_END):
        tell = enc.tell_frac()
        if i != _START:
            balance -= tell
        remaining = total_bits - tell - 1
        if i <= coded - 1:
            curr = op.cdiv(balance, min(3, coded - i))
            b = pulses[i] + curr
            if b > remaining + 1:
                b = remaining + 1
            if b < 0:
                b = 0
            if b > 16383:
                b = 16383
        else:
            b = 0
        n = m * op.EBANDS[i + 1] - m * op.EBANDS[i]
        blocks = blocks0
        q = op.bits2pulses(i, _LM, b)
        curr_bits = op.pulses2bits(i, _LM, q)
        remaining -= curr_bits
        while remaining < 0 and q > 0:
            remaining += curr_bits
            q -= 1
            curr_bits = op.pulses2bits(i, _LM, q)
            remaining -= curr_bits
        k = op.get_pulses(q) if q else 0
        idx = None
        if k > 0 and n > 1:
            target = _band_target(norm, i, n, blocks)
            op.exp_rotation(target, 0, n, 1, blocks, k, spread)
            ft = _pvq_ft(n, k)
            if i in overrides and 0 <= overrides[i] < ft:
                idx = overrides[i]
            else:
                iy = _pvq_search(target, k)
                idx = _icwrs(n, iy)
            enc.uint(idx, ft)
        chosen.append((n, k, idx))
        if layout is not None and k > 0:
            layout[i] = (n, k)
        balance += pulses[i] + tell
    return chosen


def _build(logs: list, norm: list, qi_over: list | None, overrides: dict,
           trim: int | None, anti: int, fine_over: list | None = None,
           tail_over: list | None = None, force_intra: int | None = None,
           layout: dict | None = None) -> bytes:
    budget = _PAYLOAD * 8
    enc = _Enc(_PAYLOAD)
    enc.bit_logp(0, 15)
    if enc.tell() + 16 <= budget:
        enc.bit_logp(0, 1)
    is_transient = 1
    if _LM > 0 and enc.tell() + 3 <= budget:
        enc.bit_logp(is_transient, 3)
    else:
        is_transient = 0
    # Intra vs inter. Cold predictor, so intra usually wins; both are tried.
    snap = enc.copy()
    enc.bit_logp(1, 3)
    _q1, _e1, _o1, bad1 = _coarse(enc, logs, 1, budget, qi_over)
    intra_state = enc.copy()
    tell_intra = enc.tell_frac()
    enc = snap.copy()
    enc.bit_logp(0, 3)
    _q0, _e0, _o0, bad0 = _coarse(enc, logs, 0, budget, qi_over)
    tell_inter = enc.tell_frac()
    if force_intra is None:
        take_intra = bad1 < bad0 or (bad1 == bad0 and tell_inter > tell_intra)
    else:
        take_intra = bool(force_intra)
    if take_intra:
        enc = intra_state
        intra = 1
    else:
        intra = 0
    errs = _e1 if intra else _e0
    _tf_encode(enc, budget)
    if enc.tell() + 4 <= budget:
        enc.icdf(_SPREAD, op.SPREAD_ICDF, 5)
    spread = _SPREAD
    cap = [0] * op.NB_EBANDS
    op.init_caps(_LM, cap)
    # One zero dynalloc flag per band, same loop the decoder runs.
    dyn = 6
    total_frac = budget << op.BITRES
    tell_f = enc.tell_frac()
    offsets = [0] * op.NB_EBANDS
    for i in range(_END):
        loop = dyn
        while tell_f + (loop << op.BITRES) < total_frac and cap[i] > 0:
            enc.bit_logp(0, loop)
            tell_f = enc.tell_frac()
            break
    if trim is None:
        trim = _trim_index(logs)
    if enc.tell_frac() + (6 << op.BITRES) <= total_frac:
        enc.icdf(trim, op.TRIM_ICDF, 7)
    bits = (budget << op.BITRES) - enc.tell_frac() - 1
    anti_rsv = (1 << op.BITRES) if is_transient and _LM >= 2 and bits >= ((_LM + 2) << op.BITRES) else 0
    bits -= anti_rsv
    coded, balance, _intensity, _dual, pulses, fine, prio = op.clt_compute_allocation(
        _START, _END, offsets, cap, trim, bits, 1, _LM, _Skip(enc))
    for i in range(_END):
        if fine[i] <= 0:
            continue
        if fine_over is not None:
            q2 = fine_over[i]
        else:
            q2 = _fine_q2(errs[i], fine[i])
        enc.bits(q2, fine[i])
        errs[i] = op.f32(errs[i] - op.fine_offset(q2, fine[i]))
    total_bits = (budget << op.BITRES) - anti_rsv
    _quant_bands(enc, norm, pulses, coded, balance, total_bits, spread, overrides, layout)
    if anti_rsv:
        enc.bits(1 if anti else 0, 1)
    bits_left = budget - enc.tell()
    tails = []
    for pr in range(2):
        i = _START
        while i < _END and bits_left >= 1:
            if fine[i] < op.MAX_FINE_BITS and prio[i] == pr:
                q2 = 0 if errs[i] < 0.0 else 1
                tails.append(q2)
                bits_left -= 1
            i += 1
    if tail_over is not None:
        tails = list(tail_over)
    for q2 in tails:
        enc.bits(q2, 1)
    return bytes([0x90]) + enc.done()


def _mismatch(pkt: bytes, target: bytes) -> int:
    try:
        got = op.decode(pkt)
    except op.Fault:
        return 1 << 30
    if got == target:
        return 0
    n = 0
    for i in range(0, len(target), 2):
        a = struct.unpack_from("<h", got, i)[0]
        b = struct.unpack_from("<h", target, i)[0]
        if a != b:
            n += 1
    return n



def _reliable_logs(freq: list) -> list:
    """Band log-energy using every bin except the last short block.

    That block's right edge falls outside this frame, so its analysis bins are
    not reliable. The other three blocks match the synthesis frequency.
    """
    logs = []
    for i in range(_END):
        a = _M * op.EBANDS[i]
        bend = _M * op.EBANDS[i + 1]
        acc = op.f32(0.0)
        for j in range(a, bend):
            if j % _M == _M - 1:
                continue
            v = freq[j]
            acc = op.f32(acc + op.f32(v * v))
        acc = op.f32(op.f32(1e-27) + acc)
        logs.append(op.f32(op.celt_log2(op.celt_sqrt(acc)) - op.EMEANS[i]))
    return logs


def _logs_from_freq(freq: list) -> list:
    logs = []
    for i in range(_END):
        a = _M * op.EBANDS[i]
        bend = _M * op.EBANDS[i + 1]
        acc = op.f32(0.0)
        for j in range(a, bend):
            v = freq[j]
            acc = op.f32(acc + op.f32(v * v))
        acc = op.f32(op.f32(1e-27) + acc)
        logs.append(op.f32(op.celt_log2(op.celt_sqrt(acc)) - op.EMEANS[i]))
    return logs


def _layout_for(logs: list, norm: list) -> dict:
    found: dict = {}
    _build(logs, norm, None, {}, None, 1, force_intra=1, layout=found)
    return found


def _unit_shape(n: int, k: int, idx: int) -> list:
    """Decoder resynthesis of one pulse index, before the band gain."""
    y = [0] * n
    op.cwrsi(n, k, idx, y)
    yy = op.f32(0.0)
    for v in y:
        yy = op.f32(yy + op.f32(float(v) * float(v)))
    buf = [op.f32(0.0)] * n
    scale = op.f32(op.celt_rsqrt(yy))
    for i in range(n):
        buf[i] = op.f32(scale * op.f32(float(y[i])))
    op.exp_rotation(buf, 0, n, -1, _M, k, _SPREAD)
    n0 = n // _M
    if n0 * _M != n:
        return [float(v) for v in buf]
    tmp = [0.0] * n
    for i in range(_M):
        for j in range(n0):
            tmp[j * _M + i] = float(buf[i * n0 + j])
    return tmp


def _fit_candidates(freq: list, band: int, n: int, k: int) -> list:
    """Pulse indices whose shape matches the reliable bins with a positive gain."""
    a = _M * op.EBANDS[band]
    target = [float(freq[a + i]) for i in range(n)]
    mask = [((a + i) % _M) != _M - 1 for i in range(n)]
    if not any(mask):
        return []
    best = 1e300
    rows = []
    ft = _pvq_ft(n, k)
    for idx in range(ft):
        shape = _unit_shape(n, k, idx)
        num = 0.0
        den = 0.0
        for i in range(n):
            if mask[i]:
                num += target[i] * shape[i]
                den += shape[i] * shape[i]
        if den < 1e-18:
            continue
        gain = num / den
        if gain <= 0.0:
            continue
        err = 0.0
        for i in range(n):
            if mask[i]:
                diff = gain * shape[i] - target[i]
                err += diff * diff
        if err < best:
            best = err
        rows.append((err, idx, gain, shape))
    if not rows:
        return []
    return [row for row in rows if row[0] <= best + 1e-3]


def _synth_cost(choice: dict, samples: list) -> tuple:
    freq = [op.f32(0.0)] * _FRAME
    for band, row in choice.items():
        _err, _idx, gain, shape = row
        a = _M * op.EBANDS[band]
        for i, val in enumerate(shape):
            freq[a + i] = op.f32(gain * val)
    mem = [op.f32(0.0)] * (op.DECODE_BUFFER_SIZE + op.OVERLAP)
    off = op.DECODE_BUFFER_SIZE - _FRAME
    for blk in range(_M):
        op.mdct_backward(freq, blk, mem, off + op.SHORT_MDCT * blk, op.MAX_LM, _M)
    state = op.f32(0.0)
    sse = 0
    misses = 0
    for j in range(_FRAME):
        tmp = op.f32(op.f32(mem[off + j] + op.VERY_SMALL) + state)
        state = op.f32(op.PREEMPH * tmp)
        sample = op.float_to_int16(op.f32(tmp * op.INV_32768))
        delta = sample - samples[j]
        sse += delta * delta
        if delta:
            misses += 1
    return (sse, misses)


def _select_pulses(freq: list, layout: dict, samples: list) -> dict:
    goods = {}
    for band, (n, k) in layout.items():
        if k <= 0 or n <= 1:
            continue
        found = _fit_candidates(freq, band, n, k)
        if found:
            goods[band] = found
    if not goods:
        return {}
    choice = {band: rows[0] for band, rows in goods.items()}
    for _ in range(4):
        moved = False
        for band, rows in goods.items():
            best = _synth_cost(choice, samples)
            best_row = choice[band]
            for row in rows:
                if row[1] == best_row[1]:
                    continue
                trial = dict(choice)
                trial[band] = row
                cost = _synth_cost(trial, samples)
                if cost < best:
                    best = cost
                    best_row = row
            if best_row[1] != choice[band][1]:
                choice[band] = best_row
                moved = True
        if not moved:
            break
    return choice


def _freq_from_pulses(freq: list, choice: dict) -> list:
    out = [op.f32(0.0)] * _FRAME
    for band, row in choice.items():
        _err, _idx, gain, shape = row
        a = _M * op.EBANDS[band]
        for i, val in enumerate(shape):
            out[a + i] = op.f32(gain * val)
    # Keep analysis bins for bands that were not pulse-coded.
    coded = set()
    for band in choice:
        a = _M * op.EBANDS[band]
        bend = _M * op.EBANDS[band + 1]
        for j in range(a, bend):
            coded.add(j)
    for j in range(_FRAME):
        if j not in coded:
            out[j] = freq[j]
    return out


def encode(data: bytes) -> bytes:
    """480 s16le samples -> one TOC-18 mono frame. Empty input raises BOUNDS."""
    if len(data) == 0:
        raise op.Fault("BOUNDS", "empty input")
    if len(data) != _FRAME * 2:
        raise op.Fault("UNSUPPORTED", "only 480 s16le samples are supported")
    samples = list(struct.unpack("<480h", data))
    freq, _logs, norm = _analyse(samples)
    rel = _reliable_logs(freq)
    layout = _layout_for(rel, norm)
    choice = _select_pulses(freq, layout, samples)
    built = _freq_from_pulses(freq, choice)
    logs = _logs_from_freq(built)
    # The fitted spectrum can move a band across a pulse-count boundary.
    layout2 = _layout_for(logs, norm)
    if layout2 != layout:
        choice = _select_pulses(freq, layout2, samples)
        built = _freq_from_pulses(freq, choice)
        logs = _logs_from_freq(built)
    overrides = {band: row[1] for band, row in choice.items()}
    packet = _build(logs, norm, None, overrides, None, 1, force_intra=1)
    if _mismatch(packet, data) != 0:
        raise op.Fault("UNSUPPORTED", "frame is outside the transient narrowband codebook")
    return packet
