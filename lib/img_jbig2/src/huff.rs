// huff -- the Huffman tables of JBIG2 (ITU-T T.88 Annex B): the fifteen standard tables, tables
// from a table segment (B.2) and the symbol id table of a text region (7.4.3.1.7), and the bit
// reader of the Huffman-coded data
//
// A table is a list of lines (prefix length, range length, range low, kind); the prefix codes are
// assigned from the prefix lengths in table order (B.3), so a table is written down as its lengths
// and ranges only.  A value is the line's low end plus RANGELEN bits read after the prefix (the
// lower range line subtracts them; the out-of-band line has no value).  A value beyond the 32 bits
// of JBIG2's integers is refused, not wrapped.

use crate::Error;
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Normal,
    Lower,
    Upper,
    Oob,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Line {
    pub preflen: u8,
    pub rangelen: u8,
    /// i64: a table segment's lower range line starts at HTLOW - 1
    pub low: i64,
    pub kind: Kind,
}

/// a table with its canonical codes: per prefix length the first code and its lines in code order
#[derive(Clone, Debug)]
pub(crate) struct Table {
    lines: Vec<Line>,
    /// per prefix length (0..=32): first code, index of its first line in `order`, count
    first: Vec<(u32, usize, usize)>,
    order: Vec<usize>,
}

impl Table {
    /// the codes of `lines` (B.3); None when a prefix is longer than 32 bits
    pub fn new(lines: Vec<Line>) -> Option<Table> {
        let max = lines.iter().map(|l| l.preflen).max().unwrap_or(0) as usize;
        if max > 32 {
            return None;
        }
        let mut count = vec![0usize; max + 1];
        for l in &lines {
            if l.preflen > 0 {
                count[l.preflen as usize] += 1;
            }
        }
        let mut first = vec![(0u32, 0usize, 0usize); 33];
        let mut order = Vec::with_capacity(lines.len());
        let mut code: u64 = 0;
        for len in 1..=max {
            code = (code + count[len - 1] as u64) << 1;
            if code + count[len] as u64 > (1u64 << len) {
                return None;
            }
            first[len] = (code as u32, order.len(), count[len]);
            order.extend(lines.iter().enumerate().filter(|(_, l)| l.preflen as usize == len).map(|(i, _)| i));
        }
        Some(Table { lines, first, order })
    }

    /// the code of line i (for the tests): (code, length)
    #[allow(dead_code)]
    pub fn code_of(&self, i: usize) -> (u32, u8) {
        let len = self.lines[i].preflen as usize;
        let (c, start, _) = self.first[len];
        let k = self.order[start..].iter().position(|&j| j == i).unwrap_or(0);
        (c + k as u32, len as u8)
    }

    /// one value (None: out of band)
    pub fn decode(&self, b: &mut Bits) -> Result<Option<i32>, Error> {
        let mut code = 0u32;
        for len in 1..=32usize {
            code = (code << 1) | b.bit()?;
            let (c, start, n) = self.first[len];
            if n > 0 && code >= c && code - c < n as u32 {
                let l = self.lines[self.order[start + (code - c) as usize]];
                let v = match l.kind {
                    Kind::Oob => return Ok(None),
                    Kind::Normal => l.low + b.bits(l.rangelen as u32)? as i64,
                    Kind::Upper => l.low + b.bits(32)? as i64,
                    Kind::Lower => l.low - b.bits(32)? as i64,
                };
                return i32::try_from(v).map(Some).map_err(|_| Error::Corrupt);
            }
        }
        Err(Error::Corrupt)
    }
}

/// the bits of Huffman-coded data: MSB first, no stuffing
pub(crate) struct Bits<'a> {
    pub d: &'a [u8],
    /// the position in bits
    pub pos: usize,
}

impl<'a> Bits<'a> {
    pub fn new(d: &'a [u8]) -> Bits<'a> {
        Bits { d, pos: 0 }
    }
    #[inline]
    pub fn bit(&mut self) -> Result<u32, Error> {
        let b = *self.d.get(self.pos >> 3).ok_or(Error::Truncated)?;
        let v = (b >> (7 - (self.pos & 7))) & 1;
        self.pos += 1;
        Ok(v as u32)
    }
    pub fn bits(&mut self, n: u32) -> Result<u32, Error> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Ok(v)
    }
    /// to the next byte boundary
    pub fn align(&mut self) {
        self.pos = (self.pos + 7) & !7;
    }
    /// the byte position (after `align`)
    pub fn byte_pos(&self) -> usize {
        self.pos.div_ceil(8)
    }
}

