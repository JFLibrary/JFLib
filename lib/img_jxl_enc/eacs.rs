// eacs -- AC strategy selection of the VarDCT writer (libjxl 0.11.1 enc_ac_strategy.cc:
// EstimateEntropy, FindBest8x8Transform, TryMergeAcs, FindBestFirstLevelDivisionForSquare,
// ProcessRectACS)                                                           [jxl-plus]
//
//   Cfg                        ACSConfig: the XYB image (after inverse gaborish), the initial
//                              quant field, the masks, the cost multipliers
//   tile(cfg, tx, ty, cmap_x, cmap_b, tier, d) -> [u8; 64]   strategies of one 64x64 tile
//                              (a << 1 | first block, row stride 8)
//
// A tile only looks at its own blocks (no transform crosses a 64x64 boundary), so tiles run in
// parallel and in any order.

use alloc::vec;
use alloc::vec::Vec;

use simd::{Kernel, Simd};

use crate::eaq::Img;
use crate::emath::{fast_powf, sqrtf};
use crate::etx::{self, Qm, Tx};

/// ACSConfig
pub struct Cfg<'a> {
    pub img: &'a Img,
    pub qm: &'a Qm,
    pub qf: &'a [f32],
    pub m1: &'a [f32],
    pub xb: usize,
    pub yb: usize,
    pub info_loss: f32,
    pub cost_delta: f32,
    pub zeros_mul: f32,
}

impl<'a> Cfg<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(img: &'a Img, qm: &'a Qm, qf: &'a [f32], m1: &'a [f32], d: f32) -> Cfg<'a> {
        let ratio = (d + 0.137_317_43) / (1.0 + 0.137_317_43);
        Cfg {
            img,
            qm,
            qf,
            m1,
            xb: img.w / 8,
            yb: img.h / 8,
            info_loss: 1.2 * fast_powf(ratio, 0.336_778_07),
            zeros_mul: 9.308_906 * fast_powf(ratio, 0.509_909_3),
            cost_delta: 10.833_273 * fast_powf(ratio, 0.367_029_4),
        }
    }
    #[inline]
    fn quant(&self, bx: usize, by: usize) -> f32 {
        self.qf[by * self.xb + bx]
    }
}

/// per-thread scratch of the heuristics
pub struct Scratch {
    pub tx: Tx,
    block: Vec<f32>,
    mem: Vec<f32>,
}

impl Default for Scratch {
    fn default() -> Self {
        Scratch::new()
    }
}

impl Scratch {
    pub fn new() -> Scratch {
        Scratch { tx: Tx::new(), block: vec![0.0; 3 * 4096], mem: vec![0.0; 4096] }
    }
}

static CHANNEL_MUL: [f32; 3] = [117_166_946.0, 1.0, 1.266_770_1];

/// one channel's quantization in EstimateEntropy: mem = the dequantized error, the sums of
/// sqrt |q| by position mod 8 and the non-zeros (SSE2: two registers per 8 positions, AVX2:
/// one; the same sums either way)
struct QuantK<'a> {
    bk: &'a [f32],
    by: &'a [f32],
    im: &'a [f32],
    m: &'a [f32],
    mem: &'a mut [f32],
    cf: f32,
    qn: f32,
}

impl Kernel for QuantK<'_> {
    type Out = ([f32; 8], u32);
    #[inline(always)]
    fn run<S: Simd>(self, s: S) -> ([f32; 8], u32) {
        let l = S::BYTES / 4;
        let (cf, qn) = (s.splatf(self.cf), s.splatf(self.qn));
        let absm = s.splat32(0x7FFF_FFFF);
        let one = s.splat32(1);
        let mut ent = [s.zero(), s.zero()];
        let mut nz = s.zero();
        let n = self.bk.len();
        let mut i = 0;
        while i + 8 <= n {
            for (h, e) in ent.iter_mut().enumerate().take(8 / l) {
                let k = i + h * l;
                let d = s.subf(s.load_f32(&self.bk[k..]), s.mulf(s.load_f32(&self.by[k..]), cf));
                let val = s.mulf(d, s.mulf(s.load_f32(&self.im[k..]), qn));
                let qi = s.cvtr_f32_i32(val);
                let rval = s.cvt_i32_f32(qi);
                s.store_f32(&mut self.mem[k..], s.mulf(s.load_f32(&self.m[k..]), s.subf(val, rval)));
                let q = s.and(rval, absm);
                *e = s.addf(*e, s.sqrtf(q));
                // non-zero: the integer is not 0
                nz = s.add32(nz, s.andnot(s.cmpeq32(qi, s.zero()), one));
            }
            i += 8;
        }
        let mut o = [0f32; 8];
        if l == 8 {
            s.store_f32(&mut o, ent[0]);
        } else {
            s.store_f32(&mut o[..4], ent[0]);
            s.store_f32(&mut o[4..], ent[1]);
        }
        let mut c = [0i32; 8];
        s.store_i32(&mut c[..l], nz);
        (o, c[..l].iter().sum::<i32>() as u32)
    }
}

