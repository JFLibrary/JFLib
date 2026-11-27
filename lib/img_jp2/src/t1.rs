// t1 -- the coefficient bit modelling of a code-block (ITU-T T.800 Annex D, EBCOT tier 1)
//
// The passes of a bit-plane: significance propagation, magnitude refinement, cleanup (with the
// run-length mode of four empty samples in a column of a stripe), the first bit-plane only with a
// cleanup pass.  The code-block styles: selective arithmetic coding bypass (from the 11th pass on
// the significance and refinement passes are raw bits), reset of the contexts after each pass,
// termination after each pass (every pass a segment of its own), vertically causal contexts (the
// last row of a stripe does not look into the next one), predictable termination (nothing to do
// for a decoder) and segmentation symbols (four after each cleanup pass, read and ignored).
//
// Every sample keeps the significance and sign of its eight neighbours in its own flags (set when
// a neighbour becomes significant), so a context is a table lookup.  The coefficients come out in
// half units: a sample that becomes significant in bit-plane p is 1.5 * 2^p, a refinement moves it
// by a quarter of the interval, so the value is always in the middle of what is known (an image
// with every pass decoded has its exact integers at half of these values).

use crate::mq::{Cx, Mq, Raw};
use alloc::vec::Vec;

pub(crate) const BYPASS: u8 = 1;
pub(crate) const RESET: u8 = 2;
pub(crate) const TERMALL: u8 = 4;
pub(crate) const VSC: u8 = 8;
pub(crate) const SEGMARK: u8 = 32;

// the flags of a sample: the significance of its neighbours, the signs of the four direct ones,
// its own state
const N: u16 = 1;
const S: u16 = 2;
const W: u16 = 4;
const E: u16 = 8;
const NW: u16 = 16;
const NE: u16 = 32;
const SW: u16 = 64;
const SE: u16 = 128;
const NS: u16 = 1 << 8;
const SS: u16 = 1 << 9;
const WS: u16 = 1 << 10;
const ES: u16 = 1 << 11;
const SIG: u16 = 1 << 12;
const VISIT: u16 = 1 << 13;
const REF: u16 = 1 << 14;
const NEG: u16 = 1 << 15;
/// what the last row of a stripe does not see with vertically causal contexts
const BELOW: u16 = S | SW | SE | SS;

const CX_RUN: usize = 17;
const CX_UNI: usize = 18;

/// the significance context (Table D.1) of the eight neighbour bits, per band orientation
/// (0 LL, 1 HL, 2 LH, 3 HH)
const fn sig_ctx(orient: usize, nb: usize) -> u8 {
    let mut h = ((nb >> 2) & 1) + ((nb >> 3) & 1);
    let mut v = (nb & 1) + ((nb >> 1) & 1);
    let d = ((nb >> 4) & 1) + ((nb >> 5) & 1) + ((nb >> 6) & 1) + ((nb >> 7) & 1);
    if orient == 3 {
        let hv = h + v;
        return match d {
            0 => match hv {
                0 => 0,
                1 => 1,
                _ => 2,
            },
            1 => match hv {
                0 => 3,
                1 => 4,
                _ => 5,
            },
            2 => {
                if hv == 0 {
                    6
                } else {
                    7
                }
            }
            _ => 8,
        };
    }
    if orient == 1 {
        let t = h;
        h = v;
        v = t;
    }
    match h {
        2 => 8,
        1 => {
            if v >= 1 {
                7
            } else if d >= 1 {
                6
            } else {
                5
            }
        }
        _ => match v {
            2 => 4,
            1 => 3,
            _ => match d {
                0 => 0,
                1 => 1,
                _ => 2,
            },
        },
    }
}

const fn sig_table() -> [[u8; 256]; 4] {
    let mut t = [[0u8; 256]; 4];
    let mut o = 0;
    while o < 4 {
        let mut i = 0;
        while i < 256 {
            t[o][i] = sig_ctx(o, i);
            i += 1;
        }
        o += 1;
    }
    t
}

