// lex -- the tokens of WGSL (spec section 3) and template list discovery (3.9)
//
//   tokens(src) -> (Vec<Token>, Vec<Diagnostic>)   every token (no blankspace, no comments),
//                                     ending with Eof; never fails: a bad character, an
//                                     unterminated comment, a reserved word or a malformed
//                                     number becomes an Error token (or a Reserved one) plus
//                                     a diagnostic
//   discover_templates(src)           the byte offsets of every '<' that starts and every '>'
//                                     that ends a template list (the spec's algorithm, code
//                                     point by code point); tokens() turns them into
//                                     TemplateStart / TemplateEnd
//   int_value(text) / float_value(text)   decode a literal token (range rules of 3.5.2)
//   round_bits(m, e, sticky, fmt)     m * 2^e rounded to f16 / f32 / f64 (nearest even)
//
// Blankspace is Pattern_White_Space (space, tab, LF, VT, FF, CR, NEL, LRM, RLM, LS, PS);
// block comments nest; identifiers are XID_Start (or _) + XID_Continue (xid.rs); `_` alone
// is the Underscore token, `__x` is an error.  Punctuation is matched longest first except
// where discovery found a template delimiter.  Splitting `--` / `&&` where the grammar wants
// two tokens (`a--b` is `a - -b`) is the parser's job.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::{xid, Diagnostic, LineMap};

/// token kinds: identifiers, literals, keywords, syntactic tokens, template delimiters
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tok {
    Ident,
    Int,
    Float,
    /// a reserved word (reported; the parser takes it as an identifier)
    Reserved,
    Error,
    Eof,
    // keywords
    Alias, Break, Case, Const, ConstAssert, Continue, Continuing, Default, Diagnostic, Discard,
    Else, Enable, False, Fn, For, If, Let, Loop, Override, Requires, Return, Struct, Switch,
    True, Var, While,
    // syntactic tokens
    And, AndAnd, Arrow, At, Slash, Bang, LBracket, RBracket, LBrace, RBrace, Colon, Comma,
    Eq, EqEq, NotEq, Gt, Ge, Shr, Lt, Le, Shl, Percent, Minus, MinusMinus, Dot, Plus, PlusPlus,
    Or, OrOr, LParen, RParen, Semi, Star, Tilde, Underscore, Xor, PlusEq, MinusEq, StarEq,
    SlashEq, PercentEq, AndEq, OrEq, XorEq, ShrEq, ShlEq,
    TemplateStart, TemplateEnd,
}

/// the keywords of spec section 16.1
pub const KEYWORDS: [(&str, Tok); 26] = [
    ("alias", Tok::Alias), ("break", Tok::Break), ("case", Tok::Case), ("const", Tok::Const),
    ("const_assert", Tok::ConstAssert), ("continue", Tok::Continue), ("continuing", Tok::Continuing),
    ("default", Tok::Default), ("diagnostic", Tok::Diagnostic), ("discard", Tok::Discard),
    ("else", Tok::Else), ("enable", Tok::Enable), ("false", Tok::False), ("fn", Tok::Fn),
    ("for", Tok::For), ("if", Tok::If), ("let", Tok::Let), ("loop", Tok::Loop),
    ("override", Tok::Override), ("requires", Tok::Requires), ("return", Tok::Return),
    ("struct", Tok::Struct), ("switch", Tok::Switch), ("true", Tok::True), ("var", Tok::Var),
    ("while", Tok::While),
];

