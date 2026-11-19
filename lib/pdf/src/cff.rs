// cff -- CFF (version 1) subsetter for PDF: an OpenType font's bare 'CFF ' table, embedded as
// FontFile3 /Subtype /CIDFontType0C below a CIDFontType0 descendant font
//
//   info(cff) -> CffInfo { glyphs, cid_keyed } / Err   parses and checks everything subset()
//        reads: CFF2, CharstringType other than 2, font sets with more than one font and
//        malformed data are refused (so info() Ok = subset() works with a keep of that length)
//   subset(cff, keep) -> a CFF 1.0 with one font / Err   glyph count and order stay (the content
//        stream's CID is the glyph id); the charstring of a glyph with keep[gid] false becomes a
//        lone Type 2 endchar, the others are copied byte for byte; gid 0 (.notdef) always stays
//
// The output, in this order: the header (major and minor copied, hdrSize 4, offSize 4), the
// Name INDEX, the Top DICT INDEX, the String INDEX and the Global Subr INDEX (the three copied as
// they are), the charset, the FDSelect, the CharStrings INDEX, the FDArray INDEX, the Private
// DICTs, then the local Subr INDEXes (copied as they are).
//   Top DICT: every operator copied with its operands, except Encoding, UniqueID, XUID and
// UIDBase (dropped: the font is no longer the original; PDF selects glyphs by CID, not by
// encoding) and the offsets charset, CharStrings, Private (size and offset), FDArray and
// FDSelect: those are written as 5-byte integers (29 + i32), so every DICT's size is known
// before the offsets are.
//   Name-keyed fonts: the charset copied byte for byte (a predefined one, 0 / 1 / 2, stays that
// number; none stays none), FDArray / FDSelect dropped, the Private DICT rebuilt with Subrs (if
// it has any) pointing at its local Subr INDEX right after it.
//   CID-keyed fonts (ROS): the charset written as the identity (CID = GID, since a PDF reader
// finds the glyph of a CID through the charset; CIDCount raised to the glyph count if it was
// smaller), the FDSelect copied (formats 0 and 3, every font DICT index checked), the FDArray's
// Font DICTs copied with Private rewritten (stale offsets, UniqueID, XUID, UIDBase dropped there
// too), each Private DICT with its own local Subrs; a Private DICT in the Top DICT is dropped
// (CID-keyed fonts take theirs from the font DICTs).
//   Private DICTs: every operator copied except Subrs, rewritten as a 5-byte offset relative to
// the Private DICT (Subrs 0 means none, as in FreeType: dropped; a negative one is refused).
// Font DICTs that share a Private DICT share it in the output too, Private DICTs that share a
// Subr INDEX (the same table offset) share that one.  CharstringType is checked in the Top DICT
// and in every Font DICT.
//
// Not followed: seac accents (endchar with 4 arguments, Type 1 style) name their components by
// StandardEncoding code; the caller has to keep those glyphs.  Kept charstrings are not checked,
// every global and local subr stays.  Every read is bounds-checked; malformed data gives an Err,
// never a panic; no recursion; the output is limited to about twice the input (Private DICTs or
// Subr INDEXes that overlap without being the same are refused beyond that).

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

type Res<T> = Result<T, &'static str>;

/// what subset() needs to know about a CFF table
pub(crate) struct CffInfo {
    /// the glyph count (the CharStrings INDEX's)
    pub glyphs: u16,
    /// a CID-keyed font (ROS in the Top DICT): its charset maps CIDs to glyphs (subset writes it
    /// as the identity, so the PDF side treats both kinds alike and does not read this)
    #[allow(dead_code)]
    pub cid_keyed: bool,
}

// DICT operators: one byte, or 12 x as 1200 + x
const OP_UNIQUEID: u16 = 13;
const OP_XUID: u16 = 14;
const OP_CHARSET: u16 = 15;
const OP_ENCODING: u16 = 16;
const OP_CHARSTRINGS: u16 = 17;
const OP_PRIVATE: u16 = 18;
const OP_SUBRS: u16 = 19;
const OP_CHARSTRINGTYPE: u16 = 1206;
const OP_ROS: u16 = 1230;
const OP_CIDCOUNT: u16 = 1234;
const OP_UIDBASE: u16 = 1235;
const OP_FDARRAY: u16 = 1236;
const OP_FDSELECT: u16 = 1237;

