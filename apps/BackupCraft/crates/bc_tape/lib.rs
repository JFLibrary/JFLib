// bc_tape -- the tape format of BackupCraft (BCT1): a tape is a sequence of blocks written once
// from its start and read in order.  A virtual tape is one file in the tape library.
//
//   block      "BCTB" | kind u8 | flags u8 | 0 u16 | length u32 | check u32 | payload
//              check: the first 4 bytes of BLAKE3(the first 12 header bytes + payload);
//              flags F_SEALED: the payload is sealed with the set key (XChaCha20-Poly1305)
//   LABEL      the first block of a tape: barcode, capacity, created (JSON, clear)
//   SET_BEGIN  a restore point (or its part on this tape): set id, part, pool, job, point, the
//              roots, encryption (nonce prefix, the key sealed to the recovery kit) (JSON, clear)
//   ENTRY      an entry of the point: number, root, path below it, kind, size, times (JSON)
//   DATA       a piece of a file: entry number u32 | offset u64 | bytes (at most DATA_MAX)
//   FILE_END   entry number and BLAKE3 of the whole file (JSON)
//   SET_END    the counts; last = the set ends here (else it goes on on the next tape) (JSON)
//
// Sealed payloads: nonce = nonce prefix (16 bytes) | block number in the set (u64 LE), AAD =
// "BCT1" | set id | kind.  Block numbers run on across the parts of a set; SET_BEGIN tells the
// number of its first block.  Everything after SET_BEGIN is sealed in an encrypted set, names
// included.
//
//   Tape::create / open / append / read / erase      a tape file
//   SetKey                                           sealing and opening the blocks of a set
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::j::{self, Json};
use bc_proto::record;

pub const FORMAT: &str = "BCT1";
pub const BLOCK_MAGIC: &[u8; 4] = b"BCTB";
pub const HEADER: usize = 16;
/// the largest piece of a file in one DATA block
pub const DATA_MAX: usize = 1 << 20;
/// the largest payload a reader accepts
pub const PAYLOAD_MAX: usize = 8 << 20;
/// the file name extension of a virtual tape
pub const EXT: &str = ".bctape";

pub const K_LABEL: u8 = 1;
pub const K_SET_BEGIN: u8 = 2;
pub const K_ENTRY: u8 = 3;
pub const K_DATA: u8 = 4;
pub const K_FILE_END: u8 = 5;
pub const K_SET_END: u8 = 6;

pub const F_SEALED: u8 = 1;

/// the AAD of a set key sealed to the recovery kit (crypto::sealbox)
pub const AAD_RECOVERY: &[u8] = b"bc-tape-recovery";

record! {
    /// the first block of a tape
    pub struct Label {
        pub format: String,
        pub barcode: String,
        pub capacity: u64,
        pub created: i64,
        /// the server that labeled it
        pub library: String,
    }
}

record! {
    /// the start of a set (or of its part on this tape)
    pub struct SetBegin {
        pub set_id: String,
        pub part: u32,
        pub pool_id: String,
        pub pool_name: String,
        pub job_id: String,
        pub job_name: String,
        pub point_id: String,
        pub point_time: i64,
        pub host_name: String,
        /// the paths of the roots follow Windows rules
        pub windows: bool,
        pub roots: Vec<String>,
        pub encrypted: bool,
        /// hex of the 16-byte nonce prefix
        pub nonce: String,
        /// base64url of the set key sealed to the recovery kit ("" without a kit)
        pub recovery: String,
        /// hex of KEY_CHECK(key): tells a wrong key at once
        pub key_check: String,
        /// the block number of the first block after this one
        pub block0: u64,
        pub created: i64,
    }
}

record! {
    /// an entry of the point
    pub struct Entry {
        pub i: u64,
        /// the root it belongs to (index into SetBegin.roots)
        pub r: u32,
        /// the path below the root, "/"-separated ("" = the root itself)
        pub p: String,
        /// "dir", "file", "link", "other"
        pub k: String,
        pub size: u64,
        pub mtime: i64,
        pub atime: i64,
        pub mode: u32,
        pub attrs: u32,
        pub link: String,
    }
}

record! {
    pub struct FileEnd {
        pub i: u64,
        /// hex BLAKE3 of the file
        pub hash: String,
    }
}

record! {
    pub struct SetEnd {
        pub set_id: String,
        pub part: u32,
        /// the set ends here
        pub last: bool,
        /// the writer stopped (cancelled, failed): the set is incomplete
        pub aborted: bool,
        pub entries: u64,
        pub files: u64,
        pub bytes: u64,
        /// the next block number (the following part starts with it)
        pub blocks: u64,
    }
}

fn check(head12: &[u8], payload: &[u8]) -> u32 {
    let mut h = crypto::blake3::Blake3::new();
    h.update(head12);
    h.update(payload);
    let d = h.finalize();
    u32::from_le_bytes([d[0], d[1], d[2], d[3]])
}

