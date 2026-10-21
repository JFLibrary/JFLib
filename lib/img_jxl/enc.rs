// enc -- JPEG XL writer, lossless modular (model: tools/jxl_enc_ref.py encode_lossless;
// byte-exact)                                                            [src/img/jxl_enc.s]
//
//   encode(sf, opt, out) -> 0 / ERR_*         the codec's encoder (jxl_encode)
//   encode_ex(sf, p) -> file / ERR_*          with the metadata blobs given explicitly
//   je_sizehdr(jw, w, h) / je_container(p, cs)   shared with the VarDCT writer
//
// Codestream (FF 0A, no container): 8-bit samples, sRGB or gray (all pixels R = G = B),
// straight alpha as an extra channel when a pixel is not opaque.  One regular modular frame
// without loop filters; colour through the YCoCg RCT; clamped gradient prediction; MA tree
// with one leaf per channel; ANS.  Frames up to 256 x 256 are one section, larger ones get one
// section per 256 x 256 group.  The image is read in bands of 256 rows, twice: token
// statistics, then the sections.  Lossy requests (quality < 100 without IOF_LOSSLESS; 0 = 90)
// go to the VarDCT writer (venc.rs); EXIF / XMP go into a container (IOF_EMBED_EXIF / _XMP).

use alloc::vec;
use alloc::vec::Vec;

use core::sync::atomic::{AtomicU32, Ordering};

use gfx::FMT_ARGB;
use img_core::exif::exif_copy_orient1;
use img_core::meta::get_row;
use img_core::{ImgOpt, Surface, ERR_CANCEL, ERR_DIMS, IMG_MAXDIM, IOF_EMBED_EXIF, IOF_EMBED_XMP, IOF_LOSSLESS};

use crate::ewr::{Eh, Es, Jw};
use crate::hdr::JTF_SRGB;

const GDIM: u32 = 256;
static D_SIZE: [u32; 8] = [1, 9, 1, 13, 1, 18, 1, 30];
pub static D_TOC: [u32; 8] = [0, 10, 1024, 14, 17408, 22, 4211712, 30];
/// MA tree node lists per channel count (BFS): split value, 0xFF leaf, 0xFE end
pub static TREES: [[u8; 8]; 4] = [
    [0xFF, 0xFE, 0, 0, 0, 0, 0, 0],
    [0, 0xFF, 0xFF, 0xFE, 0, 0, 0, 0],
    [1, 0xFF, 0, 0xFF, 0xFF, 0xFE, 0, 0],
    [1, 2, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE],
];

/// the encoder parameters of IMGOPT that the writers read
pub struct EncParams<'a> {
    pub flags: u32,
    pub quality: u32,
    pub exif: Option<&'a [u8]>, // IM_EXIF (with the IOF_EMBED_EXIF flag)
    pub xmp: Option<&'a [u8]>,  // IM_XMP (with the IOF_EMBED_XMP flag)
    pub cancel: Option<&'a AtomicU32>,
}

impl EncParams<'_> {
    pub fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed) != 0)
    }
}

/// the codec's encoder: the new writer (img_jxl_enc's enew.rs, linked into the app or in the
/// module file: writer.rs, jxl-module), or with IOF_JXL_COMPAT -- and when the app has no new
/// writer / the module file is missing -- the assembly's
pub fn encode(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    if opt.flags & crate::writer::IOF_JXL_COMPAT == 0 {
        let o = crate::writer::Opts::from_imgopt(opt);
        match crate::writer::write(sf, &o) {
            Some(Ok(v)) => {
                *out = v;
                return 0;
            }
            Some(Err(e)) => return e,
            None => {} // jxl-module: no new writer
        }
    }
    encode_compat(sf, opt, out)
}

