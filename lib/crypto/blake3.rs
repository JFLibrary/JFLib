// blake3 -- BLAKE3: hash, keyed hash, key derivation, extendable output
//
//   Blake3::new() / new_keyed(&key) / new_derive_key(context)
//   update(data); finalize() -> [u8; 32]; finalize_xof(&mut out) (any length)
//   hash(data) / keyed_hash(&key, data) / derive_key(context, material) -> [u8; 32]
//
// The tree mode of the reference implementation: 1 KiB chunks, a stack of chaining values
// merged by the number of finished chunks.  Portable code (no SIMD).

use crate::le32;

const OUT_LEN: usize = 32;
const BLOCK_LEN: usize = 64;
const CHUNK_LEN: usize = 1024;

const CHUNK_START: u32 = 1;
const CHUNK_END: u32 = 2;
const PARENT: u32 = 4;
const ROOT: u32 = 8;
const KEYED_HASH: u32 = 16;
const DERIVE_KEY_CONTEXT: u32 = 32;
const DERIVE_KEY_MATERIAL: u32 = 64;

const IV: [u32; 8] = [0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A, 0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19];

const PERM: [usize; 16] = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

#[inline(always)]
fn g(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize, x: u32, y: u32) {
    s[a] = s[a].wrapping_add(s[b]).wrapping_add(x);
    s[d] = (s[d] ^ s[a]).rotate_right(16);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_right(12);
    s[a] = s[a].wrapping_add(s[b]).wrapping_add(y);
    s[d] = (s[d] ^ s[a]).rotate_right(8);
    s[c] = s[c].wrapping_add(s[d]);
    s[b] = (s[b] ^ s[c]).rotate_right(7);
}

#[inline(always)]
fn round(s: &mut [u32; 16], m: &[u32; 16]) {
    g(s, 0, 4, 8, 12, m[0], m[1]);
    g(s, 1, 5, 9, 13, m[2], m[3]);
    g(s, 2, 6, 10, 14, m[4], m[5]);
    g(s, 3, 7, 11, 15, m[6], m[7]);
    g(s, 0, 5, 10, 15, m[8], m[9]);
    g(s, 1, 6, 11, 12, m[10], m[11]);
    g(s, 2, 7, 8, 13, m[12], m[13]);
    g(s, 3, 4, 9, 14, m[14], m[15]);
}

fn compress(cv: &[u32; 8], block: &[u32; 16], counter: u64, len: u32, flags: u32) -> [u32; 16] {
    let mut s = [
        cv[0], cv[1], cv[2], cv[3], cv[4], cv[5], cv[6], cv[7],
        IV[0], IV[1], IV[2], IV[3], counter as u32, (counter >> 32) as u32, len, flags,
    ];
    let mut m = *block;
    for r in 0..7 {
        round(&mut s, &m);
        if r < 6 {
            let p = m;
            for (i, w) in m.iter_mut().enumerate() {
                *w = p[PERM[i]];
            }
        }
    }
    for i in 0..8 {
        s[i] ^= s[i + 8];
        s[i + 8] ^= cv[i];
    }
    s
}

fn first8(w: [u32; 16]) -> [u32; 8] {
    [w[0], w[1], w[2], w[3], w[4], w[5], w[6], w[7]]
}

fn words(b: &[u8]) -> [u32; 16] {
    let mut w = [0u32; 16];
    for (i, x) in w.iter_mut().enumerate().take(b.len() / 4) {
        *x = le32(&b[i * 4..]);
    }
    w
}

fn key_words(key: &[u8; 32]) -> [u32; 8] {
    let mut w = [0u32; 8];
    for (i, x) in w.iter_mut().enumerate() {
        *x = le32(&key[i * 4..]);
    }
    w
}

#[derive(Clone, Copy)]
struct Output {
    cv: [u32; 8],
    block: [u32; 16],
    counter: u64,
    len: u32,
    flags: u32,
}

impl Output {
    fn chaining_value(&self) -> [u32; 8] {
        first8(compress(&self.cv, &self.block, self.counter, self.len, self.flags))
    }

    fn root_bytes(&self, out: &mut [u8], mut block_counter: u64, mut skip: usize) {
        let mut pos = 0;
        while pos < out.len() {
            let w = compress(&self.cv, &self.block, block_counter, self.len, self.flags | ROOT);
            let mut bytes = [0u8; 64];
            for (o, x) in bytes.chunks_exact_mut(4).zip(w) {
                o.copy_from_slice(&x.to_le_bytes());
            }
            let take = (64 - skip).min(out.len() - pos);
            out[pos..pos + take].copy_from_slice(&bytes[skip..skip + take]);
            pos += take;
            skip = 0;
            block_counter += 1;
        }
    }
}

#[derive(Clone)]
struct Chunk {
    cv: [u32; 8],
    counter: u64,
    block: [u8; BLOCK_LEN],
    block_len: usize,
    blocks: usize,
    flags: u32,
}

