// reader -- a small, strict PDF reader for checking lib/pdf's output. Written from ISO 32000-1
// (PDF 1.7) alone; it shares no code with the writer, so a misreading of the specification in one
// does not hide in the other.
//
// parse() checks the file structure and reports the first violation with its byte offset: header
// and binary comment line, startxref -> classic xref table (20-byte entries, free list, one entry
// per object), trailer (/Size, /Root, no /Prev, /Encrypt or /XRefStm), every in-use object complete
// up to endobj at exactly its xref offset, stream /Length (direct or indirect) up to endstream, the
// body made of nothing but the listed objects, whitespace and comments, and every reference
// resolving to an in-use object with the right generation.  Cross-reference streams, object
// streams, incremental updates and encryption are rejected with a clear message: the writer does
// not produce them.
//
// On top: the page tree (pages), stream decoding (stream_data: FlateDecode with PNG predictors,
// DCTDecode returned raw), content stream tokens (content_ops) and their grammar (check_content:
// operators, operand types, path / text object nesting, q/Q, BT/ET, BMC/EMC balance), ToUnicode
// CMaps (cmap_map) and check_all, which runs all of these over a whole document.  What a content
// stream uses is checked where it is used: shadings for sh (check_shading: types 1 .. 7, colour
// space, Coords, Domain, Extend, and their functions by check_function: types 0, 2, 3, 4 with
// Domain, Range, C0 / C1, N, Functions, Bounds, Encode), graphics states for gs (alpha values,
// soft masks with their transparency group, whose content is checked like a form), patterns for
// scn / SCN (tiling: PaintType, TilingType, BBox, XStep / YStep, Resources, content; shading
// patterns), pattern names only in a Pattern colour space.
//
// No panics on any input: every index is checked, nesting is limited, all work is linear in the
// input (plus the decoded stream sizes, capped at 256 MiB per stream).

// a toolkit for the checks: not every check uses every accessor
#![allow(dead_code)]

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// nesting limit of arrays and dictionaries
const MAX_DEPTH: usize = 64;
/// the highest object number accepted
const MAX_OBJECTS: u64 = 10_000_000;
/// the largest decoded stream
const MAX_DECODED: usize = 256 << 20;
/// the longest chain of references resolve() follows
const MAX_REF_CHAIN: usize = 32;
/// the deepest page tree
const MAX_TREE_DEPTH: usize = 64;
/// graphics state nesting limit (ISO 32000-1 Annex C, q/Q)
const MAX_Q_DEPTH: usize = 28;
/// the most entries in one beginbfchar / beginbfrange / begincodespacerange block
const MAX_CMAP_BLOCK: i64 = 100;
/// the deepest nesting of form XObjects check_all follows
const MAX_FORM_DEPTH: usize = 16;

// ---------------------------------------------------------------------------------------------
// objects

/// a PDF object
#[derive(Clone, Debug, PartialEq)]
pub enum Obj {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    /// without the leading '/', #xx escapes decoded
    Name(Vec<u8>),
    /// the decoded bytes of a literal or hex string
    Str(Vec<u8>),
    Array(Vec<Obj>),
    Dict(Dict),
    /// object number, generation
    Ref(u32, u16),
    /// the dictionary and the raw (still encoded) data
    Stream(Dict, Vec<u8>),
}

/// a dictionary in file order; keys without the leading '/'
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Dict(pub Vec<(Vec<u8>, Obj)>);

static NULL: Obj = Obj::Null;

impl Dict {
    /// the value of a key (unresolved)
    pub fn get(&self, key: &str) -> Option<&Obj> {
        self.0
            .iter()
            .find(|(k, _)| k.as_slice() == key.as_bytes())
            .map(|(_, v)| v)
    }
    pub fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn keys(&self) -> impl Iterator<Item = &[u8]> {
        self.0.iter().map(|(k, _)| k.as_slice())
    }
    /// /Type is the name t (direct)
    pub fn type_is(&self, t: &str) -> bool {
        self.get("Type").is_some_and(|o| o.is_name(t))
    }
}

