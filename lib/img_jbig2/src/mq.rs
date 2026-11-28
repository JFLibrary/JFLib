// mq -- the MQ arithmetic decoder (ITU-T T.88 Annex E, the coder of JBIG2 and JPEG 2000) and the
// integer decoding procedures of Annex A (IAx, IAID)
//
// A 0xFF byte followed by one above 0x8F is a marker, from where on the decoder is fed 1 bits, so a
// region's data can simply end; reads past the end of the slice give 0xFF.  Encoders may leave out
// the bytes at the end that equal this fill, so a bitmap's last rows may legitimately come from it
// (SerenityOS's bitmap-trailing-7fff-stripped-harder).  The integers of symbol dictionaries and
// text regions do not compress that far: a decoder fed far more fill than a flushed coder leaves
// is `exhausted` there, its data was cut or claims more symbols or instances than it holds, and
// those loops stop instead of decoding up to the work budget from nothing.

/// the probability estimation table (Table E.1): Qe, the next index after an MPS, after an LPS,
/// and whether an LPS swaps the MPS
const QE: [(u32, u8, u8, bool); 47] = [
    (0x5601, 1, 1, true),
    (0x3401, 2, 6, false),
    (0x1801, 3, 9, false),
    (0x0AC1, 4, 12, false),
    (0x0521, 5, 29, false),
    (0x0221, 38, 33, false),
    (0x5601, 7, 6, true),
    (0x5401, 8, 14, false),
    (0x4801, 9, 14, false),
    (0x3801, 10, 14, false),
    (0x3001, 11, 17, false),
    (0x2401, 12, 18, false),
    (0x1C01, 13, 20, false),
    (0x1601, 29, 21, false),
    (0x5601, 15, 14, true),
    (0x5401, 16, 14, false),
    (0x5101, 17, 15, false),
    (0x4801, 18, 16, false),
    (0x3801, 19, 17, false),
    (0x3401, 20, 18, false),
    (0x3001, 21, 19, false),
    (0x2801, 22, 19, false),
    (0x2401, 23, 20, false),
    (0x2201, 24, 21, false),
    (0x1C01, 25, 22, false),
    (0x1801, 26, 23, false),
    (0x1601, 27, 24, false),
    (0x1401, 28, 25, false),
    (0x1201, 29, 26, false),
    (0x1101, 30, 27, false),
    (0x0AC1, 31, 28, false),
    (0x09C1, 32, 29, false),
    (0x08A1, 33, 30, false),
    (0x0521, 34, 31, false),
    (0x0441, 35, 32, false),
    (0x02A1, 36, 33, false),
    (0x0221, 37, 34, false),
    (0x0141, 38, 35, false),
    (0x0111, 39, 36, false),
    (0x0085, 40, 37, false),
    (0x0049, 41, 38, false),
    (0x0025, 42, 39, false),
    (0x0015, 43, 40, false),
    (0x0009, 44, 41, false),
    (0x0005, 45, 42, false),
    (0x0001, 45, 43, false),
    (0x5601, 46, 46, false),
];

/// a context: its index into the table and its more probable symbol (all start at 0, 0)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Cx {
    pub i: u8,
    pub mps: u8,
}

/// bytes of fill (past the end or a marker) before the data of a symbol dictionary or text region
/// counts as used up; a coder's FLUSH leaves the decoder two or three
const PAST_MAX: u32 = 32;

pub(crate) struct Mq<'a> {
    data: &'a [u8],
    bp: usize,
    a: u32,
    c: u32,
    ct: u32,
    /// the bytes of fill fed so far
    past: u32,
}

