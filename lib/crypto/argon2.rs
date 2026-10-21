// argon2 -- Argon2id (RFC 9106, version 0x13) and PHC strings
//
//   argon2id(password, salt, &Params, out)          raw tag (secret / associated data empty)
//   argon2id_ex(password, salt, secret, ad, &Params, out)
//   hash(password, salt, &Params) -> "$argon2id$v=19$m=..,t=..,p=..$salt$hash"
//   verify(phc, password) -> bool                   (constant-time comparison)
//
// Lanes are filled one after the other in each slice (the result is the same as with
// threads); memory is m KiB, allocated per call.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::b64;
use crate::blake2b::{self, Blake2b};

const BLOCK_WORDS: usize = 128;
const SYNC_POINTS: u32 = 4;
const VERSION: u32 = 0x13;
const TYPE_ID: u32 = 2;

/// cost parameters
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params {
    /// memory in KiB (at least 8 * lanes)
    pub m_kib: u32,
    /// passes over the memory
    pub t: u32,
    /// lanes (parallelism)
    pub p: u32,
    /// tag length in bytes
    pub out_len: u32,
}

impl Params {
    /// the OWASP recommendation for interactive logins: 19 MiB, 2 passes, 1 lane
    pub const INTERACTIVE: Params = Params { m_kib: 19 * 1024, t: 2, p: 1, out_len: 32 };
    /// for keys that protect data at rest (passphrases): 64 MiB, 3 passes, 1 lane
    pub const KEY: Params = Params { m_kib: 64 * 1024, t: 3, p: 1, out_len: 32 };

    fn valid(&self) -> bool {
        self.p >= 1 && self.p <= 64 && self.t >= 1 && self.m_kib >= 8 * self.p && self.m_kib <= 4 * 1024 * 1024 && self.out_len >= 4 && self.out_len <= 1024
    }
}

type Block = [u64; BLOCK_WORDS];

#[inline(always)]
fn fbla(x: u64, y: u64) -> u64 {
    x.wrapping_add(y).wrapping_add(2u64.wrapping_mul(x & 0xffff_ffff).wrapping_mul(y & 0xffff_ffff))
}

