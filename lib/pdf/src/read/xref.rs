// The cross-reference of a file (ISO 32000-1, 7.5.4 to 7.5.8): classic tables and
// cross-reference streams (PDF 1.5), hybrid files (XRefStm), incremental updates through
// /Prev, newest first.  When the table is missing or wrong the file is scanned for its objects
// ("n g obj") and trailers, as Acrobat, pdf.js and Poppler do; a wrong offset alone sends just
// that object to the scan.

use super::filter;
use super::lex::{self, Lexer, Tok};
use super::object::{Dict, Object, Ref};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::cell::OnceCell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Entry {
    Free,
    /// the byte offset of "n g obj"
    At(usize),
    /// in an object stream: its object number, the index inside
    InStream(u32, u32),
}

pub(crate) struct Xref {
    /// object number -> (entry, generation)
    pub entries: BTreeMap<u32, (Entry, u16)>,
    /// the trailer: the newest section's keys before older ones'
    pub trailer: Dict,
    /// built by a scan of the file
    pub repaired: bool,
    /// object streams the scan found (their objects are added once streams can be decoded)
    pub objstms: Vec<u32>,
}

/// at most this many sections through /Prev
const MAX_SECTIONS: usize = 4096;
/// the cross-reference stream's own size limit
const XREF_STREAM_MAX: usize = 1 << 28;

impl Xref {
    fn new() -> Xref {
        Xref { entries: BTreeMap::new(), trailer: Dict::new(), repaired: false, objstms: Vec::new() }
    }

    /// a newer section's entry stays; a free one gives way to an older one that is used
    /// (hybrid files mark their stream objects free in the table)
    fn set(&mut self, num: u32, e: Entry, generation: u16) {
        match self.entries.get(&num) {
            None => {
                self.entries.insert(num, (e, generation));
            }
            Some((Entry::Free, _)) if e != Entry::Free => {
                self.entries.insert(num, (e, generation));
            }
            _ => {}
        }
    }

    fn merge_trailer(&mut self, t: &Dict) {
        for (k, v) in t.iter() {
            if !self.trailer.0.contains_key(k) {
                self.trailer.0.insert(k.clone(), v.clone());
            }
        }
    }
}

/// the cross-reference of `b`: through startxref, else by a scan
pub(crate) fn load(b: &[u8]) -> Xref {
    if let Some(x) = from_startxref(b) {
        if x.trailer.get("Root").and_then(Object::as_ref).is_some_and(|r| x.entries.contains_key(&r.num)) {
            return x;
        }
    }
    scan(b)
}

fn from_startxref(b: &[u8]) -> Option<Xref> {
    let p = lex::rfind(b, b"startxref", b.len())?;
    let mut lx = Lexer::new(b, p + 9);
    let Tok::Int(first) = lx.next() else { return None };
    let mut x = Xref::new();
    let ends = OnceCell::new();
    let ends = |from: usize| ends.get_or_init(|| lex::Ends::of(b)).after(from);
    let mut seen = BTreeSet::new();
    let mut next = Some(first);
    let mut sections = 0;
    while let Some(off) = next.take() {
        if sections >= MAX_SECTIONS || !seen.insert(off) {
            break;
        }
        let off = usize::try_from(off).ok().filter(|&o| o < b.len());
        let t = off.and_then(|o| section(b, o, &mut x, &ends));
        let Some(t) = t else {
            if sections == 0 {
                return None;
            }
            break;
        };
        sections += 1;
        x.merge_trailer(&t);
        next = t.get("Prev").and_then(Object::as_int);
    }
    Some(x)
}

/// one section at `off`: a table with its trailer, or a cross-reference stream -> its trailer
fn section(b: &[u8], off: usize, x: &mut Xref, ends: &dyn Fn(usize) -> Option<usize>) -> Option<Dict> {
    let mut lx = Lexer::new(b, off);
    if lx.next() == Tok::Kw(b"xref") {
        table(&mut lx, x)?;
        let t = match lx.object(true) {
            Ok(Object::Dict(d)) => d,
            _ => return None,
        };
        // a hybrid file: the stream's entries for the objects in object streams
        if let Some(s) = t.get("XRefStm").and_then(Object::as_int).and_then(|o| usize::try_from(o).ok()) {
            if s < b.len() {
                stream(b, s, x, ends);
            }
        }
        return Some(t);
    }
    stream(b, off, x, ends)
}

