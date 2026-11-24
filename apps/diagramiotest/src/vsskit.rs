// vsskit -- (JD17) VisioDocument streams of the binary formats (.vss, .vsd, .vst) built for the
// tests only, after the head comment of lib/diagram_io/src/vss/stream.rs (our own analysis; no
// other implementation): the header, the tree of streams behind pointers, plain, list and chunk
// streams, LZSS, chunks.  Everything can be put together freely, also wrongly: a stream can carry
// any bytes under any format, a list any pointers (gaps, the same pointer twice, pointers made
// up), a chunk any header.
//
//   Doc { version: 11, root }.build() -> the bytes of the stream VisioDocument
//   Stream::list(ty, vec![Entry::S(..), Entry::Null, ..]) / blob / chunks / raw, .packed(Pack::Repeats)
//   lzss(&data, Pack::Repeats) / bomb(n)        LZSS as the streams use it
//   chunk / list_chunk / chunk_raw / SEP        chunks and the zero words between them
//   stencil(&masters) -> Stream                 a small stencil: names, masters, a shape each
//   file(&doc) -> Vec<u8>                       a compound file with the stream VisioDocument
//
// The stream's layout (stream.rs): "Visio (TM) Drawing\r\n" at 0, the version (u16) at 0x1A, the
// length (u32) at 0x1C, the pointer to the trailer (the root of the tree) at 0x24.  A pointer is
// 18 bytes: type, a word, offset, length (u32 each), format (u16; bit 1: LZSS, the high nibble:
// 5 list, 0xD chunks, 4 plain).  A compressed list or plain stream has a length word before its
// bytes, a chunk stream not.  A list: [u32 head offset][..][order length at head - 4][count at
// head][a word][count pointers][order: u32 indices].  Streams are written children first, so
// every pointer names bytes already there; the header is 0x40 bytes.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub const SIGNATURE: &[u8] = b"Visio (TM) Drawing\r\n";
/// the bytes before the first stream
pub const HEADER: usize = 0x40;

pub const F_PLAIN: u16 = 0x40;
pub const F_LIST: u16 = 0x50;
pub const F_CHUNKS: u16 = 0xD0;
pub const COMPRESSED: u16 = 2;

// the stream types the reader looks at (vss/mod.rs)
pub const T_TRAILER: u32 = 0x14;
pub const T_STYLES: u32 = 0x1A;
pub const T_STENCILS: u32 = 0x1D;
pub const T_MASTER: u32 = 0x1E;
pub const T_NAMES: u32 = 0x32;
pub const T_NAME: u32 = 0x33;
pub const T_NAME_INDEX: u32 = 0xC9;
pub const T_PAGE_SHEET: u32 = 0x46;
pub const T_SHAPE: u32 = 0x48;
pub const T_PROMPT: u32 = 0x0A;

/// a pointer as written: type, a word, offset, length, format
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ptr {
    pub ty: u32,
    pub word: u32,
    pub offset: u32,
    pub length: u32,
    pub format: u16,
}

impl Ptr {
    pub fn bytes(&self) -> [u8; 18] {
        let mut b = [0u8; 18];
        b[0..4].copy_from_slice(&self.ty.to_le_bytes());
        b[4..8].copy_from_slice(&self.word.to_le_bytes());
        b[8..12].copy_from_slice(&self.offset.to_le_bytes());
        b[12..16].copy_from_slice(&self.length.to_le_bytes());
        b[16..18].copy_from_slice(&self.format.to_le_bytes());
        b
    }
}

/// how a stream's bytes are stored
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pack {
    /// as they are
    None,
    /// LZSS of literals only
    Literals,
    /// LZSS with references to repeated bytes (greedy, the last 256 positions of the ring)
    Repeats,
}

/// what a stream holds
#[derive(Clone, Debug)]
pub enum Body {
    /// plain bytes (packed: the length word before them)
    Blob(Vec<u8>),
    /// further streams and their order (indices into the entries)
    List { items: Vec<Entry>, order: Vec<u32> },
    /// a run of chunks (chunk, list_chunk, SEP)
    Chunks(Vec<u8>),
    /// these bytes under any format, nothing added (packed: LZSS of them)
    Raw(Vec<u8>),
}

