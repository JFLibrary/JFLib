// img_jbig2 -- JBIG2 decoder (ITU-T T.88 | ISO/IEC 14492): bilevel images as PDF embeds them
// (JBIG2Decode, with its JBIG2Globals) and JBIG2 files
//
//   decode_embedded(data, globals, &Options) -> Bitmap / Error   PDF's JBIG2Decode: page 1
//   decode_file(data, &Options) -> Vec<Bitmap> / Error           a .jb2 / .jbig2 file: every page
//   probe(data) -> bool                                          the file header's id
//   Bitmap { width, height, stride, bits }                       1 = black, MSB first, rows
//                                                                padded to bytes
//   CODEC                                                        img_core's codec (page 1 as an
//                                                                8-bit ARGB surface) for img_all
//
// Modules: mq (the arithmetic decoder, integer decoding), huff (the Huffman tables), mmr (T.6),
// bitmap (images and combination operators), generic (generic and refinement regions), symbol
// (symbol dictionaries, text regions), halftone (pattern dictionaries, halftone regions).
//
// What it decodes: every segment type of a page -- page information (striped pages, pages of
// unknown height growing with their stripes, the default pixel and operator), end of stripe, end
// of page, end of file, tables; symbol dictionaries (arithmetic and Huffman, refinement /
// aggregate coding, collective bitmaps uncompressed or MMR, exports, contexts retained and used
// again), text regions (arithmetic and Huffman, refinement, transposed, every reference corner and
// combination operator), pattern dictionaries and halftone regions (MMR or arithmetic, skip,
// grids at any angle), generic regions (MMR; templates 0 to 3 with adaptive pixels and TPGDON;
// data of unknown length ended by its marker) and refinement regions (templates 0 and 1, TPGRON),
// each intermediate, immediate or lossless; the combination operators OR, AND, XOR, XNOR and
// REPLACE.  The file organisations sequential and random-access.
//
// Not here: colour (JBIG2 Amendment 3: colour palettes and coloured regions are ignored, the page
// stays bilevel), extension segments.  The segments are decoded once, in their order: the global
// ones where they stand, a page's onto the page their association names (a file's pages may
// interleave; PDF's embedded form has one page).  A damaged segment ends its page with what was
// decoded before it and that page's further segments are left out; the global ones and the other
// pages go on.  Limits (`Options::max_pixels`, symbol counts, the work of a stream: its pages' and
// regions' pixels, at most four times `max_pixels` plus 2^22) refuse hostile files before they
// cost memory or time, and symbol dictionaries and text regions stop where their arithmetic data
// has run out (mq.rs).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

mod bitmap;
mod generic;
mod halftone;
mod huff;
mod mmr;
mod mq;
mod symbol;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use bitmap::Bmp;
use gfx::{Surface, FMT_ARGB};
use img_core::meta::surf_alloc;
use img_core::*;
use mq::{Cx, Mq};

/// why a stream could not be decoded
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// no JBIG2 file header (decode_file)
    NotJbig2,
    /// the data ends early
    Truncated,
    /// a segment that breaks the standard
    Corrupt,
    /// valid, but not decoded here
    Unsupported,
    /// larger than the limits allow (or than the memory the allocator gives)
    Limit,
    /// no page in the data
    NoPage,
}

/// what to decode
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// the largest width * height of the page and of any region
    pub max_pixels: u64,
}

impl Default for Options {
    fn default() -> Options {
        Options { max_pixels: IMG_MAXPX_DEF }
    }
}

/// a decoded page: 1 = black, MSB first, `stride` bytes a row
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub bits: Vec<u8>,
}

impl Bitmap {
    fn from(b: &Bmp) -> Bitmap {
        let stride = b.w.div_ceil(8);
        // no pixels (a page 0 wide but 2^32 high is no work either)
        if b.w == 0 || b.h == 0 {
            return Bitmap { width: b.w as u32, height: b.h as u32, stride, bits: Vec::new() };
        }
        let mut bits = vec![0u8; stride * b.h];
        for y in 0..b.h {
            let row = &b.d[y * b.w..(y + 1) * b.w];
            let out = &mut bits[y * stride..(y + 1) * stride];
            for (x, &v) in row.iter().enumerate() {
                if v != 0 {
                    out[x >> 3] |= 0x80 >> (x & 7);
                }
            }
        }
        Bitmap { width: b.w as u32, height: b.h as u32, stride, bits }
    }

