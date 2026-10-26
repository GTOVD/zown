//! AV1 still-picture decode, translated from `zown/av1.py`.
//!
//! Integer arithmetic follows that module: the same shifts, clips, CDF
//! adaptation, and MSAC. Output is 8-bit I420 (Y then U then V), or luma
//! alone when the sequence is monochrome.

// The module is private until a word is wired. Keep the translated decoder
// itself from looking unused to the non-test build.
#![allow(dead_code)]

#[path = "av1data.rs"]
mod av1data;
#[path = "av1enc.rs"]
mod av1enc;

pub use av1enc::encode;

use super::error::{BOUNDS, INTEGRITY_FAIL, UNSUPPORTED};

const MASK64: u64 = u64::MAX;
const I16N: i64 = -32768;
const I16P: i64 = 32767;

const BL_64: usize = 1;
const PART_N: [usize; 5] = [0, 9, 9, 9, 3];

const NONE: i32 = 0;
const H: i32 = 1;
const V: i32 = 2;
const SPLIT: i32 = 3;
const DC: i32 = 0;
const VERT: i32 = 1;
const HOR: i32 = 2;
const CFL: i32 = 13;
const DCT_DCT: i32 = 0;
const ADST_DCT: i32 = 1;
const DCT_ADST: i32 = 2;
const ADST_ADST: i32 = 3;
const IDTX: i32 = 9;
const TX_4: i32 = 0;
const TX_8: i32 = 1;
const TX_16: i32 = 2;
const V_DCT: i32 = 10;
const H_DCT: i32 = 11;
const WHT_WHT: i32 = 16;

const INTRA_CTX: [usize; 13] = [0, 1, 2, 3, 4, 4, 4, 4, 3, 0, 1, 2, 0];
const FILTER_TO_Y: [i32; 5] = [0, 1, 2, 6, 0];
const SKIP_CTX: [[usize; 5]; 5] = [
    [1, 2, 2, 2, 3],
    [2, 4, 4, 4, 5],
    [2, 4, 4, 4, 5],
    [2, 4, 4, 4, 5],
    [3, 5, 5, 5, 6],
];
const TX_TYPES_INTRA1: [i32; 7] = [IDTX, DCT_DCT, V_DCT, H_DCT, ADST_ADST, ADST_DCT, DCT_ADST];
const TX_TYPES_INTRA2: [i32; 5] = [IDTX, DCT_DCT, ADST_ADST, ADST_DCT, DCT_ADST];
const UV_TXTP: [i32; 13] = [
    DCT_DCT, ADST_DCT, DCT_ADST, DCT_DCT, ADST_ADST, ADST_DCT, DCT_ADST, DCT_ADST, ADST_DCT,
    ADST_ADST, ADST_DCT, DCT_ADST, ADST_ADST,
];
const LO_CTX: [[[i32; 5]; 5]; 3] = [
    [
        [0, 1, 6, 6, 21],
        [1, 6, 6, 21, 21],
        [6, 6, 21, 21, 21],
        [6, 21, 21, 21, 21],
        [21, 21, 21, 21, 21],
    ],
    [
        [0, 16, 6, 6, 21],
        [16, 16, 6, 21, 21],
        [16, 16, 21, 21, 21],
        [16, 16, 21, 21, 21],
        [16, 16, 21, 21, 21],
    ],
    [
        [0, 11, 11, 11, 11],
        [11, 11, 11, 11, 11],
        [6, 6, 21, 21, 21],
        [6, 21, 21, 21, 21],
        [21, 21, 21, 21, 21],
    ],
];

type Av1Result<T> = Result<T, (&'static str, String)>;

fn fault<T>(code: &'static str, msg: impl Into<String>) -> Av1Result<T> {
    Err((code, msg.into()))
}

fn clip8(v: i64) -> u8 {
    if v < 0 {
        0
    } else if v > 255 {
        255
    } else {
        v as u8
    }
}

fn clip16(v: i64) -> i64 {
    if v < I16N {
        I16N
    } else if v > I16P {
        I16P
    } else {
        v
    }
}

fn bit_len(n: usize) -> u32 {
    usize::BITS - n.leading_zeros()
}

struct Bits<'a> {
    data: &'a [u8],
    i: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, i: 0 }
    }

    fn u(&mut self, n: usize) -> Av1Result<u32> {
        let mut v = 0u32;
        for _ in 0..n {
            if self.i >= self.data.len() * 8 {
                return fault(INTEGRITY_FAIL, "truncated AV1 header");
            }
            let byte = self.data[self.i >> 3];
            v = (v << 1) | u32::from((byte >> (7 - (self.i & 7))) & 1);
            self.i += 1;
        }
        Ok(v)
    }

    fn su(&mut self, n: usize) -> Av1Result<i32> {
        let v = self.u(n)?;
        let sign = v >> (n - 1);
        let mag = v & ((1u32 << (n - 1)) - 1);
        if sign != 0 {
            Ok(-(mag as i32))
        } else {
            Ok(mag as i32)
        }
    }

    fn align(&mut self) -> Av1Result<()> {
        while self.i & 7 != 0 {
            if self.u(1)? != 0 {
                return fault(INTEGRITY_FAIL, "AV1 alignment bit is not zero");
            }
        }
        Ok(())
    }

    fn trailing(&mut self) -> Av1Result<()> {
        if self.i >= self.data.len() * 8 {
            return Ok(());
        }
        if self.u(1)? != 1 {
            return fault(INTEGRITY_FAIL, "AV1 trailing bit is not one");
        }
        while self.i & 7 != 0 {
            if self.u(1)? != 0 {
                return fault(INTEGRITY_FAIL, "AV1 trailing padding is not zero");
            }
        }
        if self.i != self.data.len() * 8 {
            return fault(INTEGRITY_FAIL, "AV1 header did not consume its payload");
        }
        Ok(())
    }
}

fn shl_u64(val: u64, c: i32) -> u64 {
    if c <= 0 {
        if c == 0 {
            val
        } else {
            0
        }
    } else if c >= 64 {
        0
    } else {
        val << c
    }
}

fn starved_mask(c: i32) -> u64 {
    if c >= 56 {
        MASK64
    } else if c >= 0 {
        (1u64 << (c + 8)) - 1
    } else {
        0
    }
}

struct Msac<'a> {
    buf: &'a [u8],
    pos: usize,
    end: usize,
    dif: u64,
    rng: u64,
    cnt: i32,
    update: bool,
    starved: bool,
}

impl<'a> Msac<'a> {
    fn new(data: &'a [u8], update: bool) -> Self {
        let mut ms = Self {
            buf: data,
            pos: 0,
            end: data.len(),
            dif: 0,
            rng: 0x8000,
            cnt: -15,
            update,
            starved: false,
        };
        ms.refill();
        ms
    }

    fn refill(&mut self) {
        let mut c = 64 - self.cnt - 24;
        let mut dif = self.dif;
        loop {
            if self.pos >= self.end {
                dif |= starved_mask(c);
                self.starved = true;
                break;
            }
            let b = u64::from(self.buf[self.pos] ^ 0xFF);
            dif |= shl_u64(b, c);
            self.pos += 1;
            c -= 8;
            if c < 0 {
                break;
            }
        }
        self.dif = dif;
        self.cnt = 64 - c - 24;
    }

    fn norm(&mut self, dif: u64, rng: i128) -> Av1Result<()> {
        if !(0..=u64::MAX as i128).contains(&rng) {
            return fault(INTEGRITY_FAIL, "AV1 range went negative");
        }
        let rng_u = rng as u64;
        let bit_length = if rng_u == 0 {
            0
        } else {
            64 - rng_u.leading_zeros() as i32
        };
        let clz = 32 - bit_length;
        let d = 15 ^ (31 ^ clz);
        if d < 0 {
            return fault(INTEGRITY_FAIL, "AV1 range renormalisation failed");
        }
        let cnt = self.cnt;
        self.dif = shl_u64(dif, d);
        self.rng = shl_u64(rng_u, d);
        self.cnt = cnt - d;
        if (cnt as u32) < d as u32 {
            self.refill();
        }
        Ok(())
    }

    fn adapt_bool(&mut self, cdf: &mut [u16]) -> Av1Result<i32> {
        let bit = self.bool_bit(u32::from(cdf[0]))?;
        if self.update {
            let count = cdf[1];
            let rate = u32::from(4 + (count >> 4));
            let p = u32::from(cdf[0]);
            if bit != 0 {
                cdf[0] = (p + ((32768 - p) >> rate)) as u16;
            } else {
                cdf[0] = (p - (p >> rate)) as u16;
            }
            cdf[1] = count + u16::from(count < 32);
        }
        Ok(bit)
    }

    fn bool_bit(&mut self, f: u32) -> Av1Result<i32> {
        let r = self.rng as i128;
        let dif = self.dif as i128;
        let v = (((r >> 8) * (i128::from(f) >> 6)) >> 1) + 4;
        let vw = v << 48;
        let ret = if dif >= vw { 1i128 } else { 0 };
        let dif2 = (dif - ret * vw) as u64;
        let v2 = v + ret * (r - 2 * v);
        self.norm(dif2, v2)?;
        Ok((ret as i32) ^ 1)
    }

    fn equi(&mut self) -> Av1Result<i32> {
        let r = self.rng as i128;
        let dif = self.dif as i128;
        let v = ((r >> 8) << 7) + 4;
        let vw = v << 48;
        let ret = if dif >= vw { 1i128 } else { 0 };
        let dif2 = (dif - ret * vw) as u64;
        let v2 = v + ret * (r - 2 * v);
        self.norm(dif2, v2)?;
        Ok((ret as i32) ^ 1)
    }

    fn bools(&mut self, n: i32) -> Av1Result<i32> {
        let mut v = 0i32;
        for _ in 0..n {
            v = (v << 1) | self.equi()?;
        }
        Ok(v)
    }