/// the reserved words of spec section 16.2, sorted
pub const RESERVED: &[&str] = &[
    "NULL", "Self", "abstract", "active", "alignas", "alignof", "as", "asm", "asm_fragment", "async",
    "attribute", "auto", "await", "become", "cast", "catch", "class", "co_await", "co_return",
    "co_yield", "coherent", "column_major", "common", "compile", "compile_fragment", "concept",
    "const_cast", "consteval", "constexpr", "constinit", "crate", "debugger", "decltype", "delete",
    "demote", "demote_to_helper", "do", "dynamic_cast", "enum", "explicit", "export", "extends",
    "extern", "external", "fallthrough", "filter", "final", "finally", "friend", "from", "fxgroup",
    "get", "goto", "groupshared", "highp", "impl", "implements", "import", "inline", "instanceof",
    "interface", "layout", "lowp", "macro", "macro_rules", "match", "mediump", "meta", "mod",
    "module", "move", "mut", "mutable", "namespace", "new", "nil", "noexcept", "noinline",
    "nointerpolation", "non_coherent", "noncoherent", "noperspective", "null", "nullptr", "of",
    "operator", "package", "packoffset", "partition", "pass", "patch", "pixelfragment", "precise",
    "precision", "premerge", "priv", "protected", "pub", "public", "readonly", "ref", "regardless",
    "register", "reinterpret_cast", "require", "resource", "restrict", "self", "set", "shared",
    "sizeof", "smooth", "snorm", "static", "static_assert", "static_cast", "std", "subroutine",
    "super", "target", "template", "this", "thread_local", "throw", "trait", "try", "type",
    "typedef", "typeid", "typename", "typeof", "union", "unless", "unorm", "unsafe", "unsized",
    "use", "using", "varying", "virtual", "volatile", "wgsl", "where", "with", "writeonly", "yield",
];

impl Tok {
    /// the spelling for messages ("`;`", "Bezeichner")
    pub fn text(self) -> &'static str {
        use Tok::*;
        match self {
            Ident => "Bezeichner",
            Int => "ganze Zahl",
            Float => "Gleitkommazahl",
            Reserved => "reserviertes Wort",
            Error => "ungültiges Zeichen",
            Eof => "Ende des Texts",
            And => "&", AndAnd => "&&", Arrow => "->", At => "@", Slash => "/", Bang => "!",
            LBracket => "[", RBracket => "]", LBrace => "{", RBrace => "}", Colon => ":", Comma => ",",
            Eq => "=", EqEq => "==", NotEq => "!=", Gt => ">", Ge => ">=", Shr => ">>", Lt => "<",
            Le => "<=", Shl => "<<", Percent => "%", Minus => "-", MinusMinus => "--", Dot => ".",
            Plus => "+", PlusPlus => "++", Or => "|", OrOr => "||", LParen => "(", RParen => ")",
            Semi => ";", Star => "*", Tilde => "~", Underscore => "_", Xor => "^", PlusEq => "+=",
            MinusEq => "-=", StarEq => "*=", SlashEq => "/=", PercentEq => "%=", AndEq => "&=",
            OrEq => "|=", XorEq => "^=", ShrEq => ">>=", ShlEq => "<<=", TemplateStart => "<",
            TemplateEnd => ">",
            k => KEYWORDS.iter().find(|&&(_, t)| t == k).map(|&(s, _)| s).unwrap_or("?"),
        }
    }
    /// one of KEYWORDS
    pub fn is_keyword(self) -> bool {
        KEYWORDS.iter().any(|&(_, t)| t == self)
    }
}

/// a token: kind and byte range
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Token {
    pub kind: Tok,
    pub start: u32,
    pub end: u32,
}

/// Pattern_White_Space
pub fn is_blank(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0b' | '\x0c' | '\r' | '\u{85}' | '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}')
}

fn char_at(src: &str, i: usize) -> Option<char> {
    src.get(i..).and_then(|s| s.chars().next())
}

/// past blankspace and comments from i; Err(start) for an unterminated block comment
fn skip_blank(src: &str, mut i: usize) -> Result<usize, usize> {
    let b = src.as_bytes();
    loop {
        match b.get(i) {
            Some(&c) if c < 0x80 => {
                if matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r') {
                    i += 1;
                } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
                    i += 2;
                    while i < b.len() {
                        let ch = char_at(src, i).unwrap_or('\n');
                        if matches!(ch, '\n' | '\x0b' | '\x0c' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}') {
                            break;
                        }
                        i += ch.len_utf8();
                    }
                } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
                    let start = i;
                    let mut depth = 1u32;
                    i += 2;
                    while depth > 0 {
                        if i >= b.len() {
                            return Err(start);
                        }
                        if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                            depth += 1;
                            i += 2;
                        } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                            depth -= 1;
                            i += 2;
                        } else {
                            i += 1;
                        }
                    }
                } else {
                    return Ok(i);
                }
            }
            Some(_) => {
                let ch = char_at(src, i).unwrap_or('\0');
                if is_blank(ch) {
                    i += ch.len_utf8();
                } else {
                    return Ok(i);
                }
            }
            None => return Ok(i),
        }
    }
}

