// highlight -- syntax highlighting for ui.code_editor: a small engine of token rules, the
// built-in grammars (Rust, JSON, INI, TOML, a Markdown subset, JFS: the scripts of
// lib/script) and the colours of the tokens
//
//   Highlighter::rust() / json() / ini() / toml() / markdown() / jfs() / plain()
//   Highlighter::for_name(s)            by name or file extension ("rs", "json", "ini", "cfg",
//                                       "toml", "md", "jfs", a path); plain() for anything else
//   Highlighter::new(&GRAMMAR)          an app's own grammar (a static Grammar of Rules)
//   h.line(text, state, &mut tokens) -> state   the tokens of one line (SY_* kinds, byte
//                                       ranges; the plain text between them is SY_TEXT) from
//                                       the state the line before ended in; 0 = nothing open
//   h.tokens(text) -> Vec<Vec<Token>>   every line of a text (tests)
//   CodeTheme::light() / dark() / high_contrast(&HcColors), ui.code_theme()   the colours of
//                                       the tokens and the editor (gutter, current line,
//                                       selection, bracket match, find matches, diagnostics)
//
// A grammar is a list of rules tried in order at every position: line comments, blocks (from
// open to close, nested or not, optionally only at a line start: Markdown fences), strings
// (escapes, multi-line or not), strings with code inside (Interp: the ${..} of JFS, whose
// code the same rules highlight), Rust raw strings, word lists, numbers, punctuation and
// functions of its own (Custom: JSON keys, Rust char literals / lifetimes / attributes, INI
// sections and keys, Markdown headings, lists, emphasis and links).  Where no rule matches,
// an identifier is read whole: before '(' a function (calls), before '!' a macro (macros),
// capitalised a type (caps_types).  A construct still open at a line end is the line's end
// state: rule index + 1, the nesting depth (bits 8..15) and a Rust raw string's '#' count
// (bits 16..31); the editor keeps a state per line and highlights only the visible lines.

use alloc::vec::Vec;

/// token kinds: plain text
pub const SY_TEXT: u8 = 0;
/// token kinds: keywords
pub const SY_KEYWORD: u8 = 1;
/// types (lists, capitalised names)
pub const SY_TYPE: u8 = 2;
/// functions, macros
pub const SY_FUNC: u8 = 3;
/// strings, character literals
pub const SY_STRING: u8 = 4;
/// numbers, dates
pub const SY_NUMBER: u8 = 5;
/// comments, Markdown quotations
pub const SY_COMMENT: u8 = 6;
/// true / false / null, enum constants
pub const SY_CONST: u8 = 7;
/// operators, brackets, separators
pub const SY_PUNCT: u8 = 8;
/// attributes, lifetimes, preprocessor lines
pub const SY_META: u8 = 9;
/// JSON / INI / TOML keys
pub const SY_KEY: u8 = 10;
/// Markdown headings, INI / TOML sections
pub const SY_HEADING: u8 = 11;
/// Markdown *emphasis*
pub const SY_EMPH: u8 = 12;
/// Markdown **strong**
pub const SY_STRONG: u8 = 13;
/// links, URLs
pub const SY_LINK: u8 = 14;
/// Markdown code spans and blocks
pub const SY_CODE: u8 = 15;
/// escape sequences in strings
pub const SY_ESCAPE: u8 = 16;
/// the number of kinds
pub const SY_COUNT: usize = 17;

/// a highlighted part of a line: bytes start .. end
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub kind: u8,
}

