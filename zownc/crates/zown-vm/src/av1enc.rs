//! Lossless 16×16 4:2:0 still encoder. Translated from `zown/av1enc.py`.
//!
//! qindex 0, one 16×16 DC block, 4×4 Walsh-Hadamard. `decode` of the bytes
//! this writes is the same I420 buffer.

use super::{
    av1data, fault, Av1Result, Coefs, Tabs, BOUNDS, DC, LO_CTX, NONE, PART_N, SKIP_CTX, TX_4,
    UNSUPPORTED,
};

const MASK64: u128 = (1u128 << 64) - 1;

struct MsacEnc {
    buf: Vec<u8>,
    offs: usize,
    low: u128,
    rng: u64,
    cnt: i32,
}

fn bit_length_u64(n: u64) -> i32 {
    if n == 0 {
        0
    } else {
        64 - n.leading_zeros() as i32
    }
}

fn bit_length_i64(n: i64) -> i32 {
    if n <= 0 {
        0
    } else {
        64 - n.leading_zeros() as i32
    }
}

fn shr_u128(val: u128, c: i32) -> u128 {
    if c >= 0 {
        val >> c
    } else {
        val << (-c as u32)
    }
}

impl MsacEnc {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            offs: 0,
            low: 0,
            rng: 0x8000,
            cnt: -9,
        }
    }

    fn carry(&mut self, mut offs: isize) {
        let mut carry = 1u32;
        while carry != 0 {
            if offs < 0 {
                break;
            }
            let total = u32::from(self.buf[offs as usize]) + 1;
            self.buf[offs as usize] = (total & 0xFF) as u8;
            carry = total >> 8;
            offs -= 1;
        }
    }

    fn write(&mut self, output: u128, carry: u128, num: usize) {
        let shift = (8 - num) << 3;
        let reg = (output << shift) & MASK64;
        let raw = (reg as u64).to_be_bytes();
        let end = self.offs + 8;
        if self.buf.len() < end {
            self.buf.resize(end, 0);
        }
        self.buf[self.offs..end].copy_from_slice(&raw);
        if carry != 0 {
            self.carry(self.offs as isize - 1);
        }
        self.offs += num;
    }

    fn norm(&mut self, mut low: u128, rng: u64) {
        let mut c = self.cnt;
        let d = 16 - bit_length_u64(rng);
        let mut s = c + d;
        if s >= 40 {
            let num = ((s >> 3) + 1) as usize;
            c = c + 24 - ((num as i32) << 3);
            let output = shr_u128(low, c);
            low &= if c <= 0 { 0 } else { (1u128 << c) - 1 };
            let mask = 1u128 << ((num as u32) << 3);
            let carry = output & mask;
            let output = output & (mask - 1);
            self.write(output, carry, num);
            s = c + d - 24;
        }
        self.low = (low << d) & MASK64;
        self.rng = rng << d;
        self.cnt = s;
    }

    fn bool_bit(&mut self, bit: i32, f: u32) {
        let r0 = self.rng;
        let v = ((r0 >> 8) * u64::from(f >> 6) >> 1) + 4;
        let mut low = self.low;
        let r = if bit != 0 {
            low += u128::from(r0 - v);
            v
        } else {
            r0 - v
        };
        self.norm(low, r);
    }

    fn equi(&mut self, bit: i32) {
        self.bool_bit(bit, 16384);
    }

    fn bools(&mut self, n: i32, value: i32) {
        for i in (0..n).rev() {
            self.equi((value >> i) & 1);
        }
    }

    fn adapt_bool(&mut self, cdf: &mut [u16], bit: i32) {
        self.bool_bit(bit, u32::from(cdf[0]));
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

    fn symbol(&mut self, cdf: &mut [u16], n: usize, s: usize) {
        let fl = if s == 0 { 32768u32 } else { u32::from(cdf[s - 1]) };
        let fh = u32::from(cdf[s]);
        let r0 = self.rng;
        let mut low = self.low;
        let r = if fl < 32768 {
            let u = ((r0 >> 8) * u64::from(fl >> 6) >> 1) + 4 * (n as u64 - (s as u64 - 1));
            let v = ((r0 >> 8) * u64::from(fh >> 6) >> 1) + 4 * (n as u64 - s as u64);
            low += u128::from(r0 - u);
            u - v
        } else {
            r0 - (((r0 >> 8) * u64::from(fh >> 6) >> 1) + 4 * (n as u64 - s as u64))
        };
        self.norm(low, r);
        let count = cdf[n];
        let rate = u32::from(4 + (count >> 4) + u16::from(n > 2));
        for i in 0..s {
            let p = u32::from(cdf[i]);
            cdf[i] = (p + ((32768 - p) >> rate)) as u16;
        }
        for i in s..n {
            let p = u32::from(cdf[i]);
            cdf[i] = (p - (p >> rate)) as u16;
        }
        cdf[n] = count + u16::from(count < 32);
    }

    fn hi_tok(&mut self, cdf: &mut [u16], tok: i32) {
        if tok < 6 {
            self.symbol(cdf, 3, (tok - 3) as usize);
            return;
        }
        self.symbol(cdf, 3, 3);
        if tok < 9 {
            self.symbol(cdf, 3, (tok - 6) as usize);
            return;
        }
        self.symbol(cdf, 3, 3);
        if tok < 12 {
            self.symbol(cdf, 3, (tok - 9) as usize);
            return;
        }
        self.symbol(cdf, 3, 3);
        self.symbol(cdf, 3, (tok - 12) as usize);
    }

    fn golomb(&mut self, value: i32) {
        let length = bit_length_i64(i64::from(value) + 1) - 1;
        for _ in 0..length {
            self.equi(0);
        }
        self.equi(1);
        let body = if length > 0 {
            (value + 1) & ((1 << length) - 1)
        } else {
            0
        };
        for i in (0..length).rev() {
            self.equi((body >> i) & 1);
        }
    }

    fn done(&mut self) -> Vec<u8> {
        let low = self.low;
        let mut c = self.cnt;
        let mut s = 10 + c;
        let m = 0x3FFFu128;
        let mut e = ((low + m) & !m) | (m + 1);
        if s > 0 {
            let mut n = (1u128 << (c + 16)) - 1;
            while s > 0 {
                let val = shr_u128(e, c + 16) & 0xFFFF;
                if self.buf.len() <= self.offs {
                    self.buf.push(0);
                }
                self.buf[self.offs] = (val & 0xFF) as u8;
                if val & 0x100 != 0 {
                    self.carry(self.offs as isize - 1);
                }
                self.offs += 1;
                e &= n;
                s -= 8;
                c -= 8;
                n >>= 8;
            }
        }
        self.buf[..self.offs].to_vec()
    }
}

