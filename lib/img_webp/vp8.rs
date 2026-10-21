// vp8 -- WebP lossy (VP8 key frame) decoder, bit-exact with libwebp 1.3  [src/img/webp_vp8.s]
//
//   decode(data, dec) -> Ok(surface) / Err(ERR_*)          vp8_decode
//   itx(coeffs, buf, at) / itx_dc(coeffs, buf, at) / wht(input, out)
//                                                          vp8_itx / vp8_itx_dc / vp8_wht
//   yuv_rgb(y, u, v) -> 0xFFRRGGBB                         vp8_yuv_rgb
//
// decode takes the payload of a "VP8 " chunk -- every byte the decoder may read (libwebp
// lets the last token partition run to the end of the input) -- and returns a new
// FMT_ARGB surface (alpha 255; the container applies ALPH).
//
// Decoder (RFC 6386, libwebp src/dec/{vp8,tree,quant,frame}_dec.c, dsp/dec.c,
// dsp/upsampling.c, dsp/yuv.h): key frames only (inter frames: ERR_UNSUPPORTED), boolean
// decoder with libwebp's 56-bit refill and end-of-data rule (reading a byte past a
// partition sets eof; eof in partition 0 after a macroblock row or in a token partition
// after a macroblock is ERR_TRUNC), segment header, filter header, 1..8 token partitions
// (sizes clamped like libwebp, the last one must hold >= 1 byte), dequantisation,
// coefficient probability updates, skip probability, intra modes, residual tokens, inverse
// WHT and DCT (the 16-bit arithmetic of libwebp's Transform_SSE2, the C AC3 / DC
// shortcuts), libwebp's reconstruction edge rules, the simple and normal loop filters
// (the saturating byte arithmetic of the assembly, lane by lane), then libwebp's "fancy"
// upsampling of U / V and its 14-bit fixed-point YUV -> RGB.
//
// Memory: the Y / U / V planes of the macroblock grid, per-column arrays; dec.maxmem is
// checked, dec.cancel polled once per macroblock row.

use alloc::vec;
use alloc::vec::Vec;

use gfx::{FMT_ARGB, Surface};
use img_core::meta::{dims_check, mem_check, surf_alloc};
use img_core::*;
use simd::{Simd, Sse2};

use crate::tab::{ACTAB, BMODES, CAT, DCTAB, P4TAB, PROBA0, UPDATE, ZIGZAG};

pub const BPS: usize = 32; // stride of the reconstruction work buffer
const Y_OFF: usize = BPS + 8;
const U_OFF: usize = Y_OFF + BPS * 16 + BPS;
const V_OFF: usize = U_OFF + 16;
const YUV_SIZE: usize = BPS * 17 + BPS * 9;

// intra modes (libwebp numbering)
pub const B_DC: u8 = 0;
pub const B_TM: u8 = 1;
pub const B_VE: u8 = 2;
pub const B_HE: u8 = 3;
pub const B_RD: u8 = 4;
pub const B_VR: u8 = 5;
pub const B_LD: u8 = 6;
pub const B_VL: u8 = 7;
pub const B_HD: u8 = 8;
pub const B_HU: u8 = 9;
const DC_NOTOP: u8 = 4; // 16x16 / chroma DC variants at the edges
const DC_NOLEFT: u8 = 5;
const DC_NOTOPLEFT: u8 = 6;

// band offset * 33 of coefficient positions 0..16 (libwebp kBands, 16 = sentinel)
static BANDOFF: [u16; 17] = [0, 33, 66, 99, 198, 132, 165, 198, 198, 198, 198, 198, 198, 198, 198, 231, 0];
// offsets of the 16 luma blocks in the work buffer (libwebp kScan)
pub static SCAN: [u16; 16] = [
    0,
    4,
    8,
    12,
    (4 * BPS) as u16,
    (4 * BPS + 4) as u16,
    (4 * BPS + 8) as u16,
    (4 * BPS + 12) as u16,
    (8 * BPS) as u16,
    (8 * BPS + 4) as u16,
    (8 * BPS + 8) as u16,
    (8 * BPS + 12) as u16,
    (12 * BPS) as u16,
    (12 * BPS + 4) as u16,
    (12 * BPS + 8) as u16,
    (12 * BPS + 12) as u16,
];
static UVOFF: [usize; 4] = [0, 4, 4 * BPS, 4 * BPS + 4];

// ------------------------------------------------------------------ boolean decoder

// 7 ^ floor(log2(r)) for r = 1..255 (the renormalisation shift; entry 0 unused)
static SHIFT7: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut r = 1;
    while r < 256 {
        t[r] = (7 ^ (31 - (r as u32).leading_zeros())) as u8;
        r += 1;
    }
    t
};

/// libwebp's VP8BitReader (56-bit refill, end-of-data rule)
#[derive(Clone, Copy)]
struct Br<'a> {
    d: &'a [u8],
    value: u64,
    range: u32, // range - 1
    bits: i32,
    buf: usize,
    end: usize,
    max: usize, // last position of an 8-byte load + 1
    eof: bool,
}