/// operands before one DICT operator (the CFF specification's limit)
const MAX_OPERANDS: usize = 48;
/// the charstring of a glyph that is not kept: Type 2 endchar
const ENDCHAR: &[u8] = &[14];

const E_TRUNC: &str = "CFF: truncated or out of bounds";
const E_DICT: &str = "CFF: malformed DICT";

// ------------------------------------------------------------------ byte readers

/// n bytes at pos, or an error past the end
fn bytes(d: &[u8], pos: usize, n: usize) -> Res<&[u8]> {
    let end = pos.checked_add(n).ok_or(E_TRUNC)?;
    d.get(pos..end).ok_or(E_TRUNC)
}

/// big-endian unsigned value of at most 4 bytes
fn be(b: &[u8]) -> usize {
    b.iter().fold(0usize, |v, &x| (v << 8) | x as usize)
}

/// a DICT integer operand as a table offset (0 ..= len)
fn offset(v: i32, len: usize) -> Res<usize> {
    if v < 0 || v as usize > len {
        return Err(E_TRUNC);
    }
    Ok(v as usize)
}

// ------------------------------------------------------------------ INDEX

/// a CFF INDEX, checked: offsets start at 1, never decrease, end inside the table
#[derive(Clone, Copy)]
struct Index {
    count: usize,
    offsize: usize,
    /// the offset array's position
    offs: usize,
    /// the position of the byte at offset 1
    data: usize,
    /// the INDEX's first byte and the position after it (for copying it as it is)
    start: usize,
    end: usize,
}

impl Index {
    fn parse(d: &[u8], pos: usize) -> Res<Index> {
        let count = be(bytes(d, pos, 2)?);
        let after = pos + 2; // bytes() checked pos + 2 <= len
        if count == 0 {
            return Ok(Index { count: 0, offsize: 1, offs: after, data: after, start: pos, end: after });
        }
        let offsize = be(bytes(d, after, 1)?);
        if !(1..=4).contains(&offsize) {
            return Err("CFF: INDEX offSize not 1 to 4");
        }
        let offs = after + 1;
        let n = (count + 1) * offsize; // at most 65536 x 4
        let arr = bytes(d, offs, n)?;
        let mut last = 1usize;
        for (i, c) in arr.chunks_exact(offsize).enumerate() {
            let o = be(c);
            if (i == 0 && o != 1) || o < last {
                return Err("CFF: INDEX offsets not ascending from 1");
            }
            last = o;
        }
        let data = offs + n;
        let end = (data - 1).checked_add(last).ok_or(E_TRUNC)?;
        if end > d.len() {
            return Err(E_TRUNC);
        }
        Ok(Index { count, offsize, offs, data, start: pos, end })
    }

    /// item i's bytes (i < count)
    fn item<'a>(&self, d: &'a [u8], i: usize) -> Res<&'a [u8]> {
        if i >= self.count {
            return Err(E_TRUNC);
        }
        let p = self.offs + i * self.offsize;
        let a = be(bytes(d, p, self.offsize)?);
        let b = be(bytes(d, p + self.offsize, self.offsize)?);
        // parse() checked 1 <= a <= b and data - 1 + b <= len
        let s = self.data - 1 + a;
        bytes(d, s, b - a)
    }

    /// the whole INDEX as it is in the table
    fn raw<'a>(&self, d: &'a [u8]) -> Res<&'a [u8]> {
        d.get(self.start..self.end).ok_or(E_TRUNC)
    }
}