impl Obj {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Obj::Int(n) => Some(*n),
            _ => None,
        }
    }
    /// an integer or a real
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Obj::Int(n) => Some(*n as f64),
            Obj::Real(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Obj::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_name(&self) -> Option<&[u8]> {
        match self {
            Obj::Name(n) => Some(n),
            _ => None,
        }
    }
    pub fn is_name(&self, n: &str) -> bool {
        self.as_name() == Some(n.as_bytes())
    }
    pub fn as_str(&self) -> Option<&[u8]> {
        match self {
            Obj::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_array(&self) -> Option<&[Obj]> {
        match self {
            Obj::Array(a) => Some(a),
            _ => None,
        }
    }
    /// a dictionary, or a stream's dictionary
    pub fn as_dict(&self) -> Option<&Dict> {
        match self {
            Obj::Dict(d) | Obj::Stream(d, _) => Some(d),
            _ => None,
        }
    }
    /// a stream's dictionary and raw data
    pub fn as_stream(&self) -> Option<(&Dict, &[u8])> {
        match self {
            Obj::Stream(d, s) => Some((d, s)),
            _ => None,
        }
    }
    /// a reference's object number and generation
    pub fn as_ref_id(&self) -> Option<(u32, u16)> {
        match self {
            Obj::Ref(n, g) => Some((*n, *g)),
            _ => None,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Obj::Null)
    }
    /// the kind, for messages
    pub fn kind(&self) -> &'static str {
        match self {
            Obj::Null => "null",
            Obj::Bool(_) => "a boolean",
            Obj::Int(_) => "an integer",
            Obj::Real(_) => "a real",
            Obj::Name(_) => "a name",
            Obj::Str(_) => "a string",
            Obj::Array(_) => "an array",
            Obj::Dict(_) => "a dictionary",
            Obj::Ref(..) => "a reference",
            Obj::Stream(..) => "a stream",
        }
    }
}

/// bytes for a message (names, keywords): printable ASCII as is, the rest as #xx
fn show(b: &[u8]) -> String {
    let mut s = String::new();
    for &c in b.iter().take(64) {
        if (0x21..=0x7e).contains(&c) && c != b'#' {
            s.push(c as char);
        } else {
            s.push_str(&format!("#{c:02x}"));
        }
    }
    if b.len() > 64 {
        s.push_str("...");
    }
    s
}

// ---------------------------------------------------------------------------------------------
// tokens (ISO 32000-1 7.2, 7.3)

fn is_ws(c: u8) -> bool {
    matches!(c, 0 | 9 | 10 | 12 | 13 | 32)
}

fn is_delim(c: u8) -> bool {
    matches!(c, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

fn is_regular(c: u8) -> bool {
    !is_ws(c) && !is_delim(c)
}

fn hexval(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// an end-of-line marker (CR, LF or CR LF) at p -> the position after it
fn eol_at(b: &[u8], p: usize) -> Option<usize> {
    match b.get(p) {
        Some(b'\r') => Some(if b.get(p + 1) == Some(&b'\n') { p + 2 } else { p + 1 }),
        Some(b'\n') => Some(p + 1),
        _ => None,
    }
}

/// an end-of-line marker ending at p -> its start
fn eol_before(b: &[u8], p: usize) -> Option<usize> {
    if p >= 2 && b.get(p - 2..p) == Some(b"\r\n".as_slice()) {
        Some(p - 2)
    } else if p >= 1 && matches!(b.get(p - 1), Some(b'\n' | b'\r')) {
        Some(p - 1)
    } else {
        None
    }
}

/// the keyword kw at p, followed by a non-regular byte or the end
fn keyword_at(b: &[u8], p: usize, kw: &[u8]) -> bool {
    b.get(p..).is_some_and(|r| r.starts_with(kw)) && !b.get(p + kw.len()).is_some_and(|&c| is_regular(c))
}

/// unsigned decimal digits at p -> (value, position after them); None without a digit or on overflow
fn digits_at(b: &[u8], p: usize) -> Option<(u64, usize)> {
    let mut v: u64 = 0;
    let mut q = p;
    while let Some(&c) = b.get(q) {
        if !c.is_ascii_digit() {
            break;
        }
        v = v.checked_mul(10)?.checked_add(u64::from(c - b'0'))?;
        q += 1;
    }
    if q == p { None } else { Some((v, q)) }
}

enum Tok<'a> {
    Int(i64),
    Real(f64),
    Name(Vec<u8>),
    Str(Vec<u8>),
    DictOpen,
    DictClose,
    ArrOpen,
    ArrClose,
    BraceOpen,
    BraceClose,
    /// a run of regular characters that is not a number: true, false, null, R, obj, operators ...
    Kw(&'a [u8]),
}

struct Lexer<'a> {
    b: &'a [u8],
    pos: usize,
    /// "offset" for the file, "content offset" for a content stream ...
    ctx: &'static str,
}

impl<'a> Lexer<'a> {
    fn new(b: &'a [u8], pos: usize, ctx: &'static str) -> Self {
        Lexer { b, pos, ctx }
    }

    fn err<T>(&self, at: usize, msg: &str) -> Result<T, String> {
        Err(format!("{} {}: {}", self.ctx, at, msg))
    }

    /// whitespace and comments
    fn skip_ws(&mut self) {
        while let Some(&c) = self.b.get(self.pos) {
            if is_ws(c) {
                self.pos += 1;
            } else if c == b'%' {
                while let Some(&c) = self.b.get(self.pos) {
                    if c == b'\r' || c == b'\n' {
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// the next token and its offset; None at the end of the data
    fn next(&mut self) -> Result<Option<(usize, Tok<'a>)>, String> {
        self.skip_ws();
        let at = self.pos;
        let Some(&c) = self.b.get(at) else { return Ok(None) };
        let tok = match c {
            b'(' => Tok::Str(self.lit_string()?),
            b')' => return self.err(at, "unbalanced ')'"),
            b'<' => {
                if self.b.get(at + 1) == Some(&b'<') {
                    self.pos += 2;
                    Tok::DictOpen
                } else {
                    Tok::Str(self.hex_string()?)
                }
            }
            b'>' => {
                if self.b.get(at + 1) == Some(&b'>') {
                    self.pos += 2;
                    Tok::DictClose
                } else {
                    return self.err(at, "a single '>' outside a hex string");
                }
            }
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
            b'/' => Tok::Name(self.name()?),
            _ => {
                let mut end = at;
                while self.b.get(end).is_some_and(|&c| is_regular(c)) {
                    end += 1;
                }
                let w = self.b.get(at..end).unwrap_or(&[]);
                self.pos = end;
                if matches!(c, b'0'..=b'9' | b'+' | b'-' | b'.') {
                    self.number(at, w)?
                } else {
                    Tok::Kw(w)
                }
            }
        };
        Ok(Some((at, tok)))
    }

    /// integers: [+-]digits; reals: [+-] digits with one '.' anywhere (4. .5 -.5), no exponent
    fn number(&self, at: usize, w: &[u8]) -> Result<Tok<'a>, String> {
        let (neg, body) = match w.first() {
            Some(b'+') => (false, w.get(1..).unwrap_or(&[])),
            Some(b'-') => (true, w.get(1..).unwrap_or(&[])),
            _ => (false, w),
        };
        let mut dot = false;
        let mut ndig = 0usize;
        for &c in body {
            if c == b'.' {
                if dot {
                    return self.err(at, &format!("number '{}' with two decimal points", show(w)));
                }
                dot = true;
            } else if c.is_ascii_digit() {
                ndig += 1;
            } else {
                return self.err(at, &format!("malformed number '{}'", show(w)));
            }
        }
        if ndig == 0 {
            return self.err(at, &format!("number '{}' without digits", show(w)));
        }
        if !dot {
            let mut v: i64 = 0;
            for &c in body {
                v = match v.checked_mul(10).and_then(|v| v.checked_add(i64::from(c - b'0'))) {
                    Some(v) => v,
                    None => return self.err(at, &format!("integer '{}' out of range", show(w))),
                };
            }
            return Ok(Tok::Int(if neg { -v } else { v }));
        }
        // the text is ASCII digits, '.', a sign: core's parser takes "4.", ".5" and the sign
        let v: f64 = match core::str::from_utf8(w).ok().and_then(|s| s.parse().ok()) {
            Some(v) => v,
            None => return self.err(at, &format!("malformed real '{}'", show(w))),
        };
        if !v.is_finite() {
            return self.err(at, &format!("real '{}' out of range", show(w)));
        }
        Ok(Tok::Real(v))
    }

    /// ( ... ) with balanced parentheses and the escapes of Table 3
    fn lit_string(&mut self) -> Result<Vec<u8>, String> {
        let start = self.pos;
        self.pos += 1;
        let mut out = Vec::new();
        let mut depth = 1usize;
        loop {
            let Some(&c) = self.b.get(self.pos) else {
                return self.err(start, "unterminated literal string");
            };
            self.pos += 1;
            match c {
                b'(' => {
                    depth += 1;
                    out.push(c);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(out);
                    }
                    out.push(c);
                }
                b'\\' => {
                    let Some(&e) = self.b.get(self.pos) else {
                        return self.err(start, "unterminated literal string (ends in a backslash)");
                    };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'(' | b')' | b'\\' => out.push(e),
                        b'0'..=b'7' => {
                            let mut v = u32::from(e - b'0');
                            for _ in 0..2 {
                                match self.b.get(self.pos) {
                                    Some(&d @ b'0'..=b'7') => {
                                        v = v * 8 + u32::from(d - b'0');
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            if v > 255 {
                                return self.err(self.pos - 4, "octal escape above \\377 in a literal string");
                            }
                            out.push(v as u8);
                        }
                        // a backslash before an end-of-line: the string continues on the next line
                        b'\r' => {
                            if self.b.get(self.pos) == Some(&b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        _ => {
                            return self.err(
                                self.pos - 2,
                                &format!("unknown escape '\\{}' in a literal string", show(&[e])),
                            );
                        }
                    }
                }
                // an unescaped end-of-line of any kind reads as one LF
                b'\r' => {
                    if self.b.get(self.pos) == Some(&b'\n') {
                        self.pos += 1;
                    }
                    out.push(b'\n');
                }
                _ => out.push(c),
            }
        }
    }

    /// < hex digits and whitespace >; an odd digit count is padded with 0
    fn hex_string(&mut self) -> Result<Vec<u8>, String> {
        let start = self.pos;
        self.pos += 1;
        let mut out = Vec::new();
        let mut hi: Option<u8> = None;
        loop {
            let Some(&c) = self.b.get(self.pos) else {
                return self.err(start, "unterminated hex string");
            };
            self.pos += 1;
            if c == b'>' {
                if let Some(h) = hi {
                    out.push(h << 4);
                }
                return Ok(out);
            }
            if is_ws(c) {
                continue;
            }
            let Some(v) = hexval(c) else {
                return self.err(self.pos - 1, &format!("byte 0x{c:02x} in a hex string"));
            };
            match hi {
                None => hi = Some(v),
                Some(h) => {
                    out.push(h << 4 | v);
                    hi = None;
                }
            }
        }
    }

    /// /name with #xx escapes; bytes outside ! .. ~ must be escaped, #00 is not allowed
    fn name(&mut self) -> Result<Vec<u8>, String> {
        self.pos += 1;
        let mut out = Vec::new();
        while let Some(&c) = self.b.get(self.pos) {
            if !is_regular(c) {
                break;
            }
            if c == b'#' {
                let h = self.b.get(self.pos + 1).and_then(|&x| hexval(x));
                let l = self.b.get(self.pos + 2).and_then(|&x| hexval(x));
                let (Some(h), Some(l)) = (h, l) else {
                    return self.err(self.pos, "'#' in a name not followed by two hex digits");
                };
                let v = h << 4 | l;
                if v == 0 {
                    return self.err(self.pos, "#00 in a name");
                }
                out.push(v);
                self.pos += 3;
            } else {
                if !(0x21..=0x7e).contains(&c) {
                    return self.err(
                        self.pos,
                        &format!("byte 0x{c:02x} in a name must be written as #{c:02x}"),
                    );
                }
                out.push(c);
                self.pos += 1;
            }
        }
        Ok(out)
    }

    /// after an integer n: "g R" follows -> the reference (the position moves past it), else None
    /// and the position stays
    fn try_ref(&mut self, at: usize, n: i64) -> Result<Option<Obj>, String> {
        let save = self.pos;
        let g = match self.next() {
            Ok(Some((_, Tok::Int(g)))) => g,
            _ => {
                self.pos = save;
                return Ok(None);
            }
        };
        match self.next() {
            Ok(Some((_, Tok::Kw(b"R")))) => {}
            _ => {
                self.pos = save;
                return Ok(None);
            }
        }
        if n < 1 || n as u64 > MAX_OBJECTS {
            return self.err(
                at,
                &format!("reference {n} {g} R: object numbers are 1 .. {MAX_OBJECTS}"),
            );
        }
        if !(0..=65535).contains(&g) {
            return self.err(at, &format!("reference {n} {g} R: generation above 65535"));
        }
        Ok(Some(Obj::Ref(n as u32, g as u16)))
    }
}

/// an object starting with the token tok; refs: "n g R" allowed (not in content streams)
fn parse_value(lx: &mut Lexer, tok: (usize, Tok), depth: usize, refs: bool) -> Result<Obj, String> {
    let (at, t) = tok;
    match t {
        Tok::Int(n) => {
            if refs && let Some(r) = lx.try_ref(at, n)? {
                return Ok(r);
            }
            Ok(Obj::Int(n))
        }
        Tok::Real(v) => Ok(Obj::Real(v)),
        Tok::Name(n) => Ok(Obj::Name(n)),
        Tok::Str(s) => Ok(Obj::Str(s)),
        Tok::ArrOpen => {
            if depth >= MAX_DEPTH {
                return lx.err(at, &format!("arrays and dictionaries nested deeper than {MAX_DEPTH}"));
            }
            let mut v = Vec::new();
            loop {
                let Some(t) = lx.next()? else {
                    return lx.err(at, "unterminated array");
                };
                if matches!(t.1, Tok::ArrClose) {
                    return Ok(Obj::Array(v));
                }
                v.push(parse_value(lx, t, depth + 1, refs)?);
            }
        }
        Tok::DictOpen => {
            if depth >= MAX_DEPTH {
                return lx.err(at, &format!("arrays and dictionaries nested deeper than {MAX_DEPTH}"));
            }
            let mut d: Vec<(Vec<u8>, Obj)> = Vec::new();
            let mut seen: BTreeSet<Vec<u8>> = BTreeSet::new();
            loop {
                let Some((kat, kt)) = lx.next()? else {
                    return lx.err(at, "unterminated dictionary");
                };
                let key = match kt {
                    Tok::DictClose => return Ok(Obj::Dict(Dict(d))),
                    Tok::Name(n) => n,
                    _ => return lx.err(kat, "dictionary key is not a name"),
                };
                if !seen.insert(key.clone()) {
                    return lx.err(kat, &format!("key /{} twice in one dictionary", show(&key)));
                }
                let Some(vt) = lx.next()? else {
                    return lx.err(at, "unterminated dictionary");
                };
                if matches!(vt.1, Tok::DictClose) {
                    return lx.err(vt.0, &format!("key /{} without a value", show(&key)));
                }
                let v = parse_value(lx, vt, depth + 1, refs)?;
                d.push((key, v));
            }
        }
        Tok::Kw(b"true") => Ok(Obj::Bool(true)),
        Tok::Kw(b"false") => Ok(Obj::Bool(false)),
        Tok::Kw(b"null") => Ok(Obj::Null),
        Tok::Kw(k) => lx.err(at, &format!("unexpected keyword '{}'", show(k))),
        Tok::DictClose => lx.err(at, "'>>' without '<<'"),
        Tok::ArrClose => lx.err(at, "']' without '['"),
        Tok::BraceOpen | Tok::BraceClose => lx.err(at, "'{' or '}' outside a PostScript calculator function"),
    }
}

// ---------------------------------------------------------------------------------------------
// the file (ISO 32000-1 7.5)

#[derive(Clone, Copy)]
struct Entry {
    /// in use: the byte offset; free: the next free object number
    off: u64,
    gn: u16,
    used: bool,
    /// where the entry is, for messages
    at: usize,
}

/// a parsed PDF file
#[derive(Clone, Debug)]
pub struct Pdf {
    /// from the header
    pub version: (u8, u8),
    /// the in-use objects
    pub objects: BTreeMap<u32, Obj>,
    /// their byte offsets
    pub offsets: BTreeMap<u32, usize>,
    /// their generation numbers
    pub gens: BTreeMap<u32, u16>,
    pub trailer: Dict,
    /// where the xref table starts
    pub xref_offset: usize,
}

fn ferr<T>(at: usize, msg: &str) -> Result<T, String> {
    Err(format!("offset {at}: {msg}"))
}

/// "%PDF-1.N" EOL, then a comment line with at least four bytes >= 128 -> (version, body start)
fn parse_header(b: &[u8]) -> Result<((u8, u8), usize), String> {
    if !b.starts_with(b"%PDF-") {
        return ferr(0, "the file does not start with %PDF-");
    }
    let (Some(&maj), Some(&b'.'), Some(&min)) = (b.get(5), b.get(6), b.get(7)) else {
        return ferr(5, "the header is not %PDF-M.N");
    };
    if !maj.is_ascii_digit() || !min.is_ascii_digit() {
        return ferr(5, "the header is not %PDF-M.N");
    }
    let version = (maj - b'0', min - b'0');
    if !(version.0 == 1 && version.1 <= 7 || version == (2, 0)) {
        return ferr(5, &format!("unknown PDF version {}.{}", version.0, version.1));
    }
    let Some(p) = eol_at(b, 8) else {
        return ferr(
            8,
            "the header line %PDF-M.N is not ended by an end-of-line right after the version",
        );
    };
    if b.get(p) != Some(&b'%') {
        return ferr(p, "the header is not followed by a comment line with binary bytes");
    }
    let mut e = p;
    while b.get(e).is_some_and(|&c| c != b'\r' && c != b'\n') {
        e += 1;
    }
    let binary = b.get(p + 1..e).unwrap_or(&[]).iter().filter(|&&c| c >= 128).count();
    if binary < 4 {
        return ferr(
            p,
            &format!("the comment line after the header has {binary} bytes >= 128, at least 4 needed"),
        );
    }
    let Some(body) = eol_at(b, e) else {
        return ferr(e, "the binary comment line is not ended by an end-of-line");
    };
    Ok((version, body))
}

/// "startxref" EOL offset EOL "%%EOF" [EOL] at the very end -> (position of startxref, offset)
fn parse_tail(b: &[u8]) -> Result<(usize, usize), String> {
    let mut end = b.len();
    if b.ends_with(b"\r\n") {
        end -= 2;
    } else if b.ends_with(b"\n") || b.ends_with(b"\r") {
        end -= 1;
    }
    if end < 5 || b.get(end - 5..end) != Some(b"%%EOF".as_slice()) {
        return ferr(
            end,
            "the file does not end with %%EOF (optionally followed by one end-of-line)",
        );
    }
    let eof = end - 5;
    let Some(dend) = eol_before(b, eof) else {
        return ferr(eof, "%%EOF is not on a line of its own");
    };
    let mut p = dend;
    while p > 0 && b.get(p - 1).is_some_and(|c| c.is_ascii_digit()) {
        p -= 1;
    }
    if p == dend {
        return ferr(dend, "no byte offset on the line before %%EOF");
    }
    let off = match digits_at(b, p) {
        Some((v, _)) if v <= b.len() as u64 => v as usize,
        _ => return ferr(p, "the startxref offset is beyond the end of the file"),
    };
    let Some(kend) = eol_before(b, p) else {
        return ferr(p, "the startxref offset is not on a line of its own");
    };
    if kend < 9 || b.get(kend - 9..kend) != Some(b"startxref".as_slice()) {
        return ferr(kend, "no 'startxref' line before the offset at the end of the file");
    }
    Ok((kend - 9, off))
}

/// the xref table at off -> (entries, position of 'trailer')
fn parse_xref(b: &[u8], off: usize) -> Result<(BTreeMap<u32, Entry>, usize), String> {
    if !keyword_at(b, off, b"xref") {
        if b.get(off).is_some_and(|c| c.is_ascii_digit()) {
            return ferr(
                off,
                "startxref points at an object, not at 'xref': cross-reference streams are not supported",
            );
        }
        return ferr(off, "startxref does not point exactly at the keyword 'xref'");
    }
    let Some(mut p) = eol_at(b, off + 4) else {
        return ferr(off + 4, "'xref' is not followed by an end-of-line");
    };
    let mut entries: BTreeMap<u32, Entry> = BTreeMap::new();
    loop {
        if keyword_at(b, p, b"trailer") {
            return Ok((entries, p));
        }
        // subsection header: first object number, SP, count, EOL
        let Some((start, q)) = digits_at(b, p) else {
            return ferr(p, "expected an xref subsection header 'first count' or 'trailer'");
        };
        if b.get(q) != Some(&b' ') {
            return ferr(q, "xref subsection header: expected one space between the two numbers");
        }
        let Some((count, q)) = digits_at(b, q + 1) else {
            return ferr(q + 1, "xref subsection header: no entry count");
        };
        let Some(q) = eol_at(b, q) else {
            return ferr(
                q,
                "xref subsection header 'first count' not ended by an end-of-line right after the count",
            );
        };
        if start.saturating_add(count) > MAX_OBJECTS + 1 {
            return ferr(
                p,
                &format!("xref subsection {start} {count} goes above object number {MAX_OBJECTS}"),
            );
        }
        if count.saturating_mul(20) > (b.len() - q) as u64 {
            return ferr(
                p,
                &format!("xref subsection of {count} entries runs past the end of the file"),
            );
        }
        p = q;
        for i in 0..count {
            let num = (start + i) as u32;
            let e = b.get(p..p + 20).unwrap_or(&[]);
            let ok = e.len() == 20
                && e[..10].iter().all(u8::is_ascii_digit)
                && e[10] == b' '
                && e[11..16].iter().all(u8::is_ascii_digit)
                && e[16] == b' '
                && matches!(e[17], b'n' | b'f')
                && matches!(&e[18..20], b" \r" | b" \n" | b"\r\n");
            if !ok {
                return ferr(
                    p,
                    &format!(
                        "xref entry of object {num} is not 'nnnnnnnnnn ggggg n' + a 2-byte end-of-line (20 bytes)"
                    ),
                );
            }
            let off = digits_at(e, 0).map_or(0, |v| v.0);
            let gn = digits_at(e, 11).map_or(0, |v| v.0);
            if gn > 65535 {
                return ferr(p, &format!("xref entry of object {num}: generation {gn} above 65535"));
            }
            let entry = Entry {
                off,
                gn: gn as u16,
                used: e[17] == b'n',
                at: p,
            };
            if entries.insert(num, entry).is_some() {
                return ferr(p, &format!("a second xref entry for object {num}"));
            }
            p += 20;
        }
    }
}

/// "trailer" dict, then exactly the startxref line
fn parse_trailer(b: &[u8], tpos: usize, sx: usize) -> Result<Dict, String> {
    let mut lx = Lexer::new(b, tpos + 7, "offset");
    let Some(t) = lx.next()? else {
        return ferr(tpos, "nothing after 'trailer'");
    };
    if !matches!(t.1, Tok::DictOpen) {
        return ferr(t.0, "'trailer' is not followed by a dictionary");
    }
    let Obj::Dict(d) = parse_value(&mut lx, t, 0, true)? else {
        return ferr(tpos, "'trailer' is not followed by a dictionary");
    };
    // what this reader does not follow, before anything else is judged
    if d.has("Encrypt") {
        return ferr(tpos, "the trailer has /Encrypt: encrypted files are not supported");
    }
    if d.has("Prev") {
        return ferr(
            tpos,
            "the trailer has /Prev: incremental updates and linearized files are not supported",
        );
    }
    if d.has("XRefStm") {
        return ferr(
            tpos,
            "the trailer has /XRefStm: hybrid cross-reference files are not supported",
        );
    }
    while b.get(lx.pos).is_some_and(|&c| is_ws(c)) {
        lx.pos += 1;
    }
    if lx.pos != sx {
        return ferr(
            lx.pos,
            "the trailer dictionary is not followed by the final startxref line",
        );
    }
    Ok(d)
}

/// "num gen obj" at off (from the xref entry at entry_at) -> the position after "obj"
fn obj_header(b: &[u8], off: usize, num: u32, gn: u16, entry_at: usize) -> Result<usize, String> {
    let bad = || {
        ferr(
            off,
            &format!("the xref entry of object {num} (offset {entry_at}) does not point at '{num} {gn} obj'"),
        )
    };
    let Some((n, mut p)) = digits_at(b, off) else {
        return bad();
    };
    if n != u64::from(num) {
        return ferr(
            off,
            &format!("the xref entry of object {num} (offset {entry_at}) points at object {n}"),
        );
    }
    let ws = p;
    while b.get(p).is_some_and(|&c| is_ws(c)) {
        p += 1;
    }
    if p == ws {
        return bad();
    }
    let Some((g, mut p2)) = digits_at(b, p) else {
        return bad();
    };
    if g != u64::from(gn) {
        return ferr(
            off,
            &format!("object {num}: generation {g} in the file, {gn} in the xref table"),
        );
    }
    let ws = p2;
    while b.get(p2).is_some_and(|&c| is_ws(c)) {
        p2 += 1;
    }
    if p2 == ws || !keyword_at(b, p2, b"obj") {
        return bad();
    }
    Ok(p2 + 3)
}

/// the integer value of an indirect /Length object
fn length_object(b: &[u8], entries: &BTreeMap<u32, Entry>, num: u32, gn: u16, owner: u32) -> Result<i64, String> {
    let e = match entries.get(&num) {
        Some(e) if e.used && e.gn == gn => *e,
        _ => {
            return Err(format!(
                "object {owner}: /Length {num} {gn} R is not an in-use object of the xref table"
            ));
        }
    };
    let off = usize::try_from(e.off).unwrap_or(usize::MAX);
    let p = obj_header(b, off, num, gn, e.at)?;
    let mut lx = Lexer::new(b, p, "offset");
    let v = match lx.next()? {
        Some((_, Tok::Int(v))) => v,
        _ => {
            return ferr(
                p,
                &format!("object {num}, the /Length of object {owner}, is not a direct integer"),
            );
        }
    };
    match lx.next()? {
        Some((_, Tok::Kw(b"endobj"))) => Ok(v),
        _ => ferr(
            lx.pos,
            &format!("object {num}, the /Length of object {owner}: no endobj after the integer"),
        ),
    }
}

/// the object of an in-use entry -> (object, position after endobj)
fn parse_indirect(b: &[u8], num: u32, e: &Entry, entries: &BTreeMap<u32, Entry>) -> Result<(Obj, usize), String> {
    let off = usize::try_from(e.off).unwrap_or(usize::MAX);
    if off >= b.len() {
        return ferr(
            e.at,
            &format!(
                "the xref offset {} of object {num} is beyond the end of the file",
                e.off
            ),
        );
    }
    let p = obj_header(b, off, num, e.gn, e.at)?;
    let mut lx = Lexer::new(b, p, "offset");
    let Some(t) = lx.next()? else {
        return ferr(p, &format!("object {num}: the file ends before the object's value"));
    };
    let v = parse_value(&mut lx, t, 0, true)?;
    match lx.next()? {
        Some((_, Tok::Kw(b"endobj"))) => Ok((v, lx.pos)),
        Some((at, Tok::Kw(b"stream"))) => {
            let Obj::Dict(d) = v else {
                return ferr(
                    at,
                    &format!("object {num}: 'stream' after {}, not after a dictionary", v.kind()),
                );
            };
            let s = lx.pos;
            let data_start = match (b.get(s), b.get(s + 1)) {
                (Some(b'\r'), Some(b'\n')) => s + 2,
                (Some(b'\n'), _) => s + 1,
                _ => return ferr(s, &format!("object {num}: 'stream' is not followed by CR LF or LF")),
            };
            let len = match d.get("Length") {
                None => return ferr(at, &format!("object {num}: stream dictionary without /Length")),
                Some(Obj::Int(n)) => *n,
                Some(Obj::Ref(r, g)) => length_object(b, entries, *r, *g, num)?,
                Some(o) => return ferr(at, &format!("object {num}: /Length is {}, not an integer", o.kind())),
            };
            let end = usize::try_from(len).ok().and_then(|l| data_start.checked_add(l));
            let Some(end) = end.filter(|&e| e <= b.len()) else {
                return ferr(
                    data_start,
                    &format!("object {num}: /Length {len} does not fit in the file"),
                );
            };
            let data = b.get(data_start..end).unwrap_or(&[]).to_vec();
            let q = eol_at(b, end).unwrap_or(end);
            if !keyword_at(b, q, b"endstream") {
                return ferr(
                    end,
                    &format!("object {num}: /Length {len} bytes of stream data are not followed by [EOL] 'endstream'"),
                );
            }
            lx.pos = q + 9;
            match lx.next()? {
                Some((_, Tok::Kw(b"endobj"))) => Ok((Obj::Stream(d, data), lx.pos)),
                Some((at, _)) => ferr(at, &format!("object {num}: expected 'endobj' after 'endstream'")),
                None => ferr(lx.pos, &format!("object {num}: the file ends before 'endobj'")),
            }
        }
        Some((at, _)) => ferr(at, &format!("object {num}: expected 'endobj' after the object's value")),
        None => ferr(lx.pos, &format!("object {num}: the file ends before 'endobj'")),
    }
}

/// every reference in o resolves to an in-use object with the same generation
fn check_refs(o: &Obj, entries: &BTreeMap<u32, Entry>, owner: &str) -> Result<(), String> {
    match o {
        Obj::Ref(n, g) => match entries.get(n) {
            Some(e) if e.used && e.gn == *g => Ok(()),
            Some(e) if e.used => Err(format!(
                "{owner}: reference {n} {g} R, but object {n} has generation {}",
                e.gn
            )),
            _ => Err(format!("{owner}: reference {n} {g} R to an object that is not in use")),
        },
        Obj::Array(v) => v.iter().try_for_each(|x| check_refs(x, entries, owner)),
        Obj::Dict(d) | Obj::Stream(d, _) => d.0.iter().try_for_each(|(_, x)| check_refs(x, entries, owner)),
        _ => Ok(()),
    }
}

/// parse a whole PDF file strictly; the first violation -> Err with its byte offset
pub fn parse(b: &[u8]) -> Result<Pdf, String> {
    let (version, body_start) = parse_header(b)?;
    let (sx, xref_off) = parse_tail(b)?;
    if xref_off < body_start || xref_off >= sx {
        return ferr(
            sx,
            &format!("the startxref offset {xref_off} is not between the header and startxref"),
        );
    }
    let (entries, tpos) = parse_xref(b, xref_off)?;
    let trailer = parse_trailer(b, tpos, sx)?;

    // entry 0 and the free list (7.5.4): 0 is the head, the last free entry links back to 0
    match entries.get(&0) {
        Some(e) if !e.used && e.gn == 65535 => {}
        _ => {
            return ferr(
                xref_off,
                "the xref table has no entry '0000000000 65535 f' for object 0",
            );
        }
    }
    let nfree = entries.values().filter(|e| !e.used).count();
    let mut cur = 0u32;
    let mut visited = 0usize;
    loop {
        visited += 1;
        let Some(e) = entries.get(&cur) else { break };
        if visited > nfree {
            return ferr(e.at, "the free list of the xref table loops");
        }
        let next = e.off;
        if next == 0 {
            break;
        }
        match u32::try_from(next).ok().and_then(|n| entries.get(&n).map(|x| (n, x))) {
            Some((n, x)) if !x.used => cur = n,
            _ => {
                return ferr(
                    e.at,
                    &format!("free entry {cur} links to {next}, which is not a free entry"),
                );
            }
        }
    }
    if visited != nfree {
        let on_list: BTreeSet<u32> = {
            let mut s = BTreeSet::new();
            let mut c = 0u32;
            while s.insert(c) {
                match entries.get(&c) {
                    Some(e) if e.off != 0 => c = e.off as u32,
                    _ => break,
                }
            }
            s
        };
        if let Some((n, e)) = entries.iter().find(|(n, e)| !e.used && !on_list.contains(n)) {
            return ferr(
                e.at,
                &format!("free entry {n} is not on the free list starting at object 0"),
            );
        }
    }

    // the trailer
    let max_num = entries.keys().next_back().copied().unwrap_or(0);
    match trailer.get("Size") {
        Some(Obj::Int(s)) if *s == i64::from(max_num) + 1 => {}
        Some(Obj::Int(s)) => {
            return ferr(
                tpos,
                &format!(
                    "trailer /Size {s}, but the highest object number is {max_num} (want {})",
                    max_num + 1
                ),
            );
        }
        Some(o) => return ferr(tpos, &format!("trailer /Size is {}, not an integer", o.kind())),
        None => return ferr(tpos, "the trailer has no /Size"),
    }
    match trailer.get("Root") {
        Some(Obj::Ref(..)) => {}
        Some(o) => {
            return ferr(
                tpos,
                &format!("trailer /Root is {}, not an indirect reference", o.kind()),
            );
        }
        None => return ferr(tpos, "the trailer has no /Root"),
    }
    if let Some(o) = trailer.get("Info").filter(|o| !matches!(o, Obj::Ref(..))) {
        return ferr(
            tpos,
            &format!("trailer /Info is {}, not an indirect reference", o.kind()),
        );
    }
    if let Some(id) = trailer.get("ID") {
        let ok = id
            .as_array()
            .is_some_and(|a| a.len() == 2 && a.iter().all(|x| x.as_str().is_some()));
        if !ok {
            return ferr(tpos, "trailer /ID is not an array of two strings");
        }
    }

    // the objects, in file order
    let mut by_off: BTreeMap<u64, u32> = BTreeMap::new();
    for (&num, e) in entries.iter().filter(|(_, e)| e.used) {
        if num == 0 {
            return ferr(e.at, "object 0 is marked in use");
        }
        if let Some(other) = by_off.insert(e.off, num) {
            return ferr(
                e.at,
                &format!("objects {other} and {num} have the same xref offset {}", e.off),
            );
        }
    }
    let mut objects = BTreeMap::new();
    let mut spans: BTreeMap<usize, (u32, usize)> = BTreeMap::new();
    for &num in by_off.values() {
        let Some(e) = entries.get(&num) else { continue };
        let (obj, end) = parse_indirect(b, num, e, &entries)?;
        objects.insert(num, obj);
        spans.insert(e.off as usize, (num, end));
    }

    // the body: from the end of the header to the xref table nothing but the listed objects,
    // whitespace and comments
    let mut lx = Lexer::new(b, body_start, "offset");
    let mut seen = 0usize;
    loop {
        lx.skip_ws();
        let p = lx.pos;
        if p == xref_off {
            break;
        }
        if p > xref_off {
            return ferr(p, "an object runs into the xref table");
        }
        match spans.get(&p) {
            Some(&(_, end)) => {
                seen += 1;
                lx.pos = end;
            }
            None => return ferr(p, "bytes in the body that are not an object listed in the xref table"),
        }
    }
    if seen != spans.len() {
        // an offset inside another object (e.g. inside stream data) or after the xref table
        let mut lx = Lexer::new(b, body_start, "offset");
        let mut reached = BTreeSet::new();
        loop {
            lx.skip_ws();
            match spans.get(&lx.pos) {
                Some(&(num, end)) if lx.pos < xref_off => {
                    reached.insert(num);
                    lx.pos = end;
                }
                _ => break,
            }
        }
        if let Some((off, (num, _))) = spans.iter().find(|(_, (n, _))| !reached.contains(n)) {
            return ferr(
                *off,
                &format!("object {num} lies inside another object or after the xref table"),
            );
        }
    }

    // references
    check_refs(&Obj::Dict(trailer.clone()), &entries, "trailer")?;
    for (num, o) in &objects {
        check_refs(o, &entries, &format!("object {num}"))?;
    }

    let offsets = entries
        .iter()
        .filter(|(_, e)| e.used)
        .map(|(&n, e)| (n, e.off as usize))
        .collect();
    let gens = entries
        .iter()
        .filter(|(_, e)| e.used)
        .map(|(&n, e)| (n, e.gn))
        .collect();
    Ok(Pdf {
        version,
        objects,
        offsets,
        gens,
        trailer,
        xref_offset: xref_off,
    })
}

// ---------------------------------------------------------------------------------------------
// the document

/// check_all's walk through form XObjects: the ones checked, the ones being checked
#[derive(Default)]
struct FormWalk {
    done: BTreeSet<u32>,
    stack: Vec<u32>,
}

/// the inheritable page attributes on the way down the page tree
#[derive(Clone, Copy, Default)]
struct Inherit<'a> {
    resources: Option<&'a Obj>,
    media_box: Option<&'a Obj>,
    crop_box: Option<&'a Obj>,
    rotate: Option<&'a Obj>,
}

impl Pdf {
    /// an in-use object
    pub fn get(&self, num: u32) -> Option<&Obj> {
        self.objects.get(&num)
    }

    /// follows references (at most 32); a missing object or a too long chain gives null
    pub fn resolve<'a>(&'a self, o: &'a Obj) -> &'a Obj {
        let mut cur = o;
        for _ in 0..MAX_REF_CHAIN {
            match cur {
                Obj::Ref(n, _) => match self.objects.get(n) {
                    Some(x) => cur = x,
                    None => return &NULL,
                },
                _ => return cur,
            }
        }
        &NULL
    }

    /// d[key] resolved; None when missing or null
    pub fn lookup<'a>(&'a self, d: &'a Dict, key: &str) -> Option<&'a Obj> {
        d.get(key).map(|o| self.resolve(o)).filter(|o| !o.is_null())
    }

    /// a dictionary object (not a stream)
    fn dict_obj(&self, num: u32, what: &str) -> Result<&Dict, String> {
        match self.objects.get(&num) {
            Some(Obj::Dict(d)) => Ok(d),
            Some(o) => Err(format!("{what} (object {num}) is {}, not a dictionary", o.kind())),
            None => Err(format!("{what}: object {num} does not exist")),
        }
    }

    /// trailer /Root -> the catalog (/Type /Catalog, /Pages an indirect reference)
    pub fn root(&self) -> Result<&Dict, String> {
        let Some(Obj::Ref(n, _)) = self.trailer.get("Root") else {
            return Err(String::from("trailer /Root is not an indirect reference"));
        };
        let d = self.dict_obj(*n, "the catalog")?;
        if !self.lookup(d, "Type").is_some_and(|t| t.is_name("Catalog")) {
            return Err(format!("the catalog (object {n}) does not have /Type /Catalog"));
        }
        match d.get("Pages") {
            Some(Obj::Ref(..)) => Ok(d),
            Some(o) => Err(format!("catalog /Pages is {}, not an indirect reference", o.kind())),
            None => Err(String::from("the catalog has no /Pages")),
        }
    }

    /// the page object numbers in order, through the page tree: /Type, /Kids (indirect), /Count,
    /// /Parent back-links, no node reached twice, inherited /MediaBox and /Resources on every page
    pub fn pages(&self) -> Result<Vec<u32>, String> {
        let root = self.root()?;
        let Some(Obj::Ref(n, _)) = root.get("Pages") else {
            return Err(String::from("catalog /Pages is not an indirect reference"));
        };
        let node = self.dict_obj(*n, "the page tree root")?;
        if !self.lookup(node, "Type").is_some_and(|t| t.is_name("Pages")) {
            return Err(format!("the page tree root (object {n}) does not have /Type /Pages"));
        }
        let mut out = Vec::new();
        let mut visited = BTreeSet::new();
        self.walk_pages(*n, None, Inherit::default(), 0, &mut out, &mut visited)?;
        Ok(out)
    }

    fn walk_pages<'a>(
        &'a self,
        num: u32,
        parent: Option<(u32, u16)>,
        mut inh: Inherit<'a>,
        depth: usize,
        out: &mut Vec<u32>,
        visited: &mut BTreeSet<u32>,
    ) -> Result<usize, String> {
        if depth > MAX_TREE_DEPTH {
            return Err(format!("the page tree is deeper than {MAX_TREE_DEPTH}"));
        }
        if !visited.insert(num) {
            return Err(format!("page tree node {num} is reached twice"));
        }
        let d = self.dict_obj(num, "a page tree node")?;
        match (parent, d.get("Parent")) {
            (None, None) => {}
            (None, Some(_)) => return Err(format!("the page tree root (object {num}) has /Parent")),
            (Some((p, g)), Some(Obj::Ref(q, h))) if (p, g) == (*q, *h) => {}
            (Some((p, g)), _) => {
                return Err(format!(
                    "page tree node {num}: /Parent is not {p} {g} R, the node listing it in /Kids"
                ));
            }
        }
        if let Some(o) = d.get("Resources") {
            inh.resources = Some(o);
        }
        if let Some(o) = d.get("MediaBox") {
            inh.media_box = Some(o);
        }
        if let Some(o) = d.get("CropBox") {
            inh.crop_box = Some(o);
        }
        if let Some(o) = d.get("Rotate") {
            inh.rotate = Some(o);
        }
        let ty = self.lookup(d, "Type").and_then(Obj::as_name).unwrap_or(b"");
        let gn = self.gens.get(&num).copied().unwrap_or(0);
        match ty {
            b"Pages" => {
                let Some(kids) = self.lookup(d, "Kids").and_then(Obj::as_array) else {
                    return Err(format!("page tree node {num} has no /Kids array"));
                };
                let mut count = 0usize;
                for k in kids {
                    let Obj::Ref(kn, _) = k else {
                        return Err(format!(
                            "page tree node {num}: /Kids holds {}, not an indirect reference",
                            k.kind()
                        ));
                    };
                    count += self.walk_pages(*kn, Some((num, gn)), inh, depth + 1, out, visited)?;
                }
                match self.lookup(d, "Count") {
                    Some(Obj::Int(c)) if *c == count as i64 => Ok(count),
                    Some(Obj::Int(c)) => Err(format!("page tree node {num}: /Count {c}, but {count} pages below it")),
                    _ => Err(format!("page tree node {num} has no integer /Count")),
                }
            }
            b"Page" => {
                if parent.is_none() {
                    return Err(format!("the page tree root (object {num}) is a /Page, not /Pages"));
                }
                let is_box = |o: Option<&Obj>| {
                    o.map(|o| self.resolve(o)).and_then(Obj::as_array).is_some_and(|a| {
                        a.len() == 4 && a.iter().all(|x| self.resolve(x).as_num().is_some()) && {
                            let v: Vec<f64> = a.iter().filter_map(|x| self.resolve(x).as_num()).collect();
                            v[0] != v[2] && v[1] != v[3]
                        }
                    })
                };
                if !is_box(inh.media_box) {
                    return Err(format!(
                        "page {num}: no /MediaBox of four numbers with a non-zero area (inherited or own)"
                    ));
                }
                if inh.crop_box.is_some() && !is_box(inh.crop_box) {
                    return Err(format!("page {num}: /CropBox is not four numbers with a non-zero area"));
                }
                if !inh
                    .resources
                    .map(|o| self.resolve(o))
                    .is_some_and(|o| matches!(o, Obj::Dict(_)))
                {
                    return Err(format!("page {num}: no /Resources dictionary (inherited or own)"));
                }
                if let Some(r) = inh.rotate
                    && !self.resolve(r).as_int().is_some_and(|r| r % 90 == 0)
                {
                    return Err(format!("page {num}: /Rotate is not a multiple of 90"));
                }
                match d.get("Contents") {
                    None => {}
                    Some(Obj::Ref(c, _)) => match self.objects.get(c) {
                        Some(Obj::Stream(..)) => {}
                        Some(Obj::Array(a)) => {
                            for x in a {
                                if !matches!(x, Obj::Ref(..)) || !matches!(self.resolve(x), Obj::Stream(..)) {
                                    return Err(format!(
                                        "page {num}: /Contents array holds something else than stream references"
                                    ));
                                }
                            }
                        }
                        _ => return Err(format!("page {num}: /Contents is not a stream or an array of streams")),
                    },
                    Some(Obj::Array(a)) => {
                        for x in a {
                            if !matches!(x, Obj::Ref(..)) || !matches!(self.resolve(x), Obj::Stream(..)) {
                                return Err(format!(
                                    "page {num}: /Contents array holds something else than stream references"
                                ));
                            }
                        }
                    }
                    Some(o) => {
                        return Err(format!(
                            "page {num}: /Contents is {}, not a stream reference or array",
                            o.kind()
                        ));
                    }
                }
                out.push(num);
                Ok(1)
            }
            _ => Err(format!("page tree node {num}: /Type is not /Pages or /Page")),
        }
    }

    /// a page attribute, looked up through the /Parent chain (Resources, MediaBox, CropBox, Rotate
    /// are inheritable); resolved
    pub fn inherited(&self, page: u32, key: &str) -> Option<&Obj> {
        let mut num = page;
        for _ in 0..=MAX_TREE_DEPTH {
            let d = self.objects.get(&num)?.as_dict()?;
            if let Some(v) = self.lookup(d, key) {
                return Some(v);
            }
            num = d.get("Parent")?.as_ref_id()?.0;
        }
        None
    }

    /// the decoded content of a page: its content streams joined with a newline (empty without
    /// /Contents)
    pub fn page_contents(&self, page: u32) -> Result<Vec<u8>, String> {
        let Some(d) = self.objects.get(&page).and_then(Obj::as_dict) else {
            return Err(format!("page {page} is not a dictionary"));
        };
        let mut out = Vec::new();
        match d.get("Contents").map(|o| self.resolve(o)) {
            None | Some(Obj::Null) => {}
            Some(s @ Obj::Stream(..)) => out = self.stream_data(s)?,
            Some(Obj::Array(a)) => {
                for (i, s) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(b'\n');
                    }
                    out.extend_from_slice(&self.stream_data(s)?);
                }
            }
            Some(o) => return Err(format!("page {page}: /Contents is {}", o.kind())),
        }
        Ok(out)
    }

    /// a stream's decoded data: no filter, /FlateDecode (zlib, exact length and Adler-32, PNG
    /// predictors 10-15), /DCTDecode (returned as is; it must be the last filter); filter arrays
    pub fn stream_data(&self, o: &Obj) -> Result<Vec<u8>, String> {
        let Obj::Stream(d, raw) = self.resolve(o) else {
            return Err(format!("{} is not a stream", self.resolve(o).kind()));
        };
        if d.has("F") || d.has("FFilter") || d.has("FDecodeParms") {
            return Err(String::from("external stream data (/F) is not supported"));
        }
        let filters: Vec<&[u8]> = match self.lookup(d, "Filter") {
            None => Vec::new(),
            Some(Obj::Name(n)) => vec![n.as_slice()],
            Some(Obj::Array(a)) => {
                let mut v = Vec::new();
                for f in a {
                    match self.resolve(f) {
                        Obj::Name(n) => v.push(n.as_slice()),
                        x => return Err(format!("/Filter array holds {}, not a name", x.kind())),
                    }
                }
                v
            }
            Some(x) => return Err(format!("/Filter is {}, not a name or an array", x.kind())),
        };
        let parms: Vec<Option<&Dict>> = match self.lookup(d, "DecodeParms") {
            None => vec![None; filters.len()],
            Some(Obj::Dict(p)) if filters.len() == 1 => vec![Some(p)],
            Some(Obj::Array(a)) if a.len() == filters.len() => {
                let mut v = Vec::new();
                for p in a {
                    match self.resolve(p) {
                        Obj::Null => v.push(None),
                        Obj::Dict(p) => v.push(Some(p)),
                        x => return Err(format!("/DecodeParms array holds {}", x.kind())),
                    }
                }
                v
            }
            Some(_) => return Err(String::from("/DecodeParms does not match /Filter")),
        };
        let mut data = raw.clone();
        for (i, f) in filters.iter().enumerate() {
            let p = parms.get(i).copied().flatten();
            match *f {
                b"FlateDecode" => data = self.flate(&data, p)?,
                b"DCTDecode" => {
                    if i + 1 != filters.len() {
                        return Err(String::from("/DCTDecode is not the last filter"));
                    }
                    return Ok(data);
                }
                other => return Err(format!("filter /{} is not supported", show(other))),
            }
        }
        if let Some(dl) = self.lookup(d, "DL")
            && dl.as_int() != Some(data.len() as i64)
        {
            return Err(format!("/DL is {dl:?}, the decoded data has {} bytes", data.len()));
        }
        Ok(data)
    }

    fn flate(&self, src: &[u8], parms: Option<&Dict>) -> Result<Vec<u8>, String> {
        let data = zlib_decode(src)?;
        let Some(p) = parms else { return Ok(data) };
        let int = |k: &str, def: i64| match self.lookup(p, k) {
            None => Ok(def),
            Some(Obj::Int(v)) => Ok(*v),
            Some(o) => Err(format!("/DecodeParms /{k} is {}", o.kind())),
        };
        let predictor = int("Predictor", 1)?;
        match predictor {
            1 => Ok(data),
            2 => Err(String::from("the TIFF predictor 2 is not supported")),
            10..=15 => {
                let colors = int("Colors", 1)?;
                let bpc = int("BitsPerComponent", 8)?;
                let columns = int("Columns", 1)?;
                if !(1..=64).contains(&colors)
                    || !matches!(bpc, 1 | 2 | 4 | 8 | 16)
                    || !(1..=1 << 24).contains(&columns)
                {
                    return Err(format!(
                        "PNG predictor with /Colors {colors} /BitsPerComponent {bpc} /Columns {columns}"
                    ));
                }
                png_unpredict(&data, colors as usize, bpc as usize, columns as usize)
            }
            _ => Err(format!("/Predictor {predictor} is not 1, 2 or 10 .. 15")),
        }
    }

    /// decode every stream (the filters above), each with its object number in an error
    pub fn check_streams(&self) -> Result<(), String> {
        for (num, o) in &self.objects {
            if let Obj::Stream(..) = o {
                self.stream_data(o).map_err(|e| format!("stream object {num}: {e}"))?;
            }
        }
        Ok(())
    }

    /// everything above over the whole document: catalog, page tree, every stream decodable,
    /// every page's content (and the form XObjects it draws) by check_content, with every resource
    /// name it uses present, and every font's /ToUnicode CMap readable by cmap_map
    pub fn check_all(&self) -> Result<(), String> {
        self.check_streams()?;
        let pages = self.pages()?;
        let mut forms = FormWalk::default();
        for (i, &page) in pages.iter().enumerate() {
            let data = self.page_contents(page)?;
            let res = self.inherited(page, "Resources").and_then(Obj::as_dict);
            self.check_drawing(&data, res, &format!("page {} (object {page})", i + 1), &mut forms)?;
        }
        // form XObjects no page draws (appearance streams ...) and tiling patterns, with their own
        // resources
        for (num, o) in &self.objects {
            let Obj::Stream(d, _) = o else { continue };
            let form = self.lookup(d, "Subtype").is_some_and(|s| s.is_name("Form"));
            let tiling = self.lookup(d, "PatternType").and_then(Obj::as_int) == Some(1);
            if form && !forms.done.contains(num) {
                self.check_form(*num, None, &mut forms)?;
            } else if tiling && forms.done.insert(*num) {
                let data = self.stream_data(o).map_err(|e| format!("object {num}: {e}"))?;
                let res = self.lookup(d, "Resources").and_then(Obj::as_dict);
                self.check_drawing(&data, res, &format!("tiling pattern object {num}"), &mut forms)?;
            }
        }
        for (num, o) in &self.objects {
            if let Some(d) = o.as_dict()
                && self.lookup(d, "Type").is_some_and(|t| t.is_name("Font"))
                && let Some(tu) = d.get("ToUnicode")
            {
                let data = self
                    .stream_data(tu)
                    .map_err(|e| format!("font object {num}: /ToUnicode: {e}"))?;
                cmap_map(&data).map_err(|e| format!("font object {num}: /ToUnicode: {e}"))?;
            }
        }
        Ok(())
    }

    /// a form XObject (once): /BBox, its content with its own resources (or those of the drawing
    /// content stream); one that draws itself is an error
    fn check_form(&self, fnum: u32, outer: Option<&Dict>, forms: &mut FormWalk) -> Result<(), String> {
        if forms.stack.contains(&fnum) {
            return Err(format!("form object {fnum} draws itself (through {:?})", forms.stack));
        }
        if forms.done.contains(&fnum) {
            return Ok(());
        }
        if forms.stack.len() >= MAX_FORM_DEPTH {
            return Err(format!(
                "form XObjects nested deeper than {MAX_FORM_DEPTH} ({:?})",
                forms.stack
            ));
        }
        let Some(fo @ Obj::Stream(fd, _)) = self.objects.get(&fnum) else {
            return Err(format!("form object {fnum} is not a stream"));
        };
        let bbox = self.lookup(fd, "BBox").and_then(Obj::as_array);
        if !bbox.is_some_and(|a| a.len() == 4 && a.iter().all(|x| self.resolve(x).as_num().is_some())) {
            return Err(format!("form object {fnum}: no /BBox of four numbers"));
        }
        let data = self.stream_data(fo).map_err(|e| format!("form object {fnum}: {e}"))?;
        let res = self.lookup(fd, "Resources").and_then(Obj::as_dict).or(outer);
        forms.stack.push(fnum);
        let r = self.check_drawing(&data, res, &format!("form object {fnum}"), forms);
        forms.stack.pop();
        forms.done.insert(fnum);
        r
    }

    /// a content stream: grammar, resource names, the form XObjects it draws, the shadings (sh),
    /// graphics states (gs, soft masks among them) and patterns (scn / SCN) it uses, and pattern
    /// colours only in a Pattern colour space (cs / CS, q / Q) and only there
    fn check_drawing(&self, data: &[u8], res: Option<&Dict>, what: &str, forms: &mut FormWalk) -> Result<(), String> {
        let ops = ops_at(data, "content offset").map_err(|e| format!("{what}: {e}"))?;
        check_ops(&ops).map_err(|e| format!("{what}: {e}"))?;
        // per q level: (stroking, nonstroking) colour space is a Pattern space; DeviceGray at first
        let mut spaces: Vec<(bool, bool)> = vec![(false, false)];
        for (at, operands, op) in &ops {
            let e = |msg: String| Err(format!("{what}: content offset {at}: '{}': {msg}", show(op)));
            let cur = spaces.last().copied().unwrap_or((false, false));
            match op.as_slice() {
                b"q" => spaces.push(cur),
                b"Q" if spaces.len() > 1 => {
                    spaces.pop();
                }
                b"CS" | b"cs" => {
                    let n = operands.first().and_then(Obj::as_name).unwrap_or(b"");
                    let pattern = match n {
                        b"Pattern" => true,
                        b"DeviceGray" | b"DeviceRGB" | b"DeviceCMYK" => false,
                        _ => self.res_entry(res, "ColorSpace", n).map(|o| self.resolve(o)).is_some_and(|cs| match cs {
                            Obj::Name(x) => x == b"Pattern",
                            Obj::Array(a) => a.first().is_some_and(|f| f.is_name("Pattern")),
                            _ => false,
                        }),
                    };
                    if let Some(s) = spaces.last_mut() {
                        if op.as_slice() == b"CS" { s.0 = pattern } else { s.1 = pattern }
                    }
                }
                b"G" | b"RG" | b"K" => {
                    if let Some(s) = spaces.last_mut() {
                        s.0 = false;
                    }
                }
                b"g" | b"rg" | b"k" => {
                    if let Some(s) = spaces.last_mut() {
                        s.1 = false;
                    }
                }
                b"SC" | b"SCN" | b"sc" | b"scn" => {
                    let pattern = if op.first() == Some(&b'S') { cur.0 } else { cur.1 };
                    let named = operands.last().is_some_and(|o| o.as_name().is_some());
                    if named && !pattern {
                        return e(String::from("a pattern name, but the colour space is not a Pattern space"));
                    }
                    if !named && pattern {
                        return e(String::from("a Pattern colour space needs a pattern name"));
                    }
                    if named && matches!(op.as_slice(), b"SC" | b"sc") {
                        return e(String::from("SC / sc take no pattern name (SCN / scn do)"));
                    }
                }
                _ => {}
            }
            let need: Option<(&str, &[u8])> = match (op.as_slice(), operands.as_slice()) {
                (b"Tf", [Obj::Name(n), ..]) => Some(("Font", n)),
                (b"Do", [Obj::Name(n)]) => Some(("XObject", n)),
                (b"gs", [Obj::Name(n)]) => Some(("ExtGState", n)),
                (b"sh", [Obj::Name(n)]) => Some(("Shading", n)),
                (b"CS" | b"cs", [Obj::Name(n)]) => match n.as_slice() {
                    b"DeviceGray" | b"DeviceRGB" | b"DeviceCMYK" | b"Pattern" => None,
                    _ => Some(("ColorSpace", n)),
                },
                (b"SCN" | b"scn", [.., Obj::Name(n)]) => Some(("Pattern", n)),
                (b"BDC" | b"DP", [_, Obj::Name(n)]) => Some(("Properties", n)),
                _ => None,
            };
            let Some((cat, name)) = need else { continue };
            let found = res
                .and_then(|r| self.lookup(r, cat))
                .and_then(Obj::as_dict)
                .and_then(|c| {
                    c.0.iter()
                        .find(|(k, _)| k.as_slice() == name)
                        .map(|(_, v)| self.resolve(v))
                })
                .filter(|o| !o.is_null());
            let Some(found) = found else {
                return Err(format!(
                    "{what}: content offset {at}: /{} ({}) is not in the resources' /{cat}",
                    show(name),
                    show(op)
                ));
            };
            if op.as_slice() == b"Do" {
                let Obj::Stream(fd, _) = found else {
                    return Err(format!("{what}: XObject /{} is not a stream", show(name)));
                };
                let sub = self.lookup(fd, "Subtype").and_then(Obj::as_name).unwrap_or(b"");
                match sub {
                    b"Image" => {}
                    b"Form" => {
                        let Some((fnum, _)) = res
                            .and_then(|r| self.lookup(r, "XObject"))
                            .and_then(Obj::as_dict)
                            .and_then(|c| c.0.iter().find(|(k, _)| k.as_slice() == name))
                            .and_then(|(_, v)| v.as_ref_id())
                        else {
                            return Err(format!(
                                "{what}: form XObject /{} is not an indirect reference",
                                show(name)
                            ));
                        };
                        self.check_form(fnum, res, forms).map_err(|e| format!("{what}: {e}"))?;
                    }
                    _ => return Err(format!("{what}: XObject /{} has /Subtype /{}", show(name), show(sub))),
                }
            }
            match op.as_slice() {
                b"sh" => self.check_shading(found).map_err(|e| format!("{what}: shading /{}: {e}", show(name)))?,
                b"gs" => self.check_gstate(found, forms).map_err(|e| format!("{what}: ExtGState /{}: {e}", show(name)))?,
                b"SCN" | b"scn" => {
                    let num = self.res_entry(res, "Pattern", name).and_then(Obj::as_ref_id).map(|r| r.0);
                    let paint = self.check_pattern(found, num, forms).map_err(|e| format!("{what}: pattern /{}: {e}", show(name)))?;
                    // a coloured pattern (and a shading pattern) takes the name alone, an uncoloured
                    // one colour components before it
                    let ok = if paint == 2 { operands.len() >= 2 } else { operands.len() == 1 };
                    if !ok {
                        return Err(format!("{what}: content offset {at}: '{}' with {} operands for a pattern of PaintType {paint}", show(op), operands.len()));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// a resource's entry as written (unresolved): res / cat / name
    fn res_entry<'a>(&'a self, res: Option<&'a Dict>, cat: &str, name: &[u8]) -> Option<&'a Obj> {
        res.and_then(|r| self.lookup(r, cat)).and_then(Obj::as_dict).and_then(|c| c.0.iter().find(|(k, _)| k.as_slice() == name)).map(|(_, v)| v)
    }

    /// the number of colour components of a colour space for shadings (8.6): the device spaces,
    /// [/CalGray ..] [/CalRGB ..] [/Lab ..] [/ICCBased stream] [/Separation ..] [/DeviceN names ..]
    /// [/Indexed ..]; a Pattern space is an error
    fn components(&self, cs: Option<&Obj>) -> Result<usize, String> {
        let Some(cs) = cs else { return Err(String::from("no /ColorSpace")) };
        let fam = match cs {
            Obj::Name(n) => n.as_slice(),
            Obj::Array(a) => a.first().map(|f| self.resolve(f)).and_then(Obj::as_name).unwrap_or(b""),
            o => return Err(format!("/ColorSpace is {}", o.kind())),
        };
        let arg = |i: usize| cs.as_array().and_then(|a| a.get(i)).map(|o| self.resolve(o));
        Ok(match fam {
            b"DeviceGray" | b"CalGray" | b"Separation" | b"Indexed" => 1,
            b"DeviceRGB" | b"CalRGB" | b"Lab" => 3,
            b"DeviceCMYK" => 4,
            b"ICCBased" => match arg(1).and_then(Obj::as_dict).and_then(|d| self.lookup(d, "N")).and_then(Obj::as_int) {
                Some(n @ (1 | 3 | 4)) => n as usize,
                n => return Err(format!("/ICCBased /N is {n:?}, not 1, 3 or 4")),
            },
            b"DeviceN" => match arg(1).and_then(Obj::as_array) {
                Some(names) if !names.is_empty() && names.iter().all(|n| self.resolve(n).as_name().is_some()) => names.len(),
                _ => return Err(String::from("/DeviceN without an array of colorant names")),
            },
            b"Pattern" => return Err(String::from("a Pattern colour space where a colour is needed")),
            other => return Err(format!("unknown colour space family /{}", show(other))),
        })
    }

    /// an array of n numbers (n None: any length), each resolved
    fn numbers(&self, o: Option<&Obj>, n: Option<usize>, key: &str) -> Result<Vec<f64>, String> {
        let Some(a) = o.and_then(Obj::as_array) else { return Err(format!("/{key} is not an array")) };
        let v: Vec<f64> = a.iter().filter_map(|x| self.resolve(x).as_num()).collect();
        if v.len() != a.len() {
            return Err(format!("/{key} holds something else than numbers"));
        }
        if n.is_some_and(|n| n != v.len()) {
            return Err(format!("/{key} has {} numbers, not {}", v.len(), n.unwrap_or(0)));
        }
        Ok(v)
    }

    /// a shading (8.7.4.5): ShadingType 1 .. 7 (1 .. 3 dictionaries, 4 .. 7 streams), ColorSpace
    /// (not Pattern), Background (one number per component), BBox (four numbers), AntiAlias (a
    /// boolean); types 2 and 3: Coords (4 numbers; 6 with both radii >= 0), Domain (t0 < t1),
    /// Extend (two booleans) and Function (one input and a value per component, or one function of
    /// one input and output per component); type 1: Function and Domain (four numbers)
    pub fn check_shading(&self, o: &Obj) -> Result<(), String> {
        let d = match o {
            Obj::Dict(d) | Obj::Stream(d, _) => d,
            o => return Err(format!("a shading is {}", o.kind())),
        };
        let ty = match self.lookup(d, "ShadingType") {
            Some(Obj::Int(t)) if (1..=7).contains(t) => *t,
            t => return Err(format!("/ShadingType is {t:?}, not 1 .. 7")),
        };
        if (ty <= 3) != matches!(o, Obj::Dict(_)) {
            return Err(format!("a shading of type {ty} must be a {}", if ty <= 3 { "dictionary" } else { "stream" }));
        }
        let n = self.components(self.lookup(d, "ColorSpace"))?;
        if d.has("Background") {
            self.numbers(self.lookup(d, "Background"), Some(n), "Background")?;
        }
        if d.has("BBox") {
            self.numbers(self.lookup(d, "BBox"), Some(4), "BBox")?;
        }
        if d.has("AntiAlias") && self.lookup(d, "AntiAlias").and_then(Obj::as_bool).is_none() {
            return Err(String::from("/AntiAlias is not a boolean"));
        }
        let indexed = self.lookup(d, "ColorSpace").and_then(Obj::as_array).and_then(|a| a.first()).is_some_and(|f| self.resolve(f).is_name("Indexed"));
        match ty {
            2 | 3 => {
                let c = self.numbers(self.lookup(d, "Coords"), Some(if ty == 2 { 4 } else { 6 }), "Coords")?;
                if ty == 3 && (c[2] < 0.0 || c[5] < 0.0) {
                    return Err(format!("radial /Coords with a negative radius: {c:?}"));
                }
                if d.has("Domain") {
                    let dom = self.numbers(self.lookup(d, "Domain"), Some(2), "Domain")?;
                    if dom[0] >= dom[1] {
                        return Err(format!("/Domain {dom:?} is not t0 < t1"));
                    }
                }
                if d.has("Extend") {
                    let ext = self.lookup(d, "Extend").and_then(Obj::as_array).unwrap_or(&[]);
                    if ext.len() != 2 || !ext.iter().all(|b| self.resolve(b).as_bool().is_some()) {
                        return Err(String::from("/Extend is not an array of two booleans"));
                    }
                }
                if indexed {
                    return Err(String::from("an Indexed colour space with a /Function"));
                }
                self.shading_function(d, 1, n)
            }
            1 => {
                if d.has("Domain") {
                    self.numbers(self.lookup(d, "Domain"), Some(4), "Domain")?;
                }
                self.shading_function(d, 2, n)
            }
            _ => Ok(()),
        }
    }

    /// a shading's /Function: one function of `inputs` inputs and n outputs, or an array of n
    /// functions of one output each
    fn shading_function(&self, d: &Dict, inputs: usize, n: usize) -> Result<(), String> {
        match self.lookup(d, "Function") {
            None => Err(String::from("no /Function")),
            Some(Obj::Array(fs)) => {
                if fs.len() != n {
                    return Err(format!("/Function is an array of {}, the colour space has {n} components", fs.len()));
                }
                for f in fs {
                    let out = self.check_function(f, inputs, 0)?;
                    if out != 1 {
                        return Err(format!("a function of the /Function array has {out} outputs, not 1"));
                    }
                }
                Ok(())
            }
            Some(f) => {
                let out = self.check_function(f, inputs, 0)?;
                if out != n {
                    return Err(format!("/Function has {out} outputs, the colour space {n} components"));
                }
                Ok(())
            }
        }
    }

    /// a function (7.10) with `inputs` inputs -> its number of outputs.  FunctionType 0 (a stream:
    /// Size, BitsPerSample, Range), 2 (a dictionary: C0 and C1 of one length, N a number; a
    /// non-integer N needs a domain without negative values, a negative N one without 0), 3 (a
    /// dictionary: Functions of one input with the same outputs, Bounds k - 1 numbers rising
    /// strictly inside Domain, Encode 2 k numbers), 4 (a stream with Range); Domain always (2 m
    /// numbers, each pair ordered), Range where given (pairs ordered, one per output)
    pub fn check_function(&self, o: &Obj, inputs: usize, depth: usize) -> Result<usize, String> {
        if depth > 16 {
            return Err(String::from("stitching functions nested deeper than 16"));
        }
        let o = self.resolve(o);
        let (Obj::Dict(d) | Obj::Stream(d, _)) = o else { return Err(format!("a function is {}", o.kind())) };
        let ty = match self.lookup(d, "FunctionType") {
            Some(Obj::Int(t @ (0 | 2 | 3 | 4))) => *t,
            t => return Err(format!("/FunctionType is {t:?}, not 0, 2, 3 or 4")),
        };
        if matches!(ty, 0 | 4) != matches!(o, Obj::Stream(..)) {
            return Err(format!("a function of type {ty} must be a {}", if matches!(ty, 0 | 4) { "stream" } else { "dictionary" }));
        }
        let dom = self.numbers(self.lookup(d, "Domain"), Some(2 * inputs), "Domain")?;
        if dom.chunks(2).any(|p| p[0] > p[1]) {
            return Err(format!("/Domain {dom:?} has a pair with its minimum above its maximum"));
        }
        let range = if d.has("Range") {
            let r = self.numbers(self.lookup(d, "Range"), None, "Range")?;
            if r.is_empty() || r.len() % 2 != 0 || r.chunks(2).any(|p| p[0] > p[1]) {
                return Err(format!("/Range {r:?} is not pairs of minimum and maximum"));
            }
            Some(r.len() / 2)
        } else {
            None
        };
        let out = match ty {
            0 => {
                let size = self.lookup(d, "Size").and_then(Obj::as_array).unwrap_or(&[]);
                if size.len() != inputs || !size.iter().all(|s| self.resolve(s).as_int().is_some_and(|v| v > 0)) {
                    return Err(format!("/Size is not {inputs} positive integers"));
                }
                if !self.lookup(d, "BitsPerSample").and_then(Obj::as_int).is_some_and(|b| matches!(b, 1 | 2 | 4 | 8 | 12 | 16 | 24 | 32)) {
                    return Err(String::from("/BitsPerSample is not 1, 2, 4, 8, 12, 16, 24 or 32"));
                }
                range.ok_or_else(|| String::from("a sampled function without /Range"))?
            }
            2 => {
                if inputs != 1 {
                    return Err(format!("an exponential function has one input, not {inputs}"));
                }
                let c0 = if d.has("C0") { self.numbers(self.lookup(d, "C0"), None, "C0")? } else { vec![0.0] };
                let c1 = if d.has("C1") { self.numbers(self.lookup(d, "C1"), None, "C1")? } else { vec![1.0] };
                if c0.len() != c1.len() || c0.is_empty() {
                    return Err(format!("/C0 has {} values, /C1 {}", c0.len(), c1.len()));
                }
                let Some(n) = self.lookup(d, "N").and_then(Obj::as_num) else {
                    return Err(String::from("an exponential function without a number /N"));
                };
                if n != (n as i64) as f64 && dom[0] < 0.0 {
                    return Err(format!("/N {n} is not an integer, but /Domain {dom:?} reaches below 0"));
                }
                if n < 0.0 && dom[0] <= 0.0 && dom[1] >= 0.0 {
                    return Err(format!("/N {n} is negative, but /Domain {dom:?} holds 0"));
                }
                c0.len()
            }
            3 => {
                if inputs != 1 {
                    return Err(format!("a stitching function has one input, not {inputs}"));
                }
                let fs = self.lookup(d, "Functions").and_then(Obj::as_array).unwrap_or(&[]);
                if fs.is_empty() {
                    return Err(String::from("a stitching function without /Functions"));
                }
                let mut out = None;
                for f in fs {
                    let o = self.check_function(f, 1, depth + 1)?;
                    if out.is_some_and(|x| x != o) {
                        return Err(String::from("the /Functions of a stitching function differ in their outputs"));
                    }
                    out = Some(o);
                }
                let k = fs.len();
                let b = self.numbers(self.lookup(d, "Bounds"), Some(k - 1), "Bounds")?;
                if k > 1 && dom[0] >= dom[1] {
                    return Err(format!("/Domain {dom:?} of a stitching function is empty"));
                }
                // rising strictly: an empty subdomain would make its /Encode map divide by zero
                if b.windows(2).any(|w| w[0] >= w[1]) || b.iter().any(|&x| x < dom[0] || x > dom[1]) {
                    return Err(format!("/Bounds {b:?} do not rise strictly inside /Domain {dom:?}"));
                }
                self.numbers(self.lookup(d, "Encode"), Some(2 * k), "Encode")?;
                out.unwrap_or(0)
            }
            _ => range.ok_or_else(|| String::from("a PostScript calculator function without /Range"))?,
        };
        if range.is_some_and(|r| r != out) {
            return Err(format!("/Range has {} pairs, the function {out} outputs", range.unwrap_or(0)));
        }
        Ok(out)
    }

    /// a graphics state parameter dictionary (8.4.5): /Type /ExtGState if given, ca / CA numbers
    /// 0 .. 1, LW >= 0, SMask /None or a soft-mask dictionary (11.6.5.2, Table 144: /Type /Mask if
    /// given, /S /Alpha or /Luminosity, /G an indirect form XObject that is a transparency group
    /// (11.6.6), for Luminosity with a /CS, BC one number per component of it, TR /Identity or a
    /// function of one input and output); the group's content is checked like a form's
    fn check_gstate(&self, o: &Obj, forms: &mut FormWalk) -> Result<(), String> {
        let Obj::Dict(d) = o else { return Err(format!("an ExtGState is {}", o.kind())) };
        if d.has("Type") && !self.lookup(d, "Type").is_some_and(|t| t.is_name("ExtGState")) {
            return Err(String::from("/Type is not /ExtGState"));
        }
        for key in ["ca", "CA"] {
            if d.has(key) && !self.lookup(d, key).and_then(Obj::as_num).is_some_and(|v| (0.0..=1.0).contains(&v)) {
                return Err(format!("/{key} is not a number from 0 to 1"));
            }
        }
        if d.has("LW") && !self.lookup(d, "LW").and_then(Obj::as_num).is_some_and(|v| v >= 0.0) {
            return Err(String::from("/LW is not a number >= 0"));
        }
        let Some(sm) = self.lookup(d, "SMask") else { return Ok(()) };
        let m = match sm {
            Obj::Name(n) if n == b"None" => return Ok(()),
            Obj::Dict(m) => m,
            o => return Err(format!("/SMask is {}, not /None or a soft-mask dictionary", o.kind())),
        };
        if m.has("Type") && !self.lookup(m, "Type").is_some_and(|t| t.is_name("Mask")) {
            return Err(String::from("the soft mask's /Type is not /Mask"));
        }
        let lum = match self.lookup(m, "S").and_then(Obj::as_name) {
            Some(b"Luminosity") => true,
            Some(b"Alpha") => false,
            s => return Err(format!("the soft mask's /S is {s:?}, not /Alpha or /Luminosity")),
        };
        let Some((gnum, _)) = m.get("G").and_then(Obj::as_ref_id) else {
            return Err(String::from("the soft mask's /G is not an indirect reference"));
        };
        let Some(Obj::Stream(g, _)) = self.get(gnum) else {
            return Err(format!("the soft mask's /G (object {gnum}) is not a stream"));
        };
        if !self.lookup(g, "Subtype").is_some_and(|s| s.is_name("Form")) {
            return Err(format!("the soft mask's /G (object {gnum}) is not a form XObject"));
        }
        let Some(grp) = self.lookup(g, "Group").and_then(Obj::as_dict) else {
            return Err(format!("the soft mask's /G (object {gnum}) has no /Group"));
        };
        if !self.lookup(grp, "S").is_some_and(|s| s.is_name("Transparency")) {
            return Err(format!("the soft mask's group (object {gnum}) is not /S /Transparency"));
        }
        if grp.has("CS") || lum {
            let n = self.components(self.lookup(grp, "CS")).map_err(|e| format!("the soft mask's group (object {gnum}): {e}"))?;
            if m.has("BC") {
                self.numbers(self.lookup(m, "BC"), Some(n), "BC")?;
            }
        }
        match self.lookup(m, "TR") {
            None => {}
            Some(Obj::Name(n)) if n == b"Identity" => {}
            Some(f) => {
                if self.check_function(f, 1, 0)? != 1 {
                    return Err(String::from("the soft mask's /TR has more than one output"));
                }
            }
        }
        self.check_form(gnum, None, forms)
    }

    /// a pattern (8.7.3) -> its PaintType (1 for shading patterns).  PatternType 1, tiling: a
    /// stream with PaintType 1 or 2, TilingType 1 .. 3, BBox four numbers with an area, XStep and
    /// YStep numbers other than 0, Resources a dictionary, Matrix six numbers; its content checked
    /// like a form's with those resources (once per object).  PatternType 2, shading: a dictionary
    /// whose Shading check_shading takes, Matrix six numbers, ExtGState checked by check_gstate
    fn check_pattern(&self, o: &Obj, num: Option<u32>, forms: &mut FormWalk) -> Result<i64, String> {
        let (Obj::Dict(d) | Obj::Stream(d, _)) = o else { return Err(format!("a pattern is {}", o.kind())) };
        if d.has("Type") && !self.lookup(d, "Type").is_some_and(|t| t.is_name("Pattern")) {
            return Err(String::from("/Type is not /Pattern"));
        }
        if d.has("Matrix") {
            self.numbers(self.lookup(d, "Matrix"), Some(6), "Matrix")?;
        }
        match self.lookup(d, "PatternType").and_then(Obj::as_int) {
            Some(1) => {
                let Obj::Stream(..) = o else { return Err(String::from("a tiling pattern is not a stream")) };
                let paint = match self.lookup(d, "PaintType").and_then(Obj::as_int) {
                    Some(p @ (1 | 2)) => p,
                    p => return Err(format!("/PaintType is {p:?}, not 1 or 2")),
                };
                if !self.lookup(d, "TilingType").and_then(Obj::as_int).is_some_and(|t| (1..=3).contains(&t)) {
                    return Err(String::from("/TilingType is not 1, 2 or 3"));
                }
                let bb = self.numbers(self.lookup(d, "BBox"), Some(4), "BBox")?;
                if bb[0] == bb[2] || bb[1] == bb[3] {
                    return Err(format!("/BBox {bb:?} has no area"));
                }
                for key in ["XStep", "YStep"] {
                    if !self.lookup(d, key).and_then(Obj::as_num).is_some_and(|v| v != 0.0) {
                        return Err(format!("/{key} is not a number other than 0"));
                    }
                }
                let Some(res) = self.lookup(d, "Resources").and_then(Obj::as_dict) else {
                    return Err(String::from("a tiling pattern without a /Resources dictionary"));
                };
                if let Some(n) = num {
                    if forms.stack.contains(&n) {
                        return Err(format!("tiling pattern object {n} paints itself"));
                    }
                    if forms.done.insert(n) {
                        let data = self.stream_data(o)?;
                        forms.stack.push(n);
                        let r = self.check_drawing(&data, Some(res), &format!("tiling pattern object {n}"), forms);
                        forms.stack.pop();
                        r?;
                    }
                }
                Ok(paint)
            }
            Some(2) => {
                let Obj::Dict(_) = o else { return Err(String::from("a shading pattern is not a dictionary")) };
                let Some(sh) = self.lookup(d, "Shading") else { return Err(String::from("a shading pattern without /Shading")) };
                self.check_shading(sh)?;
                if let Some(gs) = self.lookup(d, "ExtGState") {
                    self.check_gstate(gs, forms)?;
                }
                Ok(1)
            }
            t => Err(format!("/PatternType is {t:?}, not 1 or 2")),
        }
    }
}

/// zlib data -> the inflated bytes; the zlib stream must fill the data exactly (header, deflate
/// blocks, Adler-32, nothing after)
fn zlib_decode(src: &[u8]) -> Result<Vec<u8>, String> {
    let (Some(&cmf), Some(&flg)) = (src.first(), src.get(1)) else {
        return Err(String::from("FlateDecode data shorter than a zlib header"));
    };
    if cmf & 15 != 8 || cmf >> 4 > 7 {
        return Err(format!(
            "zlib header 0x{cmf:02x}: not deflate with a window up to 32 KiB"
        ));
    }
    if (u16::from(cmf) << 8 | u16::from(flg)) % 31 != 0 {
        return Err(String::from("zlib header check bits are wrong"));
    }
    if flg & 0x20 != 0 {
        return Err(String::from("zlib stream with a preset dictionary"));
    }
    let body = src.get(2..).unwrap_or(&[]);
    let mut cap = src.len().saturating_mul(4).clamp(1024, MAX_DECODED);
    loop {
        let mut dst = vec![0u8; cap];
        match img_core::inflate::inflate_raw_ex(body, &mut dst) {
            Ok((n, used)) => {
                dst.truncate(n);
                let rest = body.get(used..).unwrap_or(&[]);
                let Some(want) = rest.get(..4) else {
                    return Err(String::from("zlib stream without its Adler-32 at the end"));
                };
                let want = u32::from_be_bytes([want[0], want[1], want[2], want[3]]);
                if img_core::checksum::adler32(1, &dst) != want {
                    return Err(String::from("zlib Adler-32 mismatch"));
                }
                if rest.len() > 4 {
                    return Err(format!("{} bytes after the end of the zlib stream", rest.len() - 4));
                }
                return Ok(dst);
            }
            Err(img_core::ERR_OVERFLOW) if cap < MAX_DECODED => cap = cap.saturating_mul(2).min(MAX_DECODED),
            Err(img_core::ERR_OVERFLOW) => return Err(String::from("FlateDecode data inflates to more than 256 MiB")),
            Err(e) => return Err(format!("FlateDecode data is not valid deflate data (error {e})")),
        }
    }
}

/// undo the PNG row filters (each row: a filter type byte 0-4, then the row)
fn png_unpredict(data: &[u8], colors: usize, bpc: usize, columns: usize) -> Result<Vec<u8>, String> {
    let row = (colors * bpc).checked_mul(columns).map(|bits| bits.div_ceil(8));
    let Some(row) = row.filter(|&r| r > 0) else {
        return Err(String::from("PNG predictor row size overflows"));
    };
    let bpp = (colors * bpc).div_ceil(8).max(1);
    if !data.len().is_multiple_of(row + 1) {
        return Err(format!(
            "PNG predicted data of {} bytes is not whole rows of 1 + {row} bytes",
            data.len()
        ));
    }
    let mut out: Vec<u8> = Vec::with_capacity(data.len() / (row + 1) * row);
    let mut prev = vec![0u8; row];
    for (r, chunk) in data.chunks_exact(row + 1).enumerate() {
        let ft = chunk[0];
        let src = &chunk[1..];
        let mut cur = vec![0u8; row];
        for i in 0..row {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let b = prev[i];
            let c = if i >= bpp { prev[i - bpp] } else { 0 };
            let x = src[i];
            cur[i] = match ft {
                0 => x,
                1 => x.wrapping_add(a),
                2 => x.wrapping_add(b),
                3 => x.wrapping_add(((u16::from(a) + u16::from(b)) / 2) as u8),
                4 => {
                    let p = i16::from(a) + i16::from(b) - i16::from(c);
                    let (pa, pb, pc) = (
                        (p - i16::from(a)).abs(),
                        (p - i16::from(b)).abs(),
                        (p - i16::from(c)).abs(),
                    );
                    let pred = if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    };
                    x.wrapping_add(pred)
                }
                _ => return Err(format!("PNG predictor row {r}: filter type {ft} is not 0 .. 4")),
            };
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// content streams (ISO 32000-1 7.8, 8.2, Annex A)

/// one operation of a content stream: operands, operator
pub type Op = (Vec<Obj>, Vec<u8>);

/// an operation with the offset of its first token
type OpAt = (usize, Vec<Obj>, Vec<u8>);

/// content stream -> (offset, operands, operator)
fn ops_at(data: &[u8], ctx: &'static str) -> Result<Vec<OpAt>, String> {
    let mut lx = Lexer::new(data, 0, ctx);
    let mut ops = Vec::new();
    let mut operands = Vec::new();
    let mut first = None;
    while let Some((at, t)) = lx.next()? {
        match t {
            Tok::Kw(k) if !matches!(k, b"true" | b"false" | b"null") => {
                if k == b"BI" {
                    return lx.err(at, "inline images (BI ... ID ... EI) are not supported");
                }
                ops.push((first.take().unwrap_or(at), core::mem::take(&mut operands), k.to_vec()));
            }
            t => {
                first.get_or_insert(at);
                operands.push(parse_value(&mut lx, (at, t), 0, false)?);
            }
        }
    }
    if let Some(at) = first {
        return lx.err(at, "operands at the end of the content stream without an operator");
    }
    Ok(ops)
}

/// a content stream -> (operands, operator) per operation; inline images are rejected
pub fn content_ops(data: &[u8]) -> Result<Vec<Op>, String> {
    Ok(ops_at(data, "content offset")?
        .into_iter()
        .map(|(_, a, o)| (a, o))
        .collect())
}

/// check a content stream's grammar (see check_ops)
pub fn check_content(data: &[u8]) -> Result<(), String> {
    check_ops(&ops_at(data, "content offset")?)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cls {
    /// general graphics state: page level and text objects
    Gs,
    /// q Q cm: page level only
    Special,
    /// m re: begin a path at page level, a new subpath inside one
    PathBegin,
    /// l c v y h: inside a path
    PathCons,
    Paint,
    Clip,
    BeginText,
    EndText,
    /// text state: page level and text objects
    TState,
    /// text positioning and showing: text objects only
    TText,
    Color,
    /// sh, Do: page level only
    Draw,
    /// MP DP: page level and text objects
    McPoint,
    McBegin,
    McEnd,
    BeginCompat,
    EndCompat,
    /// d0 d1: the first operator of a Type 3 glyph
    Type3,
}

/// an operator's class and operand signature: n number, / name, s string, a array, D dictionary or
/// name, N 1-32 numbers, P numbers and an optional final name (at least one)
fn op_info(op: &[u8]) -> Option<(Cls, &'static str)> {
    use Cls::*;
    Some(match op {
        b"w" | b"J" | b"j" | b"M" | b"i" => (Gs, "n"),
        b"d" => (Gs, "an"),
        b"ri" | b"gs" => (Gs, "/"),
        b"q" | b"Q" => (Special, ""),
        b"cm" => (Special, "nnnnnn"),
        b"m" => (PathBegin, "nn"),
        b"re" => (PathBegin, "nnnn"),
        b"l" => (PathCons, "nn"),
        b"c" => (PathCons, "nnnnnn"),
        b"v" | b"y" => (PathCons, "nnnn"),
        b"h" => (PathCons, ""),
        b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => (Paint, ""),
        b"W" | b"W*" => (Clip, ""),
        b"BT" => (BeginText, ""),
        b"ET" => (EndText, ""),
        b"Tc" | b"Tw" | b"Tz" | b"TL" | b"Tr" | b"Ts" => (TState, "n"),
        b"Tf" => (TState, "/n"),
        b"Td" | b"TD" => (TText, "nn"),
        b"Tm" => (TText, "nnnnnn"),
        b"T*" => (TText, ""),
        b"Tj" | b"'" => (TText, "s"),
        b"\"" => (TText, "nns"),
        b"TJ" => (TText, "a"),
        b"d0" => (Type3, "nn"),
        b"d1" => (Type3, "nnnnnn"),
        b"CS" | b"cs" => (Color, "/"),
        b"SC" | b"sc" => (Color, "N"),
        b"SCN" | b"scn" => (Color, "P"),
        b"G" | b"g" => (Color, "n"),
        b"RG" | b"rg" => (Color, "nnn"),
        b"K" | b"k" => (Color, "nnnn"),
        b"sh" | b"Do" => (Draw, "/"),
        b"MP" => (McPoint, "/"),
        b"DP" => (McPoint, "/D"),
        b"BMC" => (McBegin, "/"),
        b"BDC" => (McBegin, "/D"),
        b"EMC" => (McEnd, ""),
        b"BX" => (BeginCompat, ""),
        b"EX" => (EndCompat, ""),
        _ => return None,
    })
}

fn sig_ok(sig: &str, a: &[Obj]) -> bool {
    let num = |o: &Obj| o.as_num().is_some();
    match sig {
        "N" => !a.is_empty() && a.len() <= 32 && a.iter().all(num),
        "P" => match a.split_last() {
            Some((last, init)) => a.len() <= 33 && init.iter().all(num) && (num(last) || last.as_name().is_some()),
            None => false,
        },
        _ => {
            sig.len() == a.len()
                && sig.bytes().zip(a).all(|(c, o)| match c {
                    b'n' => num(o),
                    b'/' => o.as_name().is_some(),
                    b's' => o.as_str().is_some(),
                    b'a' => o.as_array().is_some(),
                    b'D' => matches!(o, Obj::Dict(_) | Obj::Name(_)),
                    _ => false,
                })
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Page,
    Path,
    /// after W / W*: a painting operator must follow
    Clip,
    Text,
}

/// the grammar of a tokenized content stream: known operators (unknown ones only inside BX ... EX)
/// with the right operand types, the object structure of Figure 9 (paths end with a painting
/// operator, text operators only between BT and ET, no q / Q / cm / sh / Do / paths inside a text
/// object), q / Q (at most 28 deep), BT / ET, BMC / BDC / EMC (not crossing BT / ET) and BX / EX
/// balanced at the end
fn check_ops(ops: &[OpAt]) -> Result<(), String> {
    let mut level = Level::Page;
    let mut q = 0usize;
    let mut compat = 0usize;
    let mut mc: Vec<bool> = Vec::new(); // per open marked-content sequence: opened inside a text object
    for (i, (at, a, op)) in ops.iter().enumerate() {
        let e = |msg: &str| Err(format!("content offset {at}: '{}': {msg}", show(op)));
        let Some((cls, sig)) = op_info(op) else {
            if compat > 0 {
                continue;
            }
            return e("unknown operator (outside BX ... EX)");
        };
        if !sig_ok(sig, a) {
            return e(&format!("wrong operands ({} given)", a.len()));
        }
        match op.as_slice() {
            b"d" => {
                let dash = a.first().and_then(Obj::as_array).unwrap_or(&[]);
                let vals: Vec<f64> = dash.iter().filter_map(Obj::as_num).collect();
                if vals.len() != dash.len()
                    || vals.iter().any(|&v| v < 0.0)
                    || (!vals.is_empty() && vals.iter().all(|&v| v == 0.0))
                {
                    return e("the dash array must hold non-negative numbers, not all zero");
                }
            }
            b"TJ" => {
                let arr = a.first().and_then(Obj::as_array).unwrap_or(&[]);
                if !arr.iter().all(|x| x.as_num().is_some() || x.as_str().is_some()) {
                    return e("the TJ array holds something else than strings and numbers");
                }
            }
            _ => {}
        }
        let allowed = match (level, cls) {
            (_, Cls::Type3) => i == 0,
            (Level::Page, Cls::PathCons | Cls::Paint | Cls::Clip | Cls::EndText | Cls::TText) => false,
            (Level::Page, _) => true,
            (Level::Path, Cls::PathBegin | Cls::PathCons | Cls::Paint | Cls::Clip) => true,
            (Level::Path, _) => false,
            (Level::Clip, Cls::Paint) => true,
            (Level::Clip, _) => false,
            (Level::Text, Cls::Gs | Cls::TState | Cls::TText | Cls::Color | Cls::EndText) => true,
            (Level::Text, Cls::McPoint | Cls::McBegin | Cls::McEnd | Cls::BeginCompat | Cls::EndCompat) => true,
            (Level::Text, _) => false,
        };
        if !allowed {
            let at_level = match level {
                Level::Page => "outside a path or text object",
                Level::Path => "inside a path (before its painting operator)",
                Level::Clip => "after W / W* (a painting operator must follow)",
                Level::Text => "inside a text object (BT ... ET)",
            };
            if cls == Cls::Type3 {
                return e("d0 / d1 must be the first operator");
            }
            return e(&format!("not allowed {at_level}"));
        }
        match cls {
            Cls::PathBegin if level == Level::Page => level = Level::Path,
            Cls::Clip => level = Level::Clip,
            Cls::Paint => level = Level::Page,
            Cls::BeginText => level = Level::Text,
            Cls::EndText => {
                if mc.last() == Some(&true) {
                    return e("a marked-content sequence begun inside the text object is still open");
                }
                level = Level::Page;
            }
            Cls::McBegin => mc.push(level == Level::Text),
            Cls::McEnd => match mc.pop() {
                None => return e("EMC without BMC / BDC"),
                Some(in_text) if in_text != (level == Level::Text) => {
                    return e("the marked-content sequence crosses a BT / ET boundary");
                }
                Some(_) => {}
            },
            Cls::BeginCompat => compat += 1,
            Cls::EndCompat => {
                if compat == 0 {
                    return e("EX without BX");
                }
                compat -= 1;
            }
            Cls::Special if op.as_slice() == b"q" => {
                q += 1;
                if q > MAX_Q_DEPTH {
                    return e(&format!("q nested deeper than {MAX_Q_DEPTH}"));
                }
            }
            Cls::Special if op.as_slice() == b"Q" => {
                if q == 0 {
                    return e("Q without q");
                }
                q -= 1;
            }
            _ => {}
        }
    }
    let end = ops.last().map_or(0, |o| o.0);
    let open = match level {
        Level::Page => None,
        Level::Path | Level::Clip => Some("a path without a painting operator"),
        Level::Text => Some("BT without ET"),
    };
    if let Some(m) = open {
        return Err(format!("content offset {end}: the stream ends inside {m}"));
    }
    if q != 0 {
        return Err(format!(
            "content offset {end}: {q} q without Q at the end of the stream"
        ));
    }
    if !mc.is_empty() {
        return Err(format!(
            "content offset {end}: {} BMC / BDC without EMC at the end of the stream",
            mc.len()
        ));
    }
    if compat != 0 {
        return Err(format!("content offset {end}: BX without EX at the end of the stream"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// ToUnicode CMaps (ISO 32000-1 9.10.3, Adobe TN 5014 / 5411)

/// UTF-16BE (with surrogate pairs) -> text
fn utf16be(s: &[u8]) -> Option<String> {
    if s.is_empty() || !s.len().is_multiple_of(2) {
        return None;
    }
    let units = s.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c));
    core::char::decode_utf16(units).collect::<Result<String, _>>().ok()
}

/// a ToUnicode CMap -> code -> text, from beginbfchar (src dst) and beginbfrange (lo hi dst and
/// lo hi [dst ...]); checks the codespace ranges (codes must fall in one, with its byte length),
/// block counts (0 .. 100, matching the entries), ranges varying only in the last byte, no code
/// mapped twice, destinations valid UTF-16BE, begincmap / endcmap, and no unknown operator
pub fn cmap_map(data: &[u8]) -> Result<BTreeMap<u32, String>, String> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Block {
        Codespace,
        Char,
        Range,
    }
    let ops = ops_at(data, "cmap offset")?;
    let mut map: BTreeMap<u32, String> = BTreeMap::new();
    let mut space: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut began = false;
    let mut ended = false;
    let mut open: Option<(Block, usize)> = None;
    let in_space = |code: &[u8], space: &[(Vec<u8>, Vec<u8>)]| {
        space.iter().any(|(lo, hi)| {
            lo.len() == code.len() && code.iter().zip(lo.iter().zip(hi)).all(|(c, (l, h))| l <= c && c <= h)
        })
    };
    let code_of = |c: &[u8]| c.iter().fold(0u32, |v, &b| v << 8 | u32::from(b));
    for (at, a, op) in &ops {
        let e = |msg: &str| Err(format!("cmap offset {at}: '{}': {msg}", show(op)));
        let block = match op.as_slice() {
            b"begincodespacerange" | b"endcodespacerange" => Some(Block::Codespace),
            b"beginbfchar" | b"endbfchar" => Some(Block::Char),
            b"beginbfrange" | b"endbfrange" => Some(Block::Range),
            _ => None,
        };
        match op.as_slice() {
            b"begincmap" => {
                if began {
                    return e("a second begincmap");
                }
                began = true;
            }
            b"endcmap" => {
                if !began || ended || open.is_some() {
                    return e("endcmap without begincmap, twice or inside a block");
                }
                ended = true;
            }
            b"begincodespacerange" | b"beginbfchar" | b"beginbfrange" => {
                if !began || ended {
                    return e("outside begincmap ... endcmap");
                }
                if open.is_some() {
                    return e("inside another block");
                }
                let n = match a.as_slice() {
                    [Obj::Int(n)] if (0..=MAX_CMAP_BLOCK).contains(n) => *n as usize,
                    _ => return e(&format!("needs one entry count of 0 .. {MAX_CMAP_BLOCK}")),
                };
                if block != Some(Block::Codespace) && space.is_empty() {
                    return e("before begincodespacerange");
                }
                open = Some((block.unwrap_or(Block::Char), n));
            }
            b"endcodespacerange" | b"endbfchar" | b"endbfrange" => {
                let Some((b, n)) = open.take().filter(|(b, _)| Some(*b) == block) else {
                    return e("without its begin operator");
                };
                let per = if b == Block::Range { 3 } else { 2 };
                if a.len() != n * per {
                    return e(&format!("{} operands, the block count {n} needs {}", a.len(), n * per));
                }
                for ent in a.chunks_exact(per) {
                    let (Some(lo), Some(hi_or_dst)) = (ent[0].as_str(), ent[1].as_str()) else {
                        return e("source codes must be hex strings");
                    };
                    match b {
                        Block::Codespace => {
                            let hi = hi_or_dst;
                            if lo.is_empty()
                                || lo.len() > 4
                                || lo.len() != hi.len()
                                || lo.iter().zip(hi).any(|(l, h)| l > h)
                            {
                                return e("codespace range: 1-4 bytes, the same length, low <= high per byte");
                            }
                            space.push((lo.to_vec(), hi.to_vec()));
                        }
                        Block::Char => {
                            if !in_space(lo, &space) {
                                return e(&format!(
                                    "code <{}> is outside the codespace ranges (compared byte by byte)",
                                    hex(lo)
                                ));
                            }
                            let Some(t) = utf16be(hi_or_dst) else {
                                return e(&format!("destination of <{}> is not UTF-16BE", hex(lo)));
                            };
                            if map.insert(code_of(lo), t).is_some() {
                                return e(&format!("code <{}> mapped twice", hex(lo)));
                            }
                        }
                        Block::Range => {
                            let hi = hi_or_dst;
                            if !in_space(lo, &space) || !in_space(hi, &space) || lo.len() != hi.len() {
                                return e(&format!(
                                    "range <{}> <{}> is outside the codespace ranges (compared byte by byte)",
                                    hex(lo),
                                    hex(hi)
                                ));
                            }
                            let k = lo.len() - 1;
                            if lo[..k] != hi[..k] || lo[k] > hi[k] {
                                return e(&format!(
                                    "range <{}> <{}> must vary only in its last byte, upwards",
                                    hex(lo),
                                    hex(hi)
                                ));
                            }
                            let count = usize::from(hi[k] - lo[k]) + 1;
                            let base = code_of(lo);
                            let mut put = |i: usize, t: String| {
                                if map.insert(base + i as u32, t).is_some() {
                                    return Err(format!("cmap offset {at}: code 0x{:x} mapped twice", base + i as u32));
                                }
                                Ok(())
                            };
                            match &ent[2] {
                                Obj::Str(d) => {
                                    let Some(&last) = d.last() else {
                                        return e("empty destination");
                                    };
                                    if usize::from(last) + count - 1 > 255 {
                                        return e(&format!(
                                            "destination <{}> would step its last byte past 255",
                                            hex(d)
                                        ));
                                    }
                                    let mut d = d.clone();
                                    let dl = d.len() - 1;
                                    for i in 0..count {
                                        d[dl] = last + i as u8;
                                        let Some(t) = utf16be(&d) else {
                                            return e(&format!("destination <{}> is not UTF-16BE", hex(&d)));
                                        };
                                        put(i, t)?;
                                    }
                                }
                                Obj::Array(v) => {
                                    if v.len() != count {
                                        return e(&format!("{} destinations for {count} codes", v.len()));
                                    }
                                    for (i, x) in v.iter().enumerate() {
                                        let Some(t) = x.as_str().and_then(utf16be) else {
                                            return e("a destination in the array is not a UTF-16BE string");
                                        };
                                        put(i, t)?;
                                    }
                                }
                                _ => return e("the destination is not a string or an array"),
                            }
                        }
                    }
                }
            }
            b"usecmap" => return e("usecmap is not supported"),
            // the PostScript frame around the mappings (CMapName: the executable name in
            // "CMapName currentdict /CMap defineresource pop"; dup: "/CIDSystemInfo 3 dict dup begin")
            b"findresource" | b"begin" | b"dict" | b"dup" | b"def" | b"end" | b"currentdict" | b"defineresource"
            | b"pop" | b"CMapName" => {
                if open.is_some() {
                    return e("inside a mapping block");
                }
            }
            _ => return e("unknown operator in a CMap"),
        }
    }
    let end = ops.last().map_or(0, |o| o.0);
    if open.is_some() {
        return Err(format!("cmap offset {end}: a block is not closed"));
    }
    if !began || !ended {
        return Err(format!("cmap offset {end}: no begincmap ... endcmap"));
    }
    if space.is_empty() {
        return Err(format!("cmap offset {end}: no codespace range"));
    }
    Ok(map)
}

fn hex(b: &[u8]) -> String {
    let mut s = String::new();
    for &c in b.iter().take(32) {
        s.push_str(&format!("{c:02X}"));
    }
    s
}
