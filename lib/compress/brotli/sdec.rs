// brotli::sdec -- brotli::Decoder, a Brotli stream in pieces
//
// The decoder of mod.rs as a machine of small steps, each of which starts again when the
// input runs out (stream::drive keeps the bytes of the step that ran out): the window bits,
// a meta-block header, each of the three block-switch descriptions, the context modes, the
// prefix code of a context map and then each of its entries, each prefix code of the
// literals, commands and distances, and then per command: the insert-and-copy code with its
// extra bits, each literal, the distance.  The largest step is a prefix code of 704 symbols,
// well under a kilobyte.  Copies and uncompressed meta-blocks go out in pieces as the
// caller takes them.
//
// The output goes into a window of the stream's size ((1 << WBITS) - 16, at most 16 MiB):
// it keeps that much of the output, as far as a distance may reach, and moves when it holds
// more than twice that.  Distances beyond it are static dictionary words, as in mod.rs.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::tab as T;
use super::{dict_word, read_code, Blocks, Br, Huff, CELLS, COPY, INS};
use crate::stream::{self, Decode, Engine, Error, Stage, Step, Stop};

impl From<super::Error> for Error {
    fn from(e: super::Error) -> Error {
        match e {
            super::Error::Corrupt => Error::Corrupt,
            super::Error::Limit => Error::Limit,
        }
    }
}

const STAGE_MAX: usize = 4096;
const PEND: usize = 64 << 10; // output that may wait for the caller

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ph {
    Wbits,
    Meta,
    Skip(usize),
    Raw,
    Hdr(u8),
    CmapHead { which: u8 },
    Cmap { which: u8 },
    Tree { which: u8, k: usize },
    Cmd,
    Lits(usize),
    Dist,
    Copy { dist: usize, n: usize },
    Done,
}

/// a context map being read
struct Cmap {
    size: usize,
    rlemax: usize,
    h: Option<Huff>,
    map: Vec<u8>,
    i: usize,
}

/// the header of a compressed meta-block
#[derive(Default)]
struct Mb {
    bl: Option<Blocks>,
    bi: Option<Blocks>,
    bd: Option<Blocks>,
    npostfix: u32,
    ndirect: u32,
    cmode: Vec<u8>,
    ntl: usize,
    ntd: usize,
    cmap_l: Vec<u8>,
    cmap_d: Vec<u8>,
    lit_h: Vec<Huff>,
    cmd_h: Vec<Huff>,
    dist_h: Vec<Huff>,
}

type BState = (u32, u32, u32, u32);

fn bsave(b: &Blocks) -> BState {
    (b.cur, b.last, b.second, b.left)
}

fn bload(b: &mut Blocks, s: BState) {
    (b.cur, b.last, b.second, b.left) = s;
}

pub(crate) struct Core {
    ph: Ph,
    out: Vec<u8>,
    from: usize,
    total: u64,
    window: usize,
    last: bool,
    left: usize,
    rb: [usize; 4],
    rb_idx: usize,
    mb: Box<Mb>,
    cm: Option<Cmap>,
    clen: usize,
    d0: bool,
}

impl Core {
    fn new() -> Core {
        Core {
            ph: Ph::Wbits,
            out: Vec::new(),
            from: 0,
            total: 0,
            window: 0,
            last: false,
            left: 0,
            rb: [16, 15, 11, 4],
            rb_idx: 3,
            mb: Box::default(),
            cm: None,
            clen: 0,
            d0: false,
        }
    }

    fn push(&mut self, b: u8) {
        self.out.push(b);
        self.total += 1;
    }

    fn meta_end(&mut self) {
        self.ph = if self.last { Ph::Done } else { Ph::Meta };
    }