/// a rule of a grammar (see the module header)
pub enum Rule {
    /// from the text to the end of the line: a comment
    LineComment(&'static str),
    /// from open to close (nested: opens inside count; bol: open and close only at the start
    /// of a line, after blanks -- the whole line belongs to it); may span lines
    Block { open: &'static str, close: &'static str, kind: u8, nested: bool, bol: bool },
    /// a string from open to close; escape: a backslash takes the next character (SY_ESCAPE);
    /// multi: may span lines
    Str { open: &'static str, close: &'static str, escape: bool, multi: bool, kind: u8 },
    /// Rust raw strings: r"..", r#".."#, br".." (may span lines)
    RawStr,
    /// a string with code inside (interpolation), on one line: from open to close, a backslash
    /// takes the next character (\u{..} its hex digits and brace; SY_ESCAPE); from `start`
    /// (ending with '{') to the '}' that closes it (braces inside count) the grammar's rules
    /// highlight code, the two delimiters SY_META
    Interp { open: &'static str, close: &'static str, start: &'static str },
    /// identifiers of a list
    Words(&'static [&'static str], u8),
    /// number literals: 12, 0x1F, 0b101, 0o17, 1_000, 1.5e-3, suffixes (u8, f32)
    Number,
    /// a function of its own: (line, position, at the line start after blanks) -> (bytes, kind)
    Custom(fn(&str, usize, bool) -> Option<(usize, u8)>),
    /// single characters
    Punct(&'static str),
}

/// a language (see the module header)
pub struct Grammar {
    pub name: &'static str,
    /// file extensions (lower case, without the dot)
    pub exts: &'static [&'static str],
    pub rules: &'static [Rule],
    /// characters inside identifiers besides letters, digits and '_'
    pub ident: &'static str,
    /// an identifier before '(' is a function
    pub calls: bool,
    /// an identifier before '!' is a macro (Rust)
    pub macros: bool,
    /// capitalised identifiers are types
    pub caps_types: bool,
    /// the line comment of Ctrl+/ ("": none)
    pub comment: &'static str,
    /// Enter after one of these indents one level more
    pub openers: &'static str,
    /// one of these typed on a blank line goes one level out
    pub closers: &'static str,
    /// the bracket pairs the editor matches: "()[]{}"
    pub pairs: &'static str,
}

/// the highlighter of a grammar (see the module header)
#[derive(Clone, Copy)]
pub struct Highlighter {
    g: &'static Grammar,
}

fn push(out: &mut Vec<Token>, start: usize, end: usize, kind: u8) {
    if end <= start {
        return;
    }
    if let Some(t) = out.last_mut() {
        if t.kind == kind && t.end == start {
            t.end = end;
            return;
        }
    }
    out.push(Token { start, end, kind });
}

fn enc(rule: usize, depth: u32, extra: u32) -> u32 {
    (rule as u32 + 1) & 0xFF | (depth.min(255) << 8) | (extra.min(0xFFFF) << 16)
}

fn utf8_len(b: u8) -> usize {
    match b {
        0..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

impl Highlighter {
    /// an app's own grammar
    pub const fn new(g: &'static Grammar) -> Highlighter {
        Highlighter { g }
    }
    /// plain text: no tokens
    pub const fn plain() -> Highlighter {
        Highlighter { g: &PLAIN }
    }
    /// Rust
    pub const fn rust() -> Highlighter {
        Highlighter { g: &RUST }
    }
    /// JSON (and JSONC comments)
    pub const fn json() -> Highlighter {
        Highlighter { g: &JSON }
    }
    /// INI files
    pub const fn ini() -> Highlighter {
        Highlighter { g: &INI }
    }
    /// TOML
    pub const fn toml() -> Highlighter {
        Highlighter { g: &TOML }
    }
    /// a Markdown subset
    pub const fn markdown() -> Highlighter {
        Highlighter { g: &MARKDOWN }
    }
    /// the scripting language of lib/script (.jfs, JFNetworker's scripts)
    pub const fn jfs() -> Highlighter {
        Highlighter { g: &JFS }
    }

    /// by name or extension, also of a path ("rs", "Rust", "x/y.toml"); plain() for others
    pub fn for_name(s: &str) -> Highlighter {
        let s = s.trim();
        let ext = s.rsplit(['/', '\\']).next().unwrap_or(s);
        let ext = ext.rsplit('.').next().unwrap_or(ext).to_ascii_lowercase();
        for g in [&RUST, &JSON, &INI, &TOML, &MARKDOWN, &JFS] {
            if g.name.eq_ignore_ascii_case(s) || g.exts.contains(&ext.as_str()) {
                return Highlighter { g };
            }
        }
        Highlighter::plain()
    }

    /// the grammar (comment prefix, openers, closers, bracket pairs)
    pub fn grammar(&self) -> &'static Grammar {
        self.g
    }

    /// the grammar's name ("Rust", "JSON" ...)
    pub fn name(&self) -> &'static str {
        self.g.name
    }

    fn ident_char(&self, c: char) -> bool {
        c.is_alphanumeric() || c == '_' || self.g.ident.contains(c)
    }

    // the identifier starting at i (an index past its end) or None
    fn ident_at(&self, t: &str, i: usize) -> Option<usize> {
        let c = t[i..].chars().next()?;
        if !(c.is_alphabetic() || c == '_') {
            return None;
        }
        let mut e = i + c.len_utf8();
        for ch in t[e..].chars() {
            if !self.ident_char(ch) {
                break;
            }
            e += ch.len_utf8();
        }
        Some(e)
    }

    fn prev_ident(&self, t: &str, i: usize) -> bool {
        t[..i].chars().next_back().is_some_and(|c| self.ident_char(c))
    }

    /// the tokens of one line from the state of the line before -> the state at its end
    pub fn line(&self, t: &str, state: u32, out: &mut Vec<Token>) -> u32 {
        out.clear();
        self.line_part(t, state, 0, out, &mut |_, _| false).1.unwrap_or(0)
    }

    // (code_long.rs) the tokens of line t from `from` on -- 0, or a place outside every
    // construct that a scan of the line from `state` passed -- appended to out; at(i, out) sees
    // each place outside every construct from `from` on (and the tokens so far), true stops
    // there -> (where the scan stopped, the state at the line end; None: at stopped it)
    pub(crate) fn line_part(&self, t: &str, state: u32, from: usize, out: &mut Vec<Token>, at: &mut dyn FnMut(usize, &[Token]) -> bool) -> (usize, Option<u32>) {
        let mut i = from;
        let bol = t.len() - t.trim_start_matches([' ', '\t']).len();
        if from == 0 && state != 0 {
            let (e, st) = self.resume(t, state, bol, out);
            if st != 0 {
                return (t.len(), Some(st));
            }
            i = e;
        }
        self.scan(t, i, bol, false, out, at)
    }

    // the tokens from i on (code: inside an interpolation, up to the '}' that closes it; else
    // at(i) sees each place between constructs, true stops there) -> (where it stopped, the
    // state at the line end: 0 = nothing open; None: at stopped it)
    fn scan(&self, t: &str, mut i: usize, bol: usize, code: bool, out: &mut Vec<Token>, at: &mut dyn FnMut(usize, &[Token]) -> bool) -> (usize, Option<u32>) {
        let b = t.as_bytes();
        let n = b.len();
        let rules = self.g.rules;
        let mut braces = 0u32;
        'outer: while i < n {
            if !code && at(i, out) {
                return (i, None);
            }
            if code && b[i] == b'{' {
                braces += 1;
            } else if code && b[i] == b'}' {
                if braces == 0 {
                    return (i, Some(0));
                }
                braces -= 1;
            }
            let at_bol = i == bol;
            let prev = self.prev_ident(t, i);
            for (ri, rule) in rules.iter().enumerate() {
                match rule {
                    Rule::LineComment(p) => {
                        if t[i..].starts_with(p) {
                            push(out, i, n, SY_COMMENT);
                            return (n, Some(0));
                        }
                    }
                    Rule::Block { open, close, kind, nested, bol: only } => {
                        if (!*only || at_bol) && t[i..].starts_with(open) && !(prev && open.starts_with(|c: char| c.is_alphanumeric())) {
                            if *only {
                                push(out, i, n, *kind);
                                return (n, Some(enc(ri, 1, 0)));
                            }
                            let (e, depth) = scan_block(t, i + open.len(), open, close, *nested, 1);
                            push(out, i, e, *kind);
                            if depth > 0 {
                                return (n, Some(enc(ri, depth, 0)));
                            }
                            i = e;
                            continue 'outer;
                        }
                    }
                    Rule::Str { open, close, escape, multi, kind } => {
                        if t[i..].starts_with(open) && !(prev && open.starts_with(|c: char| c.is_alphanumeric())) {
                            let (e, done) = scan_str(t, i, i + open.len(), close, *escape, *kind, out);
                            if !done && *multi {
                                return (n, Some(enc(ri, 0, 0)));
                            }
                            i = e;
                            continue 'outer;
                        }
                    }
                    Rule::RawStr => {
                        if let Some((q, hashes)) = raw_open(t, i).filter(|_| !prev) {
                            match raw_close(t, q, hashes) {
                                Some(e) => {
                                    push(out, i, e, SY_STRING);
                                    i = e;
                                    continue 'outer;
                                }
                                None => {
                                    push(out, i, n, SY_STRING);
                                    return (n, Some(enc(ri, 0, hashes as u32)));
                                }
                            }
                        }
                    }
                    Rule::Interp { open, close, start } => {
                        if t[i..].starts_with(open) && !(prev && open.starts_with(|c: char| c.is_alphanumeric())) {
                            let (e, st) = self.interp(t, i, open.len(), close, start, bol, out);
                            if st != 0 {
                                return (n, Some(st));
                            }
                            i = e;
                            continue 'outer;
                        }
                    }
                    Rule::Words(list, kind) => {
                        if !prev {
                            if let Some(e) = self.ident_at(t, i) {
                                if list.contains(&&t[i..e]) {
                                    push(out, i, e, *kind);
                                    i = e;
                                    continue 'outer;
                                }
                            }
                        }
                    }
                    Rule::Number => {
                        if !prev && b[i].is_ascii_digit() {
                            let e = number_end(t, i);
                            push(out, i, e, SY_NUMBER);
                            i = e;
                            continue 'outer;
                        }
                    }
                    Rule::Custom(f) => {
                        if let Some((len, kind)) = f(t, i, at_bol) {
                            if len > 0 {
                                let e = (i + len).min(n);
                                push(out, i, e, kind);
                                i = e;
                                while i < n && !t.is_char_boundary(i) {
                                    i += 1;
                                }
                                continue 'outer;
                            }
                        }
                    }
                    Rule::Punct(chars) => {
                        if b[i] < 0x80 && chars.as_bytes().contains(&b[i]) {
                            push(out, i, i + 1, SY_PUNCT);
                            i += 1;
                            continue 'outer;
                        }
                    }
                }
            }
            // no rule: an identifier whole, else one character
            if !prev {
                if let Some(e) = self.ident_at(t, i) {
                    let g = self.g;
                    let mut next = e;
                    while next < n && b[next] == b' ' {
                        next += 1;
                    }
                    let kind = if g.macros && b.get(e) == Some(&b'!') && b.get(e + 1) != Some(&b'=') {
                        SY_FUNC
                    } else if g.calls && b.get(next) == Some(&b'(') {
                        SY_FUNC
                    } else if g.caps_types && t[i..].chars().next().is_some_and(|c| c.is_uppercase()) {
                        SY_TYPE
                    } else {
                        SY_TEXT
                    };
                    if kind != SY_TEXT {
                        push(out, i, if kind == SY_FUNC && g.macros && b.get(e) == Some(&b'!') { e + 1 } else { e }, kind);
                        if kind == SY_FUNC && g.macros && b.get(e) == Some(&b'!') {
                            i = e + 1;
                            continue;
                        }
                    }
                    i = e;
                    continue;
                }
            }
            i += utf8_len(b[i]);
        }
        (n, Some(0))
    }

