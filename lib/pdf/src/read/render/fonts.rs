// The glyphs of a font as outlines (ISO 32000-1, 9.6.6 and 9.7.4): the embedded program
// (FontFile: Type 1; FontFile2: TrueType; FontFile3: bare CFF, CID-keyed CFF or OpenType), else
// a substitute from the FontSource (the 14 standard fonts by their metric-compatible
// families, others by the descriptor's flags).  Which glyph a code draws:
//   - Type 1 and CFF: the code's glyph name (the /Encoding with its /Differences, else the
//     program's own encoding), through the CharStrings or the CFF charset
//   - TrueType: a non-symbolic font through the names' Unicode values and the (3,1) cmap, or
//     the Mac Roman code and (1,0); a symbolic one through (3,0) at F000 + code (also code,
//     F100, F200), then (1,0); without a cmap the code is the glyph
//   - Type 0: CIDToGIDMap (Identity by default) for CIDFontType2, the charset of a CID-keyed
//     CFF for CIDFontType0
//   - a substitute: the glyph name in its CFF charset, else its Unicode in its cmap; for a
//     narrower font the outline is scaled to the width of 'm' (as Poppler does)
// Outlines are cached per font and glyph.

use super::super::font::{glyph_unicode, Font};
use super::super::object::{Dict, Object, Stream};
use super::super::tables;
use super::super::Document;
use super::type1::Type1;
use super::{FontQuery, M};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;
use text::cff::Cff;
use text::outline::Outline;
use text::ttf::{Tb, TtFont, TFK_CFF, TT_CFF, TT_CMAP};

/// a name of tables::GLYPH_NAMES
pub(super) fn glyph_name(i: usize) -> &'static [u8] {
    let s = tables::GLYPH_START.get(i).copied().unwrap_or(0) as usize;
    let e = tables::GLYPH_START.get(i + 1).copied().unwrap_or(0) as usize;
    tables::GLYPH_NAMES.get(s..e).unwrap_or(b"")
}

fn enc_names(t: &[u16; 256]) -> Vec<Option<Vec<u8>>> {
    t.iter().map(|&i| (i != 0xFFFF).then(|| glyph_name(i as usize).to_vec())).collect()
}

/// an outline in glyph units
pub(super) struct Glyph {
    pub pts: Vec<(f32, f32)>,
    pub tags: Vec<u8>,
    pub ends: Vec<u32>,
}

impl Glyph {
    fn of(ol: &Outline) -> Glyph {
        Glyph { pts: ol.pts.clone(), tags: ol.tags.clone(), ends: ol.ends.clone() }
    }
}

// ---------------------------------------------------------------- CFF: names and encodings

fn be(d: &[u8], at: usize, n: usize) -> Option<usize> {
    let b = d.get(at..at.checked_add(n)?)?;
    Some(b.iter().fold(0usize, |v, &x| v << 8 | x as usize))
}

/// an INDEX: (count, where its offsets start, offset size, where its data starts - 1)
fn index(d: &[u8], at: usize) -> Option<(usize, usize, usize, usize, usize)> {
    let count = be(d, at, 2)?;
    if count == 0 {
        return Some((0, at, 0, at, at + 2));
    }
    let osz = *d.get(at + 2)? as usize;
    if !(1..=4).contains(&osz) {
        return None;
    }
    let offs = at + 3;
    let data = offs + (count + 1) * osz - 1;
    let last = be(d, offs + count * osz, osz)?;
    Some((count, offs, osz, data, data + last))
}

fn index_item(d: &[u8], ix: (usize, usize, usize, usize, usize), i: usize) -> Option<&[u8]> {
    let (count, offs, osz, data, _) = ix;
    if i >= count {
        return None;
    }
    let a = be(d, offs + i * osz, osz)?;
    let b = be(d, offs + (i + 1) * osz, osz)?;
    d.get(data + a..data + b)
}

