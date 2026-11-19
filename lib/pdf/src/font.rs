// font -- TrueType / OpenType fonts for a document: their metrics, the glyphs a document uses,
// and what goes into the file: a Type0 font with Identity-H encoding over a CIDFont whose CIDs
// are the font's glyph ids, a subset of the font program (TrueType: unused glyphs emptied in
// glyf; CFF: unused charstrings replaced by endchar, cff.rs), the widths (W) and a ToUnicode
// CMap for search and copy.
//
// Glyph ids stay as they are (CIDToGIDMap /Identity), so nothing in the content stream depends on
// which glyphs the document uses; the emptied entries of glyf, loca and hmtx compress to
// almost nothing.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::obj::{self, num};
use crate::{cff, Error};

/// the vertical metrics of a font in its own units (Document::font_metrics)
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FontMetrics {
    pub units_per_em: u16,
    /// above the baseline (positive)
    pub ascent: i16,
    /// below the baseline (negative)
    pub descent: i16,
    pub line_gap: i16,
    pub cap_height: i16,
    pub x_height: i16,
    /// how many glyphs the font has
    pub glyphs: u16,
}

pub(crate) struct FontData {
    data: Vec<u8>,
    /// tag, offset, length (into data)
    tables: Vec<([u8; 4], usize, usize)>,
    pub(crate) cff: bool,
    pub(crate) upem: u16,
    pub(crate) glyphs: u16,
    advances: Vec<u16>,
    ps_name: Vec<u8>,
    pub(crate) metrics: FontMetrics,
    flags: u32,
    bbox: [i16; 4],
    italic_angle: f64,
    stem_v: f64,
    no_subset: bool,
    pub(crate) engine: text_shape::Engine,
    /// the glyphs drawn, with the text the first of them stood for
    pub(crate) used: BTreeMap<u16, Option<String>>,
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(o)?, *d.get(o + 1)?]))
}

fn i16_at(d: &[u8], o: usize) -> Option<i16> {
    u16_at(d, o).map(|v| v as i16)
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*d.get(o)?, *d.get(o + 1)?, *d.get(o + 2)?, *d.get(o + 3)?]))
}

const BAD: Error = Error::Font("a damaged font file");

impl FontData {
    pub(crate) fn parse(data: Vec<u8>, face: u32) -> Result<FontData, Error> {
        let d = &data[..];
        let mut base = 0usize;
        if d.get(..4) == Some(b"ttcf") {
            let n = u32_at(d, 8).ok_or(BAD)?;
            if face >= n {
                return Err(Error::Font("no such face in the collection"));
            }
            base = u32_at(d, 12 + 4 * face as usize).ok_or(BAD)? as usize;
        } else if face != 0 {
            return Err(Error::Font("no such face in the collection"));
        }
        let version = u32_at(d, base).ok_or(BAD)?;
        let cff = match version {
            0x0001_0000 | 0x7472_7565 => false, // 'true'
            0x4F54_544F => true,                // 'OTTO'
            _ => return Err(Error::Font("not a TrueType or OpenType font")),
        };
        let n = u16_at(d, base + 4).ok_or(BAD)? as usize;
        let mut tables = Vec::with_capacity(n);
        for i in 0..n {
            let r = base + 12 + 16 * i;
            let tag: [u8; 4] = d.get(r..r + 4).ok_or(BAD)?.try_into().map_err(|_| BAD)?;
            let off = u32_at(d, r + 8).ok_or(BAD)? as usize;
            let len = u32_at(d, r + 12).ok_or(BAD)? as usize;
            if off.checked_add(len).is_none_or(|e| e > d.len()) {
                return Err(BAD);
            }
            tables.push((tag, off, len));
        }
        tables.sort_by(|a, b| a.0.cmp(&b.0));
        let mut f = FontData {
            data: Vec::new(),
            tables,
            cff,
            upem: 0,
            glyphs: 0,
            advances: Vec::new(),
            ps_name: Vec::new(),
            metrics: FontMetrics { units_per_em: 0, ascent: 0, descent: 0, line_gap: 0, cap_height: 0, x_height: 0, glyphs: 0 },
            flags: 0,
            bbox: [0; 4],
            italic_angle: 0.0,
            stem_v: 0.0,
            no_subset: false,
            engine: text_shape::Engine::new(d, face).ok_or(Error::Font("the shaper does not take the font"))?,
            used: BTreeMap::new(),
        };
        f.data = data;
        f.read_tables()?;
        Ok(f)
    }

