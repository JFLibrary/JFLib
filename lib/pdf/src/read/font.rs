// Fonts as text extraction needs them (ISO 32000-1, 9.6 to 9.10): how a string splits into
// character codes, how wide each one is, and the text it stands for.  The text comes from the
// font's ToUnicode CMap first, else from its encoding (the base encodings of Annex D, the
// /Differences, the built-in encoding of an embedded Type 1 program) through the glyph names
// (the Adobe Glyph List, uniXXXX, uXXXX[XX], ligatures as f_i).  Widths come from /Widths or
// /W; the 14 standard fonts without them use the widths of Helvetica, Times and Courier.

use super::cmap::CMap;
use super::object::{Dict, Object};
use super::tables;
use super::Document;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// the text of a glyph name, as ISO 32000-1 9.10.2 and the glyph list's rules have it
pub(crate) fn glyph_unicode(name: &[u8]) -> Option<String> {
    if name.is_empty() || name == b".notdef" {
        return None;
    }
    // a suffix after the first period (a.sc, one.oldstyle) does not change the text
    let base = match name.iter().position(|&c| c == b'.') {
        Some(0) => return None,
        Some(p) => &name[..p],
        None => name,
    };
    let mut out = String::new();
    for part in base.split(|&c| c == b'_') {
        if part.is_empty() {
            continue;
        }
        if let Some(s) = agl(part) {
            out.push_str(&s);
        } else if part.len() >= 7 && part.starts_with(b"uni") && (part.len() - 3) % 4 == 0 {
            let hex = &part[3..];
            let mut units = Vec::new();
            for c in hex.chunks(4) {
                units.push(u16::from_str_radix(core::str::from_utf8(c).ok()?, 16).ok()?);
            }
            out.extend(char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')));
        } else if (5..=7).contains(&part.len()) && part[0] == b'u' {
            let v = u32::from_str_radix(core::str::from_utf8(&part[1..]).ok()?, 16).ok()?;
            out.push(char::from_u32(v)?);
        } else {
            return None;
        }
    }
    (!out.is_empty()).then_some(out)
}

/// a name of the Adobe Glyph List
fn agl(name: &[u8]) -> Option<String> {
    let n = tables::AGL_CODE.len();
    let at = |i: usize| &tables::AGL_NAMES[tables::AGL_START[i] as usize..tables::AGL_START[i + 1] as usize];
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = (lo + hi) / 2;
        match at(mid).cmp(name) {
            core::cmp::Ordering::Less => lo = mid + 1,
            core::cmp::Ordering::Greater => hi = mid,
            core::cmp::Ordering::Equal => {
                let c = tables::AGL_CODE[mid];
                if c & 0x8000_0000 == 0 {
                    return char::from_u32(c).map(String::from);
                }
                let (start, count) = (((c >> 8) & 0x7F_FFFF) as usize, (c & 0xFF) as usize);
                return tables::AGL_SEQ.get(start..start + count).map(|s| s.iter().filter_map(|&u| char::from_u32(u)).collect());
            }
        }
    }
    None
}

#[derive(Clone, Copy, PartialEq)]
enum Std {
    Helvetica(u8),
    Times(u8),
    Courier,
    Symbol,
    Dingbats,
}

/// one of the 14 standard fonts (or a common name for them: Arial, Times New Roman,
/// Courier New), with its style (bold 1, italic 2)
fn standard(name: &str) -> Option<Std> {
    let n: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
    let style = (n.contains("bold") as u8) | ((n.contains("italic") || n.contains("oblique")) as u8) << 1;
    if n.starts_with("helvetica") || n.starts_with("arial") {
        Some(Std::Helvetica(style))
    } else if n.starts_with("times") {
        Some(Std::Times(style))
    } else if n.starts_with("courier") {
        Some(Std::Courier)
    } else if n.starts_with("symbol") {
        Some(Std::Symbol)
    } else if n.starts_with("zapfdingbats") || n.starts_with("dingbats") {
        Some(Std::Dingbats)
    } else {
        None
    }
}

