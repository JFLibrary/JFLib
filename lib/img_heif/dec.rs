// dec -- HEIF reader: HEIC (HEVC still images) and AVIF (AV1 still images)   [src/img/heic.s]
//
//   probe(data, meta) -> recognised          container walk and image properties into meta
//                                            (no blobs, no decoding)  [heic_probe]
//   decode(data, sf, dec, meta) -> 0 / ERR_* decode the primary image into a new FMT_ARGB /
//                                            FMT_PARGB surface  [heic_decode]
//
// Container: bmff.rs (hdlr "pict", pitm required).  The primary item is an hvc1 / hev1 / av01
// coded image or a grid of them (dimg references in raster order, output size from the grid
// descriptor, tiles placed at multiples of the first tile's size and clipped); other item
// types (iovl, iden, ...): ERR_UNSUPPORTED.  HEVC items: the NAL units of the hvcC arrays,
// then the length-prefixed NAL units of the item data, through img_hevc.  AV1 items: the
// configOBUs of av1C, then the OBUs of the item data, through img_av1 (the first shown frame,
// film grain applied); colour from the AV1 sequence header when there is no colr nclx;
// chroma position: unknown -> centred (location type 1), vertical -> 0, co-located -> 2.
// Colour: the first colr nclx of the primary item, else of the tile, else the HEVC VUI (no
// colour description: matrix 2 = BT.601; no VUI: limited range); img_core::yuv converts.
// Alpha: an item with an auxl reference to the primary item whose auxC type is
// "urn:mpeg:hevc:2015:auxid:1" or "urn:mpeg:mpegB:cicp:systems:auxiliary:alpha" (coded or
// grid; its Y plane, full range, clipped to the image); a prem reference from the primary to
// it: premultiplied (FMT_PARGB, colour clamped to alpha).
// Transformations of the primary item in ipma order: clap (left = floor(horizOff + (w -
// cleanW) / 2), width = round(cleanW), the same vertically, clamped to the image), irot and
// imir (axis 0 exchanges top and bottom, 1 left and right, as clarified in ISO/IEC
// 23008-12:2022).  The crop is applied here; rotation / mirroring are combined into
// ImgMeta::orient (EXIF numbering) for the dispatcher (IMGD_NO_ORIENT keeps the stored
// orientation); a crop after a rotation is mapped back first.  The EXIF orientation is
// ignored (HEIF rule).
// Metadata: ICC from colr prof / rICC (IMGD_ICC), EXIF from an Exif item (cdsc reference to
// the primary item preferred; the leading u32 TIFF header offset is skipped; IMGD_EXIF), XMP
// from a mime item "application/rdf+xml" (IMGD_XMP).  ImgMeta: size after the crop, bits =
// luma bit depth (hvcC / av1C), model YCC / GRAY, channels, IMF_ALPHA / IMF_SRC_PREMUL /
// IMF_HDR (> 8 bits, PQ / HLG), cicp (nclx, after a decode the values used), orient.
//
// Grid tiles after the first decode on the worker pool (plat::parallel_for); each converts
// into a tile-sized surface whose rows are then copied into the image under per-row locks
// (the assembly converts straight into the image); the error of the lowest failing tile
// wins.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use gfx::{FMT_ARGB, FMT_PARGB, SFF_OPAQUE, Surface};
use img_av1::Av1Dec;
use img_core::meta::{meta_blob, surf_alloc, Blob};
use img_core::yuv::{self, YuvPlanes};
use img_core::{
    ImgDec, ImgMeta, ERR_HDR, ERR_LIMIT, ERR_NOMEM, ERR_TRUNC, ERR_UNSUPPORTED, IMF_ALPHA, IMF_HDR, IMF_SRC_PREMUL,
    IMGD_EXIF, IMGD_ICC, IMGD_PROBE, IMGD_XMP, IMM_GRAY, IMM_YCC,
};
use img_hevc::Hevc;
use rt::sync::SpinLock;

use crate::bmff::*;

const KIND_CODED: u32 = 1; // HEVC (hvc1 / hev1)
const KIND_GRID: u32 = 2;
const KIND_AV1: u32 = 3; // av01
const TRC_PQ: u32 = 16;
const TRC_HLG: u32 = 18;

