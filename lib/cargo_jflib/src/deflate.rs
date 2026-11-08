// Deflate (RFC 1951) both ways, in the zlib (RFC 1950) and gzip (RFC 1952) frames: the setup's
// payload, the PNGs of previews and icons, the archives of a .deb.  The compressor finds matches
// along hash chains with one step of lazy matching and writes each block with its own Huffman
// codes, or stored when that is smaller; it does not reach zlib's level 9 but is near it.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use crate::zip::crc32;

const LEN_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
                              8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

const WINDOW: usize = 32768;
const HASH_BITS: u32 = 15;
const MAX_CHAIN: usize = 256;
const GOOD_LEN: usize = 32; // a match this long is taken without looking one byte further
const BLOCK_TOKENS: usize = 1 << 16;

// ---------------------------------------------------------------- compressing

struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn put(&mut self, v: u32, n: u32) {
        self.acc |= (v as u64) << self.n;
        self.n += n;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    fn align(&mut self) {
        if self.n > 0 {
            self.out.push(self.acc as u8);
            self.acc = 0;
            self.n = 0;
        }
    }
}

fn len_code(len: usize) -> usize {
    (0..29).rev().find(|&i| len >= LEN_BASE[i] as usize).unwrap_or(0)
}

fn dist_code(dist: usize) -> usize {
    (0..30).rev().find(|&i| dist >= DIST_BASE[i] as usize).unwrap_or(0)
}

/// Huffman code lengths of at most `limit` bits for these frequencies (two codes at least, as
/// zlib keeps them: some decoders refuse a code of one symbol)
fn lengths(freq: &[u32], limit: usize) -> Vec<u8> {
    let mut freq = freq.to_vec();
    for s in 0..freq.len() {
        if freq.iter().filter(|&&f| f > 0).count() >= 2 {
            break;
        }
        if freq[s] == 0 {
            freq[s] = 1;
        }
    }
    let mut syms: Vec<usize> = (0..freq.len()).filter(|&s| freq[s] > 0).collect();
    let m = syms.len();
    let mut parent = vec![0usize; 2 * m];
    let mut heap: BinaryHeap<Reverse<(u64, usize)>> = syms.iter().enumerate().map(|(k, &s)| Reverse((freq[s] as u64, k))).collect();
    let mut next = m;
    while heap.len() > 1 {
        let Reverse((f1, a)) = heap.pop().unwrap_or(Reverse((0, 0)));
        let Reverse((f2, b)) = heap.pop().unwrap_or(Reverse((0, 0)));
        parent[a] = next;
        parent[b] = next;
        heap.push(Reverse((f1 + f2, next)));
        next += 1;
    }
    let mut depth = vec![0usize; next];
    for node in (0..next - 1).rev() {
        depth[node] = depth[parent[node]] + 1;
    }
    // at most `limit` bits: miniz's way, longer codes moved up until the Kraft sum is exact
    let mut num = vec![0u64; limit + 1];
    for &d in &depth[..m] {
        num[d.min(limit)] += 1;
    }
    let mut total: u64 = (1..=limit).map(|i| num[i] << (limit - i)).sum();
    while total != 1 << limit {
        num[limit] -= 1;
        if let Some(i) = (1..limit).rev().find(|&i| num[i] > 0) {
            num[i] -= 1;
            num[i + 1] += 2;
        }
        total -= 1;
    }
    // the most frequent symbols get the shortest codes
    syms.sort_by_key(|&s| (freq[s], s));
    let mut out = vec![0u8; freq.len()];
    let mut j = m;
    for (len, &count) in num.iter().enumerate().skip(1) {
        for _ in 0..count {
            j -= 1;
            out[syms[j]] = len as u8;
        }
    }
    out
}

