// stream -- the VisioDocument stream of a binary file: its header, the tree of streams inside it
// (pointers), their decompression, and the chunks of a chunk stream (vss/mod.rs says what the
// format is and where this knowledge comes from)
//
// The stream (all numbers little-endian):
//   0x00   "Visio (TM) Drawing\r\n", zeros
//   0x1A   the version (u16): 11 for Visio 2003-2010, 6 for Visio 2000/2002
//   0x1C   the stream's length (u32)
//   0x24   the pointer to the trailer, the root of the tree
// A pointer is 18 bytes: type (u32), a word nobody needs (u32), offset and length in the stream
// (u32 each), format (u16).  The format's bit 1 says the bytes are compressed (LZSS, below), its
// high nibble what they hold: 5 a list of further pointers, 0xD (also 0xC, 8) a run of chunks, 4
// and anything else plain bytes.  A compressed list or plain stream starts with a word of its
// own (the plain stream's length), a chunk stream does not.
//
// A list: its first word is the offset of the list's head, which holds the order's length
// (offset - 4), the number of pointers (offset), a word nobody needs and then the pointers; after
// them the order, as many indices into the pointers as the head says (the masters of a stencil
// in the order of the stencil).  Null pointers (type 0, length 0) fill gaps.
//
// A chunk (19 bytes of header): type (u32), ID (u32), list (u32), the length of its data (u32),
// level (u16) and a byte of flags.  A chunk with a list has eight bytes more data: the lengths of
// a header and of a list of child IDs, both inside the data.  Words of four zero bytes may follow
// a chunk (most chunks with a list or a formula have one); no chunk type is zero in its low
// byte, so they never hide a chunk, and zero words at the end of a stream are padding.  The levels nest
// the chunks: an object (shape, style, sheet) at level 0, its lists (sections) at level 1, their
// rows at level 2 (a style's at 2 and 3).

use alloc::vec::Vec;

use crate::vsdx::{Error, Limits};

/// the signature at the start of the stream
pub(super) const SIGNATURE: &[u8] = b"Visio (TM) Drawing\r\n";
/// the most nested lists of pointers (Visio writes five)
const DEPTH: usize = 12;
/// the bytes of a pointer
const POINTER: usize = 18;
/// the bytes of a chunk's header
const HEADER: usize = 19;

pub(super) type R<T> = Result<T, Error>;

/// an error of the binary structure at `offset` of the VisioDocument stream (or of a stream in it)
pub(super) fn bad(offset: usize, what: &'static str) -> Error {
    Error::Binary { offset: offset as u64, what }
}

pub(super) fn u16at(b: &[u8], p: usize) -> Option<u16> {
    b.get(p..p.checked_add(2)?).map(|x| u16::from_le_bytes([x[0], x[1]]))
}
pub(super) fn u32at(b: &[u8], p: usize) -> Option<u32> {
    b.get(p..p.checked_add(4)?).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}
pub(super) fn f64at(b: &[u8], p: usize) -> Option<f64> {
    let x = b.get(p..p.checked_add(8)?)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(x);
    Some(f64::from_le_bytes(a))
}

/// LZSS as the streams use it: a ring of 4096 bytes filled with zeros, written from 4078 on; a
/// flag byte tells for its next eight items (low bit first) whether each is a literal byte (1) or
/// a reference of two bytes (0): the ring position in the low byte and the high nibble of the
/// second, the length (3..18) in its low nibble.  A cut last group ends the data.  At most `max`
/// bytes come out (more is Limit): a reference can make 18 bytes of 2, so nothing grows more than
/// nine-fold, and `max` bounds even that.
pub(super) fn lzss(src: &[u8], max: usize) -> R<Vec<u8>> {
    let mut ring = [0u8; 4096];
    let mut r = 4096 - 18;
    let mut out = Vec::with_capacity(src.len().saturating_mul(2).min(max));
    let mut i = 0;
    while i < src.len() {
        let flags = src[i];
        i += 1;
        for bit in 0..8 {
            if i >= src.len() {
                break;
            }
            if flags & (1 << bit) != 0 {
                let c = src[i];
                i += 1;
                if out.len() >= max {
                    return Err(Error::Limit("stream"));
                }
                out.push(c);
                ring[r] = c;
                r = (r + 1) & 4095;
            } else {
                let (Some(&a), Some(&b)) = (src.get(i), src.get(i + 1)) else {
                    return Ok(out);
                };
                i += 2;
                let pos = a as usize | ((b as usize & 0xF0) << 4);
                let len = (b as usize & 0x0F) + 3;
                if out.len() + len > max {
                    return Err(Error::Limit("stream"));
                }
                for k in 0..len {
                    let c = ring[(pos + k) & 4095];
                    out.push(c);
                    ring[r] = c;
                    r = (r + 1) & 4095;
                }
            }
        }
    }
    Ok(out)
}

