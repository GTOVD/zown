//! CELT encoder for one 10 ms narrowband mono frame.
//!
//! Translated from `zown/opusenc.py`. Analysis uses the same binary32
//! rounding as the decoder and a cold start. Pulse indices are chosen by
//! synthesis against that decoder.

use std::collections::BTreeMap;

use super::{
    bits2pulses, cdiv, celt_log2, celt_rsqrt, celt_sqrt, clt_compute_allocation, cwrsi, eb, exp_rotation,
    f32, fft_impl, fine_offset, float_to_int16, get_pulses, init_caps, mdct_backward, mdct_tw, pulses2bits,
    pvq_u, window_at, RateDec, Side, Store, BETA_COEF, BETA_INTRA, BITRES, BOUNDS, DECODE_BUFFER_SIZE,
    EMEANS, EPSILON, E_PROB_MODEL, INV_32768, MAX_FINE_BITS, MAX_LM, NB_EBANDS, NEG28, NEG9, OVERLAP,
    PREEMPH, PRED_COEF, SCALE_32768, SHORT_MDCT, SMALL_ENERGY_ICDF, SPREAD_ICDF, TELL_CORRECTION,
    TF_SELECT, TRIM_ICDF, UNSUPPORTED, VERY_SMALL,
};

const U32: u32 = 0xFFFF_FFFF;
const EC_SYM_BITS: i32 = 8;
const EC_CODE_BITS: i32 = 32;
const EC_SYM_MAX: u32 = (1 << EC_SYM_BITS) - 1;
const EC_CODE_SHIFT: i32 = EC_CODE_BITS - EC_SYM_BITS - 1;
const EC_CODE_TOP: u32 = 1 << (EC_CODE_BITS - 1);
const EC_CODE_BOT: u32 = EC_CODE_TOP >> EC_SYM_BITS;
const EC_UINT_BITS: i32 = 8;
const EC_WINDOW: i32 = 32;

const PAYLOAD: i32 = 19;
const FRAME: usize = 480;
const LM: i64 = 2;
const M: i64 = 1 << LM;
const START: usize = 0;
const END: usize = 13;
const SPREAD: i32 = 2;

fn floor_to_int(x: f64) -> i32 {
    let t = x as i64;
    if x >= 0.0 || x == t as f64 {
        t as i32
    } else {
        (t - 1) as i32
    }
}

fn bit_len_u32(n: u32) -> i32 {
    if n == 0 {
        0
    } else {
        32 - n.leading_zeros() as i32
    }
}

#[derive(Clone)]
struct Enc {
    buf: Vec<u8>,
    storage: usize,
    end_offs: usize,
    end_window: u32,
    nend_bits: i32,
    nbits_total: i32,
    offs: usize,
    rng: u32,
    rem: i32,
    val: u32,
    ext: i32,
    error: i32,
}

impl Enc {
    fn new(size: usize) -> Self {
        Self {
            buf: vec![0; size],
            storage: size,
            end_offs: 0,
            end_window: 0,
            nend_bits: 0,
            nbits_total: EC_CODE_BITS + 1,
            offs: 0,
            rng: EC_CODE_TOP,
            rem: -1,
            val: 0,
            ext: 0,
            error: 0,
        }
    }

    fn write(&mut self, value: u32) {
        if self.offs + self.end_offs >= self.storage {
            self.error = -1;
            return;
        }
        self.buf[self.offs] = (value & 255) as u8;
        self.offs += 1;
    }

    fn write_end(&mut self, value: u32) {
        if self.offs + self.end_offs >= self.storage {
            self.error = -1;
            return;
        }
        self.end_offs += 1;
        self.buf[self.storage - self.end_offs] = (value & 255) as u8;
    }

    fn carry_out(&mut self, c: u32) {
        if c != EC_SYM_MAX {
            let carry = c >> EC_SYM_BITS;
            if self.rem >= 0 {
                self.write((self.rem as u32).wrapping_add(carry));
            }
            if self.ext > 0 {
                let sym = EC_SYM_MAX.wrapping_add(carry) & EC_SYM_MAX;
                while self.ext > 0 {
                    self.write(sym);
                    self.ext -= 1;
                }
            }
            self.rem = (c & EC_SYM_MAX) as i32;
        } else {
            self.ext += 1;
        }
    }

    fn normalize(&mut self) {
        while self.rng <= EC_CODE_BOT {
            self.carry_out(self.val >> EC_CODE_SHIFT);
            self.val = (self.val << EC_SYM_BITS) & (EC_CODE_TOP - 1);
            self.rng = (self.rng << EC_SYM_BITS) & U32;
            self.nbits_total += EC_SYM_BITS;
        }
    }

    fn encode_sym(&mut self, fl: u32, fh: u32, ft: u32) {
        let r = if ft == 0 { 0 } else { self.rng / ft };
        if fl > 0 {
            let add = (self.rng as u64) - (r as u64) * (ft as u64 - fl as u64);
            self.val = (self.val as u64).wrapping_add(add) as u32;
            self.rng = ((r as u64) * (fh as u64 - fl as u64)) as u32;
        } else {
            self.rng = (self.rng as u64 - (r as u64) * (ft as u64 - fh as u64)) as u32;
        }
        self.normalize();
    }