/// the size of an INDEX of count items with data_len bytes of data
fn index_size(count: usize, data_len: usize) -> Res<usize> {
    if count == 0 {
        return Ok(2);
    }
    let os = offsize(data_len.checked_add(1).ok_or(E_TRUNC)?);
    count
        .checked_add(1)
        .and_then(|n| n.checked_mul(os))
        .and_then(|n| n.checked_add(3))
        .and_then(|n| n.checked_add(data_len))
        .ok_or("CFF: too large")
}

/// the smallest offSize that holds max
fn offsize(max: usize) -> usize {
    match max {
        0..=0xFF => 1,
        0x100..=0xFFFF => 2,
        0x1_0000..=0xFF_FFFF => 3,
        _ => 4,
    }
}

/// append an INDEX of items (at most 65535, data below 4 GB: the caller checked the sizes)
fn write_index(out: &mut Vec<u8>, items: &[&[u8]]) {
    out.extend_from_slice(&(items.len() as u16).to_be_bytes());
    if items.is_empty() {
        return;
    }
    let total: usize = items.iter().map(|i| i.len()).sum();
    let os = offsize(total + 1);
    out.push(os as u8);
    let mut o = 1usize;
    out.extend_from_slice(&(o as u32).to_be_bytes()[4 - os..]);
    for it in items {
        o += it.len();
        out.extend_from_slice(&(o as u32).to_be_bytes()[4 - os..]);
    }
    for it in items {
        out.extend_from_slice(it);
    }
}

// ------------------------------------------------------------------ DICT

#[derive(Clone, Copy)]
enum Tok {
    Op(u16),
    Int(i32),
    Real,
}

/// the token at i of a DICT and the position after it
fn token(d: &[u8], i: usize) -> Res<(Tok, usize)> {
    let b0 = *d.get(i).ok_or(E_DICT)?;
    let at = |k: usize| d.get(i + k).map(|&b| b as i32).ok_or(E_DICT);
    Ok(match b0 {
        12 => (Tok::Op(1200 + at(1)? as u16), i + 2),
        0..=11 | 13..=21 => (Tok::Op(b0 as u16), i + 1),
        28 => (Tok::Int((((at(1)? << 8) | at(2)?) as u16 as i16) as i32), i + 3),
        29 => {
            let b = d.get(i + 1..i + 5).ok_or(E_DICT)?;
            (Tok::Int(i32::from_be_bytes([b[0], b[1], b[2], b[3]])), i + 5)
        }
        30 => {
            // nibbles up to and including one 0xf; 0xd is reserved
            let mut j = i + 1;
            loop {
                let b = *d.get(j).ok_or(E_DICT)?;
                j += 1;
                let (hi, lo) = (b >> 4, b & 15);
                if hi == 0xd || (hi != 0xf && lo == 0xd) {
                    return Err("CFF: reserved nibble in a DICT real");
                }
                if hi == 0xf || lo == 0xf {
                    break;
                }
            }
            (Tok::Real, j)
        }
        32..=246 => (Tok::Int(b0 as i32 - 139), i + 1),
        247..=250 => (Tok::Int((b0 as i32 - 247) * 256 + at(1)? + 108), i + 2),
        251..=254 => (Tok::Int(-(b0 as i32 - 251) * 256 - at(1)? - 108), i + 2),
        _ => return Err("CFF: reserved byte in a DICT"), // 22 ..= 27, 31, 255
    })
}

/// one operator of a DICT: its operands start at `start`, the operator at `opp`, the next entry
/// at `end` (positions in the DICT)
#[derive(Clone, Copy)]
struct Entry {
    op: u16,
    start: usize,
    opp: usize,
    end: usize,
}

fn parse_dict(d: &[u8]) -> Res<Vec<Entry>> {
    let mut v = Vec::new();
    let (mut i, mut start, mut n) = (0usize, 0usize, 0usize);
    while i < d.len() {
        let (t, j) = token(d, i)?;
        match t {
            Tok::Op(op) => {
                v.push(Entry { op, start, opp: i, end: j });
                start = j;
                n = 0;
            }
            Tok::Int(_) | Tok::Real => {
                n += 1;
                if n > MAX_OPERANDS {
                    return Err("CFF: more than 48 operands in a DICT");
                }
            }
        }
        i = j;
    }
    if start != d.len() {
        return Err("CFF: DICT ends with operands");
    }
    Ok(v)
}