/// the masked error of one channel in EstimateEntropy: sums of (|mask| px)^8 by position mod 8
struct LossK<'a> {
    mask: &'a [f32], // from the block's top left, row stride ms
    ms: usize,
    px: &'a [f32], // rows of w
    w: usize,
    rows: usize,
}

impl Kernel for LossK<'_> {
    type Out = [f32; 8];
    #[inline(always)]
    fn run<S: Simd>(self, s: S) -> [f32; 8] {
        let l = S::BYTES / 4;
        let absm = s.splat32(0x7FFF_FFFF);
        let mut acc = [s.zero(), s.zero()];
        for y in 0..self.rows {
            let mut x = 0;
            while x + 8 <= self.w {
                for (h, a) in acc.iter_mut().enumerate().take(8 / l) {
                    let k = x + h * l;
                    let mk = s.and(s.load_f32(&self.mask[y * self.ms + k..]), absm);
                    let mut v = s.mulf(mk, s.load_f32(&self.px[y * self.w + k..]));
                    v = s.mulf(v, v);
                    v = s.mulf(v, v);
                    v = s.mulf(v, v);
                    *a = s.addf(*a, v);
                }
                x += 8;
            }
        }
        let mut o = [0f32; 8];
        if l == 8 {
            s.store_f32(&mut o, acc[0]);
        } else {
            s.store_f32(&mut o[..4], acc[0]);
            s.store_f32(&mut o[4..], acc[1]);
        }
        o
    }
}

/// EstimateEntropy of strategy acs at pixel (x, y)
pub fn estimate(cfg: &Cfg, sc: &mut Scratch, acs: usize, entropy_mul: f32, x: usize, y: usize, cmap: &[f32; 3]) -> f32 {
    let size = etx::size_of(acs);
    let stride = cfg.img.w;
    for c in 0..3 {
        let o = y * stride + x;
        etx::from_pixels(&mut sc.tx, acs, &cfg.img.p[c][o..], stride, &mut sc.block[c * size..(c + 1) * size]);
    }
    let (cx, cy) = etx::cover(acs);
    let nb = cx * cy;
    let mut qn = 0.0f32;
    if nb == 1 {
        qn = cfg.quant(x / 8, y / 8);
    } else if nb == 2 {
        qn = if cy == 2 {
            cfg.quant(x / 8, y / 8).max(cfg.quant(x / 8, y / 8 + 1))
        } else {
            cfg.quant(x / 8, y / 8).max(cfg.quant(x / 8 + 1, y / 8))
        };
    } else {
        for iy in 0..cy {
            for ix in 0..cx {
                let mut q = cfg.quant(x / 8 + ix, y / 8 + iy);
                q *= q;
                q *= q;
                q *= q;
                qn += q * q;
            }
        }
        qn /= nb as f32;
        qn = fast_powf(qn, 1.0 / 16.0);
    }
    let mut entropy = 0.0f32;
    let mut loss = 0.0f32;
    let w = cx * 8;
    for c in 0..3 {
        let (m, im) = cfg.qm.of(acs, c);
        let cf = cmap[c];
        // quantization (round to nearest even through the 1.5 * 2^23 bias; sums in 8 lanes so
        // the loop vectorizes): sqrt of the quantized values (a table up to 15), non-zeros
        // quantization: sqrt of the quantized values, non-zeros (sums by position mod 8)
        let (ent8, nz) = {
            let block = &sc.block;
            simd::dispatch(QuantK {
                bk: &block[c * size..(c + 1) * size],
                by: &block[size..2 * size],
                im: &im[..size],
                m: &m[..size],
                mem: &mut sc.mem[..size],
                cf,
                qn,
            })
        };
        let ent = ((ent8[0] + ent8[1]) + (ent8[2] + ent8[3])) + ((ent8[4] + ent8[5]) + (ent8[6] + ent8[7]));
        let nz = nz as usize;
        // the quantization error in pixels, masked, 8th power
        {
            let (mem, block) = (&mut sc.mem, &mut sc.block);
            etx::to_pixels_fast(&mut sc.tx, acs, &mut mem[..size], &mut block[..size], w);
        }
        let loss8 = simd::dispatch(LossK { mask: &cfg.m1[y * stride + x..], ms: stride, px: &sc.block[..size], w, rows: cy * 8 });
        let lossc = ((loss8[0] + loss8[1]) + (loss8[2] + loss8[3])) + ((loss8[4] + loss8[5]) + (loss8[6] + loss8[7]));
        loss += CHANNEL_MUL[c] * lossc;
        entropy += cfg.cost_delta * ent;
        let nbits = crate::eans::ceil_log2(nz as u32 + 1) + 1;
        entropy += cfg.zeros_mul * (crate::eans::ceil_log2(nbits + 17) + nbits) as f32;
    }
    let n = (nb * 64) as f32;
    let ls = sqrtf(sqrtf(sqrtf(loss / n))) * n / qn;
    entropy *= entropy_mul;
    entropy + cfg.info_loss * ls
}

