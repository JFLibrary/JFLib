// enc -- JPEG encoder (baseline JFIF)  [src/img/jpeg_enc.s]
//
//   encode(sf, quality) -> JPEG file / None                        (jpeg_encode)
//   encode_ex(sf, quality, cfg) -> JPEG file / None                (jpeg_encode_ex: EncCfg =
//                                                                   JENCCFG, chroma + density)
//
// Output: SOI, APP0 JFIF 1.01 (96 x 96 dpi unless the EncCfg gives a density), DQT, SOF0,
// DHT, SOS, entropy-coded data, EOI.  Quality 1..100 (clamped) scales the Annex K
// quantisation tables like libjpeg (jpeg_quality_scaling, baseline limit 255).  Colour
// images use YCbCr; chroma subsampling 444 (h1v1), 422 (h2v1, libjpeg's 0 / 1 alternating
// rounding), 420 (h2v2), 400 (luminance only, a 1-component grey JPEG); 0 = 4:2:0 for
// quality < 90 and 4:4:4 for quality >= 90.  The Huffman tables are optimised per image (two
// passes: all blocks are transformed and quantised first, their symbol statistics give the
// tables, libjpeg's jpeg_gen_optimal_table).  DCT: libjpeg islow; colour conversion and
// downsampling round like libjpeg.
//
// Input: FMT_XRGB (alpha ignored), FMT_PARGB (composited over white: c + 255 - a), FMT_ARGB
// (rows are premultiplied first -- identical to encoding the PARGB copy), FMT_A8 (a
// 1-component grey JPEG).  Any size 1..65535 x 1..65535; partial MCUs repeat the last column
// / row.  None for an empty or unsupported surface or when memory runs out.

use alloc::vec::Vec;

use gfx::{Surface, FMT_A8, FMT_ARGB, FMT_PARGB};

use simd::Simd;

use crate::dec::try_vec;
use crate::dsp;

const MAX_SIDE: u32 = 65535;

/// JENCCFG: chroma subsampling 0 (4:4:4 from quality 90, else 4:2:0) / 444 / 422 / 420 / 400;
/// JFIF density unit 0 = keep 96 x 96 dpi, 1 dpi, 2 dots per cm; x / y density 1..65535
#[derive(Clone, Copy, Default)]
pub struct EncCfg {
    pub chroma: u32,
    pub unit: u32,
    pub xd: u32,
    pub yd: u32,
}

/// SOI + APP0 JFIF 1.01, 96 x 96 dpi, no thumbnail (je_hdr)
static HDR: [u8; 20] = [
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x01, 0x00, 0x60, 0x00, 0x60,
    0x00, 0x00,
];

/// the encoder context (JE_*)
struct Enc<'a> {
    sf: &'a Surface,
    div: [[i16; 192]; 2], // u16[64] reciprocal, correction, scale per table
    qzz: [[u8; 64]; 2],   // quantisation values, zigzag order (DQT)
    freq: [[u32; 257]; 4], // DC lum, AC lum, DC chroma, AC chroma
    hbits: [[u8; 16]; 4], // code counts per length
    hvals: [[u8; 256]; 4], // symbols
    hcode: [[u32; 256]; 4], // (length << 16) | code
    coef: Vec<i16>,       // quantised blocks (zigzag order, 64 each)
    cur: usize,           // next coefficient block
    row: Vec<u8>,         // padded BGRA row
    ys: Vec<u8>,          // strips (MCU height x PADW)
    cbs: Vec<u8>,
    crs: Vec<u8>,
    cbd: Vec<u8>, // downsampled chroma (8 x PADW / 2)
    crd: Vec<u8>,
    bits: u64, // entropy-coded bits (estimate for the buffer)
    w: u32,
    h: u32,
    fmt: u8,
    nc: u32,
    sub: u32, // 0 = 4:4:4, 1 = 4:2:0, 2 = 4:2:2
    mcu: u32, // MCU width in pixels (8 or 16)
    mcux: u32,
    mcuy: u32,
    padw: u32,
    bpm: u32,      // blocks per MCU
    bmap: [u8; 8], // component of each block of an MCU
    pred: [i32; 3],
    qual: u32,
    nblk: u32,
    mcuh: u32, // MCU height in pixels (8 or 16)
    chr: u32,  // requested subsampling
}

