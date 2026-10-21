// huf -- the Huffman code of zstd's literals
//
//   read_table(b) -> (Table, used)   a tree description: weights stored directly (4 bits each)
//                                    or FSE compressed (two interleaved states); the weight of
//                                    the last symbol is implied (the code is complete)
//   decode(table, stream, out, n)    one backward stream of n symbols
//   Code::build(hist)                length-limited (11 bits) optimal code by package-merge
//   code.write_table / encode        the description and one stream
//
// Codes are canonical in zstd's order: lower weights (longer codes) take the lower values.

use alloc::vec;
use alloc::vec::Vec;

use super::bits::{BackReader, Writer};
use super::fse::{self, highbit, CTable, DTable};

pub const MAX_BITS: u32 = 11;

/// a decoding table: entry = symbol | bits << 8
#[derive(Clone)]
pub struct Table {
    pub log: u32,
    pub t: Vec<u16>,
}

/// a tree description -> the table and the bytes it took
pub fn read_table(b: &[u8]) -> Option<(Table, usize)> {
    let (w, used) = read_weights(b)?;
    Some((from_weights(w)?, used))
}

/// the explicit weights of a tree description and its size
fn read_weights(b: &[u8]) -> Option<(Vec<u8>, usize)> {
    let hb = *b.first()? as usize;
    let mut w: Vec<u8> = Vec::new();
    if hb < 128 {
        let data = b.get(1..1 + hb)?;
        let (counts, log, n) = fse::read_ncount(data, 255, 6)?;
        let dt = DTable::build(&counts, log)?;
        let mut r = BackReader::new(&data[n..])?;
        let mut s1 = r.read(log) as usize;
        let mut s2 = r.read(log) as usize;
        loop {
            if w.len() > 253 {
                return None;
            }
            let e = dt.t[s1];
            w.push(e.sym);
            s1 = e.base as usize + r.read(e.nb as u32) as usize;
            if r.pos() < 0 {
                w.push(dt.t[s2].sym);
                break;
            }
            let e = dt.t[s2];
            w.push(e.sym);
            s2 = e.base as usize + r.read(e.nb as u32) as usize;
            if r.pos() < 0 {
                w.push(dt.t[s1].sym);
                break;
            }
        }
        Some((w, 1 + hb))
    } else {
        let n = hb - 127;
        let bytes = b.get(1..1 + n.div_ceil(2))?;
        for i in 0..n {
            let x = bytes[i / 2];
            w.push(if i % 2 == 0 { x >> 4 } else { x & 15 });
        }
        Some((w, 1 + n.div_ceil(2)))
    }
}

fn from_weights(mut w: Vec<u8>) -> Option<Table> {
    let mut total = 0u32;
    for &x in &w {
        if x as u32 > MAX_BITS {
            return None;
        }
        if x > 0 {
            total += 1 << (x - 1);
        }
    }
    if total == 0 {
        return None;
    }
    let log = highbit(total) + 1;
    if log > MAX_BITS {
        return None;
    }
    let rest = (1u32 << log) - total;
    if !rest.is_power_of_two() {
        return None;
    }
    w.push((highbit(rest) + 1) as u8);
    let mut rank = [0u32; 13];
    for &x in &w {
        rank[x as usize] += 1;
    }
    if rank[1] < 2 || rank[1] & 1 != 0 {
        return None;
    }
    let mut start = [0u32; 13];
    let mut next = 0;
    for wt in 1..=log as usize {
        start[wt] = next;
        next += rank[wt] << (wt - 1);
    }
    let mut t = vec![0u16; 1 << log];
    for (s, &x) in w.iter().enumerate() {
        if x == 0 {
            continue;
        }
        let len = 1u32 << (x - 1);
        let e = s as u16 | ((log + 1 - x as u32) as u16) << 8;
        for slot in &mut t[start[x as usize] as usize..(start[x as usize] + len) as usize] {
            *slot = e;
        }
        start[x as usize] += len;
    }
    Some(Table { log, t })
}

/// decode one stream of n symbols (the stream must be used up exactly)
pub fn decode(t: &Table, src: &[u8], out: &mut Vec<u8>, n: usize) -> bool {
    let Some(mut r) = BackReader::new(src) else { return false };
    out.reserve(n);
    for _ in 0..n {
        let e = t.t[r.peek(t.log) as usize];
        out.push(e as u8);
        r.skip((e >> 8) as u32);
    }
    r.pos() == 0
}

/// a code for compression
pub struct Code {
    pub log: u32,
    pub code: [u16; 256],
    pub nb: [u8; 256],
    /// the largest symbol with a code (its weight is implied)
    pub max_sym: usize,
}