    fn symbol(&mut self, cdf: &mut [u16], n: usize) -> Av1Result<i32> {
        let c = (self.dif >> 48) as i128;
        let r = (self.rng as i128) >> 8;
        let mut u;
        let mut v = self.rng as i128;
        let mut val: i32 = -1;
        loop {
            val += 1;
            if val < 0 || val as usize >= cdf.len() {
                return fault(INTEGRITY_FAIL, "AV1 symbol CDF overrun");
            }
            u = v;
            let vv = (r * (i128::from(cdf[val as usize]) >> 6)) >> 1;
            v = vv + 4 * (n as i128 - val as i128);
            if c >= v {
                break;
            }
        }
        let dif2 = ((self.dif as i128) - (v << 48)) as u64;
        self.norm(dif2, u - v)?;
        if self.update {
            let count = cdf[n];
            let rate = u32::from(4 + (count >> 4) + u16::from(n > 2));
            let val_us = val as usize;
            for i in 0..val_us.min(cdf.len()) {
                let p = u32::from(cdf[i]);
                cdf[i] = (p + ((32768 - p) >> rate)) as u16;
            }
            let end = n.min(cdf.len());
            if val_us < end {
                for i in val_us..end {
                    let p = u32::from(cdf[i]);
                    cdf[i] = (p - (p >> rate)) as u16;
                }
            }
            if n < cdf.len() {
                cdf[n] = count + u16::from(count < 32);
            }
        }
        Ok(val)
    }

    fn hi_tok(&mut self, cdf: &mut [u16]) -> Av1Result<i32> {
        let mut br = self.symbol(cdf, 3)?;
        let mut tok = 3 + br;
        if br == 3 {
            br = self.symbol(cdf, 3)?;
            tok = 6 + br;
            if br == 3 {
                br = self.symbol(cdf, 3)?;
                tok = 9 + br;
                if br == 3 {
                    tok = 12 + self.symbol(cdf, 3)?;
                }
            }
        }
        Ok(tok)
    }

    fn golomb(&mut self) -> Av1Result<i64> {
        let mut length = 0;
        while self.equi()? == 0 && length < 32 {
            length += 1;
        }
        let mut val = 1i64;
        for _ in 0..length {
            val = (val << 1) + i64::from(self.equi()?);
        }
        Ok(val - 1)
    }
}

fn leb(data: &[u8], mut i: usize) -> Av1Result<(usize, usize)> {
    let mut size: u64 = 0;
    let mut shift = 0;
    for _ in 0..8 {
        if i >= data.len() {
            return fault(INTEGRITY_FAIL, "truncated AV1 size");
        }
        let b = data[i];
        i += 1;
        size |= u64::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            return Ok((size as usize, i));
        }
        shift += 7;
    }
    fault(INTEGRITY_FAIL, "AV1 size is not a LEB128")
}

fn obus(data: &[u8]) -> Av1Result<Vec<(u8, Vec<u8>)>> {
    let mut i = 0;
    let mut out = Vec::new();
    while i < data.len() {
        let h = data[i];
        i += 1;
        if h & 0x80 != 0 {
            return fault(INTEGRITY_FAIL, "AV1 OBU forbidden bit is set");
        }
        let typ = (h >> 3) & 0xF;
        let ext = (h >> 2) & 1;
        let has = (h >> 1) & 1;
        if ext != 0 {
            if i >= data.len() {
                return fault(INTEGRITY_FAIL, "truncated AV1 OBU extension");
            }
            i += 1;
        }
        if has == 0 {
            return fault(UNSUPPORTED, "an AV1 OBU without a size is not in this slice");
        }
        let (size, ni) = leb(data, i)?;
        i = ni;
        if size > data.len() - i {
            return fault(INTEGRITY_FAIL, "truncated AV1 OBU");
        }
        out.push((typ, data[i..i + size].to_vec()));
        i += size;
    }
    Ok(out)
}

struct Frame {
    w: usize,
    h: usize,
    ssx: usize,
    ssy: usize,
    mono: bool,
    base_q: i32,
    ydc: i32,
    udc: i32,
    uac: i32,
    tx_select: bool,
    reduced_tx: bool,
    screen: bool,
    filter_intra: bool,
    lf: [u32; 4],
    update: bool,
    lossless: bool,
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            w: 0,
            h: 0,
            ssx: 1,
            ssy: 1,
            mono: false,
            base_q: 0,
            ydc: 0,
            udc: 0,
            uac: 0,
            tx_select: false,
            reduced_tx: false,
            screen: false,
            filter_intra: false,
            lf: [0; 4],
            update: true,
            lossless: false,
        }
    }
}

fn seq(payload: &[u8], fr: &mut Frame) -> Av1Result<()> {
    let mut b = Bits::new(payload);
    let profile = b.u(3)?;
    let still = b.u(1)?;
    let reduced = b.u(1)?;
    if profile != 0 || still == 0 || reduced == 0 {
        return fault(
            UNSUPPORTED,
            "AV1 sequence is not a profile-0 reduced still picture",
        );
    }
    if b.u(5)? > 31 {
        return fault(INTEGRITY_FAIL, "AV1 level is not a level");
    }
    let wb = b.u(4)? + 1;
    let hb = b.u(4)? + 1;
    fr.w = b.u(wb as usize)? as usize + 1;
    fr.h = b.u(hb as usize)? as usize + 1;
    if fr.w > 64 || fr.h > 64 {
        return fault(UNSUPPORTED, "AV1 frame is larger than 64 pixels in this slice");
    }
    if b.u(1)? != 0 {
        return fault(UNSUPPORTED, "128x128 superblocks are not in this slice");
    }
    fr.filter_intra = b.u(1)? != 0;
    let _edge = b.u(1)?;
    if b.u(1)? != 0 || b.u(1)? != 0 || b.u(1)? != 0 {
        return fault(
            UNSUPPORTED,
            "superres, CDEF, or restoration is not in this slice",
        );
    }
    if b.u(1)? != 0 {
        return fault(UNSUPPORTED, "high bit depth is not in this slice");
    }
    fr.mono = b.u(1)? != 0;
    if b.u(1)? != 0 {
        return fault(UNSUPPORTED, "a color description is not in this slice");
    }
    if fr.mono {
        let _ = b.u(1)?;
        fr.ssx = 1;
        fr.ssy = 1;
    } else {
        let _range = b.u(1)?;
        fr.ssx = 1;
        fr.ssy = 1;
        let _csp = b.u(2)?;
        if b.u(1)? != 0 {
            return fault(UNSUPPORTED, "separate UV delta-q is not in this slice");
        }
    }
    if b.u(1)? != 0 {
        return fault(UNSUPPORTED, "film grain is not in this slice");
    }
    b.trailing()
}

fn frame_header<'a>(payload: &'a [u8], fr: &mut Frame) -> Av1Result<&'a [u8]> {
    let mut b = Bits::new(payload);
    fr.update = b.u(1)? == 0;
    fr.screen = b.u(1)? != 0;
    if fr.screen {
        let _force = b.u(1)?;
    }
    if b.u(1)? != 0 {
        return fault(UNSUPPORTED, "a distinct render size is not in this slice");
    }
    let intrabc = if fr.screen { b.u(1)? != 0 } else { false };
    if intrabc {
        return fault(UNSUPPORTED, "intrabc is not in this slice");
    }
    if b.u(1)? == 0 {
        return fault(UNSUPPORTED, "non-uniform tiles are not in this slice");
    }
    fr.base_q = b.u(8)? as i32;
    fr.ydc = if b.u(1)? != 0 { b.su(7)? } else { 0 };
    if !fr.mono {
        fr.udc = if b.u(1)? != 0 { b.su(7)? } else { 0 };
        fr.uac = if b.u(1)? != 0 { b.su(7)? } else { 0 };
    }
    if b.u(1)? != 0 {
        return fault(UNSUPPORTED, "a quantisation matrix is not in this slice");
    }
    if b.u(1)? != 0 {
        return fault(UNSUPPORTED, "segmentation is not in this slice");
    }
    if fr.base_q > 0 && b.u(1)? != 0 {
        return fault(UNSUPPORTED, "delta-q is not in this slice");
    }
    fr.lossless = fr.base_q == 0 && fr.ydc == 0 && fr.udc == 0 && fr.uac == 0;
    if !fr.lossless {
        fr.lf[0] = b.u(6)?;
        fr.lf[1] = b.u(6)?;
        if !fr.mono && (fr.lf[0] != 0 || fr.lf[1] != 0) {
            fr.lf[2] = b.u(6)?;
            fr.lf[3] = b.u(6)?;
        }
        let _sharp = b.u(3)?;
        if b.u(1)? != 0 && b.u(1)? != 0 {
            for _ in 0..8 {
                if b.u(1)? != 0 {
                    let _ = b.su(7)?;
                }
            }
            for _ in 0..2 {
                if b.u(1)? != 0 {
                    let _ = b.su(7)?;
                }
            }
        }
    }
    fr.tx_select = if fr.lossless { false } else { b.u(1)? != 0 };
    fr.reduced_tx = b.u(1)? != 0;
    b.align()?;
    let used = b.i >> 3;
    Ok(&payload[used..])
}

fn qindex(base: i32, delta: i32) -> usize {
    let v = base + delta;
    if v < 0 {
        0
    } else if v > 255 {
        255
    } else {
        v as usize
    }
}

fn qcat_of(q: i32) -> usize {
    if q <= 20 {
        0
    } else if q <= 60 {
        1
    } else if q <= 120 {
        2
    } else {
        3
    }
}

fn dct4(c: &mut [i64], stride: usize) {
    let i0 = c[0];
    let i1 = c[stride];
    let i2 = c[2 * stride];
    let i3 = c[3 * stride];
    let t0 = ((i0 + i2) * 181 + 128) >> 8;
    let t1 = ((i0 - i2) * 181 + 128) >> 8;
    let t2 = ((i1 * 1567 - i3 * (3784 - 4096) + 2048) >> 12) - i3;
    let t3 = ((i1 * (3784 - 4096) + i3 * 1567 + 2048) >> 12) + i1;
    c[0] = clip16(t0 + t3);
    c[stride] = clip16(t1 + t2);
    c[2 * stride] = clip16(t1 - t2);
    c[3 * stride] = clip16(t0 - t3);
}