    // a string with interpolations from its open quote at i (open: its length) -> (its end,
    // the state at the line end: a construct inside an interpolation stays open)
    #[allow(clippy::too_many_arguments)]
    fn interp(&self, t: &str, i: usize, open: usize, close: &str, start: &str, bol: usize, out: &mut Vec<Token>) -> (usize, u32) {
        let b = t.as_bytes();
        let mut s = i;
        let mut j = i + open;
        while j < b.len() {
            if b[j] == b'\\' {
                push(out, s, j, SY_STRING);
                let mut e = (j + 1 + b.get(j + 1).map_or(0, |&c| utf8_len(c))).min(b.len());
                if b.get(j + 1) == Some(&b'u') && b.get(j + 2) == Some(&b'{') {
                    e = j + 3 + b[j + 3..].iter().take_while(|c| c.is_ascii_hexdigit()).count();
                    if b.get(e) == Some(&b'}') {
                        e += 1;
                    }
                }
                while e < b.len() && !t.is_char_boundary(e) {
                    e += 1;
                }
                push(out, j, e, SY_ESCAPE);
                j = e;
                s = e;
                continue;
            }
            if t[j..].starts_with(close) {
                push(out, s, j + close.len(), SY_STRING);
                return (j + close.len(), 0);
            }
            if t[j..].starts_with(start) {
                push(out, s, j, SY_STRING);
                push(out, j, j + start.len(), SY_META);
                let (e, st) = self.scan(t, j + start.len(), bol, true, out, &mut |_, _| false);
                let st = st.unwrap_or(0);
                if st != 0 || e >= b.len() {
                    return (b.len(), st);
                }
                push(out, e, e + 1, SY_META);
                j = e + 1;
                s = j;
                continue;
            }
            j += utf8_len(b[j]);
        }
        push(out, s, b.len(), SY_STRING);
        (b.len(), 0)
    }

