// clipdrop -- the shared helpers of the clipboard image formats and file drops (every OS)
//                                     [src/plat/clipdrop.s; docs/modules/CLIPBOARD_DND.md]
//
//   drop_reset()                 start a new pending drop (the old list and data are dropped)
//   drop_add(path) -> bool       one path (empty, too long, NUL inside or too many: false)
//   drop_uri_list(data) -> n     text/uri-list: every file:// URI (host empty or "localhost"),
//                                %XX decoded, '#' lines skipped
//   drop_take() -> paths         hand the pending list over (E_DROP data = its count)
//   clip_dib_to_bmp(dib) -> BMP file   a clipboard DIB (BITMAPCOREHEADER / INFO / V4 / V5
//        header, masks, colour table, pixels) as a BMP file, None when the header is
//        invalid.  32-bit BI_RGB / BI_BITFIELDS / BI_ALPHABITFIELDS DIBs are rewritten with
//        a V5 header (BI_BITFIELDS, the source masks) and paint.net's alpha rule: the alpha
//        channel (the 4th byte of BI_RGB, else the header's alpha mask) is used when any
//        pixel has a non-zero alpha, else the image is opaque (alpha mask 0).  Other depths
//        are copied with a BITMAPFILEHEADER in front.
//   clip_bmp_fix(bmp) -> BMP file   the same rule for a BMP file (image/bmp of other programs)
//
// Every length is checked: clipboard / drop data come from other programs.  The pending
// list keeps raw bytes (a decoded URI need not be UTF-8); drop_take converts lossily.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use rt::sync::SpinLock;

const DROP_MAX_FILES: usize = 4096;
const DROP_MAX_PATH: usize = 32768; // bytes of one path
const DROP_MAX_BYTES: usize = 1 << 24; // the whole list
const DIB_MAX_HDR: u32 = 4096;
const V5_SIZE: usize = 124;
const BF_SIZE: usize = 14;
const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const BI_ALPHABITFIELDS: u32 = 6;

struct Pending {
    list: Vec<Vec<u8>>,
    bytes: usize,
}

static PENDING: SpinLock<Pending> = SpinLock::new(Pending { list: Vec::new(), bytes: 0 });

/// forget the pending list (and the rest of the pending drop: dnd.rs)
pub fn drop_reset() {
    let mut p = PENDING.lock();
    p.list = Vec::new();
    p.bytes = 0;
    drop(p);
    crate::dnd::rich_reset();
}

fn add_bytes(path: &[u8]) -> bool {
    let n = path.len();
    if n == 0 || n > DROP_MAX_PATH || path.contains(&0) {
        return false;
    }
    let mut p = PENDING.lock();
    if p.list.len() >= DROP_MAX_FILES || p.bytes + n + 2 > DROP_MAX_BYTES {
        return false;
    }
    p.bytes += n + 1;
    p.list.push(path.to_vec());
    true
}

/// one path (copied) -> false when it is empty, too long, has a NUL or the list is full
pub fn drop_add(path: &str) -> bool {
    add_bytes(path.as_bytes())
}

fn hexval(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        _ => {
            let l = c | 32;
            if (b'a'..=b'f').contains(&l) { Some(l - b'a' + 10) } else { None }
        }
    }
}

/// text/uri-list -> paths added
pub fn drop_uri_list(data: &[u8]) -> u32 {
    let mut n = 0;
    if data.is_empty() || data.len() > DROP_MAX_BYTES {
        return 0;
    }
    for line in data.split(|&c| c == b'\n') {
        let mut l = line;
        if l.last() == Some(&13) {
            l = &l[..l.len() - 1];
        }
        if l.len() < 6 || l[0] == b'#' {
            continue; // shorter than "file:/"
        }
        if !l[..4].eq_ignore_ascii_case(b"file") || l[4] != b':' {
            continue;
        }
        let mut r = &l[5..];
        // "//host/path": host must be empty or localhost
        if r.len() >= 2 && &r[..2] == b"//" {
            r = &r[2..];
            if r.is_empty() {
                continue;
            }
            if r[0] != b'/' {
                if r.len() < 9 || !r[..9].eq_ignore_ascii_case(b"localhost") {
                    continue;
                }
                r = &r[9..];
                if r.is_empty() || r[0] != b'/' {
                    continue;
                }
            }
        }
        if r.is_empty() || r[0] != b'/' {
            continue;
        }
        // percent-decode
        let mut out = Vec::with_capacity(r.len());
        let mut i = 0;
        let mut ok = true;
        while i < r.len() {
            let c = r[i];
            i += 1;
            if c != b'%' {
                out.push(c);
                continue;
            }
            if i + 2 > r.len() {
                ok = false; // truncated escape: skip the URI
                break;
            }
            match (hexval(r[i]), hexval(r[i + 1])) {
                (Some(a), Some(b)) if (a << 4 | b) != 0 => out.push(a << 4 | b),
                _ => {
                    ok = false;
                    break;
                }
            }
            i += 2;
        }
        if ok && add_bytes(&out) {
            n += 1;
        }
    }
    n
}