    fn table(&self, tag: &[u8; 4]) -> Option<&[u8]> {
        let i = self.tables.binary_search_by(|t| t.0.cmp(tag)).ok()?;
        let (_, off, len) = self.tables[i];
        self.data.get(off..off + len)
    }

    fn read_tables(&mut self) -> Result<(), Error> {
        let head = self.table(b"head").ok_or(Error::Font("no head table"))?;
        let upem = u16_at(head, 18).ok_or(BAD)?;
        if !(16..=16384).contains(&upem) {
            return Err(Error::Font("units per em outside 16 to 16384"));
        }
        let bbox = [i16_at(head, 36).ok_or(BAD)?, i16_at(head, 38).ok_or(BAD)?, i16_at(head, 40).ok_or(BAD)?, i16_at(head, 42).ok_or(BAD)?];
        let mac_style = u16_at(head, 44).ok_or(BAD)?;
        let maxp = self.table(b"maxp").ok_or(Error::Font("no maxp table"))?;
        let glyphs = u16_at(maxp, 4).ok_or(BAD)?;
        if glyphs == 0 {
            return Err(Error::Font("a font without glyphs"));
        }
        let hhea = self.table(b"hhea").ok_or(Error::Font("no hhea table"))?;
        let (h_asc, h_desc, h_gap) = (i16_at(hhea, 4).ok_or(BAD)?, i16_at(hhea, 6).ok_or(BAD)?, i16_at(hhea, 8).ok_or(BAD)?);
        let nhm = u16_at(hhea, 34).ok_or(BAD)?;
        if nhm == 0 || nhm > glyphs {
            return Err(BAD);
        }
        let hmtx = self.table(b"hmtx").ok_or(Error::Font("no hmtx table"))?;
        if hmtx.len() < nhm as usize * 4 + (glyphs - nhm) as usize * 2 {
            return Err(BAD);
        }
        let mut advances = Vec::with_capacity(glyphs as usize);
        for g in 0..glyphs as usize {
            let i = g.min(nhm as usize - 1);
            advances.push(u16_at(hmtx, 4 * i).ok_or(BAD)?);
        }
        if self.cff {
            if self.table(b"CFF ").is_none() {
                return Err(if self.table(b"CFF2").is_some() { Error::Unsupported("CFF2 (variable OpenType) fonts") } else { Error::Font("no CFF table") });
            }
            let info = cff::info(self.table(b"CFF ").ok_or(BAD)?).map_err(Error::Font)?;
            if info.glyphs != glyphs {
                return Err(Error::Font("the CFF table and maxp disagree on the glyph count"));
            }
        } else {
            let loca = self.table(b"loca").ok_or(Error::Font("no glyf outlines (bitmap-only fonts are not supported)"))?;
            self.table(b"glyf").ok_or(Error::Font("no glyf table"))?;
            let long = i16_at(head, 50).ok_or(BAD)? != 0;
            if loca.len() < (glyphs as usize + 1) * if long { 4 } else { 2 } {
                return Err(BAD);
            }
        }
        // OS/2: embedding rights, weight, the typographic metrics
        let (mut asc, mut desc, mut gap) = (h_asc, h_desc, h_gap);
        let (mut cap, mut xh, mut weight, mut serif, mut italic) = (0i16, 0i16, 400u16, false, mac_style & 2 != 0);
        let (mut no_subset, mut italic_angle) = (false, 0.0f64);
        if let Some(os2) = self.table(b"OS/2") {
            let fs_type = u16_at(os2, 8).unwrap_or(0);
            if fs_type & 0x000F == 0x0002 {
                return Err(Error::FontLicense("the font's licence forbids embedding (OS/2 fsType: restricted)"));
            }
            if fs_type & 0x0100 != 0 {
                return Err(Error::FontLicense("the font's licence allows embedding its bitmaps only"));
            }
            no_subset = fs_type & 0x0200 != 0;
            weight = u16_at(os2, 4).unwrap_or(400);
            let class = u16_at(os2, 30).unwrap_or(0) >> 8;
            serif = matches!(class, 1..=5 | 7);
            let sel = u16_at(os2, 62).unwrap_or(0);
            italic |= sel & 1 != 0;
            if let (Some(ta), Some(td), Some(tg)) = (i16_at(os2, 68), i16_at(os2, 70), i16_at(os2, 72)) {
                if sel & 0x80 != 0 || (asc == 0 && desc == 0) {
                    (asc, desc, gap) = (ta, td, tg);
                }
            }
            if u16_at(os2, 0).unwrap_or(0) >= 2 {
                xh = i16_at(os2, 86).unwrap_or(0);
                cap = i16_at(os2, 88).unwrap_or(0);
            }
        }
        if cap <= 0 {
            cap = (asc as i32 * 7 / 10) as i16;
        }
        if xh <= 0 {
            xh = (asc as i32 / 2) as i16;
        }
        let mut fixed = false;
        if let Some(post) = self.table(b"post") {
            let a = u32_at(post, 4).unwrap_or(0) as i32;
            italic_angle = a as f64 / 65536.0;
            fixed = u32_at(post, 12).unwrap_or(0) != 0;
        }
        italic |= italic_angle != 0.0;
        self.no_subset = no_subset;
        self.italic_angle = italic_angle;
        self.ps_name = self.postscript_name();
        self.upem = upem;
        self.glyphs = glyphs;
        self.advances = advances;
        self.bbox = bbox;
        self.flags = 4 | if fixed { 1 } else { 0 } | if serif { 2 } else { 0 } | if italic { 64 } else { 0 };
        self.stem_v = ((10.0 + 220.0 * (weight.clamp(50, 1000) as f64 - 50.0) / 900.0) + 0.5) as i64 as f64;
        self.metrics = FontMetrics { units_per_em: upem, ascent: asc, descent: desc, line_gap: gap, cap_height: cap, x_height: xh, glyphs };
        Ok(())
    }