/// one block as written
pub fn encode(kind: u8, flags: u8, payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(HEADER + payload.len());
    b.extend_from_slice(BLOCK_MAGIC);
    b.push(kind);
    b.push(flags);
    b.extend_from_slice(&[0, 0]);
    b.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    let c = check(&b[..12], payload);
    b.extend_from_slice(&c.to_le_bytes());
    b.extend_from_slice(payload);
    b
}

/// a block header: kind, flags, payload length, check
#[derive(Clone, Copy, Debug)]
pub struct Head {
    pub kind: u8,
    pub flags: u8,
    pub len: u32,
    pub check: u32,
}

pub fn parse_head(h: &[u8]) -> Option<Head> {
    if h.len() < HEADER || &h[..4] != BLOCK_MAGIC || h[6] != 0 || h[7] != 0 {
        return None;
    }
    let kind = h[4];
    if !(K_LABEL..=K_SET_END).contains(&kind) {
        return None;
    }
    let len = u32::from_le_bytes([h[8], h[9], h[10], h[11]]);
    if len as usize > PAYLOAD_MAX {
        return None;
    }
    Some(Head { kind, flags: h[5], len, check: u32::from_le_bytes([h[12], h[13], h[14], h[15]]) })
}

/// one block read from a tape
pub struct Block {
    pub kind: u8,
    pub sealed: bool,
    pub payload: Vec<u8>,
    /// where it starts and where the next one starts
    pub pos: u64,
    pub next: u64,
}

impl Block {
    pub fn json<T: Json>(&self) -> Option<T> {
        j::parse(&self.payload).and_then(|v| T::from_json(&v))
    }
}

/// a tape file
pub struct Tape {
    f: plat::File,
    pub label: Label,
    /// the end of the last good block (where the next block goes)
    pub end: u64,
}

impl Tape {
    /// a new blank tape (the file must not exist yet)
    pub fn create(path: &str, label: &Label) -> Result<Tape, String> {
        let f = plat::File::create_new(path).ok_or_else(|| format!("{}: kann nicht angelegt werden", path))?;
        let b = encode(K_LABEL, 0, j::text(&label.to_json()).as_bytes());
        if f.write_at(0, &b) != Some(b.len()) || !f.sync() {
            drop(f);
            plat::delete(path);
            return Err(format!("{}: Schreibfehler", path));
        }
        Ok(Tape { f, label: label.clone(), end: b.len() as u64 })
    }

    /// an existing tape (`write`: to append); the end is found after the last good block, a
    /// torn block at the end (a crash while writing) is cut off when writing
    pub fn open(path: &str, write: bool) -> Result<Tape, String> {
        if plat::fs::lstat(path).is_none_or(|m| !m.is_file()) {
            return Err(format!("{}: Band nicht in der Bibliothek", path));
        }
        let f = plat::File::open(path, if write { plat::FO_RW } else { plat::FO_READ }).ok_or_else(|| format!("{}: kann nicht geöffnet werden", path))?;
        let mut t = Tape { f, label: Label::default(), end: 0 };
        let first = t.read(0)?.ok_or_else(|| format!("{}: kein BackupCraft-Band", path))?;
        if first.kind != K_LABEL {
            return Err(format!("{}: kein BackupCraft-Band", path));
        }
        t.label = first.json::<Label>().filter(|l| l.format == FORMAT).ok_or_else(|| format!("{}: unbekanntes Bandformat", path))?;
        let size = t.f.size().unwrap_or(0);
        let mut pos = first.next;
        let mut last = pos;
        // the headers chain up to the end; only the last block is checked in full
        while pos + HEADER as u64 <= size {
            let mut h = [0u8; HEADER];
            if t.f.read_at(pos, &mut h) != Some(HEADER) {
                break;
            }
            let Some(head) = parse_head(&h) else { break };
            let next = pos + HEADER as u64 + head.len as u64;
            if next > size {
                break;
            }
            last = pos;
            pos = next;
        }
        if last != pos && t.read(last).ok().flatten().is_none() {
            pos = last;
        }
        t.end = pos;
        if write && size > pos {
            t.f.set_len(pos);
        }
        Ok(t)
    }

    pub fn capacity(&self) -> u64 {
        self.label.capacity
    }

    /// the bytes still free
    pub fn room(&self) -> u64 {
        self.label.capacity.saturating_sub(self.end)
    }

    /// add a block at the end
    pub fn append(&mut self, kind: u8, flags: u8, payload: &[u8]) -> Result<u64, String> {
        let b = encode(kind, flags, payload);
        if b.len() as u64 > self.room() {
            return Err(String::from("Das Band ist voll"));
        }
        let at = self.end;
        if self.f.write_at(at, &b) != Some(b.len()) {
            return Err(String::from("Schreibfehler auf dem Band (Speicher voll?)"));
        }
        self.end += b.len() as u64;
        Ok(at)
    }