static AUX_HEVC: &[u8] = b"urn:mpeg:hevc:2015:auxid:1";
static AUX_MPEG: &[u8] = b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha";
static MIME_XMP: &[u8] = b"application/rdf+xml";
/// irot angle (anticlockwise quarter turns) -> EXIF orientation
static IROT_OR: [u8; 4] = [1, 8, 3, 6];
/// EXIF orientation 0..8 -> s << 2 | fx << 1 | fy of its inverse map (display -> stored)
static OR_CODE: [u8; 9] = [0, 0, 2, 3, 1, 4, 5, 7, 6];
/// orientation of "T, then P": [T - 1][P - 1]
static COMP_TAB: [[u8; 8]; 8] = [
    [1, 2, 3, 4, 5, 6, 7, 8],
    [2, 1, 4, 3, 8, 7, 6, 5],
    [3, 4, 1, 2, 7, 8, 5, 6],
    [4, 3, 2, 1, 6, 5, 8, 7],
    [5, 6, 7, 8, 1, 2, 3, 4],
    [6, 5, 8, 7, 4, 3, 2, 1],
    [7, 8, 5, 6, 3, 4, 1, 2],
    [8, 7, 6, 5, 2, 1, 4, 3],
];

#[inline]
fn be16(d: &[u8], o: usize) -> u32 {
    u16::from_be_bytes([d[o], d[o + 1]]) as u32
}

#[inline]
fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// hc_kind: KIND_CODED (hvc1 / hev1), KIND_AV1 (av01), KIND_GRID or 0
fn kind_of(ty: u32) -> u32 {
    match ty {
        TAG_HVC1 | TAG_HEV1 => KIND_CODED,
        TAG_AV01 => KIND_AV1,
        TAG_GRID => KIND_GRID,
        _ => 0,
    }
}

/// hc_compose: orientation of "T, then P" (EXIF numbering 1..8)
fn compose(t: u32, p: u32) -> u32 {
    COMP_TAB[(t.wrapping_sub(1) & 7) as usize][(p.wrapping_sub(1) & 7) as usize] as u32
}

/// hc_nclx: CICP of a colr nclx payload (primaries | transfer << 8 | matrix << 16 | full
/// range << 24; values above 255 saturate), None when shorter than 11 bytes
fn nclx(p: &[u8]) -> Option<u32> {
    if p.len() < 11 {
        return None;
    }
    let prim = be16(p, 4).min(255);
    let trc = be16(p, 6).min(255);
    let mc = be16(p, 8).min(255);
    Some(prim | trc << 8 | mc << 16 | ((p[10] >> 7) as u32) << 24)
}

/// hc_streq: the string at p (ends at a NUL or after its length) equals s
fn streq(p: &[u8], s: &[u8]) -> bool {
    let mut i = 0;
    loop {
        let a = s.get(i).copied().unwrap_or(0);
        let b = p.get(i).copied().unwrap_or(0);
        if a != b {
            return false;
        }
        if a == 0 {
            return true;
        }
        i += 1;
    }
}

/// the rows of the target surface for grid tiles decoded in parallel
struct Rows<'s> {
    rows: Vec<SpinLock<&'s mut [u8]>>,
    w: i32,
    h: i32,
}