    // continue a construct open at the line start -> (where it ends, the state: 0 closed)
    fn resume(&self, t: &str, state: u32, bol: usize, out: &mut Vec<Token>) -> (usize, u32) {
        let n = t.len();
        let ri = (state & 0xFF) as usize - 1;
        let Some(rule) = self.g.rules.get(ri) else { return (0, 0) };
        match rule {
            Rule::Block { open, close, kind, nested, bol: only } => {
                if *only {
                    push(out, 0, n, *kind);
                    return if t[bol..].starts_with(close) { (n, 0) } else { (n, state) };
                }
                let (e, depth) = scan_block(t, 0, open, close, *nested, (state >> 8) & 0xFF);
                push(out, 0, e, *kind);
                if depth > 0 { (n, enc(ri, depth, 0)) } else { (e, 0) }
            }
            Rule::Str { close, escape, kind, .. } => {
                let (e, done) = scan_str(t, 0, 0, close, *escape, *kind, out);
                if done { (e, 0) } else { (n, state) }
            }
            Rule::RawStr => {
                let hashes = (state >> 16) as usize;
                match raw_close(t, 0, hashes) {
                    Some(e) => {
                        push(out, 0, e, SY_STRING);
                        (e, 0)
                    }
                    None => {
                        push(out, 0, n, SY_STRING);
                        (n, state)
                    }
                }
            }
            _ => (0, 0),
        }
    }

    /// the tokens of every line of a text (tests)
    pub fn tokens(&self, text: &str) -> Vec<Vec<Token>> {
        let mut st = 0;
        let mut out = Vec::new();
        for l in crate::rich_doc::lines_of(text) {
            let mut v = Vec::new();
            st = self.line(l, st, &mut v);
            out.push(v);
        }
        out
    }
}

// from i inside a block (depth open blocks): the end and the depth left (0: closed)
fn scan_block(t: &str, i: usize, open: &str, close: &str, nested: bool, depth: u32) -> (usize, u32) {
    let mut d = depth;
    let mut j = i;
    while j < t.len() {
        if t[j..].starts_with(close) {
            d -= 1;
            j += close.len();
            if d == 0 {
                return (j, 0);
            }
            continue;
        }
        if nested && t[j..].starts_with(open) {
            d += 1;
            j += open.len();
            continue;
        }
        j += utf8_len(t.as_bytes()[j]);
    }
    (t.len(), d.max(1))
}

// a string from start (its content from i): tokens with escapes -> (end, closed)
fn scan_str(t: &str, start: usize, i: usize, close: &str, escape: bool, kind: u8, out: &mut Vec<Token>) -> (usize, bool) {
    let b = t.as_bytes();
    let mut s = start;
    let mut j = i;
    while j < b.len() {
        if escape && b[j] == b'\\' {
            push(out, s, j, kind);
            let mut e = (j + 1 + b.get(j + 1).map_or(0, |&c| utf8_len(c))).min(b.len());
            if b.get(j + 1) == Some(&b'u') && b.get(j + 2) == Some(&b'{') {
                e = t[j..].find('}').map_or(b.len(), |k| j + k + 1);
            } else if b.get(j + 1) == Some(&b'x') {
                e = (j + 4).min(b.len());
            } else if b.get(j + 1) == Some(&b'u') && kind == SY_STRING {
                e = (j + 6).min(b.len());
            }
            while e < b.len() && !t.is_char_boundary(e) {
                e += 1;
            }
            push(out, j, e, SY_ESCAPE);
            j = e;
            s = e;
            continue;
        }
        if t[j..].starts_with(close) {
            let e = j + close.len();
            push(out, s, e, kind);
            return (e, true);
        }
        j += utf8_len(b[j]);
    }
    push(out, s, b.len(), kind);
    (b.len(), false)
}

// a Rust raw string at i: (the index after its opening quote, the '#' count)
fn raw_open(t: &str, i: usize) -> Option<(usize, usize)> {
    let b = t.as_bytes();
    let mut j = i;
    if b.get(j) == Some(&b'b') || b.get(j) == Some(&b'c') {
        j += 1;
    }
    if b.get(j) != Some(&b'r') {
        return None;
    }
    j += 1;
    let mut h = 0;
    while b.get(j) == Some(&b'#') {
        h += 1;
        j += 1;
    }
    (b.get(j) == Some(&b'"')).then_some((j + 1, h))
}

// the end of a raw string's closing quote and hashes from i (None: not on this line)
fn raw_close(t: &str, i: usize, hashes: usize) -> Option<usize> {
    let b = t.as_bytes();
    let mut j = i;
    while j < b.len() {
        if b[j] == b'"' && b[j + 1..].iter().take(hashes).filter(|&&c| c == b'#').count() == hashes && b.len() >= j + 1 + hashes {
            return Some(j + 1 + hashes);
        }
        j += 1;
    }
    None
}

// the end of the number literal at i
fn number_end(t: &str, i: usize) -> usize {
    let b = t.as_bytes();
    let mut j = i;
    let hex = b[i] == b'0' && matches!(b.get(i + 1), Some(b'x') | Some(b'X'));
    if hex || (b[i] == b'0' && matches!(b.get(i + 1), Some(b'b') | Some(b'o') | Some(b'B') | Some(b'O'))) {
        j += 2;
    }
    while j < b.len() {
        let c = b[j];
        if c.is_ascii_alphanumeric() || c == b'_' {
            // an exponent's sign
            if !hex && (c == b'e' || c == b'E') && matches!(b.get(j + 1), Some(b'+') | Some(b'-')) && b.get(j + 2).is_some_and(|d| d.is_ascii_digit()) {
                j += 2;
                continue;
            }
            j += 1;
        } else if c == b'.' && b.get(j + 1).is_some_and(|d| d.is_ascii_digit()) && !t[i..j].contains('.') {
            j += 1;
        } else {
            break;
        }
    }
    j
}

// ------------------------------------------------------------------ custom rules