struct Bits {
    bits: Vec<u8>,
}

impl Bits {
    fn new() -> Self {
        Self { bits: Vec::new() }
    }

    fn u(&mut self, n: usize, v: u32) {
        for i in (0..n).rev() {
            self.bits.push(((v >> i) & 1) as u8);
        }
    }

    fn align(&mut self) {
        while self.bits.len() & 7 != 0 {
            self.bits.push(0);
        }
    }

    fn trailing(&mut self) {
        self.bits.push(1);
        while self.bits.len() & 7 != 0 {
            self.bits.push(0);
        }
    }

    fn payload(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut acc = 0u8;
        let mut k = 0;
        for &b in &self.bits {
            acc = (acc << 1) | b;
            k += 1;
            if k == 8 {
                out.push(acc);
                acc = 0;
                k = 0;
            }
        }
        out
    }
}

fn leb(mut n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (n & 0x7F) as u8;
        n >>= 7;
        if n != 0 {
            out.push(b | 0x80);
        } else {
            out.push(b);
            return out;
        }
    }
}

fn obu(typ: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![(typ << 3) | 0x02];
    out.extend(leb(payload.len()));
    out.extend_from_slice(payload);
    out
}

fn seq() -> Vec<u8> {
    let mut b = Bits::new();
    b.u(3, 0);
    b.u(1, 1);
    b.u(1, 1);
    b.u(5, 0);
    b.u(4, 3);
    b.u(4, 3);
    b.u(4, 15);
    b.u(4, 15);
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 1);
    b.u(2, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.trailing();
    b.payload()
}

fn frame_header() -> Vec<u8> {
    let mut b = Bits::new();
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 1);
    b.u(8, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.u(1, 0);
    b.align();
    b.payload()
}

