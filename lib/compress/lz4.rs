// lz4 -- the LZ4 block and frame formats
//
//   compress(data) -> block                greedy matcher, 16K-entry hash table, skip
//                                          acceleration on incompressible data
//   decompress(block, max_out)             None on truncation, a bad offset, or more than
//                                          max_out bytes; never reads or writes out of bounds
//   decompress_into(block, &mut out, limit) appends (back references may reach into the
//                                          earlier contents of out: linked frame blocks)
//   frame_compress(data) / frame_decompress(frame, max_out)
//   FrameDecoder / FrameEncoder           LZ4 frames in pieces (stream::Decode / Encode)
//
// Block rules kept by the compressor: the last 5 bytes are literals, no match starts in the
// last 12 bytes, offsets 1..=65535.

use alloc::vec;
use alloc::vec::Vec;

use crate::stream::{self, gather, Decode, Encode, Error, Flush, Status, Step};
use crate::{xxh32, Xxh32};

const MIN_MATCH: usize = 4;
const LAST_LITERALS: usize = 5;
const MF_LIMIT: usize = 12;
const HASH_LOG: u32 = 14;
const MAX_DIST: usize = 65535;

/// the largest block compress() can produce for n input bytes
pub const fn bound(n: usize) -> usize {
    n + n / 255 + 16
}

#[inline(always)]
fn rd32(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

#[inline(always)]
fn hash(v: u32) -> usize {
    (v.wrapping_mul(2_654_435_761) >> (32 - HASH_LOG)) as usize
}

fn put_len(out: &mut Vec<u8>, mut n: usize) {
    while n >= 255 {
        out.push(255);
        n -= 255;
    }
    out.push(n as u8);
}

fn sequence(out: &mut Vec<u8>, lits: &[u8], offset: usize, mlen: usize) {
    let ll = lits.len();
    let ml = mlen - MIN_MATCH;
    out.push(((ll.min(15) as u8) << 4) | ml.min(15) as u8);
    if ll >= 15 {
        put_len(out, ll - 15);
    }
    out.extend_from_slice(lits);
    out.extend_from_slice(&(offset as u16).to_le_bytes());
    if ml >= 15 {
        put_len(out, ml - 15);
    }
}

fn last(out: &mut Vec<u8>, lits: &[u8]) {
    let ll = lits.len();
    out.push((ll.min(15) as u8) << 4);
    if ll >= 15 {
        put_len(out, ll - 15);
    }
    out.extend_from_slice(lits);
}

/// compress into one LZ4 block
pub fn compress(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bound(src.len()));
    let n = src.len();
    if n <= MF_LIMIT {
        last(&mut out, src);
        return out;
    }
    let mut table = vec![0u32; 1 << HASH_LOG];
    let mflimit = n - MF_LIMIT; // last position a match may start at
    let match_end = n - LAST_LITERALS; // matches end before this
    let mut anchor = 0;
    let mut ip = 1;
    let mut misses = 0usize;
    while ip <= mflimit {
        let seq = rd32(src, ip);
        let h = hash(seq);
        let cand = table[h] as usize;
        table[h] = ip as u32;
        if cand < ip && ip - cand <= MAX_DIST && rd32(src, cand) == seq {
            let (mut s, mut c) = (ip, cand);
            while s > anchor && c > 0 && src[s - 1] == src[c - 1] {
                s -= 1;
                c -= 1;
            }
            let mut len = MIN_MATCH + (ip - s);
            while s + len < match_end && src[c + len] == src[s + len] {
                len += 1;
            }
            sequence(&mut out, &src[anchor..s], s - c, len);
            ip = s + len;
            anchor = ip;
            misses = 0;
            // remember a position inside the match (better ratio on repetitive data)
            if ip >= 2 && ip - 2 <= mflimit {
                table[hash(rd32(src, ip - 2))] = (ip - 2) as u32;
            }
        } else {
            misses += 1;
            ip += 1 + (misses >> 6);
        }
    }
    last(&mut out, &src[anchor..]);
    out
}

fn get_len(src: &[u8], i: &mut usize, mut n: usize) -> Option<usize> {
    loop {
        let b = *src.get(*i)?;
        *i += 1;
        n = n.checked_add(b as usize)?;
        if b != 255 {
            return Some(n);
        }
    }
}