/// a stream of the tree
#[derive(Clone, Debug)]
pub struct Stream {
    pub ty: u32,
    /// the format without bit 1 (Pack sets it); Raw streams keep theirs as given
    pub format: u16,
    /// the pointer's word nobody needs
    pub word: u32,
    pub pack: Pack,
    pub body: Body,
}

/// an entry of a list
#[derive(Clone, Debug)]
pub enum Entry {
    /// a null pointer (a gap)
    Null,
    /// a stream written for this entry
    S(Stream),
    /// the pointer of entry k of the same list again
    Same(usize),
    /// a pointer as given (made up)
    Ptr(Ptr),
}

impl Stream {
    pub fn blob(ty: u32, data: &[u8]) -> Stream {
        Stream { ty, format: F_PLAIN, word: 0, pack: Pack::None, body: Body::Blob(data.to_vec()) }
    }
    pub fn list(ty: u32, items: Vec<Entry>) -> Stream {
        Stream { ty, format: F_LIST, word: 0, pack: Pack::None, body: Body::List { items, order: Vec::new() } }
    }
    pub fn chunks(ty: u32, data: &[u8]) -> Stream {
        Stream { ty, format: F_CHUNKS, word: 0, pack: Pack::None, body: Body::Chunks(data.to_vec()) }
    }
    /// raw bytes under `format` exactly (bit 1 included)
    pub fn raw(ty: u32, format: u16, data: &[u8]) -> Stream {
        Stream { ty, format, word: 0, pack: Pack::None, body: Body::Raw(data.to_vec()) }
    }
    pub fn packed(mut self, p: Pack) -> Stream {
        self.pack = p;
        self
    }
    pub fn order(mut self, o: &[u32]) -> Stream {
        if let Body::List { order, .. } = &mut self.body {
            *order = o.to_vec();
        }
        self
    }
    pub fn s(self) -> Entry {
        Entry::S(self)
    }
}

/// the whole stream VisioDocument
#[derive(Clone, Debug)]
pub struct Doc {
    pub version: u16,
    pub root: Stream,
}

fn p32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}

impl Doc {
    pub fn new(root: Stream) -> Doc {
        Doc { version: 11, root }
    }

    /// the bytes: header, streams (children first), the length at 0x1C
    pub fn build(&self) -> Vec<u8> {
        let mut out = vec![0u8; HEADER];
        out[..SIGNATURE.len()].copy_from_slice(SIGNATURE);
        out[0x1A..0x1C].copy_from_slice(&self.version.to_le_bytes());
        let p = place(&mut out, &self.root);
        out[0x24..0x36].copy_from_slice(&p.bytes());
        let n = out.len() as u32;
        out[0x1C..0x20].copy_from_slice(&n.to_le_bytes());
        out
    }
}

/// a list's bytes (without the length word of a packed one): head at 8
pub fn list_bytes(ptrs: &[Ptr], order: &[u32]) -> Vec<u8> {
    let mut b = Vec::new();
    p32(&mut b, 8);
    p32(&mut b, order.len() as u32);
    p32(&mut b, ptrs.len() as u32);
    p32(&mut b, 0);
    for p in ptrs {
        b.extend_from_slice(&p.bytes());
    }
    for o in order {
        p32(&mut b, *o);
    }
    b
}

// write a stream (its children first) -> its pointer
fn place(out: &mut Vec<u8>, s: &Stream) -> Ptr {
    let packed = s.pack != Pack::None;
    let content: Vec<u8> = match &s.body {
        Body::Blob(d) => {
            if packed {
                let mut b = Vec::new();
                p32(&mut b, d.len() as u32);
                b.extend_from_slice(d);
                b
            } else {
                d.clone()
            }
        }
        Body::Chunks(d) | Body::Raw(d) => d.clone(),
        Body::List { items, order } => {
            let mut ptrs: Vec<Ptr> = Vec::new();
            for e in items {
                let p = match e {
                    Entry::Null => Ptr::default(),
                    Entry::S(c) => place(out, c),
                    Entry::Same(k) => ptrs.get(*k).copied().unwrap_or_default(),
                    Entry::Ptr(p) => *p,
                };
                ptrs.push(p);
            }
            let l = list_bytes(&ptrs, order);
            if packed {
                let mut b = Vec::new();
                p32(&mut b, l.len() as u32);
                b.extend_from_slice(&l);
                b
            } else {
                l
            }
        }
    };
    let bytes = if packed { lzss(&content, s.pack) } else { content };
    let format = match s.body {
        Body::Raw(_) => s.format,
        _ => (s.format & !COMPRESSED) | if packed { COMPRESSED } else { 0 },
    };
    let p = Ptr { ty: s.ty, word: s.word, offset: out.len() as u32, length: bytes.len() as u32, format };
    out.extend_from_slice(&bytes);
    p
}