fn fwht(raster: &[i64; 16]) -> [i64; 16] {
    let mut mid = [0i64; 16];
    for i in 0..4 {
        let mut a = raster[i];
        let b0 = raster[4 + i];
        let c0 = raster[8 + i];
        let mut d = raster[12 + i];
        a += b0;
        d -= c0;
        let e = (a - d) >> 1;
        let b = e - b0;
        let c = e - c0;
        a -= c;
        d += b;
        mid[i * 4] = a;
        mid[i * 4 + 1] = c;
        mid[i * 4 + 2] = d;
        mid[i * 4 + 3] = b;
    }
    let mut out = [0i64; 16];
    for i in 0..4 {
        let mut a = mid[i];
        let b0 = mid[4 + i];
        let c0 = mid[8 + i];
        let mut d = mid[12 + i];
        a += b0;
        d -= c0;
        let e = (a - d) >> 1;
        let b = e - b0;
        let c = e - c0;
        a -= c;
        d += b;
        out[i] = a * 4;
        out[4 + i] = c * 4;
        out[8 + i] = d * 4;
        out[12 + i] = b * 4;
    }
    out
}

fn div_floor(a: i64, b: i64) -> i64 {
    let mut q = a / b;
    let r = a % b;
    if r != 0 && (a < 0) != (b < 0) {
        q -= 1;
    }
    q
}

fn dc_pred(dst: &[u8], stride: usize, x: usize, y: usize) -> i32 {
    let n = 4usize;
    if x == 0 && y == 0 {
        return 128;
    }
    if y == 0 {
        let mut s = (n >> 1) as i32;
        for i in 0..n {
            s += i32::from(dst[(y + i) * stride + (x - 1)]);
        }
        return s >> (usize::BITS - n.leading_zeros() - 1);
    }
    if x == 0 {
        let mut s = (n >> 1) as i32;
        for i in 0..n {
            s += i32::from(dst[(y - 1) * stride + (x + i)]);
        }
        return s >> (usize::BITS - n.leading_zeros() - 1);
    }
    let mut s = n as i32;
    for i in 0..n {
        s += i32::from(dst[(y - 1) * stride + x + i]);
        s += i32::from(dst[(y + i) * stride + (x - 1)]);
    }
    s >> (usize::BITS - n.leading_zeros())
}

fn skip_ctx(above: &[u32], left: &[u32], x4: usize, y4: usize) -> usize {
    let la = ((above[x4] & 0x3F) as usize).min(4);
    let ll = ((left[y4] & 0x3F) as usize).min(4);
    SKIP_CTX[la][ll]
}

fn dc_sign_ctx(above: &[u32], left: &[u32], x4: usize, y4: usize) -> usize {
    let s = ((above[x4] >> 6) as i32) + ((left[y4] >> 6) as i32) - 2;
    usize::from(s != 0) + usize::from(s > 0)
}

fn encode_eob(ms: &mut MsacEnc, coef: &mut Coefs, ctx: usize, ch: usize, eob: i32) {
    if eob <= 1 {
        ms.symbol(&mut coef.eob16[ch][0], 4, eob as usize);
        return;
    }
    let k = bit_length_i64(i64::from(eob)) - 2;
    let eob_bin = k + 2;
    ms.symbol(&mut coef.eob16[ch][0], 4, eob_bin as usize);
    let hi = i32::from(eob >= (3 << k));
    ms.adapt_bool(&mut coef.eob_hi[ctx][ch][eob_bin as usize], hi);
    if k > 0 {
        ms.bools(k, eob & ((1 << k) - 1));
    }
}

fn br_for(tok: i32, base: usize, mag: i32) -> usize {
    if tok < 3 {
        return base;
    }
    let mag = mag & 63;
    base + if mag > 12 { 6 } else { ((mag + 1) >> 1) as usize }
}

fn encode_base(
    ms: &mut MsacEnc,
    coef: &mut Coefs,
    ctx: usize,
    ch: usize,
    cctx: usize,
    br_at: usize,
    tok: i32,
) -> i32 {
    if tok < 3 {
        ms.symbol(&mut coef.base_tok[ctx][ch][cctx], 3, tok as usize);
        return tok;
    }
    ms.symbol(&mut coef.base_tok[ctx][ch][cctx], 3, 3);
    let shown = if tok >= 15 { 15 } else { tok };
    ms.hi_tok(&mut coef.br_tok[ctx.min(3)][ch][br_at], shown);
    shown
}

