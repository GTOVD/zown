//! Ed25519 (RFC 8032) and X25519 (RFC 7748). Same arithmetic as `zown/edwards.py`.
//! Not constant-time.

use std::sync::OnceLock;

use super::bigint::BigInt;
use super::sha512;

#[derive(Clone)]
struct Pt {
    x: BigInt,
    y: BigInt,
    z: BigInt,
    t: BigInt,
}

fn p() -> &'static BigInt {
    static P: OnceLock<BigInt> = OnceLock::new();
    P.get_or_init(|| BigInt::from_u64(1).shl(255).sub(&BigInt::from_u64(19)))
}

fn order() -> &'static BigInt {
    static L: OnceLock<BigInt> = OnceLock::new();
    L.get_or_init(|| {
        BigInt::from_u64(1).shl(252).add(
            &BigInt::parse_dec("27742317777372353535851937790883648493").unwrap(),
        )
    })
}

fn from_le(bytes: &[u8]) -> BigInt {
    let mut n = BigInt::zero();
    for (i, b) in bytes.iter().enumerate() {
        if *b == 0 {
            continue;
        }
        n = n.add(&BigInt::from_u64(*b as u64).shl((i as u32) * 8));
    }
    n
}

fn to_le(n: &BigInt, len: usize) -> Vec<u8> {
    let mut out = vec![0u8; len];
    let m = BigInt::from_u64(256);
    let mut cur = n.clone();
    for slot in &mut out {
        let (q, r) = cur.divmod_floor(&m).unwrap();
        *slot = r.to_u32().unwrap_or(0) as u8;
        cur = q;
    }
    out
}

fn fe_add(a: &BigInt, b: &BigInt) -> BigInt {
    a.add(b).divmod_floor(p()).unwrap().1
}

fn fe_sub(a: &BigInt, b: &BigInt) -> BigInt {
    let d = a.sub(b);
    if d.is_neg() {
        d.add(p())
    } else {
        d.divmod_floor(p()).unwrap().1
    }
}

fn fe_mul(a: &BigInt, b: &BigInt) -> BigInt {
    a.mul(b).divmod_floor(p()).unwrap().1
}

fn fe_pow(base: &BigInt, exp: &BigInt) -> BigInt {
    let mut result = BigInt::one();
    let mut b = base.clone();
    let bits = exp.bit_len();
    for i in 0..bits {
        if exp.bit(i) {
            result = fe_mul(&result, &b);
        }
        if i + 1 < bits {
            b = fe_mul(&b, &b);
        }
    }
    result
}

fn fe_inv(x: &BigInt) -> BigInt {
    fe_pow(x, &p().sub(&BigInt::from_u64(2)))
}

fn d_const() -> &'static BigInt {
    static D: OnceLock<BigInt> = OnceLock::new();
    D.get_or_init(|| {
        let num = p().sub(&BigInt::from_u64(121665));
        fe_mul(&num, &fe_inv(&BigInt::from_u64(121666)))
    })
}

fn sqrt_m1() -> &'static BigInt {
    static I: OnceLock<BigInt> = OnceLock::new();
    I.get_or_init(|| fe_pow(&BigInt::from_u64(2), &p().sub(&BigInt::one()).shr(2)))
}

fn recover_x(y: &BigInt, sign: u8) -> Option<BigInt> {
    // y >= p is not a field element.
    if !y.sub(p()).is_neg() {
        return None;
    }
    let y2 = fe_mul(y, y);
    let xx = fe_mul(
        &fe_sub(&y2, &BigInt::one()),
        &fe_inv(&fe_add(&fe_mul(d_const(), &y2), &BigInt::one())),
    );
    let mut x = fe_pow(&xx, &p().add(&BigInt::from_u64(3)).shr(3));
    if fe_mul(&x, &x) != xx {
        x = fe_mul(&x, sqrt_m1());
    }
    if fe_mul(&x, &x) != xx {
        return None;
    }
    if (low_bit(&x) as u8) != sign {
        x = fe_sub(p(), &x);
    }
    Some(x)
}

fn low_bit(n: &BigInt) -> bool {
    n.bit(0)
}

fn encode(x: &BigInt, y: &BigInt) -> [u8; 32] {
    let mut bits = y.clone();
    if low_bit(x) {
        bits = bits.add(&BigInt::from_u64(1).shl(255));
    }
    let bytes = to_le(&bits, 32);
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    out
}

