//! Arbitrary-precision integers for `big` (SPEC §11).
//!
//! Schoolbook arithmetic on little-endian base-2^32 limbs. Rounding into
//! `f64` follows round-to-nearest-even so `float`-sized results match the
//! Python oracle (`int` → `float`, and inexact `/`).

use std::cmp::Ordering;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BigInt {
    /// `true` only when the value is strictly negative. Zero is non-negative.
    neg: bool,
    /// Little-endian limbs. Empty means zero. No leading zero limb.
    d: Vec<u32>,
}

impl BigInt {
    pub fn zero() -> Self {
        BigInt {
            neg: false,
            d: Vec::new(),
        }
    }

    pub fn one() -> Self {
        BigInt::from_u64(1)
    }

    pub fn from_u64(n: u64) -> Self {
        if n == 0 {
            return BigInt::zero();
        }
        let lo = n as u32;
        let hi = (n >> 32) as u32;
        let mut d = vec![lo];
        if hi != 0 {
            d.push(hi);
        }
        BigInt { neg: false, d }
    }

    pub fn from_i64(n: i64) -> Self {
        if n >= 0 {
            BigInt::from_u64(n as u64)
        } else if n == i64::MIN {
            BigInt::from_u64(1u64 << 63).neg()
        } else {
            BigInt::from_u64((-n) as u64).neg()
        }
    }

    pub fn is_zero(&self) -> bool {
        self.d.is_empty()
    }

    pub fn is_neg(&self) -> bool {
        self.neg
    }

    pub fn neg(&self) -> Self {
        if self.is_zero() {
            return BigInt::zero();
        }
        BigInt {
            neg: !self.neg,
            d: self.d.clone(),
        }
    }

    pub fn abs(&self) -> Self {
        BigInt {
            neg: false,
            d: self.d.clone(),
        }
    }

    /// Decimal parse (`[+-]?[0-9]+`). `None` when the text is not an integer.
    pub fn parse_dec(s: &str) -> Option<Self> {
        let s = s.trim();
        let (neg, digits) = if let Some(rest) = s.strip_prefix('-') {
            (true, rest)
        } else if let Some(rest) = s.strip_prefix('+') {
            (false, rest)
        } else {
            (false, s)
        };
        if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let mut n = BigInt::zero();
        for c in digits.bytes() {
            n = n.mul_u32(10).add(&BigInt::from_u64((c - b'0') as u64));
        }
        if neg && !n.is_zero() {
            n.neg = true;
        }
        Some(n)
    }

    pub fn to_i64(&self) -> Option<i64> {
        let mag = self.mag_u64()?;
        if self.neg {
            if mag == 1u64 << 63 {
                Some(i64::MIN)
            } else if mag < 1u64 << 63 {
                Some(-(mag as i64))
            } else {
                None
            }
        } else if mag < 1u64 << 63 {
            Some(mag as i64)
        } else {
            None
        }
    }

    /// Exponent for `pow`, when it fits in `u32`.
    pub fn to_u32(&self) -> Option<u32> {
        if self.neg {
            return None;
        }
        let mag = self.mag_u64()?;
        if mag > u32::MAX as u64 {
            None
        } else {
            Some(mag as u32)
        }
    }

    pub fn bit_len(&self) -> u32 {
        if self.d.is_empty() {
            return 0;
        }
        let hi = *self.d.last().unwrap();
        let top = 32 - hi.leading_zeros();
        (self.d.len() as u32 - 1) * 32 + top
    }

    pub fn bit(&self, i: u32) -> bool {
        let limb = (i / 32) as usize;
        if limb >= self.d.len() {
            return false;
        }
        (self.d[limb] >> (i % 32)) & 1 == 1
    }

    /// Any bit strictly below index `k`.
    pub fn any_bit_below(&self, k: u32) -> bool {
        if k == 0 {
            return false;
        }
        let limb = (k / 32) as usize;
        let bit = k % 32;
        for (i, l) in self.d.iter().enumerate() {
            if i > limb {
                break;
            }
            let mask = if i < limb {
                u32::MAX
            } else if bit == 0 {
                0
            } else {
                (1u32 << bit) - 1
            };
            if l & mask != 0 {
                return true;
            }
        }
        false
    }

