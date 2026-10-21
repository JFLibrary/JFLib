// lz4 -- the LZ4 block and frame formats
//
//   compress(data) -> block                greedy matcher, 16K-entry hash table, skip
//                                          acceleration on incompressible data
//   decompress(block, max_out)             None on truncation, a bad offset, or more than
//                                          max_out bytes; never reads or writes out of bounds
//   decompress_into(block, &mut out, limit) appends (back references may reach into the
//                                          earlier contents of out: linked frame blocks)
//   frame_compress(data) / frame_decompress(frame, max_out)
//
// Block rules kept by the compressor: the last 5 bytes are literals, no match starts in the
// last 12 bytes, offsets 1..=65535.

use alloc::vec;
use alloc::vec::Vec;

use crate::xxh32;

const MIN_MATCH: usize = 4;
const LAST_LITERALS: usize = 5;
const MF_LIMIT: usize = 12;
const HASH_LOG: u32 = 14;
const MAX_DIST: usize = 65535;

/// the largest block compress() can produce for n input bytes
pub const fn bound(n: usize) -> usize {
    n + n / 255 + 16
}

#[inline(always)]
fn rd32(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

#[inline(always)]
fn hash(v: u32) -> usize {
    (v.wrapping_mul(2_654_435_761) >> (32 - HASH_LOG)) as usize
}

fn put_len(out: &mut Vec<u8>, mut n: usize) {
    while n >= 255 {
        out.push(255);
        n -= 255;
    }
    out.push(n as u8);
}

fn sequence(out: &mut Vec<u8>, lits: &[u8], offset: usize, mlen: usize) {
    let ll = lits.len();
    let ml = mlen - MIN_MATCH;
    out.push(((ll.min(15) as u8) << 4) | ml.min(15) as u8);
    if ll >= 15 {
        put_len(out, ll - 15);
    }
    out.extend_from_slice(lits);
    out.extend_from_slice(&(offset as u16).to_le_bytes());
    if ml >= 15 {
        put_len(out, ml - 15);
    }
}

fn last(out: &mut Vec<u8>, lits: &[u8]) {
    let ll = lits.len();
    out.push((ll.min(15) as u8) << 4);
    if ll >= 15 {
        put_len(out, ll - 15);
    }
    out.extend_from_slice(lits);
}

/// compress into one LZ4 block
pub fn compress(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bound(src.len()));
    let n = src.len();
    if n <= MF_LIMIT {
        last(&mut out, src);
        return out;
    }
    let mut table = vec![0u32; 1 << HASH_LOG];
    let mflimit = n - MF_LIMIT; // last position a match may start at
    let match_end = n - LAST_LITERALS; // matches end before this
    let mut anchor = 0;
    let mut ip = 1;
    let mut misses = 0usize;
    while ip <= mflimit {
        let seq = rd32(src, ip);
        let h = hash(seq);
        let cand = table[h] as usize;
        table[h] = ip as u32;
        if cand < ip && ip - cand <= MAX_DIST && rd32(src, cand) == seq {
            let (mut s, mut c) = (ip, cand);
            while s > anchor && c > 0 && src[s - 1] == src[c - 1] {
                s -= 1;
                c -= 1;
            }
            let mut len = MIN_MATCH + (ip - s);
            while s + len < match_end && src[c + len] == src[s + len] {
                len += 1;
            }
            sequence(&mut out, &src[anchor..s], s - c, len);
            ip = s + len;
            anchor = ip;
            misses = 0;
            // remember a position inside the match (better ratio on repetitive data)
            if ip >= 2 && ip - 2 <= mflimit {
                table[hash(rd32(src, ip - 2))] = (ip - 2) as u32;
            }
        } else {
            misses += 1;
            ip += 1 + (misses >> 6);
        }
    }
    last(&mut out, &src[anchor..]);
    out
}

fn get_len(src: &[u8], i: &mut usize, mut n: usize) -> Option<usize> {
    loop {
        let b = *src.get(*i)?;
        *i += 1;
        n = n.checked_add(b as usize)?;
        if b != 255 {
            return Some(n);
        }
    }
}

/// decode one block, appending to out (out.len() stays <= limit) -> false on corrupt input
pub fn decompress_into(src: &[u8], out: &mut Vec<u8>, limit: usize) -> bool {
    let mut i = 0;
    loop {
        let Some(&token) = src.get(i) else { return false };
        i += 1;
        let mut ll = (token >> 4) as usize;
        if ll == 15 {
            match get_len(src, &mut i, ll) {
                Some(v) => ll = v,
                None => return false,
            }
        }
        if ll > src.len() - i || ll > limit.saturating_sub(out.len()) {
            return false;
        }
        out.extend_from_slice(&src[i..i + ll]);
        i += ll;
        if i == src.len() {
            return true; // the last sequence has no match
        }
        if src.len() - i < 2 {
            return false;
        }
        let off = u16::from_le_bytes([src[i], src[i + 1]]) as usize;
        i += 2;
        if off == 0 || off > out.len() {
            return false;
        }
        let mut ml = (token & 15) as usize;
        if ml == 15 {
            match get_len(src, &mut i, ml) {
                Some(v) => ml = v,
                None => return false,
            }
        }
        ml += MIN_MATCH;
        if ml > limit.saturating_sub(out.len()) {
            return false;
        }
        // copy with overlap: the source repeats with period `off`
        let start = out.len() - off;
        let mut rem = ml;
        while rem > 0 {
            let k = rem.min(out.len() - start);
            out.extend_from_within(start..start + k);
            rem -= k;
        }
    }
}

