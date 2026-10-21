// enc -- HEIC writer (HEVC still images in HEIF)                     [src/img/heic_enc.s]
//
//   encode(sf, opt, out) -> 0 / ERR_*      the HEIC file of sf  [heic_encode]
//
// Colour: BT.709 full range (nclx 1 / 13 / 1 / full, also in the VUI: decoders that pick the
// matrix from the primaries, like Windows', agree), 4:2:0 (default; 2 x 2 chroma averaged) or
// 4:4:4 (opt.chroma 444; 422 is written as 4:2:0); IOF_LOSSLESS: 4:4:4 with the identity
// matrix (G, B, R planes, nclx matrix 0) and cu_transquant_bypass -- the RGB values come back
// exactly.  Quality: QP = round(48 - 0.36 * quality) (0 = 95 -> QP 14).
// Images up to 512 x 512 with codable sizes are one coded item; other sizes a grid (its
// output size crops exactly, no clean aperture): tiles of min(512, padded size) per
// direction (510 when 512 would leave a last column / row of one pixel, which Windows'
// decoder rejects), edges replicated, encoded on the worker pool (plat::parallel_for).
// Alpha (any pixel < 255): a 4:0:0 auxiliary image (auxC "urn:mpeg:hevc:2015:auxid:1", same
// layout and QP, straight alpha).  Metadata: ICC as colr prof (IOF_EMBED_ICC), EXIF as an
// Exif item with orientation 1 (IOF_EMBED_EXIF), XMP as a mime item (IOF_EMBED_XMP), both
// with cdsc references.  Container: ftyp heic (mif1, heic), meta (hdlr pict, pitm, iinf v0 /
// infe v2, iref, iprp with ipco / ipma v0, iloc v1 with 32-bit fields, idat for grid
// descriptors), mdat.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use gfx::{FMT_ARGB, Surface};
use img_core::exif::exif_copy_orient1;
use img_core::meta::get_row;
use img_core::{ImgOpt, ERR_DIMS, ERR_LIMIT, IMG_MAXDIM, IOF_EMBED_EXIF, IOF_EMBED_ICC, IOF_EMBED_XMP, IOF_LOSSLESS};
use img_hevc::{EncCfg, EncOut, HEF_LOSSLESS};
use rt::sync::SpinLock;

use crate::bmff::*;

const TILE: u32 = 512;
const TAG_MDAT: u32 = 0x7461_646D; // "mdat"
const TAG_HEIC_B: u32 = 0x6369_6568; // "heic"
const TAG_MIF1_B: u32 = 0x3166_696D; // "mif1"
const HW_MAXPATCH: usize = 8200;
const HW_MAXITEM: u32 = 8200;

// item kinds
const IK_COL: u32 = 1;
const IK_ALPHA: u32 = 2;
const IK_GRID: u32 = 3;
const IK_AGRID: u32 = 4;
const IK_EXIF: u32 = 5;
const IK_XMP: u32 = 6;

// property indices (ipco order)
const PR_HVCC: u32 = 1;
const PR_ISPE: u32 = 2; // coded item size
const PR_COLR: u32 = 3;
const PR_PIXI: u32 = 4;
const PR_ISPEI: u32 = 5; // image size (grid output)
const PR_HVCCA: u32 = 6;
const PR_AUXC: u32 = 7;
const PR_PIXIA: u32 = 8;
const PR_PROF: u32 = 9; // optional: ICC

static XMPMIME: &[u8; 20] = b"application/rdf+xml\0";
static AUXALPHA: &[u8; 27] = b"urn:mpeg:hevc:2015:auxid:1\0";