    pub fn sync(&self) -> bool {
        self.f.sync()
    }

    /// the block at `pos` (None: the end of the written part)
    pub fn read(&self, pos: u64) -> Result<Option<Block>, String> {
        let mut h = [0u8; HEADER];
        match self.f.read_at(pos, &mut h) {
            Some(n) if n == HEADER => {}
            Some(_) => return Ok(None),
            None => return Err(String::from("Lesefehler")),
        }
        if h == [0u8; HEADER] {
            return Ok(None);
        }
        let head = parse_head(&h).ok_or_else(|| format!("kein Block bei {}", pos))?;
        let mut payload = alloc::vec![0u8; head.len as usize];
        if self.f.read_at(pos + HEADER as u64, &mut payload) != Some(payload.len()) {
            return Ok(None);
        }
        if check(&h[..12], &payload) != head.check {
            return Err(format!("beschädigter Block bei {}", pos));
        }
        Ok(Some(Block { kind: head.kind, sealed: head.flags & F_SEALED != 0, payload, pos, next: pos + HEADER as u64 + head.len as u64 }))
    }

    /// blank again: only the label stays
    pub fn erase(&mut self) -> Result<(), String> {
        let b = encode(K_LABEL, 0, j::text(&self.label.to_json()).as_bytes());
        if !self.f.set_len(0) || self.f.write_at(0, &b) != Some(b.len()) || !self.f.sync() {
            return Err(String::from("Das Band lässt sich nicht löschen"));
        }
        self.end = b.len() as u64;
        Ok(())
    }
}

/// the key of an encrypted set
#[derive(Clone)]
pub struct SetKey {
    key: [u8; 32],
    prefix: [u8; 16],
    set_id: String,
}

/// hex of a key's check value (a wrong key shows at once)
pub fn key_check(key: &[u8; 32]) -> String {
    let mut h = crypto::blake3::Blake3::new_derive_key("BackupCraft tape key check");
    h.update(key);
    crypto::hex(&h.finalize()[..8])
}

impl SetKey {
    pub fn new(key: [u8; 32], prefix: [u8; 16], set_id: &str) -> SetKey {
        SetKey { key, prefix, set_id: String::from(set_id) }
    }

    /// from SET_BEGIN and the key
    pub fn for_set(key: [u8; 32], b: &SetBegin) -> Option<SetKey> {
        let p = crypto::unhex(&b.nonce).filter(|p| p.len() == 16)?;
        if key_check(&key) != b.key_check {
            return None;
        }
        let mut prefix = [0u8; 16];
        prefix.copy_from_slice(&p);
        Some(SetKey::new(key, prefix, &b.set_id))
    }

    pub fn prefix_hex(&self) -> String {
        crypto::hex(&self.prefix)
    }

    fn nonce(&self, n: u64) -> [u8; 24] {
        let mut x = [0u8; 24];
        x[..16].copy_from_slice(&self.prefix);
        x[16..].copy_from_slice(&n.to_le_bytes());
        x
    }

    fn aad(&self, kind: u8) -> Vec<u8> {
        let mut a = Vec::from(FORMAT.as_bytes());
        a.extend_from_slice(self.set_id.as_bytes());
        a.push(kind);
        a
    }

    pub fn seal(&self, n: u64, kind: u8, plain: &[u8]) -> Vec<u8> {
        crypto::aead::x_seal(&self.key, &self.nonce(n), &self.aad(kind), plain)
    }

    pub fn open(&self, n: u64, kind: u8, sealed: &[u8]) -> Option<Vec<u8>> {
        crypto::aead::x_open(&self.key, &self.nonce(n), &self.aad(kind), sealed)
    }
}

/// a DATA payload: entry number, offset, bytes
pub fn data_payload(i: u64, off: u64, bytes: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(12 + bytes.len());
    p.extend_from_slice(&(i as u32).to_le_bytes());
    p.extend_from_slice(&off.to_le_bytes());
    p.extend_from_slice(bytes);
    p
}

pub fn parse_data(p: &[u8]) -> Option<(u64, u64, &[u8])> {
    if p.len() < 12 {
        return None;
    }
    let i = u32::from_le_bytes([p[0], p[1], p[2], p[3]]) as u64;
    let mut o = [0u8; 8];
    o.copy_from_slice(&p[4..12]);
    Some((i, u64::from_le_bytes(o), &p[12..]))
}

/// the full path of an entry: its root, then the path below it with the source's separator
pub fn full_path(roots: &[String], e: &Entry, windows: bool) -> String {
    let root = roots.get(e.r as usize).map_or("", |r| r.as_str());
    if e.p.is_empty() {
        return String::from(root);
    }
    let sep = if windows { "\\" } else { "/" };
    let rest = if windows { e.p.replace('/', "\\") } else { e.p.clone() };
    if root.ends_with(sep) { format!("{}{}", root, rest) } else { format!("{}{}{}", root, sep, rest) }
}