/// the subsections of a table after "xref", up to and with "trailer"
fn table(lx: &mut Lexer, x: &mut Xref) -> Option<()> {
    loop {
        match lx.next() {
            Tok::Kw(b"trailer") => return Some(()),
            Tok::Int(start) => {
                let Tok::Int(count) = lx.next() else { return None };
                let mut start = u32::try_from(start).ok()?;
                let mut i = 0i64;
                while i < count {
                    let save = lx.pos;
                    let (Tok::Int(o), Tok::Int(g)) = (lx.next(), lx.next()) else {
                        lx.pos = save;
                        break; // fewer entries than the count says
                    };
                    let kind = lx.next();
                    // a section that says "1 n" but starts with object 0's free entry
                    if i == 0 && start == 1 && o == 0 && g == 65535 && kind == Tok::Kw(b"f") {
                        start = 0;
                    }
                    let num = start.checked_add(i as u32)?;
                    match kind {
                        Tok::Kw(b"n") if o > 0 => x.set(num, Entry::At(o as usize), g.clamp(0, 65535) as u16),
                        Tok::Kw(b"n") | Tok::Kw(b"f") => x.set(num, Entry::Free, 0),
                        _ => return None,
                    }
                    i += 1;
                }
            }
            _ => return None,
        }
    }
}

/// a cross-reference stream at `off` -> its dictionary (the trailer)
fn stream(b: &[u8], off: usize, x: &mut Xref, ends: &dyn Fn(usize) -> Option<usize>) -> Option<Dict> {
    let (_, obj) = lex::indirect(b, off, &mut |_| None, ends)?;
    let Object::Stream(s) = obj else { return None };
    if !s.dict.is("Type", "XRef") {
        return None;
    }
    let w: Vec<usize> = s.dict.get("W")?.as_array()?.iter().map(|o| o.as_int().unwrap_or(0).clamp(0, 8) as usize).collect();
    if w.len() < 3 {
        return None;
    }
    let size = s.dict.int("Size").unwrap_or(0).max(0);
    let index: Vec<i64> = match s.dict.get("Index").and_then(Object::as_array) {
        Some(a) => a.iter().filter_map(Object::as_int).collect(),
        None => alloc::vec![0, size],
    };
    let raw = b.get(s.start..s.end)?.to_vec();
    let (data, img) = filter::apply(&filter::chain(&s.dict), raw, XREF_STREAM_MAX).ok()?;
    if img.is_some() {
        return None;
    }
    let width = w[0] + w[1] + w[2];
    if width == 0 {
        return Some(s.dict);
    }
    let field = |d: &[u8]| d.iter().fold(0u64, |a, &c| a << 8 | c as u64);
    let mut rows = data.chunks_exact(width);
    for pair in index.chunks(2) {
        let (Some(&start), Some(&count)) = (pair.first(), pair.get(1)) else { break };
        let (Ok(start), Ok(count)) = (u32::try_from(start), u64::try_from(count)) else { break };
        for i in 0..count {
            let Some(row) = rows.next() else { return Some(s.dict) };
            let Some(num) = start.checked_add(i as u32) else { break };
            let kind = if w[0] == 0 { 1 } else { field(&row[..w[0]]) };
            let f2 = field(&row[w[0]..w[0] + w[1]]);
            let f3 = field(&row[w[0] + w[1]..]);
            match kind {
                0 => x.set(num, Entry::Free, 0),
                1 => match usize::try_from(f2) {
                    Ok(o) if o > 0 && o < b.len() => x.set(num, Entry::At(o), f3.min(65535) as u16),
                    _ => x.set(num, Entry::Free, 0),
                },
                2 => match (u32::try_from(f2), u32::try_from(f3)) {
                    (Ok(st), Ok(ix)) => x.set(num, Entry::InStream(st, ix), 0),
                    _ => x.set(num, Entry::Free, 0),
                },
                _ => {}
            }
        }
    }
    Some(s.dict)
}

