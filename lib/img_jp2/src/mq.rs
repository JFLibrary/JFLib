// mq -- the MQ arithmetic decoder (ITU-T T.800 Annex C) and the raw bit reader of the bypassed
// coding passes (D.6)
//
// The decoder follows the flow charts of C.3 (INITDEC, BYTEIN, DECODE, RENORMD): a 0xFF byte
// followed by one above 0x8F is a marker, from where on the decoder is fed 1 bits (as if the data
// went on with 0xFF), so a segment can simply end; reads past the end of the slice give 0xFF.

/// the probability estimation table (Table C.2): Qe, the next index after an MPS, after an LPS,
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

/// a context: its index into the table and its more probable symbol
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cx {
    pub i: u8,
    pub mps: u8,
}

impl Cx {
    pub const fn new(i: u8) -> Cx {
        Cx { i, mps: 0 }
    }
}

pub(crate) struct Mq<'a> {
    data: &'a [u8],
    bp: usize,
    a: u32,
    c: u32,
    ct: u32,
}

impl<'a> Mq<'a> {
    /// INITDEC on the start of `data`
    pub fn new(data: &'a [u8]) -> Mq<'a> {
        let mut m = Mq { data, bp: 0, a: 0x8000, c: 0, ct: 0 };
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
                // a marker (or the end): 1 bits from here on
                self.c = self.c.wrapping_add(0xFF00);
                self.ct = 8;
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

    /// DECODE: one decision in context `cx`
    #[inline(always)]
    pub fn decode(&mut self, cx: &mut Cx) -> u32 {
        let (qe, nmps, nlps, switch) = QE[cx.i as usize];
        self.a -= qe;
        let d;
        if (self.c >> 16) < qe {
            // LPS_EXCHANGE
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
                // MPS_EXCHANGE
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

/// the raw bits of a bypassed pass (D.6): MSB first, after a 0xFF byte only 7 bits of the next
/// (its MSB is a stuffed 0); at a marker or the end, 1 bits
pub(crate) struct Raw<'a> {
    data: &'a [u8],
    bp: usize,
    c: u32,
    ct: u32,
}

impl<'a> Raw<'a> {
    pub fn new(data: &'a [u8]) -> Raw<'a> {
        Raw { data, bp: 0, c: 0, ct: 0 }
    }

    #[inline(always)]
    pub fn bit(&mut self) -> u32 {
        if self.ct == 0 {
            let b = self.data.get(self.bp).copied().unwrap_or(0xFF) as u32;
            if self.c == 0xFF {
                if b > 0x8F {
                    self.c = 0xFF;
                    self.ct = 8;
                } else {
                    self.c = b;
                    self.bp += 1;
                    self.ct = 7;
                }
            } else {
                self.c = b;
                self.bp += 1;
                self.ct = 8;
            }
        }
        self.ct -= 1;
        (self.c >> self.ct) & 1
    }
}