    fn encode_bin(&mut self, fl: u32, fh: u32, bits: i32) {
        let r = self.rng >> bits;
        if fl > 0 {
            let span = (1u64 << bits) - fl as u64;
            let add = (self.rng as u64) - (r as u64) * span;
            self.val = (self.val as u64).wrapping_add(add) as u32;
            self.rng = ((r as u64) * (fh as u64 - fl as u64)) as u32;
        } else {
            let span = (1u64 << bits) - fh as u64;
            self.rng = (self.rng as u64 - (r as u64) * span) as u32;
        }
        self.normalize();
    }

    fn bit_logp(&mut self, val: i32, logp: i32) {
        let mut r = self.rng;
        let s = r >> logp;
        r = r.wrapping_sub(s);
        if val != 0 {
            self.val = self.val.wrapping_add(r);
        }
        self.rng = if val != 0 { s } else { r };
        self.normalize();
    }

    fn icdf(&mut self, s: usize, table: &[u32], ftb: i32) {
        let r = self.rng >> ftb;
        if s > 0 {
            let prev = table[s - 1];
            let add = (self.rng as u64) - (r as u64) * prev as u64;
            self.val = (self.val as u64).wrapping_add(add) as u32;
            self.rng = ((r as u64) * (prev as u64 - table[s] as u64)) as u32;
        } else {
            self.rng = (self.rng as u64 - (r as u64) * table[s] as u64) as u32;
        }
        self.normalize();
    }

    fn bits(&mut self, fl: u32, nbits: i32) {
        if nbits <= 0 {
            return;
        }
        let mut window = self.end_window;
        let mut used = self.nend_bits;
        if used + nbits > EC_WINDOW {
            while used >= EC_SYM_BITS {
                self.write_end(window & EC_SYM_MAX);
                window >>= EC_SYM_BITS;
                used -= EC_SYM_BITS;
            }
        }
        window = (window | ((fl & U32) << used)) & U32;
        used += nbits;
        self.end_window = window;
        self.nend_bits = used;
        self.nbits_total += nbits;
    }

    fn uint_sym(&mut self, fl: u32, ft: u32) {
        let top = ft.wrapping_sub(1);
        let mut ftb = bit_len_u32(top);
        if ftb > EC_UINT_BITS {
            ftb -= EC_UINT_BITS;
            let fs = (top >> ftb) + 1;
            let fh = fl >> ftb;
            self.encode_sym(fh, fh + 1, fs);
            self.bits(fl & ((1u32 << ftb) - 1), ftb);
        } else {
            self.encode_sym(fl, fl + 1, top + 1);
        }
    }

    fn tell(&self) -> i32 {
        self.nbits_total - bit_len_u32(self.rng)
    }

    fn tell_frac(&self) -> i32 {
        let nbits = self.nbits_total << BITRES;
        let mut length = bit_len_u32(self.rng);
        let r = if length >= 16 { self.rng >> (length - 16) } else { self.rng << (16 - length) };
        let mut b = (r >> 12) as i32 - 8;
        if r > TELL_CORRECTION[b as usize] {
            b += 1;
        }
        length = (length << 3) + b;
        nbits - length
    }

    fn done(&mut self) -> Vec<u8> {
        let mut l = EC_CODE_BITS - bit_len_u32(self.rng);
        let mut msk = (EC_CODE_TOP - 1) >> l;
        let mut end = self.val.wrapping_add(msk) & U32 & !msk;
        if (end | msk) >= self.val.wrapping_add(self.rng) {
            l += 1;
            msk >>= 1;
            end = self.val.wrapping_add(msk) & U32 & !msk;
        }
        while l > 0 {
            self.carry_out(end >> EC_CODE_SHIFT);
            end = (end << EC_SYM_BITS) & (EC_CODE_TOP - 1);
            l -= EC_SYM_BITS;
        }
        if self.rem >= 0 || self.ext > 0 {
            self.carry_out(0);
        }
        let mut window = self.end_window;
        let mut used = self.nend_bits;
        while used >= EC_SYM_BITS {
            self.write_end(window & EC_SYM_MAX);
            window >>= EC_SYM_BITS;
            used -= EC_SYM_BITS;
        }
        if self.error == 0 {
            for i in self.offs..self.storage - self.end_offs {
                self.buf[i] = 0;
            }
            if used > 0 {
                if self.end_offs >= self.storage {
                    self.error = -1;
                } else {
                    let spare = -l;
                    if self.offs + self.end_offs >= self.storage && spare < used {
                        window &= (1u32 << spare) - 1;
                        self.error = -1;
                    }
                    let at = self.storage - self.end_offs - 1;
                    self.buf[at] |= (window & 255) as u8;
                }
            }
        }
        self.buf.clone()
    }
}

struct Skip<'a> {
    enc: &'a mut Enc,
}