/// the canonical codes of these lengths, bit-reversed for the LSB-first writer
fn codes(lens: &[u8]) -> Vec<u16> {
    let mut count = [0u16; 16];
    for &l in lens {
        count[l as usize] += 1;
    }
    count[0] = 0;
    let mut next = [0u16; 16];
    for l in 1..16 {
        next[l] = (next[l - 1] + count[l - 1]) << 1;
    }
    lens.iter()
        .map(|&l| {
            if l == 0 {
                return 0;
            }
            let c = next[l as usize];
            next[l as usize] += 1;
            c.reverse_bits() >> (16 - l)
        })
        .collect()
}

/// code lengths as the symbols 0..18 (16: repeat the last 3-6 times, 17 / 18: 3-10 / 11-138 zeros)
fn rle(lens: &[u8]) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lens.len() {
        let l = lens[i];
        let run = lens[i..].iter().take_while(|&&x| x == l).count();
        let mut r = run;
        if l == 0 {
            while r >= 11 {
                let k = r.min(138);
                out.push((18, (k - 11) as u8));
                r -= k;
            }
            if r >= 3 {
                out.push((17, (r - 3) as u8));
                r = 0;
            }
        } else {
            out.push((l, 0));
            r -= 1;
            while r >= 3 {
                let k = r.min(6);
                out.push((16, (k - 3) as u8));
                r -= k;
            }
        }
        out.extend(std::iter::repeat_n((l, 0), r));
        i += run;
    }
    out
}

/// one token: a literal (dist 0) or a match
#[derive(Clone, Copy)]
struct Token {
    len: u16,
    dist: u16,
}

fn stored(bits: &mut Bits, data: &[u8], last: bool) {
    let mut chunks: Vec<&[u8]> = data.chunks(65535).collect();
    if chunks.is_empty() {
        chunks.push(&[]);
    }
    let n = chunks.len();
    for (k, c) in chunks.into_iter().enumerate() {
        bits.put((last && k + 1 == n) as u32, 1);
        bits.put(0, 2);
        bits.align();
        bits.out.extend_from_slice(&(c.len() as u16).to_le_bytes());
        bits.out.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        bits.out.extend_from_slice(c);
    }
}

fn block(bits: &mut Bits, data: &[u8], tokens: &[Token], last: bool) {
    let mut fl = [0u32; 286];
    let mut fd = [0u32; 30];
    fl[256] = 1;
    for t in tokens {
        if t.dist == 0 {
            fl[t.len as usize] += 1;
        } else {
            fl[257 + len_code(t.len as usize)] += 1;
            fd[dist_code(t.dist as usize)] += 1;
        }
    }
    let ll = lengths(&fl, 15);
    let dl = lengths(&fd, 15);
    let hlit = (257..=286).rev().find(|&n| ll[n - 1] != 0).unwrap_or(257);
    let hdist = (1..=30).rev().find(|&n| dl[n - 1] != 0).unwrap_or(1);
    let all: Vec<u8> = ll[..hlit].iter().chain(&dl[..hdist]).copied().collect();
    let runs = rle(&all);
    let mut fc = [0u32; 19];
    for &(s, _) in &runs {
        fc[s as usize] += 1;
    }
    let cl = lengths(&fc, 7);
    let hclen = (4..=19).rev().find(|&n| cl[CL_ORDER[n - 1]] != 0).unwrap_or(4);
    let extra = |s: u8| match s {
        16 => 2,
        17 => 3,
        18 => 7,
        _ => 0,
    };
    let mut size = 3 + 14 + 3 * hclen as u64 + ll[256] as u64;
    size += runs.iter().map(|&(s, _)| cl[s as usize] as u64 + extra(s)).sum::<u64>();
    for t in tokens {
        size += if t.dist == 0 {
            ll[t.len as usize] as u64
        } else {
            let (lc, dc) = (len_code(t.len as usize), dist_code(t.dist as usize));
            (ll[257 + lc] + LEN_EXTRA[lc] + dl[dc] + DIST_EXTRA[dc]) as u64
        };
    }
    if (data.len() as u64 + 5 * (data.len() as u64 / 65535 + 1)) * 8 + 7 < size {
        return stored(bits, data, last);
    }
    let (lc_, dc_, cc) = (codes(&ll), codes(&dl), codes(&cl));
    bits.put(last as u32, 1);
    bits.put(2, 2);
    bits.put((hlit - 257) as u32, 5);
    bits.put((hdist - 1) as u32, 5);
    bits.put((hclen - 4) as u32, 4);
    for &k in &CL_ORDER[..hclen] {
        bits.put(cl[k] as u32, 3);
    }
    for &(s, e) in &runs {
        bits.put(cc[s as usize] as u32, cl[s as usize] as u32);
        bits.put(e as u32, extra(s) as u32);
    }
    for t in tokens {
        if t.dist == 0 {
            bits.put(lc_[t.len as usize] as u32, ll[t.len as usize] as u32);
        } else {
            let (lc, dc) = (len_code(t.len as usize), dist_code(t.dist as usize));
            bits.put(lc_[257 + lc] as u32, ll[257 + lc] as u32);
            bits.put((t.len - LEN_BASE[lc]) as u32, LEN_EXTRA[lc] as u32);
            bits.put(dc_[dc] as u32, dl[dc] as u32);
            bits.put((t.dist - DIST_BASE[dc]) as u32, DIST_EXTRA[dc] as u32);
        }
    }
    bits.put(lc_[256] as u32, ll[256] as u32);
}