/// a DICT: operator -> operands
fn dict(d: &[u8]) -> BTreeMap<u16, Vec<f64>> {
    let mut out = BTreeMap::new();
    let mut ops: Vec<f64> = Vec::new();
    let mut p = 0usize;
    while p < d.len() {
        let b = d[p];
        p += 1;
        match b {
            0..=21 => {
                let op = if b == 12 {
                    let e = d.get(p).copied().unwrap_or(0) as u16;
                    p += 1;
                    1200 + e
                } else {
                    b as u16
                };
                out.insert(op, core::mem::take(&mut ops));
            }
            28 => {
                let v = be(d, p, 2).unwrap_or(0) as u16 as i16 as f64;
                p += 2;
                ops.push(v);
            }
            29 => {
                let v = be(d, p, 4).unwrap_or(0) as u32 as i32 as f64;
                p += 4;
                ops.push(v);
            }
            30 => {
                // a real: nibbles until 0xf
                let mut s = String::new();
                'n: while p < d.len() {
                    let byte = d[p];
                    p += 1;
                    for nib in [byte >> 4, byte & 15] {
                        match nib {
                            0..=9 => s.push((b'0' + nib) as char),
                            0xa => s.push('.'),
                            0xb => s.push('E'),
                            0xc => s.push_str("E-"),
                            0xe => s.push('-'),
                            0xf => break 'n,
                            _ => {}
                        }
                    }
                }
                ops.push(s.parse::<f64>().unwrap_or(0.0));
            }
            32..=246 => ops.push(b as f64 - 139.0),
            247..=250 => {
                ops.push((b as f64 - 247.0) * 256.0 + d.get(p).copied().unwrap_or(0) as f64 + 108.0);
                p += 1;
            }
            251..=254 => {
                ops.push(-(b as f64 - 251.0) * 256.0 - d.get(p).copied().unwrap_or(0) as f64 - 108.0);
                p += 1;
            }
            _ => {}
        }
        if ops.len() > 48 {
            ops.clear();
        }
    }
    out
}

/// what drawing needs of a CFF beyond its outlines: the glyph of each name or CID, and its own
/// encoding
#[derive(Default)]
pub(super) struct CffNames {
    by_name: BTreeMap<Vec<u8>, u32>,
    /// CID-keyed: CID -> glyph
    by_cid: BTreeMap<u32, u32>,
    pub cid_keyed: bool,
    /// the built-in encoding as names (None: StandardEncoding)
    pub encoding: Option<Vec<Option<Vec<u8>>>>,
}

