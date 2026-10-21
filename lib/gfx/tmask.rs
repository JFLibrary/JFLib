// tmask -- sparse tiled A8 masks                                          [src/gfx/tmask.s]
//
//   TMask::new(w, h) -> Option          1 <= w, h <= 65535, every pixel 0
//   clear(value)                        every pixel = value (0 / 255 need no memory)
//   get(x, y) -> u8                     coverage of one pixel (0 outside the mask)
//   span(x, y, maxn) -> (Span, n)       run of n (1..maxn) pixels from (x, y): Empty (all 0),
//                                       Full (all 255) or Bytes inside one tile (n <= 64);
//                                       sentinel runs span tiles; outside -> Empty
//   row(x, y, n, buf) -> (&[u8], bool)  the n bytes of (x .. x+n-1, y): inside a tile when
//                                       possible, else buf is filled (true = buf returned)
//   row_w(x, y) -> Option<&mut [u8]>    writable bytes from (x, y) to the end of its tile row
//                                       (at most w - x); materialises the tile
//   tile_w(tx, ty) -> Option<&mut [u8]> writable 64 x 64 tile;   tile_set(tx, ty, full)
//   fill_rect(x0, y0, x1, y1, value)    (x1, y1 exclusive, clipped)
//   combine(src, op) / combine_block(tx, ty, Block, op) / copy(src) / clone()
//   invert(), threshold(level), bbox() -> Option<[i32; 4]>, optimize(), count_tiles()
//   load_a8(src, stride), store_a8(dst, stride)
//   raster_path(path, ras, rule, op), raster_polygon(path, ras, pts, rule, op)
//   trace(cb) -> i32                    outlines of the pixels >= 128 as closed polylines in
//                                       pixel-edge coordinates, cb(points) -> 0 = go on
//
// Tiles are 64 x 64 bytes (row stride 64).  A slot holds Empty, Full or a tile; tiles come
// from a pool (count_tiles reports the ones in use, as the assembly does).  Pixels of edge
// tiles outside w x h are ignored everywhere.  Ops return 0 / ERR_* like the assembly.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use super::path::Path;
use super::raster::Raster;
use super::{div255, ERR_FAIL};
use rt::fx::{cvtt, maxss, minss};
use simd::{Simd, Sse2};

pub const TM_TILE: usize = 64;
const TM_SHIFT: u32 = 6;
const TB: usize = 4096; // bytes per tile
pub const TM_MAX_SIDE: u32 = 65535;
pub const TM_REPLACE: u32 = 0; // src
pub const TM_ADD: u32 = 1; // max(dst, src)
pub const TM_SUB: u32 = 2; // round(dst * (255 - src) / 255)
pub const TM_INTERSECT: u32 = 3; // round(dst * src / 255)
pub const TM_XOR: u32 = 4; // |dst - src|
pub const TM_OP_COUNT: u32 = 5;
pub const TM_TRACE_LEVEL: u8 = 128;
const CHUNK_TILES: u32 = 64; // raster_path: tiles per rasterized chunk

#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Empty,
    Full,
    Tile(u32),
}

/// a run of tm_span
pub enum Span<'a> {
    Empty,
    Full,
    Bytes(&'a [u8]),
}

/// a 64 x 64 source block of combine_block
#[derive(Clone, Copy)]
pub enum Block<'a> {
    Empty,
    Full,
    Data(&'a [u8], usize), // bytes, row stride (64 readable bytes in each of 64 rows)
}

// block operations
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bo {
    Max,
    Sub,
    Mul,
    AbsDiff,
    Inv,
}

pub struct TMask {
    pub w: u32,
    pub h: u32,
    tw: u32,
    th: u32,
    slots: Vec<Slot>,
    pool: Vec<Box<[u8; TB]>>,
    free: Vec<u32>,
    ntiles: u32,
}

fn bo_byte(op: Bo, d: u8, s: u8) -> u8 {
    match op {
        Bo::Max => d.max(s),
        Bo::Sub => div255(d as u32 * (255 - s as u32)) as u8,
        Bo::Mul => div255(d as u32 * s as u32) as u8,
        Bo::AbsDiff => d.abs_diff(s),
        Bo::Inv => 255 - d,
    }
}

// blk_class: 0 all zero / 1 all 255 / 2 mixed, looking at vw bytes of vh rows only
fn blk_class(src: &[u8], stride: usize, vw: usize, vh: usize) -> u32 {
    let (mut any, mut all) = (false, true);
    for r in 0..vh {
        for &b in &src[r * stride..r * stride + vw] {
            any |= b != 0;
            all &= b == 255;
        }
    }
    if !any {
        0
    } else if all {
        1
    } else {
        2
    }
}