/// a pointer as written
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pointer {
    pub ty: u32,
    pub offset: u32,
    pub length: u32,
    pub format: u16,
}

impl Pointer {
    fn at(b: &[u8], p: usize) -> Option<Pointer> {
        Some(Pointer { ty: u32at(b, p)?, offset: u32at(b, p + 8)?, length: u32at(b, p + 12)?, format: u16at(b, p + 16)? })
    }

    fn compressed(&self) -> bool {
        self.format & 2 != 0
    }
}

/// what a stream holds
#[derive(Debug)]
pub(super) enum Body {
    /// plain bytes
    Blob(Vec<u8>),
    /// further streams: in pointer order (gaps left out), and the order the list gives (indices
    /// into the pointers as written, gaps counted)
    List { items: Vec<(usize, Node)>, order: Vec<u32> },
    /// a run of chunks
    Chunks(Vec<u8>),
}

/// a stream of the tree
#[derive(Debug)]
pub(super) struct Node {
    /// the pointer's type
    pub ty: u32,
    pub body: Body,
}

impl Node {
    /// the streams of a list of type `ty` (none for anything else)
    pub fn kids(&self, ty: u32) -> impl Iterator<Item = &Node> {
        let items: &[(usize, Node)] = match &self.body {
            Body::List { items, .. } => items,
            _ => &[],
        };
        items.iter().map(|(_, n)| n).filter(move |n| n.ty == ty)
    }

    /// the first stream of type `ty` in a list
    pub fn kid(&self, ty: u32) -> Option<&Node> {
        self.kids(ty).next()
    }

    /// the bytes of a plain stream
    pub fn blob(&self) -> Option<&[u8]> {
        match &self.body {
            Body::Blob(b) => Some(b),
            _ => None,
        }
    }

    /// the bytes of a chunk stream
    pub fn chunk_bytes(&self) -> Option<&[u8]> {
        match &self.body {
            Body::Chunks(b) => Some(b),
            _ => None,
        }
    }
}

/// the whole tree of a VisioDocument stream and its version
pub(super) struct Tree {
    pub version: u16,
    pub root: Node,
}

// what the walk counts against the limits
struct Walk<'a> {
    doc: &'a [u8],
    lim: &'a Limits,
    // the bytes of all streams so far
    total: usize,
    // the streams so far
    streams: usize,
    // the streams read (offset, length): a file that points at one twice is refused, so no tree
    // of pointers grows beyond the file
    used: Vec<(u32, u32)>,
}

/// read the header and the tree of streams of a VisioDocument stream
pub(super) fn tree(doc: &[u8], lim: &Limits) -> R<Tree> {
    if !doc.starts_with(SIGNATURE) || doc.len() < 0x36 {
        return Err(bad(0, "no VisioDocument stream"));
    }
    let version = u16at(doc, 0x1A).ok_or_else(|| bad(0x1A, "header"))?;
    if version != 11 {
        return Err(Error::Binary { offset: 0x1A, what: if version == 6 { "version 6 (Visio 2000/2002)" } else { "version" } });
    }
    let trailer = Pointer::at(doc, 0x24).ok_or_else(|| bad(0x24, "header"))?;
    let mut w = Walk { doc, lim, total: 0, streams: 0, used: Vec::new() };
    let root = w.node(trailer, 0)?;
    Ok(Tree { version, root })
}