/// where a tile's pixels go: the surface itself, or its rows (parallel tiles)
enum Dst<'d, 's> {
    Direct(&'d mut Surface),
    Rows(&'d Rows<'s>),
}

/// the decoder context (HC)
struct Hc<'a, 'b> {
    bx: Bx<'a>,
    dec: &'b ImgDec<'b>,
    prim: u32,   // primary item id
    pidx: u32,   // its index
    w: u32,      // size of the decoded image (ispe / grid output)
    h: u32,
    bits: u32,   // luma bit depth (hvcC / av1C)
    chroma: u32, // chroma_format_idc (hvcC / av1C: 0 = monochrome)
    alpha: u32,  // alpha item id (0 = none)
    prem: u32,   // 1 = premultiplied colour
    nclx: u32,   // CICP of the primary's colr nclx
    hasnclx: u32,
    orient: u32, // EXIF orientation of irot / imir
    cx: u32,     // crop rectangle in the decoded image
    cy: u32,
    cw: u32,
    ch: u32,
    icc: Option<&'a [u8]>, // ICC profile in the file
    mode: u32,             // 0 colour, 1 alpha
    tw: u32,               // size of the last decoded tile
    th: u32,
    cicp0: u32, // CICP used for the first colour tile
    have0: u32,
}

/// what a decoded tile reports to the serial caller: its size, the CICP used
struct TileInfo {
    w: u32,
    h: u32,
    cicp: u32,
}

/// heic_probe: container walk and image properties into meta
pub fn probe(data: &[u8], meta: &mut ImgMeta) -> bool {
    let Ok(bx) = Bx::parse(data) else { return false };
    let dec = ImgDec::default();
    let mut hc = Hc::new(bx, &dec);
    if hc.info() != 0 {
        return false;
    }
    hc.meta(meta);
    true
}

/// heic_decode: decode the primary image (see the file header) into sf
pub fn decode(data: &[u8], sf: &mut Surface, dec: &ImgDec, meta: &mut ImgMeta) -> i32 {
    *sf = Surface::default();
    let bx = match Bx::parse(data) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let mut hc = Hc::new(bx, dec);
    let r = hc.info();
    if r != 0 {
        return r;
    }
    hc.meta(meta);
    hc.blobs(meta, dec.flags);
    if dec.flags & IMGD_PROBE != 0 {
        return 0;
    }
    if dec.frame != 0 {
        return ERR_LIMIT;
    }
    *sf = match surf_alloc(hc.w, hc.h, FMT_ARGB, dec) {
        Ok(s) => s,
        Err(e) => return e,
    };
    // colour
    hc.mode = 0;
    let r = hc.image(sf, hc.prim);
    if r != 0 {
        return r;
    }
    // alpha
    if hc.alpha != 0 {
        hc.mode = 1;
        let r = hc.image(sf, hc.alpha);
        if r != 0 {
            return r;
        }
        if hc.prem != 0 {
            yuv::prem_clamp(sf);
            sf.fmt = FMT_PARGB;
        }
    } else {
        sf.flags |= SFF_OPAQUE;
    }
    if hc.cx | hc.cy != 0 || hc.cw != hc.w || hc.ch != hc.h {
        let fl = sf.flags;
        match img_core::ops::crop(sf, hc.cx as i32, hc.cy as i32, hc.cw as i32, hc.ch as i32) {
            Ok(t) => *sf = t,
            Err(_) => return ERR_NOMEM,
        }
        sf.flags |= fl & SFF_OPAQUE;
    }
    if hc.have0 != 0 {
        meta.cicp = hc.cicp0;
    }
    0
}

impl<'a, 'b> Hc<'a, 'b> {
    fn new(bx: Bx<'a>, dec: &'b ImgDec<'b>) -> Hc<'a, 'b> {
        Hc {
            bx,
            dec,
            prim: 0,
            pidx: 0,
            w: 0,
            h: 0,
            bits: 0,
            chroma: 0,
            alpha: 0,
            prem: 0,
            nclx: 0,
            hasnclx: 0,
            orient: 0,
            cx: 0,
            cy: 0,
            cw: 0,
            ch: 0,
            icc: None,
            mode: 0,
            tw: 0,
            th: 0,
            cicp0: 0,
            have0: 0,
        }
    }

    // ------------------------------------------------------------------ item analysis

