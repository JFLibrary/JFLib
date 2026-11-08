// sqfs -- a SquashFS 4.0 image read in place: the file system of an AppImage, behind the runtime
//
//   Image::open(file, base) -> Result<Image, String>   the superblock at byte base of the file;
//                     compression zstd or lz4 (mksquashfs -comp zstd | lz4), uncompressed
//                     blocks of any image; gzip, lzma, lzo and xz are refused
//   img.root() / img.inode(r) -> Option<Inode>         the root folder / an inode by reference
//   img.list(&dir) -> Option<Vec<Entry>>               a folder's entries (name, inode reference)
//   img.read(&file, &mut out) -> bool                  a file's bytes in order, block by block
//
// Layout (squashfs-tools, squashfs_fs.h): the 96-byte superblock; metadata blocks of up to 8 KiB
// behind a 2-byte header (bit 15: stored uncompressed) hold the inode and directory tables;
// an inode reference is (block offset from the table start) << 16 | offset in that block; data
// blocks are listed per file with their stored sizes (bit 24: uncompressed, 0: a sparse block of
// zeros), the tail of a file may sit in a shared fragment block (fragment table: 16-byte
// entries in metadata blocks found through a list of u64 positions).  Every position, size
// and count is checked against the image: a broken image gives None or Err, never a panic.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use plat::File;

const MAGIC: u32 = 0x7371_7368; // "hsqs"
const META: usize = 8192;
const NO_FRAGMENT: u32 = 0xFFFF_FFFF;
const BLOCK_RAW: u32 = 1 << 24;
const COMP_LZ4: u16 = 5;
const COMP_ZSTD: u16 = 6;

/// what an inode is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    Symlink,
    /// devices, pipes, sockets: not extracted
    Other,
}

/// one inode, the fields of its kind filled in
pub struct Inode {
    pub kind: Kind,
    /// permission bits (07777)
    pub mode: u16,
    // a folder: its listing in the directory table
    dir_block: u32,
    dir_offset: u16,
    dir_size: u32,
    // a file: its blocks, size and the fragment holding its tail
    blocks_start: u64,
    pub size: u64,
    frag: u32,
    frag_offset: u32,
    blocks: Vec<u32>,
    /// a symbolic link: its target
    pub target: String,
}

/// one name of a folder
pub struct Entry {
    pub name: String,
    pub inode: u64,
}

struct Super {
    block_size: u32,
    frag_count: u32,
    comp: u16,
    root: u64,
    bytes_used: u64,
    inode_table: u64,
    dir_table: u64,
    frag_table: u64,
}

pub struct Image<'a> {
    file: &'a File,
    base: u64,
    sb: Super,
    // decompressed metadata blocks by position: (bytes, position of the next block)
    meta: BTreeMap<u64, (Vec<u8>, u64)>,
    // the fragment block read last: (position, bytes)
    frag_cache: Option<(u64, Vec<u8>)>,
}

fn le(b: &[u8], o: usize, n: usize) -> Option<u64> {
    Some(b.get(o..o + n)?.iter().rev().fold(0u64, |a, &x| a << 8 | x as u64))
}