/// the writer state (HW)
struct Hw<'m> {
    w: u32, // image size
    h: u32,
    chroma: u32, // 1 (4:2:0) / 3 (4:4:4)
    ll: u32,     // lossless
    qp: u32,
    alpha: u32, // 1 = alpha plane written
    tw: u32,    // coded size of one item (tile or the padded image)
    th: u32,
    cols: u32,
    rows: u32,
    nt: u32,   // coded items per image (cols * rows)
    grid: u32, // 1 = grid
    pw: u32,   // padded luma plane size
    ph: u32,
    pl: [Vec<u16>; 4], // Y, Cb, Cr, alpha planes
    pst: [u32; 4],     // strides (samples)
    vui: u32,          // CICP of colr / VUI
    out: Vec<EncOut>,  // colour tiles, then alpha tiles
    exif: Vec<u8>,     // EXIF (orientation 1)
    icc: &'m [u8],
    xmp: &'m [u8],
    buf: Vec<u8>,
    nitem: u32,
    idexif: u32, // item ids (0 = none)
    idxmp: u32,
    idalpha: u32,           // alpha item (grid or coded)
    patch: Vec<usize>,      // iloc offset fields to patch
    kinds: Vec<u32>,        // kind per id (index = id)
}

/// heic_encode: the HEIC file of sf (see the file header)
pub fn encode<'m>(sf: &Surface, opt: &ImgOpt<'m>, out: &mut Vec<u8>) -> i32 {
    let (w, h) = (sf.w, sf.h);
    if w <= 0 || h <= 0 || w as u32 > IMG_MAXDIM || h as u32 > IMG_MAXDIM {
        return ERR_DIMS;
    }
    let mut hw = Hw {
        w: w as u32,
        h: h as u32,
        chroma: 1,
        ll: opt.flags & IOF_LOSSLESS,
        qp: 0,
        alpha: 0,
        tw: 0,
        th: 0,
        cols: 0,
        rows: 0,
        nt: 0,
        grid: 0,
        pw: 0,
        ph: 0,
        pl: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        pst: [0; 4],
        vui: 1 | 13 << 8 | 1 << 16 | 1 << 24,
        out: Vec::new(),
        exif: Vec::new(),
        icc: &[],
        xmp: &[],
        buf: Vec::new(),
        nitem: 0,
        idexif: 0,
        idxmp: 0,
        idalpha: 0,
        patch: Vec::new(),
        kinds: Vec::new(),
    };
    // options
    if opt.chroma == 444 {
        hw.chroma = 3;
    }
    if hw.ll != 0 {
        hw.chroma = 3;
        hw.vui = 1 | 13 << 8 | 1 << 24;
    }
    let q = if opt.quality == 0 { 95 } else { opt.quality };
    let q = (q as i32).max(1).min(100) as u32;
    hw.qp = (4850 - 36 * q) / 100; // QP = (4800 - 36 q + 50) / 100
    // layout: padded size (even for 4:2:0), tile size min(512, padded)
    let (mut tw, mut th) = (hw.w, hw.h);
    if hw.chroma == 1 {
        tw = (tw + 1) & !1;
        th = (th + 1) & !1;
    }
    tw = tw.min(TILE);
    th = th.min(TILE);
    // a last tile column / row of one pixel trips Windows' decoder (4:2:0): 510 then
    if hw.w & (TILE - 1) == 1 && hw.w > TILE {
        tw = TILE - 2;
    }
    if hw.h & (TILE - 1) == 1 && hw.h > TILE {
        th = TILE - 2;
    }
    hw.tw = tw;
    hw.th = th;
    hw.cols = (hw.w + tw - 1) / tw;
    hw.pw = hw.cols * tw;
    hw.rows = (hw.h + th - 1) / th;
    hw.ph = hw.rows * th;
    // a grid unless one tile covers the image exactly
    if hw.pw != hw.w || hw.ph != hw.h || hw.cols != 1 || hw.rows != 1 {
        hw.grid = 1;
    }
    hw.nt = hw.cols * hw.rows;
    if hw.nt > 4096 {
        return ERR_DIMS; // more than 64 x 64 tiles (32768 x 32768)
    }
    // metadata to embed
    if let Some(m) = opt.meta {
        if opt.flags & IOF_EMBED_ICC != 0 {
            hw.icc = &m.icc;
        }
        if opt.flags & IOF_EMBED_EXIF != 0 && !m.exif.is_empty() {
            hw.exif = exif_copy_orient1(&m.exif);
        }
        if opt.flags & IOF_EMBED_XMP != 0 && !m.xmp.is_empty() {
            hw.xmp = &m.xmp;
        }
    }
    // YUV (+ alpha) planes
    hw.yuv(sf);
    // encode the coded items (colour tiles 0 .. nt - 1, then alpha tiles)
    let nt2 = (hw.nt * 2) as usize;
    let n = if hw.alpha != 0 { nt2 } else { hw.nt as usize };
    let res: Vec<AtomicU32> = (0..nt2).map(|_| AtomicU32::new(0)).collect();
    let outs: Vec<SpinLock<EncOut>> = (0..nt2).map(|_| SpinLock::new(EncOut::default())).collect();
    {
        let me = &hw;
        let item = |i: usize, _: &plat::JobCtl| {
            let (r, o) = me.item(i as u32, opt);
            res[i].store(r as u32, Ordering::Relaxed);
            *outs[i].lock() = o;
        };
        let _ = plat::parallel_for(n, &item);
    }
    for r in res.iter().rev() {
        let r = r.load(Ordering::Relaxed) as i32;
        if r != 0 {
            return r;
        }
    }
    hw.out = outs.iter().map(|o| core::mem::take(&mut *o.lock())).collect();
    // container
    let r = hw.file();
    if r != 0 {
        return r;
    }
    *out = core::mem::take(&mut hw.buf);
    0
}

