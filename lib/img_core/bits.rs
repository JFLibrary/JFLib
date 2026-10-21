// bits -- the bit readers of the codecs (MSB-first and LSB-first)  [ASMLIB/inc/img_int.inc]
//
//   BrM (MSB first: JPEG entropy data without stuffing, HEVC, AV1 headers, JXR)
//   BrL (LSB first: deflate, VP8L, JPEG XL)
//   r = BrX::new(data, pos); r.refill(); r.get(n) / r.peek(n) / r.skip(n); r.overrun()
//
// The state of the assembly macros: a 64-bit buffer, a bit count and a byte position that
// keeps counting past the end of the data (zero bytes are shifted in there, nothing is
// read).  After refill() at least 56 bits are available.  overrun() is BRx_OVERRUN: more
// zero padding bytes consumed than bits are left.  Values of n: 1..56 (get / peek / skip).

/// MSB-first reader (BRM_*)
#[derive(Clone, Copy)]
pub struct BrM<'a> {
    pub data: &'a [u8],
    pub buf: u64,
    pub cnt: u32,
    pub pos: usize, // may pass data.len()
}

impl<'a> BrM<'a> {
    /// BRM_INIT with the byte position pos
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        BrM { data, buf: 0, cnt: 0, pos }
    }

    /// BRM_REFILL: at least 56 bits in the buffer
    #[inline]
    pub fn refill(&mut self) {
        if self.cnt >= 56 {
            return;
        }
        let end = self.data.len();
        if self.pos + 8 <= end {
            let v = u64::from_be_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
            self.buf |= v >> self.cnt;
            self.pos += ((63 - self.cnt) >> 3) as usize;
            self.cnt |= 56;
            return;
        }
        while self.cnt <= 56 {
            let b = if self.pos < end { self.data[self.pos] as u64 } else { 0 };
            self.pos += 1;
            self.buf |= b << (56 - self.cnt);
            self.cnt += 8;
        }
    }

    /// BRM_PEEK: the next n bits
    #[inline]
    pub fn peek(&self, n: u32) -> u64 {
        self.buf.wrapping_shr(64 - n) // shr: the count is masked (n = 0 gives the buffer)
    }

    /// BRM_SKIP
    #[inline]
    pub fn skip(&mut self, n: u32) {
        self.buf = self.buf.wrapping_shl(n);
        self.cnt = self.cnt.wrapping_sub(n);
    }

    /// BRM_GET / BRM_GETV
    #[inline]
    pub fn get(&mut self, n: u32) -> u64 {
        let v = self.peek(n);
        self.skip(n);
        v
    }

    /// BRM_OVERRUN: true when more padding bytes were consumed than bits are left
    pub fn overrun(&self) -> bool {
        let end = self.data.len();
        self.pos > end && ((self.pos - end) as u64) * 8 > self.cnt as u64
    }
}

/// LSB-first reader (BRL_*)
#[derive(Clone, Copy)]
pub struct BrL<'a> {
    pub data: &'a [u8],
    pub buf: u64,
    pub cnt: u32,
    pub pos: usize, // may pass data.len()
}

impl<'a> BrL<'a> {
    /// BRL_INIT with the byte position pos
    pub fn new(data: &'a [u8], pos: usize) -> Self {
        BrL { data, buf: 0, cnt: 0, pos }
    }

    /// BRL_REFILL: at least 56 bits in the buffer
    #[inline]
    pub fn refill(&mut self) {
        if self.cnt >= 56 {
            return;
        }
        let end = self.data.len();
        if self.pos + 8 <= end {
            let v = u64::from_le_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
            self.buf |= v << self.cnt;
            self.pos += ((63 - self.cnt) >> 3) as usize;
            self.cnt |= 56;
            return;
        }
        while self.cnt <= 56 {
            let b = if self.pos < end { self.data[self.pos] as u64 } else { 0 };
            self.pos += 1;
            self.buf |= b << self.cnt;
            self.cnt += 8;
        }
    }

    /// BRL_PEEK: the next n bits
    #[inline]
    pub fn peek(&self, n: u32) -> u64 {
        self.buf & 1u64.wrapping_shl(n).wrapping_sub(1)
    }

    /// BRL_SKIP
    #[inline]
    pub fn skip(&mut self, n: u32) {
        self.buf = self.buf.wrapping_shr(n);
        self.cnt = self.cnt.wrapping_sub(n);
    }

    /// BRL_GET / BRL_GETV
    #[inline]
    pub fn get(&mut self, n: u32) -> u64 {
        let v = self.peek(n);
        self.skip(n);
        v
    }

    /// BRL_OVERRUN
    pub fn overrun(&self) -> bool {
        let end = self.data.len();
        self.pos > end && ((self.pos - end) as u64) * 8 > self.cnt as u64
    }
}