    /// hc_info: primary item, size, bit depth, alpha, colour, transformations of a parsed
    /// file -> 0 / ERR_*
    fn info(&mut self) -> i32 {
        if self.bx.hdlr != TAG_PICT {
            return ERR_UNSUPPORTED;
        }
        self.prim = self.bx.primary;
        let k = self.bx.find(self.prim);
        if k < 0 {
            return ERR_HDR;
        }
        self.pidx = k as u32;
        let kind = kind_of(self.bx.items[k as usize].ty);
        if kind == 0 {
            return ERR_UNSUPPORTED;
        }
        let mut src = self.prim; // item with the hvcC
        if kind == KIND_GRID {
            let gd = match self.griddesc(src) {
                Ok(g) => g,
                Err(e) => return e,
            };
            self.w = gd[2];
            self.h = gd[3];
            let t = self.bx.ref_(TAG_DIMG, src, 0);
            if t < 0 {
                return ERR_HDR;
            }
            src = t as u32; // first tile
        } else {
            let Some(p) = self.bx.prop(src, TAG_ISPE) else { return ERR_HDR };
            if p.len() < 12 {
                return ERR_HDR;
            }
            self.w = be32(p, 4);
            self.h = be32(p, 8);
        }
        if self.w == 0 || self.h == 0 {
            return ERR_HDR;
        }
        if self.itype(src) == TAG_AV01 {
            let Some(p) = self.bx.prop(src, TAG_AV1C) else { return ERR_HDR };
            if p.len() < 4 {
                return ERR_HDR;
            }
            let b = p[2]; // tier, high_bitdepth, twelve_bit, mono, ssx, ssy, csp
            self.bits = if b & 0x40 == 0 {
                8
            } else if b & 0x20 == 0 {
                10
            } else {
                12
            };
            self.chroma = if b & 0x10 != 0 {
                0 // monochrome
            } else if b & 0x08 == 0 {
                3 // 4:4:4
            } else if b & 0x04 == 0 {
                2 // 4:2:2
            } else {
                1 // 4:2:0
            };
        } else {
            let Some(p) = self.bx.prop(src, TAG_HVCC) else { return ERR_HDR };
            if p.len() < 23 {
                return ERR_HDR;
            }
            self.chroma = (p[16] & 3) as u32;
            self.bits = (p[17] & 7) as u32 + 8;
        }
        // alpha: an auxl reference to the primary with an alpha auxC
        let mut k = 0;
        loop {
            let a = self.bx.rref(TAG_AUXL, self.prim, k);
            if a < 0 {
                break;
            }
            k += 1;
            if self.isalpha(a as u32) {
                self.alpha = a as u32;
                break;
            }
        }
        if self.alpha != 0 {
            let mut k = 0;
            loop {
                let t = self.bx.ref_(TAG_PREM, self.prim, k);
                if t < 0 {
                    break;
                }
                k += 1;
                if t as u32 == self.alpha {
                    self.prem = 1;
                    break;
                }
            }
        }
        // properties of the primary in ipma order
        self.orient = 1;
        self.cw = self.w;
        self.ch = self.h;
        let mut k = 0;
        while let Some((p, _)) = self.bx.assoc(self.pidx, k) {
            k += 1;
            let pay = self.bx.payload(&p);
            match p.ty {
                TAG_COLR => {
                    if pay.len() < 4 {
                        continue;
                    }
                    match u32::from_le_bytes([pay[0], pay[1], pay[2], pay[3]]) {
                        TAG_NCLX => {
                            if self.hasnclx == 0
                                && let Some(c) = nclx(pay)
                            {
                                self.nclx = c;
                                self.hasnclx = 1;
                            }
                        }
                        TAG_PROF | TAG_RICC => {
                            if self.icc.is_none() {
                                self.icc = Some(&pay[4..]);
                            }
                        }
                        _ => {}
                    }
                }
                TAG_IROT => {
                    if !pay.is_empty() {
                        let o = IROT_OR[(pay[0] & 3) as usize] as u32;
                        self.orient = compose(self.orient, o);
                    }
                }
                TAG_IMIR => {
                    if !pay.is_empty() {
                        // axis 0: top / bottom exchanged (EXIF 4), 1: left / right (EXIF 2),
                        // ISO/IEC 23008-12:2022
                        let o = if pay[0] & 1 != 0 { 2 } else { 4 };
                        self.orient = compose(self.orient, o);
                    }
                }
                TAG_CLAP => {
                    if pay.len() >= 32 {
                        let r = self.clap(pay);
                        if r != 0 {
                            return r;
                        }
                    }
                }
                _ => {}
            }
        }
        0
    }

    /// hc_itype: item type of id (0 when there is no such item)
    fn itype(&self, id: u32) -> u32 {
        let k = self.bx.find(id);
        if k < 0 { 0 } else { self.bx.items[k as usize].ty }
    }

    /// hc_isalpha: item id is a coded / grid image with an alpha auxC
    fn isalpha(&self, id: u32) -> bool {
        let k = self.bx.find(id);
        if k < 0 || kind_of(self.bx.items[k as usize].ty) == 0 {
            return false;
        }
        let Some(p) = self.bx.prop(id, TAG_AUXC) else { return false };
        if p.len() < 4 {
            return false;
        }
        let t = &p[4..]; // aux_type
        streq(t, AUX_HEVC) || streq(t, AUX_MPEG)
    }