fn dct8(c: &mut [i64], stride: usize) {
    dct4(c, stride << 1);
    let i1 = c[stride];
    let i3 = c[3 * stride];
    let i5 = c[5 * stride];
    let i7 = c[7 * stride];
    let t4a = ((i1 * 799 - i7 * (4017 - 4096) + 2048) >> 12) - i7;
    let t5a = (i5 * 1703 - i3 * 1138 + 1024) >> 11;
    let t6a = (i5 * 1138 + i3 * 1703 + 1024) >> 11;
    let t7a = ((i1 * (4017 - 4096) + i7 * 799 + 2048) >> 12) + i1;
    let t4 = clip16(t4a + t5a);
    let t5a = clip16(t4a - t5a);
    let t7 = clip16(t7a + t6a);
    let t6a = clip16(t7a - t6a);
    let t5 = ((t6a - t5a) * 181 + 128) >> 8;
    let t6 = ((t6a + t5a) * 181 + 128) >> 8;
    let t0 = c[0];
    let t1 = c[2 * stride];
    let t2 = c[4 * stride];
    let t3 = c[6 * stride];
    c[0] = clip16(t0 + t7);
    c[stride] = clip16(t1 + t6);
    c[2 * stride] = clip16(t2 + t5);
    c[3 * stride] = clip16(t3 + t4);
    c[4 * stride] = clip16(t3 - t4);
    c[5 * stride] = clip16(t2 - t5);
    c[6 * stride] = clip16(t1 - t6);
    c[7 * stride] = clip16(t0 - t7);
}

fn dct16(c: &mut [i64], stride: usize) {
    dct8(c, stride << 1);
    let i1 = c[stride];
    let i3 = c[3 * stride];
    let i5 = c[5 * stride];
    let i7 = c[7 * stride];
    let i9 = c[9 * stride];
    let i11 = c[11 * stride];
    let i13 = c[13 * stride];
    let i15 = c[15 * stride];
    let t8a = ((i1 * 401 - i15 * (4076 - 4096) + 2048) >> 12) - i15;
    let t9a = (i9 * 1583 - i7 * 1299 + 1024) >> 11;
    let t10a = ((i5 * 1931 - i11 * (3612 - 4096) + 2048) >> 12) - i11;
    let t11a = ((i13 * (3920 - 4096) - i3 * 1189 + 2048) >> 12) + i13;
    let t12a = ((i13 * 1189 + i3 * (3920 - 4096) + 2048) >> 12) + i3;
    let t13a = ((i5 * (3612 - 4096) + i11 * 1931 + 2048) >> 12) + i5;
    let t14a = (i9 * 1299 + i7 * 1583 + 1024) >> 11;
    let t15a = ((i1 * (4076 - 4096) + i15 * 401 + 2048) >> 12) + i1;
    let t8 = clip16(t8a + t9a);
    let t9 = clip16(t8a - t9a);
    let t10 = clip16(t11a - t10a);
    let t11 = clip16(t11a + t10a);
    let t12 = clip16(t12a + t13a);
    let t13 = clip16(t12a - t13a);
    let t14 = clip16(t15a - t14a);
    let t15 = clip16(t15a + t14a);
    let t9a = ((t14 * 1567 - t9 * (3784 - 4096) + 2048) >> 12) - t9;
    let t14a = ((t14 * (3784 - 4096) + t9 * 1567 + 2048) >> 12) + t14;
    let t10a = ((-(t13 * (3784 - 4096) + t10 * 1567) + 2048) >> 12) - t13;
    let t13a = ((t13 * 1567 - t10 * (3784 - 4096) + 2048) >> 12) - t10;
    let t8a = clip16(t8 + t11);
    let t9 = clip16(t9a + t10a);
    let t10 = clip16(t9a - t10a);
    let t11a = clip16(t8 - t11);
    let t12a = clip16(t15 - t12);
    let t13 = clip16(t14a - t13a);
    let t14 = clip16(t14a + t13a);
    let t15a = clip16(t15 + t12);
    let t10a = ((t13 - t10) * 181 + 128) >> 8;
    let t13a = ((t13 + t10) * 181 + 128) >> 8;
    let t11 = ((t12a - t11a) * 181 + 128) >> 8;
    let t12 = ((t12a + t11a) * 181 + 128) >> 8;
    let ev = [
        c[0],
        c[2 * stride],
        c[4 * stride],
        c[6 * stride],
        c[8 * stride],
        c[10 * stride],
        c[12 * stride],
        c[14 * stride],
    ];
    c[0] = clip16(ev[0] + t15a);
    c[stride] = clip16(ev[1] + t14);
    c[2 * stride] = clip16(ev[2] + t13a);
    c[3 * stride] = clip16(ev[3] + t12);
    c[4 * stride] = clip16(ev[4] + t11);
    c[5 * stride] = clip16(ev[5] + t10a);
    c[6 * stride] = clip16(ev[6] + t9);
    c[7 * stride] = clip16(ev[7] + t8a);
    c[8 * stride] = clip16(ev[7] - t8a);
    c[9 * stride] = clip16(ev[6] - t9);
    c[10 * stride] = clip16(ev[5] - t10a);
    c[11 * stride] = clip16(ev[4] - t11);
    c[12 * stride] = clip16(ev[3] - t12);
    c[13 * stride] = clip16(ev[2] - t13a);
    c[14 * stride] = clip16(ev[1] - t14);
    c[15 * stride] = clip16(ev[0] - t15a);
}

fn adst4(c: &mut [i64], stride: usize) {
    let i0 = c[0];
    let i1 = c[stride];
    let i2 = c[2 * stride];
    let i3 = c[3 * stride];
    let o0 = ((1321 * i0 + (3803 - 4096) * i2 + (2482 - 4096) * i3 + (3344 - 4096) * i1 + 2048)
        >> 12)
        + i2
        + i3
        + i1;
    let o1 = (((2482 - 4096) * i0 - 1321 * i2 - (3803 - 4096) * i3 + (3344 - 4096) * i1 + 2048)
        >> 12)
        + i0
        - i3
        + i1;
    let o2 = (209 * (i0 - i2 + i3) + 128) >> 8;
    let o3 = (((3803 - 4096) * i0 + (2482 - 4096) * i2 - 1321 * i3 - (3344 - 4096) * i1 + 2048)
        >> 12)
        + i0
        + i2
        - i1;
    c[0] = o0;
    c[stride] = o1;
    c[2 * stride] = o2;
    c[3 * stride] = o3;
}

fn adst8(c: &mut [i64], stride: usize) {
    let ins: Vec<i64> = (0..8).map(|i| c[i * stride]).collect();
    let (i0, i1, i2, i3, i4, i5, i6, i7) = (
        ins[0], ins[1], ins[2], ins[3], ins[4], ins[5], ins[6], ins[7],
    );
    let t0a = (((4076 - 4096) * i7 + 401 * i0 + 2048) >> 12) + i7;
    let t1a = ((401 * i7 - (4076 - 4096) * i0 + 2048) >> 12) - i0;
    let t2a = (((3612 - 4096) * i5 + 1931 * i2 + 2048) >> 12) + i5;
    let t3a = ((1931 * i5 - (3612 - 4096) * i2 + 2048) >> 12) - i2;
    let t4a = (1299 * i3 + 1583 * i4 + 1024) >> 11;
    let t5a = (1583 * i3 - 1299 * i4 + 1024) >> 11;
    let t6a = ((1189 * i1 + (3920 - 4096) * i6 + 2048) >> 12) + i6;
    let t7a = (((3920 - 4096) * i1 - 1189 * i6 + 2048) >> 12) + i1;
    let t0 = clip16(t0a + t4a);
    let t1 = clip16(t1a + t5a);
    let t2 = clip16(t2a + t6a);
    let t3 = clip16(t3a + t7a);
    let t4 = clip16(t0a - t4a);
    let t5 = clip16(t1a - t5a);
    let t6 = clip16(t2a - t6a);
    let t7 = clip16(t3a - t7a);
    let t4a = (((3784 - 4096) * t4 + 1567 * t5 + 2048) >> 12) + t4;
    let t5a = ((1567 * t4 - (3784 - 4096) * t5 + 2048) >> 12) - t5;
    let t6a = (((3784 - 4096) * t7 - 1567 * t6 + 2048) >> 12) + t7;
    let t7a = ((1567 * t7 + (3784 - 4096) * t6 + 2048) >> 12) + t6;
    let mut out = [0i64; 8];
    out[0] = clip16(t0 + t2);
    out[7] = -clip16(t1 + t3);
    let t2 = clip16(t0 - t2);
    let t3 = clip16(t1 - t3);
    out[1] = -clip16(t4a + t6a);
    out[6] = clip16(t5a + t7a);
    let t6 = clip16(t4a - t6a);
    let t7 = clip16(t5a - t7a);
    out[3] = -(((t2 + t3) * 181 + 128) >> 8);
    out[4] = ((t2 - t3) * 181 + 128) >> 8;
    out[2] = ((t6 + t7) * 181 + 128) >> 8;
    out[5] = -(((t6 - t7) * 181 + 128) >> 8);
    for (i, v) in out.iter().enumerate() {
        c[i * stride] = *v;
    }
}

fn ident(c: &mut [i64], stride: usize, n: usize) -> Av1Result<()> {
    match n {
        4 => {
            for i in 0..4 {
                let v = c[stride * i];
                c[stride * i] = v + ((v * 1697 + 2048) >> 12);
            }
        }
        8 => {
            for i in 0..8 {
                c[stride * i] *= 2;
            }
        }
        16 => {
            for i in 0..16 {
                let v = c[stride * i];
                c[stride * i] = 2 * v + ((v * 1697 + 1024) >> 11);
            }
        }
        _ => return fault(UNSUPPORTED, "that identity transform is not in this slice"),
    }
    Ok(())
}

