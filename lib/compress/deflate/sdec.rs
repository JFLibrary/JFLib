// deflate::sdec -- deflate::Decoder, raw DEFLATE data in pieces (zlib::Decoder and
// gzip::Decoder wrap it)
//
// The decoder of dec.rs in steps that start again when the input runs out: a block header
// (at most 563 bytes: 17 bits, 19 code length codes, 316 lengths of up to 14 bits), one
// symbol (a literal, the end of a block, or a length and a distance: at most 48 bits), or a
// piece of a stored block.  A step that runs out leaves the bit position at its start; the
// bytes from there wait in the stage (stream::drive).  While a step finds 8 input bytes and
// room for a whole match, codes_fast of dec.rs runs the symbols as for a whole input.
//
// The output goes into a window of 128 KiB: the last 32 KiB of output (the reach of a
// distance) and room for new bytes.  When the caller has taken everything, the window
// keeps the last 32 KiB and moves them to its start.  A distance is checked against the
// bytes in the window, which hold every byte of the output while there are fewer than 32 KiB
// and the last 32 KiB afterwards: the same checks as for a whole input.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::dec::{self, codes_fast, copy_match, dist_x, fixed_lens, lit_x, Fast, Huff, InflateState};
use super::dec::{FAST_MASK, FAST_ROOM, X_BAD, X_EOB, X_KIND};
use super::Error as DError;
use crate::stream::{self, Decode, Engine, Error, Stage, Step, Stop};

const HIST: usize = 32 << 10;
const WIN: usize = HIST + (96 << 10);
pub(crate) const STAGE_MAX: usize = 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Head,
    Stored(usize),
    Codes,
    Done,
}

/// why a step stopped
enum Halt {
    Need,
    Bad(DError),
}

impl From<DError> for Halt {
    fn from(e: DError) -> Halt {
        Halt::Bad(e)
    }
}

/// the bit reader of a run: src from pos on, the bits before it in bb (cnt of them, zeros
/// above)
struct Rd<'a> {
    src: &'a [u8],
    pos: usize,
    bb: u64,
    cnt: u32,
}

impl<'a> Rd<'a> {
    fn at(src: &'a [u8], bit: usize) -> Rd<'a> {
        let (p, k) = (bit / 8, (bit % 8) as u32);
        if k == 0 {
            Rd { src, pos: p, bb: 0, cnt: 0 }
        } else {
            Rd { src, pos: p + 1, bb: (src[p] >> k) as u64, cnt: 8 - k }
        }
    }

    fn bit(&self) -> usize {
        self.pos * 8 - self.cnt as usize
    }

    fn save(&self) -> (usize, u64, u32) {
        (self.pos, self.bb, self.cnt)
    }

    fn restore(&mut self, s: (usize, u64, u32)) {
        (self.pos, self.bb, self.cnt) = s;
    }

    /// at least n bits in the buffer (n <= 32); false when the input ends first
    #[inline(always)]
    fn pull(&mut self, n: u32) -> bool {
        while self.cnt < n {
            if self.pos >= self.src.len() {
                return false;
            }
            self.bb |= (self.src[self.pos] as u64) << self.cnt;
            self.pos += 1;
            self.cnt += 8;
        }
        true
    }

    #[inline(always)]
    fn get(&mut self, n: u32) -> u32 {
        let v = (self.bb & ((1u64 << n) - 1)) as u32;
        self.bb >>= n;
        self.cnt -= n;
        v
    }

    /// n bits (n <= 16), or Halt::Need
    #[inline(always)]
    fn take(&mut self, n: u32) -> Result<u32, Halt> {
        if !self.pull(n) {
            return Err(Halt::Need);
        }
        Ok(self.get(n))
    }

    /// a symbol of a u16 fast table (the code length code)
    fn huff(&mut self, h: &Huff) -> Result<u32, Halt> {
        let full = self.pull(15);
        let e = h.fast[(self.bb & FAST_MASK) as usize] as u32;
        let (s, l) = if e != 0 {
            (e >> 6, e & 63)
        } else {
            match h.slow(self.bb) {
                Some(x) => x,
                // with fewer than 15 bits the code may still go on
                None => return Err(if full { Halt::Bad(DError::Code) } else { Halt::Need }),
            }
        };
        if l > self.cnt {
            return Err(Halt::Need);
        }
        self.bb >>= l;
        self.cnt -= l;
        Ok(s)
    }

    /// the fast-loop entry of the next code of xt / h (its bits dropped)
    #[inline(always)]
    fn code(&mut self, xt: &[u32; 1024], h: &Huff, xf: fn(u32) -> u32) -> Result<u32, Halt> {
        let full = self.pull(15);
        let mut e = xt[(self.bb & FAST_MASK) as usize];
        if e == 0 {
            e = xf(h.slow_entry(self.bb));
            if e == 0 {
                return Err(if full { Halt::Bad(DError::Code) } else { Halt::Need });
            }
        }
        let l = e & 15;
        if l > self.cnt {
            return Err(Halt::Need);
        }
        self.bb >>= l;
        self.cnt -= l;
        Ok(e)
    }

    /// the base of entry e plus its extra bits
    #[inline(always)]
    fn value(&mut self, e: u32) -> Result<usize, Halt> {
        let x = (e >> 10) & 15;
        Ok((e >> 16) as usize + self.take(x)? as usize)
    }
}

pub(crate) struct Core {
    st: Box<InflateState>,
    win: Vec<u8>,
    o: usize,    // the end of the output in win
    from: usize, // win[from..o] waits for the caller
    fin: bool,   // the current block is the last one
    phase: Phase,
}

impl Core {
    fn new() -> Core {
        Core { st: InflateState::new(), win: vec![0u8; WIN], o: 0, from: 0, fin: false, phase: Phase::Head }
    }

