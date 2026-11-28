// cidmap -- the predefined CMaps of Adobe's Chinese, Japanese and Korean character collections
// (character code -> CID) and their CID -> Unicode tables
//
//   predefined(name) -> Option<CMap>      a CMap of ISO 32000-1 Table 118 by its name
//                                         (90ms-RKSJ-H, GBK-EUC-H, UniJIS-UTF16-V, ...)
//   names() -> Vec<&'static str>          the CMaps this crate has (59)
//   CMap { name, collection, vertical, codespace }
//     code_len(bytes) -> usize            the length of the code at the start (1 to 4 bytes)
//     cid(code, len) -> Option<u32>       a code's CID (its CID range, else its notdef range)
//     ranges() -> &[CidRange]             every code -> CID range, with what usecmap brings in
//   Collection { Gb1, Cns1, Japan1, Korea1 }
//     from_ordering(b"Japan1"), ordering(), lang() (zh-Hans, zh-Hant, ja, ko)
//   to_unicode(collection) -> ToUnicode   CID -> text (Adobe's Adobe-Japan1-UCS2 and the like)
//     get(cid) -> Option<&str>, len()
//
// The data (data/cidmap.bin, made by tools/cidmap_gen.py from Adobe's files, BSD-3-Clause:
// licenses/adobe-cmap-resources-LICENSE.md) is one zstd frame per collection, unpacked when a
// CMap or table of it is asked for; callers keep what they use.  Identity-H and -V are not here:
// their CID is the two-byte code, with no collection.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

static DATA: &[u8] = include_bytes!("../data/cidmap.bin");

/// one of Adobe's four CJK character collections (Registry "Adobe")
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Collection {
    /// Adobe-GB1: simplified Chinese
    Gb1,
    /// Adobe-CNS1: traditional Chinese
    Cns1,
    /// Adobe-Japan1: Japanese
    Japan1,
    /// Adobe-Korea1: Korean
    Korea1,
}

impl Collection {
    pub const ALL: [Collection; 4] = [Collection::Gb1, Collection::Cns1, Collection::Japan1, Collection::Korea1];

    /// a CIDSystemInfo's Ordering (with Registry "Adobe")
    pub fn from_ordering(o: &[u8]) -> Option<Collection> {
        Collection::ALL.into_iter().find(|c| c.ordering().as_bytes() == o)
    }

    pub fn ordering(self) -> &'static str {
        match self {
            Collection::Gb1 => "GB1",
            Collection::Cns1 => "CNS1",
            Collection::Japan1 => "Japan1",
            Collection::Korea1 => "Korea1",
        }
    }

    /// the language tag of its text, for choosing a font (Han characters take its forms)
    pub fn lang(self) -> &'static str {
        match self {
            Collection::Gb1 => "zh-Hans",
            Collection::Cns1 => "zh-Hant",
            Collection::Japan1 => "ja",
            Collection::Korea1 => "ko",
        }
    }

    fn index(self) -> u8 {
        self as u8
    }
}

/// a range of the code space: codes of `len` bytes whose every byte lies between that of `lo`
/// and `hi`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeSpace {
    pub len: u8,
    pub lo: [u8; 4],
    pub hi: [u8; 4],
}

/// the codes `lo..=hi` of `len` bytes (big-endian, as u32) -> the CIDs from `cid` on
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CidRange {
    pub len: u8,
    pub lo: u32,
    pub hi: u32,
    pub cid: u32,
}

/// a predefined CMap, with what it takes in by usecmap
#[derive(Clone, Debug)]
pub struct CMap {
    pub name: &'static str,
    pub collection: Collection,
    /// WMode 1: vertical writing
    pub vertical: bool,
    pub codespace: Vec<CodeSpace>,
    /// sorted by (len, lo), not overlapping
    ranges: Vec<CidRange>,
    /// codes without a CID range: notdef ranges, the CMap's own before those it uses (one CID
    /// for the whole range)
    notdef: Vec<CidRange>,
}

impl CMap {
    /// the length of the code at the start of `b` (1 to 4 bytes): the shortest the code space
    /// has for it, else the code space's shortest length
    pub fn code_len(&self, b: &[u8]) -> usize {
        for n in 1..=4usize {
            if b.len() < n {
                break;
            }
            if self.codespace.iter().any(|c| c.len as usize == n && (0..n).all(|i| c.lo[i] <= b[i] && b[i] <= c.hi[i])) {
                return n;
            }
        }
        self.codespace.iter().map(|c| c.len as usize).min().unwrap_or(1).min(b.len()).max(1)
    }

