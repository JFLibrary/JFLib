// mtext -- MTEXT's inline format codes (G6e2): a text as paragraphs of runs, each run with its format.
//
//   \P paragraph   \N column break   \X (dimension text: a line break)   \~ no-break space   \\ \{ \} the characters
//   { } a group: the format inside ends with it
//   \L \l underline on, off   \O \o overline   \K \k strike-through
//   \f<family>|b<0|1>|i<0|1>|c<charset>|p<pitch>;  a TrueType family    \F<file>;  a font file (SHX)
//   \H<v>; \H<v>x;  height (absolute, or times the current)    \W<v>; \W<v>x;  width factor
//   \Q<deg>;  oblique   \T<v>; \T<v>x;  tracking (spacing of characters)   \A<0|1|2>;  bottom, centre, top
//   \C<aci>;  colour of the palette (0 by block, 256 by layer)   \c<bgr>;  a true colour (blue in the high byte)
//   \S<top><sep><bottom>;  stacked: / with a bar, # diagonal, ^ without a bar (tolerances)
//   \p...;  paragraph: i<first line indent>, l<left indent>, r<right>, q<l|c|r|j|d> alignment, t<tabs> (x: ignored)
//   \U+XXXX  a character   \M+NXXXX  a double-byte character (read as U+FFFD)   ^I tab, ^J line break, ^<c> control
//   %%c %%d %%p %%% %%nnn  as in TEXT (strings::specials)
//
// Unknown codes are dropped with their letter (a code with a ';' to its end); nothing recurses (the groups are a stack
// of at most GROUP_DEPTH formats; deeper ones share the deepest).

use alloc::string::String;
use alloc::vec::Vec;

use crate::Color;

/// the deepest nesting of { } groups that keeps formats of its own
pub const GROUP_DEPTH: usize = 64;

/// how a run looks
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct Format {
    /// \f's family or \F's file (None: the entity's style), \f's bold and italic flags
    pub font: Option<String>,
    pub shx: bool,
    pub bold: bool,
    pub italic: bool,
    /// the cap height (drawing units)
    pub height: f64,
    /// width factor, oblique angle (degrees; None: the style's), tracking factor
    pub width: Option<f64>,
    pub oblique: Option<f64>,
    pub tracking: f64,
    /// None: the entity's colour
    pub color: Option<Color>,
    pub underline: bool,
    pub overline: bool,
    pub strike: bool,
    /// \A: 0 bottom, 1 centre, 2 top (where a smaller run sits on the line)
    pub valign: u8,
}

impl Format {
    fn new(height: f64) -> Format {
        Format { font: None, shx: false, bold: false, italic: false, height, width: None, oblique: None, tracking: 1.0, color: None, underline: false, overline: false, strike: false, valign: 0 }
    }
}

/// a stacked text's separator
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StackKind {
    /// '/': above and below a horizontal bar
    Bar,
    /// '#': a diagonal fraction
    Diagonal,
    /// '^': above and below, no bar (tolerances)
    Tolerance,
}

/// a piece of a paragraph
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Text(String, Format),
    Stack { top: String, bottom: String, kind: StackKind, format: Format },
    Tab(Format),
}

/// a paragraph's alignment (\pq; Default: the attachment point's)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Default,
    Left,
    Center,
    Right,
    Justify,
    Distribute,
}

/// a paragraph: its alignment, indents (drawing units), tab stops, items; whether a column break came before it
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Paragraph {
    pub align: Align,
    pub indent_first: f64,
    pub indent_left: f64,
    pub indent_right: f64,
    pub tabs: Vec<f64>,
    pub column_break: bool,
    pub items: Vec<Item>,
}

/// an MTEXT's content
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Mtext {
    pub paragraphs: Vec<Paragraph>,
}

impl Mtext {
    /// the text, paragraphs joined by '\n', when it has nothing but text in the entity's own format, left aligned (what
    /// render2d's MText draws as it is); None otherwise
    pub fn plain(&self, height: f64) -> Option<String> {
        let base = Format::new(height);
        let mut s = String::new();
        for (k, p) in self.paragraphs.iter().enumerate() {
            if p.align != Align::Default || p.indent_first != 0.0 || p.indent_left != 0.0 || p.column_break {
                return None;
            }
            if k > 0 {
                s.push('\n');
            }
            for it in &p.items {
                match it {
                    Item::Text(t, f) if *f == base => s.push_str(t),
                    _ => return None,
                }
            }
        }
        Some(s)
    }

