// cliprich -- bounded rich clipboard records and Windows CF_HTML framing.
// New opt-in API by clipboard_platform; no assembly counterpart.
// CF_HTML: learn.microsoft.com/windows/win32/dataxchg/html-clipboard-format

use crate::{CLIP_DOCX, CLIP_HTML, CLIP_RTF, CLIP_TEXT};
use alloc::vec::Vec;

/// Maximum combined payload of an opt-in multi-format clipboard write (64 MiB).
pub const CLIP_RICH_MAX: usize = 64 << 20;

/// Validate a multi-format text clipboard before replacing the previous contents.
/// Formats must be distinct TEXT/HTML/RTF/DOCX; text and HTML are NUL-free UTF-8.
pub fn clip_many_valid(entries: &[(u32, &[u8])]) -> bool {
    if entries.is_empty() || entries.len() > 4 {
        return false;
    }
    let mut total = 0usize;
    for (i, &(fmt, data)) in entries.iter().enumerate() {
        if !matches!(fmt, CLIP_TEXT | CLIP_HTML | CLIP_RTF | CLIP_DOCX)
            || entries[..i].iter().any(|e| e.0 == fmt)
        {
            return false;
        }
        let Some(n) = total.checked_add(data.len()) else {
            return false;
        };
        total = n;
        if total > CLIP_RICH_MAX {
            return false;
        }
        if matches!(fmt, CLIP_TEXT | CLIP_HTML)
            && (data.contains(&0) || core::str::from_utf8(data).is_err())
        {
            return false;
        }
    }
    true
}

/// Wrap a UTF-8 HTML fragment in Windows CF_HTML, with byte offsets and final NUL.
pub fn clip_html_encode(fragment: &[u8]) -> Option<Vec<u8>> {
    if fragment.len() > CLIP_RICH_MAX
        || fragment.contains(&0)
        || core::str::from_utf8(fragment).is_err()
    {
        return None;
    }
    let prefix = b"<html><body><!--StartFragment-->";
    let suffix = b"<!--EndFragment--></body></html>";
    let header = b"Version:1.0\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
    let start = header.len();
    let from = start + prefix.len();
    let to = from + fragment.len();
    let end = to + suffix.len();
    let mut data = alloc::format!("Version:1.0\r\nStartHTML:{start:010}\r\nEndHTML:{end:010}\r\nStartFragment:{from:010}\r\nEndFragment:{to:010}\r\n").into_bytes();
    data.extend_from_slice(prefix);
    data.extend_from_slice(fragment);
    data.extend_from_slice(suffix);
    data.push(0);
    Some(data)
}

/// Extract a UTF-8 fragment from CF_HTML. Reject invalid/overflowing offsets and UTF-8.
pub fn clip_html_decode(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() > CLIP_RICH_MAX + 65536 {
        return None;
    }
    let mut from = None;
    let mut to = None;
    let mut header_end = 0;
    for line in data[..data.len().min(65536)].split_inclusive(|&b| b == b'\n' || b == b'\r') {
        if line.starts_with(b"<") {
            break;
        }
        header_end += line.len();
        for (name, target) in [
            (b"StartFragment:".as_slice(), &mut from),
            (b"EndFragment:".as_slice(), &mut to),
        ] {
            if let Some(value) = line.strip_prefix(name) {
                if target.is_some() {
                    return None;
                }
                let text = core::str::from_utf8(value).ok()?.trim();
                if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                *target = Some(text.parse::<usize>().ok()?);
            }
        }
        if from.is_some() && to.is_some() {
            break;
        }
    }
    let (from, to) = (from?, to?);
    if from < header_end || from > to || to > data.len() {
        return None;
    }
    let fragment = &data[from..to];
    if fragment.len() > CLIP_RICH_MAX || fragment.contains(&0) || core::str::from_utf8(fragment).is_err() {
        return None;
    }
    Some(fragment.to_vec())
}
