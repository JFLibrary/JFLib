// bits -- JPEG XL bit reader and field coding (fields.cc; model: tools/jxl_ref.py Bits)
//                                                                       [src/img/jxl_bits.s]
//
//   Jb::init(buf, len)          reader over buf (the codestream copy incl. its JXB_PAD zero
//                               bytes), section end = len bytes
//   Jb::section(buf, start, size)   reader over bytes start .. start + size of buf
//   f(n) -> u32                 n bits (0..32), LSB first; zero bits past the buffer
//   u32f(dist) -> u32           U32 field: dist = 4 x (offset, bits), bits 0 = the value
//   u64f() -> u64               U64 field
//   f16() -> u32                F16 field as f32 bits (inf / NaN sets err)
//   enum_() -> u32              Enum field
//   align()                     skip to the byte boundary (nonzero padding sets err)
//   ext() -> u64                extensions bundle: skips the extension bits, returns the mask
//   check() -> 0 / ERR_*        the sticky error, or ERR_TRUNC when read past the section end
//
// The position is a wrapping u64 like the assembly's (jb_ext saturates it at u64::MAX).

use img_core::{ERR_HDR, ERR_TRUNC};

/// zero bytes behind the codestream copy (JXB_PAD)
pub const JXB_PAD: usize = 16;

/// the bit reader (JB)
#[derive(Clone, Copy)]
pub struct Jb<'a> {
    pub buf: &'a [u8], // codestream copy incl. the padding
    pub pos: u64,      // bit position
    pub lim: u64,      // last byte index an 8-byte load may start at
    pub end: u64,      // end of the current section, bits
    pub err: i32,      // sticky error (0 = fine)
}

impl Default for Jb<'_> {
    fn default() -> Self {
        Jb { buf: &[], pos: 0, lim: 0, end: 0, err: 0 }
    }
}

/// Enum: 0, 1, 2 + u(4), 18 + u(6)  [jb_denum]
pub static DENUM: [u32; 8] = [0, 0, 1, 0, 2, 4, 18, 6];

impl<'a> Jb<'a> {
    /// jb_init: reader over buf (incl. padding), section end = len bytes
    pub fn init(buf: &'a [u8], len: u64) -> Self {
        Jb { buf, pos: 0, lim: (buf.len() as u64).wrapping_sub(8), end: len << 3, err: 0 }
    }

    /// jb_section: reader over bytes start .. start + size of buf
    pub fn section(buf: &'a [u8], start: u64, size: u64) -> Self {
        let pos = start << 3;
        Jb { buf, pos, lim: (buf.len() as u64).wrapping_sub(8), end: (size << 3).wrapping_add(pos), err: 0 }
    }

    /// jb_check: 0, the sticky error, or ERR_TRUNC (read past the section end)
    pub fn check(&self) -> i32 {
        if self.err != 0 {
            self.err
        } else if self.pos > self.end {
            ERR_TRUNC
        } else {
            0
        }
    }

    /// the 64 bits at the current position (LSB first; zero past the buffer)
    #[inline(always)]
    pub fn peek64(&self) -> u64 {
        let i = self.pos >> 3;
        if i > self.lim {
            return 0;
        }
        let i = i as usize;
        let v = u64::from_le_bytes([
            self.buf[i],
            self.buf[i + 1],
            self.buf[i + 2],
            self.buf[i + 3],
            self.buf[i + 4],
            self.buf[i + 5],
            self.buf[i + 6],
            self.buf[i + 7],
        ]);
        v >> (self.pos & 7)
    }

    /// jb_f: n bits (0..32)
    #[inline(always)]
    pub fn f(&mut self, n: u32) -> u32 {
        let v = self.peek64();
        self.pos = self.pos.wrapping_add(n as u64);
        (v & ((1u64 << (n & 63)).wrapping_sub(1))) as u32
    }

    /// jb_u32: U32 field
    pub fn u32f(&mut self, dist: &[u32; 8]) -> u32 {
        let s = self.f(2) as usize;
        let bits = dist[s * 2 + 1];
        if bits == 0 { dist[s * 2] } else { self.f(bits).wrapping_add(dist[s * 2]) }
    }

    /// jb_u64: U64 field
    pub fn u64f(&mut self) -> u64 {
        match self.f(2) {
            0 => 0,
            1 => self.f(4) as u64 + 1,
            2 => self.f(8) as u64 + 17,
            _ => {
                let mut v = self.f(12) as u64;
                let mut sh = 12u32;
                while self.f(1) != 0 {
                    if sh == 60 {
                        v |= (self.f(4) as u64) << 60;
                        break;
                    }
                    v |= (self.f(8) as u64) << sh;
                    sh += 8;
                }
                v
            }
        }
    }

    /// jb_f16: F16 field as f32 bits (inf / NaN: err = ERR_HDR, 0)
    pub fn f16(&mut self) -> u32 {
        let v = self.f(16);
        let e = (v >> 10) & 31;
        let m = v & 1023;
        let s = (v & 0x8000) << 16;
        if e == 31 {
            self.err = ERR_HDR;
            return 0;
        }
        if e == 0 {
            // mantissa * 2^-24, exact in f32
            return (m as f32 * f32::from_bits(0x3380_0000)).to_bits() | s;
        }
        (((e + 127 - 15) << 23) + (m << 13)) | s
    }

    /// jb_f16 as f32
    pub fn f16f(&mut self) -> f32 {
        f32::from_bits(self.f16())
    }

    /// jb_enum
    pub fn enum_(&mut self) -> u32 {
        self.u32f(&DENUM)
    }

    /// jb_align: skip to the byte boundary (nonzero padding: err = ERR_HDR)
    pub fn align(&mut self) {
        let n = (self.pos as u32).wrapping_neg() & 7;
        if n != 0 && self.f(n) != 0 {
            self.err = ERR_HDR;
        }
    }

    /// jb_ext: extensions bundle; skips the extension bits, returns the mask
    pub fn ext(&mut self) -> u64 {
        let mask = self.u64f();
        if mask == 0 {
            return 0;
        }
        let mut total: u64 = 0;
        for i in 0..64 {
            if mask >> i & 1 != 0 {
                let n = self.u64f();
                total = total.checked_add(n).unwrap_or(u64::MAX);
            }
        }
        self.pos = self.pos.checked_add(total).unwrap_or(u64::MAX);
        mask
    }
}
