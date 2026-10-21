// enc -- AVIF writer (AV1 still image in HEIF: one coded colour item, optional alpha)
//                                                    [src/img/avif_enc.s]
//
//   encode(sf, opt, out) -> 0 / ERR_*   avif_encode (the encode entry of CODEC)
//
// Colour: BT.709 full range (nclx 1 / 13 / 1 / full, also in the sequence header), 4:2:0
// (default; 2 x 2 chroma averaged), 4:4:4 (opt.chroma 444) or 4:0:0 (opt.chroma 400: luma only;
// 422 is written as 4:2:0); IOF_LOSSLESS: 4:4:4 with the identity matrix (G, B, R planes, nclx
// matrix 0) and base_q_idx 0 -- the RGB values come back exactly.  Quality q (opt.quality, 0 =
// 85): quantizer ((100 - q) * 63 + 50) / 100 (libavif's 0..63 scale), base_q_idx 4 * it
// (62 -> 249, 63 -> 255, at least 1).  The AV1 frame has the image's size (no padding, no grid);
// the encoder (img_av1::encode) works on the worker pool.  Alpha (any pixel < 255): a 4:0:0
// auxiliary item (auxC "urn:mpeg:mpegB:cicp:systems:auxiliary:alpha", same quantizer, straight
// alpha).  Metadata: ICC as colr prof (IOF_EMBED_ICC), EXIF as an Exif item with orientation 1
// (IOF_EMBED_EXIF), XMP as a mime item (IOF_EMBED_XMP), both with cdsc references.
// Container: ftyp avif (avif, mif1, miaf), meta (hdlr pict, pitm, iinf v0 / infe v2, iref, iprp
// with ipco / ipma v0, iloc v0 with 32-bit fields), mdat.  The item data are the OBUs (sequence
// header and frame OBU); av1C carries the sequence header OBU as configOBUs.

use alloc::vec::Vec;

use gfx::{Surface, FMT_ARGB};
use img_av1::{EncIn, EncOut};
use img_core::exif::exif_copy_orient1;
use img_core::meta::get_row;
use img_core::{ImgOpt, ERR_DIMS, IMG_MAXDIM, IOF_EMBED_EXIF, IOF_EMBED_ICC, IOF_EMBED_XMP, IOF_LOSSLESS};

// the auxC type string; the assembly copies 45 bytes (one beyond its NUL: a zero padding byte of
// the reference builds' .rodata), the output keeps that byte
const AUX_ALPHA: &[u8; 45] = b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha\0\0";
const XMP_MIME: &[u8; 20] = b"application/rdf+xml\0";

// property indices (ipco order)
const PR_AV1C: u8 = 1;
const PR_ISPE: u8 = 2;
const PR_COLR: u8 = 3;
const PR_PIXI: u8 = 4;
const PR_AV1CA: u8 = 5;
const PR_AUXC: u8 = 6;
const PR_PIXIA: u8 = 7;
const PR_PROF: u8 = 8; // optional: ICC

// the writer state (AW)
struct Aw<'a> {
    sf: &'a Surface,
    opt: &'a ImgOpt<'a>,
    w: u32,
    h: u32,
    chroma: u32, // 0 (4:0:0) / 1 (4:2:0) / 3 (4:4:4)
    ll: bool,    // lossless
    qidx: u32,   // base_q_idx
    alpha: bool, // alpha item written
    cicp: u32,   // CICP of the colour item
    cw: u32,     // chroma plane size
    ch: u32,
    pl: [Vec<u16>; 4], // Y, Cb, Cr, alpha planes
    pst: [u32; 4],     // strides (samples)
    exif: Vec<u8>,     // EXIF (orientation 1) / empty
    icc: &'a [u8],     // ICC profile (from the metadata) / empty
    xmp: &'a [u8],
    out: [EncOut; 2], // colour, alpha
    b: Vec<u8>,       // output
    nitem: u32,
    idalpha: u32, // item ids (0 = none)
    idexif: u32,
    idxmp: u32,
}