impl<'a> Br<'a> {
    // VP8InitBitReader: range 254, bits -8, first load
    fn new(d: &'a [u8], start: usize, size: usize) -> Br<'a> {
        let max = if size >= 8 { start + size - 7 } else { start };
        let mut b = Br { d, value: 0, range: 254, bits: -8, buf: start, end: start + size, max, eof: false };
        b.load();
        b
    }

    // VP8LoadNewBytes / VP8LoadFinalBytes
    #[inline(always)]
    fn load(&mut self) {
        if self.buf < self.max {
            let v = u64::from_be_bytes(self.d[self.buf..self.buf + 8].try_into().unwrap()) >> 8;
            self.value = (self.value << 56) | v;
            self.buf += 7;
            self.bits += 56;
        } else if self.buf < self.end {
            self.value = (self.value << 8) | self.d[self.buf] as u64;
            self.buf += 1;
            self.bits += 8;
        } else if !self.eof {
            self.value <<= 8;
            self.bits += 8;
            self.eof = true;
        } else {
            self.bits = 0;
        }
    }

    // VP8GetBit
    #[inline(always)]
    fn bit(&mut self, prob: u32) -> u32 {
        let split = (self.range * prob) >> 8;
        if self.bits < 0 {
            self.load();
        }
        let pos = self.bits as u32;
        let value = (self.value >> pos) as u32;
        let bit;
        let mut range;
        if value > split {
            range = self.range - split;
            self.value -= ((split + 1) as u64) << pos;
            bit = 1;
        } else {
            range = split + 1;
            bit = 0;
        }
        let shift = SHIFT7[range as usize & 255] as u32; // 7 ^ bsr(range), range 1..255
        range <<= shift;
        self.bits -= shift as i32;
        self.range = range - 1;
        bit
    }

    // VP8GetSigned
    #[inline(always)]
    fn signed(&mut self, v: i32) -> i32 {
        if self.bits < 0 {
            self.load();
        }
        let pos = self.bits as u32;
        let split = self.range >> 1;
        let value = (self.value >> pos) as u32;
        let mask = (split.wrapping_sub(value) as i32) >> 31; // -1 when value > split
        self.bits -= 1;
        self.range = self.range.wrapping_add(mask as u32);
        self.range |= 1;
        self.value -= (((split + 1) & mask as u32) as u64) << pos;
        (v ^ mask) - mask
    }

    // VP8GetValue (MSB first)
    fn value(&mut self, n: u32) -> u32 {
        let mut v = 0;
        for _ in 0..n {
            v = v * 2 + self.bit(0x80);
        }
        v
    }

    // VP8GetSignedValue
    fn svalue(&mut self, n: u32) -> i32 {
        let v = self.value(n) as i32;
        if self.bit(0x80) != 0 { -v } else { v }
    }

    // VP8Get() ? VP8GetSignedValue(n) : 0
    fn flagval(&mut self, n: u32) -> i32 {
        if self.bit(0x80) != 0 { self.svalue(n) } else { 0 }
    }
}

// ------------------------------------------------------------------ context

/// per-macroblock mode record (MB_*)
#[derive(Clone, Copy, Default)]
struct Mb {
    seg: u8,
    skip: u8,
    i4: u8,
    uv: u8,
    modes: [u8; 16],
}

struct V8<'a> {
    w: u32,
    h: u32,
    mbw: u32,
    mbh: u32,
    npartm1: u32,
    ftype: u32, // 0 none, 1 simple, 2 normal
    useseg: u32,
    updmap: u32,
    absdelta: u32,
    simple: u32,
    level: u32,
    sharp: u32,
    uselfd: u32,
    useskip: u32,
    skipp: u32,
    quant: [i32; 4],
    fstr: [i32; 4],
    refd: [i32; 4],
    moded: [i32; 4],
    segp: [u32; 3],
    nplog: u32,
    dqm: [[i32; 6]; 4], // y1 dc, ac, y2 dc, ac, uv dc, ac
    curdq: [i32; 2],
    fstrs: [[u32; 2]; 4], // limit | ilevel << 8 | inner << 16 | hev << 24
    part0: Br<'a>,
    parts: [Br<'a>; 8],
    proba: [u8; 1056],
    yp: Vec<u8>,
    up: Vec<u8>,
    vp: Vec<u8>,
    ys: usize,
    uvs: usize,
    mbdata: Vec<Mb>,
    intrat: Vec<[u8; 4]>,
    nz: Vec<[u8; 2]>,
    top: Vec<[u8; 32]>,
    finfo: Vec<u32>,
    lnz: u32,
    lnzdc: u32,
    intral: [u8; 4],
    mby: u32,
    nzy: u32,
    nzuv: u32,
    dc: [i16; 16],
    coef: [i16; 384],
    yuvb: [u8; YUV_SIZE],
}

/// decode a VP8 key frame (payload of a "VP8 " chunk, data = bytes readable) into a new
/// FMT_ARGB surface  [vp8_decode]
pub fn decode(data: &[u8], dec: Option<&ImgDec>) -> Result<Surface, i32> {
    let len = data.len();
    if len < 10 {
        return Err(ERR_TRUNC);
    }
    let tag = data[0] as u32 | (data[1] as u32) << 8 | (data[2] as u32) << 16;
    if tag & 1 != 0 || tag & 16 == 0 {
        return Err(ERR_UNSUPPORTED); // inter frame / not shown
    }
    if (tag >> 1) & 7 > 3 || data[3] != 0x9D || data[4] != 0x01 || data[5] != 0x2A {
        return Err(ERR_HDR);
    }
    let p0 = (tag >> 5) as usize; // first partition size
    if p0 > len - 10 {
        return Err(ERR_TRUNC);
    }
    let w = u16::from_le_bytes([data[6], data[7]]) as u32 & 0x3FFF;
    let h = u16::from_le_bytes([data[8], data[9]]) as u32 & 0x3FFF;
    if w == 0 || h == 0 {
        return Err(ERR_HDR);
    }
    let dd = dec.copied().unwrap_or_default();
    let r = dims_check(w, h, &dd);
    if r != 0 {
        return Err(r);
    }
    let mbw = (w + 15) >> 4;
    let mbh = (h + 15) >> 4;
    // v8_alloc: planes (macroblock grid) and the per-column arrays
    let ysz = mbw as u64 * mbh as u64 * 256;
    let rowb = (mbw as u64) * 16 + 64;
    if mem_check(ysz + (ysz >> 2) * 2 + 256 + rowb * 2, &dd) != 0 {
        return Err(ERR_LIMIT);
    }
    let br0 = Br::new(data, 10, p0);
    let mut v = V8 {
        w,
        h,
        mbw,
        mbh,
        npartm1: 0,
        ftype: 0,
        useseg: 0,
        updmap: 0,
        absdelta: 1,
        simple: 0,
        level: 0,
        sharp: 0,
        uselfd: 0,
        useskip: 0,
        skipp: 0,
        quant: [0; 4],
        fstr: [0; 4],
        refd: [0; 4],
        moded: [0; 4],
        segp: [255; 3],
        nplog: 0,
        dqm: [[0; 6]; 4],
        curdq: [0; 2],
        fstrs: [[0; 2]; 4],
        part0: br0,
        parts: [br0; 8],
        proba: [0; 1056],
        yp: vec![0; ysz as usize + 64],
        up: vec![0; (ysz >> 2) as usize + 128],
        vp: vec![0; (ysz >> 2) as usize + 128],
        ys: mbw as usize * 16,
        uvs: mbw as usize * 8,
        mbdata: vec![Mb::default(); mbw as usize],
        intrat: vec![[0; 4]; mbw as usize],
        nz: vec![[0; 2]; mbw as usize],
        top: vec![[0; 32]; mbw as usize + 1],
        finfo: vec![0; mbw as usize],
        lnz: 0,
        lnzdc: 0,
        intral: [0; 4],
        mby: 0,
        nzy: 0,
        nzuv: 0,
        dc: [0; 16],
        coef: [0; 384],
        yuvb: [0; YUV_SIZE],
    };
    v.headers()?;
    // token partitions: sizes (3 bytes each) after the first partition
    let mut at = 10 + p0; // partition data
    let mut left = len - p0 - 10; // bytes left
    let n = 1u32 << v.nplog;
    v.npartm1 = n - 1;
    let last = (n - 1) as usize;
    if left < 3 * last {
        return Err(ERR_TRUNC);
    }
    let sizes = at;
    at += 3 * last;
    left -= 3 * last;
    for p in 0..last {
        let o = sizes + 3 * p;
        let sz = (data[o] as usize | (data[o + 1] as usize) << 8 | (data[o + 2] as usize) << 16).min(left);
        left -= sz;
        v.parts[p] = Br::new(data, at, sz);
        at += sz;
    }
    if left == 0 {
        return Err(ERR_TRUNC); // the last partition needs a byte
    }
    v.parts[last] = Br::new(data, at, left);
    v.fstrengths();
    // macroblock rows
    while v.mby < v.mbh {
        v.modes()?;
        v.row()?;
        if v.ftype != 0 {
            v.filter_row();
        }
        if dd.cancelled() {
            return Err(ERR_CANCEL);
        }
        v.mby += 1;
    }
    let mut sf = surf_alloc(w, h, FMT_ARGB, &dd)?;
    v.output(&mut sf);
    Ok(sf)
}

impl V8<'_> {
    // v8_headers: first-partition headers: colour space and clamping bits, segment header,
    // filter header, partition count, quantisers, probability updates, skip probability
    fn headers(&mut self) -> Result<(), i32> {
        let mut b = self.part0;
        b.bit(0x80); // colour space
        b.bit(0x80); // clamping type
        self.useseg = b.bit(0x80);
        if self.useseg != 0 {
            self.updmap = b.bit(0x80);
            if b.bit(0x80) != 0 {
                // update segment data
                self.absdelta = b.bit(0x80);
                for s in 0..4 {
                    self.quant[s] = b.flagval(7);
                }
                for s in 0..4 {
                    self.fstr[s] = b.flagval(6);
                }
            }
            if self.updmap != 0 {
                for s in 0..3 {
                    self.segp[s] = if b.bit(0x80) != 0 { b.value(8) } else { 255 };
                }
            }
        }
        if b.eof {
            return Err(ERR_HDR);
        }
        // filter header
        self.simple = b.bit(0x80);
        self.level = b.value(6);
        self.sharp = b.value(3);
        self.uselfd = b.bit(0x80);
        if self.uselfd != 0 && b.bit(0x80) != 0 {
            for i in 0..4 {
                if b.bit(0x80) != 0 {
                    self.refd[i] = b.svalue(6);
                }
            }
            for i in 0..4 {
                if b.bit(0x80) != 0 {
                    self.moded[i] = b.svalue(6);
                }
            }
        }
        self.ftype = if self.level == 0 {
            0
        } else if self.simple != 0 {
            1
        } else {
            2
        };
        if b.eof {
            return Err(ERR_HDR);
        }
        self.nplog = b.value(2);
        // quantisers
        let base = b.value(7) as i32;
        let mut dq = [0i32; 5];
        for d in dq.iter_mut() {
            *d = b.flagval(4);
        }
        let clip = |v: i32, m: i32| v.clamp(0, m) as usize;
        for s in 0..4 {
            let mut q = base;
            if self.useseg != 0 {
                if self.absdelta != 0 {
                    q = self.quant[s];
                } else {
                    q += self.quant[s];
                }
            } else if s > 0 {
                self.dqm[s] = self.dqm[0]; // no segments: copy segment 0
                continue;
            }
            let m = &mut self.dqm[s];
            m[0] = DCTAB[clip(q + dq[0], 127)] as i32;
            m[1] = ACTAB[clip(q, 127)] as i32;
            m[2] = DCTAB[clip(q + dq[1], 127)] as i32 * 2;
            m[3] = ((ACTAB[clip(q + dq[2], 127)] as u32 * 101581) >> 16).max(8) as i32;
            m[4] = DCTAB[clip(q + dq[3], 117)] as i32;
            m[5] = ACTAB[clip(q + dq[4], 127)] as i32;
        }
        b.bit(0x80); // update_proba (ignored, key frame)
        // coefficient probabilities
        for i in 0..1056 {
            self.proba[i] = if b.bit(UPDATE[i] as u32) != 0 { b.value(8) as u8 } else { PROBA0[i] };
        }
        self.useskip = b.bit(0x80);
        if self.useskip != 0 {
            self.skipp = b.value(8);
        }
        self.part0 = b;
        Ok(())
    }