    /// name ID 6, only the characters a PDF name and PostScript allow
    fn postscript_name(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(t) = self.table(b"name") {
            let count = u16_at(t, 2).unwrap_or(0) as usize;
            let strings = u16_at(t, 4).unwrap_or(0) as usize;
            let mut best: Option<(u8, Vec<u8>)> = None;
            for i in 0..count {
                let r = 6 + 12 * i;
                let (Some(pid), Some(eid), Some(nid), Some(len), Some(off)) = (u16_at(t, r), u16_at(t, r + 2), u16_at(t, r + 6), u16_at(t, r + 8), u16_at(t, r + 10)) else { break };
                if nid != 6 {
                    continue;
                }
                let Some(raw) = t.get(strings + off as usize..strings + off as usize + len as usize) else { continue };
                let (rank, s): (u8, Vec<u8>) = match (pid, eid) {
                    (3, 1) | (0, _) => (0, raw.chunks_exact(2).filter(|c| c[0] == 0).map(|c| c[1]).collect()),
                    (1, 0) => (1, raw.to_vec()),
                    _ => continue,
                };
                if best.as_ref().is_none_or(|b| rank < b.0) {
                    best = Some((rank, s));
                }
            }
            if let Some((_, s)) = best {
                out = s.into_iter().filter(|c| (0x21..=0x7E).contains(c) && !b"[](){}<>/%#".contains(c)).take(63).collect();
            }
        }
        if out.is_empty() {
            out.extend_from_slice(b"JFLibFont");
        }
        out
    }