fn is_hex(c: u8) -> bool {
    c.is_ascii_hexdigit()
}

/// the end of the numeric literal starting at i (longest match of the literal regexes), and
/// whether it is a float; None when no literal starts here
pub fn scan_number(src: &str, i: usize) -> Option<(usize, bool)> {
    let b = src.as_bytes();
    let at = |k: usize| b.get(k).copied().unwrap_or(0);
    let digits = |mut k: usize, hex: bool| {
        while if hex { is_hex(at(k)) } else { at(k).is_ascii_digit() } {
            k += 1;
        }
        k
    };
    let c0 = at(i);
    if !(c0.is_ascii_digit() || (c0 == b'.' && at(i + 1).is_ascii_digit())) {
        return None;
    }
    let mut best: Option<(usize, bool)> = None;
    let mut take = |end: usize, float: bool| {
        if best.is_none_or(|(e, _)| end > e) {
            best = Some((end, float));
        }
    };
    if c0 == b'0' && matches!(at(i + 1), b'x' | b'X') {
        let s = i + 2;
        let a = digits(s, true);
        // hex integer
        if a > s {
            let e = if matches!(at(a), b'i' | b'u') { a + 1 } else { a };
            take(e, false);
        }
        // hex float: digits [. digits] with at least one digit and a point or an exponent
        let (frac_end, point) = if at(a) == b'.' { (digits(a + 1, true), true) } else { (a, false) };
        let any = a > s || frac_end > a + 1;
        if any && (point || matches!(at(frac_end), b'p' | b'P')) {
            let mut e = frac_end;
            if matches!(at(e), b'p' | b'P') {
                let mut k = e + 1;
                if matches!(at(k), b'+' | b'-') {
                    k += 1;
                }
                let d = digits(k, false);
                if d > k {
                    e = if matches!(at(d), b'f' | b'h') { d + 1 } else { d };
                    take(e, true);
                } else if point {
                    take(frac_end, true);
                }
            } else if point {
                take(e, true);
            }
        }
        return best.or(Some((i + 1, false)));
    }
    let a = digits(i, false);
    // integers: 0 or [1-9][0-9]*, optional i / u
    if a > i {
        let int_end = if c0 == b'0' { i + 1 } else { a };
        let e = if matches!(at(int_end), b'i' | b'u') { int_end + 1 } else { int_end };
        take(e, false);
        // 0f / [1-9][0-9]*f
        if matches!(at(int_end), b'f' | b'h') {
            take(int_end + 1, true);
        }
    }
    // floats with a point or an exponent
    let (mant_end, point) = if at(a) == b'.' { (digits(a + 1, false), true) } else { (a, false) };
    let mant_digits = (a - i) + if point { mant_end - a - 1 } else { 0 };
    if mant_digits > 0 && (point || matches!(at(a), b'e' | b'E')) {
        let mut e = mant_end;
        let mut ok = point && (a > i || mant_end > a + 1);
        if matches!(at(mant_end), b'e' | b'E') {
            let mut k = mant_end + 1;
            if matches!(at(k), b'+' | b'-') {
                k += 1;
            }
            let d = digits(k, false);
            if d > k {
                e = d;
                ok = true;
            }
        }
        if ok {
            if matches!(at(e), b'f' | b'h') {
                e += 1;
            }
            take(e, true);
        }
    }
    best
}

/// the end of an ident_pattern_token at i (also `_` alone)
pub fn scan_ident(src: &str, i: usize) -> Option<usize> {
    let c = char_at(src, i)?;
    if !(c == '_' || xid::is_start(c)) {
        return None;
    }
    let mut k = i + c.len_utf8();
    while let Some(d) = char_at(src, k) {
        if !xid::is_continue(d) {
            break;
        }
        k += d.len_utf8();
    }
    Some(k)
}