impl<'a> Mq<'a> {
    /// INITDEC on the start of `data`
    pub fn new(data: &'a [u8]) -> Mq<'a> {
        let mut m = Mq { data, bp: 0, a: 0x8000, c: 0, ct: 0, past: 0 };
        m.c = (m.byte(0) as u32) << 16;
        m.bytein();
        m.c = m.c.wrapping_shl(7);
        m.ct = m.ct.saturating_sub(7);
        m.a = 0x8000;
        m
    }

    #[inline(always)]
    fn byte(&self, i: usize) -> u8 {
        match self.data.get(i) {
            Some(&b) => b,
            None => 0xFF,
        }
    }

    #[inline(always)]
    fn bytein(&mut self) {
        if self.byte(self.bp) == 0xFF {
            let b1 = self.byte(self.bp + 1);
            if b1 > 0x8F {
                self.c = self.c.wrapping_add(0xFF00);
                self.ct = 8;
                self.past = self.past.saturating_add(1);
            } else {
                self.bp += 1;
                self.c = self.c.wrapping_add((b1 as u32) << 9);
                self.ct = 7;
            }
        } else {
            self.bp += 1;
            self.c = self.c.wrapping_add((self.byte(self.bp) as u32) << 8);
            self.ct = 8;
        }
    }

    #[inline(always)]
    fn renorm(&mut self) {
        loop {
            if self.ct == 0 {
                self.bytein();
            }
            self.a <<= 1;
            self.c = self.c.wrapping_shl(1);
            self.ct -= 1;
            if self.a & 0x8000 != 0 {
                break;
            }
        }
    }

    /// whether the decoder was fed far more fill than a flushed coder leaves (see the top)
    pub fn exhausted(&self) -> bool {
        self.past > PAST_MAX
    }

    /// DECODE: one decision in context `cx`
    #[inline(always)]
    pub fn decode(&mut self, cx: &mut Cx) -> u32 {
        let (qe, nmps, nlps, switch) = QE[cx.i as usize];
        self.a -= qe;
        let d;
        if (self.c >> 16) < qe {
            if self.a < qe {
                d = cx.mps;
                cx.i = nmps;
            } else {
                d = cx.mps ^ 1;
                if switch {
                    cx.mps ^= 1;
                }
                cx.i = nlps;
            }
            self.a = qe;
            self.renorm();
        } else {
            self.c -= qe << 16;
            if self.a & 0x8000 == 0 {
                if self.a < qe {
                    d = cx.mps ^ 1;
                    if switch {
                        cx.mps ^= 1;
                    }
                    cx.i = nlps;
                } else {
                    d = cx.mps;
                    cx.i = nmps;
                }
                self.renorm();
            } else {
                d = cx.mps;
            }
        }
        d as u32
    }
}

/// the 512 contexts of one integer decoding procedure (IADH, IADW, ...)
pub(crate) struct IntCx(pub [Cx; 512]);

impl IntCx {
    pub fn new() -> IntCx {
        IntCx([Cx::default(); 512])
    }
}

/// A.2: one integer, None for OOB
pub(crate) fn decode_int(mq: &mut Mq, cx: &mut IntCx) -> Option<i32> {
    let mut prev = 1usize;
    let mut bit = |mq: &mut Mq, prev: &mut usize| -> u32 {
        let d = mq.decode(&mut cx.0[*prev]);
        *prev = if *prev < 256 { (*prev << 1) | d as usize } else { (((*prev << 1) | d as usize) & 511) | 256 };
        d
    };
    let s = bit(mq, &mut prev);
    let (n, off) = if bit(mq, &mut prev) == 0 {
        (2, 0)
    } else if bit(mq, &mut prev) == 0 {
        (4, 4)
    } else if bit(mq, &mut prev) == 0 {
        (6, 20)
    } else if bit(mq, &mut prev) == 0 {
        (8, 84)
    } else if bit(mq, &mut prev) == 0 {
        (12, 340)
    } else {
        (32, 4436)
    };
    let mut v: u64 = 0;
    for _ in 0..n {
        v = (v << 1) | bit(mq, &mut prev) as u64;
    }
    let v = (v + off) as i64;
    if s == 1 && v == 0 {
        return None;
    }
    let v = if s == 1 { -v } else { v };
    Some(v.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
}

/// A.3: a symbol id of `len` bits (IAID; `cx` has 2^(len+1) contexts)
pub(crate) fn decode_id(mq: &mut Mq, cx: &mut [Cx], len: u32) -> u32 {
    let mut prev = 1usize;
    for _ in 0..len {
        let d = mq.decode(&mut cx[prev]);
        prev = (prev << 1) | d as usize;
    }
    (prev - (1usize << len)) as u32
}