impl Walk<'_> {
    fn node(&mut self, p: Pointer, depth: usize) -> R<Node> {
        if depth > DEPTH {
            return Err(bad(p.offset as usize, "pointers nested too deeply"));
        }
        self.streams += 1;
        if self.streams > self.lim.objects.saturating_add(self.lim.sections) {
            return Err(Error::Limit("objects"));
        }
        let (start, len) = (p.offset as usize, p.length as usize);
        let raw = start.checked_add(len).and_then(|end| self.doc.get(start..end)).ok_or_else(|| bad(start, "a pointer outside the stream"))?;
        if len > 0 {
            if self.used.contains(&(p.offset, p.length)) {
                return Err(bad(start, "a stream pointed at twice"));
            }
            self.used.push((p.offset, p.length));
        }
        let max = self.lim.zip.part_bytes.min(self.lim.zip.total_bytes.saturating_sub(self.total));
        let bytes = if p.compressed() {
            lzss(raw, max)?
        } else {
            if raw.len() > max {
                return Err(Error::Limit("stream"));
            }
            raw.to_vec()
        };
        self.total += bytes.len();
        let kind = p.format >> 4;
        let body = match kind {
            0x8 | 0xC | 0xD => Body::Chunks(bytes),
            0x5 => self.list(&bytes, p, depth)?,
            _ => {
                let b = if p.compressed() {
                    // the length word, then the bytes
                    let n = u32at(&bytes, 0).unwrap_or(0) as usize;
                    bytes.get(4..).map(|b| b[..n.min(b.len())].to_vec()).unwrap_or_default()
                } else {
                    bytes
                };
                Body::Blob(b)
            }
        };
        Ok(Node { ty: p.ty, body })
    }

    fn list(&mut self, bytes: &[u8], p: Pointer, depth: usize) -> R<Body> {
        let base = p.offset as usize;
        let shift = if p.compressed() { 4 } else { 0 };
        if bytes.is_empty() {
            return Ok(Body::List { items: Vec::new(), order: Vec::new() });
        }
        let head = u32at(bytes, shift).ok_or_else(|| bad(base, "a list without head"))? as usize;
        let at = head.checked_add(shift).filter(|&a| a >= 4).ok_or_else(|| bad(base, "a list's head"))?;
        let order_len = u32at(bytes, at - 4).ok_or_else(|| bad(base, "a list's head"))? as usize;
        let count = u32at(bytes, at).ok_or_else(|| bad(base, "a list's head"))? as usize;
        let first = at + 8;
        let end = count.checked_mul(POINTER).and_then(|n| n.checked_add(first)).filter(|&e| e <= bytes.len()).ok_or_else(|| bad(base, "a list longer than its stream"))?;
        let order_end = order_len.checked_mul(4).and_then(|n| n.checked_add(end)).filter(|&e| e <= bytes.len()).ok_or_else(|| bad(base, "a list's order"))?;
        let mut items = Vec::new();
        for i in 0..count {
            let q = Pointer::at(bytes, first + i * POINTER).ok_or_else(|| bad(base, "a pointer"))?;
            if q.length == 0 && (q.ty == 0 || q.offset == 0) {
                continue;
            }
            items.push((i, self.node(q, depth + 1)?));
        }
        let order = (0..order_len).filter_map(|k| u32at(bytes, end + 4 * k)).filter(|&ix| (ix as usize) < count).collect();
        let _ = order_end;
        Ok(Body::List { items, order })
    }
}

/// a chunk of a chunk stream: its header and where its data lies in the stream
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Chunk {
    pub ty: u32,
    pub id: u32,
    pub list: u32,
    pub level: u16,
    pub flags: u8,
    /// the data (with a list: the eight bytes of lengths, the header and the child IDs)
    pub start: usize,
    pub end: usize,
}

impl Chunk {
    /// the data
    pub fn data<'b>(&self, s: &'b [u8]) -> &'b [u8] {
        &s[self.start..self.end]
    }

    /// a list chunk's header (between the lengths and the child IDs) and its child IDs
    pub fn list_parts<'b>(&self, s: &'b [u8]) -> Option<(&'b [u8], Vec<u32>)> {
        let d = self.data(s);
        if self.list == 0 {
            return Some((d, Vec::new()));
        }
        let h = u32at(d, 0)? as usize;
        let n = u32at(d, 4)? as usize;
        let head = d.get(8..8usize.checked_add(h)?)?;
        let ids = d.get(8 + h..(8 + h).checked_add(n)?)?;
        Some((head, ids.chunks_exact(4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]])).collect()))
    }
}

/// the chunks of a chunk stream in their order (`max`: the most chunks; more is Limit)
pub(super) fn chunks(s: &[u8], max: usize) -> R<Vec<Chunk>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s[i..].iter().all(|&b| b == 0) {
            break;
        }
        if out.len() >= max {
            return Err(Error::Limit("items"));
        }
        let h = s.get(i..i + HEADER).ok_or_else(|| bad(i, "a cut chunk"))?;
        let ty = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
        let id = u32::from_le_bytes([h[4], h[5], h[6], h[7]]);
        let list = u32::from_le_bytes([h[8], h[9], h[10], h[11]]);
        let len = u32::from_le_bytes([h[12], h[13], h[14], h[15]]) as usize;
        let level = u16::from_le_bytes([h[16], h[17]]);
        let flags = h[18];
        let start = i + HEADER;
        let end = len
            .checked_add(if list != 0 { 8 } else { 0 })
            .and_then(|n| n.checked_add(start))
            .filter(|&e| e <= s.len())
            .ok_or_else(|| bad(i, "a chunk longer than its stream"))?;
        if level > 3 || ty == 0 {
            return Err(bad(i, "a chunk's header"));
        }
        let c = Chunk { ty, id, list, level, flags, start, end };
        if list != 0 {
            c.list_parts(s).ok_or_else(|| bad(i, "a chunk's list"))?;
        }
        out.push(c);
        i = end;
        // separators: words of four zero bytes (no chunk type is zero in its low byte)
        while s.get(i..i + 4) == Some(&[0u8; 4][..]) {
            i += 4;
        }
    }
    Ok(out)
}
