// pack -- blobs, pack files and their indexes
//
//   blob     kind:u8 codec:u8 | nonce:24 | XChaCha20-Poly1305(data key, nonce,
//            aad = chunk id ‖ kind ‖ codec, payload)       payload: plain, an LZ4 block or a
//                                                          zstd frame (the codec byte says which)
//   pack     "BCK1" | blobs ... | sealed index (as below) | len:u32 | "BCKI"
//            (the trailer repeats the index: a lost index file can be rebuilt from the pack)
//   index    "BCI1" | nonce:24 | XChaCha20-Poly1305(index key, aad = pack id, JSON
//            [[id hex, offset, length, kind, raw length], ...])
//
// Pack files are written to a temporary name, synced, renamed; then their index.  A crash
// leaves at most a pack without an index (garbage that check reports and gc removes).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::json::{self, Value};
use compress::Codec;
use crypto::aead;

use crate::{ChunkId, CasError, Keys};

pub const PACK_MAGIC: &[u8; 4] = b"BCK1";
pub const PACK_TRAILER: &[u8; 4] = b"BCKI";
pub const INDEX_MAGIC: &[u8; 4] = b"BCI1";
/// a pack is closed when it reaches this size
pub const PACK_TARGET: usize = 32 << 20;
pub const BLOB_OVERHEAD: usize = 2 + 24 + 16;

pub type PackId = [u8; 16];

/// where a chunk lives
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Loc {
    pub pack: PackId,
    pub off: u32,
    pub len: u32,
    pub kind: u8,
    /// the plain length
    pub raw: u32,
}

/// seal one chunk into a blob (compressed with `codec` when that saves something; `level`:
/// zstd's level)
pub fn seal_blob(keys: &Keys, id: &ChunkId, kind: u8, data: &[u8], codec: Codec, level: i32) -> Vec<u8> {
    let (codec, payload) = match codec {
        Codec::Lz4 | Codec::Zstd if data.len() >= 64 => {
            let c = codec.pack(data, level);
            if c.len() + 16 < data.len() { (codec, c) } else { (Codec::None, Vec::from(data)) }
        }
        _ => (Codec::None, Vec::from(data)),
    };
    let nonce: [u8; 24] = plat::random_bytes();
    let mut aad = [0u8; 34];
    aad[..32].copy_from_slice(id);
    aad[32] = kind;
    aad[33] = codec.id();
    let mut out = Vec::with_capacity(payload.len() + BLOB_OVERHEAD);
    out.push(kind);
    out.push(codec.id());
    out.extend_from_slice(&nonce);
    let mut body = payload;
    let tag = aead::x_seal_in_place(&keys.data, &nonce, &aad, &mut body);
    out.extend_from_slice(&body);
    out.extend_from_slice(&tag);
    out
}

/// open a blob: the plain chunk (checked against its id)
pub fn open_blob(keys: &Keys, id: &ChunkId, blob: &[u8], raw_len: usize) -> Result<Vec<u8>, CasError> {
    if blob.len() < BLOB_OVERHEAD {
        return Err(CasError::Corrupt(String::from("blob too short")));
    }
    let (kind, codec) = (blob[0], blob[1]);
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&blob[2..26]);
    let mut aad = [0u8; 34];
    aad[..32].copy_from_slice(id);
    aad[32] = kind;
    aad[33] = codec;
    let n = blob.len() - 16;
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&blob[n..]);
    let mut body = Vec::from(&blob[26..n]);
    if !aead::x_open_in_place(&keys.data, &nonce, &aad, &mut body, &tag) {
        return Err(CasError::Corrupt(format!("blob {} does not authenticate", crypto::hex(&id[..8]))));
    }
    let plain = match Codec::from_id(codec) {
        Some(Codec::None) => body,
        Some(c) => c.unpack(&body, raw_len.max(1) + 64).ok_or_else(|| CasError::Corrupt(format!("{} data damaged", c.name())))?,
        None => return Err(CasError::Corrupt(format!("unknown codec {}", codec))),
    };
    if keys.chunk_id(&plain) != *id {
        return Err(CasError::Corrupt(format!("chunk {} has other content", crypto::hex(&id[..8]))));
    }
    Ok(plain)
}