impl<'a> Image<'a> {
    pub fn open(file: &'a File, base: u64) -> Result<Image<'a>, String> {
        let mut b = [0u8; 96];
        if !read_full(file, base, &mut b) {
            return Err(String::from("kein SquashFS-Abbild hinter der Runtime (Datei zu kurz)"));
        }
        let f = |o, n| le(&b, o, n).unwrap_or(0);
        if f(0, 4) as u32 != MAGIC {
            return Err(String::from("kein SquashFS-Abbild hinter der Runtime"));
        }
        if (f(28, 2), f(30, 2)) != (4, 0) {
            return Err(format!("SquashFS {}.{} wird nicht unterstützt (nur 4.0)", f(28, 2), f(30, 2)));
        }
        let sb = Super {
            block_size: f(12, 4) as u32,
            frag_count: f(16, 4) as u32,
            comp: f(20, 2) as u16,
            root: f(32, 8),
            bytes_used: f(40, 8),
            inode_table: f(64, 8),
            dir_table: f(72, 8),
            frag_table: f(80, 8),
        };
        let log = f(22, 2) as u32;
        if !(12..=20).contains(&log) || sb.block_size != 1 << log {
            return Err(format!("ungültige Blockgröße {}", sb.block_size));
        }
        if !matches!(sb.comp, COMP_LZ4 | COMP_ZSTD) {
            let name = match sb.comp { 1 => "gzip", 2 => "lzma", 3 => "lzo", 4 => "xz", _ => "unbekannt" };
            return Err(format!("Kompression {name} wird nicht unterstützt (zstd oder lz4: mksquashfs -comp zstd)"));
        }
        if sb.inode_table >= sb.bytes_used || sb.dir_table >= sb.bytes_used || sb.inode_table > sb.dir_table {
            return Err(String::from("beschädigtes SquashFS-Abbild (Tabellen)"));
        }
        let total = file.size().unwrap_or(0);
        if base.checked_add(sb.bytes_used).is_none_or(|e| e > total) {
            return Err(String::from("das SquashFS-Abbild ist abgeschnitten"));
        }
        Ok(Image { file, base, sb, meta: BTreeMap::new(), frag_cache: None })
    }

    // len bytes at pos of the image (inside bytes_used)
    fn raw(&self, pos: u64, len: usize) -> Option<Vec<u8>> {
        if pos.checked_add(len as u64)? > self.sb.bytes_used {
            return None;
        }
        let mut v = vec![0u8; len];
        read_full(self.file, self.base + pos, &mut v).then_some(v)
    }

    fn unpack(&self, data: &[u8], max: usize) -> Option<Vec<u8>> {
        match self.sb.comp {
            COMP_ZSTD => compress::zstd::decompress(data, max),
            COMP_LZ4 => compress::lz4::decompress(data, max),
            _ => None,
        }
    }

    // the metadata block at pos: its bytes and where the next one starts
    fn block(&mut self, pos: u64) -> Option<&(Vec<u8>, u64)> {
        if !self.meta.contains_key(&pos) {
            let h = le(&self.raw(pos, 2)?, 0, 2)? as u16;
            let n = (h & 0x7FFF) as usize;
            if n == 0 || n > META {
                return None;
            }
            let stored = self.raw(pos + 2, n)?;
            let data = if h & 0x8000 != 0 { stored } else { self.unpack(&stored, META)? };
            self.meta.insert(pos, (data, pos + 2 + n as u64));
        }
        self.meta.get(&pos)
    }

    // n bytes of the metadata stream from (block, offset), on into the following blocks
    fn take(&mut self, at: &mut (u64, usize), n: usize) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            let (data, next) = self.block(at.0)?;
            if at.1 > data.len() {
                return None;
            }
            let k = (n - out.len()).min(data.len() - at.1);
            out.extend_from_slice(&data[at.1..at.1 + k]);
            at.1 += k;
            if at.1 == data.len() {
                *at = (*next, 0);
            }
        }
        Some(out)
    }

    pub fn root(&mut self) -> Option<Inode> {
        self.inode(self.sb.root)
    }

    pub fn inode(&mut self, r: u64) -> Option<Inode> {
        let mut at = (self.sb.inode_table.checked_add(r >> 16)?, (r & 0xFFFF) as usize);
        let h = self.take(&mut at, 16)?;
        let typ = le(&h, 0, 2)? as u16;
        let mut n = Inode {
            kind: Kind::Other,
            mode: le(&h, 2, 2)? as u16 & 0o7777,
            dir_block: 0,
            dir_offset: 0,
            dir_size: 0,
            blocks_start: 0,
            size: 0,
            frag: NO_FRAGMENT,
            frag_offset: 0,
            blocks: Vec::new(),
            target: String::new(),
        };
        match typ {
            1 => {
                let d = self.take(&mut at, 16)?;
                n.kind = Kind::Dir;
                (n.dir_block, n.dir_size, n.dir_offset) = (le(&d, 0, 4)? as u32, le(&d, 8, 2)? as u32, le(&d, 10, 2)? as u16);
            }
            8 => {
                let d = self.take(&mut at, 24)?;
                n.kind = Kind::Dir;
                (n.dir_size, n.dir_block, n.dir_offset) = (le(&d, 4, 4)? as u32, le(&d, 8, 4)? as u32, le(&d, 18, 2)? as u16);
            }
            2 => {
                let d = self.take(&mut at, 16)?;
                n.kind = Kind::File;
                n.blocks_start = le(&d, 0, 4)?;
                (n.frag, n.frag_offset, n.size) = (le(&d, 4, 4)? as u32, le(&d, 8, 4)? as u32, le(&d, 12, 4)?);
                self.block_list(&mut at, &mut n)?;
            }
            9 => {
                let d = self.take(&mut at, 40)?;
                n.kind = Kind::File;
                (n.blocks_start, n.size) = (le(&d, 0, 8)?, le(&d, 8, 8)?);
                (n.frag, n.frag_offset) = (le(&d, 28, 4)? as u32, le(&d, 32, 4)? as u32);
                self.block_list(&mut at, &mut n)?;
            }
            3 | 10 => {
                let d = self.take(&mut at, 8)?;
                let len = le(&d, 4, 4)? as usize;
                if len == 0 || len > 4096 {
                    return None;
                }
                n.kind = Kind::Symlink;
                n.target = String::from_utf8(self.take(&mut at, len)?).ok()?;
            }
            4..=7 | 11..=14 => {}
            _ => return None,
        }
        Some(n)
    }

    // the stored sizes of a file's full blocks (the tail is in the fragment when there is one)
    fn block_list(&mut self, at: &mut (u64, usize), n: &mut Inode) -> Option<()> {
        let bs = self.sb.block_size as u64;
        let count = if n.frag == NO_FRAGMENT { n.size.div_ceil(bs) } else { n.size / bs };
        if count > 1 << 24 {
            return None; // a block list of more than 64 MiB: a broken inode
        }
        if n.frag != NO_FRAGMENT && (n.frag >= self.sb.frag_count || n.frag_offset as u64 + n.size % bs > bs) {
            return None;
        }
        let raw = self.take(at, usize::try_from(count).ok()?.checked_mul(4)?)?;
        n.blocks = raw.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        Some(())
    }

    pub fn list(&mut self, dir: &Inode) -> Option<Vec<Entry>> {
        if dir.kind != Kind::Dir {
            return None;
        }
        // the listing's size counts "." and ".." as 3 bytes
        let mut left = dir.dir_size.checked_sub(3)? as usize;
        let mut at = (self.sb.dir_table.checked_add(dir.dir_block as u64)?, dir.dir_offset as usize);
        let mut out = Vec::new();
        while left > 0 {
            let h = self.take(&mut at, 12)?;
            left = left.checked_sub(12)?;
            let count = le(&h, 0, 4)? + 1;
            let start = le(&h, 4, 4)?;
            if count > 256 {
                return None;
            }
            for _ in 0..count {
                let e = self.take(&mut at, 8)?;
                let len = le(&e, 6, 2)? as usize + 1;
                let name = String::from_utf8(self.take(&mut at, len)?).ok()?;
                left = left.checked_sub(8 + len)?;
                out.push(Entry { name, inode: start << 16 | le(&e, 0, 2)? });
            }
        }
        Some(out)
    }

    // (position, stored size) of fragment block i
    fn fragment(&mut self, i: u32) -> Option<(u64, u32)> {
        if i >= self.sb.frag_count {
            return None;
        }
        let list = le(&self.raw(self.sb.frag_table.checked_add(i as u64 / 512 * 8)?, 8)?, 0, 8)?;
        let mut at = (list, (i % 512) as usize * 16);
        let e = self.take(&mut at, 16)?;
        Some((le(&e, 0, 8)?, le(&e, 8, 4)? as u32))
    }

    // a data block: stored size (with the uncompressed bit) at pos -> exactly len bytes
    fn data(&self, pos: u64, stored: u32, len: usize) -> Option<Vec<u8>> {
        let n = (stored & !BLOCK_RAW) as usize;
        if n == 0 {
            return Some(vec![0u8; len]); // sparse
        }
        if n > self.sb.block_size as usize {
            return None;
        }
        let raw = self.raw(pos, n)?;
        let v = if stored & BLOCK_RAW != 0 { raw } else { self.unpack(&raw, self.sb.block_size as usize)? };
        (v.len() >= len).then_some(v)
    }

    /// the bytes of a file, in pieces of at most one block, to out (false stops) -> all read
    pub fn read(&mut self, n: &Inode, out: &mut dyn FnMut(&[u8]) -> bool) -> bool {
        if n.kind != Kind::File {
            return false;
        }
        let bs = self.sb.block_size as u64;
        let (mut pos, mut left) = (n.blocks_start, n.size);
        for &stored in &n.blocks {
            let len = left.min(bs) as usize;
            let Some(v) = self.data(pos, stored, len) else { return false };
            if !out(&v[..len]) {
                return false;
            }
            pos += (stored & !BLOCK_RAW) as u64;
            left -= len as u64;
        }
        if left == 0 {
            return true;
        }
        let Some((fpos, fsize)) = self.fragment(n.frag) else { return false };
        if self.frag_cache.as_ref().is_none_or(|c| c.0 != fpos) {
            let Some(v) = self.data(fpos, fsize, 0) else { return false };
            self.frag_cache = Some((fpos, v));
        }
        let Some((_, v)) = &self.frag_cache else { return false };
        let o = n.frag_offset as usize;
        match v.get(o..o + left as usize) {
            Some(tail) => out(tail),
            None => false,
        }
    }
}

/// the whole of buf from off of the file
pub fn read_full(f: &File, off: u64, buf: &mut [u8]) -> bool {
    let mut done = 0;
    while done < buf.len() {
        match f.read_at(off + done as u64, &mut buf[done..]) {
            Some(n) if n > 0 => done += n,
            _ => return false,
        }
    }
    true
}