fn adst16(c: &mut [i64], stride: usize) {
    let ins: Vec<i64> = (0..16).map(|i| c[i * stride]).collect();
    let i0 = ins[0];
    let i1 = ins[1];
    let i2 = ins[2];
    let i3 = ins[3];
    let i4 = ins[4];
    let i5 = ins[5];
    let i6 = ins[6];
    let i7 = ins[7];
    let i8 = ins[8];
    let i9 = ins[9];
    let i10 = ins[10];
    let i11 = ins[11];
    let i12 = ins[12];
    let i13 = ins[13];
    let i14 = ins[14];
    let i15 = ins[15];
    let t0 = ((i15 * (4091 - 4096) + i0 * 201 + 2048) >> 12) + i15;
    let t1 = ((i15 * 201 - i0 * (4091 - 4096) + 2048) >> 12) - i0;
    let t2 = ((i13 * (3973 - 4096) + i2 * 995 + 2048) >> 12) + i13;
    let t3 = ((i13 * 995 - i2 * (3973 - 4096) + 2048) >> 12) - i2;
    let t4 = ((i11 * (3703 - 4096) + i4 * 1751 + 2048) >> 12) + i11;
    let t5 = ((i11 * 1751 - i4 * (3703 - 4096) + 2048) >> 12) - i4;
    let t6 = (i9 * 1645 + i6 * 1220 + 1024) >> 11;
    let t7 = (i9 * 1220 - i6 * 1645 + 1024) >> 11;
    let t8 = ((i7 * 2751 + i8 * (3035 - 4096) + 2048) >> 12) + i8;
    let t9 = ((i7 * (3035 - 4096) - i8 * 2751 + 2048) >> 12) + i7;
    let t10 = ((i5 * 2106 + i10 * (3513 - 4096) + 2048) >> 12) + i10;
    let t11 = ((i5 * (3513 - 4096) - i10 * 2106 + 2048) >> 12) + i5;
    let t12 = ((i3 * 1380 + i12 * (3857 - 4096) + 2048) >> 12) + i12;
    let t13 = ((i3 * (3857 - 4096) - i12 * 1380 + 2048) >> 12) + i3;
    let t14 = ((i1 * 601 + i14 * (4052 - 4096) + 2048) >> 12) + i14;
    let t15 = ((i1 * (4052 - 4096) - i14 * 601 + 2048) >> 12) + i1;
    let t0a = clip16(t0 + t8);
    let t1a = clip16(t1 + t9);
    let t2a = clip16(t2 + t10);
    let t3a = clip16(t3 + t11);
    let t4a = clip16(t4 + t12);
    let t5a = clip16(t5 + t13);
    let t6a = clip16(t6 + t14);
    let t7a = clip16(t7 + t15);
    let t8a = clip16(t0 - t8);
    let t9a = clip16(t1 - t9);
    let t10a = clip16(t2 - t10);
    let t11a = clip16(t3 - t11);
    let t12a = clip16(t4 - t12);
    let t13a = clip16(t5 - t13);
    let t14a = clip16(t6 - t14);
    let t15a = clip16(t7 - t15);
    let t8 = ((t8a * (4017 - 4096) + t9a * 799 + 2048) >> 12) + t8a;
    let t9 = ((t8a * 799 - t9a * (4017 - 4096) + 2048) >> 12) - t9a;
    let t10 = ((t10a * 2276 + t11a * (3406 - 4096) + 2048) >> 12) + t11a;
    let t11 = ((t10a * (3406 - 4096) - t11a * 2276 + 2048) >> 12) + t10a;
    let t12 = ((t13a * (4017 - 4096) - t12a * 799 + 2048) >> 12) + t13a;
    let t13 = ((t13a * 799 + t12a * (4017 - 4096) + 2048) >> 12) + t12a;
    let t14 = ((t15a * 2276 - t14a * (3406 - 4096) + 2048) >> 12) - t14a;
    let t15 = ((t15a * (3406 - 4096) + t14a * 2276 + 2048) >> 12) + t15a;
    let t0 = clip16(t0a + t4a);
    let t1 = clip16(t1a + t5a);
    let t2 = clip16(t2a + t6a);
    let t3 = clip16(t3a + t7a);
    let t4 = clip16(t0a - t4a);
    let t5 = clip16(t1a - t5a);
    let t6 = clip16(t2a - t6a);
    let t7 = clip16(t3a - t7a);
    let t8a = clip16(t8 + t12);
    let t9a = clip16(t9 + t13);
    let t10a = clip16(t10 + t14);
    let t11a = clip16(t11 + t15);
    let t12a = clip16(t8 - t12);
    let t13a = clip16(t9 - t13);
    let t14a = clip16(t10 - t14);
    let t15a = clip16(t11 - t15);
    let t4a = ((t4 * (3784 - 4096) + t5 * 1567 + 2048) >> 12) + t4;
    let t5a = ((t4 * 1567 - t5 * (3784 - 4096) + 2048) >> 12) - t5;
    let t6a = ((t7 * (3784 - 4096) - t6 * 1567 + 2048) >> 12) + t7;
    let t7a = ((t7 * 1567 + t6 * (3784 - 4096) + 2048) >> 12) + t6;
    let t12 = ((t12a * (3784 - 4096) + t13a * 1567 + 2048) >> 12) + t12a;
    let t13 = ((t12a * 1567 - t13a * (3784 - 4096) + 2048) >> 12) - t13a;
    let t14 = ((t15a * (3784 - 4096) - t14a * 1567 + 2048) >> 12) + t15a;
    let t15 = ((t15a * 1567 + t14a * (3784 - 4096) + 2048) >> 12) + t14a;
    let mut out = [0i64; 16];
    out[0] = clip16(t0 + t2);
    out[15] = -clip16(t1 + t3);
    let t2a = clip16(t0 - t2);
    let t3a = clip16(t1 - t3);
    out[3] = -clip16(t4a + t6a);
    out[12] = clip16(t5a + t7a);
    let t6 = clip16(t4a - t6a);
    let t7 = clip16(t5a - t7a);
    out[1] = -clip16(t8a + t10a);
    out[14] = clip16(t9a + t11a);
    let t10 = clip16(t8a - t10a);
    let t11 = clip16(t9a - t11a);
    out[2] = clip16(t12 + t14);
    out[13] = -clip16(t13 + t15);
    let t14a = clip16(t12 - t14);
    let t15a = clip16(t13 - t15);
    out[7] = -(((t2a + t3a) * 181 + 128) >> 8);
    out[8] = ((t2a - t3a) * 181 + 128) >> 8;
    out[4] = ((t6 + t7) * 181 + 128) >> 8;
    out[11] = -(((t6 - t7) * 181 + 128) >> 8);
    out[6] = ((t10 + t11) * 181 + 128) >> 8;
    out[9] = -(((t10 - t11) * 181 + 128) >> 8);
    out[5] = -(((t14a + t15a) * 181 + 128) >> 8);
    out[10] = ((t14a - t15a) * 181 + 128) >> 8;
    for (i, v) in out.iter().enumerate() {
        c[i * stride] = *v;
    }
}

#[derive(Clone, Copy)]
enum Axis {
    Dct4,
    Dct8,
    Dct16,
    Adst4,
    Adst8,
    Adst16,
    Ident(usize),
}

fn px(tx: i32) -> Av1Result<usize> {
    match tx {
        TX_4 => Ok(4),
        TX_8 => Ok(8),
        TX_16 => Ok(16),
        _ => fault(UNSUPPORTED, format!("transform size {tx} is not in this slice")),
    }
}

fn tx_class(txtp: i32) -> i32 {
    if txtp == H_DCT {
        1
    } else if txtp == V_DCT {
        2
    } else {
        0
    }
}

fn tx_pair(txtp: i32, tx: i32) -> Av1Result<(Axis, Axis)> {
    let n = match tx {
        0 => 4,
        1 => 8,
        2 => 16,
        _ => return fault(UNSUPPORTED, format!("transform size {tx} is not in this slice")),
    };
    let pair = match (txtp, tx) {
        (DCT_DCT, 0) => (Axis::Dct4, Axis::Dct4),
        (ADST_DCT, 0) => (Axis::Dct4, Axis::Adst4),
        (DCT_ADST, 0) => (Axis::Adst4, Axis::Dct4),
        (ADST_ADST, 0) => (Axis::Adst4, Axis::Adst4),
        (IDTX, 0) => (Axis::Ident(4), Axis::Ident(4)),
        (DCT_DCT, 1) => (Axis::Dct8, Axis::Dct8),
        (ADST_DCT, 1) => (Axis::Dct8, Axis::Adst8),
        (DCT_ADST, 1) => (Axis::Adst8, Axis::Dct8),
        (ADST_ADST, 1) => (Axis::Adst8, Axis::Adst8),
        (IDTX, 1) => (Axis::Ident(8), Axis::Ident(8)),
        (H_DCT, 0) => (Axis::Dct4, Axis::Ident(4)),
        (V_DCT, 0) => (Axis::Ident(4), Axis::Dct4),
        (H_DCT, 1) => (Axis::Dct8, Axis::Ident(8)),
        (V_DCT, 1) => (Axis::Ident(8), Axis::Dct8),
        (H_DCT, 2) => (Axis::Dct16, Axis::Ident(16)),
        (V_DCT, 2) => (Axis::Ident(16), Axis::Dct16),
        (DCT_DCT, 2) => (Axis::Dct16, Axis::Dct16),
        (ADST_DCT, 2) => (Axis::Dct16, Axis::Adst16),
        (DCT_ADST, 2) => (Axis::Adst16, Axis::Dct16),
        (ADST_ADST, 2) => (Axis::Adst16, Axis::Adst16),
        (IDTX, 2) => (Axis::Ident(16), Axis::Ident(16)),
        _ => {
            return fault(
                UNSUPPORTED,
                format!("transform type {txtp} at size {n} is not in this slice"),
            )
        }
    };
    Ok(pair)
}

fn apply(kind: Axis, c: &mut [i64], stride: usize) -> Av1Result<()> {
    match kind {
        Axis::Dct4 => dct4(c, stride),
        Axis::Dct8 => dct8(c, stride),
        Axis::Dct16 => dct16(c, stride),
        Axis::Adst4 => adst4(c, stride),
        Axis::Adst8 => adst8(c, stride),
        Axis::Adst16 => adst16(c, stride),
        Axis::Ident(n) => ident(c, stride, n)?,
    }
    Ok(())
}

fn shift_of(tx: i32) -> u32 {
    match tx {
        0 => 0,
        1 => 1,
        _ => 2,
    }
}

fn wht4(c: &mut [i64], stride: usize) {
    let i0 = c[0];
    let i1 = c[stride];
    let i2 = c[2 * stride];
    let i3 = c[3 * stride];
    let t0 = i0 + i1;
    let t2 = i2 - i3;
    let t4 = (t0 - t2) >> 1;
    let t3 = t4 - i3;
    let t1 = t4 - i1;
    c[0] = t0 - t3;
    c[stride] = t3;
    c[2 * stride] = t1;
    c[3 * stride] = t2 + t1;
}