struct T8 {
    t: usize,
    max_tier: i32,
    mul: f32,
}

static T8S: [T8; 10] = [
    T8 { t: etx::DCT, max_tier: 9, mul: 0.8 },
    T8 { t: etx::DCT4X4, max_tier: 5, mul: 1.08 },
    T8 { t: etx::DCT2X2, max_tier: 5, mul: 0.95 },
    T8 { t: etx::DCT4X8, max_tier: 4, mul: 0.859_316_4 },
    T8 { t: etx::DCT8X4, max_tier: 4, mul: 0.859_316_4 },
    T8 { t: etx::IDENTITY, max_tier: 5, mul: 1.042_754_3 },
    T8 { t: 14, max_tier: 4, mul: 0.817_794_9 },
    T8 { t: 15, max_tier: 4, mul: 0.817_794_9 },
    T8 { t: 16, max_tier: 4, mul: 0.817_794_9 },
    T8 { t: 17, max_tier: 4, mul: 0.817_794_9 },
];

/// FindBest8x8Transform -> (strategy, entropy)
fn best8x8(cfg: &Cfg, sc: &mut Scratch, x: usize, y: usize, tier: i32, d: f32, cmap: &[f32; 3]) -> (usize, f32) {
    let mut best = 1e30f32;
    let mut bt = etx::DCT;
    for t in &T8S {
        if t.max_tier < tier {
            continue;
        }
        let mut mul = t.mul / T8S[0].mul;
        if (t.t == etx::DCT2X2 || t.t == etx::IDENTITY) && d < 5.0 {
            let w = ((5.0 - d) / 5.0) * ((5.0 - d) / 5.0);
            mul -= 0.4 * w;
        }
        if t.t != etx::DCT && t.t != etx::DCT2X2 && t.t != etx::IDENTITY && d > 4.0 {
            let mut m = 1.0f32;
            if d < 12.0 {
                m *= (12.0 - 4.0) / (d - 4.0);
            }
            mul += 0.5 * m;
        }
        let e = estimate(cfg, sc, t.t, mul, x, y, cmap);
        if e < best {
            bt = t.t;
            best = e;
        }
    }
    (bt, best)
}

/// the strategies of a tile (local coordinates, 8 x 8)
struct Tile {
    a: [u8; 64],
    w: usize,
    h: usize,
}