    // v8_fstrengths: libwebp PrecomputeFilterStrengths
    fn fstrengths(&mut self) {
        if self.ftype == 0 {
            return;
        }
        for s in 0..4 {
            let mut base = self.level as i32;
            if self.useseg != 0 {
                if self.absdelta != 0 {
                    base = self.fstr[s];
                } else {
                    base += self.fstr[s];
                }
            }
            for i4 in 0..2 {
                let mut l = base;
                if self.uselfd != 0 {
                    l += self.refd[0];
                    if i4 != 0 {
                        l += self.moded[0];
                    }
                }
                let l = l.clamp(0, 63);
                let mut info = (i4 as u32) << 16; // inner = i4x4
                if l > 0 {
                    let mut il = l;
                    let sh = self.sharp as i32;
                    if sh != 0 {
                        il >>= if sh > 4 { 2 } else { 1 };
                        il = il.min(9 - sh);
                    }
                    il = il.max(1);
                    let hev = if l >= 40 {
                        2
                    } else if l >= 15 {
                        1
                    } else {
                        0
                    };
                    info |= ((2 * l + il) as u32 & 0xFF) | (il as u32) << 8 | hev << 24;
                }
                self.fstrs[s][i4] = info;
            }
        }
    }

    // ------------------------------------------------------------------ intra modes

    // v8_modes: the intra modes of one macroblock row (libwebp ParseIntraMode) from the
    // first partition
    fn modes(&mut self) -> Result<(), i32> {
        let mut b = self.part0;
        self.intral = [B_DC; 4];
        for mx in 0..self.mbw as usize {
            let mut m = self.mbdata[mx];
            m.seg = 0;
            if self.updmap != 0 {
                m.seg = if b.bit(self.segp[0]) == 0 { b.bit(self.segp[1]) as u8 } else { b.bit(self.segp[2]) as u8 + 2 };
            }
            if self.useskip != 0 {
                m.skip = b.bit(self.skipp) as u8;
            }
            m.i4 = (b.bit(145) ^ 1) as u8;
            if m.i4 == 0 {
                // 16x16 mode
                let ym = if b.bit(156) != 0 {
                    if b.bit(128) != 0 { B_TM } else { B_HE }
                } else if b.bit(163) != 0 {
                    B_VE
                } else {
                    B_DC
                };
                m.modes[0] = ym;
                self.intrat[mx] = [ym; 4];
                self.intral = [ym; 4];
            } else {
                for y in 0..4 {
                    let mut ym = self.intral[y];
                    for x in 0..4 {
                        let t = self.intrat[mx][x] as usize;
                        let p = &BMODES[(t * 10 + ym as usize) * 9..];
                        ym = bmode(&mut b, p);
                        self.intrat[mx][x] = ym;
                        m.modes[y * 4 + x] = ym;
                    }
                    self.intral[y] = ym;
                }
            }
            m.uv = if b.bit(142) == 0 {
                B_DC
            } else if b.bit(114) == 0 {
                B_VE
            } else if b.bit(183) != 0 {
                B_TM
            } else {
                B_HE
            };
            self.mbdata[mx] = m;
        }
        self.part0 = b;
        if b.eof { Err(ERR_TRUNC) } else { Ok(()) }
    }

    // ------------------------------------------------------------------ macroblock rows

    // v8_row: residual tokens of every macroblock of row mby (libwebp VP8DecodeMB /
    // ParseResiduals) + reconstruction
    fn row(&mut self) -> Result<(), i32> {
        let pi = (self.mby & self.npartm1) as usize;
        let mut b = self.parts[pi];
        self.lnz = 0;
        self.lnzdc = 0;
        for mx in 0..self.mbw as usize {
            let m = self.mbdata[mx];
            let dq = self.dqm[(m.seg & 3) as usize];
            let noco;
            if self.useskip != 0 && m.skip != 0 {
                // skipped macroblock: no coefficients
                self.nz[mx][0] = 0;
                self.lnz = 0;
                if m.i4 == 0 {
                    self.nz[mx][1] = 0;
                    self.lnzdc = 0;
                }
                self.nzy = 0;
                self.nzuv = 0;
                noco = true;
            } else {
                self.coef = [0; 384];
                let mut first = 0usize;
                let mut ty = 3 * 264;
                if m.i4 == 0 {
                    // 16x16: second-order DC block (type 1)
                    self.dc = [0; 16];
                    self.curdq = [dq[2], dq[3]];
                    let ctx = self.nz[mx][1] as usize + self.lnzdc as usize;
                    let mut dc = [0i16; 16];
                    let pr: &[u8; 264] = self.proba[264..2 * 264].try_into().unwrap();
                    let n = coeffs(&mut b, pr, ctx, 0, self.curdq, &mut dc);
                    self.dc = dc;
                    let f = (n != 0) as u32;
                    self.nz[mx][1] = f as u8;
                    self.lnzdc = f;
                    if n > 1 {
                        wht(&self.dc, &mut self.coef);
                    } else {
                        let d0 = ((self.dc[0] as i32 + 3) >> 3) as i16;
                        for k in 0..16 {
                            self.coef[k * 16] = d0;
                        }
                    }
                    first = 1;
                    ty = 0;
                }
                self.curdq = [dq[0], dq[1]];
                let mut tnz = (self.nz[mx][0] & 15) as u32;
                let mut lnz = self.lnz & 15;
                let mut nzy = 0u32;
                let mut blk = 0usize;
                for _y in 0..4 {
                    let mut l = lnz & 1;
                    let mut nzc = 0u32;
                    for _x in 0..4 {
                        let ctx = ((tnz & 1) + l) as usize;
                        let pr: &[u8; 264] = self.proba[ty..ty + 264].try_into().unwrap();
                        let c = &mut self.coef[blk * 16..blk * 16 + 16];
                        let n = coeffs(&mut b, pr, ctx, first, self.curdq, c);
                        l = (n > first) as u32;
                        tnz = ((tnz >> 1) | l << 7) & 255;
                        nzc = (nzc << 2) | nzcode(n, c[0]);
                        blk += 1;
                    }
                    tnz >>= 4;
                    lnz = ((lnz >> 1) | l << 7) & 255;
                    nzy = (nzy << 8) | nzc;
                }
                let mut out_t = tnz;
                let mut out_l = lnz >> 4;
                // chroma (type 2)
                self.curdq = [dq[4], dq[5]];
                let mut nzuv = 0u32;
                for ch in [0u32, 2] {
                    let mut nzc = 0u32;
                    let mut tnz = (self.nz[mx][0] as u32) >> (ch + 4);
                    let mut lnz = (self.lnz & 255) >> (ch + 4);
                    for _y in 0..2 {
                        let mut l = lnz & 1;
                        for _x in 0..2 {
                            let ctx = ((tnz & 1) + l) as usize;
                            let pr: &[u8; 264] = self.proba[2 * 264..3 * 264].try_into().unwrap();
                            let c = &mut self.coef[blk * 16..blk * 16 + 16];
                            let n = coeffs(&mut b, pr, ctx, 0, self.curdq, c);
                            l = (n != 0) as u32;
                            tnz = ((tnz >> 1) | l << 3) & 255;
                            nzc = (nzc << 2) | nzcode(n, c[0]);
                            blk += 1;
                        }
                        tnz >>= 2;
                        lnz = ((lnz >> 1) | l << 5) & 255;
                    }
                    nzuv |= nzc << (4 * ch);
                    out_t |= (tnz << 4) << ch;
                    out_l |= (lnz & 0xF0) << ch;
                }
                self.nz[mx][0] = out_t as u8;
                self.lnz = out_l & 255;
                self.nzy = nzy;
                self.nzuv = nzuv;
                noco = (nzy | nzuv) == 0;
            }
            if self.ftype != 0 {
                let mut f = self.fstrs[(m.seg & 3) as usize][(m.i4 & 1) as usize];
                if !noco {
                    f |= 0x10000; // inner edges when coefficients
                }
                self.finfo[mx] = f;
            }
            if b.eof {
                self.parts[pi] = b;
                return Err(ERR_TRUNC);
            }
            self.recon(mx);
        }
        self.parts[pi] = b;
        Ok(())
    }

    // ------------------------------------------------------------------ reconstruction