/// exactly N integer operands of e, else Err(what)
fn ints<const N: usize>(dict: &[u8], e: &Entry, what: &'static str) -> Res<[i32; N]> {
    let mut out = [0i32; N];
    let (mut k, mut i) = (0usize, e.start);
    while i < e.opp {
        let (t, j) = token(dict, i)?;
        match t {
            Tok::Int(v) if k < N => out[k] = v,
            _ => return Err(what),
        }
        k += 1;
        i = j;
    }
    if k != N {
        return Err(what);
    }
    Ok(out)
}

/// the entry of op, None when absent, Err when it is there twice
fn find(es: &[Entry], op: u16) -> Res<Option<Entry>> {
    let mut it = es.iter().filter(|e| e.op == op);
    let first = it.next().copied();
    if it.next().is_some() {
        return Err("CFF: an operator twice in a DICT");
    }
    Ok(first)
}

/// append a 5-byte integer operand
fn put_int5(out: &mut Vec<u8>, v: usize) {
    out.push(29);
    out.extend_from_slice(&(v as u32).to_be_bytes());
}

fn put_op(out: &mut Vec<u8>, op: u16) {
    if op >= 1200 {
        out.push(12);
        out.push((op - 1200) as u8);
    } else {
        out.push(op as u8);
    }
}

// ------------------------------------------------------------------ the parsed font

struct Private<'a> {
    dict: &'a [u8],
    es: Vec<Entry>,
    /// index into Font::subrs
    subrs: Option<usize>,
}

struct FontDict<'a> {
    dict: &'a [u8],
    es: Vec<Entry>,
    /// index into Font::privates
    private: Option<usize>,
}

struct Font<'a> {
    d: &'a [u8],
    major: u8,
    minor: u8,
    name: &'a [u8],
    strings: &'a [u8],
    gsubrs: &'a [u8],
    top: &'a [u8],
    top_es: Vec<Entry>,
    cs: Index,
    glyphs: usize,
    cid: bool,
    /// name-keyed: a custom charset as it is (empty: predefined or none)
    charset: &'a [u8],
    /// CID-keyed: the FDSelect as it is
    fdselect: &'a [u8],
    fds: Vec<FontDict<'a>>,
    /// name-keyed: the Top DICT's Private DICT
    top_private: Option<usize>,
    privates: Vec<Private<'a>>,
    priv_keys: BTreeMap<(usize, usize), usize>,
    subrs: Vec<&'a [u8]>,
    subrs_keys: BTreeMap<usize, usize>,
}

impl<'a> Font<'a> {
    /// the Private DICT of a Private operator's (size, offset): its index in privates
    fn private(&mut self, size: i32, off: i32) -> Res<usize> {
        if size < 0 {
            return Err(E_TRUNC);
        }
        let off = offset(off, self.d.len())?;
        let size = size as usize;
        if let Some(&i) = self.priv_keys.get(&(off, size)) {
            return Ok(i);
        }
        let dict = bytes(self.d, off, size)?;
        let es = parse_dict(dict)?;
        let subrs = match find(&es, OP_SUBRS)? {
            None => None,
            Some(e) => match ints::<1>(dict, &e, "CFF: Subrs operand")? {
                // relative to the Private DICT; 0 means none (FreeType, text's cff.rs), a negative
                // one is refused (FreeType fails on it, text's cff.rs ignores it)
                [0] => None,
                [rel] if rel < 0 => return Err("CFF: negative Subrs offset"),
                [rel] => {
                    let abs = offset(rel, self.d.len())?
                        .checked_add(off)
                        .filter(|&a| a <= self.d.len())
                        .ok_or(E_TRUNC)?;
                    match self.subrs_keys.get(&abs) {
                        Some(&i) => Some(i),
                        None => {
                            let raw = Index::parse(self.d, abs)?.raw(self.d)?;
                            self.subrs.push(raw);
                            self.subrs_keys.insert(abs, self.subrs.len() - 1);
                            Some(self.subrs.len() - 1)
                        }
                    }
                }
            },
        };
        self.privates.push(Private { dict, es, subrs });
        self.priv_keys.insert((off, size), self.privates.len() - 1);
        Ok(self.privates.len() - 1)
    }
}