impl<'a> Enc<'a> {
    /// je_setup: quantisation tables, geometry, buffers -> false when memory runs out
    fn setup(&mut self) -> bool {
        // libjpeg quality scaling
        let q = self.qual;
        let scale = if q < 50 { 5000 / q } else { 200 - 2 * q };
        for t in 0..2 {
            let base = if t == 0 { &dsp::STD_QL } else { &dsp::STD_QC };
            for (i, &b) in base.iter().enumerate() {
                let v = ((b as u32 * scale + 50) / 100).clamp(1, 255);
                // zigzag copy for the DQT segment: position of natural index i
                let zp = dsp::ZZ.iter().position(|&z| z as usize == i).unwrap_or(0);
                self.qzz[t][zp] = v as u8;
                // reciprocal for the divisor q * 8 (libjpeg-turbo compute_reciprocal)
                let d = (v * 8) as u64;
                let mut r = (63 - d.leading_zeros()) + 16;
                let mut fq = (1u64 << r) / d;
                let fr = (1u64 << r) % d;
                let mut c = (d >> 1) as u32;
                if fr == 0 {
                    fq >>= 1;
                    r -= 1;
                } else if fr <= c as u64 {
                    c += 1;
                } else {
                    fq += 1;
                }
                self.div[t][i] = fq as u16 as i16;
                self.div[t][64 + i] = c as u16 as i16;
                self.div[t][128 + i] = (1u32 << (32 - r)) as u16 as i16;
            }
        }
        // geometry
        self.nc = 1;
        self.sub = 0;
        self.mcu = 8;
        self.mcuh = 8;
        self.bpm = 1;
        self.bmap = [0; 8];
        if self.fmt != FMT_A8 && self.chr != 400 {
            self.nc = 3;
            self.bpm = 3;
            self.bmap = [0, 1, 2, 0, 0, 0, 0, 0];
            let s420 = match self.chr {
                444 => false,
                420 => true,
                422 => {
                    self.sub = 2;
                    self.mcu = 16;
                    self.bpm = 4;
                    self.bmap = [0, 0, 1, 2, 0, 0, 0, 0];
                    false
                }
                _ => self.qual < 90,
            };
            if s420 {
                self.sub = 1;
                self.mcu = 16;
                self.mcuh = 16;
                self.bpm = 6;
                self.bmap = [0, 0, 0, 0, 1, 2, 0, 0];
            }
        }
        self.mcux = (self.w + self.mcu - 1) / self.mcu;
        self.padw = self.mcux * self.mcu;
        self.mcuy = (self.h + self.mcuh - 1) / self.mcuh;
        self.nblk = self.mcuy.wrapping_mul(self.mcux).wrapping_mul(self.bpm);
        let Some(cf) = try_vec(self.nblk as usize * 64, 0i16) else { return false };
        self.coef = cf;
        // buffers: BGRA row (PADW * 4 + 64), 3 strips (MCUH x PADW), 2 half strips (8 x PADW / 2)
        let padw = self.padw as usize;
        let strip = self.mcuh as usize * padw;
        let (Some(row), Some(ys), Some(cbs), Some(crs), Some(cbd), Some(crd)) = (
            try_vec(padw * 4 + 64, 0u8),
            try_vec(strip + 64, 0u8),
            try_vec(strip + 64, 0u8),
            try_vec(strip + 64, 0u8),
            try_vec(padw * 4 + 64, 0u8),
            try_vec(padw * 4 + 64, 0u8),
        ) else {
            return false;
        };
        self.row = row;
        self.ys = ys;
        self.cbs = cbs;
        self.crs = crs;
        self.cbd = cbd;
        self.crd = crd;
        true
    }

    // ============================================================== pass 1

    /// je_pass1: colour convert, DCT and quantise every block into coef (zigzag order) and
    /// count the Huffman symbols
    fn pass1(&mut self) {
        self.cur = 0;
        let padw = self.padw as usize;
        for my in 0..self.mcuy {
            self.rows(my);
            for mx in 0..self.mcux as usize {
                match self.sub {
                    1 => {
                        // four luminance blocks, then Cb and Cr from the downsampled strips
                        let o = mx * 16;
                        self.block(0, o, padw);
                        self.block(0, o + 8, padw);
                        self.block(0, o + padw * 8, padw);
                        self.block(0, o + padw * 8 + 8, padw);
                        self.block(3, mx * 8, padw / 2);
                        self.block(4, mx * 8, padw / 2);
                    }
                    2 => {
                        // two luminance blocks, then Cb and Cr from the horizontally halved strips
                        let o = mx * 16;
                        self.block(0, o, padw);
                        self.block(0, o + 8, padw);
                        self.block(3, mx * 8, padw / 2);
                        self.block(4, mx * 8, padw / 2);
                    }
                    _ => {
                        // one block per component at x = mx * 8
                        self.block(0, mx * 8, padw);
                        if self.nc != 1 {
                            self.block(1, mx * 8, padw);
                            self.block(2, mx * 8, padw);
                        }
                    }
                }
            }
        }
    }