/// the assembly-compatible encoder: the blobs of opt.meta (an empty blob counts as absent)
pub fn encode_compat(sf: &Surface, opt: &ImgOpt, out: &mut Vec<u8>) -> i32 {
    let (exif, xmp) = match opt.meta {
        Some(m) => (
            if m.exif.is_empty() { None } else { Some(&m.exif[..]) },
            if m.xmp.is_empty() { None } else { Some(&m.xmp[..]) },
        ),
        None => (None, None),
    };
    let p = EncParams { flags: opt.flags, quality: opt.quality, exif, xmp, cancel: opt.cancel };
    match encode_ex(sf, &p) {
        Ok(v) => {
            *out = v;
            0
        }
        Err(e) => e,
    }
}

/// jxl_encode with explicit parameters
pub fn encode_ex(sf: &Surface, p: &EncParams) -> Result<Vec<u8>, i32> {
    if p.flags & IOF_LOSSLESS == 0 && p.quality < 100 {
        return crate::venc::jxl_venc(sf, p);
    }
    let w = sf.w;
    let h = sf.h;
    if w <= 0 || h <= 0 || w as u32 > IMG_MAXDIM || h as u32 > IMG_MAXDIM {
        return Err(ERR_DIMS);
    }
    let mut je = Je::new(sf, p);
    je.run()?;
    let mut cs = je.outw.buf;
    je_container(p, &mut cs)?;
    Ok(cs)
}

/// the lossless encoder state (JE)
struct Je<'a> {
    sf: &'a Surface,
    p: &'a EncParams<'a>,
    w: u32,
    h: u32,
    nch: u32,
    gray: bool,
    alpha: bool,
    xg: u32,
    yg: u32,
    ng: u32,
    row: Vec<u8>,
    band: Vec<i16>, // i16 channels of a band: [c][row][x]
    by0: u32,
    bh: u32,
    plane: usize, // samples per channel in the band (w * 256)
    goff: Vec<usize>,
    gw: Jw,
    grpw: Jw,
    outw: Jw,
}