/// raw deflate of `data`
pub(crate) fn deflate(data: &[u8]) -> Vec<u8> {
    let n = data.len();
    let mut bits = Bits { out: Vec::with_capacity(n / 2 + 64), acc: 0, n: 0 };
    let mut head = vec![u32::MAX; 1 << HASH_BITS];
    let mut prev = vec![u32::MAX; WINDOW];
    let hash = |i: usize| -> usize {
        let v = (data[i] as u32) << 16 | (data[i + 1] as u32) << 8 | data[i + 2] as u32;
        (v.wrapping_mul(2_654_435_761) >> (32 - HASH_BITS)) as usize
    };
    // the longest match at i along its chain (i itself not inserted yet)
    let find = |i: usize, head: &[u32], prev: &[u32]| -> (usize, usize) {
        if i + 3 > n {
            return (0, 0);
        }
        let max = (n - i).min(258);
        let (mut best, mut dist) = (0, 0);
        let mut cand = head[hash(i)];
        let mut chain = MAX_CHAIN;
        while cand != u32::MAX && chain > 0 {
            let c = cand as usize;
            if c >= i || i - c > WINDOW {
                break;
            }
            if data[c + best.min(max - 1)] == data[i + best.min(max - 1)] {
                let len = data[c..c + max].iter().zip(&data[i..i + max]).take_while(|(a, b)| a == b).count();
                if len > best {
                    best = len;
                    dist = i - c;
                    if len == max {
                        break;
                    }
                }
            }
            let p = prev[c % WINDOW];
            if p == u32::MAX || p as usize >= c {
                break;
            }
            cand = p;
            chain -= 1;
        }
        if best >= 3 { (best, dist) } else { (0, 0) }
    };
    let insert = |i: usize, head: &mut [u32], prev: &mut [u32]| {
        if i + 3 <= n {
            let h = hash(i);
            prev[i % WINDOW] = head[h];
            head[h] = i as u32;
        }
    };
    let mut tokens = Vec::with_capacity(BLOCK_TOKENS);
    let mut start = 0;
    let mut i = 0;
    while i < n {
        let (len, dist) = find(i, &head, &prev);
        insert(i, &mut head, &mut prev);
        let later = if (3..GOOD_LEN).contains(&len) { find(i + 1, &head, &prev).0 } else { 0 };
        if len >= 3 && later <= len {
            tokens.push(Token { len: len as u16, dist: dist as u16 });
            for k in i + 1..i + len {
                insert(k, &mut head, &mut prev);
            }
            i += len;
        } else {
            tokens.push(Token { len: data[i] as u16, dist: 0 });
            i += 1;
        }
        if tokens.len() >= BLOCK_TOKENS {
            block(&mut bits, &data[start..i], &tokens, false);
            tokens.clear();
            start = i;
        }
    }
    block(&mut bits, &data[start..], &tokens, true);
    bits.align();
    bits.out
}