/// a name-keyed font's custom charset at pos: its bytes (formats 0, 1, 2)
fn charset_raw(d: &[u8], pos: usize, glyphs: usize) -> Res<&[u8]> {
    let fmt = *d.get(pos).ok_or(E_TRUNC)?;
    let n = glyphs - 1; // every glyph but .notdef
    let len = match fmt {
        0 => 1 + 2 * n,
        1 | 2 => {
            let rl = if fmt == 1 { 3 } else { 4 };
            let (mut p, mut covered) = (pos + 1, 0usize);
            while covered < n {
                let r = bytes(d, p, rl)?;
                covered += 1 + if fmt == 1 { r[2] as usize } else { be(&r[2..4]) };
                p += rl;
            }
            p - pos
        }
        _ => return Err("CFF: unknown charset format"),
    };
    bytes(d, pos, len)
}

/// a CID-keyed font's FDSelect at pos: its bytes (formats 0 and 3), every font DICT index < nfd
fn fdselect_raw(d: &[u8], pos: usize, glyphs: usize, nfd: usize) -> Res<&[u8]> {
    let fmt = *d.get(pos).ok_or(E_TRUNC)?;
    let len = match fmt {
        0 => {
            if bytes(d, pos + 1, glyphs)?.iter().any(|&f| f as usize >= nfd) {
                return Err("CFF: FDSelect names a missing font DICT");
            }
            1 + glyphs
        }
        3 => {
            let nr = be(bytes(d, pos + 1, 2)?);
            if nr == 0 {
                return Err("CFF: FDSelect without ranges");
            }
            let r = bytes(d, pos + 3, nr * 3 + 2)?;
            let mut prev = 0usize;
            for (k, g) in r[..nr * 3].as_chunks::<3>().0.iter().enumerate() {
                let first = be(&g[..2]);
                if (k == 0 && first != 0) || (k > 0 && first <= prev) {
                    return Err("CFF: FDSelect ranges not ascending from 0");
                }
                if g[2] as usize >= nfd {
                    return Err("CFF: FDSelect names a missing font DICT");
                }
                prev = first;
            }
            let sentinel = be(&r[nr * 3..]);
            if sentinel <= prev || sentinel < glyphs {
                return Err("CFF: FDSelect sentinel below the glyph count");
            }
            3 + nr * 3 + 2
        }
        _ => return Err("CFF: unknown FDSelect format"),
    };
    bytes(d, pos, len)
}

/// a Top or Font DICT's CharstringType: absent or 2
fn charstring_type(dict: &[u8], es: &[Entry]) -> Res<()> {
    if let Some(e) = find(es, OP_CHARSTRINGTYPE)? {
        let [t] = ints::<1>(dict, &e, "CFF: CharstringType is not 2")?;
        if t != 2 {
            return Err("CFF: CharstringType is not 2");
        }
    }
    Ok(())
}

