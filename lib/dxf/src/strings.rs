// strings -- a DXF's bytes as text: UTF-8 from AutoCAD 2007 (AC1021) on, before that the header's code page
// ($DWGCODEPAGE: formats::encoding's character sets and multi-byte encodings), \U+XXXX and \M+NXXXX in any
// version; TEXT's %% codes (specials).

use alloc::string::String;
use alloc::vec::Vec;

use formats::encoding::{Charset, MultiByte};

/// how a file's strings are decoded
#[derive(Clone, Copy)]
pub(crate) struct Decoder {
    /// AC1021 and later: UTF-8
    pub utf8: bool,
    /// the code page (windows-1252 also for unknown pages)
    pub charset: Charset,
    /// a multi-byte page: Shift_JIS (ANSI_932, DOS932), GBK (ANSI_936, GB2312), EUC-KR (ANSI_949), Big5 (ANSI_950, BIG5)
    pub multi: Option<MultiByte>,
    /// a multi-byte page without a table here (Johab: ANSI_1361, JOHAB): bytes above 0x7F become U+FFFD
    pub unknown: bool,
}

impl Default for Decoder {
    fn default() -> Decoder {
        Decoder { utf8: false, charset: Charset::Windows1252, multi: None, unknown: false }
    }
}

/// the character set of a $DWGCODEPAGE name (upper case): ANSI_874 and ANSI_1250 to 1258, DOS437 to
/// DOS869 by their code page, MACINTOSH, a few ISO-8859 and KOI8-R; None for the multi-byte pages and
/// unknown names
fn charset(cp: &str) -> Option<Charset> {
    if let Some(n) = cp.strip_prefix("ANSI_").or_else(|| cp.strip_prefix("DOS")).and_then(|n| n.parse::<u16>().ok()) {
        return Charset::from_code_page(n);
    }
    Some(match cp {
        "ISO8859-1" | "ISO8859_1" => Charset::Windows1252,
        "ISO8859-2" | "ISO8859_2" => Charset::Iso8859_2,
        "ISO8859-5" | "ISO8859_5" => Charset::Iso8859_5,
        "ISO8859-7" | "ISO8859_7" => Charset::Iso8859_7,
        "ISO8859-9" | "ISO8859_9" => Charset::Windows1254,
        "MACINTOSH" | "MAC_ROMAN" => Charset::Macintosh,
        "KOI8-R" | "KOI8_R" => Charset::Koi8R,
        _ => return None,
    })
}

/// the multi-byte encoding of a $DWGCODEPAGE name (upper case)
fn multi(cp: &str) -> Option<MultiByte> {
    Some(match cp {
        "ANSI_932" | "DOS932" => MultiByte::ShiftJis,
        "ANSI_936" | "GB2312" => MultiByte::Gbk,
        "ANSI_949" => MultiByte::EucKr,
        "ANSI_950" | "BIG5" => MultiByte::Big5,
        _ => return None,
    })
}

impl Decoder {
    /// the decoder of a header's $ACADVER and $DWGCODEPAGE
    pub fn of(version: &str, codepage: &str) -> Decoder {
        let utf8 = version.len() == 6 && version.is_ascii() && version.starts_with("AC") && version[2..].parse::<u32>().is_ok_and(|v| v >= 1021);
        let cp: String = codepage.trim().chars().map(|c| c.to_ascii_uppercase()).collect();
        let multi = multi(&cp);
        let unknown = matches!(cp.as_str(), "ANSI_1361" | "JOHAB");
        let charset = if multi.is_some() || unknown { Charset::Windows1252 } else { charset(&cp).unwrap_or(Charset::Windows1252) };
        Decoder { utf8, charset, multi, unknown }
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
        if let Some(m) = self.multi {
            return m.decode(b);
        }
        if !self.unknown {
            return self.charset.decode(b);
        }
        for &c in b {
            out.push(if c < 0x80 { c as char } else { '\u{FFFD}' });
        }
        out
    }

    /// a single byte of the code page (%%nnn)
    pub fn byte(&self, c: u8) -> char {
        if c < 0x80 {
            return c as char;
        }
        if let (false, Some(m)) = (self.utf8, self.multi) {
            // (a single byte of a multi-byte page: Shift_JIS's half-width katakana)
            return m.decode(&[c]).chars().next().unwrap_or('\u{FFFD}');
        }
        let cs = if self.utf8 || self.unknown { Charset::Windows1252 } else { self.charset };
        cs.decode_char(c).unwrap_or('\u{FFFD}')
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

/// \U+XXXX (four or five hex digits) as the character it names; \M+NXXXX as the double byte XXXX of the East Asian
/// page N (`mbcs`).  Everything else stays (also "\\": TEXT has no escapes; MTEXT is parsed by mtext::parse)
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
                mbcs(&mut out, b[i + 3], &h[..4]);
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

/// \M+NXXXX: the double byte XXXX (four hex digits, lead byte first) of AutoCAD's East Asian page N: 1 Shift_JIS
/// (932), 2 Big5 (950), 3 EUC-KR (949), 5 GBK (936); 4 (Johab, 1361) and the others as U+FFFD
pub(crate) fn mbcs(out: &mut String, n: u8, hex: &[u8]) {
    let m = match n {
        b'1' => Some(MultiByte::ShiftJis),
        b'2' => Some(MultiByte::Big5),
        b'3' => Some(MultiByte::EucKr),
        b'5' => Some(MultiByte::Gbk),
        _ => None,
    };
    let v = hex.iter().take(4).fold(0u16, |v, c| v << 4 | (*c as char).to_digit(16).unwrap_or(0) as u16);
    match m {
        // (two bytes that are no character: one U+FFFD, not the trail byte as ASCII)
        Some(m) => {
            let s = m.decode(&v.to_be_bytes());
            out.push_str(if s.starts_with('\u{FFFD}') { "\u{FFFD}" } else { &s });
        }
        None => out.push('\u{FFFD}'),
    }
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