    pub fn shl(&self, bits: u32) -> Self {
        if self.is_zero() || bits == 0 {
            return self.clone();
        }
        let limb_shift = (bits / 32) as usize;
        let bit_shift = bits % 32;
        let mut d = vec![0u32; limb_shift];
        if bit_shift == 0 {
            d.extend_from_slice(&self.d);
        } else {
            let mut carry = 0u32;
            for &limb in &self.d {
                let val = ((limb as u64) << bit_shift) | carry as u64;
                d.push(val as u32);
                carry = (val >> 32) as u32;
            }
            if carry != 0 {
                d.push(carry);
            }
        }
        BigInt { neg: self.neg, d }
    }

    pub fn shr(&self, bits: u32) -> Self {
        if self.is_zero() || bits == 0 {
            return self.clone();
        }
        let limb_shift = (bits / 32) as usize;
        if limb_shift >= self.d.len() {
            return BigInt::zero();
        }
        let bit_shift = bits % 32;
        let mut d = Vec::new();
        if bit_shift == 0 {
            d.extend_from_slice(&self.d[limb_shift..]);
        } else {
            for idx in limb_shift..self.d.len() {
                let cur = self.d[idx] as u64;
                let high = if idx + 1 < self.d.len() {
                    self.d[idx + 1] as u64
                } else {
                    0
                };
                d.push(((cur | (high << 32)) >> bit_shift) as u32);
            }
        }
        let mut b = BigInt { neg: self.neg, d };
        b.normalize();
        b
    }

    pub fn add(&self, other: &Self) -> Self {
        if self.neg == other.neg {
            let mut r = self.add_mag(other);
            r.neg = self.neg && !r.is_zero();
            r
        } else if self.cmp_mag(other) != Ordering::Less {
            let mut r = self.sub_mag(other);
            r.neg = self.neg && !r.is_zero();
            r
        } else {
            let mut r = other.sub_mag(self);
            r.neg = other.neg && !r.is_zero();
            r
        }
    }

    pub fn sub(&self, other: &Self) -> Self {
        self.add(&other.neg())
    }

    pub fn mul(&self, other: &Self) -> Self {
        if self.is_zero() || other.is_zero() {
            return BigInt::zero();
        }
        let mut d = vec![0u32; self.d.len() + other.d.len()];
        for i in 0..self.d.len() {
            let mut carry: u128 = 0;
            for j in 0..other.d.len() {
                let t = d[i + j] as u128 + self.d[i] as u128 * other.d[j] as u128 + carry;
                d[i + j] = t as u32;
                carry = t >> 32;
            }
            let mut k = i + other.d.len();
            while carry != 0 {
                if k == d.len() {
                    d.push(0);
                }
                let t = d[k] as u128 + carry;
                d[k] = t as u32;
                carry = t >> 32;
                k += 1;
            }
        }
        let mut b = BigInt {
            neg: self.neg ^ other.neg,
            d,
        };
        b.normalize();
        b
    }

    pub fn mul_u32(&self, m: u32) -> Self {
        if self.is_zero() || m == 0 {
            return BigInt::zero();
        }
        let mut carry = 0u64;
        let mut d = Vec::with_capacity(self.d.len() + 1);
        for &limb in &self.d {
            let t = limb as u64 * m as u64 + carry;
            d.push(t as u32);
            carry = t >> 32;
        }
        if carry != 0 {
            d.push(carry as u32);
        }
        BigInt { neg: self.neg, d }
    }

    /// Python floor division. `None` when `b` is zero.
    /// `a = q*b + r` with `r` having the sign of `b` (or zero).
    pub fn divmod_floor(&self, b: &Self) -> Option<(Self, Self)> {
        if b.is_zero() {
            return None;
        }
        let (q_mag, r_mag) = divmod_mag(&self.abs(), &b.abs());
        if self.neg == b.neg {
            let r = if self.neg { r_mag.neg() } else { r_mag };
            Some((q_mag, r))
        } else if r_mag.is_zero() {
            Some((q_mag.neg(), BigInt::zero()))
        } else {
            let q = q_mag.add(&BigInt::one()).neg();
            let r_abs = b.abs().sub(&r_mag);
            let r = if b.neg { r_abs.neg() } else { r_abs };
            Some((q, r))
        }
    }

    pub fn pow_u32(&self, mut exp: u32) -> Self {
        if exp == 0 {
            return BigInt::one();
        }
        let mut result = BigInt::one();
        let mut base = self.clone();
        while exp > 0 {
            if exp & 1 == 1 {
                result = result.mul(&base);
            }
            exp >>= 1;
            if exp > 0 {
                base = base.mul(&base);
            }
        }
        result
    }

