// ini -- settings files: UTF-8 "[section]" / "key=value" text              [src/core/ini.s]
//
//   Ini::new() / Ini::load(path) / Ini::parse(bytes)    a missing or unreadable file is empty
//   ini.save(path) -> bool       plat::write_file, creates missing directories
//   ini.to_bytes()               the text save writes
//   ini.get(sec, key) -> Option<&str>   (get_bytes for values that are not UTF-8)
//   ini.get_int(sec, key, def)   decimal (+/-), 0x hex, true/false/yes/no/on/off; else def
//   ini.get_f64(sec, key, def)   '.' decimal point only; else def
//   ini.set(sec, key, value) / set_int / set_f64 -> bool   add or replace (false: bad name)
//   ini.remove(sec, key) -> bool
//   ini.section(sec) -> Vec<(&str, &str)>   the keys of a section and their values, in order
//   ini.sections() -> Vec<&str>   the section names in file order (each once; not "")
//
// Format: one entry per line; sec "" = the keys before the first [section].  Section and
// key names compare ASCII case-insensitively; values are stored as written (blanks around
// names and values trimmed; no quoting, no inline comments, so values may contain ';' and
// '#').  Lines starting with ';' or '#', blank lines and malformed lines (no '=', "[" without
// "]", NUL bytes, longer than LINE_MAX) are kept verbatim and written back unchanged, as are
// all lines not touched by set (the original spacing survives).  New keys go after the
// last key of their section, new sections to the end.  A UTF-8 BOM is skipped.  Files are
// read up to FILE_MAX bytes; after LINES_MAX lines the rest is kept as one verbatim block.
// Line ends: LF, CRLF (any number of CRs before the LF are dropped) when reading; written as
// CRLF on Windows, LF on Linux.  Key names must not contain '=', CR or LF nor start with
// ';' '#' '['; section names must not contain ']', CR or LF; neither may start or end with
// a blank (set -> false).  CR / LF inside a value are written as blanks.

use alloc::format;
use alloc::vec::Vec;

use crate::num::{f64_format, f64_parse, parse_i64};

const FILE_MAX: u64 = 0x100_0000; // 16 MB read at most
const LINE_MAX: usize = 0x10000; // longer lines are kept verbatim, not interpreted
const LINES_MAX: usize = 0x40000;
#[cfg(windows)]
const EOL: &[u8] = b"\r\n";
#[cfg(not(windows))]
const EOL: &[u8] = b"\n";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Raw, // blank, comment, malformed
    Sec,
    Kv,
}

struct Line {
    kind: Kind,
    dirty: bool, // written as key=value / [name] instead of the raw text
    sec: i32,    // line index of the section header, -1 = before any header
    raw: Vec<u8>, // the line without its end
    name: Vec<u8>, // section name / key
    value: Vec<u8>,
}

impl Line {
    fn new(kind: Kind, sec: i32) -> Line {
        Line { kind, dirty: false, sec, raw: Vec::new(), name: Vec::new(), value: Vec::new() }
    }
}

/// a settings object (one thread at a time)
#[derive(Default)]
pub struct Ini {
    lines: Vec<Line>,
}

/// n bytes equal ignoring ASCII case (only letters may differ in case)  [ieq]
fn ieq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(&x, &y)| x == y || (x | 32 == y | 32 && (x | 32).wrapping_sub(b'a') <= 25))
}

