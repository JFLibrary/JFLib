// CCITTFaxDecode (ISO 32000-1, 7.4.6; ITU-T T.4 and T.6): Group 3 one-dimensional (K = 0),
// mixed (K > 0, a tag bit after each line's EOL) and Group 4 (K < 0), with EncodedByteAlign,
// EOLs where they are (fill bits before them), EndOfBlock, BlackIs1, Columns and Rows.  The
// output is one bit per pixel, rows padded to bytes, 0 black (1 black with BlackIs1), as the
// filter gives it.  Damaged data ends the image: the rows after it stay white.

use super::super::object::Dict;
use super::super::Document;
use alloc::vec;
use alloc::vec::Vec;

const WHITE_TERM: [&str; 64] = [
    "00110101", "000111", "0111", "1000", "1011", "1100", "1110", "1111", "10011", "10100", "00111", "01000", "001000", "000011", "110100",
    "110101", "101010", "101011", "0100111", "0001100", "0001000", "0010111", "0000011", "0000100", "0101000", "0101011", "0010011",
    "0100100", "0011000", "00000010", "00000011", "00011010", "00011011", "00010010", "00010011", "00010100", "00010101", "00010110",
    "00010111", "00101000", "00101001", "00101010", "00101011", "00101100", "00101101", "00000100", "00000101", "00001010", "00001011",
    "01010010", "01010011", "01010100", "01010101", "00100100", "00100101", "01011000", "01011001", "01011010", "01011011", "01001010",
    "01001011", "00110010", "00110011", "00110100",
];
const WHITE_MAKEUP: [&str; 27] = [
    "11011", "10010", "010111", "0110111", "00110110", "00110111", "01100100", "01100101", "01101000", "01100111", "011001100", "011001101",
    "011010010", "011010011", "011010100", "011010101", "011010110", "011010111", "011011000", "011011001", "011011010", "011011011",
    "010011000", "010011001", "010011010", "011000", "010011011",
];
const BLACK_TERM: [&str; 64] = [
    "0000110111", "010", "11", "10", "011", "0011", "0010", "00011", "000101", "000100", "0000100", "0000101", "0000111", "00000100",
    "00000111", "000011000", "0000010111", "0000011000", "0000001000", "00001100111", "00001101000", "00001101100", "00000110111",
    "00000101000", "00000010111", "00000011000", "000011001010", "000011001011", "000011001100", "000011001101", "000001101000",
    "000001101001", "000001101010", "000001101011", "000011010010", "000011010011", "000011010100", "000011010101", "000011010110",
    "000011010111", "000001101100", "000001101101", "000011011010", "000011011011", "000001010100", "000001010101", "000001010110",
    "000001010111", "000001100100", "000001100101", "000001010010", "000001010011", "000000100100", "000000110111", "000000111000",
    "000000100111", "000000101000", "000001011000", "000001011001", "000000101011", "000000101100", "000001011010", "000001100110",
    "000001100111",
];
const BLACK_MAKEUP: [&str; 27] = [
    "0000001111", "000011001000", "000011001001", "000001011011", "000000110011", "000000110100", "000000110101", "0000001101100",
    "0000001101101", "0000001001010", "0000001001011", "0000001001100", "0000001001101", "0000001110010", "0000001110011",
    "0000001110100", "0000001110101", "0000001110110", "0000001110111", "0000001010010", "0000001010011", "0000001010100",
    "0000001010101", "0000001011010", "0000001011011", "0000001100100", "0000001100101",
];
/// the makeup codes from 1792 to 2560, the same for both colours
const EXT_MAKEUP: [&str; 13] = [
    "00000001000", "00000001100", "00000001101", "000000010010", "000000010011", "000000010100", "000000010101", "000000010110",
    "000000010111", "000000011100", "000000011101", "000000011110", "000000011111",
];

