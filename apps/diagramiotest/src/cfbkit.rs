// cfbkit -- (JD17) compound files ("OLE", [MS-CFB]) written for the tests only: the container of
// the binary .vss files, built from a tree of storages and streams, and the hostile variants of
// one (Hostile) patched into the written bytes.  Never used to write a real file.
//
//   Cfb { version: 3, kids: vec![stream("VisioDocument", doc)], ..Cfb::default() }.build() -> Built
//   built.bytes                      the file
//   Hostile::FatLoop.apply(&mut built)   a variant (false: the file has no place for it)
//
// What it writes: version 3 (512-byte sectors) or 4 (4096-byte sectors, the header padded to a
// sector); streams under 4096 bytes in the mini stream (64-byte sectors, the root's stream) with
// their mini FAT, the others in sectors of their own; the directory with each storage's children
// as a balanced tree of siblings (ordered as the specification orders names: shorter first, then
// by upper case), all black; the FAT after everything else, and a DIFAT when the FAT needs more
// than the header's 109 sectors (Cfb::min_fat makes one without a large file).  Built says where
// everything lies, so a test can change any field afterwards.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
pub const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
pub const FREESECT: u32 = 0xFFFF_FFFF;
pub const FATSECT: u32 = 0xFFFF_FFFD;
pub const DIFSECT: u32 = 0xFFFF_FFFC;
pub const NOSTREAM: u32 = 0xFFFF_FFFF;
/// streams below this many bytes live in the mini stream
pub const CUTOFF: usize = 4096;

/// a storage or a stream of the tree to write
#[derive(Clone, Debug)]
pub enum Node {
    Stream { name: String, data: Vec<u8> },
    Storage { name: String, kids: Vec<Node> },
}

pub fn stream(name: &str, data: &[u8]) -> Node {
    Node::Stream { name: String::from(name), data: data.to_vec() }
}

pub fn storage(name: &str, kids: Vec<Node>) -> Node {
    Node::Storage { name: String::from(name), kids }
}

/// the file to write
#[derive(Clone, Debug)]
pub struct Cfb {
    /// 3 (512-byte sectors) or 4 (4096-byte sectors)
    pub version: u16,
    /// the root storage's children
    pub kids: Vec<Node>,
    /// at least this many FAT sectors (over 109: a DIFAT, over 109 + 127: two DIFAT sectors)
    pub min_fat: usize,
    /// unused directory slots at the end (at least this many)
    pub spare_entries: usize,
}

impl Default for Cfb {
    fn default() -> Self {
        Cfb { version: 3, kids: Vec::new(), min_fat: 1, spare_entries: 1 }
    }
}

/// one directory entry as written
#[derive(Clone, Debug)]
pub struct Placed {
    pub name: String,
    /// 5 root, 1 storage, 2 stream
    pub kind: u8,
    /// the file offset of its 128 bytes
    pub at: usize,
    /// a large stream's sectors, the root's mini stream sectors
    pub sectors: Vec<u32>,
    /// a small stream's mini sectors
    pub mini: Vec<u32>,
    pub size: u64,
    /// the children of a storage (directory indices)
    pub kids: Vec<usize>,
}

/// a written file and where its parts lie
#[derive(Clone, Debug)]
pub struct Built {
    pub bytes: Vec<u8>,
    pub version: u16,
    /// the bytes of a sector
    pub sector: usize,
    /// every directory entry by index (0: the root); unused slots are not listed
    pub entries: Vec<Placed>,
    /// the directory's sectors
    pub dir: Vec<u32>,
    /// the FAT's sectors
    pub fat: Vec<u32>,
    /// the DIFAT's sectors
    pub difat: Vec<u32>,
    /// the mini FAT's sectors
    pub minifat: Vec<u32>,
    /// the index of the first unused directory slot
    pub unused: Option<usize>,
}

fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