/// the contribution of one neighbour: 0 when not significant, else +1 or -1 by its sign
const fn contrib(i: usize, sig: usize, neg: usize) -> i32 {
    if (i >> sig) & 1 == 0 {
        0
    } else if (i >> neg) & 1 == 1 {
        -1
    } else {
        1
    }
}

/// the sign context and its XOR bit (Table D.3) from the significance (bits 0..3: N, S, W, E) and
/// signs (bits 4..7) of the four direct neighbours
const fn sign_table() -> [(u8, u8); 256] {
    let mut t = [(0u8, 0u8); 256];
    let mut i = 0;
    while i < 256 {
        let hs = contrib(i, 2, 6) + contrib(i, 3, 7);
        let vs = contrib(i, 0, 4) + contrib(i, 1, 5);
        let h = if hs > 0 { 1 } else if hs < 0 { -1 } else { 0 };
        let v = if vs > 0 { 1 } else if vs < 0 { -1 } else { 0 };
        t[i] = match (h, v) {
            (1, 1) => (13, 0),
            (1, 0) => (12, 0),
            (1, _) => (11, 0),
            (0, 1) => (10, 0),
            (0, 0) => (9, 0),
            (0, _) => (10, 1),
            (_, 1) => (11, 1),
            (_, 0) => (12, 1),
            _ => (13, 1),
        };
        i += 1;
    }
    t
}

static SIG_LUT: [[u8; 256]; 4] = sig_table();
static SIGN_LUT: [(u8, u8); 256] = sign_table();

/// one codeword segment of a code-block: its bytes and how many passes it holds
pub(crate) struct Seg<'a> {
    pub data: &'a [u8],
    pub passes: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    Sig,
    Ref,
    Clean,
}

fn pass_kind(idx: u32) -> Pass {
    if idx == 0 {
        return Pass::Clean;
    }
    match (idx - 1) % 3 {
        0 => Pass::Sig,
        1 => Pass::Ref,
        _ => Pass::Clean,
    }
}

/// the most passes a segment that starts with pass `first` can hold (B.10.7.1)
pub(crate) fn seg_max_passes(style: u8, first: u32) -> u32 {
    if style & TERMALL != 0 {
        1
    } else if style & BYPASS != 0 {
        if first < 10 {
            10 - first
        } else if pass_kind(first) == Pass::Clean {
            1
        } else {
            2
        }
    } else {
        u32::MAX
    }
}

fn init_cx() -> [Cx; 19] {
    let mut cx = [Cx::new(0); 19];
    cx[0] = Cx::new(4);
    cx[CX_RUN] = Cx::new(3);
    cx[CX_UNI] = Cx::new(46);
    cx
}

/// a source of decisions: the MQ decoder with the contexts, or raw bits
trait Bits {
    fn bit(&mut self, cx: usize) -> u32;
    fn sign(&mut self, cx: usize, xor: u32) -> u32;
}

struct MqBits<'a, 'b> {
    mq: Mq<'a>,
    cx: &'b mut [Cx; 19],
}

impl Bits for MqBits<'_, '_> {
    #[inline(always)]
    fn bit(&mut self, cx: usize) -> u32 {
        self.mq.decode(&mut self.cx[cx])
    }
    #[inline(always)]
    fn sign(&mut self, cx: usize, xor: u32) -> u32 {
        self.mq.decode(&mut self.cx[cx]) ^ xor
    }
}

struct RawBits<'a>(Raw<'a>);

impl Bits for RawBits<'_> {
    #[inline(always)]
    fn bit(&mut self, _cx: usize) -> u32 {
        self.0.bit()
    }
    #[inline(always)]
    fn sign(&mut self, _cx: usize, _xor: u32) -> u32 {
        self.0.bit()
    }
}

/// the scratch of the code-block decoder (kept between code-blocks)
pub(crate) struct T1 {
    flags: Vec<u16>,
    pub coef: Vec<i32>,
    w: usize,
    h: usize,
    stride: usize,
    orient: usize,
    vsc: bool,
}

