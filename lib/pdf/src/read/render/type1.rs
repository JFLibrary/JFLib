// Type 1 font programs (FontFile; Adobe Type 1 Font Format): the clear text (FontMatrix, the
// built-in Encoding), the eexec-encrypted private part (hex or binary, PFB segment headers
// removed) with /lenIV, /Subrs and /CharStrings, and the charstrings as outlines: hsbw / sbw,
// the path operators, closepath, callsubr / return, the OtherSubrs of flex (0, 1, 2) and hint
// replacement (3) as the standard ones do them, seac (base and accent by StandardEncoding
// name), div, pop, setcurrentpoint; hints are skipped.  Bounded: subroutine nesting, the
// operators of one glyph, the size of the stacks.

use super::super::lex::{self, Lexer, Tok};
use super::super::tables;
use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use text::outline::{Outline, OLT_CUBIC, OLT_ON};

const MAX_SUBRS: usize = 1 << 16;
const MAX_GLYPHS: usize = 1 << 16;
const DEPTH: u32 = 10;
const BUDGET: i32 = 100_000;

pub(super) struct Type1 {
    data: Vec<u8>,
    len_iv: i32,
    /// the subroutines: ranges in `data` while parsing, then in `subr_data`, decrypted once (a
    /// glyph may call a long one thousands of times; each call copied and decrypted it)
    subrs: Vec<(usize, usize)>,
    subr_data: Vec<u8>,
    glyphs: BTreeMap<Vec<u8>, (usize, usize)>,
    pub matrix: [f64; 6],
    /// the built-in encoding (None: StandardEncoding)
    pub encoding: Option<Vec<Option<Vec<u8>>>>,
}

fn decrypt(data: &[u8], r0: u16) -> Vec<u8> {
    let mut r = r0;
    let (c1, c2) = (52845u16, 22719u16);
    data.iter()
        .map(|&c| {
            let p = c ^ (r >> 8) as u8;
            r = (c as u16).wrapping_add(r).wrapping_mul(c1).wrapping_add(c2);
            p
        })
        .collect()
}