pub(crate) fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for c in data.chunks(5552) {
        for &x in c {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

/// `data` in the zlib frame (as zlib.compress)
pub(crate) fn zlib(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0xDA];
    out.extend(deflate(data));
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// `data` as a .gz file (no name, time 0)
pub(crate) fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 2, 3];
    out.extend(deflate(data));
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

// ---------------------------------------------------------------- decompressing

struct Input<'a> {
    src: &'a [u8],
    pos: usize,
    acc: u32,
    n: u32,
}

impl Input<'_> {
    fn bits(&mut self, need: u32) -> Result<u32, String> {
        while self.n < need {
            let b = *self.src.get(self.pos).ok_or("deflate: the data ends early")?;
            self.pos += 1;
            self.acc |= (b as u32) << self.n;
            self.n += 8;
        }
        let v = self.acc & ((1u64 << need) - 1) as u32;
        self.acc = if need == 32 { 0 } else { self.acc >> need };
        self.n -= need;
        Ok(v)
    }
}

/// a canonical Huffman code for decoding (puff's form: counts per length, symbols in order)
struct Huff {
    count: [u16; 16],
    symbol: Vec<u16>,
}

impl Huff {
    fn new(lens: &[u8]) -> Huff {
        let mut count = [0u16; 16];
        for &l in lens {
            count[l as usize] += 1;
        }
        let mut offs = [0u16; 16];
        for l in 1..15 {
            offs[l + 1] = offs[l] + count[l];
        }
        let mut symbol = vec![0u16; lens.len()];
        for (s, &l) in lens.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Huff { count, symbol }
    }

    fn decode(&self, inp: &mut Input) -> Result<usize, String> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= inp.bits(1)? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return Ok(self.symbol[(index + code - first) as usize] as usize);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("deflate: a code that is not there".to_string())
    }
}