/// the template list delimiters (spec 3.9): (starts, ends), ascending byte offsets
pub fn discover_templates(src: &str) -> (Vec<u32>, Vec<u32>) {
    let b = src.as_bytes();
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    // pending candidates: (position, depth)
    let mut pending: Vec<(usize, u32)> = Vec::new();
    let mut depth: u32 = 0;
    let mut i = 0;
    let at = |k: usize| b.get(k).copied().unwrap_or(0);
    loop {
        i = match skip_blank(src, i) {
            Ok(k) => k,
            Err(_) => break,
        };
        if i >= b.len() {
            break;
        }
        if let Some((e, _)) = scan_number(src, i) {
            i = e;
            continue;
        }
        if let Some(e) = scan_ident(src, i) {
            let word = &src[i..e];
            i = e;
            if word == "true" || word == "false" || word == "_" {
                continue;
            }
            i = match skip_blank(src, i) {
                Ok(k) => k,
                Err(_) => break,
            };
            if at(i) == b'<' {
                pending.push((i, depth));
                i += 1;
                if at(i) == b'<' || at(i) == b'=' {
                    pending.pop();
                    i += 1;
                }
            }
            continue;
        }
        let c = at(i);
        match c {
            b'>' => {
                if let Some(&(p, d)) = pending.last() {
                    if d == depth {
                        starts.push(p as u32);
                        ends.push(i as u32);
                        pending.pop();
                        i += 1;
                        continue;
                    }
                }
                i += 1;
                if at(i) == b'=' {
                    i += 1;
                }
            }
            b'(' | b'[' => {
                depth += 1;
                i += 1;
            }
            b')' | b']' => {
                while pending.last().is_some_and(|&(_, d)| d >= depth) {
                    pending.pop();
                }
                depth = depth.saturating_sub(1);
                i += 1;
            }
            b'!' => {
                i += 1;
                if at(i) == b'=' {
                    i += 1;
                }
            }
            b'=' => {
                i += 1;
                if at(i) == b'=' {
                    i += 1;
                    continue;
                }
                depth = 0;
                pending.clear();
            }
            b';' | b'{' | b':' => {
                depth = 0;
                pending.clear();
                i += 1;
            }
            b'&' | b'|' if at(i + 1) == c => {
                while pending.last().is_some_and(|&(_, d)| d >= depth) {
                    pending.pop();
                }
                i += 2;
            }
            _ => i += char_at(src, i).map(|ch| ch.len_utf8()).unwrap_or(1),
        }
    }
    starts.sort_unstable();
    (starts, ends)
}

const PUNCT: &[(&str, Tok)] = &[
    (">>=", Tok::ShrEq), ("<<=", Tok::ShlEq),
    ("&&", Tok::AndAnd), ("->", Tok::Arrow), ("==", Tok::EqEq), ("!=", Tok::NotEq), (">=", Tok::Ge),
    (">>", Tok::Shr), ("<=", Tok::Le), ("<<", Tok::Shl), ("--", Tok::MinusMinus), ("++", Tok::PlusPlus),
    ("||", Tok::OrOr), ("+=", Tok::PlusEq), ("-=", Tok::MinusEq), ("*=", Tok::StarEq), ("/=", Tok::SlashEq),
    ("%=", Tok::PercentEq), ("&=", Tok::AndEq), ("|=", Tok::OrEq), ("^=", Tok::XorEq),
    ("&", Tok::And), ("@", Tok::At), ("/", Tok::Slash), ("!", Tok::Bang), ("[", Tok::LBracket),
    ("]", Tok::RBracket), ("{", Tok::LBrace), ("}", Tok::RBrace), (":", Tok::Colon), (",", Tok::Comma),
    ("=", Tok::Eq), (">", Tok::Gt), ("<", Tok::Lt), ("%", Tok::Percent), ("-", Tok::Minus), (".", Tok::Dot),
    ("+", Tok::Plus), ("|", Tok::Or), ("(", Tok::LParen), (")", Tok::RParen), (";", Tok::Semi),
    ("*", Tok::Star), ("~", Tok::Tilde), ("^", Tok::Xor),
];