impl T1 {
    pub fn new() -> T1 {
        T1 { flags: Vec::new(), coef: Vec::new(), w: 0, h: 0, stride: 0, orient: 0, vsc: false }
    }

    /// decode a code-block of w x h samples whose first bit-plane is `p0` (the cleanup pass of
    /// `p0` comes first) from its segments into `self.coef` (w * h, row by row, half units)
    pub fn decode(&mut self, segs: &[Seg], w: usize, h: usize, orient: u8, style: u8, p0: i32) {
        self.w = w;
        self.h = h;
        self.stride = w + 2;
        self.orient = (orient & 3) as usize;
        self.vsc = style & VSC != 0;
        self.flags.clear();
        self.flags.resize(self.stride * (h + 2), 0);
        self.coef.clear();
        self.coef.resize(w * h, 0);
        if w == 0 || h == 0 || p0 < 0 {
            return;
        }
        let mut cx = init_cx();
        let mut idx = 0u32;
        for seg in segs {
            if seg.passes == 0 {
                continue;
            }
            let raw = style & BYPASS != 0 && idx >= 10 && pass_kind(idx) != Pass::Clean;
            if raw {
                let mut b = RawBits(Raw::new(seg.data));
                for _ in 0..seg.passes {
                    let p = p0 - ((idx + 2) / 3) as i32;
                    if p < 0 {
                        return;
                    }
                    match pass_kind(idx) {
                        Pass::Sig => self.sig_pass(&mut b, p),
                        Pass::Ref => self.ref_pass(&mut b, p),
                        // a cleanup pass in a raw segment is not valid: the rest is lost
                        Pass::Clean => return,
                    }
                    if style & RESET != 0 {
                        cx = init_cx();
                    }
                    idx += 1;
                }
            } else {
                let mut b = MqBits { mq: Mq::new(seg.data), cx: &mut cx };
                for _ in 0..seg.passes {
                    let p = p0 - ((idx + 2) / 3) as i32;
                    if p < 0 {
                        return;
                    }
                    match pass_kind(idx) {
                        Pass::Sig => self.sig_pass(&mut b, p),
                        Pass::Ref => self.ref_pass(&mut b, p),
                        Pass::Clean => {
                            self.clean_pass(&mut b, p);
                            if style & SEGMARK != 0 {
                                for _ in 0..4 {
                                    b.bit(CX_UNI);
                                }
                            }
                        }
                    }
                    if style & RESET != 0 {
                        *b.cx = init_cx();
                    }
                    idx += 1;
                }
            }
        }
    }

    /// the neighbour bits a sample in row `y` sees
    #[inline(always)]
    fn seen(&self, f: u16, y: usize) -> u16 {
        if self.vsc && y & 3 == 3 {
            f & !BELOW
        } else {
            f
        }
    }

    #[inline(always)]
    fn set_sig(&mut self, i: usize, neg: bool, coef: usize, p: i32) {
        let st = self.stride;
        let f = &mut self.flags;
        if neg {
            f[i] |= SIG | NEG;
            f[i - st] |= S | SS;
            f[i + st] |= N | NS;
            f[i - 1] |= E | ES;
            f[i + 1] |= W | WS;
        } else {
            f[i] |= SIG;
            f[i - st] |= S;
            f[i + st] |= N;
            f[i - 1] |= E;
            f[i + 1] |= W;
        }
        f[i - st - 1] |= SE;
        f[i - st + 1] |= SW;
        f[i + st - 1] |= NE;
        f[i + st + 1] |= NW;
        let v = 3i32 << p;
        self.coef[coef] = if neg { -v } else { v };
    }

    #[inline(always)]
    fn decode_sign<B: Bits>(&self, b: &mut B, f: u16) -> bool {
        let k = ((f & 0xF) | ((f >> 4) & 0xF0)) as usize;
        let (cx, xor) = SIGN_LUT[k];
        b.sign(cx as usize, xor as u32) != 0
    }