    /// the pixel at (x, y): true for black
    pub fn get(&self, x: u32, y: u32) -> bool {
        x < self.width && y < self.height && self.bits[y as usize * self.stride + (x >> 3) as usize] & (0x80 >> (x & 7)) != 0
    }
}

const FILE_ID: [u8; 8] = [0x97, 0x4A, 0x42, 0x32, 0x0D, 0x0A, 0x1A, 0x0A];

/// a JBIG2 file (its header's id)
pub fn probe(d: &[u8]) -> bool {
    d.starts_with(&FILE_ID)
}

// ------------------------------------------------------------------ segments

struct Seg<'a> {
    num: u32,
    ty: u8,
    /// the page association (0: global)
    page: u32,
    refs: Vec<u32>,
    data: &'a [u8],
    /// the header's data length was unknown (7.2.7): the data ends with its row count
    unknown_len: bool,
}

fn be32(d: &[u8], i: usize) -> Result<u32, Error> {
    d.get(i..i + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])).ok_or(Error::Truncated)
}

/// a segment header at *p (7.2); its data length (0xFFFFFFFF: unknown)
fn header<'a>(d: &'a [u8], p: &mut usize) -> Result<(Seg<'a>, u32), Error> {
    let num = be32(d, *p)?;
    let flags = *d.get(*p + 4).ok_or(Error::Truncated)?;
    let rts = *d.get(*p + 5).ok_or(Error::Truncated)?;
    *p += 6;
    let mut count = (rts >> 5) as usize;
    if count == 7 {
        count = (be32(d, *p - 1)? & 0x1FFF_FFFF) as usize;
        *p += 3 + (count + 1).div_ceil(8);
        if count > 1 << 16 {
            return Err(Error::Limit);
        }
    } else if count > 4 {
        return Err(Error::Corrupt);
    }
    let size = if num <= 256 { 1 } else if num <= 65536 { 2 } else { 4 };
    let mut refs = Vec::with_capacity(count);
    for _ in 0..count {
        let b = d.get(*p..*p + size).ok_or(Error::Truncated)?;
        refs.push(b.iter().fold(0u32, |v, &x| (v << 8) | x as u32));
        *p += size;
    }
    // the page association: four bytes or one
    let page = if flags & 0x40 != 0 {
        let v = be32(d, *p)?;
        *p += 4;
        v
    } else {
        let v = *d.get(*p).ok_or(Error::Truncated)? as u32;
        *p += 1;
        v
    };
    let len = be32(d, *p)?;
    *p += 4;
    Ok((Seg { num, ty: flags & 0x3F, page, refs, data: &[], unknown_len: false }, len))
}

/// the length of an immediate generic region's data of unknown length (7.2.7): up to its end
/// marker (0xFF 0xAC after arithmetic data, 0x00 0x00 after MMR data) and the row count after it
fn unknown_length(d: &[u8], start: usize) -> Result<u32, Error> {
    // region segment information (17), the flags byte and the adaptive pixels (7.4.6.1: two bytes
    // each, four of them for template 0, none with MMR; -1, -84 are 0xFF 0xAC as well)
    let flags = *d.get(start + 17).ok_or(Error::Truncated)?;
    let mmr = flags & 1 != 0;
    let marker: [u8; 2] = if mmr { [0x00, 0x00] } else { [0xFF, 0xAC] };
    let at = if mmr { 0 } else if (flags >> 1) & 3 == 0 { 8 } else { 2 };
    let mut i = start + 18 + at;
    while i + 6 <= d.len() {
        if d[i] == marker[0] && d[i + 1] == marker[1] {
            return Ok((i + 6 - start) as u32);
        }
        i += 1;
    }
    Err(Error::Truncated)
}

/// the segments of an embedded stream or a sequential file: header and data one after the other
fn sequential(d: &[u8]) -> Result<Vec<Seg<'_>>, Error> {
    let mut v = Vec::new();
    let mut p = 0usize;
    while p < d.len() {
        let (mut s, mut len) = match header(d, &mut p) {
            Ok(x) => x,
            Err(e) => {
                if v.is_empty() {
                    return Err(e);
                }
                break;
            }
        };
        if len == 0xFFFF_FFFF {
            // the segments before an unreadable one are kept (as after a damaged header)
            let found = if s.ty == 38 || s.ty == 39 { unknown_length(d, p) } else { Err(Error::Corrupt) };
            match found {
                Ok(l) => len = l,
                Err(e) if v.is_empty() => return Err(e),
                Err(_) => break,
            }
            s.unknown_len = true;
        }
        let end = p.saturating_add(len as usize).min(d.len());
        s.data = &d[p..end];
        let ty = s.ty;
        v.push(s);
        p = end;
        if ty == 51 {
            break;
        }
    }
    Ok(v)
}