// dst = f(dst, src) for the 64 rows of a tile, src rows of 64 bytes at stride
#[inline(always)]
fn rows_op(tile: &mut [u8; TB], src: &[u8], stride: usize, f: impl Fn(u8, u8) -> u8) {
    for (r, row) in tile.as_chunks_mut::<64>().0.iter_mut().enumerate() {
        let s: &[u8; 64] = src[r * stride..r * stride + 64].try_into().expect("64 bytes");
        for (d, &v) in row.iter_mut().zip(s) {
            *d = f(*d, v);
        }
    }
}

// bit k = byte k of the 64-byte row has its top bit set (>= 128); perf-raster: pmovmskb as
// blk_rowbits does
#[inline]
fn rowbits_top(row: &[u8; 64]) -> u64 {
    let x = Sse2;
    let mut bits = 0u64;
    for k in 0..4 {
        bits |= (x.movemask8(x.load(&row[k * 16..k * 16 + 16])) as u64) << (16 * k);
    }
    bits
}

// bit k = byte k of the 64-byte row is not 0 (blk_rownz)
#[inline]
fn rowbits_nz(row: &[u8; 64]) -> u64 {
    let x = Sse2;
    let mut bits = 0u64;
    for k in 0..4 {
        bits |= (x.movemask8(x.cmpeq8(x.load(&row[k * 16..k * 16 + 16]), x.zero())) as u64) << (16 * k);
    }
    !bits
}

fn vmask64(vw: u32) -> u64 {
    if vw >= 64 { u64::MAX } else { (1u64 << vw) - 1 }
}

// f2i_clamp: trunc(clamp(x, -1, 70000)) (NaN -> -1)
fn f2i_clamp(x: f32) -> i32 {
    cvtt(minss(maxss(x, -1.0), 70000.0))
}

impl TMask {
    /// a mask of w x h pixels, every pixel 0 (None: bad size)  [tm_alloc]
    pub fn new(w: u32, h: u32) -> Option<TMask> {
        if !(1..=TM_MAX_SIDE).contains(&w) || !(1..=TM_MAX_SIDE).contains(&h) {
            return None;
        }
        let tw = (w + 63) >> TM_SHIFT;
        let th = (h + 63) >> TM_SHIFT;
        Some(TMask { w, h, tw, th, slots: vec![Slot::Empty; (tw * th) as usize], pool: Vec::new(), free: Vec::new(), ntiles: 0 })
    }

    pub fn tiles_x(&self) -> u32 {
        self.tw
    }

    pub fn tiles_y(&self) -> u32 {
        self.th
    }

    // ---- tiles and slots

    fn si(&self, tx: u32, ty: u32) -> usize {
        (ty * self.tw + tx) as usize
    }

    // in-range width / height of a tile (1..64)
    fn tile_dims(&self, tx: u32, ty: u32) -> (usize, usize) {
        let vw = (self.w - (tx << TM_SHIFT)).min(64);
        let vh = (self.h - (ty << TM_SHIFT)).min(64);
        (vw as usize, vh as usize)
    }

    fn tile_alloc(&mut self) -> u32 {
        self.ntiles += 1;
        if let Some(t) = self.free.pop() {
            return t;
        }
        self.pool.push(Box::new([0u8; TB]));
        self.pool.len() as u32 - 1
    }

    // a sentinel into a slot, releasing its tile
    fn slot_put(&mut self, i: usize, v: Slot) {
        if let Slot::Tile(t) = self.slots[i] {
            self.free.push(t);
            self.ntiles -= 1;
        }
        self.slots[i] = v;
    }

    // the slot's tile, materialised if it was a sentinel (fill: with the sentinel's value)
    fn slot_tile(&mut self, i: usize, fill: bool) -> u32 {
        match self.slots[i] {
            Slot::Tile(t) => t,
            s => {
                let t = self.tile_alloc();
                self.slots[i] = Slot::Tile(t);
                if fill {
                    self.pool[t as usize].fill(if s == Slot::Full { 255 } else { 0 });
                }
                t
            }
        }
    }

    // a uniform tile becomes a sentinel
    fn slot_norm(&mut self, i: usize, vw: usize, vh: usize) {
        if let Slot::Tile(t) = self.slots[i] {
            match blk_class(&self.pool[t as usize][..], 64, vw, vh) {
                0 => self.slot_put(i, Slot::Empty),
                1 => self.slot_put(i, Slot::Full),
                _ => {}
            }
        }
    }

    fn drop_tiles(&mut self) {
        self.pool.clear();
        self.free.clear();
        self.ntiles = 0;
    }