/// avif_encode: see the file header
pub fn encode(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    out.clear();
    let (w, h) = (sf.w, sf.h);
    if w <= 0 || h <= 0 || w as u32 > IMG_MAXDIM || h as u32 > IMG_MAXDIM {
        return ERR_DIMS;
    }
    let mut a = Aw {
        sf,
        opt,
        w: w as u32,
        h: h as u32,
        chroma: 1,
        ll: false,
        qidx: 0,
        alpha: false,
        cicp: 1 | (13 << 8) | (1 << 16) | (1 << 24),
        cw: 0,
        ch: 0,
        pl: [Vec::new(), Vec::new(), Vec::new(), Vec::new()],
        pst: [0; 4],
        exif: Vec::new(),
        icc: &[],
        xmp: &[],
        out: [EncOut::default(), EncOut::default()],
        b: Vec::new(),
        nitem: 0,
        idalpha: 0,
        idexif: 0,
        idxmp: 0,
    };
    // options
    if opt.chroma == 444 {
        a.chroma = 3;
    }
    if opt.chroma == 400 {
        a.chroma = 0;
    }
    a.ll = opt.flags & IOF_LOSSLESS != 0;
    if a.ll {
        a.chroma = 3;
        a.cicp = 1 | (13 << 8) | (1 << 24);
        a.qidx = 0;
    } else {
        let mut q = opt.quality as i32;
        if q == 0 {
            q = 85;
        }
        let q = q.clamp(1, 100) as u32;
        let quant = ((100 - q) * 63 + 50) / 100; // 0..62 (q >= 1)
        a.qidx = if quant == 62 { 249 } else { (quant * 4).max(1) };
    }
    (a.cw, a.ch) = if a.chroma == 1 { (a.w.div_ceil(2), a.h.div_ceil(2)) } else { (a.w, a.h) };
    a.meta();
    a.yuv();
    // AV1 items: colour, then alpha
    if let Err(e) = a.item(0) {
        return e;
    }
    if a.alpha {
        if let Err(e) = a.item(1) {
            return e;
        }
    }
    a.file();
    *out = a.b;
    0
}

// aw_cb: Cb of R, G, B (BT.709 full range, rounded)
#[inline]
fn aw_cb(r: i32, g: i32, b: i32) -> u16 {
    ((r * -7509 + g * -25259 + (b << 15) + 8421376) >> 16).clamp(0, 255) as u16
}

// aw_cr: Cr
#[inline]
fn aw_cr(r: i32, g: i32, b: i32) -> u16 {
    (((r << 15) + g * -29763 + b * -3005 + 8421376) >> 16).clamp(0, 255) as u16
}

impl<'a> Aw<'a> {
    // aw_meta: the ICC / EXIF (orientation 1) / XMP blobs to embed
    fn meta(&mut self) {
        let Some(m) = self.opt.meta else { return };
        let fl = self.opt.flags;
        if fl & IOF_EMBED_ICC != 0 {
            self.icc = &m.icc;
        }
        if fl & IOF_EMBED_EXIF != 0 && !m.exif.is_empty() {
            self.exif = exif_copy_orient1(&m.exif);
        }
        if fl & IOF_EMBED_XMP != 0 {
            self.xmp = &m.xmp;
        }
    }