// the order of siblings ([MS-CFB] 2.6.4): the shorter name first, then by upper case
fn name_key(n: &str) -> (usize, Vec<u16>) {
    let up: String = n.chars().flat_map(char::to_uppercase).collect();
    (n.encode_utf16().count(), up.encode_utf16().collect())
}

// a flat entry while building
struct Flat {
    name: String,
    kind: u8,
    data: Vec<u8>,
    kids: Vec<usize>,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: u64,
    sectors: Vec<u32>,
    mini: Vec<u32>,
}

fn flatten(nodes: &[Node], parent: usize, flat: &mut Vec<Flat>) {
    for n in nodes {
        let i = flat.len();
        flat[parent].kids.push(i);
        match n {
            Node::Stream { name, data } => flat.push(Flat { name: name.clone(), kind: 2, data: data.clone(), kids: Vec::new(), left: NOSTREAM, right: NOSTREAM, child: NOSTREAM, start: ENDOFCHAIN, size: data.len() as u64, sectors: Vec::new(), mini: Vec::new() }),
            Node::Storage { name, kids } => {
                flat.push(Flat { name: name.clone(), kind: 1, data: Vec::new(), kids: Vec::new(), left: NOSTREAM, right: NOSTREAM, child: NOSTREAM, start: 0, size: 0, sectors: Vec::new(), mini: Vec::new() });
                flatten(kids, i, flat);
            }
        }
    }
}

// a balanced tree of the sorted siblings: the middle one at the top -> its index
fn tree(sorted: &[usize], flat: &mut [Flat]) -> u32 {
    if sorted.is_empty() {
        return NOSTREAM;
    }
    let m = sorted.len() / 2;
    let l = tree(&sorted[..m], flat);
    let r = tree(&sorted[m + 1..], flat);
    flat[sorted[m]].left = l;
    flat[sorted[m]].right = r;
    sorted[m] as u32
}

