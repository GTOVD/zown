"""CELT-only Opus decoder for one 10 ms narrowband mono frame.

Float build of Opus 1.4: QCONST16 is the identity and SHL32/PSHR32/SHR32 on a
float are no-ops, so coarse energy is ``tmp = coef*old + prev + q``. Every
arithmetic step is rounded to binary32.
"""

from __future__ import annotations

import math
import struct

from . import opusdata as T

NB_EBANDS = 21
SHORT_MDCT = 120
MAX_LM = 3
OVERLAP = 120
DECODE_BUFFER_SIZE = 2048
COMBFILTER_MINPERIOD = 15
BITRES = 3
MAX_FINE_BITS = 8
FINE_OFFSET = 21
QTHETA_OFFSET = 4
QTHETA_OFFSET_TWOPHASE = 16
ALLOC_STEPS = 6
SPREAD_NONE = 0
SPREAD_LIGHT = 1
SPREAD_NORMAL = 2
SPREAD_AGGRESSIVE = 3

TRIM_ICDF = (126, 124, 119, 109, 87, 41, 19, 9, 4, 2, 0)
SPREAD_ICDF = (25, 23, 2, 0)
TAPSET_ICDF = (2, 1, 0)
SMALL_ENERGY_ICDF = (2, 1, 0)
LOG2_FRAC = (
    0, 8, 13, 16, 19, 21, 23, 24, 26, 27, 28, 29, 30, 31, 32,
    32, 33, 34, 34, 35, 36, 36, 37, 37,
)
EXP2_TABLE8 = (16384, 17866, 19483, 21247, 23170, 25267, 27554, 30048)
ORDERY = (
    1, 0,
    3, 0, 2, 1,
    7, 0, 4, 3, 6, 1, 5, 2,
    15, 0, 8, 7, 12, 3, 11, 4, 14, 1, 9, 6, 13, 2, 10, 5,
)
BIT_INTERLEAVE = (0, 1, 1, 1, 2, 3, 3, 3, 2, 3, 3, 3, 2, 3, 3, 3)
BIT_DEINTERLEAVE = (
    0x00, 0x03, 0x0C, 0x0F, 0x30, 0x33, 0x3C, 0x3F,
    0xC0, 0xC3, 0xCC, 0xCF, 0xF0, 0xF3, 0xFC, 0xFF,
)
SPREAD_FACTOR = (15, 10, 5)
EC_CODE_BOT = 1 << 23
TELL_CORRECTION = (35733, 38967, 42495, 46340, 50535, 55109, 60097, 65535)

# e_prob_model[LM][intra][2*band + {0:prob0, 1:decay}], 480-sample row is LM=2.
E_PROB_MODEL = (
    (
        (72, 127, 65, 129, 66, 128, 65, 128, 64, 128, 62, 128, 64, 128,
         64, 128, 92, 78, 92, 79, 92, 78, 90, 79, 116, 41, 115, 40,
         114, 40, 132, 26, 132, 26, 145, 17, 161, 12, 176, 10, 177, 11),
        (24, 179, 48, 138, 54, 135, 54, 132, 53, 134, 56, 133, 55, 132,
         55, 132, 61, 114, 70, 96, 74, 88, 75, 88, 87, 74, 89, 66,
         91, 67, 100, 59, 108, 50, 120, 40, 122, 37, 97, 43, 78, 50),
    ),
    (
        (83, 78, 84, 81, 88, 75, 86, 74, 87, 71, 90, 73, 93, 74,
         93, 74, 109, 40, 114, 36, 117, 34, 117, 34, 143, 17, 145, 18,
         146, 19, 162, 12, 165, 10, 178, 7, 189, 6, 190, 8, 177, 9),
        (23, 178, 54, 115, 63, 102, 66, 98, 69, 99, 74, 89, 71, 91,
         73, 91, 78, 89, 86, 80, 92, 66, 93, 64, 102, 59, 103, 60,
         104, 60, 117, 52, 123, 44, 138, 35, 133, 31, 97, 38, 77, 45),
    ),
    (
        (61, 90, 93, 60, 105, 42, 107, 41, 110, 45, 116, 38, 113, 38,
         112, 38, 124, 26, 132, 27, 136, 19, 140, 20, 155, 14, 159, 16,
         158, 18, 170, 13, 177, 10, 187, 8, 192, 6, 175, 9, 159, 10),
        (21, 178, 59, 110, 71, 86, 75, 85, 84, 83, 91, 66, 88, 73,
         87, 72, 92, 75, 98, 72, 105, 58, 107, 54, 115, 52, 114, 55,
         112, 56, 129, 51, 132, 40, 150, 33, 140, 29, 98, 35, 77, 42),
    ),
    (
        (42, 121, 96, 66, 108, 43, 111, 40, 117, 44, 123, 32, 120, 36,
         119, 33, 127, 33, 134, 34, 139, 21, 147, 23, 152, 20, 158, 25,
         154, 26, 166, 21, 173, 16, 184, 13, 184, 10, 150, 13, 139, 15),
        (22, 178, 63, 114, 74, 82, 84, 83, 92, 82, 103, 62, 96, 72,
         96, 67, 101, 73, 107, 72, 113, 55, 118, 52, 125, 52, 118, 52,
         117, 55, 135, 49, 137, 39, 157, 32, 145, 29, 97, 33, 77, 40),
    ),
)
TF_SELECT = (
    (0, -1, 0, -1, 0, -1, 0, -1),
    (0, -1, 0, -2, 1, 0, 1, -1),
    (0, -2, 0, -3, 2, 0, 1, -1),
    (0, -2, 0, -3, 3, 0, 1, -1),
)


def f32(x: float) -> float:
    return struct.unpack("<f", struct.pack("<f", float(x)))[0]


# Mean band energy in the float log domain (quant_bands.c eMeans).
EMEANS = (
    f32(6.437500), f32(6.250000), f32(5.750000), f32(5.312500), f32(5.062500),
    f32(4.812500), f32(4.500000), f32(4.375000), f32(4.875000), f32(4.687500),
    f32(4.562500), f32(4.437500), f32(4.875000), f32(4.625000), f32(4.312500),
    f32(4.500000), f32(4.375000), f32(4.625000), f32(4.750000), f32(4.437500),
    f32(3.750000), f32(3.750000), f32(3.750000), f32(3.750000), f32(3.750000),
)
PRED_COEF = tuple(f32(n / 32768.0) for n in (29440, 26112, 21248, 16384))
BETA_COEF = tuple(f32(n / 32768.0) for n in (30147, 22282, 12124, 6554))
BETA_INTRA = f32(4915 / 32768.0)
VERY_SMALL = f32(1e-30)
EPSILON = f32(1e-15)
PI = f32(3.141592653)
HALF_PI = f32(f32(0.5) * PI)
INV_32768 = f32(1.0 / 32768.0)
SCALE_32768 = f32(32768.0)
PREEMPH = f32(0.85000610)
HAAR_C = f32(0.70710678)
FFT_C = f32(0.7071067812)
SQRT2 = f32(1.41421356)
NEG9 = f32(-9.0)
NEG28 = f32(-28.0)
POSTFILTER_STEP = f32(0.09375)
GAINS = (
    (f32(0.3066406250), f32(0.2170410156), f32(0.1296386719)),
    (f32(0.4638671875), f32(0.2680664062), f32(0.0)),
    (f32(0.7998046875), f32(0.1000976562), f32(0.0)),
)
EBANDS = T.EBAND5MS
LOGN = T.LOGN400
ALLOC = T.BAND_ALLOCATION
WINDOW = T.WINDOW120


class Fault(Exception):
    def __init__(self, code: str, msg: str):
        super().__init__(msg)
        self.code = code
        self.msg = msg


def s16(x: int) -> int:
    x &= 0xFFFF
    return x - 0x10000 if x >= 0x8000 else x


def s32(x: int) -> int:
    x &= 0xFFFFFFFF
    return x - 0x100000000 if x >= 0x80000000 else x