    fn blk_op(&mut self, t: u32, src: Block, op: Bo) {
        let tile = &mut self.pool[t as usize];
        match src {
            // perf-raster: 64-byte rows, one loop per op (the compiler vectorizes them)
            Block::Data(s, stride) => match op {
                Bo::Max => rows_op(tile, s, stride, |d, s| d.max(s)),
                Bo::Sub => rows_op(tile, s, stride, |d, s| div255(d as u32 * (255 - s as u32)) as u8),
                Bo::Mul => rows_op(tile, s, stride, |d, s| div255(d as u32 * s as u32) as u8),
                Bo::AbsDiff => rows_op(tile, s, stride, |d, s| d.abs_diff(s)),
                Bo::Inv => rows_op(tile, s, stride, |d, _| 255 - d),
            },
            _ => {
                // (Inv only: the source is the tile itself)
                for b in tile.iter_mut() {
                    *b = bo_byte(op, *b, *b);
                }
            }
        }
    }

    // ---- public API

    /// every pixel = value  [tm_clear]
    pub fn clear(&mut self, value: u8) -> i32 {
        self.drop_tiles();
        let s = if value == 255 { Slot::Full } else { Slot::Empty };
        self.slots.iter_mut().for_each(|x| *x = s);
        if value != 0 && value != 255 {
            for i in 0..self.slots.len() {
                let t = self.slot_tile(i, false);
                self.pool[t as usize].fill(value);
            }
        }
        0
    }

    /// coverage of one pixel (0 outside)  [tm_get]
    pub fn get(&self, x: i32, y: i32) -> u8 {
        if x as u32 >= self.w || y as u32 >= self.h {
            return 0;
        }
        let (x, y) = (x as u32, y as u32);
        match self.slots[self.si(x >> TM_SHIFT, y >> TM_SHIFT)] {
            Slot::Empty => 0,
            Slot::Full => 255,
            Slot::Tile(t) => self.pool[t as usize][((y & 63) << TM_SHIFT | (x & 63)) as usize],
        }
    }

    /// a run of n (1..maxn) pixels from (x, y)  [tm_span]
    pub fn span(&self, x: i32, y: i32, maxn: i32) -> (Span<'_>, i32) {
        if maxn <= 0 {
            return (Span::Empty, 0);
        }
        if y as u32 >= self.h {
            return (Span::Empty, maxn); // row outside (also y < 0)
        }
        if x < 0 {
            return (Span::Empty, (-(x as i64)).min(maxn as i64) as i32);
        }
        if x as u32 >= self.w {
            return (Span::Empty, maxn);
        }
        let maxn = maxn.min(self.w as i32 - x);
        let (xu, yu) = (x as u32, y as u32);
        let mut i = self.si(xu >> TM_SHIFT, yu >> TM_SHIFT);
        let s = self.slots[i];
        let mut left = 64 - (xu & 63) as i32;
        match s {
            Slot::Tile(t) => {
                let o = ((yu & 63) << TM_SHIFT | (xu & 63)) as usize;
                let n = left.min(maxn);
                (Span::Bytes(&self.pool[t as usize][o..o + n as usize]), n)
            }
            _ => {
                while left < maxn {
                    i += 1;
                    if self.slots[i] != s {
                        break;
                    }
                    left += 64;
                }
                (if s == Slot::Full { Span::Full } else { Span::Empty }, left.min(maxn))
            }
        }
    }