impl Cfb {
    pub fn build(&self) -> Built {
        let v4 = self.version == 4;
        let sector = if v4 { 4096 } else { 512 };
        let per = sector / 4;
        let mut flat = vec![Flat { name: String::from("Root Entry"), kind: 5, data: Vec::new(), kids: Vec::new(), left: NOSTREAM, right: NOSTREAM, child: NOSTREAM, start: ENDOFCHAIN, size: 0, sectors: Vec::new(), mini: Vec::new() }];
        flatten(&self.kids, 0, &mut flat);
        // the siblings' trees
        for i in 0..flat.len() {
            if flat[i].kind == 2 {
                continue;
            }
            let mut kids = flat[i].kids.clone();
            kids.sort_by_key(|&k| name_key(&flat[k].name));
            let top = tree(&kids, &mut flat);
            flat[i].child = top;
        }
        // the mini stream: the small streams one after the other in 64-byte sectors
        let mut mini_bytes: Vec<u8> = Vec::new();
        let mut minifat: Vec<u32> = Vec::new();
        for f in flat.iter_mut().filter(|f| f.kind == 2 && !f.data.is_empty() && f.data.len() < CUTOFF) {
            let first = minifat.len() as u32;
            let n = f.data.len().div_ceil(64);
            for k in 0..n {
                minifat.push(if k + 1 == n { ENDOFCHAIN } else { first + k as u32 + 1 });
                f.mini.push(first + k as u32);
            }
            f.start = first;
            mini_bytes.extend_from_slice(&f.data);
            mini_bytes.resize(minifat.len() * 64, 0);
        }
        // the sectors: large streams, the mini stream, the mini FAT, the directory
        let mut next = 0u32;
        let chain = |count: usize, next: &mut u32| -> Vec<u32> {
            let v: Vec<u32> = (0..count as u32).map(|k| *next + k).collect();
            *next += count as u32;
            v
        };
        for f in flat.iter_mut().filter(|f| f.kind == 2 && f.data.len() >= CUTOFF) {
            f.sectors = chain(f.data.len().div_ceil(sector), &mut next);
            f.start = f.sectors[0];
        }
        let mini_sectors = chain(mini_bytes.len().div_ceil(sector), &mut next);
        flat[0].sectors = mini_sectors.clone();
        flat[0].start = mini_sectors.first().copied().unwrap_or(ENDOFCHAIN);
        flat[0].size = mini_bytes.len() as u64;
        let minifat_sectors = chain((minifat.len() * 4).div_ceil(sector), &mut next);
        let slots = (flat.len() + self.spare_entries).div_ceil(sector / 128) * (sector / 128);
        let dir_sectors = chain((slots * 128).div_ceil(sector), &mut next);
        // the FAT (and the DIFAT) last: as many sectors as cover everything, themselves too
        let data = next as usize;
        let mut nfat = self.min_fat.max(1);
        let ndifat = loop {
            let nd = if nfat > 109 { (nfat - 109).div_ceil(per - 1) } else { 0 };
            let need = (data + nfat + nd).div_ceil(per);
            if need <= nfat {
                break nd;
            }
            nfat = need;
        };
        let fat_sectors = chain(nfat, &mut next);
        let difat_sectors = chain(ndifat, &mut next);
        let total = next as usize;
        let mut fat = vec![FREESECT; nfat * per];
        let link = |fat: &mut Vec<u32>, s: &[u32]| {
            for (k, &x) in s.iter().enumerate() {
                fat[x as usize] = s.get(k + 1).copied().unwrap_or(ENDOFCHAIN);
            }
        };
        for f in &flat {
            link(&mut fat, &f.sectors);
        }
        link(&mut fat, &minifat_sectors);
        link(&mut fat, &dir_sectors);
        for &s in &fat_sectors {
            fat[s as usize] = FATSECT;
        }
        for &s in &difat_sectors {
            fat[s as usize] = DIFSECT;
        }
        // the file
        let mut b = vec![0u8; sector + total * sector];
        b[..8].copy_from_slice(&SIGNATURE);
        put16(&mut b, 0x18, 0x3E);
        put16(&mut b, 0x1A, self.version);
        put16(&mut b, 0x1C, 0xFFFE);
        put16(&mut b, 0x1E, if v4 { 12 } else { 9 });
        put16(&mut b, 0x20, 6);
        put32(&mut b, 0x28, if v4 { dir_sectors.len() as u32 } else { 0 });
        put32(&mut b, 0x2C, nfat as u32);
        put32(&mut b, 0x30, dir_sectors[0]);
        put32(&mut b, 0x38, CUTOFF as u32);
        put32(&mut b, 0x3C, minifat_sectors.first().copied().unwrap_or(ENDOFCHAIN));
        put32(&mut b, 0x40, minifat_sectors.len() as u32);
        put32(&mut b, 0x44, difat_sectors.first().copied().unwrap_or(ENDOFCHAIN));
        put32(&mut b, 0x48, ndifat as u32);
        for k in 0..109 {
            put32(&mut b, 0x4C + 4 * k, fat_sectors.get(k).copied().unwrap_or(FREESECT));
        }
        let off = |s: u32| (s as usize + 1) * sector;
        let write_chain = |b: &mut Vec<u8>, s: &[u32], data: &[u8]| {
            for (k, &x) in s.iter().enumerate() {
                let from = k * sector;
                let n = data.len().saturating_sub(from).min(sector);
                b[off(x)..off(x) + n].copy_from_slice(&data[from..from + n]);
            }
        };
        for f in &flat {
            if f.kind == 2 && f.data.len() >= CUTOFF {
                write_chain(&mut b, &f.sectors, &f.data);
            }
        }
        write_chain(&mut b, &mini_sectors, &mini_bytes);
        let mut mf: Vec<u8> = minifat.iter().flat_map(|x| x.to_le_bytes()).collect();
        mf.resize(minifat_sectors.len() * sector, 0xFF);
        write_chain(&mut b, &minifat_sectors, &mf);
        let fb: Vec<u8> = fat.iter().flat_map(|x| x.to_le_bytes()).collect();
        write_chain(&mut b, &fat_sectors, &fb);
        // the DIFAT sectors: per - 1 FAT sector numbers, then the next DIFAT sector
        let rest: Vec<u32> = fat_sectors.iter().skip(109).copied().collect();
        for (k, &d) in difat_sectors.iter().enumerate() {
            let o = off(d);
            for j in 0..per - 1 {
                put32(&mut b, o + 4 * j, rest.get(k * (per - 1) + j).copied().unwrap_or(FREESECT));
            }
            put32(&mut b, o + sector - 4, difat_sectors.get(k + 1).copied().unwrap_or(ENDOFCHAIN));
        }
        // the directory
        let mut dir = vec![0u8; dir_sectors.len() * sector];
        for slot in 0..dir.len() / 128 {
            let e = &mut dir[slot * 128..(slot + 1) * 128];
            for at in [68, 72, 76] {
                put32(e, at, NOSTREAM);
            }
        }
        let mut entries = Vec::new();
        for (i, f) in flat.iter().enumerate() {
            let e = &mut dir[i * 128..(i + 1) * 128];
            let units: Vec<u16> = f.name.encode_utf16().take(31).collect();
            for (k, u) in units.iter().enumerate() {
                put16(e, 2 * k, *u);
            }
            put16(e, 64, (units.len() as u16 + 1) * 2);
            e[66] = f.kind;
            e[67] = 1;
            put32(e, 68, f.left);
            put32(e, 72, f.right);
            put32(e, 76, f.child);
            put32(e, 116, if f.kind == 2 && f.size == 0 { ENDOFCHAIN } else { f.start });
            put32(e, 120, f.size as u32);
            put32(e, 124, (f.size >> 32) as u32);
            let slot_sector = dir_sectors[i * 128 / sector];
            entries.push(Placed { name: f.name.clone(), kind: f.kind, at: off(slot_sector) + (i * 128) % sector, sectors: f.sectors.clone(), mini: f.mini.clone(), size: f.size, kids: f.kids.clone() });
        }
        write_chain(&mut b, &dir_sectors, &dir);
        let unused = (flat.len() < slots).then_some(flat.len());
        Built { bytes: b, version: self.version, sector, entries, dir: dir_sectors, fat: fat_sectors, difat: difat_sectors, minifat: minifat_sectors, unused }
    }
}

