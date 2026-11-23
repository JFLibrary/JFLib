// media -- the base64 data of the XML format: pictures (ForeignData), the macro project
//
//   base64(text, max) -> Option<Vec<u8>>     standard alphabet, white space skipped, padding
//                                            optional; None: another character or more than max
//   picture(bytes, foreign_type) -> Option<(ext, content type, bytes)>   what the bytes are, by
//                                            their signature; a bitmap without file header (a DIB,
//                                            as ForeignType='Bitmap' may hold it) gets one

use alloc::vec::Vec;

/// the signature of an OLE compound file (vbaProject.bin)
pub(super) const OLE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// base64 text into bytes, at most `max` of them
pub(super) fn base64(s: &[u8], max: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity((s.len() / 4 * 3).min(max));
    let mut acc = 0u32;
    let mut bits = 0u32;
    let mut pad = false;
    for &c in s {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                pad = true;
                continue;
            }
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        if pad {
            // data after the padding
            return None;
        }
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            if out.len() >= max {
                return None;
            }
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|x| u16::from_le_bytes([x[0], x[1]]))
}

/// what a picture's bytes are: (extension, content type, the bytes of the file)
pub(super) fn picture(b: Vec<u8>, foreign_type: &str) -> Option<(&'static str, &'static str, Vec<u8>)> {
    let kind = if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        ("png", "image/png")
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        ("jpeg", "image/jpeg")
    } else if b.starts_with(b"GIF8") {
        ("gif", "image/gif")
    } else if b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") {
        ("tiff", "image/tiff")
    } else if b.starts_with(b"BM") {
        ("bmp", "image/bmp")
    } else if le32(&b, 0) == Some(1) && b.get(40..44) == Some(b" EMF") {
        ("emf", "image/x-emf")
    } else if b.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) || (le16(&b, 0) == Some(1) && le16(&b, 2) == Some(9)) {
        ("wmf", "image/x-wmf")
    } else if foreign_type.eq_ignore_ascii_case("Bitmap") {
        return dib(b).map(|f| ("bmp", "image/bmp", f));
    } else {
        return None;
    };
    Some((kind.0, kind.1, b))
}

// a device-independent bitmap (BITMAPINFOHEADER and the pixels) with the 14-byte file header
// in front that makes it a .bmp file
fn dib(b: Vec<u8>) -> Option<Vec<u8>> {
    let size = le32(&b, 0)?;
    if !matches!(size, 12 | 40 | 52 | 56 | 64 | 108 | 124) || b.len() < size as usize {
        return None;
    }
    // the colour table: core headers (12) have 3-byte entries, the others 4-byte ones
    let (bits, used, compression) = if size == 12 { (le16(&b, 10)? as u32, 0, 0) } else { (le16(&b, 14)? as u32, le32(&b, 32)?, le32(&b, 16)?) };
    let entries = if used != 0 { used } else if bits <= 8 { 1u32 << bits } else { 0 };
    let entry = if size == 12 { 3 } else { 4 };
    // BI_BITFIELDS / BI_ALPHABITFIELDS with the 40-byte header: the masks follow it
    let masks = if size == 40 && compression == 3 { 12 } else if size == 40 && compression == 6 { 16 } else { 0 };
    let offset = 14u64 + size as u64 + entries as u64 * entry + masks;
    let total = 14u64 + b.len() as u64;
    if offset > total || total > u32::MAX as u64 {
        return None;
    }
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(&b);
    Some(out)
}
