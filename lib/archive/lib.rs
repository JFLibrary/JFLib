// In-memory ZIP32 interchange. New native library code, independent of document/UI code.
// Raw DEFLATE and CRC reuse the existing safe codec primitives; no paths are extracted.
// ZipFs shows an archive as a read-only plat::vfs tree:
//   plat::vfs::mount("pack", Arc::new(archive::ZipFs::new(&bytes, Limits::default())?))
//   -> plat::read_file("pack://textures/a.png"), plat::File::open("pack://..", FO_READ),
//   plat::fs::stat (size; mtime: the entry's DOS time, read as UTC)
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
use alloc::{string::String, vec, vec::Vec};
use img_core::{checksum::crc32, inflate::inflate_raw_ex};

/// One decoded archive part with a normalized, relative UTF-8 name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub data: Vec<u8>,
}
/// Bounds are checked before decompression or allocation of advertised sizes.
#[derive(Clone, Copy)]
pub struct Limits {
    pub entries: usize,
    pub part_bytes: usize,
    pub total_bytes: usize,
    pub archive_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            entries: 512,
            part_bytes: 8 << 20,
            total_bytes: 32 << 20,
            archive_bytes: 32 << 20,
        }
    }
}
/// Structural, integrity, feature or resource-limit failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Unsupported,
    Checksum,
    Limit,
}
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
fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 1024
        && !s.contains(['\\', ':', '%'])
        && !s.chars().any(char::is_control)
        && s.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}