impl Chunk {
    fn new(key: [u32; 8], counter: u64, flags: u32) -> Chunk {
        Chunk { cv: key, counter, block: [0; BLOCK_LEN], block_len: 0, blocks: 0, flags }
    }

    fn len(&self) -> usize {
        BLOCK_LEN * self.blocks + self.block_len
    }

    fn start(&self) -> u32 {
        if self.blocks == 0 { CHUNK_START } else { 0 }
    }

    fn update(&mut self, mut input: &[u8]) {
        while !input.is_empty() {
            if self.block_len == BLOCK_LEN {
                let w = words(&self.block);
                self.cv = first8(compress(&self.cv, &w, self.counter, BLOCK_LEN as u32, self.flags | self.start()));
                self.blocks += 1;
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
            block: words(&self.block),
            counter: self.counter,
            len: self.block_len as u32,
            flags: self.flags | self.start() | CHUNK_END,
        }
    }
}

fn parent_output(left: [u32; 8], right: [u32; 8], key: [u32; 8], flags: u32) -> Output {
    let mut block = [0u32; 16];
    block[..8].copy_from_slice(&left);
    block[8..].copy_from_slice(&right);
    Output { cv: key, block, counter: 0, len: BLOCK_LEN as u32, flags: PARENT | flags }
}

/// an incremental BLAKE3 hasher
#[derive(Clone)]
pub struct Blake3 {
    chunk: Chunk,
    key: [u32; 8],
    stack: [[u32; 8]; 54],
    depth: usize,
    flags: u32,
}

impl Default for Blake3 {
    fn default() -> Self {
        Self::new()
    }
}

impl Blake3 {
    fn with(key: [u32; 8], flags: u32) -> Blake3 {
        Blake3 { chunk: Chunk::new(key, 0, flags), key, stack: [[0; 8]; 54], depth: 0, flags }
    }

    /// the plain hash function
    pub fn new() -> Blake3 {
        Blake3::with(IV, 0)
    }

    /// a keyed hash (MAC / PRF) with a 32-byte key
    pub fn new_keyed(key: &[u8; 32]) -> Blake3 {
        Blake3::with(key_words(key), KEYED_HASH)
    }

    /// key derivation: `context` is a hard-coded, unique, application-specific string
    pub fn new_derive_key(context: &str) -> Blake3 {
        let mut h = Blake3::with(IV, DERIVE_KEY_CONTEXT);
        h.update(context.as_bytes());
        let ck = h.finalize();
        Blake3::with(key_words(&ck), DERIVE_KEY_MATERIAL)
    }

    fn push_cv(&mut self, mut cv: [u32; 8], mut total: u64) {
        // merge completed subtrees: one merge per trailing zero bit of the chunk count
        while total & 1 == 0 {
            self.depth -= 1;
            cv = parent_output(self.stack[self.depth], cv, self.key, self.flags).chaining_value();
            total >>= 1;
        }
        self.stack[self.depth] = cv;
        self.depth += 1;
    }

    pub fn update(&mut self, mut input: &[u8]) -> &mut Blake3 {
        while !input.is_empty() {
            if self.chunk.len() == CHUNK_LEN {
                let cv = self.chunk.output().chaining_value();
                let total = self.chunk.counter + 1;
                self.push_cv(cv, total);
                self.chunk = Chunk::new(self.key, total, self.flags);
            }
            let take = (CHUNK_LEN - self.chunk.len()).min(input.len());
            self.chunk.update(&input[..take]);
            input = &input[take..];
        }
        self
    }

    fn root(&self) -> Output {
        let mut out = self.chunk.output();
        let mut n = self.depth;
        while n > 0 {
            n -= 1;
            out = parent_output(self.stack[n], out.chaining_value(), self.key, self.flags);
        }
        out
    }

    /// the 32-byte digest (the hasher stays usable)
    pub fn finalize(&self) -> [u8; OUT_LEN] {
        let mut out = [0u8; OUT_LEN];
        self.root().root_bytes(&mut out, 0, 0);
        out
    }

    /// any number of output bytes (the first 32 equal finalize())
    pub fn finalize_xof(&self, out: &mut [u8]) {
        self.root().root_bytes(out, 0, 0);
    }

    /// output bytes starting at byte `offset` of the output stream
    pub fn finalize_xof_at(&self, offset: u64, out: &mut [u8]) {
        self.root().root_bytes(out, offset / 64, (offset % 64) as usize);
    }
}

/// BLAKE3 of a byte string
pub fn hash(data: &[u8]) -> [u8; 32] {
    Blake3::new().update(data).finalize()
}

/// keyed BLAKE3 (a MAC / PRF)
pub fn keyed_hash(key: &[u8; 32], data: &[u8]) -> [u8; 32] {
    Blake3::new_keyed(key).update(data).finalize()
}

/// a 32-byte key derived from key material for a context string
pub fn derive_key(context: &str, material: &[u8]) -> [u8; 32] {
    Blake3::new_derive_key(context).update(material).finalize()
}
