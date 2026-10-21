// exif -- TIFF IFD reader (EXIF blocks; shared with the TIFF codec) and EXIF helpers
//                                                                          [src/img/exif.s]
//
//   Tifr::open(data) -> Ok(reader) / Err(ERR_TRUNC / ERR_HDR)
//   tr.visit(off) -> 0 / ERR_LIMIT        record an IFD (a cycle or TIFR_MAXIFD IFDs: limit)
//   tr.count(off) -> entries (clamped to what fits) / ERR_TRUNC
//   tr.ifd(off) -> entries / ERR_*        visit + count
//   tr.entry(off, i) -> Ok(TEnt) / Err(ERR_HDR unknown type / ERR_TRUNC)
//   tr.find(off, tag) -> Some(TEnt)       first entry with the tag (bad entries skipped)
//   tr.get(te, i) -> u64                  integer value i (signed types sign-extended,
//                                         RATIONAL: numerator, FLOAT / DOUBLE: raw bits)
//   tr.rational(te, i) -> (num, den)      RATIONAL / SRATIONAL value i (others: value / 1)
//   tr.next(off) -> offset of the next IFD (0 = none)
//   exif_read(blob) -> (0 / ERR_*, ExifInfo)   orientation, resolution, colour space
//   exif_orientation(blob) -> 1..8
//   exif_set_orientation(blob, o) -> bool      patch the IFD0 Orientation SHORT in place
//   exif_copy_orient1(blob) -> Vec             copy with orientation 1
//
// The reader works on a TIFF structure in memory: "II*\0" / "MM\0*" (classic, 4-byte
// offsets, 12-byte entries) or BigTIFF "II+\0" / "MM\0+" (8-byte offsets, 20-byte
// entries); every read is bounds-checked, value spans must lie inside the data.

use alloc::vec::Vec;

use crate::{ERR_HDR, ERR_LIMIT, ERR_TRUNC};

pub const TIFR_MAXIFD: usize = 1024; // IFDs per reader (chain + sub-IFDs), cycles detected
pub const TIFR_MAXDEPTH: u32 = 3; // sub-IFD nesting callers must respect

const TAG_ORIENT: u32 = 0x0112;
const TAG_XRES: u32 = 0x011A;
const TAG_YRES: u32 = 0x011B;
const TAG_RESUNIT: u32 = 0x0128;
const TAG_EXIFIFD: u32 = 0x8769;
const TAG_CSPACE: u32 = 0xA001;
const TAG_INTEROP: u32 = 0xA005;
const TT_SHORT: u32 = 3;
const TT_RATIONAL: u32 = 5;
const TT_SRATIONAL: u32 = 10;
const TT_MAX: u32 = 18;

// bytes per value of TIFF field types 0..18 (0 = unknown type)
static TSIZE: [u8; 19] = [0, 1, 1, 2, 4, 8, 1, 1, 2, 4, 8, 4, 8, 4, 0, 0, 8, 8, 8];

/// one decoded IFD entry (TENT)
#[derive(Clone, Copy, Default, Debug)]
pub struct TEnt {
    pub tag: u32,
    pub typ: u32,   // TIFF field type 1..18
    pub count: u64, // number of values
    pub voff: u64,  // offset of the values in the TIFF structure
    pub bytes: u64, // count * type size (inside the data)
}

/// a TIFF IFD reader (TIFR)
pub struct Tifr<'a> {
    pub data: &'a [u8],
    pub be: bool,  // "MM"
    pub big: bool, // BigTIFF
    pub ifd0: u64, // offset of the first IFD
    vis: Vec<u64>, // IFDs read so far
}

impl<'a> Tifr<'a> {
    fn rd16(&self, off: u64) -> u32 {
        let o = off as usize;
        let b = [self.data[o], self.data[o + 1]];
        (if self.be { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }) as u32
    }

    fn rd32(&self, off: u64) -> u32 {
        let o = off as usize;
        let b = [self.data[o], self.data[o + 1], self.data[o + 2], self.data[o + 3]];
        if self.be { u32::from_be_bytes(b) } else { u32::from_le_bytes(b) }
    }