/// the segments of a random-access file: every header, then the data in the same order
fn random_access(d: &[u8]) -> Result<Vec<Seg<'_>>, Error> {
    let mut heads = Vec::new();
    let mut p = 0usize;
    loop {
        let (s, len) = header(d, &mut p)?;
        let ty = s.ty;
        heads.push((s, len));
        if ty == 51 || p >= d.len() {
            break;
        }
    }
    let mut v = Vec::with_capacity(heads.len());
    for (mut s, len) in heads {
        if len == 0xFFFF_FFFF {
            return Err(Error::Corrupt);
        }
        let end = p.saturating_add(len as usize).min(d.len());
        s.data = &d[p.min(end)..end];
        p = end;
        v.push(s);
    }
    Ok(v)
}

// ------------------------------------------------------------------ decoding

enum Res {
    Symbols(Vec<Rc<Bmp>>, Option<symbol::Stats>),
    Patterns(Vec<Rc<Bmp>>),
    Table(huff::Table),
    Region(Bmp),
}

struct Page {
    /// its place among the pages (the order of their page information segments)
    slot: usize,
    bmp: Bmp,
    default: u8,
    /// the height grows with the stripes
    unknown_height: bool,
}

struct Dec {
    max: u64,
    /// the pixels decoded and drawn so far, and how many a stream may cost
    work: u64,
    limit: u64,
    res: BTreeMap<u32, Res>,
    /// the pages open, by their number
    open: BTreeMap<u32, Page>,
    /// the number of the page opened last
    last: u32,
    /// the pages in the order of their page information segments (None: still open)
    done: Vec<Option<Bitmap>>,
    /// a segment goes to the page its association names (files); else to the page opened last
    /// (PDF's embedded form: one page)
    by_page: bool,
    /// the pages a damaged segment ended: their further segments are left out
    damaged: BTreeSet<u32>,
    /// the pages wanted (PDF and the codec: the first)
    want: usize,
}

/// region segment information (7.4.1): w, h, x, y, operator
fn region_info(d: &[u8]) -> Result<(usize, usize, i64, i64, u8), Error> {
    if d.len() < 17 {
        return Err(Error::Truncated);
    }
    Ok((be32(d, 0)? as usize, be32(d, 4)? as usize, be32(d, 8)? as i64, be32(d, 12)? as i64, d[16] & 7))
}

impl Dec {
    fn new(max: u64, want: usize, by_page: bool) -> Dec {
        let limit = max.saturating_mul(4).saturating_add(1 << 22);
        Dec { max, work: 0, limit, res: BTreeMap::new(), open: BTreeMap::new(), last: 0, done: Vec::new(), by_page, damaged: BTreeSet::new(), want }
    }

    /// the open page a segment draws on: the one its association names; for a global segment (or
    /// any of the embedded form) the page opened last
    fn target(&self, s: &Seg) -> Option<u32> {
        if self.by_page && self.open.contains_key(&s.page) {
            Some(s.page)
        } else if (!self.by_page || s.page == 0) && self.open.contains_key(&self.last) {
            Some(self.last)
        } else {
            None
        }
    }

    /// page `pg` into the results
    fn finish(&mut self, pg: u32) {
        if let Some(p) = self.open.remove(&pg) {
            self.done[p.slot] = Some(Bitmap::from(&p.bmp));
        }
    }

    /// every open page into the results
    fn finish_all(&mut self) {
        for (_, p) in core::mem::take(&mut self.open) {
            self.done[p.slot] = Some(Bitmap::from(&p.bmp));
        }
    }

    /// the pages wanted are decoded
    fn complete(&self) -> bool {
        self.done.len() >= self.want && self.done[..self.want].iter().all(Option::is_some)
    }

    /// the decoded pages in their order
    fn pages(mut self) -> Vec<Bitmap> {
        self.finish_all();
        self.done.into_iter().flatten().collect()
    }