    /// all text without formats (for counts and tests): paragraphs by '\n', stacks as top/bottom, tabs as '\t'
    pub fn text(&self) -> String {
        let mut s = String::new();
        for (k, p) in self.paragraphs.iter().enumerate() {
            if k > 0 {
                s.push('\n');
            }
            for it in &p.items {
                match it {
                    Item::Text(t, _) => s.push_str(t),
                    Item::Stack { top, bottom, .. } => {
                        s.push_str(top);
                        s.push('/');
                        s.push_str(bottom);
                    }
                    Item::Tab(_) => s.push('\t'),
                }
            }
        }
        s
    }
}

struct P<'a> {
    s: &'a str,
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    /// the argument of a code up to ';' (or the end), and past the ';'
    fn arg(&mut self) -> &'a str {
        let start = self.i;
        while self.i < self.b.len() && self.b[self.i] != b';' {
            self.i += 1;
        }
        let a = &self.s[start..self.i];
        if self.i < self.b.len() {
            self.i += 1;
        }
        a
    }
}

/// a number at the start of an argument (an optional 'x' after it: relative)
fn number(a: &str) -> Option<(f64, bool)> {
    let a = a.trim();
    let (v, rel) = match a.strip_suffix(['x', 'X']) {
        Some(v) => (v, true),
        None => (a, false),
    };
    let v: f64 = v.trim().parse().ok()?;
    v.is_finite().then_some((v, rel))
}

