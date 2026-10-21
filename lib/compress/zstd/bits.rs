// bits -- the bit streams of zstd
//
//   Writer      forward, least significant bit first; close() adds the end marker (a 1 bit)
//               that the backward reader starts from (FSE, Huffman and sequence streams)
//   BackReader  reads such a stream from its end marker towards the start; reading past the
//               start yields zero bits and leaves pos() negative (FSE weight streams end that way)
//   FwdReader   forward, least significant bit first (FSE table descriptions)

use alloc::vec::Vec;

#[inline(always)]
fn mask(n: u32) -> u64 {
    if n >= 64 { !0 } else { (1u64 << n) - 1 }
}

/// little-endian 64 bits from byte i (zero beyond the end)
#[inline(always)]
fn load64(b: &[u8], i: usize) -> u64 {
    if i + 8 <= b.len() {
        u64::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3], b[i + 4], b[i + 5], b[i + 6], b[i + 7]])
    } else {
        let mut v = 0u64;
        for k in 0..8 {
            if let Some(&x) = b.get(i + k) {
                v |= (x as u64) << (8 * k);
            }
        }
        v
    }
}

pub struct Writer {
    pub out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Writer {
    pub fn new() -> Writer {
        Writer { out: Vec::new(), acc: 0, n: 0 }
    }

    /// append the low `bits` bits of v (bits <= 32)
    #[inline(always)]
    pub fn add(&mut self, v: u64, bits: u32) {
        if bits == 0 {
            return;
        }
        self.acc |= (v & mask(bits)) << self.n;
        self.n += bits;
        if self.n >= 32 {
            self.out.extend_from_slice(&(self.acc as u32).to_le_bytes());
            self.acc >>= 32;
            self.n -= 32;
        }
    }

    /// the bytes so far, the last one padded with zero bits
    pub fn finish(mut self) -> Vec<u8> {
        while self.n > 0 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n = self.n.saturating_sub(8);
        }
        self.out
    }

    /// the end marker, then finish
    pub fn close(mut self) -> Vec<u8> {
        self.add(1, 1);
        self.finish()
    }
}

pub struct BackReader<'a> {
    b: &'a [u8],
    /// bits not read yet (negative: read past the start)
    pos: isize,
}

impl<'a> BackReader<'a> {
    /// None for an empty stream or a last byte without the end marker
    pub fn new(b: &'a [u8]) -> Option<BackReader<'a>> {
        let last = *b.last()?;
        if last == 0 {
            return None;
        }
        let hb = 7 - last.leading_zeros() as isize;
        Some(BackReader { b, pos: (b.len() as isize - 1) * 8 + hb })
    }

    #[inline(always)]
    fn bits_at(&self, start: isize, n: u32) -> u64 {
        if start >= 0 {
            let s = start as usize;
            (load64(self.b, s / 8) >> (s % 8)) & mask(n)
        } else {
            let avail = start + n as isize;
            if avail <= 0 { 0 } else { (load64(self.b, 0) & mask(avail as u32)) << (-start) as u32 }
        }
    }

    /// the next n bits (n <= 56), most significant first in the value
    #[inline(always)]
    pub fn read(&mut self, n: u32) -> u64 {
        if n == 0 {
            return 0;
        }
        self.pos -= n as isize;
        self.bits_at(self.pos, n)
    }

    #[inline(always)]
    pub fn peek(&self, n: u32) -> u64 {
        if n == 0 { 0 } else { self.bits_at(self.pos - n as isize, n) }
    }

    #[inline(always)]
    pub fn skip(&mut self, n: u32) {
        self.pos -= n as isize;
    }

    pub fn pos(&self) -> isize {
        self.pos
    }
}

pub struct FwdReader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> FwdReader<'a> {
    pub fn new(b: &'a [u8]) -> FwdReader<'a> {
        FwdReader { b, pos: 0 }
    }

    pub fn peek(&self, n: u32) -> u64 {
        (load64(self.b, self.pos / 8) >> (self.pos % 8)) & mask(n)
    }

    pub fn skip(&mut self, n: u32) {
        self.pos += n as usize;
    }

    pub fn read(&mut self, n: u32) -> u64 {
        let v = self.peek(n);
        self.skip(n);
        v
    }

    /// whole bytes touched so far
    pub fn bytes(&self) -> usize {
        self.pos.div_ceil(8)
    }
}