impl RateDec for Skip<'_> {
    fn bit_logp(&mut self, logp: u32) -> i32 {
        self.enc.bit_logp(1, logp as i32);
        1
    }

    fn uint(&mut self, ft: i64) -> u32 {
        self.enc.uint_sym(0, ft as u32);
        0
    }
}

fn laplace(enc: &mut Enc, mut value: i32, fs0: i32, decay: i32) -> i32 {
    let mut fl = 0i32;
    let mut fs = fs0;
    let mut val = value;
    if val != 0 {
        let s = if val < 0 { -1 } else { 0 };
        val = (val + s) ^ s;
        fl = fs;
        fs = super::laplace_get_freq1(fs, decay);
        let mut i = 1i32;
        while fs > 0 && i < val {
            fs *= 2;
            fl += fs + 2;
            fs = (fs * decay) >> 15;
            i += 1;
        }
        if fs == 0 {
            let mut ndi_max = 32768 - fl;
            ndi_max = (ndi_max - s) >> 1;
            let di = (val - i).min(ndi_max - 1);
            fl += 2 * di + 1 + s;
            fs = 1.min(32768 - fl);
            value = (i + di + s) ^ s;
        } else {
            fs += 1;
            fl += fs & (!s);
        }
    }
    enc.encode_bin((fl as u32) & U32, ((fl + fs) as u32) & U32, 15);
    value
}

fn icwrs(n: i64, y: &[i32]) -> u32 {
    let mut j = n - 1;
    let mut acc = if y[j as usize] < 0 { 1u32 } else { 0 };
    let mut k = y[j as usize].unsigned_abs() as i64;
    loop {
        j -= 1;
        acc = acc.wrapping_add(pvq_u(n - j, k));
        k += y[j as usize].unsigned_abs() as i64;
        if y[j as usize] < 0 {
            acc = acc.wrapping_add(pvq_u(n - j, k + 1));
        }
        if j <= 0 {
            break;
        }
    }
    acc
}

fn pvq_ft(n: i64, k: i64) -> u32 {
    pvq_u(n, k).wrapping_add(pvq_u(n, k + 1))
}

fn pvq_search(x: &[f64], k: i64) -> Vec<i32> {
    let n = x.len();
    let sign: Vec<i32> = x.iter().map(|v| if *v < 0.0 { 1 } else { 0 }).collect();
    let mut vec: Vec<f64> = x.iter().map(|v| f32(v.abs())).collect();
    let mut iy = vec![0i32; n];
    let mut y = vec![f32(0.0); n];
    let mut xy = f32(0.0);
    let mut yy = f32(0.0);
    let mut left = k;
    if k > (n as i64 >> 1) {
        let mut total = f32(0.0);
        for v in &vec {
            total = f32(total + *v);
        }
        if !(total > EPSILON && total < f32(64.0)) {
            vec = vec![f32(0.0); n];
            vec[0] = f32(1.0);
            total = f32(1.0);
        }
        let rcp = f32(f32(k as f64 + f32(0.8)) / total);
        for j in 0..n {
            iy[j] = floor_to_int(rcp * vec[j]);
            let yj = f32(iy[j] as f64);
            y[j] = yj;
            yy = f32(yy + f32(yj * yj));
            xy = f32(xy + f32(vec[j] * yj));
            y[j] = f32(yj * f32(2.0));
            left -= iy[j] as i64;
        }
    }
    if left > n as i64 + 3 {
        let tmp = f32(left as f64);
        yy = f32(yy + f32(tmp * tmp));
        yy = f32(yy + f32(tmp * y[0]));
        iy[0] += left as i32;
        left = 0;
    }
    for _ in 0..left {
        yy = f32(yy + f32(1.0));
        let mut rxy = f32(xy + vec[0]);
        let mut ryy = f32(yy + y[0]);
        let mut num = f32(rxy * rxy);
        let mut best_den = ryy;
        let mut best_num = num;
        let mut best = 0usize;
        for j in 1..n {
            rxy = f32(xy + vec[j]);
            ryy = f32(yy + y[j]);
            num = f32(rxy * rxy);
            if f32(best_den * num) > f32(ryy * best_num) {
                best_den = ryy;
                best_num = num;
                best = j;
            }
        }
        xy = f32(xy + vec[best]);
        yy = f32(yy + y[best]);
        y[best] = f32(y[best] + f32(2.0));
        iy[best] += 1;
    }
    for j in 0..n {
        if sign[j] != 0 {
            iy[j] = -iy[j];
        }
    }
    iy
}