    /// hc_griddesc: the grid descriptor of item id: rows, columns, output width, height
    fn griddesc(&self, id: u32) -> Result<[u32; 4], i32> {
        let d = self.bx.item(id)?;
        if d.len() < 8 || d[0] != 0 {
            return Err(ERR_HDR);
        }
        let rows = d[2] as u32 + 1;
        let cols = d[3] as u32 + 1;
        if d[1] & 1 == 0 {
            Ok([rows, cols, be16(&d, 4), be16(&d, 6)])
        } else if d.len() < 12 {
            Err(ERR_HDR)
        } else {
            Ok([rows, cols, be32(&d, 4), be32(&d, 8)])
        }
    }

    /// hc_clap: fold a clean aperture (given in the coordinates after the current
    /// orientation) into the crop rectangle -> 0 / ERR_HDR
    fn clap(&mut self, p: &[u8]) -> i32 {
        // denominators 1..65535, widths / heights 1..2^31 - 1
        for i in 0..8 {
            let v = be32(p, i * 4);
            if i & 1 != 0 {
                if v.wrapping_sub(1) > 65534 {
                    return 0;
                }
            } else if i < 4 && v.wrapping_sub(1) > 0x7FFF_FFFE {
                return 0;
            }
        }
        // display size of the current crop
        let o = if self.orient <= 8 { self.orient } else { 1 };
        let code = OR_CODE[o as usize] as u32; // s << 2 | fx << 1 | fy
        let (mut dw, mut dh) = (self.cw, self.ch);
        if code & 4 != 0 {
            core::mem::swap(&mut dw, &mut dh);
        }
        // width = floor((2 wN + wD) / (2 wD)); left = floor((2 oN cD + (dim cD - cN) oD) /
        // (2 oD cD)); the same vertically
        let mut lt = [0i64; 2];
        let mut sz = [0i64; 2];
        for d in 0..2 {
            let b = d * 8;
            let cn = be32(p, b) as u64;
            let cd = be32(p, b + 4) as u64;
            sz[d] = ((cn * 2 + cd) / (cd * 2)) as i64;
            let on = be32(p, b + 16) as i32 as i64;
            let od = be32(p, b + 20) as i64;
            let dim = if d == 0 { dw } else { dh } as i64;
            let num = (dim * cd as i64 - cn as i64) * od + on * 2 * cd as i64;
            let den = od * cd as i64 * 2;
            let mut q = num / den;
            if num % den < 0 {
                q -= 1;
            }
            lt[d] = q;
        }
        // rectangle in display coordinates, clamped
        let mut l = lt[0];
        let mut t = lt[1];
        let mut r = l + sz[0] - 1;
        let mut b = t + sz[1] - 1;
        l = l.max(0);
        t = t.max(0);
        r = r.min(dw as i64 - 1);
        b = b.min(dh as i64 - 1);
        if l > r || t > b {
            return ERR_HDR;
        }
        // back to stored coordinates: (u, v) = s ? (y, x) : (x, y), flips
        if code & 4 != 0 {
            core::mem::swap(&mut l, &mut t);
            core::mem::swap(&mut r, &mut b);
        }
        if code & 2 != 0 {
            let m = self.cw.wrapping_sub(1) as i64;
            l = m - l;
            r = m - r;
        }
        if code & 1 != 0 {
            let m = self.ch.wrapping_sub(1) as i64;
            t = m - t;
            b = m - b;
        }
        if l > r {
            core::mem::swap(&mut l, &mut r);
        }
        if t > b {
            core::mem::swap(&mut t, &mut b);
        }
        self.cx = self.cx.wrapping_add(l as u32);
        self.cy = self.cy.wrapping_add(t as u32);
        self.cw = (r - l) as u32 + 1;
        self.ch = (b - t) as u32 + 1;
        0
    }

    /// hc_meta: ImgMeta fields of the analysed file
    fn meta(&self, m: &mut ImgMeta) {
        m.w = self.cw;
        m.h = self.ch;
        m.bits = self.bits as u8;
        let mut ch = 1u8;
        m.model = IMM_GRAY;
        if self.chroma != 0 {
            m.model = IMM_YCC;
            ch = 3;
        }
        let mut fl = 0u32;
        if self.alpha != 0 {
            ch += 1;
            fl |= IMF_ALPHA;
            if self.prem != 0 {
                fl |= IMF_SRC_PREMUL;
            }
        }
        m.channels = ch;
        if self.bits > 8 {
            fl |= IMF_HDR;
        }
        if self.hasnclx != 0 {
            m.cicp = self.nclx;
            let trc = (self.nclx >> 8) & 255;
            if trc == TRC_PQ || trc == TRC_HLG {
                fl |= IMF_HDR;
            }
        }
        m.flags |= fl;
        m.orient = self.orient as u8;
    }