    pub(crate) fn advance(&self, gid: u16) -> u16 {
        self.advances.get(gid as usize).copied().unwrap_or(0)
    }

    /// a glyph drawn; `unicode`: the text it stands for (the first mapping wins)
    pub(crate) fn mark(&mut self, gid: u16, unicode: Option<String>) {
        let e = self.used.entry(gid).or_insert(None);
        if e.is_none() {
            *e = unicode;
        }
    }

    /// width in thousandths of an em (PDF glyph space)
    fn width(&self, gid: u16) -> f64 {
        self.advance(gid) as f64 * 1000.0 / self.upem as f64
    }

    fn keep(&self) -> Vec<bool> {
        let mut keep = vec![self.no_subset; self.glyphs as usize];
        keep[0] = true;
        for &g in self.used.keys() {
            if let Some(k) = keep.get_mut(g as usize) {
                *k = true;
            }
        }
        keep
    }

    /// six capitals from the glyphs used (the same glyphs always give the same tag)
    pub(crate) fn subset_tag(&self) -> [u8; 6] {
        // the name, head (dates, checksum) and the glyph count tell fonts of one name apart
        let mut b = Vec::with_capacity(self.used.len() * 2 + self.ps_name.len() + 64);
        b.extend_from_slice(&self.ps_name);
        b.extend_from_slice(self.table(b"head").unwrap_or(&[]));
        b.extend_from_slice(&self.glyphs.to_be_bytes());
        for &g in self.used.keys() {
            b.extend_from_slice(&g.to_be_bytes());
        }
        let mut h = obj::hash64(0x5044_4654, &b);
        let mut tag = [0u8; 6];
        for t in tag.iter_mut() {
            *t = b'A' + (h % 26) as u8;
            h /= 26;
        }
        tag
    }

    /// TAG+PostScriptName
    pub(crate) fn base_font(&self) -> Vec<u8> {
        let mut n = self.subset_tag().to_vec();
        n.push(b'+');
        n.extend_from_slice(&self.ps_name);
        n
    }

    /// the font program for FontFile2 (an sfnt) or FontFile3 /CIDFontType0C (a bare CFF)
    pub(crate) fn program(&self) -> Result<Vec<u8>, Error> {
        let keep = self.keep();
        if self.cff {
            return cff::subset(self.table(b"CFF ").ok_or(BAD)?, &keep).map_err(Error::Font);
        }
        self.truetype_subset(keep)
    }