    /// the CID of a code of `len` bytes: its CID range's, else its notdef range's
    pub fn cid(&self, code: u32, len: u8) -> Option<u32> {
        let i = self.ranges.partition_point(|r| (r.len, r.hi) < (len, code));
        if let Some(r) = self.ranges.get(i) {
            if r.len == len && r.lo <= code {
                return Some(r.cid.wrapping_add(code - r.lo));
            }
        }
        self.notdef.iter().find(|r| r.len == len && r.lo <= code && code <= r.hi).map(|r| r.cid)
    }

    /// every code -> CID range, sorted by (len, lo)
    pub fn ranges(&self) -> &[CidRange] {
        &self.ranges
    }
}

/// CID -> Unicode text of a collection
#[derive(Clone, Debug, Default)]
pub struct ToUnicode {
    text: String,
    /// per CID the start of its text in `text`, and the end of the last one
    starts: Vec<u32>,
}

impl ToUnicode {
    /// a CID's text (a character, or a few of them)
    pub fn get(&self, cid: u32) -> Option<&str> {
        let i = cid as usize;
        let (s, e) = (*self.starts.get(i)? as usize, *self.starts.get(i + 1)? as usize);
        (s < e).then(|| &self.text[s..e])
    }

    /// the number of CIDs (with text or not)
    pub fn len(&self) -> usize {
        self.starts.len().saturating_sub(1)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ------------------------------------------------------------------ the data file

struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.d.get(self.p)?;
        self.p += 1;
        Some(v)
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let v = self.d.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(v)
    }
    fn u16(&mut self) -> Option<u16> {
        let b = self.bytes(2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        let b = self.bytes(4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn varint(&mut self) -> Option<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            v |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
        None
    }
}

fn unzigzag(v: u64) -> i64 {
    (v >> 1) as i64 ^ -((v & 1) as i64)
}

struct Entry {
    kind: u8,
    collection: u8,
    name: &'static str,
    off: usize,
    len: usize,
}

/// the frames (unpacked length, offset, length) and the entries of the data file
fn index() -> Option<(Vec<(usize, usize, usize)>, Vec<Entry>)> {
    let mut r = Rd { d: DATA, p: 0 };
    if r.bytes(5)? != b"CIDM\x01" {
        return None;
    }
    let nf = r.u8()? as usize;
    let mut frames = Vec::with_capacity(nf);
    for _ in 0..nf {
        frames.push((r.u32()? as usize, r.u32()? as usize, r.u32()? as usize));
    }
    let count = r.u16()? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let (kind, collection, nl) = (r.u8()?, r.u8()?, r.u8()? as usize);
        let name = core::str::from_utf8(r.bytes(nl)?).ok()?;
        let (off, len) = (r.u32()? as usize, r.u32()? as usize);
        entries.push(Entry { kind, collection, name, off, len });
    }
    Some((frames, entries))
}

/// a collection's frame, unpacked
fn frame(frames: &[(usize, usize, usize)], c: u8) -> Option<Vec<u8>> {
    let &(raw, off, len) = frames.get(c as usize)?;
    let out = compress::zstd::decompress(DATA.get(off..off.checked_add(len)?)?, raw)?;
    (out.len() == raw).then_some(out)
}

fn collection_of(i: u8) -> Option<Collection> {
    Collection::ALL.into_iter().find(|c| c.index() == i)
}

/// one CMap's part: (vertical, the index of the CMap it uses, code space, ranges, notdef)
#[allow(clippy::type_complexity)]
fn parse_cmap(d: &[u8]) -> Option<(bool, u16, Vec<CodeSpace>, Vec<CidRange>, Vec<CidRange>)> {
    let mut r = Rd { d, p: 0 };
    let vertical = r.u8()? == 1;
    let base = r.u16()?;
    let n = r.varint()? as usize;
    let mut cs = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        let len = r.u8()?;
        if !(1..=4).contains(&len) {
            return None;
        }
        let (mut lo, mut hi) = ([0u8; 4], [0u8; 4]);
        lo[..len as usize].copy_from_slice(r.bytes(len as usize)?);
        hi[..len as usize].copy_from_slice(r.bytes(len as usize)?);
        cs.push(CodeSpace { len, lo, hi });
    }
    let n = r.varint()? as usize;
    let mut ranges = Vec::with_capacity(n.min(1 << 16));
    let (mut prev_len, mut prev_end, mut next_cid) = (0u8, 0u64, 0i64);
    for _ in 0..n {
        let len = r.u8()?;
        if len != prev_len {
            prev_len = len;
            prev_end = 0;
        }
        let lo = prev_end.checked_add(r.varint()?)?;
        let hi = lo.checked_add(r.varint()?)?;
        let cid = next_cid.checked_add(unzigzag(r.varint()?))?;
        if hi > u32::MAX as u64 || !(0..=u32::MAX as i64).contains(&cid) {
            return None;
        }
        ranges.push(CidRange { len, lo: lo as u32, hi: hi as u32, cid: cid as u32 });
        prev_end = hi + 1;
        next_cid = cid + (hi - lo) as i64 + 1;
    }
    let n = r.varint()? as usize;
    let mut notdef = Vec::with_capacity(n.min(64));
    for _ in 0..n {
        let len = r.u8()?;
        let lo = r.varint()?;
        let hi = lo.checked_add(r.varint()?)?;
        let cid = r.varint()?;
        if hi > u32::MAX as u64 || cid > u32::MAX as u64 {
            return None;
        }
        notdef.push(CidRange { len, lo: lo as u32, hi: hi as u32, cid: cid as u32 });
    }
    Some((vertical, base, cs, ranges, notdef))
}

