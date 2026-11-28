// The lexer and parser of PDF objects (ISO 32000-1, 7.2 and 7.3), for files, object streams
// and content streams.  Lenient as the common readers are: a stray delimiter or keyword inside
// an array or dictionary is skipped, an unterminated string or array ends with the input, a
// dictionary key that is not a name is dropped.  Bounded: nesting deeper than MAX_DEPTH and more
// than MAX_OBJECTS objects in one parse are errors, never a deep recursion or memory without end.

use super::object::{Dict, Name, Object, Ref, Stream};
use alloc::boxed::Box;
use alloc::vec::Vec;

/// arrays and dictionaries inside each other
pub(crate) const MAX_DEPTH: u32 = 100;
/// objects in one top-level parse (a CID font's widths have some ten thousand)
pub(crate) const MAX_OBJECTS: u32 = 1 << 20;

pub(crate) fn is_ws(c: u8) -> bool {
    matches!(c, 0 | 9 | 10 | 12 | 13 | 32)
}

pub(crate) fn is_delim(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

pub(crate) fn is_regular(c: u8) -> bool {
    !is_ws(c) && !is_delim(c)
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug, PartialEq)]
pub(crate) enum Tok<'a> {
    Int(i64),
    Real(f64),
    Str(Vec<u8>),
    Name(Vec<u8>),
    ArrOpen,
    ArrClose,
    DictOpen,
    DictClose,
    BraceOpen,
    BraceClose,
    /// any other run of regular characters (true, obj, R, operators), or a stray ) or >
    Kw(&'a [u8]),
    Eof,
}

pub(crate) struct Lexer<'a> {
    pub b: &'a [u8],
    pub pos: usize,
    /// objects left for this parse
    budget: u32,
}