/// Read stored/DEFLATE ZIP32 files. Reject encryption, multi-disk/ZIP64, ambiguous names,
/// duplicate parts, overlapping local records, trailing payloads and CRC mismatches.
pub fn read(b: &[u8], limits: Limits) -> Result<Vec<Entry>, Error> {
    read_parts(b, limits).map(|(e, _)| e)
}
// read, with each part's DOS time and date (time in the low 16 bits)
fn read_parts(b: &[u8], limits: Limits) -> Result<(Vec<Entry>, Vec<u32>), Error> {
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
    if u16at(b, end + 4)? != 0 || u16at(b, end + 6)? != 0 {
        return Err(Error::Unsupported);
    }
    let count = u16at(b, end + 10)? as usize;
    if count == 65535 || u16at(b, end + 8)? as usize != count {
        return Err(Error::Unsupported);
    }
    if count > limits.entries {
        return Err(Error::Limit);
    }
    let cdsize = u32at(b, end + 12)? as usize;
    let cd = u32at(b, end + 16)? as usize;
    if cd.checked_add(cdsize) != Some(end) {
        return Err(Error::Invalid);
    }
    let mut p = cd;
    let mut total = 0usize;
    let mut entries: Vec<Entry> = Vec::new();
    let mut times: Vec<u32> = Vec::new();
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for _ in 0..count {
        if u32at(b, p)? != 0x02014b50 {
            return Err(Error::Invalid);
        }
        range(b, p, 46)?;
        let flags = u16at(b, p + 8)?;
        let method = u16at(b, p + 10)?;
        if flags & !0x080e != 0
            || !matches!(method, 0 | 8)
            || u16at(b, p + 34)? != 0
            || u16at(b, p + 6)? > 20
        {
            return Err(Error::Unsupported);
        }
        let crc = u32at(b, p + 16)?;
        let packed = u32at(b, p + 20)? as usize;
        let size = u32at(b, p + 24)? as usize;
        total = total.checked_add(size).ok_or(Error::Limit)?;
        if size > limits.part_bytes || total > limits.total_bytes {
            return Err(Error::Limit);
        }
        let nl = u16at(b, p + 28)? as usize;
        let xl = u16at(b, p + 30)? as usize;
        let cl = u16at(b, p + 32)? as usize;
        let raw = range(b, p + 46, nl)?;
        if flags & 0x800 == 0 && !raw.is_ascii() {
            return Err(Error::Unsupported);
        }
        let name = core::str::from_utf8(raw).map_err(|_| Error::Invalid)?;
        if !name_ok(name) || entries.iter().any(|e| e.name.eq_ignore_ascii_case(name)) {
            return Err(Error::Invalid);
        }
        let local = u32at(b, p + 42)? as usize;
        if u32at(b, local)? != 0x04034b50
            || u16at(b, local + 6)? != flags
            || u16at(b, local + 8)? != method
        {
            return Err(Error::Invalid);
        }
        if u16at(b, local + 4)? > 20 {
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
        if flags & 8 == 0 {
            if u32at(b, local + 14)? != crc
                || u32at(b, local + 18)? as usize != packed
                || u32at(b, local + 22)? as usize != size
            {
                return Err(Error::Invalid);
            }
        } else {
            let d = if u32at(b, stop)? == 0x08074b50 {
                stop + 4
            } else {
                stop
            };
            if u32at(b, d)? != crc
                || u32at(b, d + 4)? as usize != packed
                || u32at(b, d + 8)? as usize != size
            {
                return Err(Error::Invalid);
            }
            stop = d + 12;
        }
        if stop > cd || spans.iter().any(|&(a, z)| local < z && stop > a) {
            return Err(Error::Invalid);
        }
        spans.push((local, stop));
        let src = range(b, start, packed)?;
        let data = if method == 0 {
            if packed != size {
                return Err(Error::Invalid);
            }
            src.to_vec()
        } else {
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
        if p > end {
            return Err(Error::Invalid);
        }
    }
    if p != end {
        return Err(Error::Invalid);
    }
    spans.sort_unstable();
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
// a DOS time and date (time in the low 16 bits) -> ns since 1970, read as UTC; 0 when invalid
fn dos_ns(td: u32) -> i64 {
    let (t, d) = ((td & 0xFFFF) as i64, (td >> 16) as i64);
    let (y, m, day) = (1980 + (d >> 9), (d >> 5) & 15, d & 31);
    if !(1..=12).contains(&m) || day == 0 {
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
/// Deterministic stored ZIP32 writer (UTF-8 names, fixed DOS date 1980-01-01).
pub fn write(entries: &[Entry], limits: Limits) -> Result<Vec<u8>, Error> {
    if entries.len() >= 65535 || entries.len() > limits.entries {
        return Err(Error::Limit);
    }
    let mut total = 0usize;
    let mut expected = 22usize;
    for (i, e) in entries.iter().enumerate() {
        if !name_ok(&e.name)
            || entries[..i]
                .iter()
                .any(|a| a.name.eq_ignore_ascii_case(&e.name))
        {
            return Err(Error::Invalid);
        }
        if e.data.len() > limits.part_bytes || e.data.len() >= u32::MAX as usize {
            return Err(Error::Limit);
        }
        total = total.checked_add(e.data.len()).ok_or(Error::Limit)?;
        expected = expected
            .checked_add(76 + 2 * e.name.len() + e.data.len())
            .ok_or(Error::Limit)?;
    }
    if total > limits.total_bytes
        || expected > limits.archive_bytes
        || expected >= u32::MAX as usize
    {
        return Err(Error::Limit);
    }
    let mut b = Vec::with_capacity(expected);
    let mut records = Vec::new();
    for e in entries {
        let offset = b.len() as u32;
        let crc = crc32(0, &e.data);
        let size = e.data.len() as u32;
        put32(&mut b, 0x04034b50);
        put16(&mut b, 20);
        put16(&mut b, 0x800);
        put16(&mut b, 0);
        put16(&mut b, 0);
        put16(&mut b, 33);
        put32(&mut b, crc);
        put32(&mut b, size);
        put32(&mut b, size);
        put16(&mut b, e.name.len() as u16);
        put16(&mut b, 0);
        b.extend_from_slice(e.name.as_bytes());
        b.extend_from_slice(&e.data);
        records.push((offset, crc));
    }
    let cd = b.len() as u32;
    for (e, (offset, crc)) in entries.iter().zip(records) {
        put32(&mut b, 0x02014b50);
        put16(&mut b, 20);
        put16(&mut b, 20);
        put16(&mut b, 0x800);
        put16(&mut b, 0);
        put16(&mut b, 0);
        put16(&mut b, 33);
        put32(&mut b, crc);
        put32(&mut b, e.data.len() as u32);
        put32(&mut b, e.data.len() as u32);
        put16(&mut b, e.name.len() as u16);
        put16(&mut b, 0);
        put16(&mut b, 0);
        put16(&mut b, 0);
        put16(&mut b, 0);
        put32(&mut b, 0);
        put32(&mut b, offset);
        b.extend_from_slice(e.name.as_bytes());
    }
    let cdsize = b.len() as u32 - cd;
    put32(&mut b, 0x06054b50);
    put16(&mut b, 0);
    put16(&mut b, 0);
    put16(&mut b, entries.len() as u16);
    put16(&mut b, entries.len() as u16);
    put32(&mut b, cdsize);
    put32(&mut b, cd);
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
        let (entries, times) = read_parts(b, limits)?;
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