/// hand the pending list over (it starts empty again)
pub fn drop_take() -> Vec<String> {
    let mut p = PENDING.lock();
    let l = core::mem::take(&mut p.list);
    p.bytes = 0;
    l.into_iter().map(|b| String::from_utf8_lossy(&b).into_owned()).collect()
}

fn u16_at(d: &[u8], o: usize) -> u32 {
    u16::from_le_bytes([d[o], d[o + 1]]) as u32
}

fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// a clipboard DIB as a BMP file (None: invalid header)
pub fn clip_dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let len = dib.len();
    if len < 12 {
        return None;
    }
    let hs = u32_at(dib, 0);
    let (w, h, bpp, comp, used, pe): (u32, i32, u32, u32, u32, u64);
    if hs == 12 {
        w = u16_at(dib, 4);
        h = u16_at(dib, 6) as u16 as i16 as i32;
        bpp = u16_at(dib, 10);
        comp = BI_RGB;
        used = 0;
        pe = 3;
    } else {
        if hs < 40 || hs > DIB_MAX_HDR || hs as usize > len {
            return None;
        }
        w = u32_at(dib, 4);
        h = u32_at(dib, 8) as i32;
        bpp = u16_at(dib, 14);
        comp = u32_at(dib, 16);
        used = u32_at(dib, 32);
        pe = 4;
    }
    if w.wrapping_sub(1) >= 65535 || (h.wrapping_abs() as u32).wrapping_sub(1) >= 65535 {
        return None; // width / |height| 1..65535 (INT_MIN stays out)
    }
    if ![1, 4, 8, 16, 24, 32].contains(&bpp) || comp > 6 {
        return None;
    }
    // pixel offset: header, masks after a 40-byte header, colour table
    let mut off = hs as u64;
    if hs == 40 {
        if comp == BI_BITFIELDS {
            off += 12;
        } else if comp == BI_ALPHABITFIELDS {
            off += 16;
        }
    }
    let mut ncol = used as u64;
    if bpp <= 8 {
        if ncol == 0 {
            ncol = 1 << bpp;
        }
        ncol = ncol.min(256);
    }
    off += ncol * pe;
    if off > len as u64 {
        return None;
    }
    let off = off as usize;
    let mut masks = [0u32; 4];
    let v5 = bpp == 32
        && match comp {
            BI_RGB => {
                masks = [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000];
                true
            }
            BI_BITFIELDS | BI_ALPHABITFIELDS => {
                // masks at offset 40 (inside a V2+ header or right after a 40-byte one)
                if len < 52 {
                    return None;
                }
                masks = [u32_at(dib, 40), u32_at(dib, 44), u32_at(dib, 48), 0];
                if hs >= 56 || (comp == BI_ALPHABITFIELDS && hs == 40) {
                    if len < 56 {
                        return None;
                    }
                    masks[3] = u32_at(dib, 52);
                }
                true
            }
            _ => false,
        };
    if !v5 {
        // any other DIB: a file header in front of the bytes as they are
        let total = len + BF_SIZE;
        let mut f = vec![0u8; BF_SIZE];
        f[0] = b'B';
        f[1] = b'M';
        f[2..6].copy_from_slice(&(total as u32).to_le_bytes());
        f[10..14].copy_from_slice(&((off + BF_SIZE) as u32).to_le_bytes());
        f.extend_from_slice(dib);
        return Some(f);
    }
    // pixel bytes: w * 4 * |h|
    let pb = w as u64 * 4 * h.wrapping_abs() as u32 as u64;
    if pb > (len - off) as u64 {
        return None; // truncated pixel data
    }
    let pb = pb as usize;
    let px = &dib[off..off + pb];
    // the alpha rule: any non-zero alpha keeps the mask, else opaque
    if masks[3] != 0 && !px.chunks_exact(4).any(|p| u32::from_le_bytes([p[0], p[1], p[2], p[3]]) & masks[3] != 0) {
        masks[3] = 0;
    }
    // BITMAPFILEHEADER + BITMAPV5HEADER + pixels
    let total = pb + BF_SIZE + V5_SIZE;
    let mut f = vec![0u8; BF_SIZE + V5_SIZE];
    f[0] = b'B';
    f[1] = b'M';
    f[2..6].copy_from_slice(&(total as u32).to_le_bytes());
    f[10..14].copy_from_slice(&((BF_SIZE + V5_SIZE) as u32).to_le_bytes());
    let v = &mut f[BF_SIZE..];
    v[0..4].copy_from_slice(&(V5_SIZE as u32).to_le_bytes());
    v[4..8].copy_from_slice(&w.to_le_bytes());
    v[8..12].copy_from_slice(&h.to_le_bytes());
    v[12..14].copy_from_slice(&1u16.to_le_bytes());
    v[14..16].copy_from_slice(&32u16.to_le_bytes());
    v[16..20].copy_from_slice(&BI_BITFIELDS.to_le_bytes());
    v[20..24].copy_from_slice(&(pb as u32).to_le_bytes());
    v[24..28].copy_from_slice(&2835u32.to_le_bytes());
    v[28..32].copy_from_slice(&2835u32.to_le_bytes());
    for (k, m) in masks.iter().enumerate() {
        v[40 + k * 4..44 + k * 4].copy_from_slice(&m.to_le_bytes());
    }
    v[56..60].copy_from_slice(&0x7352_4742u32.to_le_bytes()); // LCS_sRGB
    v[108..112].copy_from_slice(&4u32.to_le_bytes()); // LCS_GM_IMAGES
    f.extend_from_slice(px);
    Some(f)
}

