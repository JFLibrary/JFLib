// lex -- the tokens of .jfs scripts
//
//   lex(src) -> (Vec<Tok>, Vec<Diagnostic>)   every byte in exactly one token (Space, Newline
//                                             and Comment included), then Eof; never fails: an
//                                             unknown character or a bad number becomes an
//                                             Error token plus a diagnostic
//   significant(&toks, src)                   the tokens the parser reads: no Space, Newline,
//                                             Comment; Tok::nl marks a line break before one
//   Strings stay on one line.  "a${x}b" is StrHead `"a${`, the tokens of x, StrTail `}b"`;
//   between two interpolations StrMid `}..${`.  Braces inside ${..} nest, so do strings.
//   int_value / float_value / str_value decode literal tokens; TokKind::class() groups the
//   kinds for syntax highlighting.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::Diagnostic;

/// the kind of a token; the keywords are Let .. Nil, the other names are the punctuation
/// and operator characters
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokKind {
    Space,          // spaces, tabs and other white space
    Newline,        // \n or \r\n
    Comment,        // // to the line end, /* .. */
    Error,          // a character or number that is no token (lex() reported it)
    Eof,            // the end of the source (lex() only, never in tokens())
    Ident,
    Int,
    Float,
    Str,            // a whole string; in tokens() also the text pieces of an interpolated one
    StrHead,        // "..${ (lex() only: tokens() splits it into Str and Interp)
    StrMid,         // }..${ (lex() only)
    StrTail,        // }.." (lex() only)
    Interp,         // the ${ or } of an interpolation (tokens() only)
    Let, Fn, If, Else, While, For, In, Break, Continue, Return, Try, Catch, Throw, True, False, Nil,
    LParen, RParen, LBracket, RBracket, LBrace, RBrace, Comma, Dot, Colon, Semi,
    DotDot,         // ..
    Plus, Minus, Star, Slash, Percent, PlusEq, MinusEq, StarEq, SlashEq, PercentEq,
    Assign,         // =
    EqEq,           // ==
    NotEq,          // !=
    Not,            // !
    Lt, Le, Gt, Ge,
    AndAnd,         // &&
    OrOr,           // ||
    Coalesce,       // ??
}

/// what a token is for syntax highlighting
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TokClass {
    Space, Comment, Keyword,
    /// true, false, nil
    Constant,
    Number, Str, Ident, Op, Punct, Error,
}

const KEYWORDS: [(&str, TokKind); 16] = [
    ("let", TokKind::Let), ("fn", TokKind::Fn), ("if", TokKind::If), ("else", TokKind::Else),
    ("while", TokKind::While), ("for", TokKind::For), ("in", TokKind::In), ("break", TokKind::Break),
    ("continue", TokKind::Continue), ("return", TokKind::Return), ("try", TokKind::Try),
    ("catch", TokKind::Catch), ("throw", TokKind::Throw), ("true", TokKind::True),
    ("false", TokKind::False), ("nil", TokKind::Nil),
];

impl TokKind {
    /// the highlighting class
    pub fn class(self) -> TokClass {
        use TokKind::*;
        match self {
            Space | Newline | Eof => TokClass::Space,
            Comment => TokClass::Comment,
            Error => TokClass::Error,
            Ident => TokClass::Ident,
            Int | Float => TokClass::Number,
            Str | StrHead | StrMid | StrTail => TokClass::Str,
            True | False | Nil => TokClass::Constant,
            Interp | LParen | RParen | LBracket | RBracket | LBrace | RBrace | Comma | Dot | Colon | Semi => TokClass::Punct,
            k if k.is_keyword() => TokClass::Keyword,
            _ => TokClass::Op,
        }
    }
    /// let, fn, .. nil
    pub fn is_keyword(self) -> bool {
        KEYWORDS.iter().any(|&(_, k)| k == self)
    }
    /// the keyword for a word
    pub fn keyword(word: &str) -> Option<TokKind> {
        KEYWORDS.iter().find(|&&(w, _)| w == word).map(|&(_, k)| k)
    }
}

/// one token: kind, byte range, 1-based line and column (columns count characters) of its
/// start; nl: a line break stands between it and the previous significant token
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tok {
    pub kind: TokKind,
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub col: u32,
    pub nl: bool,
}

struct Lexer<'a> {
    src: &'a str,
    b: &'a [u8],
    pos: usize,
    line: u32,
    col: u32,
    toks: Vec<Tok>,
    diags: Vec<Diagnostic>,
    /// open ${..}: the brace depth inside it and where its string started (byte, line, col)
    interp: Vec<(u32, usize, u32, u32)>,
}