fn mdct_forward(inp: &[f64], out: &mut [f64], out_base: usize, shift: usize, stride: i64) {
    let mut n = 1920i64;
    let mut trig_off = 0i64;
    for _ in 0..shift {
        n >>= 1;
        trig_off += n;
    }
    let n2 = n >> 1;
    let n4 = n >> 2;
    let overlap = OVERLAP;
    let mut xp1 = overlap >> 1;
    let mut xp2 = n2 - 1 + (overlap >> 1);
    let mut f = vec![f32(0.0); n2 as usize];
    let mut yp = 0i64;
    let mut wp1 = overlap >> 1;
    let mut wp2 = (overlap >> 1) - 1;
    let mut i = 0i64;
    let limit = (overlap + 3) >> 2;
    while i < limit {
        let a = window_at(wp2 as usize);
        let b = window_at(wp1 as usize);
        f[yp as usize] = f32(f32(a * inp[(xp1 + n2) as usize]) + f32(b * inp[xp2 as usize]));
        f[(yp + 1) as usize] = f32(f32(b * inp[xp1 as usize]) - f32(a * inp[(xp2 - n2) as usize]));
        xp1 += 2;
        xp2 -= 2;
        wp1 += 2;
        wp2 -= 2;
        yp += 2;
        i += 1;
    }
    let mut wp1 = 0i64;
    let mut wp2 = overlap - 1;
    while i < n4 - limit {
        f[yp as usize] = f32(inp[xp2 as usize]);
        f[(yp + 1) as usize] = f32(inp[xp1 as usize]);
        xp1 += 2;
        xp2 -= 2;
        yp += 2;
        i += 1;
    }
    while i < n4 {
        let a = window_at(wp1 as usize);
        let b = window_at(wp2 as usize);
        f[yp as usize] = f32(f32(f32(-a) * inp[(xp1 - n2) as usize]) + f32(b * inp[xp2 as usize]));
        f[(yp + 1) as usize] = f32(f32(b * inp[xp1 as usize]) + f32(a * inp[(xp2 + n2) as usize]));
        xp1 += 2;
        xp2 -= 2;
        wp1 += 2;
        wp2 -= 2;
        yp += 2;
        i += 1;
    }
    let scale = super::opusdata::FFT_SCALE[shift] as f64;
    let bitrev = super::opusdata::FFT_BITREV[shift];
    let mut f2 = vec![(f32(0.0), f32(0.0)); n4 as usize];
    yp = 0;
    for i in 0..n4 {
        let t0 = mdct_tw(trig_off + i);
        let t1 = mdct_tw(trig_off + n4 + i);
        let re = f[yp as usize];
        let im = f[(yp + 1) as usize];
        yp += 2;
        let yr = f32(f32(re * t0) - f32(im * t1));
        let yi = f32(f32(im * t0) + f32(re * t1));
        f2[bitrev[i as usize] as usize] = (f32(yr * scale), f32(yi * scale));
    }
    fft_impl(shift, &mut f2);
    let mut yp1 = 0i64;
    let mut yp2 = stride * (n2 - 1);
    for i in 0..n4 {
        let t0 = mdct_tw(trig_off + i);
        let t1 = mdct_tw(trig_off + n4 + i);
        let (fr, fi) = f2[i as usize];
        out[out_base + yp1 as usize] = f32(f32(fi * t1) - f32(fr * t0));
        out[out_base + yp2 as usize] = f32(f32(fr * t1) + f32(fi * t0));
        yp1 += 2 * stride;
        yp2 -= 2 * stride;
    }
}

fn analyse(samples: &[i16]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let mut m = f32(0.0);
    let inv = f32(1.0 / 32768.0);
    let mut frame = Vec::with_capacity(FRAME);
    for &s in samples {
        let y = f32(s as f64 * inv);
        let x = f32(y * SCALE_32768);
        frame.push(f32(x - m));
        m = f32(PREEMPH * x);
    }
    let mut sig = frame;
    sig.extend(std::iter::repeat(f32(0.0)).take(OVERLAP as usize));
    let mut freq = vec![f32(0.0); FRAME];
    for b in 0..M as usize {
        let start = b * SHORT_MDCT as usize;
        let block = &sig[start..start + SHORT_MDCT as usize + OVERLAP as usize];
        mdct_forward(block, &mut freq, b, MAX_LM as usize, M);
    }
    let mut logs = Vec::new();
    let mut norm = vec![f32(0.0); FRAME];
    for i in 0..END {
        let a = (M * eb(i as i64)) as usize;
        let bend = (M * eb(i as i64 + 1)) as usize;
        let mut acc = f32(0.0);
        for j in a..bend {
            let v = freq[j];
            acc = f32(acc + f32(v * v));
        }
        acc = f32(f32(1e-27) + acc);
        let amp = celt_sqrt(acc);
        logs.push(f32(celt_log2(amp) - EMEANS[i]));
        let inv_a = if amp != 0.0 { f32(1.0 / amp) } else { f32(0.0) };
        for j in a..bend {
            norm[j] = f32(freq[j] * inv_a);
        }
    }
    (freq, logs, norm)
}