/// a keyword, a reserved word or an identifier
pub fn word_kind(w: &str) -> Tok {
    if let Some(&(_, t)) = KEYWORDS.iter().find(|&&(s, _)| s == w) {
        return t;
    }
    if RESERVED.binary_search(&w).is_ok() {
        return Tok::Reserved;
    }
    if w == "_" { Tok::Underscore } else { Tok::Ident }
}

fn char_name(c: char) -> String {
    if c.is_control() || is_blank(c) || (c as u32) >= 0x2000 && (c as u32) < 0x2070 || c == '\u{feff}' {
        format!("U+{:04X}", c as u32)
    } else {
        format!("`{}` (U+{:04X})", c, c as u32)
    }
}

/// see the module comment
pub fn tokens(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let lm = LineMap::new(src);
    let (starts, ends) = discover_templates(src);
    let (mut si, mut ei) = (0usize, 0usize);
    let mut toks = Vec::new();
    let mut diags = Vec::new();
    let b = src.as_bytes();
    let mut i = 0usize;
    // U+0000 is not allowed anywhere, comments included
    if let Some(z) = src.find('\0') {
        diags.push(lm.diag(src, z as u32, z as u32 + 1, true, String::from("das Zeichen U+0000 ist in WGSL nicht erlaubt")));
    }
    let err = |diags: &mut Vec<Diagnostic>, s: usize, e: usize, msg: String| {
        if diags.len() < 50 {
            diags.push(lm.diag(src, s as u32, e as u32, true, msg));
        }
    };
    loop {
        i = match skip_blank(src, i) {
            Ok(k) => k,
            Err(s) => {
                err(&mut diags, s, s + 2, String::from("Blockkommentar ohne Ende (`*/` fehlt)"));
                b.len()
            }
        };
        if i >= b.len() {
            toks.push(Token { kind: Tok::Eof, start: b.len() as u32, end: b.len() as u32 });
            return (toks, diags);
        }
        while si < starts.len() && (starts[si] as usize) < i {
            si += 1;
        }
        while ei < ends.len() && (ends[ei] as usize) < i {
            ei += 1;
        }
        let start = i;
        let kind;
        if si < starts.len() && starts[si] as usize == i {
            kind = Tok::TemplateStart;
            i += 1;
        } else if ei < ends.len() && ends[ei] as usize == i {
            kind = Tok::TemplateEnd;
            i += 1;
        } else if let Some((e, float)) = scan_number(src, i) {
            // a leading zero followed by more digits ("012") is no literal
            if !float && b[i] == b'0' && b.get(e).is_some_and(|c| c.is_ascii_digit()) {
                let mut k = e;
                while b.get(k).is_some_and(|c| c.is_ascii_digit()) {
                    k += 1;
                }
                err(&mut diags, i, k, String::from("ganze Zahlen dürfen nicht mit 0 beginnen (`012` ist keine Oktalzahl)"));
                kind = Tok::Error;
                i = k;
            } else {
                kind = if float { Tok::Float } else { Tok::Int };
                i = e;
            }
        } else if let Some(e) = scan_ident(src, i) {
            let w = &src[i..e];
            kind = word_kind(w);
            i = e;
            if kind == Tok::Reserved {
                err(&mut diags, start, i, format!("`{}` ist ein reserviertes Wort und kann kein Name sein", w));
            } else if kind == Tok::Ident && w.starts_with("__") {
                err(&mut diags, start, i, format!("`{}`: Namen dürfen nicht mit zwei Unterstrichen beginnen", w));
            }
        } else if let Some(&(p, t)) = PUNCT.iter().find(|(p, _)| src[i..].starts_with(p)) {
            kind = t;
            i += p.len();
        } else {
            let c = char_at(src, i).unwrap_or('\0');
            i += c.len_utf8().max(1);
            let msg = if c == '\0' {
                String::from("das Zeichen U+0000 ist in WGSL nicht erlaubt")
            } else if c == '\u{feff}' && start == 0 {
                String::from("WGSL-Text darf kein Byte Order Mark (U+FEFF) haben")
            } else {
                format!("unerwartetes Zeichen {}", char_name(c))
            };
            err(&mut diags, start, i, msg);
            kind = Tok::Error;
        }
        toks.push(Token { kind, start: start as u32, end: i as u32 });
    }
}

