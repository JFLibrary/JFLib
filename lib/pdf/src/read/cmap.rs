// CMaps (ISO 32000-1, 9.7.5 and 9.10.3; Adobe technical note 5014): the code space of a
// composite font's encoding (how many bytes each character code takes), the CIDs of its codes,
// and ToUnicode maps from codes to text.  Embedded CMaps are parsed; the predefined ones are
// Identity-H / -V and those of lib/cidmap (Table 118: code space and CIDs; a CMap that names one
// with usecmap keeps it under its own entries).  The Unicode ones (Uni...-UCS2-H,
// Uni...-UTF16-H) give their text directly, a surrogate pair of four bytes as one character.
// Ranges are kept as ranges, so <0000> <FFFF> costs one entry.

use super::lex::{Lexer, Tok};
use super::Document;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, Default)]
pub(crate) struct CMap {
    /// (bytes, low, high): a code of n bytes lies in one of them, byte by byte
    pub codespace: Vec<(u8, [u8; 4], [u8; 4])>,
    /// code -> text (bfchar, and bfrange with an array)
    pub chars: BTreeMap<u32, String>,
    /// (low, high, first text as UTF-16 units): code low + k is the text with its last unit + k
    pub ranges: Vec<(u32, u32, Vec<u16>)>,
    /// code -> CID (cidchar) and (low, high, first CID) (cidrange)
    pub cids: BTreeMap<u32, u32>,
    pub cid_ranges: Vec<(u32, u32, u32)>,
    /// Identity-H / -V: the CID is the two-byte code
    pub identity: bool,
    /// the codes are UTF-16BE text (the predefined Unicode CMaps)
    pub unicode: bool,
    /// WMode 1: vertical writing
    pub vertical: bool,
    /// a predefined CMap of lib/cidmap (this one, or the one its usecmap names): the CIDs of the
    /// codes this one has none for
    pub base: Option<Rc<cidmap::CMap>>,
}

fn units(b: &[u8]) -> Vec<u16> {
    if b.len() == 1 {
        return alloc::vec![b[0] as u16];
    }
    b.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect()
}

fn utf16(u: &[u16]) -> String {
    char::decode_utf16(u.iter().copied()).map(|r| r.unwrap_or('\u{FFFD}')).collect()
}

fn code_of(b: &[u8]) -> u32 {
    b.iter().take(4).fold(0u32, |a, &c| a << 8 | c as u32)
}

impl CMap {
    /// a predefined CMap by name
    pub fn predefined(doc: &Document, name: &[u8]) -> CMap {
        let vertical = name.ends_with(b"-V");
        let mut c = CMap { vertical, ..CMap::default() };
        let unicode = name.starts_with(b"Uni") && (name.windows(4).any(|w| w == b"UCS2") || name.windows(5).any(|w| w == b"UTF16"));
        if name.starts_with(b"Identity") {
            c.identity = true;
            c.codespace.push((2, [0; 4], [0xFF, 0xFF, 0, 0]));
        } else if let Some(p) = doc.cidmap(name) {
            c.codespace = p.codespace.iter().map(|s| (s.len, s.lo, s.hi)).collect();
            c.vertical = p.vertical;
            c.unicode = unicode;
            c.base = Some(p);
        } else if unicode {
            c.unicode = true;
            // UTF-16: two bytes, four for a surrogate pair (taken as two codes of two)
            c.codespace.push((2, [0; 4], [0xFF, 0xFF, 0, 0]));
        } else {
            // a code space this reader does not know: two bytes, as most of them use
            c.codespace.push((2, [0; 4], [0xFF, 0xFF, 0, 0]));
        }
        c
    }