    fn sig_pass<B: Bits>(&mut self, b: &mut B, p: i32) {
        let (w, h, st) = (self.w, self.h, self.stride);
        let lut = &SIG_LUT[self.orient];
        let mut y0 = 0;
        while y0 < h {
            let rows = (h - y0).min(4);
            for x in 0..w {
                for dy in 0..rows {
                    let y = y0 + dy;
                    let i = (y + 1) * st + x + 1;
                    let f = self.flags[i];
                    if f & (SIG | VISIT) != 0 {
                        continue;
                    }
                    let fs = self.seen(f, y);
                    if fs & 0xFF == 0 {
                        continue;
                    }
                    if b.bit(lut[(fs & 0xFF) as usize] as usize) != 0 {
                        let neg = self.decode_sign(b, fs);
                        self.set_sig(i, neg, y * w + x, p);
                    }
                    self.flags[i] |= VISIT;
                }
            }
            y0 += 4;
        }
    }

    fn ref_pass<B: Bits>(&mut self, b: &mut B, p: i32) {
        let (w, h, st) = (self.w, self.h, self.stride);
        let d = 1i32 << p;
        let mut y0 = 0;
        while y0 < h {
            let rows = (h - y0).min(4);
            for x in 0..w {
                for dy in 0..rows {
                    let y = y0 + dy;
                    let i = (y + 1) * st + x + 1;
                    let f = self.flags[i];
                    if f & (SIG | VISIT) != SIG {
                        continue;
                    }
                    let cx = if f & REF != 0 {
                        16
                    } else if self.seen(f, y) & 0xFF != 0 {
                        15
                    } else {
                        14
                    };
                    let up = b.bit(cx) != 0;
                    let c = &mut self.coef[y * w + x];
                    let delta = if up { d } else { -d };
                    *c = if *c < 0 { *c - delta } else { *c + delta };
                    self.flags[i] |= REF;
                }
            }
            y0 += 4;
        }
    }

    fn clean_pass<B: Bits>(&mut self, b: &mut B, p: i32) {
        let (w, h, st) = (self.w, self.h, self.stride);
        let lut = &SIG_LUT[self.orient];
        let mut y0 = 0;
        while y0 < h {
            let rows = (h - y0).min(4);
            for x in 0..w {
                let mut start = 0;
                if rows == 4 {
                    // the run-length mode: four samples, none significant, none visited, none with
                    // a significant neighbour
                    let mut empty = true;
                    for dy in 0..4 {
                        let i = (y0 + dy + 1) * st + x + 1;
                        let f = self.flags[i];
                        if f & (SIG | VISIT) != 0 || self.seen(f, y0 + dy) & 0xFF != 0 {
                            empty = false;
                            break;
                        }
                    }
                    if empty {
                        if b.bit(CX_RUN) == 0 {
                            continue;
                        }
                        let r = ((b.bit(CX_UNI) << 1) | b.bit(CX_UNI)) as usize;
                        let y = y0 + r;
                        let i = (y + 1) * st + x + 1;
                        let fs = self.seen(self.flags[i], y);
                        let neg = self.decode_sign(b, fs);
                        self.set_sig(i, neg, y * w + x, p);
                        start = r + 1;
                    }
                }
                for dy in start..rows {
                    let y = y0 + dy;
                    let i = (y + 1) * st + x + 1;
                    let f = self.flags[i];
                    if f & (SIG | VISIT) != 0 {
                        continue;
                    }
                    let fs = self.seen(f, y);
                    if b.bit(lut[(fs & 0xFF) as usize] as usize) != 0 {
                        let neg = self.decode_sign(b, fs);
                        self.set_sig(i, neg, y * w + x, p);
                    }
                }
            }
            y0 += 4;
        }
        // the next bit-plane starts with nothing visited
        for f in self.flags.iter_mut() {
            *f &= !VISIT;
        }
    }
}