    // one step; Err with br.short: it ran out of input
    fn step(&mut self, br: &mut Br) -> Result<(), Error> {
        match self.ph {
            Ph::Wbits => {
                let wbits = if br.bits(1)? == 0 {
                    16
                } else {
                    let n = br.bits(3)?;
                    if n != 0 {
                        17 + n
                    } else {
                        let m = br.bits(3)?;
                        if m == 1 {
                            return Err(Error::Unsupported); // a large window
                        }
                        if m != 0 { 8 + m } else { 17 }
                    }
                };
                self.window = (1usize << wbits) - 16;
                self.ph = Ph::Meta;
            }
            Ph::Meta => {
                let last = br.bits(1)? != 0;
                if last && br.bits(1)? != 0 {
                    self.ph = Ph::Done; // ISLASTEMPTY
                    return Ok(());
                }
                let nib = br.bits(2)?;
                if nib == 3 {
                    if br.bits(1)? != 0 {
                        return Err(Error::Corrupt);
                    }
                    let nb = br.bits(2)?;
                    let skip = if nb > 0 { br.bits(8 * nb)? as usize + 1 } else { 0 };
                    br.align()?;
                    self.last = last;
                    self.ph = Ph::Skip(skip);
                    return Ok(());
                }
                let mlen = br.bits(4 * (nib + 4))? as usize + 1;
                if !last && br.bits(1)? != 0 {
                    br.align()?;
                    self.last = false;
                    self.left = mlen;
                    self.ph = Ph::Raw;
                    return Ok(());
                }
                self.last = last;
                self.left = mlen;
                *self.mb = Mb::default();
                self.ph = Ph::Hdr(0);
            }
            Ph::Hdr(k) => {
                match k {
                    0 => self.mb.bl = Some(Blocks::read(br)?),
                    1 => self.mb.bi = Some(Blocks::read(br)?),
                    2 => self.mb.bd = Some(Blocks::read(br)?),
                    _ => {
                        let npostfix = br.bits(2)?;
                        let ndirect = br.bits(4)? << npostfix;
                        let n = self.mb.bl.as_ref().map_or(1, |b| b.n) as usize;
                        let mut cmode = vec![0u8; n];
                        for m in cmode.iter_mut() {
                            *m = br.bits(2)? as u8;
                        }
                        let ntl = br.varlen8()? as usize + 1;
                        self.mb.npostfix = npostfix;
                        self.mb.ndirect = ndirect;
                        self.mb.cmode = cmode;
                        self.mb.ntl = ntl;
                        self.ph = Ph::CmapHead { which: 0 };
                        return Ok(());
                    }
                }
                self.ph = Ph::Hdr(k + 1);
            }
            Ph::CmapHead { which } => {
                // the tree count (NTREESD: before the distance map), the map's prefix code
                let (size, ntrees) = if which == 0 {
                    (64 * self.mb.bl.as_ref().map_or(1, |b| b.n) as usize, self.mb.ntl)
                } else {
                    (4 * self.mb.bd.as_ref().map_or(1, |b| b.n) as usize, br.varlen8()? as usize + 1)
                };
                if ntrees < 2 {
                    if which == 1 {
                        self.mb.ntd = ntrees;
                    }
                    self.cmap_done(which, vec![0u8; size]);
                    return Ok(());
                }
                let rlemax = if br.bits(1)? != 0 { br.bits(4)? as usize + 1 } else { 0 };
                let h = read_code(br, ntrees + rlemax)?;
                if which == 1 {
                    self.mb.ntd = ntrees;
                }
                self.cm = Some(Cmap { size, rlemax, h: Some(h), map: vec![0u8; size], i: 0 });
                self.ph = Ph::Cmap { which };
            }
            Ph::Cmap { which } => {
                let cm = self.cm.as_mut().ok_or(Error::Corrupt)?;
                let h = cm.h.as_ref().ok_or(Error::Corrupt)?;
                if cm.i < cm.size {
                    let c = h.decode(br)? as usize;
                    if c == 0 {
                        cm.i += 1;
                    } else if c <= cm.rlemax {
                        let reps = (1usize << c) + br.bits(c as u32)? as usize;
                        if cm.i + reps > cm.size {
                            return Err(Error::Corrupt);
                        }
                        cm.i += reps;
                    } else {
                        cm.map[cm.i] = (c - cm.rlemax) as u8;
                        cm.i += 1;
                    }
                    return Ok(());
                }
                if br.bits(1)? != 0 {
                    // inverse move to front
                    let mut mtf: Vec<u8> = (0..=255u8).collect();
                    for v in cm.map.iter_mut() {
                        let idx = *v as usize;
                        let val = mtf[idx];
                        *v = val;
                        mtf.copy_within(0..idx, 1);
                        mtf[0] = val;
                    }
                }
                let map = core::mem::take(&mut cm.map);
                self.cm = None;
                self.cmap_done(which, map);
            }
            Ph::Tree { which, k } => {
                let (n, size) = match which {
                    0 => (self.mb.ntl, 256),
                    1 => (self.mb.bi.as_ref().map_or(1, |b| b.n) as usize, 704),
                    _ => (self.mb.ntd, 16 + self.mb.ndirect as usize + (48usize << self.mb.npostfix)),
                };
                if k < n {
                    let h = read_code(br, size)?;
                    match which {
                        0 => self.mb.lit_h.push(h),
                        1 => self.mb.cmd_h.push(h),
                        _ => self.mb.dist_h.push(h),
                    }
                    self.ph = Ph::Tree { which, k: k + 1 };
                } else if which < 2 {
                    self.ph = Ph::Tree { which: which + 1, k: 0 };
                } else {
                    self.ph = Ph::Cmd;
                }
            }
            Ph::Cmd => {
                let mb = &mut *self.mb;
                let bi = mb.bi.as_mut().ok_or(Error::Corrupt)?;
                let save = bsave(bi);
                let r = (|| -> Result<(usize, usize, bool), Error> {
                    bi.next(br)?;
                    let cmd = mb.cmd_h.get(bi.cur as usize).ok_or(Error::Corrupt)?.decode(br)?;
                    let (ib, cb, d0) = CELLS[(cmd >> 6) as usize];
                    let ic = (ib + ((cmd >> 3) & 7)) as usize;
                    let cc = (cb + (cmd & 7)) as usize;
                    let ilen = INS[ic].0 as usize + br.bits(INS[ic].1)? as usize;
                    let clen = COPY[cc].0 as usize + br.bits(COPY[cc].1)? as usize;
                    Ok((ilen, clen, d0))
                })();
                match r {
                    Ok((ilen, clen, d0)) => {
                        self.clen = clen;
                        self.d0 = d0;
                        self.ph = Ph::Lits(ilen);
                    }
                    Err(e) => {
                        bload(bi, save);
                        return Err(e);
                    }
                }
            }
            Ph::Lits(n) => {
                if n == 0 {
                    self.ph = Ph::Dist;
                    return Ok(());
                }
                let mb = &mut *self.mb;
                let bl = mb.bl.as_mut().ok_or(Error::Corrupt)?;
                let save = bsave(bl);
                let k = self.out.len();
                let p1 = if self.total > 0 { self.out[k - 1] } else { 0 } as usize;
                let p2 = if self.total > 1 { self.out[k - 2] } else { 0 } as usize;
                let r = (|| -> Result<u32, Error> {
                    bl.next(br)?;
                    let lut = &T::CONTEXT_LUT[(*mb.cmode.get(bl.cur as usize).ok_or(Error::Corrupt)? as usize) << 9..];
                    let ctx = (lut[p1] | lut[256 + p2]) as usize;
                    let tree = *mb.cmap_l.get(bl.cur as usize * 64 + ctx).ok_or(Error::Corrupt)? as usize;
                    Ok(mb.lit_h.get(tree).ok_or(Error::Corrupt)?.decode(br)?)
                })();
                match r {
                    Ok(lit) => {
                        self.push(lit as u8);
                        self.left -= 1;
                        if self.left == 0 {
                            self.meta_end();
                        } else {
                            self.ph = Ph::Lits(n - 1);
                        }
                    }
                    Err(e) => {
                        bload(bl, save);
                        return Err(e);
                    }
                }
            }
            Ph::Dist => {
                let clen = self.clen;
                let (dist, dcode) = if self.d0 {
                    (self.rb[self.rb_idx], 0usize)
                } else {
                    let mb = &mut *self.mb;
                    let bd = mb.bd.as_mut().ok_or(Error::Corrupt)?;
                    let save = bsave(bd);
                    let (rb, rb_idx) = (self.rb, self.rb_idx);
                    let (npostfix, ndirect) = (mb.npostfix as usize, mb.ndirect as usize);
                    let r = (|| -> Result<(usize, usize), Error> {
                        bd.next(br)?;
                        let dctx = if clen > 4 { 3 } else { clen - 2 };
                        let tree = *mb.cmap_d.get(bd.cur as usize * 4 + dctx).ok_or(Error::Corrupt)? as usize;
                        let dc = mb.dist_h.get(tree).ok_or(Error::Corrupt)?.decode(br)? as usize;
                        let d = if dc < 16 {
                            let r = |k: usize| rb[(rb_idx + 4 - k) & 3] as i64;
                            static IDX: [usize; 16] = [0, 1, 2, 3, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1];
                            static OFF: [i64; 16] = [0, 0, 0, 0, -1, 1, -2, 2, -3, 3, -1, 1, -2, 2, -3, 3];
                            let v = r(IDX[dc]) + OFF[dc];
                            if v <= 0 {
                                return Err(Error::Corrupt);
                            }
                            v as usize
                        } else if dc < 16 + ndirect {
                            dc - 15
                        } else {
                            let x = dc - ndirect - 16;
                            let nb = 1 + (x >> (npostfix + 1));
                            let hcode = x >> npostfix;
                            let lcode = x & ((1usize << npostfix) - 1);
                            let off = ((2 + (hcode & 1)) << nb) - 4;
                            ((off + br.bits(nb as u32)? as usize) << npostfix) + lcode + ndirect + 1
                        };
                        Ok((d, dc))
                    })();
                    match r {
                        Ok(x) => x,
                        Err(e) => {
                            bload(bd, save);
                            return Err(e);
                        }
                    }
                };
                let maxd = (self.total.min(self.window as u64)) as usize;
                if dist > maxd {
                    // a static dictionary word
                    if !(4..=24).contains(&clen) {
                        return Err(Error::Corrupt);
                    }
                    let nb = T::SIZE_BITS[clen] as usize;
                    let id = dist - maxd - 1;
                    let idx = id & ((1usize << nb) - 1);
                    let t = id >> nb;
                    if t >= T::TRANSFORMS.len() || nb == 0 {
                        return Err(Error::Corrupt);
                    }
                    let before = self.out.len();
                    dict_word(&mut self.out, clen, idx, t);
                    let n = self.out.len() - before;
                    self.total += n as u64;
                    if n > self.left {
                        return Err(Error::Corrupt);
                    }
                    self.left -= n;
                    if self.left == 0 {
                        self.meta_end();
                    } else {
                        self.ph = Ph::Cmd;
                    }
                } else {
                    if clen > self.left {
                        return Err(Error::Corrupt);
                    }
                    if dcode != 0 {
                        self.rb_idx = (self.rb_idx + 1) & 3;
                        self.rb[self.rb_idx] = dist;
                    }
                    self.left -= clen;
                    self.ph = Ph::Copy { dist, n: clen };
                }
            }
            Ph::Skip(_) | Ph::Raw | Ph::Copy { .. } | Ph::Done => {}
        }
        Ok(())
    }