fn std_width(s: Std, ch: Option<char>) -> f64 {
    let table: &[(u16, u16)] = match s {
        Std::Courier => return 0.6,
        Std::Symbol | Std::Dingbats => return 0.6,
        Std::Helvetica(0) => &tables::WIDTHS_HELVETICA,
        Std::Helvetica(1) => &tables::WIDTHS_HELVETICA_BOLD,
        Std::Helvetica(2) => &tables::WIDTHS_HELVETICA_OBLIQUE,
        Std::Helvetica(_) => &tables::WIDTHS_HELVETICA_BOLD_OBLIQUE,
        Std::Times(0) => &tables::WIDTHS_TIMES,
        Std::Times(1) => &tables::WIDTHS_TIMES_BOLD,
        Std::Times(2) => &tables::WIDTHS_TIMES_ITALIC,
        Std::Times(_) => &tables::WIDTHS_TIMES_BOLD_ITALIC,
    };
    let Some(c) = ch.map(|c| c as u32).filter(|&c| c < 0x10000) else { return 0.5 };
    match table.binary_search_by_key(&(c as u16), |e| e.0) {
        Ok(i) => table[i].1 as f64 / 1000.0,
        Err(_) => 0.5,
    }
}

pub(crate) struct Font {
    /// /BaseFont without a subset tag (ABCDEF+)
    pub name: String,
    /// a composite (Type 0) font's encoding CMap
    cmap: Option<CMap>,
    to_unicode: Option<CMap>,
    /// a simple font: the text of each code from its encoding
    simple: Vec<Option<String>>,
    /// a simple font's widths per code, in text space per unit of size
    widths: Vec<f64>,
    /// a composite font's widths: (first CID, last CID, width), and the default
    cid_widths: Vec<(u32, u32, f64)>,
    dw: f64,
    /// vertical writing: the vertical advance (DW2's second number, negative)
    pub vertical: bool,
    vy: f64,
}

/// a width array of a Type 0 font's descendant (9.7.4.3): c [w1 w2 ...] or c1 c2 w
fn cid_widths(doc: &Document, w: &Object, scale: f64) -> Vec<(u32, u32, f64)> {
    let Object::Array(a) = doc.resolve(w) else { return Vec::new() };
    let mut out = Vec::new();
    let mut i = 0;
    while i < a.len() && out.len() < 1 << 20 {
        let Some(c) = doc.resolve(&a[i]).as_int().and_then(|c| u32::try_from(c).ok()) else { break };
        match a.get(i + 1).map(|o| doc.resolve(o)) {
            Some(Object::Array(ws)) => {
                for (k, v) in ws.iter().enumerate() {
                    if let Some(v) = doc.resolve(v).as_f64() {
                        let cid = c.saturating_add(k as u32);
                        out.push((cid, cid, v * scale));
                    }
                }
                i += 2;
            }
            Some(o) => {
                let (Some(c2), Some(v)) = (o.as_int().and_then(|c| u32::try_from(c).ok()), a.get(i + 2).and_then(|v| doc.resolve(v).as_f64())) else { break };
                out.push((c, c2.max(c), v * scale));
                i += 3;
            }
            None => break,
        }
    }
    out.sort_by_key(|e| e.0);
    out
}

/// the built-in encoding of a Type 1 font program: "dup 32 /space put" in its clear text
fn type1_encoding(prog: &[u8]) -> Option<Vec<Option<Vec<u8>>>> {
    let end = super::lex::find(prog, b"eexec", 0).unwrap_or(prog.len());
    let clear = &prog[..end];
    let at = super::lex::find(clear, b"/Encoding", 0)?;
    let rest = &clear[at + 9..];
    if rest.iter().take(32).copied().collect::<Vec<u8>>().windows(16).any(|w| w == b"StandardEncoding") {
        return None;
    }
    let mut enc = vec![None; 256];
    let mut lx = super::lex::Lexer::new(rest, 0);
    let mut any = false;
    loop {
        match lx.next() {
            super::lex::Tok::Kw(b"dup") => {
                if let (super::lex::Tok::Int(code), super::lex::Tok::Name(n)) = (lx.next(), lx.next()) {
                    if (0..256).contains(&code) {
                        enc[code as usize] = Some(n);
                        any = true;
                    }
                }
            }
            super::lex::Tok::Kw(b"readonly") | super::lex::Tok::Kw(b"def") if any => break,
            super::lex::Tok::Eof => break,
            _ => {}
        }
        lx.reset_budget();
    }
    any.then_some(enc)
}

fn strip_subset(name: &[u8]) -> String {
    let s = String::from_utf8_lossy(name).into_owned();
    match s.find('+') {
        Some(6) if s[..6].bytes().all(|c| c.is_ascii_uppercase()) => s[7..].into(),
        _ => s,
    }
}