/// 13-bit lookup: (run length, code length); length 0 = no code
fn table(term: &[&str; 64], makeup: &[&str; 27]) -> Vec<(u16, u8)> {
    let mut t = vec![(0u16, 0u8); 1 << 13];
    let mut put = |code: &str, run: u16| {
        let len = code.len();
        let v = code.bytes().fold(0usize, |a, c| a << 1 | (c - b'0') as usize);
        let shift = 13 - len;
        for k in 0..(1 << shift) {
            t[(v << shift) | k] = (run, len as u8);
        }
    };
    for (i, c) in term.iter().enumerate() {
        put(c, i as u16);
    }
    for (i, c) in makeup.iter().enumerate() {
        put(c, 64 * (i as u16 + 1));
    }
    for (i, c) in EXT_MAKEUP.iter().enumerate() {
        put(c, 1792 + 64 * i as u16);
    }
    t
}

struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    fn peek(&self, n: usize) -> u32 {
        let mut v = 0u32;
        for i in 0..n {
            let p = self.pos + i;
            let b = self.d.get(p / 8).map_or(0, |&b| (b >> (7 - p % 8)) & 1);
            v = v << 1 | b as u32;
        }
        v
    }

    fn skip(&mut self, n: usize) {
        self.pos += n;
    }

    fn end(&self) -> bool {
        self.pos >= self.d.len() * 8
    }

    fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }

    /// an EOL here (fill zeros before it allowed): consumed
    fn eol(&mut self) -> bool {
        let mut p = self.pos;
        let mut zeros = 0;
        while p < self.d.len() * 8 {
            let b = (self.d[p / 8] >> (7 - p % 8)) & 1;
            p += 1;
            if b == 1 {
                if zeros >= 11 {
                    self.pos = p;
                    return true;
                }
                return false;
            }
            zeros += 1;
            if zeros > 4096 {
                return false;
            }
        }
        false
    }
}

struct Dec<'a> {
    b: Bits<'a>,
    white: Vec<(u16, u8)>,
    black: Vec<(u16, u8)>,
    cols: u32,
}

impl Dec<'_> {
    /// one run of a colour (makeup codes summed): None on a bad code
    fn run(&mut self, black: bool) -> Option<u32> {
        let mut total = 0u32;
        loop {
            let code = self.b.peek(13) as usize;
            let (run, len) = if black { self.black[code] } else { self.white[code] };
            if len == 0 {
                return None;
            }
            self.b.skip(len as usize);
            total = total.saturating_add(run as u32);
            if run < 64 {
                return Some(total);
            }
            if total > 1 << 20 {
                return None;
            }
        }
    }

    fn row_1d(&mut self, cur: &mut Vec<u32>) -> bool {
        let mut a0 = 0u32;
        let mut black = false;
        while a0 < self.cols {
            let Some(r) = self.run(black) else { return false };
            a0 = a0.saturating_add(r).min(self.cols);
            cur.push(a0);
            black = !black;
        }
        true
    }

    fn row_2d(&mut self, refl: &[u32], cur: &mut Vec<u32>) -> bool {
        let cols = self.cols;
        let mut a0: i64 = -1;
        let mut black = false;
        let mut guard = 0u32;
        // the first change on the reference line right of a0: a0 only grows, so the search goes on
        // from where it stood (a row costs its changes, not their square)
        let mut k = 0usize;
        while a0 < cols as i64 {
            guard += 1;
            if guard > 4 * cols + 64 {
                return false;
            }
            // b1: the first change on the reference line right of a0 that starts the other
            // colour (even indices start black runs), b2 the one after it
            while k < refl.len() && (refl[k] as i64) <= a0 {
                k += 1;
            }
            let mut i = k;
            while i < refl.len() && (i % 2 == 1) != black {
                i += 1;
            }
            let b1 = refl.get(i).copied().unwrap_or(cols) as i64;
            let b2 = refl.get(i + 1).copied().unwrap_or(cols) as i64;
            let p7 = self.b.peek(7);
            if p7 >> 6 == 1 {
                // V0
                self.b.skip(1);
                a0 = b1;
            } else if p7 >> 4 == 0b001 {
                // horizontal: two runs
                self.b.skip(3);
                let start = a0.max(0) as u32;
                let (Some(r1), Some(r2)) = (self.run(black), self.run(!black)) else { return false };
                let a1 = start.saturating_add(r1).min(cols);
                let a2 = a1.saturating_add(r2).min(cols);
                cur.push(a1);
                cur.push(a2);
                a0 = a2 as i64;
                continue;
            } else if p7 >> 3 == 0b0001 {
                // pass
                self.b.skip(4);
                a0 = b2;
                continue;
            } else if p7 >> 4 == 0b011 {
                self.b.skip(3);
                a0 = b1 + 1;
            } else if p7 >> 4 == 0b010 {
                self.b.skip(3);
                a0 = b1 - 1;
            } else if p7 >> 1 == 0b000011 {
                self.b.skip(6);
                a0 = b1 + 2;
            } else if p7 >> 1 == 0b000010 {
                self.b.skip(6);
                a0 = b1 - 2;
            } else if p7 == 0b0000011 {
                self.b.skip(7);
                a0 = b1 + 3;
            } else if p7 == 0b0000010 {
                self.b.skip(7);
                a0 = b1 - 3;
            } else {
                return false;
            }
            // a vertical mode: a change at a0
            a0 = a0.clamp(0, cols as i64);
            if cur.last().is_some_and(|&l| (l as i64) > a0) {
                return false;
            }
            cur.push(a0 as u32);
            black = !black;
        }
        true
    }
}