    /// je_rows: fill the Y / Cb / Cr strips for MCU row `my` (rows past the bottom repeat the
    /// last image row, columns past the right edge the last pixel)
    fn rows(&mut self, my: u32) {
        let sf = self.sf;
        let padw = self.padw as usize;
        let w = self.w as usize;
        for r in 0..self.mcuh as usize {
            let y = (my * self.mcuh + r as u32).min(self.h - 1) as usize;
            let src = &sf.px[sf.stride as usize * y..];
            let so = r * padw; // strip offset
            if self.fmt == FMT_A8 {
                // grey: bytes straight into the Y strip
                self.ys[so..so + w].copy_from_slice(&src[..w]);
                let last = src[w - 1];
                self.ys[so + w..so + padw].fill(last);
                continue;
            }
            if self.fmt == FMT_ARGB {
                // straight: premultiplied row (img_get_row FMT_PARGB)
                for (d, s) in self.row[..w * 4].chunks_exact_mut(4).zip(src[..w * 4].chunks_exact(4)) {
                    let a = s[3] as u32;
                    let pm = |c: u8| (((c as u32 * a + 128) * 257) >> 16) as u8;
                    d.copy_from_slice(&[pm(s[0]), pm(s[1]), pm(s[2]), s[3]]);
                }
            } else {
                self.row[..w * 4].copy_from_slice(&src[..w * 4]);
            }
            let last = [self.row[w * 4 - 4], self.row[w * 4 - 3], self.row[w * 4 - 2], self.row[w * 4 - 1]];
            for x in w..padw {
                self.row[x * 4..x * 4 + 4].copy_from_slice(&last);
            }
            let mode = (self.fmt == FMT_PARGB || self.fmt == FMT_ARGB) as u32; // ARGB rows are premultiplied now
            dsp::rgb_ycc(&self.row, &mut self.ys[so..], &mut self.cbs[so..], &mut self.crs[so..], padw, mode);
        }
        match self.sub {
            1 => {
                // 2x2 downsampling of 16 chroma rows into 8
                for r in 0..8 {
                    let o = r * padw / 2;
                    let i = 2 * r * padw;
                    dsp::down_h2v2(&mut self.cbd[o..], &self.cbs[i..], &self.cbs[i + padw..], padw / 2);
                    dsp::down_h2v2(&mut self.crd[o..], &self.crs[i..], &self.crs[i + padw..], padw / 2);
                }
            }
            2 => {
                // 2:1 horizontal downsampling of the 8 chroma rows
                for r in 0..8 {
                    let i = r * padw;
                    let o = i / 2;
                    dsp::down_h2v1(&mut self.cbd[o..], &self.cbs[i..], padw / 2);
                    dsp::down_h2v1(&mut self.crd[o..], &self.crs[i..], padw / 2);
                }
            }
            _ => {}
        }
    }

    /// je_block: DCT, quantise and store one block (zigzag order), count its symbols.
    /// src: 0 Y, 1 Cb, 2 Cr strip, 3 / 4 downsampled Cb / Cr
    fn block(&mut self, src: u32, off: usize, stride: usize) {
        let comp = match src {
            0 => 0,
            1 | 3 => 1,
            _ => 2,
        };
        let s = match src {
            0 => &self.ys[off..],
            1 => &self.cbs[off..],
            2 => &self.crs[off..],
            3 => &self.cbd[off..],
            _ => &self.crd[off..],
        };
        let mut qb = [0i16; 64];
        dsp::fdct_quant(s, stride, &mut qb, &self.div[(comp != 0) as usize]);
        // zigzag reorder
        let zb: &mut [i16; 64] = (&mut self.coef[self.cur * 64..self.cur * 64 + 64]).try_into().unwrap();
        for k in 0..64 {
            zb[k] = qb[dsp::ZZ[k] as usize];
        }
        self.cur += 1;
        // statistics
        let t = if comp != 0 { 2 } else { 0 };
        let dc = zb[0] as i32;
        let diff = dc.wrapping_sub(self.pred[comp]);
        self.pred[comp] = dc;
        self.freq[t][nbits(diff) as usize] += 1;
        let mut nz = nzmask(zb) & !1;
        let mut last = 0u32;
        while nz != 0 {
            let k = nz.trailing_zeros();
            nz &= nz - 1;
            let mut run = k - last - 1;
            last = k;
            while run >= 16 {
                self.freq[t + 1][0xF0] += 1;
                run -= 16;
            }
            let size = nbits(zb[k as usize] as i32);
            self.freq[t + 1][((run << 4) + size) as usize] += 1;
        }
        if last != 63 {
            self.freq[t + 1][0] += 1; // EOB
        }
    }