    // v8_recon: predict + add the residuals of macroblock (mx, mby) in the work buffer
    // with libwebp's edge rules, stash the unfiltered bottom row, copy the macroblock into
    // the planes (libwebp ReconstructRow)
    fn recon(&mut self, mx: usize) {
        let my = self.mby;
        let yb = &mut self.yuvb;
        let m = self.mbdata[mx];
        if mx == 0 {
            // first macroblock of the row: left = 129, top-left 129 / top row 127
            for j in 0..16 {
                yb[Y_OFF + j * BPS - 1] = 129;
                if j < 8 {
                    yb[U_OFF + j * BPS - 1] = 129;
                    yb[V_OFF + j * BPS - 1] = 129;
                }
            }
            if my != 0 {
                yb[Y_OFF - BPS - 1] = 129;
                yb[U_OFF - BPS - 1] = 129;
                yb[V_OFF - BPS - 1] = 129;
            } else {
                yb[Y_OFF - BPS - 1..Y_OFF - BPS + 20].fill(127);
                yb[U_OFF - BPS - 1..U_OFF - BPS + 8].fill(127);
                yb[V_OFF - BPS - 1..V_OFF - BPS + 8].fill(127);
            }
        } else {
            // rotate in the left samples of the previous macroblock (rows -1..15 / -1..7)
            for j in 0..17 {
                let r = Y_OFF + j * BPS - BPS;
                yb.copy_within(r + 12..r + 16, r - 4);
                if j < 9 {
                    let u = U_OFF + j * BPS - BPS;
                    yb.copy_within(u + 4..u + 8, u - 4);
                    let v = V_OFF + j * BPS - BPS;
                    yb.copy_within(v + 4..v + 8, v - 4);
                }
            }
        }
        if my != 0 {
            let t = &self.top[mx];
            yb[Y_OFF - BPS..Y_OFF - BPS + 16].copy_from_slice(&t[..16]);
            yb[U_OFF - BPS..U_OFF - BPS + 8].copy_from_slice(&t[16..24]);
            yb[V_OFF - BPS..V_OFF - BPS + 8].copy_from_slice(&t[24..32]);
        }
        if m.i4 != 0 {
            // 4x4: top-right samples
            if my != 0 {
                let tr = if mx + 1 >= self.mbw as usize {
                    [self.top[mx][15]; 4]
                } else {
                    let t = &self.top[mx + 1];
                    [t[0], t[1], t[2], t[3]]
                };
                yb[Y_OFF - BPS + 16..Y_OFF - BPS + 20].copy_from_slice(&tr);
            }
            let mut tr = [0u8; 4];
            tr.copy_from_slice(&yb[Y_OFF - BPS + 16..Y_OFF - BPS + 20]);
            for r in [3, 7, 11] {
                yb[Y_OFF + r * BPS + 16..Y_OFF + r * BPS + 20].copy_from_slice(&tr);
            }
            let mut bits = self.nzy;
            for n in 0..16 {
                let at = Y_OFF + SCAN[n] as usize;
                let mode = m.modes[n];
                if mode <= 9 {
                    pred4(yb, at, mode);
                    let c = &self.coef[n * 16..n * 16 + 16];
                    dotx(c, yb, at, bits >> 30);
                }
                bits <<= 2;
            }
        } else {
            let mode = check_mode(mx as u32, my, m.modes[0]);
            pred16(yb, Y_OFF, mode);
            let mut bits = self.nzy;
            if bits != 0 {
                for n in 0..16 {
                    let at = Y_OFF + SCAN[n] as usize;
                    dotx(&self.coef[n * 16..n * 16 + 16], yb, at, bits >> 30);
                    bits <<= 2;
                }
            }
        }
        let mode = check_mode(mx as u32, my, m.uv);
        pred8(yb, U_OFF, mode);
        pred8(yb, V_OFF, mode);
        uvtx(self.nzuv, yb, U_OFF, &self.coef[16 * 16..20 * 16]);
        uvtx(self.nzuv >> 8, yb, V_OFF, &self.coef[20 * 16..24 * 16]);
        // stash the unfiltered bottom row for the next macroblock row
        if my < self.mbh - 1 {
            let t = &mut self.top[mx];
            t[..16].copy_from_slice(&yb[Y_OFF + 15 * BPS..Y_OFF + 15 * BPS + 16]);
            t[16..24].copy_from_slice(&yb[U_OFF + 7 * BPS..U_OFF + 7 * BPS + 8]);
            t[24..32].copy_from_slice(&yb[V_OFF + 7 * BPS..V_OFF + 7 * BPS + 8]);
        }
        // macroblock -> planes
        let ys = self.ys;
        let mut o = (my as usize * 16) * ys + mx * 16;
        for j in 0..16 {
            self.yp[o..o + 16].copy_from_slice(&yb[Y_OFF + j * BPS..Y_OFF + j * BPS + 16]);
            o += ys;
        }
        let uvs = self.uvs;
        let mut o = (my as usize * 8) * uvs + mx * 8;
        for j in 0..8 {
            self.up[o..o + 8].copy_from_slice(&yb[U_OFF + j * BPS..U_OFF + j * BPS + 8]);
            self.vp[o..o + 8].copy_from_slice(&yb[V_OFF + j * BPS..V_OFF + j * BPS + 8]);
            o += uvs;
        }
    }
}

// v8_bmode: a 4x4 sub-block mode decoded with the 9 probabilities p (libwebp's tree)
#[inline]
fn bmode(b: &mut Br, p: &[u8]) -> u8 {
    if b.bit(p[0] as u32) == 0 {
        return B_DC;
    }
    if b.bit(p[1] as u32) == 0 {
        return B_TM;
    }
    if b.bit(p[2] as u32) == 0 {
        return B_VE;
    }
    if b.bit(p[3] as u32) == 0 {
        if b.bit(p[4] as u32) == 0 {
            return B_HE;
        }
        return if b.bit(p[5] as u32) == 0 { B_RD } else { B_VR };
    }
    if b.bit(p[6] as u32) == 0 {
        return B_LD;
    }
    if b.bit(p[7] as u32) == 0 {
        return B_VL;
    }
    if b.bit(p[8] as u32) == 0 { B_HD } else { B_HU }
}

// NZCODE: libwebp NzCodeBits code of a block: nz > 3 -> 3, > 1 -> 2, else first
// coefficient != 0
#[inline(always)]
fn nzcode(n: usize, c0: i16) -> u32 {
    if n > 3 {
        3
    } else if n > 1 {
        2
    } else {
        (c0 != 0) as u32
    }
}

// v8_coeffs: libwebp GetCoeffs(probabilities of the block type p, context ctx, first
// position n, dequantiser dq, output out) -> position after the last coefficient
#[inline]
fn coeffs(br: &mut Br, p: &[u8; 264], ctx: usize, first: usize, dq: [i32; 2], out: &mut [i16]) -> usize {
    // the reader in a local, so its state stays in registers (written back at the end)
    let mut b = *br;
    let n = coeffs_in(&mut b, p, ctx, first, dq, out);
    *br = b;
    n
}

#[inline(always)]
fn coeffs_in(b: &mut Br, p: &[u8; 264], ctx: usize, first: usize, dq: [i32; 2], out: &mut [i16]) -> usize {
    let out: &mut [i16; 16] = (&mut out[..16]).try_into().unwrap();
    let mut n = first;
    let mut q = BANDOFF[n] as usize + ctx * 11; // p = probas[band(n)][ctx]
    while n < 16 {
        if b.bit(p[q] as u32) == 0 {
            return n; // end of block
        }
        while b.bit(p[q + 1] as u32) == 0 {
            n += 1; // zero coefficient
            if n == 16 {
                return 16;
            }
            q = BANDOFF[n] as usize;
        }
        let v;
        let nq;
        if b.bit(p[q + 2] as u32) == 0 {
            v = 1;
            nq = 11;
        } else {
            v = large(b, &p[q..]);
            nq = 22;
        }
        let nb = BANDOFF[n + 1] as usize + nq;
        let s = b.signed(v);
        out[ZIGZAG[n] as usize & 15] = (s * dq[(n > 0) as usize]) as i16; // int16 like libwebp
        q = nb;
        n += 1;
    }
    16
}

// v8_large: libwebp GetLargeValue (>= 2)
#[inline(always)]
fn large(b: &mut Br, p: &[u8]) -> i32 {
    if b.bit(p[3] as u32) == 0 {
        if b.bit(p[4] as u32) == 0 {
            return 2;
        }
        return 3 + b.bit(p[5] as u32) as i32;
    }
    if b.bit(p[6] as u32) == 0 {
        if b.bit(p[7] as u32) == 0 {
            return 5 + b.bit(159) as i32;
        }
        let v = 7 + 2 * b.bit(165) as i32;
        return v + b.bit(145) as i32;
    }
    let b1 = b.bit(p[8] as u32);
    let b0 = b.bit(p[9 + b1 as usize] as u32);
    let cat = (2 * b1 + b0) as usize; // category 0..3 (cat 3..6)
    let mut v = 0i32;
    for &pr in &CAT[cat * 12..cat * 12 + 12] {
        if pr == 0 {
            break;
        }
        v = 2 * v + b.bit(pr as u32) as i32;
    }
    v + 3 + (8 << cat)
}

// .rc_check: libwebp CheckMode: DC at the left / top picture edges becomes DC_NOLEFT /
// DC_NOTOP / DC_NOTOPLEFT
#[inline]
fn check_mode(mx: u32, my: u32, mode: u8) -> u8 {
    let m = mode & 3;
    if m != 0 {
        return m;
    }
    if mx == 0 {
        if my == 0 { DC_NOTOPLEFT } else { DC_NOLEFT }
    } else if my == 0 {
        DC_NOTOP
    } else {
        B_DC
    }
}

// ------------------------------------------------------------------ transforms