    /// n pixels of work, refused beyond the stream's budget
    fn spend(&mut self, n: u64) -> Result<(), Error> {
        self.work = self.work.saturating_add(n);
        if self.work > self.limit { Err(Error::Limit) } else { Ok(()) }
    }

    /// page `pg`, of unknown height, grown to h rows; the new rows cost their pixels
    fn grow_page(&mut self, pg: u32, h: usize) -> Result<(), Error> {
        let more = match self.open.get(&pg) {
            Some(p) if p.unknown_height && h > p.bmp.h => (p.bmp.w as u64).saturating_mul((h - p.bmp.h) as u64),
            _ => return Ok(()),
        };
        self.spend(more)?;
        let max = self.max;
        if let Some(p) = self.open.get_mut(&pg) {
            p.bmp.grow(h, p.default, max)?;
        }
        Ok(())
    }

    /// a region of segment `s` onto its page
    fn compose(&mut self, s: &Seg, b: &Bmp, x: i64, y: i64, op: u8) -> Result<(), Error> {
        let Some(pg) = self.target(s) else { return Ok(()) };
        self.grow_page(pg, (y + b.h as i64).max(0) as usize)?;
        if let Some(page) = self.open.get_mut(&pg) {
            page.bmp.compose(b, x, y, op);
        }
        Ok(())
    }

    fn symbols_of(&self, s: &Seg) -> Result<Vec<Rc<Bmp>>, Error> {
        let mut v = Vec::new();
        for r in &s.refs {
            if let Some(Res::Symbols(syms, _)) = self.res.get(r) {
                if v.len() + syms.len() > 1 << 21 {
                    return Err(Error::Limit);
                }
                v.extend(syms.iter().cloned());
            }
        }
        Ok(v)
    }

    fn tables_of(&self, s: &Seg) -> Vec<&huff::Table> {
        s.refs.iter().filter_map(|r| if let Some(Res::Table(t)) = self.res.get(r) { Some(t) } else { None }).collect()
    }