/// tokenize a whole source (see the module comment)
pub fn lex(src: &str) -> (Vec<Tok>, Vec<Diagnostic>) {
    let mut lx = Lexer { src, b: src.as_bytes(), pos: 0, line: 1, col: 1, toks: Vec::new(), diags: Vec::new(), interp: Vec::new() };
    while lx.pos < lx.b.len() {
        lx.token();
    }
    lx.open_strings();
    let end = src.len() as u32;
    lx.toks.push(Tok { kind: TokKind::Eof, start: end, end, line: lx.line, col: lx.col, nl: false });
    (lx.toks, lx.diags)
}

/// the tokens the parser reads (see the module comment); a /* */ comment with a line break
/// counts as a line break
pub fn significant(toks: &[Tok], src: &str) -> Vec<Tok> {
    let mut out = Vec::with_capacity(toks.len() / 2 + 1);
    let mut nl = false;
    for t in toks {
        match t.kind {
            TokKind::Space => {}
            TokKind::Newline => nl = true,
            TokKind::Comment => nl |= src[t.start as usize..t.end as usize].contains('\n'),
            _ => {
                out.push(Tok { nl, ..*t });
                nl = false;
            }
        }
    }
    out
}

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_ascii_alphabetic() || (!c.is_ascii() && c.is_alphabetic())
}

fn is_ident_char(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric() || (!c.is_ascii() && c.is_alphanumeric())
}

