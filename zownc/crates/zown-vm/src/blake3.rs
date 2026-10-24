//! Unkeyed BLAKE3, matching `zown/blake3.py` and the official test vectors.
//! Keyed hash and key derivation are not implemented.

const BLOCK_LEN: usize = 64;
const CHUNK_LEN: usize = 1024;

const CHUNK_START: u32 = 1 << 0;
const CHUNK_END: u32 = 1 << 1;
const PARENT: u32 = 1 << 2;
const ROOT: u32 = 1 << 3;

const IV: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];

const MSG_PERMUTATION: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

fn rotr(x: u32, n: u32) -> u32 {
    x.rotate_right(n)
}

fn g(state: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, mx: u32, my: u32) {
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(mx);
    state[d] = rotr(state[d] ^ state[a], 16);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = rotr(state[b] ^ state[c], 12);
    state[a] = state[a].wrapping_add(state[b]).wrapping_add(my);
    state[d] = rotr(state[d] ^ state[a], 8);
    state[c] = state[c].wrapping_add(state[d]);
    state[b] = rotr(state[b] ^ state[c], 7);
}

fn compress(cv: &[u32; 8], block: &[u32; 16], counter: u64, block_len: u32, flags: u32) -> [u32; 16] {
    let mut state = [
        cv[0], cv[1], cv[2], cv[3], cv[4], cv[5], cv[6], cv[7], IV[0], IV[1], IV[2], IV[3],
        counter as u32,
        (counter >> 32) as u32,
        block_len,
        flags,
    ];
    let mut m = *block;
    for _ in 0..7 {
        g(&mut state, 0, 4, 8, 12, m[0], m[1]);
        g(&mut state, 1, 5, 9, 13, m[2], m[3]);
        g(&mut state, 2, 6, 10, 14, m[4], m[5]);
        g(&mut state, 3, 7, 11, 15, m[6], m[7]);
        g(&mut state, 0, 5, 10, 15, m[8], m[9]);
        g(&mut state, 1, 6, 11, 12, m[10], m[11]);
        g(&mut state, 2, 7, 8, 13, m[12], m[13]);
        g(&mut state, 3, 4, 9, 14, m[14], m[15]);
        let mut permuted = [0u32; 16];
        for i in 0..16 {
            permuted[i] = m[MSG_PERMUTATION[i]];
        }
        m = permuted;
    }
    for i in 0..8 {
        state[i] ^= state[i + 8];
        state[i + 8] ^= cv[i];
    }
    state
}

fn words_from(block: &[u8]) -> [u32; 16] {
    let mut buf = [0u8; BLOCK_LEN];
    let n = block.len().min(BLOCK_LEN);
    buf[..n].copy_from_slice(&block[..n]);
    let mut words = [0u32; 16];
    for i in 0..16 {
        words[i] = u32::from_le_bytes(buf[i * 4..i * 4 + 4].try_into().unwrap());
    }
    words
}

struct Output {
    cv: [u32; 8],
    block: [u32; 16],
    counter: u64,
    block_len: u32,
    flags: u32,
}

impl Output {
    fn chaining_value(&self) -> [u32; 8] {
        let state = compress(&self.cv, &self.block, self.counter, self.block_len, self.flags);
        let mut cv = [0u32; 8];
        cv.copy_from_slice(&state[..8]);
        cv
    }

    fn root_bytes(&self) -> [u8; 32] {
        let words = compress(
            &self.cv,
            &self.block,
            0,
            self.block_len,
            self.flags | ROOT,
        );
        let mut out = [0u8; 32];
        for (i, word) in words.iter().take(8).enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        out
    }
}

struct Chunk {
    cv: [u32; 8],
    counter: u64,
    block: [u8; BLOCK_LEN],
    block_len: usize,
    blocks_compressed: usize,
    flags: u32,
}

impl Chunk {
    fn new(key: [u32; 8], counter: u64, flags: u32) -> Self {
        Chunk {
            cv: key,
            counter,
            block: [0; BLOCK_LEN],
            block_len: 0,
            blocks_compressed: 0,
            flags,
        }
    }

    fn len(&self) -> usize {
        BLOCK_LEN * self.blocks_compressed + self.block_len
    }

    fn start_flag(&self) -> u32 {
        if self.blocks_compressed == 0 { CHUNK_START } else { 0 }
    }

    fn update(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            if self.block_len == BLOCK_LEN {
                let words = words_from(&self.block);
                let state = compress(
                    &self.cv,
                    &words,
                    self.counter,
                    BLOCK_LEN as u32,
                    self.flags | self.start_flag(),
                );
                self.cv.copy_from_slice(&state[..8]);
                self.blocks_compressed += 1;
                self.block = [0; BLOCK_LEN];
                self.block_len = 0;
            }
            let take = (BLOCK_LEN - self.block_len).min(input.len());
            self.block[self.block_len..self.block_len + take].copy_from_slice(&input[..take]);
            self.block_len += take;
            input = &input[take..];
        }
    }

    fn output(&self) -> Output {
        Output {
            cv: self.cv,
            block: words_from(&self.block[..self.block_len]),
            counter: self.counter,
            block_len: self.block_len as u32,
            flags: self.flags | self.start_flag() | CHUNK_END,
        }
    }
}

fn parent(left: [u32; 8], right: [u32; 8], key: [u32; 8], flags: u32) -> Output {
    let mut block = [0u32; 16];
    block[..8].copy_from_slice(&left);
    block[8..].copy_from_slice(&right);
    Output {
        cv: key,
        block,
        counter: 0,
        block_len: BLOCK_LEN as u32,
        flags: PARENT | flags,
    }
}

/// 32-byte BLAKE3 digest.
pub fn hash(data: &[u8]) -> [u8; 32] {
    let key = IV;
    let mut chunk = Chunk::new(key, 0, 0);
    let mut stack: Vec<[u32; 8]> = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        if chunk.len() == CHUNK_LEN {
            let cv = chunk.output().chaining_value();
            let mut total = chunk.counter + 1;
            let mut cv = cv;
            while total & 1 == 0 {
                let left = stack.pop().unwrap();
                cv = parent(left, cv, key, 0).chaining_value();
                total >>= 1;
            }
            stack.push(cv);
            chunk = Chunk::new(key, chunk.counter + 1, 0);
        }
        let take = (CHUNK_LEN - chunk.len()).min(data.len() - offset);
        chunk.update(&data[offset..offset + take]);
        offset += take;
    }
    let mut output = chunk.output();
    for parent_cv in stack.iter().rev() {
        output = parent(*parent_cv, output.chaining_value(), key, 0);
    }
    output.root_bytes()
}

pub fn hex(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let dig = hash(data);
    let mut out = String::with_capacity(64);
    for byte in dig {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn official_vectors() {
        let cases = [
            (0, "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"),
            (1, "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213"),
            (1024, "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7"),
            (1025, "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444"),
        ];
        for (n, want) in cases {
            assert_eq!(hex(&pattern(n)), want, "len {n}");
        }
        assert_eq!(hex(b"hello"), "ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f");
    }
}