    fn rd64(&self, off: u64) -> u64 {
        let o = off as usize;
        let b: [u8; 8] = self.data[o..o + 8].try_into().unwrap();
        if self.be { u64::from_be_bytes(b) } else { u64::from_le_bytes(b) }
    }

    fn len(&self) -> u64 {
        self.data.len() as u64
    }

    /// start reading the TIFF structure: byte order, classic / BigTIFF, first IFD offset
    /// [tifr_open]
    pub fn open(data: &'a [u8]) -> Result<Tifr<'a>, i32> {
        let mut t = Tifr { data, be: false, big: false, ifd0: 0, vis: Vec::new() };
        if data.len() < 8 {
            return Err(ERR_TRUNC);
        }
        match (data[0], data[1]) {
            (0x49, 0x49) => {}
            (0x4D, 0x4D) => t.be = true,
            _ => return Err(ERR_HDR),
        }
        match t.rd16(2) {
            42 => t.ifd0 = t.rd32(4) as u64,
            43 => {
                if data.len() < 16 {
                    return Err(ERR_TRUNC);
                }
                if t.rd16(4) != 8 || t.rd16(6) != 0 {
                    return Err(ERR_HDR);
                }
                t.big = true;
                t.ifd0 = t.rd64(8);
            }
            _ => return Err(ERR_HDR),
        }
        Ok(t)
    }

    /// record that the IFD at off is being read: ERR_LIMIT when it was read before or
    /// TIFR_MAXIFD IFDs were read  [tifr_visit]
    pub fn visit(&mut self, off: u64) -> i32 {
        if self.vis.contains(&off) || self.vis.len() >= TIFR_MAXIFD {
            return ERR_LIMIT;
        }
        self.vis.push(off);
        0
    }

    /// entries of the IFD at off, clamped to the entries that fit, or ERR_TRUNC
    /// [tifr_count]
    pub fn count(&self, off: u64) -> i64 {
        let len = self.len();
        if off > len {
            return ERR_TRUNC as i64;
        }
        let rest = len - off;
        if !self.big {
            if rest < 2 {
                return ERR_TRUNC as i64;
            }
            let fit = (rest - 2) / 12;
            (self.rd16(off) as u64).min(fit) as i64
        } else {
            if rest < 8 {
                return ERR_TRUNC as i64;
            }
            let fit = (rest - 8) / 20;
            self.rd64(off).min(fit) as i64
        }
    }

    /// visit + count  [tifr_ifd]
    pub fn ifd(&mut self, off: u64) -> i64 {
        let r = self.visit(off);
        if r != 0 {
            return r as i64;
        }
        self.count(off)
    }

    /// entry i (i < count) of the IFD at off; the value span is checked to lie inside the
    /// data  [tifr_entry]
    pub fn entry(&self, off: u64, i: u64) -> Result<TEnt, i32> {
        let len = self.len();
        let (esz, hdr) = if self.big { (20u64, 8u64) } else { (12, 2) };
        // e = off + hdr + i * esz (the assembly: lea + imul, a carry of the last add fails)
        let (e, carry) = off.wrapping_add(hdr).overflowing_add(i.wrapping_mul(esz));
        if carry {
            return Err(ERR_TRUNC);
        }
        if e > len || len - e < esz {
            return Err(ERR_TRUNC);
        }
        let tag = self.rd16(e);
        let typ = self.rd16(e + 2);
        if typ > TT_MAX {
            return Err(ERR_HDR);
        }
        let sz = TSIZE[typ as usize] as u64;
        if sz == 0 {
            return Err(ERR_HDR);
        }
        let (count, cap, vf) = if self.big {
            (self.rd64(e + 4), 8u64, e + 12)
        } else {
            (self.rd32(e + 4) as u64, 4u64, e + 8)
        };
        if count > len {
            return Err(ERR_TRUNC);
        }
        let bytes = count * sz;
        let mut voff = vf;
        if bytes > cap {
            voff = if self.big { self.rd64(vf) } else { self.rd32(vf) as u64 };
            if voff > len || len - voff < bytes {
                return Err(ERR_TRUNC);
            }
        }
        Ok(TEnt { tag, typ, count, voff, bytes })
    }

    /// the first entry of the IFD at off with the given tag (entries that fail entry() are
    /// skipped)  [tifr_find]
    pub fn find(&self, off: u64, tag: u32) -> Option<TEnt> {
        let n = self.count(off);
        if n <= 0 {
            return None;
        }
        (0..n as u64).filter_map(|i| self.entry(off, i).ok()).find(|t| t.tag == tag)
    }

    /// value i of an entry as an integer: BYTE / ASCII / UNDEFINED / SHORT / LONG / IFD /
    /// LONG8 / IFD8 zero-extended, SBYTE / SSHORT / SLONG / SLONG8 sign-extended,
    /// RATIONAL: numerator, FLOAT / DOUBLE: raw bits; 0 when i >= count  [tifr_get]
    pub fn get(&self, te: &TEnt, i: u64) -> u64 {
        if i >= te.count || te.typ > TT_MAX {
            return 0;
        }
        let o = te.voff + i * TSIZE[te.typ as usize] as u64;
        match te.typ {
            1 | 2 | 7 => self.data[o as usize] as u64,
            6 => self.data[o as usize] as i8 as i64 as u64,
            3 => self.rd16(o) as u64,
            8 => self.rd16(o) as u16 as i16 as i64 as u64,
            9 => self.rd32(o) as i32 as i64 as u64,
            12 | 16.. => self.rd64(o),
            _ => self.rd32(o) as u64, // LONG, IFD, RATIONAL, SRATIONAL, FLOAT
        }
    }

    /// RATIONAL value i as (numerator, denominator) (SRATIONAL: sign-extended); other types
    /// give value / 1; i >= count: 0 / 0  [tifr_rational]
    pub fn rational(&self, te: &TEnt, i: u64) -> (u64, u64) {
        if te.typ != TT_RATIONAL && te.typ != TT_SRATIONAL {
            return (self.get(te, i), 1);
        }
        if i >= te.count {
            return (0, 0);
        }
        let o = te.voff + i * 8;
        let (n, d) = (self.rd32(o), self.rd32(o + 4));
        if te.typ == TT_SRATIONAL {
            (n as i32 as i64 as u64, d as i32 as i64 as u64)
        } else {
            (n as u64, d as u64)
        }
    }

    /// offset of the IFD following the one at off (0 = none, or it lies outside the data)
    /// [tifr_next]
    pub fn next(&self, off: u64) -> u64 {
        let len = self.len();
        if off > len {
            return 0;
        }
        let rest = len - off;
        let v = if !self.big {
            if rest < 2 {
                return 0;
            }
            let p = off + self.rd16(off) as u64 * 12 + 2;
            if p > len || len - p < 4 {
                return 0;
            }
            self.rd32(p) as u64
        } else {
            if rest < 8 {
                return 0;
            }
            let n = self.rd64(off);
            if n > len {
                return 0;
            }
            let p = off + n * 20 + 8;
            if p > len || len - p < 8 {
                return 0;
            }
            self.rd64(p)
        };
        if v < len { v } else { 0 }
    }
}

/// what exif_read finds (EXIFINFO)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExifInfo {
    pub orient: u32, // 1..8 (1 when absent / invalid)
    pub unit: u32,   // ResolutionUnit (1 none, 2 inch, 3 cm; 0 absent)
    pub dpix: u32,   // 16.16 dpi (0 = unknown)
    pub dpiy: u32,
    pub cspace: u32, // ColorSpace (1 = sRGB, 0xFFFF uncalibrated, 0 absent)
    pub orioff: u32, // offset of the orientation SHORT in the blob (0 = none)
}

