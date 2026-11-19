// bits -- a bit stream for the netcode's messages: values of any width up to 32 bits, little-endian
// bit order (the first bit written is bit 0 of byte 0), and two variable-length codes
//
//   BitWriter   bits(value, n), bool, uvar (counts, gaps: 0 + 4 | 10 + 8 | 110 + 16 | 111 + 32 bits),
//               svar (field deltas, zigzag: 0 + 6 | 10 + 10 | 110 + 16 | 111 + 32 bits), u8 / u16 / u32,
//               bit_len, finish
//   BitReader   the same reads; past the end or on a value out of range it returns 0 and sets a sticky
//               error flag (ok() false): bytes from the network never panic, the caller checks once
//
// The codes are sized for a game's snapshots: an id gap is mostly 0 (5 bits), a position delta of a
// walking figure between two snapshots (a few hundred steps of 1/1024 m) takes 12 bits, an unchanged
// field is not written at all (the field mask of the snapshot says which are).

use alloc::vec::Vec;

/// writes bits into bytes (see the module header)
#[derive(Clone, Debug, Default)]
pub struct BitWriter {
    buf: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    /// an empty stream
    pub fn new() -> BitWriter {
        BitWriter::default()
    }

    /// an empty stream after `prefix` (whole bytes, such as a message type)
    pub fn with_prefix(prefix: &[u8]) -> BitWriter {
        BitWriter { buf: prefix.to_vec(), acc: 0, n: 0 }
    }

    /// the low `n` bits of `v` (n at most 32)
    pub fn bits(&mut self, v: u32, n: u32) {
        debug_assert!(n <= 32);
        if n == 0 {
            return;
        }
        let v = if n == 32 { v as u64 } else { (v & ((1u32 << n) - 1)) as u64 };
        self.acc |= v << self.n;
        self.n += n;
        while self.n >= 8 {
            self.buf.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// one bit
    pub fn bool(&mut self, b: bool) {
        self.bits(b as u32, 1);
    }

    /// 8 bits
    pub fn u8(&mut self, v: u8) {
        self.bits(v as u32, 8);
    }

    /// 16 bits
    pub fn u16(&mut self, v: u16) {
        self.bits(v as u32, 16);
    }

    /// 32 bits
    pub fn u32(&mut self, v: u32) {
        self.bits(v, 32);
    }

    /// an unsigned number in the code for counts and gaps: 0 + 4 bits, 10 + 8, 110 + 16, 111 + 32
    pub fn uvar(&mut self, v: u32) {
        if v < 1 << 4 {
            self.bits(0, 1);
            self.bits(v, 4);
        } else if v < 1 << 8 {
            self.bits(0b01, 2);
            self.bits(v, 8);
        } else if v < 1 << 16 {
            self.bits(0b011, 3);
            self.bits(v, 16);
        } else {
            self.bits(0b111, 3);
            self.bits(v, 32);
        }
    }

    /// a signed difference in the code for field deltas (zigzag: 0, -1, 1, -2, ...): 0 + 6 bits (-32 ..= 31),
    /// 10 + 10 (-512 ..= 511), 110 + 16, 111 + 32
    pub fn svar(&mut self, d: i32) {
        let z = zigzag(d);
        if z < 1 << 6 {
            self.bits(0, 1);
            self.bits(z, 6);
        } else if z < 1 << 10 {
            self.bits(0b01, 2);
            self.bits(z, 10);
        } else if z < 1 << 16 {
            self.bits(0b011, 3);
            self.bits(z, 16);
        } else {
            self.bits(0b111, 3);
            self.bits(z, 32);
        }
    }

    /// bits written so far (prefix included)
    pub fn bit_len(&self) -> usize {
        self.buf.len() * 8 + self.n as usize
    }

    /// the bytes, the last one filled up with zero bits
    pub fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.buf.push(self.acc as u8);
        }
        self.buf
    }
}

/// the bits uvar takes for v
pub fn uvar_bits(v: u32) -> usize {
    if v < 1 << 4 {
        5
    } else if v < 1 << 8 {
        10
    } else if v < 1 << 16 {
        19
    } else {
        35
    }
}

/// the bits svar takes for d
pub fn svar_bits(d: i32) -> usize {
    let z = zigzag(d);
    if z < 1 << 6 {
        7
    } else if z < 1 << 10 {
        12
    } else if z < 1 << 16 {
        19
    } else {
        35
    }
}

/// 0, -1, 1, -2, 2 ... -> 0, 1, 2, 3, 4 ...
pub fn zigzag(d: i32) -> u32 {
    ((d << 1) ^ (d >> 31)) as u32
}

/// the inverse of zigzag
pub fn unzigzag(z: u32) -> i32 {
    ((z >> 1) as i32) ^ -((z & 1) as i32)
}

/// reads bits from bytes (see the module header)
#[derive(Clone, Debug)]
pub struct BitReader<'a> {
    buf: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
    bad: bool,
}

impl<'a> BitReader<'a> {
    /// a stream over `buf`
    pub fn new(buf: &'a [u8]) -> BitReader<'a> {
        BitReader { buf, pos: 0, acc: 0, n: 0, bad: false }
    }

    /// `n` bits (at most 32); 0 and the error flag past the end
    pub fn bits(&mut self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        if n == 0 {
            return 0;
        }
        while self.n < n {
            if self.pos >= self.buf.len() {
                self.bad = true;
                return 0;
            }
            self.acc |= (self.buf[self.pos] as u64) << self.n;
            self.pos += 1;
            self.n += 8;
        }
        let v = if n == 32 { self.acc as u32 } else { (self.acc as u32) & ((1u32 << n) - 1) };
        self.acc >>= n;
        self.n -= n;
        v
    }

    /// one bit
    pub fn bool(&mut self) -> bool {
        self.bits(1) != 0
    }

    /// 8 bits
    pub fn u8(&mut self) -> u8 {
        self.bits(8) as u8
    }

    /// 16 bits
    pub fn u16(&mut self) -> u16 {
        self.bits(16) as u16
    }

    /// 32 bits
    pub fn u32(&mut self) -> u32 {
        self.bits(32)
    }

    /// BitWriter::uvar
    pub fn uvar(&mut self) -> u32 {
        if !self.bool() {
            return self.bits(4);
        }
        if !self.bool() {
            return self.bits(8);
        }
        if !self.bool() {
            return self.bits(16);
        }
        self.bits(32)
    }

    /// BitWriter::svar
    pub fn svar(&mut self) -> i32 {
        let z = if !self.bool() {
            self.bits(6)
        } else if !self.bool() {
            self.bits(10)
        } else if !self.bool() {
            self.bits(16)
        } else {
            self.bits(32)
        };
        unzigzag(z)
    }

    /// mark the stream as bad (a value the caller found out of range)
    pub fn fail(&mut self) {
        self.bad = true;
    }

    /// no read went past the end and nothing was marked bad
    pub fn ok(&self) -> bool {
        !self.bad
    }

    /// whole bytes not yet touched (the bits of a started byte do not count)
    pub fn bytes_left(&self) -> usize {
        self.buf.len() - self.pos
    }
}