// v8_dotx: libwebp DoTransform: code 3 full transform (the SSE2 version), 2
// TransformAC3 (C), 1 TransformDC (C), 0 nothing
#[inline]
fn dotx(c: &[i16], buf: &mut [u8], at: usize, code: u32) {
    match code {
        3 => itx(c, buf, at),
        2 => itx_ac3(c, buf, at),
        1 => itx_dc(c, buf, at),
        _ => {}
    }
}

// v8_uvtx: libwebp DoUVTransform for one 8x8 chroma plane: any AC -> the full transform
// on all 4 blocks, else TransformDC on the blocks with a DC
fn uvtx(bits: u32, buf: &mut [u8], at: usize, c: &[i16]) {
    if bits & 0xFF == 0 {
        return;
    }
    if bits & 0xAA != 0 {
        for k in 0..4 {
            itx(&c[k * 16..k * 16 + 16], buf, at + UVOFF[k]);
        }
    } else {
        for k in 0..4 {
            if c[k * 16] != 0 {
                itx_dc(&c[k * 16..k * 16 + 16], buf, at + UVOFF[k]);
            }
        }
    }
}

// .tx_pass: one 1-D pass of Transform_SSE2 on the 16-bit rows (in0 / dc, in1, in2, in3)
// -> (a + d, b + c, b - c, a - d), 16-bit wrap-around (c = MUL2(in1) - MUL1(in3), d =
// MUL1(in1) + MUL2(in3) with pmulhw by 20091 / -30068)
#[inline(always)]
fn tx_pass(s: Sse2, i0: V, i1: V, i2: V, i3: V) -> (V, V, V, V) {
    let k1 = s.splat16(20091);
    let k2 = s.splat16((-30068i16) as u16);
    let a = s.add16(i0, i2);
    let b = s.sub16(i0, i2);
    let c = s.add16(s.sub16(s.mulhis16(i1, k2), s.mulhis16(i3, k1)), s.sub16(i1, i3));
    let d = s.add16(s.add16(s.mulhis16(i1, k1), s.mulhis16(i3, k2)), s.add16(i1, i3));
    (s.add16(a, d), s.add16(b, c), s.sub16(b, c), s.sub16(a, d))
}

// 4 bytes of the work buffer
#[inline(always)]
fn ld32(s: Sse2, buf: &[u8], at: usize) -> V {
    s.load_lane32(&buf[at..at + 4])
}

#[inline(always)]
fn st32(s: Sse2, buf: &mut [u8], at: usize, v: V) {
    buf[at..at + 4].copy_from_slice(&(s.to_i32(v) as u32).to_le_bytes());
}

// dst rows r, r + 1 += the 8 words of v (rows r | r + 1), saturated to bytes
#[inline(always)]
fn add_rows2(s: Sse2, buf: &mut [u8], at: usize, v: V) {
    let d = unpacklo32(s, ld32(s, buf, at), ld32(s, buf, at + BPS));
    let w = s.add16(s.unpacklo8(d, s.zero()), v);
    let p = s.packus16(w, w);
    st32(s, buf, at, p);
    st32(s, buf, at + BPS, s.bsrl::<4>(p));
}

/// inverse DCT of one 4x4 block added to buf at `at` (stride BPS), the exact arithmetic of
/// libwebp's Transform_SSE2 (16-bit lanes, 20091 / -30068 multipliers, wrap-around)
/// [vp8_itx]
pub fn itx(c: &[i16], buf: &mut [u8], at: usize) {
    let s = Sse2;
    let r01 = s.load_i16(&c[0..8]);
    let r23 = s.load_i16(&c[8..16]);
    let (t0, t1, t2, t3) = tx_pass(s, r01, s.shuf32::<0x4E>(r01), r23, s.shuf32::<0x4E>(r23));
    // transpose tmp0, tmp1, tmp2, tmp3
    let x4 = s.unpacklo16(t0, t1);
    let x0 = s.unpacklo16(t2, t3);
    let a = unpacklo32(s, x4, x0); // T0 | T1
    let b = unpackhi32(s, x4, x0); // T2 | T3
    let dc = s.add16(a, s.splat16(4)); // dc = T0 + 4
    let (o0, o1, o2, o3) = tx_pass(s, dc, s.shuf32::<0x4E>(a), b, s.shuf32::<0x4E>(b));
    let x4 = s.unpacklo16(s.sra16::<3>(o0), s.sra16::<3>(o1));
    let x0 = s.unpacklo16(s.sra16::<3>(o2), s.sra16::<3>(o3));
    add_rows2(s, buf, at, unpacklo32(s, x4, x0)); // rows 0 | 1
    add_rows2(s, buf, at + 2 * BPS, unpackhi32(s, x4, x0)); // rows 2 | 3
}

// v8_itx_ac3: libwebp TransformAC3_C (coefficients 0, 1, 4 only; int arithmetic)
fn itx_ac3(c: &[i16], buf: &mut [u8], at: usize) {
    let s = Sse2;
    let a = c[0] as i32 + 4;
    let i4 = c[4] as i32;
    let c4 = (i4 * 35468) >> 16;
    let d4 = ((i4 * 20091) >> 16) + i4;
    let i1 = c[1] as i32;
    let c1 = (i1 * 35468) >> 16;
    let d1 = ((i1 * 20091) >> 16) + i1;
    let lanes = s.load_i32(&[d1, c1, -c1, -d1]);
    let z = s.zero();
    for (y, dc) in [a + d4, a + c4, a - c4, a - d4].into_iter().enumerate() {
        let r = at + y * BPS;
        let v = s.sra32::<3>(s.add32(s.splat32(dc as u32), lanes));
        let d = s.unpacklo16(s.unpacklo8(ld32(s, buf, r), z), z);
        let w = s.add32(v, d);
        let w = s.packs32(w, w);
        st32(s, buf, r, s.packus16(w, w));
    }
}

/// libwebp TransformDC_C: dst += (in[0] + 4) >> 3  [vp8_itx_dc]
pub fn itx_dc(c: &[i16], buf: &mut [u8], at: usize) {
    let s = Sse2;
    let v = s.splat16(((c[0] as i32 + 4) >> 3) as i16 as u16);
    let z = s.zero();
    for j in 0..4 {
        let r = at + j * BPS;
        let w = s.add16(s.unpacklo8(ld32(s, buf, r), z), v);
        st32(s, buf, r, s.packus16(w, w));
    }
}

/// libwebp TransformWHT_C: second-order inverse WHT of input[16] into the DC of the 16
/// luma blocks (out[16 * n], int16 truncation)  [vp8_wht]
pub fn wht(input: &[i16; 16], out: &mut [i16]) {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let a0 = input[i] as i32 + input[12 + i] as i32;
        let a1 = input[4 + i] as i32 + input[8 + i] as i32;
        let a2 = input[4 + i] as i32 - input[8 + i] as i32;
        let a3 = input[i] as i32 - input[12 + i] as i32;
        tmp[i] = a0 + a1;
        tmp[8 + i] = a0 - a1;
        tmp[4 + i] = a3 + a2;
        tmp[12 + i] = a3 - a2;
    }
    for i in 0..4 {
        let dc = tmp[i * 4] + 3;
        let a0 = dc + tmp[3 + i * 4];
        let a1 = tmp[1 + i * 4] + tmp[2 + i * 4];
        let a2 = tmp[1 + i * 4] - tmp[2 + i * 4];
        let a3 = dc - tmp[3 + i * 4];
        let o = i * 64;
        out[o] = ((a0 + a1) >> 3) as i16;
        out[o + 16] = ((a3 + a2) >> 3) as i16;
        out[o + 32] = ((a0 - a1) >> 3) as i16;
        out[o + 48] = ((a3 - a2) >> 3) as i16;
    }
}

// ------------------------------------------------------------------ predictors
// All take the destination `at` in the BPS work buffer (top row at -BPS, left column at
// -1).

fn fill(buf: &mut [u8], at: usize, n: usize, v: u8) {
    for j in 0..n {
        buf[at + j * BPS..at + j * BPS + n].fill(v);
    }
}

fn sum_top(buf: &[u8], at: usize, n: usize) -> u32 {
    buf[at - BPS..at - BPS + n].iter().map(|&v| v as u32).sum()
}

fn sum_left(buf: &[u8], at: usize, n: usize) -> u32 {
    (0..n).map(|j| buf[at + j * BPS - 1] as u32).sum()
}

// v8_tm: TrueMotion of n = 4 / 8 / 16 pixels: clip(top[x] + left[y] - top_left)
fn tm(buf: &mut [u8], at: usize, n: usize) {
    let tl = buf[at - BPS - 1] as i32;
    let mut top = [0i32; 16];
    for x in 0..n {
        top[x] = buf[at - BPS + x] as i32;
    }
    for j in 0..n {
        let d = buf[at + j * BPS - 1] as i32 - tl;
        for x in 0..n {
            buf[at + j * BPS + x] = (top[x] + d).clamp(0, 255) as u8;
        }
    }
}

