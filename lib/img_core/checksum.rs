// checksum -- CRC-32 (zlib / PNG polynomial) and Adler-32  [src/img/checksum.s]
//
//   crc32(crc, data) -> u32        start with crc = 0 (like zlib's crc32())
//   adler32(adler, data) -> u32    start with adler = 1
//
// CRC: table driven (slice-by-8 here, slice-by-4 in the assembly: the same values).
// Adler-32: blocks of at most 5552 bytes summed exactly in 64 bit, both sums reduced mod
// 65521 after every block (as the assembly's SSE2 loop); a call with no data returns the
// start value unchanged.

use simd::{Kernel, Simd};

const CRC_POLY: u32 = 0xEDB8_8320;
const ADLER_MOD: u64 = 65521;
const ADLER_NMAX: usize = 5552;

// t[k][n] = (t[k-1][n] >> 8) ^ t[0][t[k-1][n] & 255]
static CRC_TAB: [[u32; 256]; 8] = {
    let mut t = [[0u32; 256]; 8];
    let mut n = 0;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { CRC_POLY ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[0][n] = c;
        n += 1;
    }
    let mut k = 1;
    while k < 8 {
        let mut n = 0;
        while n < 256 {
            let p = t[k - 1][n];
            t[k][n] = (p >> 8) ^ t[0][(p & 255) as usize];
            n += 1;
        }
        k += 1;
    }
    t
};

/// CRC-32 (zlib / PNG) of data, continuing crc (start with 0)  [crc32]
pub fn crc32(crc: u32, data: &[u8]) -> u32 {
    let t = &CRC_TAB;
    let mut c = !crc;
    let mut chunks = data.chunks_exact(8);
    for b in &mut chunks {
        let lo = c ^ u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let hi = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
        c = t[7][(lo & 255) as usize]
            ^ t[6][((lo >> 8) & 255) as usize]
            ^ t[5][((lo >> 16) & 255) as usize]
            ^ t[4][(lo >> 24) as usize]
            ^ t[3][(hi & 255) as usize]
            ^ t[2][((hi >> 8) & 255) as usize]
            ^ t[1][((hi >> 16) & 255) as usize]
            ^ t[0][(hi >> 24) as usize];
    }
    for &b in chunks.remainder() {
        c = t[0][((c ^ b as u32) & 255) as usize] ^ (c >> 8);
    }
    !c
}

/// Adler-32 (zlib) of data, continuing adler (start with 1)  [adler32]
pub fn adler32(adler: u32, data: &[u8]) -> u32 {
    let mut a = (adler & 0xFFFF) as u64;
    let mut b = (adler >> 16) as u64;
    for blk in data.chunks(ADLER_NMAX) {
        let nv = blk.len() / 16;
        let (s1, s3, w) = simd::dispatch_sse2(AdlerK { v: &blk[..nv * 16] });
        // b += 16 * k * a + 16 * (prefix sums) + weighted sums, a += bytes (exact)
        b += 16 * nv as u64 * a + 16 * s3 + w;
        a += s1;
        for &x in &blk[nv * 16..] {
            a += x as u64;
            b += a;
        }
        a %= ADLER_MOD;
        b %= ADLER_MOD;
    }
    (b as u32) << 16 | a as u32
}

// the 16-byte vectors of one block: (byte sum, sum of the running byte sums before each
// vector, weighted sums 16 * x0 + 15 * x1 + ... + x15) -- the assembly's psadbw / pmaddwd
// loop (exact integer sums: the grouping into lanes changes no total)
struct AdlerK<'a> {
    v: &'a [u8],
}

static W_LO: [i16; 8] = [16, 15, 14, 13, 12, 11, 10, 9];
static W_HI: [i16; 8] = [8, 7, 6, 5, 4, 3, 2, 1];

impl Kernel for AdlerK<'_> {
    type Out = (u64, u64, u64);
    #[inline(always)]
    fn run<S: Simd>(self, s: S) -> (u64, u64, u64) {
        // (run through dispatch_sse2: 16-byte vectors)
        let z = s.zero();
        let wl = s.load_i16(&W_LO);
        let wh = s.load_i16(&W_HI);
        let (mut s1, mut s3, mut w) = (z, z, z);
        for x in self.v.chunks_exact(16) {
            let v = s.load(x);
            s3 = s.add32(s3, s1);
            s1 = s.add32(s1, s.sadu8(v, z)); // the byte sums of each half in its low lane
            w = s.add32(w, s.add32(s.madd16(s.unpacklo8(v, z), wl), s.madd16(s.unpackhi8(v, z), wh)));
        }
        (hsum32(s, s1), hsum32(s, s3), hsum32(s, w))
    }
}

// the sum of the u32 lanes of an SSE2 register
#[inline(always)]
fn hsum32<S: Simd>(s: S, v: S::V) -> u64 {
    let mut t = [0i32; 4];
    s.store_i32(&mut t, v);
    t.iter().map(|&x| x as u32 as u64).sum()
}