/// code lengths limited to max_bits (package-merge; at least two symbols)
fn lengths(hist: &[u32; 256], syms: &[usize], max_bits: u32) -> [u8; 256] {
    let m = syms.len();
    let mut leaves: Vec<(u64, u32)> = syms.iter().enumerate().map(|(i, &s)| (hist[s] as u64, i as u32)).collect();
    leaves.sort_by_key(|x| x.0);
    // nodes m.. are packages of two items
    let mut kids: Vec<(u32, u32)> = Vec::new();
    let mut cur = leaves.clone();
    for _ in 1..max_bits {
        let mut pk = Vec::with_capacity(cur.len() / 2);
        for p in cur.chunks_exact(2) {
            kids.push((p[0].1, p[1].1));
            pk.push((p[0].0 + p[1].0, (m + kids.len() - 1) as u32));
        }
        let mut merged = Vec::with_capacity(leaves.len() + pk.len());
        let (mut a, mut b) = (0, 0);
        while a < leaves.len() || b < pk.len() {
            if b >= pk.len() || (a < leaves.len() && leaves[a].0 <= pk[b].0) {
                merged.push(leaves[a]);
                a += 1;
            } else {
                merged.push(pk[b]);
                b += 1;
            }
        }
        cur = merged;
    }
    let mut len = [0u8; 256];
    let mut stack: Vec<u32> = Vec::new();
    for item in cur.iter().take(2 * m - 2) {
        stack.push(item.1);
        while let Some(id) = stack.pop() {
            if (id as usize) < m {
                len[syms[id as usize]] += 1;
            } else {
                let (x, y) = kids[id as usize - m];
                stack.push(x);
                stack.push(y);
            }
        }
    }
    len
}

impl Code {
    /// None with fewer than two different symbols
    pub fn build(hist: &[u32; 256]) -> Option<Code> {
        let syms: Vec<usize> = (0..256).filter(|&s| hist[s] > 0).collect();
        if syms.len() < 2 {
            return None;
        }
        let len = lengths(hist, &syms, MAX_BITS);
        let log = *len.iter().max()? as u32;
        let mut rank = [0u32; 13];
        for &s in &syms {
            rank[(log + 1 - len[s] as u32) as usize] += 1;
        }
        let mut start = [0u32; 13];
        let mut next = 0;
        for wt in 1..=log as usize {
            start[wt] = next;
            next += rank[wt] << (wt - 1);
        }
        let mut c = Code { log, code: [0; 256], nb: [0; 256], max_sym: *syms.last()? };
        for &s in &syms {
            let wt = (log + 1 - len[s] as u32) as usize;
            c.code[s] = (start[wt] >> (wt - 1)) as u16;
            c.nb[s] = len[s];
            start[wt] += 1 << (wt - 1);
        }
        Some(c)
    }

    fn weight(&self, s: usize) -> u8 {
        if self.nb[s] == 0 { 0 } else { (self.log + 1 - self.nb[s] as u32) as u8 }
    }

    /// bits of the data with this code
    pub fn cost(&self, hist: &[u32; 256]) -> u64 {
        hist.iter().zip(self.nb.iter()).map(|(&c, &n)| c as u64 * n as u64).sum()
    }

    /// the tree description (false: it does not fit the format; use raw literals)
    pub fn write_table(&self, out: &mut Vec<u8>) -> bool {
        let w: Vec<u8> = (0..self.max_sym).map(|s| self.weight(s)).collect();
        let n = w.len();
        let mut best: Option<Vec<u8>> = None;
        if n <= 128 {
            let mut d = alloc::vec![(127 + n) as u8];
            for p in w.chunks(2) {
                d.push(p[0] << 4 | p.get(1).copied().unwrap_or(0));
            }
            best = Some(d);
        }
        if n >= 2 {
            if let Some(d) = fse_weights(&w) {
                if best.as_ref().is_none_or(|b| d.len() < b.len()) {
                    best = Some(d);
                }
            }
        }
        match best {
            Some(d) => {
                out.extend_from_slice(&d);
                true
            }
            None => false,
        }
    }

    /// one stream (the decoder reads it backward: the last symbol is written first)
    pub fn encode(&self, data: &[u8]) -> Vec<u8> {
        let mut w = Writer::new();
        for &b in data.iter().rev() {
            w.add(self.code[b as usize] as u64, self.nb[b as usize] as u32);
        }
        w.close()
    }
}

/// weights FSE compressed with two interleaved states; checked by decoding them again (the
/// decoder stops when it reads past the stream, which needs a state with bits to read)
fn fse_weights(w: &[u8]) -> Option<Vec<u8>> {
    let n = w.len();
    let mut hist = [0u32; 13];
    for &x in w {
        hist[x as usize] += 1;
    }
    let top = (0..13).rev().find(|&i| hist[i] > 0)?;
    let log = fse::choose_log(n, top, 6);
    let norm = fse::normalize(&hist[..=top], log);
    let ct = CTable::build(&norm, log)?;
    let mut out = alloc::vec![0u8];
    fse::write_ncount(&mut out, &norm, log);
    let mut bw = Writer::new();
    let mut st = [0u32; 2];
    st[(n - 1) % 2] = ct.init(w[n - 1]);
    st[(n - 2) % 2] = ct.init(w[n - 2]);
    for i in (0..n - 2).rev() {
        st[i % 2] = ct.encode(&mut bw, st[i % 2], w[i]);
    }
    ct.flush(&mut bw, st[1]);
    ct.flush(&mut bw, st[0]);
    out.extend_from_slice(&bw.close());
    let size = out.len() - 1;
    if size >= 128 {
        return None;
    }
    out[0] = size as u8;
    let (back, used) = read_weights(&out)?;
    (used == out.len() && back == w).then_some(out)
}