fn decode(raw: &[u8]) -> Option<(BigInt, BigInt)> {
    if raw.len() != 32 {
        return None;
    }
    let mut ybytes = raw.to_vec();
    let sign = ybytes[31] >> 7;
    ybytes[31] &= 127;
    let y = from_le(&ybytes);
    let x = recover_x(&y, sign)?;
    Some((x, y))
}

fn affine(x: BigInt, y: BigInt) -> Pt {
    let t = fe_mul(&x, &y);
    Pt { x, y, z: BigInt::one(), t }
}

fn identity() -> Pt {
    Pt {
        x: BigInt::zero(),
        y: BigInt::one(),
        z: BigInt::one(),
        t: BigInt::zero(),
    }
}

fn add(p: &Pt, q: &Pt) -> Pt {
    let two_d = fe_add(d_const(), d_const());
    let a = fe_mul(&fe_sub(&p.y, &p.x), &fe_sub(&q.y, &q.x));
    let b = fe_mul(&fe_add(&p.y, &p.x), &fe_add(&q.y, &q.x));
    let c = fe_mul(&fe_mul(&two_d, &p.t), &q.t);
    let d = fe_add(&fe_mul(&p.z, &q.z), &fe_mul(&p.z, &q.z));
    let e = fe_sub(&b, &a);
    let f = fe_sub(&d, &c);
    let g = fe_add(&d, &c);
    let h = fe_add(&b, &a);
    Pt {
        x: fe_mul(&e, &f),
        y: fe_mul(&g, &h),
        z: fe_mul(&f, &g),
        t: fe_mul(&e, &h),
    }
}

fn dbl(p: &Pt) -> Pt {
    let a = fe_mul(&p.x, &p.x);
    let b = fe_mul(&p.y, &p.y);
    let zz = fe_mul(&p.z, &p.z);
    let c = fe_add(&zz, &zz);
    let h = fe_add(&a, &b);
    let sum = fe_add(&p.x, &p.y);
    let e = fe_sub(&h, &fe_mul(&sum, &sum));
    let g = fe_sub(&a, &b);
    let f = fe_add(&c, &g);
    Pt {
        x: fe_mul(&e, &f),
        y: fe_mul(&g, &h),
        z: fe_mul(&f, &g),
        t: fe_mul(&e, &h),
    }
}

fn encode_pt(p: &Pt) -> [u8; 32] {
    let zinv = fe_inv(&p.z);
    encode(&fe_mul(&p.x, &zinv), &fe_mul(&p.y, &zinv))
}

fn scalarmult(p: &Pt, mut s: BigInt) -> Pt {
    let mut r = identity();
    let mut q = p.clone();
    while !s.is_zero() {
        if s.bit(0) {
            r = add(&r, &q);
        }
        q = dbl(&q);
        s = s.shr(1);
    }
    r
}

fn base() -> &'static Pt {
    static B: OnceLock<Pt> = OnceLock::new();
    B.get_or_init(|| {
        let y = fe_mul(&BigInt::from_u64(4), &fe_inv(&BigInt::from_u64(5)));
        let x = recover_x(&y, 0).expect("base point");
        affine(x, y)
    })
}

fn clamp_ed(h0: &[u8]) -> BigInt {
    let mut a = h0[..32].to_vec();
    a[0] &= 248;
    a[31] &= 63;
    a[31] |= 64;
    from_le(&a)
}

#[allow(dead_code)] // used by the RFC test; sign derives the same point itself
pub fn public_key(seed: &[u8]) -> [u8; 32] {
    let digest = sha512::hash(seed);
    let a = clamp_ed(&digest);
    encode_pt(&scalarmult(base(), a))
}

pub fn sign(seed: &[u8], message: &[u8]) -> [u8; 64] {
    let digest = sha512::hash(seed);
    let a = clamp_ed(&digest);
    let prefix = &digest[32..];
    let pub_bytes = encode_pt(&scalarmult(base(), a.clone()));
    let mut r_in = prefix.to_vec();
    r_in.extend_from_slice(message);
    let r = from_le(&sha512::hash(&r_in)).divmod_floor(order()).unwrap().1;
    let big_r = encode_pt(&scalarmult(base(), r.clone()));
    let mut k_in = big_r.to_vec();
    k_in.extend_from_slice(&pub_bytes);
    k_in.extend_from_slice(message);
    let k = from_le(&sha512::hash(&k_in)).divmod_floor(order()).unwrap().1;
    let s = r.add(&k.mul(&a)).divmod_floor(order()).unwrap().1;
    let mut out = [0u8; 64];
    out[..32].copy_from_slice(&big_r);
    out[32..].copy_from_slice(&to_le(&s, 32));
    out
}