fn hexval(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// the program without PFB segment headers (0x80 type length)
fn strip_pfb(d: &[u8]) -> Vec<u8> {
    if d.first() != Some(&0x80) {
        return d.to_vec();
    }
    let mut out = Vec::new();
    let mut p = 0usize;
    while p + 6 <= d.len() && d[p] == 0x80 && (d[p + 1] == 1 || d[p + 1] == 2) {
        let n = u32::from_le_bytes([d[p + 2], d[p + 3], d[p + 4], d[p + 5]]) as usize;
        let end = (p + 6).saturating_add(n).min(d.len());
        out.extend_from_slice(&d[p + 6..end]);
        p = end;
    }
    out
}

impl Type1 {
    pub fn parse(prog: &[u8]) -> Option<Type1> {
        let prog = strip_pfb(prog);
        let at = lex::find(&prog, b"eexec", 0)?;
        let clear = &prog[..at];
        let mut p = at + 5;
        while p < prog.len() && matches!(prog[p], b' ' | b'\r' | b'\n' | b'\t') {
            p += 1;
        }
        let rest = &prog[p..];
        // hex when the first four bytes are hex digits
        let enc: Vec<u8> = if rest.len() >= 4 && rest[..4].iter().all(|&c| hexval(c).is_some()) {
            let mut out = Vec::with_capacity(rest.len() / 2);
            let mut hi: Option<u8> = None;
            for &c in rest {
                let Some(v) = hexval(c) else {
                    if matches!(c, b' ' | b'\r' | b'\n' | b'\t') {
                        continue;
                    }
                    break;
                };
                match hi.take() {
                    Some(h) => out.push(h << 4 | v),
                    None => hi = Some(v),
                }
            }
            out
        } else {
            rest.to_vec()
        };
        let data = decrypt(&enc, 55665);
        let data = data.get(4..)?.to_vec();
        let mut t = Type1 { data, len_iv: 4, subrs: Vec::new(), subr_data: Vec::new(), glyphs: BTreeMap::new(), matrix: [0.001, 0.0, 0.0, 0.001, 0.0, 0.0], encoding: None };
        t.clear_text(clear);
        t.private();
        (!t.glyphs.is_empty()).then_some(t)
    }

    fn clear_text(&mut self, clear: &[u8]) {
        if let Some(at) = lex::find(clear, b"/FontMatrix", 0) {
            let mut lx = Lexer::new(clear, at + 11);
            let mut v = Vec::new();
            if matches!(lx.next(), Tok::ArrOpen | Tok::BraceOpen) {
                while v.len() < 6 {
                    match lx.next() {
                        Tok::Int(i) => v.push(i as f64),
                        Tok::Real(r) => v.push(r),
                        _ => break,
                    }
                }
            }
            if v.len() == 6 && v.iter().all(|x| x.is_finite()) && (v[0] * v[3] - v[1] * v[2]).abs() > 1e-12 {
                self.matrix = [v[0], v[1], v[2], v[3], v[4], v[5]];
            }
        }
        if let Some(at) = lex::find(clear, b"/Encoding", 0) {
            let rest = &clear[at + 9..];
            let head = &rest[..rest.len().min(40)];
            if lex::find(head, b"StandardEncoding", 0).is_some() {
                return;
            }
            let mut enc = vec![None; 256];
            let mut lx = Lexer::new(rest, 0);
            let mut any = false;
            loop {
                match lx.next() {
                    Tok::Kw(b"dup") => {
                        if let (Tok::Int(code), Tok::Name(n)) = (lx.next(), lx.next()) {
                            if (0..256).contains(&code) {
                                enc[code as usize] = Some(n);
                                any = true;
                            }
                        }
                    }
                    Tok::Kw(b"readonly") | Tok::Kw(b"def") if any => break,
                    Tok::Eof => break,
                    _ => {}
                }
                lx.reset_budget();
            }
            if any {
                self.encoding = Some(enc);
            }
        }
    }

    /// a binary run after "n RD " (or "-|"): its start and end
    fn binary(d: &[u8], lx: &mut Lexer, n: i64) -> Option<(usize, usize)> {
        let kw = lx.next();
        if !matches!(kw, Tok::Kw(_)) {
            return None;
        }
        let start = lx.pos + 1;
        let end = start.checked_add(usize::try_from(n).ok()?)?;
        if end > d.len() {
            return None;
        }
        lx.pos = end;
        Some((start, end))
    }

    fn private(&mut self) {
        let d = core::mem::take(&mut self.data);
        if let Some(at) = lex::find(&d, b"/lenIV", 0) {
            if let Tok::Int(v) = Lexer::new(&d, at + 6).next() {
                self.len_iv = v.clamp(-1, 64) as i32;
            }
        }
        if let Some(at) = lex::find(&d, b"/Subrs", 0) {
            let mut lx = Lexer::new(&d, at + 6);
            if let Tok::Int(count) = lx.next() {
                let count = (count.max(0) as usize).min(MAX_SUBRS);
                self.subrs = vec![(0, 0); count];
                let mut seen = 0;
                while seen < count {
                    lx.reset_budget();
                    match lx.next() {
                        Tok::Kw(b"dup") => {
                            let (Tok::Int(i), Tok::Int(n)) = (lx.next(), lx.next()) else { continue };
                            let Some(r) = Type1::binary(&d, &mut lx, n) else { break };
                            if (0..count as i64).contains(&i) {
                                self.subrs[i as usize] = r;
                            }
                            seen += 1;
                        }
                        Tok::Eof => break,
                        Tok::Name(n) if n == b"CharStrings" => break,
                        _ => {}
                    }
                }
            }
        }
        if let Some(at) = lex::find(&d, b"/CharStrings", 0) {
            let mut lx = Lexer::new(&d, at + 12);
            loop {
                lx.reset_budget();
                match lx.next() {
                    Tok::Name(name) => {
                        let Tok::Int(n) = lx.next() else { continue };
                        let Some(r) = Type1::binary(&d, &mut lx, n) else { break };
                        if self.glyphs.len() < MAX_GLYPHS {
                            self.glyphs.entry(name).or_insert(r);
                        }
                    }
                    Tok::Eof => break,
                    Tok::Kw(b"end") if !self.glyphs.is_empty() => break,
                    _ => {}
                }
            }
        }
        self.data = d;
        let mut dec = Vec::new();
        for i in 0..self.subrs.len() {
            let cs = self.charstring(self.subrs[i]);
            let s = dec.len();
            dec.extend_from_slice(&cs);
            self.subrs[i] = (s, dec.len());
        }
        self.subr_data = dec;
    }

    pub fn has(&self, name: &[u8]) -> bool {
        self.glyphs.contains_key(name)
    }

    fn charstring(&self, r: (usize, usize)) -> Vec<u8> {
        let raw = &self.data[r.0..r.1];
        if self.len_iv < 0 {
            return raw.to_vec();
        }
        let dec = decrypt(raw, 4330);
        dec.get(self.len_iv as usize..).map(<[u8]>::to_vec).unwrap_or_default()
    }

    /// the outline of a glyph in glyph space (before FontMatrix), its advance
    pub fn outline(&self, name: &[u8], ol: &mut Outline) -> bool {
        ol.reset();
        let Some(&r) = self.glyphs.get(name) else { return false };
        let mut st = St { t: self, stack: Vec::new(), ps: Vec::new(), x: 0.0, y: 0.0, sbx: 0.0, open: false, flex: None, flex_start: (0.0, 0.0), budget: BUDGET, ol, seac_depth: 0, dx: 0.0, dy: 0.0 };
        let cs = self.charstring(r);
        let ok = st.run(&cs, 0).is_some();
        if st.open {
            st.ol.close();
        }
        ok && !st.ol.err
    }
}

struct St<'a, 'o> {
    t: &'a Type1,
    stack: Vec<f64>,
    /// the results of an OtherSubr for pop
    ps: Vec<f64>,
    x: f64,
    y: f64,
    sbx: f64,
    open: bool,
    /// flex: the points collected after OtherSubr 1
    flex: Option<Vec<(f64, f64)>>,
    flex_start: (f64, f64),
    budget: i32,
    ol: &'o mut Outline,
    seac_depth: u32,
    /// where the glyph is moved (an accent of seac)
    dx: f64,
    dy: f64,
}

impl St<'_, '_> {
    fn pt(&mut self, tag: u8) {
        self.ol.point((self.x + self.dx) as f32, (self.y + self.dy) as f32, tag);
    }

    fn moveto(&mut self, dx: f64, dy: f64) {
        self.x += dx;
        self.y += dy;
        if let Some(f) = &mut self.flex {
            if f.len() < 16 {
                f.push((self.x, self.y));
            }
            return;
        }
        if self.open {
            self.ol.close();
            self.open = false;
        }
    }

    fn begin(&mut self) {
        if !self.open {
            self.pt(OLT_ON);
            self.open = true;
        }
    }

    fn lineto(&mut self, dx: f64, dy: f64) {
        self.begin();
        self.x += dx;
        self.y += dy;
        self.pt(OLT_ON);
    }

    fn curveto(&mut self, d: [f64; 6]) {
        self.begin();
        let (x1, y1) = (self.x + d[0], self.y + d[1]);
        let (x2, y2) = (x1 + d[2], y1 + d[3]);
        let (x3, y3) = (x2 + d[4], y2 + d[5]);
        for (x, y, tag) in [(x1, y1, OLT_CUBIC), (x2, y2, OLT_CUBIC), (x3, y3, OLT_ON)] {
            self.x = x;
            self.y = y;
            self.pt(tag);
        }
    }

    fn arg(&self, i: usize) -> f64 {
        self.stack.get(i).copied().unwrap_or(0.0)
    }

    /// run a charstring: Some(true) at endchar, Some(false) at return or its end
    fn run(&mut self, cs: &[u8], depth: u32) -> Option<bool> {
        if depth > DEPTH {
            return None;
        }
        let mut p = 0usize;
        while p < cs.len() {
            self.budget -= 1;
            if self.budget < 0 || self.stack.len() > 64 {
                return None;
            }
            let b = cs[p];
            p += 1;
            if b >= 32 {
                let v = match b {
                    32..=246 => b as i64 - 139,
                    247..=250 => (b as i64 - 247) * 256 + *cs.get(p)? as i64 + 108,
                    251..=254 => -(b as i64 - 251) * 256 - *cs.get(p)? as i64 - 108,
                    _ => {
                        let q = cs.get(p..p + 4)?;
                        i32::from_be_bytes([q[0], q[1], q[2], q[3]]) as i64
                    }
                };
                p += match b {
                    32..=246 => 0,
                    247..=254 => 1,
                    _ => 4,
                };
                self.stack.push(v as f64);
                continue;
            }
            match b {
                1 | 3 => self.stack.clear(), // hstem, vstem
                4 => {
                    let dy = self.arg(0);
                    self.moveto(0.0, dy);
                    self.stack.clear();
                }
                5 => {
                    self.lineto(self.arg(0), self.arg(1));
                    self.stack.clear();
                }
                6 => {
                    self.lineto(self.arg(0), 0.0);
                    self.stack.clear();
                }
                7 => {
                    self.lineto(0.0, self.arg(0));
                    self.stack.clear();
                }
                8 => {
                    let d = [self.arg(0), self.arg(1), self.arg(2), self.arg(3), self.arg(4), self.arg(5)];
                    self.curveto(d);
                    self.stack.clear();
                }
                9 => {
                    if self.open {
                        self.ol.close();
                        self.open = false;
                    }
                    self.stack.clear();
                }
                10 => {
                    let i = self.stack.pop()? as i64;
                    let t = self.t;
                    let r = *t.subrs.get(usize::try_from(i).ok()?)?;
                    let sub = t.subr_data.get(r.0..r.1)?;
                    if self.run(sub, depth + 1)? {
                        return Some(true);
                    }
                }
                11 => return Some(false),
                13 => {
                    // hsbw: sbx wx
                    self.sbx = self.arg(0);
                    self.x = self.sbx;
                    self.y = 0.0;
                    self.stack.clear();
                }
                14 => {
                    if self.open {
                        self.ol.close();
                        self.open = false;
                    }
                    return Some(true);
                }
                21 => {
                    let (dx, dy) = (self.arg(0), self.arg(1));
                    self.moveto(dx, dy);
                    self.stack.clear();
                }
                22 => {
                    let dx = self.arg(0);
                    self.moveto(dx, 0.0);
                    self.stack.clear();
                }
                30 => {
                    let d = [0.0, self.arg(0), self.arg(1), self.arg(2), self.arg(3), 0.0];
                    self.curveto(d);
                    self.stack.clear();
                }
                31 => {
                    let d = [self.arg(0), 0.0, self.arg(1), self.arg(2), 0.0, self.arg(3)];
                    self.curveto(d);
                    self.stack.clear();
                }
                12 => {
                    let e = *cs.get(p)?;
                    p += 1;
                    match e {
                        0 | 1 | 2 => self.stack.clear(), // dotsection, vstem3, hstem3
                        6 => {
                            // seac: asb adx ady bchar achar
                            if self.seac_depth > 0 {
                                return None;
                            }
                            let (asb, adx, ady) = (self.arg(0), self.arg(1), self.arg(2));
                            let (bc, ac) = (self.arg(3) as i64, self.arg(4) as i64);
                            let name = |c: i64| -> Option<Vec<u8>> {
                                let i = *tables::NAMES_STANDARD.get(usize::try_from(c).ok()?)?;
                                (i != 0xFFFF).then(|| super::fonts::glyph_name(i as usize).to_vec())
                            };
                            let (bn, an) = (name(bc)?, name(ac)?);
                            self.seac_depth = 1;
                            // the base at the origin, then the accent moved by (adx - asb, ady)
                            for (n, dx, dy) in [(bn, 0.0, 0.0), (an, adx - asb, ady)] {
                                let Some(&r) = self.t.glyphs.get(&n) else { continue };
                                let cs2 = self.t.charstring(r);
                                self.stack.clear();
                                self.x = 0.0;
                                self.y = 0.0;
                                self.dx = dx;
                                self.dy = dy;
                                self.open = false;
                                self.run(&cs2, depth + 1)?;
                                if self.open {
                                    self.ol.close();
                                    self.open = false;
                                }
                            }
                            return Some(true);
                        }
                        7 => {
                            // sbw: sbx sby wx wy
                            self.sbx = self.arg(0);
                            self.x = self.sbx;
                            self.y = self.arg(1);
                            self.stack.clear();
                        }
                        12 => {
                            let b = self.stack.pop()?;
                            let a = self.stack.pop()?;
                            self.stack.push(if b == 0.0 { 0.0 } else { a / b });
                        }
                        16 => {
                            // callothersubr: args n othersubr#
                            let k = self.stack.pop()? as i64;
                            let n = (self.stack.pop()? as i64).clamp(0, 16) as usize;
                            if n > self.stack.len() {
                                return None;
                            }
                            let args: Vec<f64> = self.stack.split_off(self.stack.len() - n);
                            match k {
                                1 => {
                                    self.flex = Some(Vec::new());
                                    self.flex_start = (self.x, self.y);
                                }
                                2 => {}
                                0 => {
                                    // flex end: the reference point, then the two curves
                                    // from where the flex began
                                    let p = self.flex.take().unwrap_or_default();
                                    if p.len() >= 7 {
                                        self.x = self.flex_start.0;
                                        self.y = self.flex_start.1;
                                        let rel = |a: (f64, f64), b: (f64, f64)| [b.0 - a.0, b.1 - a.1];
                                        let (c1, c2, c3) = (rel((self.x, self.y), p[1]), rel(p[1], p[2]), rel(p[2], p[3]));
                                        self.curveto([c1[0], c1[1], c2[0], c2[1], c3[0], c3[1]]);
                                        let (d1, d2, d3) = (rel(p[3], p[4]), rel(p[4], p[5]), rel(p[5], p[6]));
                                        self.curveto([d1[0], d1[1], d2[0], d2[1], d3[0], d3[1]]);
                                    }
                                    // the end point for "pop pop setcurrentpoint" (x first)
                                    self.ps = vec![self.y, self.x];
                                }
                                _ => {
                                    self.ps = args.into_iter().collect();
                                }
                            }
                        }
                        17 => {
                            let v = self.ps.pop().unwrap_or(0.0);
                            self.stack.push(v);
                        }
                        33 => {
                            // setcurrentpoint
                            self.x = self.arg(0);
                            self.y = self.arg(1);
                            self.stack.clear();
                        }
                        _ => self.stack.clear(),
                    }
                }
                _ => self.stack.clear(),
            }
        }
        Some(false)
    }
}