fn coarse(enc: &mut Enc, logs: &[f64], intra: i32, budget: i32) -> (Vec<i32>, Vec<f64>, i32) {
    let prob = &E_PROB_MODEL[LM as usize][if intra != 0 { 1 } else { 0 }];
    let mut prev = f32(0.0);
    let coef = if intra != 0 { f32(0.0) } else { PRED_COEF[LM as usize] };
    let beta = if intra != 0 { BETA_INTRA } else { BETA_COEF[LM as usize] };
    let mut old = vec![f32(0.0); NB_EBANDS];
    let mut max_decay = f32(16.0);
    if END - START > 10 {
        max_decay = f32(16.0_f64.min(0.125 * PAYLOAD as f64));
    }
    let mut qis = Vec::new();
    let mut errs = Vec::new();
    let mut badness = 0i32;
    for i in 0..END {
        let x = logs[i];
        let old_c = if old[i] > NEG9 { old[i] } else { NEG9 };
        let pred = f32(f32(coef * old_c) + prev);
        let ferr = f32(x - pred);
        let mut qi = floor_to_int(ferr + 0.5);
        let base = if old[i] > NEG28 { old[i] } else { NEG28 };
        let decay_bound = f32(base - max_decay);
        if qi < 0 && x < decay_bound {
            qi += floor_to_int(f32(decay_bound - x) as f64);
            if qi > 0 {
                qi = 0;
            }
        }
        let qi0 = qi;
        let tell = enc.tell();
        let bits_left = budget - tell - 3 * (END as i32 - i as i32);
        if i != START && bits_left < 30 {
            if bits_left < 24 {
                qi = qi.min(1);
            }
            if bits_left < 16 {
                qi = qi.max(-1);
            }
        }
        if budget - tell >= 15 {
            let pi = 2 * i.min(20);
            qi = laplace(enc, qi, prob[pi] << 7, prob[pi + 1] << 6);
        } else if budget - tell >= 2 {
            qi = qi.max(-1).min(1);
            let sym = if qi < 0 { 1 } else if qi > 0 { 2 } else { 0 };
            enc.icdf(sym, &SMALL_ENERGY_ICDF, 2);
        } else if budget - tell >= 1 {
            qi = qi.min(0);
            enc.bit_logp(-qi, 1);
        } else {
            qi = -1;
        }
        badness += (qi0 - qi).abs();
        let q = f32(qi as f64);
        errs.push(f32(ferr - q));
        old[i] = f32(pred + q);
        prev = f32(f32(prev + q) - f32(beta * q));
        qis.push(qi);
    }
    (qis, errs, badness)
}

fn tf_encode(enc: &mut Enc, mut budget: i32) -> Vec<i64> {
    let mut logp = 2i32;
    let mut tell = enc.tell();
    let rsv = if LM > 0 && tell + logp + 1 <= budget { 1 } else { 0 };
    budget -= rsv;
    let mut curr = 0i32;
    let raw = vec![1i32; END];
    for i in 0..END {
        if tell + logp <= budget {
            enc.bit_logp(raw[i] ^ curr, logp);
            tell = enc.tell();
            curr = raw[i];
        }
        logp = 4;
    }
    if rsv != 0 {
        enc.bit_logp(0, 1);
    }
    let mut tf = Vec::new();
    for i in 0..END {
        tf.push(TF_SELECT[LM as usize][(4 + raw[i]) as usize] as i64);
    }
    tf
}

fn trim_index(logs: &[f64]) -> i32 {
    let mut trim = 4.0;
    let mut diff = 0.0;
    for i in 0..END - 1 {
        diff += logs[i] * (2 + 2 * i as i32 - END as i32) as f64;
    }
    diff /= (END - 1) as f64;
    let mut adj = diff + 1.0;
    if adj > 2.0 {
        adj = 2.0;
    }
    if adj < -2.0 {
        adj = -2.0;
    }
    trim -= adj / 6.0;
    let mut idx = floor_to_int(trim + 0.5);
    if idx < 0 {
        idx = 0;
    }
    if idx > 10 {
        idx = 10;
    }
    idx
}

fn fine_q2(error: f64, fine: i64) -> i32 {
    let frac = 1i32 << fine;
    let mut q2 = floor_to_int((error + 0.5) * frac as f64);
    if q2 > frac - 1 {
        q2 = frac - 1;
    }
    if q2 < 0 {
        q2 = 0;
    }
    q2
}

fn deinterleave(buf: &[f64], n0: usize, stride: usize) -> Vec<f64> {
    let mut tmp = vec![f32(0.0); n0 * stride];
    for i in 0..stride {
        for j in 0..n0 {
            tmp[i * n0 + j] = buf[j * stride + i];
        }
    }
    tmp
}

fn band_target(norm: &[f64], band: usize, n: usize, blocks: usize) -> Vec<f64> {
    let a = (M * eb(band as i64)) as usize;
    let mut raw: Vec<f64> = norm[a..a + n].to_vec();
    if blocks > 1 {
        let n0 = n / blocks;
        raw = deinterleave(&raw, n0, blocks);
    }
    raw
}

fn rotate(buf: &mut [f64], n: i64, direction: i64, stride: i64, k: i64) {
    let mut st = Store { x: buf.to_vec(), norm: Vec::new() };
    exp_rotation(&mut st, Side::X, 0, n, direction, stride, k, SPREAD);
    buf.copy_from_slice(&st.x);
}