/// hw_cb: Cb (BT.709 full range, rounded)
fn cb(r: i32, g: i32, b: i32) -> u16 {
    let v = (r * -7509 + g * -25259 + (b << 15) + 8_421_376) >> 16;
    v.max(0).min(255) as u16
}

/// hw_cr: Cr
fn cr(r: i32, g: i32, b: i32) -> u16 {
    let v = ((r << 15) + g * -29763 + b * -3005 + 8_421_376) >> 16;
    v.max(0).min(255) as u16
}

// the byte writer (bb_*): big-endian fields, four-CCs as stored, boxes with patched sizes
fn put8(b: &mut Vec<u8>, v: u32) {
    b.push(v as u8);
}
fn put16(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&(v as u16).to_be_bytes());
}
fn put32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_be_bytes());
}
fn tag(b: &mut Vec<u8>, t: u32) {
    b.extend_from_slice(&t.to_le_bytes());
}
/// bb_begin: start of a box (size patched by end)
fn begin(b: &mut Vec<u8>, t: u32) -> usize {
    let s = b.len();
    put32(b, 0);
    tag(b, t);
    s
}
/// bb_end: write the box size
fn end(b: &mut [u8], s: usize) {
    let n = (b.len() - s) as u32;
    b[s..s + 4].copy_from_slice(&n.to_be_bytes());
}

