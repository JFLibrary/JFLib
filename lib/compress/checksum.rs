// checksum -- CRC-32 (zlib / PNG polynomial) and Adler-32 (until 0.2 img_core::checksum)
//                                                                       [src/img/checksum.s]
//
//   crc32(crc, data) -> u32        start with crc = 0 (like zlib's crc32())
//   adler32(adler, data) -> u32    start with adler = 1
//
// CRC: table driven (slice-by-8 here, slice-by-4 in the assembly: the same values).
// Adler-32: blocks of at most 5552 bytes summed exactly in 64 bit, both sums reduced mod
// 65521 after every block (as the assembly's SSE2 loop); a call with no data returns the
// start value unchanged.  The 16-byte groups add into 16 column sums, which the
// compiler turns into vector code on every target (the crate depends on nothing, so the
// build tools with std can use it too); the sums are exact, so the value is the same as the
// assembly's psadbw / pmaddwd loop.

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
        let mut groups = blk.chunks_exact(16);
        // per byte position of a group: col = the sum of its bytes so far, run = the sum of
        // col before each group (the earlier groups' bytes, counted once per later group);
        // at most 347 groups per block keep both within u32
        let mut col = [0u32; 16];
        let mut run = [0u32; 16];
        for g in &mut groups {
            for i in 0..16 {
                run[i] += col[i];
                col[i] += g[i] as u32;
            }
        }
        let s1: u64 = col.iter().map(|&x| x as u64).sum();
        let s3: u64 = run.iter().map(|&x| x as u64).sum();
        let w: u64 = col.iter().zip(1..=16u64).map(|(&x, k)| x as u64 * (17 - k)).sum();
        // b += 16 * k * a + 16 * (sums of the earlier groups) + weighted sums, a += bytes
        let k = (blk.len() / 16) as u64;
        b += 16 * k * a + 16 * s3 + w;
        a += s1;
        for &x in groups.remainder() {
            a += x as u64;
            b += a;
        }
        a %= ADLER_MOD;
        b %= ADLER_MOD;
    }
    (b as u32) << 16 | a as u32
}