fn parse(d: &[u8]) -> Res<Font<'_>> {
    let hdr = bytes(d, 0, 4)?;
    match hdr[0] {
        1 => {}
        2 => return Err("CFF2"),
        _ => return Err("CFF: unknown major version"),
    }
    let hs = hdr[2] as usize;
    if hs < 4 {
        return Err("CFF: header size below 4");
    }
    let name = Index::parse(d, hs)?;
    match name.count {
        1 => {}
        0 => return Err("CFF: no font"),
        _ => return Err("CFF: more than one font"),
    }
    let topix = Index::parse(d, name.end)?;
    if topix.count != 1 {
        return Err("CFF: Top DICT INDEX does not hold one DICT");
    }
    let strings = Index::parse(d, topix.end)?;
    let gsubrs = Index::parse(d, strings.end)?;
    let top = topix.item(d, 0)?;
    let top_es = parse_dict(top)?;
    charstring_type(top, &top_es)?;
    let e = find(&top_es, OP_CHARSTRINGS)?.ok_or("CFF: no CharStrings")?;
    let [o] = ints::<1>(top, &e, "CFF: CharStrings operand")?;
    let cs = Index::parse(d, offset(o, d.len())?)?;
    if cs.count == 0 {
        return Err("CFF: no glyphs");
    }
    let glyphs = cs.count;
    let cid = find(&top_es, OP_ROS)?.is_some();
    let mut f = Font {
        d,
        major: hdr[0],
        minor: hdr[1],
        name: name.raw(d)?,
        strings: strings.raw(d)?,
        gsubrs: gsubrs.raw(d)?,
        top,
        top_es,
        cs,
        glyphs,
        cid,
        charset: &[],
        fdselect: &[],
        fds: Vec::new(),
        top_private: None,
        privates: Vec::new(),
        priv_keys: BTreeMap::new(),
        subrs: Vec::new(),
        subrs_keys: BTreeMap::new(),
    };
    // checked here, though only subset() needs them: info() Ok = subset() works
    find(&f.top_es, OP_CHARSET)?;
    find(&f.top_es, OP_PRIVATE)?;
    find(&f.top_es, OP_FDARRAY)?;
    find(&f.top_es, OP_FDSELECT)?;
    find(&f.top_es, OP_CIDCOUNT)?;
    if cid {
        let e = find(&f.top_es, OP_FDARRAY)?.ok_or("CFF: CID-keyed font without FDArray")?;
        let [o] = ints::<1>(top, &e, "CFF: FDArray operand")?;
        let fda = Index::parse(d, offset(o, d.len())?)?;
        if fda.count == 0 {
            return Err("CFF: empty FDArray");
        }
        for i in 0..fda.count {
            let dict = fda.item(d, i)?;
            let es = parse_dict(dict)?;
            charstring_type(dict, &es)?;
            let private = match find(&es, OP_PRIVATE)? {
                None => None,
                Some(e) => {
                    let [size, off] = ints::<2>(dict, &e, "CFF: Private operands")?;
                    Some(f.private(size, off)?)
                }
            };
            f.fds.push(FontDict { dict, es, private });
        }
        let e = find(&f.top_es, OP_FDSELECT)?.ok_or("CFF: CID-keyed font without FDSelect")?;
        let [o] = ints::<1>(top, &e, "CFF: FDSelect operand")?;
        f.fdselect = fdselect_raw(d, offset(o, d.len())?, glyphs, fda.count)?;
    } else {
        if let Some(e) = find(&f.top_es, OP_CHARSET)? {
            let [o] = ints::<1>(top, &e, "CFF: charset operand")?;
            if o > 2 {
                f.charset = charset_raw(d, offset(o, d.len())?, glyphs)?;
            } else if o < 0 {
                return Err("CFF: charset operand");
            }
        }
        if let Some(e) = find(&f.top_es, OP_PRIVATE)? {
            let [size, off] = ints::<2>(top, &e, "CFF: Private operands")?;
            f.top_private = Some(f.private(size, off)?);
        }
    }
    Ok(f)
}

// ------------------------------------------------------------------ writing

/// where the parts go in the output (pass 1: all 0, for the sizes)
#[derive(Default)]
struct Layout {
    charset: usize,
    fdselect: usize,
    charstrings: usize,
    fdarray: usize,
    /// per Private DICT: position and size
    privates: Vec<(usize, usize)>,
    /// per local Subr INDEX: position
    subrs: Vec<usize>,
}

impl Layout {
    fn zero(f: &Font) -> Layout {
        Layout {
            privates: f.privates.iter().map(|_| (0, 0)).collect(),
            subrs: f.subrs.iter().map(|_| 0).collect(),
            ..Layout::default()
        }
    }
}

