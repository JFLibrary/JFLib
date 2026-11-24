// deflate::senc -- deflate::Encoder, zlib::Encoder and gzip::Encoder: a stream in pieces
//
// The input collects in pieces of 256 KiB, each compressed by enc.rs as for a whole input
// but with the 32 KiB before it as the window of its matches and its bits after the bits of
// the piece before (DeflateState::piece).  A piece ends a block, so a stream of more than
// 256 KiB is a little larger than compress() makes it; one of at most 256 KiB, given before
// Flush::Finish, is the same bytes as compress(), zlib::compress() and gzip::compress().
// Flush::Sync compresses what is there and adds an empty stored block (00 00 FF FF), after
// which a decoder can give back everything so far.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::enc::{deflate_bound, DeflateState};
use crate::checksum::{adler32, crc32};
use crate::stream::{self, Encode, Error, Flush, Status, Step};

const HIST: usize = 32 << 10;
const PIECE: usize = 256 << 10;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Wrap {
    Raw,
    Zlib,
    Gzip,
}

pub(crate) struct Core {
    st: Box<DeflateState>,
    level: u32,
    wrap: Wrap,
    buf: Vec<u8>, // the window (earlier input) and new input from `start` on
    start: usize,
    out: Vec<u8>, // compressed bytes, out[from..n] wait for the caller
    from: usize,
    n: usize,
    head: bool,    // the header is written
    aligned: bool, // nothing compressed since the stream's start or the last sync point
    done: bool,    // the stream is ended
    sum: u32,      // Adler-32 (zlib) or CRC-32 (gzip) of the input
    size: u32,     // gzip: the input length mod 2^32
}

impl Core {
    pub(crate) fn new(level: i32, wrap: Wrap) -> Core {
        Core {
            st: DeflateState::new(),
            level: if level < 0 { 6 } else { level.min(9) as u32 },
            wrap,
            buf: Vec::new(),
            start: 0,
            out: vec![0u8; deflate_bound(PIECE) + 64],
            from: 0,
            n: 0,
            head: false,
            aligned: true,
            done: false,
            sum: if wrap == Wrap::Zlib { 1 } else { 0 },
            size: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        // a used state gives the same bytes as a fresh one once its positions are farther
        // than any match reaches
        self.st.slid(self.buf.len() + (1 << 16));
        self.st.begin();
        self.buf.clear();
        self.start = 0;
        self.from = 0;
        self.n = 0;
        self.head = false;
        self.aligned = true;
        self.done = false;
        self.sum = if self.wrap == Wrap::Zlib { 1 } else { 0 };
        self.size = 0;
    }

    fn header(&mut self) {
        let h: &[u8] = match self.wrap {
            Wrap::Raw => &[],
            Wrap::Zlib => match self.level {
                0 | 1 => &[0x78, 0x01],
                2..=5 => &[0x78, 0x5E],
                6 => &[0x78, 0x9C],
                _ => &[0x78, 0xDA],
            },
            Wrap::Gzip => {
                let xfl = match self.level {
                    9 => 2,
                    1 => 4,
                    _ => 0,
                };
                self.out[..10].copy_from_slice(&[0x1F, 0x8B, 8, 0, 0, 0, 0, 0, xfl, 255]);
                self.n = 10;
                self.head = true;
                return;
            }
        };
        self.out[..h.len()].copy_from_slice(h);
        self.n = h.len();
        self.head = true;
    }

    // compress the new input as a piece after the output that waits (from == n here)
    fn piece(&mut self, fin: bool, sync: bool) {
        let base = self.n;
        let new = &self.buf[self.start..];
        match self.wrap {
            Wrap::Zlib => self.sum = adler32(self.sum, new),
            Wrap::Gzip => {
                self.sum = crc32(self.sum, new);
                self.size = self.size.wrapping_add(new.len() as u32);
            }
            Wrap::Raw => {}
        }
        let had = !new.is_empty();
        let mut k = base;
        if had || fin {
            k += self.st.piece(&self.buf, self.start, self.level, fin, &mut self.out[base..]);
            self.aligned = false;
        }
        if sync && !self.aligned {
            k += self.st.sync_mark(&mut self.out[k..]);
            self.aligned = true;
        }
        if fin {
            match self.wrap {
                Wrap::Raw => {}
                Wrap::Zlib => {
                    self.out[k..k + 4].copy_from_slice(&self.sum.to_be_bytes());
                    k += 4;
                }
                Wrap::Gzip => {
                    self.out[k..k + 4].copy_from_slice(&self.sum.to_le_bytes());
                    self.out[k + 4..k + 8].copy_from_slice(&self.size.to_le_bytes());
                    k += 8;
                }
            }
            self.done = true;
        }
        self.n = k;
        // keep the last 32 KiB as the window of the next piece
        self.start = self.buf.len();
        if self.buf.len() > HIST {
            let drop = self.buf.len() - HIST;
            self.buf.drain(..drop);
            self.start = HIST;
            self.st.slid(drop);
        }
    }
}

impl Encode for Core {
    fn encode(&mut self, input: &[u8], out: &mut [u8], flush: Flush) -> Result<Step, Error> {
        let (mut read, mut written) = (0, 0);
        loop {
            stream::deliver(&self.out, &mut self.from, self.n, out, &mut written);
            if self.from < self.n {
                return Ok(Step { read, written, status: Status::Output });
            }
            self.from = 0;
            self.n = 0;
            if self.done {
                return Ok(Step { read, written, status: Status::End });
            }
            if !self.head {
                self.header();
                continue;
            }
            let k = (PIECE - (self.buf.len() - self.start)).min(input.len() - read);
            self.buf.extend_from_slice(&input[read..read + k]);
            read += k;
            let full = self.buf.len() - self.start == PIECE;
            if full && (read < input.len() || flush == Flush::None) {
                self.piece(false, false);
                continue;
            }
            match flush {
                Flush::None => return Ok(Step { read, written, status: Status::Input }),
                Flush::Sync => {
                    if self.buf.len() == self.start && self.aligned {
                        return Ok(Step { read, written, status: Status::Input });
                    }
                    self.piece(false, true);
                }
                Flush::Finish => self.piece(true, false),
            }
        }
    }
}
