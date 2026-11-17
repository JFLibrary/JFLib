// strings -- a DXF's bytes as text: UTF-8 from AutoCAD 2007 (AC1021) on, before that the header's code page
// ($DWGCODEPAGE; codepages.rs), \U+XXXX in any version; TEXT's %% codes (specials).

use alloc::string::String;
use alloc::vec::Vec;

use crate::codepages;

/// how a file's strings are decoded
#[derive(Clone, Copy)]
pub(crate) struct Decoder {
    /// AC1021 and later: UTF-8
    pub utf8: bool,
    /// the code page's table (None: Windows-1252, also for unknown pages)
    pub table: Option<&'static [u16; 128]>,
    /// a multi-byte page this reader has no table of (bytes above 0x7F become U+FFFD)
    pub multibyte: bool,
}

impl Default for Decoder {
    fn default() -> Decoder {
        Decoder { utf8: false, table: codepages::table("ANSI_1252"), multibyte: false }
    }
}

impl Decoder {
    /// the decoder of a header's $ACADVER and $DWGCODEPAGE
    pub fn of(version: &str, codepage: &str) -> Decoder {
        let utf8 = version.len() == 6 && version.is_ascii() && version.starts_with("AC") && version[2..].parse::<u32>().is_ok_and(|v| v >= 1021);
        let cp: String = codepage.trim().chars().map(|c| c.to_ascii_uppercase()).collect();
        let multibyte = matches!(cp.as_str(), "ANSI_932" | "ANSI_936" | "ANSI_949" | "ANSI_950" | "ANSI_1361" | "DOS932" | "BIG5" | "GB2312" | "JOHAB");
        let table = codepages::table(&cp).or_else(|| if multibyte { None } else { codepages::table("ANSI_1252") });
        Decoder { utf8, table, multibyte }
    }

    /// bytes as text: valid UTF-8 stays UTF-8 in any version (writers put UTF-8 into older files: GDAL's utf-8.dxf), the
    /// rest through the code page; invalid UTF-8 of a UTF-8 file becomes U+FFFD
    pub fn text(&self, b: &[u8]) -> String {
        if b.is_ascii() {
            // (ASCII is ASCII in every page)
            return String::from(core::str::from_utf8(b).unwrap_or(""));
        }
        if let Ok(s) = core::str::from_utf8(b) {
            return String::from(s);
        }
        let mut out = String::with_capacity(b.len() + 8);
        if self.utf8 {
            lossy(b, &mut out);
            return out;
        }
        for &c in b {
            if c < 0x80 {
                out.push(c as char);
            } else if self.multibyte {
                out.push('\u{FFFD}');
            } else {
                let u = self.table.map_or(0xFFFD, |t| t[(c - 0x80) as usize]);
                out.push(char::from_u32(u as u32).unwrap_or('\u{FFFD}'));
            }
        }
        out
    }

    /// a single byte of the code page (%%nnn)
    pub fn byte(&self, c: u8) -> char {
        if c < 0x80 {
            return c as char;
        }
        let t = if self.utf8 || self.multibyte { codepages::table("ANSI_1252") } else { self.table };
        t.and_then(|t| char::from_u32(t[(c - 0x80) as usize] as u32)).unwrap_or('\u{FFFD}')
    }
}

/// invalid UTF-8 with U+FFFD for each bad sequence
fn lossy(mut b: &[u8], out: &mut String) {
    loop {
        match core::str::from_utf8(b) {
            Ok(s) => {
                out.push_str(s);
                return;
            }
            Err(e) => {
                let (good, rest) = b.split_at(e.valid_up_to());
                out.push_str(core::str::from_utf8(good).unwrap_or(""));
                out.push('\u{FFFD}');
                let skip = e.error_len().unwrap_or(rest.len()).max(1);
                b = &rest[skip.min(rest.len())..];
                if b.is_empty() {
                    return;
                }
            }
        }
    }
}