/// the Private operator: size and offset as 5-byte integers
fn put_private(out: &mut Vec<u8>, l: &Layout, i: usize) {
    let (pos, size) = l.privates[i];
    put_int5(out, size);
    put_int5(out, pos);
    put_op(out, OP_PRIVATE);
}

fn top_dict(f: &Font, l: &Layout) -> Vec<u8> {
    let mut out = Vec::with_capacity(f.top.len() + 64);
    let mut charset_done = false;
    for e in &f.top_es {
        match e.op {
            OP_UNIQUEID | OP_XUID | OP_ENCODING | OP_UIDBASE => {}
            OP_CHARSET => {
                charset_done = true;
                if f.cid || !f.charset.is_empty() {
                    put_int5(&mut out, l.charset);
                    put_op(&mut out, OP_CHARSET);
                } else {
                    out.extend_from_slice(&f.top[e.start..e.end]); // predefined
                }
            }
            OP_CHARSTRINGS => {
                put_int5(&mut out, l.charstrings);
                put_op(&mut out, OP_CHARSTRINGS);
            }
            OP_PRIVATE => {
                if let Some(i) = f.top_private {
                    put_private(&mut out, l, i);
                }
            }
            OP_FDARRAY | OP_FDSELECT => {
                if f.cid {
                    put_int5(&mut out, if e.op == OP_FDARRAY { l.fdarray } else { l.fdselect });
                    put_op(&mut out, e.op);
                }
            }
            OP_CIDCOUNT => match ints::<1>(f.top, e, "") {
                Ok([n]) if f.cid && n >= 0 && (n as usize) < f.glyphs => {
                    put_int5(&mut out, f.glyphs);
                    put_op(&mut out, OP_CIDCOUNT);
                }
                _ => out.extend_from_slice(&f.top[e.start..e.end]),
            },
            _ => out.extend_from_slice(&f.top[e.start..e.end]),
        }
    }
    if f.cid && !charset_done {
        put_int5(&mut out, l.charset);
        put_op(&mut out, OP_CHARSET);
    }
    out
}

fn font_dict(fd: &FontDict, l: &Layout) -> Vec<u8> {
    let mut out = Vec::with_capacity(fd.dict.len() + 16);
    for e in &fd.es {
        match e.op {
            OP_PRIVATE => {
                if let Some(i) = fd.private {
                    put_private(&mut out, l, i);
                }
            }
            OP_UNIQUEID | OP_XUID | OP_UIDBASE | OP_CHARSET | OP_ENCODING | OP_CHARSTRINGS
            | OP_FDARRAY | OP_FDSELECT => {}
            _ => out.extend_from_slice(&fd.dict[e.start..e.end]),
        }
    }
    out
}

/// Private DICT i at its layout position (Subrs relative to it)
fn private_dict(f: &Font, l: &Layout, i: usize) -> Vec<u8> {
    let p = &f.privates[i];
    let mut out = Vec::with_capacity(p.dict.len() + 8);
    for e in &p.es {
        if e.op == OP_SUBRS {
            if let Some(s) = p.subrs {
                // Subr INDEXes follow every Private DICT: never before this one
                put_int5(&mut out, l.subrs[s].saturating_sub(l.privates[i].0));
                put_op(&mut out, OP_SUBRS);
            }
        } else {
            out.extend_from_slice(&p.dict[e.start..e.end]);
        }
    }
    out
}

/// the identity charset of a CID-keyed font: glyph g has CID g
fn identity_charset(glyphs: usize) -> Vec<u8> {
    if glyphs < 2 {
        return Vec::from([0u8]); // format 0, no glyph besides .notdef
    }
    let n = (glyphs - 2) as u16;
    let mut v = Vec::from([2u8, 0, 1]); // format 2, first CID 1
    v.extend_from_slice(&n.to_be_bytes());
    v
}

/// parse and check a CFF table: what subset() needs to know
pub(crate) fn info(cff: &[u8]) -> Result<CffInfo, &'static str> {
    let f = parse(cff)?;
    Ok(CffInfo { glyphs: f.glyphs as u16, cid_keyed: f.cid })
}

