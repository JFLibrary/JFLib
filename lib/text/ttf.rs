// ttf -- sfnt loader for runtime fonts: TrueType (glyf), OpenType (CFF outlines via cff.rs),
// collections (TTC / OTC), colour tables (colr.rs)                      [src/text/ttf.s]
//
//   face_count(data) -> faces (1 for a plain sfnt, n for a TTC / OTC) / ERR_*
//   TtFont::open(data, face, ol, serial) -> the font context / ERR_*
//   glyph(cp), has(cp), adv(g), kern(g1, g2), kern_table(g1, g2), name(which)
//   bbox(g, ol), path(g, ol, path, gp)       the outline through the outline buffer
//   sfnt_name(table, id) -> UTF-8            best record: Windows Unicode German (0x0407),
//        German variants, English (0x0409), other Windows languages, Unicode platform,
//        Mac Roman English
//
// Errors: ERR_TRUNC (too short / truncated directory), ERR_HDR (not an sfnt, bad head /
// maxp), ERR_UNSUPPORTED (no usable outlines: e.g. CFF2 only), ERR_FAIL (face index out of
// range), ERR_LIMIT (file >= 2 GB).
//
// Tables: head, hhea, hmtx, maxp, loca, glyf (simple + composite: offsets, scale, x/y scale,
// 2x2, point matching, SCALED_COMPONENT_OFFSET; nesting <= 16, <= 4096 components per
// glyph), cmap (formats 0, 4, 6, 12, 13; (3,10) > (0,x) fmt 12 > (3,1) > (0,x) > (3,0)
// symbol > (1,0) Mac Roman), OS/2, post, name, kern (version 0 and Apple version 1, format
// 0), GPOS pair adjustment (lookup type 2 formats 1 and 2, extension type 9; x advance of
// the first glyph; 'kern' feature of the DFLT and latn default language systems).  Variable
// fonts: the default instance.  Every offset is checked against its table and the file;
// malformed glyphs come out empty.  Like GDI / FreeType: glyf outlines are shifted by hmtx
// lsb - xMin; symbol fonts ((3,0) cmap) also map U+0000..U+00FF and the Windows-1252
// characters to U+F000 + code page byte; a CFF FontMatrix that is not exactly 1/unitsPerEm
// scales the outline.

use alloc::boxed::Box;
use alloc::vec::Vec;

use gfx::path::Path;
use rt::fx::sqrt;

use crate::cff::Cff;
use crate::colr;
use crate::font::*;
use crate::outline::{Outline, OLT_ON};

pub const ERR_FAIL: i32 = -1;
pub const ERR_TRUNC: i32 = -1;
pub const ERR_HDR: i32 = -2;
pub const ERR_NOMEM: i32 = -8;
pub const ERR_UNSUPPORTED: i32 = -9;
pub const ERR_IO: i32 = -13;
pub const ERR_LIMIT: i32 = -15;

const TAG_TTCF: u32 = 0x7474_6366;
const TAG_TRUE: u32 = 0x7472_7565;
const TAG_OTTO: u32 = 0x4F54_544F;
const TAG_V1: u32 = 0x0001_0000;
const TAG_DFLT: u32 = 0x4446_4C54;
const TAG_LATN: u32 = 0x6C61_746E;
const TAG_KERNF: u32 = 0x6B65_726E;
pub const FILE_MAX: u64 = 0x7FFF_FFF0;
const GLYF_DEPTH: u32 = 16;
const GLYF_BUDGET: i32 = 4096;
const GPOS_LK_MAX: usize = 64;
const KERN_SUB_MAX: usize = 8;
const GPOS_SUB_MAX: usize = 96;
pub const NAME_CAP: usize = 256;

// table indices (TT_*)
pub const TT_HEAD: usize = 0;
pub const TT_HHEA: usize = 1;
pub const TT_HMTX: usize = 2;
pub const TT_MAXP: usize = 3;
pub const TT_LOCA: usize = 4;
pub const TT_GLYF: usize = 5;
pub const TT_CMAP: usize = 6;
pub const TT_OS2: usize = 7;
pub const TT_POST: usize = 8;
pub const TT_NAME: usize = 9;
pub const TT_KERN: usize = 10;
pub const TT_GPOS: usize = 11;
pub const TT_CFF: usize = 12;
pub const TT_COLR: usize = 13;
pub const TT_CPAL: usize = 14;
pub const TT_CBDT: usize = 15;
pub const TT_CBLC: usize = 16;
pub const TT_FVAR: usize = 17;
pub const TT_COUNT: usize = 20;

pub const TFK_TTF: u32 = 1;
pub const TFK_CFF: u32 = 2;

pub const TFF_LOCA_LONG: u32 = 2;
pub const TFF_SYMBOL: u32 = 4;
pub const TFF_COLR: u32 = 8;
pub const TFF_CBDT: u32 = 16;
pub const TFF_MACCMAP: u32 = 32;

// table tags in TT_* order (big-endian values)
static TT_TAGS: [u32; TT_COUNT] = [
    0x6865_6164, 0x6868_6561, 0x686D_7478, 0x6D61_7870, 0x6C6F_6361, 0x676C_7966, 0x636D_6170,
    0x4F53_2F32, 0x706F_7374, 0x6E61_6D65, 0x6B65_726E, 0x4750_4F53, 0x4346_4620, 0x434F_4C52,
    0x4350_414C, 0x4342_4454, 0x4342_4C43, 0x6676_6172, 0x4346_4632, 0x7668_6561,
];
static NAME_IDS: [u32; 5] = [FN_FAMILY, FN_STYLE, FN_FULL, FN_TYPO_FAMILY, FN_TYPO_STYLE];
const F_INV16384: f32 = 0.000_061_035_156_25;

// Windows-1252 0x80..0x9F -> Unicode (0 = undefined)
static CP1252_HI: [u16; 32] = [
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0,
    0x017D, 0, 0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A,
    0x0153, 0, 0x017E, 0x0178,
];

/// Mac OS Roman 0x80..0xFF -> Unicode
pub static MAC_ROMAN: [u16; 128] = [
    0x00C4, 0x00C5, 0x00C7, 0x00C9, 0x00D1, 0x00D6, 0x00DC, 0x00E1, 0x00E0, 0x00E2, 0x00E4, 0x00E3, 0x00E5,
    0x00E7, 0x00E9, 0x00E8, 0x00EA, 0x00EB, 0x00ED, 0x00EC, 0x00EE, 0x00EF, 0x00F1, 0x00F3, 0x00F2, 0x00F4,
    0x00F6, 0x00F5, 0x00FA, 0x00F9, 0x00FB, 0x00FC, 0x2020, 0x00B0, 0x00A2, 0x00A3, 0x00A7, 0x2022, 0x00B6,
    0x00DF, 0x00AE, 0x00A9, 0x2122, 0x00B4, 0x00A8, 0x2260, 0x00C6, 0x00D8, 0x221E, 0x00B1, 0x2264, 0x2265,
    0x00A5, 0x00B5, 0x2202, 0x2211, 0x220F, 0x03C0, 0x222B, 0x00AA, 0x00BA, 0x03A9, 0x00E6, 0x00F8, 0x00BF,
    0x00A1, 0x00AC, 0x221A, 0x0192, 0x2248, 0x2206, 0x00AB, 0x00BB, 0x2026, 0x00A0, 0x00C0, 0x00C3, 0x00D5,
    0x0152, 0x0153, 0x2013, 0x2014, 0x201C, 0x201D, 0x2018, 0x2019, 0x00F7, 0x25CA, 0x00FF, 0x0178, 0x2044,
    0x20AC, 0x2039, 0x203A, 0xFB01, 0xFB02, 0x2021, 0x00B7, 0x201A, 0x201E, 0x2030, 0x00C2, 0x00CA, 0x00C1,
    0x00CB, 0x00C8, 0x00CD, 0x00CE, 0x00CF, 0x00CC, 0x00D3, 0x00D4, 0xF8FF, 0x00D2, 0x00DA, 0x00DB, 0x00D9,
    0x0131, 0x02C6, 0x02DC, 0x00AF, 0x02D8, 0x02D9, 0x02DA, 0x00B8, 0x02DD, 0x02DB, 0x02C7,
];