// v8_pred16: the 16x16 predictors (DC, TM, VE, HE, DC_NOTOP, DC_NOLEFT, DC_NOTOPLEFT)
fn pred16(buf: &mut [u8], at: usize, mode: u8) {
    match mode {
        B_DC => {
            let v = (sum_top(buf, at, 16) + sum_left(buf, at, 16) + 16) >> 5;
            fill(buf, at, 16, v as u8);
        }
        B_TM => tm(buf, at, 16),
        B_VE => {
            let mut t = [0u8; 16];
            t.copy_from_slice(&buf[at - BPS..at - BPS + 16]);
            for j in 0..16 {
                buf[at + j * BPS..at + j * BPS + 16].copy_from_slice(&t);
            }
        }
        B_HE => {
            for j in 0..16 {
                let v = buf[at + j * BPS - 1];
                buf[at + j * BPS..at + j * BPS + 16].fill(v);
            }
        }
        DC_NOTOP => {
            let v = (sum_left(buf, at, 16) + 8) >> 4;
            fill(buf, at, 16, v as u8);
        }
        DC_NOLEFT => {
            let v = (sum_top(buf, at, 16) + 8) >> 4;
            fill(buf, at, 16, v as u8);
        }
        _ => fill(buf, at, 16, 0x80),
    }
}

// v8_pred8: the 8x8 chroma predictors
fn pred8(buf: &mut [u8], at: usize, mode: u8) {
    match mode {
        B_DC => {
            let v = (sum_top(buf, at, 8) + sum_left(buf, at, 8) + 8) >> 4;
            fill(buf, at, 8, v as u8);
        }
        B_TM => tm(buf, at, 8),
        B_VE => {
            let mut t = [0u8; 8];
            t.copy_from_slice(&buf[at - BPS..at - BPS + 8]);
            for j in 0..8 {
                buf[at + j * BPS..at + j * BPS + 8].copy_from_slice(&t);
            }
        }
        B_HE => {
            for j in 0..8 {
                let v = buf[at + j * BPS - 1];
                buf[at + j * BPS..at + j * BPS + 8].fill(v);
            }
        }
        DC_NOTOP => {
            let v = (sum_left(buf, at, 8) + 4) >> 3;
            fill(buf, at, 8, v as u8);
        }
        DC_NOLEFT => {
            let v = (sum_top(buf, at, 8) + 4) >> 3;
            fill(buf, at, 8, v as u8);
        }
        _ => fill(buf, at, 8, 0x80),
    }
}

/// the 4x4 predictors (modes 0..9) at `at`; modes VE .. HU from the recipe table (edge
/// e[13] = L K J I X A B C D E F G H)  [v8_pred4]
pub fn pred4(buf: &mut [u8], at: usize, mode: u8) {
    match mode {
        B_DC => {
            let v = (sum_top(buf, at, 4) + sum_left(buf, at, 4) + 4) >> 3;
            fill(buf, at, 4, v as u8);
        }
        B_TM => tm(buf, at, 4),
        _ => {
            let mut e = [0u8; 13];
            e[0] = buf[at + 3 * BPS - 1];
            e[1] = buf[at + 2 * BPS - 1];
            e[2] = buf[at + BPS - 1];
            e[3] = buf[at - 1];
            e[4] = buf[at - BPS - 1];
            e[5..13].copy_from_slice(&buf[at - BPS..at - BPS + 8]);
            let r = &P4TAB[(mode as usize - 2) * 64..];
            for p in 0..16 {
                let op = r[p * 4];
                let a = e[r[p * 4 + 1] as usize] as u32;
                let v = if op == 2 {
                    a
                } else {
                    let b = e[r[p * 4 + 2] as usize] as u32;
                    if op == 1 {
                        (a + b + 1) >> 1
                    } else {
                        let c = e[r[p * 4 + 3] as usize] as u32;
                        (a + 2 * b + 2 + c) >> 2
                    }
                };
                buf[at + (p >> 2) * BPS + (p & 3)] = v as u8;
            }
        }
    }
}

// ------------------------------------------------------------------ loop filter (SSE2)
// 16 lanes at a time: one luma edge of 16 pixels, or the 8 U and 8 V pixels of a chroma
// edge (same thresholds, like libwebp's HFilter8 / VFilter8).  The rows p3 p2 p1 p0 q0 q1
// q2 q3 across the edge are 8 registers; the arithmetic is the assembly's (= libwebp's C
// filters): NeedsFilter 4|p0-q0| + |p1-q1| <= 2t + 1 as 2|p0-q0| + |p1-q1|/2 <= t
// (saturating), DoFilter2 / 4 / 6 with the signed clamps as saturating byte arithmetic.
// SSE2 only, like the assembly (every lane is independent).

type V = <Sse2 as Simd>::V;

/// an edge job: edge limit, interior limit, hev threshold, mode (0 simple, 1 inner edge,
/// 2 macroblock edge)
#[derive(Clone, Copy)]
struct Edge {
    t: u8,
    it: u8,
    hev: u8,
    mode: u8,
}

// ABSD: |a - b| (bytes)
#[inline(always)]
fn absd(s: Sse2, a: V, b: V) -> V {
    s.or(s.subus8(a, b), s.subus8(b, a))
}

// SSHR: signed bytes >> 3 / >> 1 (arithmetic)
#[inline(always)]
fn sshr3(s: Sse2, x: V) -> V {
    let z = s.zero();
    s.packs16(s.sra16::<11>(s.unpacklo8(z, x)), s.sra16::<11>(s.unpackhi8(z, x)))
}

#[inline(always)]
fn sshr1(s: Sse2, x: V) -> V {
    let z = s.zero();
    s.packs16(s.sra16::<9>(s.unpacklo8(z, x)), s.sra16::<9>(s.unpackhi8(z, x)))
}

// BLEND: m ? new : old
#[inline(always)]
fn blend(s: Sse2, old: V, new: V, m: V) -> V {
    s.or(s.and(new, m), s.andnot(m, old))
}

// row + d / row - d (signed saturating, in the signed domain)
#[inline(always)]
fn sadd(s: Sse2, row: V, d: V) -> V {
    let k = s.splat8(0x80);
    s.xor(s.adds8(s.xor(row, k), d), k)
}

#[inline(always)]
fn ssub(s: Sse2, row: V, d: V) -> V {
    let k = s.splat8(0x80);
    s.xor(s.subs8(s.xor(row, k), d), k)
}

// .lk_f6: w = (k * a + 63) >> 7 as signed bytes (wlo / whi: a as words)
#[inline(always)]
fn f6w(s: Sse2, wlo: V, whi: V, k: u16) -> V {
    let kk = s.splat16(k);
    let c63 = s.splat16(63);
    let lo = s.sra16::<7>(s.add16(s.mullo16(wlo, kk), c63));
    let hi = s.sra16::<7>(s.add16(s.mullo16(whi, kk), c63));
    s.packs16(lo, hi)
}

// v8_lfk: filter the 16 lanes of r = [p3, p2, p1, p0, q0, q1, q2, q3] with the edge job
#[inline(always)]
fn lfk(s: Sse2, r: &mut [V; 8], e: Edge) {
    let z = s.zero();
    let d0 = absd(s, r[3], r[4]);
    let mut m = s.addus8(d0, d0); // 2 |p0 - q0|
    let d1 = s.srl16::<1>(s.and(absd(s, r[2], r[5]), s.splat8(0xFE))); // |p1 - q1| / 2
    m = s.addus8(m, d1);
    m = s.cmpeq8(s.subus8(m, s.splat8(e.t)), z); // edge mask
    let (m2, mn);
    if e.mode != 0 {
        let mut i = s.maxu8(absd(s, r[0], r[1]), absd(s, r[1], r[2]));
        i = s.maxu8(i, absd(s, r[7], r[6]));
        i = s.maxu8(i, absd(s, r[6], r[5]));
        let h = s.maxu8(absd(s, r[2], r[3]), absd(s, r[5], r[4])); // high edge variance
        i = s.maxu8(i, h); // interior differences
        m = s.and(m, s.cmpeq8(s.subus8(i, s.splat8(e.it)), z)); // filter mask
        let nh = s.cmpeq8(s.subus8(h, s.splat8(e.hev)), z); // not hev
        m2 = s.andnot(nh, m); // mask & hev -> DoFilter2
        mn = s.and(m, nh); // mask & !hev -> DoFilter4 / 6
    } else {
        m2 = m;
        mn = z;
    }
    // .lk_a: A = clamp(3 (q0 - p0) + clamp(p1 - q1)), D = clamp(q0 - p0)
    let k80 = s.splat8(0x80);
    let dd = s.subs8(s.xor(r[4], k80), s.xor(r[3], k80));
    let mut a = s.subs8(s.xor(r[2], k80), s.xor(r[5], k80));
    a = s.adds8(a, dd);
    a = s.adds8(a, dd);
    a = s.adds8(a, dd);
    if e.mode == 2 {
        // macroblock edge: DoFilter6 on !hev lanes
        let wlo = s.sra16::<8>(s.unpacklo8(a, a));
        let whi = s.sra16::<8>(s.unpackhi8(a, a));
        for (k, p, q) in [(27u16, 3usize, 4usize), (18, 2, 5), (9, 1, 6)] {
            let w = f6w(s, wlo, whi, k);
            r[p] = blend(s, r[p], sadd(s, r[p], w), mn);
            r[q] = blend(s, r[q], ssub(s, r[q], w), mn);
        }
    } else if e.mode == 1 {
        // inner edge: DoFilter4 on !hev lanes
        let a4 = s.adds8(s.adds8(dd, dd), dd); // a = 3 (q0 - p0)
        let a1 = sshr3(s, s.adds8(a4, s.splat8(4)));
        let a2 = sshr3(s, s.adds8(a4, s.splat8(3)));
        let a3 = sshr1(s, s.adds8(a1, s.splat8(1))); // (a1 + 1) >> 1
        r[2] = blend(s, r[2], sadd(s, r[2], a3), mn);
        r[5] = blend(s, r[5], ssub(s, r[5], a3), mn);
        r[3] = blend(s, r[3], sadd(s, r[3], a2), mn);
        r[4] = blend(s, r[4], ssub(s, r[4], a1), mn);
    }
    // DoFilter2 on the lanes of m2: p0 += a2, q0 -= a1
    let a1 = sshr3(s, s.adds8(a, s.splat8(4)));
    let a2 = sshr3(s, s.adds8(a, s.splat8(3)));
    r[3] = blend(s, r[3], sadd(s, r[3], a2), m2);
    r[4] = blend(s, r[4], ssub(s, r[4], a1), m2);
}

