// multibyte -- the multi-byte encodings of the WHATWG Encoding Standard for formats::encoding: Shift_JIS,
// EUC-JP, ISO-2022-JP, GBK, gb18030, Big5 and EUC-KR.  The encoders here, the decoders in decode.rs, both as
// the standard writes them step by step; the indexes are generated (cjk.rs, cjk.bin).
//
// A pointer is a character's place in an index (row × columns + column); the decoders compute it from the
// bytes and look the code point up, the encoders search the pointers sorted by code point (*_ORDER) and
// compute the bytes from it.
use alloc::{string::String, vec::Vec};

use crate::cjk::{self, Table};
use crate::decode::Decoder;
use crate::encoding::{Error, Label};

/// A multi-byte encoding of the WHATWG Encoding Standard: the legacy encodings of Chinese, Japanese and
/// Korean text that browsers, mail and old files still use.  Each is the standard's: the Windows variants
/// where the web uses them (Shift_JIS is Windows' code page 932, EUC-KR its 949, GBK its 936).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum MultiByte {
    /// Shift_JIS: Japanese as Windows writes it (code page 932, windows-31j: JIS X 0208 with the NEC and
    /// IBM extensions, half-width katakana, the user-defined rows as the Private Use Area)
    ShiftJis,
    /// EUC-JP: Japanese on Unix (JIS X 0208, JIS X 0212 after 0x8F, half-width katakana after 0x8E)
    EucJp,
    /// ISO-2022-JP: Japanese in mail and news, 7 bits with escape sequences (ASCII, JIS X 0201 Roman and
    /// katakana, JIS X 0208)
    Iso2022Jp,
    /// GBK: simplified Chinese (code page 936; also what the labels gb2312 and chinese name).  It reads
    /// gb18030; it writes only its two-byte characters and the euro sign as 0x80.
    Gbk,
    /// gb18030: simplified Chinese with all of Unicode (GB18030-2022; code page 54936)
    Gb18030,
    /// Big5: traditional Chinese with the Hong Kong Supplementary Character Set (HKSCS-2008; code page 950)
    Big5,
    /// EUC-KR: Korean as Windows writes it (code page 949, the Unified Hangul Code: all 11 172 syllables)
    EucKr,
}

impl MultiByte {
    /// every multi-byte encoding, in declaration order
    pub const ALL: &'static [MultiByte] =
        &[MultiByte::ShiftJis, MultiByte::EucJp, MultiByte::Iso2022Jp, MultiByte::Gbk, MultiByte::Gb18030, MultiByte::Big5, MultiByte::EucKr];