    fn segment(&mut self, s: &Seg) -> Result<(), Error> {
        let d = s.data;
        match s.ty {
            0 => {
                let input = self.symbols_of(s)?;
                let tables = self.tables_of(s);
                // the contexts of the last referred dictionary that retained them
                let retained = s.refs.iter().rev().find_map(|r| if let Some(Res::Symbols(_, Some(st))) = self.res.get(r) { Some(st) } else { None });
                let mut work = self.work;
                let r = symbol::dictionary(d, &input, &tables, retained, &mut work, self.limit, self.max);
                self.work = work;
                let (syms, stats) = r?;
                self.res.insert(s.num, Res::Symbols(syms, stats));
            }
            4 | 6 | 7 => {
                let (w, h, x, y, op) = region_info(d)?;
                let b = self.text(s, &d[17..], w, h)?;
                if s.ty == 4 {
                    self.res.insert(s.num, Res::Region(b));
                } else {
                    self.compose(s, &b, x, y, op)?;
                }
            }
            16 => {
                let mut work = self.work;
                let r = halftone::patterns(d, &mut work, self.limit, self.max);
                self.work = work;
                let p = r?;
                self.res.insert(s.num, Res::Patterns(p));
            }
            20 | 22 | 23 => {
                let (w, h, x, y, op) = region_info(d)?;
                let pats = s.refs.iter().find_map(|r| if let Some(Res::Patterns(p)) = self.res.get(r) { Some(p.clone()) } else { None }).ok_or(Error::Corrupt)?;
                let mut work = self.work;
                let r = halftone::region(&d[17..], w, h, &pats, &mut work, self.limit, self.max);
                self.work = work;
                let b = r?;
                if s.ty == 20 {
                    self.res.insert(s.num, Res::Region(b));
                } else {
                    self.compose(s, &b, x, y, op)?;
                }
            }
            36 | 38 | 39 => {
                let (w, mut h, x, y, op) = region_info(d)?;
                let flags = *d.get(17).ok_or(Error::Truncated)?;
                let mmr_on = flags & 1 != 0;
                let template = (flags >> 1) & 3;
                let tpgdon = flags & 8 != 0;
                let mut p = 18usize;
                let mut at = [(0i32, 0i32); 4];
                if !mmr_on {
                    let n = if template == 0 { 4 } else { 1 };
                    for a in at.iter_mut().take(n) {
                        *a = (*d.get(p).ok_or(Error::Truncated)? as i8 as i32, *d.get(p + 1).ok_or(Error::Truncated)? as i8 as i32);
                        p += 2;
                    }
                }
                let mut data = d.get(p..).unwrap_or(&[]);
                // data of unknown length (7.4.6.4): the rows are the row count after the end
                // marker, at most the header's height (0xFFFFFFFF there: not known in advance)
                if s.unknown_len {
                    if data.len() < 6 {
                        return Err(Error::Truncated);
                    }
                    let rows = be32(data, data.len() - 4)? as usize;
                    if rows > h {
                        return Err(Error::Corrupt);
                    }
                    h = rows;
                    data = &data[..data.len() - 6];
                }
                self.spend((w as u64).saturating_mul(h as u64))?;
                let b = if mmr_on {
                    mmr::decode(data, w, h, self.max)?.0
                } else {
                    let mut mq = Mq::new(data);
                    let mut cx = vec![Cx::default(); 1 << 16];
                    generic::generic(&mut mq, &mut cx, &generic::Gb { w, h, template, tpgdon, at, skip: None }, self.max)?
                };
                if s.ty == 36 {
                    self.res.insert(s.num, Res::Region(b));
                } else {
                    self.compose(s, &b, x, y, op)?;
                }
            }
            40 | 42 | 43 => {
                let (w, h, x, y, op) = region_info(d)?;
                let flags = *d.get(17).ok_or(Error::Truncated)?;
                let template = flags & 1;
                let tpgron = flags & 2 != 0;
                let mut p = 18usize;
                let mut at = [(0i8, 0i8); 2];
                if template == 0 {
                    for a in at.iter_mut() {
                        *a = (*d.get(p).ok_or(Error::Truncated)? as i8, *d.get(p + 1).ok_or(Error::Truncated)? as i8);
                        p += 2;
                    }
                }
                // the region's pixels, before a copy of the page as its reference costs them
                self.spend((w as u64).saturating_mul(h as u64))?;
                let max = self.max;
                // the reference: the intermediate region it refers to (borrowed, not copied), else
                // a copy of the page under it
                let page_part;
                let reference: &Bmp = match s.refs.iter().find_map(|r| if let Some(Res::Region(b)) = self.res.get(r) { Some(b) } else { None }) {
                    Some(b) => b,
                    None => {
                        let page = self.target(s).and_then(|pg| self.open.get(&pg)).ok_or(Error::Corrupt)?;
                        page_part = page.bmp.sub(x, y, w, h, max)?;
                        &page_part
                    }
                };
                let mut mq = Mq::new(d.get(p..).unwrap_or(&[]));
                let mut cx = vec![Cx::default(); 1 << 13];
                let g = generic::Gr { w, h, template, reference, dx: 0, dy: 0, tpgron, at };
                let b = generic::refine(&mut mq, &mut cx, &g, max)?;
                if s.ty == 40 {
                    self.res.insert(s.num, Res::Region(b));
                } else {
                    self.compose(s, &b, x, y, op)?;
                }
            }
            48 => {
                let w = be32(d, 0)? as usize;
                let h = be32(d, 4)?;
                let flags = *d.get(16).ok_or(Error::Truncated)?;
                let default = (flags >> 2) & 1;
                let unknown = h == 0xFFFF_FFFF;
                let rows = if unknown { 0 } else { h as usize };
                // a page costs its pixels (64 pages of the largest size are no stream's work)
                self.spend((w as u64).saturating_mul(rows as u64))?;
                let bmp = Bmp::new(w, rows, default, self.max)?;
                self.open.insert(s.page, Page { slot: self.done.len(), bmp, default, unknown_height: unknown });
                self.done.push(None);
                self.last = s.page;
            }
            49 => {
                if let Some(pg) = self.target(s) {
                    self.finish(pg);
                }
            }
            50 => {
                let y = be32(d, 0)? as usize;
                if let Some(pg) = self.target(s) {
                    self.grow_page(pg, y.saturating_add(1))?;
                }
            }
            53 => {
                let t = huff::from_segment(d)?;
                self.res.insert(s.num, Res::Table(t));
            }
            // end of file, profiles, colour palettes, extensions
            _ => {}
        }
        Ok(())
    }

