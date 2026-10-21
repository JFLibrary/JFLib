// Strict Unicode interchange codecs. New native library functionality (no legacy counterpart).
use alloc::{string::String, vec::Vec};

/// Explicit text encodings; unmarked input is interpreted as UTF-8.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}

/// Invalid Unicode, unsupported UTF-32, or a caller-specified byte limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unsupported,
    Limit,
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