fn quant_bands(
    enc: &mut Enc,
    norm: &[f64],
    pulses: &[i64],
    coded: i64,
    mut balance: i64,
    total_bits: i64,
    overrides: &BTreeMap<usize, u32>,
    layout: &mut BTreeMap<usize, (i64, i64)>,
) {
    for i in 0..END {
        let tell = enc.tell_frac() as i64;
        if i != START {
            balance -= tell;
        }
        let remaining = total_bits - tell - 1;
        let b = if i as i64 <= coded - 1 {
            let curr = cdiv(balance, 3.min(coded - i as i64));
            let mut b = pulses[i] + curr;
            if b > remaining + 1 {
                b = remaining + 1;
            }
            if b < 0 {
                b = 0;
            }
            if b > 16383 {
                b = 16383;
            }
            b
        } else {
            0
        };
        let n = M * eb(i as i64 + 1) - M * eb(i as i64);
        let mut q = bits2pulses(i, LM as i32, b);
        let mut curr_bits = pulses2bits(i, LM as i32, q);
        let mut remaining = remaining - curr_bits;
        while remaining < 0 && q > 0 {
            remaining += curr_bits;
            q -= 1;
            curr_bits = pulses2bits(i, LM as i32, q);
            remaining -= curr_bits;
        }
        let k = if q != 0 { get_pulses(q) } else { 0 };
        if k > 0 && n > 1 {
            let mut target = band_target(norm, i, n as usize, M as usize);
            rotate(&mut target, n, 1, M, k);
            let ft = pvq_ft(n, k);
            let idx = if let Some(&over) = overrides.get(&i) {
                if over < ft { over } else {
                    icwrs(n, &pvq_search(&target, k))
                }
            } else {
                icwrs(n, &pvq_search(&target, k))
            };
            enc.uint_sym(idx, ft);
        }
        if k > 0 {
            layout.insert(i, (n, k));
        }
        balance += pulses[i] + tell;
    }
}

fn build(logs: &[f64], norm: &[f64], overrides: &BTreeMap<usize, u32>, layout: Option<&mut BTreeMap<usize, (i64, i64)>>) -> Vec<u8> {
    let budget = PAYLOAD * 8;
    let mut enc = Enc::new(PAYLOAD as usize);
    enc.bit_logp(0, 15);
    if enc.tell() + 16 <= budget {
        enc.bit_logp(0, 1);
    }
    let mut is_transient = 1i32;
    if LM > 0 && enc.tell() + 3 <= budget {
        enc.bit_logp(is_transient, 3);
    } else {
        is_transient = 0;
    }
    let snap = enc.clone();
    enc.bit_logp(1, 3);
    let (_q1, e1, bad1) = coarse(&mut enc, logs, 1, budget);
    let intra_state = enc.clone();
    let tell_intra = enc.tell_frac();
    enc = snap;
    enc.bit_logp(0, 3);
    let (_q0, e0, bad0) = coarse(&mut enc, logs, 0, budget);
    let tell_inter = enc.tell_frac();
    let take_intra = true;
    let _ = (bad1, bad0, tell_intra, tell_inter, take_intra);
    enc = intra_state;
    let mut errs = e1;
    let _ = e0;
    tf_encode(&mut enc, budget);
    if enc.tell() + 4 <= budget {
        enc.icdf(SPREAD as usize, &SPREAD_ICDF, 5);
    }
    let mut cap = vec![0i64; NB_EBANDS];
    init_caps(LM, &mut cap);
    let dyn_log = 6i32;
    let total_frac = (budget as i64) << BITRES;
    let mut tell_f = enc.tell_frac() as i64;
    let offsets = vec![0i64; NB_EBANDS];
    for i in 0..END {
        let loop_logp = dyn_log as i64;
        if tell_f + (loop_logp << BITRES) < total_frac && cap[i] > 0 {
            enc.bit_logp(0, loop_logp as i32);
            tell_f = enc.tell_frac() as i64;
        }
    }
    let trim = trim_index(logs);
    if (enc.tell_frac() as i64) + (6 << BITRES) <= total_frac {
        enc.icdf(trim as usize, &TRIM_ICDF, 7);
    }
    let mut bits = ((budget as i64) << BITRES) - enc.tell_frac() as i64 - 1;
    let anti_rsv = if is_transient != 0 && LM >= 2 && bits >= ((LM + 2) << BITRES) {
        1 << BITRES
    } else {
        0
    };
    bits -= anti_rsv;
    let alloc = {
        let mut skip = Skip { enc: &mut enc };
        clt_compute_allocation(START, END, &offsets, &cap, trim as i64, bits, 1, LM, &mut skip)
    };
    for i in 0..END {
        if alloc.ebits[i] <= 0 {
            continue;
        }
        let q2 = fine_q2(errs[i], alloc.ebits[i]);
        enc.bits(q2 as u32, alloc.ebits[i] as i32);
        errs[i] = f32(errs[i] - fine_offset(q2 as i64, alloc.ebits[i]));
    }
    let total_bits = ((budget as i64) << BITRES) - anti_rsv;
    let mut local_layout = BTreeMap::new();
    let layout_ref = layout.unwrap_or(&mut local_layout);
    quant_bands(
        &mut enc,
        norm,
        &alloc.pulses,
        alloc.coded,
        alloc.balance,
        total_bits,
        overrides,
        layout_ref,
    );
    if anti_rsv != 0 {
        enc.bits(1, 1);
    }
    let mut bits_left = budget - enc.tell();
    let mut tails = Vec::new();
    for pr in 0..2 {
        let mut i = START;
        while i < END && bits_left >= 1 {
            if alloc.ebits[i] < MAX_FINE_BITS && alloc.fine_priority[i] == pr {
                let q2 = if errs[i] < 0.0 { 0 } else { 1 };
                tails.push(q2);
                bits_left -= 1;
            }
            i += 1;
        }
    }
    for q2 in tails {
        enc.bits(q2 as u32, 1);
    }
    let mut out = vec![0x90];
    out.extend(enc.done());
    out
}