/// \U+XXXX (four or five hex digits) as the character it names; \M+NXXXX (a double byte of an East Asian page) as
/// U+FFFD.  Everything else stays (also "\\": TEXT has no escapes; MTEXT is parsed by mtext::parse)
pub fn decode_unicode_escapes(s: &str) -> String {
    if !s.contains('\\') {
        return String::from(s);
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut last = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 2 < b.len() && (b[i + 1] == b'U' || b[i + 1] == b'u') && b[i + 2] == b'+' {
            if let Some((c, n)) = hex_char(&b[i + 3..]) {
                out.push_str(&s[last..i]);
                out.push(c);
                i += 3 + n;
                last = i;
                continue;
            }
        }
        if b[i] == b'\\' && i + 3 < b.len() && (b[i + 1] == b'M' || b[i + 1] == b'm') && b[i + 2] == b'+' && b[i + 3].is_ascii_digit() {
            let h = &b[i + 4..];
            if h.len() >= 4 && h[..4].iter().all(|c| c.is_ascii_hexdigit()) {
                out.push_str(&s[last..i]);
                out.push('\u{FFFD}');
                i += 8;
                last = i;
                continue;
            }
        }
        i += 1;
    }
    out.push_str(&s[last..]);
    out
}

/// the character of four (or five) hex digits at the start, and how many digits it took
pub(crate) fn hex_char(h: &[u8]) -> Option<(char, usize)> {
    let digits = h.iter().take(5).take_while(|c| c.is_ascii_hexdigit()).count();
    if digits < 4 {
        return None;
    }
    let mut v = 0u32;
    for &c in &h[..4] {
        v = v * 16 + (c as char).to_digit(16).unwrap_or(0);
    }
    Some((char::from_u32(v).unwrap_or('\u{FFFD}'), 4))
}

/// a run of a TEXT after its %% codes: the text and how it is decorated
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Special {
    pub text: String,
    pub underline: bool,
    pub overline: bool,
    pub strike: bool,
}

/// TEXT's %% codes: %%c the diameter sign (U+2205), %%d the degree sign, %%p plus-minus, %%nnn the character of that
/// code (Windows-1252 above 127), %%% a percent sign; %%u, %%o and %%k switch underline, overline and strike-through on
/// and off (the runs they start).  Unknown codes stay as they are
pub fn specials(s: &str) -> Vec<Special> {
    let mut runs: Vec<Special> = Vec::new();
    let mut cur = Special::default();
    if !s.contains("%%") {
        cur.text = String::from(s);
        runs.push(cur);
        return runs;
    }
    let dec = Decoder::default();
    let b = s.as_bytes();
    let mut i = 0;
    let mut last = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() && b[i + 1] == b'%' {
            let c = b[i + 2];
            let put = |cur: &mut Special, s2: &str, i: usize, last: &mut usize, n: usize| {
                cur.text.push_str(&s[*last..i]);
                cur.text.push_str(s2);
                *last = i + n;
            };
            match c.to_ascii_lowercase() {
                b'c' => put(&mut cur, "\u{2205}", i, &mut last, 3),
                b'd' => put(&mut cur, "\u{B0}", i, &mut last, 3),
                b'p' => put(&mut cur, "\u{B1}", i, &mut last, 3),
                b'%' => put(&mut cur, "%", i, &mut last, 3),
                b'u' | b'o' | b'k' => {
                    cur.text.push_str(&s[last..i]);
                    let mut next = cur.clone();
                    next.text = String::new();
                    match c.to_ascii_lowercase() {
                        b'u' => next.underline = !cur.underline,
                        b'o' => next.overline = !cur.overline,
                        _ => next.strike = !cur.strike,
                    }
                    if !cur.text.is_empty() {
                        runs.push(cur);
                    }
                    cur = next;
                    last = i + 3;
                }
                d if d.is_ascii_digit() => {
                    let n = b[i + 2..].iter().take(3).take_while(|c| c.is_ascii_digit()).count();
                    let v: u32 = s[i + 2..i + 2 + n].parse().unwrap_or(0);
                    let ch = if v <= 255 { dec.byte(v as u8) } else { '\u{FFFD}' };
                    let mut tmp = [0u8; 4];
                    put(&mut cur, ch.encode_utf8(&mut tmp), i, &mut last, 2 + n);
                    i += 2 + n;
                    continue;
                }
                _ => {
                    i += 2;
                    continue;
                }
            }
            i = last;
            continue;
        }
        i += 1;
    }
    cur.text.push_str(&s[last..]);
    if !cur.text.is_empty() || runs.is_empty() {
        runs.push(cur);
    }
    runs
}