    // a context map is complete (which 0: literals, 1: distances): the next part of the
    // header
    fn cmap_done(&mut self, which: u8, map: Vec<u8>) {
        if which == 0 {
            self.mb.cmap_l = map;
            self.ph = Ph::CmapHead { which: 1 };
        } else {
            self.mb.cmap_d = map;
            self.ph = Ph::Tree { which: 0, k: 0 };
        }
    }

    fn steps(&mut self, src: &[u8], br: &mut Br) -> Result<Stop, Error> {
        loop {
            match self.ph {
                Ph::Done => return Ok(Stop::End),
                Ph::Skip(n) => {
                    let p = br.pos / 8;
                    let k = n.min(src.len() - p);
                    br.pos += 8 * k;
                    if k < n {
                        self.ph = Ph::Skip(n - k);
                        return Ok(Stop::Need);
                    }
                    self.meta_end();
                    continue;
                }
                Ph::Raw => {
                    if self.left == 0 {
                        self.ph = Ph::Meta;
                        continue;
                    }
                    let room = PEND.saturating_sub(self.out.len() - self.from);
                    if room == 0 {
                        return Ok(Stop::Full);
                    }
                    let p = br.pos / 8;
                    let k = self.left.min(src.len() - p).min(room);
                    if k == 0 {
                        return Ok(Stop::Need);
                    }
                    self.out.extend_from_slice(&src[p..p + k]);
                    self.total += k as u64;
                    br.pos += 8 * k;
                    self.left -= k;
                    continue;
                }
                Ph::Copy { dist, n } => {
                    if n == 0 {
                        if self.left == 0 {
                            self.meta_end();
                        } else {
                            self.ph = Ph::Cmd;
                        }
                        continue;
                    }
                    let room = PEND.saturating_sub(self.out.len() - self.from);
                    if room == 0 {
                        return Ok(Stop::Full);
                    }
                    let k = n.min(room);
                    let s = self.out.len() - dist;
                    if dist >= k {
                        self.out.extend_from_within(s..s + k);
                    } else {
                        for j in 0..k {
                            let v = self.out[s + j];
                            self.out.push(v);
                        }
                    }
                    self.total += k as u64;
                    self.ph = Ph::Copy { dist, n: n - k };
                    continue;
                }
                _ => {}
            }
            // a step that reads input; room for a dictionary word (at most 40 bytes)
            if self.out.len() - self.from + 64 > PEND {
                return Ok(Stop::Full);
            }
            let at = br.pos;
            let ph = self.ph;
            br.short = false;
            match self.step(br) {
                Ok(()) => {}
                Err(_) if br.short => {
                    br.pos = at;
                    self.ph = ph;
                    return Ok(Stop::Need);
                }
                Err(e) => return Err(e),
            }
        }
    }
}