const fn n(preflen: u8, rangelen: u8, low: i64) -> Line {
    Line { preflen, rangelen, low, kind: Kind::Normal }
}
const fn lo(preflen: u8, low: i64) -> Line {
    Line { preflen, rangelen: 32, low, kind: Kind::Lower }
}
const fn up(preflen: u8, low: i64) -> Line {
    Line { preflen, rangelen: 32, low, kind: Kind::Upper }
}
const fn oob(preflen: u8) -> Line {
    Line { preflen, rangelen: 0, low: 0, kind: Kind::Oob }
}

/// the lines of standard table B.k (1..=15), in the order of the standard
pub(crate) fn standard_lines(k: u8) -> Vec<Line> {
    let v: &[Line] = match k {
        1 => &[n(1, 4, 0), n(2, 8, 16), n(3, 16, 272), up(3, 65808)],
        2 => &[n(1, 0, 0), n(2, 0, 1), n(3, 0, 2), n(4, 3, 3), n(5, 6, 11), up(6, 75), oob(6)],
        3 => &[n(8, 8, -256), n(1, 0, 0), n(2, 0, 1), n(3, 0, 2), n(4, 3, 3), n(5, 6, 11), lo(8, -257), up(7, 75), oob(6)],
        4 => &[n(1, 0, 1), n(2, 0, 2), n(3, 0, 3), n(4, 3, 4), n(5, 6, 12), up(5, 76)],
        5 => &[n(7, 8, -255), n(1, 0, 1), n(2, 0, 2), n(3, 0, 3), n(4, 3, 4), n(5, 6, 12), lo(7, -256), up(6, 76)],
        6 => &[
            n(5, 10, -2048),
            n(4, 9, -1024),
            n(4, 8, -512),
            n(4, 7, -256),
            n(5, 6, -128),
            n(5, 5, -64),
            n(4, 5, -32),
            n(2, 7, 0),
            n(3, 7, 128),
            n(3, 8, 256),
            n(4, 9, 512),
            n(4, 10, 1024),
            lo(6, -2049),
            up(6, 2048),
        ],
        7 => &[
            n(4, 9, -1024),
            n(3, 8, -512),
            n(4, 7, -256),
            n(5, 6, -128),
            n(5, 5, -64),
            n(4, 5, -32),
            n(4, 5, 0),
            n(5, 5, 32),
            n(5, 6, 64),
            n(4, 7, 128),
            n(3, 8, 256),
            n(3, 9, 512),
            n(3, 10, 1024),
            lo(5, -1025),
            up(5, 2048),
        ],
        8 => &[
            n(8, 3, -15),
            n(9, 1, -7),
            n(8, 1, -5),
            n(9, 0, -3),
            n(7, 0, -2),
            n(4, 0, -1),
            n(2, 1, 0),
            n(5, 0, 2),
            n(6, 0, 3),
            n(3, 4, 4),
            n(6, 1, 20),
            n(4, 4, 22),
            n(4, 5, 38),
            n(5, 6, 70),
            n(5, 7, 134),
            n(6, 7, 262),
            n(7, 8, 390),
            n(6, 10, 646),
            lo(9, -16),
            up(9, 1670),
            oob(2),
        ],
        9 => &[
            n(8, 4, -31),
            n(9, 2, -15),
            n(8, 2, -11),
            n(9, 1, -7),
            n(7, 1, -5),
            n(4, 1, -3),
            n(3, 1, -1),
            n(3, 1, 1),
            n(5, 1, 3),
            n(6, 1, 5),
            n(3, 5, 7),
            n(6, 2, 39),
            n(4, 5, 43),
            n(4, 6, 75),
            n(5, 7, 139),
            n(5, 8, 267),
            n(6, 8, 523),
            n(7, 9, 779),
            n(6, 11, 1291),
            lo(9, -32),
            up(9, 3339),
            oob(2),
        ],
        10 => &[
            n(7, 4, -21),
            n(8, 0, -5),
            n(7, 0, -4),
            n(5, 0, -3),
            n(2, 2, -2),
            n(5, 0, 2),
            n(6, 0, 3),
            n(7, 0, 4),
            n(8, 0, 5),
            n(2, 6, 6),
            n(5, 5, 70),
            n(6, 5, 102),
            n(6, 6, 134),
            n(6, 7, 198),
            n(6, 8, 326),
            n(6, 9, 582),
            n(6, 10, 1094),
            n(7, 11, 2118),
            lo(8, -22),
            up(8, 4166),
            oob(2),
        ],
        11 => &[n(1, 0, 1), n(2, 1, 2), n(4, 0, 4), n(4, 1, 5), n(5, 1, 7), n(5, 2, 9), n(6, 2, 13), n(7, 2, 17), n(7, 3, 21), n(7, 4, 29), n(7, 5, 45), n(7, 6, 77), up(7, 141)],
        12 => &[n(1, 0, 1), n(2, 0, 2), n(3, 1, 3), n(5, 0, 5), n(5, 1, 6), n(6, 1, 8), n(7, 0, 10), n(7, 1, 11), n(7, 2, 13), n(7, 3, 17), n(7, 4, 25), n(8, 5, 41), up(8, 73)],
        13 => &[n(1, 0, 1), n(3, 0, 2), n(4, 0, 3), n(5, 0, 4), n(4, 1, 5), n(3, 3, 7), n(6, 1, 15), n(6, 2, 17), n(6, 3, 21), n(6, 4, 29), n(6, 5, 45), n(7, 6, 77), up(7, 141)],
        14 => &[n(3, 0, -2), n(3, 0, -1), n(1, 0, 0), n(3, 0, 1), n(3, 0, 2)],
        15 => &[n(7, 4, -24), n(6, 2, -8), n(5, 1, -4), n(4, 0, -2), n(3, 0, -1), n(1, 0, 0), n(3, 0, 1), n(4, 0, 2), n(5, 1, 3), n(6, 2, 5), n(7, 4, 9), lo(7, -25), up(7, 25)],
        _ => &[],
    };
    v.to_vec()
}

