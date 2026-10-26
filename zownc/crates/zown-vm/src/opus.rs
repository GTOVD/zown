//! CELT-only Opus decoder for one 10 ms narrowband mono frame.
//!
//! Line-by-line translation of `zown/opus.py`. Every `f32()` step rounds to
//! binary32 the way `struct.pack/unpack('<f')` does. The float build treats
//! QCONST16 as the identity, so coarse energy is `tmp = coef*old + prev + q`.

#![allow(dead_code)]

#[path = "opusdata.rs"]
mod opusdata;

use super::error::{BOUNDS, INTEGRITY_FAIL, UNSUPPORTED};

use std::cmp::{max, min};

const NB_EBANDS: usize = 21;
const SHORT_MDCT: i64 = 120;
const MAX_LM: i64 = 3;
const OVERLAP: i64 = 120;
const DECODE_BUFFER_SIZE: i64 = 2048;
const COMBFILTER_MINPERIOD: i64 = 15;
const BITRES: i64 = 3;
const MAX_FINE_BITS: i64 = 8;
const FINE_OFFSET: i64 = 21;
const QTHETA_OFFSET: i64 = 4;
const QTHETA_OFFSET_TWOPHASE: i64 = 16;
const ALLOC_STEPS: i64 = 6;
const SPREAD_NONE: i32 = 0;
const SPREAD_AGGRESSIVE: i32 = 3;
const EC_CODE_BOT: u32 = 1 << 23;

const TRIM_ICDF: [u32; 11] = [126, 124, 119, 109, 87, 41, 19, 9, 4, 2, 0];
const SPREAD_ICDF: [u32; 4] = [25, 23, 2, 0];
const TAPSET_ICDF: [u32; 3] = [2, 1, 0];
const SMALL_ENERGY_ICDF: [u32; 3] = [2, 1, 0];
const LOG2_FRAC: [i64; 24] = [
    0, 8, 13, 16, 19, 21, 23, 24, 26, 27, 28, 29, 30, 31, 32, 32, 33, 34, 34, 35, 36, 36, 37, 37,
];
const EXP2_TABLE8: [i64; 8] = [16384, 17866, 19483, 21247, 23170, 25267, 27554, 30048];
const ORDERY: [i32; 30] = [
    1, 0, 3, 0, 2, 1, 7, 0, 4, 3, 6, 1, 5, 2, 15, 0, 8, 7, 12, 3, 11, 4, 14, 1, 9, 6, 13, 2, 10, 5,
];
const BIT_INTERLEAVE: [i32; 16] = [0, 1, 1, 1, 2, 3, 3, 3, 2, 3, 3, 3, 2, 3, 3, 3];
const BIT_DEINTERLEAVE: [i32; 16] = [
    0x00, 0x03, 0x0C, 0x0F, 0x30, 0x33, 0x3C, 0x3F, 0xC0, 0xC3, 0xCC, 0xCF, 0xF0, 0xF3, 0xFC, 0xFF,
];
const SPREAD_FACTOR: [i64; 3] = [15, 10, 5];
const TELL_CORRECTION: [u32; 8] = [35733, 38967, 42495, 46340, 50535, 55109, 60097, 65535];

// e_prob_model[LM][intra][2*band + {0:prob0, 1:decay}], 480-sample row is LM=2.
const E_PROB_MODEL: [[[i32; 42]; 2]; 4] = [
    [
        [
            72, 127, 65, 129, 66, 128, 65, 128, 64, 128, 62, 128, 64, 128, 64, 128, 92, 78, 92, 79, 92, 78,
            90, 79, 116, 41, 115, 40, 114, 40, 132, 26, 132, 26, 145, 17, 161, 12, 176, 10, 177, 11,
        ],
        [
            24, 179, 48, 138, 54, 135, 54, 132, 53, 134, 56, 133, 55, 132, 55, 132, 61, 114, 70, 96, 74, 88,
            75, 88, 87, 74, 89, 66, 91, 67, 100, 59, 108, 50, 120, 40, 122, 37, 97, 43, 78, 50,
        ],
    ],
    [
        [
            83, 78, 84, 81, 88, 75, 86, 74, 87, 71, 90, 73, 93, 74, 93, 74, 109, 40, 114, 36, 117, 34, 117,
            34, 143, 17, 145, 18, 146, 19, 162, 12, 165, 10, 178, 7, 189, 6, 190, 8, 177, 9,
        ],
        [
            23, 178, 54, 115, 63, 102, 66, 98, 69, 99, 74, 89, 71, 91, 73, 91, 78, 89, 86, 80, 92, 66, 93,
            64, 102, 59, 103, 60, 104, 60, 117, 52, 123, 44, 138, 35, 133, 31, 97, 38, 77, 45,
        ],
    ],
    [
        [
            61, 90, 93, 60, 105, 42, 107, 41, 110, 45, 116, 38, 113, 38, 112, 38, 124, 26, 132, 27, 136, 19,
            140, 20, 155, 14, 159, 16, 158, 18, 170, 13, 177, 10, 187, 8, 192, 6, 175, 9, 159, 10,
        ],
        [
            21, 178, 59, 110, 71, 86, 75, 85, 84, 83, 91, 66, 88, 73, 87, 72, 92, 75, 98, 72, 105, 58, 107,
            54, 115, 52, 114, 55, 112, 56, 129, 51, 132, 40, 150, 33, 140, 29, 98, 35, 77, 42,
        ],
    ],
    [
        [
            42, 121, 96, 66, 108, 43, 111, 40, 117, 44, 123, 32, 120, 36, 119, 33, 127, 33, 134, 34, 139,
            21, 147, 23, 152, 20, 158, 25, 154, 26, 166, 21, 173, 16, 184, 13, 184, 10, 150, 13, 139, 15,
        ],
        [
            22, 178, 63, 114, 74, 82, 84, 83, 92, 82, 103, 62, 96, 72, 96, 67, 101, 73, 107, 72, 113, 55,
            118, 52, 125, 52, 118, 52, 117, 55, 135, 49, 137, 39, 157, 32, 145, 29, 97, 33, 77, 40,
        ],
    ],
];

const TF_SELECT: [[i32; 8]; 4] = [
    [0, -1, 0, -1, 0, -1, 0, -1],
    [0, -1, 0, -2, 1, 0, 1, -1],
    [0, -2, 0, -3, 2, 0, 1, -1],
    [0, -2, 0, -3, 3, 0, 1, -1],
];