fn blank(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

/// usable as a key (is_key) or section name  [name_ok]
fn name_ok(s: &[u8], is_key: bool) -> bool {
    let (Some(&first), Some(&last)) = (s.first(), s.last()) else { return false };
    if blank(first) || blank(last) || (is_key && matches!(first, b';' | b'#' | b'[')) {
        return false;
    }
    let forbid = if is_key { b'=' } else { b']' };
    !s.iter().any(|&c| c == b'\r' || c == b'\n' || c == forbid)
}

fn trim(mut s: &[u8]) -> &[u8] {
    while let [c, rest @ ..] = s {
        if !blank(*c) {
            break;
        }
        s = rest;
    }
    while let [rest @ .., c] = s {
        if !blank(*c) {
            break;
        }
        s = rest;
    }
    s
}

impl Ini {
    pub fn new() -> Ini {
        Ini { lines: Vec::new() }
    }

    /// the parsed file; empty when it is missing or unreadable  [ini_load]
    pub fn load(path: &str) -> Ini {
        let Some(f) = plat::File::open(path, plat::FO_READ) else { return Ini::new() };
        let size = f.size().unwrap_or(0).min(FILE_MAX) as usize;
        if size == 0 {
            return Ini::new();
        }
        let mut data = alloc::vec![0u8; size];
        let got = f.read_at(0, &mut data).unwrap_or(0);
        drop(f);
        Ini::parse(&data[..got])
    }

    /// parse settings text  [ini_parse]
    pub fn parse(data: &[u8]) -> Ini {
        let mut ini = Ini::new();
        let mut pos = if data.starts_with(&[0xEF, 0xBB, 0xBF]) { 3 } else { 0 };
        let end = data.len();
        let mut cur: i32 = -1;
        while pos < end {
            if ini.lines.len() >= LINES_MAX - 1 {
                // too many lines: the rest as one verbatim block (without its last line end)
                let mut e = end;
                if data[e - 1] == b'\n' {
                    e -= 1;
                }
                while e > pos && data[e - 1] == b'\r' {
                    e -= 1;
                }
                ini.classify(&data[pos..e], true, &mut cur);
                break;
            }
            let lf = data[pos..].iter().position(|&c| c == b'\n').map_or(end, |k| pos + k);
            let nul = data[pos..lf].contains(&0);
            let mut e = lf;
            while e > pos && data[e - 1] == b'\r' {
                e -= 1; // every CR before the LF: a kept one would read back differently
            }
            ini.classify(&data[pos..e], nul, &mut cur);
            pos = lf + 1;
        }
        ini
    }

    /// append one line  [classify]
    fn classify(&mut self, raw: &[u8], verbatim: bool, cur: &mut i32) {
        let mut ln = Line::new(Kind::Raw, *cur);
        ln.raw = raw.to_vec();
        let t = trim(raw);
        if !verbatim && raw.len() <= LINE_MAX && !t.is_empty() && t[0] != b';' && t[0] != b'#' {
            if t[0] == b'[' {
                if let Some(close) = t.iter().position(|&c| c == b']') {
                    let after = trim(&t[close + 1..]);
                    let name = trim(&t[1..close]);
                    if (after.is_empty() || after[0] == b';' || after[0] == b'#') && !name.is_empty() {
                        ln.kind = Kind::Sec;
                        ln.name = name.to_vec();
                        ln.sec = self.lines.len() as i32;
                        *cur = ln.sec; // following keys belong here
                    }
                }
            } else if let Some(eq) = t.iter().position(|&c| c == b'=') {
                let key = trim(&t[..eq]);
                if !key.is_empty() {
                    ln.kind = Kind::Kv;
                    ln.name = key.to_vec();
                    ln.value = trim(&t[eq + 1..]).to_vec();
                }
            }
        }
        self.lines.push(ln);
    }

    /// the text save writes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for l in &self.lines {
            if !l.dirty {
                out.extend_from_slice(&l.raw);
            } else if l.kind == Kind::Sec {
                out.push(b'[');
                out.extend_from_slice(&l.name);
                out.push(b']');
            } else {
                out.extend_from_slice(&l.name);
                out.push(b'=');
                out.extend_from_slice(&l.value);
            }
            out.extend_from_slice(EOL);
        }
        out
    }

    /// Write through plat::write_file; missing directories are created. [ini_save]
    pub fn save(&self, path: &str) -> bool {
        let text = self.to_bytes();
        mkparent(path);
        plat::write_file(path, &text)
    }

    /// the line of the key or None  [find_kv]
    fn find_kv(&self, sec: &str, key: &str) -> Option<usize> {
        let (sec, key) = (sec.as_bytes(), key.as_bytes());
        self.lines.iter().position(|l| {
            l.kind == Kind::Kv
                && ieq(&l.name, key)
                && if l.sec < 0 { sec.is_empty() } else { !sec.is_empty() && ieq(&self.lines[l.sec as usize].name, sec) }
        })
    }

    /// the first header named sec  [find_sec]
    fn find_sec(&self, sec: &str) -> Option<usize> {
        self.lines.iter().position(|l| l.kind == Kind::Sec && ieq(&l.name, sec.as_bytes()))
    }

    /// where a new key of section line s (-1 = global) goes: after its last key, else right
    /// after the header (global: before the first header)  [sec_end]
    fn sec_end(&self, s: i32) -> usize {
        let mut after = None;
        let mut first_hdr = None;
        for (i, l) in self.lines.iter().enumerate() {
            if l.kind == Kind::Sec {
                first_hdr.get_or_insert(i);
            } else if l.kind == Kind::Kv && l.sec == s {
                after = Some(i + 1);
            }
        }
        match after {
            Some(a) => a,
            None if s >= 0 => s as usize + 1,
            None => first_hdr.unwrap_or(self.lines.len()),
        }
    }

    /// insert a line; section references at or behind pos move with their header
    fn line_insert(&mut self, pos: usize, ln: Line) {
        for l in self.lines.iter_mut() {
            if l.sec >= pos as i32 {
                l.sec += 1;
            }
        }
        self.lines.insert(pos, ln);
    }

    /// remove a line (references to it become -1, those behind it move down)
    fn line_delete(&mut self, pos: usize) {
        self.lines.remove(pos);
        for l in self.lines.iter_mut() {
            if l.sec == pos as i32 {
                l.sec = -1;
            } else if l.sec > pos as i32 {
                l.sec -= 1;
            }
        }
    }

    /// every key of a section with its value, in file order (entries that are not UTF-8 are
    /// left out; sec "" = the keys before the first header)
    pub fn section(&self, sec: &str) -> Vec<(&str, &str)> {
        let s = sec.as_bytes();
        self.lines
            .iter()
            .filter(|l| l.kind == Kind::Kv && if l.sec < 0 { s.is_empty() } else { !s.is_empty() && ieq(&self.lines[l.sec as usize].name, s) })
            .filter_map(|l| Some((core::str::from_utf8(&l.name).ok()?, core::str::from_utf8(&l.value).ok()?)))
            .collect()
    }

    /// the names of the sections in file order, each once (names that are not UTF-8 are left
    /// out; the keys before the first header are not a section here)
    pub fn sections(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for l in self.lines.iter().filter(|l| l.kind == Kind::Sec) {
            if let Ok(n) = core::str::from_utf8(&l.name) {
                if !out.iter().any(|o| ieq(o.as_bytes(), n.as_bytes())) {
                    out.push(n);
                }
            }
        }
        out
    }

    /// the value, or None when missing or not UTF-8  [ini_get]
    pub fn get(&self, sec: &str, key: &str) -> Option<&str> {
        core::str::from_utf8(self.get_bytes(sec, key)?).ok()
    }

    pub fn get_bytes(&self, sec: &str, key: &str) -> Option<&[u8]> {
        self.find_kv(sec, key).map(|i| self.lines[i].value.as_slice())
    }

    /// [ini_get_int]
    pub fn get_int(&self, sec: &str, key: &str, def: i64) -> i64 {
        self.get_bytes(sec, key).and_then(parse_i64).unwrap_or(def)
    }

    /// the whole value must be the number  [ini_get_f64]
    pub fn get_f64(&self, sec: &str, key: &str, def: f64) -> f64 {
        match self.get_bytes(sec, key) {
            Some(v) => match f64_parse(v) {
                (x, used) if used > 0 && used == v.len() => x,
                _ => def,
            },
            None => def,
        }
    }

    /// add or replace (false: a bad name)  [ini_set]
    pub fn set(&mut self, sec: &str, key: &str, value: &str) -> bool {
        if !name_ok(key.as_bytes(), true) || (!sec.is_empty() && !name_ok(sec.as_bytes(), false)) {
            return false;
        }
        let v: Vec<u8> = value.bytes().map(|c| if c == b'\r' || c == b'\n' { b' ' } else { c }).collect();
        if let Some(i) = self.find_kv(sec, key) {
            let l = &mut self.lines[i];
            l.value = v;
            l.dirty = true;
            return true;
        }
        let mut s: i32 = -1; // section line of the new key
        if !sec.is_empty() {
            s = match self.find_sec(sec) {
                Some(i) => i as i32,
                None => {
                    // a new section at the end, after a blank line
                    let n = self.lines.len();
                    if n > 0 && !(self.lines[n - 1].kind == Kind::Raw && self.lines[n - 1].raw.is_empty()) {
                        self.line_insert(n, Line::new(Kind::Raw, -1));
                    }
                    let n = self.lines.len();
                    let mut h = Line::new(Kind::Sec, n as i32);
                    h.dirty = true;
                    h.name = sec.as_bytes().to_vec();
                    self.line_insert(n, h);
                    n as i32
                }
            };
        }
        let pos = self.sec_end(s);
        let mut k = Line::new(Kind::Kv, s);
        k.dirty = true;
        k.name = key.as_bytes().to_vec();
        k.value = v;
        self.line_insert(pos, k);
        true
    }

    /// [ini_set_int]
    pub fn set_int(&mut self, sec: &str, key: &str, value: i64) -> bool {
        self.set(sec, key, &format!("{value}"))
    }

    /// the shortest text that reads back to the same double  [ini_set_f64]
    pub fn set_f64(&mut self, sec: &str, key: &str, value: f64) -> bool {
        self.set(sec, key, &f64_format(value))
    }

    /// false: not there  [ini_remove]
    pub fn remove(&mut self, sec: &str, key: &str) -> bool {
        match self.find_kv(sec, key) {
            Some(i) => {
                self.line_delete(i);
                true
            }
            None => false,
        }
    }
}

/// create the directory part of path (if any)  [mkparent]
fn mkparent(path: &str) {
    let b = path.as_bytes();
    let Some(sep) = b.iter().rposition(|&c| c == b'/' || c == b'\\') else { return };
    if sep == 0 || (sep == 2 && b[1] == b':') {
        return; // "/name", "C:/name"
    }
    plat::mkdir(&path[..sep]);
}