    // ============================================================== Huffman tables

    /// je_huff: optimal tables for the used classes, code tables, bit count
    fn huff(&mut self) {
        self.bits = 0;
        let nt = if self.nc == 1 { 2 } else { 4 };
        for t in 0..nt {
            self.gen_table(t);
            // canonical codes
            let mut code = 0u32;
            let mut si = 0usize;
            for len in 1..=16u32 {
                for _ in 0..self.hbits[t][len as usize - 1] {
                    let sym = self.hvals[t][si] as usize;
                    self.hcode[t][sym] = len << 16 | code;
                    code += 1;
                    si += 1;
                }
                code <<= 1;
            }
            // bits: sum of freq * (code length + extra bits)
            for s in 0..256usize {
                let f = self.freq[t][s];
                if f != 0 {
                    let extra = if t & 1 != 0 { s as u32 & 15 } else { s as u32 };
                    self.bits += f as u64 * ((self.hcode[t][s] >> 16) + extra) as u64;
                }
            }
        }
    }

    /// je_gen_table: libjpeg jpeg_gen_optimal_table for the counts of table t: 16 code
    /// counts to hbits[t], symbols to hvals[t]
    fn gen_table(&mut self, t: usize) {
        let mut freq = self.freq[t];
        freq[256] = 1; // reserved symbol
        let mut size = [0u32; 257];
        let mut oth = [-1i32; 257];
        let mut bits = [0u32; 264];
        loop {
            // c1 = least frequent (last among equals), c2 = next least frequent
            let (mut c1, mut c2) = (-1i32, -1i32);
            let (mut v1, mut v2) = (u32::MAX, u32::MAX);
            for (i, &f) in freq.iter().enumerate() {
                if f == 0 {
                    continue;
                }
                if f <= v1 {
                    c2 = c1; // old c1 becomes c2
                    v2 = v1;
                    c1 = i as i32;
                    v1 = f;
                } else if f <= v2 {
                    c2 = i as i32;
                    v2 = f;
                }
            }
            if c2 < 0 {
                break;
            }
            let (mut a, mut b) = (c1 as usize, c2 as usize);
            freq[a] += freq[b];
            freq[b] = 0;
            loop {
                size[a] += 1;
                if oth[a] < 0 {
                    break;
                }
                a = oth[a] as usize;
            }
            oth[a] = c2;
            loop {
                size[b] += 1;
                if oth[b] < 0 {
                    break;
                }
                b = oth[b] as usize;
            }
        }
        // count codes per length
        for &s in size.iter() {
            if s != 0 {
                bits[s.min(263) as usize] += 1;
            }
        }
        // limit code lengths to 16 (Annex K.3)
        let mut i = 263;
        while i > 16 {
            while bits[i] != 0 {
                let mut j = i - 2;
                while bits[j] == 0 {
                    j -= 1;
                }
                bits[i] -= 2;
                bits[i - 1] += 1;
                bits[j + 1] += 2;
                bits[j] -= 1;
            }
            i -= 1;
        }
        // remove the reserved code point from the longest length
        let mut i = 16;
        while bits[i] == 0 {
            i -= 1;
            if i == 0 {
                break;
            }
        }
        bits[i] = bits[i].wrapping_sub(1);
        for k in 1..=16 {
            self.hbits[t][k - 1] = bits[k] as u8;
        }
        // symbols sorted by code size
        let mut p = 0;
        for s in 1..=263u32 {
            for (j, &sz) in size[..256].iter().enumerate() {
                if sz == s {
                    self.hvals[t][p] = j as u8;
                    p += 1;
                }
            }
        }
    }

    // ============================================================== output