    /// Integer square root (floor). The caller guarantees `self >= 0`.
    pub fn isqrt(&self) -> Self {
        if self.is_zero() {
            return BigInt::zero();
        }
        let mut x = BigInt::one().shl(self.bit_len() / 2);
        if x.is_zero() {
            x = BigInt::one();
        }
        loop {
            let q = self.divmod_floor(&x).unwrap().0;
            let y = x.add(&q).shr(1);
            if y.cmp(&x) != Ordering::Less {
                return x;
            }
            x = y;
        }
    }

    /// Reduce into a `bits`-wide lane. Signed results are two's complement.
    pub fn wrap_width(&self, signed: bool, bits: u32) -> Self {
        let modulus = BigInt::one().shl(bits);
        let mut r = self.divmod_floor(&modulus).unwrap().1;
        if signed {
            let half = BigInt::one().shl(bits - 1);
            if r.cmp(&half) != Ordering::Less {
                r = r.sub(&modulus);
            }
        }
        r
    }

    /// The exact integer value of a finite whole float. `None` for non-integers.
    pub fn from_f64_exact(v: f64) -> Option<Self> {
        if !v.is_finite() || v.fract() != 0.0 {
            return None;
        }
        if v == 0.0 {
            return Some(BigInt::zero());
        }
        let neg = v.is_sign_negative();
        let bits = v.abs().to_bits();
        let exp_bits = ((bits >> 52) & 0x7ff) as i32;
        if exp_bits == 0 {
            return None;
        }
        let mant = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
        let shift = exp_bits - 1023 - 52;
        let mut b = BigInt::from_u64(mant);
        if shift > 0 {
            b = b.shl(shift as u32);
        } else if shift < 0 {
            b = b.shr((-shift) as u32);
        }
        if neg {
            b.neg = true;
        }
        Some(b)
    }

    /// Correctly rounded conversion. `None` when the magnitude overflows `f64`.
    pub fn to_f64(&self) -> Option<f64> {
        self.true_div_f64(&BigInt::one())
    }

    /// Correctly rounded `self / b` as `f64`. `None` on overflow (not on zero:
    /// the caller rejects a zero divisor first; a zero `b` also returns `None`).
    pub fn true_div_f64(&self, b: &Self) -> Option<f64> {
        if b.is_zero() {
            return None;
        }
        let neg = self.neg ^ b.neg;
        let a = self.abs();
        let b = b.abs();
        if a.is_zero() {
            return Some(signed_zero(neg));
        }
        let exp_est = a.bit_len() as i64 - b.bit_len() as i64;
        if exp_est > 1026 {
            return None;
        }
        if exp_est < -1100 {
            return Some(signed_zero(neg));
        }
        // ~56 bits of quotient so the 53-bit significand has a round bit and sticky.
        let s = 56 - exp_est;
        let (q, r) = if s >= 0 {
            a.shl(s as u32).divmod_floor(&b)?
        } else {
            a.divmod_floor(&b.shl((-s) as u32))?
        };
        f64_from_scaled(&q, -s as i32, !r.is_zero(), neg)
    }

    fn mag_u64(&self) -> Option<u64> {
        if self.bit_len() > 64 {
            return None;
        }
        match self.d.as_slice() {
            [] => Some(0),
            [lo] => Some(*lo as u64),
            [lo, hi] => Some(*lo as u64 | ((*hi as u64) << 32)),
            _ => None,
        }
    }

    fn limb(&self, i: usize) -> u32 {
        self.d.get(i).copied().unwrap_or(0)
    }

    fn cmp_mag(&self, other: &Self) -> Ordering {
        match self.bit_len().cmp(&other.bit_len()) {
            Ordering::Equal => {}
            o => return o,
        }
        for i in (0..self.d.len()).rev() {
            match self.d[i].cmp(&other.limb(i)) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        Ordering::Equal
    }

    fn add_mag(&self, other: &Self) -> Self {
        let n = self.d.len().max(other.d.len());
        let mut d = Vec::with_capacity(n + 1);
        let mut carry = 0u64;
        for i in 0..n {
            let s = self.limb(i) as u64 + other.limb(i) as u64 + carry;
            d.push(s as u32);
            carry = s >> 32;
        }
        if carry != 0 {
            d.push(carry as u32);
        }
        BigInt { neg: false, d }
    }

    fn sub_mag(&self, other: &Self) -> Self {
        let mut d = Vec::with_capacity(self.d.len());
        let mut borrow = 0i64;
        for i in 0..self.d.len() {
            let diff = self.limb(i) as i64 - other.limb(i) as i64 - borrow;
            if diff < 0 {
                d.push((diff + (1i64 << 32)) as u32);
                borrow = 1;
            } else {
                d.push(diff as u32);
                borrow = 0;
            }
        }
        let mut b = BigInt { neg: false, d };
        b.normalize();
        b
    }

    fn set_bit(&mut self, i: u32) {
        let limb = (i / 32) as usize;
        if limb >= self.d.len() {
            self.d.resize(limb + 1, 0);
        }
        self.d[limb] |= 1 << (i % 32);
    }

    fn normalize(&mut self) {
        while self.d.last() == Some(&0) {
            self.d.pop();
        }
        if self.d.is_empty() {
            self.neg = false;
        }
    }
}

impl Ord for BigInt {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.neg, other.neg) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => self.cmp_mag(other),
            (true, true) => other.cmp_mag(self),
        }
    }
}