pub fn verify(public: &[u8], message: &[u8], signature: &[u8]) -> bool {
    if public.len() != 32 || signature.len() != 64 {
        return false;
    }
    let (big_r, s_bytes) = signature.split_at(32);
    let s = from_le(s_bytes);
    if !s.sub(order()).is_neg() {
        return false;
    }
    let Some((ax, ay)) = decode(public) else {
        return false;
    };
    let Some((rx, ry)) = decode(big_r) else {
        return false;
    };
    let mut k_in = big_r.to_vec();
    k_in.extend_from_slice(public);
    k_in.extend_from_slice(message);
    let k = from_le(&sha512::hash(&k_in)).divmod_floor(order()).unwrap().1;
    let left = encode_pt(&scalarmult(base(), s));
    let right = encode_pt(&add(&affine(rx, ry), &scalarmult(&affine(ax, ay), k)));
    left == right
}

pub fn x25519(scalar: &[u8], point: &[u8]) -> [u8; 32] {
    let mut k = scalar.to_vec();
    k[0] &= 248;
    k[31] &= 127;
    k[31] |= 64;
    let x1 = from_le(point).divmod_floor(p()).unwrap().1;
    let mut x2 = BigInt::one();
    let mut z2 = BigInt::zero();
    let mut x3 = x1.clone();
    let mut z3 = BigInt::one();
    let mut swap = 0u8;
    let a24 = BigInt::from_u64(121665);
    for t in (0..255).rev() {
        let kt = (k[t / 8] >> (t % 8)) & 1;
        swap ^= kt;
        if swap == 1 {
            std::mem::swap(&mut x2, &mut x3);
            std::mem::swap(&mut z2, &mut z3);
        }
        swap = kt;
        let a = fe_add(&x2, &z2);
        let aa = fe_mul(&a, &a);
        let b = fe_sub(&x2, &z2);
        let bb = fe_mul(&b, &b);
        let e = fe_sub(&aa, &bb);
        let c = fe_add(&x3, &z3);
        let d = fe_sub(&x3, &z3);
        let da = fe_mul(&d, &a);
        let cb = fe_mul(&c, &b);
        let da_cb = fe_add(&da, &cb);
        x3 = fe_mul(&da_cb, &da_cb);
        let da_m_cb = fe_sub(&da, &cb);
        let sq = fe_mul(&da_m_cb, &da_m_cb);
        z3 = fe_mul(&x1, &sq);
        x2 = fe_mul(&aa, &bb);
        let aa_e = fe_add(&aa, &fe_mul(&a24, &e));
        z2 = fe_mul(&e, &aa_e);
    }
    if swap == 1 {
        x2 = x3;
        z2 = z3;
    }
    let out = fe_mul(&x2, &fe_inv(&z2));
    let bytes = to_le(&out, 32);
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    arr
}

#[cfg(test)]
mod tests {
    use super::{public_key, sign, verify, x25519};

    fn hx(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn rfc8032_test1_and_test2() {
        let seed = hx("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let pubk = hx("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        let sig = hx("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b");
        assert_eq!(public_key(&seed).to_vec(), pubk);
        assert_eq!(sign(&seed, b"").to_vec(), sig);
        assert!(verify(&pubk, b"", &sig));
        let mut bad = sig.clone();
        bad[0] ^= 1;
        assert!(!verify(&pubk, b"", &bad));

        let seed2 = hx("4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb");
        let pub2 = hx("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c");
        let sig2 = hx("92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00");
        assert_eq!(public_key(&seed2).to_vec(), pub2);
        assert_eq!(sign(&seed2, b"r").to_vec(), sig2);
        assert!(verify(&pub2, b"r", &sig2));
    }

    #[test]
    fn rfc7748_shared_secret() {
        let a = hx("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a");
        let b = hx("5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb");
        let ka = hx("8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a");
        let kb = hx("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f");
        let k = hx("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742");
        let mut base = vec![9u8];
        base.extend(std::iter::repeat(0).take(31));
        assert_eq!(x25519(&a, &base).to_vec(), ka);
        assert_eq!(x25519(&b, &base).to_vec(), kb);
        assert_eq!(x25519(&a, &kb).to_vec(), k);
        assert_eq!(x25519(&b, &ka).to_vec(), k);
    }
}