// Rust: 'a' / '\n' / '\u{..}' char literals, else a lifetime 'a
fn rust_quote(t: &str, i: usize, _: bool) -> Option<(usize, u8)> {
    let b = t.as_bytes();
    if b[i] != b'\'' {
        return None;
    }
    if b.get(i + 1) == Some(&b'\\') {
        let e = t[i + 2..].find('\'').map_or(t.len(), |k| i + 2 + k + 1);
        return Some((e - i, SY_STRING));
    }
    let c = t[i + 1..].chars().next()?;
    let after = i + 1 + c.len_utf8();
    if b.get(after) == Some(&b'\'') {
        return Some((after + 1 - i, SY_STRING));
    }
    if c.is_alphabetic() || c == '_' {
        let mut e = after;
        for ch in t[after..].chars() {
            if !(ch.is_alphanumeric() || ch == '_') {
                break;
            }
            e += ch.len_utf8();
        }
        return Some((e - i, SY_META));
    }
    None
}

// Rust: #[...] / #![...] attributes (brackets counted, to the line end when unclosed)
fn rust_attr(t: &str, i: usize, _: bool) -> Option<(usize, u8)> {
    let r = &t[i..];
    let start = if r.starts_with("#[") {
        2
    } else if r.starts_with("#![") {
        3
    } else {
        return None;
    };
    let mut d = 1;
    let mut in_str = false;
    for (k, c) in r[start..].char_indices() {
        match c {
            '"' => in_str = !in_str,
            '[' if !in_str => d += 1,
            ']' if !in_str => {
                d -= 1;
                if d == 0 {
                    return Some((start + k + 1, SY_META));
                }
            }
            _ => {}
        }
    }
    Some((r.len(), SY_META))
}

// JSON: a string; followed by ':' a key
fn json_string(t: &str, i: usize, _: bool) -> Option<(usize, u8)> {
    let b = t.as_bytes();
    if b[i] != b'"' {
        return None;
    }
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'"' => {
                j += 1;
                let rest = t[j.min(t.len())..].trim_start();
                return Some((j - i, if rest.starts_with(':') { SY_KEY } else { SY_STRING }));
            }
            _ => j += 1,
        }
    }
    Some((b.len() - i, SY_STRING))
}

// INI / TOML: [section] / [[table]] at a line start
fn ini_section(t: &str, i: usize, bol: bool) -> Option<(usize, u8)> {
    if !bol || t.as_bytes()[i] != b'[' {
        return None;
    }
    let e = t[i..].find(']').map_or(t.len(), |k| {
        let mut e = i + k + 1;
        if t[e..].starts_with(']') {
            e += 1;
        }
        e
    });
    Some((e - i, SY_HEADING))
}

// INI / TOML: the key before '=' (INI also ':') at a line start
fn ini_key(t: &str, i: usize, bol: bool) -> Option<(usize, u8)> {
    if !bol {
        return None;
    }
    let r = &t[i..];
    let c = r.chars().next()?;
    if matches!(c, '[' | ';' | '#' | '=') {
        return None;
    }
    let sep = r.find(['=', ':'])?;
    let key = r[..sep].trim_end();
    if key.is_empty() || key.contains(['"', '\'']) && !key.starts_with(['"', '\'']) {
        return None;
    }
    Some((key.len(), SY_KEY))
}

// a URL: http(s)://.. to a blank or a closing bracket / quote
fn url_len(r: &str) -> Option<usize> {
    if !(r.starts_with("http://") || r.starts_with("https://")) {
        return None;
    }
    let e = r.find(|c: char| c.is_whitespace() || matches!(c, ')' | '>' | '"' | '\'' | ']')).unwrap_or(r.len());
    Some(e)
}

// Markdown: '#'.. heading lines
fn md_heading(t: &str, i: usize, bol: bool) -> Option<(usize, u8)> {
    if !bol {
        return None;
    }
    let r = &t[i..];
    let h = r.bytes().take_while(|&c| c == b'#').count();
    if h == 0 || h > 6 || !(r.len() == h || r.as_bytes()[h] == b' ') {
        return None;
    }
    Some((r.len(), SY_HEADING))
}

// Markdown: '>' quotation lines
fn md_quote(t: &str, i: usize, bol: bool) -> Option<(usize, u8)> {
    (bol && t.as_bytes()[i] == b'>').then_some((t.len() - i, SY_COMMENT))
}

// Markdown: ---, *** and ___ rules
fn md_rule(t: &str, i: usize, bol: bool) -> Option<(usize, u8)> {
    if !bol {
        return None;
    }
    let r = t[i..].trim_end();
    let c = r.bytes().next()?;
    if !matches!(c, b'-' | b'*' | b'_') {
        return None;
    }
    let n = r.bytes().filter(|&x| x == c).count();
    (n >= 3 && r.bytes().all(|x| x == c || x == b' ')).then_some((t.len() - i, SY_PUNCT))
}

// Markdown: list markers "- ", "* ", "+ ", "1. ", "1) "
fn md_list(t: &str, i: usize, bol: bool) -> Option<(usize, u8)> {
    if !bol {
        return None;
    }
    let b = &t.as_bytes()[i..];
    if b.len() >= 2 && matches!(b[0], b'-' | b'*' | b'+') && b[1] == b' ' {
        return Some((1, SY_KEYWORD));
    }
    let d = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if d > 0 && d < 10 && b.len() > d + 1 && matches!(b[d], b'.' | b')') && b[d + 1] == b' ' {
        return Some((d + 1, SY_KEYWORD));
    }
    None
}