    /// hc_blobs: ICC / EXIF / XMP into meta (unreadable metadata is skipped)
    fn blobs(&self, m: &mut ImgMeta, flags: u32) {
        if flags & IMGD_ICC != 0
            && let Some(icc) = self.icc
        {
            meta_blob(m, Blob::Icc, icc);
        }
        if flags & IMGD_EXIF != 0 {
            let id = self.metaitem(TAG_EXIF, None);
            if id >= 0
                && let Ok(d) = self.bx.item(id as u32)
                && d.len() >= 4
            {
                let off = be32(&d, 0) as usize; // exif_tiff_header_offset
                if off <= d.len() - 4 {
                    meta_blob(m, Blob::Exif, &d[off + 4..]);
                }
            }
        }
        if flags & IMGD_XMP != 0 {
            let id = self.metaitem(TAG_MIME, Some(MIME_XMP));
            if id >= 0
                && let Ok(d) = self.bx.item(id as u32)
            {
                meta_blob(m, Blob::Xmp, &d);
            }
        }
    }

    /// hc_metaitem: id of a metadata item of that type (and content_type ctype), preferring
    /// one with a cdsc reference to the primary item, or -1
    fn metaitem(&self, ty: u32, ctype: Option<&[u8]>) -> i32 {
        for pass in 0..2 {
            for it in &self.bx.items {
                if it.ty != ty {
                    continue;
                }
                if let Some(s) = ctype {
                    if it.ctype == 0 {
                        continue;
                    }
                    let o = it.ctype as usize;
                    if !streq(&self.bx.data[o..o + it.ctlen as usize], s) {
                        continue;
                    }
                }
                if pass == 1 {
                    return it.id as i32;
                }
                let mut k = 0;
                loop {
                    let r = self.bx.ref_(TAG_CDSC, it.id, k);
                    if r < 0 {
                        break;
                    }
                    k += 1;
                    if r as u32 == self.prim {
                        return it.id as i32;
                    }
                }
            }
        }
        -1
    }

    // ------------------------------------------------------------------ decoding

    /// hc_image: decode item id (coded or grid) into sf: colour (mode 0) or alpha (1); grid
    /// tiles after the first run on the worker pool
    fn image(&mut self, sf: &mut Surface, id: u32) -> i32 {
        let k = self.bx.find(id);
        if k < 0 {
            return ERR_HDR;
        }
        let kind = kind_of(self.bx.items[k as usize].ty);
        if kind == 0 {
            return ERR_UNSUPPORTED;
        }
        if kind != KIND_GRID {
            return self.tile_serial(sf, id, 0, 0);
        }
        // tile 0 first (tile size, colour), then the others on the worker pool; the error
        // of the lowest failing tile wins
        let gd = match self.griddesc(id) {
            Ok(g) => g,
            Err(e) => return e,
        };
        let tiles = gd[0].wrapping_mul(gd[1]);
        let t0 = self.bx.ref_(TAG_DIMG, id, 0);
        if t0 < 0 {
            return ERR_HDR;
        }
        let r = self.tile_serial(sf, t0 as u32, 0, 0);
        if r != 0 {
            return r;
        }
        if tiles <= 1 {
            return 0;
        }
        let cols = gd[1];
        let (gtw, gth) = (self.tw, self.th);
        let res: Vec<AtomicU32> = (0..tiles).map(|_| AtomicU32::new(0)).collect();
        let stride = (sf.stride as usize).max(1);
        let (w, h) = (sf.w, sf.h);
        let rows = Rows { rows: sf.px.chunks_mut(stride).map(SpinLock::new).collect(), w, h };
        let me: &Hc = self;
        let item = |i: usize, _: &plat::JobCtl| {
            let t = i as u32 + 1; // tile
            let tid = me.bx.ref_(TAG_DIMG, id, t);
            let r = if tid < 0 {
                ERR_HDR
            } else {
                let dx = (t % cols).wrapping_mul(gtw);
                let dy = (t / cols).wrapping_mul(gth);
                match me.tile(&mut Dst::Rows(&rows), tid as u32, dx, dy) {
                    Ok(_) => 0,
                    Err(e) => e,
                }
            };
            res[t as usize].store(r as u32, Ordering::Relaxed);
        };
        let _ = plat::parallel_for((tiles - 1) as usize, &item);
        for r in res.iter().skip(1) {
            let r = r.load(Ordering::Relaxed) as i32;
            if r != 0 {
                return r;
            }
        }
        0
    }

