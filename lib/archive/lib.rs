// In-memory ZIP interchange (ZIP32, ZIP64 when it is needed). New native library code,
// independent of document/UI code. Raw DEFLATE, inflate and CRC reuse img_core's safe codec
// primitives; no paths are extracted. `opc` builds the Open Packaging Conventions on top.
// ZipFs shows an archive as a read-only plat::vfs tree:
//   plat::vfs::mount("pack", Arc::new(archive::ZipFs::new(&bytes, Limits::default())?))
//   -> plat::read_file("pack://textures/a.png"), plat::File::open("pack://..", FO_READ),
//   plat::fs::stat (size; mtime: the entry's DOS time, read as UTC)
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
use alloc::{
    collections::{BTreeMap, BTreeSet},
    string::String,
    vec,
    vec::Vec,
};
use img_core::{checksum::crc32, deflate::deflate_raw, inflate::inflate_raw_ex};

pub mod cfb;
pub mod opc;

/// One decoded archive part with a normalized, relative UTF-8 name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}
/// Bounds are checked before decompression or allocation of advertised sizes. The default
/// (512 parts, 8 MiB a part, 32 MiB in all, archives up to 32 MiB) suits documents; readers of
/// large packages pass larger values and a smaller `ratio`:
/// `Limits { entries: 100_000, part_bytes: 512 << 20, total_bytes: 2 << 30, archive_bytes: 1 << 30, ratio: 200 }`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Most parts in one archive.
    pub entries: usize,
    /// Most bytes of one part, uncompressed.
    pub part_bytes: usize,
    /// Most bytes of all parts together, uncompressed.
    pub total_bytes: usize,
    /// Most bytes of the archive itself.
    pub archive_bytes: usize,
    /// Highest ratio of a DEFLATE part's size to its compressed size, checked for parts over
    /// 1 MiB before they are inflated: a denser part is refused with `Limit` (a ZIP bomb). The
    /// default, 1032, is the most DEFLATE can reach and so refuses nothing; a part claiming more
    /// is `Invalid` whatever this value is.
    pub ratio: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            entries: 512,
            part_bytes: 8 << 20,
            total_bytes: 32 << 20,
            archive_bytes: 32 << 20,
            ratio: MAX_RATIO,
        }
    }
}
/// Structural, integrity, feature or resource-limit failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    Invalid,
    Unsupported,
    Checksum,
    Limit,
}
/// How `write_with` stores the parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Method {
    /// Uncompressed, as `write` stores them.
    Stored,
    /// DEFLATE at a level from 1 (fastest) to 9 (smallest); 6 is the usual choice, 0 counts as
    /// 1 and higher values as 9. A part that does not get smaller is stored instead.
    Deflate(u8),
}

// the most a DEFLATE stream can expand: 258 bytes from a 1-bit length and a 1-bit distance code
const MAX_RATIO: usize = 1032;
// parts up to this size are bounded by part_bytes alone, whatever Limits::ratio says
const RATIO_FLOOR: u64 = 1 << 20;
// the 16- and 32-bit fields that point to a ZIP64 record
const M16: u16 = 0xFFFF;
const M32: u32 = 0xFFFF_FFFF;
// version needed to extract: 2.0 (DEFLATE), 4.5 (ZIP64)
const V20: u16 = 20;
const V45: u16 = 45;