// punpck{l,h}{dq,qdq} through shufps / pshufd
#[inline(always)]
fn unpacklo32(s: Sse2, a: V, b: V) -> V {
    s.shuf32::<0xD8>(s.shufps::<0x44>(a, b))
}

#[inline(always)]
fn unpackhi32(s: Sse2, a: V, b: V) -> V {
    s.shuf32::<0xD8>(s.shufps::<0xEE>(a, b))
}

#[inline(always)]
fn lo64(s: Sse2, a: V, b: V) -> V {
    s.shufps::<0x44>(a, b)
}

#[inline(always)]
fn hi64(s: Sse2, a: V, b: V) -> V {
    s.shufps::<0xEE>(a, b)
}

// v8_tr_s2r: 16 rows of 8 bytes (low halves) -> 8 columns of 16 lanes
#[inline(always)]
fn tr_s2r(s: Sse2, rows: &[V; 16]) -> [V; 8] {
    let mut a = [s.zero(); 8];
    for k in 0..8 {
        a[k] = s.unpacklo8(rows[2 * k], rows[2 * k + 1]); // rows 2k, 2k + 1 interleaved
    }
    let mut blo = [s.zero(); 4];
    let mut bhi = [s.zero(); 4];
    for k in 0..4 {
        blo[k] = s.unpacklo16(a[2 * k], a[2 * k + 1]); // 4 rows, columns 0..3
        bhi[k] = s.unpackhi16(a[2 * k], a[2 * k + 1]); // columns 4..7
    }
    let mut c = [s.zero(); 8];
    for (h, k) in [(0usize, 0usize), (1, 2)] {
        // rows 0..7 (h 0) / 8..15 (h 1): two columns per register (qwords)
        c[h * 4] = unpacklo32(s, blo[k], blo[k + 1]); // columns 0, 1
        c[h * 4 + 1] = unpackhi32(s, blo[k], blo[k + 1]); // columns 2, 3
        c[h * 4 + 2] = unpacklo32(s, bhi[k], bhi[k + 1]); // columns 4, 5
        c[h * 4 + 3] = unpackhi32(s, bhi[k], bhi[k + 1]); // columns 6, 7
    }
    let mut o = [s.zero(); 8];
    for j in 0..4 {
        o[2 * j] = lo64(s, c[j], c[4 + j]);
        o[2 * j + 1] = hi64(s, c[j], c[4 + j]);
    }
    o
}

// v8_tr_r2s: the inverse: 8 columns of 16 lanes -> 16 rows (two per register: row 2k in
// the low 8 bytes, 2k + 1 in the high 8 bytes; registers 0..3 rows 0..7, 4..7 rows 8..15)
#[inline(always)]
fn tr_r2s(s: Sse2, c: &[V; 8]) -> [V; 8] {
    let mut elo = [s.zero(); 4];
    let mut ehi = [s.zero(); 4];
    for j in 0..4 {
        elo[j] = s.unpacklo8(c[2 * j], c[2 * j + 1]); // rows 0..7: columns 2j, 2j + 1
        ehi[j] = s.unpackhi8(c[2 * j], c[2 * j + 1]); // rows 8..15
    }
    let mut o = [s.zero(); 8];
    for h in 0..2 {
        let e = if h == 0 { &elo } else { &ehi };
        let f0 = s.unpacklo16(e[0], e[1]); // rows 0..3: columns 0..3
        let f1 = s.unpackhi16(e[0], e[1]); // rows 4..7
        let g0 = s.unpacklo16(e[2], e[3]); // rows 0..3: columns 4..7
        let g1 = s.unpackhi16(e[2], e[3]);
        o[h * 4] = unpacklo32(s, f0, g0); // rows 0, 1
        o[h * 4 + 1] = unpackhi32(s, f0, g0); // rows 2, 3
        o[h * 4 + 2] = unpacklo32(s, f1, g1); // rows 4, 5
        o[h * 4 + 3] = unpackhi32(s, f1, g1); // rows 6, 7
    }
    o
}

// v8_lfv (luma): filter across a horizontal edge at row `at` (16 columns), rows -4..3
fn lfv(pl: &mut [u8], at: usize, stride: usize, e: Edge) {
    let s = Sse2;
    let base = at - 4 * stride;
    let mut r = [s.zero(); 8];
    for k in 0..8 {
        let o = base + k * stride;
        r[k] = s.load(&pl[o..o + 16]);
    }
    lfk(s, &mut r, e);
    for k in 1..7 {
        let o = base + k * stride;
        s.store(&mut pl[o..o + 16], r[k]);
    }
}

// v8_lfv (chroma): lanes 0..7 in U, 8..15 in V
fn lfv_uv(u: &mut [u8], v: &mut [u8], at: usize, stride: usize, e: Edge) {
    let s = Sse2;
    let base = at - 4 * stride;
    let mut r = [s.zero(); 8];
    for k in 0..8 {
        let o = base + k * stride;
        r[k] = lo64(s, s.load_raw(&u[o..o + 8]), s.load_raw(&v[o..o + 8]));
    }
    lfk(s, &mut r, e);
    for k in 1..7 {
        let o = base + k * stride;
        s.store_raw(&mut u[o..o + 8], r[k]);
        s.store_raw(&mut v[o..o + 8], s.bsrl::<8>(r[k]));
    }
}

// v8_lfh (luma): filter across a vertical edge at column `at`: rows 0..15, columns
// -4..3 transposed into lanes and back
fn lfh(pl: &mut [u8], at: usize, stride: usize, e: Edge) {
    let s = Sse2;
    let mut rows = [s.zero(); 16];
    for y in 0..16 {
        let o = at + y * stride - 4;
        rows[y] = s.load_raw(&pl[o..o + 8]);
    }
    let mut c = tr_s2r(s, &rows);
    lfk(s, &mut c, e);
    let o2 = tr_r2s(s, &c);
    for k in 0..8 {
        let y = (k / 4) * 8 + (k % 4) * 2;
        let o = at + y * stride - 4;
        s.store_raw(&mut pl[o..o + 8], o2[k]);
        let o = o + stride;
        s.store_raw(&mut pl[o..o + 8], s.bsrl::<8>(o2[k]));
    }
}

// v8_lfh (chroma): rows 0..7 of U, then 0..7 of V
fn lfh_uv(u: &mut [u8], v: &mut [u8], at: usize, stride: usize, e: Edge) {
    let s = Sse2;
    let mut rows = [s.zero(); 16];
    for y in 0..8 {
        let o = at + y * stride - 4;
        rows[y] = s.load_raw(&u[o..o + 8]);
        rows[8 + y] = s.load_raw(&v[o..o + 8]);
    }
    let mut c = tr_s2r(s, &rows);
    lfk(s, &mut c, e);
    let o2 = tr_r2s(s, &c);
    for k in 0..8 {
        let x = o2[k];
        let pl: &mut [u8] = if k < 4 { &mut *u } else { &mut *v };
        let y = (k % 4) * 2;
        let o = at + y * stride - 4;
        s.store_raw(&mut pl[o..o + 8], x);
        let o = o + stride;
        s.store_raw(&mut pl[o..o + 8], s.bsrl::<8>(x));
    }
}