// ---- LZSS

/// LZSS as the streams use it (stream.rs): a ring of 4096 bytes of zeros written from 4078 on;
/// a flag byte for eight items (low bit first, 1 a literal); a reference is two bytes, the ring
/// position (12 bits: the low byte and the second's high nibble) and the length - 3 (its low
/// nibble).  Pack::None gives the bytes back unchanged.
pub fn lzss(data: &[u8], pack: Pack) -> Vec<u8> {
    if pack == Pack::None {
        return data.to_vec();
    }
    let mut ring = [0u8; 4096];
    let mut r = 4096 - 18;
    let mut out = Vec::with_capacity(data.len() + data.len() / 8 + 1);
    let mut i = 0;
    while i < data.len() {
        let flag_at = out.len();
        out.push(0);
        let mut flags = 0u8;
        for bit in 0..8 {
            if i >= data.len() {
                break;
            }
            let (pos, len) = if pack == Pack::Repeats { best(&ring, r, &data[i..]) } else { (0, 0) };
            if len >= 3 {
                out.push(pos as u8);
                out.push((((pos >> 4) & 0xF0) | (len - 3)) as u8);
                // as the reader copies: from the ring, each byte written back at once
                for k in 0..len {
                    let c = ring[(pos + k) & 4095];
                    ring[r] = c;
                    r = (r + 1) & 4095;
                }
                i += len;
            } else {
                flags |= 1 << bit;
                out.push(data[i]);
                ring[r] = data[i];
                r = (r + 1) & 4095;
                i += 1;
            }
        }
        out[flag_at] = flags;
    }
    out
}

// the longest match (3..18) of `rest` among the last 256 positions of the ring -> (position,
// length); the copy may run into the bytes it writes, as the reader's does
fn best(ring: &[u8; 4096], r: usize, rest: &[u8]) -> (usize, usize) {
    let mut found = (0, 0);
    for back in 1..=256usize {
        let p = (r + 4096 - back) & 4095;
        let mut len = 0;
        while len < 18 && len < rest.len() {
            let q = (p + len) & 4095;
            let d = q.wrapping_sub(r) & 4095;
            let c = if d < len { rest[d] } else { ring[q] };
            if c != rest[len] {
                break;
            }
            len += 1;
        }
        if len > found.1 {
            found = (p, len);
            if len == 18 {
                break;
            }
        }
    }
    found
}

/// LZSS that makes at least `n` zero bytes out of about n / 8.5: references of 18 zeros
pub fn bomb(n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut made = 0;
    while made < n {
        out.push(0);
        for _ in 0..8 {
            out.push(0x00);
            out.push(0x0F);
            made += 18;
        }
    }
    out
}

// ---- chunks

/// the zero word that may follow a chunk
pub const SEP: [u8; 4] = [0; 4];

/// a chunk without list: [type][ID][list 0][length][level u16][flags u8] and its data
pub fn chunk(ty: u32, id: u32, level: u16, data: &[u8]) -> Vec<u8> {
    chunk_raw(ty, id, 0, data.len() as u32, level, 0, data)
}

/// a chunk with a list (`list` != 0): the lengths of `head` and of the child IDs, `head`, the IDs
pub fn list_chunk(ty: u32, id: u32, list: u32, level: u16, head: &[u8], ids: &[u32]) -> Vec<u8> {
    let mut d = Vec::new();
    p32(&mut d, head.len() as u32);
    p32(&mut d, ids.len() as u32 * 4);
    d.extend_from_slice(head);
    for i in ids {
        p32(&mut d, *i);
    }
    let len = (d.len() - 8) as u32;
    chunk_raw(ty, id, list.max(1), len, level, 0, &d)
}

/// a chunk header as given (the length need not fit) and these data after it
pub fn chunk_raw(ty: u32, id: u32, list: u32, len: u32, level: u16, flags: u8, data: &[u8]) -> Vec<u8> {
    let mut b = Vec::new();
    p32(&mut b, ty);
    p32(&mut b, id);
    p32(&mut b, list);
    p32(&mut b, len);
    b.extend_from_slice(&level.to_le_bytes());
    b.push(flags);
    b.extend_from_slice(data);
    b
}