fn range(b: &[u8], p: usize, n: usize) -> Result<&[u8], Error> {
    b.get(p..p.checked_add(n).ok_or(Error::Invalid)?)
        .ok_or(Error::Invalid)
}
fn u16at(b: &[u8], p: usize) -> Result<u16, Error> {
    let s = range(b, p, 2)?;
    Ok(u16::from_le_bytes([s[0], s[1]]))
}
fn u32at(b: &[u8], p: usize) -> Result<u32, Error> {
    let s = range(b, p, 4)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn u64at(b: &[u8], p: usize) -> Result<u64, Error> {
    Ok(u32at(b, p)? as u64 | (u32at(b, p + 4)? as u64) << 32)
}
fn index(v: u64) -> Result<usize, Error> {
    usize::try_from(v).map_err(|_| Error::Invalid)
}
// escaped: an OPC package's names, whose '%' starts a percent-escape (opc checks them)
fn name_ok(s: &str, escaped: bool) -> bool {
    !s.is_empty()
        && s.len() <= 1024
        && !s.contains(['\\', ':'])
        && (escaped || !s.contains('%'))
        && !s.chars().any(char::is_control)
        && s.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}
/// Read stored/DEFLATE ZIP files, ZIP64 included. Reject encryption, multi-disk archives,
/// ambiguous names (absolute, `..`, `\`, `:`, `%`, control characters, a case-insensitive
/// duplicate), duplicate parts, overlapping local records, data between or after the parts,
/// sizes a DEFLATE stream cannot reach and CRC mismatches.
pub fn read(b: &[u8], limits: Limits) -> Result<Vec<Entry>, Error> {
    read_parts(b, limits, false).map(|(e, _)| e)
}
// The ZIP64 end of central directory (locator right before the classic end record, record right
// before the locator): (entries, central directory size, its offset, the record's offset).
// None when there is no such locator or it does not lead to a well-formed record.
fn zip64_end(b: &[u8], end: usize) -> Option<(u64, u64, u64, usize)> {
    let loc = end.checked_sub(20)?;
    if u32at(b, loc).ok()? != 0x07064b50 || u32at(b, loc + 4).ok()? != 0 || u32at(b, loc + 16).ok()? > 1 {
        return None;
    }
    let z = usize::try_from(u64at(b, loc + 8).ok()?).ok()?;
    let size = usize::try_from(u64at(b, z + 4).ok()?).ok()?;
    if u32at(b, z).ok()? != 0x06064b50 || size < 44 || z.checked_add(12)?.checked_add(size)? != loc {
        return None;
    }
    Some((u64at(b, z + 32).ok()?, u64at(b, z + 40).ok()?, u64at(b, z + 48).ok()?, z))
}
// the ZIP64 extended information field (0x0001) in the extra block x
fn zip64_extra(x: &[u8]) -> Result<Option<&[u8]>, Error> {
    let mut p = 0;
    while p < x.len() {
        let id = u16at(x, p)?;
        let n = u16at(x, p + 2)? as usize;
        let data = range(x, p + 4, n)?;
        if id == 1 {
            return Ok(Some(data));
        }
        p += 4 + n;
    }
    Ok(None)
}
// read, with each part's DOS time and date (time in the low 16 bits)
fn read_parts(b: &[u8], limits: Limits, escaped: bool) -> Result<(Vec<Entry>, Vec<u32>), Error> {
    if b.len() > limits.archive_bytes {
        return Err(Error::Limit);
    }
    if b.len() < 22 {
        return Err(Error::Invalid);
    }
    let end = (b.len().saturating_sub(65557)..=b.len() - 22)
        .rev()
        .find(|&p| {
            b[p..].starts_with(b"PK\x05\x06")
                && u16at(b, p + 20).is_ok_and(|n| p + 22 + n as usize == b.len())
        })
        .ok_or(Error::Invalid)?;
    let disks = (u16at(b, end + 4)?, u16at(b, end + 6)?);
    let count16 = u16at(b, end + 10)?;
    let (cdsize32, cd32) = (u32at(b, end + 12)?, u32at(b, end + 16)?);
    let marked = disks.0 == M16 || disks.1 == M16 || count16 == M16 || cdsize32 == M32 || cd32 == M32;
    // the central directory: entries, its size and offset, where it ends
    let (count, cdsize, cd, cd_end) = match zip64_end(b, end) {
        Some((count, cdsize, cd, z)) => {
            let z_disks = (u32at(b, z + 16)?, u32at(b, z + 20)?);
            if z_disks != (0, 0) || u64at(b, z + 24)? != count || u16at(b, z + 14)? > V45 {
                return Err(Error::Unsupported);
            }
            // each classic field is the ZIP64 value or the mark that points to it
            if (disks.0 != 0 && disks.0 != M16)
                || (disks.1 != 0 && disks.1 != M16)
                || (count16 != M16 && count16 as u64 != count)
                || (u16at(b, end + 8)? != M16 && u16at(b, end + 8)? as u64 != count)
                || (cdsize32 != M32 && cdsize32 as u64 != cdsize)
                || (cd32 != M32 && cd32 as u64 != cd)
            {
                return Err(Error::Invalid);
            }
            if count > limits.entries as u64 {
                return Err(Error::Limit);
            }
            (count as usize, index(cdsize)?, index(cd)?, z)
        }
        None => {
            if disks != (0, 0) {
                return Err(Error::Unsupported);
            }
            if count16 == M16 || u16at(b, end + 8)? != count16 {
                return Err(Error::Unsupported);
            }
            if count16 as usize > limits.entries {
                return Err(Error::Limit);
            }
            if marked {
                return Err(Error::Invalid);
            }
            (count16 as usize, cdsize32 as usize, cd32 as usize, end)
        }
    };
    if cd.checked_add(cdsize) != Some(cd_end) {
        return Err(Error::Invalid);
    }
    let mut p = cd;
    let mut total = 0u64;
    let mut entries: Vec<Entry> = Vec::new();
    let mut times: Vec<u32> = Vec::new();
    // the records so far, start -> end (they never overlap), and the names folded for case
    let mut spans: BTreeMap<usize, usize> = BTreeMap::new();
    let mut folded: BTreeSet<String> = BTreeSet::new();
    for _ in 0..count {
        if u32at(b, p)? != 0x02014b50 {
            return Err(Error::Invalid);
        }
        range(b, p, 46)?;
        let flags = u16at(b, p + 8)?;
        let method = u16at(b, p + 10)?;
        let disk = u16at(b, p + 34)?;
        if flags & !0x080e != 0
            || !matches!(method, 0 | 8)
            || (disk != 0 && disk != M16)
            || u16at(b, p + 6)? > V45
        {
            return Err(Error::Unsupported);
        }
        let crc = u32at(b, p + 16)?;
        let mut packed = u32at(b, p + 20)? as u64;
        let mut size = u32at(b, p + 24)? as u64;
        let mut local = u32at(b, p + 42)? as u64;
        let nl = u16at(b, p + 28)? as usize;
        let xl = u16at(b, p + 30)? as usize;
        let cl = u16at(b, p + 32)? as usize;
        let sizes_marked = size == M32 as u64 || packed == M32 as u64 || local == M32 as u64;
        if sizes_marked || disk == M16 {
            // the marked fields follow in the ZIP64 field in this order (a disk number alone
            // without one: another disk, as before ZIP64)
            let missing = if sizes_marked { Error::Invalid } else { Error::Unsupported };
            let x = range(b, p + 46 + nl, xl)
                .ok()
                .and_then(|x| zip64_extra(x).ok().flatten())
                .ok_or(missing)?;
            let mut at = 0;
            for v in [&mut size, &mut packed, &mut local] {
                if *v == M32 as u64 {
                    *v = u64at(x, at)?;
                    at += 8;
                }
            }
            if disk == M16 && u32at(x, at).map_err(|_| Error::Unsupported)? != 0 {
                return Err(Error::Unsupported);
            }
        }
        total = total.checked_add(size).ok_or(Error::Limit)?;
        if size > limits.part_bytes as u64 || total > limits.total_bytes as u64 {
            return Err(Error::Limit);
        }
        let (packed, size, local) = (index(packed)?, size as usize, index(local)?);
        let raw = range(b, p + 46, nl)?;
        if flags & 0x800 == 0 && !raw.is_ascii() {
            return Err(Error::Unsupported);
        }
        let name = core::str::from_utf8(raw).map_err(|_| Error::Invalid)?;
        if !name_ok(name, escaped) || !folded.insert(fold(name)) {
            return Err(Error::Invalid);
        }
        if u32at(b, local)? != 0x04034b50
            || u16at(b, local + 6)? != flags
            || u16at(b, local + 8)? != method
        {
            return Err(Error::Invalid);
        }
        if u16at(b, local + 4)? > V45 {
            return Err(Error::Unsupported);
        }
        let ln = u16at(b, local + 26)? as usize;
        let lx = u16at(b, local + 28)? as usize;
        if range(b, local + 30, ln)? != raw {
            return Err(Error::Invalid);
        }
        let start = local.checked_add(30 + ln + lx).ok_or(Error::Invalid)?;
        let payload_end = start.checked_add(packed).ok_or(Error::Invalid)?;
        let mut stop = payload_end;
        let (lpacked, lsize) = (u32at(b, local + 18)?, u32at(b, local + 22)?);
        // a ZIP64 field in the local header: its sizes (and those of a data descriptor) are 64-bit
        let local64 = if flags & 8 == 0 && lpacked != M32 && lsize != M32 {
            None
        } else {
            zip64_extra(range(b, local + 30 + ln, lx)?).ok().flatten()
        };
        if flags & 8 == 0 {
            let (lpacked, lsize) = match local64 {
                Some(x) => (
                    if lpacked == M32 { u64at(x, 8)? } else { lpacked as u64 },
                    if lsize == M32 { u64at(x, 0)? } else { lsize as u64 },
                ),
                None => (lpacked as u64, lsize as u64),
            };
            if u32at(b, local + 14)? != crc || lpacked != packed as u64 || lsize != size as u64 {
                return Err(Error::Invalid);
            }
        } else {
            let d = if u32at(b, stop)? == 0x08074b50 {
                stop + 4
            } else {
                stop
            };
            if u32at(b, d)? != crc {
                return Err(Error::Invalid);
            }
            // the width the local header announces first, the other one if that does not match
            let wide = u64at(b, d + 4).ok() == Some(packed as u64) && u64at(b, d + 12).ok() == Some(size as u64);
            let narrow = u32at(b, d + 4).ok() == Some(packed as u32)
                && u32at(b, d + 8).ok() == Some(size as u32)
                && packed <= M32 as usize
                && size <= M32 as usize;
            stop = match (local64.is_some(), wide, narrow) {
                (true, true, _) | (false, true, false) => d + 20,
                (_, _, true) => d + 12,
                _ => return Err(Error::Invalid),
            };
        }
        // (of records that do not overlap, the last one starting before `stop` ends last)
        if stop > cd || spans.range(..stop).next_back().is_some_and(|(_, &z)| local < z) {
            return Err(Error::Invalid);
        }
        spans.insert(local, stop);
        let src = range(b, start, packed)?;
        let data = if method == 0 {
            if packed != size {
                return Err(Error::Invalid);
            }
            src.to_vec()
        } else {
            // no DEFLATE stream expands more than MAX_RATIO times: refuse before allocating
            if size as u64 > (packed as u64).saturating_mul(MAX_RATIO as u64) {
                return Err(Error::Invalid);
            }
            if size as u64 > RATIO_FLOOR && size as u64 > (packed as u64).saturating_mul(limits.ratio as u64) {
                return Err(Error::Limit);
            }
            let mut out = vec![0; size];
            let (written, used) = inflate_raw_ex(src, &mut out).map_err(|_| Error::Invalid)?;
            if written != size || used != packed {
                return Err(Error::Invalid);
            }
            out
        };
        if crc32(0, &data) != crc {
            return Err(Error::Checksum);
        }
        entries.push(Entry {
            name: String::from(name),
            data,
        });
        times.push(u32at(b, p + 12)?);
        p = p.checked_add(46 + nl + xl + cl).ok_or(Error::Invalid)?;
        if p > cd_end {
            return Err(Error::Invalid);
        }
    }
    if p != cd_end {
        return Err(Error::Invalid);
    }
    let mut previous = 0;
    for (a, z) in spans {
        if a != previous {
            return Err(Error::Unsupported);
        }
        previous = z;
    }
    if previous != cd {
        return Err(Error::Invalid);
    }
    Ok((entries, times))
}
// names that collide on a case-insensitive file system fold to the same (Unicode lower-casing)
fn fold(name: &str) -> String {
    name.to_lowercase()
}
// a DOS time and date (time in the low 16 bits) -> ns since 1970, read as UTC; 0 when invalid
fn dos_ns(td: u32) -> i64 {
    let (t, d) = ((td & 0xFFFF) as i64, (td >> 16) as i64);
    let (y, m, day) = (1980 + (d >> 9), (d >> 5) & 15, d & 31);
    let leap = (1980 + (d >> 9)) % 4 == 0;
    let mdays = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1..=12).contains(&m) || day == 0 || day > mdays[(m - 1) as usize] || (t >> 11) > 23 || ((t >> 5) & 63) > 59 || (t & 31) > 29 {
        return 0;
    }
    // days since 1970-01-01 of a civil date (Howard Hinnant's algorithm)
    let y = if m <= 2 { y - 1 } else { y };
    let (era, yoe) = (y.div_euclid(400), y.rem_euclid(400));
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    (days * 86_400 + (t >> 11) * 3600 + ((t >> 5) & 63) * 60 + (t & 31) * 2) * 1_000_000_000
}
fn put16(b: &mut Vec<u8>, n: u16) {
    b.extend_from_slice(&n.to_le_bytes());
}
fn put32(b: &mut Vec<u8>, n: u32) {
    b.extend_from_slice(&n.to_le_bytes());
}
fn put64(b: &mut Vec<u8>, n: u64) {
    b.extend_from_slice(&n.to_le_bytes());
}
/// Deterministic stored ZIP writer (UTF-8 names, fixed DOS date 1980-01-01): the same parts
/// always give the same bytes. ZIP64 records only where an archive needs them (65535 parts or
/// more, a size or offset of 4 GiB or more).
pub fn write(entries: &[Entry], limits: Limits) -> Result<Vec<u8>, Error> {
    write_parts(&pairs(entries), limits, Method::Stored, false, M32 as u64)
}
/// `write` with DEFLATE when `method` asks for it; just as deterministic.
pub fn write_with(entries: &[Entry], limits: Limits, method: Method) -> Result<Vec<u8>, Error> {
    write_parts(&pairs(entries), limits, method, false, M32 as u64)
}
/// For tests: `write_with` that writes ZIP64 records for every size and offset from `at` bytes
/// on, as if `at` were 4 GiB.
#[doc(hidden)]
pub fn write_zip64_from(entries: &[Entry], limits: Limits, method: Method, at: u64) -> Result<Vec<u8>, Error> {
    write_parts(&pairs(entries), limits, method, false, at.min(M32 as u64))
}
fn pairs(entries: &[Entry]) -> Vec<(&str, &[u8])> {
    entries.iter().map(|e| (e.name.as_str(), e.data.as_slice())).collect()
}
// one part's name and bytes while it is written
struct Part<'a> {
    name: &'a str,
    data: &'a [u8],
}
// escaped: an OPC package's names (percent-escapes allowed); at: where ZIP64 starts
pub(crate) fn write_parts(list: &[(&str, &[u8])], limits: Limits, method: Method, escaped: bool, at: u64) -> Result<Vec<u8>, Error> {
    let entries: Vec<Part> = list.iter().map(|&(name, data)| Part { name, data }).collect();
    if entries.len() > limits.entries {
        return Err(Error::Limit);
    }
    let mut total = 0usize;
    let mut folded = BTreeSet::new();
    for e in &entries {
        if !name_ok(e.name, escaped) || !folded.insert(fold(e.name)) {
            return Err(Error::Invalid);
        }
        if e.data.len() > limits.part_bytes {
            return Err(Error::Limit);
        }
        total = total.checked_add(e.data.len()).ok_or(Error::Limit)?;
    }
    if total > limits.total_bytes {
        return Err(Error::Limit);
    }
    // each part's method and stored bytes (DEFLATE only where it makes the part smaller)
    let packed: Vec<(u16, Option<Vec<u8>>)> = entries
        .iter()
        .map(|e| match method {
            Method::Deflate(level) => {
                let z = deflate_raw(e.data, level.clamp(1, 9) as i32);
                // (an empty result would be a failed compression, never a smaller part)
                if !z.is_empty() && z.len() < e.data.len() { (8, Some(z)) } else { (0, None) }
            }
            Method::Stored => (0, None),
        })
        .collect();
    let body = |i: usize| packed[i].1.as_deref().unwrap_or(entries[i].data);
    // the layout: offsets, which records take ZIP64 fields, the archive's size
    let mut offsets = Vec::with_capacity(entries.len());
    let mut pos = 0u64;
    for (i, e) in entries.iter().enumerate() {
        offsets.push(pos);
        let wide = e.data.len() as u64 >= at || body(i).len() as u64 >= at;
        pos += 30 + e.name.len() as u64 + if wide { 20 } else { 0 } + body(i).len() as u64;
    }
    let cd = pos;
    for (i, e) in entries.iter().enumerate() {
        let n = [e.data.len() as u64, body(i).len() as u64, offsets[i]].iter().filter(|&&v| v >= at).count() as u64;
        pos += 46 + e.name.len() as u64 + if n > 0 { 4 + 8 * n } else { 0 };
    }
    let cdsize = pos - cd;
    let end64 = entries.len() >= M16 as usize || cdsize >= at || cd >= at;
    pos += if end64 { 56 + 20 } else { 0 } + 22;
    if pos > limits.archive_bytes as u64 {
        return Err(Error::Limit);
    }
    let mut b = Vec::with_capacity(usize::try_from(pos).map_err(|_| Error::Limit)?);
    let mut crcs = Vec::with_capacity(entries.len());
    for (i, e) in entries.iter().enumerate() {
        let crc = crc32(0, e.data);
        let (size, stored) = (e.data.len() as u64, body(i).len() as u64);
        let wide = size >= at || stored >= at;
        put32(&mut b, 0x04034b50);
        put16(&mut b, if wide { V45 } else { V20 });
        put16(&mut b, 0x800);
        put16(&mut b, packed[i].0);
        put16(&mut b, 0);
        put16(&mut b, 33);
        put32(&mut b, crc);
        put32(&mut b, if wide { M32 } else { stored as u32 });
        put32(&mut b, if wide { M32 } else { size as u32 });
        put16(&mut b, e.name.len() as u16);
        put16(&mut b, if wide { 20 } else { 0 });
        b.extend_from_slice(e.name.as_bytes());
        if wide {
            put16(&mut b, 1);
            put16(&mut b, 16);
            put64(&mut b, size);
            put64(&mut b, stored);
        }
        b.extend_from_slice(body(i));
        crcs.push(crc);
    }
    for (i, e) in entries.iter().enumerate() {
        let fields = [e.data.len() as u64, body(i).len() as u64, offsets[i]];
        let marked: Vec<u64> = fields.iter().copied().filter(|&v| v >= at).collect();
        let short = |v: u64| if v >= at { M32 } else { v as u32 };
        let version = if marked.is_empty() { V20 } else { V45 };
        put32(&mut b, 0x02014b50);
        put16(&mut b, version);
        put16(&mut b, version);
        put16(&mut b, 0x800);
        put16(&mut b, packed[i].0);
        put16(&mut b, 0);
        put16(&mut b, 33);
        put32(&mut b, crcs[i]);
        put32(&mut b, short(fields[1]));
        put32(&mut b, short(fields[0]));
        put16(&mut b, e.name.len() as u16);
        put16(&mut b, if marked.is_empty() { 0 } else { 4 + 8 * marked.len() as u16 });
        put16(&mut b, 0);
        put16(&mut b, 0);
        put16(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, short(fields[2]));
        b.extend_from_slice(e.name.as_bytes());
        if !marked.is_empty() {
            put16(&mut b, 1);
            put16(&mut b, 8 * marked.len() as u16);
            for v in marked {
                put64(&mut b, v);
            }
        }
    }
    let count = entries.len() as u64;
    if end64 {
        let z = b.len() as u64;
        put32(&mut b, 0x06064b50);
        put64(&mut b, 44);
        put16(&mut b, V45);
        put16(&mut b, V45);
        put32(&mut b, 0);
        put32(&mut b, 0);
        put64(&mut b, count);
        put64(&mut b, count);
        put64(&mut b, cdsize);
        put64(&mut b, cd);
        put32(&mut b, 0x07064b50);
        put32(&mut b, 0);
        put64(&mut b, z);
        put32(&mut b, 1);
    }
    let count16 = if count >= M16 as u64 { M16 } else { count as u16 };
    put32(&mut b, 0x06054b50);
    put16(&mut b, 0);
    put16(&mut b, 0);
    put16(&mut b, count16);
    put16(&mut b, count16);
    put32(&mut b, if cdsize >= at { M32 } else { cdsize as u32 });
    put32(&mut b, if cd >= at { M32 } else { cd as u32 });
    put16(&mut b, 0);
    Ok(b)
}