impl Built {
    /// the file offset of sector n
    pub fn offset(&self, n: u32) -> usize {
        (n as usize + 1) * self.sector
    }

    /// the file offset of sector n's FAT entry
    pub fn fat_entry(&self, n: u32) -> usize {
        let per = self.sector / 4;
        self.offset(self.fat[n as usize / per]) + 4 * (n as usize % per)
    }

    /// the file offset of mini sector n's mini FAT entry
    pub fn minifat_entry(&self, n: u32) -> usize {
        let per = self.sector / 4;
        self.offset(self.minifat[n as usize / per]) + 4 * (n as usize % per)
    }

    /// the directory index of the (first) entry of this name
    pub fn find(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|e| e.name == name)
    }

    pub fn set16(&mut self, at: usize, v: u16) {
        put16(&mut self.bytes, at, v);
    }
    pub fn set32(&mut self, at: usize, v: u32) {
        put32(&mut self.bytes, at, v);
    }

    /// set the left (68), right (72) or child (76) field of entry i
    pub fn link(&mut self, i: usize, field: usize, to: u32) {
        let at = self.entries[i].at + field;
        self.set32(at, to);
    }
}

/// the left, right and child fields of a directory entry
pub const LEFT: usize = 68;
pub const RIGHT: usize = 72;
pub const CHILD: usize = 76;

