// dict -- Zstandard dictionaries (RFC 8878 section 5)
//
//   Dictionary::parse(bytes)   the format of `zstd --train` (magic EC30A437, the ID, the
//                              literals' Huffman table, the FSE tables of the offset, match
//                              length and literal length codes, three repeat offsets, the
//                              content), or bytes without the magic as raw content (ID 0)
//   Dictionary::raw(content)   content only: matches may reach into it, no tables
//   Dictionary::train(samples, max_size)   train.rs
//   to_bytes / id / content
//
// A frame that names a dictionary starts with its tables and repeat offsets, and its content
// stands before the frame's output, where matches may reach.  Every table is checked as a
// frame's would be: a damaged dictionary is refused, never a panic.

use alloc::boxed::Box;
use alloc::vec::Vec;

use super::fse::{self, CTable, DTable};
use super::huf;
use super::seq::*;

pub(super) const MAGIC: u32 = 0xEC30_A437;

/// a table of a sequence code: the decoder's and the encoder's (for the repeat mode)
#[derive(Clone)]
pub(super) struct Tab {
    pub norm: Vec<i16>,
    pub log: u32,
    pub ct: CTable,
}

impl Tab {
    pub(super) fn new(norm: Vec<i16>, log: u32) -> Option<Tab> {
        let ct = CTable::build(&norm, log)?;
        DTable::build(&norm, log)?;
        Some(Tab { norm, log, ct })
    }
}

/// the tables of a dictionary in the zstd format
#[derive(Clone)]
pub(super) struct Entropy {
    pub huf: huf::Table,
    pub code: huf::Code,
    pub of: Tab,
    pub ml: Tab,
    pub ll: Tab,
}

/// a Zstandard dictionary: content that matches may reach into, and for the trained
/// format the entropy tables and repeat offsets a frame starts with
#[derive(Clone)]
pub struct Dictionary {
    pub(super) id: u32,
    pub(super) content: Vec<u8>,
    pub(super) rep: [u32; 3],
    pub(super) ent: Option<Box<Entropy>>,
    bytes: Vec<u8>, // the whole dictionary as to_bytes gives it
}

fn rd32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

impl Dictionary {
    /// a dictionary of content only (ID 0): frames made with it name no dictionary, so the
    /// decoder must be given it
    pub fn raw(content: &[u8]) -> Dictionary {
        Dictionary { id: 0, content: content.to_vec(), rep: [1, 4, 8], ent: None, bytes: content.to_vec() }
    }

    /// a dictionary in the zstd format, or raw content when the bytes do not start with its
    /// magic (as libzstd reads them); None for a damaged zstd-format dictionary
    pub fn parse(b: &[u8]) -> Option<Dictionary> {
        if b.len() < 8 || rd32(b, 0)? != MAGIC {
            return Some(Dictionary::raw(b));
        }
        let id = rd32(b, 4)?;
        let mut p = 8;
        let (w, used) = huf::read_weights(&b[p..])?;
        p += used;
        let table = huf::from_weights(w.clone())?;
        let (full, _) = huf::full_weights(w)?;
        let code = huf::Code::from_weights(&full)?;
        let mut tab = |max_sym: usize, max_log: u32| -> Option<Tab> {
            let (norm, log, used) = fse::read_ncount(b.get(p..)?, max_sym, max_log)?;
            p += used;
            Tab::new(norm, log)
        };
        let of = tab(OF_MAX, OF_LOG_MAX)?;
        let ml = tab(ML_MAX, ML_LOG_MAX)?;
        let ll = tab(LL_MAX, LL_LOG_MAX)?;
        let rep = [rd32(b, p)?, rd32(b, p + 4)?, rd32(b, p + 8)?];
        p += 12;
        let content = b[p..].to_vec();
        if rep.iter().any(|&r| r == 0 || r as usize > content.len()) {
            return None;
        }
        Some(Dictionary { id, content, rep, ent: Some(Box::new(Entropy { huf: table, code, of, ml, ll })), bytes: b.to_vec() })
    }

    /// a zstd-format dictionary of its parts (train.rs)
    pub(super) fn build(id: u32, code: &huf::Code, of: Tab, ml: Tab, ll: Tab, content: Vec<u8>) -> Option<Dictionary> {
        let mut b = Vec::new();
        b.extend_from_slice(&MAGIC.to_le_bytes());
        b.extend_from_slice(&id.to_le_bytes());
        if !code.write_table(&mut b) {
            return None;
        }
        for t in [&of, &ml, &ll] {
            fse::write_ncount(&mut b, &t.norm, t.log);
        }
        for r in [1u32, 4, 8] {
            b.extend_from_slice(&r.to_le_bytes());
        }
        b.extend_from_slice(&content);
        // read back as a decoder would: the tables must survive their description
        let d = Dictionary::parse(&b)?;
        (d.content == content && d.id == id).then_some(d)
    }

    /// a dictionary of at most max_size bytes (at least 256; zstd's default is 112 640)
    /// trained on samples of the data it will compress, as `zstd --train` makes them
    /// (train.rs); None without enough samples
    pub fn train(samples: &[&[u8]], max_size: usize) -> Option<Dictionary> {
        super::train::train(samples, max_size)
    }

    /// the dictionary as a file of `zstd --train` (raw content: the content)
    pub fn to_bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    /// the ID frames name it by (0: raw content, named by none)
    pub fn id(&self) -> u32 {
        self.id
    }

    /// the content matches may reach into
    pub fn content(&self) -> &[u8] {
        &self.content
    }

    /// whether it has the trained format's entropy tables
    pub fn has_tables(&self) -> bool {
        self.ent.is_some()
    }
}

impl core::fmt::Debug for Dictionary {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Dictionary {{ id: {}, content: {} bytes, tables: {} }}", self.id, self.content.len(), self.ent.is_some())
    }
}