type OpusResult<T> = Result<T, (&'static str, String)>;

const fn f32(x: f64) -> f64 {
    x as f32 as f64
}

const EMEANS: [f64; 25] = [
    f32(6.437500),
    f32(6.250000),
    f32(5.750000),
    f32(5.312500),
    f32(5.062500),
    f32(4.812500),
    f32(4.500000),
    f32(4.375000),
    f32(4.875000),
    f32(4.687500),
    f32(4.562500),
    f32(4.437500),
    f32(4.875000),
    f32(4.625000),
    f32(4.312500),
    f32(4.500000),
    f32(4.375000),
    f32(4.625000),
    f32(4.750000),
    f32(4.437500),
    f32(3.750000),
    f32(3.750000),
    f32(3.750000),
    f32(3.750000),
    f32(3.750000),
];
const PRED_COEF: [f64; 4] = [
    f32(29440.0 / 32768.0),
    f32(26112.0 / 32768.0),
    f32(21248.0 / 32768.0),
    f32(16384.0 / 32768.0),
];
const BETA_COEF: [f64; 4] = [
    f32(30147.0 / 32768.0),
    f32(22282.0 / 32768.0),
    f32(12124.0 / 32768.0),
    f32(6554.0 / 32768.0),
];
const BETA_INTRA: f64 = f32(4915.0 / 32768.0);
const VERY_SMALL: f64 = f32(1e-30);
const EPSILON: f64 = f32(1e-15);
const PI: f64 = f32(3.141592653);
const HALF_PI: f64 = f32(f32(0.5) * PI);
const INV_32768: f64 = f32(1.0 / 32768.0);
const SCALE_32768: f64 = f32(32768.0);
const PREEMPH: f64 = f32(0.85000610);
const HAAR_C: f64 = f32(0.70710678);
const FFT_C: f64 = f32(0.7071067812);
const SQRT2: f64 = f32(1.41421356);
const NEG9: f64 = f32(-9.0);
const NEG28: f64 = f32(-28.0);
const POSTFILTER_STEP: f64 = f32(0.09375);
const GAINS: [[f64; 3]; 3] = [
    [f32(0.3066406250), f32(0.2170410156), f32(0.1296386719)],
    [f32(0.4638671875), f32(0.2680664062), f32(0.0)],
    [f32(0.7998046875), f32(0.1000976562), f32(0.0)],
];

fn fault<T>(code: &'static str, msg: &str) -> OpusResult<T> {
    Err((code, msg.to_string()))
}

fn fmin(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}

fn fmax(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

fn s16(x: i64) -> i64 {
    let x = x & 0xFFFF;
    if x >= 0x8000 {
        x - 0x10000
    } else {
        x
    }
}

fn floor_div(n: i64, d: i64) -> i64 {
    let n = n as i128;
    let d = d as i128;
    let q = n / d;
    let r = n % d;
    let q = if r != 0 && (n < 0) != (d < 0) { q - 1 } else { q };
    q as i64
}

fn cdiv(n: i64, d: i64) -> i64 {
    let n = n as i128;
    let d = d as i128;
    let (n, d) = if d < 0 { (-n, -d) } else { (n, d) };
    let q = if n >= 0 { n / d } else { -((-n) / d) };
    q as i64
}

fn asr(v: i64, s: i64) -> i64 {
    if s < 0 {
        panic!("negative shift count");
    }
    if s == 0 {
        return v;
    }
    if s >= 63 {
        return if v < 0 { -1 } else { 0 };
    }
    v >> s
}

fn shl(v: i64, s: i64) -> i64 {
    if s < 0 {
        panic!("negative shift count");
    }
    if s >= 63 {
        panic!("shift overflow");
    }
    v << s
}

fn bit_length(n: i64) -> i32 {
    if n == 0 {
        0
    } else {
        64 - n.unsigned_abs().leading_zeros() as i32
    }
}

fn frac_mul16(a: i64, b: i64) -> i64 {
    (16384 + s16(a) * s16(b)) >> 15
}

fn eb(i: i64) -> i64 {
    let t = &opusdata::EBAND5MS;
    let n = t.len() as i64;
    let j = if i < 0 { i + n } else { i };
    t[j as usize] as i64
}

fn logn_at(i: i64) -> i64 {
    opusdata::LOGN400[i as usize] as i64
}

fn alloc_at(i: i64) -> i64 {
    opusdata::BAND_ALLOCATION[i as usize] as i64
}

fn window_at(i: usize) -> f64 {
    opusdata::WINDOW120[i] as f64
}

fn mdct_tw(i: i64) -> f64 {
    opusdata::MDCT_TWIDDLES960[i as usize] as f64
}

fn twiddle(i: usize) -> (f64, f64) {
    let t = opusdata::FFT_TWIDDLES[i];
    (t.0 as f64, t.1 as f64)
}

struct EcDec {
    buf: Vec<u8>,
    storage: usize,
    end_offs: usize,
    end_window: u32,
    nend_bits: i64,
    nbits_total: i64,
    offs: usize,
    rng: u32,
    rem: u32,
    val: u32,
    error: i32,
    read_err: i32,
    front_past: i32,
    end_past: i32,
    ext: u32,
}

impl EcDec {
    fn new(data: &[u8]) -> Self {
        let mut s = Self {
            buf: data.to_vec(),
            storage: data.len(),
            end_offs: 0,
            end_window: 0,
            nend_bits: 0,
            nbits_total: 9,
            offs: 0,
            rng: 1 << 7,
            rem: 0,
            val: 0,
            error: 0,
            read_err: 0,
            front_past: 0,
            end_past: 0,
            ext: 0,
        };
        s.rem = s.read();
        s.val = s.rng.wrapping_sub(1).wrapping_sub(s.rem >> 1);
        // The first read's overrun count is cleared by the following stores
        // once `front_past` exists. An empty buffer still overruns in normalize.
        s.front_past = 0;
        s.end_past = 0;
        s.normalize();
        s
    }

    fn read(&mut self) -> u32 {
        if self.offs >= self.storage {
            self.read_err = 1;
            self.front_past += 1;
            return 0;
        }
        let b = self.buf[self.offs];
        self.offs += 1;
        b as u32
    }

    fn read_end(&mut self) -> u32 {
        if self.end_offs >= self.storage {
            self.read_err = 1;
            self.end_past += 1;
            return 0;
        }
        self.end_offs += 1;
        self.buf[self.storage - self.end_offs] as u32
    }

    fn normalize(&mut self) {
        while self.rng <= EC_CODE_BOT {
            self.nbits_total += 8;
            self.rng = self.rng.wrapping_shl(8);
            let sym = self.rem;
            self.rem = self.read();
            let sym = ((sym << 8) | self.rem) >> 1;
            let inv = (sym ^ 0xFF) & 0xFF;
            self.val = self.val.wrapping_shl(8).wrapping_add(inv) & 0x7FFF_FFFF;
        }
    }

    fn tell(&self) -> i64 {
        let bl = (32 - self.rng.leading_zeros()) as i64;
        self.nbits_total - bl
    }

    fn tell_frac(&self) -> i64 {
        let nbits = self.nbits_total << BITRES;
        let mut length = (32 - self.rng.leading_zeros()) as i64;
        let r = self.rng >> (length as u32 - 16);
        let mut b = (r >> 12) as i64 - 8;
        if r > TELL_CORRECTION[b as usize] {
            b += 1;
        }
        length = (length << 3) + b;
        nbits - length
    }

    fn decode(&mut self, ft: u32) -> u32 {
        self.ext = if ft == 0 { 0 } else { self.rng / ft };
        if self.ext == 0 {
            self.error = 1;
            return 0;
        }
        let s = self.val / self.ext;
        ft - min(s + 1, ft)
    }

    fn decode_bin(&mut self, bits: u32) -> u32 {
        self.ext = self.rng >> bits;
        let ft = 1u32 << bits;
        if self.ext == 0 {
            self.error = 1;
            return 0;
        }
        let s = self.val / self.ext;
        ft - min(s + 1, ft)
    }

    fn update(&mut self, fl: u32, fh: u32, ft: u32) {
        let s = self.ext.wrapping_mul(ft.wrapping_sub(fh));
        self.val = self.val.wrapping_sub(s);
        if fl > 0 {
            self.rng = self.ext.wrapping_mul(fh.wrapping_sub(fl));
        } else {
            self.rng = self.rng.wrapping_sub(s);
        }
        self.normalize();
    }

    fn bit_logp(&mut self, logp: u32) -> i32 {
        let r = self.rng;
        let d = self.val;
        let s = r >> logp;
        let ret = if d < s { 1 } else { 0 };
        if ret == 0 {
            self.val = d.wrapping_sub(s);
        }
        self.rng = if ret != 0 { s } else { r.wrapping_sub(s) };
        self.normalize();
        ret
    }

    fn icdf(&mut self, table: &[u32], ftb: u32) -> i32 {
        let mut s = self.rng;
        let d = self.val;
        let r = s >> ftb;
        let mut ret: i32 = -1;
        loop {
            let t = s;
            ret += 1;
            s = r.wrapping_mul(table[ret as usize]);
            if d >= s {
                self.val = d.wrapping_sub(s);
                self.rng = t.wrapping_sub(s);
                self.normalize();
                return ret;
            }
        }
    }

    fn bits(&mut self, n: i32) -> u32 {
        if n < 0 {
            panic!("negative bit count");
        }
        let mut window = self.end_window;
        let mut available = self.nend_bits;
        if available < n as i64 {
            loop {
                if available < 0 || available >= 32 {
                    panic!("raw-bit window shift");
                }
                let b = self.read_end();
                window |= b.wrapping_shl(available as u32);
                available += 8;
                if available > 24 {
                    break;
                }
            }
        }
        let ret = if n == 0 {
            0
        } else if n >= 32 {
            window
        } else {
            window & ((1u32 << n) - 1)
        };
        if n > 0 {
            window = if n >= 32 { 0 } else { window >> n };
        }
        self.end_window = window;
        self.nend_bits = available - n as i64;
        self.nbits_total += n as i64;
        ret
    }

    fn uint(&mut self, ft: i64) -> u32 {
        if ft <= 1 {
            self.error = 1;
            return 0;
        }
        let top = (ft - 1) as u32;
        let mut ftb = 32 - top.leading_zeros();
        if ftb > 8 {
            ftb -= 8;
            let fs = (top >> ftb) + 1;
            let s = self.decode(fs);
            self.update(s, s + 1, fs);
            let t = s.wrapping_shl(ftb) | self.bits(ftb as i32);
            if t <= top {
                return t;
            }
            self.error = 1;
            return top;
        }
        let s = self.decode(ft as u32);
        self.update(s, s + 1, ft as u32);
        s
    }
}

fn laplace_get_freq1(fs0: i32, decay: i32) -> i32 {
    let ft = 32768 - 2 * 16 - fs0;
    (ft * (16384 - decay)) >> 15
}

fn ec_laplace_decode(dec: &mut EcDec, mut fs: i32, decay: i32) -> i32 {
    let mut val = 0i32;
    let fm = dec.decode_bin(15) as i32;
    let mut fl = 0i32;
    if fm >= fs {
        val = 1;
        fl = fs;
        fs = laplace_get_freq1(fs, decay) + 1;
        while fs > 1 && fm >= fl + 2 * fs {
            fs *= 2;
            fl += fs;
            fs = (((fs - 2) * decay) >> 15) + 1;
            val += 1;
        }
        if fs <= 1 {
            let di = (fm - fl) >> 1;
            val += di;
            fl += 2 * di;
        }
        if fm < fl + fs {
            val = -val;
        } else {
            fl += fs;
        }
    }
    let fh = min(fl + fs, 32768);
    dec.update(fl as u32, fh as u32, 32768);
    val
}

fn celt_sqrt(x: f64) -> f64 {
    f32(f32(x).sqrt())
}

fn celt_rsqrt(x: f64) -> f64 {
    f32(1.0 / celt_sqrt(x))
}

fn floor_to_i32(x: f64) -> i32 {
    let trunc = x as i32;
    if x >= 0.0 || x == trunc as f64 {
        trunc
    } else {
        trunc - 1
    }
}

fn celt_exp2(x: f64) -> f64 {
    let x = f32(x);
    let integer = floor_to_i32(x);
    if integer < -50 {
        return f32(0.0);
    }
    let frac = f32(x - integer as f64);
    let mut res = f32(f32(0.078024523) * frac);
    res = f32(f32(0.22606716) + res);
    res = f32(frac * res);
    res = f32(f32(0.69583354) + res);
    res = f32(frac * res);
    res = f32(f32(0.99992522) + res);
    let bits = (res as f32).to_bits();
    let add = (integer as u32).wrapping_shl(23);
    let bits = bits.wrapping_add(add) & 0x7FFF_FFFF;
    f32::from_bits(bits) as f64
}

fn celt_log2(x: f64) -> f64 {
    let mut bits = (f32(x) as f32).to_bits();
    let integer = (bits >> 23) as i32 - 127;
    bits = bits.wrapping_sub((integer as u32).wrapping_shl(23));
    let mut frac = f32::from_bits(bits) as f64;
    frac = f32(frac - f32(1.5));
    let mut acc = f32(f32(0.16541097) * frac);
    acc = f32(f32(-0.33951290) + acc);
    acc = f32(frac * acc);
    acc = f32(f32(0.95909232) + acc);
    acc = f32(frac * acc);
    frac = f32(f32(-0.41445418) + acc);
    f32((1 + integer) as f64 + frac)
}

fn k_cos(z: f64) -> f64 {
    let z2 = z * z;
    let w2 = z2 * z2;
    let mut r = z2
        * (4.16666666666666019037e-02
            + z2 * (-1.38888888888741095749e-03 + z2 * 2.48015872894767294178e-05));
    r += w2
        * w2
        * (-2.75573143513906633035e-07
            + z2 * (2.08757232129817482790e-09 + z2 * -1.13596475577881948265e-11));
    let hz = 0.5 * z2;
    let w = 1.0 - hz;
    w + (((1.0 - w) - hz) + (z2 * r))
}

fn k_sin(z: f64) -> f64 {
    let z2 = z * z;
    let v = z * z2;
    let r = 8.33333333332248946124e-03
        + z2 * (-1.98412698298579493134e-04
            + z2 * (2.75573137070700676789e-06
                + z2 * (-2.50507602534068634195e-08 + z2 * 1.58969099521155010221e-10)));
    z + v * (-1.66666666666666324348e-01 + z2 * r)
}

fn cos_f64(mut x: f64) -> f64 {
    if x < 0.0 {
        x = -x;
    }
    let pio2_1 = 1.57079632673412561417;
    let pio2_2 = 6.07710050650619224932e-11;
    let pio2_3 = 2.02226624879595063154e-21;
    let n = (x / 1.5707963267948966 + 0.5) as i32;
    let nf = n as f64;
    let r = ((x - nf * pio2_1) - nf * pio2_2) - nf * pio2_3;
    match n & 3 {
        0 => k_cos(r),
        1 => -k_sin(r),
        2 => -k_cos(r),
        _ => k_sin(r),
    }
}

fn celt_cos_norm(x: f64) -> f64 {
    let arg = f32(HALF_PI * f32(x));
    f32(cos_f64(arg))
}

fn isqrt32(mut val: u32) -> u32 {
    if val == 0 {
        return 0;
    }
    let mut g: u32 = 0;
    let mut bshift = ((32 - val.leading_zeros()) - 1) >> 1;
    let mut b = 1u32 << bshift;
    loop {
        let t = ((((g as u64) << 1) + b as u64) << bshift) as u32;
        if t <= val {
            g += b;
            val -= t;
        }
        b >>= 1;
        if bshift == 0 {
            break;
        }
        bshift -= 1;
    }
    g
}

fn bitexact_cos(x: i64) -> i64 {
    let x = s16(x);
    let tmp = (4096 + x * x) >> 13;
    let mut x2 = s16(tmp);
    let mut inner = frac_mul16(-626, x2);
    inner = frac_mul16(x2, 8277 + inner);
    inner = frac_mul16(x2, -7651 + inner);
    x2 = s16((32767 - x2) + inner);
    1 + x2
}

fn bitexact_log2tan(mut isin: i64, mut icos: i64) -> i64 {
    let lc = bit_length(icos);
    let ls = bit_length(isin);
    icos = s16(shl(icos, 15 - lc as i64));
    isin = s16(shl(isin, 15 - ls as i64));
    (ls as i64 - lc as i64) * (1i64 << 11)
        + frac_mul16(isin, frac_mul16(isin, -2597) + 7932)
        - frac_mul16(icos, frac_mul16(icos, -2597) + 7932)
}

fn unquant_coarse_energy(
    dec: &mut EcDec,
    start: usize,
    end: usize,
    old_e: &mut [f64],
    intra: i32,
    lm: usize,
) {
    let prob = &E_PROB_MODEL[lm][if intra != 0 { 1 } else { 0 }];
    let mut prev = [f32(0.0), f32(0.0)];
    let (coef, beta) = if intra != 0 {
        (f32(0.0), BETA_INTRA)
    } else {
        (PRED_COEF[lm], BETA_COEF[lm])
    };
    let budget = dec.storage as i64 * 8;
    for i in start..end {
        let tell = dec.tell();
        let qi = if budget - tell >= 15 {
            let pi = 2 * min(i, 20);
            ec_laplace_decode(dec, prob[pi] << 7, prob[pi + 1] << 6) as i64
        } else if budget - tell >= 2 {
            let qi = dec.icdf(&SMALL_ENERGY_ICDF, 2) as i64;
            (qi >> 1) ^ -(qi & 1)
        } else if budget - tell >= 1 {
            -(dec.bit_logp(1) as i64)
        } else {
            -1
        };
        let q = f32(qi as f64);
        let old = fmax(NEG9, old_e[i]);
        let mut tmp = f32(f32(coef * old) + prev[0]);
        tmp = f32(tmp + q);
        old_e[i] = tmp;
        prev[0] = f32(f32(prev[0] + q) - f32(beta * q));
    }
}

fn fine_offset(q2: i64, fine: i64) -> f64 {
    let scale = (1i64 << (14 - fine)) as f64;
    let mut t = f32((q2 as f64) + f32(0.5));
    t = f32(t * f32(scale));
    t = f32(t * f32(1.0 / 16384.0));
    f32(t - f32(0.5))
}

fn final_offset(q2: i64, fine: i64) -> f64 {
    let scale = (1i64 << (14 - fine - 1)) as f64;
    let mut t = f32((q2 as f64) - f32(0.5));
    t = f32(t * f32(scale));
    f32(t * f32(1.0 / 16384.0))
}

fn unquant_fine_energy(dec: &mut EcDec, start: usize, end: usize, old_e: &mut [f64], fine: &[i64]) {
    for i in start..end {
        if fine[i] <= 0 {
            continue;
        }
        let q2 = dec.bits(fine[i] as i32) as i64;
        old_e[i] = f32(old_e[i] + fine_offset(q2, fine[i]));
    }
}

fn unquant_energy_finalise(
    dec: &mut EcDec,
    start: usize,
    end: usize,
    old_e: &mut [f64],
    fine: &[i64],
    prio: &[i64],
    mut bits_left: i64,
) {
    for pr in 0..2 {
        let mut i = start;
        while i < end && bits_left >= 1 {
            if fine[i] < MAX_FINE_BITS && prio[i] == pr {
                let q2 = dec.bits(1) as i64;
                old_e[i] = f32(old_e[i] + final_offset(q2, fine[i]));
                bits_left -= 1;
            }
            i += 1;
        }
    }
}

fn tf_decode(dec: &mut EcDec, start: usize, end: usize, is_transient: i32, lm: usize) -> Vec<i64> {
    let mut budget = dec.storage as i64 * 8;
    let mut tell = dec.tell();
    let mut logp = if is_transient != 0 { 2 } else { 4 };
    let tf_select_rsv = if lm > 0 && tell + logp + 1 <= budget { 1 } else { 0 };
    budget -= tf_select_rsv;
    let mut tf_changed = 0i64;
    let mut curr = 0i64;
    let mut tf_res = vec![0i64; NB_EBANDS];
    for i in start..end {
        if tell + logp <= budget {
            curr ^= dec.bit_logp(logp as u32) as i64;
            tell = dec.tell();
            tf_changed |= curr;
        }
        tf_res[i] = curr;
        logp = if is_transient != 0 { 4 } else { 5 };
    }
    let mut tf_select = 0i64;
    if tf_select_rsv != 0
        && TF_SELECT[lm][(4 * is_transient as i64 + tf_changed) as usize]
            != TF_SELECT[lm][(4 * is_transient as i64 + 2 + tf_changed) as usize]
    {
        tf_select = dec.bit_logp(1) as i64;
    }
    for i in start..end {
        let idx = (4 * is_transient as i64 + 2 * tf_select + tf_res[i]) as usize;
        tf_res[i] = TF_SELECT[lm][idx] as i64;
    }
    tf_res
}

fn init_caps(lm: i64, cap: &mut [i64]) {
    let base = (NB_EBANDS as i64) * (2 * lm + 1 - 1);
    for i in 0..NB_EBANDS {
        let n = (eb(i as i64 + 1) - eb(i as i64)) << lm;
        cap[i] = (opusdata::CACHE_CAPS50[base as usize + i] as i64 + 64) * n >> 2;
    }
}

fn get_pulses(i: i64) -> i64 {
    if i < 8 {
        i
    } else {
        (8 + (i & 7)) << ((i >> 3) - 1)
    }
}

fn cache_row(lm: i32, band: usize) -> &'static [i32] {
    let slot = (lm + 1) as usize * NB_EBANDS + band;
    let index = opusdata::CACHE_INDEX50[slot];
    let start = if index < 0 {
        opusdata::CACHE_BITS50.len() - 1
    } else {
        index as usize
    };
    &opusdata::CACHE_BITS50[start..]
}

fn bits2pulses(band: usize, lm: i32, mut bits: i64) -> i64 {
    let cache = cache_row(lm, band);
    let mut lo = 0i64;
    let mut hi = cache[0] as i64;
    bits -= 1;
    for _ in 0..6 {
        let mid = (lo + hi + 1) >> 1;
        if cache[mid as usize] as i64 >= bits {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let left = bits - if lo == 0 { -1 } else { cache[lo as usize] as i64 };
    let right = cache[hi as usize] as i64 - bits;
    if left <= right {
        lo
    } else {
        hi
    }
}

fn pulses2bits(band: usize, lm: i32, pulses: i64) -> i64 {
    if pulses == 0 {
        return 0;
    }
    cache_row(lm, band)[pulses as usize] as i64 + 1
}

fn interp_bits2pulses(
    start: usize,
    end: usize,
    skip_start: usize,
    bits1: &[i64],
    bits2: &[i64],
    thresh: &[i64],
    cap: &[i64],
    mut total: i64,
    skip_rsv: i64,
    _intensity: i64,
    mut intensity_rsv: i64,
    _dual_stereo: i64,
    mut dual_stereo_rsv: i64,
    bits: &mut [i64],
    ebits: &mut [i64],
    fine_priority: &mut [i64],
    c: i64,
    lm: i64,
    dec: &mut EcDec,
    encode: i64,
) -> (i64, i64, i64, i64) {
    let _ = encode;
    let alloc_floor = c << BITRES;
    let logm = lm << BITRES;
    let mut lo = 0i64;
    let mut hi = 1i64 << ALLOC_STEPS;
    for _ in 0..ALLOC_STEPS {
        let mid = (lo + hi) >> 1;
        let mut psum = 0i64;
        let mut done = false;
        for j in (start..end).rev() {
            let tmp = bits1[j] + ((mid * bits2[j]) >> ALLOC_STEPS);
            if tmp >= thresh[j] || done {
                done = true;
                psum += min(tmp, cap[j]);
            } else if tmp >= alloc_floor {
                psum += alloc_floor;
            }
        }
        if psum > total {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let mut psum = 0i64;
    let mut done = false;
    for j in (start..end).rev() {
        let mut tmp = bits1[j] + ((lo * bits2[j]) >> ALLOC_STEPS);
        if tmp < thresh[j] && !done {
            tmp = if tmp >= alloc_floor { alloc_floor } else { 0 };
        } else {
            done = true;
        }
        tmp = min(tmp, cap[j]);
        bits[j] = tmp;
        psum += tmp;
    }
    let mut coded_bands = end as i64;
    loop {
        let j = coded_bands - 1;
        if j <= skip_start as i64 {
            total += skip_rsv;
            break;
        }
        let ju = j as usize;
        let left = total - psum;
        let width_all = eb(coded_bands) - eb(start as i64);
        let percoeff = floor_div(left, width_all);
        let left = left - width_all * percoeff;
        let rem = max(left - (eb(j) - eb(start as i64)), 0);
        let band_width = eb(coded_bands) - eb(j);
        let mut band_bits = bits[ju] + percoeff * band_width + rem;
        if band_bits >= max(thresh[ju], alloc_floor + (1 << BITRES)) {
            if dec.bit_logp(1) != 0 {
                break;
            }
            psum += 1 << BITRES;
            band_bits -= 1 << BITRES;
        }
        psum -= bits[ju] + intensity_rsv;
        if intensity_rsv > 0 {
            intensity_rsv = LOG2_FRAC[(j - start as i64) as usize];
        }
        psum += intensity_rsv;
        if band_bits >= alloc_floor {
            psum += alloc_floor;
            bits[ju] = alloc_floor;
        } else {
            bits[ju] = 0;
        }
        coded_bands -= 1;
    }
    let intensity = if intensity_rsv > 0 {
        start as i64 + dec.uint(coded_bands + 1 - start as i64) as i64
    } else {
        0
    };
    if intensity <= start as i64 {
        total += dual_stereo_rsv;
        dual_stereo_rsv = 0;
    }
    let dual_stereo = if dual_stereo_rsv > 0 {
        dec.bit_logp(1) as i64
    } else {
        0
    };
    let mut left = total - psum;
    let width_all = eb(coded_bands) - eb(start as i64);
    let percoeff = floor_div(left, width_all);
    left -= width_all * percoeff;
    for j in start..coded_bands as usize {
        bits[j] += percoeff * (eb(j as i64 + 1) - eb(j as i64));
    }
    for j in start..coded_bands as usize {
        let tmp = min(left, eb(j as i64 + 1) - eb(j as i64));
        bits[j] += tmp;
        left -= tmp;
    }
    let mut balance = 0i64;
    let mut j = start;
    while j < coded_bands as usize {
        let n0 = eb(j as i64 + 1) - eb(j as i64);
        let n = n0 << lm;
        let bit = bits[j] + balance;
        let mut excess = if n > 1 {
            let excess = max(bit - cap[j], 0);
            bits[j] = bit - excess;
            let den = c * n
                + if c == 2 && n > 2 && dual_stereo == 0 && (j as i64) < intensity {
                    1
                } else {
                    0
                };
            let nclog_n = den * (logn_at(j as i64) + logm);
            let mut offset = (nclog_n >> 1) - den * FINE_OFFSET;
            if n == 2 {
                offset += den << BITRES >> 2;
            }
            if bits[j] + offset < den * 2 << BITRES {
                offset += nclog_n >> 2;
            } else if bits[j] + offset < den * 3 << BITRES {
                offset += nclog_n >> 3;
            }
            let mut e = max(0, bits[j] + offset + (den << (BITRES - 1)));
            e = floor_div(e, den) >> BITRES;
            if c * e > (bits[j] >> BITRES) {
                e = bits[j] >> (if c > 1 { 1 } else { 0 }) >> BITRES;
            }
            e = min(e, MAX_FINE_BITS);
            fine_priority[j] = if e * (den << BITRES) >= bits[j] + offset { 1 } else { 0 };
            bits[j] -= c * e << BITRES;
            ebits[j] = e;
            excess
        } else {
            let excess = max(0, bit - (c << BITRES));
            bits[j] = bit - excess;
            ebits[j] = 0;
            fine_priority[j] = 1;
            excess
        };
        if excess > 0 {
            let stereo = if c > 1 { 1 } else { 0 };
            let extra_fine = min(excess >> (stereo + BITRES), MAX_FINE_BITS - ebits[j]);
            ebits[j] += extra_fine;
            let extra_bits = extra_fine * c << BITRES;
            fine_priority[j] = if extra_bits >= excess - balance { 1 } else { 0 };
            excess -= extra_bits;
        }
        balance = excess;
        j += 1;
    }
    while j < end {
        ebits[j] = bits[j] >> (if c > 1 { 1 } else { 0 }) >> BITRES;
        bits[j] = 0;
        fine_priority[j] = if ebits[j] < 1 { 1 } else { 0 };
        j += 1;
    }
    (coded_bands, balance, intensity, dual_stereo)
}

struct Alloc {
    coded: i64,
    balance: i64,
    intensity: i64,
    dual: i64,
    pulses: Vec<i64>,
    ebits: Vec<i64>,
    fine_priority: Vec<i64>,
}

fn clt_compute_allocation(
    start: usize,
    end: usize,
    offsets: &[i64],
    cap: &[i64],
    alloc_trim: i64,
    mut total: i64,
    c: i64,
    lm: i64,
    dec: &mut EcDec,
) -> Alloc {
    total = max(total, 0);
    let mut skip_start = start;
    let skip_rsv = if total >= 1 << BITRES { 1 << BITRES } else { 0 };
    total -= skip_rsv;
    let mut intensity_rsv = 0i64;
    let mut dual_stereo_rsv = 0i64;
    if c == 2 {
        intensity_rsv = LOG2_FRAC[end - start];
        if intensity_rsv > total {
            intensity_rsv = 0;
        } else {
            total -= intensity_rsv;
            dual_stereo_rsv = if total >= 1 << BITRES { 1 << BITRES } else { 0 };
            total -= dual_stereo_rsv;
        }
    }
    let mut bits1 = vec![0i64; NB_EBANDS];
    let mut bits2 = vec![0i64; NB_EBANDS];
    let mut thresh = vec![0i64; NB_EBANDS];
    let mut trim_offset = vec![0i64; NB_EBANDS];
    for j in start..end {
        thresh[j] = max(
            c << BITRES,
            (3 * (eb(j as i64 + 1) - eb(j as i64)) << lm << BITRES) >> 4,
        );
        trim_offset[j] = c
            * (eb(j as i64 + 1) - eb(j as i64))
            * (alloc_trim - 5 - lm)
            * (end as i64 - j as i64 - 1)
            * (1 << (lm + BITRES))
            >> 6;
        if (eb(j as i64 + 1) - eb(j as i64)) << lm == 1 {
            trim_offset[j] -= c << BITRES;
        }
    }
    let mut lo = 1i64;
    let mut hi = 10i64;
    loop {
        let mut done = false;
        let mut psum = 0i64;
        let mid = (lo + hi) >> 1;
        for j in (start..end).rev() {
            let n = eb(j as i64 + 1) - eb(j as i64);
            let mut bitsj = ((c * n * alloc_at(mid * NB_EBANDS as i64 + j as i64)) << lm) >> 2;
            if bitsj > 0 {
                bitsj = max(0, bitsj + trim_offset[j]);
            }
            bitsj += offsets[j];
            if bitsj >= thresh[j] || done {
                done = true;
                psum += min(bitsj, cap[j]);
            } else if bitsj >= c << BITRES {
                psum += c << BITRES;
            }
        }
        if psum > total {
            hi = mid - 1;
        } else {
            lo = mid + 1;
        }
        if lo > hi {
            break;
        }
    }
    hi = lo;
    lo -= 1;
    for j in start..end {
        let n = eb(j as i64 + 1) - eb(j as i64);
        let mut bits1j = ((c * n * alloc_at(lo * NB_EBANDS as i64 + j as i64)) << lm) >> 2;
        let mut bits2j = if hi >= 11 {
            cap[j]
        } else {
            ((c * n * alloc_at(hi * NB_EBANDS as i64 + j as i64)) << lm) >> 2
        };
        if bits1j > 0 {
            bits1j = max(0, bits1j + trim_offset[j]);
        }
        if bits2j > 0 {
            bits2j = max(0, bits2j + trim_offset[j]);
        }
        if lo > 0 {
            bits1j += offsets[j];
        }
        bits2j += offsets[j];
        if offsets[j] > 0 {
            skip_start = j;
        }
        bits2j = max(0, bits2j - bits1j);
        bits1[j] = bits1j;
        bits2[j] = bits2j;
    }
    let mut pulses = vec![0i64; NB_EBANDS];
    let mut ebits = vec![0i64; NB_EBANDS];
    let mut fine_priority = vec![0i64; NB_EBANDS];
    let (coded, balance, intensity, dual) = interp_bits2pulses(
        start,
        end,
        skip_start,
        &bits1,
        &bits2,
        &thresh,
        cap,
        total,
        skip_rsv,
        0,
        intensity_rsv,
        0,
        dual_stereo_rsv,
        &mut pulses,
        &mut ebits,
        &mut fine_priority,
        c,
        lm,
        dec,
        0,
    );
    Alloc {
        coded,
        balance,
        intensity,
        dual,
        pulses,
        ebits,
        fine_priority,
    }
}

fn pvq_u(mut n: i64, mut k: i64) -> u32 {
    if n > k {
        std::mem::swap(&mut n, &mut k);
    }
    opusdata::PVQ_U_DATA[opusdata::PVQ_U_ROW[n as usize] as usize + k as usize]
}

fn yy_add(yy: f64, val: i64) -> f64 {
    f32(yy + f32((val as f64) * (val as f64)))
}

fn cwrsi(mut n: i64, mut k: i64, idx: u32, y: &mut [i32]) -> f64 {
    let mut yy = f32(0.0);
    let mut pos = 0usize;
    let mut i = idx;
    while n > 2 {
        if k >= n {
            let row = opusdata::PVQ_U_ROW[n as usize] as usize;
            let p_sign = opusdata::PVQ_U_DATA[row + k as usize + 1];
            let s: i64 = if i >= p_sign { -1 } else { 0 };
            if s != 0 {
                i = i.wrapping_sub(p_sign);
            }
            let k0 = k;
            let q = opusdata::PVQ_U_DATA[row + n as usize];
            let p = if q > i {
                k = n;
                let mut p;
                loop {
                    k -= 1;
                    p = opusdata::PVQ_U_DATA[opusdata::PVQ_U_ROW[k as usize] as usize + n as usize];
                    if p <= i {
                        break;
                    }
                }
                p
            } else {
                let mut p = opusdata::PVQ_U_DATA[row + k as usize];
                while p > i {
                    k -= 1;
                    p = opusdata::PVQ_U_DATA[row + k as usize];
                }
                p
            };
            i = i.wrapping_sub(p);
            let val = (k0 - k + s) ^ s;
            y[pos] = val as i32;
            pos += 1;
            yy = yy_add(yy, val);
        } else {
            let mut p = opusdata::PVQ_U_DATA[opusdata::PVQ_U_ROW[k as usize] as usize + n as usize];
            let q = opusdata::PVQ_U_DATA[opusdata::PVQ_U_ROW[(k + 1) as usize] as usize + n as usize];
            if p <= i && i < q {
                i = i.wrapping_sub(p);
                y[pos] = 0;
                pos += 1;
            } else {
                let s: i64 = if i >= q { -1 } else { 0 };
                if s != 0 {
                    i = i.wrapping_sub(q);
                }
                let k0 = k;
                loop {
                    k -= 1;
                    p = opusdata::PVQ_U_DATA[opusdata::PVQ_U_ROW[k as usize] as usize + n as usize];
                    if p <= i {
                        break;
                    }
                }
                i = i.wrapping_sub(p);
                let val = (k0 - k + s) ^ s;
                y[pos] = val as i32;
                pos += 1;
                yy = yy_add(yy, val);
            }
        }
        n -= 1;
    }
    let p = (2 * k + 1) as u32;
    let mut s: i64 = if i >= p { -1 } else { 0 };
    if s != 0 {
        i = i.wrapping_sub(p);
    }
    let k0 = k;
    k = (i.wrapping_add(1) >> 1) as i64;
    if k != 0 {
        i = i.wrapping_sub((2 * k as u32) - 1);
    }
    let val = (k0 - k + s) ^ s;
    y[pos] = val as i32;
    pos += 1;
    yy = yy_add(yy, val);
    let si = i as i32 as i64;
    s = -si;
    let val = (k + s) ^ s;
    y[pos] = val as i32;
    yy_add(yy, val)
}

fn decode_pulses(y: &mut [i32], n: i64, k: i64, dec: &mut EcDec) -> f64 {
    let v = pvq_u(n, k).wrapping_add(pvq_u(n, k + 1));
    cwrsi(n, k, dec.uint(v as i64), y)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    X,
    Norm,
}

struct Store {
    x: Vec<f64>,
    norm: Vec<f64>,
}

impl Store {
    fn get(&self, side: Side, i: usize) -> f64 {
        match side {
            Side::X => self.x[i],
            Side::Norm => self.norm[i],
        }
    }

    fn set(&mut self, side: Side, i: usize, v: f64) {
        match side {
            Side::X => self.x[i] = v,
            Side::Norm => self.norm[i] = v,
        }
    }

    fn slice_mut(&mut self, side: Side) -> &mut [f64] {
        match side {
            Side::X => &mut self.x,
            Side::Norm => &mut self.norm,
        }
    }
}

fn exp_rotation1(st: &mut Store, side: Side, off: usize, length: i64, stride: i64, c: f64, s: f64) {
    let ms = f32(-s);
    let mut xptr = off;
    let mut i = 0i64;
    let limit = length - stride;
    while i < limit {
        let x1 = st.get(side, xptr);
        let x2 = st.get(side, xptr + stride as usize);
        st.set(side, xptr + stride as usize, f32(f32(c * x2) + f32(s * x1)));
        st.set(side, xptr, f32(f32(c * x1) + f32(ms * x2)));
        xptr += 1;
        i += 1;
    }
    let mut i = length - 2 * stride - 1;
    if i >= 0 {
        let mut xptr = off + i as usize;
        loop {
            let x1 = st.get(side, xptr);
            let x2 = st.get(side, xptr + stride as usize);
            st.set(side, xptr + stride as usize, f32(f32(c * x2) + f32(s * x1)));
            st.set(side, xptr, f32(f32(c * x1) + f32(ms * x2)));
            i -= 1;
            if i < 0 {
                break;
            }
            xptr -= 1;
        }
    }
}

fn exp_rotation(
    st: &mut Store,
    side: Side,
    off: usize,
    mut length: i64,
    direction: i64,
    stride: i64,
    k: i64,
    spread: i32,
) {
    if 2 * k >= length || spread == SPREAD_NONE {
        return;
    }
    let factor = SPREAD_FACTOR[(spread - 1) as usize];
    let gain = f32(f32(length as f64) / f32((length + factor * k) as f64));
    let theta = f32(f32(0.5) * f32(gain * gain));
    let c = celt_cos_norm(theta);
    let s = celt_cos_norm(f32(f32(1.0) - theta));
    let mut stride2 = 0i64;
    if length >= 8 * stride {
        stride2 = 1;
        while (stride2 * stride2 + stride2) * stride + (stride >> 2) < length {
            stride2 += 1;
        }
    }
    length /= stride;
    for i in 0..stride {
        let base = off + (i * length) as usize;
        if direction < 0 {
            if stride2 != 0 {
                exp_rotation1(st, side, base, length, stride2, s, c);
            }
            exp_rotation1(st, side, base, length, 1, c, s);
        } else {
            exp_rotation1(st, side, base, length, 1, c, f32(-s));
            if stride2 != 0 {
                exp_rotation1(st, side, base, length, stride2, s, f32(-c));
            }
        }
    }
}

fn normalise_residual(iy: &[i32], st: &mut Store, side: Side, off: usize, n: i64, ryy: f64, gain: f64) {
    let g = f32(celt_rsqrt(ryy) * gain);
    for i in 0..n as usize {
        st.set(side, off + i, f32(g * f32(iy[i] as f64)));
    }
}

fn extract_collapse_mask(iy: &[i32], n: i64, b: i64) -> i64 {
    if b <= 1 {
        return 1;
    }
    let n0 = n / b;
    let mut mask = 0i64;
    for i in 0..b {
        let mut tmp = 0i64;
        let base = (i * n0) as usize;
        for j in 0..n0 {
            tmp |= iy[base + j as usize] as i64;
        }
        if tmp != 0 {
            mask |= 1i64 << i;
        }
    }
    mask
}

fn alg_unquant(
    st: &mut Store,
    side: Side,
    off: usize,
    n: i64,
    k: i64,
    spread: i32,
    blocks: i64,
    dec: &mut EcDec,
    gain: f64,
) -> i64 {
    let mut iy = vec![0i32; n as usize];
    let ryy = decode_pulses(&mut iy, n, k, dec);
    normalise_residual(&iy, st, side, off, n, ryy, gain);
    exp_rotation(st, side, off, n, -1, blocks, k, spread);
    extract_collapse_mask(&iy, n, blocks)
}

fn renormalise_vector(buf: &mut [f64], off: usize, n: i64, gain: f64) {
    let mut acc = f32(0.0);
    for i in 0..n as usize {
        let v = buf[off + i];
        acc = f32(acc + f32(v * v));
    }
    let energy = f32(EPSILON + acc);
    let g = f32(celt_rsqrt(energy) * gain);
    for i in 0..n as usize {
        buf[off + i] = f32(g * buf[off + i]);
    }
}

fn lcg(seed: u32) -> u32 {
    seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)
}

fn haar1(st: &mut Store, side: Side, off: usize, mut n0: i64, stride: i64) {
    n0 >>= 1;
    for i in 0..stride {
        for j in 0..n0 {
            let i0 = off + stride as usize * 2 * j as usize + i as usize;
            let i1 = off + stride as usize * (2 * j as usize + 1) + i as usize;
            let t1 = f32(HAAR_C * st.get(side, i0));
            let t2 = f32(HAAR_C * st.get(side, i1));
            st.set(side, i0, f32(t1 + t2));
            st.set(side, i1, f32(t1 - t2));
        }
    }
}

fn deinterleave_hadamard(st: &mut Store, side: Side, off: usize, n0: i64, stride: i64, hadamard: i64) {
    let n = (n0 * stride) as usize;
    let mut tmp = vec![f32(0.0); n];
    if hadamard != 0 {
        let base = (stride as usize) - 2;
        for i in 0..stride as usize {
            for j in 0..n0 as usize {
                let ord = ORDERY[base + i] as usize;
                tmp[ord * n0 as usize + j] = st.get(side, off + j * stride as usize + i);
            }
        }
    } else {
        for i in 0..stride as usize {
            for j in 0..n0 as usize {
                tmp[i * n0 as usize + j] = st.get(side, off + j * stride as usize + i);
            }
        }
    }
    for i in 0..n {
        st.set(side, off + i, tmp[i]);
    }
}

fn interleave_hadamard(st: &mut Store, side: Side, off: usize, n0: i64, stride: i64, hadamard: i64) {
    let n = (n0 * stride) as usize;
    let mut tmp = vec![f32(0.0); n];
    if hadamard != 0 {
        let base = (stride as usize) - 2;
        for i in 0..stride as usize {
            for j in 0..n0 as usize {
                let ord = ORDERY[base + i] as usize;
                tmp[j * stride as usize + i] = st.get(side, off + ord * n0 as usize + j);
            }
        }
    } else {
        for i in 0..stride as usize {
            for j in 0..n0 as usize {
                tmp[j * stride as usize + i] = st.get(side, off + i * n0 as usize + j);
            }
        }
    }
    for i in 0..n {
        st.set(side, off + i, tmp[i]);
    }
}

fn compute_qn(n: i64, b: i64, offset: i64, pulse_cap: i64, stereo: i64) -> i64 {
    let mut n2 = 2 * n - 1;
    if stereo != 0 && n == 2 {
        n2 -= 1;
    }
    let mut qb = cdiv(b + n2 * offset, n2);
    qb = min(b - pulse_cap - (4 << BITRES), qb);
    qb = min(8 << BITRES, qb);
    if qb < (1 << BITRES >> 1) {
        return 1;
    }
    let qn = asr(EXP2_TABLE8[(qb & 7) as usize], 14 - (qb >> BITRES));
    ((qn + 1) >> 1) << 1
}

struct Theta {
    b: i64,
    fill: i64,
    imid: i64,
    iside: i64,
    delta: i64,
    itheta: i64,
    qalloc: i64,
}

struct Ctx<'a> {
    ec: &'a mut EcDec,
    encode: i32,
    resynth: i32,
    intensity: i64,
    spread: i32,
    seed: u32,
    disable_inv: i32,
    avoid_split_noise: i32,
    band: usize,
    tf_change: i64,
    remaining_bits: i64,
}

fn compute_theta(ctx: &mut Ctx, n: i64, mut b: i64, bb: i64, b0: i64, lm: i32, stereo: i64, mut fill: i64) -> Theta {
    let m_logn = logn_at(ctx.band as i64);
    let pulse_cap = m_logn + lm as i64 * (1 << BITRES);
    let offset = (pulse_cap >> 1) - if stereo != 0 && n == 2 { QTHETA_OFFSET_TWOPHASE } else { QTHETA_OFFSET };
    let mut qn = compute_qn(n, b, offset, pulse_cap, stereo);
    if stereo != 0 && ctx.band as i64 >= ctx.intensity {
        qn = 1;
    }
    let tell = ctx.ec.tell_frac();
    let mut itheta = 0i64;
    let mut inv = 0i64;
    if qn != 1 {
        if stereo != 0 && n > 2 {
            let p0 = 3i64;
            let x0 = qn / 2;
            let ft = p0 * (x0 + 1) + x0;
            let fs = ctx.ec.decode(ft as u32) as i64;
            if fs < (x0 + 1) * p0 {
                itheta = floor_div(fs, p0);
            } else {
                itheta = x0 + 1 + (fs - (x0 + 1) * p0);
            }
            let fl = if itheta <= x0 { p0 * itheta } else { (itheta - 1 - x0) + (x0 + 1) * p0 };
            let fh = if itheta <= x0 { p0 * (itheta + 1) } else { (itheta - x0) + (x0 + 1) * p0 };
            ctx.ec.update(fl as u32, fh as u32, ft as u32);
        } else if b0 > 1 || stereo != 0 {
            itheta = ctx.ec.uint(qn + 1) as i64;
        } else {
            let ft = ((qn >> 1) + 1) * ((qn >> 1) + 1);
            let fm = ctx.ec.decode(ft as u32) as i64;
            let half = ((qn >> 1) * ((qn >> 1) + 1)) >> 1;
            let (fs, fl) = if fm < half {
                itheta = (isqrt32((8 * fm + 1) as u32) as i64 - 1) >> 1;
                let fs = itheta + 1;
                let fl = (itheta * (itheta + 1)) >> 1;
                (fs, fl)
            } else {
                itheta = (2 * (qn + 1) - isqrt32((8 * (ft - fm - 1) + 1) as u32) as i64) >> 1;
                let fs = qn + 1 - itheta;
                let fl = ft - (((qn + 1 - itheta) * (qn + 2 - itheta)) >> 1);
                (fs, fl)
            };
            ctx.ec.update(fl as u32, (fl + fs) as u32, ft as u32);
        }
        itheta = floor_div(itheta * 16384, qn);
    } else if stereo != 0 {
        if b > 2 << BITRES && ctx.remaining_bits > 2 << BITRES {
            inv = ctx.ec.bit_logp(2) as i64;
        } else {
            inv = 0;
        }
        if ctx.disable_inv != 0 {
            inv = 0;
        }
        itheta = 0;
    }
    let _ = inv;
    let qalloc = ctx.ec.tell_frac() - tell;
    b -= qalloc;
    let (imid, iside, delta) = if itheta == 0 {
        fill &= (1 << bb) - 1;
        (32767, 0, -16384)
    } else if itheta == 16384 {
        fill &= ((1 << bb) - 1) << bb;
        (0, 32767, 16384)
    } else {
        let imid = bitexact_cos(itheta);
        let iside = bitexact_cos(16384 - itheta);
        let delta = frac_mul16((n - 1) << 7, bitexact_log2tan(iside, imid));
        (imid, iside, delta)
    };
    Theta { b, fill, imid, iside, delta, itheta, qalloc }
}

fn quant_partition(
    ctx: &mut Ctx,
    st: &mut Store,
    buf: Side,
    off: usize,
    mut n: i64,
    mut b: i64,
    mut blocks: i64,
    low: Option<Side>,
    low_off: usize,
    mut lm: i32,
    gain: f64,
    mut fill: i64,
) -> i64 {
    let cache = cache_row(lm, ctx.band);
    if lm != -1 && b > cache[cache[0] as usize] as i64 + 12 && n > 2 {
        n >>= 1;
        let yoff = off + n as usize;
        lm -= 1;
        let b0 = blocks;
        if blocks == 1 {
            fill = (fill & 1) | (fill << 1);
        }
        blocks = (blocks + 1) >> 1;
        let th = compute_theta(ctx, n, b, blocks, b0, lm, 0, fill);
        b = th.b;
        fill = th.fill;
        let mut delta = th.delta;
        let itheta = th.itheta;
        let qalloc = th.qalloc;
        let mid = f32(f32(1.0 / 32768.0) * f32(th.imid as f64));
        let side_g = f32(f32(1.0 / 32768.0) * f32(th.iside as f64));
        if b0 > 1 && (itheta & 0x3FFF) != 0 {
            if itheta > 8192 {
                delta -= asr(delta, 4 - lm as i64);
            } else {
                delta = min(0, delta + asr(n << BITRES, 5 - lm as i64));
            }
        }
        let mut mbits = max(0, min(b, cdiv(b - delta, 2)));
        let mut sbits = b - mbits;
        ctx.remaining_bits -= qalloc;
        let next_low = low;
        let next_low_off = if low.is_some() { low_off + n as usize } else { 0 };
        let rebalance0 = ctx.remaining_bits;
        if mbits >= sbits {
            let mut cm = quant_partition(
                ctx, st, buf, off, n, mbits, blocks, low, low_off, lm, f32(gain * mid), fill,
            );
            let rebalance = mbits - (rebalance0 - ctx.remaining_bits);
            if rebalance > 3 << BITRES && itheta != 0 {
                sbits += rebalance - (3 << BITRES);
            }
            cm |= quant_partition(
                ctx,
                st,
                buf,
                yoff,
                n,
                sbits,
                blocks,
                next_low,
                next_low_off,
                lm,
                f32(gain * side_g),
                asr(fill, blocks),
            ) << (b0 >> 1);
            cm
        } else {
            let mut cm = quant_partition(
                ctx,
                st,
                buf,
                yoff,
                n,
                sbits,
                blocks,
                next_low,
                next_low_off,
                lm,
                f32(gain * side_g),
                asr(fill, blocks),
            ) << (b0 >> 1);
            let rebalance = sbits - (rebalance0 - ctx.remaining_bits);
            if rebalance > 3 << BITRES && itheta != 16384 {
                mbits += rebalance - (3 << BITRES);
            }
            cm |= quant_partition(
                ctx, st, buf, off, n, mbits, blocks, low, low_off, lm, f32(gain * mid), fill,
            );
            cm
        }
    } else {
        let mut q = bits2pulses(ctx.band, lm, b);
        let mut curr_bits = pulses2bits(ctx.band, lm, q);
        ctx.remaining_bits -= curr_bits;
        while ctx.remaining_bits < 0 && q > 0 {
            ctx.remaining_bits += curr_bits;
            q -= 1;
            curr_bits = pulses2bits(ctx.band, lm, q);
            ctx.remaining_bits -= curr_bits;
        }
        if q != 0 {
            return alg_unquant(st, buf, off, n, get_pulses(q), ctx.spread, blocks, ctx.ec, gain);
        }
        if ctx.resynth != 0 {
            let cm_mask = (1i64 << blocks) - 1;
            fill &= cm_mask;
            if fill == 0 {
                for j in 0..n as usize {
                    st.set(buf, off + j, f32(0.0));
                }
                return 0;
            }
            let cm = if let Some(low_side) = low {
                for j in 0..n as usize {
                    ctx.seed = lcg(ctx.seed);
                    let mut tmp = f32(1.0 / 256.0);
                    if ctx.seed & 0x8000 != 0 {
                        tmp = f32(-tmp);
                    }
                    let base = st.get(low_side, low_off + j);
                    st.set(buf, off + j, f32(base + tmp));
                }
                fill
            } else {
                for j in 0..n as usize {
                    ctx.seed = lcg(ctx.seed);
                    let shifted = (ctx.seed as i32) >> 20;
                    st.set(buf, off + j, f32(shifted as f64));
                }
                cm_mask
            };
            let slice = st.slice_mut(buf);
            renormalise_vector(slice, off, n, gain);
            return cm;
        }
        0
    }
}

fn quant_band_n1(
    ctx: &mut Ctx,
    st: &mut Store,
    buf: Side,
    off: usize,
    low_out: Option<Side>,
    low_out_off: usize,
) -> i64 {
    let mut sign = 0i64;
    if ctx.remaining_bits >= 1 << BITRES {
        sign = ctx.ec.bits(1) as i64;
        ctx.remaining_bits -= 1 << BITRES;
    }
    if ctx.resynth != 0 {
        st.set(buf, off, if sign != 0 { f32(-1.0) } else { f32(1.0) });
    }
    if let Some(out) = low_out {
        let v = st.get(buf, off);
        st.set(out, low_out_off, v);
    }
    1
}

fn quant_band(
    ctx: &mut Ctx,
    st: &mut Store,
    buf: Side,
    off: usize,
    n: i64,
    b: i64,
    mut blocks: i64,
    mut low: Option<Side>,
    mut low_off: usize,
    lm: i32,
    low_out: Option<Side>,
    low_out_off: usize,
    gain: f64,
    scratch: Option<Side>,
    scratch_off: usize,
    mut fill: i64,
) -> i64 {
    let n0 = n;
    if n == 1 {
        return quant_band_n1(ctx, st, buf, off, low_out, low_out_off);
    }
    let mut tf_change = ctx.tf_change;
    let b0_orig = blocks;
    let long_blocks = if b0_orig == 1 { 1 } else { 0 };
    let mut n_b = n / blocks;
    let recombine = if tf_change > 0 { tf_change } else { 0 };
    if scratch.is_some() && low.is_some() && (recombine != 0 || ((n_b & 1) == 0 && tf_change < 0) || b0_orig > 1)
    {
        let sc = scratch.unwrap();
        let src = low.unwrap();
        for j in 0..n as usize {
            let v = st.get(src, low_off + j);
            st.set(sc, scratch_off + j, v);
        }
        low = scratch;
        low_off = scratch_off;
    }
    for k in 0..recombine {
        if let Some(side) = low {
            haar1(st, side, low_off, n >> k, 1i64 << k);
        }
        let lo = (fill & 0xF) as usize;
        let hi = (fill >> 4) as usize;
        fill = BIT_INTERLEAVE[lo] as i64 | ((BIT_INTERLEAVE[hi] as i64) << 2);
    }
    blocks >>= recombine;
    n_b <<= recombine;
    let mut time_divide = 0i64;
    while (n_b & 1) == 0 && tf_change < 0 {
        if let Some(side) = low {
            haar1(st, side, low_off, n_b, blocks);
        }
        fill |= fill << blocks;
        blocks <<= 1;
        n_b >>= 1;
        time_divide += 1;
        tf_change += 1;
    }
    let b0 = blocks;
    let n_b0 = n_b;
    if b0 > 1 {
        if let Some(side) = low {
            deinterleave_hadamard(st, side, low_off, asr(n_b, recombine), shl(b0, recombine), long_blocks);
        }
    }
    let mut cm = quant_partition(ctx, st, buf, off, n, b, blocks, low, low_off, lm, gain, fill);
    if ctx.resynth != 0 {
        if b0 > 1 {
            interleave_hadamard(st, buf, off, asr(n_b, recombine), shl(b0, recombine), long_blocks);
        }
        let mut n_b = n_b0;
        let mut blocks = b0;
        for _k in 0..time_divide {
            blocks >>= 1;
            n_b <<= 1;
            cm |= asr(cm, blocks);
            haar1(st, buf, off, n_b, blocks);
        }
        for k in 0..recombine {
            cm = BIT_DEINTERLEAVE[(cm & 0xF) as usize] as i64;
            haar1(st, buf, off, n0 >> k, 1i64 << k);
        }
        blocks = shl(blocks, recombine);
        if let Some(out) = low_out {
            let normv = celt_sqrt(f32(n0 as f64));
            for j in 0..n0 as usize {
                let v = f32(normv * st.get(buf, off + j));
                st.set(out, low_out_off + j, v);
            }
        }
        cm &= (1i64 << blocks) - 1;
    }
    cm
}

fn quant_all_bands(
    dec: &mut EcDec,
    start: usize,
    end: usize,
    st: &mut Store,
    pulses: &[i64],
    short_blocks: i64,
    spread: i32,
    intensity: i64,
    tf_res: &[i64],
    total_bits: i64,
    mut balance: i64,
    lm: i32,
    coded_bands: i64,
    seed: u32,
) -> (Vec<i64>, u32, i64) {
    let m = 1i64 << lm as i64;
    let blocks0 = if short_blocks != 0 { m } else { 1 };
    let norm_offset = m * eb(start as i64);
    let norm_len = m * eb(NB_EBANDS as i64 - 1) - norm_offset;
    st.norm = vec![f32(0.0); norm_len.max(1) as usize];
    let scratch_off = (m * eb(NB_EBANDS as i64 - 1)) as usize;
    let mut lowband_offset = 0i64;
    let mut update_lowband = 1i64;
    let mut ctx = Ctx {
        ec: dec,
        encode: 0,
        resynth: 1,
        intensity,
        spread,
        seed,
        disable_inv: 1,
        avoid_split_noise: if blocks0 > 1 { 1 } else { 0 },
        band: 0,
        tf_change: 0,
        remaining_bits: 0,
    };
    let mut collapse = vec![0i64; NB_EBANDS];
    for i in start..end {
        ctx.band = i;
        let last = i + 1 == end;
        let xoff = (m * eb(i as i64)) as usize;
        let n = m * eb(i as i64 + 1) - m * eb(i as i64);
        let tell = ctx.ec.tell_frac();
        if i != start {
            balance -= tell;
        }
        let remaining = total_bits - tell - 1;
        ctx.remaining_bits = remaining;
        let b = if (i as i64) <= coded_bands - 1 {
            let curr_balance = cdiv(balance, min(3, coded_bands - i as i64));
            max(0, min(16383, min(remaining + 1, pulses[i] + curr_balance)))
        } else {
            0
        };
        if (m * eb(i as i64) - n >= m * eb(start as i64) || i == start + 1)
            && (update_lowband != 0 || lowband_offset == 0)
        {
            lowband_offset = i as i64;
        }
        if i == start + 1 {
            let n1 = m * (eb(start as i64 + 1) - eb(start as i64));
            let n2 = m * (eb(start as i64 + 2) - eb(start as i64 + 1));
            if n2 > n1 {
                for t in 0..(n2 - n1) {
                    let v = st.get(Side::Norm, (2 * n1 - n2 + t) as usize);
                    st.set(Side::Norm, (n1 + t) as usize, v);
                }
            }
        }
        let tf_change = tf_res[i];
        ctx.tf_change = tf_change;
        let (low_scratch, low_scratch_off) = if last { (None, 0) } else { (Some(Side::X), scratch_off) };
        let (x_cm, low, low_off) = if lowband_offset != 0
            && (spread != SPREAD_AGGRESSIVE || blocks0 > 1 || tf_change < 0)
        {
            let effective = max(0, m * eb(lowband_offset) - norm_offset - n);
            let mut fold_start = lowband_offset;
            loop {
                fold_start -= 1;
                if !(m * eb(fold_start) > effective + norm_offset) {
                    break;
                }
            }
            let mut fold_end = lowband_offset - 1;
            loop {
                fold_end += 1;
                if !(fold_end < i as i64 && m * eb(fold_end) < effective + norm_offset + n) {
                    break;
                }
            }
            let mut x_cm = 0i64;
            let mut fold_i = fold_start;
            loop {
                x_cm |= collapse[fold_i as usize];
                fold_i += 1;
                if !(fold_i < fold_end) {
                    break;
                }
            }
            (x_cm, Some(Side::Norm), effective as usize)
        } else {
            ((1i64 << blocks0) - 1, None, 0usize)
        };
        let (out, out_off) = if last {
            (None, 0usize)
        } else {
            (Some(Side::Norm), (m * eb(i as i64) - norm_offset) as usize)
        };
        let cm = quant_band(
            &mut ctx,
            st,
            Side::X,
            xoff,
            n,
            b,
            blocks0,
            low,
            low_off,
            lm,
            out,
            out_off,
            f32(1.0),
            low_scratch,
            low_scratch_off,
            x_cm,
        );
        collapse[i] = cm & 0xFF;
        balance += pulses[i] + tell;
        update_lowband = if b > (n << BITRES) { 1 } else { 0 };
        ctx.avoid_split_noise = 0;
    }
    let _ = ctx.encode;
    (collapse, ctx.seed, balance)
}

fn anti_collapse(
    x: &mut [f64],
    collapse: &[i64],
    lm: i64,
    start: usize,
    end: usize,
    log_e: &[f64],
    prev1: &[f64],
    prev2: &[f64],
    pulses: &[i64],
    mut seed: u32,
) -> u32 {
    let m = 1i64 << lm;
    for i in start..end {
        let n0 = eb(i as i64 + 1) - eb(i as i64);
        let depth = floor_div(1 + pulses[i], n0) >> lm;
        let thresh = f32(f32(0.5) * celt_exp2(f32(f32(-0.125) * f32(depth as f64))));
        let sqrt_1 = celt_rsqrt(f32((n0 << lm) as f64));
        let p1 = fmax(prev1[i], prev1[NB_EBANDS + i]);
        let p2 = fmax(prev2[i], prev2[NB_EBANDS + i]);
        let mut ediff = f32(log_e[i] - fmin(p1, p2));
        if ediff < 0.0 {
            ediff = f32(0.0);
        }
        let mut r = f32(f32(2.0) * celt_exp2(f32(-ediff)));
        if lm == 3 {
            r = f32(r * SQRT2);
        }
        let r = if thresh < r { thresh } else { r };
        let r = f32(r * sqrt_1);
        let base = (eb(i as i64) << lm) as usize;
        let mut renormalize = false;
        let mask = collapse[i];
        for k in 0..m {
            if (mask & (1i64 << k)) == 0 {
                for j in 0..n0 {
                    seed = lcg(seed);
                    let val = if seed & 0x8000 != 0 { r } else { f32(-r) };
                    x[base + ((j << lm) as usize) + k as usize] = val;
                }
                renormalize = true;
            }
        }
        if renormalize {
            renormalise_vector(x, base, n0 << lm, f32(1.0));
        }
    }
    seed
}

type Cpx = (f64, f64);

fn cmul(a: Cpx, b: Cpx) -> Cpx {
    let (ar, ai) = a;
    let (br, bi) = b;
    (f32(f32(ar * br) - f32(ai * bi)), f32(f32(ar * bi) + f32(ai * br)))
}

fn cadd(a: Cpx, b: Cpx) -> Cpx {
    (f32(a.0 + b.0), f32(a.1 + b.1))
}

fn csub(a: Cpx, b: Cpx) -> Cpx {
    (f32(a.0 - b.0), f32(a.1 - b.1))
}

fn kf_bfly2(fout: &mut [Cpx], _m: i64, n: i64, base: usize) {
    let tw = FFT_C;
    let mut f = base;
    for _i in 0..n {
        let t0 = fout[f + 4];
        let a = fout[f];
        fout[f + 4] = csub(a, t0);
        fout[f] = cadd(a, t0);
        let s = fout[f + 5];
        let tr = f32(f32(s.0 + s.1) * tw);
        let ti = f32(f32(s.1 - s.0) * tw);
        let t = (tr, ti);
        let a = fout[f + 1];
        fout[f + 5] = csub(a, t);
        fout[f + 1] = cadd(a, t);
        let s = fout[f + 6];
        let t = (s.1, f32(-s.0));
        let a = fout[f + 2];
        fout[f + 6] = csub(a, t);
        fout[f + 2] = cadd(a, t);
        let s = fout[f + 7];
        let tr = f32(f32(s.1 - s.0) * tw);
        let ti = f32(f32(-f32(s.1 + s.0)) * tw);
        let t = (tr, ti);
        let a = fout[f + 3];
        fout[f + 7] = csub(a, t);
        fout[f + 3] = cadd(a, t);
        f += 8;
    }
}

fn kf_bfly4(fout: &mut [Cpx], fstride: i64, m: i64, n: i64, mm: i64) {
    if m == 1 {
        let mut f = 0usize;
        for _i in 0..n {
            let a0 = fout[f];
            let a1 = fout[f + 1];
            let a2 = fout[f + 2];
            let a3 = fout[f + 3];
            let s0 = csub(a0, a2);
            let mut n0 = cadd(a0, a2);
            let mut s1 = cadd(a1, a3);
            let f2 = csub(n0, s1);
            n0 = cadd(n0, s1);
            s1 = csub(a1, a3);
            fout[f] = n0;
            fout[f + 2] = f2;
            fout[f + 1] = (f32(s0.0 + s1.1), f32(s0.1 - s1.0));
            fout[f + 3] = (f32(s0.0 - s1.1), f32(s0.1 + s1.0));
            f += 4;
        }
        return;
    }
    for i in 0..n {
        let mut f = (i * mm) as usize;
        let mut t1 = 0i64;
        let mut t2 = 0i64;
        let mut t3 = 0i64;
        for _j in 0..m {
            let a0 = fout[f];
            let s0 = cmul(fout[f + m as usize], twiddle(t1 as usize));
            let s1 = cmul(fout[f + (2 * m) as usize], twiddle(t2 as usize));
            let s2 = cmul(fout[f + (3 * m) as usize], twiddle(t3 as usize));
            let s5 = csub(a0, s1);
            let mut n0 = cadd(a0, s1);
            let s3 = cadd(s0, s2);
            let s4 = csub(s0, s2);
            let f2 = csub(n0, s3);
            t1 += fstride;
            t2 += fstride * 2;
            t3 += fstride * 3;
            n0 = cadd(n0, s3);
            fout[f] = n0;
            fout[f + (2 * m) as usize] = f2;
            fout[f + m as usize] = (f32(s5.0 + s4.1), f32(s5.1 - s4.0));
            fout[f + (3 * m) as usize] = (f32(s5.0 - s4.1), f32(s5.1 + s4.0));
            f += 1;
        }
    }
}

fn kf_bfly3(fout: &mut [Cpx], fstride: i64, m: i64, n: i64, mm: i64) {
    let epi = twiddle((fstride * m) as usize).1;
    for i in 0..n {
        let mut f = (i * mm) as usize;
        let mut t1 = 0i64;
        let mut t2 = 0i64;
        let mut k = m;
        while k != 0 {
            let a0 = fout[f];
            let s1 = cmul(fout[f + m as usize], twiddle(t1 as usize));
            let s2 = cmul(fout[f + (2 * m) as usize], twiddle(t2 as usize));
            let s3 = cadd(s1, s2);
            let s0 = csub(s1, s2);
            fout[f + m as usize] = (
                f32(a0.0 - f32(s3.0 * f32(0.5))),
                f32(a0.1 - f32(s3.1 * f32(0.5))),
            );
            let s0 = (f32(s0.0 * epi), f32(s0.1 * epi));
            let n0 = cadd(a0, s3);
            fout[f] = n0;
            let fm = fout[f + m as usize];
            fout[f + (2 * m) as usize] = (f32(fm.0 + s0.1), f32(fm.1 - s0.0));
            fout[f + m as usize] = (f32(fm.0 - s0.1), f32(fm.1 + s0.0));
            t1 += fstride;
            t2 += fstride * 2;
            f += 1;
            k -= 1;
        }
    }
}

fn kf_bfly5(fout: &mut [Cpx], fstride: i64, m: i64, n: i64, mm: i64) {
    let ya = twiddle((fstride * m) as usize);
    let yb = twiddle((fstride * 2 * m) as usize);
    for i in 0..n {
        let mut f = (i * mm) as usize;
        for u in 0..m {
            let s0 = fout[f];
            let s1 = cmul(fout[f + m as usize], twiddle((u * fstride) as usize));
            let s2 = cmul(fout[f + (2 * m) as usize], twiddle((2 * u * fstride) as usize));
            let s3 = cmul(fout[f + (3 * m) as usize], twiddle((3 * u * fstride) as usize));
            let s4 = cmul(fout[f + (4 * m) as usize], twiddle((4 * u * fstride) as usize));
            let s7 = cadd(s1, s4);
            let s10 = csub(s1, s4);
            let s8 = cadd(s2, s3);
            let s9 = csub(s2, s3);
            fout[f] = (f32(s0.0 + f32(s7.0 + s8.0)), f32(s0.1 + f32(s7.1 + s8.1)));
            let s5r = f32(s0.0 + f32(f32(s7.0 * ya.0) + f32(s8.0 * yb.0)));
            let s5i = f32(s0.1 + f32(f32(s7.1 * ya.0) + f32(s8.1 * yb.0)));
            let s6r = f32(f32(s10.1 * ya.1) + f32(s9.1 * yb.1));
            let s6i = f32(-f32(f32(s10.0 * ya.1) + f32(s9.0 * yb.1)));
            fout[f + m as usize] = (f32(s5r - s6r), f32(s5i - s6i));
            fout[f + (4 * m) as usize] = (f32(s5r + s6r), f32(s5i + s6i));
            let s11r = f32(s0.0 + f32(f32(s7.0 * yb.0) + f32(s8.0 * ya.0)));
            let s11i = f32(s0.1 + f32(f32(s7.1 * yb.0) + f32(s8.1 * ya.0)));
            let s12r = f32(f32(s9.1 * ya.1) - f32(s10.1 * yb.1));
            let s12i = f32(f32(s10.0 * yb.1) - f32(s9.0 * ya.1));
            fout[f + (2 * m) as usize] = (f32(s11r + s12r), f32(s11i + s12i));
            fout[f + (3 * m) as usize] = (f32(s11r - s12r), f32(s11i - s12i));
            f += 1;
        }
    }
}

fn fft_impl(state: usize, fout: &mut [Cpx]) {
    let factors = opusdata::FFT_FACTORS[state];
    let mut shift = opusdata::FFT_SHIFT[state] as i64;
    if shift < 0 {
        shift = 0;
    }
    let mut fstride = vec![1i64];
    let mut level = 0usize;
    loop {
        let p = factors[2 * level] as i64;
        let m = factors[2 * level + 1];
        let next = fstride[level] * p;
        fstride.push(next);
        level += 1;
        if m == 1 {
            break;
        }
    }
    let mut m = factors[2 * level - 1] as i64;
    for i in (0..level).rev() {
        let m2 = if i != 0 { factors[2 * i - 1] as i64 } else { 1 };
        let radix = factors[2 * i];
        if radix == 2 {
            kf_bfly2(fout, m, fstride[i], 0);
        } else if radix == 4 {
            kf_bfly4(fout, fstride[i] << shift, m, fstride[i], m2);
        } else if radix == 3 {
            kf_bfly3(fout, fstride[i] << shift, m, fstride[i], m2);
        } else if radix == 5 {
            kf_bfly5(fout, fstride[i] << shift, m, fstride[i], m2);
        }
        m = m2;
    }
}

fn mdct_backward(freq: &[f64], foff: usize, out: &mut [f64], ooff: usize, shift: i64, stride: i64) {
    let mut n = 1920i64;
    let mut trig_off = 0i64;
    for _ in 0..shift {
        n >>= 1;
        trig_off += n;
    }
    let n2 = n >> 1;
    let n4 = n >> 2;
    let bitrev = opusdata::FFT_BITREV[shift as usize];
    let base = ooff + (OVERLAP as usize) / 2;
    let mut xp1 = 0i64;
    let mut xp2 = stride * (n2 - 1);
    for i in 0..n4 {
        let rev = bitrev[i as usize] as usize;
        let x1 = freq[foff + xp1 as usize];
        let x2 = freq[foff + xp2 as usize];
        let ti = mdct_tw(trig_off + i);
        let tn = mdct_tw(trig_off + n4 + i);
        let yr = f32(f32(x2 * ti) + f32(x1 * tn));
        let yi = f32(f32(x1 * ti) - f32(x2 * tn));
        out[base + 2 * rev + 1] = yr;
        out[base + 2 * rev] = yi;
        xp1 += 2 * stride;
        xp2 -= 2 * stride;
    }
    let mut fout = Vec::with_capacity(n4 as usize);
    for i in 0..n4 as usize {
        fout.push((out[base + 2 * i], out[base + 2 * i + 1]));
    }
    fft_impl(shift as usize, &mut fout);
    for i in 0..n4 as usize {
        out[base + 2 * i] = fout[i].0;
        out[base + 2 * i + 1] = fout[i].1;
    }
    let mut yp0 = base;
    let mut yp1 = base + n2 as usize - 2;
    for i in 0..((n4 + 1) >> 1) {
        let re = out[yp0 + 1];
        let im = out[yp0];
        let t0 = mdct_tw(trig_off + i);
        let t1 = mdct_tw(trig_off + n4 + i);
        let yr = f32(f32(re * t0) + f32(im * t1));
        let yi = f32(f32(re * t1) - f32(im * t0));
        let re2 = out[yp1 + 1];
        let im2 = out[yp1];
        out[yp0] = yr;
        out[yp1 + 1] = yi;
        let t0 = mdct_tw(trig_off + (n4 - i - 1));
        let t1 = mdct_tw(trig_off + (n2 - i - 1));
        let yr = f32(f32(re2 * t0) + f32(im2 * t1));
        let yi = f32(f32(re2 * t1) - f32(im2 * t0));
        out[yp1] = yr;
        out[yp0 + 1] = yi;
        yp0 += 2;
        yp1 -= 2;
    }
    let mut xp = ooff + OVERLAP as usize - 1;
    let mut yp = ooff;
    let mut w1 = 0usize;
    let mut w2 = OVERLAP as usize - 1;
    for _i in 0..(OVERLAP as usize / 2) {
        let x1 = out[xp];
        let x2 = out[yp];
        let a = window_at(w1);
        let b = window_at(w2);
        out[yp] = f32(f32(b * x2) - f32(a * x1));
        out[xp] = f32(f32(a * x2) + f32(b * x1));
        yp += 1;
        xp -= 1;
        w1 += 1;
        w2 -= 1;
    }
}

fn denormalise_bands(x: &[f64], freq: &mut [f64], band_log_e: &[f64], start: usize, end: usize, m: i64, silence: bool) {
    let n = (m * SHORT_MDCT) as usize;
    if silence {
        for i in 0..n {
            freq[i] = f32(0.0);
        }
        return;
    }
    let bound = (m * eb(end as i64)) as usize;
    for i in 0..(m * eb(start as i64)) as usize {
        freq[i] = f32(0.0);
    }
    let mut pos = (m * eb(start as i64)) as usize;
    for i in start..end {
        let mut lg = f32(band_log_e[i] + EMEANS[i]);
        if f32(32.0) < lg {
            lg = f32(32.0);
        }
        let g = celt_exp2(lg);
        let band_end = (m * eb(i as i64 + 1)) as usize;
        while pos < band_end {
            freq[pos] = f32(x[pos] * g);
            pos += 1;
        }
    }
    for i in bound..n {
        freq[i] = f32(0.0);
    }
}

fn buf_at(buf: &[f64], i: i64) -> f64 {
    buf[i as usize]
}

fn comb_filter(
    buf: &mut [f64],
    off: i64,
    mut t0: i64,
    mut t1: i64,
    n: i64,
    g0: f64,
    g1: f64,
    tap0: usize,
    tap1: usize,
    mut overlap: i64,
) {
    if g0 == 0.0 && g1 == 0.0 {
        return;
    }
    if t0 < COMBFILTER_MINPERIOD {
        t0 = COMBFILTER_MINPERIOD;
    }
    if t1 < COMBFILTER_MINPERIOD {
        t1 = COMBFILTER_MINPERIOD;
    }
    let g00 = f32(g0 * GAINS[tap0][0]);
    let g01 = f32(g0 * GAINS[tap0][1]);
    let g02 = f32(g0 * GAINS[tap0][2]);
    let g10 = f32(g1 * GAINS[tap1][0]);
    let g11 = f32(g1 * GAINS[tap1][1]);
    let g12 = f32(g1 * GAINS[tap1][2]);
    let mut x1 = buf_at(buf, off - t1 + 1);
    let mut x2 = buf_at(buf, off - t1);
    let mut x3 = buf_at(buf, off - t1 - 1);
    let mut x4 = buf_at(buf, off - t1 - 2);
    if g0 == g1 && t0 == t1 && tap0 == tap1 {
        overlap = 0;
    }
    for i in 0..overlap {
        let x0 = buf_at(buf, off + i - t1 + 2);
        let fwin = f32(window_at(i as usize) * window_at(i as usize));
        let om = f32(f32(1.0) - fwin);
        let mut acc = buf_at(buf, off + i);
        acc = f32(acc + f32(f32(om * g00) * buf_at(buf, off + i - t0)));
        acc = f32(acc + f32(f32(om * g01) * f32(buf_at(buf, off + i - t0 + 1) + buf_at(buf, off + i - t0 - 1))));
        acc = f32(acc + f32(f32(om * g02) * f32(buf_at(buf, off + i - t0 + 2) + buf_at(buf, off + i - t0 - 2))));
        acc = f32(acc + f32(f32(fwin * g10) * x2));
        acc = f32(acc + f32(f32(fwin * g11) * f32(x1 + x3)));
        acc = f32(acc + f32(f32(fwin * g12) * f32(x0 + x4)));
        buf[(off + i) as usize] = acc;
        let nx4 = x3;
        let nx3 = x2;
        let nx2 = x1;
        let nx1 = x0;
        x4 = nx4;
        x3 = nx3;
        x2 = nx2;
        x1 = nx1;
    }
    if g1 == 0.0 {
        return;
    }
    let base = off + overlap;
    x4 = buf_at(buf, base - t1 - 2);
    x3 = buf_at(buf, base - t1 - 1);
    x2 = buf_at(buf, base - t1);
    x1 = buf_at(buf, base - t1 + 1);
    for j in 0..(n - overlap) {
        let x0 = buf_at(buf, base + j - t1 + 2);
        let mut acc = buf_at(buf, base + j);
        acc = f32(acc + f32(g10 * x2));
        acc = f32(acc + f32(g11 * f32(x1 + x3)));
        acc = f32(acc + f32(g12 * f32(x0 + x4)));
        buf[(base + j) as usize] = acc;
        let nx4 = x3;
        let nx3 = x2;
        let nx2 = x1;
        let nx1 = x0;
        x4 = nx4;
        x3 = nx3;
        x2 = nx2;
        x1 = nx1;
    }
}

fn soft_clip(pcm: &mut [f64]) {
    let n = pcm.len();
    for i in 0..n {
        let mut v = pcm[i];
        if v > f32(2.0) {
            v = f32(2.0);
        }
        if v < f32(-2.0) {
            v = f32(-2.0);
        }
        pcm[i] = v;
    }
    let a = f32(0.0);
    let mut i = 0usize;
    while i < n && f32(pcm[i] * a) < 0.0 {
        let v = pcm[i];
        pcm[i] = f32(v + f32(f32(a * v) * v));
        i += 1;
    }
    let mut curr = 0usize;
    let x0 = pcm[0];
    loop {
        i = curr;
        while i < n && !(pcm[i] > f32(1.0) || pcm[i] < f32(-1.0)) {
            i += 1;
        }
        if i == n {
            break;
        }
        let mut peak = i;
        let mut start = i;
        let mut end = i;
        let mut maxval = pcm[i].abs();
        while start > 0 && f32(pcm[i] * pcm[start - 1]) >= 0.0 {
            start -= 1;
        }
        while end < n && f32(pcm[i] * pcm[end]) >= 0.0 {
            let av = pcm[end].abs();
            if av > maxval {
                maxval = av;
                peak = end;
            }
            end += 1;
        }
        let special = start == 0 && f32(pcm[i] * pcm[0]) >= 0.0;
        let mut aa = f32(f32(maxval - f32(1.0)) / f32(maxval * maxval));
        aa = f32(aa + f32(aa * f32(2.4e-7)));
        if pcm[i] > 0.0 {
            aa = f32(-aa);
        }
        for j in start..end {
            let v = pcm[j];
            pcm[j] = f32(v + f32(f32(aa * v) * v));
        }
        if special && peak >= 2 {
            let mut offset = f32(x0 - pcm[0]);
            let delta = f32(offset / f32(peak as f64));
            for j in curr..peak {
                offset = f32(offset - delta);
                let mut v = f32(pcm[j] + offset);
                if v > f32(1.0) {
                    v = f32(1.0);
                }
                if v < f32(-1.0) {
                    v = f32(-1.0);
                }
                pcm[j] = v;
            }
        }
        curr = end;
        if curr == n {
            break;
        }
    }
}

fn round_half_even(x: f64) -> i32 {
    let floor = x.floor();
    let diff = x - floor;
    let r = if diff > 0.5 {
        floor + 1.0
    } else if diff < 0.5 {
        floor
    } else if (floor as i64) % 2 == 0 {
        floor
    } else {
        floor + 1.0
    };
    r as i32
}

fn float_to_int16(y: f64) -> i16 {
    let mut x = f32(y * SCALE_32768);
    if !(x > f32(-32768.0)) {
        x = f32(-32768.0);
    }
    if !(x < f32(32767.0)) {
        x = f32(32767.0);
    }
    round_half_even(x) as i16
}

fn celt_decode(payload: &[u8]) -> OpusResult<Vec<u8>> {
    let mut dec = EcDec::new(payload);
    let start = 0usize;
    let end = 13usize;
    let lm = 2i64;
    let c = 1i64;
    let total_bits = payload.len() as i64 * 8;
    let tell = dec.tell();
    let silence = if tell >= total_bits {
        1
    } else if tell == 1 {
        dec.bit_logp(15)
    } else {
        0
    };
    if silence != 0 {
        let add = total_bits - dec.tell();
        dec.nbits_total += add;
    }
    let mut post_gain = f32(0.0);
    let mut post_pitch = 0i64;
    let mut post_tap = 0i64;
    if start == 0 && dec.tell() + 16 <= total_bits {
        if dec.bit_logp(1) != 0 {
            let octave = dec.uint(6) as i64;
            post_pitch = (16i64 << octave) + dec.bits((4 + octave) as i32) as i64 - 1;
            let qg = dec.bits(3) as i64;
            if dec.tell() + 2 <= total_bits {
                post_tap = dec.icdf(&TAPSET_ICDF, 2) as i64;
            }
            post_gain = f32(POSTFILTER_STEP * f32((qg + 1) as f64));
        }
    }
    let is_transient = if lm > 0 && dec.tell() + 3 <= total_bits {
        dec.bit_logp(3)
    } else {
        0
    };
    let intra = if dec.tell() + 3 <= total_bits { dec.bit_logp(3) } else { 0 };
    let mut old_e = vec![f32(0.0); 2 * NB_EBANDS];
    let old_log = vec![NEG28; 2 * NB_EBANDS];
    let old_log2 = vec![NEG28; 2 * NB_EBANDS];
    for i in 0..NB_EBANDS {
        old_e[i] = fmax(old_e[i], old_e[NB_EBANDS + i]);
    }
    unquant_coarse_energy(&mut dec, start, end, &mut old_e, intra, lm as usize);
    let tf_res = tf_decode(&mut dec, start, end, is_transient, lm as usize);
    let mut spread = 2i32;
    if dec.tell() + 4 <= total_bits {
        spread = dec.icdf(&SPREAD_ICDF, 5);
    }
    let mut cap = vec![0i64; NB_EBANDS];
    init_caps(lm, &mut cap);
    let mut offsets = vec![0i64; NB_EBANDS];
    let mut dyn_logp = 6i64;
    let mut total_frac = total_bits << BITRES;
    let mut tell_f = dec.tell_frac();
    for i in start..end {
        let width = (eb(i as i64 + 1) - eb(i as i64)) << lm;
        let quanta = min(width << BITRES, max(6 << BITRES, width));
        let mut loop_logp = dyn_logp;
        let mut boost = 0i64;
        while tell_f + (loop_logp << BITRES) < total_frac && boost < cap[i] {
            let flag = dec.bit_logp(loop_logp as u32);
            tell_f = dec.tell_frac();
            if flag == 0 {
                break;
            }
            boost += quanta;
            total_frac -= quanta;
            loop_logp = 1;
        }
        offsets[i] = boost;
        if boost > 0 {
            dyn_logp = max(2, dyn_logp - 1);
        }
    }
    let alloc_trim = if dec.tell_frac() + (6 << BITRES) <= total_frac {
        dec.icdf(&TRIM_ICDF, 7) as i64
    } else {
        5
    };
    let mut bits = ((payload.len() as i64 * 8) << BITRES) - dec.tell_frac() - 1;
    let anti_rsv = if is_transient != 0 && lm >= 2 && bits >= ((lm + 2) << BITRES) {
        1 << BITRES
    } else {
        0
    };
    bits -= anti_rsv;
    let alloc = clt_compute_allocation(start, end, &offsets, &cap, alloc_trim, bits, c, lm, &mut dec);
    unquant_fine_energy(&mut dec, start, end, &mut old_e, &alloc.ebits);
    let m = 1i64 << lm;
    let n = m * SHORT_MDCT;
    let mut st = Store {
        x: vec![f32(0.0); n as usize],
        norm: Vec::new(),
    };
    let (collapse, seed, _balance) = quant_all_bands(
        &mut dec,
        start,
        end,
        &mut st,
        &alloc.pulses,
        if is_transient != 0 { m } else { 0 },
        spread,
        alloc.intensity,
        &tf_res,
        payload.len() as i64 * (8 << BITRES) - anti_rsv,
        alloc.balance,
        lm as i32,
        alloc.coded,
        0,
    );
    let _ = seed;
    let anti_on = if anti_rsv > 0 { dec.bits(1) } else { 0 };
    let bits_left = payload.len() as i64 * 8 - dec.tell();
    unquant_energy_finalise(&mut dec, start, end, &mut old_e, &alloc.ebits, &alloc.fine_priority, bits_left);
    if anti_on != 0 {
        anti_collapse(
            &mut st.x,
            &collapse,
            lm,
            start,
            end,
            &old_e,
            &old_log,
            &old_log2,
            &alloc.pulses,
            seed,
        );
    }
    if silence != 0 {
        for i in 0..(c as usize * NB_EBANDS) {
            old_e[i] = NEG28;
        }
    }
    let mut freq = vec![f32(0.0); n as usize];
    denormalise_bands(&st.x, &mut freq, &old_e, start, end, m, silence != 0);
    let mut mem = vec![f32(0.0); (DECODE_BUFFER_SIZE + OVERLAP) as usize];
    let out_off = (DECODE_BUFFER_SIZE - n) as usize;
    let (blocks, nb, shift) = if is_transient != 0 {
        (m, SHORT_MDCT, MAX_LM)
    } else {
        (1i64, n, MAX_LM - lm)
    };
    for b in 0..blocks {
        mdct_backward(&freq, b as usize, &mut mem, out_off + (nb * b) as usize, shift, blocks);
    }
    comb_filter(
        &mut mem,
        out_off as i64,
        COMBFILTER_MINPERIOD,
        COMBFILTER_MINPERIOD,
        SHORT_MDCT,
        f32(0.0),
        f32(0.0),
        0,
        0,
        OVERLAP,
    );
    if lm != 0 {
        comb_filter(
            &mut mem,
            out_off as i64 + SHORT_MDCT,
            COMBFILTER_MINPERIOD,
            post_pitch,
            n - SHORT_MDCT,
            f32(0.0),
            post_gain,
            0,
            post_tap as usize,
            OVERLAP,
        );
    }
    let mut pcm = vec![f32(0.0); n as usize];
    let mut mem_d = f32(0.0);
    for j in 0..n as usize {
        let tmp = f32(f32(mem[out_off + j] + VERY_SMALL) + mem_d);
        mem_d = f32(PREEMPH * tmp);
        pcm[j] = f32(tmp * INV_32768);
    }
    soft_clip(&mut pcm);
    if dec.error != 0 || dec.tell() > 8 * payload.len() as i64 || dec.front_past > 1 || dec.end_past != 0 {
        return fault(INTEGRITY_FAIL, "range coder overran the payload");
    }
    let mut out = Vec::with_capacity(pcm.len() * 2);
    for sample in pcm {
        out.extend_from_slice(&float_to_int16(sample).to_le_bytes());
    }
    Ok(out)
}

pub fn decode(data: &[u8]) -> Result<Vec<u8>, (&'static str, String)> {
    if data.is_empty() {
        return fault(BOUNDS, "empty input");
    }
    let toc = data[0];
    let config = toc >> 3;
    let stereo = (toc >> 2) & 1;
    let code = toc & 3;
    if config != 18 || stereo != 0 || code != 0 {
        return fault(UNSUPPORTED, "only TOC config 18 mono code 0 is supported");
    }
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| celt_decode(&data[1..]))) {
        Ok(result) => result,
        Err(_) => fault(INTEGRITY_FAIL, "payload could not be decoded"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BOUNDS, INTEGRITY_FAIL, UNSUPPORTED};

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn bits_of(x: f64) -> u32 {
        (x as f32).to_bits()
    }

    #[test]
    fn f32_rounds_like_python() {
        assert_eq!(bits_of(f32(0.1)), 0x3dcc_cccd);
        assert_eq!(bits_of(celt_exp2(0.0)), 0x3f7f_fb19);
        assert_eq!(bits_of(celt_exp2(1.0)), 0x3fff_fb19);
        assert_eq!(bits_of(celt_cos_norm(0.0)), 0x3f80_0000);
        assert_eq!(bits_of(celt_cos_norm(f32(0.25))), 0x3f6c_835e);
        assert_eq!(bits_of(celt_cos_norm(1.0)), 0xb33b_bd2e);
        assert_eq!(bitexact_cos(0), 32768);
        assert_eq!(bitexact_cos(100), 32767);
        assert_eq!(bitexact_cos(1000), 32618);
        assert_eq!(bitexact_cos(8192), 23171);
        assert_eq!(bitexact_cos(16384), 16554);
        assert_eq!(bitexact_log2tan(1000, 20000), -8844);
        assert_eq!(isqrt32(0), 0);
        assert_eq!(isqrt32(10), 3);
        assert_eq!(isqrt32(1000), 31);
        assert_eq!(isqrt32(u32::MAX), 65535);
        assert_eq!(bits_of(fine_offset(3, 2)), 0x3ec0_0000);
        assert_eq!(bits_of(final_offset(1, 3)), 0x3d00_0000);
    }

    const PACKET: &str = "907f144ef4125748bef98d0cddc8e97892c9d1a5";
    const PCM_HEX: &str = "00000000000000000000000000000000ffff000000000100030007000b001100180020002a003600450056006b0082009d00ba00d900f800180139015a017e01a501d201030239027102a902df020f0337035703700384039603aa03c003d903f10305040f040a04f103c20380032e03d30277021f02cf01880146010301b8005d00eeff67ffcdfe25fe77fdcffc32fca6fb2cfbc2fa61fa03faa2f93af9cbf859f8e8f782f72ef7eff6c9f6baf6bdf6ccf6e1f6f7f60cf71ff735f751f778f7adf7f1f744f8a2f807f96ff9d8f940faa8fa12fb80fbf4fb6cfce6fc5dfdcbfd2bfe7bfebdfef8fe38ff8bff0300ac009301bb022104bc057e0755092f0bfe0cb50e4f10c91127136d14a115c716e117ee18ee19dc1ab51b771c211db51d321e9c1ef31e361f641f7a1f741f511f0d1fab1e2b1e931de51c271c591b7e1a92199418811755161015b3134312c4103d0fb20d270c9e0a15098807f3055304a402e80021ff54fd87fbc1f908f85ff6c6f43ef3c2f14ff0e3ee7ded1cecc4ea77e93be813e703e60de531e46fe3c6e234e2b8e152e103e1cce0b0e0afe0cae002e156e1c6e151e2f4e2afe382e46be56be681e7ade8ede941eba8ec1feea6ef3bf1dcf28af441f602f8cbf99afb6dfd44ff1b01f302c9049b066a08330af40bae0d5d0f0011941218148b15e91633186619821a851b6e1c3c1df01d871e011f5f1f9e1fc11fc51fac1f751f201faf1e211e771db31cd41bdc1acb19a51868171816b4144013bd112b108e0ee60c350b7e09c10700063e047a02b800f9fe3dfd87fbd7f930f891f6fdf475f3f9f18bf02cefdced9eec71eb57ea51e95fe884e7bee611e67be5ffe49ce454e427e415e41fe446e489e4e8e463e5fbe5aee67de766e868e984eab7eb00ed5eeecfef53f1e6f287f435f6edf7aef974fb3ffd0dffda00a5026d042f06e90799093e0bd60c600eda0f42119712d91306151e161f170818db189519371ac11a311b8a1bca1bf11b011cf81bd51b9a1b451bd71a501ab119fb182d184a17521646152714f812b8116a100f0fa90d380cbf0a4009ba073106a60419038d0103007bfef8fc7bfb04fa95f82ef7d2f580f439f3fef1d1f0b0ef9eee9beda6ecc1ebecea27ea73e9d0e83de8bce74de7efe6a4e66ae642e62de62ae639e65ae68ee6d5e62ee799e716e8a6e848e9fbe9c1ea97eb7fec77ed80ee99efc1f0f9f13ef391f4f1f55ef7d5f856fae1fb74fd0effae005202f903a2054b07f308980a390cd30d660fef106d12df1342159516d8170719221a271b161cec1caa1d4e1ed71e451f971fcd1fe71fe51fc61f8c1f361fc51e3a1e951dd81c";

    fn to_hex(data: &[u8]) -> String {
        data.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn golden_packet_matches_python() {
        let got = decode(&unhex(PACKET)).expect("decode");
        assert_eq!(got.len(), 960);
        assert_eq!(to_hex(&got), PCM_HEX);
    }

    #[test]
    fn empty_is_bounds() {
        let err = decode(&[]).unwrap_err();
        assert_eq!(err.0, BOUNDS);
        assert_eq!(err.1, "empty input");
    }

    #[test]
    fn toc_only_is_integrity_fail() {
        let err = decode(&[0x90]).unwrap_err();
        assert_eq!(err.0, INTEGRITY_FAIL);
    }

    #[test]
    fn other_toc_is_unsupported() {
        let err = decode(&[0x00]).unwrap_err();
        assert_eq!(err.0, UNSUPPORTED);
    }
}