/// every "n g obj" of the file -> object number -> (offset, generation); a later one wins
pub(crate) fn scan_objects(b: &[u8]) -> BTreeMap<u32, (usize, u16)> {
    let mut out = BTreeMap::new();
    let mut i = 0;
    while let Some(p) = lex::find(b, b"obj", i) {
        i = p + 3;
        if b.get(p + 3).is_some_and(|&c| lex::is_regular(c)) {
            continue; // "objx", "object"
        }
        // back over "generation" and "num"
        let mut j = p;
        // a number before position *j, after white space ("12 0obj" has none)
        let field = |j: &mut usize| -> Option<(usize, usize)> {
            while *j > 0 && lex::is_ws(b[*j - 1]) {
                *j -= 1;
            }
            let e = *j;
            while *j > 0 && b[*j - 1].is_ascii_digit() && e - *j < 10 {
                *j -= 1;
            }
            if *j == e {
                return None;
            }
            Some((*j, e))
        };
        let Some((gs, ge)) = field(&mut j) else { continue };
        let Some((ns, ne)) = field(&mut j) else { continue };
        if ns > 0 && lex::is_regular(b[ns - 1]) {
            continue;
        }
        let parse = |s: usize, e: usize| core::str::from_utf8(&b[s..e]).ok().and_then(|t| t.parse::<u64>().ok());
        let (Some(num), Some(generation)) = (parse(ns, ne), parse(gs, ge)) else { continue };
        let (Ok(num), Ok(generation)) = (u32::try_from(num), u16::try_from(generation)) else { continue };
        out.insert(num, (ns, generation));
    }
    out
}

/// the cross-reference rebuilt from the objects and trailers of the file
pub(crate) fn scan(b: &[u8]) -> Xref {
    let mut x = Xref::new();
    x.repaired = true;
    let objs = scan_objects(b);
    let ends = lex::Ends::of(b);
    let mut catalog = None;
    let mut trailers: Vec<(usize, Dict)> = Vec::new();
    for (&num, &(off, generation)) in &objs {
        x.entries.insert(num, (Entry::At(off), generation));
        // what kind of object: catalogs, object streams, cross-reference streams
        let Some((_, obj)) = lex::indirect(b, off, &mut |_| None, &|from| ends.after(from)) else { continue };
        let Some(d) = obj.as_dict() else { continue };
        if d.is("Type", "Catalog") && catalog.is_none_or(|(o, _)| o < off) {
            catalog = Some((off, Ref { num, generation }));
        }
        if d.is("Type", "ObjStm") {
            x.objstms.push(num);
        }
        if d.is("Type", "XRef") && d.contains("Root") && trailers.len() < 64 {
            trailers.push((off, d.clone()));
        }
    }
    // the last trailers of the file, newest first (an old one is rarely needed)
    let mut before = b.len();
    for _ in 0..16 {
        let Some(p) = lex::rfind(b, b"trailer", before) else { break };
        before = p;
        let mut lx = Lexer::new(b, p + 7);
        if let Ok(Object::Dict(d)) = lx.object(true) {
            trailers.push((p, d));
        }
    }
    // newest first: the last trailer in the file, then the others for what it lacks
    trailers.sort_by_key(|t| core::cmp::Reverse(t.0));
    for (_, t) in &trailers {
        x.merge_trailer(t);
    }
    let root_ok = x.trailer.get("Root").and_then(Object::as_ref).is_some_and(|r| x.entries.contains_key(&r.num));
    if !root_ok {
        if let Some((_, r)) = catalog {
            x.trailer.insert("Root", Object::Ref(r));
        }
    }
    x.trailer.0.remove(b"Prev".as_slice());
    x
}
