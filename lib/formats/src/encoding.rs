// encoding -- text encodings: strict Unicode interchange, the single-byte character sets of the web and of
// old files, the labels of the WHATWG Encoding Standard.  New native library functionality.
//
//   decode(bytes, limit) -> (String, Encoding)   strict UTF-8 / UTF-16 by the byte-order mark, no guessing
//   encode(text, Encoding) -> bytes
//   Charset: the WHATWG single-byte encodings (windows-1252, ISO-8859-x, KOI8, macintosh, IBM866 ...) and
//            the DOS code pages (IBM437 ... 869), an exact ISO-8859-1; from_label (WHATWG), from_code_page
//            (Windows numbers), name, code_page, decode (U+FFFD for unassigned bytes), decode_char,
//            encode (Unmappable), encode_char
//   label(name) -> Label      what a WHATWG label names: UTF-8, UTF-16, a Charset, the replacement
//                             encoding, or an encoding this crate does not decode (Shift_JIS, GBK ...)
//   decode_label(label, bytes, limit) -> String   the standard's decode: a byte-order mark wins over the
//                             label, then the label's encoding, errors become U+FFFD
//
// The tables and labels are generated (charsets.rs, tools/gen_encodings.py): Python's codecs, made the
// standard's index where it differs (checked against the standard's data with --verify).
use alloc::{string::String, vec::Vec};

pub use crate::charsets::Charset;

/// Explicit text encodings; unmarked input is interpreted as UTF-8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

/// Invalid Unicode, an encoding this crate does not decode (UTF-32, an unknown or multi-byte label), a
/// caller-specified byte limit, or a character the character set cannot encode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    Invalid,
    Unsupported,
    Limit,
    Unmappable(char),
}

/// Decode without replacement characters or guessed legacy code pages.
pub fn decode(bytes: &[u8], limit: usize) -> Result<(String, Encoding), Error> {
    if bytes.len() > limit {
        return Err(Error::Limit);
    }
    if bytes.starts_with(&[0xff, 0xfe, 0, 0]) || bytes.starts_with(&[0, 0, 0xfe, 0xff]) {
        return Err(Error::Unsupported);
    }
    let (b, enc) = if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        (&bytes[3..], Encoding::Utf8Bom)
    } else if bytes.starts_with(&[0xff, 0xfe]) {
        (&bytes[2..], Encoding::Utf16Le)
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        (&bytes[2..], Encoding::Utf16Be)
    } else {
        (bytes, Encoding::Utf8)
    };
    let text = match enc {
        Encoding::Utf8 | Encoding::Utf8Bom => {
            String::from(core::str::from_utf8(b).map_err(|_| Error::Invalid)?)
        }
        _ => {
            if b.len() % 2 != 0 {
                return Err(Error::Invalid);
            }
            let units = b.chunks_exact(2).map(|p| {
                if enc == Encoding::Utf16Le {
                    u16::from_le_bytes([p[0], p[1]])
                } else {
                    u16::from_be_bytes([p[0], p[1]])
                }
            });
            let mut s = String::new();
            for c in core::char::decode_utf16(units) {
                s.push(c.map_err(|_| Error::Invalid)?);
            }
            s
        }
    };
    if text.len() > limit {
        return Err(Error::Limit);
    }
    Ok((text, enc))
}

/// Encode Unicode; UTF-16 output always includes its byte-order mark.
pub fn encode(text: &str, enc: Encoding) -> Vec<u8> {
    let mut out = Vec::new();
    match enc {
        Encoding::Utf8 => out.extend_from_slice(text.as_bytes()),
        Encoding::Utf8Bom => {
            out.extend_from_slice(&[0xef, 0xbb, 0xbf]);
            out.extend_from_slice(text.as_bytes());
        }
        Encoding::Utf16Le | Encoding::Utf16Be => {
            out.extend_from_slice(if enc == Encoding::Utf16Le {
                &[0xff, 0xfe]
            } else {
                &[0xfe, 0xff]
            });
            for u in text.encode_utf16() {
                out.extend_from_slice(&if enc == Encoding::Utf16Le {
                    u.to_le_bytes()
                } else {
                    u.to_be_bytes()
                });
            }
        }
    }
    out
}

/// What a label of the WHATWG Encoding Standard names (`label`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Label {
    /// UTF-8
    Utf8,
    /// UTF-16 little-endian (also the label "utf-16")
    Utf16Le,
    /// UTF-16 big-endian
    Utf16Be,
    /// a single-byte character set of this crate
    Single(Charset),
    /// the standard's replacement encoding (ISO-2022-KR, HZ and other labels it closes): any input
    /// is one U+FFFD
    Replacement,
    /// an encoding of the standard this crate does not decode, by its name: "Big5", "EUC-JP",
    /// "EUC-KR", "gb18030", "GBK", "ISO-2022-JP", "Shift_JIS"
    Other(&'static str),
}