/// raw deflate data -> the bytes (at most `limit`)
pub(crate) fn inflate(src: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    let mut inp = Input { src, pos: 0, acc: 0, n: 0 };
    let mut out: Vec<u8> = Vec::new();
    let too_big = || format!("deflate: more than {limit} bytes");
    loop {
        let last = inp.bits(1)?;
        match inp.bits(2)? {
            0 => {
                inp.acc = 0;
                inp.n = 0;
                let p = inp.pos;
                let head = src.get(p..p + 4).ok_or("deflate: a stored block ends early")?;
                let len = u16::from_le_bytes([head[0], head[1]]) as usize;
                if u16::from_le_bytes([head[2], head[3]]) != !(len as u16) {
                    return Err("deflate: a stored block with a bad length".to_string());
                }
                let data = src.get(p + 4..p + 4 + len).ok_or("deflate: a stored block ends early")?;
                if out.len() + len > limit {
                    return Err(too_big());
                }
                out.extend_from_slice(data);
                inp.pos = p + 4 + len;
            }
            t @ (1 | 2) => {
                let (lit, dist) = if t == 1 {
                    let mut l = [8u8; 288];
                    l[144..256].fill(9);
                    l[256..280].fill(7);
                    (Huff::new(&l), Huff::new(&[5u8; 30]))
                } else {
                    let hlit = inp.bits(5)? as usize + 257;
                    let hdist = inp.bits(5)? as usize + 1;
                    let hclen = inp.bits(4)? as usize + 4;
                    let mut cl = [0u8; 19];
                    for &k in &CL_ORDER[..hclen] {
                        cl[k] = inp.bits(3)? as u8;
                    }
                    let clh = Huff::new(&cl);
                    let mut lens = Vec::with_capacity(hlit + hdist);
                    while lens.len() < hlit + hdist {
                        let s = clh.decode(&mut inp)?;
                        let (v, r) = match s {
                            0..=15 => (s as u8, 1),
                            16 => (*lens.last().ok_or("deflate: a repeat with nothing before it")?, 3 + inp.bits(2)? as usize),
                            17 => (0, 3 + inp.bits(3)? as usize),
                            _ => (0, 11 + inp.bits(7)? as usize),
                        };
                        lens.extend(std::iter::repeat_n(v, r));
                    }
                    if lens.len() > hlit + hdist {
                        return Err("deflate: too many code lengths".to_string());
                    }
                    (Huff::new(&lens[..hlit]), Huff::new(&lens[hlit..]))
                };
                loop {
                    let s = lit.decode(&mut inp)?;
                    if s < 256 {
                        if out.len() >= limit {
                            return Err(too_big());
                        }
                        out.push(s as u8);
                    } else if s == 256 {
                        break;
                    } else {
                        let lc = s - 257;
                        if lc >= 29 {
                            return Err("deflate: a bad length code".to_string());
                        }
                        let len = LEN_BASE[lc] as usize + inp.bits(LEN_EXTRA[lc] as u32)? as usize;
                        let dc = dist.decode(&mut inp)?;
                        if dc >= 30 {
                            return Err("deflate: a bad distance code".to_string());
                        }
                        let d = DIST_BASE[dc] as usize + inp.bits(DIST_EXTRA[dc] as u32)? as usize;
                        if d > out.len() {
                            return Err("deflate: a distance before the start".to_string());
                        }
                        if out.len() + len > limit {
                            return Err(too_big());
                        }
                        let from = out.len() - d;
                        for k in 0..len {
                            out.push(out[from + k]);
                        }
                    }
                }
            }
            _ => return Err("deflate: a block of type 3".to_string()),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

/// zlib-framed data -> the bytes (at most `limit`), the checksum checked
pub(crate) fn unzlib(src: &[u8], limit: usize) -> Result<Vec<u8>, String> {
    if src.len() < 6 || src[0] & 0x0F != 8 || !(src[0] as u16 * 256 + src[1] as u16).is_multiple_of(31) || src[1] & 0x20 != 0 {
        return Err("no zlib data".to_string());
    }
    let out = inflate(&src[2..], limit)?;
    // the checksum follows the deflate data, which may end before the last bytes
    let sum = u32::from_be_bytes(src[src.len() - 4..].try_into().unwrap_or_default());
    if adler32(&out) != sum {
        return Err("zlib: the checksum differs".to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        let mut v = Vec::new();
        let mut x = 12345u32;
        for k in 0..200_000u32 {
            x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
            v.push(if k % 7 < 4 { b"JFLib packs this text "[(k % 22) as usize] } else { (x >> 24) as u8 });
        }
        v
    }

    #[test]
    fn round_trip() {
        for data in [Vec::new(), b"a".to_vec(), b"abcabcabcabcabcabc".to_vec(), vec![0u8; 100_000], sample()] {
            let z = zlib(&data);
            assert_eq!(unzlib(&z, data.len()).unwrap(), data);
        }
        let s = sample();
        assert!(zlib(&s).len() < s.len() * 3 / 4);
    }

    #[test]
    fn zlibs_own_output() {
        // zlib.compress(b"hello hello hello hello, JFLib!", 9)
        let z = [0x78, 0xDA, 0xCB, 0x48, 0xCD, 0xC9, 0xC9, 0x57, 0xC8, 0x40, 0x27, 0x75, 0x14, 0xBC, 0xDC, 0x7C, 0x32, 0x93, 0x14, 0x01,
                 0xB6, 0x35, 0x0A, 0xC5];
        assert_eq!(unzlib(&z, 100).unwrap(), b"hello hello hello hello, JFLib!");
    }

    #[test]
    fn lengths_stay_within_the_limit() {
        let freq: Vec<u32> = (0..40).map(|k| 1u32 << (k % 30)).collect();
        let l = lengths(&freq, 7);
        assert!(l.iter().all(|&x| x > 0 && x <= 7));
        let kraft: f64 = l.iter().map(|&x| 0.5f64.powi(x as i32)).sum();
        assert!((kraft - 1.0).abs() < 1e-9);
    }
}