impl Hw<'_> {
    /// hw_yuv: the padded Y / Cb / Cr (and alpha) planes of the image: BT.709 full range
    /// (4:2:0: chroma of the 2 x 2 pixels, clamped at the edges), lossless: G, B, R
    fn yuv(&mut self, sf: &Surface) {
        for c in 0..4 {
            let (mut w, mut h) = (self.pw, self.ph);
            if (1..3).contains(&c) && self.chroma == 1 {
                w >>= 1;
                h >>= 1;
            }
            self.pst[c] = w;
            self.pl[c] = vec![0u16; w as usize * h as usize];
        }
        let w = self.w as usize;
        let mut row = vec![0u8; w * 8]; // two BGRA rows
        // luma, alpha (and 4:4:4 chroma) per row
        let st0 = self.pst[0] as usize;
        for y in 0..self.h as usize {
            get_row(sf, y, &mut row, FMT_ARGB);
            for x in 0..w {
                let (b, g, r, a) = (row[x * 4] as i32, row[x * 4 + 1] as i32, row[x * 4 + 2] as i32, row[x * 4 + 3] as i32);
                let i = y * st0 + x; // luma index
                self.pl[3][i] = a as u16;
                if a != 255 {
                    self.alpha = 1;
                }
                if self.ll != 0 {
                    self.pl[0][i] = g as u16; // Y = G
                    self.pl[1][i] = b as u16; // U = B
                    self.pl[2][i] = r as u16; // V = R
                    continue;
                }
                self.pl[0][i] = ((r * 13933 + g * 46871 + b * 4732 + 32768) as u32 >> 16) as u16;
                if self.chroma == 3 {
                    self.pl[1][i] = cb(r, g, b);
                    self.pl[2][i] = cr(r, g, b);
                }
            }
        }
        // 4:2:0 chroma: 2 x 2 pixels (clamped to the image), averaged with rounding
        if self.chroma == 1 && self.ll == 0 {
            let st1 = self.pst[1] as usize;
            let (r0, r1) = row.split_at_mut(w * 4);
            let mut cy = 0usize;
            while cy * 2 < self.h as usize {
                get_row(sf, cy * 2, r0, FMT_ARGB);
                get_row(sf, (cy * 2 + 1).min(self.h as usize - 1), r1, FMT_ARGB);
                let mut cx = 0usize;
                while cx * 2 < w {
                    let (u, v) = cpair(r0, r1, cx * 2, w);
                    self.pl[1][cy * st1 + cx] = u;
                    self.pl[2][cy * st1 + cx] = v;
                    cx += 1;
                }
                cy += 1;
            }
        }
        // replicate the last column / row into the padding
        for c in 0..4 {
            let (mut vw, mut vh) = (self.w, self.h); // valid size of the plane
            let mut ph = self.ph;
            if (1..3).contains(&c) && self.chroma == 1 {
                vw = (vw + 1) >> 1;
                vh = (vh + 1) >> 1;
                ph >>= 1;
            }
            padplane(&mut self.pl[c], self.pst[c] as usize, vw as usize, vh as usize, ph as usize);
        }
    }

    /// hw_item: encode coded item index (colour tiles 0 .. nt - 1, then alpha tiles)
    fn item(&self, index: u32, opt: &ImgOpt) -> (i32, EncOut) {
        let (t, alpha) = if index < self.nt { (index, false) } else { (index - self.nt, true) };
        let x0 = (t % self.cols) * self.tw; // x0 of the tile
        let y0 = (t / self.cols) * self.th;
        let mut cfg = EncCfg {
            pl: [&[], &[], &[]],
            st: [0; 3],
            w: self.tw,
            h: self.th,
            chroma: 0,
            qp: self.qp,
            flags: self.ll * HEF_LOSSLESS,
            vui: 2 | 2 << 8 | 2 << 16 | 1 << 24,
            cancel: opt.cancel,
        };
        if !alpha {
            cfg.vui = self.vui;
            cfg.chroma = self.chroma;
            for c in 0..3 {
                let (mut x, mut y) = (x0, y0);
                if c != 0 && self.chroma == 1 {
                    x >>= 1;
                    y >>= 1;
                }
                cfg.st[c] = self.pst[c];
                cfg.pl[c] = &self.pl[c][(self.pst[c] * y + x) as usize..];
            }
        } else {
            cfg.st[0] = self.pst[3];
            cfg.pl[0] = &self.pl[3][(self.pst[3] * y0 + x0) as usize..];
        }
        match img_hevc::encode(&cfg) {
            Ok(o) => (0, o),
            Err(e) => (e, EncOut::default()),
        }
    }

    // ================================================================== container

    /// hw_file -> 0 / ERR_LIMIT: the HEIF file into buf
    fn file(&mut self) -> i32 {
        // item table: ids 1..
        let mut k = vec![0u32]; // kinds by id
        k.push(if self.grid != 0 { IK_GRID } else { IK_COL });
        if self.grid != 0 {
            for _ in 0..self.nt {
                k.push(IK_COL);
            }
        }
        if self.alpha != 0 {
            self.idalpha = k.len() as u32;
            k.push(if self.grid != 0 { IK_AGRID } else { IK_ALPHA });
            if self.grid != 0 {
                for _ in 0..self.nt {
                    k.push(IK_ALPHA);
                }
            }
        }
        if !self.exif.is_empty() {
            self.idexif = k.len() as u32;
            k.push(IK_EXIF);
        }
        if !self.xmp.is_empty() {
            self.idxmp = k.len() as u32;
            k.push(IK_XMP);
        }
        self.nitem = k.len() as u32 - 1;
        self.kinds = k;
        if self.nitem > HW_MAXITEM {
            return ERR_LIMIT;
        }
        let mut b = core::mem::take(&mut self.buf);
        // ---- ftyp
        put32(&mut b, 24);
        tag(&mut b, TAG_FTYP);
        tag(&mut b, TAG_HEIC_B);
        put32(&mut b, 0);
        tag(&mut b, TAG_MIF1_B);
        tag(&mut b, TAG_HEIC_B);
        // ---- meta
        let meta = begin(&mut b, TAG_META);
        put32(&mut b, 0);
        // hdlr
        put32(&mut b, 33);
        tag(&mut b, TAG_HDLR);
        put32(&mut b, 0);
        put32(&mut b, 0);
        tag(&mut b, TAG_PICT);
        put32(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, 0);
        put8(&mut b, 0);
        // pitm
        put32(&mut b, 14);
        tag(&mut b, TAG_PITM);
        put32(&mut b, 0);
        put16(&mut b, 1);
        // iinf
        let bs = begin(&mut b, TAG_IINF);
        put32(&mut b, 0);
        put16(&mut b, self.nitem);
        for id in 1..=self.nitem {
            let kind = self.kinds[id as usize];
            let ty = match kind {
                IK_COL | IK_ALPHA => TAG_HVC1,
                IK_GRID | IK_AGRID => TAG_GRID,
                IK_EXIF => TAG_EXIF,
                _ => TAG_MIME,
            };
            let s = begin(&mut b, TAG_INFE);
            let mut vf = 0x0200_0000u32; // version 2, flags 0
            if self.grid != 0 && ty == TAG_HVC1 && id != 1 && id != self.idalpha {
                vf |= 1; // tiles: hidden
            }
            put32(&mut b, vf);
            put16(&mut b, id);
            put16(&mut b, 0);
            tag(&mut b, ty);
            put8(&mut b, 0); // item_name ""
            if ty == TAG_MIME {
                b.extend_from_slice(XMPMIME);
            }
            end(&mut b, s);
        }
        end(&mut b, bs);
        // iref
        if self.grid | self.alpha | self.idexif | self.idxmp != 0 {
            let bs = begin(&mut b, TAG_IREF);
            put32(&mut b, 0);
            if self.grid != 0 {
                let s = begin(&mut b, TAG_DIMG); // grid -> tiles
                put16(&mut b, 1);
                put16(&mut b, self.nt);
                for i in 0..self.nt {
                    put16(&mut b, i + 2);
                }
                end(&mut b, s);
                if self.alpha != 0 {
                    let s = begin(&mut b, TAG_DIMG); // alpha grid -> alpha tiles
                    put16(&mut b, self.idalpha);
                    put16(&mut b, self.nt);
                    for i in 0..self.nt {
                        put16(&mut b, self.idalpha + i + 1);
                    }
                    end(&mut b, s);
                }
            }
            if self.alpha != 0 {
                put32(&mut b, 14);
                tag(&mut b, TAG_AUXL);
                put16(&mut b, self.idalpha);
                put16(&mut b, 1);
                put16(&mut b, 1);
            }
            if self.idexif != 0 {
                put32(&mut b, 14);
                tag(&mut b, TAG_CDSC);
                put16(&mut b, self.idexif);
                put16(&mut b, 1);
                put16(&mut b, 1);
            }
            if self.idxmp != 0 {
                put32(&mut b, 14);
                tag(&mut b, TAG_CDSC);
                put16(&mut b, self.idxmp);
                put16(&mut b, 1);
                put16(&mut b, 1);
            }
            end(&mut b, bs);
        }
        // iprp / ipco
        let iprp = begin(&mut b, TAG_IPRP);
        let ipco = begin(&mut b, TAG_IPCO);
        self.hvcc(&mut b, 0); // colour hvcC (first colour item)
        put32(&mut b, 20); // ispe of a coded item
        tag(&mut b, TAG_ISPE);
        put32(&mut b, 0);
        put32(&mut b, self.tw);
        put32(&mut b, self.th);
        put32(&mut b, 19); // colr nclx
        tag(&mut b, TAG_COLR);
        tag(&mut b, TAG_NCLX);
        put16(&mut b, self.vui & 255);
        put16(&mut b, (self.vui >> 8) & 255);
        put16(&mut b, (self.vui >> 16) & 255);
        put8(&mut b, 0x80);
        put32(&mut b, 16); // pixi 3 x 8 bits
        tag(&mut b, TAG_PIXI);
        put32(&mut b, 0);
        put8(&mut b, 3);
        put8(&mut b, 8);
        put8(&mut b, 8);
        put8(&mut b, 8);
        put32(&mut b, 20); // ispe of the image
        tag(&mut b, TAG_ISPE);
        put32(&mut b, 0);
        put32(&mut b, self.w);
        put32(&mut b, self.h);
        // alpha hvcC (first alpha item; without alpha: the colour one again, unused)
        let ai = if self.alpha != 0 { self.nt as usize } else { 0 };
        self.hvcc(&mut b, ai);
        let s = begin(&mut b, TAG_AUXC); // auxC alpha
        put32(&mut b, 0);
        b.extend_from_slice(AUXALPHA);
        end(&mut b, s);
        put32(&mut b, 14); // pixi 1 x 8 bits
        tag(&mut b, TAG_PIXI);
        put32(&mut b, 0);
        put8(&mut b, 1);
        put8(&mut b, 8);
        let icc = !self.icc.is_empty();
        if icc {
            let s = begin(&mut b, TAG_COLR); // colr prof
            tag(&mut b, TAG_PROF);
            b.extend_from_slice(self.icc);
            end(&mut b, s);
        }
        end(&mut b, ipco);
        // ipma v0, 7-bit indices
        let s = begin(&mut b, TAG_IPMA);
        put32(&mut b, 0);
        let mut ne = self.nitem - 2; // entries: every image item
        if self.idexif == 0 {
            ne += 1;
        }
        if self.idxmp == 0 {
            ne += 1;
        }
        put32(&mut b, ne);
        for id in 1..=self.nitem {
            let kind = self.kinds[id as usize];
            if kind >= IK_EXIF {
                continue;
            }
            put16(&mut b, id);
            match kind {
                IK_GRID => {
                    put8(&mut b, 3 + icc as u32);
                    put8(&mut b, PR_ISPEI);
                    put8(&mut b, PR_COLR);
                    if icc {
                        put8(&mut b, PR_PROF);
                    }
                    put8(&mut b, PR_PIXI);
                }
                IK_AGRID => {
                    put8(&mut b, 3);
                    put8(&mut b, PR_ISPEI);
                    put8(&mut b, 0x80 | PR_AUXC);
                    put8(&mut b, PR_PIXIA);
                }
                _ if self.grid != 0 => {
                    // a tile
                    put8(&mut b, 2);
                    put8(&mut b, 0x80 | if kind == IK_ALPHA { PR_HVCCA } else { PR_HVCC });
                    put8(&mut b, PR_ISPE);
                }
                IK_ALPHA => {
                    put8(&mut b, 4);
                    put8(&mut b, 0x80 | PR_HVCCA);
                    put8(&mut b, PR_ISPE);
                    put8(&mut b, 0x80 | PR_AUXC);
                    put8(&mut b, PR_PIXIA);
                }
                _ => {
                    // single colour item
                    put8(&mut b, 4 + icc as u32);
                    put8(&mut b, 0x80 | PR_HVCC);
                    put8(&mut b, PR_ISPE);
                    put8(&mut b, PR_COLR);
                    if icc {
                        put8(&mut b, PR_PROF);
                    }
                    put8(&mut b, PR_PIXI);
                }
            }
        }
        end(&mut b, s);
        end(&mut b, iprp);
        // iloc v1: offset 4, length 4, no base / index; one extent per item
        let s = begin(&mut b, TAG_ILOC);
        put32(&mut b, 0x0100_0000);
        put8(&mut b, 0x44);
        put8(&mut b, 0);
        put16(&mut b, self.nitem);
        let mut idat = 0u32; // idat bytes so far
        for id in 1..=self.nitem {
            put16(&mut b, id);
            let kind = self.kinds[id as usize];
            if kind == IK_GRID || kind == IK_AGRID {
                put16(&mut b, 1); // construction_method 1: idat
                put16(&mut b, 0);
                put16(&mut b, 1);
                put32(&mut b, idat);
                put32(&mut b, 8);
                idat += 8;
            } else {
                put16(&mut b, 0); // file offsets
                put16(&mut b, 0);
                put16(&mut b, 1);
                // offset field: patched after the mdat is written
                if self.patch.len() >= HW_MAXPATCH {
                    self.buf = b;
                    return ERR_LIMIT;
                }
                self.patch.push(b.len());
                put32(&mut b, 0);
                let l = self.itemdata(id);
                put32(&mut b, l);
            }
        }
        end(&mut b, s);
        // idat: grid descriptors (v0, 16-bit output size)
        if self.grid != 0 {
            let s = begin(&mut b, TAG_IDAT);
            for id in 1..=self.nitem {
                let kind = self.kinds[id as usize];
                if kind == IK_GRID || kind == IK_AGRID {
                    put8(&mut b, 0);
                    put8(&mut b, 0);
                    put8(&mut b, self.rows - 1);
                    put8(&mut b, self.cols - 1);
                    put16(&mut b, self.w);
                    put16(&mut b, self.h);
                }
            }
            end(&mut b, s);
        }
        end(&mut b, meta);
        // ---- mdat: the item data in id order
        let s = begin(&mut b, TAG_MDAT);
        let mut pi = 0usize; // patch index
        for id in 1..=self.nitem {
            let kind = self.kinds[id as usize];
            if kind == IK_GRID || kind == IK_AGRID || pi >= self.patch.len() {
                continue;
            }
            // patch the iloc offset of this item
            let f = self.patch[pi];
            let o = b.len() as u32;
            b[f..f + 4].copy_from_slice(&o.to_be_bytes());
            pi += 1;
            self.putdata(&mut b, id);
        }
        end(&mut b, s);
        self.buf = b;
        0
    }

    /// hw_coded: the encoder output of coded item id (colour / alpha tile or single item)
    fn coded(&self, id: u32) -> Option<&EncOut> {
        let mut i = id.wrapping_sub(1); // single: colour 0, alpha 1
        if self.grid != 0 {
            i = i.wrapping_sub(1); // grid: tiles from id 2
            if i >= self.nt {
                i = i.wrapping_sub(1); // alpha grid id nt + 2, alpha tiles after it
            }
        } else if i == 1 {
            i = self.nt; // alpha = out[nt]
        }
        if i >= self.nt * 2 {
            return None;
        }
        self.out.get(i as usize)
    }

    /// hw_itemdata: length of the mdat data of item id
    fn itemdata(&self, id: u32) -> u32 {
        if id == self.idexif {
            return self.exif.len() as u32 + 4;
        }
        if id == self.idxmp {
            return self.xmp.len() as u32;
        }
        match self.coded(id) {
            Some(o) => o.nal[3].len() as u32 + 4, // slice NAL, 4-byte length prefix
            None => 0,
        }
    }

    /// hw_putdata: append the mdat data of item id
    fn putdata(&self, b: &mut Vec<u8>, id: u32) {
        if id == self.idexif {
            put32(b, 0); // exif_tiff_header_offset
            b.extend_from_slice(&self.exif);
            return;
        }
        if id == self.idxmp {
            b.extend_from_slice(self.xmp);
            return;
        }
        if let Some(o) = self.coded(id) {
            put32(b, o.nal[3].len() as u32);
            b.extend_from_slice(&o.nal[3]);
        }
    }

    /// hw_hvcc: hvcC property of out[index] (PTL, chroma format, 8 bits, 4-byte NAL lengths,
    /// VPS / SPS / PPS arrays)
    fn hvcc(&self, b: &mut Vec<u8>, index: usize) {
        let o = &self.out[index];
        let s = begin(b, TAG_HVCC);
        put8(b, 1);
        b.extend_from_slice(&o.ptl);
        put16(b, 0xF000);
        put8(b, 0xFC);
        // chroma_format_idc from the profile: Main 4:2:0, RExt 4:4:4 (0x9E) / 4:0:0 (0x9F)
        let cf = if o.ptl[0] == 1 {
            0xFD
        } else if o.ptl[5] == 0x9E {
            0xFF
        } else {
            0xFC
        };
        put8(b, cf);
        put8(b, 0xF8);
        put8(b, 0xF8);
        put16(b, 0);
        put8(b, 0x0F);
        put8(b, 3);
        for a in 0..3u32 {
            put8(b, (a + 32) | 0x80);
            put16(b, 1);
            put16(b, o.nal[a as usize].len() as u32);
            b.extend_from_slice(&o.nal[a as usize]);
        }
        end(b, s);
    }
}