def cdiv(n: int, d: int) -> int:
    """C99 toward-zero integer division."""
    if d < 0:
        n, d = -n, -d
    if n >= 0:
        return n // d
    return -((-n) // d)


def frac_mul16(a: int, b: int) -> int:
    return (16384 + s16(a) * s16(b)) >> 15


def fmin(a: float, b: float) -> float:
    return a if a < b else b


def fmax(a: float, b: float) -> float:
    return a if a > b else b


class EcDec:
    """Opus range decoder. A read with offs or end_offs past storage sets read_err."""

    def __init__(self, data: bytes):
        self.buf = data
        self.storage = len(data)
        self.end_offs = 0
        self.end_window = 0
        self.nend_bits = 0
        self.nbits_total = 9
        self.offs = 0
        self.rng = 1 << 7
        self.rem = self._read()
        self.val = (self.rng - 1 - (self.rem >> 1)) & 0xFFFFFFFF
        self.error = 0
        self.read_err = 0
        self.front_past = 0
        self.end_past = 0
        self.ext = 0
        self._normalize()

    def _read(self) -> int:
        if self.offs >= self.storage:
            self.read_err = 1
            self.front_past += 1
            return 0
        b = self.buf[self.offs]
        self.offs += 1
        return b

    def _read_end(self) -> int:
        if self.end_offs >= self.storage:
            self.read_err = 1
            self.end_past += 1
            return 0
        self.end_offs += 1
        return self.buf[self.storage - self.end_offs]

    def _normalize(self) -> None:
        while self.rng <= EC_CODE_BOT:
            self.nbits_total += 8
            self.rng = (self.rng << 8) & 0xFFFFFFFF
            sym = self.rem
            self.rem = self._read()
            sym = ((sym << 8) | self.rem) >> 1
            self.val = (((self.val << 8) & 0xFFFFFFFF) + (0xFF & (~sym))) & 0x7FFFFFFF

    def tell(self) -> int:
        return self.nbits_total - self.rng.bit_length()

    def tell_frac(self) -> int:
        nbits = self.nbits_total << BITRES
        length = self.rng.bit_length()
        r = self.rng >> (length - 16)
        b = (r >> 12) - 8
        if r > TELL_CORRECTION[b]:
            b += 1
        length = (length << 3) + b
        return nbits - length

    def decode(self, ft: int) -> int:
        self.ext = (self.rng // ft) if ft else 0
        if self.ext == 0:
            self.error = 1
            return 0
        s = self.val // self.ext
        return ft - min(s + 1, ft)

    def decode_bin(self, bits: int) -> int:
        self.ext = self.rng >> bits
        ft = 1 << bits
        if self.ext == 0:
            self.error = 1
            return 0
        s = self.val // self.ext
        return ft - min(s + 1, ft)

    def update(self, fl: int, fh: int, ft: int) -> None:
        s = (self.ext * (ft - fh)) & 0xFFFFFFFF
        self.val = (self.val - s) & 0xFFFFFFFF
        if fl > 0:
            self.rng = (self.ext * (fh - fl)) & 0xFFFFFFFF
        else:
            self.rng = (self.rng - s) & 0xFFFFFFFF
        self._normalize()

    def bit_logp(self, logp: int) -> int:
        r = self.rng
        d = self.val
        s = r >> logp
        ret = 1 if d < s else 0
        if not ret:
            self.val = (d - s) & 0xFFFFFFFF
        self.rng = s if ret else (r - s) & 0xFFFFFFFF
        self._normalize()
        return ret

    def icdf(self, table: tuple, ftb: int) -> int:
        s = self.rng
        d = self.val
        r = s >> ftb
        ret = -1
        while True:
            t = s
            ret += 1
            s = (r * table[ret]) & 0xFFFFFFFF
            if d >= s:
                break
        self.val = (d - s) & 0xFFFFFFFF
        self.rng = (t - s) & 0xFFFFFFFF
        self._normalize()
        return ret

    def bits(self, n: int) -> int:
        window = self.end_window
        available = self.nend_bits
        if available < n:
            while True:
                window = (window | ((self._read_end() << available) & 0xFFFFFFFF)) & 0xFFFFFFFF
                available += 8
                if available > 24:
                    break
        ret = window & ((1 << n) - 1) if n else 0
        window = (window >> n) & 0xFFFFFFFF if n else window
        self.end_window = window
        self.nend_bits = available - n
        self.nbits_total += n
        return ret

    def uint(self, ft: int) -> int:
        if ft <= 1:
            self.error = 1
            return 0
        top = ft - 1
        ftb = top.bit_length()
        if ftb > 8:
            ftb -= 8
            fs = (top >> ftb) + 1
            s = self.decode(fs)
            self.update(s, s + 1, fs)
            t = ((s << ftb) | self.bits(ftb)) & 0xFFFFFFFF
            if t <= top:
                return t
            self.error = 1
            return top
        s = self.decode(ft)
        self.update(s, s + 1, ft)
        return s


def laplace_get_freq1(fs0: int, decay: int) -> int:
    ft = 32768 - 2 * 16 - fs0
    return (ft * (16384 - decay)) >> 15


def ec_laplace_decode(dec: EcDec, fs: int, decay: int) -> int:
    val = 0
    fm = dec.decode_bin(15)
    fl = 0
    if fm >= fs:
        val = 1
        fl = fs
        fs = laplace_get_freq1(fs, decay) + 1
        while fs > 1 and fm >= fl + 2 * fs:
            fs *= 2
            fl += fs
            fs = (((fs - 2) * decay) >> 15) + 1
            val += 1
        if fs <= 1:
            di = (fm - fl) >> 1
            val += di
            fl += 2 * di
        if fm < fl + fs:
            val = -val
        else:
            fl += fs
    dec.update(fl, min(fl + fs, 32768), 32768)
    return val


def celt_sqrt(x: float) -> float:
    return f32(math.sqrt(f32(x)))


def celt_rsqrt(x: float) -> float:
    return f32(1.0 / celt_sqrt(x))


def celt_exp2(x: float) -> float:
    """FLOAT_APPROX polynomial. The exponent is inserted with the C bit trick."""
    x = f32(x)
    # floor for a value that fits in an int; toward -inf, matching (int)floor.
    integer = int(x) if x >= 0.0 or x == int(x) else int(x) - 1
    if integer < -50:
        return f32(0.0)
    frac = f32(x - float(integer))
    res = f32(f32(0.078024523) * frac)
    res = f32(f32(0.22606716) + res)
    res = f32(frac * res)
    res = f32(f32(0.69583354) + res)
    res = f32(frac * res)
    res = f32(f32(0.99992522) + res)
    bits = struct.unpack("<I", struct.pack("<f", res))[0]
    add = ((integer & 0xFFFFFFFF) << 23) & 0xFFFFFFFF
    bits = (bits + add) & 0x7FFFFFFF
    return struct.unpack("<f", struct.pack("<I", bits))[0]


def celt_log2(x: float) -> float:
    bits = struct.unpack("<I", struct.pack("<f", f32(x)))[0]
    integer = (bits >> 23) - 127
    bits = (bits - ((integer & 0xFFFFFFFF) << 23)) & 0xFFFFFFFF
    frac = struct.unpack("<f", struct.pack("<I", bits))[0]
    frac = f32(frac - f32(1.5))
    acc = f32(f32(0.16541097) * frac)
    acc = f32(f32(-0.33951290) + acc)
    acc = f32(frac * acc)
    acc = f32(f32(0.95909232) + acc)
    acc = f32(frac * acc)
    frac = f32(f32(-0.41445418) + acc)
    return f32(float(1 + integer) + frac)


def _k_cos(z: float) -> float:
    z2 = z * z
    w = z2 * z2
    r = z2 * (4.16666666666666019037e-02 + z2 * (-1.38888888888741095749e-03 + z2 * 2.48015872894767294178e-05))
    r = r + w * w * (-2.75573143513906633035e-07 + z2 * (2.08757232129817482790e-09 + z2 * -1.13596475577881948265e-11))
    hz = 0.5 * z2
    w = 1.0 - hz
    return w + (((1.0 - w) - hz) + (z2 * r))


def _k_sin(z: float) -> float:
    z2 = z * z
    v = z * z2
    r = 8.33333333332248946124e-03 + z2 * (
        -1.98412698298579493134e-04 + z2 * (
            2.75573137070700676789e-06 + z2 * (
                -2.50507602534068634195e-08 + z2 * 1.58969099521155010221e-10)))
    return z + v * (-1.66666666666666324348e-01 + z2 * r)


def _cos_f64(x: float) -> float:
    """Double-precision cosine used only to implement celt_cos_norm = (float)cos(arg)."""
    if x < 0.0:
        x = -x
    pio2_1 = 1.57079632673412561417
    pio2_2 = 6.07710050650619224932e-11
    pio2_3 = 2.02226624879595063154e-21
    n = int(x / 1.5707963267948966 + 0.5)
    r = ((x - n * pio2_1) - n * pio2_2) - n * pio2_3
    q = n & 3
    if q == 0:
        return _k_cos(r)
    if q == 1:
        return -_k_sin(r)
    if q == 2:
        return -_k_cos(r)
    return _k_sin(r)


def celt_cos_norm(x: float) -> float:
    arg = f32(HALF_PI * f32(x))
    return f32(_cos_f64(float(arg)))


def isqrt32(val: int) -> int:
    val &= 0xFFFFFFFF
    if val == 0:
        return 0
    g = 0
    bshift = (val.bit_length() - 1) >> 1
    b = 1 << bshift
    while True:
        t = (((g << 1) + b) << bshift) & 0xFFFFFFFF
        if t <= val:
            g += b
            val = (val - t) & 0xFFFFFFFF
        b >>= 1
        bshift -= 1
        if bshift < 0:
            break
    return g


def bitexact_cos(x: int) -> int:
    x = s16(x)
    tmp = (4096 + x * x) >> 13
    x2 = s16(tmp)
    inner = frac_mul16(-626, x2)
    inner = frac_mul16(x2, 8277 + inner)
    inner = frac_mul16(x2, -7651 + inner)
    x2 = s16((32767 - x2) + inner)
    return 1 + x2


def bitexact_log2tan(isin: int, icos: int) -> int:
    lc = icos.bit_length()
    ls = isin.bit_length()
    icos = s16(icos << (15 - lc))
    isin = s16(isin << (15 - ls))
    return (ls - lc) * (1 << 11) + frac_mul16(isin, frac_mul16(isin, -2597) + 7932) - frac_mul16(icos, frac_mul16(icos, -2597) + 7932)


def unquant_coarse_energy(dec: EcDec, start: int, end: int, old_e: list, intra: int, lm: int) -> None:
    # Float build: SHL32/PSHR32 are no-ops, so q is the integer step itself.
    prob = E_PROB_MODEL[lm][1 if intra else 0]
    prev = [f32(0.0), f32(0.0)]
    if intra:
        coef = f32(0.0)
        beta = BETA_INTRA
    else:
        coef = PRED_COEF[lm]
        beta = BETA_COEF[lm]
    budget = dec.storage * 8
    for i in range(start, end):
        tell = dec.tell()
        if budget - tell >= 15:
            pi = 2 * min(i, 20)
            qi = ec_laplace_decode(dec, prob[pi] << 7, prob[pi + 1] << 6)
        elif budget - tell >= 2:
            qi = dec.icdf(SMALL_ENERGY_ICDF, 2)
            qi = (qi >> 1) ^ -(qi & 1)
        elif budget - tell >= 1:
            qi = -dec.bit_logp(1)
        else:
            qi = -1
        q = f32(float(qi))
        old = old_e[i]
        old = fmax(NEG9, old)
        tmp = f32(f32(coef * old) + prev[0])
        tmp = f32(tmp + q)
        old_e[i] = tmp
        prev[0] = f32(f32(prev[0] + q) - f32(beta * q))


def fine_offset(q2: int, fine: int) -> float:
    scale = float(1 << (14 - fine))
    t = f32(float(q2) + f32(0.5))
    t = f32(t * f32(scale))
    t = f32(t * f32(1.0 / 16384.0))
    return f32(t - f32(0.5))


def final_offset(q2: int, fine: int) -> float:
    scale = float(1 << (14 - fine - 1))
    t = f32(float(q2) - f32(0.5))
    t = f32(t * f32(scale))
    return f32(t * f32(1.0 / 16384.0))


def unquant_fine_energy(dec: EcDec, start: int, end: int, old_e: list, fine: list) -> None:
    for i in range(start, end):
        if fine[i] <= 0:
            continue
        q2 = dec.bits(fine[i])
        old_e[i] = f32(old_e[i] + fine_offset(q2, fine[i]))


def unquant_energy_finalise(dec: EcDec, start: int, end: int, old_e: list, fine: list, prio: list, bits_left: int) -> None:
    for pr in range(2):
        i = start
        while i < end and bits_left >= 1:
            if fine[i] < MAX_FINE_BITS and prio[i] == pr:
                q2 = dec.bits(1)
                old_e[i] = f32(old_e[i] + final_offset(q2, fine[i]))
                bits_left -= 1
            i += 1


def tf_decode(dec: EcDec, start: int, end: int, is_transient: int, lm: int) -> list:
    budget = dec.storage * 8
    tell = dec.tell()
    logp = 2 if is_transient else 4
    tf_select_rsv = 1 if lm > 0 and tell + logp + 1 <= budget else 0
    budget -= tf_select_rsv
    tf_changed = 0
    curr = 0
    tf_res = [0] * NB_EBANDS
    for i in range(start, end):
        if tell + logp <= budget:
            curr ^= dec.bit_logp(logp)
            tell = dec.tell()
            tf_changed |= curr
        tf_res[i] = curr
        logp = 4 if is_transient else 5
    tf_select = 0
    if tf_select_rsv and TF_SELECT[lm][4 * is_transient + tf_changed] != TF_SELECT[lm][4 * is_transient + 2 + tf_changed]:
        tf_select = dec.bit_logp(1)
    for i in range(start, end):
        tf_res[i] = TF_SELECT[lm][4 * is_transient + 2 * tf_select + tf_res[i]]
    return tf_res


def init_caps(lm: int, cap: list) -> None:
    base = NB_EBANDS * (2 * lm + 1 - 1)
    for i in range(NB_EBANDS):
        n = (EBANDS[i + 1] - EBANDS[i]) << lm
        cap[i] = (T.CACHE_CAPS50[base + i] + 64) * n >> 2


def get_pulses(i: int) -> int:
    return i if i < 8 else (8 + (i & 7)) << ((i >> 3) - 1)


def cache_row(lm: int, band: int) -> list:
    """bits cache for this band at this LM. lm may be -1."""
    index = T.CACHE_INDEX50[(lm + 1) * NB_EBANDS + band]
    return T.CACHE_BITS50[index:]


def bits2pulses(band: int, lm: int, bits: int) -> int:
    cache = cache_row(lm, band)
    lo = 0
    hi = cache[0]
    bits -= 1
    for _ in range(6):
        mid = (lo + hi + 1) >> 1
        if cache[mid] >= bits:
            hi = mid
        else:
            lo = mid
    left = bits - (-1 if lo == 0 else cache[lo])
    right = cache[hi] - bits
    return lo if left <= right else hi


def pulses2bits(band: int, lm: int, pulses: int) -> int:
    if pulses == 0:
        return 0
    return cache_row(lm, band)[pulses] + 1


def interp_bits2pulses(start, end, skip_start, bits1, bits2, thresh, cap, total, skip_rsv,
                       intensity, intensity_rsv, dual_stereo, dual_stereo_rsv, bits, ebits,
                       fine_priority, c, lm, dec, encode):
    alloc_floor = c << BITRES
    logm = lm << BITRES
    lo = 0
    hi = 1 << ALLOC_STEPS
    for _ in range(ALLOC_STEPS):
        mid = (lo + hi) >> 1
        psum = 0
        done = 0
        for j in range(end - 1, start - 1, -1):
            tmp = bits1[j] + ((mid * bits2[j]) >> ALLOC_STEPS)
            if tmp >= thresh[j] or done:
                done = 1
                psum += min(tmp, cap[j])
            elif tmp >= alloc_floor:
                psum += alloc_floor
        if psum > total:
            hi = mid
        else:
            lo = mid
    psum = 0
    done = 0
    for j in range(end - 1, start - 1, -1):
        tmp = bits1[j] + ((lo * bits2[j]) >> ALLOC_STEPS)
        if tmp < thresh[j] and not done:
            tmp = alloc_floor if tmp >= alloc_floor else 0
        else:
            done = 1
        tmp = min(tmp, cap[j])
        bits[j] = tmp
        psum += tmp
    coded_bands = end
    while True:
        j = coded_bands - 1
        if j <= skip_start:
            total += skip_rsv
            break
        left = total - psum
        width_all = EBANDS[coded_bands] - EBANDS[start]
        percoeff = left // width_all
        left -= width_all * percoeff
        rem = max(left - (EBANDS[j] - EBANDS[start]), 0)
        band_width = EBANDS[coded_bands] - EBANDS[j]
        band_bits = bits[j] + percoeff * band_width + rem
        if band_bits >= max(thresh[j], alloc_floor + (1 << BITRES)):
            if dec.bit_logp(1):
                break
            psum += 1 << BITRES
            band_bits -= 1 << BITRES
        psum -= bits[j] + intensity_rsv
        if intensity_rsv > 0:
            intensity_rsv = LOG2_FRAC[j - start]
        psum += intensity_rsv
        if band_bits >= alloc_floor:
            psum += alloc_floor
            bits[j] = alloc_floor
        else:
            bits[j] = 0
        coded_bands -= 1
    if intensity_rsv > 0:
        intensity = start + dec.uint(coded_bands + 1 - start)
    else:
        intensity = 0
    if intensity <= start:
        total += dual_stereo_rsv
        dual_stereo_rsv = 0
    if dual_stereo_rsv > 0:
        dual_stereo = dec.bit_logp(1)
    else:
        dual_stereo = 0
    left = total - psum
    width_all = EBANDS[coded_bands] - EBANDS[start]
    percoeff = left // width_all
    left -= width_all * percoeff
    for j in range(start, coded_bands):
        bits[j] += percoeff * (EBANDS[j + 1] - EBANDS[j])
    for j in range(start, coded_bands):
        tmp = min(left, EBANDS[j + 1] - EBANDS[j])
        bits[j] += tmp
        left -= tmp
    balance = 0
    j = start
    while j < coded_bands:
        n0 = EBANDS[j + 1] - EBANDS[j]
        n = n0 << lm
        bit = bits[j] + balance
        if n > 1:
            excess = max(bit - cap[j], 0)
            bits[j] = bit - excess
            den = c * n + (1 if (c == 2 and n > 2 and not dual_stereo and j < intensity) else 0)
            nclog_n = den * (LOGN[j] + logm)
            offset = (nclog_n >> 1) - den * FINE_OFFSET
            if n == 2:
                offset += den << BITRES >> 2
            if bits[j] + offset < den * 2 << BITRES:
                offset += nclog_n >> 2
            elif bits[j] + offset < den * 3 << BITRES:
                offset += nclog_n >> 3
            e = max(0, bits[j] + offset + (den << (BITRES - 1)))
            e = (e // den) >> BITRES
            if c * e > (bits[j] >> BITRES):
                e = bits[j] >> (1 if c > 1 else 0) >> BITRES
            e = min(e, MAX_FINE_BITS)
            fine_priority[j] = 1 if e * (den << BITRES) >= bits[j] + offset else 0
            bits[j] -= c * e << BITRES
            ebits[j] = e
        else:
            excess = max(0, bit - (c << BITRES))
            bits[j] = bit - excess
            ebits[j] = 0
            fine_priority[j] = 1
        if excess > 0:
            stereo = 1 if c > 1 else 0
            extra_fine = min(excess >> (stereo + BITRES), MAX_FINE_BITS - ebits[j])
            ebits[j] += extra_fine
            extra_bits = extra_fine * c << BITRES
            fine_priority[j] = 1 if extra_bits >= excess - balance else 0
            excess -= extra_bits
        balance = excess
        j += 1
    while j < end:
        ebits[j] = bits[j] >> (1 if c > 1 else 0) >> BITRES
        bits[j] = 0
        fine_priority[j] = 1 if ebits[j] < 1 else 0
        j += 1
    return coded_bands, balance, intensity, dual_stereo


def clt_compute_allocation(start, end, offsets, cap, alloc_trim, total, c, lm, dec):
    total = max(total, 0)
    skip_start = start
    skip_rsv = 1 << BITRES if total >= 1 << BITRES else 0
    total -= skip_rsv
    intensity_rsv = 0
    dual_stereo_rsv = 0
    if c == 2:
        intensity_rsv = LOG2_FRAC[end - start]
        if intensity_rsv > total:
            intensity_rsv = 0
        else:
            total -= intensity_rsv
            dual_stereo_rsv = 1 << BITRES if total >= 1 << BITRES else 0
            total -= dual_stereo_rsv
    bits1 = [0] * NB_EBANDS
    bits2 = [0] * NB_EBANDS
    thresh = [0] * NB_EBANDS
    trim_offset = [0] * NB_EBANDS
    for j in range(start, end):
        thresh[j] = max(c << BITRES, (3 * (EBANDS[j + 1] - EBANDS[j]) << lm << BITRES) >> 4)
        trim_offset[j] = c * (EBANDS[j + 1] - EBANDS[j]) * (alloc_trim - 5 - lm) * (end - j - 1) * (1 << (lm + BITRES)) >> 6
        if (EBANDS[j + 1] - EBANDS[j]) << lm == 1:
            trim_offset[j] -= c << BITRES
    lo = 1
    hi = 11 - 1
    while True:
        done = 0
        psum = 0
        mid = (lo + hi) >> 1
        for j in range(end - 1, start - 1, -1):
            n = EBANDS[j + 1] - EBANDS[j]
            bitsj = ((c * n * ALLOC[mid * NB_EBANDS + j]) << lm) >> 2
            if bitsj > 0:
                bitsj = max(0, bitsj + trim_offset[j])
            bitsj += offsets[j]
            if bitsj >= thresh[j] or done:
                done = 1
                psum += min(bitsj, cap[j])
            elif bitsj >= c << BITRES:
                psum += c << BITRES
        if psum > total:
            hi = mid - 1
        else:
            lo = mid + 1
        if lo > hi:
            break
    hi = lo
    lo = lo - 1
    for j in range(start, end):
        n = EBANDS[j + 1] - EBANDS[j]
        bits1j = ((c * n * ALLOC[lo * NB_EBANDS + j]) << lm) >> 2
        if hi >= 11:
            bits2j = cap[j]
        else:
            bits2j = ((c * n * ALLOC[hi * NB_EBANDS + j]) << lm) >> 2
        if bits1j > 0:
            bits1j = max(0, bits1j + trim_offset[j])
        if bits2j > 0:
            bits2j = max(0, bits2j + trim_offset[j])
        if lo > 0:
            bits1j += offsets[j]
        bits2j += offsets[j]
        if offsets[j] > 0:
            skip_start = j
        bits2j = max(0, bits2j - bits1j)
        bits1[j] = bits1j
        bits2[j] = bits2j
    pulses = [0] * NB_EBANDS
    ebits = [0] * NB_EBANDS
    fine_priority = [0] * NB_EBANDS
    coded, balance, intensity, dual = interp_bits2pulses(
        start, end, skip_start, bits1, bits2, thresh, cap, total, skip_rsv,
        0, intensity_rsv, 0, dual_stereo_rsv, pulses, ebits, fine_priority, c, lm, dec, 0)
    return coded, balance, intensity, dual, pulses, ebits, fine_priority


def pvq_u(n: int, k: int) -> int:
    if n > k:
        n, k = k, n
    return T.PVQ_U_DATA[T.PVQ_U_ROW[n] + k]


def cwrsi(n: int, k: int, idx: int, y: list) -> float:
    """Non-small-footprint PVQ index to pulse vector. Returns the sum of squares."""
    yy = f32(0.0)
    pos = 0
    i = idx & 0xFFFFFFFF
    while n > 2:
        if k >= n:
            row = T.PVQ_U_ROW[n]
            p = T.PVQ_U_DATA[row + k + 1]
            s = -1 if i >= p else 0
            if s:
                i = (i - p) & 0xFFFFFFFF
            k0 = k
            q = T.PVQ_U_DATA[row + n]
            if q > i:
                k = n
                while True:
                    k -= 1
                    p = T.PVQ_U_DATA[T.PVQ_U_ROW[k] + n]
                    if p <= i:
                        break
            else:
                p = T.PVQ_U_DATA[row + k]
                while p > i:
                    k -= 1
                    p = T.PVQ_U_DATA[row + k]
            i = (i - p) & 0xFFFFFFFF
            val = (k0 - k + s) ^ s
            y[pos] = val
            pos += 1
            yy = f32(yy + f32(float(val) * float(val)))
        else:
            p = T.PVQ_U_DATA[T.PVQ_U_ROW[k] + n]
            q = T.PVQ_U_DATA[T.PVQ_U_ROW[k + 1] + n]
            if p <= i < q:
                i = (i - p) & 0xFFFFFFFF
                y[pos] = 0
                pos += 1
            else:
                s = -1 if i >= q else 0
                if s:
                    i = (i - q) & 0xFFFFFFFF
                k0 = k
                while True:
                    k -= 1
                    p = T.PVQ_U_DATA[T.PVQ_U_ROW[k] + n]
                    if p <= i:
                        break
                i = (i - p) & 0xFFFFFFFF
                val = (k0 - k + s) ^ s
                y[pos] = val
                pos += 1
                yy = f32(yy + f32(float(val) * float(val)))
        n -= 1
    p = 2 * k + 1
    s = -1 if i >= p else 0
    if s:
        i = (i - p) & 0xFFFFFFFF
    k0 = k
    k = ((i + 1) & 0xFFFFFFFF) >> 1
    if k:
        i = (i - (2 * k - 1)) & 0xFFFFFFFF
    val = (k0 - k + s) ^ s
    y[pos] = val
    pos += 1
    yy = f32(yy + f32(float(val) * float(val)))
    si = s32(i)
    s = -si
    val = (k + s) ^ s
    y[pos] = val
    yy = f32(yy + f32(float(val) * float(val)))
    return yy


def decode_pulses(y: list, n: int, k: int, dec: EcDec) -> float:
    v = (pvq_u(n, k) + pvq_u(n, k + 1)) & 0xFFFFFFFF
    return cwrsi(n, k, dec.uint(v), y)


def exp_rotation1(buf: list, off: int, length: int, stride: int, c: float, s: float) -> None:
    ms = f32(-s)
    xptr = off
    i = 0
    limit = length - stride
    while i < limit:
        x1 = buf[xptr]
        x2 = buf[xptr + stride]
        buf[xptr + stride] = f32(f32(c * x2) + f32(s * x1))
        buf[xptr] = f32(f32(c * x1) + f32(ms * x2))
        xptr += 1
        i += 1
    i = length - 2 * stride - 1
    xptr = off + i
    while i >= 0:
        x1 = buf[xptr]
        x2 = buf[xptr + stride]
        buf[xptr + stride] = f32(f32(c * x2) + f32(s * x1))
        buf[xptr] = f32(f32(c * x1) + f32(ms * x2))
        xptr -= 1
        i -= 1


def exp_rotation(buf: list, off: int, length: int, direction: int, stride: int, k: int, spread: int) -> None:
    if 2 * k >= length or spread == SPREAD_NONE:
        return
    factor = SPREAD_FACTOR[spread - 1]
    gain = f32(f32(float(length)) / f32(float(length + factor * k)))
    theta = f32(f32(0.5) * f32(gain * gain))
    c = celt_cos_norm(theta)
    s = celt_cos_norm(f32(f32(1.0) - theta))
    stride2 = 0
    if length >= 8 * stride:
        stride2 = 1
        while (stride2 * stride2 + stride2) * stride + (stride >> 2) < length:
            stride2 += 1
    length //= stride
    for i in range(stride):
        base = off + i * length
        if direction < 0:
            if stride2:
                exp_rotation1(buf, base, length, stride2, s, c)
            exp_rotation1(buf, base, length, 1, c, s)
        else:
            exp_rotation1(buf, base, length, 1, c, f32(-s))
            if stride2:
                exp_rotation1(buf, base, length, stride2, s, f32(-c))


def normalise_residual(iy: list, buf: list, off: int, n: int, ryy: float, gain: float) -> None:
    # Float VSHR32 discards the shift, so the gain is gain/sqrt(sum of squares).
    g = f32(celt_rsqrt(ryy) * gain)
    for i in range(n):
        buf[off + i] = f32(g * f32(float(iy[i])))


def extract_collapse_mask(iy: list, n: int, b: int) -> int:
    if b <= 1:
        return 1
    n0 = n // b
    mask = 0
    for i in range(b):
        tmp = 0
        base = i * n0
        for j in range(n0):
            tmp |= iy[base + j]
        if tmp:
            mask |= 1 << i
    return mask


def alg_unquant(buf: list, off: int, n: int, k: int, spread: int, b: int, dec: EcDec, gain: float) -> int:
    iy = [0] * n
    ryy = decode_pulses(iy, n, k, dec)
    normalise_residual(iy, buf, off, n, ryy, gain)
    exp_rotation(buf, off, n, -1, b, k, spread)
    return extract_collapse_mask(iy, n, b)


def renormalise_vector(buf: list, off: int, n: int, gain: float) -> None:
    acc = f32(0.0)
    for i in range(n):
        v = buf[off + i]
        acc = f32(acc + f32(v * v))
    energy = f32(EPSILON + acc)
    g = f32(celt_rsqrt(energy) * gain)
    for i in range(n):
        buf[off + i] = f32(g * buf[off + i])


def lcg(seed: int) -> int:
    return (1664525 * (seed & 0xFFFFFFFF) + 1013904223) & 0xFFFFFFFF


def haar1(buf: list, off: int, n0: int, stride: int) -> None:
    n0 >>= 1
    for i in range(stride):
        for j in range(n0):
            i0 = off + stride * 2 * j + i
            i1 = off + stride * (2 * j + 1) + i
            t1 = f32(HAAR_C * buf[i0])
            t2 = f32(HAAR_C * buf[i1])
            buf[i0] = f32(t1 + t2)
            buf[i1] = f32(t1 - t2)


def deinterleave_hadamard(buf: list, off: int, n0: int, stride: int, hadamard: int) -> None:
    n = n0 * stride
    tmp = [f32(0.0)] * n
    if hadamard:
        ordery = ORDERY[stride - 2:]
        for i in range(stride):
            for j in range(n0):
                tmp[ordery[i] * n0 + j] = buf[off + j * stride + i]
    else:
        for i in range(stride):
            for j in range(n0):
                tmp[i * n0 + j] = buf[off + j * stride + i]
    for i in range(n):
        buf[off + i] = tmp[i]


def interleave_hadamard(buf: list, off: int, n0: int, stride: int, hadamard: int) -> None:
    n = n0 * stride
    tmp = [f32(0.0)] * n
    if hadamard:
        ordery = ORDERY[stride - 2:]
        for i in range(stride):
            for j in range(n0):
                tmp[j * stride + i] = buf[off + ordery[i] * n0 + j]
    else:
        for i in range(stride):
            for j in range(n0):
                tmp[j * stride + i] = buf[off + i * n0 + j]
    for i in range(n):
        buf[off + i] = tmp[i]


def compute_qn(n: int, b: int, offset: int, pulse_cap: int, stereo: int) -> int:
    n2 = 2 * n - 1
    if stereo and n == 2:
        n2 -= 1
    qb = cdiv(b + n2 * offset, n2)
    qb = min(b - pulse_cap - (4 << BITRES), qb)
    qb = min(8 << BITRES, qb)
    if qb < (1 << BITRES >> 1):
        return 1
    qn = EXP2_TABLE8[qb & 7] >> (14 - (qb >> BITRES))
    return ((qn + 1) >> 1) << 1


def compute_theta(ctx, x, xoff, y, yoff, n, b, bb, b0, lm, stereo, fill):
    m_logn = LOGN[ctx.i]
    pulse_cap = m_logn + lm * (1 << BITRES)
    offset = (pulse_cap >> 1) - (QTHETA_OFFSET_TWOPHASE if stereo and n == 2 else QTHETA_OFFSET)
    qn = compute_qn(n, b, offset, pulse_cap, stereo)
    if stereo and ctx.i >= ctx.intensity:
        qn = 1
    tell = ctx.ec.tell_frac()
    itheta = 0
    inv = 0
    if qn != 1:
        if stereo and n > 2:
            p0 = 3
            x0 = qn // 2
            ft = p0 * (x0 + 1) + x0
            fs = ctx.ec.decode(ft)
            if fs < (x0 + 1) * p0:
                itheta = fs // p0
            else:
                itheta = x0 + 1 + (fs - (x0 + 1) * p0)
            fl = p0 * itheta if itheta <= x0 else (itheta - 1 - x0) + (x0 + 1) * p0
            fh = p0 * (itheta + 1) if itheta <= x0 else (itheta - x0) + (x0 + 1) * p0
            ctx.ec.update(fl, fh, ft)
        elif b0 > 1 or stereo:
            itheta = ctx.ec.uint(qn + 1)
        else:
            ft = ((qn >> 1) + 1) * ((qn >> 1) + 1)
            fm = ctx.ec.decode(ft)
            half = (qn >> 1) * ((qn >> 1) + 1) >> 1
            if fm < half:
                itheta = (isqrt32(8 * fm + 1) - 1) >> 1
                fs = itheta + 1
                fl = itheta * (itheta + 1) >> 1
            else:
                itheta = (2 * (qn + 1) - isqrt32(8 * (ft - fm - 1) + 1)) >> 1
                fs = qn + 1 - itheta
                fl = ft - ((qn + 1 - itheta) * (qn + 2 - itheta) >> 1)
            ctx.ec.update(fl, fl + fs, ft)
        itheta = (itheta * 16384) // qn
    elif stereo:
        if b > 2 << BITRES and ctx.remaining_bits > 2 << BITRES:
            inv = ctx.ec.bit_logp(2)
        else:
            inv = 0
        if ctx.disable_inv:
            inv = 0
        itheta = 0
    qalloc = ctx.ec.tell_frac() - tell
    b -= qalloc
    if itheta == 0:
        imid = 32767
        iside = 0
        fill &= (1 << bb) - 1
        delta = -16384
    elif itheta == 16384:
        imid = 0
        iside = 32767
        fill &= ((1 << bb) - 1) << bb
        delta = 16384
    else:
        imid = bitexact_cos(itheta)
        iside = bitexact_cos(16384 - itheta)
        delta = frac_mul16((n - 1) << 7, bitexact_log2tan(iside, imid))
    return b, fill, inv, imid, iside, delta, itheta, qalloc


def quant_band_n1(ctx, buf, off, low_out, low_out_off) -> int:
    sign = 0
    if ctx.remaining_bits >= 1 << BITRES:
        sign = ctx.ec.bits(1)
        ctx.remaining_bits -= 1 << BITRES
    if ctx.resynth:
        buf[off] = f32(-1.0) if sign else f32(1.0)
    if low_out is not None:
        low_out[low_out_off] = buf[off]
    return 1


def quant_partition(ctx, buf, off, n, b, blocks, low, low_off, lm, gain, fill) -> int:
    cache = cache_row(lm, ctx.i)
    if lm != -1 and b > cache[cache[0]] + 12 and n > 2:
        n >>= 1
        yoff = off + n
        lm -= 1
        b0 = blocks
        if blocks == 1:
            fill = (fill & 1) | (fill << 1)
        blocks = (blocks + 1) >> 1
        b, fill, _inv, imid, iside, delta, itheta, qalloc = compute_theta(
            ctx, buf, off, buf, yoff, n, b, blocks, b0, lm, 0, fill)
        mid = f32(f32(1.0 / 32768.0) * f32(float(imid)))
        side = f32(f32(1.0 / 32768.0) * f32(float(iside)))
        if b0 > 1 and (itheta & 0x3FFF):
            if itheta > 8192:
                delta -= delta >> (4 - lm)
            else:
                delta = min(0, delta + (n << BITRES >> (5 - lm)))
        mbits = max(0, min(b, cdiv(b - delta, 2)))
        sbits = b - mbits
        ctx.remaining_bits -= qalloc
        next_low = None
        next_low_off = 0
        if low is not None:
            next_low = low
            next_low_off = low_off + n
        rebalance = ctx.remaining_bits
        if mbits >= sbits:
            cm = quant_partition(ctx, buf, off, n, mbits, blocks, low, low_off, lm,
                                 f32(gain * mid), fill)
            rebalance = mbits - (rebalance - ctx.remaining_bits)
            if rebalance > 3 << BITRES and itheta != 0:
                sbits += rebalance - (3 << BITRES)
            cm |= quant_partition(ctx, buf, yoff, n, sbits, blocks, next_low, next_low_off, lm,
                                  f32(gain * side), fill >> blocks) << (b0 >> 1)
        else:
            cm = quant_partition(ctx, buf, yoff, n, sbits, blocks, next_low, next_low_off, lm,
                                 f32(gain * side), fill >> blocks) << (b0 >> 1)
            rebalance = sbits - (rebalance - ctx.remaining_bits)
            if rebalance > 3 << BITRES and itheta != 16384:
                mbits += rebalance - (3 << BITRES)
            cm |= quant_partition(ctx, buf, off, n, mbits, blocks, low, low_off, lm,
                                  f32(gain * mid), fill)
        return cm
    q = bits2pulses(ctx.i, lm, b)
    curr_bits = pulses2bits(ctx.i, lm, q)
    ctx.remaining_bits -= curr_bits
    while ctx.remaining_bits < 0 and q > 0:
        ctx.remaining_bits += curr_bits
        q -= 1
        curr_bits = pulses2bits(ctx.i, lm, q)
        ctx.remaining_bits -= curr_bits
    if q != 0:
        return alg_unquant(buf, off, n, get_pulses(q), ctx.spread, blocks, ctx.ec, gain)
    if ctx.resynth:
        cm_mask = (1 << blocks) - 1
        fill &= cm_mask
        if not fill:
            for j in range(n):
                buf[off + j] = f32(0.0)
            return 0
        if low is None:
            for j in range(n):
                ctx.seed = lcg(ctx.seed)
                buf[off + j] = f32(float(s32(ctx.seed) >> 20))
            cm = cm_mask
        else:
            for j in range(n):
                ctx.seed = lcg(ctx.seed)
                tmp = f32(1.0 / 256)
                if ctx.seed & 0x8000:
                    tmp = f32(-tmp)
                buf[off + j] = f32(low[low_off + j] + tmp)
            cm = fill
        renormalise_vector(buf, off, n, gain)
        return cm
    return 0


def quant_band(ctx, buf, off, n, b, blocks, low, low_off, lm, low_out, low_out_off, gain, scratch, scratch_off, fill) -> int:
    n0 = n
    if n == 1:
        return quant_band_n1(ctx, buf, off, low_out, low_out_off)
    tf_change = ctx.tf_change
    b0_orig = blocks
    long_blocks = 1 if b0_orig == 1 else 0
    n_b = n // blocks
    recombine = tf_change if tf_change > 0 else 0
    if scratch is not None and low is not None and (recombine or ((n_b & 1) == 0 and tf_change < 0) or b0_orig > 1):
        for j in range(n):
            scratch[scratch_off + j] = low[low_off + j]
        low = scratch
        low_off = scratch_off
    for _k in range(recombine):
        if low is not None:
            haar1(low, low_off, n >> _k, 1 << _k)
        fill = BIT_INTERLEAVE[fill & 0xF] | (BIT_INTERLEAVE[fill >> 4] << 2)
    blocks >>= recombine
    n_b <<= recombine
    time_divide = 0
    while (n_b & 1) == 0 and tf_change < 0:
        if low is not None:
            haar1(low, low_off, n_b, blocks)
        fill |= fill << blocks
        blocks <<= 1
        n_b >>= 1
        time_divide += 1
        tf_change += 1
    b0 = blocks
    n_b0 = n_b
    if b0 > 1 and low is not None:
        deinterleave_hadamard(low, low_off, n_b >> recombine, b0 << recombine, long_blocks)
    cm = quant_partition(ctx, buf, off, n, b, blocks, low, low_off, lm, gain, fill)
    if ctx.resynth:
        if b0 > 1:
            interleave_hadamard(buf, off, n_b >> recombine, b0 << recombine, long_blocks)
        n_b = n_b0
        blocks = b0
        for _k in range(time_divide):
            blocks >>= 1
            n_b <<= 1
            cm |= cm >> blocks
            haar1(buf, off, n_b, blocks)
        for k in range(recombine):
            cm = BIT_DEINTERLEAVE[cm & 0xF]
            haar1(buf, off, n0 >> k, 1 << k)
        blocks <<= recombine
        if low_out is not None:
            norm = celt_sqrt(f32(float(n0)))
            for j in range(n0):
                low_out[low_out_off + j] = f32(norm * buf[off + j])
        cm &= (1 << blocks) - 1
    return cm


def quant_all_bands(dec, start, end, x, pulses, short_blocks, spread, dual_stereo, intensity,
                    tf_res, total_bits, balance, lm, coded_bands, seed):
    m = 1 << lm
    blocks0 = m if short_blocks else 1
    norm_offset = m * EBANDS[start]
    norm_len = m * EBANDS[NB_EBANDS - 1] - norm_offset
    norm = [f32(0.0)] * max(norm_len, 1)
    scratch_off = m * EBANDS[NB_EBANDS - 1]
    lowband_offset = 0
    update_lowband = 1
    ctx = type("C", (), {})()
    ctx.encode = 0
    ctx.resynth = 1
    ctx.intensity = intensity
    ctx.spread = spread
    ctx.ec = dec
    ctx.seed = seed & 0xFFFFFFFF
    ctx.disable_inv = 1
    ctx.avoid_split_noise = 1 if blocks0 > 1 else 0
    collapse = [0] * NB_EBANDS
    for i in range(start, end):
        ctx.i = i
        last = i == end - 1
        xoff = m * EBANDS[i]
        n = m * EBANDS[i + 1] - xoff
        tell = dec.tell_frac()
        if i != start:
            balance -= tell
        remaining = total_bits - tell - 1
        ctx.remaining_bits = remaining
        if i <= coded_bands - 1:
            curr_balance = cdiv(balance, min(3, coded_bands - i))
            b = max(0, min(16383, min(remaining + 1, pulses[i] + curr_balance)))
        else:
            b = 0
        if (m * EBANDS[i] - n >= m * EBANDS[start] or i == start + 1) and (update_lowband or lowband_offset == 0):
            lowband_offset = i
        if i == start + 1:
            # special_hybrid_folding copies nothing for the standard 48 kHz edges at band 0.
            n1 = m * (EBANDS[start + 1] - EBANDS[start])
            n2 = m * (EBANDS[start + 2] - EBANDS[start + 1])
            if n2 > n1:
                for t in range(n2 - n1):
                    norm[n1 + t] = norm[2 * n1 - n2 + t]
        tf_change = tf_res[i]
        ctx.tf_change = tf_change
        low_scratch = None if last else x
        low_scratch_off = 0 if last else scratch_off
        if lowband_offset != 0 and (spread != SPREAD_AGGRESSIVE or blocks0 > 1 or tf_change < 0):
            effective = max(0, m * EBANDS[lowband_offset] - norm_offset - n)
            fold_start = lowband_offset
            while True:
                fold_start -= 1
                if not (m * EBANDS[fold_start] > effective + norm_offset):
                    break
            fold_end = lowband_offset - 1
            while True:
                fold_end += 1
                if not (fold_end < i and m * EBANDS[fold_end] < effective + norm_offset + n):
                    break
            x_cm = 0
            fold_i = fold_start
            while True:
                x_cm |= collapse[fold_i]
                fold_i += 1
                if not (fold_i < fold_end):
                    break
            low = norm
            low_off = effective
        else:
            x_cm = (1 << blocks0) - 1
            effective = -1
            low = None
            low_off = 0
        out = None if last else norm
        out_off = 0 if last else m * EBANDS[i] - norm_offset
        cm = quant_band(ctx, x, xoff, n, b, blocks0, low, low_off, lm, out, out_off,
                        f32(1.0), low_scratch, low_scratch_off, x_cm)
        collapse[i] = cm & 0xFF
        balance += pulses[i] + tell
        update_lowband = 1 if b > (n << BITRES) else 0
        ctx.avoid_split_noise = 0
    return collapse, ctx.seed, balance


def anti_collapse(x, collapse, lm, n, start, end, log_e, prev1, prev2, pulses, seed):
    m = 1 << lm
    for i in range(start, end):
        n0 = EBANDS[i + 1] - EBANDS[i]
        depth = ((1 + pulses[i]) // n0) >> lm
        thresh = f32(f32(0.5) * celt_exp2(f32(f32(-0.125) * f32(float(depth)))))
        sqrt_1 = celt_rsqrt(f32(float(n0 << lm)))
        p1 = fmax(prev1[i], prev1[NB_EBANDS + i])
        p2 = fmax(prev2[i], prev2[NB_EBANDS + i])
        ediff = f32(log_e[i] - fmin(p1, p2))
        if ediff < 0.0:
            ediff = f32(0.0)
        r = f32(f32(2.0) * celt_exp2(f32(-ediff)))
        if lm == 3:
            r = f32(r * SQRT2)
        if thresh < r:
            r = thresh
        r = f32(r * sqrt_1)
        base = EBANDS[i] << lm
        renormalize = 0
        mask = collapse[i]
        for k in range(m):
            if mask & (1 << k) == 0:
                for j in range(n0):
                    seed = lcg(seed)
                    val = r if seed & 0x8000 else f32(-r)
                    x[base + (j << lm) + k] = val
                renormalize = 1
        if renormalize:
            renormalise_vector(x, base, n0 << lm, f32(1.0))
    return seed


def cmul(a, b):
    ar, ai = a
    br, bi = b
    return (f32(f32(ar * br) - f32(ai * bi)), f32(f32(ar * bi) + f32(ai * br)))


def cadd(a, b):
    return (f32(a[0] + b[0]), f32(a[1] + b[1]))


def csub(a, b):
    return (f32(a[0] - b[0]), f32(a[1] - b[1]))


def kf_bfly2_at(fout, m, n, base):
    tw = FFT_C
    f = base
    for _i in range(n):
        t0 = fout[f + 4]
        fout[f + 4] = csub(fout[f], t0)
        fout[f] = cadd(fout[f], t0)
        s = fout[f + 5]
        tr = f32(f32(s[0] + s[1]) * tw)
        ti = f32(f32(s[1] - s[0]) * tw)
        t = (tr, ti)
        fout[f + 5] = csub(fout[f + 1], t)
        fout[f + 1] = cadd(fout[f + 1], t)
        s = fout[f + 6]
        t = (s[1], f32(-s[0]))
        fout[f + 6] = csub(fout[f + 2], t)
        fout[f + 2] = cadd(fout[f + 2], t)
        s = fout[f + 7]
        tr = f32(f32(s[1] - s[0]) * tw)
        ti = f32(f32(-f32(s[1] + s[0])) * tw)
        t = (tr, ti)
        fout[f + 7] = csub(fout[f + 3], t)
        fout[f + 3] = cadd(fout[f + 3], t)
        f += 8


def kf_bfly4_at(fout, fstride, m, n, mm, twiddles):
    if m == 1:
        f = 0
        for _i in range(n):
            a0 = fout[f]
            a1 = fout[f + 1]
            a2 = fout[f + 2]
            a3 = fout[f + 3]
            s0 = csub(a0, a2)
            n0 = cadd(a0, a2)
            s1 = cadd(a1, a3)
            f2 = csub(n0, s1)
            n0 = cadd(n0, s1)
            s1 = csub(a1, a3)
            fout[f] = n0
            fout[f + 2] = f2
            fout[f + 1] = (f32(s0[0] + s1[1]), f32(s0[1] - s1[0]))
            fout[f + 3] = (f32(s0[0] - s1[1]), f32(s0[1] + s1[0]))
            f += 4
        return
    for i in range(n):
        f = i * mm
        t1 = t2 = t3 = 0
        for _j in range(m):
            a0 = fout[f]
            s0 = cmul(fout[f + m], twiddles[t1])
            s1 = cmul(fout[f + 2 * m], twiddles[t2])
            s2 = cmul(fout[f + 3 * m], twiddles[t3])
            s5 = csub(a0, s1)
            n0 = cadd(a0, s1)
            s3 = cadd(s0, s2)
            s4 = csub(s0, s2)
            f2 = csub(n0, s3)
            t1 += fstride
            t2 += fstride * 2
            t3 += fstride * 3
            n0 = cadd(n0, s3)
            fout[f] = n0
            fout[f + 2 * m] = f2
            fout[f + m] = (f32(s5[0] + s4[1]), f32(s5[1] - s4[0]))
            fout[f + 3 * m] = (f32(s5[0] - s4[1]), f32(s5[1] + s4[0]))
            f += 1


def kf_bfly3_at(fout, fstride, m, n, mm, twiddles):
    epi = twiddles[fstride * m][1]
    for i in range(n):
        f = i * mm
        t1 = t2 = 0
        k = m
        while k:
            a0 = fout[f]
            s1 = cmul(fout[f + m], twiddles[t1])
            s2 = cmul(fout[f + 2 * m], twiddles[t2])
            s3 = cadd(s1, s2)
            s0 = csub(s1, s2)
            fout[f + m] = (f32(a0[0] - f32(s3[0] * f32(0.5))), f32(a0[1] - f32(s3[1] * f32(0.5))))
            s0 = (f32(s0[0] * epi), f32(s0[1] * epi))
            n0 = cadd(a0, s3)
            fout[f] = n0
            fm = fout[f + m]
            fout[f + 2 * m] = (f32(fm[0] + s0[1]), f32(fm[1] - s0[0]))
            fout[f + m] = (f32(fm[0] - s0[1]), f32(fm[1] + s0[0]))
            t1 += fstride
            t2 += fstride * 2
            f += 1
            k -= 1


def kf_bfly5_at(fout, fstride, m, n, mm, twiddles):
    ya = twiddles[fstride * m]
    yb = twiddles[fstride * 2 * m]
    for i in range(n):
        f = i * mm
        for u in range(m):
            s0 = fout[f]
            s1 = cmul(fout[f + m], twiddles[u * fstride])
            s2 = cmul(fout[f + 2 * m], twiddles[2 * u * fstride])
            s3 = cmul(fout[f + 3 * m], twiddles[3 * u * fstride])
            s4 = cmul(fout[f + 4 * m], twiddles[4 * u * fstride])
            s7 = cadd(s1, s4)
            s10 = csub(s1, s4)
            s8 = cadd(s2, s3)
            s9 = csub(s2, s3)
            fout[f] = (f32(s0[0] + f32(s7[0] + s8[0])), f32(s0[1] + f32(s7[1] + s8[1])))
            s5r = f32(s0[0] + f32(f32(s7[0] * ya[0]) + f32(s8[0] * yb[0])))
            s5i = f32(s0[1] + f32(f32(s7[1] * ya[0]) + f32(s8[1] * yb[0])))
            s6r = f32(f32(s10[1] * ya[1]) + f32(s9[1] * yb[1]))
            s6i = f32(-f32(f32(s10[0] * ya[1]) + f32(s9[0] * yb[1])))
            fout[f + m] = (f32(s5r - s6r), f32(s5i - s6i))
            fout[f + 4 * m] = (f32(s5r + s6r), f32(s5i + s6i))
            s11r = f32(s0[0] + f32(f32(s7[0] * yb[0]) + f32(s8[0] * ya[0])))
            s11i = f32(s0[1] + f32(f32(s7[1] * yb[0]) + f32(s8[1] * ya[0])))
            s12r = f32(f32(s9[1] * ya[1]) - f32(s10[1] * yb[1]))
            s12i = f32(f32(s10[0] * yb[1]) - f32(s9[0] * ya[1]))
            fout[f + 2 * m] = (f32(s11r + s12r), f32(s11i + s12i))
            fout[f + 3 * m] = (f32(s11r - s12r), f32(s11i - s12i))
            f += 1


def fft_impl(state: int, fout: list) -> None:
    factors = T.FFT_FACTORS[state]
    shift = T.FFT_SHIFT[state]
    if shift < 0:
        shift = 0
    twiddles = T.FFT_TWIDDLES
    fstride = [1]
    level = 0
    while True:
        p = factors[2 * level]
        m = factors[2 * level + 1]
        fstride.append(fstride[level] * p)
        level += 1
        if m == 1:
            break
    m = factors[2 * level - 1]
    for i in range(level - 1, -1, -1):
        m2 = factors[2 * i - 1] if i != 0 else 1
        radix = factors[2 * i]
        if radix == 2:
            kf_bfly2_at(fout, m, fstride[i], 0)
        elif radix == 4:
            kf_bfly4_at(fout, fstride[i] << shift, m, fstride[i], m2, twiddles)
        elif radix == 3:
            kf_bfly3_at(fout, fstride[i] << shift, m, fstride[i], m2, twiddles)
        elif radix == 5:
            kf_bfly5_at(fout, fstride[i] << shift, m, fstride[i], m2, twiddles)
        m = m2


def mdct_backward(freq, foff, out, ooff, shift, stride):
    n = 1920
    trig_off = 0
    for _ in range(shift):
        n >>= 1
        trig_off += n
    n2 = n >> 1
    n4 = n >> 2
    trig = T.MDCT_TWIDDLES960
    bitrev = T.FFT_BITREV[shift]
    base = ooff + (OVERLAP >> 1)
    xp1 = 0
    xp2 = stride * (n2 - 1)
    for i in range(n4):
        rev = bitrev[i]
        x1 = freq[foff + xp1]
        x2 = freq[foff + xp2]
        ti = trig[trig_off + i]
        tn = trig[trig_off + n4 + i]
        yr = f32(f32(x2 * ti) + f32(x1 * tn))
        yi = f32(f32(x1 * ti) - f32(x2 * tn))
        out[base + 2 * rev + 1] = yr
        out[base + 2 * rev] = yi
        xp1 += 2 * stride
        xp2 -= 2 * stride
    fout = [(out[base + 2 * i], out[base + 2 * i + 1]) for i in range(n4)]
    fft_impl(shift, fout)
    for i in range(n4):
        out[base + 2 * i] = fout[i][0]
        out[base + 2 * i + 1] = fout[i][1]
    yp0 = base
    yp1 = base + n2 - 2
    for i in range((n4 + 1) >> 1):
        re = out[yp0 + 1]
        im = out[yp0]
        t0 = trig[trig_off + i]
        t1 = trig[trig_off + n4 + i]
        yr = f32(f32(re * t0) + f32(im * t1))
        yi = f32(f32(re * t1) - f32(im * t0))
        re = out[yp1 + 1]
        im = out[yp1]
        out[yp0] = yr
        out[yp1 + 1] = yi
        t0 = trig[trig_off + (n4 - i - 1)]
        t1 = trig[trig_off + (n2 - i - 1)]
        yr = f32(f32(re * t0) + f32(im * t1))
        yi = f32(f32(re * t1) - f32(im * t0))
        out[yp1] = yr
        out[yp0 + 1] = yi
        yp0 += 2
        yp1 -= 2
    xp = ooff + OVERLAP - 1
    yp = ooff
    w1 = 0
    w2 = OVERLAP - 1
    for _i in range(OVERLAP // 2):
        x1 = out[xp]
        x2 = out[yp]
        a = WINDOW[w1]
        b = WINDOW[w2]
        out[yp] = f32(f32(b * x2) - f32(a * x1))
        out[xp] = f32(f32(a * x2) + f32(b * x1))
        yp += 1
        xp -= 1
        w1 += 1
        w2 -= 1


def denormalise_bands(x, freq, band_log_e, start, end, m, silence):
    n = m * SHORT_MDCT
    if silence:
        for i in range(n):
            freq[i] = f32(0.0)
        return
    bound = m * EBANDS[end]
    for i in range(m * EBANDS[start]):
        freq[i] = f32(0.0)
    pos = m * EBANDS[start]
    for i in range(start, end):
        lg = f32(band_log_e[i] + EMEANS[i])
        if f32(32.0) < lg:
            lg = f32(32.0)
        g = celt_exp2(lg)
        band_end = m * EBANDS[i + 1]
        while pos < band_end:
            freq[pos] = f32(x[pos] * g)
            pos += 1
    for i in range(bound, n):
        freq[i] = f32(0.0)


def comb_filter(buf, off, t0, t1, n, g0, g1, tap0, tap1, overlap):
    if g0 == 0.0 and g1 == 0.0:
        return
    if t0 < COMBFILTER_MINPERIOD:
        t0 = COMBFILTER_MINPERIOD
    if t1 < COMBFILTER_MINPERIOD:
        t1 = COMBFILTER_MINPERIOD
    g00 = f32(g0 * GAINS[tap0][0])
    g01 = f32(g0 * GAINS[tap0][1])
    g02 = f32(g0 * GAINS[tap0][2])
    g10 = f32(g1 * GAINS[tap1][0])
    g11 = f32(g1 * GAINS[tap1][1])
    g12 = f32(g1 * GAINS[tap1][2])
    x1 = buf[off - t1 + 1]
    x2 = buf[off - t1]
    x3 = buf[off - t1 - 1]
    x4 = buf[off - t1 - 2]
    if g0 == g1 and t0 == t1 and tap0 == tap1:
        overlap = 0
    for i in range(overlap):
        x0 = buf[off + i - t1 + 2]
        f = f32(WINDOW[i] * WINDOW[i])
        om = f32(f32(1.0) - f)
        acc = buf[off + i]
        acc = f32(acc + f32(f32(om * g00) * buf[off + i - t0]))
        acc = f32(acc + f32(f32(om * g01) * f32(buf[off + i - t0 + 1] + buf[off + i - t0 - 1])))
        acc = f32(acc + f32(f32(om * g02) * f32(buf[off + i - t0 + 2] + buf[off + i - t0 - 2])))
        acc = f32(acc + f32(f32(f * g10) * x2))
        acc = f32(acc + f32(f32(f * g11) * f32(x1 + x3)))
        acc = f32(acc + f32(f32(f * g12) * f32(x0 + x4)))
        buf[off + i] = acc
        x4, x3, x2, x1 = x3, x2, x1, x0
    if g1 == 0.0:
        return
    # comb_filter_const re-reads the delay line at the end of the overlap.
    base = off + overlap
    x4 = buf[base - t1 - 2]
    x3 = buf[base - t1 - 1]
    x2 = buf[base - t1]
    x1 = buf[base - t1 + 1]
    for j in range(n - overlap):
        x0 = buf[base + j - t1 + 2]
        acc = buf[base + j]
        acc = f32(acc + f32(g10 * x2))
        acc = f32(acc + f32(g11 * f32(x1 + x3)))
        acc = f32(acc + f32(g12 * f32(x0 + x4)))
        buf[base + j] = acc
        x4, x3, x2, x1 = x3, x2, x1, x0


def soft_clip(pcm: list) -> None:
    n = len(pcm)
    for i in range(n):
        v = pcm[i]
        if v > f32(2.0):
            v = f32(2.0)
        if v < f32(-2.0):
            v = f32(-2.0)
        pcm[i] = v
    a = f32(0.0)
    i = 0
    while i < n and f32(pcm[i] * a) < 0.0:
        v = pcm[i]
        pcm[i] = f32(v + f32(f32(a * v) * v))
        i += 1
    curr = 0
    x0 = pcm[0]
    while True:
        i = curr
        while i < n and not (pcm[i] > f32(1.0) or pcm[i] < f32(-1.0)):
            i += 1
        if i == n:
            break
        peak = i
        start = end = i
        maxval = abs(pcm[i])
        while start > 0 and f32(pcm[i] * pcm[start - 1]) >= 0.0:
            start -= 1
        while end < n and f32(pcm[i] * pcm[end]) >= 0.0:
            av = abs(pcm[end])
            if av > maxval:
                maxval = av
                peak = end
            end += 1
        special = start == 0 and f32(pcm[i] * pcm[0]) >= 0.0
        aa = f32(f32(maxval - f32(1.0)) / f32(maxval * maxval))
        aa = f32(aa + f32(aa * f32(2.4e-7)))
        if pcm[i] > 0.0:
            aa = f32(-aa)
        for j in range(start, end):
            v = pcm[j]
            pcm[j] = f32(v + f32(f32(aa * v) * v))
        if special and peak >= 2:
            offset = f32(x0 - pcm[0])
            delta = f32(offset / f32(float(peak)))
            for j in range(curr, peak):
                offset = f32(offset - delta)
                v = f32(pcm[j] + offset)
                if v > f32(1.0):
                    v = f32(1.0)
                if v < f32(-1.0):
                    v = f32(-1.0)
                pcm[j] = v
        curr = end
        if curr == n:
            break


def float_to_int16(y: float) -> int:
    x = f32(y * SCALE_32768)
    if not (x > f32(-32768.0)):
        x = f32(-32768.0)
    if not (x < f32(32767.0)):
        x = f32(32767.0)
    return int(round(x))


def celt_decode(payload: bytes) -> bytes:
    dec = EcDec(payload)
    start, end, lm, c = 0, 13, 2, 1
    total_bits = len(payload) * 8
    tell = dec.tell()
    if tell >= total_bits:
        silence = 1
    elif tell == 1:
        silence = dec.bit_logp(15)
    else:
        silence = 0
    if silence:
        dec.nbits_total += len(payload) * 8 - dec.tell()
    post_gain = f32(0.0)
    post_pitch = 0
    post_tap = 0
    if start == 0 and dec.tell() + 16 <= total_bits:
        if dec.bit_logp(1):
            octave = dec.uint(6)
            post_pitch = (16 << octave) + dec.bits(4 + octave) - 1
            qg = dec.bits(3)
            if dec.tell() + 2 <= total_bits:
                post_tap = dec.icdf(TAPSET_ICDF, 2)
            post_gain = f32(POSTFILTER_STEP * f32(float(qg + 1)))
    if lm > 0 and dec.tell() + 3 <= total_bits:
        is_transient = dec.bit_logp(3)
    else:
        is_transient = 0
    intra = dec.bit_logp(3) if dec.tell() + 3 <= total_bits else 0
    old_e = [f32(0.0)] * (2 * NB_EBANDS)
    old_log = [NEG28] * (2 * NB_EBANDS)
    old_log2 = [NEG28] * (2 * NB_EBANDS)
    for i in range(NB_EBANDS):
        old_e[i] = fmax(old_e[i], old_e[NB_EBANDS + i])
    unquant_coarse_energy(dec, start, end, old_e, intra, lm)
    tf_res = tf_decode(dec, start, end, is_transient, lm)
    spread = SPREAD_NORMAL
    if dec.tell() + 4 <= total_bits:
        spread = dec.icdf(SPREAD_ICDF, 5)
    cap = [0] * NB_EBANDS
    init_caps(lm, cap)
    offsets = [0] * NB_EBANDS
    dyn = 6
    total_frac = total_bits << BITRES
    tell_f = dec.tell_frac()
    for i in range(start, end):
        width = (EBANDS[i + 1] - EBANDS[i]) << lm
        quanta = min(width << BITRES, max(6 << BITRES, width))
        loop = dyn
        boost = 0
        while tell_f + (loop << BITRES) < total_frac and boost < cap[i]:
            flag = dec.bit_logp(loop)
            tell_f = dec.tell_frac()
            if not flag:
                break
            boost += quanta
            total_frac -= quanta
            loop = 1
        offsets[i] = boost
        if boost > 0:
            dyn = max(2, dyn - 1)
    alloc_trim = dec.icdf(TRIM_ICDF, 7) if dec.tell_frac() + (6 << BITRES) <= total_frac else 5
    bits = ((len(payload) * 8) << BITRES) - dec.tell_frac() - 1
    anti_rsv = (1 << BITRES) if is_transient and lm >= 2 and bits >= ((lm + 2) << BITRES) else 0
    bits -= anti_rsv
    coded, balance, intensity, dual, pulses, fine, prio = clt_compute_allocation(
        start, end, offsets, cap, alloc_trim, bits, c, lm, dec)
    unquant_fine_energy(dec, start, end, old_e, fine)
    m = 1 << lm
    n = m * SHORT_MDCT
    x = [f32(0.0)] * n
    collapse, seed, _balance = quant_all_bands(
        dec, start, end, x, pulses, m if is_transient else 0, spread, dual, intensity,
        tf_res, len(payload) * (8 << BITRES) - anti_rsv, balance, lm, coded, 0)
    anti_on = dec.bits(1) if anti_rsv > 0 else 0
    bits_left = len(payload) * 8 - dec.tell()
    unquant_energy_finalise(dec, start, end, old_e, fine, prio, bits_left)
    if anti_on:
        anti_collapse(x, collapse, lm, n, start, end, old_e, old_log, old_log2, pulses, seed)
    if silence:
        for i in range(c * NB_EBANDS):
            old_e[i] = NEG28
    freq = [f32(0.0)] * n
    denormalise_bands(x, freq, old_e, start, end, m, silence)
    mem = [f32(0.0)] * (DECODE_BUFFER_SIZE + OVERLAP)
    out_off = DECODE_BUFFER_SIZE - n
    if is_transient:
        blocks = m
        nb = SHORT_MDCT
        shift = MAX_LM
    else:
        blocks = 1
        nb = n
        shift = MAX_LM - lm
    for b in range(blocks):
        mdct_backward(freq, b, mem, out_off + nb * b, shift, blocks)
    # Fresh decoder: both postfilter gains are zero, so comb_filter is a copy.
    comb_filter(mem, out_off, COMBFILTER_MINPERIOD, COMBFILTER_MINPERIOD, SHORT_MDCT,
                f32(0.0), f32(0.0), 0, 0, OVERLAP)
    if lm != 0:
        comb_filter(mem, out_off + SHORT_MDCT, COMBFILTER_MINPERIOD, post_pitch, n - SHORT_MDCT,
                    f32(0.0), post_gain, 0, post_tap, OVERLAP)
    pcm = [f32(0.0)] * n
    mem_d = f32(0.0)
    for j in range(n):
        tmp = f32(f32(mem[out_off + j] + VERY_SMALL) + mem_d)
        mem_d = f32(PREEMPH * tmp)
        pcm[j] = f32(tmp * INV_32768)
    soft_clip(pcm)
    # A finished packet still pulls one implicit zero during the last
    # renormalisation (libopus does too, and does not fail the frame).
    # More than that, or a raw-bit read off the end, means the payload was short.
    if dec.error or dec.tell() > 8 * len(payload) or dec.front_past > 1 or dec.end_past:
        raise Fault("INTEGRITY_FAIL", "range coder overran the payload")
    out = bytearray()
    for sample in pcm:
        out += struct.pack("<h", float_to_int16(sample))
    return bytes(out)


def decode(data: bytes) -> bytes:
    """480 s16le samples. Empty -> Fault('BOUNDS'). Short/corrupt -> Fault('INTEGRITY_FAIL'). Other TOC -> Fault('UNSUPPORTED')."""
    if len(data) == 0:
        raise Fault("BOUNDS", "empty input")
    toc = data[0]
    config = toc >> 3
    stereo = (toc >> 2) & 1
    code = toc & 3
    if config != 18 or stereo != 0 or code != 0:
        raise Fault("UNSUPPORTED", "only TOC config 18 mono code 0 is supported")
    try:
        return celt_decode(data[1:])
    except Fault:
        raise
    except Exception as exc:
        raise Fault("INTEGRITY_FAIL", "payload could not be decoded") from exc