/// MTEXT's text (3 and 1 joined) as paragraphs of runs; `height` is the entity's cap height
pub fn parse(text: &str, height: f64) -> Mtext {
    let mut out = Mtext::default();
    let mut para = Paragraph::default();
    let mut fmt = Format::new(height);
    let mut groups: Vec<Format> = Vec::new();
    let mut deep = 0usize;
    let mut cur = String::new();
    let mut p = P { s: text, b: text.as_bytes(), i: 0 };
    // the current run ends (before a format changes, a tab, a stack, a paragraph)
    fn flush(cur: &mut String, fmt: &Format, para: &mut Paragraph) {
        if !cur.is_empty() {
            para.items.push(Item::Text(core::mem::take(cur), fmt.clone()));
        }
    }
    macro_rules! change {
        ($e:expr) => {{
            flush(&mut cur, &fmt, &mut para);
            $e;
        }};
    }
    let dec = crate::strings::Decoder::default();
    while p.i < p.b.len() {
        let c = p.b[p.i];
        match c {
            b'\\' if p.i + 1 < p.b.len() => {
                let code = p.b[p.i + 1];
                p.i += 2;
                match code {
                    b'P' | b'X' => {
                        flush(&mut cur, &fmt, &mut para);
                        out.paragraphs.push(core::mem::take(&mut para));
                        // a paragraph's settings (\p) carry on to the next
                        let last = out.paragraphs.last().map(|l| (l.align, l.indent_first, l.indent_left, l.indent_right, l.tabs.clone()));
                        if let Some((a, f, l, r, t)) = last {
                            para.align = a;
                            para.indent_first = f;
                            para.indent_left = l;
                            para.indent_right = r;
                            para.tabs = t;
                        }
                    }
                    b'N' => {
                        flush(&mut cur, &fmt, &mut para);
                        out.paragraphs.push(core::mem::take(&mut para));
                        para.column_break = true;
                    }
                    b'~' => cur.push('\u{A0}'),
                    b'\\' => cur.push('\\'),
                    b'{' => cur.push('{'),
                    b'}' => cur.push('}'),
                    b'L' => change!(fmt.underline = true),
                    b'l' => change!(fmt.underline = false),
                    b'O' => change!(fmt.overline = true),
                    b'o' => change!(fmt.overline = false),
                    b'K' => change!(fmt.strike = true),
                    b'k' => change!(fmt.strike = false),
                    b'f' | b'F' => {
                        let a = p.arg();
                        let mut parts = a.split('|');
                        let name = parts.next().unwrap_or("").trim();
                        let (mut bold, mut italic) = (false, false);
                        for q in parts {
                            let q = q.trim();
                            if let Some(v) = q.strip_prefix('b') {
                                bold = v.trim() == "1";
                            } else if let Some(v) = q.strip_prefix('i') {
                                italic = v.trim() == "1";
                            }
                        }
                        change!({
                            fmt.font = if name.is_empty() { None } else { Some(String::from(name)) };
                            fmt.shx = code == b'F';
                            fmt.bold = bold;
                            fmt.italic = italic;
                        });
                    }
                    b'H' => {
                        if let Some((v, rel)) = number(p.arg()) {
                            let h = if rel { fmt.height * v } else { v };
                            if h > 0.0 && h.is_finite() {
                                change!(fmt.height = h);
                            }
                        }
                    }
                    b'W' => {
                        if let Some((v, rel)) = number(p.arg()) {
                            let w = if rel { fmt.width.unwrap_or(1.0) * v } else { v };
                            if w > 0.0 && w.is_finite() {
                                change!(fmt.width = Some(w));
                            }
                        }
                    }
                    b'Q' => {
                        if let Some((v, _)) = number(p.arg()) {
                            change!(fmt.oblique = Some(v));
                        }
                    }
                    b'T' => {
                        if let Some((v, rel)) = number(p.arg()) {
                            let t = if rel { fmt.tracking * v } else { v };
                            if t > 0.0 && t.is_finite() {
                                change!(fmt.tracking = t);
                            }
                        }
                    }
                    b'A' => {
                        if let Some((v, _)) = number(p.arg()) {
                            change!(fmt.valign = (v as i64).clamp(0, 2) as u8);
                        }
                    }
                    b'C' => {
                        if let Some((v, _)) = number(p.arg()) {
                            let v = v as i64;
                            let col = match v {
                                0 => Color::ByBlock,
                                1..=255 => Color::Aci(v as u8),
                                _ => Color::ByLayer,
                            };
                            change!(fmt.color = if col == Color::ByLayer { None } else { Some(col) });
                        }
                    }
                    b'c' => {
                        if let Some((v, _)) = number(p.arg()) {
                            let v = (v as i64).clamp(0, 0xFF_FFFF) as u32;
                            // blue, green, red from the high byte down
                            let rgb = ((v & 0xFF) << 16) | (v & 0xFF00) | ((v >> 16) & 0xFF);
                            change!(fmt.color = Some(Color::Rgb(rgb)));
                        }
                    }
                    b'S' => {
                        // up to an unescaped ';'; the first unescaped '/', '#' or '^' separates
                        let mut top = String::new();
                        let mut bottom = String::new();
                        let mut kind = None;
                        while p.i < p.b.len() && p.b[p.i] != b';' {
                            let ch = p.s[p.i..].chars().next().unwrap_or(' ');
                            let n = ch.len_utf8();
                            if ch == '\\' && p.i + 1 < p.b.len() {
                                let e = p.s[p.i + 1..].chars().next().unwrap_or(' ');
                                let dst = if kind.is_some() { &mut bottom } else { &mut top };
                                dst.push(e);
                                p.i += 1 + e.len_utf8();
                                continue;
                            }
                            if kind.is_none() && (ch == '/' || ch == '#' || ch == '^') {
                                kind = Some(match ch {
                                    '/' => StackKind::Bar,
                                    '#' => StackKind::Diagonal,
                                    _ => StackKind::Tolerance,
                                });
                            } else if kind.is_some() {
                                bottom.push(ch);
                            } else {
                                top.push(ch);
                            }
                            p.i += n;
                        }
                        if p.i < p.b.len() {
                            p.i += 1;
                        }
                        flush(&mut cur, &fmt, &mut para);
                        match kind {
                            Some(k) => para.items.push(Item::Stack { top: caret(&top), bottom: caret(&bottom), kind: k, format: fmt.clone() }),
                            None => cur.push_str(&top),
                        }
                    }
                    b'p' => {
                        let a = p.arg();
                        for q in a.split(',') {
                            let q = q.trim();
                            let q = q.strip_prefix('x').unwrap_or(q);
                            let mut chars = q.chars();
                            let Some(k) = chars.next() else { continue };
                            let rest = chars.as_str();
                            let val = || rest.trim().parse::<f64>().ok().filter(|v| v.is_finite());
                            match k {
                                'i' => para.indent_first = val().unwrap_or(0.0),
                                'l' => para.indent_left = val().unwrap_or(0.0),
                                'r' => para.indent_right = val().unwrap_or(0.0),
                                'q' => {
                                    para.align = match rest.trim() {
                                        "l" => Align::Left,
                                        "c" => Align::Center,
                                        "r" => Align::Right,
                                        "j" => Align::Justify,
                                        "d" => Align::Distribute,
                                        _ => Align::Default,
                                    }
                                }
                                't' => {
                                    para.tabs.clear();
                                    for t in rest.split(['t', ' ']) {
                                        if let Ok(v) = t.trim().trim_start_matches(['c', 'r', 'd']).parse::<f64>() {
                                            if v.is_finite() && para.tabs.len() < 64 {
                                                para.tabs.push(v);
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    b'U' | b'u' if p.i < p.b.len() && p.b[p.i] == b'+' => match crate::strings::hex_char(&p.b[p.i + 1..]) {
                        Some((ch, n)) => {
                            cur.push(ch);
                            p.i += 1 + n;
                        }
                        None => cur.push_str("\\U"),
                    },
                    b'M' | b'm' if p.i + 1 < p.b.len() && p.b[p.i] == b'+' => {
                        let h = &p.b[p.i + 2..];
                        if h.len() >= 4 && h[..4].iter().all(|c| c.is_ascii_hexdigit()) {
                            cur.push('\u{FFFD}');
                            p.i += 6;
                        }
                    }
                    _ => {
                        // an unknown code: its letter goes (a non-ASCII character after the backslash stays)
                        if code >= 0x80 {
                            p.i -= 1;
                            while p.i < p.b.len() && !p.s.is_char_boundary(p.i) {
                                p.i -= 1;
                            }
                        }
                    }
                }
            }
            b'{' => {
                p.i += 1;
                if groups.len() < GROUP_DEPTH {
                    groups.push(fmt.clone());
                } else {
                    deep += 1;
                }
            }
            b'}' => {
                p.i += 1;
                if deep > 0 {
                    deep -= 1;
                } else if let Some(f) = groups.pop() {
                    if f != fmt {
                        flush(&mut cur, &fmt, &mut para);
                        fmt = f;
                    }
                }
            }
            b'^' if p.i + 1 < p.b.len() => {
                let n = p.b[p.i + 1];
                p.i += 2;
                match n {
                    b'I' => {
                        flush(&mut cur, &fmt, &mut para);
                        para.items.push(Item::Tab(fmt.clone()));
                    }
                    b'J' => {
                        flush(&mut cur, &fmt, &mut para);
                        out.paragraphs.push(core::mem::take(&mut para));
                    }
                    b' ' => cur.push('^'),
                    b'M' => {}
                    _ => {
                        if n >= 0x80 {
                            // a non-ASCII character after the caret stays
                            p.i -= 1;
                            while !p.s.is_char_boundary(p.i) {
                                p.i -= 1;
                            }
                        }
                    }
                }
            }
            b'%' if p.i + 2 < p.b.len() && p.b[p.i + 1] == b'%' => {
                let k = p.b[p.i + 2];
                match k.to_ascii_lowercase() {
                    b'c' => cur.push('\u{2205}'),
                    b'd' => cur.push('\u{B0}'),
                    b'p' => cur.push('\u{B1}'),
                    b'%' => cur.push('%'),
                    b'u' => change!(fmt.underline = !fmt.underline),
                    b'o' => change!(fmt.overline = !fmt.overline),
                    b'k' => change!(fmt.strike = !fmt.strike),
                    d if d.is_ascii_digit() => {
                        let n = p.b[p.i + 2..].iter().take(3).take_while(|c| c.is_ascii_digit()).count();
                        let v: u32 = p.s[p.i + 2..p.i + 2 + n].parse().unwrap_or(0);
                        cur.push(if v <= 255 { dec.byte(v as u8) } else { '\u{FFFD}' });
                        p.i += 2 + n;
                        continue;
                    }
                    _ => {
                        cur.push_str("%%");
                        p.i += 2;
                        continue;
                    }
                }
                p.i += 3;
            }
            b'\n' => {
                p.i += 1;
                flush(&mut cur, &fmt, &mut para);
                out.paragraphs.push(core::mem::take(&mut para));
            }
            b'\t' => {
                p.i += 1;
                flush(&mut cur, &fmt, &mut para);
                para.items.push(Item::Tab(fmt.clone()));
            }
            b'\r' => p.i += 1,
            _ => {
                let ch = p.s[p.i..].chars().next().unwrap_or(' ');
                if !ch.is_control() {
                    cur.push(ch);
                }
                p.i += ch.len_utf8();
            }
        }
    }
    flush(&mut cur, &fmt, &mut para);
    out.paragraphs.push(para);
    out
}

/// a stack's part: ^I etc. as in the text ("^ " a caret)
fn caret(s: &str) -> String {
    if !s.contains('^') {
        return String::from(s);
    }
    s.replace("^ ", "^")
}