/// `own` over `base`: the codes of `own` take its CIDs, the others keep the base's
fn overlay(own: Vec<CidRange>, base: &[CidRange]) -> Vec<CidRange> {
    let mut out = own.clone();
    for b in base {
        let mut lo = b.lo as u64;
        let i = own.partition_point(|o| (o.len, o.hi) < (b.len, b.lo));
        for o in own[i..].iter().take_while(|o| o.len == b.len && o.lo <= b.hi) {
            if (o.lo as u64) > lo {
                out.push(CidRange { len: b.len, lo: lo as u32, hi: o.lo - 1, cid: b.cid.wrapping_add((lo - b.lo as u64) as u32) });
            }
            lo = lo.max(o.hi as u64 + 1);
        }
        if lo <= b.hi as u64 {
            out.push(CidRange { len: b.len, lo: lo as u32, hi: b.hi, cid: b.cid.wrapping_add((lo - b.lo as u64) as u32) });
        }
    }
    out.sort_by_key(|r| (r.len, r.lo));
    out
}

/// a predefined CMap by its name (ISO 32000-1 Table 118: GB-EUC-H to UniKS-UTF16-V)
pub fn predefined(name: &[u8]) -> Option<CMap> {
    let (frames, entries) = index()?;
    let at = entries.iter().position(|e| e.kind == 0 && e.name.as_bytes() == name)?;
    let collection = collection_of(entries[at].collection)?;
    let data = frame(&frames, entries[at].collection)?;
    let part = |i: usize| -> Option<&[u8]> {
        let e = entries.get(i)?;
        if e.kind != 0 || e.collection != entries[at].collection {
            return None;
        }
        data.get(e.off..e.off.checked_add(e.len)?)
    };
    let (vertical, mut base, codespace, mut ranges, mut notdef) = parse_cmap(part(at)?)?;
    let mut codespace = codespace;
    // what usecmap brings in, CMap by CMap (a chain of a few at most)
    let mut depth = 0;
    while base != 0xFFFF {
        depth += 1;
        if depth > 8 {
            return None;
        }
        let (_, next, cs, r, nd) = parse_cmap(part(base as usize)?)?;
        if codespace.is_empty() {
            codespace = cs;
        }
        ranges = overlay(ranges, &r);
        notdef.extend(nd);
        base = next;
    }
    Some(CMap { name: entries[at].name, collection, vertical, codespace, ranges, notdef })
}

/// the names of the CMaps this crate has, in the order of the data file
pub fn names() -> Vec<&'static str> {
    index().map(|(_, e)| e.into_iter().filter(|e| e.kind == 0).map(|e| e.name).collect()).unwrap_or_default()
}

/// a collection's CID -> Unicode table (empty if the data is damaged, which the tests rule out)
pub fn to_unicode(c: Collection) -> ToUnicode {
    let parse = || -> Option<ToUnicode> {
        let (frames, entries) = index()?;
        let e = entries.iter().find(|e| e.kind == 1 && e.collection == c.index())?;
        let data = frame(&frames, e.collection)?;
        let mut r = Rd { d: data.get(e.off..e.off.checked_add(e.len)?)?, p: 0 };
        let n = r.varint()? as usize;
        if n > 1 << 20 {
            return None;
        }
        let mut t = ToUnicode { text: String::with_capacity(n * 3), starts: Vec::with_capacity(n + 1) };
        let mut prev = -1i64;
        for _ in 0..n {
            t.starts.push(t.text.len() as u32);
            match r.varint()? {
                0 => {}
                1 => {
                    let k = r.varint()?;
                    if k > 64 {
                        return None;
                    }
                    for _ in 0..k {
                        t.text.push(char::from_u32(u32::try_from(r.varint()?).ok()?)?);
                    }
                }
                v => {
                    let cp = prev.checked_add(1)?.checked_add(unzigzag(v - 2))?;
                    t.text.push(char::from_u32(u32::try_from(cp).ok()?)?);
                    prev = cp;
                }
            }
        }
        t.starts.push(t.text.len() as u32);
        Some(t)
    };
    parse().unwrap_or_default()
}