impl PartialOrd for BigInt {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Display for BigInt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_zero() {
            return write!(f, "0");
        }
        let mut n = self.abs();
        let mut digits = Vec::new();
        while !n.is_zero() {
            let (q, r) = divmod_u32(&n, 10);
            digits.push(r);
            n = q;
        }
        if self.neg {
            write!(f, "-")?;
        }
        for d in digits.iter().rev() {
            write!(f, "{d}")?;
        }
        Ok(())
    }
}

fn divmod_u32(a: &BigInt, m: u32) -> (BigInt, u32) {
    let mut rem = 0u64;
    let mut d = vec![0u32; a.d.len()];
    for i in (0..a.d.len()).rev() {
        let cur = (rem << 32) | a.d[i] as u64;
        d[i] = (cur / m as u64) as u32;
        rem = cur % m as u64;
    }
    let mut q = BigInt { neg: false, d };
    q.normalize();
    (q, rem as u32)
}

fn divmod_mag(a: &BigInt, b: &BigInt) -> (BigInt, BigInt) {
    if a.cmp_mag(b) == Ordering::Less {
        return (BigInt::zero(), a.clone());
    }
    let mut q = BigInt::zero();
    let mut r = BigInt::zero();
    for i in (0..a.bit_len()).rev() {
        r = r.shl(1);
        if a.bit(i) {
            r = r.add(&BigInt::one());
        }
        if r.cmp_mag(b) != Ordering::Less {
            r = r.sub_mag(b);
            q.set_bit(i);
        }
    }
    (q, r)
}

fn signed_zero(neg: bool) -> f64 {
    if neg {
        -0.0
    } else {
        0.0
    }
}