fn encode_eob_tok(
    ms: &mut MsacEnc,
    coef: &mut Coefs,
    ctx: usize,
    ch: usize,
    eob_ctx: usize,
    br_at: usize,
    tok: i32,
) -> i32 {
    let et = if tok >= 3 { 2 } else { tok - 1 };
    ms.symbol(&mut coef.eob_base[ctx][ch][eob_ctx], 2, et as usize);
    if et < 2 {
        return tok;
    }
    let shown = if tok >= 15 { 15 } else { tok };
    ms.hi_tok(&mut coef.br_tok[ctx.min(3)][ch][br_at], shown);
    shown
}

fn lo_ctx(levels: &[i32], stride_l: usize, lx: usize, ly: usize) -> (usize, i32) {
    let at = |dx: usize, dy: usize| levels[(lx + dx) * stride_l + (ly + dy)];
    let mut mag = at(0, 1) + at(1, 0) + at(1, 1);
    let hi = mag;
    mag += at(0, 2) + at(2, 0);
    let off = LO_CTX[0][ly.min(4)][lx.min(4)];
    let add = if mag > 512 { 4 } else { (mag + 64) >> 7 };
    ((off + add) as usize, hi)
}

fn hi_base(lx: usize, ly: usize) -> usize {
    // `(lx or ly) > 1`: a non-zero x wins, otherwise y.
    let cond = if lx != 0 { lx } else { ly };
    if cond > 1 {
        14
    } else {
        7
    }
}

fn dc_hi(levels: &[i32], stride: usize) -> usize {
    let mag = (levels[1] + levels[stride] + levels[stride + 1]) & 63;
    if mag > 12 {
        6
    } else {
        ((mag + 1) >> 1) as usize
    }
}

fn encode_block(
    ms: &mut MsacEnc,
    coef: &mut Coefs,
    above: &mut [u32],
    left: &mut [u32],
    small: &[i64; 16],
    x4: usize,
    y4: usize,
    plane: usize,
) {
    let ctx = TX_4 as usize;
    let ch = usize::from(plane != 0);
    let sctx = if plane != 0 { 7 } else { skip_ctx(above, left, x4, y4) };
    let scan = av1data::SCAN_4X4;
    let mut scaled = [0i64; 16];
    for i in 0..16 {
        scaled[i] = div_floor(small[i], 4);
    }
    let mut eob = -1i32;
    for i in (0..16).rev() {
        if scaled[scan[i] as usize] != 0 {
            eob = i as i32;
            break;
        }
    }
    if eob < 0 {
        ms.adapt_bool(&mut coef.skip[ctx][sctx], 1);
        above[x4] = 0x40;
        left[y4] = 0x40;
        return;
    }
    ms.adapt_bool(&mut coef.skip[ctx][sctx], 0);
    encode_eob(ms, coef, ctx, ch, eob);
    let n = 4usize;
    let stride_l = n;
    let mut levels = vec![0i32; stride_l * (n + 2)];
    let pos = |i: i32| {
        let rc = scan[i as usize] as usize;
        (rc / n, rc % n)
    };
    let store = |levels: &mut [i32], lx: usize, ly: usize, tok: i32| {
        let idx = lx * stride_l + ly;
        levels[idx] = if tok >= 3 { tok + (3 << 6) } else { tok * 0x41 };
    };
    let mut acs: Vec<i32> = Vec::new();
    let dc_tok = if eob != 0 {
        let (lx, ly) = pos(eob);
        let eob_ctx = 1 + usize::from(eob > 2) + usize::from(eob > 4);
        let tok = scaled[scan[eob as usize] as usize].unsigned_abs() as i32;
        let shown = encode_eob_tok(ms, coef, ctx, ch, eob_ctx, hi_base(lx, ly), tok);
        store(&mut levels, lx, ly, shown);
        acs.push(eob);
        for i in (1..eob).rev() {
            let (lx, ly) = pos(i);
            let (cctx, mag) = lo_ctx(&levels, stride_l, lx, ly);
            let tok = scaled[scan[i as usize] as usize].unsigned_abs() as i32;
            let shown = encode_base(
                ms,
                coef,
                ctx,
                ch,
                cctx,
                br_for(tok, hi_base(lx, ly), mag),
                tok,
            );
            store(&mut levels, lx, ly, shown);
            if tok != 0 {
                acs.push(i);
            }
        }
        let dc_tok = scaled[scan[0] as usize].unsigned_abs() as i32;
        encode_base(ms, coef, ctx, ch, 0, dc_hi(&levels, stride_l), dc_tok);
        dc_tok
    } else {
        let dc_tok = scaled[scan[0] as usize].unsigned_abs() as i32;
        encode_eob_tok(ms, coef, ctx, ch, 0, 0, dc_tok);
        dc_tok
    };
    let mut cul = 0i32;
    let neg = if dc_tok != 0 {
        let neg = i32::from(scaled[scan[0] as usize] < 0);
        ms.adapt_bool(
            &mut coef.dc_sign[ch][dc_sign_ctx(above, left, x4, y4)],
            neg,
        );
        if dc_tok >= 15 {
            ms.golomb(dc_tok - 15);
        }
        cul += dc_tok;
        neg
    } else {
        0
    };
    acs.sort();
    for i in acs {
        let coef_v = scaled[scan[i as usize] as usize];
        ms.equi(i32::from(coef_v < 0));
        let tok = coef_v.unsigned_abs() as i32;
        if tok >= 15 {
            ms.golomb(tok - 15);
        }
        cul += tok;
    }
    let sign_level = if dc_tok == 0 {
        64u32
    } else {
        ((neg - 1) & 128) as u32
    };
    above[x4] = (cul.min(63) as u32) | sign_level;
    left[y4] = above[x4];
}