    // aw_yuv: Y / Cb / Cr (and alpha) planes of the image size: BT.709 full range (4:2:0: chroma
    // of the 2 x 2 pixels, clamped at the edges), lossless: G, B, R
    fn yuv(&mut self) {
        let (w, h) = (self.w as usize, self.h as usize);
        for k in 0..4 {
            let (pw, ph) = if k == 1 || k == 2 { (self.cw, self.ch) } else { (self.w, self.h) };
            self.pst[k] = pw;
            self.pl[k] = alloc::vec![0u16; pw as usize * ph as usize];
        }
        let mut row = alloc::vec![0u8; w * 8]; // two BGRA rows
        // luma, alpha (and 4:4:4 chroma) per row
        for y in 0..h {
            get_row(self.sf, y, &mut row[..w * 4], FMT_ARGB);
            let base = y * w;
            for x in 0..w {
                let px = &row[x * 4..x * 4 + 4];
                let (b, g, r, al) = (px[0] as i32, px[1] as i32, px[2] as i32, px[3]);
                let i = base + x;
                self.pl[3][i] = al as u16;
                if al != 255 {
                    self.alpha = true;
                }
                if self.ll {
                    self.pl[0][i] = g as u16; // Y = G
                    self.pl[1][i] = b as u16; // U = B
                    self.pl[2][i] = r as u16; // V = R
                    continue;
                }
                self.pl[0][i] = ((13933 * r + 46871 * g + 4732 * b + 32768) as u32 >> 16) as u16;
                if self.chroma == 3 {
                    self.pl[1][i] = aw_cb(r, g, b);
                    self.pl[2][i] = aw_cr(r, g, b);
                }
            }
        }
        // 4:2:0 chroma: 2 x 2 pixels (clamped to the image), averaged with rounding
        if self.chroma != 1 {
            return;
        }
        let (cw, ch) = (self.cw as usize, self.ch as usize);
        for cy in 0..ch {
            let (r0, r1) = row.split_at_mut(w * 4);
            get_row(self.sf, 2 * cy, r0, FMT_ARGB);
            get_row(self.sf, (2 * cy + 1).min(h - 1), r1, FMT_ARGB);
            for cx in 0..cw {
                let (cb, cr) = cpair(r0, r1, w, 2 * cx);
                self.pl[1][cy * cw + cx] = cb;
                self.pl[2][cy * cw + cx] = cr;
            }
        }
    }

    // aw_item: encode the colour (k 0) or alpha (k 1) AV1 item into out[k]
    fn item(&mut self, k: usize) -> Result<(), i32> {
        let cfg = if k == 0 {
            EncIn {
                w: self.w,
                h: self.h,
                bd: 8,
                chroma: self.chroma,
                qidx: self.qidx,
                cicp: self.cicp,
                tile_sb: 0,
                flags: 0,
                cancel: self.opt.cancel,
                planes: [&self.pl[0], &self.pl[1], &self.pl[2]],
                strides: [self.pst[0], self.pst[1], self.pst[2]],
            }
        } else {
            EncIn {
                w: self.w,
                h: self.h,
                bd: 8,
                chroma: 0,
                qidx: self.qidx,
                cicp: 2 | (2 << 8) | (2 << 16) | (1 << 24),
                tile_sb: 0,
                flags: 0,
                cancel: self.opt.cancel,
                planes: [&self.pl[3], &[], &[]],
                strides: [self.pst[3], 0, 0],
            }
        };
        self.out[k] = img_av1::encode(&cfg)?;
        Ok(())
    }

    // ---- container