    /// the n bytes of (x .. x+n-1, y): (bytes, true when they are buf)  [tm_row]
    pub fn row<'a>(&'a self, x: i32, y: i32, n: i32, buf: &'a mut [u8]) -> (&'a [u8], bool) {
        if n <= 0 {
            return (&buf[..0], true);
        }
        if let (Span::Bytes(b), k) = self.span(x, y, n) {
            if k == n {
                return (b, false); // one tile holds the whole run
            }
        }
        let (mut x, mut left, mut o) = (x, n, 0usize);
        while left > 0 {
            let (sp, k) = self.span(x, y, left);
            let k = k as usize;
            match sp {
                Span::Empty => buf[o..o + k].fill(0),
                Span::Full => buf[o..o + k].fill(255),
                Span::Bytes(b) => buf[o..o + k].copy_from_slice(b),
            }
            o += k;
            x += k as i32;
            left -= k as i32;
        }
        (&buf[..n as usize], true)
    }

    /// writable bytes from (x, y) to the end of its tile row (at most w - x)  [tm_row_w]
    pub fn row_w(&mut self, x: i32, y: i32) -> Option<&mut [u8]> {
        if x as u32 >= self.w || y as u32 >= self.h {
            return None;
        }
        let (xu, yu) = (x as u32, y as u32);
        let i = self.si(xu >> TM_SHIFT, yu >> TM_SHIFT);
        let t = self.slot_tile(i, true);
        let o = ((yu & 63) << TM_SHIFT | (xu & 63)) as usize;
        let n = ((64 - (xu & 63)).min(self.w - xu)) as usize;
        Some(&mut self.pool[t as usize][o..o + n])
    }

    /// writable 64 x 64 tile (materialised with its content)  [tm_tile_w]
    pub fn tile_w(&mut self, tx: u32, ty: u32) -> Option<&mut [u8]> {
        if tx >= self.tw || ty >= self.th {
            return None;
        }
        let i = self.si(tx, ty);
        let t = self.slot_tile(i, true);
        Some(&mut self.pool[t as usize][..])
    }

    /// tile (tx, ty) := all 0 / all 255  [tm_tile_set]
    pub fn tile_set(&mut self, tx: u32, ty: u32, full: bool) {
        if tx >= self.tw || ty >= self.th {
            return;
        }
        let i = self.si(tx, ty);
        self.slot_put(i, if full { Slot::Full } else { Slot::Empty });
    }

    /// set the pixels of the rectangle (x1, y1 exclusive, clipped) to value; whole tiles of
    /// 0 / 255 become sentinels  [tm_fill_rect]
    pub fn fill_rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, value: u8) -> i32 {
        let x0 = x0.max(0);
        let y0 = y0.max(0);
        let x1 = x1.min(self.w as i32);
        let y1 = y1.min(self.h as i32);
        if x1 <= x0 || y1 <= y0 {
            return 0;
        }
        let kind = match value {
            0 => Some(Slot::Empty),
            255 => Some(Slot::Full),
            _ => None,
        };
        for ty in (y0 as u32 >> TM_SHIFT)..=((y1 - 1) as u32 >> TM_SHIFT) {
            for tx in (x0 as u32 >> TM_SHIFT)..=((x1 - 1) as u32 >> TM_SHIFT) {
                let (vw, vh) = self.tile_dims(tx, ty);
                let (tx0, ty0) = ((tx << TM_SHIFT) as i32, (ty << TM_SHIFT) as i32);
                let (tx1, ty1) = (tx0 + vw as i32, ty0 + vh as i32);
                let (rx0, rx1, ry0, ry1) = (x0.max(tx0), x1.min(tx1), y0.max(ty0), y1.min(ty1));
                let i = self.si(tx, ty);
                if let Some(k) = kind {
                    if rx0 == tx0 && rx1 == tx1 && ry0 == ty0 && ry1 == ty1 {
                        self.slot_put(i, k);
                        continue;
                    }
                    if self.slots[i] == k {
                        continue; // already uniform with this value
                    }
                }
                let t = self.slot_tile(i, true) as usize;
                for y in ry0..ry1 {
                    let o = ((y & 63) << TM_SHIFT) as usize + (rx0 & 63) as usize;
                    self.pool[t][o..o + (rx1 - rx0) as usize].fill(value);
                }
            }
        }
        0
    }

    /// combine a 64 x 64 block into tile (tx, ty)  [tm_combine_block]
    pub fn combine_block(&mut self, tx: u32, ty: u32, src: Block, op: u32) -> i32 {
        if tx >= self.tw || ty >= self.th {
            return 0;
        }
        let (vw, vh) = self.tile_dims(tx, ty);
        let i = self.si(tx, ty);
        let mut src = src;
        if let Block::Data(b, stride) = src {
            match blk_class(b, stride, vw, vh) {
                0 => src = Block::Empty, // uniform source -> sentinel
                1 => src = Block::Full,
                _ => {}
            }
        }
        let d = self.slots[i];
        enum Act {
            Nothing,
            Set(Slot),
            Copy,
            InvCopy,
            Op(Bo),
            InvTile,
        }
        let act = match op {
            TM_REPLACE => match src {
                Block::Empty => Act::Set(Slot::Empty),
                Block::Full => Act::Set(Slot::Full),
                _ => Act::Copy,
            },
            TM_ADD => match (src, d) {
                (Block::Empty, _) => Act::Nothing,
                (Block::Full, _) => Act::Set(Slot::Full),
                (_, Slot::Empty) => Act::Copy,
                (_, Slot::Full) => Act::Nothing,
                _ => Act::Op(Bo::Max),
            },
            TM_SUB => match (src, d) {
                (Block::Empty, _) => Act::Nothing,
                (Block::Full, _) => Act::Set(Slot::Empty),
                (_, Slot::Empty) => Act::Nothing,
                (_, Slot::Full) => Act::InvCopy,
                _ => Act::Op(Bo::Sub),
            },
            TM_INTERSECT => match (src, d) {
                (Block::Empty, _) => Act::Set(Slot::Empty),
                (Block::Full, _) => Act::Nothing,
                (_, Slot::Empty) => Act::Nothing,
                (_, Slot::Full) => Act::Copy,
                _ => Act::Op(Bo::Mul),
            },
            TM_XOR => match (src, d) {
                (Block::Empty, _) => Act::Nothing,
                (Block::Full, Slot::Empty) => Act::Set(Slot::Full),
                (Block::Full, Slot::Full) => Act::Set(Slot::Empty),
                (Block::Full, _) => Act::InvTile,
                (_, Slot::Empty) => Act::Copy,
                (_, Slot::Full) => Act::InvCopy,
                _ => Act::Op(Bo::AbsDiff),
            },
            _ => return ERR_FAIL,
        };
        match act {
            Act::Nothing => {}
            Act::Set(s) => self.slot_put(i, s),
            Act::Copy | Act::InvCopy => {
                let t = self.slot_tile(i, false) as usize;
                if let Block::Data(b, stride) = src {
                    let tile = &mut self.pool[t];
                    // perf-raster: rows of 64 bytes (blk_copy) instead of a byte loop
                    if matches!(act, Act::InvCopy) {
                        rows_op(tile, b, stride, |_, v| 255 - v);
                    } else {
                        for (r, row) in tile.as_chunks_mut::<64>().0.iter_mut().enumerate() {
                            row.copy_from_slice(&b[r * stride..r * stride + 64]);
                        }
                    }
                }
            }
            Act::InvTile => {
                if let Slot::Tile(t) = d {
                    self.blk_op(t, Block::Empty, Bo::Inv);
                }
            }
            Act::Op(bo) => {
                if let Slot::Tile(t) = d {
                    self.blk_op(t, src, bo);
                    self.slot_norm(i, vw, vh);
                }
            }
        }
        0
    }

    /// dst = dst OP src per pixel (same size)  [tm_combine]
    pub fn combine(&mut self, src: &TMask, op: u32) -> i32 {
        if self.w != src.w || self.h != src.h {
            return ERR_FAIL;
        }
        for ty in 0..self.th {
            for tx in 0..self.tw {
                let b = match src.slots[src.si(tx, ty)] {
                    Slot::Empty => Block::Empty,
                    Slot::Full => Block::Full,
                    Slot::Tile(t) => Block::Data(&src.pool[t as usize][..], 64),
                };
                let r = self.combine_block(tx, ty, b, op);
                if r != 0 {
                    return r;
                }
            }
        }
        0
    }

    /// dst = dst OP dst (tm_combine with the same mask as both operands)
    pub fn combine_self(&mut self, op: u32) -> i32 {
        let copy = self.clone();
        self.combine(&copy, op)
    }

    /// dst = src (same size)  [tm_copy]
    pub fn copy(&mut self, src: &TMask) -> i32 {
        self.combine(src, TM_REPLACE)
    }

    /// v = 255 - v  [tm_invert]
    pub fn invert(&mut self) {
        for i in 0..self.slots.len() {
            match self.slots[i] {
                Slot::Empty => self.slots[i] = Slot::Full,
                Slot::Full => self.slots[i] = Slot::Empty,
                Slot::Tile(t) => self.blk_op(t, Block::Empty, Bo::Inv),
            }
        }
    }

    /// v = v >= level ? 255 : 0 (level clamped to 1..255)  [tm_threshold]
    pub fn threshold(&mut self, level: i32) {
        let level = level.clamp(1, 255) as u8;
        for ty in 0..self.th {
            for tx in 0..self.tw {
                let i = self.si(tx, ty);
                if let Slot::Tile(t) = self.slots[i] {
                    for b in self.pool[t as usize].iter_mut() {
                        *b = if *b >= level { 255 } else { 0 };
                    }
                    let (vw, vh) = self.tile_dims(tx, ty);
                    self.slot_norm(i, vw, vh);
                }
            }
        }
    }

    /// every uniform tile becomes a sentinel  [tm_optimize]
    pub fn optimize(&mut self) {
        for ty in 0..self.th {
            for tx in 0..self.tw {
                let (vw, vh) = self.tile_dims(tx, ty);
                let i = self.si(tx, ty);
                self.slot_norm(i, vw, vh);
            }
        }
    }

    /// (tiles held in memory, slots that are all 255)  [tm_count_tiles]
    pub fn count_tiles(&self) -> (u32, u32) {
        (self.ntiles, self.slots.iter().filter(|s| **s == Slot::Full).count() as u32)
    }

    /// x0, y0, x1, y1 (exclusive) of the pixels != 0, None when the mask is empty  [tm_bbox]
    pub fn bbox(&self) -> Option<[i32; 4]> {
        let mut b = [i32::MAX, i32::MAX, -1, -1]; // inclusive
        let mut add = |x0: i32, y0: i32, x1: i32, y1: i32| {
            b[0] = b[0].min(x0);
            b[1] = b[1].min(y0);
            b[2] = b[2].max(x1);
            b[3] = b[3].max(y1);
        };
        for ty in 0..self.th {
            for tx in 0..self.tw {
                let (vw, vh) = self.tile_dims(tx, ty);
                let (x0, y0) = ((tx << TM_SHIFT) as i32, (ty << TM_SHIFT) as i32);
                match self.slots[self.si(tx, ty)] {
                    Slot::Empty => {}
                    Slot::Full => add(x0, y0, x0 + vw as i32 - 1, y0 + vh as i32 - 1),
                    Slot::Tile(t) => {
                        let m = vmask64(vw as u32);
                        for r in 0..vh {
                            let row: &[u8; 64] = self.pool[t as usize][r * 64..r * 64 + 64].try_into().expect("64 bytes");
                            let bits = rowbits_nz(row) & m;
                            if bits != 0 {
                                let (lo, hi) = (bits.trailing_zeros() as i32, 63 - bits.leading_zeros() as i32);
                                add(x0 + lo, y0 + r as i32, x0 + hi, y0 + r as i32);
                            }
                        }
                    }
                }
            }
        }
        if b[2] < 0 { None } else { Some([b[0], b[1], b[2] + 1, b[3] + 1]) }
    }

    /// the mask becomes a copy of the dense w x h A8 image at src (row stride `stride`)  [tm_load_a8]
    pub fn load_a8(&mut self, src: &[u8], stride: usize) -> i32 {
        let mut tmp = vec![0u8; TB];
        for ty in 0..self.th {
            for tx in 0..self.tw {
                let (vw, vh) = self.tile_dims(tx, ty);
                let o = (ty as usize * 64) * stride + tx as usize * 64;
                let r = if vw == 64 && vh == 64 {
                    self.combine_block(tx, ty, Block::Data(&src[o..], stride), TM_REPLACE)
                } else {
                    // edge tile: the in-range part into the temporary block
                    for row in 0..vh {
                        tmp[row * 64..row * 64 + vw].copy_from_slice(&src[o + row * stride..o + row * stride + vw]);
                    }
                    self.combine_block(tx, ty, Block::Data(&tmp, 64), TM_REPLACE)
                };
                if r != 0 {
                    return r;
                }
            }
        }
        0
    }

    /// write the mask as a dense w x h A8 image (row stride `stride`)  [tm_store_a8]
    pub fn store_a8(&self, dst: &mut [u8], stride: usize) {
        for ty in 0..self.th {
            for tx in 0..self.tw {
                let (vw, vh) = self.tile_dims(tx, ty);
                let o = (ty as usize * 64) * stride + tx as usize * 64;
                let s = self.slots[self.si(tx, ty)];
                for row in 0..vh {
                    let d = &mut dst[o + row * stride..o + row * stride + vw];
                    match s {
                        Slot::Empty => d.fill(0),
                        Slot::Full => d.fill(255),
                        Slot::Tile(t) => d.copy_from_slice(&self.pool[t as usize][row * 64..row * 64 + vw]),
                    }
                }
            }
        }
    }

    /// rasterize the path (subpaths closed implicitly, rule RULE_*) with exact anti-aliased
    /// coverage and combine it into the mask with op (TM_*)  [tm_raster_path]
    pub fn raster_path(&mut self, path: &Path, ras: &mut Raster, rule: u32, op: u32) -> i32 {
        if op >= TM_OP_COUNT {
            return ERR_FAIL;
        }
        path.fill_lines(ras);
        let mut op = op;
        if op == TM_REPLACE {
            self.clear(0);
            op = TM_ADD;
        }
        let range = if ras.has_lines() {
            let lb = ras.lbox();
            let tx0 = (f2i_clamp(lb[0]).max(0) as u32) >> TM_SHIFT;
            let ty0 = (f2i_clamp(lb[1]).max(0) as u32) >> TM_SHIFT;
            let ex = f2i_clamp(lb[2]) + 2 + 63; // ceil + the spill pixel, rounded up to tiles
            let ey = f2i_clamp(lb[3]) + 2 + 63;
            if ex < 0 || ey < 0 {
                None
            } else {
                let tx1 = ((ex as u32) >> TM_SHIFT).min(self.tw);
                let ty1 = ((ey as u32) >> TM_SHIFT).min(self.th);
                if tx1 <= tx0 || ty1 <= ty0 { None } else { Some((tx0, tx1, ty0, ty1)) }
            }
        } else {
            None
        };
        let Some((tx0, tx1, ty0, ty1)) = range else {
            // nothing covered: only intersect changes the mask (replace was cleared above)
            if op == TM_INTERSECT {
                self.clear(0);
            }
            return 0;
        };
        if op == TM_INTERSECT {
            for ty in 0..self.th {
                for tx in 0..self.tw {
                    if ty < ty0 || ty >= ty1 || tx < tx0 || tx >= tx1 {
                        self.tile_set(tx, ty, false);
                    }
                }
            }
        }
        let chunk = (tx1 - tx0).min(CHUNK_TILES);
        let rowb = (chunk * 64) as usize;
        let mut buf = vec![0u8; rowb * 64];
        for ty in ty0..ty1 {
            let mut cx = tx0;
            while cx < tx1 {
                let cols = (tx1 - cx).min(chunk);
                let (bx, by) = ((cx * 64) as i32, (ty * 64) as i32);
                let clip = [bx, by, (bx + (cols * 64) as i32).min(self.w as i32), (by + 64).min(self.h as i32)];
                // (the assembly: ras_fill with k_a8_over at alpha 255 into the zeroed buffer;
                // every row is emitted once, so that is the coverage itself -- perf-raster:
                // a copy, and the written row parts are remembered: tile columns outside
                // all of them are empty blocks (what blk_class finds there), and only they
                // are zeroed again)
                let mut rows = [(0usize, 0usize); 64]; // (offset, bytes) per buffer row
                let (mut lo, mut hi) = (usize::MAX, 0usize);
                ras.fill_rows(clip, rule, &mut |y, x, cov| {
                    let (r, o) = ((y - by) as usize, (x - bx) as usize);
                    buf[r * rowb + o..r * rowb + o + cov.len()].copy_from_slice(cov);
                    rows[r] = (o, cov.len());
                    lo = lo.min(o);
                    hi = hi.max(o + cov.len());
                });
                for col in 0..cols {
                    let c0 = (col * 64) as usize;
                    let src = if c0 + 64 <= lo || c0 >= hi { Block::Empty } else { Block::Data(&buf[c0..], rowb) };
                    let r = self.combine_block(cx + col, ty, src, op);
                    if r != 0 {
                        return r;
                    }
                }
                for (r, &(o, n)) in rows.iter().enumerate() {
                    buf[r * rowb + o..r * rowb + o + n].fill(0);
                }
                cx += chunk;
            }
        }
        0
    }

    /// raster_path of one closed polygon given as points in mask pixels (replaces the path)
    pub fn raster_polygon(&mut self, path: &mut Path, ras: &mut Raster, pts: &[(f32, f32)], rule: u32, op: u32) -> i32 {
        path.reset();
        if let Some(&(x, y)) = pts.first() {
            path.move_f(x, y);
            for &(x, y) in &pts[1..] {
                path.line_f(x, y);
            }
        }
        self.raster_path(path, ras, rule, op)
    }

    // ---- outline tracing

    // bit i = pixel (tx*64 + i, y) is inside (>= 128); 0 outside the mask
    fn tr_rowbits(&self, tx: u32, y: i32) -> u64 {
        if y as u32 >= self.h {
            return 0;
        }
        let m = vmask64((self.w - (tx << TM_SHIFT)).min(64));
        match self.slots[self.si(tx, y as u32 >> TM_SHIFT)] {
            Slot::Empty => 0,
            Slot::Full => m,
            Slot::Tile(t) => {
                let o = ((y as u32 & 63) << TM_SHIFT) as usize;
                // (>= TM_TRACE_LEVEL = 128: the top bit)
                rowbits_top(self.pool[t as usize][o..o + 64].try_into().expect("64 bytes")) & m
            }
        }
    }

    /// outlines of the pixels with coverage >= 128 along pixel edges: cb(points) for every
    /// closed contour (corners in pixel-edge coordinates, implicitly closed, inside on the
    /// right); returns 0 or the first nonzero cb result  [tm_trace]
    pub fn trace(&self, cb: &mut dyn FnMut(&[(i32, i32)]) -> i32) -> i32 {
        let mut tr = Tracer { tm: self, vis: vec![None; (self.th + 1) as usize], pts: Vec::new(), cb };
        let tw = self.tw as usize;
        for ty in 0..self.th {
            // perf-raster: the slots of this tile row and the row above as slices
            let row = &self.slots[ty as usize * tw..(ty as usize + 1) * tw];
            let above_row = if ty == 0 { &[][..] } else { &self.slots[(ty as usize - 1) * tw..ty as usize * tw] };
            for tx in 0..self.tw {
                let s = row[tx as usize];
                let above = above_row.get(tx as usize).copied().unwrap_or(Slot::Empty);
                let y0 = (ty << TM_SHIFT) as i32;
                let last = if (y0 + 64) < self.h as i32 { y0 + 63 } else { self.h as i32 };
                let r = if let Slot::Tile(_) = s {
                    let mut r = 0;
                    for y in y0..=last {
                        r = tr.scan(tx, y);
                        if r != 0 {
                            break;
                        }
                    }
                    r
                } else {
                    // uniform tile: only its top row (if the tile above differs) and the bottom
                    let mut r = 0;
                    if s != above {
                        r = tr.scan(tx, y0);
                    }
                    if r == 0 && last == self.h as i32 && s == Slot::Full {
                        r = tr.scan(tx, last);
                    }
                    r
                };
                if r != 0 {
                    return r;
                }
            }
        }
        0
    }
}