    /// parse an embedded CMap (`usecmap` of a predefined one is taken in)
    pub fn parse(doc: &Document, data: &[u8]) -> CMap {
        let mut c = CMap::default();
        let mut lx = Lexer::new(data, 0);
        let mut last_name: Option<Vec<u8>> = None;
        loop {
            lx.reset_budget();
            let t = lx.next();
            match t {
                Tok::Eof => break,
                Tok::Name(n) => {
                    if n == b"WMode" {
                        if let Tok::Int(v) = lx.next() {
                            c.vertical = v == 1;
                        }
                    }
                    last_name = Some(n);
                }
                Tok::Kw(b"usecmap") => {
                    if let Some(n) = last_name.take() {
                        c.use_cmap(CMap::predefined(doc, &n));
                    }
                }
                Tok::Kw(b"begincodespacerange") => loop {
                    let (Tok::Str(lo), Tok::Str(hi)) = (lx.next(), lx.next()) else { break };
                    if lo.is_empty() {
                        continue;
                    }
                    let n = lo.len().min(4);
                    let (mut l, mut h) = ([0u8; 4], [0u8; 4]);
                    l[..n].copy_from_slice(&lo[..n]);
                    h[..n.min(hi.len())].copy_from_slice(&hi[..n.min(hi.len())]);
                    c.codespace.push((n as u8, l, h));
                },
                Tok::Kw(b"beginbfchar") => loop {
                    let save = lx.pos;
                    let Tok::Str(src) = lx.next() else {
                        lx.pos = save;
                        break;
                    };
                    match lx.next() {
                        Tok::Str(dst) => {
                            c.chars.insert(code_of(&src), utf16(&units(&dst)));
                        }
                        // a glyph name instead of text: seen in the wild
                        Tok::Name(n) => {
                            if let Some(s) = super::font::glyph_unicode(&n) {
                                c.chars.insert(code_of(&src), s);
                            }
                        }
                        _ => break,
                    }
                },
                Tok::Kw(b"beginbfrange") => loop {
                    let save = lx.pos;
                    let (Tok::Str(lo), Tok::Str(hi)) = (lx.next(), lx.next()) else {
                        lx.pos = save;
                        break;
                    };
                    let (lo, hi) = (code_of(&lo), code_of(&hi));
                    if hi < lo {
                        let _ = lx.next();
                        continue;
                    }
                    match lx.next() {
                        Tok::Str(dst) => c.ranges.push((lo, hi, units(&dst))),
                        Tok::ArrOpen => {
                            let mut k = lo;
                            loop {
                                match lx.next() {
                                    Tok::Str(dst) => {
                                        if k <= hi {
                                            c.chars.insert(k, utf16(&units(&dst)));
                                        }
                                        k = k.saturating_add(1);
                                    }
                                    Tok::Name(n) => {
                                        if let (true, Some(s)) = (k <= hi, super::font::glyph_unicode(&n)) {
                                            c.chars.insert(k, s);
                                        }
                                        k = k.saturating_add(1);
                                    }
                                    _ => break,
                                }
                            }
                        }
                        _ => break,
                    }
                },
                Tok::Kw(b"begincidchar") => loop {
                    let save = lx.pos;
                    let (Tok::Str(src), Tok::Int(cid)) = (lx.next(), lx.next()) else {
                        lx.pos = save;
                        break;
                    };
                    c.cids.insert(code_of(&src), cid.clamp(0, u32::MAX as i64) as u32);
                },
                Tok::Kw(b"begincidrange") => loop {
                    let save = lx.pos;
                    let (Tok::Str(lo), Tok::Str(hi), Tok::Int(cid)) = (lx.next(), lx.next(), lx.next()) else {
                        lx.pos = save;
                        break;
                    };
                    let (lo, hi) = (code_of(&lo), code_of(&hi));
                    if hi >= lo {
                        c.cid_ranges.push((lo, hi, cid.clamp(0, u32::MAX as i64) as u32));
                    }
                },
                _ => {}
            }
        }
        c.ranges.sort_by_key(|r| r.0);
        c.cid_ranges.sort_by_key(|r| r.0);
        c
    }

    /// take in a CMap that usecmap names: its code space when this one has none, its CIDs
    /// under this one's
    pub fn use_cmap(&mut self, base: CMap) {
        if self.codespace.is_empty() {
            self.codespace = base.codespace;
        }
        self.identity |= base.identity;
        self.unicode |= base.unicode;
        if self.base.is_none() {
            self.base = base.base;
        }
    }

    /// the length of the code at the start of `b` (1 to 4 bytes)
    pub fn code_len(&self, b: &[u8]) -> usize {
        if self.codespace.is_empty() {
            return 1;
        }
        for n in 1..=4usize {
            if b.len() < n {
                break;
            }
            let fits = self.codespace.iter().any(|&(len, lo, hi)| len as usize == n && (0..n).all(|i| lo[i] <= b[i] && b[i] <= hi[i]));
            if fits {
                return n;
            }
        }
        // no range fits: the shortest length of the code space
        self.codespace.iter().map(|c| c.0 as usize).min().unwrap_or(1).min(b.len()).max(1)
    }

    /// the text of a code
    pub fn lookup(&self, code: u32) -> Option<String> {
        if let Some(s) = self.chars.get(&code) {
            return Some(s.clone());
        }
        let i = self.ranges.partition_point(|r| r.0 <= code);
        for r in self.ranges[..i].iter().rev().take(8) {
            if r.0 <= code && code <= r.1 {
                let mut u = r.2.clone();
                if let Some(last) = u.last_mut() {
                    *last = last.wrapping_add((code - r.0) as u16);
                }
                return Some(utf16(&u));
            }
        }
        if self.unicode {
            // a surrogate pair: one code of four bytes
            if code > 0xFFFF {
                let units = [(code >> 16) as u16, code as u16];
                return char::decode_utf16(units).collect::<Result<String, _>>().ok();
            }
            return char::from_u32(code).map(String::from);
        }
        None
    }

    /// the CID of a code
    pub fn cid(&self, code: u32) -> u32 {
        if let Some(&c) = self.cids.get(&code) {
            return c;
        }
        let i = self.cid_ranges.partition_point(|r| r.0 <= code);
        for r in self.cid_ranges[..i].iter().rev().take(8) {
            if r.0 <= code && code <= r.1 {
                return r.2.saturating_add(code - r.0);
            }
        }
        // a predefined CMap's: the code's length from its value (Adobe's CMaps have no code
        // that a shorter one of the same value would shadow), its notdef CID when none fits
        if let Some(b) = &self.base {
            return (1..=4u8).filter(|&n| n == 4 || code < 1u32 << (8 * n as u32)).find_map(|n| b.cid(code, n)).unwrap_or(0);
        }
        if self.identity || (self.cids.is_empty() && self.cid_ranges.is_empty()) {
            return code;
        }
        0
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty() && self.ranges.is_empty()
    }
}