#[inline(always)]
fn gb(v: &mut Block, a: usize, b: usize, c: usize, d: usize) {
    v[a] = fbla(v[a], v[b]);
    v[d] = (v[d] ^ v[a]).rotate_right(32);
    v[c] = fbla(v[c], v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(24);
    v[a] = fbla(v[a], v[b]);
    v[d] = (v[d] ^ v[a]).rotate_right(16);
    v[c] = fbla(v[c], v[d]);
    v[b] = (v[b] ^ v[c]).rotate_right(63);
}

/// the BLAKE2b round without message on 16 words given by index
#[inline(always)]
fn round(v: &mut Block, i: [usize; 16]) {
    gb(v, i[0], i[4], i[8], i[12]);
    gb(v, i[1], i[5], i[9], i[13]);
    gb(v, i[2], i[6], i[10], i[14]);
    gb(v, i[3], i[7], i[11], i[15]);
    gb(v, i[0], i[5], i[10], i[15]);
    gb(v, i[1], i[6], i[11], i[12]);
    gb(v, i[2], i[7], i[8], i[13]);
    gb(v, i[3], i[4], i[9], i[14]);
}

/// next = G(prev, refb) (xor the old next when `xor`)
fn fill_block(prev: &Block, refb: &Block, next: &mut Block, xor: bool) {
    let mut r = [0u64; BLOCK_WORDS];
    for i in 0..BLOCK_WORDS {
        r[i] = prev[i] ^ refb[i];
    }
    let mut tmp = r;
    if xor {
        for i in 0..BLOCK_WORDS {
            tmp[i] ^= next[i];
        }
    }
    for i in 0..8 {
        let b = 16 * i;
        round(&mut r, [b, b + 1, b + 2, b + 3, b + 4, b + 5, b + 6, b + 7, b + 8, b + 9, b + 10, b + 11, b + 12, b + 13, b + 14, b + 15]);
    }
    for i in 0..8 {
        let b = 2 * i;
        round(&mut r, [b, b + 1, b + 16, b + 17, b + 32, b + 33, b + 48, b + 49, b + 64, b + 65, b + 80, b + 81, b + 96, b + 97, b + 112, b + 113]);
    }
    for i in 0..BLOCK_WORDS {
        next[i] = tmp[i] ^ r[i];
    }
}

fn block_from_bytes(b: &[u8]) -> Block {
    let mut w = [0u64; BLOCK_WORDS];
    for (i, x) in w.iter_mut().enumerate() {
        *x = crate::le64(&b[i * 8..]);
    }
    w
}

fn block_bytes(w: &Block) -> [u8; 1024] {
    let mut b = [0u8; 1024];
    for (o, x) in b.chunks_exact_mut(8).zip(w) {
        o.copy_from_slice(&x.to_le_bytes());
    }
    b
}

/// Argon2id with an empty secret and no associated data
pub fn argon2id(password: &[u8], salt: &[u8], params: &Params, out: &mut [u8]) -> bool {
    argon2id_ex(password, salt, &[], &[], params, out)
}

/// Argon2id: false for invalid parameters (out.len() must equal params.out_len)
pub fn argon2id_ex(password: &[u8], salt: &[u8], secret: &[u8], ad: &[u8], params: &Params, out: &mut [u8]) -> bool {
    if !params.valid() || out.len() != params.out_len as usize || salt.len() < 8 {
        return false;
    }
    let lanes = params.p;
    // H0
    let mut h = Blake2b::new(64, &[]);
    for v in [lanes, params.out_len, params.m_kib, params.t, VERSION, TYPE_ID] {
        h.update(&v.to_le_bytes());
    }
    for part in [password, salt, secret, ad] {
        h.update(&(part.len() as u32).to_le_bytes());
        h.update(part);
    }
    let mut h0 = [0u8; 72];
    h.finalize_into(&mut h0[..64]);
    // memory
    let segment = params.m_kib / (SYNC_POINTS * lanes);
    let lane_len = segment * SYNC_POINTS;
    let blocks = (lane_len * lanes) as usize;
    let mut mem: Vec<Block> = vec![[0u64; BLOCK_WORDS]; blocks];
    let mut buf = vec![0u8; 1024];
    for l in 0..lanes {
        for j in 0..2u32 {
            h0[64..68].copy_from_slice(&j.to_le_bytes());
            h0[68..72].copy_from_slice(&l.to_le_bytes());
            blake2b::long(&mut buf, &[&h0]);
            mem[(l * lane_len + j) as usize] = block_from_bytes(&buf);
        }
    }
    crate::zeroize(&mut h0);
    let zero: Block = [0; BLOCK_WORDS];
    for pass in 0..params.t {
        for slice in 0..SYNC_POINTS {
            for lane in 0..lanes {
                let independent = pass == 0 && slice < SYNC_POINTS / 2;
                let mut input: Block = [0; BLOCK_WORDS];
                let mut addresses: Block = [0; BLOCK_WORDS];
                if independent {
                    input[0] = pass as u64;
                    input[1] = lane as u64;
                    input[2] = slice as u64;
                    input[3] = blocks as u64;
                    input[4] = params.t as u64;
                    input[5] = TYPE_ID as u64;
                }
                let next_addresses = |input: &mut Block, addresses: &mut Block| {
                    input[6] += 1;
                    fill_block(&zero, input, addresses, false);
                    let a = *addresses;
                    fill_block(&zero, &a, addresses, false);
                };
                let mut start = 0u32;
                if pass == 0 && slice == 0 {
                    start = 2;
                    if independent {
                        next_addresses(&mut input, &mut addresses);
                    }
                }
                let mut cur = lane * lane_len + slice * segment + start;
                let mut prev = if cur % lane_len == 0 { cur + lane_len - 1 } else { cur - 1 };
                for i in start..segment {
                    if cur % lane_len == 1 {
                        prev = cur - 1;
                    }
                    let rand = if independent {
                        if i % BLOCK_WORDS as u32 == 0 {
                            next_addresses(&mut input, &mut addresses);
                        }
                        addresses[(i % BLOCK_WORDS as u32) as usize]
                    } else {
                        mem[prev as usize][0]
                    };
                    let ref_lane = if pass == 0 && slice == 0 { lane } else { ((rand >> 32) % lanes as u64) as u32 };
                    let same = ref_lane == lane;
                    // the reference area (RFC 9106 3.4.1.2)
                    let area: u64 = if pass == 0 {
                        if slice == 0 {
                            (i - 1) as u64
                        } else if same {
                            (slice * segment + i - 1) as u64
                        } else {
                            (slice * segment) as u64 - if i == 0 { 1 } else { 0 }
                        }
                    } else if same {
                        (lane_len - segment + i - 1) as u64
                    } else {
                        (lane_len - segment) as u64 - if i == 0 { 1 } else { 0 }
                    };
                    let x = rand & 0xffff_ffff;
                    let y = (x * x) >> 32;
                    let rel = area - 1 - ((area * y) >> 32);
                    let start_pos = if pass != 0 && slice != SYNC_POINTS - 1 { ((slice + 1) * segment) as u64 } else { 0 };
                    let ref_index = ((start_pos + rel) % lane_len as u64) as u32;
                    let refb = mem[(ref_lane * lane_len + ref_index) as usize];
                    let pb = mem[prev as usize];
                    fill_block(&pb, &refb, &mut mem[cur as usize], pass != 0);
                    cur += 1;
                    prev += 1;
                }
            }
        }
    }
    // final block: XOR of the lanes' last blocks
    let mut c = mem[(lane_len - 1) as usize];
    for l in 1..lanes {
        let b = &mem[(l * lane_len + lane_len - 1) as usize];
        for (x, y) in c.iter_mut().zip(b) {
            *x ^= y;
        }
    }
    let cb = block_bytes(&c);
    blake2b::long(out, &[&cb]);
    for b in mem.iter_mut() {
        b.fill(0);
    }
    core::hint::black_box(&mem);
    true
}

/// a PHC string of the password with a caller-chosen random salt (>= 16 bytes recommended)
pub fn hash(password: &[u8], salt: &[u8], params: &Params) -> Option<String> {
    let mut out = vec![0u8; params.out_len as usize];
    if !argon2id(password, salt, params, &mut out) {
        return None;
    }
    Some(format!("$argon2id$v=19$m={},t={},p={}${}${}", params.m_kib, params.t, params.p, b64::encode_nopad(salt), b64::encode_nopad(&out)))
}

/// the parameters, salt and tag of a PHC string
pub fn parse(phc: &str) -> Option<(Params, Vec<u8>, Vec<u8>)> {
    let mut it = phc.split('$');
    if it.next()? != "" || it.next()? != "argon2id" || it.next()? != "v=19" {
        return None;
    }
    let (mut m, mut t, mut p) = (None, None, None);
    for kv in it.next()?.split(',') {
        let (k, v) = kv.split_once('=')?;
        let v: u32 = v.parse().ok()?;
        match k {
            "m" => m = Some(v),
            "t" => t = Some(v),
            "p" => p = Some(v),
            _ => return None,
        }
    }
    let salt = b64::decode(it.next()?)?;
    let tag = b64::decode(it.next()?)?;
    if it.next().is_some() {
        return None;
    }
    let params = Params { m_kib: m?, t: t?, p: p?, out_len: tag.len() as u32 };
    params.valid().then_some((params, salt, tag))
}

/// whether the password matches a PHC string
pub fn verify(phc: &str, password: &[u8]) -> bool {
    let Some((params, salt, tag)) = parse(phc) else { return false };
    let mut out = vec![0u8; tag.len()];
    argon2id(password, &salt, &params, &mut out) && crate::ct_eq(&out, &tag)
}