    /// its name in the WHATWG Encoding Standard ("Shift_JIS", "EUC-JP", "ISO-2022-JP", "GBK", "gb18030",
    /// "Big5", "EUC-KR")
    pub const fn name(self) -> &'static str {
        match self {
            MultiByte::ShiftJis => "Shift_JIS",
            MultiByte::EucJp => "EUC-JP",
            MultiByte::Iso2022Jp => "ISO-2022-JP",
            MultiByte::Gbk => "GBK",
            MultiByte::Gb18030 => "gb18030",
            MultiByte::Big5 => "Big5",
            MultiByte::EucKr => "EUC-KR",
        }
    }

    /// its Windows code page number: 932, 20932, 50220, 936, 54936, 950, 949
    pub const fn code_page(self) -> u16 {
        match self {
            MultiByte::ShiftJis => 932,
            MultiByte::EucJp => 20932,
            MultiByte::Iso2022Jp => 50220,
            MultiByte::Gbk => 936,
            MultiByte::Gb18030 => 54936,
            MultiByte::Big5 => 950,
            MultiByte::EucKr => 949,
        }
    }

    /// the encoding of a Windows code page number: 932, 936, 949, 950, 20932 and 51932 (EUC-JP), 50220 to
    /// 50222 (ISO-2022-JP), 51949 (EUC-KR), 54936.  None for the others (Johab, 1361, is none of the standard's).
    pub const fn from_code_page(cp: u16) -> Option<MultiByte> {
        Some(match cp {
            932 => MultiByte::ShiftJis,
            20932 | 51932 => MultiByte::EucJp,
            50220..=50222 => MultiByte::Iso2022Jp,
            936 => MultiByte::Gbk,
            54936 => MultiByte::Gb18030,
            950 => MultiByte::Big5,
            949 | 51949 => MultiByte::EucKr,
            _ => return None,
        })
    }

    /// the encoding a WHATWG label names ("shift_jis", "sjis", "windows-31j", "gb2312", "big5-hkscs",
    /// "ks_c_5601-1987" ...; `encoding::label`), multi-byte encodings only
    pub fn from_label(name: &str) -> Option<MultiByte> {
        match crate::encoding::label(name)? {
            Label::Multi(m) => Some(m),
            _ => None,
        }
    }

    /// The text of these bytes, as the standard decodes them: malformed or unassigned sequences become
    /// U+FFFD, ASCII bytes after a broken lead byte are kept.  For input in pieces: `Decoder`.
    pub fn decode(self, bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() + bytes.len() / 2);
        Decoder::new(Label::Multi(self)).decode(bytes, true, &mut out);
        out
    }

    /// The bytes of a text; Unmappable: the first character this encoding does not have (ISO-2022-JP ends
    /// in ASCII, with ESC ( B where needed).
    pub fn encode(self, text: &str) -> Result<Vec<u8>, Error> {
        self.encode_with(text, |c, _, _| Err(Error::Unmappable(c)))
    }

    /// The bytes of a text, a character this encoding does not have as a decimal character reference
    /// ("&#128512;"), as browsers submit forms (the standard's "html" error mode).
    pub fn encode_html(self, text: &str) -> Vec<u8> {
        self.encode_with(text, |_, reported, out| {
            out.extend_from_slice(b"&#");
            let mut n = reported as u32;
            let mut digits = [0u8; 7];
            let mut k = 0;
            loop {
                digits[k] = b'0' + (n % 10) as u8;
                k += 1;
                n /= 10;
                if n == 0 {
                    break;
                }
            }
            out.extend(digits[..k].iter().rev());
            out.push(b';');
            Ok(())
        })
        .unwrap_or_default()
    }

    // the encoder: each character through the standard's algorithm; `fallback` gets a character without bytes,
    // the code point the standard reports for it (U+FFFD for ISO-2022-JP's SO, SI and ESC) and the output
    fn encode_with(self, text: &str, mut fallback: impl FnMut(char, char, &mut Vec<u8>) -> Result<(), Error>) -> Result<Vec<u8>, Error> {
        let mut out = Vec::with_capacity(text.len() + text.len() / 4);
        let mut jis = Jis::Ascii;
        for c in text.chars() {
            if c.is_ascii() && (self != MultiByte::Iso2022Jp || jis == Jis::Ascii && !matches!(c, '\x0E' | '\x0F' | '\x1B')) {
                out.push(c as u8);
                continue;
            }
            let r = match self {
                MultiByte::ShiftJis => shift_jis(c, &mut out),
                MultiByte::EucJp => euc_jp(c, &mut out),
                MultiByte::Iso2022Jp => iso2022jp(c, &mut jis, &mut out),
                MultiByte::Gbk => gb18030(c, true, &mut out),
                MultiByte::Gb18030 => gb18030(c, false, &mut out),
                MultiByte::Big5 => big5(c, &mut out),
                MultiByte::EucKr => euc_kr(c, &mut out),
            };
            if let Err(reported) = r {
                fallback(c, reported, &mut out)?;
            }
        }
        if jis != Jis::Ascii {
            out.extend_from_slice(b"\x1B(B");
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------- the indexes

/// the code point of a pointer in big5 (0: none)
pub(crate) fn big5_code_point(p: usize) -> u32 {
    let low = cjk::BIG5.get(p) as u32;
    if low != 0 && cjk::BIG5_ASTRAL.get(p / 16) >> (p % 16) & 1 != 0 { low + 0x20000 } else { low }
}

/// index gb18030 ranges code point: the code point of a four-byte pointer (None: no character)
pub(crate) fn gb18030_range_code_point(p: u32) -> Option<u32> {
    if p > 39419 && p < 189000 || p > 1237575 {
        return None;
    }
    if p == 7457 {
        return Some(0xE7C7);
    }
    let r = cjk::GB18030_RANGES;
    let i = r.partition_point(|e| e.0 <= p) - 1;
    Some(r[i].1 + p - r[i].0)
}

// index gb18030 ranges pointer
fn gb18030_range_pointer(c: u32) -> u32 {
    if c == 0xE7C7 {
        return 7457;
    }
    let r = cjk::GB18030_RANGES;
    let i = r.partition_point(|e| e.1 <= c) - 1;
    r[i].0 + c - r[i].1
}

// the pointers of a code point in an index, lowest first: the place of the first in `order`
fn first_of(order: Table, value: impl Fn(usize) -> u32, c: u32) -> usize {
    let (mut lo, mut hi) = (0, order.len);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if value(order.get(mid) as usize) < c {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

// the first pointer of a code point that `keep` accepts
fn pointer(order: Table, value: impl Fn(usize) -> u32, c: u32, keep: impl Fn(usize) -> bool) -> Option<usize> {
    let mut i = first_of(order, &value, c);
    while i < order.len {
        let p = order.get(i) as usize;
        if value(p) != c {
            return None;
        }
        if keep(p) {
            return Some(p);
        }
        i += 1;
    }
    None
}

fn put(out: &mut Vec<u8>, b: u8) -> Result<(), char> {
    out.push(b);
    Ok(())
}

fn jis0208(p: usize) -> u32 {
    cjk::JIS0208.get(p) as u32
}

// ---------------------------------------------------------------- the encoders (Err: the code point reported)

fn shift_jis(c: char, out: &mut Vec<u8>) -> Result<(), char> {
    let mut u = c as u32;
    match u {
        0x80 => return put(out, 0x80),
        0xA5 => return put(out, 0x5C),
        0x203E => return put(out, 0x7E),
        0xFF61..=0xFF9F => return put(out, (u - 0xFF61 + 0xA1) as u8),
        0x2212 => u = 0xFF0D,
        _ => {}
    }
    // index Shift_JIS pointer: not the NEC-selected IBM extensions (the IBM extensions have them again)
    let p = pointer(cjk::JIS0208_ORDER, jis0208, u, |p| !(8272..=8835).contains(&p)).ok_or(c)?;
    let (lead, trail) = (p / 188, p % 188);
    out.push((lead + if lead < 0x1F { 0x81 } else { 0xC1 }) as u8);
    out.push((trail + if trail < 0x3F { 0x40 } else { 0x41 }) as u8);
    Ok(())
}

fn euc_jp(c: char, out: &mut Vec<u8>) -> Result<(), char> {
    let mut u = c as u32;
    match u {
        0xA5 => return put(out, 0x5C),
        0x203E => return put(out, 0x7E),
        0xFF61..=0xFF9F => {
            out.push(0x8E);
            return put(out, (u - 0xFF61 + 0xA1) as u8);
        }
        0x2212 => u = 0xFF0D,
        _ => {}
    }
    let p = pointer(cjk::JIS0208_ORDER, jis0208, u, |_| true).ok_or(c)?;
    out.push((p / 94 + 0xA1) as u8);
    out.push((p % 94 + 0xA1) as u8);
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Jis {
    Ascii,
    Roman,
    Jis0208,
}

fn iso2022jp(c: char, state: &mut Jis, out: &mut Vec<u8>) -> Result<(), char> {
    let u = c as u32;
    // each turn either writes the character or switches the state and tries again (the standard's "restore")
    loop {
        if *state != Jis::Jis0208 && matches!(u, 0x0E | 0x0F | 0x1B) {
            return Err('\u{FFFD}');
        }
        if *state == Jis::Ascii && u < 0x80 {
            return put(out, u as u8);
        }
        if *state == Jis::Roman && (u < 0x80 && u != 0x5C && u != 0x7E || u == 0xA5 || u == 0x203E) {
            return put(out, match u {
                0xA5 => 0x5C,
                0x203E => 0x7E,
                _ => u as u8,
            });
        }
        if u < 0x80 {
            out.extend_from_slice(b"\x1B(B");
            *state = Jis::Ascii;
            continue;
        }
        if (u == 0xA5 || u == 0x203E) && *state != Jis::Roman {
            out.extend_from_slice(b"\x1B(J");
            *state = Jis::Roman;
            continue;
        }
        let v = match u {
            0x2212 => 0xFF0D,
            0xFF61..=0xFF9F => cjk::ISO2022JP_KATAKANA[(u - 0xFF61) as usize] as u32,
            _ => u,
        };
        let Some(p) = pointer(cjk::JIS0208_ORDER, jis0208, v, |_| true) else {
            if *state == Jis::Jis0208 {
                out.extend_from_slice(b"\x1B(B");
                *state = Jis::Ascii;
                continue;
            }
            return Err(c);
        };
        if *state != Jis::Jis0208 {
            out.extend_from_slice(b"\x1B$B");
            *state = Jis::Jis0208;
            continue;
        }
        out.push((p / 94 + 0x21) as u8);
        out.push((p % 94 + 0x21) as u8);
        return Ok(());
    }
}

fn gb18030(c: char, gbk: bool, out: &mut Vec<u8>) -> Result<(), char> {
    let u = c as u32;
    if u == 0xE5E5 {
        return Err(c);
    }
    if gbk && u == 0x20AC {
        return put(out, 0x80);
    }
    let two = if let Ok(i) = cjk::GB18030_PUA.binary_search_by_key(&u, |e| e.0 as u32) {
        Some(cjk::GB18030_PUA[i].1 as usize)
    } else {
        pointer(cjk::GB18030_ORDER, |p| cjk::GB18030.get(p) as u32, u, |_| true)
    };
    if let Some(p) = two {
        let (lead, trail) = (p / 190, p % 190);
        out.push((lead + 0x81) as u8);
        out.push((trail + if trail < 0x3F { 0x40 } else { 0x41 }) as u8);
        return Ok(());
    }
    if gbk {
        return Err(c);
    }
    let p = gb18030_range_pointer(u);
    out.extend_from_slice(&[(p / 12600 + 0x81) as u8, (p / 1260 % 10 + 0x30) as u8, (p / 10 % 126 + 0x81) as u8, (p % 10 + 0x30) as u8]);
    Ok(())
}

fn big5(c: char, out: &mut Vec<u8>) -> Result<(), char> {
    let u = c as u32;
    // index Big5 pointer: from row 0xA1 on (HKSCS's rows decode, they are not written), the last of six
    // characters that are there twice
    let p = if matches!(u, 0x2550 | 0x255E | 0x2561 | 0x256A | 0x5341 | 0x5345) {
        let mut i = first_of(cjk::BIG5_ORDER, big5_code_point, u);
        let mut last = None;
        while i < cjk::BIG5_ORDER.len && big5_code_point(cjk::BIG5_ORDER.get(i) as usize) == u {
            last = Some(cjk::BIG5_ORDER.get(i) as usize);
            i += 1;
        }
        last
    } else {
        pointer(cjk::BIG5_ORDER, big5_code_point, u, |_| true)
    }
    .ok_or(c)?;
    let (lead, trail) = (p / 157, p % 157);
    out.push((lead + 0x81) as u8);
    out.push((trail + if trail < 0x3F { 0x40 } else { 0x62 }) as u8);
    Ok(())
}

fn euc_kr(c: char, out: &mut Vec<u8>) -> Result<(), char> {
    let p = pointer(cjk::EUC_KR_ORDER, |p| cjk::EUC_KR.get(p) as u32, c as u32, |_| true).ok_or(c)?;
    out.push((p / 190 + 0x81) as u8);
    out.push((p % 190 + 0x41) as u8);
    Ok(())
}
