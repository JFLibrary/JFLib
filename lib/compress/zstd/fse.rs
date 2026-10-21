// fse -- finite state entropy (tANS) as zstd uses it
//
//   read_ncount / write_ncount   the table description (normalized counts, accuracy log)
//   DTable::build                the decoding table (the spread of RFC 8878 4.1.1)
//   CTable::build / encode       the matching encoder: for a symbol and the decoder state of
//                                the following symbol, the bits to write and the new state
//   normalize, cost              counts -> a distribution of 1 << log; estimated bits
//
// A count of -1 is a symbol below one slot ("less than 1"): it gets one slot at the end of the
// table, reached with a full state reload.

use alloc::vec;
use alloc::vec::Vec;

use super::bits::{FwdReader, Writer};

#[inline(always)]
pub fn highbit(v: u32) -> u32 {
    31 - v.leading_zeros()
}

/// read a table description -> (counts, accuracy log, bytes used)
pub fn read_ncount(b: &[u8], max_sym: usize, max_log: u32) -> Option<(Vec<i16>, u32, usize)> {
    let mut r = FwdReader::new(b);
    let log = r.read(4) as u32 + 5;
    if log > max_log {
        return None;
    }
    let mut remaining: i32 = (1 << log) + 1;
    let mut threshold: i32 = 1 << log;
    let mut nb = log + 1;
    let mut counts: Vec<i16> = Vec::new();
    let mut prev0 = false;
    while remaining > 1 {
        if counts.len() > max_sym {
            return None;
        }
        if prev0 {
            let mut n0 = 0usize;
            loop {
                let rep = r.read(2) as usize;
                n0 += rep;
                if rep != 3 {
                    break;
                }
                if n0 > 256 {
                    return None;
                }
            }
            if counts.len() + n0 > max_sym {
                return None;
            }
            counts.extend(core::iter::repeat_n(0, n0));
        }
        let max = (2 * threshold - 1) - remaining;
        let low = r.peek(nb - 1) as i32;
        let mut count;
        if low < max {
            count = low;
            r.skip(nb - 1);
        } else {
            count = r.peek(nb) as i32;
            if count >= threshold {
                count -= max;
            }
            r.skip(nb);
        }
        count -= 1;
        remaining -= count.abs();
        if remaining < 1 {
            return None;
        }
        counts.push(count as i16);
        prev0 = count == 0;
        while remaining < threshold {
            nb -= 1;
            threshold >>= 1;
        }
    }
    if remaining != 1 || r.bytes() > b.len() {
        return None;
    }
    Some((counts, log, r.bytes()))
}

/// write a table description (counts trimmed after the last used symbol)
pub fn write_ncount(out: &mut Vec<u8>, norm: &[i16], log: u32) {
    let mut w = Writer::new();
    w.add((log - 5) as u64, 4);
    let size: i32 = 1 << log;
    let mut remaining = size + 1;
    let mut threshold = size;
    let mut nb = log + 1;
    let mut s = 0;
    let mut prev0 = false;
    while s < norm.len() && remaining > 1 {
        if prev0 {
            let start = s;
            while s < norm.len() && norm[s] == 0 {
                s += 1;
            }
            if s == norm.len() {
                break; // not a complete distribution: the reader will refuse it
            }
            let mut n0 = s - start;
            while n0 >= 24 {
                w.add(0xFFFF, 16);
                n0 -= 24;
            }
            while n0 >= 3 {
                w.add(3, 2);
                n0 -= 3;
            }
            w.add(n0 as u64, 2);
        }
        let mut count = norm[s] as i32;
        s += 1;
        let max = (2 * threshold - 1) - remaining;
        remaining -= count.abs();
        count += 1;
        if count >= threshold {
            count += max;
        }
        w.add(count as u64, if count < max { nb - 1 } else { nb });
        prev0 = count == 1;
        while remaining < threshold {
            nb -= 1;
            threshold >>= 1;
        }
    }
    out.extend_from_slice(&w.finish());
}

/// the positions of the symbols in a table of 1 << log (decoder state -> symbol)
fn spread(counts: &[i16], log: u32) -> Option<Vec<u8>> {
    let size = 1usize << log;
    let mut t = vec![0u8; size];
    let mut high = size as isize - 1;
    let mut total = 0usize;
    for (s, &c) in counts.iter().enumerate() {
        if c == -1 {
            if high < 0 {
                return None;
            }
            t[high as usize] = s as u8;
            high -= 1;
            total += 1;
        } else if c > 0 {
            total += c as usize;
        } else if c < -1 {
            return None;
        }
    }
    if total != size {
        return None;
    }
    let step = (size >> 1) + (size >> 3) + 3;
    let m = size - 1;
    let mut pos = 0usize;
    for (s, &c) in counts.iter().enumerate() {
        for _ in 0..c.max(0) {
            t[pos] = s as u8;
            loop {
                pos = (pos + step) & m;
                if pos as isize <= high {
                    break;
                }
            }
        }
    }
    (pos == 0).then_some(t)
}

#[derive(Clone, Copy, Default)]
pub struct DEntry {
    pub sym: u8,
    pub nb: u8,
    pub base: u16,
}

#[derive(Clone)]
pub struct DTable {
    pub log: u32,
    pub t: Vec<DEntry>,
}