    fn reset(&mut self) {
        self.o = 0;
        self.from = 0;
        self.fin = false;
        self.phase = Phase::Head;
    }

    fn end_block(&mut self) {
        self.phase = if self.fin { Phase::Done } else { Phase::Head };
    }

    // a block header -> the phase of its data
    fn head(&mut self, r: &mut Rd) -> Result<(), Halt> {
        if !r.pull(3) {
            return Err(Halt::Need);
        }
        let fin = r.get(1) != 0;
        let st = &mut *self.st;
        match r.get(2) {
            0 => {
                let k = r.cnt & 7;
                r.bb >>= k;
                r.cnt -= k;
                if !r.pull(32) {
                    return Err(Halt::Need);
                }
                let n = r.get(16);
                if n != r.get(16) ^ 0xFFFF {
                    return Err(Halt::Bad(DError::Block));
                }
                self.phase = Phase::Stored(n as usize);
            }
            1 => {
                if !st.fixed {
                    fixed_lens(&mut st.lens);
                    st.tabs.lit.build_x(&st.lens[..288], &mut st.tabs.lx, lit_x)?;
                    st.tabs.dist.build_x(&st.lens[288..320], &mut st.tabs.dx, dist_x)?;
                    st.fixed = true;
                }
                self.phase = Phase::Codes;
            }
            2 => {
                st.fixed = false;
                let hlit = r.take(5)? as usize + 257;
                let hdist = r.take(5)? as usize + 1;
                let hclen = r.take(4)? as usize + 4;
                if hlit > 286 || hdist > 30 {
                    return Err(Halt::Bad(DError::Block));
                }
                let total = hlit + hdist;
                st.cll = [0; 24];
                for &c in &dec::CLORDER[..hclen] {
                    st.cll[c as usize] = r.take(3)? as u8;
                }
                st.cl.build(&st.cll[..19])?;
                let mut i = 0usize;
                while i < total {
                    let sym = r.huff(&st.cl)?;
                    if sym < 16 {
                        st.lens[i] = sym as u8;
                        i += 1;
                        continue;
                    }
                    let (rep, val) = match sym {
                        16 => {
                            if i == 0 {
                                return Err(Halt::Bad(DError::Block));
                            }
                            (r.take(2)? as usize + 3, st.lens[i - 1])
                        }
                        17 => (r.take(3)? as usize + 3, 0),
                        _ => (r.take(7)? as usize + 11, 0),
                    };
                    if i + rep > total {
                        return Err(Halt::Bad(DError::Block));
                    }
                    st.lens[i..i + rep].fill(val);
                    i += rep;
                }
                if st.lens[256] == 0 {
                    return Err(Halt::Bad(DError::Code));
                }
                st.tabs.lit.build_x(&st.lens[..hlit], &mut st.tabs.lx, lit_x)?;
                st.tabs.dist.build_x(&st.lens[hlit..hlit + hdist], &mut st.tabs.dx, dist_x)?;
                self.phase = Phase::Codes;
            }
            _ => return Err(Halt::Bad(DError::Block)),
        }
        self.fin = fin;
        Ok(())
    }

    // one symbol: Ok(true) at the end of the block; room for 258 bytes is there
    fn symbol(&mut self, r: &mut Rd) -> Result<bool, Halt> {
        let t = &self.st.tabs;
        let e = r.code(&t.lx, &t.lit, lit_x)?;
        if e & X_KIND == 0 {
            self.win[self.o] = (e >> 16) as u8;
            self.o += 1;
            return Ok(false);
        }
        if e & X_KIND == X_EOB {
            return Ok(true);
        }
        if e & X_KIND == X_BAD {
            return Err(Halt::Bad(DError::Code));
        }
        let len = r.value(e)?;
        let e = r.code(&t.dx, &t.dist, dist_x)?;
        if e & X_KIND != 0 {
            return Err(Halt::Bad(DError::Distance));
        }
        let dis = r.value(e)?;
        if dis > self.o {
            return Err(Halt::Bad(DError::Distance));
        }
        copy_match(&mut self.win, self.o, dis, len);
        self.o += len;
        Ok(false)
    }