fn mismatch(pkt: &[u8], target: &[u8]) -> i32 {
    match super::decode(pkt) {
        Ok(got) if got == target => 0,
        Ok(got) => {
            let mut n = 0i32;
            for i in (0..target.len()).step_by(2) {
                let a = i16::from_le_bytes([got[i], got[i + 1]]);
                let b = i16::from_le_bytes([target[i], target[i + 1]]);
                if a != b {
                    n += 1;
                }
            }
            n
        }
        Err(_) => 1 << 30,
    }
}

fn reliable_logs(freq: &[f64]) -> Vec<f64> {
    let mut logs = Vec::new();
    for i in 0..END {
        let a = (M * eb(i as i64)) as usize;
        let bend = (M * eb(i as i64 + 1)) as usize;
        let mut acc = f32(0.0);
        for j in a..bend {
            if j % M as usize == M as usize - 1 {
                continue;
            }
            let v = freq[j];
            acc = f32(acc + f32(v * v));
        }
        acc = f32(f32(1e-27) + acc);
        logs.push(f32(celt_log2(celt_sqrt(acc)) - EMEANS[i]));
    }
    logs
}

fn logs_from_freq(freq: &[f64]) -> Vec<f64> {
    let mut logs = Vec::new();
    for i in 0..END {
        let a = (M * eb(i as i64)) as usize;
        let bend = (M * eb(i as i64 + 1)) as usize;
        let mut acc = f32(0.0);
        for j in a..bend {
            let v = freq[j];
            acc = f32(acc + f32(v * v));
        }
        acc = f32(f32(1e-27) + acc);
        logs.push(f32(celt_log2(celt_sqrt(acc)) - EMEANS[i]));
    }
    logs
}

fn layout_for(logs: &[f64], norm: &[f64]) -> BTreeMap<usize, (i64, i64)> {
    let mut found = BTreeMap::new();
    let _ = build(logs, norm, &BTreeMap::new(), Some(&mut found));
    found
}

fn unit_shape(n: i64, k: i64, idx: u32) -> Vec<f64> {
    let mut y = vec![0i32; n as usize];
    let _ = cwrsi(n, k, idx, &mut y);
    let mut yy = f32(0.0);
    for &v in &y {
        yy = f32(yy + f32(v as f64 * v as f64));
    }
    let mut buf = vec![f32(0.0); n as usize];
    let scale = f32(celt_rsqrt(yy));
    for i in 0..n as usize {
        buf[i] = f32(scale * f32(y[i] as f64));
    }
    rotate(&mut buf, n, -1, M, k);
    let n0 = (n / M) as usize;
    if n0 * M as usize != n as usize {
        return buf;
    }
    let mut tmp = vec![0.0; n as usize];
    for i in 0..M as usize {
        for j in 0..n0 {
            tmp[j * M as usize + i] = buf[i * n0 + j];
        }
    }
    tmp
}

fn fit_candidates(freq: &[f64], band: usize, n: i64, k: i64) -> Vec<(f64, u32, f64, Vec<f64>)> {
    let a = (M * eb(band as i64)) as usize;
    let target: Vec<f64> = freq[a..a + n as usize].iter().map(|v| *v as f64).collect();
    let mask: Vec<bool> = (0..n as usize).map(|i| ((a + i) % M as usize) != M as usize - 1).collect();
    if !mask.iter().any(|m| *m) {
        return Vec::new();
    }
    let mut best = 1e300;
    let mut rows = Vec::new();
    let ft = pvq_ft(n, k);
    for idx in 0..ft {
        let shape = unit_shape(n, k, idx);
        let mut num = 0.0;
        let mut den = 0.0;
        for i in 0..n as usize {
            if mask[i] {
                num += target[i] * shape[i];
                den += shape[i] * shape[i];
            }
        }
        if den < 1e-18 {
            continue;
        }
        let gain = num / den;
        if gain <= 0.0 {
            continue;
        }
        let mut err = 0.0;
        for i in 0..n as usize {
            if mask[i] {
                let diff = gain * shape[i] - target[i];
                err += diff * diff;
            }
        }
        if err < best {
            best = err;
        }
        rows.push((err, idx, gain, shape));
    }
    rows.into_iter().filter(|row| row.0 <= best + 1e-3).collect()
}