impl DTable {
    pub fn build(counts: &[i16], log: u32) -> Option<DTable> {
        let syms = spread(counts, log)?;
        let size = 1u32 << log;
        let mut next: Vec<u32> = counts.iter().map(|&c| if c == -1 { 1 } else { c.max(0) as u32 }).collect();
        let t = syms
            .iter()
            .map(|&s| {
                let ns = next[s as usize];
                next[s as usize] += 1;
                let nb = log - highbit(ns);
                DEntry { sym: s, nb: nb as u8, base: ((ns << nb) - size) as u16 }
            })
            .collect();
        Some(DTable { log, t })
    }

    /// one symbol, always (RLE mode)
    pub fn rle(sym: u8) -> DTable {
        DTable { log: 0, t: vec![DEntry { sym, nb: 0, base: 0 }] }
    }
}

/// the encoder of a distribution (or of one symbol: RLE, no bits at all)
#[derive(Clone)]
pub struct CTable {
    pub log: u32,
    /// per symbol: (first index into `states`, slot count)
    sym: Vec<(u32, u32)>,
    /// the decoder states of each symbol, in increasing order
    states: Vec<u16>,
    rle: bool,
}

impl CTable {
    pub fn build(counts: &[i16], log: u32) -> Option<CTable> {
        let syms = spread(counts, log)?;
        let mut sym = vec![(0u32, 0u32); counts.len()];
        let mut at = 0u32;
        for (s, &c) in counts.iter().enumerate() {
            let k = if c == -1 { 1 } else { c.max(0) as u32 };
            sym[s] = (at, k);
            at += k;
        }
        let mut states = vec![0u16; at as usize];
        let mut fill: Vec<u32> = sym.iter().map(|x| x.0).collect();
        for (u, &s) in syms.iter().enumerate() {
            states[fill[s as usize] as usize] = u as u16;
            fill[s as usize] += 1;
        }
        Some(CTable { log, sym, states, rle: false })
    }

    pub fn rle() -> CTable {
        CTable { log: 0, sym: Vec::new(), states: Vec::new(), rle: true }
    }

    /// the state the decoder starts with for the last symbol
    pub fn init(&self, s: u8) -> u32 {
        if self.rle { 0 } else { self.states[self.sym[s as usize].0 as usize] as u32 }
    }

    /// code symbol s before the symbol whose decoder state is x -> the state of s
    #[inline(always)]
    pub fn encode(&self, w: &mut Writer, x: u32, s: u8) -> u32 {
        if self.rle {
            return 0;
        }
        let (first, k) = self.sym[s as usize];
        let size = 1u32 << self.log;
        let v = x + size;
        let mut nb = self.log - highbit(k);
        if (v >> nb) < k {
            nb -= 1;
        }
        let j = v >> nb;
        w.add((v & ((1 << nb) - 1)) as u64, nb);
        self.states[(first + j - k) as usize] as u32
    }

    /// the final state (read first by the decoder)
    pub fn flush(&self, w: &mut Writer, x: u32) {
        w.add(x as u64, self.log);
    }
}

/// counts -> a distribution summing to 1 << log; every used symbol keeps at least one slot
pub fn normalize(hist: &[u32], log: u32) -> Vec<i16> {
    let total: u64 = hist.iter().map(|&c| c as u64).sum();
    let size = 1i64 << log;
    let mut norm = vec![0i16; hist.len()];
    let mut sum = 0i64;
    let mut largest = 0;
    for (s, &c) in hist.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let p = ((c as u64 * size as u64 + total / 2) / total.max(1)).max(1) as i64;
        norm[s] = p as i16;
        sum += p;
        if c > hist[largest] {
            largest = s;
        }
    }
    let mut diff = size - sum;
    if diff >= 0 || norm[largest] as i64 + diff >= (norm[largest] as i64 + 1) / 2 {
        norm[largest] = (norm[largest] as i64 + diff) as i16;
        return norm;
    }
    // many rare symbols were rounded up: take the slots back from the biggest ones
    while diff < 0 {
        let (s, _) = norm.iter().enumerate().max_by_key(|&(_, &n)| n).unwrap_or((0, &0));
        if norm[s] <= 1 {
            break;
        }
        norm[s] -= 1;
        diff += 1;
    }
    norm
}

/// log2 in 1/256 bits
fn log2_256(v: u32) -> u32 {
    if v == 0 {
        return 0;
    }
    let hb = highbit(v);
    let frac = if hb >= 8 { (v >> (hb - 8)) & 0xFF } else { (v << (8 - hb)) & 0xFF };
    hb * 256 + frac
}

/// estimated bits (x 256) to code `hist` with a distribution; None when a used symbol has no slot
pub fn cost(hist: &[u32], norm: &[i16], log: u32) -> Option<u64> {
    let mut bits = 0u64;
    for (s, &c) in hist.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let n = *norm.get(s)?;
        let slots = if n == -1 { 1 } else if n > 0 { n as u32 } else { return None };
        bits += c as u64 * (log * 256 - log2_256(slots).min(log * 256)) as u64;
    }
    Some(bits)
}

/// the accuracy log for `n` symbols of an alphabet up to max_sym (RFC limits: max_log)
pub fn choose_log(n: usize, max_sym: usize, max_log: u32) -> u32 {
    let n = n.max(2) as u32;
    let max_from_src = highbit(n - 1).saturating_sub(2);
    let min_bits = (highbit(n) + 1).min(highbit(max_sym.max(1) as u32) + 2);
    max_log.min(max_from_src).max(min_bits).clamp(5, max_log)
}