/// hw_cpair: Cb, Cr of the pixels x, x + 1 (clamped) of the two rows
fn cpair(r0: &[u8], r1: &[u8], x: usize, w: usize) -> (u16, u16) {
    let x1 = (x + 1).min(w - 1);
    let mut su = 0i32; // sum Cb (unshifted)
    let mut sv = 0i32; // sum Cr
    for k in 0..4 {
        let px = if k & 1 != 0 { x1 } else { x };
        let row = if k & 2 != 0 { r1 } else { r0 };
        let (b, g, r) = (row[px * 4] as i32, row[px * 4 + 1] as i32, row[px * 4 + 2] as i32);
        su = su.wrapping_add(r * -7509 + g * -25259 + (b << 15));
        sv = sv.wrapping_add((r << 15) + g * -29763 + b * -3005);
    }
    // 4 * (128 << 16) + 2 << 16
    let u = (su.wrapping_add(33_685_504) >> 18).max(0).min(255) as u16;
    let v = (sv.wrapping_add(33_685_504) >> 18).max(0).min(255) as u16;
    (u, v)
}

/// hw_padplane: replicate column w - 1 and row h - 1 of the plane to its size (stride =
/// padded width, ph rows)
fn padplane(p: &mut [u16], st: usize, w: usize, h: usize, ph: usize) {
    for r in 0..h {
        let o = r * st;
        let v = p[o + w - 1];
        p[o + w..o + st].fill(v);
    }
    let last = (h - 1) * st;
    for r in h..ph {
        p.copy_within(last..last + st, r * st);
    }
}