// ------------------------------------------------------------------ readers

pub(crate) fn be16(d: &[u8], o: usize) -> u32 {
    u16::from_be_bytes([d[o], d[o + 1]]) as u32
}

pub(crate) fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// bounds-checked big-endian reads of one table (t16 / t32 of the assembly: -1 when the
/// read would cross the table end; offsets are u32 and wrap like the 32-bit registers)
#[derive(Clone, Copy)]
pub struct Tb<'a>(pub &'a [u8]);

impl Tb<'_> {
    pub fn len(&self) -> u32 {
        self.0.len() as u32
    }

    pub fn u8(&self, off: u32) -> i32 {
        if off as usize >= self.0.len() { -1 } else { self.0[off as usize] as i32 }
    }

    /// t16 -> u16 or -1
    pub fn u16(&self, off: u32) -> i32 {
        if off as u64 + 2 > self.0.len() as u64 { -1 } else { be16(self.0, off as usize) as i32 }
    }

    /// t32 -> u32 or -1
    pub fn u32(&self, off: u32) -> i64 {
        if off as u64 + 4 > self.0.len() as u64 { -1 } else { be32(self.0, off as usize) as i64 }
    }
}

fn popcnt8(v: u32) -> u32 {
    (v & 0xFF).count_ones()
}