/// `q * 2^scale`, round-to-nearest-even. `sticky_extra` is a nonzero remainder
/// below `q`. `None` when the result overflows to infinity.
fn f64_from_scaled(q: &BigInt, scale: i32, sticky_extra: bool, neg: bool) -> Option<f64> {
    if q.is_zero() {
        return Some(signed_zero(neg));
    }
    let q = q.abs();
    let bl = q.bit_len() as i32;
    let mut exp = (bl as i64 - 1) + scale as i64;
    let scale = scale as i64;
    let mut mant: u64 = 0;
    for i in 0..53 {
        let q_bit = (exp - i) - scale;
        if q_bit >= 0 && q_bit <= u32::MAX as i64 && q.bit(q_bit as u32) {
            mant |= 1u64 << (52 - i);
        }
    }
    let q_round = (exp - 53) - scale;
    let (round_bit, sticky) = if (0..=u32::MAX as i64).contains(&q_round) {
        (
            q.bit(q_round as u32),
            sticky_extra || q.any_bit_below(q_round as u32),
        )
    } else {
        (false, sticky_extra || q_round > 0)
    };
    if round_bit && (sticky || mant & 1 == 1) {
        mant += 1;
        if mant == (1u64 << 53) {
            mant = 1u64 << 52;
            exp += 1;
        }
    }
    if exp > 1023 {
        return None;
    }
    if exp < -1074 {
        return Some(signed_zero(neg));
    }
    let bits = if exp < -1022 {
        let k = (-1022 - exp) as u32;
        if k >= 64 {
            return Some(signed_zero(neg));
        }
        let top_lost = k > 0 && ((mant >> (k - 1)) & 1) == 1;
        let sticky_lost = (k > 1 && mant & ((1u64 << (k - 1)) - 1) != 0) || round_bit || sticky;
        let mut sig = mant >> k;
        if top_lost && (sticky_lost || sig & 1 == 1) {
            sig += 1;
        }
        if sig == 0 {
            return Some(signed_zero(neg));
        }
        if sig >= (1u64 << 52) {
            1u64 << 52 // smallest normal
        } else {
            sig
        }
    } else {
        (((exp + 1023) as u64) << 52) | (mant & ((1u64 << 52) - 1))
    };
    let bits = if neg { bits | (1u64 << 63) } else { bits };
    Some(f64::from_bits(bits))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bi(n: i64) -> BigInt {
        BigInt::from_i64(n)
    }

    #[test]
    fn dec_roundtrip_and_pow() {
        let n = BigInt::parse_dec("99999999999999999999").unwrap();
        assert_eq!(n.add(&BigInt::one()).to_string(), "100000000000000000000");
        assert_eq!(
            bi(2).pow_u32(100).to_string(),
            "1267650600228229401496703205376"
        );
        assert_eq!(BigInt::parse_dec("-0").unwrap().to_string(), "0");
        assert_eq!(bi(i64::MIN).to_i64(), Some(i64::MIN));
        assert!(bi(i64::MAX).add(&BigInt::one()).to_i64().is_none());
    }

    #[test]
    fn floor_divmod_matches_python() {
        let cases = [
            (7, 3, 2, 1),
            (-7, 3, -3, 2),
            (7, -3, -3, -2),
            (-7, -3, 2, -1),
            (-8, 4, -2, 0),
            (-1, 256, -1, 255),
        ];
        for (a, b, q, r) in cases {
            let (gq, gr) = bi(a).divmod_floor(&bi(b)).unwrap();
            assert_eq!(gq.to_i64(), Some(q), "{a} // {b}");
            assert_eq!(gr.to_i64(), Some(r), "{a} % {b}");
        }
    }

    #[test]
    fn isqrt_and_wrap() {
        assert_eq!(bi(9).isqrt().to_i64(), Some(3));
        assert_eq!(bi(8).isqrt().to_i64(), Some(2));
        assert_eq!(bi(0).isqrt().to_i64(), Some(0));
        assert_eq!(bi(128).wrap_width(true, 8).to_i64(), Some(-128));
        assert_eq!(bi(300).wrap_width(false, 8).to_i64(), Some(44));
        assert_eq!(bi(-1).wrap_width(false, 8).to_i64(), Some(255));
        assert_eq!(
            bi(-1).wrap_width(false, 64).to_string(),
            "18446744073709551615"
        );
    }

    #[test]
    fn f64_matches_python() {
        // Bit patterns are CPython's correctly-rounded float(int) / true division.
        let eq = |n: &str, bits: u64| {
            let got = BigInt::parse_dec(n).unwrap().to_f64().unwrap();
            assert_eq!(got.to_bits(), bits, "{n} -> {got:?}");
        };
        eq("10000000000000000000000", 0x4480f0cf064dd592);
        eq("9007199254740993", 0x4340000000000000);
        eq("1152921504606846977", 0x43b0000000000000);
        eq("1208925819614629174706176", 0x44f0000000000000);
        eq("1000000000000000000000000000000", 0x46293e5939a08cea);
        eq("-100000000000000000000", 0xc415af1d78b58c40);
        for n in [0, 1, 2, 3, -7, 10, 1 << 20, i64::MAX, i64::MIN] {
            assert_eq!(
                bi(n).to_f64().unwrap().to_bits(),
                (n as f64).to_bits(),
                "{n}"
            );
        }
        let q = BigInt::parse_dec("1000000000000000000000000")
            .unwrap()
            .true_div_f64(&bi(3))
            .unwrap();
        assert_eq!(q.to_bits(), 0x44d1a582513bbe78);
        assert_eq!(bi(10).true_div_f64(&bi(4)).unwrap(), 2.5);
        assert_eq!(bi(1).true_div_f64(&bi(3)).unwrap(), 1.0 / 3.0);
        assert_eq!(bi(-7).true_div_f64(&bi(2)).unwrap(), -3.5);
        assert!(BigInt::one().shl(1024).to_f64().is_none());
    }

    #[test]
    fn whole_float_roundtrip() {
        let b = BigInt::from_f64_exact(1e30).unwrap();
        assert_eq!(b.to_f64().unwrap().to_bits(), (1e30f64).to_bits());
        assert!(BigInt::from_f64_exact(2.5).is_none());
        assert!(BigInt::from_f64_exact(f64::INFINITY).is_none());
    }
}