fn wht_add(dst: &mut [u8], stride: usize, coef: &[i64]) {
    // The stored coefficient is four times the Walsh value.
    let mut cs = [0i64; 16];
    for i in 0..16 {
        cs[i] = coef[i] >> 2;
    }
    let mut tmp = [0i64; 16];
    for y in 0..4 {
        let mut row = [cs[y], cs[y + 4], cs[y + 8], cs[y + 12]];
        wht4(&mut row, 1);
        for x in 0..4 {
            tmp[y * 4 + x] = row[x];
        }
    }
    for x in 0..4 {
        let mut col = [tmp[x], tmp[4 + x], tmp[8 + x], tmp[12 + x]];
        wht4(&mut col, 1);
        for y in 0..4 {
            let i = y * stride + x;
            dst[i] = clip8(i64::from(dst[i]) + col[y]);
        }
    }
}

fn itx_add(dst: &mut [u8], stride: usize, coef: &[i64], eob: i32, tx: i32, txtp: i32) -> Av1Result<()> {
    if txtp == WHT_WHT {
        if tx != TX_4 {
            return fault(
                UNSUPPORTED,
                "lossless Walsh-Hadamard above 4x4 is not in this slice",
            );
        }
        wht_add(dst, stride, coef);
        return Ok(());
    }
    let n = px(tx)?;
    let (first, second) = tx_pair(txtp, tx)?;
    let shift = shift_of(tx);
    let rnd = (1i64 << shift) >> 1;
    if eob < 1 && txtp == DCT_DCT {
        let mut dc = coef[0];
        dc = (dc * 181 + 128) >> 8;
        dc = (dc + rnd) >> shift;
        dc = (dc * 181 + 128 + 2048) >> 12;
        for y in 0..n {
            for x in 0..n {
                let i = y * stride + x;
                dst[i] = clip8(i64::from(dst[i]) + dc);
            }
        }
        return Ok(());
    }
    let mut tmp = vec![0i64; n * n];
    for y in 0..n {
        let mut row = vec![0i64; n];
        for x in 0..n {
            row[x] = coef[y + x * n];
        }
        apply(first, &mut row, 1)?;
        for x in 0..n {
            tmp[y * n + x] = clip16((row[x] + rnd) >> shift);
        }
    }
    for x in 0..n {
        let mut col = vec![0i64; n];
        for y in 0..n {
            col[y] = tmp[y * n + x];
        }
        apply(second, &mut col, 1)?;
        for y in 0..n {
            tmp[y * n + x] = col[y];
        }
    }
    for y in 0..n {
        for x in 0..n {
            let i = y * stride + x;
            dst[i] = clip8(i64::from(dst[i]) + ((tmp[y * n + x] + 8) >> 4));
        }
    }
    Ok(())
}

fn scan_of(tx: i32, i: usize) -> usize {
    match tx {
        TX_4 => av1data::SCAN_4X4[i] as usize,
        TX_8 => av1data::SCAN_8X8[i] as usize,
        _ => av1data::SCAN_16X16[i] as usize,
    }
}

fn coef_pos(tx_class: i32, tx: i32, n: usize, i: usize) -> (usize, usize, usize) {
    if tx_class == 0 {
        let rc = scan_of(tx, i);
        return (rc / n, rc % n, rc);
    }
    let mask = n - 1;
    let bit_shift = (tx + 2) as usize;
    let x = i & mask;
    let y = i >> bit_shift;
    if tx_class == 1 {
        (x, y, i)
    } else {
        (x, y, (x << bit_shift) | y)
    }
}

fn store_level(levels: &mut [i32], stride_l: usize, lx: usize, ly: usize, tok: i32) {
    let idx = lx * stride_l + ly;
    levels[idx] = if tok >= 3 { tok + (3 << 6) } else { tok * 0x41 };
}

fn levels_at(levels: &[i32], stride_l: usize, lx: usize, ly: usize, dx: usize, dy: usize) -> i32 {
    levels[(lx + dx) * stride_l + (ly + dy)]
}

fn lo_ctx(class: i32, levels: &[i32], stride_l: usize, lx: usize, ly: usize) -> (usize, i32) {
    if class == 0 {
        let mut mag = levels_at(levels, stride_l, lx, ly, 0, 1)
            + levels_at(levels, stride_l, lx, ly, 1, 0)
            + levels_at(levels, stride_l, lx, ly, 1, 1);
        let hi = mag;
        mag += levels_at(levels, stride_l, lx, ly, 0, 2) + levels_at(levels, stride_l, lx, ly, 2, 0);
        let off = LO_CTX[0][ly.min(4)][lx.min(4)];
        let add = if mag > 512 { 4 } else { (mag + 64) >> 7 };
        ((off + add) as usize, hi)
    } else {
        let mut mag = levels_at(levels, stride_l, lx, ly, 0, 1)
            + levels_at(levels, stride_l, lx, ly, 1, 0)
            + levels_at(levels, stride_l, lx, ly, 0, 2);
        let hi = mag;
        mag += levels_at(levels, stride_l, lx, ly, 0, 3) + levels_at(levels, stride_l, lx, ly, 0, 4);
        let off = 26 + if ly > 1 { 10 } else { (ly as i32) * 5 };
        let add = if mag > 512 { 4 } else { (mag + 64) >> 7 };
        ((off + add) as usize, hi)
    }
}

fn hi_base(class: i32, lx: usize, ly: usize) -> usize {
    if class == 0 {
        let cond = if lx != 0 { lx } else { ly };
        if cond > 1 {
            14
        } else {
            7
        }
    } else if ly != 0 {
        14
    } else {
        7
    }
}

struct Coefs {
    skip: [[[u16; 2]; 13]; 5],
    eob16: [[[u16; 5]; 2]; 2],
    eob64: [[[u16; 7]; 2]; 2],
    eob256: [[[u16; 9]; 2]; 2],
    eob_hi: [[[[u16; 2]; 11]; 2]; 5],
    eob_base: [[[[u16; 3]; 4]; 2]; 5],
    base_tok: [[[[u16; 4]; 41]; 2]; 5],
    dc_sign: [[[u16; 2]; 3]; 2],
    br_tok: [[[[u16; 4]; 21]; 2]; 4],
}

impl Coefs {
    fn load(qcat: usize) -> Self {
        Self {
            skip: av1data::COEF_SKIP[qcat],
            eob16: av1data::EOB_BIN_16[qcat],
            eob64: av1data::EOB_BIN_64[qcat],
            eob256: av1data::EOB_BIN_256[qcat],
            eob_hi: av1data::EOB_HI_BIT[qcat],
            eob_base: av1data::EOB_BASE_TOK[qcat],
            base_tok: av1data::BASE_TOK[qcat],
            dc_sign: av1data::DC_SIGN[qcat],
            br_tok: av1data::BR_TOK[qcat],
        }
    }

    fn eob_cdf(&mut self, tx2d: usize, ch: usize, is_1d: usize) -> Av1Result<&mut [u16]> {
        match tx2d {
            0 => Ok(&mut self.eob16[ch][is_1d]),
            2 => Ok(&mut self.eob64[ch][is_1d]),
            4 => Ok(&mut self.eob256[ch][is_1d]),
            _ => fault(UNSUPPORTED, format!("eob bin {tx2d} is not in this slice")),
        }
    }
}

struct Tabs {
    kfym: [[[u16; 13]; 5]; 5],
    use_filter_intra: [[u16; 2]; 22],
    filter_intra: [u16; 5],
    uv_mode: [[u16; 14]; 13],
    angle_delta: [[u16; 7]; 8],
    part0: [[u16; 8]; 4],
    part1: [[u16; 10]; 4],
    part2: [[u16; 10]; 4],
    part3: [[u16; 10]; 4],
    part4: [[u16; 4]; 4],
    skip: [[u16; 2]; 3],
    txsz0: [[u16; 2]; 3],
    txsz1: [[u16; 3]; 3],
    txsz2: [[u16; 3]; 3],
    txsz3: [[u16; 3]; 3],
    txtp1: [[[u16; 7]; 13]; 2],
    txtp2: [[[u16; 5]; 13]; 3],
    pal_y: [[[u16; 2]; 3]; 7],
}

impl Tabs {
    fn load() -> Self {
        let mut tabs = Self {
            kfym: av1data::KFYM,
            use_filter_intra: av1data::USE_FILTER_INTRA,
            filter_intra: av1data::FILTER_INTRA,
            uv_mode: av1data::UV_MODE1,
            angle_delta: av1data::ANGLE_DELTA,
            part0: av1data::PARTITION0,
            part1: av1data::PARTITION1,
            part2: av1data::PARTITION2,
            part3: av1data::PARTITION3,
            part4: av1data::PARTITION4,
            skip: av1data::SKIP,
            txsz0: av1data::TXSZ0,
            txsz1: av1data::TXSZ1,
            txsz2: av1data::TXSZ2,
            txsz3: av1data::TXSZ3,
            txtp1: av1data::TXTP_INTRA1,
            txtp2: av1data::TXTP_INTRA2,
            pal_y: av1data::PAL_Y,
        };
        // Decoder.__init__ overrides. The raw table lists these sizes out of order.
        tabs.use_filter_intra[12] = [32768 - 12408, 0];
        tabs.use_filter_intra[17] = [32768 - 7866, 0];
        tabs.use_filter_intra[21] = [32768 - 4621, 0];
        tabs
    }

    fn partition(&mut self, bl: usize) -> Av1Result<&mut [u16]> {
        match bl {
            0 => Ok(&mut self.part0[0]),
            1 => Ok(&mut self.part1[0]),
            2 => Ok(&mut self.part2[0]),
            3 => Ok(&mut self.part3[0]),
            4 => Ok(&mut self.part4[0]),
            _ => fault(UNSUPPORTED, format!("partition level {bl} is not in this slice")),
        }
    }

    fn txsz(&mut self, idx: usize) -> Av1Result<&mut [u16]> {
        match idx {
            0 => Ok(&mut self.txsz0[0]),
            1 => Ok(&mut self.txsz1[0]),
            2 => Ok(&mut self.txsz2[0]),
            3 => Ok(&mut self.txsz3[0]),
            _ => fault(UNSUPPORTED, "transform size CDF is not in this slice"),
        }
    }
}