// Markdown: **strong** / __strong__ and *emphasis* / _emphasis_ (not inside words)
fn md_emph(t: &str, i: usize, _: bool) -> Option<(usize, u8)> {
    let b = t.as_bytes();
    let c = b[i];
    if c != b'*' && c != b'_' {
        return None;
    }
    if c == b'_' && t[..i].chars().next_back().is_some_and(|p| p.is_alphanumeric()) {
        return None;
    }
    let strong = b.get(i + 1) == Some(&c);
    let delim = if strong { 2 } else { 1 };
    let inner = i + delim;
    if b.get(inner).is_none_or(|x| *x == b' ' || *x == c) {
        return None;
    }
    let pat: &[u8] = if strong { &b[i..i + 2] } else { &b[i..i + 1] };
    let mut j = inner + 1;
    while j + delim <= b.len() {
        if &b[j..j + delim] == pat && b[j - 1] != b' ' && (strong || b.get(j + 1) != Some(&c)) {
            if c == b'_' && t[j + delim..].chars().next().is_some_and(|p| p.is_alphanumeric()) {
                j += 1;
                continue;
            }
            return Some((j + delim - i, if strong { SY_STRONG } else { SY_EMPH }));
        }
        j += 1;
    }
    None
}

// Markdown: [text](url), ![alt](url), <url>, bare URLs
fn md_link(t: &str, i: usize, _: bool) -> Option<(usize, u8)> {
    let r = &t[i..];
    if let Some(e) = url_len(r) {
        return Some((e, SY_LINK));
    }
    if r.starts_with('<') {
        let e = r.find('>')?;
        return url_len(&r[1..]).filter(|&u| u + 1 == e).map(|_| (e + 1, SY_LINK));
    }
    let s = if r.starts_with("![") { 1 } else if r.starts_with('[') { 0 } else { return None };
    let close = r[s..].find("](")? + s;
    let end = r[close + 2..].find(')')? + close + 3;
    Some((end, SY_LINK))
}

// Markdown: \* and other backslash escapes
fn md_escape(t: &str, i: usize, _: bool) -> Option<(usize, u8)> {
    let b = t.as_bytes();
    (b[i] == b'\\' && b.get(i + 1).is_some_and(|c| c.is_ascii_punctuation())).then_some((2, SY_ESCAPE))
}

// TOML: dates and times (1979-05-27, 07:32:00, 1979-05-27T07:32:00Z) as one number
fn toml_date(t: &str, i: usize, _: bool) -> Option<(usize, u8)> {
    let b = &t.as_bytes()[i..];
    if b.len() < 5 || !b[0].is_ascii_digit() {
        return None;
    }
    let date = b.len() >= 10 && b[4] == b'-' && b[7] == b'-';
    let time = b.len() >= 8 && b[2] == b':' && b[5] == b':';
    if !date && !time {
        return None;
    }
    let e = b.iter().position(|c| !(c.is_ascii_alphanumeric() || matches!(c, b'-' | b':' | b'.' | b'+'))).unwrap_or(b.len());
    Some((e, SY_NUMBER))
}

// ------------------------------------------------------------------ the grammars

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "fn", "for", "if", "impl", "in", "let", "loop",
    "match", "mod", "move", "mut", "pub", "ref", "return", "self", "static", "struct", "super", "trait", "type", "union", "unsafe", "use", "where",
    "while", "yield", "macro_rules", "try", "gen",
];
const RUST_TYPES: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64", "bool", "char", "str", "Self",
];
const RUST_CONSTS: &[&str] = &["true", "false", "None", "Some", "Ok", "Err"];

static RUST_RULES: [Rule; 12] = [
    Rule::LineComment("//"),
    Rule::Block { open: "/*", close: "*/", kind: SY_COMMENT, nested: true, bol: false },
    Rule::RawStr,
    Rule::Str { open: "b\"", close: "\"", escape: true, multi: true, kind: SY_STRING },
    Rule::Str { open: "\"", close: "\"", escape: true, multi: true, kind: SY_STRING },
    Rule::Custom(rust_quote),
    Rule::Custom(rust_attr),
    Rule::Words(RUST_KEYWORDS, SY_KEYWORD),
    Rule::Words(RUST_TYPES, SY_TYPE),
    Rule::Words(RUST_CONSTS, SY_CONST),
    Rule::Number,
    Rule::Punct("+-*/%=<>!&|^~?:;,.()[]{}@#$"),
];

/// Rust
pub static RUST: Grammar = Grammar {
    name: "Rust",
    exts: &["rs"],
    rules: &RUST_RULES,
    ident: "",
    calls: true,
    macros: true,
    caps_types: true,
    comment: "//",
    openers: "{[(",
    closers: "}])",
    pairs: "()[]{}",
};

static JSON_RULES: [Rule; 6] = [
    Rule::LineComment("//"),
    Rule::Block { open: "/*", close: "*/", kind: SY_COMMENT, nested: false, bol: false },
    Rule::Custom(json_string),
    Rule::Words(&["true", "false", "null"], SY_CONST),
    Rule::Number,
    Rule::Punct("{}[],:-"),
];

/// JSON (comments of JSONC too)
pub static JSON: Grammar = Grammar {
    name: "JSON",
    exts: &["json", "jsonc", "geojson"],
    rules: &JSON_RULES,
    ident: "",
    calls: false,
    macros: false,
    caps_types: false,
    comment: "//",
    openers: "{[",
    closers: "}]",
    pairs: "[]{}",
};

const INI_CONSTS: &[&str] = &["true", "false", "yes", "no", "on", "off", "TRUE", "FALSE", "Yes", "No", "On", "Off", "True", "False"];

fn ini_comment(t: &str, i: usize, bol: bool) -> Option<(usize, u8)> {
    (bol && matches!(t.as_bytes()[i], b';' | b'#')).then_some((t.len() - i, SY_COMMENT))
}

static INI_RULES: [Rule; 8] = [
    Rule::Custom(ini_comment),
    Rule::Custom(ini_section),
    Rule::Custom(ini_key),
    Rule::Str { open: "\"", close: "\"", escape: true, multi: false, kind: SY_STRING },
    Rule::Str { open: "'", close: "'", escape: false, multi: false, kind: SY_STRING },
    Rule::Words(INI_CONSTS, SY_CONST),
    Rule::Number,
    Rule::Punct("=:,[]"),
];