    /// a tile decoded by the calling thread: hc_tile with the tile size and the first
    /// colour CICP kept
    fn tile_serial(&mut self, sf: &mut Surface, id: u32, dx: u32, dy: u32) -> i32 {
        match self.tile(&mut Dst::Direct(sf), id, dx, dy) {
            Ok(ti) => {
                self.tw = ti.w;
                self.th = ti.h;
                if self.mode == 0 && self.have0 == 0 {
                    self.cicp0 = ti.cicp;
                    self.have0 = 1;
                }
                0
            }
            Err(e) => e,
        }
    }

    /// hc_tile: decode the coded image item id and convert it into the target at (dx, dy):
    /// colour (mode 0) or alpha (1)
    fn tile(&self, dst: &mut Dst, id: u32, dx: u32, dy: u32) -> Result<TileInfo, i32> {
        if self.itype(id) == TAG_AV01 {
            return self.av1tile(dst, id, dx, dy);
        }
        let Some(hvcc) = self.bx.prop(id, TAG_HVCC) else { return Err(ERR_HDR) };
        let hl = hvcc.len();
        if hl < 23 {
            return Err(ERR_HDR);
        }
        let mut hv = Hevc::new(0, 0);
        hv.set_cancel(self.dec.cancel);
        // parameter sets of the hvcC arrays
        let mut p = 23usize;
        for _ in 0..hvcc[22] {
            if p + 3 > hl {
                return Err(ERR_HDR);
            }
            let cnt = be16(hvcc, p + 1);
            p += 3;
            for _ in 0..cnt {
                if p + 2 > hl {
                    return Err(ERR_HDR);
                }
                let n = be16(hvcc, p) as usize;
                p += 2;
                if n > hl - p {
                    return Err(ERR_HDR);
                }
                let r = hv.nal(&hvcc[p..p + n]);
                if r != 0 {
                    return Err(r);
                }
                p += n;
            }
        }
        // the length-prefixed NAL units of the item data
        let lsz = (hvcc[21] & 3) as usize + 1;
        let data = self.bx.item(id)?;
        let len = data.len();
        let mut p = 0usize;
        while p < len {
            if p + lsz > len {
                return Err(ERR_TRUNC);
            }
            let mut n = 0usize;
            for _ in 0..lsz {
                n = n << 8 | data[p] as usize;
                p += 1;
            }
            if n > len - p {
                return Err(ERR_TRUNC);
            }
            if n == 0 {
                continue;
            }
            let r = hv.nal(&data[p..p + n]);
            if r != 0 {
                return Err(r);
            }
            p += n;
        }
        drop(data);
        let pic = hv.finish()?;
        let mut yp = YuvPlanes {
            w: pic.w,
            h: pic.h,
            subw: pic.subw,
            subh: pic.subh,
            bits: pic.bd,
            bitsc: pic.bdc,
            ..Default::default()
        };
        // planes at the conformance window
        for c in 0..3 {
            let (sw, sh) = if c == 0 { (1, 1) } else { (pic.subw, pic.subh) };
            if c != 0 && pic.chroma == 0 {
                break;
            }
            let o = (pic.y0 / sh) as usize * pic.pw[c] as usize + (pic.x0 / sw) as usize;
            yp.st[c] = pic.pw[c];
            yp.pl[c] = &pic.planes[c][o..];
        }
        // colour: the primary's nclx, else the tile's, else the VUI
        let cicp = if self.hasnclx != 0 {
            self.nclx
        } else if let Some(c) = self.itemnclx(id) {
            c
        } else {
            let mut c = 0x020202; // unspecified (BT.601)
            if pic.vuicol != 0 {
                c = (pic.prim & 255) | (pic.trc & 255) << 8 | (pic.mc & 255) << 16;
            }
            c | (pic.full & 1) << 24
        };
        yp.cicp = cicp;
        self.put(&yp, dst, dx, dy)?;
        Ok(TileInfo { w: pic.w, h: pic.h, cicp })
    }