impl<'a> Lexer<'a> {
    pub fn new(b: &'a [u8], pos: usize) -> Lexer<'a> {
        Lexer { b, pos: pos.min(b.len()), budget: MAX_OBJECTS }
    }

    pub fn reset_budget(&mut self) {
        self.budget = MAX_OBJECTS;
    }

    /// skip white space and comments
    pub fn skip_ws(&mut self) {
        let b = self.b;
        while self.pos < b.len() {
            let c = b[self.pos];
            if is_ws(c) {
                self.pos += 1;
            } else if c == b'%' {
                while self.pos < b.len() && b[self.pos] != b'\n' && b[self.pos] != b'\r' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    pub fn next(&mut self) -> Tok<'a> {
        self.skip_ws();
        let b = self.b;
        let Some(&c) = b.get(self.pos) else { return Tok::Eof };
        match c {
            b'[' => {
                self.pos += 1;
                Tok::ArrOpen
            }
            b']' => {
                self.pos += 1;
                Tok::ArrClose
            }
            b'{' => {
                self.pos += 1;
                Tok::BraceOpen
            }
            b'}' => {
                self.pos += 1;
                Tok::BraceClose
            }
            b'<' if b.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                Tok::DictOpen
            }
            b'>' if b.get(self.pos + 1) == Some(&b'>') => {
                self.pos += 2;
                Tok::DictClose
            }
            b'<' => Tok::Str(self.hex_string()),
            b'(' => Tok::Str(self.literal_string()),
            b'/' => Tok::Name(self.name()),
            b')' | b'>' => {
                self.pos += 1;
                Tok::Kw(&b[self.pos - 1..self.pos])
            }
            b'0'..=b'9' | b'+' | b'-' | b'.' => self.number(),
            _ => {
                let s = self.pos;
                while self.pos < b.len() && is_regular(b[self.pos]) {
                    self.pos += 1;
                }
                Tok::Kw(&b[s..self.pos])
            }
        }
    }

    fn number(&mut self) -> Tok<'a> {
        let b = self.b;
        let mut neg = false;
        while self.pos < b.len() && (b[self.pos] == b'+' || b[self.pos] == b'-') {
            neg ^= b[self.pos] == b'-';
            self.pos += 1;
        }
        let (mut int, mut real, mut big, mut digits) = (0i64, 0f64, false, 0);
        while self.pos < b.len() && b[self.pos].is_ascii_digit() {
            let d = (b[self.pos] - b'0') as i64;
            match int.checked_mul(10).and_then(|v| v.checked_add(d)) {
                Some(v) if !big => int = v,
                _ => big = true,
            }
            real = real * 10.0 + d as f64;
            digits += 1;
            self.pos += 1;
        }
        let mut frac = false;
        if self.pos < b.len() && b[self.pos] == b'.' {
            frac = true;
            self.pos += 1;
            let mut scale = 0.1;
            while self.pos < b.len() && b[self.pos].is_ascii_digit() {
                real += (b[self.pos] - b'0') as f64 * scale;
                scale *= 0.1;
                digits += 1;
                self.pos += 1;
            }
        }
        // "4.-5" and "1.5e3" are not PDF; the rest of such a token is dropped as pdf.js does
        while self.pos < b.len() && is_regular(b[self.pos]) && !b[self.pos].is_ascii_alphabetic() {
            self.pos += 1;
        }
        if digits == 0 {
            return Tok::Int(0);
        }
        if frac || big {
            Tok::Real(if neg { -real } else { real })
        } else {
            Tok::Int(if neg { -int } else { int })
        }
    }

    fn name(&mut self) -> Vec<u8> {
        let b = self.b;
        self.pos += 1;
        let mut out = Vec::new();
        while self.pos < b.len() && is_regular(b[self.pos]) {
            let c = b[self.pos];
            if c == b'#' {
                if let (Some(h), Some(l)) = (b.get(self.pos + 1).and_then(|&c| hex(c)), b.get(self.pos + 2).and_then(|&c| hex(c))) {
                    out.push(h << 4 | l);
                    self.pos += 3;
                    continue;
                }
            }
            out.push(c);
            self.pos += 1;
        }
        out
    }

    fn hex_string(&mut self) -> Vec<u8> {
        let b = self.b;
        self.pos += 1;
        let mut out = Vec::new();
        let mut half: Option<u8> = None;
        while self.pos < b.len() {
            let c = b[self.pos];
            self.pos += 1;
            if c == b'>' {
                break;
            }
            if let Some(v) = hex(c) {
                match half.take() {
                    Some(h) => out.push(h << 4 | v),
                    None => half = Some(v),
                }
            }
        }
        if let Some(h) = half {
            out.push(h << 4);
        }
        out
    }

    fn literal_string(&mut self) -> Vec<u8> {
        let b = self.b;
        self.pos += 1;
        let mut out = Vec::new();
        let mut depth = 1u32;
        while self.pos < b.len() {
            let c = b[self.pos];
            self.pos += 1;
            match c {
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(c);
                }
                b'\r' => {
                    // an end of line inside a string is a line feed
                    if b.get(self.pos) == Some(&b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                b'\\' => {
                    let Some(&e) = b.get(self.pos) else { break };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut v = (e - b'0') as u32;
                            for _ in 0..2 {
                                match b.get(self.pos) {
                                    Some(&d @ b'0'..=b'7') => {
                                        v = v * 8 + (d - b'0') as u32;
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(v as u8);
                        }
                        b'\r' => {
                            if b.get(self.pos) == Some(&b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        _ => out.push(e),
                    }
                }
                _ => out.push(c),
            }
        }
        out
    }

    /// one object; `refs` reads "n g R" as a reference (not in content streams).  A keyword
    /// that is not true, false or null is an error at the top and skipped inside.
    pub fn object(&mut self, refs: bool) -> Result<Object, ()> {
        let t = self.next();
        self.value(t, refs, 0)
    }

    fn value(&mut self, t: Tok<'a>, refs: bool, depth: u32) -> Result<Object, ()> {
        if self.budget == 0 || depth > MAX_DEPTH {
            return Err(());
        }
        self.budget -= 1;
        Ok(match t {
            Tok::Int(i) => {
                if refs && (0..=u32::MAX as i64).contains(&i) {
                    let save = self.pos;
                    if let Tok::Int(g) = self.next() {
                        if (0..=65535).contains(&g) && self.next() == Tok::Kw(b"R") {
                            return Ok(Object::Ref(Ref { num: i as u32, generation: g as u16 }));
                        }
                    }
                    self.pos = save;
                }
                Object::Int(i)
            }
            Tok::Real(r) => Object::Real(r),
            Tok::Str(s) => Object::String(s),
            Tok::Name(n) => Object::Name(Name(n)),
            Tok::ArrOpen => {
                let mut a = Vec::new();
                loop {
                    let save = self.pos;
                    match self.next() {
                        Tok::ArrClose | Tok::Eof => break,
                        Tok::Kw(k) if matches!(k, b"endobj" | b"stream" | b"endstream" | b"obj" | b">") => {
                            self.pos = save;
                            break;
                        }
                        Tok::DictClose => {
                            self.pos = save;
                            break;
                        }
                        Tok::Kw(k) if !matches!(k, b"true" | b"false" | b"null") => self.spend()?,
                        t => a.push(self.value(t, refs, depth + 1)?),
                    }
                }
                Object::Array(a)
            }
            Tok::DictOpen => Object::Dict(self.dict_body(refs, depth)?),
            Tok::Kw(b"true") => Object::Bool(true),
            Tok::Kw(b"false") => Object::Bool(false),
            Tok::Kw(b"null") => Object::Null,
            Tok::BraceOpen | Tok::BraceClose | Tok::ArrClose | Tok::DictClose | Tok::Kw(_) | Tok::Eof => return Err(()),
        })
    }

    /// the entries of a dictionary after its <<
    fn dict_body(&mut self, refs: bool, depth: u32) -> Result<Dict, ()> {
        let mut d = alloc::collections::BTreeMap::new();
        loop {
            let save = self.pos;
            match self.next() {
                Tok::DictClose | Tok::Eof => break,
                Tok::Kw(k) if matches!(k, b"endobj" | b"stream" | b"endstream" | b"obj") => {
                    self.pos = save;
                    break;
                }
                Tok::Name(key) => {
                    let save = self.pos;
                    let v = match self.next() {
                        Tok::DictClose => {
                            self.pos = save;
                            Object::Null
                        }
                        Tok::Kw(k) if !matches!(k, b"true" | b"false" | b"null") => {
                            self.pos = save;
                            if matches!(k, b"endobj" | b"stream" | b"endstream" | b"obj") {
                                d.insert(Name(key), Object::Null);
                                break;
                            }
                            self.next();
                            self.spend()?;
                            Object::Null
                        }
                        t => self.value(t, refs, depth + 1)?,
                    };
                    d.insert(Name(key), v);
                }
                _ => self.spend()?,
            }
        }
        Ok(Dict::from_map(d))
    }

    /// a token skipped costs as much as an object read
    fn spend(&mut self) -> Result<(), ()> {
        if self.budget == 0 {
            return Err(());
        }
        self.budget -= 1;
        Ok(())
    }

    /// the value after a key, for inline-image dictionaries whose ID ends them
    pub fn value_of(&mut self, t: Tok<'a>) -> Result<Object, ()> {
        self.value(t, false, 0)
    }
}

/// the first `needle` at or after `from`
pub(crate) fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let last = hay.len() - needle.len();
    let mut i = from;
    while i <= last {
        match hay[i..=last].iter().position(|&c| c == needle[0]) {
            Some(p) => i += p,
            None => return None,
        }
        if &hay[i..i + needle.len()] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// the last `needle` that starts before `before`
pub(crate) fn rfind(hay: &[u8], needle: &[u8], before: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let mut i = before.min(hay.len() - needle.len() + 1);
    while i > 0 {
        i -= 1;
        if hay[i] == needle[0] && &hay[i..i + needle.len()] == needle {
            return Some(i);
        }
    }
    None
}

/// "num generation obj" at `off` (after white space) -> (the reference, the position after "obj")
pub(crate) fn obj_header(b: &[u8], off: usize) -> Option<(Ref, usize)> {
    let mut lx = Lexer::new(b, off);
    let num = match lx.next() {
        Tok::Int(n) if (0..=u32::MAX as i64).contains(&n) => n as u32,
        _ => return None,
    };
    let generation = match lx.next() {
        Tok::Int(g) if (0..=65535).contains(&g) => g as u16,
        _ => return None,
    };
    if lx.next() != Tok::Kw(b"obj") {
        return None;
    }
    Some((Ref { num, generation }, lx.pos))
}

/// where "endstream" stands in a file, found once: a stream whose /Length is wrong looks its
/// end up here (a search through the file for each such stream would take quadratic time)
pub(crate) struct Ends(Vec<usize>);

impl Ends {
    pub fn of(b: &[u8]) -> Ends {
        let mut v = Vec::new();
        let mut i = 0;
        while let Some(p) = find(b, b"endstream", i) {
            v.push(p);
            i = p + 9;
        }
        Ends(v)
    }

    /// the first "endstream" at or after `from`
    pub fn after(&self, from: usize) -> Option<usize> {
        self.0.get(self.0.partition_point(|&p| p < from)).copied()
    }
}

/// the indirect object at `off`: "num generation obj ... endobj", a stream with its data range.  A
/// stream's /Length that is a reference is asked of `len_of`; a length that does not end at
/// "endstream" is replaced by the next "endstream" (`ends`), as every reader does.  The object
/// (a stream's dictionary) is read up to `limit` (a repaired file: the next object's header).
pub(crate) fn indirect(b: &[u8], off: usize, limit: usize, len_of: &mut dyn FnMut(Ref) -> Option<i64>, ends: &dyn Fn(usize) -> Option<usize>) -> Option<(Ref, Object)> {
    let (r, pos) = obj_header(b, off)?;
    let mut lx = Lexer::new(&b[..limit.clamp(pos, b.len())], pos);
    let save = lx.pos;
    let obj = match lx.next() {
        Tok::Kw(b"endobj") => return Some((r, Object::Null)),
        _ => {
            lx.pos = save;
            lx.object(true).ok()?
        }
    };
    let is_stream = lx.next() == Tok::Kw(b"stream");
    let dict = match obj {
        Object::Dict(d) if is_stream => d,
        obj => return Some((r, obj)),
    };
    {
        let mut start = lx.pos;
        // the end of line after "stream" (CR LF or LF; a lone CR or spaces before it are seen)
        while start < b.len() && b[start] == b' ' {
            start += 1;
        }
        if b.get(start) == Some(&b'\r') {
            start += 1;
        }
        if b.get(start) == Some(&b'\n') {
            start += 1;
        }
        let len = match dict.get("Length") {
            Some(Object::Ref(lr)) if *lr != r => len_of(*lr),
            Some(o) => o.as_int(),
            None => None,
        };
        let mut end = None;
        if let Some(n) = len.filter(|&n| n >= 0).and_then(|n| usize::try_from(n).ok()) {
            if let Some(e) = start.checked_add(n).filter(|&e| e <= b.len()) {
                let mut l = Lexer::new(b, e);
                if l.next() == Tok::Kw(b"endstream") {
                    end = Some(e);
                }
            }
        }
        let end = match end {
            Some(e) => e,
            None => match ends(start) {
                Some(mut e) => {
                    if e > start && b[e - 1] == b'\n' {
                        e -= 1;
                    }
                    if e > start && b[e - 1] == b'\r' {
                        e -= 1;
                    }
                    e
                }
                None => b.len(),
            },
        };
        Some((r, Object::Stream(Box::new(Stream { dict, start, end: end.max(start), owner: Some(r) }))))
    }
}
