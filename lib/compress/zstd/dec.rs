// dec -- the zstd frame decoder (RFC 8878): frames and skippable frames, raw / RLE /
// compressed blocks, raw / RLE / Huffman / treeless literals with 1 or 4 streams, sequences
// with predefined / RLE / FSE / repeat tables, repeat offsets, content size and checksum.
// Dictionaries are not supported (a frame that names one is refused).  Every length, offset
// and table is checked: corrupt input gives None, never a panic or more than max_out bytes.

use alloc::vec::Vec;

use super::bits::BackReader;
use super::fse::{self, DTable};
use super::huf;
use super::seq::*;
use crate::xxh::xxh64;

pub const MAGIC: u32 = 0xFD2F_B528;
pub const BLOCK_MAX: usize = 128 << 10;

/// what carries over from block to block inside a frame
struct State {
    huf: Option<huf::Table>,
    ll: Option<DTable>,
    of: Option<DTable>,
    ml: Option<DTable>,
    rep: [u64; 3],
    lits: Vec<u8>,
}

fn rd_le(b: &[u8]) -> u64 {
    b.iter().rev().fold(0u64, |a, &x| a << 8 | x as u64)
}

/// decode all frames of src -> None on corruption, unsupported features or > max_out bytes
pub fn decompress(src: &[u8], max_out: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let magic = rd_le(src.get(i..i + 4)?) as u32;
        if magic & 0xFFFF_FFF0 == 0x184D_2A50 {
            let n = rd_le(src.get(i + 4..i + 8)?) as usize;
            i = i.checked_add(8 + n).filter(|&e| e <= src.len())?;
            continue;
        }
        if magic != MAGIC {
            return None;
        }
        i += frame(&src[i..], &mut out, max_out)?;
    }
    Some(out)
}

/// the content size a frame header declares (None: not stated or not a zstd frame)
pub fn content_size(src: &[u8]) -> Option<u64> {
    if rd_le(src.get(..4)?) as u32 != MAGIC {
        return None;
    }
    let fhd = *src.get(4)?;
    let single = fhd & 0x20 != 0;
    let mut p = 5 + if single { 0 } else { 1 };
    p += [0, 1, 2, 4][(fhd & 3) as usize];
    match (fhd >> 6, single) {
        (0, false) => None,
        (0, true) => Some(*src.get(p)? as u64),
        (1, _) => Some(rd_le(src.get(p..p + 2)?) + 256),
        (2, _) => Some(rd_le(src.get(p..p + 4)?)),
        _ => Some(rd_le(src.get(p..p + 8)?)),
    }
}

fn frame(src: &[u8], out: &mut Vec<u8>, max_out: usize) -> Option<usize> {
    let fhd = *src.get(4)?;
    if fhd & 0x08 != 0 {
        return None; // reserved bit
    }
    let single = fhd & 0x20 != 0;
    let checksum = fhd & 0x04 != 0;
    let mut p = 5;
    if !single {
        let wd = *src.get(p)?;
        p += 1;
        if (wd >> 3) + 10 > 41 {
            return None;
        }
    }
    let did_size = [0, 1, 2, 4][(fhd & 3) as usize];
    if did_size > 0 {
        if rd_le(src.get(p..p + did_size)?) != 0 {
            return None; // dictionaries are not supported
        }
        p += did_size;
    }
    let fcs_size = match fhd >> 6 {
        0 => usize::from(single),
        1 => 2,
        2 => 4,
        _ => 8,
    };
    let fcs = if fcs_size > 0 {
        let v = rd_le(src.get(p..p + fcs_size)?) + if fcs_size == 2 { 256 } else { 0 };
        p += fcs_size;
        if v > (max_out - out.len()) as u64 {
            return None;
        }
        Some(v)
    } else {
        None
    };
    let start = out.len();
    if let Some(n) = fcs {
        out.reserve(n as usize);
    }
    let mut st = State { huf: None, ll: None, of: None, ml: None, rep: [1, 4, 8], lits: Vec::new() };
    loop {
        let h = rd_le(src.get(p..p + 3)?) as usize;
        p += 3;
        let last = h & 1 != 0;
        let size = h >> 3;
        if size > BLOCK_MAX {
            return None;
        }
        match (h >> 1) & 3 {
            0 => {
                let b = src.get(p..p + size)?;
                if b.len() > max_out - out.len() {
                    return None;
                }
                out.extend_from_slice(b);
                p += size;
            }
            1 => {
                let b = *src.get(p)?;
                if size > max_out - out.len() {
                    return None;
                }
                out.resize(out.len() + size, b);
                p += 1;
            }
            2 => {
                let b = src.get(p..p + size)?;
                block(&mut st, b, out, start, max_out)?;
                p += size;
            }
            _ => return None,
        }
        if last {
            break;
        }
    }
    if fcs.is_some_and(|n| n != (out.len() - start) as u64) {
        return None;
    }
    if checksum {
        let c = rd_le(src.get(p..p + 4)?) as u32;
        if xxh64(&out[start..], 0) as u32 != c {
            return None;
        }
        p += 4;
    }
    Some(p)
}