// ---- literal values

/// the suffix of a numeric literal
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Suffix {
    None,
    I,
    U,
    F,
    H,
}

/// an integer literal's value and suffix; Err: a German message (out of range)
pub fn int_value(text: &str) -> Result<(i64, Suffix), String> {
    let (body, suf) = match text.as_bytes().last() {
        Some(b'i') => (&text[..text.len() - 1], Suffix::I),
        Some(b'u') => (&text[..text.len() - 1], Suffix::U),
        _ => (text, Suffix::None),
    };
    let (digits, radix) = if body.len() > 2 && (body.starts_with("0x") || body.starts_with("0X")) { (&body[2..], 16) } else { (body, 10) };
    let mut v: u128 = 0;
    for c in digits.bytes() {
        let d = (c as char).to_digit(radix).unwrap_or(0) as u128;
        v = (v * radix as u128 + d).min(u128::MAX >> 8);
    }
    let (max, name) = match suf {
        Suffix::I => (i32::MAX as u128, "i32"),
        Suffix::U => (u32::MAX as u128, "u32"),
        _ => (i64::MAX as u128, "AbstractInt"),
    };
    if v > max {
        return Err(format!("`{}` ist zu groß für {} (höchstens {})", text, name, max));
    }
    Ok((v as i64, suf))
}

/// the format a float literal is rounded to
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fmt {
    F16,
    F32,
    F64,
}

impl Fmt {
    fn params(self) -> (u32, i64, i64) {
        // precision, minimum normal exponent, maximum exponent
        match self {
            Fmt::F16 => (11, -14, 15),
            Fmt::F32 => (24, -126, 127),
            Fmt::F64 => (53, -1022, 1023),
        }
    }
}

fn pow2(k: i64) -> f64 {
    f64::from_bits(((k + 1023) as u64) << 52)
}

/// m * 2^e (plus a sticky bit for nonzero digits beyond m) rounded to nearest even in fmt:
/// Some((value, exact)), None on overflow; the value is returned as f64 (exact for every fmt)
pub fn round_bits(m: u128, e: i64, sticky: bool, fmt: Fmt) -> Option<(f64, bool)> {
    if m == 0 {
        return Some((0.0, !sticky));
    }
    let (p, emin, emax) = fmt.params();
    let nbits = 128 - m.leading_zeros() as i64;
    let exp = nbits - 1 + e; // exponent of the leading bit
    let prec = if exp < emin { p as i64 - (emin - exp) } else { p as i64 };
    // keep `prec` bits (prec may be <= 0: the value rounds to 0 or the smallest subnormal)
    let drop = nbits - prec;
    let (mut q, mut qe, exact);
    if drop <= 0 {
        q = m;
        qe = e;
        exact = !sticky;
    } else if drop >= 128 {
        q = 0;
        qe = e + drop;
        exact = false;
        // anything below half of the smallest step rounds to zero
        let _ = (&mut q, &mut qe);
    } else {
        let lo = m & ((1u128 << drop) - 1);
        let half = 1u128 << (drop - 1);
        q = m >> drop;
        qe = e + drop;
        exact = lo == 0 && !sticky;
        if lo > half || (lo == half && (sticky || q & 1 == 1)) {
            q += 1;
        }
    }
    if q == 0 {
        return Some((0.0, exact));
    }
    // carry into a new bit
    let qbits = 128 - q.leading_zeros() as i64;
    let top = qbits - 1 + qe;
    if top > emax {
        return None;
    }
    // q < 2^54 here; build q * 2^qe exactly in two steps
    let qf = q as f64;
    let v = if qe < -900 { qf * pow2(qe + 600) * pow2(-600) } else if qe > 900 { qf * pow2(qe - 600) * pow2(600) } else { qf * pow2(qe) };
    Some((v, exact))
}