    fn truetype_subset(&self, mut keep: Vec<bool>) -> Result<Vec<u8>, Error> {
        let head = self.table(b"head").ok_or(BAD)?;
        let loca = self.table(b"loca").ok_or(BAD)?;
        let glyf = self.table(b"glyf").ok_or(BAD)?;
        let n = self.glyphs as usize;
        let long = i16_at(head, 50).ok_or(BAD)? != 0;
        let mut offs = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let o = if long { u32_at(loca, 4 * i).ok_or(BAD)? as usize } else { u16_at(loca, 2 * i).ok_or(BAD)? as usize * 2 };
            offs.push(o);
        }
        let range = |g: usize| -> Option<(usize, usize)> {
            let (s, e) = (offs[g], offs[g + 1]);
            // a glyph whose end comes before its start is empty (as readers treat it)
            if s <= e && e <= glyf.len() { Some((s, e)) } else if s > e { Some((0, 0)) } else { None }
        };
        // composite glyphs need their components
        let mut work: Vec<usize> = (0..n).filter(|&g| keep[g]).collect();
        while let Some(g) = work.pop() {
            let (s, e) = range(g).ok_or(BAD)?;
            let gl = &glyf[s..e];
            if gl.len() < 10 || i16_at(gl, 0).ok_or(BAD)? >= 0 {
                continue;
            }
            let mut p = 10;
            loop {
                let flags = u16_at(gl, p).ok_or(BAD)?;
                let comp = u16_at(gl, p + 2).ok_or(BAD)? as usize;
                if comp >= n {
                    return Err(BAD);
                }
                if !keep[comp] {
                    keep[comp] = true;
                    work.push(comp);
                }
                p += 4 + if flags & 1 != 0 { 4 } else { 2 };
                p += if flags & 8 != 0 {
                    2
                } else if flags & 0x40 != 0 {
                    4
                } else if flags & 0x80 != 0 {
                    8
                } else {
                    0
                };
                if flags & 0x20 == 0 {
                    break;
                }
            }
        }
        let mut new_glyf = Vec::new();
        let mut new_loca = Vec::with_capacity((n + 1) * 4);
        for (g, &k) in keep.iter().enumerate() {
            new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
            if k {
                let (s, e) = range(g).ok_or(BAD)?;
                new_glyf.extend_from_slice(&glyf[s..e]);
                while new_glyf.len() % 4 != 0 {
                    new_glyf.push(0);
                }
            }
        }
        new_loca.extend_from_slice(&(new_glyf.len() as u32).to_be_bytes());
        let mut new_head = head.to_vec();
        if new_head.len() < 54 {
            return Err(BAD);
        }
        new_head[8..12].copy_from_slice(&[0; 4]);
        new_head[50..52].copy_from_slice(&1u16.to_be_bytes());
        // hmtx with the unused glyphs' metrics zeroed
        let hhea = self.table(b"hhea").ok_or(BAD)?;
        let nhm = u16_at(hhea, 34).ok_or(BAD)? as usize;
        let mut hmtx = self.table(b"hmtx").ok_or(BAD)?[..nhm * 4 + (n - nhm) * 2].to_vec();
        for (g, &k) in keep.iter().enumerate() {
            if !k {
                let (o, l) = if g < nhm { (4 * g, 4) } else { (nhm * 4 + (g - nhm) * 2, 2) };
                hmtx[o..o + l].fill(0);
            }
        }
        let mut post = Vec::with_capacity(32);
        match self.table(b"post") {
            Some(p) if p.len() >= 32 => post.extend_from_slice(&p[..32]),
            _ => post.resize(32, 0),
        }
        post[..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
        // a cmap that maps nothing: CIDFontType2 selects glyphs by CIDToGIDMap
        let cmap: Vec<u8> = [0u16, 1, 3, 1, 0, 12, 4, 24, 0, 2, 2, 0, 0, 0xFFFF, 0, 0xFFFF, 1, 0].iter().flat_map(|v| v.to_be_bytes()).collect();
        let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
            (*b"head", new_head),
            (*b"hhea", hhea.to_vec()),
            (*b"maxp", self.table(b"maxp").ok_or(BAD)?.to_vec()),
            (*b"hmtx", hmtx),
            (*b"loca", new_loca),
            (*b"glyf", new_glyf),
            (*b"post", post),
            (*b"cmap", cmap),
        ];
        for tag in [b"cvt ", b"fpgm", b"prep", b"OS/2", b"name", b"gasp"] {
            if let Some(t) = self.table(tag) {
                tables.push((*tag, t.to_vec()));
            }
        }
        Ok(write_sfnt(0x0001_0000, tables))
    }

    /// the W array of the glyphs used: `[first [w w ...] first [...]]`
    pub(crate) fn widths(&self) -> Vec<u8> {
        let mut o = Vec::new();
        o.push(b'[');
        let mut prev: Option<u16> = None;
        for &g in self.used.keys().chain(core::iter::once(&0)).collect::<alloc::collections::BTreeSet<_>>() {
            if prev.is_some_and(|p| p + 1 == g) {
                o.push(b' ');
            } else {
                if prev.is_some() {
                    o.extend_from_slice(b"] ");
                }
                obj::int(&mut o, g as i64);
                o.extend_from_slice(b" [");
            }
            num(&mut o, self.width(g));
            prev = Some(g);
        }
        o.extend_from_slice(b"]]");
        o
    }

    /// the FontDescriptor's entries (without FontFile)
    pub(crate) fn descriptor(&self, o: &mut Vec<u8>) {
        let s = 1000.0 / self.upem as f64;
        o.extend_from_slice(b"/Type /FontDescriptor /FontName ");
        obj::name(o, &self.base_font());
        o.extend_from_slice(b" /Flags ");
        obj::int(o, self.flags as i64);
        o.extend_from_slice(b" /FontBBox ");
        obj::nums(o, &[self.bbox[0] as f64 * s, self.bbox[1] as f64 * s, self.bbox[2] as f64 * s, self.bbox[3] as f64 * s]);
        o.extend_from_slice(b" /ItalicAngle ");
        num(o, self.italic_angle);
        o.extend_from_slice(b" /Ascent ");
        num(o, self.metrics.ascent as f64 * s);
        o.extend_from_slice(b" /Descent ");
        num(o, self.metrics.descent as f64 * s);
        o.extend_from_slice(b" /CapHeight ");
        num(o, self.metrics.cap_height as f64 * s);
        o.extend_from_slice(b" /XHeight ");
        num(o, self.metrics.x_height as f64 * s);
        o.extend_from_slice(b" /StemV ");
        num(o, self.stem_v);
    }

    /// the ToUnicode CMap of the glyphs that stand for text
    pub(crate) fn to_unicode(&self) -> Vec<u8> {
        let mut o = Vec::new();
        o.extend_from_slice(
            b"/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
        );
        let maps: Vec<(u16, Vec<u8>)> = self
            .used
            .iter()
            .filter_map(|(&g, t)| t.as_ref().map(|t| (g, obj::utf16be(t, 256))))
            .filter(|(_, u)| !u.is_empty())
            .collect();
        for chunk in maps.chunks(100) {
            obj::int(&mut o, chunk.len() as i64);
            o.extend_from_slice(b" beginbfchar\n");
            for (g, u) in chunk {
                obj::hex(&mut o, &g.to_be_bytes());
                o.push(b' ');
                obj::hex(&mut o, u);
                o.push(b'\n');
            }
            o.extend_from_slice(b"endbfchar\n");
        }
        o.extend_from_slice(b"endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        o
    }
}