/// the literals section -> bytes used (the literals land in st.lits)
fn literals(st: &mut State, src: &[u8]) -> Option<usize> {
    let b0 = *src.first()? as usize;
    let ty = b0 & 3;
    let sf = (b0 >> 2) & 3;
    st.lits.clear();
    if ty < 2 {
        let (n, hs) = match sf {
            0 | 2 => (b0 >> 3, 1),
            1 => ((b0 >> 4) | (*src.get(1)? as usize) << 4, 2),
            _ => ((b0 >> 4) | (*src.get(1)? as usize) << 4 | (*src.get(2)? as usize) << 12, 3),
        };
        if n > BLOCK_MAX {
            return None;
        }
        if ty == 0 {
            st.lits.extend_from_slice(src.get(hs..hs + n)?);
            return Some(hs + n);
        }
        st.lits.resize(n, *src.get(hs)?);
        return Some(hs + 1);
    }
    let (hs, bits, streams) = match sf {
        0 => (3, 10, 1),
        1 => (3, 10, 4),
        2 => (4, 14, 4),
        _ => (5, 18, 4),
    };
    let h = rd_le(src.get(..hs)?) as usize;
    let m = (1usize << bits) - 1;
    let regen = (h >> 4) & m;
    let csize = (h >> (4 + bits)) & m;
    if regen > BLOCK_MAX {
        return None;
    }
    let mut data = src.get(hs..hs + csize)?;
    if ty == 2 {
        let (t, used) = huf::read_table(data)?;
        st.huf = Some(t);
        data = &data[used..];
    }
    let t = st.huf.as_ref()?;
    if streams == 1 {
        if !huf::decode(t, data, &mut st.lits, regen) {
            return None;
        }
    } else {
        if data.len() < 10 {
            return None;
        }
        let s1 = rd_le(&data[0..2]) as usize;
        let s2 = rd_le(&data[2..4]) as usize;
        let s3 = rd_le(&data[4..6]) as usize;
        let rest = data.len() - 6;
        let s4 = rest.checked_sub(s1 + s2 + s3)?;
        let seg = regen.div_ceil(4);
        let last = regen.checked_sub(3 * seg)?;
        let mut at = 6;
        for (k, sz) in [s1, s2, s3, s4].into_iter().enumerate() {
            if !huf::decode(t, &data[at..at + sz], &mut st.lits, if k < 3 { seg } else { last }) {
                return None;
            }
            at += sz;
        }
    }
    Some(hs + csize)
}

/// one table of the sequences section -> bytes used
fn table(mode: u8, src: &[u8], prev: &mut Option<DTable>, def: &[i16], def_log: u32, max_sym: usize, max_log: u32) -> Option<usize> {
    match mode {
        0 => {
            *prev = Some(DTable::build(def, def_log)?);
            Some(0)
        }
        1 => {
            let s = *src.first()?;
            if s as usize > max_sym {
                return None;
            }
            *prev = Some(DTable::rle(s));
            Some(1)
        }
        2 => {
            let (counts, log, used) = fse::read_ncount(src, max_sym, max_log)?;
            *prev = Some(DTable::build(&counts, log)?);
            Some(used)
        }
        _ => prev.is_some().then_some(0),
    }
}