impl Lexer<'_> {
    fn peek(&self, k: usize) -> u8 {
        self.b.get(self.pos + k).copied().unwrap_or(0)
    }

    fn ch(&self) -> char {
        self.src[self.pos..].chars().next().unwrap_or('\0')
    }

    /// one character forward (line and column follow)
    fn bump(&mut self) {
        let c = self.ch();
        self.pos += c.len_utf8().max(1);
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
    }

    fn bump_n(&mut self, n: usize) {
        for _ in 0..n {
            self.bump();
        }
    }

    fn error(&mut self, line: u32, col: u32, end_col: u32, msg: String) {
        self.diags.push(Diagnostic { line, col, end_col: end_col.max(col + 1), error: true, msg });
    }

    /// strings still open at a line end or the end of the source
    fn open_strings(&mut self) {
        if let Some(&(_, _, line, col)) = self.interp.first() {
            let end = if line == self.line { self.col } else { col + 1 };
            self.error(line, col, end, String::from("Zeichenkette ohne Ende: hier fehlt \""));
            self.interp.clear();
        }
    }

    fn token(&mut self) {
        let (start, line, col) = (self.pos, self.line, self.col);
        let c = self.peek(0);
        let kind = match c {
            b'\n' => {
                self.open_strings();
                self.bump();
                TokKind::Newline
            }
            b'\r' if self.peek(1) == b'\n' => {
                self.open_strings();
                self.bump_n(2);
                TokKind::Newline
            }
            b'/' if self.peek(1) == b'/' => {
                while self.pos < self.b.len() && self.peek(0) != b'\n' {
                    self.bump();
                }
                TokKind::Comment
            }
            b'/' if self.peek(1) == b'*' => {
                self.bump_n(2);
                while self.pos < self.b.len() && !(self.peek(0) == b'*' && self.peek(1) == b'/') {
                    if self.peek(0) == b'\n' {
                        self.open_strings();
                    }
                    self.bump();
                }
                if self.pos < self.b.len() {
                    self.bump_n(2);
                } else {
                    self.error(line, col, col + 2, String::from("Kommentar ohne Ende: hier fehlt */"));
                }
                TokKind::Comment
            }
            b'"' => {
                self.bump();
                self.string(start, line, col, true)
            }
            b'}' if self.interp.last().is_some_and(|f| f.0 == 0) => {
                let (_, s, l, c) = self.interp.pop().unwrap_or((0, start, line, col));
                self.bump();
                self.string(s, l, c, false)
            }
            b'0'..=b'9' => self.number(),
            _ => {
                let ch = self.ch();
                if is_ident_start(ch) {
                    while self.pos < self.b.len() && is_ident_char(self.ch()) {
                        self.bump();
                    }
                    TokKind::keyword(&self.src[start..self.pos]).unwrap_or(TokKind::Ident)
                } else if ch.is_whitespace() || ch == '\u{feff}' {
                    while self.pos < self.b.len() && (self.ch().is_whitespace() || self.ch() == '\u{feff}')
                        && self.peek(0) != b'\n' && !(self.peek(0) == b'\r' && self.peek(1) == b'\n') {
                        self.bump();
                    }
                    TokKind::Space
                } else {
                    self.punct(ch, line, col)
                }
            }
        };
        self.toks.push(Tok { kind, start: start as u32, end: self.pos as u32, line, col, nl: false });
    }

    fn punct(&mut self, ch: char, line: u32, col: u32) -> TokKind {
        use TokKind::*;
        let two = |a: u8, b: u8, s: &Self| s.peek(0) == a && s.peek(1) == b;
        let (kind, n) = if two(b'.', b'.', self) {
            (DotDot, 2)
        } else if two(b'=', b'=', self) {
            (EqEq, 2)
        } else if two(b'!', b'=', self) {
            (NotEq, 2)
        } else if two(b'<', b'=', self) {
            (Le, 2)
        } else if two(b'>', b'=', self) {
            (Ge, 2)
        } else if two(b'&', b'&', self) {
            (AndAnd, 2)
        } else if two(b'|', b'|', self) {
            (OrOr, 2)
        } else if two(b'?', b'?', self) {
            (Coalesce, 2)
        } else if two(b'+', b'=', self) {
            (PlusEq, 2)
        } else if two(b'-', b'=', self) {
            (MinusEq, 2)
        } else if two(b'*', b'=', self) {
            (StarEq, 2)
        } else if two(b'/', b'=', self) {
            (SlashEq, 2)
        } else if two(b'%', b'=', self) {
            (PercentEq, 2)
        } else {
            let k = match ch {
                '(' => LParen,
                ')' => RParen,
                '[' => LBracket,
                ']' => RBracket,
                '{' => LBrace,
                '}' => RBrace,
                ',' => Comma,
                '.' => Dot,
                ':' => Colon,
                ';' => Semi,
                '+' => Plus,
                '-' => Minus,
                '*' => Star,
                '/' => Slash,
                '%' => Percent,
                '=' => Assign,
                '!' => Not,
                '<' => Lt,
                '>' => Gt,
                _ => Error,
            };
            (k, 1)
        };
        match kind {
            LBrace => {
                if let Some(f) = self.interp.last_mut() {
                    f.0 += 1;
                }
            }
            RBrace => {
                if let Some(f) = self.interp.last_mut() {
                    f.0 -= 1;
                }
            }
            Error => {
                let hint = match ch {
                    '&' => " (für „und“ schreibe &&)",
                    '|' => " (für „oder“ schreibe ||)",
                    '?' => " (meinst du ?? für einen Ersatzwert?)",
                    '\'' => " (Zeichenketten stehen in doppelten Anführungszeichen \"…\")",
                    '$' => " (${…} gibt es nur in Zeichenketten)",
                    _ => "",
                };
                let shown = if ch.is_ascii_graphic() {
                    format!("{}", ch)
                } else if ch.is_control() {
                    format!("U+{:04X}", ch as u32)
                } else {
                    format!("{} (U+{:04X})", ch, ch as u32)
                };
                self.error(line, col, col + 1, format!("Unerwartetes Zeichen {}{}", shown, hint));
            }
            _ => {}
        }
        self.bump_n(n);
        kind
    }

    /// the rest of a string after `"` (open) or after the `}` of an interpolation
    fn string(&mut self, s: usize, line: u32, col: u32, open: bool) -> TokKind {
        loop {
            let c = self.peek(0);
            if self.pos >= self.b.len() || c == b'\n' || (c == b'\r' && self.peek(1) == b'\n') {
                // one report for this string and the interpolations around it (the outermost)
                if self.interp.is_empty() {
                    self.interp.push((0, s, line, col));
                }
                self.open_strings();
                return if open { TokKind::Str } else { TokKind::StrTail };
            }
            match c {
                b'"' => {
                    self.bump();
                    return if open { TokKind::Str } else { TokKind::StrTail };
                }
                b'$' if self.peek(1) == b'{' => {
                    self.bump_n(2);
                    self.interp.push((0, s, line, col));
                    return if open { TokKind::StrHead } else { TokKind::StrMid };
                }
                b'\\' => self.escape(),
                _ => self.bump(),
            }
        }
    }

    fn escape(&mut self) {
        let (line, col) = (self.line, self.col);
        self.bump();
        let c = self.peek(0);
        match c {
            b'n' | b't' | b'r' | b'0' | b'"' | b'\\' | b'$' => self.bump(),
            b'u' => {
                self.bump();
                let mut ok = false;
                if self.peek(0) == b'{' {
                    self.bump();
                    let d0 = self.pos;
                    while self.peek(0).is_ascii_hexdigit() {
                        self.bump();
                    }
                    let digits = &self.src[d0..self.pos];
                    let v = u32::from_str_radix(digits, 16).ok().filter(|_| digits.len() <= 6);
                    ok = self.peek(0) == b'}' && v.and_then(char::from_u32).is_some();
                }
                if ok {
                    self.bump();
                } else {
                    let msg = "Ungültige Escape-Sequenz: \\u{…} braucht 1 bis 6 Hex-Ziffern eines Unicode-Zeichens";
                    self.error(line, col, self.col, String::from(msg));
                }
            }
            // the string ends here: string() reports it
            _ if self.pos >= self.b.len() || c == b'\n' || (c == b'\r' && self.peek(1) == b'\n') => {}
            _ => {
                let ch = self.ch();
                self.bump();
                self.error(line, col, self.col, format!("Unbekannte Escape-Sequenz \\{} (erlaubt: \\n \\t \\r \\0 \\\" \\\\ \\$ \\u{{…}})", ch));
            }
        }
    }

    fn number(&mut self) -> TokKind {
        let (start, line, col) = (self.pos, self.line, self.col);
        let mut kind = TokKind::Int;
        if self.peek(0) == b'0' && matches!(self.peek(1), b'x' | b'X' | b'b' | b'B') {
            self.bump_n(2);
        } else {
            self.digits();
            if self.peek(0) == b'.' && self.peek(1).is_ascii_digit() {
                kind = TokKind::Float;
                self.bump();
                self.digits();
            }
            let exp = (self.peek(0) | 0x20) == b'e';
            if exp && (self.peek(1).is_ascii_digit() || (matches!(self.peek(1), b'+' | b'-') && self.peek(2).is_ascii_digit())) {
                kind = TokKind::Float;
                self.bump_n(2);
                self.digits();
            }
        }
        while self.pos < self.b.len() && is_ident_char(self.ch()) {
            self.bump();
        }
        let text = &self.src[start..self.pos];
        let too_big = match kind {
            TokKind::Int if int_value(text).is_some() => return kind,
            TokKind::Int => int_digits(text).is_some(),
            _ => match float_value(text) {
                Some(v) if v.is_finite() => return kind,
                v => v.is_some(),
            },
        };
        let msg = if too_big {
            format!("Die Zahl {} ist zu groß (ganze Zahlen höchstens 9223372036854775807)", text)
        } else {
            format!("Ungültige Zahl {}", text)
        };
        self.error(line, col, self.col, msg);
        TokKind::Error
    }

    fn digits(&mut self) {
        while self.peek(0).is_ascii_digit() || self.peek(0) == b'_' {
            self.bump();
        }
    }
}