/// an index (entries of one pack) as sealed bytes
pub fn seal_index(keys: &Keys, pack: &PackId, entries: &[(ChunkId, Loc)]) -> Vec<u8> {
    let arr: Vec<Value> = entries
        .iter()
        .map(|(id, l)| Value::Array(alloc::vec![Value::string(crypto::hex(id)), Value::u64(l.off as u64), Value::u64(l.len as u64), Value::u64(l.kind as u64), Value::u64(l.raw as u64)]))
        .collect();
    let text = json::stringify_with_limits(&Value::Array(arr), crate::big_limits()).unwrap_or_default();
    let nonce: [u8; 24] = plat::random_bytes();
    let mut out = Vec::from(&INDEX_MAGIC[..]);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&aead::x_seal(&keys.index, &nonce, pack, text.as_bytes()));
    out
}

/// the entries of a sealed index
pub fn open_index(keys: &Keys, pack: &PackId, b: &[u8]) -> Result<Vec<(ChunkId, Loc)>, CasError> {
    if b.len() < 4 + 24 + 16 || &b[..4] != INDEX_MAGIC {
        return Err(CasError::Corrupt(String::from("not an index")));
    }
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&b[4..28]);
    let plain = aead::x_open(&keys.index, &nonce, pack, &b[28..]).ok_or_else(|| CasError::Corrupt(String::from("index does not authenticate")))?;
    let v = json::parse_with_limits(&plain, crate::big_limits()).map_err(|_| CasError::Corrupt(String::from("index unreadable")))?;
    let mut out = Vec::new();
    for e in v.as_array().unwrap_or(&[]) {
        let a = e.as_array().ok_or_else(|| CasError::Corrupt(String::from("index entry")))?;
        let id = a.first().and_then(|x| x.as_str()).and_then(crate::parse_id).ok_or_else(|| CasError::Corrupt(String::from("index id")))?;
        let n = |i: usize| a.get(i).and_then(|x| x.as_u64()).unwrap_or(0);
        out.push((id, Loc { pack: *pack, off: n(1) as u32, len: n(2) as u32, kind: n(3) as u8, raw: n(4) as u32 }));
    }
    Ok(out)
}

/// the index stored in a pack's trailer
pub fn trailer_index(keys: &Keys, pack: &PackId, b: &[u8]) -> Result<Vec<(ChunkId, Loc)>, CasError> {
    if b.len() < 4 + 8 || &b[b.len() - 4..] != PACK_TRAILER || &b[..4] != PACK_MAGIC {
        return Err(CasError::Corrupt(String::from("no pack trailer")));
    }
    let n = u32::from_le_bytes([b[b.len() - 8], b[b.len() - 7], b[b.len() - 6], b[b.len() - 5]]) as usize;
    let start = b.len().checked_sub(8 + n).ok_or_else(|| CasError::Corrupt(String::from("pack trailer length")))?;
    open_index(keys, pack, &b[start..b.len() - 8])
}

/// a pack being filled
pub struct PackBuf {
    pub id: PackId,
    pub data: Vec<u8>,
    pub entries: Vec<(ChunkId, Loc)>,
}

impl PackBuf {
    pub fn new() -> PackBuf {
        let id: PackId = plat::random_bytes();
        let mut data = Vec::with_capacity(1 << 20);
        data.extend_from_slice(PACK_MAGIC);
        PackBuf { id, data, entries: Vec::new() }
    }

    pub fn push(&mut self, id: ChunkId, kind: u8, raw: usize, blob: &[u8]) -> Loc {
        let loc = Loc { pack: self.id, off: self.data.len() as u32, len: blob.len() as u32, kind, raw: raw as u32 };
        self.data.extend_from_slice(blob);
        self.entries.push((id, loc));
        loc
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// the complete pack file (with its trailer)
    pub fn finish(&mut self, keys: &Keys) -> Vec<u8> {
        let idx = seal_index(keys, &self.id, &self.entries);
        let mut out = core::mem::take(&mut self.data);
        out.extend_from_slice(&idx);
        out.extend_from_slice(&(idx.len() as u32).to_le_bytes());
        out.extend_from_slice(PACK_TRAILER);
        out
    }
}

impl Default for PackBuf {
    fn default() -> Self {
        PackBuf::new()
    }
}

pub fn pack_hex(p: &PackId) -> String {
    crypto::hex(p)
}

pub fn parse_pack(s: &str) -> Option<PackId> {
    let b = crypto::unhex(s)?;
    (b.len() == 16).then(|| {
        let mut p = [0u8; 16];
        p.copy_from_slice(&b);
        p
    })
}