/// a compressed block, appended to out (`start`: where the frame's output begins)
fn block(st: &mut State, src: &[u8], out: &mut Vec<u8>, start: usize, max_out: usize) -> Option<()> {
    let mut p = literals(st, src)?;
    let limit = max_out.min(out.len() + BLOCK_MAX);
    let b0 = *src.get(p)? as usize;
    let nseq = if b0 < 128 {
        p += 1;
        b0
    } else if b0 < 255 {
        p += 2;
        ((b0 - 128) << 8) + *src.get(p - 1)? as usize
    } else {
        p += 3;
        rd_le(src.get(p - 2..p)?) as usize + 0x7F00
    };
    let lits = core::mem::take(&mut st.lits);
    if nseq == 0 {
        if p != src.len() || lits.len() > limit - out.len() {
            return None;
        }
        out.extend_from_slice(&lits);
        st.lits = lits;
        return Some(());
    }
    let modes = *src.get(p)?;
    p += 1;
    if modes & 3 != 0 {
        return None;
    }
    p += table(modes >> 6, src.get(p..)?, &mut st.ll, &LL_DEFAULT, LL_DEFAULT_LOG, LL_MAX, LL_LOG_MAX)?;
    p += table((modes >> 4) & 3, src.get(p..)?, &mut st.of, &OF_DEFAULT, OF_DEFAULT_LOG, OF_MAX, OF_LOG_MAX)?;
    p += table((modes >> 2) & 3, src.get(p..)?, &mut st.ml, &ML_DEFAULT, ML_DEFAULT_LOG, ML_MAX, ML_LOG_MAX)?;
    let (llt, oft, mlt) = (st.ll.as_ref()?, st.of.as_ref()?, st.ml.as_ref()?);
    let mut r = BackReader::new(src.get(p..)?)?;
    let mut sll = r.read(llt.log) as usize;
    let mut sof = r.read(oft.log) as usize;
    let mut sml = r.read(mlt.log) as usize;
    let mut lp = 0usize;
    let mut rep = st.rep;
    for i in 0..nseq {
        let (el, eo, em) = (llt.t[sll], oft.t[sof], mlt.t[sml]);
        let ofc = eo.sym as u32;
        let ov = (1u64 << ofc) + r.read(ofc);
        let ml = ML_BASE[em.sym as usize] as usize + r.read(ML_BITS[em.sym as usize] as u32) as usize;
        let ll = LL_BASE[el.sym as usize] as usize + r.read(LL_BITS[el.sym as usize] as u32) as usize;
        let off = if ov > 3 {
            let o = ov - 3;
            rep = [o, rep[0], rep[1]];
            o
        } else {
            match ov as usize - 1 + usize::from(ll == 0) {
                0 => rep[0],
                1 => {
                    rep = [rep[1], rep[0], rep[2]];
                    rep[0]
                }
                2 => {
                    rep = [rep[2], rep[0], rep[1]];
                    rep[0]
                }
                _ => {
                    let o = rep[0].checked_sub(1).filter(|&o| o > 0)?;
                    rep = [o, rep[0], rep[1]];
                    o
                }
            }
        };
        if i + 1 < nseq {
            sll = el.base as usize + r.read(el.nb as u32) as usize;
            sml = em.base as usize + r.read(em.nb as u32) as usize;
            sof = eo.base as usize + r.read(eo.nb as u32) as usize;
        }
        // literals, then the match (it may overlap its own output)
        if ll > lits.len() - lp || ll + ml > limit - out.len() {
            return None;
        }
        out.extend_from_slice(&lits[lp..lp + ll]);
        lp += ll;
        let off = off as usize;
        if off > out.len() - start {
            return None;
        }
        let from = out.len() - off;
        if off >= ml {
            out.extend_from_within(from..from + ml);
        } else {
            let mut rem = ml;
            while rem > 0 {
                let k = rem.min(out.len() - from);
                out.extend_from_within(from..from + k);
                rem -= k;
            }
        }
    }
    if r.pos() != 0 || lits.len() - lp > limit - out.len() {
        return None;
    }
    out.extend_from_slice(&lits[lp..]);
    st.rep = rep;
    st.lits = lits;
    Some(())
}