/// INI files (; and # comments at a line start, [sections], key = value, key: value)
pub static INI: Grammar = Grammar {
    name: "INI",
    exts: &["ini", "cfg", "conf", "desktop", "properties", "inf", "reg"],
    rules: &INI_RULES,
    ident: "-.",
    calls: false,
    macros: false,
    caps_types: false,
    comment: ";",
    openers: "",
    closers: "",
    pairs: "[]",
};

static TOML_RULES: [Rule; 12] = [
    Rule::LineComment("#"),
    Rule::Custom(ini_section),
    Rule::Custom(ini_key),
    Rule::Str { open: "\"\"\"", close: "\"\"\"", escape: true, multi: true, kind: SY_STRING },
    Rule::Str { open: "'''", close: "'''", escape: false, multi: true, kind: SY_STRING },
    Rule::Str { open: "\"", close: "\"", escape: true, multi: false, kind: SY_STRING },
    Rule::Str { open: "'", close: "'", escape: false, multi: false, kind: SY_STRING },
    Rule::Words(&["true", "false", "inf", "nan"], SY_CONST),
    Rule::Custom(toml_date),
    Rule::Number,
    Rule::Punct("=,.[]{}+-"),
    Rule::Punct(":"),
];

/// TOML
pub static TOML: Grammar = Grammar {
    name: "TOML",
    exts: &["toml", "lock"],
    rules: &TOML_RULES,
    ident: "-",
    calls: false,
    macros: false,
    caps_types: false,
    comment: "#",
    openers: "[{",
    closers: "]}",
    pairs: "[]{}",
};

static MD_RULES: [Rule; 11] = [
    Rule::Block { open: "```", close: "```", kind: SY_CODE, nested: false, bol: true },
    Rule::Block { open: "~~~", close: "~~~", kind: SY_CODE, nested: false, bol: true },
    Rule::Block { open: "<!--", close: "-->", kind: SY_COMMENT, nested: false, bol: false },
    Rule::Custom(md_heading),
    Rule::Custom(md_quote),
    Rule::Custom(md_rule),
    Rule::Custom(md_list),
    Rule::Custom(md_escape),
    Rule::Str { open: "`", close: "`", escape: false, multi: false, kind: SY_CODE },
    Rule::Custom(md_emph),
    Rule::Custom(md_link),
];

/// a Markdown subset: headings, quotations, rules, lists, fenced code, code spans, emphasis,
/// strong, links, escapes, HTML comments
pub static MARKDOWN: Grammar = Grammar {
    name: "Markdown",
    exts: &["md", "markdown", "mdown"],
    rules: &MD_RULES,
    ident: "",
    calls: false,
    macros: false,
    caps_types: false,
    comment: "",
    openers: "",
    closers: "",
    pairs: "()[]",
};

// the lexical rules of lib/script/lex.rs: keywords, true / false / nil, one-line strings
// with escapes and ${..} (code inside, nested strings too), numbers with _, 0x, 0b, a
// fraction and an exponent, // and /* */ comments (not nested)
const JFS_KEYWORDS: &[&str] = &["let", "fn", "if", "else", "while", "for", "in", "break", "continue", "return", "try", "catch", "throw"];

static JFS_RULES: [Rule; 7] = [
    Rule::LineComment("//"),
    Rule::Block { open: "/*", close: "*/", kind: SY_COMMENT, nested: false, bol: false },
    Rule::Interp { open: "\"", close: "\"", start: "${" },
    Rule::Words(JFS_KEYWORDS, SY_KEYWORD),
    Rule::Words(&["true", "false", "nil"], SY_CONST),
    Rule::Number,
    Rule::Punct("+-*/%=<>!&|?:;,.()[]{}"),
];

/// the scripting language of lib/script (.jfs): calls before '(' are functions, the
/// delimiters of ${..} in strings SY_META
pub static JFS: Grammar = Grammar {
    name: "JFS",
    exts: &["jfs"],
    rules: &JFS_RULES,
    ident: "",
    calls: true,
    macros: false,
    caps_types: false,
    comment: "//",
    openers: "{[(",
    closers: "}])",
    pairs: "()[]{}",
};

static PLAIN_RULES: [Rule; 0] = [];

/// plain text: no tokens
pub static PLAIN: Grammar = Grammar {
    name: "Text",
    exts: &["txt", "text", "log"],
    rules: &PLAIN_RULES,
    ident: "",
    calls: false,
    macros: false,
    caps_types: false,
    comment: "",
    openers: "",
    closers: "",
    pairs: "()[]{}",
};

// ------------------------------------------------------------------ colours

/// the colours of a code editor (0xAARRGGBB); bold / italic: a bit per SY_* kind
#[derive(Clone, Debug, PartialEq)]
pub struct CodeTheme {
    pub bg: u32,
    pub fg: u32,
    pub gutter_bg: u32,
    pub gutter_fg: u32,
    /// the current line's number
    pub gutter_cur: u32,
    /// the current line's background
    pub line_hl: u32,
    /// the selection (focused / not focused)
    pub sel: u32,
    pub sel_inactive: u32,
    /// the selected text (0: the tokens keep their colours)
    pub sel_text: u32,
    /// the frame around a matching bracket pair
    pub bracket: u32,
    /// the find matches and the current one
    pub find: u32,
    pub find_cur: u32,
    /// find matches framed instead of filled (high contrast)
    pub find_frame: bool,
    pub caret: u32,
    pub tokens: [u32; SY_COUNT],
    pub bold: u32,
    pub italic: u32,
    /// the wavy underlines and gutter markers of errors, warnings and infos (DIAG_* order)
    pub diag: [u32; 3],
}

const fn argb(rgb: u32) -> u32 {
    0xFF00_0000 | rgb
}

