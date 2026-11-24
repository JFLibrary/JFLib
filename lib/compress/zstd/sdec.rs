// zstd::sdec -- zstd::Decoder, Zstandard frames in pieces
//
// A frame is a header, blocks of at most 128 KiB whose sizes stand in their 3-byte headers,
// and a checksum: each part is collected whole (at most one block's bytes wait) and decoded
// with dec.rs.  The output goes into a window that holds what the frame's matches may reach:
// its Window_Size (the content size when it is smaller), refused above the decoder's limit
// (128 MiB by default, as the zstd tool's).  A match farther back than the window is an
// error, as in the zstd tool's streaming decoder (dec.rs's one-shot decompress allows any
// distance inside the frame).  The next block is decoded once the caller has taken the
// output of the last one; the window moves when it holds more than twice its size.  With a
// dictionary (with_dictionary), its content goes into the window before the frame's output
// (not given to the caller) and widens the reach by its length.

use alloc::vec::Vec;

use super::dec::{block, pick, rd_le, State, BLOCK_MAX, MAGIC};
use super::dict::Dictionary;
use crate::stream::{self, gather, Decode, Error, Status, Step};
use crate::xxh::Xxh64;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ph {
    Magic,
    SkipLen,
    Skip(usize),
    Head(usize), // the header of this many bytes (descriptor included)
    BlockHead,
    Block { ty: u32, size: usize, last: bool },
    Check,
}

/// a Zstandard decoder that takes its input in pieces (stream::Decode): frames one after the
/// other as `zstd -d` reads them, skippable frames skipped (`single()`: only the first frame).
/// Its memory is about twice a frame's window and one block.
pub struct Decoder {
    ph: Ph,
    buf: Vec<u8>,
    out: Vec<u8>,
    from: usize,
    fstart: usize,
    window: usize,
    reach: usize, // how far a match may reach: the window and the dictionary's content
    max_window: usize,
    dicts: Vec<Dictionary>,
    st: State,
    fcs: Option<u64>,
    made: u64,
    check: Option<Xxh64>,
    frames: u64,
    single: bool,
    ended: bool,
}

impl Decoder {
    /// a decoder of all frames, windows up to 128 MiB
    pub fn new() -> Decoder {
        Decoder {
            ph: Ph::Magic,
            buf: Vec::new(),
            out: Vec::new(),
            from: 0,
            fstart: 0,
            window: 0,
            reach: 0,
            max_window: 1 << 27,
            dicts: Vec::new(),
            st: State::new(),
            fcs: None,
            made: 0,
            check: None,
            frames: 0,
            single: false,
            ended: false,
        }
    }

    /// a decoder of one frame: Status::End after it, input behind it is not taken
    pub fn single() -> Decoder {
        Decoder { single: true, ..Decoder::new() }
    }

    /// refuse frames whose window is larger than `bytes` (Error::Limit)
    pub fn with_window_max(mut self, bytes: usize) -> Decoder {
        self.max_window = bytes;
        self
    }

    /// a dictionary for the frames that name its ID (several may be given); a frame that
    /// names none takes the only one given (raw content: Dictionary::raw).  A frame that names
    /// one not given is Error::Dictionary.
    pub fn with_dictionary(mut self, dict: Dictionary) -> Decoder {
        self.dicts.push(dict);
        self
    }

    /// start anew (the limit and single() stay)
    pub fn reset(&mut self) {
        self.ph = Ph::Magic;
        self.buf.clear();
        self.out.clear();
        self.from = 0;
        self.fstart = 0;
        self.frames = 0;
        self.ended = false;
    }

    /// the frames ended so far (skippable ones not counted)
    pub fn frames(&self) -> u64 {
        self.frames
    }

    // the frame header in buf
    fn header(&mut self) -> Result<(), Error> {
        let h = &self.buf;
        let fhd = h[4];
        if fhd & 0x08 != 0 {
            return Err(Error::Corrupt);
        }
        let single = fhd & 0x20 != 0;
        let mut p = 5;
        let mut window = 0u64;
        if !single {
            let wd = h[p];
            p += 1;
            let exp = (wd >> 3) as u32 + 10;
            if exp > 41 {
                return Err(Error::Corrupt);
            }
            let base = 1u64 << exp;
            window = base + (base / 8) * (wd & 7) as u64;
        }
        let did = [0, 1, 2, 4][(fhd & 3) as usize];
        let id = rd_le(&h[p..p + did]) as u32;
        p += did;
        let dict = match id {
            0 if self.dicts.len() == 1 => Some(&self.dicts[0]),
            0 => None,
            _ => Some(self.dicts.iter().find(|d| d.id == id).ok_or(Error::Dictionary)?),
        };
        let dict = pick(id, dict).map_err(|_| Error::Dictionary)?;
        let fcs_size = fcs_size(fhd);
        self.fcs = if fcs_size > 0 { Some(rd_le(&h[p..p + fcs_size]) + if fcs_size == 2 { 256 } else { 0 }) } else { None };
        if single {
            window = self.fcs.unwrap_or(0);
        }
        if let Some(n) = self.fcs {
            window = window.min(n);
        }
        if window > self.max_window as u64 {
            return Err(Error::Limit);
        }
        self.window = window as usize;
        self.check = if fhd & 0x04 != 0 { Some(Xxh64::new(0)) } else { None };
        self.st = State::with(dict);
        self.made = 0;
        self.fstart = self.out.len();
        let extra = dict.map_or(0, |d| d.content.len());
        if let Some(d) = dict {
            self.out.extend_from_slice(&d.content);
            self.from = self.out.len(); // (history, not output)
        }
        self.reach = self.window + extra;
        Ok(())
    }

