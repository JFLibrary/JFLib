// zstd::senc -- zstd::Encoder, one Zstandard frame in pieces
//
// The input collects in a buffer that keeps the frame's window (8 MiB, or the content when
// a smaller size is given) before the bytes not yet compressed; a block of 128 KiB is
// compressed once 8 more bytes are there (the match finder looks that far ahead), with the
// same finder, tables and block choices as compress().  So a frame whose size is given
// (with_size) is the same bytes as compress() of the whole input; without a size the
// frame header states none, the window is 8 MiB and the tables have their full size.
// Flush::Sync ends a block early.  The buffer moves by 4 MiB once it holds more than the
// window and 4 MiB; the tables move with it.

use alloc::vec;
use alloc::vec::Vec;

use super::dec::BLOCK_MAX;
use super::dict::Dictionary;
use super::enc::{block_header, frame_header, params, table_logs, Blocks, Finder, Params, WINDOW_LOG};
use crate::stream::{self, Encode, Error, Flush, Status, Step};
use crate::xxh::Xxh64;

const STEP: usize = 4 << 20;

/// a Zstandard encoder that takes its input in pieces (stream::Encode): one frame with a
/// checksum, the content size when it is given; memory up to about 12 MiB plus the tables
/// of the level
pub struct Encoder {
    prm: Params,
    size: Option<u64>,
    buf: Vec<u8>,
    bs: usize, // buf[bs..]: not yet compressed
    hash: Vec<u32>,
    chain: Vec<u32>,
    hash_log: u32,
    cmask: usize,
    next: usize,
    window: usize,
    dict: Option<Dictionary>,
    pending: usize, // dictionary positions to enter into the tables with the first block
    blocks: Blocks,
    out: Vec<u8>,
    from: usize,
    head: bool,
    done: bool,
    taken: u64,
    xxh: Xxh64,
}

impl Encoder {
    /// an encoder at `level` (1..=19 as compress)
    pub fn new(level: i32) -> Encoder {
        let prm = params(level);
        Encoder {
            prm,
            size: None,
            buf: Vec::new(),
            bs: 0,
            hash: Vec::new(),
            chain: Vec::new(),
            hash_log: 0,
            cmask: 0,
            next: 0,
            window: 0,
            dict: None,
            pending: 0,
            blocks: Blocks::new(None),
            out: Vec::new(),
            from: 0,
            head: false,
            done: false,
            taken: 0,
            xxh: Xxh64::new(0),
        }
    }

    /// state the content size in the frame (the input must have exactly this length, else
    /// Error::Size); the frame is then the same bytes as compress()
    pub fn with_size(mut self, bytes: u64) -> Encoder {
        self.size = Some(bytes);
        self
    }

    /// compress with a dictionary (as compress_with_dict; with a size, the same bytes)
    pub fn with_dictionary(mut self, dict: &Dictionary) -> Encoder {
        self.dict = Some(dict.clone());
        self
    }

    fn start(&mut self) {
        let pre = self.dict.as_ref().map_or(0, |d| d.content.len());
        let single = frame_header(&mut self.out, self.size, self.dict.as_ref().map_or(0, |d| d.id));
        let (h, c) = table_logs(&self.prm, self.size.map(|n| n + pre as u64));
        self.hash = vec![0; 1 << h];
        self.chain = if self.prm.chain_log == 0 { Vec::new() } else { vec![0; 1 << c] };
        self.hash_log = h;
        self.cmask = (1usize << c) - 1;
        self.window = if single { self.size.unwrap_or(0) as usize + pre } else { (1 << WINDOW_LOG) - 1 };
        self.blocks = Blocks::new(self.dict.as_ref());
        if let Some(d) = &self.dict {
            // the content first; its positions enter the tables with the first block, once
            // the bytes after it are there as for compress_with_dict
            self.buf.extend_from_slice(&d.content);
            self.bs = pre;
            self.pending = pre;
        }
        self.head = true;
    }

    // compress buf[bs..be] as a block
    fn block(&mut self, be: usize, last: bool) {
        if self.bs > self.window + STEP {
            // the oldest 4 MiB are out of reach: move the buffer and the positions in the tables
            self.buf.drain(..STEP);
            self.bs -= STEP;
            self.next = self.next.saturating_sub(STEP);
            let shift = STEP as u32;
            for e in self.hash.iter_mut().chain(self.chain.iter_mut()) {
                *e = e.saturating_sub(shift);
            }
            return self.block(be - STEP, last);
        }
        let mut f = Finder {
            s: &self.buf,
            hash: core::mem::take(&mut self.hash),
            chain: core::mem::take(&mut self.chain),
            hash_log: self.hash_log,
            cmask: self.cmask,
            depth: self.prm.depth,
            window: self.window,
            min: self.prm.min,
            sparse: self.prm.sparse,
            next: self.next,
        };
        if self.pending > 0 {
            f.enter(self.pending);
            self.pending = 0;
        }
        self.blocks.put(&mut f, &self.prm, self.bs, be, last, &mut self.out);
        self.hash = f.hash;
        self.chain = f.chain;
        self.next = f.next;
        self.bs = be;
    }
}

impl Encode for Encoder {
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
                self.start();
                continue;
            }
            // take input up to a block and the 8 bytes after it
            let room = (BLOCK_MAX + 8).saturating_sub(self.buf.len() - self.bs);
            let k = room.min(input.len() - read);
            let new = &input[read..read + k];
            self.taken += k as u64;
            if self.size.is_some_and(|n| self.taken > n) {
                return Err(Error::Size);
            }
            self.xxh.update(new);
            self.buf.extend_from_slice(new);
            read += k;
            let pend = self.buf.len() - self.bs;
            if pend == BLOCK_MAX + 8 {
                self.block(self.bs + BLOCK_MAX, false);
                continue;
            }
            match flush {
                Flush::None => return Ok(Step { read, written, status: Status::Input }),
                Flush::Sync => {
                    if pend == 0 {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    self.block(self.buf.len(), false);
                }
                Flush::Finish => {
                    if self.size.is_some_and(|n| self.taken != n) {
                        return Err(Error::Size);
                    }
                    if pend > BLOCK_MAX {
                        self.block(self.bs + BLOCK_MAX, false);
                        continue;
                    }
                    if pend == 0 {
                        // nothing left (an empty frame, or all of it went out with Sync): an
                        // empty raw block ends the frame
                        block_header(&mut self.out, true, 0, 0);
                    } else {
                        self.block(self.buf.len(), true);
                    }
                    self.out.extend_from_slice(&(self.xxh.digest() as u32).to_le_bytes());
                    self.done = true;
                }
            }
        }
    }
}