/// the hostile variants of a file: what is changed after writing
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hostile {
    /// the first byte of the signature
    Signature,
    /// byte order 0xFEFF
    ByteOrder,
    /// version 3 with 4096-byte sectors
    SectorShift,
    /// mini sectors of 128 bytes
    MiniShift,
    /// the mini stream cutoff 8192
    Cutoff,
    /// major version 5
    Version5,
    /// major version 4 with 512-byte sectors
    Version4Small,
    /// more FAT sectors than the file has sectors
    NfatHuge,
    /// the FAT chain of the stream VisioDocument (in sectors of its own) back to its start from its second sector
    FatLoop,
    /// the same from its last sector (the chain still as long as the stream)
    FatLoopEnd,
    /// the stream's second sector a number beyond the file
    FatOutside,
    /// the stream's chain ends after its first sector
    FatShort,
    /// the stream's chain goes on to a FAT sector marker
    FatSpecial,
    /// the directory's chain back to its first sector
    DirLoop,
    /// the directory starts beyond the file
    DirOutside,
    /// the first DIFAT sector names itself as the next (needs two DIFAT sectors)
    DifatLoop,
    /// the header claims no DIFAT sector while the FAT needs one
    DifatShort,
    /// the DIFAT starts beyond the file
    DifatOutside,
    /// an entry is its own left sibling
    LeftSelf,
    /// two siblings point at each other (left one way, right back)
    SiblingCycle,
    /// a storage's child is the root
    ChildRoot,
    /// a storage's child is an entry another storage already has (VisioDocument)
    ChildShared,
    /// a storage's child is no entry of the directory
    ChildOutside,
    /// a storage's child is an unused slot
    ChildUnused,
    /// a stream has a child
    StreamChild,
    /// the root's type is a storage's
    RootKind,
    /// an entry of type 3
    EntryKind,
    /// a name length that is odd
    NameOdd,
    /// a name length of 66 bytes
    NameLong,
    /// a name without its NUL
    NameNoNul,
    /// a name with a "/"
    NameSlash,
    /// an empty name
    NameEmpty,
    /// the stream VisioDocument claims 2 GiB
    StreamHuge,
    /// the stream VisioDocument claims three sectors more than its chain has
    StreamLonger,
    /// the root's mini stream claims 2 GiB
    RootHuge,
    /// the mini FAT chain of the (small) stream VisioDocument back to its start from its second mini sector
    MiniLoop,
    /// the (small) stream VisioDocument starts beyond the mini stream
    MiniOutside,
    /// the header has no mini FAT while small streams exist
    MiniFatMissing,
    /// the file cut in half
    Half,
    /// the file cut within the header
    CutHeader,
    /// version 3 with garbage in the high half of VisioDocument's size (to be ignored: a control)
    HighSizeV3,
}

impl Hostile {
    pub const ALL: [Hostile; 41] = [
        Hostile::Signature,
        Hostile::ByteOrder,
        Hostile::SectorShift,
        Hostile::MiniShift,
        Hostile::Cutoff,
        Hostile::Version5,
        Hostile::Version4Small,
        Hostile::NfatHuge,
        Hostile::FatLoop,
        Hostile::FatLoopEnd,
        Hostile::FatOutside,
        Hostile::FatShort,
        Hostile::FatSpecial,
        Hostile::DirLoop,
        Hostile::DirOutside,
        Hostile::DifatLoop,
        Hostile::DifatShort,
        Hostile::DifatOutside,
        Hostile::LeftSelf,
        Hostile::SiblingCycle,
        Hostile::ChildRoot,
        Hostile::ChildShared,
        Hostile::ChildOutside,
        Hostile::ChildUnused,
        Hostile::StreamChild,
        Hostile::RootKind,
        Hostile::EntryKind,
        Hostile::NameOdd,
        Hostile::NameLong,
        Hostile::NameNoNul,
        Hostile::NameSlash,
        Hostile::NameEmpty,
        Hostile::StreamHuge,
        Hostile::StreamLonger,
        Hostile::RootHuge,
        Hostile::MiniLoop,
        Hostile::MiniOutside,
        Hostile::MiniFatMissing,
        Hostile::Half,
        Hostile::CutHeader,
        Hostile::HighSizeV3,
    ];