    // output of a block: counted, hashed, checked against the content size
    fn produced(&mut self, from: usize) -> Result<(), Error> {
        let new = &self.out[from..];
        self.made += new.len() as u64;
        if let Some(x) = &mut self.check {
            x.update(new);
        }
        if self.fcs.is_some_and(|n| self.made > n) {
            return Err(Error::Corrupt);
        }
        Ok(())
    }

    // drop output the caller has taken and no match can reach any more
    fn trim(&mut self) {
        let keep = self.reach.max(1);
        if self.from == self.out.len() && self.out.len() > 2 * keep + BLOCK_MAX {
            let drop = self.out.len() - keep;
            self.out.drain(..drop);
            self.from -= drop;
            self.fstart = self.fstart.saturating_sub(drop);
        }
    }
}

fn fcs_size(fhd: u8) -> usize {
    match fhd >> 6 {
        0 => usize::from(fhd & 0x20 != 0),
        1 => 2,
        2 => 4,
        _ => 8,
    }
}

impl Default for Decoder {
    fn default() -> Decoder {
        Decoder::new()
    }
}

impl Decode for Decoder {
    fn decode(&mut self, input: &[u8], out: &mut [u8]) -> Result<Step, Error> {
        let (mut read, mut written) = (0, 0);
        loop {
            stream::deliver(&self.out, &mut self.from, self.out.len(), out, &mut written);
            if self.from < self.out.len() {
                return Ok(Step { read, written, status: Status::Output });
            }
            if self.ended {
                return Ok(Step { read, written, status: Status::End });
            }
            let input_left = read < input.len();
            match self.ph {
                Ph::Magic => {
                    if self.buf.is_empty() && !input_left {
                        let status = if self.frames > 0 { Status::End } else { Status::Input };
                        return Ok(Step { read, written, status });
                    }
                    if !gather(&mut self.buf, 4, input, &mut read) {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    let m = rd_le(&self.buf) as u32;
                    if m & 0xFFFF_FFF0 == 0x184D_2A50 {
                        self.buf.clear();
                        self.ph = Ph::SkipLen;
                    } else if m == MAGIC {
                        if !gather(&mut self.buf, 5, input, &mut read) {
                            return Ok(Step { read, written, status: Status::Input });
                        }
                        let fhd = self.buf[4];
                        let n = 5 + usize::from(fhd & 0x20 == 0) + [0, 1, 2, 4][(fhd & 3) as usize] + fcs_size(fhd);
                        self.ph = Ph::Head(n);
                    } else {
                        return Err(Error::Corrupt);
                    }
                }
                Ph::SkipLen => {
                    if !gather(&mut self.buf, 4, input, &mut read) {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    let n = rd_le(&self.buf) as usize;
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
                Ph::Head(n) => {
                    if !gather(&mut self.buf, n, input, &mut read) {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    self.header()?;
                    self.buf.clear();
                    self.ph = Ph::BlockHead;
                }
                Ph::BlockHead => {
                    if !gather(&mut self.buf, 3, input, &mut read) {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    let h = rd_le(&self.buf) as usize;
                    self.buf.clear();
                    let (last, ty, size) = (h & 1 != 0, ((h >> 1) & 3) as u32, h >> 3);
                    if size > BLOCK_MAX || ty == 3 {
                        return Err(Error::Corrupt);
                    }
                    self.ph = Ph::Block { ty, size, last };
                }
                Ph::Block { ty, size, last } => {
                    let want = if ty == 1 { 1 } else { size };
                    if !gather(&mut self.buf, want, input, &mut read) {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    self.trim();
                    let from = self.out.len();
                    match ty {
                        0 => self.out.extend_from_slice(&self.buf),
                        1 => self.out.resize(from + size, self.buf[0]),
                        _ => block(&mut self.st, &self.buf, &mut self.out, self.fstart, usize::MAX, self.reach).ok_or(Error::Corrupt)?,
                    }
                    self.produced(from)?;
                    self.buf.clear();
                    if !last {
                        self.ph = Ph::BlockHead;
                        continue;
                    }
                    if self.fcs.is_some_and(|n| n != self.made) {
                        return Err(Error::Corrupt);
                    }
                    self.ph = Ph::Check;
                }
                Ph::Check => {
                    if let Some(x) = &self.check {
                        let want = x.digest() as u32;
                        if !gather(&mut self.buf, 4, input, &mut read) {
                            return Ok(Step { read, written, status: Status::Input });
                        }
                        if rd_le(&self.buf) as u32 != want {
                            return Err(Error::Checksum);
                        }
                        self.buf.clear();
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
