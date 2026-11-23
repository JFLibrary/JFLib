// Compound files ("OLE", "structured storage") as Microsoft's open specification [MS-CFB]
// describes them: a file system of storages and streams inside one file, the container of
// Office's binary formats (Visio's .vss/.vsd, Word's .doc, Outlook's .msg, the VBA project of
// a macro document). New native library code from the specification; read only.
//
//   cfb::File::read(&bytes, cfb::Limits::default())? -> the directory, checked
//   file.entries() / file.children(i) / file.find(&["VisioDocument"]) -> entries by path
//   file.stream(i)? -> the bytes of a stream
//
// Every file is untrusted input. Everything the header and the directory claim is checked
// before it is believed: the file's and each stream's size against Limits, every sector
// number against the file, each chain of sectors walked at most once around (a loop is
// Invalid, never a hang), the directory's tree of siblings without cycles and with every entry
// at most once, names as the specification allows them. Nothing is ever written.

use crate::Error;
use alloc::{string::String, vec, vec::Vec};

/// The first eight bytes of every compound file.
pub const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

// special sector numbers
const MAXREG: u32 = 0xFFFF_FFFA;
const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
const FREESECT: u32 = 0xFFFF_FFFF;
const NOSTREAM: u32 = 0xFFFF_FFFF;
const HEADER: usize = 512;

/// Bounds of a compound file, checked before anything is allocated. The default (64 MiB a
/// file and a stream, 4096 directory entries) suits documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Most bytes of the file itself.
    pub file_bytes: usize,
    /// Most bytes of one stream.
    pub stream_bytes: usize,
    /// Most directory entries (storages, streams and unused slots together).
    pub entries: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self { file_bytes: 64 << 20, stream_bytes: 64 << 20, entries: 4096 }
    }
}

/// What a directory entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The root storage (entry 0); its stream is the mini stream.
    Root,
    /// A storage: a directory of further entries.
    Storage,
    /// A stream of bytes.
    Stream,
}

/// One storage or stream of the directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The name (at most 31 UTF-16 code units, without "/", "\", ":" and "!").
    pub name: String,
    pub kind: Kind,
    /// The class id of a storage (zeros for a stream).
    pub clsid: [u8; 16],
    /// The size of a stream in bytes.
    pub size: u64,
    start: u32,
    left: u32,
    right: u32,
    child: u32,
}

/// A compound file read into memory: its directory and the tables to find a stream's bytes.
#[derive(Clone, Debug)]
pub struct File<'a> {
    bytes: &'a [u8],
    sector: usize,
    fat: Vec<u32>,
    minifat: Vec<u32>,
    mini: Vec<u8>,
    cutoff: u64,
    sectors: usize,
    entries: Vec<Entry>,
    // the children of each storage, in directory order (the tree's in-order walk)
    kids: Vec<Vec<usize>>,
    limits: Limits,
}

fn u16at(b: &[u8], p: usize) -> Result<u16, Error> {
    b.get(p..p + 2).map(|x| u16::from_le_bytes([x[0], x[1]])).ok_or(Error::Invalid)
}
fn u32at(b: &[u8], p: usize) -> Result<u32, Error> {
    b.get(p..p + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]])).ok_or(Error::Invalid)
}

/// Whether `bytes` start like a compound file (only the signature is looked at).
pub fn sniff(bytes: &[u8]) -> bool {
    bytes.starts_with(&SIGNATURE)
}