impl Clone for TMask {
    fn clone(&self) -> TMask {
        let mut m = TMask::new(self.w, self.h).expect("valid size");
        m.copy(self);
        m
    }
}

// lattice walk: pixel ahead-right / ahead-left of point (px, py) for direction d
// (0 = +x, 1 = +y, 2 = -x, 3 = -y)
const ARX: [i32; 4] = [0, -1, -1, 0];
const ARY: [i32; 4] = [0, 0, -1, -1];
const ALX: [i32; 4] = [0, 0, -1, -1];
const ALY: [i32; 4] = [-1, 0, 0, -1];

struct Tracer<'a> {
    tm: &'a TMask,
    // visited horizontal edges per (tile column, 64 lattice rows), allocated on first use;
    // perf-raster: one list of tile columns per lattice tile row (th + 1 of them), so a huge
    // sparse mask needs no table of tw * (th + 1) entries
    vis: Vec<Option<Box<[Option<Box<[u64; 64]>>]>>>,
    pts: Vec<(i32, i32)>,
    cb: &'a mut dyn FnMut(&[(i32, i32)]) -> i32,
}

impl Tracer<'_> {
    fn word(&mut self, tx: u32, y: i32) -> &mut u64 {
        let tw = self.tm.tw as usize;
        let cols = self.vis[(y as u32 >> TM_SHIFT) as usize].get_or_insert_with(|| vec![None; tw].into_boxed_slice());
        let blk = cols[tx as usize].get_or_insert_with(|| Box::new([0u64; 64]));
        &mut blk[(y & 63) as usize]
    }

    fn mark(&mut self, x: i32, y: i32) {
        *self.word(x as u32 >> TM_SHIFT, y) |= 1u64 << (x & 63);
    }

    // trace the contour through the horizontal boundary edge (x .. x+1, y) and report it;
    // below: pixel (x, y) is inside (walk right along its top), else (x, y-1) (walk left)
    fn follow(&mut self, x: i32, y: i32, below: bool) -> i32 {
        let (mut p0x, p0y, d0) = (x, y, if below { 0 } else { 2 });
        if !below {
            p0x += 1;
        }
        self.pts.clear();
        self.pts.push((p0x, p0y));
        let (mut px, mut py, mut d) = (p0x, p0y, d0);
        loop {
            match d {
                0 => {
                    self.mark(px, py);
                    px += 1;
                }
                1 => py += 1,
                2 => {
                    self.mark(px - 1, py);
                    px -= 1;
                }
                _ => py -= 1,
            }
            let ar = self.tm.get(px + ARX[d], py + ARY[d]);
            let al = self.tm.get(px + ALX[d], py + ALY[d]);
            let nd = if ar < TM_TRACE_LEVEL {
                (d + 1) & 3 // turn right
            } else if al < TM_TRACE_LEVEL {
                d // straight on
            } else {
                (d + 3) & 3 // turn left
            };
            if px == p0x && py == p0y && nd == d0 {
                // arrived straight: the start is no corner
                let pts = if d == d0 { &self.pts[1..] } else { &self.pts[..] };
                return (self.cb)(pts);
            }
            if nd != d {
                self.pts.push((px, py));
            }
            d = nd;
        }
    }

    // trace every contour through an unvisited boundary edge of lattice row y in column tx
    fn scan(&mut self, tx: u32, y: i32) -> i32 {
        let above = self.tm.tr_rowbits(tx, y - 1);
        let below = self.tm.tr_rowbits(tx, y);
        let edges = above ^ below;
        if edges == 0 {
            return 0;
        }
        loop {
            let open = !*self.word(tx, y) & edges;
            if open == 0 {
                return 0;
            }
            let bit = open.trailing_zeros() as i32;
            let r = self.follow((tx << TM_SHIFT) as i32 + bit, y, below >> bit & 1 != 0);
            if r != 0 {
                return r;
            }
        }
    }
}