/// A ZIP archive as a read-only plat::vfs provider (mount it under a scheme of its own). The
/// parts are checked and decompressed once, within `limits`.
pub struct ZipFs(plat::vfs::MemFs);

impl ZipFs {
    /// Read every part of the archive; Invalid also for names that collide as file and folder.
    pub fn new(b: &[u8], limits: Limits) -> Result<ZipFs, Error> {
        use plat::vfs::Provider;
        let fs = plat::vfs::MemFs::new();
        let (entries, times) = read_parts(b, limits, false)?;
        for (e, td) in entries.iter().zip(times) {
            let t = dos_ns(td);
            if !fs.write(&e.name, &e.data) || !fs.set_times(&e.name, t, t) {
                return Err(Error::Invalid);
            }
        }
        Ok(ZipFs(fs))
    }
}

impl plat::vfs::Provider for ZipFs {
    fn kind(&self, path: &str) -> u32 {
        self.0.kind(path)
    }
    fn read(&self, path: &str) -> Option<Vec<u8>> {
        self.0.read(path)
    }
    fn list(&self, path: &str, f: &mut dyn FnMut(&str, bool, u64)) -> bool {
        self.0.list(path, f)
    }
    fn stat(&self, path: &str) -> Option<plat::vfs::Stat> {
        self.0.stat(path)
    }
    fn open(&self, path: &str, mode: u32) -> plat::vfs::Opened {
        if mode == plat::FO_READ { self.0.open(path, mode) } else { plat::vfs::Opened::Refused }
    }
}