    /// the file the variant needs: (VisioDocument in sectors of its own, FAT sectors at least)
    pub fn needs(self) -> (bool, usize) {
        match self {
            Hostile::FatLoop | Hostile::FatLoopEnd | Hostile::FatOutside | Hostile::FatShort | Hostile::FatSpecial | Hostile::StreamLonger | Hostile::DirLoop => (true, 1),
            Hostile::DifatLoop => (true, 109 + 127 + 2),
            Hostile::DifatShort | Hostile::DifatOutside => (true, 110),
            _ => (false, 1),
        }
    }

    /// a short name for the checks
    pub fn name(self) -> String {
        let s = alloc::format!("{self:?}");
        // CamelCase -> kebab-case
        let mut o = String::new();
        for (i, c) in s.chars().enumerate() {
            if c.is_ascii_uppercase() && i > 0 {
                o.push('-');
            }
            o.push(c.to_ascii_lowercase());
        }
        o
    }

    /// change the written file; false when it has no place for this variant
    pub fn apply(self, b: &mut Built) -> bool {
        let doc = b.find("VisioDocument");
        let sub = b.entries.iter().position(|e| e.kind == 1);
        match self {
            Hostile::Signature => b.bytes[0] ^= 0xFF,
            Hostile::ByteOrder => b.set16(0x1C, 0xFEFF),
            Hostile::SectorShift => {
                if b.version != 3 {
                    return false;
                }
                b.set16(0x1E, 12);
            }
            Hostile::MiniShift => b.set16(0x20, 7),
            Hostile::Cutoff => b.set32(0x38, 8192),
            Hostile::Version5 => b.set16(0x1A, 5),
            Hostile::Version4Small => {
                if b.version != 3 {
                    return false;
                }
                b.set16(0x1A, 4);
            }
            Hostile::NfatHuge => {
                let n = (b.bytes.len() / b.sector) as u32 + 1;
                b.set32(0x2C, n);
            }
            Hostile::FatLoop | Hostile::FatLoopEnd | Hostile::FatOutside | Hostile::FatShort | Hostile::FatSpecial => {
                let Some(d) = doc else { return false };
                let s = b.entries[d].sectors.clone();
                if s.len() < 3 {
                    return false;
                }
                let (at, to) = match self {
                    Hostile::FatLoop => (s[1], s[0]),
                    Hostile::FatLoopEnd => (s[s.len() - 1], s[0]),
                    Hostile::FatOutside => (s[0], 0x00FF_FFFF),
                    Hostile::FatShort => (s[0], ENDOFCHAIN),
                    _ => (s[0], FATSECT),
                };
                let e = b.fat_entry(at);
                b.set32(e, to);
            }
            Hostile::DirLoop => {
                let last = *b.dir.last().unwrap_or(&0);
                let first = b.dir[0];
                let e = b.fat_entry(last);
                b.set32(e, first);
            }
            Hostile::DirOutside => b.set32(0x30, 0x00FF_FFFF),
            Hostile::DifatLoop => {
                if b.difat.len() < 2 {
                    return false;
                }
                let d0 = b.difat[0];
                let at = b.offset(d0) + b.sector - 4;
                b.set32(at, d0);
            }
            Hostile::DifatShort => {
                if b.difat.is_empty() {
                    return false;
                }
                b.set32(0x48, 0);
            }
            Hostile::DifatOutside => {
                if b.difat.is_empty() {
                    return false;
                }
                b.set32(0x44, 0x00FF_FFFF);
            }
            Hostile::LeftSelf => {
                let Some(d) = doc else { return false };
                b.link(d, LEFT, d as u32);
            }
            Hostile::SiblingCycle => {
                let kids = b.entries[0].kids.clone();
                if kids.len() < 2 {
                    return false;
                }
                let (c1, c2) = (kids[0], kids[1]);
                b.link(0, CHILD, c1 as u32);
                b.link(c1, LEFT, c2 as u32);
                b.link(c1, RIGHT, NOSTREAM);
                b.link(c2, LEFT, NOSTREAM);
                b.link(c2, RIGHT, c1 as u32);
            }
            Hostile::ChildRoot | Hostile::ChildShared | Hostile::ChildOutside | Hostile::ChildUnused => {
                let Some(s) = sub else { return false };
                let to = match self {
                    Hostile::ChildRoot => 0,
                    Hostile::ChildShared => match doc {
                        Some(d) => d as u32,
                        None => return false,
                    },
                    Hostile::ChildOutside => 1000,
                    _ => match b.unused {
                        Some(u) => u as u32,
                        None => return false,
                    },
                };
                b.link(s, CHILD, to);
            }
            Hostile::StreamChild => {
                let (Some(d), Some(s)) = (doc, sub) else { return false };
                let Some(&inner) = b.entries[s].kids.first() else { return false };
                b.link(d, CHILD, inner as u32);
            }
            Hostile::RootKind => {
                let at = b.entries[0].at + 66;
                b.bytes[at] = 1;
            }
            Hostile::EntryKind => {
                let Some(d) = doc else { return false };
                let at = b.entries[d].at + 66;
                b.bytes[at] = 3;
            }
            Hostile::NameOdd | Hostile::NameLong | Hostile::NameNoNul | Hostile::NameEmpty | Hostile::NameSlash => {
                let Some(d) = doc else { return false };
                let at = b.entries[d].at;
                let len = u16::from_le_bytes([b.bytes[at + 64], b.bytes[at + 65]]) as usize;
                match self {
                    Hostile::NameOdd => b.set16(at + 64, len as u16 - 1),
                    Hostile::NameLong => b.set16(at + 64, 66),
                    Hostile::NameNoNul => b.set16(at + len - 2, u16::from(b'x')),
                    Hostile::NameEmpty => {
                        b.set16(at, 0);
                        b.set16(at + 64, 2);
                    }
                    _ => b.set16(at + 2, u16::from(b'/')),
                }
            }
            Hostile::StreamHuge | Hostile::StreamLonger | Hostile::HighSizeV3 => {
                let Some(d) = doc else { return false };
                let at = b.entries[d].at;
                match self {
                    Hostile::StreamHuge => b.set32(at + 120, 0x7FFF_FFFF),
                    Hostile::StreamLonger => {
                        let size = b.entries[d].size as u32 + 3 * b.sector as u32;
                        b.set32(at + 120, size);
                    }
                    _ => {
                        if b.version != 3 {
                            return false;
                        }
                        b.set32(at + 124, 0xDEAD_BEEF);
                    }
                }
            }
            Hostile::RootHuge => {
                let at = b.entries[0].at + 120;
                b.set32(at, 0x7FFF_FFFF);
            }
            Hostile::MiniLoop | Hostile::MiniOutside => {
                let Some(d) = doc else { return false };
                let m = b.entries[d].mini.clone();
                if m.len() < 3 {
                    return false;
                }
                if self == Hostile::MiniLoop {
                    let e = b.minifat_entry(m[1]);
                    b.set32(e, m[0]);
                } else {
                    let at = b.entries[d].at + 116;
                    b.set32(at, 100_000);
                }
            }
            Hostile::MiniFatMissing => {
                if b.minifat.is_empty() {
                    return false;
                }
                b.set32(0x3C, ENDOFCHAIN);
                b.set32(0x40, 0);
            }
            Hostile::Half => {
                let n = b.bytes.len() / 2;
                b.bytes.truncate(n);
            }
            Hostile::CutHeader => b.bytes.truncate(300),
        }
        true
    }
}