impl Font {
    pub fn load(doc: &Document, d: &Dict) -> Font {
        let sub = d.name("Subtype").unwrap_or(b"Type1");
        let name = strip_subset(d.name("BaseFont").unwrap_or(b""));
        let to_unicode = match doc.key(d, "ToUnicode") {
            Object::Stream(s) => doc.decode(&s).ok().map(|b| CMap::parse(&b)).filter(|c| !c.is_empty()),
            _ => None,
        };
        if sub == b"Type0" {
            let cmap = match doc.key(d, "Encoding") {
                Object::Name(n) => CMap::predefined(&n.0),
                Object::Stream(s) => {
                    let mut c = doc.decode(&s).ok().map(|b| CMap::parse(&b)).unwrap_or_default();
                    if c.codespace.is_empty() {
                        c.codespace.push((2, [0; 4], [0xFF, 0xFF, 0, 0]));
                    }
                    if let Some(Object::Name(n)) = s.dict.get("UseCMap") {
                        let b = CMap::predefined(&n.0);
                        c.identity |= b.identity;
                        c.unicode |= b.unicode;
                    }
                    c
                }
                _ => CMap::predefined(b"Identity-H"),
            };
            let desc = match doc.key(d, "DescendantFonts") {
                Object::Array(a) => a.first().map(|o| doc.dict(o)).unwrap_or_default(),
                _ => Dict::new(),
            };
            let dw = doc.key(&desc, "DW").as_f64().unwrap_or(1000.0) / 1000.0;
            let vy = match doc.key(&desc, "DW2") {
                Object::Array(a) => a.get(1).and_then(|v| doc.resolve(v).as_f64()).unwrap_or(-1000.0) / 1000.0,
                _ => -1.0,
            };
            let cid_w = desc.get("W").map(|w| cid_widths(doc, w, 0.001)).unwrap_or_default();
            let name = if name.is_empty() { strip_subset(desc.name("BaseFont").unwrap_or(b"")) } else { name };
            return Font { name, vertical: cmap.vertical, cmap: Some(cmap), to_unicode, simple: Vec::new(), widths: Vec::new(), cid_widths: cid_w, dw, vy };
        }
        // a simple font: Type1, MMType1, TrueType, Type3
        let desc = doc.dict(&d.get("FontDescriptor").cloned().unwrap_or_default());
        let flags = doc.key(&desc, "Flags").as_int().unwrap_or(0);
        let symbolic = flags & 4 != 0 && flags & 32 == 0;
        let std = standard(&name);
        let type3 = sub == b"Type3";
        // the base encoding
        let enc_obj = doc.key(d, "Encoding");
        let base_name: Option<Vec<u8>> = match &enc_obj {
            Object::Name(n) => Some(n.0.clone()),
            Object::Dict(e) => e.name("BaseEncoding").map(<[u8]>::to_vec),
            _ => None,
        };
        let table: Option<&[u16; 256]> = match base_name.as_deref() {
            Some(b"WinAnsiEncoding") => Some(&tables::WINANSI),
            Some(b"MacRomanEncoding") => Some(&tables::MACROMAN),
            Some(b"MacExpertEncoding") => Some(&tables::MACEXPERT),
            Some(b"StandardEncoding") => Some(&tables::STANDARD),
            _ => None,
        };
        let mut simple: Vec<Option<String>> = vec![None; 256];
        let builtin = if table.is_none() && !type3 {
            match doc.key(&desc, "FontFile") {
                Object::Stream(s) => doc.decode(&s).ok().and_then(|p| type1_encoding(&p)),
                _ => None,
            }
        } else {
            None
        };
        if let Some(b) = builtin {
            for (i, n) in b.iter().enumerate() {
                simple[i] = n.as_deref().and_then(glyph_unicode);
            }
        } else {
            let t = match (table, std) {
                (Some(t), _) => Some(t),
                (None, Some(Std::Symbol)) => Some(&tables::SYMBOL),
                (None, Some(Std::Dingbats)) => None,
                (None, _) if type3 => None,
                (None, _) if sub == b"TrueType" && !symbolic => Some(&tables::WINANSI),
                (None, _) if symbolic && sub == b"TrueType" => None,
                (None, _) => Some(&tables::STANDARD),
            };
            if let Some(t) = t {
                for (i, &u) in t.iter().enumerate() {
                    if u != 0 {
                        simple[i] = char::from_u32(u as u32).map(String::from);
                    }
                }
            }
        }
        if let Object::Dict(e) = &enc_obj {
            if let Object::Array(diff) = doc.key(e, "Differences") {
                let mut code: i64 = -1;
                for o in diff {
                    match doc.resolve(&o) {
                        Object::Int(c) => code = c,
                        Object::Name(n) if (0..256).contains(&code) => {
                            simple[code as usize] = glyph_unicode(&n.0);
                            code += 1;
                        }
                        Object::Name(_) => code = code.saturating_add(1),
                        _ => {}
                    }
                }
            }
        }
        // a symbolic font without an encoding: codes as printable ASCII, F0xx as F0 + code
        if simple.iter().all(Option::is_none) && !type3 {
            for (i, s) in simple.iter_mut().enumerate().take(0x7F).skip(0x20) {
                *s = Some(String::from(i as u8 as char));
            }
        }
        // widths
        let scale = if type3 {
            match doc.key(d, "FontMatrix") {
                Object::Array(m) => m.first().and_then(|v| doc.resolve(v).as_f64()).unwrap_or(0.001),
                _ => 0.001,
            }
        } else {
            0.001
        };
        let missing = doc.key(&desc, "MissingWidth").as_f64().map(|w| w * scale);
        let mut widths = vec![f64::NAN; 256];
        let first = doc.key(d, "FirstChar").as_int().unwrap_or(0);
        if let Object::Array(w) = doc.key(d, "Widths") {
            for (k, v) in w.iter().enumerate().take(256) {
                let code = first.saturating_add(k as i64);
                if (0..256).contains(&code) {
                    if let Some(v) = doc.resolve(v).as_f64() {
                        widths[code as usize] = v * scale;
                    }
                }
            }
        }
        for (i, w) in widths.iter_mut().enumerate() {
            if w.is_nan() {
                *w = match (missing, std) {
                    (Some(m), _) if m > 0.0 => m,
                    (_, Some(s)) => std_width(s, simple[i].as_ref().and_then(|t| t.chars().next())),
                    (Some(m), None) => m,
                    (None, None) => 0.5,
                };
            }
        }
        Font { name, cmap: None, to_unicode, simple, widths, cid_widths: Vec::new(), dw: 1.0, vertical: false, vy: -1.0 }
    }