impl<'a> File<'a> {
    /// Read the header, the allocation tables and the directory of a compound file, version 3
    /// (512-byte sectors) or 4 (4096-byte sectors). Refused with `Invalid`: a wrong signature,
    /// byte order or sector size, a sector number outside the file, a chain that loops or ends
    /// early, a directory tree with a cycle, an entry reached twice or a malformed name;
    /// `Unsupported`: another major version; `Limit`: the file, a stream or the directory over
    /// `limits`.
    pub fn read(bytes: &'a [u8], limits: Limits) -> Result<File<'a>, Error> {
        if bytes.len() > limits.file_bytes {
            return Err(Error::Limit);
        }
        if bytes.len() < HEADER || !sniff(bytes) {
            return Err(Error::Invalid);
        }
        let major = u16at(bytes, 0x1A)?;
        let shift = u16at(bytes, 0x1E)?;
        if u16at(bytes, 0x1C)? != 0xFFFE {
            return Err(Error::Invalid);
        }
        match (major, shift) {
            (3, 9) | (4, 12) => {}
            (3 | 4, _) => return Err(Error::Invalid),
            _ => return Err(Error::Unsupported),
        }
        if u16at(bytes, 0x20)? != 6 {
            return Err(Error::Invalid);
        }
        let sector = 1usize << shift;
        let cutoff = u32at(bytes, 0x38)? as u64;
        if cutoff != 4096 {
            return Err(Error::Invalid);
        }
        // sector n starts at (n + 1) * sector (version 4 pads the header to a whole sector); only
        // whole sectors count
        let sectors = bytes.len().saturating_sub(sector) / sector;
        let nfat = u32at(bytes, 0x2C)? as usize;
        let dir_start = u32at(bytes, 0x30)?;
        let minifat_start = u32at(bytes, 0x3C)?;
        let nminifat = u32at(bytes, 0x40)? as usize;
        let difat_start = u32at(bytes, 0x44)?;
        let ndifat = u32at(bytes, 0x48)? as usize;
        let per = sector / 4;
        if nfat > sectors || ndifat > sectors || nminifat > sectors {
            return Err(Error::Invalid);
        }
        let mut f = File {
            bytes,
            sector,
            fat: Vec::new(),
            minifat: Vec::new(),
            mini: Vec::new(),
            cutoff,
            sectors,
            entries: Vec::new(),
            kids: Vec::new(),
            limits,
        };
        // the DIFAT: 109 entries in the header, the rest in a chain of its own sectors
        let mut difat: Vec<u32> = Vec::with_capacity(nfat);
        for i in 0..109 {
            difat.push(u32at(bytes, 0x4C + 4 * i)?);
        }
        let mut d = difat_start;
        for _ in 0..ndifat {
            if difat.len() >= nfat {
                break;
            }
            let s = f.sector_bytes(d)?;
            for i in 0..per - 1 {
                difat.push(u32at(s, 4 * i)?);
            }
            d = u32at(s, sector - 4)?;
        }
        if difat.len() < nfat {
            return Err(Error::Invalid);
        }
        let mut fat = Vec::with_capacity(nfat.saturating_mul(per).min(sectors.saturating_add(per)));
        for &n in &difat[..nfat] {
            let s = f.sector_bytes(n)?;
            for i in 0..per {
                fat.push(u32at(s, 4 * i)?);
            }
        }
        f.fat = fat;
        // the directory: a chain of sectors of 128-byte entries
        let dir = f.chain_bytes(dir_start, None)?;
        let count = dir.len() / 128;
        if count == 0 {
            return Err(Error::Invalid);
        }
        let mut entries = Vec::new();
        for i in 0..count {
            let e = &dir[i * 128..(i + 1) * 128];
            let kind = match e[66] {
                0 => None,
                1 => Some(Kind::Storage),
                2 => Some(Kind::Stream),
                5 if i == 0 => Some(Kind::Root),
                _ => return Err(Error::Invalid),
            };
            let Some(kind) = kind else {
                // an unused slot: kept so indices stay the file's, never linked
                entries.push(None);
                continue;
            };
            if i == 0 && kind != Kind::Root {
                return Err(Error::Invalid);
            }
            if entries.iter().flatten().count() >= limits.entries {
                return Err(Error::Limit);
            }
            let nlen = u16at(e, 64)? as usize;
            if nlen < 2 || nlen > 64 || nlen % 2 != 0 {
                return Err(Error::Invalid);
            }
            let units: Vec<u16> = (0..nlen / 2 - 1).map(|k| u16::from_le_bytes([e[2 * k], e[2 * k + 1]])).collect();
            if u16at(e, nlen - 2)? != 0 {
                return Err(Error::Invalid);
            }
            let name = String::from_utf16(&units).map_err(|_| Error::Invalid)?;
            if name.is_empty() || name.contains(['/', '\\', ':', '!']) {
                return Err(Error::Invalid);
            }
            let mut clsid = [0u8; 16];
            clsid.copy_from_slice(&e[80..96]);
            let start = u32at(e, 116)?;
            let lo = u32at(e, 120)? as u64;
            let hi = u32at(e, 124)? as u64;
            // version 3: the high half is to be ignored (older writers left garbage there)
            let size = if major == 3 { lo } else { lo | hi << 32 };
            if kind == Kind::Storage && size != 0 && major == 4 {
                return Err(Error::Invalid);
            }
            entries.push(Some(Entry {
                name,
                kind,
                clsid,
                size: if kind == Kind::Storage { 0 } else { size },
                start,
                left: u32at(e, 68)?,
                right: u32at(e, 72)?,
                child: u32at(e, 76)?,
            }));
        }
        // the tree: every entry reached at most once, from the root down
        let n = entries.len();
        let mut seen = vec![false; n];
        let mut kids = vec![Vec::new(); n];
        seen[0] = true;
        let mut storages = vec![0usize];
        while let Some(st) = storages.pop() {
            let Some(e) = &entries[st] else { return Err(Error::Invalid) };
            let first = e.child;
            if first == NOSTREAM {
                continue;
            }
            // in-order walk of the siblings' tree, with a stack of its own
            let mut stack: Vec<usize> = Vec::new();
            let mut cur = Some(first);
            let mut out = Vec::new();
            loop {
                while let Some(c) = cur {
                    let c = c as usize;
                    if c >= n || seen[c] {
                        return Err(Error::Invalid);
                    }
                    let Some(ce) = &entries[c] else { return Err(Error::Invalid) };
                    if ce.kind == Kind::Root {
                        return Err(Error::Invalid);
                    }
                    seen[c] = true;
                    stack.push(c);
                    cur = (ce.left != NOSTREAM).then_some(ce.left);
                }
                let Some(c) = stack.pop() else { break };
                out.push(c);
                let ce = entries[c].as_ref().ok_or(Error::Invalid)?;
                if ce.kind == Kind::Storage {
                    storages.push(c);
                } else if ce.child != NOSTREAM {
                    return Err(Error::Invalid);
                }
                cur = (ce.right != NOSTREAM).then_some(ce.right);
            }
            kids[st] = out;
        }
        f.entries = entries
            .into_iter()
            .map(|e| {
                e.unwrap_or(Entry {
                    name: String::new(),
                    kind: Kind::Stream,
                    clsid: [0; 16],
                    size: 0,
                    start: ENDOFCHAIN,
                    left: NOSTREAM,
                    right: NOSTREAM,
                    child: NOSTREAM,
                })
            })
            .collect();
        f.kids = kids;
        // unused slots and entries no storage reaches are hidden: they are no part of the tree
        for (i, s) in seen.iter().enumerate() {
            if !s {
                f.entries[i].name.clear();
            }
        }
        // the mini stream (the root's stream) and its allocation table
        let root_size = f.entries[0].size;
        if root_size > 0 {
            if root_size > limits.stream_bytes as u64 {
                return Err(Error::Limit);
            }
            f.mini = f.chain_bytes(f.entries[0].start, Some(root_size as usize))?;
        }
        if minifat_start != ENDOFCHAIN && minifat_start != FREESECT {
            let t = f.chain_bytes(minifat_start, None)?;
            f.minifat = t.chunks_exact(4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]])).collect();
        }
        Ok(f)
    }

    // the bytes of sector n (a cut last sector is cut here too)
    fn sector_bytes(&self, n: u32) -> Result<&'a [u8], Error> {
        if n > MAXREG || n as usize >= self.sectors {
            return Err(Error::Invalid);
        }
        let start = (n as usize + 1) * self.sector;
        self.bytes.get(start..start + self.sector).ok_or(Error::Invalid)
    }

    // the bytes of a chain of sectors from `start`, `size` of them (None: the whole chain, as
    // for the directory and the mini FAT); each sector at most once
    fn chain_bytes(&self, start: u32, size: Option<usize>) -> Result<Vec<u8>, Error> {
        let sectors = self.sectors;
        let need = size.map(|s| s.div_ceil(self.sector));
        if let Some(s) = size {
            if s > self.limits.stream_bytes {
                return Err(Error::Limit);
            }
        }
        let mut out = Vec::with_capacity(size.unwrap_or(0));
        let mut n = start;
        let mut hops = 0usize;
        while n != ENDOFCHAIN {
            if need.is_some_and(|k| hops >= k) {
                break;
            }
            hops += 1;
            if hops > sectors || hops > self.fat.len() {
                return Err(Error::Invalid);
            }
            if size.is_none() && out.len() + self.sector > self.limits.stream_bytes {
                return Err(Error::Limit);
            }
            out.extend_from_slice(self.sector_bytes(n)?);
            n = *self.fat.get(n as usize).ok_or(Error::Invalid)?;
            if n != ENDOFCHAIN && n > MAXREG {
                return Err(Error::Invalid);
            }
        }
        if let Some(s) = size {
            if out.len() < s {
                return Err(Error::Invalid);
            }
            out.truncate(s);
        }
        Ok(out)
    }

    /// All directory entries; index 0 is the root. Unused slots and entries no storage reaches
    /// have an empty name and are nobody's child.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The children of the storage at `index` (the root is 0), in the directory's order.
    pub fn children(&self, index: usize) -> &[usize] {
        self.kids.get(index).map_or(&[], |k| k.as_slice())
    }

    /// The entry at a path of names from the root (`&["VisioDocument"]`,
    /// `&["Macros", "VBA", "dir"]`); names compare as the specification says, upper case
    /// against upper case.
    pub fn find(&self, path: &[&str]) -> Option<usize> {
        let mut at = 0;
        for part in path {
            at = *self.children(at).iter().find(|&&c| same_name(&self.entries[c].name, part))?;
        }
        Some(at)
    }

    /// The bytes of the stream at `index`: `Invalid` for a storage or a chain that is too
    /// short, `Limit` for a stream over `Limits::stream_bytes`.
    pub fn stream(&self, index: usize) -> Result<Vec<u8>, Error> {
        let e = self.entries.get(index).ok_or(Error::Invalid)?;
        if e.kind != Kind::Stream || e.name.is_empty() {
            return Err(Error::Invalid);
        }
        if e.size > self.limits.stream_bytes as u64 {
            return Err(Error::Limit);
        }
        let size = e.size as usize;
        if size == 0 {
            return Ok(Vec::new());
        }
        if e.size >= self.cutoff {
            return self.chain_bytes(e.start, Some(size));
        }
        // a small stream lives in the mini stream, in 64-byte sectors
        let mut out = Vec::with_capacity(size);
        let mut n = e.start;
        let mut hops = 0usize;
        while out.len() < size {
            hops += 1;
            if n > MAXREG || hops > self.minifat.len() {
                return Err(Error::Invalid);
            }
            let at = (n as usize).checked_mul(64).ok_or(Error::Invalid)?;
            let s = self.mini.get(at..at.checked_add(64).ok_or(Error::Invalid)?).ok_or(Error::Invalid)?;
            out.extend_from_slice(&s[..64.min(size - out.len())]);
            n = *self.minifat.get(n as usize).ok_or(Error::Invalid)?;
        }
        Ok(out)
    }
}

// names compare by their upper case ([MS-CFB] 2.6.4: a simple upper-casing of each character)
fn same_name(a: &str, b: &str) -> bool {
    a.chars().flat_map(char::to_uppercase).eq(b.chars().flat_map(char::to_uppercase))
}