fn utf8_put(out: &mut Vec<u8>, cp: u32) {
    if cp < 0x80 {
        out.push(cp as u8);
    } else if cp < 0x800 {
        out.push(0xC0 | (cp >> 6) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else if cp < 0x10000 {
        out.push(0xE0 | (cp >> 12) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    } else {
        out.push(0xF0 | (cp >> 18) as u8);
        out.push(0x80 | ((cp >> 12) & 0x3F) as u8);
        out.push(0x80 | ((cp >> 6) & 0x3F) as u8);
        out.push(0x80 | (cp & 0x3F) as u8);
    }
}

/// font_face_count(data) -> faces / ERR_*
pub fn face_count(d: &[u8]) -> i32 {
    if d.len() < 12 {
        return ERR_TRUNC;
    }
    match be32(d, 0) {
        TAG_TTCF => {
            let n = be32(d, 8);
            if n == 0 || n > 65536 {
                return ERR_HDR;
            }
            if n as u64 * 4 + 12 > d.len() as u64 {
                return ERR_TRUNC;
            }
            n as i32
        }
        TAG_V1 | TAG_TRUE | TAG_OTTO => 1,
        _ => ERR_HDR,
    }
}

/// sfnt_name(table, id) -> UTF-8 (empty = not found), at most cap - 1 bytes
pub fn sfnt_name(tb: Tb, id: u32, cap: usize) -> Vec<u8> {
    let mut out = Vec::new();
    if cap == 0 || tb.len() < 6 {
        return out;
    }
    let count = tb.u16(2) as u32;
    let (mut best, mut best_rec) = (0u32, 0u32);
    let mut i = 0u32;
    while i < count {
        let rec = i.wrapping_mul(12).wrapping_add(6);
        let nid = tb.u16(rec.wrapping_add(6));
        if nid < 0 {
            break;
        }
        if nid as u32 == id {
            let pid = tb.u16(rec) as u32;
            let eid = tb.u16(rec.wrapping_add(2)) as u32;
            let lang = tb.u16(rec.wrapping_add(4)) as u32;
            let score = if pid == 3 {
                if eid == 1 || eid == 10 || eid == 0 {
                    if lang == 0x0407 {
                        100
                    } else if lang & 0x3FF == 0x07 {
                        95
                    } else if lang == 0x0409 {
                        90
                    } else {
                        50
                    }
                } else {
                    0
                }
            } else if pid == 0 {
                60
            } else if pid == 1 && eid == 0 && lang == 0 {
                40
            } else {
                0
            };
            if score > best {
                best = score;
                best_rec = rec;
            }
        }
        i += 1;
    }
    if best == 0 {
        return out;
    }
    let r = best_rec;
    let mut s = (tb.u16(4) as u32).wrapping_add(tb.u16(r.wrapping_add(10)) as u32);
    let mut len = tb.u16(r.wrapping_add(8)) as u32;
    let tl = tb.len();
    if tl <= s {
        return out;
    }
    if len > tl - s {
        len = tl - s;
    }
    let end = len + s;
    let mac = tb.u16(r) == 1;
    let d = tb.0;
    while s < end {
        let mut cp;
        if mac {
            cp = d[s as usize] as u32;
            s += 1;
            if cp >= 0x80 {
                cp = MAC_ROMAN[(cp - 0x80) as usize] as u32;
            }
        } else {
            if s + 2 > end {
                break;
            }
            cp = be16(d, s as usize);
            s += 2;
            let hi = cp & 0xFC00;
            if hi == 0xD800 {
                // high surrogate: combine with a following low surrogate
                if s + 2 > end {
                    cp = 0xFFFD;
                } else {
                    let lo = be16(d, s as usize);
                    if lo & 0xFC00 != 0xDC00 {
                        cp = 0xFFFD;
                    } else {
                        s += 2;
                        cp = (((cp & 0x3FF) << 10) | (lo & 0x3FF)) + 0x10000;
                    }
                }
            } else if hi == 0xDC00 {
                cp = 0xFFFD;
            }
        }
        if cp == 0 {
            continue;
        }
        if cp < 0x20 {
            cp = 0x20; // control characters -> blank
        }
        if cap - out.len() < 5 {
            break; // keep room for 4 bytes + NUL
        }
        utf8_put(&mut out, cp);
    }
    out
}

// glyf decoding state shared by the recursion (g_tt_budget, g_tt_xmin, g_tt_rg, g_tt_hasx)
struct GlyfSt {
    budget: i32,
    xmin: i32,
    rg: u32,
    hasx: bool,
}

/// a runtime font context (TrueType or CFF outlines)
pub struct TtFont {
    pub face_index: u32,
    pub data: Vec<u8>,
    pub hdr: Hdr,
    pub fm: Metrics,
    pub serial: u64, // outline buffer owner identity
    pub kind: u32,
    pub flags: u32,
    pub nglyphs: u32,
    pub upem: u32,
    nhm: u32,
    cmap: u32,
    cmapend: u32,
    cmapfmt: u32,
    pub tab: [(u32, u32); TT_COUNT],
    kernsub: Vec<(u32, u32)>,
    gpossub: Vec<(u32, u32)>,
    pub names: [Vec<u8>; 5],
    cff: Option<Box<Cff>>,
    pub cpal: u32,
    pub cpaln: u32,
}

impl TtFont {
    /// the bytes of table idx (empty when missing; clamped to the file at load)
    pub(crate) fn tb(&self, idx: usize) -> Tb<'_> {
        let (o, l) = self.tab[idx];
        Tb(&self.data[o as usize..(o + l) as usize])
    }

    /// ttf_open(data, face): the context, or ERR_*
    pub fn open(data: Vec<u8>, face: u32, ol: &mut Outline, serial: u64) -> Result<Box<TtFont>, i32> {
        let len = data.len() as u64;
        if len < 12 {
            return Err(ERR_TRUNC);
        }
        if len > FILE_MAX {
            return Err(ERR_LIMIT);
        }
        let mut dir = 0u32;
        if be32(&data, 0) == TAG_TTCF {
            let nfonts = be32(&data, 8);
            if face >= nfonts {
                return Err(ERR_FAIL);
            }
            if face as u64 * 4 + 16 > len {
                return Err(ERR_TRUNC);
            }
            dir = be32(&data, face as usize * 4 + 12);
        } else if face != 0 {
            return Err(ERR_FAIL);
        }
        if dir as u64 + 12 > len {
            return Err(ERR_TRUNC);
        }
        let v = be32(&data, dir as usize);
        if v != TAG_V1 && v != TAG_TRUE && v != TAG_OTTO {
            return Err(ERR_HDR);
        }
        let ntab = be16(&data, dir as usize + 4);
        if ((ntab as u64) << 4) + dir as u64 + 12 > len {
            return Err(ERR_TRUNC);
        }
        let mut f = Box::new(TtFont {
            face_index: face,
            data: Vec::new(),
            hdr: Hdr::default(),
            fm: Metrics::default(),
            serial,
            kind: 0,
            flags: 0,
            nglyphs: 0,
            upem: 0,
            nhm: 0,
            cmap: 0,
            cmapend: 0,
            cmapfmt: 0,
            tab: [(0, 0); TT_COUNT],
            kernsub: Vec::new(),
            gpossub: Vec::new(),
            names: Default::default(),
            cff: None,
            cpal: 0,
            cpaln: 0,
        });
        // ---- table records (the first record of a tag wins)
        for i in 0..ntab {
            let r = (dir + 12 + i * 16) as usize;
            let tag = be32(&data, r);
            let off = be32(&data, r + 8);
            let tl = be32(&data, r + 12);
            let Some(k) = TT_TAGS.iter().position(|&t| t == tag) else { continue };
            if f.tab[k].1 != 0 || off as u64 >= len {
                continue;
            }
            let avail = len - off as u64;
            let l = (tl as u64).min(avail) as u32;
            if l == 0 {
                continue;
            }
            f.tab[k] = (off, l);
        }
        // ---- head
        if f.tab[TT_HEAD].1 < 54 {
            return Err(ERR_HDR);
        }
        let h = f.tab[TT_HEAD].0 as usize;
        let upem = be16(&data, h + 18);
        if !(16..=16384).contains(&upem) {
            return Err(ERR_HDR);
        }
        f.upem = upem;
        let mac_style = be16(&data, h + 44);
        if be16(&data, h + 50) as u16 != 0 {
            f.flags |= TFF_LOCA_LONG;
        }
        // ---- maxp
        if f.tab[TT_MAXP].1 < 6 {
            return Err(ERR_HDR);
        }
        let ng = be16(&data, f.tab[TT_MAXP].0 as usize + 4);
        if ng == 0 {
            return Err(ERR_HDR);
        }
        f.nglyphs = ng;
        // ---- hhea / hmtx: numberOfHMetrics clamped to hmtx and numGlyphs
        let mut nhm = 0;
        if f.tab[TT_HHEA].1 >= 36 {
            nhm = be16(&data, f.tab[TT_HHEA].0 as usize + 34);
        }
        nhm = nhm.min(f.tab[TT_HMTX].1 >> 2).min(f.nglyphs);
        f.nhm = nhm;
        f.data = data;
        // ---- outlines
        if f.tab[TT_GLYF].1 != 0 && f.tab[TT_LOCA].1 != 0 {
            f.kind = TFK_TTF;
        } else if f.tab[TT_CFF].1 != 0 {
            f.kind = TFK_CFF;
            let cff = Cff::open(f.tb(TT_CFF), f.upem)?;
            f.cff = Some(Box::new(cff));
        } else if f.tab[TT_CBDT].1 != 0 {
            f.kind = TFK_TTF; // bitmap-only colour fonts have no outlines at all
        } else {
            return Err(ERR_UNSUPPORTED);
        }
        f.cmap_setup();
        f.names_setup();
        f.kern_setup();
        f.gpos_setup();
        colr::open(&mut f);
        f.metrics_setup(mac_style, ol);
        Ok(f)
    }

    // ------------------------------------------------------------------ cmap

    // tt_cmap_score(pid, eid, fmt) -> preference (0 = unusable)
    fn cmap_score(pid: u32, eid: u32, fmt: u32) -> u32 {
        match fmt {
            12 => {
                if pid == 3 {
                    match eid {
                        10 => 10,
                        1 => 8,
                        0 => 5,
                        _ => 0,
                    }
                } else if pid == 0 {
                    9
                } else {
                    0
                }
            }
            4 => {
                if pid == 3 {
                    match eid {
                        1 => 8,
                        0 => 5,
                        _ => 0,
                    }
                } else if pid == 0 {
                    7
                } else {
                    0
                }
            }
            6 | 0 => {
                let s = if fmt == 6 { 4 } else { 3 };
                if pid == 0 || (pid == 3 && eid == 1) {
                    s
                } else if pid == 1 && eid == 0 {
                    2
                } else {
                    0
                }
            }
            13 => {
                if pid == 0 || (pid == 3 && eid == 10) {
                    1
                } else {
                    0
                }
            }
            _ => 0,
        }
    }

    // choose the cmap subtable  [tt_cmap_setup]
    fn cmap_setup(&mut self) {
        let tb = self.tb(TT_CMAP);
        if tb.len() < 4 {
            return;
        }
        let n = tb.u16(2);
        if n < 0 {
            return;
        }
        let (mut best, mut boff, mut bfmt, mut bpid, mut beid) = (0u32, 0u32, 0u32, 0u32, 0u32);
        for i in 0..n as u32 {
            let pid = tb.u16(i * 8 + 4);
            if pid < 0 {
                break;
            }
            let eid = tb.u16(i * 8 + 6) as u32;
            let off = tb.u32(i * 8 + 8);
            if off < 0 {
                break;
            }
            let off = off as u32;
            let fmt = tb.u16(off);
            if fmt < 0 {
                continue;
            }
            let s = Self::cmap_score(pid as u32, eid, fmt as u32);
            if s > best {
                best = s;
                boff = off;
                bfmt = fmt as u32;
                bpid = pid as u32;
                beid = eid;
            }
        }
        if best == 0 {
            return;
        }
        // subtable length (u16 at +2 for the 16-bit formats, u32 at +4 otherwise)
        let l: i64 = if bfmt < 8 { tb.u16(boff.wrapping_add(2)) as i64 } else { tb.u32(boff.wrapping_add(4)) };
        if l < 0 {
            return;
        }
        let left = tb.len().wrapping_sub(boff); // bytes left in the table
        let mut l = (l as u64).min(left as u64) as u32;
        if bfmt == 4 {
            l = left; // the 16-bit length of a big format 4 table can be wrong
        }
        let start = self.tab[TT_CMAP].0.wrapping_add(boff);
        self.cmap = start;
        self.cmapend = l.wrapping_add(start);
        self.cmapfmt = bfmt;
        if bpid == 1 {
            self.flags |= TFF_MACCMAP;
        }
        if bpid == 3 && beid == 0 {
            self.flags |= TFF_SYMBOL;
        }
    }

    // glyph through the chosen subtable (0 = not mapped)  [tt_cmap_raw]
    fn cmap_raw(&self, cp: u32) -> u32 {
        if self.cmap == 0 {
            return 0;
        }
        let tb = Tb(&self.data[self.cmap as usize..self.cmapend as usize]);
        let mut cp = cp;
        // Mac Roman subtables: Unicode -> Mac Roman byte
        if self.flags & TFF_MACCMAP != 0 && cp >= 0x80 {
            match MAC_ROMAN.iter().position(|&u| u as u32 == cp) {
                Some(i) => cp = i as u32 + 0x80,
                None => return 0,
            }
        }
        let g: u32 = match self.cmapfmt {
            0 => {
                if cp >= 256 || cp + 6 >= tb.len() {
                    return 0;
                }
                tb.0[(cp + 6) as usize] as u32
            }
            6 => {
                let first = tb.u16(6);
                if first < 0 || cp < first as u32 {
                    return 0;
                }
                let i = cp - first as u32;
                if i >= tb.u16(8) as u32 {
                    return 0;
                }
                let g = tb.u16(i.wrapping_mul(2).wrapping_add(10));
                if g < 0 {
                    return 0;
                }
                g as u32
            }
            4 => {
                if cp > 0xFFFF {
                    return 0;
                }
                let sx2 = tb.u16(6);
                if sx2 < 0 {
                    return 0;
                }
                let sx2 = sx2 as u32 & !1;
                if sx2 == 0 {
                    return 0;
                }
                let (mut lo, mut hi) = (0u32, sx2 >> 1);
                while lo < hi {
                    let mid = (lo + hi) >> 1;
                    let end = tb.u16(mid * 2 + 14);
                    if end < 0 {
                        return 0;
                    }
                    if end as u32 >= cp {
                        hi = mid;
                    } else {
                        lo = mid + 1;
                    }
                }
                if lo >= sx2 >> 1 {
                    return 0;
                }
                let start = tb.u16(sx2 + lo * 2 + 16);
                if start < 0 || start as u32 > cp {
                    return 0;
                }
                let delta = tb.u16(sx2 * 2 + 16 + lo * 2);
                if delta < 0 {
                    return 0;
                }
                let ro_pos = sx2 * 3 + 16 + lo * 2;
                let ro = tb.u16(ro_pos);
                if ro < 0 {
                    return 0;
                }
                if ro == 0 {
                    cp.wrapping_add(delta as u32) & 0xFFFF
                } else {
                    let p = (ro as u32).wrapping_add(ro_pos).wrapping_add((cp - start as u32).wrapping_mul(2));
                    let g = tb.u16(p);
                    if g <= 0 {
                        return 0;
                    }
                    (g as u32).wrapping_add(delta as u32) & 0xFFFF
                }
            }
            12 | 13 => {
                let n = tb.u32(12);
                if n < 0 || tb.len() < 16 {
                    return 0;
                }
                let n = (n as u32).min((tb.len() - 16) / 12);
                let (mut lo, mut hi) = (0u32, n);
                let mut found = None;
                while lo < hi {
                    let mid = (lo + hi) >> 1;
                    let rec = mid * 12 + 16;
                    let start = tb.u32(rec) as u32;
                    if cp < start {
                        hi = mid;
                        continue;
                    }
                    let end = tb.u32(rec + 4) as u32;
                    if cp > end {
                        lo = mid + 1;
                        continue;
                    }
                    let mut g = tb.u32(rec + 8) as u32;
                    if self.cmapfmt != 13 {
                        g = g.wrapping_add(cp - start);
                    }
                    found = Some(g);
                    break;
                }
                match found {
                    Some(g) if g <= 0xFFFF => g,
                    _ => return 0,
                }
            }
            _ => return 0,
        };
        if g < self.nglyphs { g } else { 0 }
    }

    /// glyph for cp (0 = .notdef).  Symbol fonts ((3,0) cmap) map like GDI: U+0000..U+00FF
    /// and the Windows-1252 characters to U+F000 + their code page byte  [ttf_glyph]
    pub fn glyph(&self, cp: u32) -> u32 {
        let g = self.cmap_raw(cp);
        if g != 0 || self.flags & TFF_SYMBOL == 0 {
            return g;
        }
        let b = if cp <= 0xFF {
            cp
        } else {
            match CP1252_HI.iter().position(|&u| u as u32 == cp) {
                Some(i) => i as u32 + 0x80,
                None => return 0,
            }
        };
        self.cmap_raw(b + 0xF000)
    }

    pub fn has(&self, cp: u32) -> bool {
        self.glyph(cp) != 0
    }

    /// advance (hmtx; the last long metric repeats)  [ttf_adv]
    pub fn adv(&self, g: u32) -> i32 {
        if self.nhm == 0 {
            return 0;
        }
        let i = g.min(self.nhm - 1);
        be16(&self.data, (self.tab[TT_HMTX].0 + i * 4) as usize) as i32
    }

    // ------------------------------------------------------------------ kerning

    // collect horizontal format 0 subtables of the 'kern' table  [tt_kern_setup]
    fn kern_setup(&mut self) {
        let tb = self.tb(TT_KERN);
        if tb.len() < 4 {
            return;
        }
        let kbase = self.tab[TT_KERN].0;
        let mut subs = Vec::new();
        // ks_add(pairs_off, npairs): npairs clamped to the table
        let add = |subs: &mut Vec<(u32, u32)>, off: u32, np: u32| {
            if subs.len() >= KERN_SUB_MAX || tb.len() <= off {
                return;
            }
            let fit = (tb.len() - off) / 6;
            let np = np.min(fit);
            if np == 0 {
                return;
            }
            subs.push((off.wrapping_add(kbase), np));
        };
        if tb.u32(0) == 0x0001_0000 {
            // Apple version 1: u32 nTables, subtables {u32 length, u16 coverage, u16 tuple}
            let n = tb.u32(4);
            if n < 0 {
                self.kernsub = subs;
                return;
            }
            let mut n = (n as u32).min(64);
            let mut off = 8u32;
            while n > 0 {
                n -= 1;
                let l = tb.u32(off);
                if l < 0 {
                    break;
                }
                let cov = tb.u16(off.wrapping_add(4));
                if cov < 0 {
                    break;
                }
                if cov & 0xE000 == 0 && cov & 0xFF == 0 {
                    let np = tb.u16(off.wrapping_add(8));
                    if np < 0 {
                        break;
                    }
                    add(&mut subs, off.wrapping_add(16), np as u32);
                }
                let l = l as u32;
                if l < 8 {
                    break;
                }
                let (o2, carry) = off.overflowing_add(l);
                if carry {
                    break;
                }
                off = o2;
            }
        } else if tb.u16(0) == 0 {
            // Microsoft version 0: u16 nTables, subtables {u16 version, length, coverage}
            let mut n = tb.u16(2) as u32;
            let mut off = 4u32;
            while n > 0 {
                n -= 1;
                let l = tb.u16(off.wrapping_add(2));
                if l < 0 {
                    break;
                }
                let cov = tb.u16(off.wrapping_add(4));
                if cov < 0 {
                    break;
                }
                if cov >> 8 == 0 && cov & 7 == 1 {
                    let np = tb.u16(off.wrapping_add(6));
                    if np < 0 {
                        break;
                    }
                    add(&mut subs, off.wrapping_add(14), np as u32);
                    // length is only 16 bits: recompute it from the pair count
                    let np = tb.u16(off.wrapping_add(6)) as u32;
                    off = off.wrapping_add(np.wrapping_mul(6)).wrapping_add(14);
                    continue;
                }
                if (l as u32) < 6 {
                    break;
                }
                off = off.wrapping_add(l as u32);
            }
        }
        self.kernsub = subs;
    }

    /// sum of the 'kern' format 0 values  [tt_kern_table]
    pub fn kern_table(&self, g1: u32, g2: u32) -> i32 {
        let key = (g1 << 16) | g2;
        let mut sum = 0i32;
        let d = &self.data;
        for &(off, np) in &self.kernsub {
            let (mut lo, mut hi) = (0u32, np);
            while lo < hi {
                let mid = (lo + hi) >> 1;
                let o = off as usize + mid as usize * 6;
                let k = be32(d, o);
                if key < k {
                    hi = mid;
                } else if key > k {
                    lo = mid + 1;
                } else {
                    sum = sum.wrapping_add(be16(d, o + 4) as u16 as i16 as i32);
                    break;
                }
            }
        }
        sum
    }

    /// GPOS 'kern' when present, else 'kern'  [ttf_kern]
    pub fn kern(&self, g1: u32, g2: u32) -> i32 {
        if self.gpossub.is_empty() { self.kern_table(g1, g2) } else { self.kern_gpos(g1, g2) }
    }

    // ------------------------------------------------------------------ GPOS

    // collect the pair adjustment subtables of the 'kern' feature  [tt_gpos_setup]
    fn gpos_setup(&mut self) {
        let tb = self.tb(TT_GPOS);
        if tb.len() < 10 || tb.u16(0) != 1 {
            return;
        }
        let sl = tb.u16(4) as u32;
        let fl = tb.u16(6) as u32;
        let ll = tb.u16(8) as u32;
        let mut lks: Vec<u16> = Vec::new();
        // scripts
        let ns = tb.u16(sl);
        if ns > 0 {
            let mut found = false;
            for i in 0..ns as u32 {
                let rec = i.wrapping_mul(6).wrapping_add(sl).wrapping_add(2);
                let tag = tb.u32(rec) as u32;
                if tag == TAG_DFLT || tag == TAG_LATN {
                    let o = tb.u16(rec.wrapping_add(4));
                    if o < 0 {
                        continue;
                    }
                    gs_langsys(tb, (o as u32).wrapping_add(sl), fl, &mut lks);
                    found = true;
                }
            }
            if !found {
                // neither DFLT nor latn: the first script
                let o = tb.u16(sl.wrapping_add(6));
                if o >= 0 {
                    gs_langsys(tb, (o as u32).wrapping_add(sl), fl, &mut lks);
                }
            }
        }
        lks.sort_unstable();
        // subtables of every lookup (type 2, or type 9 wrapping type 2)
        let lcount = tb.u16(ll);
        if lcount <= 0 {
            return;
        }
        let mut subs = Vec::new();
        'lk: for (ord, &li) in lks.iter().enumerate() {
            if li as i32 >= lcount {
                continue;
            }
            let lo = tb.u16(ll.wrapping_add(li as u32 * 2).wrapping_add(2));
            if lo < 0 {
                continue;
            }
            let lt = (lo as u32).wrapping_add(ll);
            let ty = tb.u16(lt);
            if ty != 2 && ty != 9 {
                continue;
            }
            let nsub = tb.u16(lt.wrapping_add(4));
            if nsub <= 0 {
                continue;
            }
            for s in 0..nsub as u32 {
                let so = tb.u16(lt.wrapping_add(s * 2).wrapping_add(6));
                if so < 0 {
                    continue 'lk;
                }
                let mut sub = (so as u32).wrapping_add(lt);
                if ty == 9 {
                    // extension: u16 format 1, u16 type, u32 offset
                    if tb.u16(sub) != 1 || tb.u16(sub.wrapping_add(2)) != 2 {
                        continue;
                    }
                    let off = tb.u32(sub.wrapping_add(4));
                    if off < 0 {
                        continue;
                    }
                    let s64 = sub as u64 + off as u64;
                    if s64 >= tb.len() as u64 {
                        continue;
                    }
                    sub = s64 as u32;
                }
                if subs.len() >= GPOS_SUB_MAX {
                    break 'lk;
                }
                subs.push((sub, ord as u32));
            }
        }
        self.gpossub = subs;
    }

    // sum over the 'kern' lookups (first applicable subtable of each lookup)  [tt_kern_gpos]
    fn kern_gpos(&self, g1: u32, g2: u32) -> i32 {
        let tb = self.tb(TT_GPOS);
        let mut sum = 0i32;
        let mut done = u32::MAX;
        for &(sub, ord) in &self.gpossub {
            if ord == done {
                continue; // this lookup already applied
            }
            if let Some(v) = gp_pair(tb, sub, g1, g2) {
                sum = sum.wrapping_add(v);
                done = ord;
            }
        }
        sum
    }

    // ------------------------------------------------------------------ names

    // family, style, full, typographic family / style  [tt_names_setup]
    fn names_setup(&mut self) {
        let tb = self.tb(TT_NAME);
        let mut names: [Vec<u8>; 5] = Default::default();
        for (i, &id) in NAME_IDS.iter().enumerate() {
            names[i] = sfnt_name(tb, id, NAME_CAP);
        }
        // typographic names default to the legacy ones
        if names[3].is_empty() {
            names[3] = names[0].clone();
        }
        if names[4].is_empty() {
            names[4] = names[1].clone();
        }
        self.names = names;
    }

    /// font_name of this font ("" for an unknown id)  [ttf_name]
    pub fn name(&self, which: u32) -> &[u8] {
        match which {
            FN_FAMILY => &self.names[0],
            FN_STYLE => &self.names[1],
            FN_FULL => &self.names[2],
            FN_TYPO_FAMILY => &self.names[3],
            FN_TYPO_STYLE => &self.names[4],
            _ => b"",
        }
    }

    // ------------------------------------------------------------------ metrics

    // FONTMETRICS and the SGF1-style header  [tt_metrics_setup]
    fn metrics_setup(&mut self, mac_style: u32, ol: &mut Outline) {
        let mut m = Metrics { upem: self.upem as i32, nglyphs: self.nglyphs as i32, ..Default::default() };
        // hhea (defaults 0.8 / -0.2 em)
        m.asc = (self.upem * 4 / 5) as i32;
        m.desc = m.asc - self.upem as i32;
        let t = self.tb(TT_HHEA);
        if t.len() >= 36 {
            m.asc = t.u16(4) as i16 as i32;
            m.desc = t.u16(6) as i16 as i32;
            m.gap = t.u16(8) as i16 as i32;
            m.advmax = t.u16(10);
        }
        // OS/2 defaults from hhea / head
        m.typo_asc = m.asc;
        m.win_asc = m.asc;
        m.typo_desc = m.desc;
        m.win_desc = m.desc.wrapping_neg();
        m.typo_gap = m.gap;
        m.weight = 400;
        m.width = 5;
        let mut fl = 0i32;
        if mac_style & 1 != 0 {
            fl |= FMF_BOLD;
            m.weight = 700;
        }
        if mac_style & 2 != 0 {
            fl |= FMF_ITALIC;
        }
        let t = self.tb(TT_OS2);
        if t.len() >= 68 {
            fl = 0; // OS/2 decides bold / italic
            let mut w = t.u16(4) as u32;
            if w < 10 {
                w *= 100; // old fonts: 1..9
            }
            if w > 1000 {
                w = 1000;
            }
            if w == 0 {
                w = 400;
            }
            m.weight = w as i32;
            m.width = t.u16(6);
            m.st_thick = t.u16(26) as i16 as i32;
            m.st_pos = t.u16(28) as i16 as i32;
            let s = t.u16(62);
            if s & 1 != 0 {
                fl |= FMF_ITALIC;
            }
            if s & 0x20 != 0 {
                fl |= FMF_BOLD;
            }
            if s & 0x200 != 0 {
                fl |= FMF_OBLIQUE;
            }
            if s & 0x80 != 0 {
                fl |= FMF_TYPO;
            }
            if t.len() >= 78 {
                m.typo_asc = t.u16(68) as i16 as i32;
                m.typo_desc = t.u16(70) as i16 as i32;
                m.typo_gap = t.u16(72) as i16 as i32;
                m.win_asc = t.u16(74);
                m.win_desc = t.u16(76);
            }
            if t.len() >= 90 && t.u16(0) >= 2 {
                m.xh = t.u16(86) as i16 as i32;
                m.caph = t.u16(88) as i16 as i32;
            }
        }
        let t = self.tb(TT_POST);
        if t.len() >= 16 {
            m.ul_pos = t.u16(8) as i16 as i32;
            m.ul_thick = t.u16(10) as i16 as i32;
            if t.u32(12) as u32 != 0 {
                fl |= FMF_MONO;
            }
        }
        if self.kind == TFK_CFF {
            fl |= FMF_CFF;
        }
        if self.tab[TT_FVAR].1 != 0 {
            fl |= FMF_VAR;
        }
        if self.flags & TFF_SYMBOL != 0 {
            fl |= FMF_SYMBOL;
        }
        if self.flags & TFF_COLR != 0 {
            fl |= FMF_COLOR;
        }
        if self.flags & TFF_CBDT != 0 {
            fl |= FMF_COLOR | FMF_BITMAP;
        }
        if !self.kernsub.is_empty() || !self.gpossub.is_empty() {
            fl |= FMF_KERN;
        }
        m.flags = fl;
        self.fm = m;
        // x-height / cap height from 'x' / 'H' when OS/2 has none (not for symbol fonts)
        if fl & FMF_SYMBOL == 0 {
            if m.xh <= 0 {
                m.xh = self.glyph_ymax('x' as u32, ol);
            }
            if m.caph <= 0 {
                m.caph = self.glyph_ymax('H' as u32, ol);
            }
        }
        // underline / strikeout defaults
        let mut d = (self.upem / 14) as i32;
        if d == 0 {
            d = 1;
        }
        if m.ul_thick <= 0 {
            m.ul_thick = d;
        }
        if m.ul_pos >= 0 {
            m.ul_pos = (m.ul_thick + m.ul_thick).wrapping_neg();
        }
        if m.st_thick <= 0 {
            m.st_thick = m.ul_thick;
        }
        if m.st_pos <= 0 {
            let mut c = m.xh;
            if c <= 0 {
                c = (self.upem >> 1) as i32;
            }
            m.st_pos = c.wrapping_add(m.st_thick) >> 1;
        }
        self.fm = m;
        // SGF1-compatible header (i16 fields clamped)
        let mut flags = SGH_RUNTIME;
        if self.flags & TFF_COLR != 0 {
            flags |= SGH_COLR;
        }
        if self.flags & TFF_CBDT != 0 {
            flags |= SGH_CBDT;
        }
        let c16 = |v: i32| v.clamp(-32768, 32767);
        self.hdr = Hdr {
            flags,
            upem: self.upem as i32,
            asc: c16(m.asc),
            desc: c16(m.desc),
            gap: c16(m.gap),
            caph: c16(m.caph),
            xh: if fl & FMF_SYMBOL != 0 { 0 } else { c16(m.xh) }, // no x-height snapping for symbol fonts
            ulpos: c16(m.ul_pos),
            ulthick: c16(m.ul_thick),
            nglyphs: self.nglyphs & 0xFFFF,
            baseid: 0,
        };
    }

    // yMax of the glyph for cp (0 when missing / empty)  [tt_glyph_ymax]
    fn glyph_ymax(&self, cp: u32, ol: &mut Outline) -> i32 {
        let g = self.glyph(cp);
        if g == 0 {
            return 0;
        }
        self.bbox(g, ol).map_or(0, |b| b[3])
    }

    // ------------------------------------------------------------------ outlines

    /// the outline of glyph into the buffer (owner set either way): true = not empty
    /// [ttf_prep]
    pub fn prep(&self, glyph: u32, ol: &mut Outline) -> bool {
        ol.reset();
        let ok = if self.kind == TFK_CFF {
            match &self.cff {
                Some(c) => c.outline(self.tb(TT_CFF), glyph, ol),
                None => false,
            }
        } else {
            let mut st = GlyfSt { budget: GLYF_BUDGET, xmin: 0, rg: 0, hasx: false };
            let ok = self.glyf_decode(glyph, 0, ol, &mut st);
            if ok && st.hasx {
                // the origin is at xMin - lsb (hmtx): shift like the TrueType rasterizers do
                if let Some(lsb) = self.lsb(st.rg) {
                    let d = lsb.wrapping_sub(st.xmin);
                    if d != 0 {
                        ol.translate(0, d as f32, 0.0);
                    }
                }
            }
            ok
        };
        if !ok || ol.err {
            ol.reset();
            ol.owner_set(self.serial, glyph);
            return false;
        }
        ol.owner_set(self.serial, glyph);
        ol.npts() != 0
    }

    // left side bearing (hmtx)  [tt_lsb]
    fn lsb(&self, g: u32) -> Option<i32> {
        let nhm = self.nhm;
        let off = if g < nhm { g * 4 + 2 } else { (nhm * 4).wrapping_add((g - nhm).wrapping_mul(2)) };
        if off.wrapping_add(2) > self.tab[TT_HMTX].1 {
            return None;
        }
        Some(be16(&self.data, (self.tab[TT_HMTX].0 + off) as usize) as u16 as i16 as i32)
    }

    /// control box (font units) and the prepared outline  [ttf_box]
    pub fn bbox(&self, g: u32, ol: &mut Outline) -> Option<[i32; 4]> {
        if self.prep(g, ol) { ol.bbox() } else { None }
    }

    /// append the outline to the path with the gp transform  [ttf_path]
    pub fn path(&self, g: u32, ol: &mut Outline, path: &mut Path, gp: &Gp) {
        if !ol.owner_is(self.serial, g) {
            self.prep(g, ol);
        }
        if ol.npts() != 0 {
            ol.emit(path, gp, None);
        }
    }

    // glyf_decode(glyph, depth): false = malformed (points appended)
    fn glyf_decode(&self, glyph: u32, depth: u32, ol: &mut Outline, st: &mut GlyfSt) -> bool {
        let mut my_rg = glyph; // metrics glyph (USE_MY_METRICS may change it)
        let mut my_xmin = 0i32;
        if depth > GLYF_DEPTH {
            return false;
        }
        st.budget -= 1;
        if st.budget < 0 {
            return false;
        }
        let ok = |st: &mut GlyfSt, xmin: i32, rg: u32| {
            st.xmin = xmin;
            st.rg = rg;
            st.hasx = true;
            true
        };
        if glyph >= self.nglyphs {
            return ok(st, my_xmin, my_rg);
        }
        let d = &self.data;
        // ---- loca
        let (loca, lcl) = self.tab[TT_LOCA];
        let (start, end);
        if self.flags & TFF_LOCA_LONG != 0 {
            if glyph as u64 * 4 + 8 > lcl as u64 {
                return ok(st, my_xmin, my_rg); // beyond loca: empty
            }
            let p = loca as usize + glyph as usize * 4;
            start = be32(d, p);
            end = be32(d, p + 4);
        } else {
            if glyph as u64 * 2 + 4 > lcl as u64 {
                return ok(st, my_xmin, my_rg);
            }
            let p = loca as usize + glyph as usize * 2;
            start = be16(d, p) * 2;
            end = be16(d, p + 2) * 2;
        }
        if end <= start {
            return ok(st, my_xmin, my_rg); // empty glyph
        }
        if end > self.tab[TT_GLYF].1 {
            return false;
        }
        if end - start < 10 {
            return ok(st, my_xmin, my_rg);
        }
        let gl = self.tab[TT_GLYF].0 as usize;
        let p = gl + start as usize;
        let pend = gl + end as usize;
        my_xmin = be16(d, p + 2) as u16 as i16 as i32; // phantom point pp1 = xMin - lsb
        let nc = be16(d, p) as u16 as i16 as i32;
        if nc == 0 {
            return ok(st, my_xmin, my_rg);
        }
        if nc > 0 {
            // ---- simple glyph
            let nc = nc as usize;
            if p + nc * 2 + 12 > pend {
                return false;
            }
            let npts = be16(d, p + nc * 2 + 8) as usize + 1;
            let ins = be16(d, p + nc * 2 + 10) as usize;
            let mut q = p + nc * 2 + 12 + ins;
            if q > pend {
                return false;
            }
            let Some(base) = ol.reserve(npts) else { return false };
            let mut i = 0;
            while i < npts {
                if q >= pend {
                    return false;
                }
                let f = d[q];
                q += 1;
                ol.tags[base + i] = f;
                i += 1;
                if f & 8 == 0 {
                    continue;
                }
                if q >= pend {
                    return false;
                }
                let mut r = d[q];
                q += 1;
                while r != 0 && i < npts {
                    ol.tags[base + i] = f;
                    i += 1;
                    r -= 1;
                }
            }
            let mut x = 0i32;
            for i in 0..npts {
                let f = ol.tags[base + i];
                if f & 2 != 0 {
                    if q >= pend {
                        return false;
                    }
                    let mut v = d[q] as i32;
                    q += 1;
                    if f & 16 == 0 {
                        v = -v;
                    }
                    x = x.wrapping_add(v);
                } else if f & 16 == 0 {
                    if q + 2 > pend {
                        return false;
                    }
                    x = x.wrapping_add(be16(d, q) as u16 as i16 as i32);
                    q += 2;
                }
                ol.pts[base + i].0 = x as f32;
            }
            let mut y = 0i32;
            for i in 0..npts {
                let f = ol.tags[base + i];
                if f & 4 != 0 {
                    if q >= pend {
                        return false;
                    }
                    let mut v = d[q] as i32;
                    q += 1;
                    if f & 32 == 0 {
                        v = -v;
                    }
                    y = y.wrapping_add(v);
                } else if f & 32 == 0 {
                    if q + 2 > pend {
                        return false;
                    }
                    y = y.wrapping_add(be16(d, q) as u16 as i16 as i32);
                    q += 2;
                }
                ol.pts[base + i].1 = y as f32;
            }
            for i in 0..npts {
                ol.tags[base + i] &= OLT_ON;
            }
            let mut prev = -1i32;
            for c in 0..nc {
                let e = be16(d, p + 10 + c * 2) as i32;
                if e as usize >= npts || e < prev {
                    return false;
                }
                if e == prev {
                    continue;
                }
                prev = e;
                if !ol.contour(base as u32 + e as u32) {
                    return false;
                }
            }
            return ok(st, my_xmin, my_rg);
        }
        // ---- composite glyph
        let mut q = p + 10;
        let cbase = ol.npts() as u32; // first point of this composite
        loop {
            if q + 4 > pend {
                return false;
            }
            let flags = be16(d, q);
            let comp = be16(d, q + 2);
            q += 4;
            let (a1, a2): (i32, i32);
            if flags & 1 != 0 {
                if q + 4 > pend {
                    return false;
                }
                let (u, v) = (be16(d, q), be16(d, q + 2));
                if flags & 2 != 0 {
                    a1 = u as u16 as i16 as i32;
                    a2 = v as u16 as i16 as i32;
                } else {
                    a1 = u as i32;
                    a2 = v as i32;
                }
                q += 4;
            } else {
                if q + 2 > pend {
                    return false;
                }
                if flags & 2 != 0 {
                    a1 = d[q] as i8 as i32;
                    a2 = d[q + 1] as i8 as i32;
                } else {
                    a1 = d[q] as i32;
                    a2 = d[q + 1] as i32;
                }
                q += 2;
            }
            // transform
            let f2 = |o: usize| (be16(d, o) as u16 as i16 as i32) as f32 * F_INV16384;
            let (mut ma, mut mb, mut mc, mut md) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            let mut hasmat = true;
            if flags & 0x08 != 0 {
                if q + 2 > pend {
                    return false;
                }
                ma = f2(q);
                md = ma;
                q += 2;
            } else if flags & 0x40 != 0 {
                if q + 4 > pend {
                    return false;
                }
                ma = f2(q);
                md = f2(q + 2);
                q += 4;
            } else if flags & 0x80 != 0 {
                if q + 8 > pend {
                    return false;
                }
                ma = f2(q);
                mb = f2(q + 2);
                mc = f2(q + 4);
                md = f2(q + 6);
                q += 8;
            } else {
                hasmat = false;
            }
            let from = ol.npts();
            if !self.glyf_decode(comp, depth + 1, ol, st) {
                return false;
            }
            if flags & 0x200 != 0 {
                // USE_MY_METRICS: the component's phantom points
                my_xmin = st.xmin;
                my_rg = st.rg;
            }
            if hasmat {
                ol.transform(from, ma, mb, mc, md);
            }
            if flags & 2 != 0 {
                let mut dx = a1 as f32;
                let mut dy = a2 as f32;
                if hasmat && flags & 0x800 != 0 && flags & 0x1000 == 0 {
                    // SCALED_COMPONENT_OFFSET: scale the offset by the column lengths
                    dx = dx * sqrt(ma * ma + mb * mb);
                    dy = dy * sqrt(mc * mc + md * md);
                }
                ol.translate(from, dx, dy);
            } else {
                // point matching: parent point a1 (from this composite's first point) onto
                // child point a2
                let pa = (a1 as u32).wrapping_add(cbase);
                let pb = (a2 as u32).wrapping_add(from as u32);
                if (pa as usize) < from && (pb as usize) < ol.npts() {
                    let (xa, ya) = ol.pts[pa as usize];
                    let (xb, yb) = ol.pts[pb as usize];
                    ol.translate(from, xa - xb, ya - yb);
                }
            }
            if flags & 0x20 == 0 {
                break;
            }
        }
        ok(st, my_xmin, my_rg)
    }
}

// ------------------------------------------------------------------ GPOS helpers

// add the 'kern' lookups of the script's default language system  [gs_langsys]
fn gs_langsys(tb: Tb, script: u32, fl: u32, lks: &mut Vec<u16>) {
    let o = tb.u16(script);
    if o <= 0 {
        return;
    }
    let ls = script.wrapping_add(o as u32);
    let req = tb.u16(ls.wrapping_add(2));
    if req != 0xFFFF {
        if req < 0 {
            return;
        }
        gs_feature(tb, req as u32, fl, lks);
    }
    let n = tb.u16(ls.wrapping_add(4));
    if n <= 0 {
        return;
    }
    for i in 0..n as u32 {
        let fi = tb.u16(ls.wrapping_add(i * 2).wrapping_add(6));
        if fi < 0 {
            return;
        }
        gs_feature(tb, fi as u32, fl, lks);
    }
}

// add the lookups of feature `index` when its tag is 'kern'  [gs_feature]
fn gs_feature(tb: Tb, index: u32, fl: u32, lks: &mut Vec<u16>) {
    if index >= tb.u16(fl) as u32 {
        return;
    }
    let rec = index.wrapping_mul(6).wrapping_add(fl).wrapping_add(2);
    if tb.u32(rec) as u32 != TAG_KERNF {
        return;
    }
    let o = tb.u16(rec.wrapping_add(4));
    if o < 0 {
        return;
    }
    let ft = (o as u32).wrapping_add(fl);
    let n = tb.u16(ft.wrapping_add(2));
    if n <= 0 {
        return;
    }
    for i in 0..n as u32 {
        let li = tb.u16(ft.wrapping_add(i * 2).wrapping_add(4));
        if li < 0 {
            return;
        }
        let li = li as u16;
        if lks.contains(&li) {
            continue;
        }
        if lks.len() >= GPOS_LK_MAX {
            return;
        }
        lks.push(li);
    }
}

// coverage index of glyph or -1  [gp_coverage]
fn gp_coverage(tb: Tb, off: u32, g: u32) -> i32 {
    match tb.u16(off) {
        1 => {
            let n = tb.u16(off.wrapping_add(2));
            if n <= 0 {
                return -1;
            }
            let (mut lo, mut hi) = (0u32, n as u32);
            while lo < hi {
                let mid = (lo + hi) >> 1;
                let v = tb.u16(off.wrapping_add(mid * 2).wrapping_add(4));
                if v < 0 {
                    return -1;
                }
                let v = v as u32;
                if g < v {
                    hi = mid;
                } else if g > v {
                    lo = mid + 1;
                } else {
                    return mid as i32;
                }
            }
            -1
        }
        2 => {
            let n = tb.u16(off.wrapping_add(2));
            if n <= 0 {
                return -1;
            }
            let (mut lo, mut hi) = (0u32, n as u32);
            while lo < hi {
                let mid = (lo + hi) >> 1;
                let r = mid.wrapping_mul(6).wrapping_add(off).wrapping_add(4);
                let s = tb.u16(r);
                if s < 0 {
                    return -1;
                }
                if g < s as u32 {
                    hi = mid;
                    continue;
                }
                let e = tb.u16(r.wrapping_add(2));
                if g > e as u32 {
                    lo = mid + 1;
                    continue;
                }
                let si = tb.u16(r.wrapping_add(4));
                if si < 0 {
                    return -1;
                }
                return (si as u32).wrapping_add(g).wrapping_sub(s as u32) as i32;
            }
            -1
        }
        _ => -1,
    }
}

// class of glyph (0 when not listed)  [gp_class]
fn gp_class(tb: Tb, off: u32, g: u32) -> u32 {
    match tb.u16(off) {
        1 => {
            let s = tb.u16(off.wrapping_add(2));
            if s < 0 || g < s as u32 {
                return 0;
            }
            let i = g - s as u32;
            if i >= tb.u16(off.wrapping_add(4)) as u32 {
                return 0;
            }
            let c = tb.u16(off.wrapping_add(i.wrapping_mul(2)).wrapping_add(6));
            if c < 0 { 0 } else { c as u32 }
        }
        2 => {
            let n = tb.u16(off.wrapping_add(2));
            if n <= 0 {
                return 0;
            }
            let (mut lo, mut hi) = (0u32, n as u32);
            while lo < hi {
                let mid = (lo + hi) >> 1;
                let r = mid.wrapping_mul(6).wrapping_add(off).wrapping_add(4);
                let s = tb.u16(r);
                if s < 0 {
                    return 0;
                }
                if g < s as u32 {
                    hi = mid;
                    continue;
                }
                let e = tb.u16(r.wrapping_add(2));
                if g > e as u32 {
                    lo = mid + 1;
                    continue;
                }
                let c = tb.u16(r.wrapping_add(4));
                return if c < 0 { 0 } else { c as u32 };
            }
            0
        }
        _ => 0,
    }
}

// x advance adjustment of the first glyph: Some(v) when the subtable applies  [gp_pair]
fn gp_pair(tb: Tb, sub: u32, g1: u32, g2: u32) -> Option<i32> {
    let vf1 = tb.u16(sub.wrapping_add(4));
    if vf1 < 0 {
        return None;
    }
    let vf1 = vf1 as u32;
    let vf1_size = popcnt8(vf1) * 2;
    let xadv: i32 = if vf1 & 4 != 0 { (popcnt8(vf1 & 3) * 2) as i32 } else { -1 };
    let vf2 = tb.u16(sub.wrapping_add(6));
    if vf2 < 0 {
        return None;
    }
    let rec = popcnt8(vf2 as u32) * 2 + vf1_size; // size of both value records
    let co = tb.u16(sub.wrapping_add(2));
    if co < 0 {
        return None;
    }
    let cov = gp_coverage(tb, sub.wrapping_add(co as u32), g1);
    if cov < 0 {
        return None;
    }
    let value = match tb.u16(sub) {
        2 => {
            // class pair matrix
            let c1o = tb.u16(sub.wrapping_add(8));
            if c1o < 0 {
                return None;
            }
            let c1 = gp_class(tb, sub.wrapping_add(c1o as u32), g1);
            let c2o = tb.u16(sub.wrapping_add(10));
            if c2o < 0 {
                return None;
            }
            let c2 = gp_class(tb, sub.wrapping_add(c2o as u32), g2);
            if c1 >= tb.u16(sub.wrapping_add(12)) as u32 {
                return None;
            }
            let n2 = tb.u16(sub.wrapping_add(14));
            if n2 <= 0 || c2 >= n2 as u32 {
                return None;
            }
            (n2 as u32).wrapping_mul(c1).wrapping_add(c2).wrapping_mul(rec).wrapping_add(16).wrapping_add(sub)
        }
        1 => {
            // pair sets
            let cov = cov as u32;
            if cov >= tb.u16(sub.wrapping_add(8)) as u32 {
                return None;
            }
            let pso = tb.u16(sub.wrapping_add(cov.wrapping_mul(2)).wrapping_add(10));
            if pso < 0 {
                return None;
            }
            let ps = (pso as u32).wrapping_add(sub);
            let n = tb.u16(ps);
            if n <= 0 {
                return None;
            }
            let size = rec + 2; // PairValueRecord
            let (mut lo, mut hi) = (0u32, n as u32);
            let mut found = None;
            while lo < hi {
                let mid = (lo + hi) >> 1;
                let r = ps.wrapping_add(mid.wrapping_mul(size)).wrapping_add(2);
                let s = tb.u16(r);
                if s < 0 {
                    return None;
                }
                if g2 < s as u32 {
                    hi = mid;
                } else if g2 > s as u32 {
                    lo = mid + 1;
                } else {
                    found = Some(r.wrapping_add(2));
                    break;
                }
            }
            found?
        }
        _ => return None,
    };
    if xadv < 0 {
        return Some(0);
    }
    let v = tb.u16((xadv as u32).wrapping_add(value));
    if v < 0 { Some(0) } else { Some(v as u16 as i16 as i32) }
}