struct Decoder<'a> {
    fr: Frame,
    ms: Msac<'a>,
    tabs: Tabs,
    coef: Coefs,
    bw4: usize,
    bh4: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    cw: usize,
    ch: usize,
    above_mode: [i32; 64],
    left_mode: [i32; 64],
    above_skip: [i32; 64],
    left_skip: [i32; 64],
    above_coef: [u32; 64],
    left_coef: [u32; 64],
    above_ccoef: [u32; 64],
    left_ccoef: [u32; 64],
    q_y: [u16; 2],
    q_u: [u16; 2],
    q_v: [u16; 2],
}

impl<'a> Decoder<'a> {
    fn new(fr: Frame, tile: &'a [u8]) -> Self {
        let bw4 = (fr.w + 3) >> 2;
        let bh4 = (fr.h + 3) >> 2;
        let cw = (fr.w + fr.ssx) >> fr.ssx;
        let ch = (fr.h + fr.ssy) >> fr.ssy;
        let qy_dc = qindex(fr.base_q, fr.ydc);
        let q_y = [av1data::DQ8[qy_dc][0], av1data::DQ8[fr.base_q as usize][1]];
        let uq = qindex(fr.base_q, fr.udc);
        let q_u = [av1data::DQ8[uq][0], av1data::DQ8[qindex(fr.base_q, fr.uac)][1]];
        Self {
            ms: Msac::new(tile, fr.update),
            coef: Coefs::load(qcat_of(fr.base_q)),
            tabs: Tabs::load(),
            y: vec![0; fr.w * fr.h],
            u: vec![128; cw * ch],
            v: vec![128; cw * ch],
            bw4,
            bh4,
            cw,
            ch,
            above_mode: [DC; 64],
            left_mode: [DC; 64],
            above_skip: [0; 64],
            left_skip: [0; 64],
            above_coef: [0x40; 64],
            left_coef: [0x40; 64],
            above_ccoef: [0x40; 64],
            left_ccoef: [0x40; 64],
            q_y,
            q_u,
            q_v: q_u,
            fr,
        }
    }

    fn pic(&self) -> Vec<u8> {
        if self.fr.mono {
            return self.y.clone();
        }
        let mut out = Vec::with_capacity(self.y.len() + self.u.len() + self.v.len());
        out.extend_from_slice(&self.y);
        out.extend_from_slice(&self.u);
        out.extend_from_slice(&self.v);
        out
    }

    fn decode_tile(&mut self) -> Av1Result<()> {
        self.sb(BL_64, 0, 0)?;
        if self.ms.starved && self.ms.pos > self.ms.end {
            return fault(INTEGRITY_FAIL, "truncated AV1 tile");
        }
        Ok(())
    }

    fn sb(&mut self, bl: usize, bx: usize, by: usize) -> Av1Result<()> {
        if bx >= self.bw4 || by >= self.bh4 {
            return Ok(());
        }
        let hsz = 16 >> bl;
        let have_h = self.bw4 > bx + hsz;
        let have_v = self.bh4 > by + hsz;
        if !have_h && !have_v {
            return self.sb(bl + 1, bx, by);
        }
        let bp = if have_h && have_v {
            let n = *PART_N
                .get(bl)
                .ok_or_else(|| (UNSUPPORTED, format!("partition level {bl} is not in this slice")))?;
            let cdf = self.tabs.partition(bl)?;
            self.ms.symbol(cdf, n)?
        } else if have_v {
            let cdf = self.tabs.partition(bl)?;
            let flag = self.ms.symbol(cdf, 2)?;
            if flag != 0 {
                SPLIT
            } else {
                H
            }
        } else if have_h {
            let cdf = self.tabs.partition(bl)?;
            let flag = self.ms.symbol(cdf, 2)?;
            if flag != 0 {
                SPLIT
            } else {
                V
            }
        } else {
            SPLIT
        };
        if bp == NONE {
            self.block(bx, by, hsz * 2)?;
        } else if bp == SPLIT {
            let h = hsz >> 1;
            self.sb(bl + 1, bx, by)?;
            self.sb(bl + 1, bx + h, by)?;
            self.sb(bl + 1, bx, by + h)?;
            self.sb(bl + 1, bx + h, by + h)?;
        } else if bp == H {
            self.block(bx, by, hsz)?;
            self.block(bx, by + (hsz >> 1), hsz)?;
        } else if bp == V {
            self.block(bx, by, hsz)?;
            self.block(bx + (hsz >> 1), by, hsz)?;
        } else {
            return fault(UNSUPPORTED, format!("partition {bp} is not in this slice"));
        }
        Ok(())
    }

    fn block(&mut self, bx: usize, by: usize, bw4: usize) -> Av1Result<()> {
        if bx >= self.bw4 || by >= self.bh4 {
            return Ok(());
        }
        let skip_i = (self.above_skip[bx] + self.left_skip[by]) as usize;
        let skip = {
            let cdf = &mut self.tabs.skip[skip_i];
            self.ms.adapt_bool(cdf)?
        };
        let ac = INTRA_CTX[self.above_mode[bx] as usize];
        let lc = INTRA_CTX[self.left_mode[by] as usize];
        let mut ymode = {
            let cdf = &mut self.tabs.kfym[ac][lc];
            self.ms.symbol(cdf, 12)?
        };
        if bw4 >= 2 && (VERT..=8).contains(&ymode) {
            let cdf = &mut self.tabs.angle_delta[(ymode - VERT) as usize];
            let _ = self.ms.symbol(cdf, 7)?;
        }
        let has_c = !self.fr.mono;
        let mut uv = DC;
        if has_c {
            uv = {
                let cdf = &mut self.tabs.uv_mode[ymode as usize];
                self.ms.symbol(cdf, 13)?
            };
            if uv == CFL {
                return fault(UNSUPPORTED, "chroma-from-luma is not in this slice");
            }
            if bw4 >= 2 && (VERT..=8).contains(&uv) {
                let cdf = &mut self.tabs.angle_delta[(uv - VERT) as usize];
                let _ = self.ms.symbol(cdf, 7)?;
            }
        }
        if self.fr.screen && ymode == DC && bw4 <= 16 {
            let sz = if bw4 == 4 { 2 } else { 1 };
            let pal = {
                let cdf = &mut self.tabs.pal_y[sz][0];
                self.ms.adapt_bool(cdf)?
            };
            if pal != 0 {
                return fault(UNSUPPORTED, "palette blocks are not in this slice");
            }
        }
        if ymode == DC && self.fr.filter_intra && bw4 <= 8 {
            let bs = match bw4 {
                4 => 12,
                2 => 17,
                1 => 21,
                _ => return fault(UNSUPPORTED, "filter intra block size is not in this slice"),
            };
            let fib = {
                let cdf = &mut self.tabs.use_filter_intra[bs];
                self.ms.adapt_bool(cdf)?
            };
            if fib != 0 {
                let filt = {
                    let cdf = &mut self.tabs.filter_intra[..];
                    self.ms.symbol(cdf, 4)?
                };
                ymode = 100 + filt;
            }
        }
        let mut tx = match bw4 {
            1 => TX_4,
            2 => TX_8,
            4 => TX_16,
            _ => return fault(UNSUPPORTED, "block size is not in this slice"),
        };
        if self.fr.lossless {
            tx = TX_4;
        } else if self.fr.tx_select && tx > TX_4 {
            let mut depth = {
                let cdf = self.tabs.txsz((tx - 1) as usize)?;
                self.ms.symbol(cdf, tx.min(2) as usize)?
            };
            while depth != 0 {
                tx -= 1;
                depth -= 1;
            }
        }
        self.paint(0, bx * 4, by * 4, bw4 * 4, ymode, tx, None, skip)?;
        if has_c {
            let cw = (bw4 * 4) >> self.fr.ssx;
            let ctx = if self.fr.lossless || cw != 8 { TX_4 } else { TX_8 };
            let uv_tx = UV_TXTP[uv as usize];
            let ux = (bx * 4) >> self.fr.ssx;
            let uy = (by * 4) >> self.fr.ssy;
            self.paint(1, ux, uy, cw, uv, ctx, Some(uv_tx), skip)?;
            self.paint(2, ux, uy, cw, uv, ctx, Some(uv_tx), skip)?;
        }
        for i in 0..bw4 {
            self.above_mode[bx + i] = ymode;
            self.above_skip[bx + i] = skip;
            self.left_mode[by + i] = ymode;
            self.left_skip[by + i] = skip;
        }
        Ok(())
    }

    fn paint(
        &mut self,
        plane: usize,
        x: usize,
        y: usize,
        n: usize,
        mode: i32,
        tx: i32,
        txtp: Option<i32>,
        skip: i32,
    ) -> Av1Result<()> {
        let tw = px(tx)?;
        let mut yy = 0;
        while yy < n {
            let mut xx = 0;
            while xx < n {
                self.one(plane, x + xx, y + yy, mode, tx, txtp, skip)?;
                xx += tw;
            }
            yy += tw;
        }
        Ok(())
    }

    fn one(
        &mut self,
        plane: usize,
        x: usize,
        y: usize,
        mode: i32,
        tx: i32,
        txtp: Option<i32>,
        block_skip: i32,
    ) -> Av1Result<()> {
        let n = px(tx)?;
        let mut block = vec![0u8; n * n];
        self.predict(&mut block, n, plane, x, y, mode)?;
        let (eob, coef, tp) = if block_skip == 0 {
            self.coefs(tx, txtp, mode, plane, x, y)?
        } else {
            (-1, Vec::new(), DCT_DCT)
        };
        if eob >= 0 {
            itx_add(&mut block, n, &coef, eob, tx, tp)?;
        }
        let h = if plane == 0 { self.fr.h } else { self.ch };
        let stride = if plane == 0 { self.fr.w } else { self.cw };
        let dst = match plane {
            0 => &mut self.y,
            1 => &mut self.u,
            _ => &mut self.v,
        };
        for row in 0..n {
            if y + row >= h {
                break;
            }
            for col in 0..n {
                if x + col >= stride {
                    break;
                }
                dst[(y + row) * stride + (x + col)] = block[row * n + col];
            }
        }
        Ok(())
    }