fn synth_cost(choice: &BTreeMap<usize, (f64, u32, f64, Vec<f64>)>, samples: &[i16]) -> (i64, i32) {
    let mut freq = vec![f32(0.0); FRAME];
    for (band, row) in choice {
        let (_err, _idx, gain, shape) = row;
        let a = (M * eb(*band as i64)) as usize;
        for (i, val) in shape.iter().enumerate() {
            freq[a + i] = f32(*gain * *val);
        }
    }
    let mut mem = vec![f32(0.0); (DECODE_BUFFER_SIZE + OVERLAP) as usize];
    let off = (DECODE_BUFFER_SIZE - FRAME as i64) as usize;
    for blk in 0..M as usize {
        mdct_backward(&freq, blk, &mut mem, off + (SHORT_MDCT as usize) * blk, MAX_LM, M);
    }
    let mut state = f32(0.0);
    let mut sse = 0i64;
    let mut misses = 0i32;
    for j in 0..FRAME {
        let tmp = f32(f32(mem[off + j] + VERY_SMALL) + state);
        state = f32(PREEMPH * tmp);
        let sample = float_to_int16(f32(tmp * INV_32768)) as i64;
        let delta = sample - samples[j] as i64;
        sse += delta * delta;
        if delta != 0 {
            misses += 1;
        }
    }
    (sse, misses)
}

fn select_pulses(
    freq: &[f64],
    layout: &BTreeMap<usize, (i64, i64)>,
    samples: &[i16],
) -> BTreeMap<usize, (f64, u32, f64, Vec<f64>)> {
    let mut goods = BTreeMap::new();
    for (band, (n, k)) in layout {
        if *k <= 0 || *n <= 1 {
            continue;
        }
        let found = fit_candidates(freq, *band, *n, *k);
        if !found.is_empty() {
            goods.insert(*band, found);
        }
    }
    if goods.is_empty() {
        return BTreeMap::new();
    }
    let mut choice = BTreeMap::new();
    for (band, rows) in &goods {
        choice.insert(*band, rows[0].clone());
    }
    for _ in 0..4 {
        let mut moved = false;
        let bands: Vec<usize> = goods.keys().copied().collect();
        for band in bands {
            let rows = &goods[&band];
            let mut best = synth_cost(&choice, samples);
            let mut best_row = choice[&band].clone();
            for row in rows {
                if row.1 == best_row.1 {
                    continue;
                }
                let mut trial = choice.clone();
                trial.insert(band, row.clone());
                let cost = synth_cost(&trial, samples);
                if cost < best {
                    best = cost;
                    best_row = row.clone();
                }
            }
            if best_row.1 != choice[&band].1 {
                choice.insert(band, best_row);
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }
    choice
}

fn freq_from_pulses(freq: &[f64], choice: &BTreeMap<usize, (f64, u32, f64, Vec<f64>)>) -> Vec<f64> {
    let mut out = vec![f32(0.0); FRAME];
    let mut coded = vec![false; FRAME];
    for (band, row) in choice {
        let (_err, _idx, gain, shape) = row;
        let a = (M * eb(*band as i64)) as usize;
        for (i, val) in shape.iter().enumerate() {
            out[a + i] = f32(*gain * *val);
            coded[a + i] = true;
        }
        let bend = (M * eb(*band as i64 + 1)) as usize;
        for j in a..bend {
            coded[j] = coded[j] || j < a + shape.len();
        }
    }
    for j in 0..FRAME {
        if !coded[j] {
            out[j] = freq[j];
        }
    }
    out
}

/// 960 s16le bytes to one TOC config-18 packet. Empty is [`BOUNDS`].
/// Any other length is [`UNSUPPORTED`].
pub fn encode(data: &[u8]) -> Result<Vec<u8>, (&'static str, String)> {
    if data.is_empty() {
        return super::fault(BOUNDS, "empty input");
    }
    if data.len() != FRAME * 2 {
        return super::fault(UNSUPPORTED, "only 480 s16le samples are supported");
    }
    let mut samples = [0i16; FRAME];
    for i in 0..FRAME {
        samples[i] = i16::from_le_bytes([data[2 * i], data[2 * i + 1]]);
    }
    let (freq, _logs, norm) = analyse(&samples);
    let rel = reliable_logs(&freq);
    let layout = layout_for(&rel, &norm);
    let mut choice = select_pulses(&freq, &layout, &samples);
    let mut built = freq_from_pulses(&freq, &choice);
    let mut logs = logs_from_freq(&built);
    let layout2 = layout_for(&logs, &norm);
    if layout2 != layout {
        choice = select_pulses(&freq, &layout2, &samples);
        built = freq_from_pulses(&freq, &choice);
        logs = logs_from_freq(&built);
    }
    let overrides: BTreeMap<usize, u32> = choice.iter().map(|(b, row)| (*b, row.1)).collect();
    let packet = build(&logs, &norm, &overrides, None);
    if mismatch(&packet, data) != 0 {
        return super::fault(UNSUPPORTED, "frame is outside the transient narrowband codebook");
    }
    Ok(packet)
}