/// a BMP file of another program with the same alpha rule
pub fn clip_bmp_fix(bmp: &[u8]) -> Option<Vec<u8>> {
    if bmp.len() < BF_SIZE + 12 || bmp[0] != b'B' || bmp[1] != b'M' {
        return None;
    }
    clip_dib_to_bmp(&bmp[BF_SIZE..])
}

// ---- plat-win: the CLIPIMG record of clip_set(CLIP_IMAGE, data) as bytes (the assembly passes a
// record of pointers CI_PNG / CI_PNG_LEN / CI_BMP / CI_BMP_LEN): u64 LE PNG length, the PNG file,
// u64 LE BMP length, the BMP file (BITMAPV5HEADER, 32 bit, BI_BITFIELDS, straight alpha) -- the
// same bytes as ui::clipimg_record / clipimg_parse (ui-core).  The OS layers read it with
// clipimg_split.

/// the CLIPIMG bytes of a PNG file and a BMP file (either may be empty)
pub fn clipimg_pack(png: &[u8], bmp: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(16 + png.len() + bmp.len());
    for p in [png, bmp] {
        v.extend_from_slice(&(p.len() as u64).to_le_bytes());
        v.extend_from_slice(p);
    }
    v
}

/// (PNG, BMP) of CLIPIMG bytes (an absent part is empty; None: malformed)
pub fn clipimg_split(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let mut parts: [&[u8]; 2] = [&[], &[]];
    let mut o = 0usize;
    for p in parts.iter_mut() {
        let n = u64::from_le_bytes(data.get(o..o.checked_add(8)?)?.try_into().ok()?);
        o += 8;
        let n = usize::try_from(n).ok()?;
        *p = data.get(o..o.checked_add(n)?)?;
        o += n;
    }
    Some((parts[0], parts[1]))
}