fn checksum(d: &[u8]) -> u32 {
    let mut s = 0u32;
    for c in d.chunks(4) {
        let mut b = [0u8; 4];
        b[..c.len()].copy_from_slice(c);
        s = s.wrapping_add(u32::from_be_bytes(b));
    }
    s
}

/// an sfnt of the tables (sorted by tag, each 4-byte aligned), head's checkSumAdjustment set
fn write_sfnt(version: u32, mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by(|a, b| a.0.cmp(&b.0));
    let n = tables.len() as u16;
    let mut sel = 0u16;
    while (2u16 << sel) <= n {
        sel += 1;
    }
    let range = 16u16 << sel;
    let mut o = Vec::new();
    o.extend_from_slice(&version.to_be_bytes());
    o.extend_from_slice(&n.to_be_bytes());
    o.extend_from_slice(&range.to_be_bytes());
    o.extend_from_slice(&sel.to_be_bytes());
    o.extend_from_slice(&(n * 16 - range).to_be_bytes());
    let mut off = 12 + 16 * tables.len();
    let mut head_at = None;
    for (tag, t) in &tables {
        o.extend_from_slice(tag);
        o.extend_from_slice(&checksum(t).to_be_bytes());
        o.extend_from_slice(&(off as u32).to_be_bytes());
        o.extend_from_slice(&(t.len() as u32).to_be_bytes());
        if tag == b"head" {
            head_at = Some(off);
        }
        off += t.len().div_ceil(4) * 4;
    }
    for (_, t) in &tables {
        o.extend_from_slice(t);
        while o.len() % 4 != 0 {
            o.push(0);
        }
    }
    if let Some(h) = head_at {
        let adj = 0xB1B0_AFBAu32.wrapping_sub(checksum(&o));
        o[h + 8..h + 12].copy_from_slice(&adj.to_be_bytes());
    }
    o
}