    /// a text region segment's data after the region segment information
    fn text(&mut self, s: &Seg, d: &[u8], w: usize, h: usize) -> Result<Bmp, Error> {
        if d.len() < 2 {
            return Err(Error::Truncated);
        }
        let flags = u16::from_be_bytes([d[0], d[1]]);
        let sbhuff = flags & 1 != 0;
        let refine = flags & 2 != 0;
        let logstrips = (flags >> 2) & 3;
        let refcorner = ((flags >> 4) & 3) as u8;
        let transposed = flags & 0x40 != 0;
        let combop = ((flags >> 7) & 3) as u8;
        let defpixel = ((flags >> 9) & 1) as u8;
        let dsoffset = (((flags >> 10) & 0x1F) as i32) << 27 >> 27;
        let rtemplate = ((flags >> 15) & 1) as u8;
        let mut p = 2usize;
        let hflags = if sbhuff {
            let v = u16::from_be_bytes([*d.get(p).ok_or(Error::Truncated)?, *d.get(p + 1).ok_or(Error::Truncated)?]);
            p += 2;
            v
        } else {
            0
        };
        let mut rat = [(0i8, 0i8); 2];
        if refine && rtemplate == 0 {
            for a in rat.iter_mut() {
                *a = (*d.get(p).ok_or(Error::Truncated)? as i8, *d.get(p + 1).ok_or(Error::Truncated)? as i8);
                p += 2;
            }
        }
        let instances = be32(d, p)?;
        p += 4;
        let syms = self.symbols_of(s)?;
        let nsyms = syms.len() as u64;
        let codelen = if nsyms <= 1 { 0 } else { 64 - (nsyms - 1).leading_zeros() };
        let tp = symbol::Text { w, h, instances, strips: 1 << logstrips, refine, refcorner, transposed, combop, defpixel, dsoffset, rtemplate, rat, syms: &syms, codelen };
        let mut gr = vec![Cx::default(); 1 << 13];
        let mut work = self.work;
        let limit = self.limit;
        let data = &d[p..];
        if sbhuff {
            let custom = self.tables_of(s);
            let mut ci = custom.iter();
            // 7.4.3.1.2: per selector the standard tables, 0 the next custom one
            let r = huff::RESERVED;
            let fs = symbol::pick(hflags & 3, &[6, 7, r, 0], &mut ci)?;
            let ds = symbol::pick((hflags >> 2) & 3, &[8, 9, 10, 0], &mut ci)?;
            let dt = symbol::pick((hflags >> 4) & 3, &[11, 12, 13, 0], &mut ci)?;
            let rdw = symbol::pick((hflags >> 6) & 3, &[14, 15, r, 0], &mut ci)?;
            let rdh = symbol::pick((hflags >> 8) & 3, &[14, 15, r, 0], &mut ci)?;
            let rdx = symbol::pick((hflags >> 10) & 3, &[14, 15, r, 0], &mut ci)?;
            let rdy = symbol::pick((hflags >> 12) & 3, &[14, 15, r, 0], &mut ci)?;
            let rsize = symbol::pick((hflags >> 14) & 1, &[1, 0], &mut ci)?;
            let mut b = huff::Bits::new(data);
            let id = huff::symbol_id_table(&mut b, syms.len())?;
            let th = symbol::TextHuff { fs, ds, dt, rdw, rdh, rdx, rdy, rsize, id: Some(id) };
            let r = symbol::text_region(&mut symbol::Src::Huff { b: &mut b, t: &th }, &tp, &mut gr, &mut work, limit, self.max);
            self.work = work;
            r
        } else {
            let mut mq = Mq::new(data);
            let mut cx = symbol::TextCx::new(codelen);
            let r = symbol::text_region(&mut symbol::Src::Arith { mq: &mut mq, cx: &mut cx }, &tp, &mut gr, &mut work, limit, self.max);
            self.work = work;
            r
        }
    }