/// a CFF 1.0 with one font: keep[gid] true = charstring copied byte for byte, false = endchar;
/// gid 0 (.notdef) always stays; keep.len() must be the glyph count
pub(crate) fn subset(cff: &[u8], keep: &[bool]) -> Result<Vec<u8>, &'static str> {
    let f = parse(cff)?;
    if keep.len() != f.glyphs {
        return Err("CFF: keep does not match the glyph count");
    }
    let mut css: Vec<&[u8]> = Vec::with_capacity(f.glyphs);
    for (g, &k) in keep.iter().enumerate() {
        css.push(if k || g == 0 { f.cs.item(cff, g)? } else { ENDCHAR });
    }
    let cs_data: usize = css.iter().map(|c| c.len()).sum();
    let charset = if f.cid { identity_charset(f.glyphs) } else { f.charset.to_vec() };

    // pass 1: the sizes (they do not depend on the offsets)
    let mut l = Layout::zero(&f);
    for i in 0..f.privates.len() {
        l.privates[i].1 = private_dict(&f, &l, i).len();
    }
    let top_len = top_dict(&f, &l).len();
    let fd_data: usize = f.fds.iter().map(|fd| font_dict(fd, &l).len()).sum();
    let priv_sizes: Vec<usize> = l.privates.iter().map(|p| p.1).collect();

    let add = |a: usize, b: usize| a.checked_add(b).ok_or("CFF: too large");
    let mut pos = add(4, f.name.len())?;
    pos = add(pos, index_size(1, top_len)?)?;
    pos = add(pos, f.strings.len())?;
    pos = add(pos, f.gsubrs.len())?;
    l.charset = pos;
    pos = add(pos, charset.len())?;
    l.fdselect = pos;
    pos = add(pos, f.fdselect.len())?;
    l.charstrings = pos;
    pos = add(pos, index_size(f.glyphs, cs_data)?)?;
    l.fdarray = pos;
    if f.cid {
        pos = add(pos, index_size(f.fds.len(), fd_data)?)?;
    }
    for (i, &s) in priv_sizes.iter().enumerate() {
        l.privates[i] = (pos, s);
        pos = add(pos, s)?;
    }
    for (i, s) in f.subrs.iter().enumerate() {
        l.subrs[i] = pos;
        pos = add(pos, s.len())?;
    }
    // a sane font's parts are disjoint pieces of it: twice its size and the rewritten offsets
    // are plenty; more means overlapping Private DICTs or Subr INDEXes
    let cap = cff
        .len()
        .saturating_mul(2)
        .saturating_add(f.glyphs.saturating_mul(6))
        .saturating_add(f.fds.len().saturating_mul(32))
        .saturating_add(1024);
    if pos > cap || pos > i32::MAX as usize {
        return Err("CFF: subset would be too large");
    }

    // pass 2: the bytes
    let mut out = Vec::with_capacity(pos);
    out.extend_from_slice(&[f.major, f.minor, 4, 4]);
    out.extend_from_slice(f.name);
    let top = top_dict(&f, &l);
    write_index(&mut out, &[&top]);
    out.extend_from_slice(f.strings);
    out.extend_from_slice(f.gsubrs);
    out.extend_from_slice(&charset);
    out.extend_from_slice(f.fdselect);
    write_index(&mut out, &css);
    if f.cid {
        let fds: Vec<Vec<u8>> = f.fds.iter().map(|fd| font_dict(fd, &l)).collect();
        let items: Vec<&[u8]> = fds.iter().map(|v| v.as_slice()).collect();
        write_index(&mut out, &items);
    }
    for i in 0..f.privates.len() {
        out.extend_from_slice(&private_dict(&f, &l, i));
    }
    for s in &f.subrs {
        out.extend_from_slice(s);
    }
    if out.len() != pos || top.len() != top_len {
        return Err("CFF: internal layout mismatch");
    }
    Ok(out)
}