    /// the codes of a string: (code, its length in bytes)
    pub fn codes(&self, s: &[u8], out: &mut Vec<(u32, u8)>) {
        out.clear();
        match &self.cmap {
            None => out.extend(s.iter().map(|&c| (c as u32, 1))),
            Some(cm) => {
                let mut i = 0;
                while i < s.len() {
                    let n = cm.code_len(&s[i..]).min(s.len() - i);
                    out.push((s[i..i + n].iter().fold(0u32, |a, &c| a << 8 | c as u32), n as u8));
                    i += n;
                }
            }
        }
    }

    /// the text of a code (empty when the font does not say)
    pub fn text(&self, code: u32) -> Option<String> {
        if let Some(t) = self.to_unicode.as_ref().and_then(|tu| tu.lookup(code)) {
            return Some(t);
        }
        match &self.cmap {
            None => self.simple.get(code as usize).cloned().flatten(),
            Some(cm) => cm.lookup(code),
        }
    }

    /// the CID of a composite font's code (a simple font's code is its own)
    pub fn cid(&self, code: u32) -> u32 {
        match &self.cmap {
            Some(cm) => cm.cid(code),
            None => code,
        }
    }

    /// a composite font's horizontal advance of a code (the origin of a vertical glyph is
    /// half of it to the left)
    pub fn h_width(&self, code: u32) -> f64 {
        match &self.cmap {
            None => self.width(code),
            Some(cm) => {
                let cid = cm.cid(code);
                let i = self.cid_widths.partition_point(|e| e.0 <= cid);
                for e in self.cid_widths[..i].iter().rev().take(4) {
                    if e.0 <= cid && cid <= e.1 {
                        return e.2;
                    }
                }
                self.dw
            }
        }
    }

    /// the advance of a code in text space per unit of font size (horizontal; for vertical
    /// writing the vertical advance, negative downwards)
    pub fn width(&self, code: u32) -> f64 {
        match &self.cmap {
            None => self.widths.get(code as usize).copied().unwrap_or(0.5),
            Some(cm) => {
                if self.vertical {
                    return self.vy;
                }
                let cid = cm.cid(code);
                let i = self.cid_widths.partition_point(|e| e.0 <= cid);
                for e in self.cid_widths[..i].iter().rev().take(4) {
                    if e.0 <= cid && cid <= e.1 {
                        return e.2;
                    }
                }
                self.dw
            }
        }
    }
}