    /// hc_av1tile: hc_tile for an av01 item
    fn av1tile(&self, dst: &mut Dst, id: u32, dx: u32, dy: u32) -> Result<TileInfo, i32> {
        let Some(c) = self.bx.prop(id, TAG_AV1C) else { return Err(ERR_HDR) };
        if c.len() < 4 {
            return Err(ERR_HDR);
        }
        let mut av = Av1Dec::new();
        av.set_cancel(self.dec.cancel); // r9-av1
        // configOBUs (normally the sequence header)
        if c.len() > 4 {
            av.obus(&c[4..])?;
        }
        let data = self.bx.item(id)?;
        av.obus(&data)?;
        drop(data);
        let pic = av.finish()?;
        let mut yp = YuvPlanes {
            w: pic.w,
            h: pic.h,
            subw: 1u32 << (pic.subx & 31),
            subh: 1u32 << (pic.suby & 31),
            bits: pic.bd,
            bitsc: pic.bd,
            ..Default::default()
        };
        yp.pl[0] = pic.planes[0];
        yp.st[0] = pic.strides[0];
        if pic.mono == 0 {
            yp.pl[1] = pic.planes[1];
            yp.pl[2] = pic.planes[2];
            yp.st[1] = pic.strides[1];
            yp.st[2] = pic.strides[2];
        }
        // chroma position: unknown -> 1 (centred), vertical -> 0, co-located -> 2
        yp.chloc = match pic.csp {
            1 => 0,
            2 => 2,
            _ => 1,
        };
        // colour: the primary's nclx, else the item's, else the sequence header
        let cicp = if self.hasnclx != 0 {
            self.nclx
        } else if let Some(c) = self.itemnclx(id) {
            c
        } else {
            let mut c = 0x020202;
            if pic.cdp != 0 {
                c = (pic.prim & 255) | (pic.trc & 255) << 8 | (pic.mc & 255) << 16;
            }
            c | (pic.full & 1) << 24
        };
        yp.cicp = cicp;
        self.put(&yp, dst, dx, dy)?;
        Ok(TileInfo { w: pic.w, h: pic.h, cicp })
    }

    /// yuv_to_argb (colour) / yuv_alpha (alpha) of a tile into the target at (dx, dy)
    fn put(&self, yp: &YuvPlanes, dst: &mut Dst, dx: u32, dy: u32) -> Result<(), i32> {
        let alpha = self.mode != 0;
        let r = match dst {
            Dst::Direct(sf) => {
                if alpha {
                    yuv::alpha(yp, sf, dx, dy)
                } else {
                    yuv::to_argb(yp, sf, dx, dy)
                }
            }
            Dst::Rows(rows) => {
                // the clipped tile into a surface of its own, then its rows into the image
                let cols = rows.w.wrapping_sub(dx as i32);
                let nr = rows.h.wrapping_sub(dy as i32);
                let (c, n) = if cols <= 0 || nr <= 0 { (0, 0) } else { ((cols as u32).min(yp.w), (nr as u32).min(yp.h)) };
                let mut t = Surface::new(c as i32, n as i32, FMT_ARGB);
                let r = if alpha { yuv::alpha(yp, &mut t, 0, 0) } else { yuv::to_argb(yp, &mut t, 0, 0) };
                if r == 0 {
                    let ts = t.stride as usize;
                    let (x0, nb) = (dx as usize * 4, c as usize * 4);
                    for y in 0..n as usize {
                        let src = &t.px[y * ts..y * ts + nb];
                        let mut row = rows.rows[dy as usize + y].lock();
                        let d = &mut row[x0..x0 + nb];
                        if alpha {
                            for (dp, sp) in d.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                                dp[3] = sp[3];
                            }
                        } else {
                            d.copy_from_slice(src);
                        }
                    }
                }
                r
            }
        };
        if r != 0 { Err(r) } else { Ok(()) }
    }

    /// hc_itemnclx: CICP of the first colr nclx associated with item id
    fn itemnclx(&self, id: u32) -> Option<u32> {
        let k = self.bx.find(id);
        if k < 0 {
            return None;
        }
        let mut j = 0;
        while let Some((p, _)) = self.bx.assoc(k as u32, j) {
            j += 1;
            if p.ty != TAG_COLR || p.len < 4 {
                continue;
            }
            let pay = self.bx.payload(&p);
            if u32::from_le_bytes([pay[0], pay[1], pay[2], pay[3]]) != TAG_NCLX {
                continue;
            }
            if let Some(c) = nclx(pay) {
                return Some(c);
            }
        }
        None
    }
}