/// decode one block of at most max_out bytes
pub fn decompress(src: &[u8], max_out: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(max_out.min(src.len().saturating_mul(4)).min(64 << 20));
    decompress_into(src, &mut out, max_out).then_some(out)
}

// ---------------------------------------------------------------------------- frames

const MAGIC: u32 = 0x184D_2204;
const BLOCK_MAX_ID: u8 = 7; // 4 MiB blocks
const BLOCK_MAX: usize = 4 << 20;

fn block_max(id: u8) -> Option<usize> {
    match id {
        4 => Some(64 << 10),
        5 => Some(256 << 10),
        6 => Some(1 << 20),
        7 => Some(4 << 20),
        _ => None,
    }
}

/// an LZ4 frame: independent 4 MiB blocks, content size and content checksum
pub fn frame_compress(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bound(src.len()) + 32);
    out.extend_from_slice(&MAGIC.to_le_bytes());
    // FLG: version 01, independent blocks, content size, content checksum
    let flg = 0b0110_1100u8;
    let bd = BLOCK_MAX_ID << 4;
    let desc_start = out.len();
    out.push(flg);
    out.push(bd);
    out.extend_from_slice(&(src.len() as u64).to_le_bytes());
    let hc = (xxh32(&out[desc_start..], 0) >> 8) as u8;
    out.push(hc);
    for chunk in src.chunks(BLOCK_MAX) {
        let c = compress(chunk);
        if c.len() < chunk.len() {
            out.extend_from_slice(&(c.len() as u32).to_le_bytes());
            out.extend_from_slice(&c);
        } else {
            out.extend_from_slice(&(chunk.len() as u32 | 0x8000_0000).to_le_bytes());
            out.extend_from_slice(chunk);
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&xxh32(src, 0).to_le_bytes());
    out
}

fn rd_u32(b: &[u8], i: &mut usize) -> Option<u32> {
    let v = b.get(*i..*i + 4)?;
    *i += 4;
    Some(u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
}

/// decode one or more concatenated LZ4 frames (skippable frames are skipped) of at most
/// max_out bytes in total; None on corruption, an unsupported feature (dictionary ids), a
/// checksum mismatch, or too much output
pub fn frame_decompress(src: &[u8], max_out: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let magic = rd_u32(src, &mut i)?;
        if magic & 0xFFFF_FFF0 == 0x184D_2A50 {
            let n = rd_u32(src, &mut i)? as usize;
            i = i.checked_add(n).filter(|&e| e <= src.len())?;
            continue;
        }
        if magic != MAGIC {
            return None;
        }
        let desc_start = i;
        let flg = *src.get(i)?;
        let bd = *src.get(i + 1)?;
        i += 2;
        if flg >> 6 != 1 || flg & 0b10 != 0 || flg & 1 != 0 || bd & 0x8F != 0 {
            return None; // version, reserved bit, dictionary id, reserved BD bits
        }
        let block_checksum = flg & 0x10 != 0;
        let has_size = flg & 0x08 != 0;
        let content_checksum = flg & 0x04 != 0;
        let bmax = block_max((bd >> 4) & 7)?;
        let mut size = None;
        if has_size {
            let b = src.get(i..i + 8)?;
            size = Some(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]));
            i += 8;
        }
        let hc = *src.get(i)?;
        if (xxh32(&src[desc_start..i], 0) >> 8) as u8 != hc {
            return None;
        }
        i += 1;
        let frame_start = out.len();
        loop {
            let word = rd_u32(src, &mut i)?;
            if word == 0 {
                break;
            }
            let raw = word & 0x8000_0000 != 0;
            let n = (word & 0x7FFF_FFFF) as usize;
            if n > bmax {
                return None;
            }
            let data = src.get(i..i + n)?;
            i += n;
            if block_checksum {
                let c = rd_u32(src, &mut i)?;
                if xxh32(data, 0) != c {
                    return None;
                }
            }
            if raw {
                if data.len() > max_out - out.len() {
                    return None;
                }
                out.extend_from_slice(data);
            } else {
                let limit = max_out.min(out.len() + bmax);
                if !decompress_into(data, &mut out, limit) {
                    return None;
                }
            }
        }
        if let Some(s) = size {
            if (out.len() - frame_start) as u64 != s {
                return None;
            }
        }
        if content_checksum {
            let c = rd_u32(src, &mut i)?;
            if xxh32(&out[frame_start..], 0) != c {
                return None;
            }
        }
    }
    Some(out)
}