impl<'a> Je<'a> {
    fn new(sf: &'a Surface, p: &'a EncParams<'a>) -> Je<'a> {
        let w = sf.w as u32;
        let h = sf.h as u32;
        let xg = (w + GDIM - 1) >> 8;
        let yg = (h + GDIM - 1) >> 8;
        Je {
            sf,
            p,
            w,
            h,
            nch: 0,
            gray: true,
            alpha: false,
            xg,
            yg,
            ng: xg * yg,
            row: vec![0; w as usize * 4 + 64],
            band: vec![0; w as usize * 256 * 4],
            by0: 0,
            bh: 0,
            plane: w as usize * 256,
            goff: vec![0; (xg * yg) as usize],
            gw: Jw::new(),
            grpw: Jw::new(),
            outw: Jw::new(),
        }
    }

    fn run(&mut self) -> Result<(), i32> {
        self.scan();
        self.nch = if self.gray { 1 } else { 3 } + self.alpha as u32;
        // pass 1: residual statistics
        let mut eh = Eh::new(self.nch, None);
        for gy in 0..self.yg {
            self.band_load(gy)?;
            for gx in 0..self.xg {
                self.group(gx, &mut Sink::Count(&mut eh));
            }
        }
        let r = eh.finish();
        if r != 0 {
            return Err(r);
        }
        // DC global section: default DC quantization, global tree and histograms, header
        let mut gw = core::mem::take(&mut self.gw);
        gw.bits(3, 2);
        let r = self.tree(&mut gw);
        if r != 0 {
            return Err(r);
        }
        let r = eh.write(&mut gw);
        if r != 0 {
            return Err(r);
        }
        ghdr(&mut gw, !self.gray);
        if self.ng == 1 {
            // one group: its tokens follow (band still loaded)
            let mut es = Es::new();
            self.group(0, &mut Sink::Put(&eh, &mut gw, &mut es));
            es.flush(&eh, &mut gw);
        } else {
            // pass 2: one section per group
            let mut grpw = core::mem::take(&mut self.grpw);
            for gy in 0..self.yg {
                self.band_load(gy)?;
                for gx in 0..self.xg {
                    ghdr(&mut grpw, false);
                    let mut es = Es::new();
                    self.group(gx, &mut Sink::Put(&eh, &mut grpw, &mut es));
                    es.flush(&eh, &mut grpw);
                    grpw.pad();
                    self.goff[(gy * self.xg + gx) as usize] = grpw.len();
                }
            }
            self.grpw = grpw;
        }
        gw.pad();
        self.gw = gw;
        // codestream: headers, frame header, TOC, sections
        self.headers();
        self.toc();
        let g = core::mem::take(&mut self.gw.buf);
        self.outw.data(&g);
        let g = core::mem::take(&mut self.grpw.buf);
        self.outw.data(&g);
        Ok(())
    }

    /// je_scan: gray (every pixel has R = G = B), alpha (a pixel is not opaque)
    fn scan(&mut self) {
        self.gray = true;
        self.alpha = false;
        for y in 0..self.h as usize {
            get_row(self.sf, y, &mut self.row, FMT_ARGB);
            for x in 0..self.w as usize {
                let p = &self.row[x * 4..x * 4 + 4];
                if p[3] != 255 {
                    self.alpha = true;
                }
                if p[0] != p[1] || p[0] != p[2] {
                    self.gray = false;
                }
            }
        }
    }

    /// je_band: the channels of group row gy (gray: R; colour: Y, Co, Cg; then alpha)
    fn band_load(&mut self, gy: u32) -> Result<(), i32> {
        if self.p.cancelled() {
            return Err(ERR_CANCEL);
        }
        self.by0 = gy << 8;
        self.bh = (self.h - self.by0).min(GDIM);
        let w = self.w as usize;
        let pl = self.plane;
        for r in 0..self.bh as usize {
            get_row(self.sf, self.by0 as usize + r, &mut self.row, FMT_ARGB);
            for x in 0..w {
                let b = self.row[x * 4] as i32;
                let g = self.row[x * 4 + 1] as i32;
                let rr = self.row[x * 4 + 2] as i32;
                let o = r * w + x;
                let c;
                if self.gray {
                    self.band[o] = rr as i16;
                    c = 1;
                } else {
                    let co = rr - b;
                    let tmp = b + (co >> 1);
                    let cg = g - tmp;
                    let y = tmp + (cg >> 1);
                    self.band[o] = y as i16;
                    self.band[pl + o] = co as i16;
                    self.band[2 * pl + o] = cg as i16;
                    c = 3;
                }
                if self.alpha {
                    self.band[c * pl + o] = self.row[x * 4 + 3] as i16;
                }
            }
        }
        Ok(())
    }

    /// je_group: the residuals of group gx in the loaded band, channel by channel; leaf of
    /// channel c = nch - 1 - c
    fn group(&self, gx: u32, sink: &mut Sink) {
        let x0 = (gx << 8) as usize;
        let gw = (self.w as usize - x0).min(GDIM as usize);
        let w = self.w as usize;
        for c in 0..self.nch as usize {
            let leaf = self.nch - 1 - c as u32;
            for y in 0..self.bh as usize {
                let row = &self.band[c * self.plane + y * w + x0..];
                let above = if y > 0 { Some(&self.band[c * self.plane + (y - 1) * w + x0..]) } else { None };
                for x in 0..gw {
                    let v = row[x] as i32;
                    let (l, t, tl) = match above {
                        None => {
                            let l = if x > 0 { row[x - 1] as i32 } else { 0 };
                            (l, l, l)
                        }
                        Some(a) => {
                            let t = a[x] as i32;
                            if x > 0 { (row[x - 1] as i32, t, a[x - 1] as i32) } else { (t, t, t) }
                        }
                    };
                    let r = v.wrapping_sub(cgrad(l, t, tl));
                    sink.emit(leaf, pack_signed(r));
                }
            }
        }
    }

    /// je_tree: the MA tree (splits on the channel down to one leaf per channel, BFS order)
    /// with its own histograms (6 contexts)
    fn tree(&self, jw: &mut Jw) -> i32 {
        tree_write(jw, self.nch)
    }

    /// je_headers: signature, size header, image metadata, transform data, frame header
    fn headers(&mut self) {
        let o = &mut self.outw;
        o.bits(0x0AFF, 16);
        je_sizehdr(o, self.w, self.h);
        // metadata: not all_default, no extra fields, integer, 8 bits (sel 0), modular_16bit
        o.bits(0b100000, 6);
        o.bits(self.alpha as u64, 2); // extra channels: 0 or 1 (all_default alpha)
        if self.alpha {
            o.bits(1, 1);
        }
        o.bits(0, 1); // xyb_encoded 0
        if !self.gray {
            o.bits(1, 1); // colour encoding all_default (sRGB)
        } else {
            o.bits(0, 2); // not all_default, no ICC
            o.enum_(1); // gray
            o.enum_(1); // D65
            o.bits(0, 1); // no gamma
            o.enum_(JTF_SRGB as u32);
            o.enum_(1); // relative intent
        }
        o.bits(0b100, 3); // extensions 0; transform data all_default
        o.pad();
        // frame header: not all_default, regular, modular, flags 0, no YCbCr, upsampling 1
        o.bits(0b1000, 9);
        if self.alpha {
            o.bits(0, 2); // extra channel upsampling 1
        }
        o.bits(1, 7); // group size shift 1, passes 1, no custom size, blending replace
        if self.alpha {
            o.bits(0, 2); // extra channel blending replace
        }
        // is_last, name length 0, loop filter: not all_default, no gaborish, EPF 0,
        // extensions 0; frame extensions 0
        o.bits(1, 11);
    }

    /// je_toc: no permutation; section sizes (global, DC groups, AC global, AC groups)
    fn toc(&mut self) {
        let glen = self.gw.len() as u32;
        let o = &mut self.outw;
        o.bits(0, 1);
        o.pad();
        o.u32(glen, &D_TOC);
        if self.ng != 1 {
            let ndg = ((self.w + 2047) >> 11) * ((self.h + 2047) >> 11);
            for _ in 0..ndg + 1 {
                o.u32(0, &D_TOC);
            }
            let mut prev = 0u32;
            for &e in &self.goff {
                let e = e as u32;
                o.u32(e.wrapping_sub(prev), &D_TOC);
                prev = e;
            }
        }
        o.pad();
    }
}

/// where the residuals go: counting (pass 1) or the token stream
pub enum Sink<'s, 'e> {
    Count(&'s mut Eh),
    Put(&'e Eh, &'s mut Jw, &'s mut Es),
}

impl Sink<'_, '_> {
    #[inline]
    pub fn emit(&mut self, ctx: u32, v: u32) {
        match self {
            Sink::Count(eh) => eh.add(ctx, v),
            Sink::Put(eh, jw, es) => es.put(eh, jw, ctx, v),
        }
    }
}

/// clamped gradient( left, top, topleft ) (32 bit)
#[inline(always)]
pub fn cgrad(l: i32, t: i32, tl: i32) -> i32 {
    let (m, mx) = if l > t { (t, l) } else { (l, t) };
    if tl > mx {
        m
    } else if tl < m {
        mx
    } else {
        l.wrapping_add(t).wrapping_sub(tl)
    }
}

/// pack_signed
#[inline(always)]
pub fn pack_signed(r: i32) -> u32 {
    if r >= 0 { (r as u32).wrapping_mul(2) } else { (r.wrapping_neg() as u32).wrapping_mul(2).wrapping_sub(1) }
}

/// the MA tree of nch channels (one leaf each) with its own histograms  [je_tree / ve_tree]
pub fn tree_write(jw: &mut Jw, nch: u32) -> i32 {
    let nodes = &TREES[(nch - 1) as usize & 3];
    let mut tok: Vec<(u32, u32)> = Vec::with_capacity(32);
    for &n in nodes.iter() {
        if n == 0xFE {
            break;
        }
        if n == 0xFF {
            tok.push((1, 0)); // leaf
            tok.push((2, 5)); // gradient
            tok.push((3, 0)); // offset 0
            tok.push((4, 0)); // multiplier log 0
            tok.push((5, 0)); // multiplier bits 0
        } else {
            tok.push((1, 1)); // property 0 + 1
            tok.push((0, (n as u32) * 2)); // split value
        }
    }
    let mut eh = Eh::new(6, None);
    for &(c, v) in &tok {
        eh.add(c, v);
    }
    let r = eh.finish();
    if r != 0 {
        return r;
    }
    let r = eh.write(jw);
    if r != 0 {
        return r;
    }
    let mut es = Es::new();
    for &(c, v) in &tok {
        es.put(&eh, jw, c, v);
    }
    es.flush(&eh, jw);
    0
}

/// je_ghdr: modular group header: global tree, default WP header, RCT 6 or no transforms
fn ghdr(jw: &mut Jw, rct: bool) {
    jw.bits(3, 2); // use_global_tree, wp all_default
    if rct {
        jw.bits(1, 11); // 1 transform, RCT, begin_c 0, rct_type 6
    } else {
        jw.bits(0, 2); // 0 transforms
    }
}

/// je_sizehdr: SizeHeader: small when both are <= 256 and multiples of 8, else U32 h, w
pub fn je_sizehdr(jw: &mut Jw, w: u32, h: u32) {
    if w <= 256 && h <= 256 && (w | h) & 7 == 0 {
        let c = ((h >> 3) - 1) << 1 | 1;
        let v = ((w >> 3) - 1) << 9 | c;
        jw.bits(v as u64, 14);
        return;
    }
    jw.bits(0, 1);
    jw.u32(h, &D_SIZE);
    jw.bits(0, 3);
    jw.u32(w, &D_SIZE);
}

/// je_container: with IOF_EMBED_EXIF / IOF_EMBED_XMP and the blobs: the codestream goes into
/// the container: signature box, ftyp (jxl ), Exif (offset 0, orientation reset to 1), xml,
/// jxlc
pub fn je_container(p: &EncParams, cs: &mut Vec<u8>) -> Result<(), i32> {
    let mut exif: Option<Vec<u8>> = None;
    let mut xmp: Option<&[u8]> = None;
    if p.flags & IOF_EMBED_EXIF != 0
        && let Some(e) = p.exif
        && !e.is_empty()
    {
        exif = Some(exif_copy_orient1(e));
    }
    if p.flags & IOF_EMBED_XMP != 0 {
        xmp = p.xmp;
    }
    if exif.is_none() && xmp.is_none() {
        return Ok(());
    }
    let mut o: Vec<u8> = Vec::with_capacity(cs.len() + 64);
    o.extend_from_slice(&[0, 0, 0, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A]);
    o.extend_from_slice(&[0, 0, 0, 0x14, b'f', b't', b'y', b'p', b'j', b'x', b'l', b' ', 0, 0, 0, 0, b'j', b'x', b'l', b' ']);
    if let Some(e) = &exif {
        o.extend_from_slice(&((e.len() as u32).wrapping_add(12)).to_be_bytes());
        o.extend_from_slice(b"Exif");
        o.extend_from_slice(&[0, 0, 0, 0]);
        o.extend_from_slice(e);
    }
    if let Some(x) = xmp {
        o.extend_from_slice(&((x.len() as u32).wrapping_add(8)).to_be_bytes());
        o.extend_from_slice(b"xml ");
        o.extend_from_slice(x);
    }
    o.extend_from_slice(&((cs.len() as u32).wrapping_add(8)).to_be_bytes());
    o.extend_from_slice(b"jxlc");
    o.extend_from_slice(cs);
    *cs = o;
    Ok(())
}