impl CodeTheme {
    /// on a light background (the colours of GitHub's light theme)
    pub fn light() -> CodeTheme {
        let fg = argb(0x1F2328);
        CodeTheme {
            bg: argb(0xFFFFFF),
            fg,
            gutter_bg: argb(0xF6F8FA),
            gutter_fg: argb(0x8C959F),
            gutter_cur: fg,
            line_hl: argb(0xF1F4F8),
            sel: argb(0xB6D7FF),
            sel_inactive: argb(0xE1E6EB),
            sel_text: 0,
            bracket: argb(0x8C959F),
            find: argb(0xFFE58A),
            find_cur: argb(0xFFA94D),
            find_frame: false,
            caret: fg,
            tokens: [
                fg,
                argb(0xCF222E),
                argb(0x953800),
                argb(0x8250DF),
                argb(0x0A3069),
                argb(0x0550AE),
                argb(0x6E7781),
                argb(0x0550AE),
                argb(0x57606A),
                argb(0x116329),
                argb(0x0550AE),
                argb(0x0550AE),
                fg,
                fg,
                argb(0x0969DA),
                argb(0x0A3069),
                argb(0x116329),
            ],
            bold: 1 << SY_HEADING | 1 << SY_STRONG,
            italic: 1 << SY_COMMENT | 1 << SY_EMPH,
            diag: [argb(0xCF222E), argb(0x9A6700), argb(0x0969DA)],
        }
    }

    /// on a dark background (the colours of GitHub's dark theme)
    pub fn dark() -> CodeTheme {
        let fg = argb(0xE6EDF3);
        CodeTheme {
            bg: argb(0x0D1117),
            fg,
            gutter_bg: argb(0x0D1117),
            gutter_fg: argb(0x6E7681),
            gutter_cur: fg,
            line_hl: argb(0x161B22),
            sel: argb(0x264F78),
            sel_inactive: argb(0x30363D),
            sel_text: 0,
            bracket: argb(0x8B949E),
            find: argb(0x5C4A14),
            find_cur: argb(0x9E6A03),
            find_frame: false,
            caret: fg,
            tokens: [
                fg,
                argb(0xFF7B72),
                argb(0xFFA657),
                argb(0xD2A8FF),
                argb(0xA5D6FF),
                argb(0x79C0FF),
                argb(0x8B949E),
                argb(0x79C0FF),
                argb(0xC9D1D9),
                argb(0x7EE787),
                argb(0x79C0FF),
                argb(0x79C0FF),
                fg,
                fg,
                argb(0x58A6FF),
                argb(0xA5D6FF),
                argb(0x7EE787),
            ],
            bold: 1 << SY_HEADING | 1 << SY_STRONG,
            italic: 1 << SY_COMMENT | 1 << SY_EMPH,
            diag: [argb(0xF85149), argb(0xD29922), argb(0x58A6FF)],
        }
    }

    /// the colours of a high-contrast scheme: few colours, keywords and headings bold;
    /// diagnostics in the text, link and highlight colours (the gutter markers' shapes tell
    /// them apart too)
    pub fn high_contrast(c: &plat::HcColors) -> CodeTheme {
        let (bg, fg, hi, link, dis) = (argb(c.bg), argb(c.text), argb(c.highlight), argb(c.link), argb(c.disabled));
        CodeTheme {
            bg,
            fg,
            gutter_bg: bg,
            gutter_fg: fg,
            gutter_cur: link,
            line_hl: bg,
            sel: hi,
            sel_inactive: dis,
            sel_text: argb(c.highlight_text),
            bracket: link,
            find: link,
            find_cur: hi,
            find_frame: true,
            caret: fg,
            tokens: [fg, link, link, fg, hi, hi, dis, hi, fg, link, link, link, fg, fg, link, hi, link],
            bold: 1 << SY_KEYWORD | 1 << SY_HEADING | 1 << SY_STRONG | 1 << SY_FUNC,
            italic: 1 << SY_COMMENT | 1 << SY_EMPH,
            diag: [fg, link, hi],
        }
    }
}

impl crate::Ui {
    /// the code editors' colours for the UI's theme: dark / light syntax colours on the UI's
    /// text box colours (accent selection, diagnostics in its critical / warning / info
    /// colours), or the high-contrast scheme's colours
    pub fn code_theme(&self) -> CodeTheme {
        use crate::consts::*;
        use crate::theme::color_mix;
        if self.high_contrast {
            let c = self.hc_sys.unwrap_or(if self.theme_dark { plat::HcColors::BLACK } else { plat::HcColors::WHITE });
            return CodeTheme::high_contrast(&c);
        }
        let mut t = if self.theme_dark { CodeTheme::dark() } else { CodeTheme::light() };
        let over = |bg: u32, c: u32| -> u32 {
            let a = (c >> 24) as i32;
            color_mix(bg | 0xFF00_0000, c | 0xFF00_0000, a + (a >> 7))
        };
        let bg = over(self.color(T_BG), self.color(T_INPUT_ACTIVE));
        let fg = over(bg, self.color(T_TEXT));
        t.bg = bg;
        t.fg = fg;
        t.tokens[SY_TEXT as usize] = fg;
        t.tokens[SY_EMPH as usize] = fg;
        t.tokens[SY_STRONG as usize] = fg;
        t.caret = fg;
        t.gutter_bg = color_mix(bg, fg, 10);
        t.gutter_fg = over(t.gutter_bg, self.color(T_TEXT3));
        t.gutter_cur = fg;
        t.line_hl = color_mix(bg, fg, if self.theme_dark { 16 } else { 10 });
        t.sel = color_mix(bg, self.color(T_ACCENT) | 0xFF00_0000, if self.theme_dark { 110 } else { 80 });
        t.sel_inactive = color_mix(bg, fg, if self.theme_dark { 40 } else { 30 });
        t.diag = [over(bg, self.color(T_CRIT)), over(bg, self.color(T_WARN)), over(bg, self.color(T_INFO))];
        t
    }
}