impl Tile {
    fn set(&mut self, x: usize, y: usize, acs: usize) {
        let (cx, cy) = etx::cover(acs);
        for iy in 0..cy {
            for ix in 0..cx {
                self.a[(y + iy) * 8 + x + ix] = (acs as u8) << 1 | (iy == 0 && ix == 0) as u8;
            }
        }
    }
    #[inline]
    fn first(&self, x: usize, y: usize) -> bool {
        self.a[y * 8 + x] & 1 != 0
    }
    #[inline]
    fn strategy(&self, x: usize, y: usize) -> usize {
        (self.a[y * 8 + x] >> 1) as usize
    }
    /// MultiBlockTransformCrossesHorizontalBoundary
    fn crosses_h(&self, start_x: usize, y: usize, end_x: usize) -> bool {
        if start_x >= self.w || y >= self.h || y % 8 == 0 {
            return false;
        }
        let end_x = end_x.min(self.w);
        let lim = start_x & !7;
        let mut sx = start_x;
        while sx != lim && !self.first(sx, y) {
            sx -= 1;
        }
        let mut x = sx;
        while x < end_x {
            if self.first(x, y) {
                x += etx::cover(self.strategy(x, y)).0;
            } else {
                return true;
            }
        }
        false
    }
    /// MultiBlockTransformCrossesVerticalBoundary
    fn crosses_v(&self, x: usize, start_y: usize, end_y: usize) -> bool {
        if x >= self.w || start_y >= self.h || x % 8 == 0 {
            return false;
        }
        let end_y = end_y.min(self.h);
        let lim = start_y & !7;
        let mut sy = start_y;
        while sy != lim && !self.first(x, sy) {
            sy -= 1;
        }
        let mut y = sy;
        while y < end_y {
            if self.first(x, y) {
                y += etx::cover(self.strategy(x, y)).1;
            } else {
                return true;
            }
        }
        false
    }
}

fn set_entropy(cx: usize, cy: usize, acs: usize, e: f32, est: &mut [f32; 64]) {
    let (w, h) = etx::cover(acs);
    for dy in 0..h {
        for dx in 0..w {
            est[(cy + dy) * 8 + cx + dx] = 0.0;
        }
    }
    est[cy * 8 + cx] = e;
}

fn acs_square(b: usize) -> usize {
    match b {
        2 => etx::DCT16X16,
        4 => etx::DCT32X32,
        _ => etx::DCT64X64,
    }
}

fn acs_vsplit(b: usize) -> usize {
    match b {
        2 => etx::DCT16X8,
        4 => etx::DCT32X16,
        _ => etx::DCT64X32,
    }
}

fn acs_hsplit(b: usize) -> usize {
    match b {
        2 => etx::DCT8X16,
        4 => etx::DCT16X32,
        _ => etx::DCT32X64,
    }
}

struct Ctx<'c, 'a> {
    cfg: &'c Cfg<'a>,
    bx: usize,
    by: usize,
    cmap: [f32; 3],
}

/// TryMergeAcs
fn try_merge(c: &Ctx, sc: &mut Scratch, t: &mut Tile, acs: usize, cx: usize, cy: usize, mul: f32, prio: u8, pr: &mut [u8; 64], est: &mut [f32; 64]) {
    let (w, h) = etx::cover(acs);
    let mut cur = 0.0f32;
    for iy in 0..h {
        for ix in 0..w {
            if pr[(cy + iy) * 8 + cx + ix] >= prio {
                return;
            }
            cur += est[(cy + iy) * 8 + cx + ix];
        }
    }
    let e = estimate(c.cfg, sc, acs, mul, (c.bx + cx) * 8, (c.by + cy) * 8, &c.cmap);
    if e >= cur {
        return;
    }
    for iy in 0..h {
        for ix in 0..w {
            est[(cy + iy) * 8 + cx + ix] = 0.0;
            pr[(cy + iy) * 8 + cx + ix] = prio;
        }
    }
    t.set(cx, cy, acs);
    est[cy * 8 + cx] = e;
}