impl Engine for Core {
    fn run(&mut self, src: &[u8], bit: &mut usize) -> Result<Stop, Error> {
        let mut br = Br { d: src, pos: *bit, short: false };
        let r = self.steps(src, &mut br);
        *bit = br.pos;
        r
    }

    fn room(&mut self) -> bool {
        if self.from < self.out.len() {
            return false;
        }
        let keep = self.window.max(2);
        if self.out.len() > 2 * keep + PEND {
            let drop = self.out.len() - keep;
            self.out.drain(..drop);
            self.from -= drop;
        }
        true
    }

    fn deliver(&mut self, out: &mut [u8], written: &mut usize) {
        stream::deliver(&self.out, &mut self.from, self.out.len(), out, written);
    }

    fn waiting(&self) -> bool {
        self.from < self.out.len()
    }

    fn ended(&self) -> bool {
        self.ph == Ph::Done
    }
}

/// a Brotli decoder that takes its input in pieces (stream::Decode): one stream of RFC 7932,
/// its window (at most 16 MiB, as the stream says) twice and 64 KiB of output
pub struct Decoder {
    core: Core,
    stage: Stage,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder { core: Core::new(), stage: Stage { buf: Vec::new(), bit0: 0 } }
    }

    /// start a new stream
    pub fn reset(&mut self) {
        *self = Decoder::new();
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