    /// je_write: headers, pass 2, EOI
    fn write(&mut self, cfg: Option<&EncCfg>) -> Option<Vec<u8>> {
        let mut o: Vec<u8> = Vec::new();
        o.try_reserve(((self.bits >> 3) as usize + 8) * 2 + 4096).ok()?;
        o.extend_from_slice(&HDR); // SOI + APP0 JFIF
        if let Some(c) = cfg {
            if (1..=2).contains(&c.unit) {
                o[13] = c.unit as u8; // units 1 dpi / 2 dpcm
                o[14] = (c.xd >> 8) as u8;
                o[15] = c.xd as u8;
                o[16] = (c.yd >> 8) as u8;
                o[17] = c.yd as u8;
            }
        }
        // DQT
        let nc = self.nc;
        o.extend_from_slice(&[0xFF, 0xDB, 0, if nc == 1 { 67 } else { 132 }]);
        for t in 0..nc.min(2) as usize {
            o.push(t as u8);
            o.extend_from_slice(&self.qzz[t]);
        }
        // SOF0
        let (w, h) = (self.w, self.h);
        o.extend_from_slice(&[0xFF, 0xC0, 0, (nc * 3 + 8) as u8, 8, (h >> 8) as u8, h as u8, (w >> 8) as u8, w as u8]);
        o.push(nc as u8);
        let samp = match self.sub {
            0 => 0x11,
            1 => 0x22,
            _ => 0x21, // 4:2:2
        };
        o.extend_from_slice(&[1, samp, 0]);
        if nc != 1 {
            o.extend_from_slice(&[2, 0x11, 1, 3, 0x11, 1]);
        }
        // DHT with every used table
        let dht = o.len();
        o.extend_from_slice(&[0xFF, 0xC4, 0, 0]);
        let nt = if nc == 1 { 2 } else { 4 };
        for t in 0..nt {
            o.push(((t & 1) << 4 | t >> 1) as u8); // class, table id
            o.extend_from_slice(&self.hbits[t]);
            let n: usize = self.hbits[t].iter().map(|&c| c as usize).sum();
            o.extend_from_slice(&self.hvals[t][..n]);
        }
        let len = o.len() - dht - 2;
        o[dht + 2] = (len >> 8) as u8;
        o[dht + 3] = len as u8;
        // SOS
        o.extend_from_slice(&[0xFF, 0xDA, 0, (nc * 2 + 6) as u8, nc as u8, 1, 0x00]);
        if nc != 1 {
            o.extend_from_slice(&[2, 0x11, 3, 0x11]);
        }
        o.extend_from_slice(&[0, 63, 0]); // Ss, Se, Ah / Al
        self.pass2(&mut o);
        o.extend_from_slice(&[0xFF, 0xD9]); // EOI
        Some(o)
    }

    /// je_pass2: Huffman code every stored block
    fn pass2(&mut self, o: &mut Vec<u8>) {
        let start = o.len();
        o.resize(start + ((self.bits >> 3) as usize + 8) * 2, 0);
        let mut bw = BitW { buf: 0, n: 0, o: &mut o[start..], p: 0 };
        self.pred = [0; 3];
        let mut bi = 0usize; // block within the MCU
        for b in 0..self.nblk as usize {
            let comp = self.bmap[bi] as usize;
            let t = if comp != 0 { 2 } else { 0 };
            let zb: &[i16; 64] = (&self.coef[b * 64..b * 64 + 64]).try_into().unwrap();
            // DC difference
            let dc = zb[0] as i32;
            let diff = dc.wrapping_sub(self.pred[comp]);
            self.pred[comp] = dc;
            let size = nbits(diff);
            let extra = (diff + (diff >> 31)) as u32 & ((1u32 << size) - 1);
            let hc = self.hcode[t][size as usize];
            bw.emit(((hc & 0xFFFF) << size) | extra, size + (hc >> 16));
            // AC coefficients
            let mut nz = nzmask(zb) & !1;
            let mut last = 0u32;
            while nz != 0 {
                let k = nz.trailing_zeros();
                nz &= nz - 1;
                let mut run = k - last - 1;
                last = k;
                while run >= 16 {
                    let z = self.hcode[t + 1][0xF0];
                    bw.emit(z & 0xFFFF, z >> 16);
                    run -= 16;
                }
                let v = zb[k as usize] as i32;
                let size = nbits(v);
                let extra = (v + (v >> 31)) as u32 & ((1u32 << size) - 1);
                let hc = self.hcode[t + 1][((run << 4) + size) as usize];
                bw.emit(((hc & 0xFFFF) << size) | extra, size + (hc >> 16));
            }
            if last != 63 {
                let e = self.hcode[t + 1][0]; // EOB
                bw.emit(e & 0xFFFF, e >> 16);
            }
            bi += 1;
            if bi == self.bpm as usize {
                bi = 0;
            }
        }
        // pad with 1 bits to a byte boundary, then write the remaining bytes
        let pad = bw.n.wrapping_neg() & 7;
        if pad != 0 {
            bw.emit((1 << pad) - 1, pad);
        }
        while bw.n != 0 {
            bw.n -= 8;
            bw.put((bw.buf >> bw.n) as u8);
        }
        let end = start + bw.p;
        o.truncate(end);
    }
}