/// in a list of standard tables per selector value: a value the standard reserves (7.4.2.1.1,
/// 7.4.3.1.2; 0 there: the next custom table)
pub(crate) const RESERVED: u8 = 0xFF;

/// standard table B.k
pub(crate) fn standard(k: u8) -> Table {
    Table::new(standard_lines(k)).unwrap_or(Table { lines: Vec::new(), first: vec![(0, 0, 0); 33], order: Vec::new() })
}

/// a table segment (B.2)
pub(crate) fn from_segment(d: &[u8]) -> Result<Table, Error> {
    if d.len() < 9 {
        return Err(Error::Truncated);
    }
    let flags = d[0];
    let htoob = flags & 1 != 0;
    let htps = ((flags >> 1) & 7) as u32 + 1;
    let htrs = ((flags >> 4) & 7) as u32 + 1;
    let low = i32::from_be_bytes([d[1], d[2], d[3], d[4]]);
    let high = i32::from_be_bytes([d[5], d[6], d[7], d[8]]);
    let mut b = Bits::new(&d[9..]);
    let mut lines = Vec::new();
    let mut cur = low as i64;
    while cur < high as i64 {
        let preflen = b.bits(htps)? as u8;
        let rangelen = b.bits(htrs)? as u8;
        if rangelen > 31 || lines.len() > 4096 {
            return Err(Error::Corrupt);
        }
        lines.push(n(preflen, rangelen, cur));
        cur += 1i64 << rangelen;
    }
    lines.push(lo(b.bits(htps)? as u8, low as i64 - 1));
    lines.push(up(b.bits(htps)? as u8, high as i64));
    if htoob {
        lines.push(oob(b.bits(htps)? as u8));
    }
    Table::new(lines).ok_or(Error::Corrupt)
}

/// the symbol id table of a Huffman-coded text region (7.4.3.1.7): run codes, then a code length
/// per symbol; the reader is left at the next byte boundary
pub(crate) fn symbol_id_table(b: &mut Bits, nsyms: usize) -> Result<Table, Error> {
    let mut run = Vec::with_capacity(35);
    for i in 0..35 {
        run.push(n(b.bits(4)? as u8, 0, i));
    }
    let runs = Table::new(run).ok_or(Error::Corrupt)?;
    let mut lens: Vec<u8> = Vec::with_capacity(nsyms);
    while lens.len() < nsyms {
        let r = runs.decode(b)?.ok_or(Error::Corrupt)?;
        let (len, times) = match r {
            0..=31 => (r as u8, 1),
            32 => (*lens.last().ok_or(Error::Corrupt)?, 3 + b.bits(2)? as usize),
            33 => (0, 3 + b.bits(3)? as usize),
            _ => (0, 11 + b.bits(7)? as usize),
        };
        for _ in 0..times {
            if lens.len() < nsyms {
                lens.push(len);
            }
        }
    }
    b.align();
    Table::new(lens.iter().enumerate().map(|(i, &l)| n(l, 0, i as i64)).collect()).ok_or(Error::Corrupt)
}