impl Default for ExifInfo {
    fn default() -> Self {
        ExifInfo { orient: 1, unit: 0, dpix: 0, dpiy: 0, cspace: 0, orioff: 0 }
    }
}

impl ExifInfo {
    /// the 24-byte EXIFINFO record
    pub fn to_bytes(&self) -> [u8; 24] {
        let mut b = [0u8; 24];
        for (k, v) in [self.orient, self.unit, self.dpix, self.dpiy, self.cspace, self.orioff].iter().enumerate() {
            b[k * 4..k * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }
}

// exif_dpi(num, den, unit): 16.16 dots per inch (0 = unknown): unit 2 inch, 3 cm; rounded,
// clamped to 0xFFFFFFFF
fn exif_dpi(num: u64, den: u64, unit: u32) -> u32 {
    if den == 0 {
        return 0;
    }
    let mut d = den & 0xFFFF_FFFF;
    if d == 0 {
        return 0;
    }
    let mut n = (num & 0xFFFF_FFFF) << 16;
    match unit {
        2 => {}
        3 => {
            n *= 254;
            d *= 100;
        }
        _ => return 0,
    }
    ((n + (d >> 1)) / d).min(0xFFFF_FFFF) as u32
}

/// fill an ExifInfo from an EXIF TIFF structure: IFD0 Orientation, X / YResolution,
/// ResolutionUnit (default inch), the Exif IFD (ColorSpace; the Interop IFD is visited),
/// then the IFD chain is walked until its end, a cycle or the limit (errors there are
/// ignored) -> (0 / ERR_TRUNC / ERR_HDR / ERR_LIMIT, info)  [exif_read]
pub fn exif_read(blob: &[u8]) -> (i32, ExifInfo) {
    let mut info = ExifInfo::default();
    let mut tr = match Tifr::open(blob) {
        Ok(t) => t,
        Err(e) => return (e, info),
    };
    let ifd0 = tr.ifd0;
    let n = tr.ifd(ifd0);
    if n < 0 {
        return (n as i32, info);
    }
    let (mut xr, mut yr) = ((0u64, 0u64), (0u64, 0u64));
    let mut unit = 0u32;
    let mut exififd: Option<u64> = None;
    for i in 0..n as u64 {
        let Ok(te) = tr.entry(ifd0, i) else { continue };
        match te.tag {
            TAG_ORIENT => {
                if te.typ != TT_SHORT || info.orioff != 0 {
                    continue; // the first one wins
                }
                info.orioff = te.voff as u32;
                let v = tr.get(&te, 0);
                if (1..=8).contains(&v) {
                    info.orient = v as u32;
                }
            }
            TAG_XRES => xr = tr.rational(&te, 0),
            TAG_YRES => yr = tr.rational(&te, 0),
            TAG_RESUNIT => {
                unit = tr.get(&te, 0) as u32;
                info.unit = unit;
            }
            TAG_EXIFIFD => exififd = Some(tr.get(&te, 0)),
            _ => {}
        }
    }
    let u = if unit == 0 { 2 } else { unit };
    info.dpix = exif_dpi(xr.0, xr.1, u);
    info.dpiy = exif_dpi(yr.0, yr.1, u);
    if let Some(off) = exififd
        && tr.visit(off) == 0
    {
        if let Some(te) = tr.find(off, TAG_CSPACE) {
            info.cspace = tr.get(&te, 0) as u32;
        }
        if let Some(te) = tr.find(off, TAG_INTEROP) {
            let o = tr.get(&te, 0);
            tr.ifd(o); // validate only
        }
    }
    // IFD chain after IFD0 (IFD1 = thumbnail, ...)
    let mut off = ifd0;
    loop {
        let nx = tr.next(off);
        if nx == 0 {
            break;
        }
        off = nx;
        if tr.ifd(off) < 0 {
            break;
        }
    }
    (0, info)
}

/// the IFD0 orientation of an EXIF TIFF structure (1 when missing, invalid or unreadable)
/// [exif_orientation]
pub fn exif_orientation(blob: &[u8]) -> u32 {
    exif_read(blob).1.orient
}

/// overwrite the IFD0 Orientation SHORT in place (in the blob's byte order) -> true
/// patched / false no orientation tag  [exif_set_orientation]
pub fn exif_set_orientation(blob: &mut [u8], orient: u32) -> bool {
    let off = exif_read(blob).1.orioff as usize;
    if off == 0 || off + 2 > blob.len() {
        return false;
    }
    let v = orient as u16;
    let b = if blob[0] == 0x4D { v.to_be_bytes() } else { v.to_le_bytes() };
    blob[off..off + 2].copy_from_slice(&b);
    true
}

/// a copy of an EXIF block with its orientation set to 1 (for pixels rotated already);
/// empty for an empty blob  [exif_copy_orient1]
pub fn exif_copy_orient1(blob: &[u8]) -> Vec<u8> {
    let mut v = blob.to_vec();
    if !v.is_empty() {
        exif_set_orientation(&mut v, 1);
    }
    v
}
