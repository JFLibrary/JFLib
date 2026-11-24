// brotli::senc -- brotli::Encoder, a Brotli stream in pieces
//
// The input collects in a buffer that keeps the window before the bytes not yet compressed;
// a meta-block is compressed once its input and the 8 bytes after it are there (the match
// finder looks that far ahead), with the same finder, codes and choices as compress().  So a
// stream whose size is given (with_size) is the same bytes as compress() of the whole input;
// without a size the window is the level's (4 MiB, 16 MiB at levels 10 and 11) or the one
// with_window asks for.  Flush::Sync ends a meta-block early and fills the last byte with an
// empty metadata block.  The buffer moves by a step (1 MiB, or the reach of the hash chains)
// once it holds that much more than the window; the tables move with it.

use alloc::vec::Vec;

use super::enc::{end, sync, wbits, Bw, Core};
use crate::stream::{self, Encode, Error, Flush, Status, Step};

/// a Brotli encoder that takes its input in pieces (stream::Encode): one stream of RFC 7932;
/// memory about twice the window and a meta-block (1 MiB at most), plus the tables of the level
pub struct Encoder {
    level: i32,
    size: Option<u64>,
    lgwin: Option<u32>,
    core: Option<Core>,
    buf: Vec<u8>,
    bs: usize, // buf[bs..]: not yet compressed
    base: u64, // the stream position of buf[0]
    blk: usize,
    step: usize,
    bw: Bw,
    from: usize,
    done: bool,
    taken: u64,
}

impl Encoder {
    /// an encoder at `level` (0..=11 as compress)
    pub fn new(level: i32) -> Encoder {
        Encoder {
            level,
            size: None,
            lgwin: None,
            core: None,
            buf: Vec::new(),
            bs: 0,
            base: 0,
            blk: 0,
            step: 0,
            bw: Bw::new(),
            from: 0,
            done: false,
            taken: 0,
        }
    }

    /// the input is exactly this long (else Error::Size): the window and the tables fit it,
    /// and the stream is the same bytes as compress()
    pub fn with_size(mut self, bytes: u64) -> Encoder {
        self.size = Some(bytes);
        self
    }

    /// a window of 2^log - 16 bytes (log 10..=24) instead of the level's: what a decoder
    /// must keep of the output
    pub fn with_window(mut self, log: u32) -> Encoder {
        self.lgwin = Some(log.clamp(10, 24));
        self
    }

    fn start(&mut self) {
        let core = Core::new(self.level, self.size, self.lgwin);
        wbits(&mut self.bw, core.lgwin);
        self.blk = 1 << core.prm.block_log;
        self.step = (core.cmask + 1).max(1 << 20);
        self.core = Some(core);
    }

    // compress buf[bs..be] as a meta-block
    fn block(&mut self, mut be: usize, last: bool) {
        let Some(core) = self.core.as_mut() else { return };
        if self.bs > core.window + self.step {
            // the oldest step is out of reach: move the buffer and the positions in the tables
            self.buf.drain(..self.step);
            self.bs -= self.step;
            be -= self.step;
            self.base += self.step as u64;
            core.shift(self.step);
        }
        core.block(&self.buf, self.bs, be, self.base, last, &mut self.bw);
        self.bs = be;
    }
}

impl Encode for Encoder {
    fn encode(&mut self, input: &[u8], out: &mut [u8], flush: Flush) -> Result<Step, Error> {
        let (mut read, mut written) = (0, 0);
        loop {
            let ready = self.bw.out.len();
            stream::deliver(&self.bw.out, &mut self.from, ready, out, &mut written);
            if self.from < ready {
                return Ok(Step { read, written, status: Status::Output });
            }
            self.bw.out.clear();
            self.from = 0;
            if self.done {
                return Ok(Step { read, written, status: Status::End });
            }
            if self.core.is_none() {
                self.start();
                continue;
            }
            // take input up to a meta-block and the 8 bytes after it
            let room = (self.blk + 8).saturating_sub(self.buf.len() - self.bs);
            let k = room.min(input.len() - read);
            self.taken += k as u64;
            if self.size.is_some_and(|n| self.taken > n) {
                return Err(Error::Size);
            }
            self.buf.extend_from_slice(&input[read..read + k]);
            read += k;
            let pend = self.buf.len() - self.bs;
            if pend == self.blk + 8 {
                self.block(self.bs + self.blk, false);
                continue;
            }
            match flush {
                Flush::None => return Ok(Step { read, written, status: Status::Input }),
                Flush::Sync => {
                    if pend == 0 && !self.bw.pending() {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    if pend > 0 {
                        self.block(self.buf.len(), false);
                    }
                    if self.bw.pending() {
                        sync(&mut self.bw);
                    }
                }
                Flush::Finish => {
                    if self.size.is_some_and(|n| self.taken != n) {
                        return Err(Error::Size);
                    }
                    if pend > self.blk {
                        self.block(self.bs + self.blk, false);
                        continue;
                    }
                    if pend == 0 {
                        end(&mut self.bw);
                    } else {
                        self.block(self.buf.len(), true);
                    }
                    self.done = true;
                }
            }
        }
    }
}