impl V8<'_> {
    // v8_filter_row: loop filter of the macroblocks of row mby in the planes (libwebp
    // DoFilter order: left edge, inner vertical edges, top edge, inner horizontal edges;
    // U / V with the normal filter only)
    fn filter_row(&mut self) {
        let simple = self.ftype == 1;
        let (ys, uvs) = (self.ys, self.uvs);
        let my = self.mby as usize;
        for mx in 0..self.mbw as usize {
            let f = self.finfo[mx];
            let limit = (f & 0xFF) as u8;
            if limit == 0 {
                continue; // no filtering
            }
            let it = (f >> 8) as u8;
            let inner = (f >> 16) & 1 != 0;
            let hev = (f >> 24) as u8;
            let yo = my * 16 * ys + mx * 16;
            let uo = my * 8 * uvs + mx * 8;
            let mbe = Edge { t: limit.wrapping_add(4), it, hev, mode: if simple { 0 } else { 2 } };
            let ine = Edge { t: limit, it, hev, mode: if simple { 0 } else { 1 } };
            // left edge
            if mx > 0 {
                lfh(&mut self.yp, yo, ys, mbe);
                if !simple {
                    lfh_uv(&mut self.up, &mut self.vp, uo, uvs, mbe);
                }
            }
            // inner vertical edges
            if inner {
                for x in [4, 8, 12] {
                    lfh(&mut self.yp, yo + x, ys, ine);
                }
                if !simple {
                    lfh_uv(&mut self.up, &mut self.vp, uo + 4, uvs, ine);
                }
            }
            // top edge
            if my > 0 {
                lfv(&mut self.yp, yo, ys, mbe);
                if !simple {
                    lfv_uv(&mut self.up, &mut self.vp, uo, uvs, mbe);
                }
            }
            // inner horizontal edges
            if inner {
                for y in [4, 8, 12] {
                    lfv(&mut self.yp, yo + y * ys, ys, ine);
                }
                if !simple {
                    lfv_uv(&mut self.up, &mut self.vp, uo + 4 * uvs, uvs, ine);
                }
            }
        }
    }

    // ------------------------------------------------------------------ output

    // v8_output: libwebp's fancy upsampling of U / V (EmitFancyRGB +
    // UpsampleRgbaLinePair) and YUV -> RGB into the FMT_ARGB surface, one row at a time
    fn output(&self, sf: &mut Surface) {
        let (w, h) = (self.w as usize, self.h as usize);
        let mut ur = vec![0u8; w + 64];
        let mut vr = vec![0u8; w + 64];
        let last = (h + 1) / 2 - 1; // last chroma row
        let st = sf.stride as usize;
        let uvs = self.uvs;
        for y in 0..h {
            // chroma rows: y 0 -> (0, 0); odd y -> near (y - 1) / 2, far min((y + 1) / 2,
            // last); even y -> near y / 2, far y / 2 - 1
            let (near, far) = if y == 0 {
                (0, 0)
            } else if y & 1 != 0 {
                ((y - 1) / 2, ((y + 1) / 2).min(last))
            } else {
                (y / 2, y / 2 - 1)
            };
            uprow(&self.up[near * uvs..], &self.up[far * uvs..], &mut ur, w);
            uprow(&self.vp[near * uvs..], &self.vp[far * uvs..], &mut vr, w);
            cvtrow(&self.yp[y * self.ys..], &ur, &vr, &mut sf.px[y * st..y * st + w * 4], w);
        }
    }
}

// v8_uprow: one row of libwebp's fancy chroma upsampling: out[0] and (even w) out[w - 1] =
// (3 n + f + 2) >> 2; out[2j - 1] = ((n[j-1] + 3 n[j] + 3 f[j-1] + f[j] + 8) >> 3 + n[j-1])
// >> 1; out[2j] = ((3 n[j-1] + n[j] + f[j-1] + 3 f[j] + 8) >> 3 + n[j]) >> 1 (8 pairs per
// step; the rows have slack)
fn uprow(n: &[u8], f: &[u8], out: &mut [u8], w: usize) {
    out[0] = ((3 * n[0] as u32 + f[0] as u32 + 2) >> 2) as u8;
    let lastp = (w - 1) >> 1;
    let s = Sse2;
    let z = s.zero();
    let k8 = s.splat16(8);
    let mut j = 1;
    while j <= lastp {
        let a = s.unpacklo8(s.load_raw(&n[j - 1..j + 7]), z); // n[j - 1]
        let b = s.unpacklo8(s.load_raw(&n[j..j + 8]), z); // n[j]
        let c = s.unpacklo8(s.load_raw(&f[j - 1..j + 7]), z); // f[j - 1]
        let d = s.unpacklo8(s.load_raw(&f[j..j + 8]), z); // f[j]
        let ad = s.add16(a, d);
        let bc = s.add16(b, c);
        let aa = s.srl16::<3>(s.add16(s.add16(s.add16(s.add16(bc, bc), bc), ad), k8));
        let bb = s.srl16::<3>(s.add16(s.add16(s.add16(s.add16(ad, ad), ad), bc), k8));
        let o1 = s.srl16::<1>(s.add16(aa, a)); // out[2j - 1]
        let o2 = s.srl16::<1>(s.add16(bb, b)); // out[2j]
        let p = s.unpacklo8(s.packus16(o1, o1), s.packus16(o2, o2));
        s.store(&mut out[2 * j - 1..2 * j + 15], p);
        j += 8;
    }
    if w & 1 == 0 {
        out[w - 1] = ((3 * n[lastp] as u32 + f[lastp] as u32 + 2) >> 2) as u8;
    }
}

// YUV8: 8 pixels (0xFFRRGGBB) of libwebp's VP8YuvToRgb from 8 Y, U, V bytes (the SSE2
// arithmetic of libwebp yuv_sse2.c, equal to its C version)
#[inline(always)]
fn yuv8(s: Sse2, y: &[u8], u: &[u8], v: &[u8]) -> (V, V) {
    let z = s.zero();
    let y1 = s.mulhiu16(s.unpacklo8(z, s.load_raw(y)), s.splat16(19077));
    let u8_ = s.unpacklo8(z, s.load_raw(u));
    let v8_ = s.unpacklo8(z, s.load_raw(v));
    let r = s.sra16::<6>(s.sub16(s.add16(s.mulhiu16(v8_, s.splat16(26149)), y1), s.splat16(14234)));
    let g4 = s.add16(s.mulhiu16(u8_, s.splat16(6419)), s.mulhiu16(v8_, s.splat16(13320)));
    let g = s.sra16::<6>(s.sub16(s.add16(y1, s.splat16(8708)), g4));
    // paddusw in the assembly: the sum stays below 2^16, so it never saturates
    let b = s.srl16::<6>(s.subus16(s.add16(s.mulhiu16(u8_, s.splat16(33050)), y1), s.splat16(17685)));
    let bg = s.unpacklo8(s.packus16(b, b), s.packus16(g, g));
    let ra = s.unpacklo8(s.packus16(r, r), s.splat8(0xFF));
    (s.unpacklo16(bg, ra), s.unpackhi16(bg, ra))
}

// v8_cvtrow: w pixels of VP8YuvToRgba into dst (B, G, R, 255), 8 at a time
fn cvtrow(y: &[u8], u: &[u8], v: &[u8], dst: &mut [u8], w: usize) {
    let s = Sse2;
    let mut x = 0;
    while x + 8 <= w {
        let (p0, p1) = yuv8(s, &y[x..x + 8], &u[x..x + 8], &v[x..x + 8]);
        s.store(&mut dst[x * 4..x * 4 + 16], p0);
        s.store(&mut dst[x * 4 + 16..x * 4 + 32], p1);
        x += 8;
    }
    if x < w {
        let (p0, p1) = yuv8(s, &y[x..x + 8], &u[x..x + 8], &v[x..x + 8]);
        let mut t = [0u8; 32];
        s.store(&mut t[..16], p0);
        s.store(&mut t[16..], p1);
        let n = (w - x) * 4;
        dst[x * 4..x * 4 + n].copy_from_slice(&t[..n]);
    }
}

// VP8Clip8 of a 14-bit fixed-point value
#[inline(always)]
fn clip8(v: i32) -> u32 {
    if v & !16383 == 0 {
        (v >> 6) as u32
    } else if v < 0 {
        0
    } else {
        255
    }
}

/// libwebp VP8YuvToRgb (yuv.h, 14-bit multipliers, 6 fractional bits, VP8Clip8) ->
/// 0xFFRRGGBB  [vp8_yuv_rgb]
#[inline(always)]
pub fn yuv_rgb(y: u32, u: u32, v: u32) -> u32 {
    let y1 = ((y * 19077) >> 8) as i32;
    let r = clip8(y1 + ((v * 26149) >> 8) as i32 - 14234);
    let g = clip8(y1 - ((u * 6419) >> 8) as i32 - ((v * 13320) >> 8) as i32 + 8708);
    let b = clip8(y1 + ((u * 33050) >> 8) as i32 - 17685);
    0xFF00_0000 | r << 16 | g << 8 | b
}