    fn u8(&mut self, v: u32) {
        self.b.push(v as u8);
    }
    fn u16(&mut self, v: u32) {
        self.b.extend_from_slice(&(v as u16).to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.b.extend_from_slice(&v.to_be_bytes());
    }
    fn tag(&mut self, t: &[u8; 4]) {
        self.b.extend_from_slice(t);
    }
    // ab_begin: start of a box (size patched by end)
    fn begin(&mut self, t: &[u8; 4]) -> usize {
        let s = self.b.len();
        self.u32(0);
        self.tag(t);
        s
    }
    // ab_end: write the box size
    fn end(&mut self, s: usize) {
        let n = (self.b.len() - s) as u32;
        self.b[s..s + 4].copy_from_slice(&n.to_be_bytes());
    }

    // aw_file: the AVIF file into b
    fn file(&mut self) {
        // item ids: 1 colour, then alpha, Exif, XMP
        let mut id = 1;
        if self.alpha {
            id += 1;
            self.idalpha = id;
        }
        if !self.exif.is_empty() {
            id += 1;
            self.idexif = id;
        }
        if !self.xmp.is_empty() {
            id += 1;
            self.idxmp = id;
        }
        self.nitem = id;
        self.b.reserve(self.icc.len() + 8192);
        // ---- ftyp
        self.u32(28);
        self.tag(b"ftyp");
        self.tag(b"avif");
        self.u32(0);
        self.tag(b"avif");
        self.tag(b"mif1");
        self.tag(b"miaf");
        // ---- meta
        let meta = self.begin(b"meta");
        self.u32(0);
        // hdlr
        self.u32(33);
        self.tag(b"hdlr");
        self.u32(0);
        self.u32(0);
        self.tag(b"pict");
        self.u32(0);
        self.u32(0);
        self.u32(0);
        self.u8(0);
        // pitm
        self.u32(14);
        self.tag(b"pitm");
        self.u32(0);
        self.u16(1);
        // iinf
        let iinf = self.begin(b"iinf");
        self.u32(0);
        self.u16(self.nitem);
        for i in 1..=self.nitem {
            let t = if i == self.idxmp {
                b"mime"
            } else if i == self.idexif {
                b"Exif"
            } else {
                b"av01"
            };
            let infe = self.begin(b"infe");
            self.u32(0x02000000); // version 2, flags 0
            self.u16(i);
            self.u16(0);
            self.tag(t);
            self.u8(0); // item_name ""
            if t == b"mime" {
                self.b.extend_from_slice(XMP_MIME);
            }
            self.end(infe);
        }
        self.end(iinf);
        // iref: auxl (alpha -> colour), cdsc (Exif / XMP -> colour)
        if self.idalpha | self.idexif | self.idxmp != 0 {
            let iref = self.begin(b"iref");
            self.u32(0);
            for (rid, t) in [(self.idalpha, b"auxl"), (self.idexif, b"cdsc"), (self.idxmp, b"cdsc")] {
                if rid != 0 {
                    self.u32(14);
                    self.tag(t);
                    self.u16(rid);
                    self.u16(1);
                    self.u16(1);
                }
            }
            self.end(iref);
        }
        // iprp / ipco
        let iprp = self.begin(b"iprp");
        let ipco = self.begin(b"ipco");
        self.av1c(0); // colour av1C
        self.u32(20); // ispe
        self.tag(b"ispe");
        self.u32(0);
        self.u32(self.w);
        self.u32(self.h);
        self.u32(19); // colr nclx
        self.tag(b"colr");
        self.tag(b"nclx");
        self.u16(self.cicp & 0xFF);
        self.u16((self.cicp >> 8) & 0xFF);
        self.u16((self.cicp >> 16) & 0xFF);
        self.u8(0x80);
        if self.chroma != 0 {
            self.u32(16); // pixi 3 x 8 bits
            self.tag(b"pixi");
            self.u32(0);
            self.u8(3);
            self.u8(8);
            self.u8(8);
            self.u8(8);
        } else {
            self.u32(14); // pixi 1 x 8 bits (4:0:0)
            self.tag(b"pixi");
            self.u32(0);
            self.u8(1);
            self.u8(8);
        }
        self.av1c(self.alpha as usize); // alpha av1C (the colour one when no alpha)
        let auxc = self.begin(b"auxC"); // auxC alpha
        self.u32(0);
        self.b.extend_from_slice(AUX_ALPHA);
        self.end(auxc);
        self.u32(14); // pixi 1 x 8 bits
        self.tag(b"pixi");
        self.u32(0);
        self.u8(1);
        self.u8(8);
        if !self.icc.is_empty() {
            let prof = self.begin(b"colr"); // colr prof
            self.tag(b"prof");
            self.b.extend_from_slice(self.icc);
            self.end(prof);
        }
        self.end(ipco);
        // ipma v0, 7-bit indices: the image items
        let ipma = self.begin(b"ipma");
        self.u32(0);
        self.u32(1 + self.alpha as u32);
        self.u16(1); // colour
        self.u8(4 + !self.icc.is_empty() as u32);
        self.u8(0x80 | PR_AV1C as u32);
        self.u8(PR_ISPE as u32);
        self.u8(PR_COLR as u32);
        if !self.icc.is_empty() {
            self.u8(PR_PROF as u32);
        }
        self.u8(PR_PIXI as u32);
        if self.alpha {
            self.u16(self.idalpha);
            self.u8(4);
            self.u8(0x80 | PR_AV1CA as u32);
            self.u8(PR_ISPE as u32);
            self.u8(0x80 | PR_AUXC as u32);
            self.u8(PR_PIXIA as u32);
        }
        self.end(ipma);
        self.end(iprp);
        // iloc v0: offset 4, length 4, no base offset; one extent per item
        let iloc = self.begin(b"iloc");
        self.u32(0);
        self.u8(0x44);
        self.u8(0);
        self.u16(self.nitem);
        let mut patch = [0usize; 4];
        for i in 1..=self.nitem {
            self.u16(i);
            self.u16(0); // data_reference_index
            self.u16(1); // extent_count
            patch[i as usize - 1] = self.b.len();
            self.u32(0); // extent_offset (patched)
            let n = self.itemlen(i);
            self.u32(n);
        }
        self.end(iloc);
        self.end(meta);
        // ---- mdat: the item data in id order
        let mdat = self.begin(b"mdat");
        for i in 1..=self.nitem {
            let p = patch[i as usize - 1];
            let off = self.b.len() as u32;
            self.b[p..p + 4].copy_from_slice(&off.to_be_bytes());
            self.putdata(i);
        }
        self.end(mdat);
    }

    // aw_itemdata: the length of the mdat data of item id (Exif: with the 4-byte offset header)
    fn itemlen(&self, id: u32) -> u32 {
        if id == self.idexif {
            (self.exif.len() as u32).wrapping_add(4)
        } else if id == self.idxmp {
            self.xmp.len() as u32
        } else {
            self.out[(id == self.idalpha) as usize].data.len() as u32
        }
    }

    // aw_putdata: append the mdat data of item id
    fn putdata(&mut self, id: u32) {
        if id == self.idexif {
            self.u32(0); // exif_tiff_header_offset
            self.b.extend_from_slice(&self.exif);
        } else if id == self.idxmp {
            self.b.extend_from_slice(self.xmp);
        } else {
            let k = (id == self.idalpha) as usize;
            self.b.extend_from_slice(&self.out[k].data);
        }
    }

    // aw_av1c: av1C property of out[k] (k 1: the alpha item): marker / version 1, seq_profile,
    // seq_level_idx_0, tier 0, high_bitdepth 0, monochrome, chroma subsampling,
    // chroma_sample_position 0, configOBUs = the sequence header OBU
    fn av1c(&mut self, k: usize) {
        let ch = if k != 0 { 0 } else { self.chroma }; // alpha: 4:0:0
        let s = self.begin(b"av1C");
        self.u8(0x81);
        let o = &self.out[k];
        let pl = o.profile << 5 | o.level;
        self.u8(pl);
        self.u8(match ch {
            0 => 0x1C, // monochrome, subsampling 1 / 1
            1 => 0x0C, // 4:2:0
            _ => 0,    // 4:4:4
        });
        self.u8(0);
        let n = self.out[k].seqlen as usize;
        self.b.extend_from_slice(&self.out[k].data[..n]);
        self.end(s);
    }
}

// aw_cpair: Cb, Cr of the pixels x, x + 1 (clamped) of the two rows r0, r1
fn cpair(r0: &[u8], r1: &[u8], w: usize, x: usize) -> (u16, u16) {
    let x1 = (x + 1).min(w - 1);
    let mut scb = 0i32; // sums (unshifted)
    let mut scr = 0i32;
    for k in 0..4 {
        let xx = if k & 1 != 0 { x1 } else { x };
        let r = if k & 2 != 0 { r1 } else { r0 };
        let (b, g, rr) = (r[xx * 4] as i32, r[xx * 4 + 1] as i32, r[xx * 4 + 2] as i32);
        scb = scb.wrapping_add(rr * -7509 + g * -25259 + (b << 15));
        scr = scr.wrapping_add((rr << 15) + g * -29763 + b * -3005);
    }
    let cb = ((scb + 33685504) >> 18).clamp(0, 255) as u16; // 4 * (128 << 16) + 2 << 16
    let cr = ((scr + 33685504) >> 18).clamp(0, 255) as u16;
    (cb, cr)
}