/// decode one block, appending to out (out.len() stays <= limit) -> false on corrupt input
pub fn decompress_into(src: &[u8], out: &mut Vec<u8>, limit: usize) -> bool {
    let mut i = 0;
    loop {
        let Some(&token) = src.get(i) else { return false };
        i += 1;
        let mut ll = (token >> 4) as usize;
        if ll == 15 {
            match get_len(src, &mut i, ll) {
                Some(v) => ll = v,
                None => return false,
            }
        }
        if ll > src.len() - i || ll > limit.saturating_sub(out.len()) {
            return false;
        }
        out.extend_from_slice(&src[i..i + ll]);
        i += ll;
        if i == src.len() {
            return true; // the last sequence has no match
        }
        if src.len() - i < 2 {
            return false;
        }
        let off = u16::from_le_bytes([src[i], src[i + 1]]) as usize;
        i += 2;
        if off == 0 || off > out.len() {
            return false;
        }
        let mut ml = (token & 15) as usize;
        if ml == 15 {
            match get_len(src, &mut i, ml) {
                Some(v) => ml = v,
                None => return false,
            }
        }
        ml += MIN_MATCH;
        if ml > limit.saturating_sub(out.len()) {
            return false;
        }
        // copy with overlap: the source repeats with period `off`
        let start = out.len() - off;
        let mut rem = ml;
        while rem > 0 {
            let k = rem.min(out.len() - start);
            out.extend_from_within(start..start + k);
            rem -= k;
        }
    }
}

/// decode one block of at most max_out bytes
pub fn decompress(src: &[u8], max_out: usize) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(max_out.min(src.len().saturating_mul(4)).min(64 << 20));
    decompress_into(src, &mut out, max_out).then_some(out)
}

// ---------------------------------------------------------------------------- frames

const MAGIC: u32 = 0x184D_2204;
const BLOCK_MAX_ID: u8 = 7; // 4 MiB blocks
const BLOCK_MAX: usize = 4 << 20;

fn block_max(id: u8) -> Option<usize> {
    match id {
        4 => Some(64 << 10),
        5 => Some(256 << 10),
        6 => Some(1 << 20),
        7 => Some(4 << 20),
        _ => None,
    }
}

/// an LZ4 frame: independent 4 MiB blocks, content size and content checksum
pub fn frame_compress(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bound(src.len()) + 32);
    out.extend_from_slice(&MAGIC.to_le_bytes());
    // FLG: version 01, independent blocks, content size, content checksum
    let flg = 0b0110_1100u8;
    let bd = BLOCK_MAX_ID << 4;
    let desc_start = out.len();
    out.push(flg);
    out.push(bd);
    out.extend_from_slice(&(src.len() as u64).to_le_bytes());
    let hc = (xxh32(&out[desc_start..], 0) >> 8) as u8;
    out.push(hc);
    for chunk in src.chunks(BLOCK_MAX) {
        let c = compress(chunk);
        if c.len() < chunk.len() {
            out.extend_from_slice(&(c.len() as u32).to_le_bytes());
            out.extend_from_slice(&c);
        } else {
            out.extend_from_slice(&(chunk.len() as u32 | 0x8000_0000).to_le_bytes());
            out.extend_from_slice(chunk);
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&xxh32(src, 0).to_le_bytes());
    out
}

