// The operators of a content stream (ISO 32000-1, 7.8.2): operands, then the operator.  An
// inline image (BI ... ID data EI) comes as one operator "BI" with two operands, its
// dictionary and its data.  Damaged content gives what can be read: an unknown token is skipped,
// operands beyond MAX_OPERANDS are dropped, and the stream ends where the bytes end.

use super::lex::{self, Lexer, Tok};
use super::object::{Dict, Object};
use alloc::vec::Vec;

/// the most operands kept for one operator (a TJ array is one operand)
pub const MAX_OPERANDS: usize = 4096;

/// one operator with its operands
#[derive(Clone, Debug, PartialEq)]
pub struct Op<'a> {
    /// the operator: b"Tj", b"re", b"BI" (an inline image), ...
    pub operator: &'a [u8],
    pub operands: Vec<Object>,
}

/// the operators of content-stream bytes, in order
pub struct Ops<'a> {
    lx: Lexer<'a>,
    /// the bytes after EI candidates that may still be looked at (their windows overlap; when
    /// none are left, an EI between white space ends an image)
    after_budget: usize,
}

impl<'a> Ops<'a> {
    pub fn new(content: &'a [u8]) -> Ops<'a> {
        Ops { lx: Lexer::new(content, 0), after_budget: content.len().saturating_mul(4).saturating_add(1 << 16) }
    }

    /// the inline image after BI: its dictionary up to ID, its data up to EI
    fn inline_image(&mut self) -> (Dict, Vec<u8>) {
        let mut d = Dict::new();
        loop {
            match self.lx.next() {
                Tok::Kw(b"ID") | Tok::Eof => break,
                Tok::Name(k) => {
                    let t = self.lx.next();
                    if t == Tok::Kw(b"ID") {
                        break;
                    }
                    if let Ok(v) = self.lx.value_of(t) {
                        d.map_mut().insert(super::object::Name(k), v);
                    }
                }
                _ => {}
            }
            self.lx.reset_budget();
        }
        let b = self.lx.b;
        // one white-space byte after ID
        let start = (self.lx.pos + 1).min(b.len());
        // PDF 2.0 gives the length; else EI between white space, with operators after it
        if let Some(n) = d.get("L").or_else(|| d.get("Length")).and_then(Object::as_int).and_then(|n| usize::try_from(n).ok()) {
            if let Some(end) = start.checked_add(n).filter(|&e| e <= b.len()) {
                let mut l = Lexer::new(b, end);
                if l.next() == Tok::Kw(b"EI") {
                    self.lx.pos = l.pos;
                    return (d, b[start..end].to_vec());
                }
            }
        }
        let mut i = start;
        while let Some(p) = lex::find(b, b"EI", i) {
            i = p + 2;
            let before = p == start || lex::is_ws(b[p - 1]);
            let after = b.get(p + 2).is_none_or(|&c| lex::is_ws(c) || lex::is_delim(c));
            if before && after && plausible_after(&b[p + 2..], &mut self.after_budget) {
                let end = if p > start && lex::is_ws(b[p - 1]) { p - 1 } else { p };
                self.lx.pos = p + 2;
                return (d, b[start..end.max(start)].to_vec());
            }
        }
        self.lx.pos = b.len();
        (d, b[start..].to_vec())
    }
}

/// how far after an EI the content is looked at: each candidate costs at most this much (a
/// hex string after each of millions of EIs was read to its end for each of them)
const AFTER_WINDOW: usize = 256;

/// the operators of content streams (ISO 32000-1, Annex A)
const OPERATORS: &[&[u8]] = &[
    b"b", b"B", b"b*", b"B*", b"BDC", b"BI", b"BMC", b"BT", b"BX", b"c", b"cm", b"CS", b"cs", b"d", b"d0", b"d1", b"Do", b"DP", b"EI", b"EMC", b"ET", b"EX", b"f",
    b"F", b"f*", b"G", b"g", b"gs", b"h", b"i", b"ID", b"j", b"J", b"K", b"k", b"l", b"m", b"M", b"MP", b"n", b"q", b"Q", b"re", b"RG", b"rg", b"ri", b"s", b"S",
    b"SC", b"sc", b"SCN", b"scn", b"sh", b"T*", b"Tc", b"Td", b"TD", b"Tf", b"Tj", b"TJ", b"TL", b"Tm", b"Tr", b"Ts", b"Tw", b"Tz", b"v", b"w", b"W", b"W*",
    b"y", b"'", b"\"",
];

/// the bytes after an EI read as content again: the next tokens are operands and operators
/// (binary image data that happens to hold "EI" rarely does).  Only AFTER_WINDOW bytes are read,
/// a token cut by the window's end counts as content, and what all the candidates of a content
/// stream read together is bounded (`budget`; when it is spent, an EI between white space ends
/// the image as in the simpler readers): the windows of candidates close together overlap
fn plausible_after(rest: &[u8], budget: &mut usize) -> bool {
    if *budget == 0 {
        return true;
    }
    let cut = rest.len() > AFTER_WINDOW;
    let win = &rest[..rest.len().min(AFTER_WINDOW)];
    let mut lx = Lexer::new(win, 0);
    let r = after_tokens(&mut lx, cut);
    *budget = budget.saturating_sub(lx.pos.max(1));
    r
}

fn after_tokens(lx: &mut Lexer, cut: bool) -> bool {
    let end = lx.b.len();
    for _ in 0..8 {
        lx.reset_budget();
        let t = lx.next();
        // a token that reaches the window's end may go on after it
        if cut && lx.pos >= end {
            return true;
        }
        match t {
            Tok::Eof => return true,
            Tok::Kw(k) if matches!(k, b"true" | b"false" | b"null") => {}
            Tok::Kw(k) => {
                if !OPERATORS.contains(&k) {
                    return false;
                }
            }
            Tok::BraceOpen | Tok::BraceClose | Tok::ArrClose | Tok::DictClose => return false,
            t => {
                let ok = lx.value_of(t).is_ok();
                if cut && lx.pos >= end {
                    return true;
                }
                if !ok {
                    return false;
                }
            }
        }
    }
    true
}

impl<'a> Iterator for Ops<'a> {
    type Item = Op<'a>;

    fn next(&mut self) -> Option<Op<'a>> {
        let mut operands = Vec::new();
        loop {
            self.lx.reset_budget();
            let t = self.lx.next();
            match t {
                Tok::Eof => {
                    return None;
                }
                Tok::Kw(b"BI") => {
                    let (d, data) = self.inline_image();
                    return Some(Op { operator: b"BI", operands: alloc::vec![Object::Dict(d), Object::String(data)] });
                }
                Tok::Kw(k) if matches!(k, b"true" | b"false" | b"null") => {
                    if let Ok(v) = self.lx.value_of(Tok::Kw(k)) {
                        if operands.len() < MAX_OPERANDS {
                            operands.push(v);
                        }
                    }
                }
                Tok::Kw(k) if k.len() <= 3 || k.iter().all(|c| c.is_ascii_alphabetic() || *c == b'*' || *c == b'\'' || *c == b'"') => {
                    return Some(Op { operator: k, operands });
                }
                Tok::Kw(_) | Tok::ArrClose | Tok::DictClose | Tok::BraceOpen | Tok::BraceClose => {
                    // a stray token of damaged content
                }
                t => {
                    if let Ok(v) = self.lx.value_of(t) {
                        if operands.len() < MAX_OPERANDS {
                            operands.push(v);
                        }
                    }
                }
            }
        }
    }
}