pub(super) fn decode(doc: &Document, data: &[u8], parms: &Dict, w: usize, h: usize) -> Option<Vec<u8>> {
    let k = doc.key(parms, "K").as_int().unwrap_or(0);
    let align = doc.key(parms, "EncodedByteAlign").as_bool() == Some(true);
    let black1 = doc.key(parms, "BlackIs1").as_bool() == Some(true);
    let cols = doc.key(parms, "Columns").as_int().unwrap_or(1728).clamp(1, 1 << 16) as u32;
    let rows = match doc.key(parms, "Rows").as_int() {
        Some(r) if r > 0 => (r as usize).min(h),
        _ => h,
    };
    let width = w.max(cols as usize);
    let stride = width.div_ceil(8);
    // white everywhere to begin with
    let white_byte = if black1 { 0x00 } else { 0xFF };
    let mut out = vec![white_byte; stride.checked_mul(h)?];
    let mut d = Dec { b: Bits { d: data, pos: 0 }, white: table(&WHITE_TERM, &WHITE_MAKEUP), black: table(&BLACK_TERM, &BLACK_MAKEUP), cols };
    let mut refl: Vec<u32> = vec![cols, cols];
    let mut cur: Vec<u32> = Vec::with_capacity(cols as usize + 4);
    for y in 0..rows {
        if align {
            d.b.align();
        }
        // an EOL (twice: the end of the block)
        let mut eols = 0;
        while d.b.eol() {
            eols += 1;
            if align && k >= 0 {
                // fill bits before the next EOL are byte aligned too
            }
            if eols > 1 {
                break;
            }
        }
        if eols > 1 || d.b.end() {
            break;
        }
        let two_d = match k {
            k if k < 0 => true,
            0 => false,
            _ => {
                let t = d.b.peek(1) == 0;
                d.b.skip(1);
                t
            }
        };
        cur.clear();
        let ok = if two_d { d.row_2d(&refl, &mut cur) } else { d.row_1d(&mut cur) };
        // the row: changes alternate white -> black -> white ...
        let row = &mut out[y * stride..(y + 1) * stride];
        let mut x0 = 0u32;
        let mut black = false;
        for &c in &cur {
            let c = c.min(cols);
            if black {
                for x in x0..c {
                    let (byte, bit) = ((x / 8) as usize, 7 - x % 8);
                    if black1 {
                        row[byte] |= 1 << bit;
                    } else {
                        row[byte] &= !(1 << bit);
                    }
                }
            }
            x0 = c;
            black = !black;
        }
        if !ok {
            break;
        }
        // the reference line for the next row, with two sentinels
        refl.clear();
        refl.extend(cur.iter().copied().filter(|&c| c < cols));
        // keep the alternation: a change at cols does not count
        refl.push(cols);
        refl.push(cols);
    }
    Some(out)
}