/// The encoding a label names, as the WHATWG Encoding Standard reads it ("get an encoding"): leading
/// and trailing ASCII white space ignored, ASCII case ignored, the standard's labels only ("latin1",
/// "iso-8859-1" and "ascii" are windows-1252, "utf-16" is UTF-16LE).  None: no label of the standard.
pub fn label(name: &str) -> Option<Label> {
    let t = name.trim_matches(|c| matches!(c, '\t' | '\n' | '\x0C' | '\r' | ' '));
    // the longest label has 19 bytes
    if t.is_empty() || t.len() > 32 || !t.is_ascii() {
        return None;
    }
    let mut buf = [0u8; 32];
    for (d, s) in buf.iter_mut().zip(t.bytes()) {
        *d = s.to_ascii_lowercase();
    }
    crate::charsets::lookup(core::str::from_utf8(&buf[..t.len()]).ok()?)
}

/// Decode as the WHATWG Encoding Standard's "decode": a UTF-8 or UTF-16 byte-order mark wins over the
/// label (and is dropped), else the label's encoding; malformed input becomes U+FFFD, never an error.
/// Limit: more than `limit` bytes in or out; Unsupported: no label of the standard, or an encoding this
/// crate does not decode (`Label::Other`).
pub fn decode_label(label: &str, bytes: &[u8], limit: usize) -> Result<String, Error> {
    if bytes.len() > limit {
        return Err(Error::Limit);
    }
    let (named, body) = if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        (Some(Label::Utf8), rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0xff, 0xfe]) {
        (Some(Label::Utf16Le), rest)
    } else if let Some(rest) = bytes.strip_prefix(&[0xfe, 0xff]) {
        (Some(Label::Utf16Be), rest)
    } else {
        (self::label(label), bytes)
    };
    let text = match named.ok_or(Error::Unsupported)? {
        Label::Utf8 => String::from(String::from_utf8_lossy(body)),
        Label::Utf16Le => utf16_lossy(body, true),
        Label::Utf16Be => utf16_lossy(body, false),
        Label::Single(c) => c.decode(body),
        Label::Replacement if body.is_empty() => String::new(),
        Label::Replacement => String::from(char::REPLACEMENT_CHARACTER),
        Label::Other(_) => return Err(Error::Unsupported),
    };
    if text.len() > limit {
        return Err(Error::Limit);
    }
    Ok(text)
}

// UTF-16 with U+FFFD for lone surrogates and a last odd byte
fn utf16_lossy(b: &[u8], le: bool) -> String {
    let units = b.chunks_exact(2).map(|p| if le { u16::from_le_bytes([p[0], p[1]]) } else { u16::from_be_bytes([p[0], p[1]]) });
    let mut s: String = core::char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect();
    if b.len() % 2 != 0 {
        s.push(char::REPLACEMENT_CHARACTER);
    }
    s
}

impl Charset {
    /// The character set a WHATWG label names (`label`, single-byte sets only): "windows-1252",
    /// "latin1", "koi8-r", "cp1251", "iso8859-2" ...  The DOS code pages and the exact ISO-8859-1
    /// have no web label: `from_code_page` or the variant.
    pub fn from_label(name: &str) -> Option<Charset> {
        match label(name)? {
            Label::Single(c) => Some(c),
            _ => None,
        }
    }

    /// The text of these bytes; an unassigned byte becomes U+FFFD.
    pub fn decode(self, bytes: &[u8]) -> String {
        if bytes.is_ascii() {
            return String::from(core::str::from_utf8(bytes).unwrap_or(""));
        }
        let t = self.table();
        let mut s = String::with_capacity(bytes.len() + bytes.len() / 2);
        for &b in bytes {
            s.push(if b < 0x80 { b as char } else { char_of(t[b as usize - 0x80]).unwrap_or(char::REPLACEMENT_CHARACTER) });
        }
        s
    }

    /// The character of one byte (None: unassigned in this set).
    pub fn decode_char(self, b: u8) -> Option<char> {
        if b < 0x80 {
            Some(b as char)
        } else {
            char_of(self.table()[b as usize - 0x80])
        }
    }

    /// The byte of one character (None: not in this set; the first byte where a set has two).
    pub fn encode_char(self, c: char) -> Option<u8> {
        let u = c as u32;
        if u < 0x80 {
            return Some(u as u8);
        }
        self.table().iter().position(|&v| v != 0 && v as u32 == u).map(|i| 0x80 + i as u8)
    }

    /// The bytes of a text; Unmappable: the first character this set does not have.
    pub fn encode(self, text: &str) -> Result<Vec<u8>, Error> {
        let mut out = Vec::with_capacity(text.len());
        for c in text.chars() {
            out.push(self.encode_char(c).ok_or(Error::Unmappable(c))?);
        }
        Ok(out)
    }
}

fn char_of(v: u16) -> Option<char> {
    if v == 0 {
        None
    } else {
        char::from_u32(v as u32)
    }
}