/// FindBestFirstLevelDivisionForSquare
#[allow(clippy::too_many_arguments)]
fn division(c: &Ctx, sc: &mut Scratch, t: &mut Tile, blocks: usize, allow_sq: bool, cx: usize, cy: usize, mul_jxk: f32, mul_jxj: f32, est: &mut [f32; 64]) {
    let half = blocks / 2;
    let jxk = acs_vsplit(blocks);
    let kxj = acs_hsplit(blocks);
    let jxj = acs_square(blocks);
    if t.crosses_h(cx, cy, cx + blocks)
        || t.crosses_h(cx, cy + blocks, cx + blocks)
        || t.crosses_v(cx, cy, cy + blocks)
        || t.crosses_v(cx + blocks, cy, cy + blocks)
    {
        return;
    }
    let allow_jxk = !t.crosses_v(cx + half, cy, cy + blocks);
    let allow_kxj = !t.crosses_h(cx, cy + half, cx + blocks);
    let mut ent = [[0f32; 2]; 2];
    for dy in 0..blocks {
        for dx in 0..blocks {
            ent[dy / half][dx / half] += est[(cy + dy) * 8 + cx + dx];
        }
    }
    let px = |x: usize| (c.bx + x) * 8;
    let py = |y: usize| (c.by + y) * 8;
    let (mut jl, mut jr, mut kt, mut kb, mut jj) = (f32::MAX, f32::MAX, f32::MAX, f32::MAX, f32::MAX);
    if allow_jxk {
        if t.strategy(cx, cy) != jxk {
            jl = estimate(c.cfg, sc, jxk, mul_jxk, px(cx), py(cy), &c.cmap);
        }
        if t.strategy(cx + half, cy) != jxk {
            jr = estimate(c.cfg, sc, jxk, mul_jxk, px(cx + half), py(cy), &c.cmap);
        }
    }
    if allow_kxj {
        if t.strategy(cx, cy) != kxj {
            kt = estimate(c.cfg, sc, kxj, mul_jxk, px(cx), py(cy), &c.cmap);
        }
        if t.strategy(cx, cy + half) != kxj {
            kb = estimate(c.cfg, sc, kxj, mul_jxk, px(cx), py(cy + half), &c.cmap);
        }
    }
    if allow_sq {
        jj = estimate(c.cfg, sc, jxj, mul_jxj, px(cx), py(cy), &c.cmap);
    }
    let cost_jn = jl.min(ent[0][0] + ent[1][0]) + jr.min(ent[0][1] + ent[1][1]);
    let cost_nj = kt.min(ent[0][0] + ent[0][1]) + kb.min(ent[1][0] + ent[1][1]);
    if jj < cost_jn && jj < cost_nj {
        t.set(cx, cy, jxj);
        set_entropy(cx, cy, jxj, jj, est);
    } else if cost_jn < cost_nj {
        if jl < ent[0][0] + ent[1][0] {
            t.set(cx, cy, jxk);
            set_entropy(cx, cy, jxk, jl, est);
        }
        if jr < ent[0][1] + ent[1][1] {
            t.set(cx + half, cy, jxk);
            set_entropy(cx + half, cy, jxk, jr, est);
        }
    } else {
        if kt < ent[0][0] + ent[0][1] {
            t.set(cx, cy, kxj);
            set_entropy(cx, cy, kxj, kt, est);
        }
        if kb < ent[1][0] + ent[1][1] {
            t.set(cx, cy + half, kxj);
            set_entropy(cx, cy + half, kxj, kb, est);
        }
    }
}