impl CffNames {
    pub fn parse(d: &[u8]) -> Option<CffNames> {
        let hdr = *d.get(2)? as usize;
        let names = index(d, hdr)?;
        let top = index(d, names.4)?;
        let strings = index(d, top.4)?;
        let td = dict(index_item(d, top, 0)?);
        let cs_off = *td.get(&17)?.first()? as usize;
        let nglyphs = index(d, cs_off)?.0.min(1 << 16);
        let cid_keyed = td.contains_key(&1230);
        let sid_name = |sid: usize| -> Option<Vec<u8>> {
            if sid < 391 {
                Some(glyph_name(tables::CFF_STRINGS[sid] as usize).to_vec())
            } else {
                index_item(d, strings, sid - 391).map(<[u8]>::to_vec)
            }
        };
        // charset: glyph -> SID (or CID)
        let mut sids = vec![0usize; nglyphs];
        let cso = td.get(&15).and_then(|v| v.first().copied()).unwrap_or(0.0) as usize;
        match cso {
            0 => {
                for (g, s) in sids.iter_mut().enumerate() {
                    *s = if g <= 228 { g } else { 0 };
                }
            }
            1 | 2 => {} // the expert charsets: names unknown here
            _ => {
                let fmt = *d.get(cso)?;
                let mut g = 1usize;
                let mut p = cso + 1;
                match fmt {
                    0 => {
                        while g < nglyphs {
                            sids[g] = be(d, p, 2)?;
                            p += 2;
                            g += 1;
                        }
                    }
                    1 | 2 => {
                        let nl = if fmt == 1 { 1 } else { 2 };
                        while g < nglyphs {
                            let first = be(d, p, 2)?;
                            let left = be(d, p + 2, nl)?;
                            p += 2 + nl;
                            for k in 0..=left {
                                if g >= nglyphs {
                                    break;
                                }
                                sids[g] = first + k;
                                g += 1;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut out = CffNames { cid_keyed, ..Default::default() };
        for (g, &s) in sids.iter().enumerate() {
            if cid_keyed {
                out.by_cid.entry(s as u32).or_insert(g as u32);
            } else if let Some(n) = sid_name(s) {
                out.by_name.entry(n).or_insert(g as u32);
            }
        }
        // the built-in encoding: code -> glyph -> name
        if !cid_keyed {
            let eo = td.get(&16).and_then(|v| v.first().copied()).unwrap_or(0.0) as usize;
            if eo > 1 {
                let mut enc: Vec<Option<Vec<u8>>> = vec![None; 256];
                let gname = |g: usize| sids.get(g).and_then(|&s| sid_name(s));
                if let Some(&fmt) = d.get(eo) {
                    let mut p = eo + 1;
                    match fmt & 0x7F {
                        0 => {
                            let n = *d.get(p)? as usize;
                            p += 1;
                            for g in 1..=n {
                                let c = *d.get(p)? as usize;
                                p += 1;
                                enc[c] = gname(g);
                            }
                        }
                        1 => {
                            let n = *d.get(p)? as usize;
                            p += 1;
                            let mut g = 1;
                            for _ in 0..n {
                                let first = *d.get(p)? as usize;
                                let left = *d.get(p + 1)? as usize;
                                p += 2;
                                for k in 0..=left {
                                    if first + k < 256 {
                                        enc[first + k] = gname(g);
                                    }
                                    g += 1;
                                }
                            }
                        }
                        _ => {}
                    }
                    if fmt & 0x80 != 0 {
                        let n = *d.get(p)? as usize;
                        p += 1;
                        for _ in 0..n {
                            let c = *d.get(p)? as usize;
                            let s = be(d, p + 1, 2)?;
                            p += 3;
                            enc[c] = sid_name(s);
                        }
                    }
                }
                out.encoding = Some(enc);
            } else if eo == 1 {
                out.encoding = Some(enc_names(&tables::NAMES_MACEXPERT));
            }
        }
        Some(out)
    }

    pub fn glyph(&self, name: &[u8]) -> Option<u32> {
        self.by_name.get(name).copied()
    }
}

// ---------------------------------------------------------------- sfnt cmaps

/// the subtables of an sfnt's cmap: (platform, encoding) -> offset in the file
fn cmaps(f: &TtFont) -> Vec<((u16, u16), usize)> {
    let (off, len) = f.tab[TT_CMAP];
    let (off, len) = (off as usize, len as usize);
    let d = &f.data;
    let mut out = Vec::new();
    if len < 4 {
        return out;
    }
    let n = be(d, off + 2, 2).unwrap_or(0).min(64);
    for i in 0..n {
        let r = off + 4 + i * 8;
        let (Some(p), Some(e), Some(o)) = (be(d, r, 2), be(d, r + 2, 2), be(d, r + 4, 4)) else { break };
        if o < len {
            out.push(((p as u16, e as u16), off + o));
        }
    }
    out
}

/// a code's glyph in one cmap subtable (formats 0, 2 for one byte, 4, 6, 10, 12, 13)
fn cmap_get(d: &[u8], at: usize, c: u32) -> u32 {
    let Some(fmt) = be(d, at, 2) else { return 0 };
    let g = |o: usize| be(d, o, 2).unwrap_or(0) as u32;
    match fmt {
        0 => {
            if c < 256 {
                d.get(at + 6 + c as usize).copied().unwrap_or(0) as u32
            } else {
                0
            }
        }
        4 => {
            if c > 0xFFFF {
                return 0;
            }
            let segx2 = g(at + 6) as usize;
            let ends = at + 14;
            let starts = ends + segx2 + 2;
            let deltas = starts + segx2;
            let ranges = deltas + segx2;
            let (mut lo, mut hi) = (0usize, segx2 / 2);
            while lo < hi {
                let mid = (lo + hi) / 2;
                if g(ends + mid * 2) < c {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            if lo >= segx2 / 2 {
                return 0;
            }
            let s = g(starts + lo * 2);
            if c < s {
                return 0;
            }
            let delta = g(deltas + lo * 2);
            let ro = g(ranges + lo * 2) as usize;
            if ro == 0 {
                return (c + delta) & 0xFFFF;
            }
            let p = ranges + lo * 2 + ro + 2 * (c - s) as usize;
            let v = g(p);
            if v == 0 { 0 } else { (v + delta) & 0xFFFF }
        }
        6 => {
            let first = g(at + 6);
            let n = g(at + 8);
            if c >= first && c < first + n { g(at + 10 + 2 * (c - first) as usize) } else { 0 }
        }
        12 | 13 => {
            let n = be(d, at + 12, 4).unwrap_or(0).min(1 << 20);
            let (mut lo, mut hi) = (0usize, n);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let r = at + 16 + mid * 12;
                let (s, e) = (be(d, r, 4).unwrap_or(0) as u32, be(d, r + 4, 4).unwrap_or(0) as u32);
                if e < c {
                    lo = mid + 1;
                } else if s > c {
                    hi = mid;
                } else {
                    let base = be(d, r + 8, 4).unwrap_or(0) as u32;
                    return if fmt == 12 { base.saturating_add(c - s) } else { base };
                }
            }
            0
        }
        _ => 0,
    }
}

// ---------------------------------------------------------------- programs

pub(super) enum Prog {
    Sfnt { f: Box<TtFont>, cmaps: Vec<((u16, u16), usize)>, cff: Option<CffNames> },
    Cff { data: Vec<u8>, cff: Cff, names: CffNames },
    T1(Type1),
}

impl Prog {
    /// an embedded or substitute program from its bytes; kind: 1 Type 1, 2 TrueType or OpenType,
    /// 3 bare CFF
    pub fn open(data: Vec<u8>, face: u32, kind: u8) -> Option<Prog> {
        let mut ol = Outline::default();
        match kind {
            1 => {
                // a Type 1 program, or a CFF / sfnt under the wrong key
                if let Some(t) = Type1::parse(&data) {
                    return Some(Prog::T1(t));
                }
                Prog::open(data, face, 2)
            }
            3 => {
                if data.len() >= 4 && data[0] == 1 {
                    let cff = Cff::open(Tb(&data), 1000).ok()?;
                    let names = CffNames::parse(&data)?;
                    return Some(Prog::Cff { data, cff, names });
                }
                Prog::open(data, face, 2)
            }
            _ => {
                if data.len() >= 4 && data[0] == 1 && data[1] == 0 && data[2] == 4 {
                    return Prog::open(data, face, 3);
                }
                if data.starts_with(b"%!") || data.first() == Some(&0x80) {
                    return Type1::parse(&data).map(Prog::T1);
                }
                let f = TtFont::open(data, face, &mut ol, 0).ok()?;
                let cmaps = cmaps(&f);
                let cff = if f.kind == TFK_CFF {
                    let (o, l) = f.tab[TT_CFF];
                    f.data.get(o as usize..(o as usize).saturating_add(l as usize)).and_then(CffNames::parse)
                } else {
                    None
                };
                Some(Prog::Sfnt { f, cmaps, cff })
            }
        }
    }

    /// glyph units -> text space
    pub fn matrix(&self) -> M {
        match self {
            Prog::Sfnt { f, .. } => {
                let s = 1.0 / f.upem.max(1) as f64;
                [s, 0.0, 0.0, s, 0.0, 0.0]
            }
            Prog::Cff { .. } => [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
            Prog::T1(t) => t.matrix,
        }
    }

    /// the built-in encoding as names (None: none, or StandardEncoding)
    fn builtin(&self) -> Option<Vec<Option<Vec<u8>>>> {
        match self {
            Prog::T1(t) => t.encoding.clone(),
            Prog::Cff { names, .. } => names.encoding.clone(),
            Prog::Sfnt { cff: Some(c), .. } => c.encoding.clone(),
            _ => None,
        }
    }

    fn by_name(&self, name: &[u8]) -> Option<u32> {
        match self {
            Prog::Cff { names, .. } | Prog::Sfnt { cff: Some(names), .. } => names.glyph(name),
            _ => None,
        }
    }

    fn cmap(&self, pe: (u16, u16), c: u32) -> u32 {
        let Prog::Sfnt { f, cmaps, .. } = self else { return 0 };
        cmaps.iter().filter(|e| e.0 == pe).map(|e| cmap_get(&f.data, e.1, c)).find(|&g| g != 0).unwrap_or(0)
    }

    fn has_cmap(&self, pe: (u16, u16)) -> bool {
        matches!(self, Prog::Sfnt { cmaps, .. } if cmaps.iter().any(|e| e.0 == pe))
    }

    fn unicode(&self, u: u32) -> u32 {
        match self {
            Prog::Sfnt { f, .. } => {
                let g = self.cmap((3, 10), u);
                if g != 0 {
                    return g;
                }
                let g = self.cmap((3, 1), u);
                if g != 0 {
                    return g;
                }
                let g = [(0, 3), (0, 4), (0, 1), (0, 0), (0, 6)].iter().map(|&pe| self.cmap(pe, u)).find(|&g| g != 0).unwrap_or(0);
                if g != 0 {
                    return g;
                }
                f.glyph(u)
            }
            _ => 0,
        }
    }

    fn glyphs(&self) -> u32 {
        match self {
            Prog::Sfnt { f, .. } => f.nglyphs,
            Prog::Cff { names, .. } => names.by_name.len().max(names.by_cid.len()) as u32,
            Prog::T1(_) => 0,
        }
    }

    /// the outline of glyph g (Type 1: by name) in glyph units
    fn outline(&self, g: u32, name: Option<&[u8]>, ol: &mut Outline) -> bool {
        match self {
            Prog::Sfnt { f, .. } => {
                if f.kind == TFK_CFF {
                    if let (Some(c), Prog::Sfnt { cff: Some(names), .. }) = (f.cff(), self) {
                        let (o, l) = f.tab[TT_CFF];
                        let t = Tb(f.data.get(o as usize..(o as usize).saturating_add(l as usize)).unwrap_or(&[]));
                        return cff_seac(c, t, names, g, ol);
                    }
                }
                f.prep(g, ol)
            }
            Prog::Cff { data, cff, names } => cff_seac(cff, Tb(data), names, g, ol),
            Prog::T1(t) => name.is_some_and(|n| t.outline(n, ol)),
        }
    }
}

/// a CFF glyph with a seac accent put together from its base and accent
fn cff_seac(cff: &Cff, t: Tb, names: &CffNames, g: u32, ol: &mut Outline) -> bool {
    ol.reset();
    let (ok, seac) = cff.outline_seac(t, g, ol);
    let Some([adx, ady, bc, ac]) = seac else { return ok };
    let name = |c: f32| -> Option<u32> {
        let i = *tables::NAMES_STANDARD.get(c as usize)?;
        if i == 0xFFFF {
            return None;
        }
        names.glyph(glyph_name(i as usize))
    };
    let (Some(bg), Some(ag)) = (name(bc), name(ac)) else { return ok };
    let mut base = Outline::default();
    let mut acc = Outline::default();
    let _ = cff.outline_seac(t, bg, &mut base);
    let _ = cff.outline_seac(t, ag, &mut acc);
    // the offset goes through the FontMatrix like the outlines did
    let (dx, dy) = match cff.matrix() {
        Some(m) => (m[0] * adx + m[2] * ady, m[1] * adx + m[3] * ady),
        None => (adx, ady),
    };
    ol.reset();
    for (src, ox, oy) in [(&base, 0.0, 0.0), (&acc, dx, dy)] {
        let mut start = 0usize;
        for &e in &src.ends {
            let e = e as usize;
            for k in start..=e.min(src.pts.len().saturating_sub(1)) {
                ol.point(src.pts[k].0 + ox, src.pts[k].1 + oy, src.tags[k]);
            }
            ol.close();
            start = e + 1;
        }
    }
    true
}

// ---------------------------------------------------------------- fonts

pub(super) struct Type3 {
    pub matrix: M,
    pub procs: Vec<Option<Stream>>,
    pub res: Option<Dict>,
}

pub(super) struct GFont {
    pub base: Rc<Font>,
    prog: Option<Rc<Prog>>,
    /// simple fonts: each code's glyph (u32::MAX: none) and name
    gids: Vec<u32>,
    names: Vec<Option<Vec<u8>>>,
    /// composite fonts: CID -> glyph (None: the CID is the glyph)
    cid_gid: Option<Vec<u16>>,
    composite: bool,
    embedded: bool,
    pub matrix: M,
    pub type3: Option<Type3>,
    cache: RefCell<BTreeMap<u32, Option<Rc<Glyph>>>>,
    /// a substitute's own advances, for a font without /Widths whose widths the reader does
    /// not know (Symbol, ZapfDingbats)
    sub_widths: Option<Vec<f64>>,
}

fn strip_subset(name: &[u8]) -> String {
    let s = String::from_utf8_lossy(name).into_owned();
    match s.find('+') {
        Some(6) if s[..6].bytes().all(|c| c.is_ascii_uppercase()) => s[7..].into(),
        _ => s,
    }
}

impl GFont {
    pub fn load(doc: &Document, d: &Dict, subst: &mut dyn FnMut(&FontQuery) -> Option<Rc<Prog>>) -> GFont {
        let base = Rc::new(Font::load(doc, d));
        let sub = d.name("Subtype").unwrap_or(b"Type1").to_vec();
        let composite = sub == b"Type0";
        let desc = if composite {
            match doc.key(d, "DescendantFonts") {
                Object::Array(a) => a.first().map(|o| doc.dict(o)).unwrap_or_default(),
                _ => Dict::new(),
            }
        } else {
            d.clone()
        };
        let fd = doc.dict(&desc.get("FontDescriptor").cloned().unwrap_or_default());
        let mut g = GFont {
            base,
            prog: None,
            gids: Vec::new(),
            names: Vec::new(),
            cid_gid: None,
            composite,
            embedded: false,
            matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0],
            type3: None,
            cache: RefCell::new(BTreeMap::new()),
            sub_widths: None,
        };
        if sub == b"Type3" {
            g.type3 = Some(GFont::type3(doc, d));
            g.names = GFont::code_names(doc, d);
            return g;
        }
        // the embedded program
        let prog = [("FontFile", 1u8), ("FontFile2", 2), ("FontFile3", 3)].iter().find_map(|&(k, kind)| {
            let Object::Stream(s) = doc.key(&fd, k) else { return None };
            let data = doc.decode(&s).ok()?;
            let kind = match (k, s.dict.name("Subtype")) {
                ("FontFile3", Some(b"OpenType")) => 2,
                _ => kind,
            };
            Prog::open(data, 0, kind).map(Rc::new)
        });
        let flags = doc.key(&fd, "Flags").as_int().unwrap_or(0);
        let symbolic = flags & 4 != 0;
        let bname = strip_subset(desc.name("BaseFont").or_else(|| d.name("BaseFont")).unwrap_or(b""));
        let embedded = prog.is_some();
        let prog = prog.or_else(|| {
            let lower = bname.to_ascii_lowercase();
            let q = FontQuery {
                name: &bname,
                family: None,
                bold: flags & (1 << 18) != 0 || lower.contains("bold") || lower.contains("black") || lower.contains("heavy") || doc.key(&fd, "FontWeight").as_f64().is_some_and(|w| w >= 600.0),
                italic: flags & 64 != 0 || lower.contains("italic") || lower.contains("oblique"),
                serif: flags & 2 != 0,
                fixed: flags & 1 != 0,
                symbolic: symbolic && flags & 32 == 0,
                text: !composite,
            };
            subst(&q)
        });
        let Some(prog) = prog else { return g };
        g.embedded = embedded;
        g.matrix = prog.matrix();
        if composite {
            if let Object::Stream(s) = doc.key(&desc, "CIDToGIDMap") {
                if let Ok(m) = doc.decode(&s) {
                    g.cid_gid = Some(m.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect());
                }
            }
            g.prog = Some(prog);
            return g;
        }
        let builtin = if embedded { prog.builtin() } else { None };
        let std_name = |n: &str| -> Option<&'static [u16; 256]> {
            let n = n.to_ascii_lowercase();
            if n.contains("symbol") {
                Some(&tables::NAMES_SYMBOL)
            } else if n.contains("dingbats") {
                Some(&tables::NAMES_DINGBATS)
            } else {
                None
            }
        };
        let fallback: Vec<Option<Vec<u8>>> = match (&builtin, std_name(&bname)) {
            (Some(b), _) => b.clone(),
            (None, Some(t)) if !embedded => enc_names(t),
            _ => {
                if matches!(&*prog, Prog::Sfnt { .. }) && embedded && !symbolic {
                    enc_names(&tables::NAMES_WINANSI)
                } else if symbolic && matches!(&*prog, Prog::Sfnt { .. }) {
                    vec![None; 256]
                } else {
                    enc_names(&tables::NAMES_STANDARD)
                }
            }
        };
        let (names, differs) = {
            let mut diffs = Vec::new();
            let n = GFont::code_names_with(doc, d, &fallback, &mut diffs);
            (n, diffs)
        };
        g.names = names;
        // each code's glyph
        let mut gids = vec![u32::MAX; 256];
        let is_sfnt_tt = matches!(&*prog, Prog::Sfnt { cff: None, .. });
        let has_enc = !doc.key(d, "Encoding").is_null();
        for code in 0..256u32 {
            let name = g.names[code as usize].as_deref();
            let gid = match &*prog {
                Prog::T1(t) => {
                    if name.is_some_and(|n| t.has(n)) {
                        Some(0)
                    } else {
                        None
                    }
                }
                Prog::Cff { .. } | Prog::Sfnt { cff: Some(_), .. } if embedded || !is_sfnt_tt => {
                    name.and_then(|n| prog.by_name(n)).or_else(|| {
                        // a substitute without the name: through Unicode
                        if embedded {
                            None
                        } else {
                            name.and_then(glyph_unicode).and_then(|u| u.chars().next()).map(|c| prog.unicode(c as u32)).filter(|&g| g != 0)
                        }
                    })
                }
                _ => {
                    let g = GFont::truetype_gid(&prog, code, name, symbolic && !has_enc && !differs.contains(&(code as u8)), embedded);
                    (g != 0).then_some(g)
                }
            };
            if let Some(v) = gid {
                gids[code as usize] = v;
            }
        }
        g.gids = gids;
        // Symbol and ZapfDingbats without widths: the substitute's
        if !embedded && d.get("Widths").is_none() && (bname.contains("Symbol") || bname.contains("Dingbats")) {
            if let Prog::Sfnt { f, .. } = &*prog {
                let upem = f.upem.max(1) as f64;
                g.sub_widths = Some(g.gids.iter().map(|&gid| if gid == u32::MAX { f64::NAN } else { f.adv(gid) as f64 / upem }).collect());
            }
        }
        // a narrower substitute: scale to the width of 'm'
        if !embedded {
            if let Some(code) = (0..256).find(|&c| g.names[c].as_deref() == Some(b"m")) {
                let w = g.base.width(code as u32);
                let gid = g.gids[code];
                if let Prog::Sfnt { f, .. } = &*prog {
                    if gid != u32::MAX && d.get("Widths").is_some() {
                        let adv = f.adv(gid) as f64 / f.upem.max(1) as f64;
                        if w > 0.01 && adv > 0.0 && w < 0.9 * adv {
                            g.matrix[0] *= w / adv;
                            g.matrix[2] *= w / adv;
                        }
                    }
                }
            }
        }
        g.prog = Some(prog);
        g
    }

    fn truetype_gid(prog: &Prog, code: u32, name: Option<&[u8]>, symbolic: bool, embedded: bool) -> u32 {
        let uni = name.and_then(glyph_unicode).and_then(|u| u.chars().next()).map(|c| c as u32);
        if !symbolic {
            if let Some(u) = uni {
                if prog.has_cmap((3, 1)) || prog.has_cmap((3, 10)) || prog.has_cmap((0, 3)) || prog.has_cmap((0, 4)) || !embedded {
                    let g = prog.unicode(u);
                    if g != 0 {
                        return g;
                    }
                }
            }
            if let Some(n) = name {
                if prog.has_cmap((1, 0)) {
                    if let Some(mc) = tables::NAMES_MACROMAN.iter().position(|&i| i != 0xFFFF && glyph_name(i as usize) == n) {
                        let g = prog.cmap((1, 0), mc as u32);
                        if g != 0 {
                            return g;
                        }
                    }
                }
            }
        }
        for base in [0xF000u32, 0, 0xF100, 0xF200] {
            let g = prog.cmap((3, 0), base + code);
            if g != 0 {
                return g;
            }
        }
        let g = prog.cmap((1, 0), code);
        if g != 0 {
            return g;
        }
        if symbolic {
            let g = prog.cmap((3, 1), code);
            if g != 0 {
                return g;
            }
            if let Some(u) = uni {
                let g = prog.unicode(u);
                if g != 0 {
                    return g;
                }
            }
        }
        if let Prog::Sfnt { cmaps, .. } = prog {
            if cmaps.is_empty() && code < prog.glyphs() {
                return code;
            }
        }
        0
    }

    /// the glyph name of each code: the base names with the /Differences on top
    fn code_names_with(doc: &Document, d: &Dict, base: &[Option<Vec<u8>>], diffs: &mut Vec<u8>) -> Vec<Option<Vec<u8>>> {
        let enc = doc.key(d, "Encoding");
        let named: Option<&[u16; 256]> = match &enc {
            Object::Name(n) => GFont::enc_table(&n.0),
            Object::Dict(e) => e.name("BaseEncoding").and_then(GFont::enc_table),
            _ => None,
        };
        let mut names: Vec<Option<Vec<u8>>> = match named {
            Some(t) => enc_names(t),
            None => base.to_vec(),
        };
        names.resize(256, None);
        if let Object::Dict(e) = &enc {
            if let Object::Array(arr) = doc.key(e, "Differences") {
                let mut code: i64 = -1;
                for o in arr.iter().take(4096) {
                    match doc.resolve(o) {
                        Object::Int(c) => code = c,
                        Object::Name(n) => {
                            if (0..256).contains(&code) {
                                names[code as usize] = Some(n.0);
                                diffs.push(code as u8);
                            }
                            code = code.saturating_add(1);
                        }
                        _ => {}
                    }
                }
            }
        }
        names
    }

    /// a Type 3 font's names: its /Encoding (a base encoding and /Differences)
    fn code_names(doc: &Document, d: &Dict) -> Vec<Option<Vec<u8>>> {
        GFont::code_names_with(doc, d, &vec![None; 256], &mut Vec::new())
    }

    fn enc_table(n: &[u8]) -> Option<&'static [u16; 256]> {
        match n {
            b"WinAnsiEncoding" => Some(&tables::NAMES_WINANSI),
            b"MacRomanEncoding" => Some(&tables::NAMES_MACROMAN),
            b"MacExpertEncoding" => Some(&tables::NAMES_MACEXPERT),
            b"StandardEncoding" => Some(&tables::NAMES_STANDARD),
            _ => None,
        }
    }

    fn type3(doc: &Document, d: &Dict) -> Type3 {
        let fm = super::color::nums(doc, &doc.key(d, "FontMatrix"));
        let matrix = if fm.len() == 6 { [fm[0], fm[1], fm[2], fm[3], fm[4], fm[5]] } else { [0.001, 0.0, 0.0, 0.001, 0.0, 0.0] };
        let procs_d = doc.dict(&d.get("CharProcs").cloned().unwrap_or_default());
        let names = GFont::code_names(doc, d);
        let procs = names
            .iter()
            .map(|n| {
                let n = n.as_ref()?;
                match doc.resolve(procs_d.get_bytes(n)?) {
                    Object::Stream(s) => Some(*s),
                    _ => None,
                }
            })
            .collect();
        let res = d.get("Resources").map(|r| doc.dict(r));
        Type3 { matrix, procs, res }
    }

    /// the advance of a code when the reader's does not hold (a substitute's own)
    pub fn width(&self, code: u32) -> Option<f64> {
        self.sub_widths.as_ref()?.get(code as usize).copied().filter(|w| w.is_finite())
    }

    /// the outline of a code's glyph, cached
    pub fn glyph(&self, code: u32) -> Option<Rc<Glyph>> {
        let prog = self.prog.as_ref()?;
        let (gid, name) = if self.composite {
            let cid = self.base.cid(code);
            let gid = if !self.embedded {
                // a substitute: through the code's text
                let u = self.base.text(code)?.chars().next()? as u32;
                prog.unicode(u)
            } else {
                match &self.cid_gid {
                    Some(m) => *m.get(cid as usize)? as u32,
                    None => match &**prog {
                        Prog::Cff { names, .. } | Prog::Sfnt { cff: Some(names), .. } if names.cid_keyed => *names.by_cid.get(&cid)?,
                        _ => cid,
                    },
                }
            };
            (gid, None)
        } else {
            let c = code as usize;
            let g = *self.gids.get(c)?;
            if g == u32::MAX {
                return None;
            }
            (g, self.names.get(c).and_then(|n| n.clone()))
        };
        let key = if matches!(&**prog, Prog::T1(_)) { code } else { gid };
        if let Some(g) = self.cache.borrow().get(&key) {
            return g.clone();
        }
        let mut ol = Outline::default();
        let ok = prog.outline(gid, name.as_deref(), &mut ol) && !ol.pts.is_empty();
        let g = ok.then(|| Rc::new(Glyph::of(&ol)));
        let mut c = self.cache.borrow_mut();
        if c.len() < 1 << 16 {
            c.insert(key, g.clone());
        }
        g
    }
}