    fn plane<'b>(&'b self, plane: usize) -> (&'b [u8], usize, usize) {
        if plane == 0 {
            (&self.y, self.fr.w, self.fr.h)
        } else if plane == 1 {
            (&self.u, self.cw, self.ch)
        } else {
            (&self.v, self.cw, self.ch)
        }
    }

    fn predict(
        &self,
        block: &mut [u8],
        n: usize,
        plane: usize,
        x: usize,
        y: usize,
        mode: i32,
    ) -> Av1Result<()> {
        let (dst, stride, height) = self.plane(plane);
        if mode == DC && (x == 0 || y == 0) {
            let val = if x == 0 && y == 0 {
                128
            } else if y == 0 {
                let mut s = (n >> 1) as i32;
                for i in 0..n {
                    s += i32::from(dst[(y + i) * stride + (x - 1)]);
                }
                s >> (bit_len(n) - 1)
            } else {
                let mut s = (n >> 1) as i32;
                for i in 0..n {
                    s += i32::from(dst[(y - 1) * stride + (x + i)]);
                }
                s >> (bit_len(n) - 1)
            };
            let b = (val & 255) as u8;
            for px in block.iter_mut() {
                *px = b;
            }
            return Ok(());
        }
        if mode == DC {
            let mut s = n as i32;
            for i in 0..n {
                s += i32::from(dst[(y - 1) * stride + x + i]);
                s += i32::from(dst[(y + i) * stride + (x - 1)]);
            }
            let val = s >> bit_len(n);
            let b = (val & 255) as u8;
            for px in block.iter_mut() {
                *px = b;
            }
            return Ok(());
        }
        if mode == VERT {
            for row in 0..n {
                for col in 0..n {
                    let src = if y == 0 {
                        127
                    } else {
                        dst[(y - 1) * stride + x + col]
                    };
                    block[row * n + col] = src;
                }
            }
            return Ok(());
        }
        if mode == HOR {
            for row in 0..n {
                let src = if x == 0 {
                    129
                } else {
                    dst[(y + row) * stride + (x - 1)]
                };
                for col in 0..n {
                    block[row * n + col] = src;
                }
            }
            return Ok(());
        }
        if mode >= 100 {
            filter_intra(block, n, dst, stride, height, x, y, (mode - 100) as usize)?;
            return Ok(());
        }
        fault(UNSUPPORTED, format!("intra mode {mode} is not in this slice"))
    }

    fn dc_sign_ctx(&self, tx: i32, x4: usize, y4: usize, plane: usize) -> usize {
        let above = if plane == 0 { &self.above_coef } else { &self.above_ccoef };
        let left = if plane == 0 { &self.left_coef } else { &self.left_ccoef };
        let mask = 0xC0C0C0C0u32;
        let s = if tx == TX_4 {
            ((above[x4] >> 6) as i32) + ((left[y4] >> 6) as i32) - 2
        } else if tx == TX_8 {
            let t = u64::from(load_bytes(above, x4, 2) & mask) + u64::from(load_bytes(left, y4, 2) & mask);
            let t = (t * 0x0404_0404) & 0xFFFF_FFFF;
            (t >> 24) as i32 - 4
        } else {
            let t = u64::from((load_bytes(above, x4, 4) & mask) >> 6)
                + u64::from((load_bytes(left, y4, 4) & mask) >> 6);
            let t = (t * 0x0101_0101) & 0xFFFF_FFFF;
            (t >> 24) as i32 - 8
        };
        usize::from(s != 0) + usize::from(s > 0)
    }

    fn skip_ctx(&self, tx: i32, plane: usize, x4: usize, y4: usize) -> usize {
        if plane != 0 {
            return 7;
        }
        let n4 = match tx {
            TX_4 => 1,
            TX_8 => 2,
            _ => 4,
        };
        let mut la = pack_coef(&self.above_coef, x4, n4);
        let mut ll = pack_coef(&self.left_coef, y4, n4);
        if tx >= TX_8 {
            la |= la >> 8;
            ll |= ll >> 8;
        }
        if tx >= TX_16 {
            la |= la >> 16;
            ll |= ll >> 16;
        }
        let la = ((la & 0x3F) as usize).min(4);
        let ll = ((ll & 0x3F) as usize).min(4);
        SKIP_CTX[la][ll]
    }

    fn coefs(
        &mut self,
        tx: i32,
        mut txtp: Option<i32>,
        mode: i32,
        plane: usize,
        x: usize,
        y: usize,
    ) -> Av1Result<(i32, Vec<i64>, i32)> {
        let n = px(tx)?;
        let ctx = tx as usize;
        let sctx = self.skip_ctx(tx, plane, x >> 2, y >> 2);
        let all_skip = {
            let cdf = &mut self.coef.skip[ctx][sctx];
            self.ms.adapt_bool(cdf)?
        };
        let mut coef = vec![0i64; n * n];
        if all_skip != 0 {
            let n4 = n / 4;
            let x4 = x >> 2;
            let y4 = y >> 2;
            for i in 0..n4 {
                if plane == 0 {
                    self.above_coef[x4 + i] = 0x40;
                    self.left_coef[y4 + i] = 0x40;
                } else {
                    self.above_ccoef[x4 + i] = 0x40;
                    self.left_ccoef[y4 + i] = 0x40;
                }
            }
            return Ok((-1, coef, DCT_DCT));
        }
        if self.fr.lossless {
            txtp = Some(WHT_WHT);
        } else if plane == 0 {
            let ymode = if mode >= 100 {
                FILTER_TO_Y[(mode - 100) as usize]
            } else {
                mode
            };
            if self.fr.reduced_tx || tx == TX_16 {
                let idx = {
                    let cdf = &mut self.tabs.txtp2[tx as usize][ymode as usize];
                    self.ms.symbol(cdf, 4)?
                };
                txtp = Some(TX_TYPES_INTRA2[idx as usize]);
            } else {
                let idx = {
                    let cdf = &mut self.tabs.txtp1[tx as usize][ymode as usize];
                    self.ms.symbol(cdf, 6)?
                };
                txtp = Some(TX_TYPES_INTRA1[idx as usize]);
            }
        }
        let txtp = match txtp {
            Some(t) => t,
            None => return fault(UNSUPPORTED, "missing transform type"),
        };
        if txtp == WHT_WHT {
            if tx != TX_4 {
                return fault(
                    UNSUPPORTED,
                    "lossless Walsh-Hadamard above 4x4 is not in this slice",
                );
            }
        } else {
            let _ = tx_pair(txtp, tx)?;
        }
        let class = tx_class(txtp);
        let is_1d = class != 0;
        let tx2d = (tx + tx) as usize;
        let ch = if plane == 0 { 0 } else { 1 };
        let eob_bin = {
            let cdf = self.coef.eob_cdf(tx2d, ch, usize::from(is_1d))?;
            self.ms.symbol(cdf, 4 + tx2d)?
        };
        let eob = if eob_bin > 1 {
            let hi = {
                let cdf = &mut self.coef.eob_hi[ctx][ch][eob_bin as usize];
                self.ms.adapt_bool(cdf)?
            };
            ((hi | 2) << (eob_bin - 2)) | self.ms.bools(eob_bin - 2)?
        } else {
            eob_bin
        };
        let dq = if plane == 0 {
            self.q_y
        } else if plane == 1 {
            self.q_u
        } else {
            self.q_v
        };
        let cf_max = 32767i64;
        let dq_shift = (tx - 2).max(0) as u32;
        let stride_l = if is_1d { 16 } else { n };
        let mut levels = vec![0i32; stride_l * (n + 2)];
        let mut acs: Vec<(i32, usize, i32)> = Vec::new();
        let mut dc_tok;
        if eob != 0 {
            let sw = (n / 4).min(8);
            let (lx, ly, rc) = coef_pos(class, tx, n, eob as usize);
            let eob_ctx = 1
                + usize::from(eob > (sw * sw * 2) as i32)
                + usize::from(eob > (sw * sw * 4) as i32);
            let et = {
                let cdf = &mut self.coef.eob_base[ctx][ch][eob_ctx];
                self.ms.symbol(cdf, 2)?
            };
            let mut tok = et + 1;
            if et == 2 {
                let hb = hi_base(class, lx, ly);
                tok = {
                    let cdf = &mut self.coef.br_tok[ctx.min(3)][ch][hb];
                    self.ms.hi_tok(cdf)?
                };
            }
            store_level(&mut levels, stride_l, lx, ly, tok);
            acs.push((eob, rc, tok));
            let mut i = eob - 1;
            while i >= 1 {
                let (lx, ly, rc) = coef_pos(class, tx, n, i as usize);
                let (cctx, mag) = lo_ctx(class, &levels, stride_l, lx, ly);
                let mut tok = {
                    let cdf = &mut self.coef.base_tok[ctx][ch][cctx];
                    self.ms.symbol(cdf, 3)?
                };
                if tok == 3 {
                    let mag = mag & 63;
                    let hctx = hi_base(class, lx, ly)
                        + if mag > 12 { 6 } else { ((mag + 1) >> 1) as usize };
                    tok = {
                        let cdf = &mut self.coef.br_tok[ctx.min(3)][ch][hctx];
                        self.ms.hi_tok(cdf)?
                    };
                }
                store_level(&mut levels, stride_l, lx, ly, tok);
                if tok != 0 {
                    acs.push((i, rc, tok));
                }
                i -= 1;
            }
            let (dc_ctx, mut dc_mag) = if class == 0 {
                (0usize, 0i32)
            } else {
                lo_ctx(class, &levels, stride_l, 0, 0)
            };
            dc_tok = {
                let cdf = &mut self.coef.base_tok[ctx][ch][dc_ctx];
                self.ms.symbol(cdf, 3)?
            };
            if dc_tok == 3 {
                if class == 0 {
                    dc_mag = levels[1] + levels[stride_l] + levels[stride_l + 1];
                }
                dc_mag &= 63;
                let hctx = if dc_mag > 12 { 6 } else { ((dc_mag + 1) >> 1) as usize };
                dc_tok = {
                    let cdf = &mut self.coef.br_tok[ctx.min(3)][ch][hctx];
                    self.ms.hi_tok(cdf)?
                };
            }
        } else {
            let et = {
                let cdf = &mut self.coef.eob_base[ctx][ch][0];
                self.ms.symbol(cdf, 2)?
            };
            dc_tok = et + 1;
            if et == 2 {
                dc_tok = {
                    let cdf = &mut self.coef.br_tok[ctx.min(3)][ch][0];
                    self.ms.hi_tok(cdf)?
                };
            }
        }
        let mut dc_sign = 0i32;
        let mut cul = 0i64;
        if dc_tok != 0 {
            let sc = self.dc_sign_ctx(tx, x >> 2, y >> 2, plane);
            dc_sign = {
                let cdf = &mut self.coef.dc_sign[ch][sc];
                self.ms.adapt_bool(cdf)?
            };
            let mut tok = i64::from(dc_tok);
            if tok == 15 {
                tok = (self.ms.golomb()? + 15) & 0xF_FFFF;
            }
            cul += tok;
            let dqv = ((i64::from(dq[0]) * tok) & 0xFF_FFFF) >> dq_shift;
            coef[0] = if dc_sign != 0 {
                -dqv.min(cf_max + i64::from(dc_sign))
            } else {
                dqv.min(cf_max)
            };
        }
        acs.sort();
        for (_i, rc, tok0) in acs {
            let sign = self.ms.equi()?;
            let mut tok = i64::from(tok0);
            if tok >= 15 {
                tok = (self.ms.golomb()? + 15) & 0xF_FFFF;
            }
            let mut dqv = ((i64::from(dq[1]) * tok) & 0xFF_FFFF) >> dq_shift;
            dqv = dqv.min(cf_max + i64::from(sign));
            coef[rc] = if sign != 0 { -dqv } else { dqv };
            cul += tok;
        }
        let sign_level = if dc_tok == 0 {
            64u32
        } else {
            ((dc_sign - 1) & 128) as u32
        };
        let level = (cul.min(63) as u32) | sign_level;
        let n4 = n / 4;
        let x4 = x >> 2;
        let y4 = y >> 2;
        for i in 0..n4 {
            if plane == 0 {
                self.above_coef[x4 + i] = level;
                self.left_coef[y4 + i] = level;
            } else {
                self.above_ccoef[x4 + i] = level;
                self.left_ccoef[y4 + i] = level;
            }
        }
        Ok((eob, coef, txtp))
    }
}