fn plane_blocks(
    ms: &mut MsacEnc,
    coef: &mut Coefs,
    src: &[u8],
    rec: &mut [u8],
    stride: usize,
    n: usize,
    above: &mut [u32],
    left: &mut [u32],
    plane: usize,
) {
    let mut by = 0;
    while by < n {
        let mut bx = 0;
        while bx < n {
            let pred = dc_pred(rec, stride, bx, by);
            let mut raster = [0i64; 16];
            for row in 0..4 {
                for col in 0..4 {
                    let pix = i64::from(src[(by + row) * stride + bx + col]);
                    raster[row * 4 + col] = pix - i64::from(pred);
                }
            }
            let small = fwht(&raster);
            encode_block(ms, coef, above, left, &small, bx >> 2, by >> 2, plane);
            for row in 0..4 {
                for col in 0..4 {
                    rec[(by + row) * stride + bx + col] = src[(by + row) * stride + bx + col];
                }
            }
            bx += 4;
        }
        by += 4;
    }
}

/// 16×16 4:2:0 I420 to a raw OBU sequence. Empty is [`BOUNDS`]. Any other
/// length is [`UNSUPPORTED`].
pub fn encode(data: &[u8]) -> Av1Result<Vec<u8>> {
    if data.is_empty() {
        return fault(BOUNDS, "empty AV1 frame");
    }
    if data.len() != 384 {
        return fault(UNSUPPORTED, "this slice encodes a 16x16 4:2:0 frame");
    }
    let y = &data[..256];
    let u = &data[256..320];
    let v = &data[320..384];
    let mut rec_y = vec![0u8; 256];
    let mut rec_u = vec![128u8; 64];
    let mut rec_v = vec![128u8; 64];
    let mut ms = MsacEnc::new();
    let mut tabs = Tabs::load();
    let mut coef = Coefs::load(0);
    ms.symbol(&mut tabs.part3[0], PART_N[3], NONE as usize);
    ms.adapt_bool(&mut tabs.skip[0], 0);
    ms.symbol(&mut tabs.kfym[0][0], 12, DC as usize);
    ms.symbol(&mut tabs.uv_mode[DC as usize], 13, DC as usize);
    let mut above_y = [0x40u32; 64];
    let mut left_y = [0x40u32; 64];
    let mut above_c = [0x40u32; 64];
    let mut left_c = [0x40u32; 64];
    plane_blocks(&mut ms, &mut coef, y, &mut rec_y, 16, 16, &mut above_y, &mut left_y, 0);
    plane_blocks(&mut ms, &mut coef, u, &mut rec_u, 8, 8, &mut above_c, &mut left_c, 1);
    plane_blocks(&mut ms, &mut coef, v, &mut rec_v, 8, 8, &mut above_c, &mut left_c, 2);
    let tile = ms.done();
    let mut payload = frame_header();
    payload.extend(tile);
    let mut out = obu(2, &[]);
    out.extend(obu(1, &seq()));
    out.extend(obu(6, &payload));
    Ok(out)
}