/// decompose a finite f64 into (mantissa, exponent) with x = m * 2^e
pub fn f64_parts(x: f64) -> (u128, i64, bool) {
    let bits = x.to_bits();
    let neg = bits >> 63 == 1;
    let be = ((bits >> 52) & 0x7ff) as i64;
    let frac = bits & ((1u64 << 52) - 1);
    if be == 0 {
        (frac as u128, -1074, neg)
    } else {
        ((frac | 1u64 << 52) as u128, be - 1075, neg)
    }
}

/// round a finite f64 to f16 / f32 (nearest even): None on overflow
pub fn round_f64(x: f64, fmt: Fmt) -> Option<f64> {
    if !x.is_finite() {
        return None;
    }
    let (m, e, neg) = f64_parts(x);
    let (v, _) = round_bits(m, e, false, fmt)?;
    Some(if neg { -v } else { v })
}

/// a float literal's value (in f64; exact for the suffix's type) and suffix; Err: a German
/// message (overflow, an inexact hexadecimal f / h literal)
pub fn float_value(text: &str) -> Result<(f64, Suffix), String> {
    let hex = text.starts_with("0x") || text.starts_with("0X");
    let (body, suf) = match text.as_bytes().last() {
        Some(b'f') if !hex || text.contains(['p', 'P']) => (&text[..text.len() - 1], Suffix::F),
        Some(b'h') => (&text[..text.len() - 1], Suffix::H),
        _ => (text, Suffix::None),
    };
    let fmt = match suf {
        Suffix::F => Fmt::F32,
        Suffix::H => Fmt::F16,
        _ => Fmt::F64,
    };
    let tname = match fmt {
        Fmt::F16 => "f16",
        Fmt::F32 => "f32",
        Fmt::F64 => "AbstractFloat",
    };
    if hex {
        let s = &body[2..];
        let (mant, exp) = match s.find(['p', 'P']) {
            Some(k) => (&s[..k], &s[k + 1..]),
            None => (s, "0"),
        };
        let mut m: u128 = 0;
        let mut e: i64 = 0;
        let mut sticky = false;
        let mut seen_point = false;
        for c in mant.bytes() {
            if c == b'.' {
                seen_point = true;
                continue;
            }
            let d = (c as char).to_digit(16).unwrap_or(0) as u128;
            if m >> 120 != 0 {
                sticky |= d != 0;
                if !seen_point {
                    e += 4;
                }
            } else {
                m = m * 16 + d;
                if seen_point {
                    e -= 4;
                }
            }
        }
        let (neg_e, digits) = match exp.as_bytes().first() {
            Some(b'-') => (true, &exp[1..]),
            Some(b'+') => (false, &exp[1..]),
            _ => (false, exp),
        };
        let mut x: i64 = 0;
        for c in digits.bytes() {
            x = (x * 10 + (c - b'0') as i64).min(1 << 40);
        }
        e += if neg_e { -x } else { x };
        return match round_bits(m, e, sticky, fmt) {
            None => Err(format!("`{}` ist zu groß für {}", text, tname)),
            Some((_, false)) if fmt != Fmt::F64 => Err(format!("`{}` lässt sich in {} nicht exakt darstellen", text, tname)),
            Some((v, _)) => Ok((v, suf)),
        };
    }
    // decimal: core's correctly rounded parser (it wants a digit before the point)
    let mut norm = String::with_capacity(body.len() + 2);
    if body.starts_with('.') {
        norm.push('0');
    }
    norm.push_str(body);
    let v = match fmt {
        Fmt::F32 => norm.parse::<f32>().map(|x| x as f64).unwrap_or(f64::INFINITY),
        _ => norm.parse::<f64>().unwrap_or(f64::INFINITY),
    };
    let v = if fmt == Fmt::F16 { round_f64(v, Fmt::F16).unwrap_or(f64::INFINITY) } else { v };
    if !v.is_finite() {
        return Err(format!("`{}` ist zu groß für {}", text, tname));
    }
    Ok((v, suf))
}