fn load_bytes(arr: &[u32], i: usize, n: usize) -> u32 {
    let mut v = 0u32;
    for k in 0..n {
        v |= (arr[i + k] & 0xFF) << (8 * k);
    }
    v
}

fn pack_coef(arr: &[u32], i: usize, n4: usize) -> u32 {
    let mut v = 0u32;
    for k in 0..n4 {
        v |= arr[i + k] << (8 * k);
    }
    v
}

fn filter_intra(
    block: &mut [u8],
    n: usize,
    dst: &[u8],
    stride: usize,
    height: usize,
    x: usize,
    y: usize,
    filt: usize,
) -> Av1Result<()> {
    if filt >= av1data::FILTER_TAPS.len() {
        return fault(UNSUPPORTED, format!("filter intra {filt} is not in this slice"));
    }
    let taps = &av1data::FILTER_TAPS[filt];
    let mut top = Vec::with_capacity(n * 2);
    let mut left = Vec::with_capacity(n * 2);
    for i in 0..n * 2 {
        let pix = if y > 0 && x + i < stride {
            dst[(y - 1) * stride + (x + i).min(stride - 1)]
        } else if y > 0 {
            *top.last().unwrap()
        } else if x > 0 {
            dst[y * stride + (x - 1)]
        } else {
            127
        };
        top.push(pix);
        let lp = if x > 0 && y + i < height {
            dst[(y + i) * stride + (x - 1)]
        } else if x > 0 {
            *left.last().unwrap()
        } else if y > 0 {
            dst[(y - 1) * stride + x]
        } else {
            129
        };
        left.push(lp);
    }
    let corner = if x > 0 && y > 0 {
        dst[(y - 1) * stride + (x - 1)]
    } else if x > 0 {
        left[0]
    } else if y > 0 {
        top[0]
    } else {
        128
    };
    let center = n * 2;
    let mut edge = vec![128u8; n * 4 + 4];
    edge[center] = corner;
    for i in 0..n * 2 {
        edge[center + 1 + i] = top[i];
        edge[center - 1 - i] = left[i];
    }
    let mut yy = 0;
    while yy < n {
        let top_base = center + 1;
        let mut xx = 0;
        while xx < n {
            let tops: [u8; 4] = if yy == 0 {
                [
                    edge[top_base + xx],
                    edge[top_base + xx + 1],
                    edge[top_base + xx + 2],
                    edge[top_base + xx + 3],
                ]
            } else {
                [
                    block[(yy - 1) * n + xx],
                    block[(yy - 1) * n + xx + 1],
                    block[(yy - 1) * n + xx + 2],
                    block[(yy - 1) * n + xx + 3],
                ]
            };
            let (p0, p5, p6) = if xx == 0 {
                (
                    edge[center - yy],
                    edge[center - yy - 1],
                    edge[center - yy - 2],
                )
            } else if yy == 0 {
                (
                    edge[top_base + xx - 1],
                    block[yy * n + xx - 1],
                    block[(yy + 1) * n + xx - 1],
                )
            } else {
                (
                    block[(yy - 1) * n + xx - 1],
                    block[yy * n + xx - 1],
                    block[(yy + 1) * n + xx - 1],
                )
            };
            let (p1, p2, p3, p4) = (tops[0], tops[1], tops[2], tops[3]);
            for row in 0..2 {
                for col in 0..4 {
                    let f = taps[row * 4 + col];
                    let acc = i32::from(f[0]) * i32::from(p0)
                        + i32::from(f[1]) * i32::from(p1)
                        + i32::from(f[2]) * i32::from(p2)
                        + i32::from(f[3]) * i32::from(p3)
                        + i32::from(f[4]) * i32::from(p4)
                        + i32::from(f[5]) * i32::from(p5)
                        + i32::from(f[6]) * i32::from(p6);
                    block[(yy + row) * n + xx + col] = clip8((i64::from(acc) + 8) >> 4);
                }
            }
            xx += 4;
        }
        yy += 2;
    }
    Ok(())
}

/// One still picture. Empty input is [`BOUNDS`]. A short OBU is [`INTEGRITY_FAIL`].
pub fn decode(data: &[u8]) -> Result<Vec<u8>, (&'static str, String)> {
    if data.is_empty() {
        return fault(BOUNDS, "empty AV1 payload");
    }
    let obus = obus(data)?;
    let mut fr = Frame::default();
    let mut saw_seq = false;
    let mut saw_frame = false;
    let mut tile = Vec::new();
    for (typ, payload) in obus {
        if typ == 2 || typ == 15 {
            continue;
        } else if typ == 1 {
            seq(&payload, &mut fr)?;
            saw_seq = true;
        } else if typ == 6 {
            if !saw_seq {
                return fault(INTEGRITY_FAIL, "AV1 frame before the sequence header");
            }
            tile = frame_header(&payload, &mut fr)?.to_vec();
            saw_frame = true;
        } else {
            return fault(UNSUPPORTED, format!("AV1 OBU type {typ} is not in this slice"));
        }
    }
    if !saw_frame {
        return fault(INTEGRITY_FAIL, "AV1 payload has no frame");
    }
    // Non-zero loop-filter levels are parsed. The filter itself is a no-op in this slice.
    let _ = fr.lf;
    let mut dec = Decoder::new(fr, &tile);
    dec.decode_tile()?;
    Ok(dec.pic())
}

#[cfg(test)]
mod tests {
    use super::decode;
    use crate::{BOUNDS, INTEGRITY_FAIL, UNSUPPORTED};

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    const PAYLOAD_HEX: &str = "12000a06180cffd80080325915000000500e38c8bc443dfd8e548a388c486185f1e6c8710f49084b592b9181ce0481b245e9d503ee5d9f28bbe12a2226f2532a78f1b0c62396b0cb04ce1c4098452f50183178a8e6fa4ae2c3fdd302e4b83fff395615b990";
    const YUV_HEX: &str = "01070f1820293238414d52586068737e13182129323a43494d575d646c747e8923293139424b53595d686f767f86909a31374048505962676c79828b9298a1ab41475059616972787d8a929aa2a7afb950565f67707982878c9aa2abb2b7bfc860666f77808992989ca9b0b8c1c6ced570767f889099a2a8abb8c0c8d0d5dee682858d969fa9b2bbc7cad0d7e0e7f0f8999ca0a7afb8c0c8d1d7e0eaf4f7060ca7a9aeb5bfc8d1d8e4e6eff700060c13b5b8bec5cfd9e1eaf2f1020812191f26c8cbd0d8e0e9f0f80005121b2128343dcdd7e1ebf5f8060a0e1c202a303a433fe0e6eef500050f18202b2f3841464c59f0f3030710181f282b3a434950585e648080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080808080";

    #[test]
    fn still_picture_matches_reference() {
        let payload = unhex(PAYLOAD_HEX);
        let yuv = unhex(YUV_HEX);
        assert_eq!(payload.len(), 101);
        assert_eq!(yuv.len(), 384);
        let got = decode(&payload).expect("decode");
        assert_eq!(got, yuv);
    }

    #[test]
    fn empty_slice_is_bounds() {
        let err = decode(&[]).unwrap_err();
        assert_eq!(err.0, BOUNDS);
    }

    #[test]
    fn truncated_obu_is_integrity_fail() {
        let payload = unhex(PAYLOAD_HEX);
        let err = decode(&payload[..payload.len() - 10]).unwrap_err();
        assert_eq!(err.0, INTEGRITY_FAIL);
    }

    #[test]
    fn lossless_round_trip_matches_the_still() {
        let yuv = unhex(YUV_HEX);
        let obu = super::encode(&yuv).expect("encode");
        let back = decode(&obu).expect("decode encoded");
        assert_eq!(back, yuv);
    }

    #[test]
    fn lossless_empty_is_bounds() {
        let err = super::encode(&[]).unwrap_err();
        assert_eq!(err.0, BOUNDS);
    }

    #[test]
    fn lossless_other_size_is_unsupported() {
        let err = super::encode(&[0u8; 16]).unwrap_err();
        assert_eq!(err.0, UNSUPPORTED);
    }
}