// ---- a small stencil

/// a NUL-terminated UTF-16 string
pub fn utf16z(s: &str) -> Vec<u8> {
    let mut b: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    b.extend_from_slice(&[0, 0]);
    b
}

/// a row of the XForm section as cells.rs reads it today (0x9B: PinX .. Angle as unit byte and
/// double, FlipX, FlipY, ResizeMode, the byte 1, a mask of two bytes with every cell local)
pub fn xform(w: f64, h: f64) -> Vec<u8> {
    let mut d = Vec::new();
    for v in [w / 2.0, h / 2.0, w, h, w / 2.0, h / 2.0] {
        d.push(0x41);
        d.extend_from_slice(&v.to_le_bytes());
    }
    d.push(0x51);
    d.extend_from_slice(&0f64.to_le_bytes());
    d.extend_from_slice(&[0, 0, 0, 1, 0xFF, 0x03]);
    d
}

/// the chunk stream of one shape: the shape (0x48, level 0, a list of one section), its
/// property list (0x68, level 1) with an XForm row (level 2)
pub fn shape_chunks(id: u32, w: f64, h: f64) -> Vec<u8> {
    let mut s = list_chunk(T_SHAPE, id, 1, 0, &[], &[id * 100 + 1]);
    s.extend_from_slice(&SEP);
    s.extend(list_chunk(0x68, id * 100 + 1, 1, 1, &[], &[id * 100 + 2]));
    s.extend(chunk(0x9B, id * 100 + 2, 2, &xform(w, h)));
    s.extend_from_slice(&SEP);
    s
}

/// a stencil of masters (Name, NameU): the name list (packed), the stencils list with a master
/// list each (page sheet, prompt, one shape of 1 x 0.5 in at pointer index 3) and the masters'
/// name index; the masters' IDs are their pointer indices 0, 1, ..
pub fn stencil(masters: &[(&str, &str)]) -> Stream {
    let mut names = Vec::new();
    for (n, nu) in masters {
        for s in [n, nu] {
            let mut b = Vec::new();
            p32(&mut b, 1);
            b.extend(utf16z(s));
            names.push(Stream::blob(T_NAME, &b).packed(Pack::Repeats).s());
        }
    }
    let mut items = Vec::new();
    let mut index = Vec::new();
    p32(&mut index, masters.len() as u32);
    for (k, _) in masters.iter().enumerate() {
        let sheet = chunk(T_PAGE_SHEET, 1, 0, &[0u8; 16]);
        let m = Stream::list(
            T_MASTER,
            vec![
                Stream::chunks(T_PAGE_SHEET, &sheet).packed(Pack::Repeats).s(),
                Entry::Null,
                Stream::blob(T_PROMPT, &utf16z(&alloc::format!("Hinweis {k}"))).s(),
                Stream::chunks(T_SHAPE, &shape_chunks(3, 1.0, 0.5)).packed(Pack::Repeats).s(),
            ],
        )
        .packed(Pack::Repeats);
        items.push(m.s());
        p32(&mut index, 2 * k as u32);
        p32(&mut index, 2 * k as u32 + 1);
        p32(&mut index, k as u32);
        index.push(0);
    }
    items.push(Stream::blob(T_NAME_INDEX, &index).s());
    let order: Vec<u32> = (0..masters.len() as u32).collect();
    let list = Stream::list(T_STENCILS, items).order(&order).packed(Pack::Repeats);
    Stream::list(T_TRAILER, vec![Stream::list(T_NAMES, names).packed(Pack::Literals).s(), list.s()])
}

/// a compound file (version 3) with `doc` as its stream VisioDocument
pub fn file(doc: &[u8]) -> Vec<u8> {
    crate::cfbkit::Cfb { kids: vec![crate::cfbkit::stream("VisioDocument", doc)], ..Default::default() }.build().bytes
}

/// a stencil's VisioDocument bytes with `extra` streams in the trailer besides the stencil
pub fn with(extra: Vec<Entry>) -> Vec<u8> {
    let mut root = stencil(&[("Kasten", "Box"), ("Kreis", "Circle")]);
    if let Body::List { items, .. } = &mut root.body {
        items.extend(extra);
    }
    Doc::new(root).build()
}

/// a String of a byte string (for messages)
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| alloc::format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}
