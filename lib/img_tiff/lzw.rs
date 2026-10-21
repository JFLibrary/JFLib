// lzw -- TIFF LZW decoding                                      [src/img/tiff.s: tc_lzw]
//
//   decode(src, pb, cap, tab) -> bytes written (up to cap) / ERR_CODE
//
// pb: a plane buffer, the literal area pb[..LIT] (bytes 0..255) in front of the data
// pb[LIT..LIT + cap], slack behind it.  tab: per code its string as offset in pb | length
// << 32, the literals preset (code c: offset c, length 1).  Streams starting with 00 and an
// odd second byte are libtiff's old bit-reversed variant (LZWDecodeCompat: LSB-first
// codes, width change at 2^n).  Decoding ends at EOI, at the end of the input or when the
// output is full.  Strings are copied from earlier output 16 bytes at a time like the
// assembly (a string always ends before the write position; the overcopy lands in bytes
// written later or in the slack).

use img_core::ERR_CODE;

pub(crate) const LIT: usize = 256;
const LEN1: u64 = 1 << 32; // length 1 in a table entry

/// the code table: offset in the plane buffer | length << 32
pub(crate) type Table = [u64; 4096];

/// a table with the literals preset
pub(crate) fn table() -> alloc::boxed::Box<Table> {
    let mut t = alloc::boxed::Box::new([0u64; 4096]);
    for (c, e) in t.iter_mut().enumerate().take(256) {
        *e = c as u64 | LEN1; // literal c: offset c in the literal area
    }
    t
}

/// decode a TIFF LZW strip / tile  [tc_lzw]
pub(crate) fn decode(src: &[u8], pb: &mut [u8], cap: usize, tab: &mut Table) -> i64 {
    let compat = src.len() >= 2 && src[0] == 0 && src[1] & 1 != 0;
    let n = if compat { run::<true>(src, pb, LIT + cap, tab) } else { run::<false>(src, pb, LIT + cap, tab) };
    if n < 0 { n } else { n - LIT as i64 }
}

// n bytes from pb[s..] to pb[d..], 16 at a time (load, then store)
#[inline(always)]
fn copy16(pb: &mut [u8], s: usize, d: usize, n: usize) {
    let t: [u8; 16] = pb[s..s + 16].try_into().unwrap();
    pb[d..d + 16].copy_from_slice(&t);
    let mut k = 16;
    while k < n {
        let t: [u8; 16] = pb[s + k..s + k + 16].try_into().unwrap();
        pb[d + k..d + k + 16].copy_from_slice(&t);
        k += 16;
    }
}

// the decoding loop (positions are absolute in pb: the data starts at LIT, dend = LIT +
// capacity); the bit reader is img_int.inc's BRM_* (MSB first) / BRL_* (LSB first): a
// 64-bit buffer refilled to >= 56 bits, zero bytes past the end, an overrun = more bits
// consumed than the data has (ends the stream).  The previous string is kept as one table
// entry `ps` (position | length << 32; 0 = none after a clear).  Returns the end position or
// ERR_CODE.
#[inline(never)]
fn run<const COMPAT: bool>(src: &[u8], pb: &mut [u8], dend: usize, tab: &mut Table) -> i64 {
    // every access lies below dend + 16 (a copy of m <= dend - d bytes from s + m <= d writes
    // and reads at most 15 bytes more); the length tied to dend frees a register
    let pb = &mut pb[..dend + 16];
    let end = src.len();
    let (mut buf, mut cnt, mut ptr) = (0u64, 0u64, 0usize);
    let mut d = LIT; // write position
    let mut next = 258usize;
    let mut width = 9u64;
    let mut ps = 0u64;
    // after adding a code: one bit more once next reaches the threshold lim (new style
    // 2^n - 1, libtiff's early change; old style 2^n), 12 max  [LZW_WIDTH]
    let first_lim = (1usize << 9) - (!COMPAT as usize);
    let mut lim = first_lim;
    macro_rules! bump {
        () => {
            if next >= lim {
                width += 1;
                lim = if width < 12 { (1 << width) - (!COMPAT as usize) } else { usize::MAX };
            }
        };
    }
    loop {
        // refill
        if cnt < 56 {
            if end >= ptr + 8 {
                let w: [u8; 8] = src[ptr..ptr + 8].try_into().unwrap();
                if COMPAT {
                    buf |= u64::from_le_bytes(w) << cnt;
                } else {
                    buf |= u64::from_be_bytes(w) >> cnt;
                }
                ptr += ((63 - cnt) >> 3) as usize;
                cnt |= 56;
            } else {
                while cnt <= 56 {
                    let t = if ptr < end { src[ptr] as u64 } else { 0 };
                    ptr += 1;
                    if COMPAT {
                        buf |= t << cnt;
                    } else {
                        buf |= t << (56 - cnt);
                    }
                    cnt += 8;
                }
            }
        }
        let code = if COMPAT {
            let v = buf & ((1 << width) - 1);
            buf >>= width;
            v
        } else {
            let v = buf >> (64 - width);
            buf <<= width;
            v
        } as usize
            & 4095;
        cnt -= width;
        if ptr > end && ((ptr - end) as u64) * 8 > cnt {
            break; // overrun: the data ended
        }
        // the rare cases (clear, EOI, the first code after a clear) behind one predictable
        // branch (`code < 256` alone is not predictable)
        if (code.wrapping_sub(256) < 2) | (ps == 0) {
            if code == 256 {
                next = 258;
                width = 9;
                lim = first_lim;
                ps = 0;
                continue;
            }
            if code == 257 {
                break;
            }
            if code > 255 {
                return ERR_CODE as i64; // first code after a clear: a literal
            }
            pb[d] = code as u8;
            ps = d as u64 | LEN1;
            d += 1;
            if d >= dend {
                break;
            }
            continue;
        }
        if code >= next {
            if code > next {
                return ERR_CODE as i64;
            }
            // code == next code: previous string + its own first byte
            tab[next & 4095] = ps + LEN1;
            next += 1;
            bump!();
            let s = ps as u32 as usize;
            let len = (ps >> 32) as usize;
            ps = d as u64 | (ps & !0xFFFF_FFFF) + LEN1;
            let room = dend - d;
            if len >= room {
                copy16(pb, s, d, room);
                d = dend;
                break;
            }
            let first = pb[s];
            copy16(pb, s, d, len);
            d += len + 1;
            pb[d - 1] = first;
            if d < dend {
                continue;
            }
            break;
        }
        // known code: its string
        let e = tab[code];
        if next < 4096 {
            tab[next] = ps + LEN1; // new code = previous string + first byte
            next += 1;
            bump!();
        }
        ps = d as u64 | (e & !0xFFFF_FFFF);
        let m = ((e >> 32) as usize).min(dend - d);
        copy16(pb, e as u32 as usize, d, m);
        d += m;
        if d >= dend {
            break;
        }
    }
    d as i64
}