/// (radix, digits) of an integer literal when it has only digits of its radix and `_`
fn int_digits(text: &str) -> Option<(u32, &[u8])> {
    let t = text.as_bytes();
    let (radix, digits) = match t {
        [b'0', b'x' | b'X', rest @ ..] => (16, rest),
        [b'0', b'b' | b'B', rest @ ..] => (2, rest),
        _ => (10, t),
    };
    let ok = digits.iter().any(|&c| c != b'_') && digits.iter().all(|&c| c == b'_' || (c as char).is_digit(radix));
    ok.then_some((radix, digits))
}

/// the value of an Int token: decimal up to i64::MAX, 0x / 0b with up to 64 bits (two's
/// complement), `_` between digits
pub fn int_value(text: &str) -> Option<i64> {
    let (radix, digits) = int_digits(text)?;
    let mut v: u64 = 0;
    for &c in digits {
        if let Some(d) = (c as char).to_digit(radix) {
            v = v.checked_mul(radix as u64)?.checked_add(d as u64)?;
        }
    }
    if radix == 10 && v > i64::MAX as u64 {
        return None;
    }
    Some(v as i64)
}

/// the value of a Float token (`_` between digits)
pub fn float_value(text: &str) -> Option<f64> {
    let s: String = text.chars().filter(|&c| c != '_').collect();
    if !s.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-')) {
        return None;
    }
    s.parse::<f64>().ok()
}

/// the text between the delimiters of a Str / StrHead / StrMid / StrTail token
pub fn str_inner(kind: TokKind, text: &str) -> &str {
    let a = 1.min(text.len());
    let b = match kind {
        TokKind::StrHead | TokKind::StrMid => text.len().saturating_sub(2),
        _ if text.len() > a && text.ends_with('"') && text[..text.len() - 1].bytes().rev().take_while(|&c| c == b'\\').count() % 2 == 0 => text.len() - 1,
        _ => text.len(),
    };
    text.get(a..b.max(a)).unwrap_or("")
}

/// the value of a string token: its inner text with the escapes decoded (bad escapes, which
/// lex() reported, are dropped)
pub fn str_value(kind: TokKind, text: &str) -> String {
    let s = str_inner(kind, text);
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some(c @ ('"' | '\\' | '$')) => out.push(c),
            Some('u') => {
                let rest = it.as_str();
                if let Some(end) = rest.strip_prefix('{').and_then(|r| r.find('}')) {
                    if let Some(ch) = u32::from_str_radix(&rest[1..end + 1], 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                    it = rest[end + 2..].chars();
                }
            }
            _ => {}
        }
    }
    out
}