    /// the segments in their order, each once: the global ones where they stand, a page's onto
    /// its page.  A damaged segment ends its page with what was decoded before it and that page's
    /// further segments are left out (a global one that fails is left out alone; in the embedded
    /// form any damage ends the page); a stream whose work is spent ends there.  Err when no page
    /// came before the damage.
    fn run(&mut self, segs: &[Seg]) -> Result<(), Error> {
        for s in segs {
            if s.ty == 51 {
                break;
            }
            // a page information segment ends the page of its number (the embedded form: the one
            // open) and lets that page's segments count again
            if s.ty == 48 {
                if self.by_page {
                    self.finish(s.page);
                    self.damaged.remove(&s.page);
                } else {
                    self.finish_all();
                    self.damaged.clear();
                }
            }
            // the pages wanted are done (after an end of page too): what follows can't change them
            if self.complete() {
                break;
            }
            let ended = if self.by_page { self.damaged.contains(&s.page) } else { !self.damaged.is_empty() };
            if s.ty != 48 && s.page != 0 && ended {
                continue;
            }
            if let Err(e) = self.segment(s) {
                if self.done.is_empty() {
                    return Err(e);
                }
                if e == Error::Limit {
                    self.finish_all();
                    break;
                }
                if s.page != 0 || !self.by_page {
                    if let Some(pg) = self.target(s) {
                        self.finish(pg);
                    }
                    self.damaged.insert(s.page);
                }
            }
        }
        Ok(())
    }
}

/// PDF's JBIG2Decode: the embedded segments (and those of JBIG2Globals) -> the first page
pub fn decode_embedded(data: &[u8], globals: Option<&[u8]>, opt: &Options) -> Result<Bitmap, Error> {
    let mut dec = Dec::new(opt.max_pixels, 1, false);
    if let Some(g) = globals {
        if let Ok(gs) = sequential(g) {
            dec.run(&gs)?;
        }
    }
    let segs = sequential(data)?;
    dec.run(&segs)?;
    dec.pages().into_iter().next().ok_or(Error::NoPage)
}

/// a JBIG2 file (sequential or random-access) -> its pages, in the order of their page information
/// segments; one budget of work for the file (four times `Options::max_pixels` plus 2^22): the
/// pages past it are left out
pub fn decode_file(data: &[u8], opt: &Options) -> Result<Vec<Bitmap>, Error> {
    file_pages(data, opt, usize::MAX).map(|(pages, _)| pages)
}

/// the first `want` pages of a file, and how many page information segments it has
fn file_pages(data: &[u8], opt: &Options, want: usize) -> Result<(Vec<Bitmap>, usize), Error> {
    if !probe(data) || data.len() < 9 {
        return Err(Error::NotJbig2);
    }
    let flags = data[8];
    let start = if flags & 2 != 0 { 9 } else { 13 };
    let body = data.get(start..).ok_or(Error::Truncated)?;
    let segs = if flags & 1 != 0 { sequential(body)? } else { random_access(body)? };
    let count = segs.iter().filter(|s| s.ty == 48).count();
    let mut dec = Dec::new(opt.max_pixels, want, true);
    dec.run(&segs)?;
    let pages = dec.pages();
    if pages.is_empty() {
        return Err(Error::NoPage);
    }
    Ok((pages, count))
}

// ------------------------------------------------------------------ the codec of img_core

/// jbig2_codec: the first page of a JBIG2 file as 8-bit ARGB (black on white)
pub static CODEC: Codec = Codec { probe: Some(codec_probe), decode: Some(codec_decode), caps: IFC_DECODE, ..Codec::EMPTY };

fn codec_probe(d: &[u8], _m: &mut ImgMeta) -> bool {
    probe(d)
}

fn codec_decode(d: &[u8], sf: &mut Surface, dec: &ImgDec, m: &mut ImgMeta) -> i32 {
    // only the first page is decoded; `frames` counts them all
    let (pages, count) = match file_pages(d, &Options { max_pixels: dec.max_pixels() }, 1) {
        Ok(p) => p,
        Err(Error::NotJbig2) | Err(Error::Corrupt) | Err(Error::NoPage) => return ERR_HDR,
        Err(Error::Truncated) => return ERR_TRUNC,
        Err(Error::Unsupported) => return ERR_UNSUPPORTED,
        Err(Error::Limit) => return ERR_LIMIT,
    };
    let Some(b) = pages.first() else { return ERR_HDR };
    *sf = match surf_alloc(b.width, b.height, FMT_ARGB, dec) {
        Ok(s) => s,
        Err(e) => return e,
    };
    m.w = b.width;
    m.h = b.height;
    m.bits = 1;
    m.model = IMM_GRAY;
    m.channels = 1;
    m.frames = count.max(1) as u32;
    let stride = sf.stride as usize;
    for y in 0..b.height {
        let row = &mut sf.px[y as usize * stride..][..b.width as usize * 4];
        for x in 0..b.width {
            let v = if b.get(x, y) { 0 } else { 255 };
            row[x as usize * 4..][..4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    0
}