/// ProcessRectACS of tile (ttx, tty); cmap = (Y-to-X, Y-to-B) factors of the tile
pub fn tile(cfg: &Cfg, sc: &mut Scratch, ttx: usize, tty: usize, cmap_x: f32, cmap_b: f32, tier: i32, d: f32) -> [u8; 64] {
    let bx = ttx * 8;
    let by = tty * 8;
    let w = (cfg.xb - bx).min(8);
    let h = (cfg.yb - by).min(8);
    let mut t = Tile { a: [1; 64], w, h }; // DCT8, first
    if tier > 5 {
        return t.a;
    }
    let c = Ctx { cfg, bx, by, cmap: [cmap_x, 0.0, cmap_b] };
    let mut est = [0f32; 64];
    let mul8 = 1.0 + -0.4 / (d + 1.4);
    for iy in 0..h {
        for ix in 0..w {
            let (bt, e) = best8x8(cfg, sc, 8 * (bx + ix), 8 * (by + iy), tier, d, &c.cmap);
            t.set(ix, iy, bt);
            est[iy * 8 + ix] = e * mul8;
        }
    }
    let m16x8 = 1.25f32;
    let m16x16 = 1.35f32;
    let m16x32 = 1.5f32;
    let m32x32 = 1.5f32;
    let m64x32 = 2.26f32;
    let m64x64 = 2.26f32;
    struct M {
        t: usize,
        prio: u8,
        mul: f32,
    }
    let merges = [
        M { t: etx::DCT16X8, prio: 2, mul: m16x8 },
        M { t: etx::DCT8X16, prio: 2, mul: m16x8 },
        M { t: etx::DCT16X32, prio: 4, mul: m16x32 },
        M { t: etx::DCT32X16, prio: 4, mul: m16x32 },
        M { t: etx::DCT64X32, prio: 6, mul: m64x32 },
        M { t: etx::DCT32X64, prio: 6, mul: m64x32 },
    ];
    let mut pr = [0u8; 64];
    for m in &merges {
        let (cw, ch) = etx::cover(m.t);
        let mut cy = 0usize;
        while cy + ch <= h {
            let mut cx = 0usize;
            while cx + cw <= w {
                let step_done = 'blk: {
                    if cy + 7 < h && cx + 7 < w {
                        if m.t == etx::DCT32X64 {
                            if (cy | cx) % 8 == 0 {
                                division(&c, sc, &mut t, 8, true, cx, cy, m.mul, m64x64, &mut est);
                            }
                            break 'blk true;
                        } else if m.t == etx::DCT32X16 {
                            break 'blk true;
                        }
                    }
                    if (m.t == etx::DCT16X32 && cy % 4 != 0) || (m.t == etx::DCT32X16 && cx % 4 != 0) {
                        break 'blk true;
                    }
                    if cy + 3 < h && cx + 3 < w {
                        if m.t == etx::DCT16X32 {
                            if (cy | cx) % 4 == 0 {
                                division(&c, sc, &mut t, 4, true, cx, cy, m.mul, m32x32, &mut est);
                            }
                            break 'blk true;
                        } else if m.t == etx::DCT32X16 {
                            break 'blk true;
                        }
                    }
                    if (m.t == etx::DCT16X32 && cy % 4 != 0) || (m.t == etx::DCT32X16 && cx % 4 != 0) {
                        break 'blk true;
                    }
                    if cy + 1 < h && cx + 1 < w {
                        if m.t == etx::DCT8X16 {
                            if (cy | cx) % 2 == 0 {
                                division(&c, sc, &mut t, 2, true, cx, cy, m.mul, m16x16, &mut est);
                            }
                            break 'blk true;
                        } else if m.t == etx::DCT16X8 {
                            break 'blk true;
                        }
                    }
                    if (m.t == etx::DCT8X16 && cy % 2 == 1) || (m.t == etx::DCT16X8 && cx % 2 == 1) {
                        break 'blk true;
                    }
                    try_merge(&c, sc, &mut t, m.t, cx, cy, m.mul, m.prio, &mut pr, &mut est);
                    true
                };
                let _ = step_done;
                cx += cw;
            }
            cy += ch;
        }
    }
    if tier >= 5 {
        return t.a;
    }
    // non-aligned 16x8 / 8x16 / 16x16
    for cy in 0..h.saturating_sub(1) {
        for cx in 0..w.saturating_sub(1) {
            if (cy | cx) % 2 != 0 {
                division(&c, sc, &mut t, 2, true, cx, cy, m16x8, m16x16, &mut est);
            }
        }
    }
    // non-aligned 32x32, 16x32, 32x16
    let step = if tier >= 1 { 2 } else { 1 };
    let mut cy = 0usize;
    while cy + 3 < h {
        let mut cx = 0usize;
        while cx + 3 < w {
            if (cy | cx) % 4 != 0 {
                division(&c, sc, &mut t, 4, true, cx, cy, m16x32, m32x32, &mut est);
            }
            cx += step;
        }
        cy += step;
    }
    t.a
}
