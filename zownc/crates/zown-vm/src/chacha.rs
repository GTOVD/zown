//! ChaCha20-Poly1305, IETF AEAD (RFC 8439). Same construction as `zown/aead.py`.

use super::bigint::BigInt;

fn rotl(x: u32, n: u32) -> u32 {
    x.rotate_left(n)
}

fn quarter(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = rotl(s[d] ^ s[a], 16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = rotl(s[b] ^ s[c], 12);
    s[a] = s[a].wrapping_add(s[b]);
    s[d] = rotl(s[d] ^ s[a], 8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = rotl(s[b] ^ s[c], 7);
}

fn word(b: &[u8]) -> u32 {
    u32::from_le_bytes(b.try_into().unwrap())
}

pub fn chacha20_block(key: &[u8], nonce: &[u8], counter: u32) -> [u8; 64] {
    let mut state = [0u32; 16];
    state[0] = word(b"expa");
    state[1] = word(b"nd 3");
    state[2] = word(b"2-by");
    state[3] = word(b"te k");
    for i in 0..8 {
        state[4 + i] = word(&key[i * 4..i * 4 + 4]);
    }
    state[12] = counter;
    for i in 0..3 {
        state[13 + i] = word(&nonce[i * 4..i * 4 + 4]);
    }
    let orig = state;
    for _ in 0..10 {
        quarter(&mut state, 0, 4, 8, 12);
        quarter(&mut state, 1, 5, 9, 13);
        quarter(&mut state, 2, 6, 10, 14);
        quarter(&mut state, 3, 7, 11, 15);
        quarter(&mut state, 0, 5, 10, 15);
        quarter(&mut state, 1, 6, 11, 12);
        quarter(&mut state, 2, 7, 8, 13);
        quarter(&mut state, 3, 4, 9, 14);
    }
    let mut out = [0u8; 64];
    for i in 0..16 {
        out[i * 4..i * 4 + 4].copy_from_slice(&state[i].wrapping_add(orig[i]).to_le_bytes());
    }
    out
}

pub fn chacha20_xor(key: &[u8], nonce: &[u8], mut counter: u32, data: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; data.len()];
    let mut offset = 0;
    while offset < data.len() {
        let stream = chacha20_block(key, nonce, counter);
        let take = (data.len() - offset).min(64);
        for i in 0..take {
            out[offset + i] = data[offset + i] ^ stream[i];
        }
        offset += take;
        counter = counter.wrapping_add(1);
    }
    out
}

fn le_int(bytes: &[u8]) -> BigInt {
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

fn poly_p() -> BigInt {
    BigInt::from_u64(1).shl(130).sub(&BigInt::from_u64(5))
}

fn clamp_r(key: &[u8]) -> BigInt {
    let mut r = key[..16].to_vec();
    r[3] &= 15;
    r[7] &= 15;
    r[11] &= 15;
    r[15] &= 15;
    r[4] &= 252;
    r[8] &= 252;
    r[12] &= 252;
    le_int(&r)
}

pub fn poly1305(key: &[u8], msg: &[u8]) -> [u8; 16] {
    let r = clamp_r(key);
    let s = le_int(&key[16..32]);
    let p = poly_p();
    let mut h = BigInt::zero();
    let mut i = 0;
    while i < msg.len() {
        let end = (i + 16).min(msg.len());
        let chunk = &msg[i..end];
        let mut n = le_int(chunk);
        n = n.add(&BigInt::from_u64(1).shl((8 * chunk.len()) as u32));
        h = h.add(&n).mul(&r);
        h = h.divmod_floor(&p).unwrap().1;
        i += 16;
    }
    let mask = BigInt::from_u64(1).shl(128).sub(&BigInt::one());
    let tag = h.add(&s);
    let tag = tag.divmod_floor(&mask.add(&BigInt::one())).unwrap().1;
    let bytes = to_le(&tag, 16);
    let mut out = [0u8; 16];
    out.copy_from_slice(&bytes);
    out
}

fn pad16(n: usize) -> usize {
    if n % 16 == 0 {
        0
    } else {
        16 - (n % 16)
    }
}

pub fn seal(key: &[u8], nonce: &[u8], plain: &[u8], aad: &[u8]) -> Vec<u8> {
    let otk = chacha20_block(key, nonce, 0);
    let ct = chacha20_xor(key, nonce, 1, plain);
    let mut mac = Vec::with_capacity(aad.len() + ct.len() + 48);
    mac.extend_from_slice(aad);
    mac.extend(std::iter::repeat(0).take(pad16(aad.len())));
    mac.extend_from_slice(&ct);
    mac.extend(std::iter::repeat(0).take(pad16(ct.len())));
    mac.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    mac.extend_from_slice(&(ct.len() as u64).to_le_bytes());
    let tag = poly1305(&otk[..32], &mac);
    let mut out = ct;
    out.extend_from_slice(&tag);
    out
}

pub fn open_box(key: &[u8], nonce: &[u8], boxed: &[u8], aad: &[u8]) -> Option<Vec<u8>> {
    if boxed.len() < 16 {
        return None;
    }
    let (ct, tag) = boxed.split_at(boxed.len() - 16);
    let otk = chacha20_block(key, nonce, 0);
    let mut mac = Vec::with_capacity(aad.len() + ct.len() + 48);
    mac.extend_from_slice(aad);
    mac.extend(std::iter::repeat(0).take(pad16(aad.len())));
    mac.extend_from_slice(ct);
    mac.extend(std::iter::repeat(0).take(pad16(ct.len())));
    mac.extend_from_slice(&(aad.len() as u64).to_le_bytes());
    mac.extend_from_slice(&(ct.len() as u64).to_le_bytes());
    let got = poly1305(&otk[..32], &mac);
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= got[i] ^ tag[i];
    }
    if diff != 0 {
        return None;
    }
    Some(chacha20_xor(key, nonce, 1, ct))
}

#[cfg(test)]
mod tests {
    use super::{open_box, seal};

    fn hx(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn rfc8439_aead() {
        let key = hx("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f");
        let nonce = hx("070000004041424344454647");
        let aad = hx("50515253c0c1c2c3c4c5c6c7");
        let plain = hx("4c616469657320616e642047656e746c656d656e206f662074686520636c617373206f66202739393a204966204920636f756c64206f6666657220796f75206f6e6c79206f6e652074697020666f7220746865206675747572652c2073756e73637265656e20776f756c642062652069742e");
        let ct = hx("d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de576d26586cec64b6116");
        let tag = hx("1ae10b594f09e26a7e902ecbd0600691");
        let mut want = ct;
        want.extend_from_slice(&tag);
        let got = seal(&key, &nonce, &plain, &aad);
        assert_eq!(got, want);
        assert_eq!(open_box(&key, &nonce, &got, &aad).unwrap(), plain);
        let mut bad = got.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        assert!(open_box(&key, &nonce, &bad, &aad).is_none());
    }
}