fn rd_u32(b: &[u8], i: &mut usize) -> Option<u32> {
    let v = b.get(*i..*i + 4)?;
    *i += 4;
    Some(u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
}

/// decode one or more concatenated LZ4 frames (skippable frames are skipped) of at most
/// max_out bytes in total; None on corruption, an unsupported feature (dictionary ids), a
/// checksum mismatch, or too much output
pub fn frame_decompress(src: &[u8], max_out: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < src.len() {
        let magic = rd_u32(src, &mut i)?;
        if magic & 0xFFFF_FFF0 == 0x184D_2A50 {
            let n = rd_u32(src, &mut i)? as usize;
            i = i.checked_add(n).filter(|&e| e <= src.len())?;
            continue;
        }
        if magic != MAGIC {
            return None;
        }
        let desc_start = i;
        let flg = *src.get(i)?;
        let bd = *src.get(i + 1)?;
        i += 2;
        if flg >> 6 != 1 || flg & 0b10 != 0 || flg & 1 != 0 || bd & 0x8F != 0 {
            return None; // version, reserved bit, dictionary id, reserved BD bits
        }
        let block_checksum = flg & 0x10 != 0;
        let has_size = flg & 0x08 != 0;
        let content_checksum = flg & 0x04 != 0;
        let bmax = block_max((bd >> 4) & 7)?;
        let mut size = None;
        if has_size {
            let b = src.get(i..i + 8)?;
            size = Some(u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]));
            i += 8;
        }
        let hc = *src.get(i)?;
        if (xxh32(&src[desc_start..i], 0) >> 8) as u8 != hc {
            return None;
        }
        i += 1;
        let frame_start = out.len();
        loop {
            let word = rd_u32(src, &mut i)?;
            if word == 0 {
                break;
            }
            let raw = word & 0x8000_0000 != 0;
            let n = (word & 0x7FFF_FFFF) as usize;
            if n > bmax {
                return None;
            }
            let data = src.get(i..i + n)?;
            i += n;
            if block_checksum {
                let c = rd_u32(src, &mut i)?;
                if xxh32(data, 0) != c {
                    return None;
                }
            }
            if raw {
                if data.len() > max_out - out.len() {
                    return None;
                }
                out.extend_from_slice(data);
            } else {
                let limit = max_out.min(out.len() + bmax);
                if !decompress_into(data, &mut out, limit) {
                    return None;
                }
            }
        }
        if let Some(s) = size {
            if (out.len() - frame_start) as u64 != s {
                return None;
            }
        }
        if content_checksum {
            let c = rd_u32(src, &mut i)?;
            if xxh32(&out[frame_start..], 0) != c {
                return None;
            }
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------- frames in pieces

const HISTORY: usize = 64 << 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ph {
    Magic,
    SkipLen,
    Skip(usize),
    Desc(usize), // the descriptor of this many bytes (FLG and BD included)
    Word,
    Data { n: usize, raw: bool },
    BlockSum { n: usize, raw: bool },
    Check,
}

/// an LZ4 frame decoder that takes its input in pieces (stream::Decode): frames one after
/// the other, skippable frames skipped (`single()`: only the first frame).  It holds one
/// block (at most 4 MiB, as the frame's descriptor allows) and the last 64 KiB of output,
/// which linked blocks may reach.
pub struct FrameDecoder {
    ph: Ph,
    buf: Vec<u8>,
    out: Vec<u8>,
    from: usize,
    flg: u8,
    bmax: usize,
    size: Option<u64>,
    made: u64,
    sum: Xxh32,
    frames: u64,
    single: bool,
    ended: bool,
}

impl FrameDecoder {
    /// a decoder of all frames
    pub fn new() -> FrameDecoder {
        FrameDecoder {
            ph: Ph::Magic,
            buf: Vec::new(),
            out: Vec::new(),
            from: 0,
            flg: 0,
            bmax: 0,
            size: None,
            made: 0,
            sum: Xxh32::new(0),
            frames: 0,
            single: false,
            ended: false,
        }
    }

    /// a decoder of one frame: Status::End after it, input behind it is not taken
    pub fn single() -> FrameDecoder {
        FrameDecoder { single: true, ..FrameDecoder::new() }
    }

    /// start anew (single() stays)
    pub fn reset(&mut self) {
        self.ph = Ph::Magic;
        self.buf.clear();
        self.out.clear();
        self.from = 0;
        self.frames = 0;
        self.ended = false;
    }

    /// the frames ended so far (skippable ones not counted)
    pub fn frames(&self) -> u64 {
        self.frames
    }

    // a block of buf: its output appended to out
    fn block(&mut self, raw: bool) -> Result<(), Error> {
        if self.from == self.out.len() && self.out.len() > HISTORY + (1 << 20) {
            let drop = self.out.len() - HISTORY;
            self.out.drain(..drop);
            self.from -= drop;
        }
        let at = self.out.len();
        if raw {
            self.out.extend_from_slice(&self.buf);
        } else if !decompress_into(&self.buf, &mut self.out, at + self.bmax) {
            return Err(Error::Corrupt);
        }
        let new = &self.out[at..];
        self.made += new.len() as u64;
        if self.size.is_some_and(|n| self.made > n) {
            return Err(Error::Corrupt);
        }
        self.sum.update(new);
        self.buf.clear();
        Ok(())
    }
}

impl Default for FrameDecoder {
    fn default() -> FrameDecoder {
        FrameDecoder::new()
    }
}

impl Decode for FrameDecoder {
    fn decode(&mut self, input: &[u8], out: &mut [u8]) -> Result<Step, Error> {
        let (mut read, mut written) = (0, 0);
        macro_rules! want {
            ($n:expr) => {
                if !gather(&mut self.buf, $n, input, &mut read) {
                    return Ok(Step { read, written, status: Status::Input });
                }
            };
        }
        loop {
            stream::deliver(&self.out, &mut self.from, self.out.len(), out, &mut written);
            if self.from < self.out.len() {
                return Ok(Step { read, written, status: Status::Output });
            }
            if self.ended {
                return Ok(Step { read, written, status: Status::End });
            }
            match self.ph {
                Ph::Magic => {
                    if self.buf.is_empty() && read == input.len() {
                        let status = if self.frames > 0 { Status::End } else { Status::Input };
                        return Ok(Step { read, written, status });
                    }
                    want!(4);
                    let m = u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
                    self.buf.clear();
                    if m & 0xFFFF_FFF0 == 0x184D_2A50 {
                        self.ph = Ph::SkipLen;
                    } else if m == MAGIC {
                        self.ph = Ph::Desc(3);
                    } else {
                        return Err(Error::Corrupt);
                    }
                }
                Ph::SkipLen => {
                    want!(4);
                    let n = u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]) as usize;
                    self.buf.clear();
                    self.ph = Ph::Skip(n);
                }
                Ph::Skip(n) => {
                    let k = n.min(input.len() - read);
                    read += k;
                    if k < n {
                        self.ph = Ph::Skip(n - k);
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    self.ph = Ph::Magic;
                }
                Ph::Desc(n) => {
                    want!(n);
                    let (flg, bd) = (self.buf[0], self.buf[1]);
                    if flg >> 6 != 1 || flg & 0b10 != 0 || bd & 0x8F != 0 {
                        return Err(Error::Corrupt); // version, reserved bits
                    }
                    if flg & 1 != 0 {
                        return Err(Error::Unsupported); // a dictionary id
                    }
                    let full = 3 + if flg & 0x08 != 0 { 8 } else { 0 };
                    if n < full {
                        self.ph = Ph::Desc(full);
                        continue;
                    }
                    if (xxh32(&self.buf[..n - 1], 0) >> 8) as u8 != self.buf[n - 1] {
                        return Err(Error::Checksum);
                    }
                    self.flg = flg;
                    self.bmax = block_max((bd >> 4) & 7).ok_or(Error::Corrupt)?;
                    self.size = (flg & 0x08 != 0).then(|| u64::from_le_bytes(self.buf[2..10].try_into().unwrap()));
                    self.made = 0;
                    self.sum = Xxh32::new(0);
                    self.buf.clear();
                    self.ph = Ph::Word;
                }
                Ph::Word => {
                    want!(4);
                    let w = u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
                    self.buf.clear();
                    if w == 0 {
                        if self.size.is_some_and(|n| n != self.made) {
                            return Err(Error::Corrupt);
                        }
                        self.ph = Ph::Check;
                        continue;
                    }
                    let n = (w & 0x7FFF_FFFF) as usize;
                    if n > self.bmax {
                        return Err(Error::Corrupt);
                    }
                    self.ph = Ph::Data { n, raw: w & 0x8000_0000 != 0 };
                }
                Ph::Data { n, raw } => {
                    want!(n);
                    if self.flg & 0x10 != 0 {
                        self.ph = Ph::BlockSum { n, raw };
                        continue;
                    }
                    self.block(raw)?;
                    self.ph = Ph::Word;
                }
                Ph::BlockSum { n, raw } => {
                    want!(n + 4);
                    let c = u32::from_le_bytes(self.buf[n..n + 4].try_into().unwrap());
                    if xxh32(&self.buf[..n], 0) != c {
                        return Err(Error::Checksum);
                    }
                    self.buf.truncate(n);
                    self.block(raw)?;
                    self.ph = Ph::Word;
                }
                Ph::Check => {
                    if self.flg & 0x04 != 0 {
                        want!(4);
                        let c = u32::from_le_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
                        self.buf.clear();
                        if c != self.sum.digest() {
                            return Err(Error::Checksum);
                        }
                    }
                    self.frames += 1;
                    self.ph = Ph::Magic;
                    self.ended = self.single;
                }
            }
        }
    }

    fn finish(&self) -> Result<(), Error> {
        if self.ph == Ph::Magic && self.buf.is_empty() && self.from == self.out.len() {
            Ok(())
        } else {
            Err(Error::Truncated)
        }
    }
}

/// an LZ4 frame encoder that takes its input in pieces (stream::Encode): independent blocks
/// of 4 MiB and the content checksum, as frame_compress writes them; the content size only
/// when it is given (with_size), and then the same bytes as frame_compress.  Flush::Sync ends
/// a block early.  It holds one block of input and its output.
pub struct FrameEncoder {
    size: Option<u64>,
    buf: Vec<u8>,
    out: Vec<u8>,
    from: usize,
    head: bool,
    done: bool,
    taken: u64,
    sum: Xxh32,
}

impl FrameEncoder {
    pub fn new() -> FrameEncoder {
        FrameEncoder { size: None, buf: Vec::new(), out: Vec::new(), from: 0, head: false, done: false, taken: 0, sum: Xxh32::new(0) }
    }

    /// state the content size in the frame (the input must have exactly this length, else
    /// Error::Size)
    pub fn with_size(mut self, bytes: u64) -> FrameEncoder {
        self.size = Some(bytes);
        self
    }

    fn block(&mut self) {
        let c = compress(&self.buf);
        if c.len() < self.buf.len() {
            self.out.extend_from_slice(&(c.len() as u32).to_le_bytes());
            self.out.extend_from_slice(&c);
        } else {
            self.out.extend_from_slice(&(self.buf.len() as u32 | 0x8000_0000).to_le_bytes());
            self.out.extend_from_slice(&self.buf);
        }
        self.buf.clear();
    }
}

impl Default for FrameEncoder {
    fn default() -> FrameEncoder {
        FrameEncoder::new()
    }
}

impl Encode for FrameEncoder {
    fn encode(&mut self, input: &[u8], out: &mut [u8], flush: Flush) -> Result<Step, Error> {
        let (mut read, mut written) = (0, 0);
        loop {
            stream::deliver(&self.out, &mut self.from, self.out.len(), out, &mut written);
            if self.from < self.out.len() {
                return Ok(Step { read, written, status: Status::Output });
            }
            self.out.clear();
            self.from = 0;
            if self.done {
                return Ok(Step { read, written, status: Status::End });
            }
            if !self.head {
                self.out.extend_from_slice(&MAGIC.to_le_bytes());
                let flg = if self.size.is_some() { 0b0110_1100u8 } else { 0b0110_0100 };
                self.out.push(flg);
                self.out.push(BLOCK_MAX_ID << 4);
                if let Some(n) = self.size {
                    self.out.extend_from_slice(&n.to_le_bytes());
                }
                let hc = (xxh32(&self.out[4..], 0) >> 8) as u8;
                self.out.push(hc);
                self.head = true;
                continue;
            }
            let k = (BLOCK_MAX - self.buf.len()).min(input.len() - read);
            let new = &input[read..read + k];
            self.taken += k as u64;
            if self.size.is_some_and(|n| self.taken > n) {
                return Err(Error::Size);
            }
            self.sum.update(new);
            self.buf.extend_from_slice(new);
            read += k;
            if self.buf.len() == BLOCK_MAX && (read < input.len() || flush != Flush::Finish) {
                self.block();
                continue;
            }
            match flush {
                Flush::None => return Ok(Step { read, written, status: Status::Input }),
                Flush::Sync => {
                    if self.buf.is_empty() {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    self.block();
                }
                Flush::Finish => {
                    if self.size.is_some_and(|n| self.taken != n) {
                        return Err(Error::Size);
                    }
                    if !self.buf.is_empty() {
                        self.block();
                    }
                    self.out.extend_from_slice(&0u32.to_le_bytes());
                    self.out.extend_from_slice(&self.sum.digest().to_le_bytes());
                    self.done = true;
                }
            }
        }
    }
}