/// the bit writer (rbx / r12d / rdi of the assembly)
struct BitW<'a> {
    buf: u64,
    n: u32,
    o: &'a mut [u8], // sized for the worst case (every byte stuffed)
    p: usize,
}

impl BitW<'_> {
    /// EMIT: append n (1..32) bits
    #[inline(always)]
    fn emit(&mut self, v: u32, n: u32) {
        self.buf = (self.buf << n) | v as u64;
        self.n += n;
        if self.n >= 32 {
            self.flush32();
        }
    }

    /// je_flush32: write the oldest 32 bits with 0xFF byte stuffing
    #[inline(always)]
    fn flush32(&mut self) {
        self.n -= 32;
        let v = (self.buf >> self.n) as u32;
        let x = !v; // a zero byte here = an 0xFF byte
        if x.wrapping_sub(0x0101_0101) & !x & 0x8080_8080 == 0 {
            self.o[self.p..self.p + 4].copy_from_slice(&v.to_be_bytes());
            self.p += 4;
            return;
        }
        for b in v.to_be_bytes() {
            self.put(b);
        }
    }

    /// one byte with 0xFF stuffing
    #[inline(always)]
    fn put(&mut self, b: u8) {
        self.o[self.p] = b;
        self.p += 1;
        if b == 0xFF {
            self.o[self.p] = 0;
            self.p += 1;
        }
    }
}

/// magnitude category: bsr(|v|) + 1, 0 for 0
#[inline(always)]
fn nbits(v: i32) -> u32 {
    32 - v.unsigned_abs().leading_zeros()
}

/// NZMASK: bit k set when coefficient k of the block is non-zero (packsswb keeps non-zero
/// values non-zero)
#[inline(always)]
fn nzmask(b: &[i16; 64]) -> u64 {
    let x = simd::Sse2;
    let z = x.zero();
    let mut m = 0u64;
    for k in 0..4 {
        let v = x.packs16(x.load_i16(&b[16 * k..]), x.load_i16(&b[16 * k + 8..]));
        m |= (x.movemask8(x.cmpeq8(v, z)) as u64) << (16 * k);
    }
    !m
}

/// jpeg_encode
pub fn encode(sf: &Surface, quality: i32) -> Option<Vec<u8>> {
    encode_ex(sf, quality, None)
}

/// jpeg_encode_ex: jpeg_encode with an EncCfg (None = defaults)
pub fn encode_ex(sf: &Surface, quality: i32, cfg: Option<&EncCfg>) -> Option<Vec<u8>> {
    let (w, h) = (sf.w as u32, sf.h as u32);
    if w.wrapping_sub(1) > MAX_SIDE - 1 || h.wrapping_sub(1) > MAX_SIDE - 1 || sf.fmt > FMT_ARGB || sf.px.is_empty() {
        return None;
    }
    let mut e = alloc::boxed::Box::new(Enc {
        sf,
        div: [[0; 192]; 2],
        qzz: [[0; 64]; 2],
        freq: [[0; 257]; 4],
        hbits: [[0; 16]; 4],
        hvals: [[0; 256]; 4],
        hcode: [[0; 256]; 4],
        coef: Vec::new(),
        cur: 0,
        row: Vec::new(),
        ys: Vec::new(),
        cbs: Vec::new(),
        crs: Vec::new(),
        cbd: Vec::new(),
        crd: Vec::new(),
        bits: 0,
        w,
        h,
        fmt: sf.fmt,
        nc: 1,
        sub: 0,
        mcu: 8,
        mcux: 0,
        mcuy: 0,
        padw: 0,
        bpm: 1,
        bmap: [0; 8],
        pred: [0; 3],
        qual: quality.clamp(1, 100) as u32,
        nblk: 0,
        mcuh: 8,
        chr: cfg.map_or(0, |c| c.chroma),
    });
    if !e.setup() {
        return None;
    }
    e.pass1();
    e.huff();
    e.write(cfg)
}