    fn steps(&mut self, r: &mut Rd) -> Result<Stop, Error> {
        loop {
            match self.phase {
                Phase::Done => return Ok(Stop::End),
                Phase::Head => {
                    let save = r.save();
                    match self.head(r) {
                        Ok(()) => {}
                        Err(Halt::Need) => {
                            r.restore(save);
                            return Ok(Stop::Need);
                        }
                        Err(Halt::Bad(e)) => return Err(e.into()),
                    }
                }
                Phase::Stored(left) => {
                    if left == 0 {
                        self.end_block();
                        continue;
                    }
                    let room = WIN - self.o;
                    if room == 0 {
                        return Ok(Stop::Full);
                    }
                    if r.cnt >= 8 {
                        // whole bytes the symbol loop read ahead
                        self.win[self.o] = r.bb as u8;
                        r.bb >>= 8;
                        r.cnt -= 8;
                        self.o += 1;
                        self.phase = Phase::Stored(left - 1);
                        continue;
                    }
                    let avail = r.src.len() - r.pos;
                    if avail == 0 {
                        return Ok(Stop::Need);
                    }
                    let n = left.min(room).min(avail);
                    self.win[self.o..self.o + n].copy_from_slice(&r.src[r.pos..r.pos + n]);
                    r.pos += n;
                    self.o += n;
                    self.phase = Phase::Stored(left - n);
                }
                Phase::Codes => {
                    if r.src.len() - r.pos >= 16 && WIN - self.o > FAST_ROOM {
                        let mut f = Fast { rest: &r.src[r.pos..], bb: r.bb, cnt: r.cnt, o: self.o };
                        let eob = codes_fast(&mut f, &self.st.tabs, &mut self.win)?;
                        r.pos = r.src.len() - f.rest.len();
                        r.cnt = f.cnt;
                        r.bb = f.bb & ((1u64 << f.cnt) - 1); // (the fast loop's look-ahead bits)
                        self.o = f.o;
                        if eob {
                            self.end_block();
                        }
                        continue;
                    }
                    if WIN - self.o < 258 {
                        return Ok(Stop::Full);
                    }
                    let save = r.save();
                    match self.symbol(r) {
                        Ok(true) => self.end_block(),
                        Ok(false) => {}
                        Err(Halt::Need) => {
                            r.restore(save);
                            return Ok(Stop::Need);
                        }
                        Err(Halt::Bad(e)) => return Err(e.into()),
                    }
                }
            }
        }
    }
}

impl Engine for Core {
    fn run(&mut self, src: &[u8], bit: &mut usize) -> Result<Stop, Error> {
        let mut r = Rd::at(src, *bit);
        let stop = self.steps(&mut r);
        *bit = r.bit();
        stop
    }

    fn room(&mut self) -> bool {
        if self.from < self.o {
            return false;
        }
        if WIN - self.o < 1024 {
            let keep = self.o.min(HIST);
            self.win.copy_within(self.o - keep..self.o, 0);
            self.o = keep;
            self.from = keep;
        }
        true
    }

    fn deliver(&mut self, out: &mut [u8], written: &mut usize) {
        stream::deliver(&self.win, &mut self.from, self.o, out, written);
    }

    fn waiting(&self) -> bool {
        self.from < self.o
    }

    fn ended(&self) -> bool {
        self.phase == Phase::Done
    }
}

/// a decoder of raw DEFLATE data (RFC 1951) that takes its input in pieces (stream::Decode):
/// about 150 KB of state, of which 128 KiB are the window
pub struct Decoder {
    core: Core,
    stage: Stage,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder { core: Core::new(), stage: Stage { buf: Vec::new(), bit0: 0 } }
    }

    /// start a new stream (the memory stays)
    pub fn reset(&mut self) {
        self.core.reset();
        self.stage.buf.clear();
        self.stage.bit0 = 0;
    }
}

impl Default for Decoder {
    fn default() -> Decoder {
        Decoder::new()
    }
}

impl Decode for Decoder {
    fn decode(&mut self, input: &[u8], out: &mut [u8]) -> Result<Step, Error> {
        stream::drive(&mut self.core, &mut self.stage, STAGE_MAX, input, out)
    }

    fn finish(&self) -> Result<(), Error> {
        if self.core.ended() && !self.core.waiting() { Ok(()) } else { Err(Error::Truncated) }
    }
}
