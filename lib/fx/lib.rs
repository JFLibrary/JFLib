// fx -- the effect and adjustment kernels: the FXDESC registry, prepare / render on ROIs from
// worker threads, deterministic per pixel                       [src/fx/*.s, inc/fx.inc]
//
// The contract (fx.inc).  An application (PaintCraft, efftest) runs an effect by
//   1. copying the source layer (FMT_ARGB, straight alpha) -> FxArgs::src,
//   2. calling desc.prepare once when it has one (global statistics, LUTs, noise tables):
//      Ok(Some(aux)) / Ok(None) / Err(ERR_*) -- ERR_CANCEL when the job was cancelled,
//   3. calling desc.render for many rectangles (ROIs) -- from several worker threads at once
//      -- until the ROIs cover the area to change; every call gets the aux of step 2,
//   4. blending the destination back through the selection mask itself
//      (gfx::straight::lerp_mask_row).
// Rules: render writes the destination only inside its ROI (Dst holds exactly those pixels)
// and may read the whole source; the result of a pixel depends only on (source, params, aux)
// -- never on the ROI split or the thread count (bit-identical); render polls the job's
// cancel flag once per row (FxArgs::cancelled) and returns ERR_CANCEL at once.  Pixels are
// straight ARGB (0xAARRGGBB = bytes B, G, R, A).  Random effects use hash32(x, y, seed,
// stream), never a sequential generator.  FXF_BANDS: render wants full-width row bands (the
// runner splits by rows only); FXF_WHOLE: one call over the whole image.
//
// API (the asm records as Rust types):
//   FxDesc   FXDESC: id (FX_*), flags (FXF_*), name (ASCII key), prepare / render (fn
//            pointers; FXD_FREE is the Drop of the aux), props (FXPROP records), category
//            (FXC_*), user (free for the registering code, e.g. a plugin index)
//   FxProp   FXPROP: typ (FP_*), dec, slot, flags (FPF_*), name, min, max, def (FP_COLOR:
//            the f64 bits are the ARGB colour -- color()), def_y (FP_PAN: the y default),
//            choices (FP_CHOICE keys)
//   Params   the parameter block: FX_NSLOTS u64 slots (i64 for FP_INT / BOOL / CHOICE / SEED,
//            f64 bits for FP_DEC / ANGLE / PAN, ARGB in the low half for FP_COLOR, raw bytes
//            for FP_BLOB); defaults(desc, &mut params) fills it
//   FxArgs   FXARGS without the destination: src, ROI x0 y0 x1 y1, params, aux, job (a
//            Cancel flag: the worker pool's JobCtl, an AtomicBool / AtomicU32), sel (the
//            selection TMask: statistics may be limited to it)
//   Dst      the destination pixels of one render call: one &mut [u8] per ROI row with the
//            pixels x0..x1 -- of_surface (one call owns the whole rectangle), tiles (disjoint
//            rectangles of one surface for parallel calls; the row slices of all of them live
//            in one caller-owned Vec), strided (rows of a tile buffer), or new (a row list)
//   Aux      Box<dyn Any + Send + Sync>: the result of prepare, read by render (aux::<T>())
//   registry use_all() registers the built-in groups (menu order), register(group) adds a
//            group (&'static [FxDesc], e.g. plugin effects), count(), get(i), find(name),
//            find_id(id), defaults(desc, params)
//   hash32(x, y, seed, stream)   deterministic per-pixel random bits (fx_common.s)
//   rows(a, dst, f)              fx_rows: f(dst_row, src_row, x0, y) for every ROI row
//
// Modules: lut (fx_lut.s), hist (fx_hist.s), adj (fx_adj.s), color (fx_color.s), stylize
// (fx_stylize.s), conv (fx_conv.s), lochist (fx_lochist.s), blur (fx_blur.s), artistic
// (fx_artistic.s), noise (fx_noise.s), photo (fx_photo.s), object (fx_object.s), warp
// (fx_warp.s), perlin (fx_perlin.s), distort (fx_distort.s), render (fx_render.s).
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod adj;
pub mod artistic;
pub mod enhance;
pub mod blur;
pub mod color;
pub mod conv;
pub mod distort;
pub mod hist;
pub mod lochist;
pub mod lut;
pub mod noise;
pub mod object;
pub mod perlin;
pub mod photo;
pub mod render;
pub mod stylize;
pub mod warp;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::any::Any;

pub use gfx::tmask::TMask;
pub use gfx::Surface;
pub use plat::JobCtl;
use rt::sync::SpinLock;

// ---------------------------------------------------------------- constants (fx.inc)

pub const FX_NSLOTS: usize = 32; // parameter slots per effect

pub const FXF_ADJ: u32 = 1; // adjustment (Korrekturen) rather than effect
pub const FXF_PREPARE: u32 = 2; // has a prepare step (statistics over the source / selection)
pub const FXF_SEED: u32 = 4; // uses a random seed (has an FP_SEED property)
pub const FXF_BANDS: u32 = 8; // render wants full-width row bands
pub const FXF_WHOLE: u32 = 16; // render must get the whole image in one call
pub const FXF_NO_DIALOG: u32 = 32; // applied immediately (e.g. Invert Colors, Sepia)
pub const FXF_SLOW: u32 = 64; // hint: show progress

pub const FXC_ADJUST: u32 = 0;
pub const FXC_ARTISTIC: u32 = 1;
pub const FXC_BLUR: u32 = 2;
pub const FXC_COLOR: u32 = 3;
pub const FXC_DISTORT: u32 = 4;
pub const FXC_NOISE: u32 = 5;
pub const FXC_OBJECT: u32 = 6;
pub const FXC_PHOTO: u32 = 7;
pub const FXC_RENDER: u32 = 8;
pub const FXC_STYLIZE: u32 = 9;

pub const FP_INT: u8 = 1; // slot: i64
pub const FP_DEC: u8 = 2; // slot: f64
pub const FP_BOOL: u8 = 3; // slot: i64 0/1
pub const FP_CHOICE: u8 = 4; // slot: i64 index
pub const FP_ANGLE: u8 = 5; // slot: f64 degrees
pub const FP_PAN: u8 = 6; // slots: f64 x, f64 y (normalised -1..1, 0 = centre)
pub const FP_COLOR: u8 = 7; // slot: u32 straight ARGB in the low half
pub const FP_SEED: u8 = 8; // slot: i64 seed
pub const FP_BLOB: u8 = 9; // slots slot .. slot + max - 1 hold raw bytes (default all 0)
pub const FPF_LOG: u8 = 1; // slider maps logarithmically

pub const FX_AUTO_LEVEL: u32 = 1; // Korrekturen
pub const FX_BLACK_WHITE: u32 = 2;
pub const FX_BRIGHT_CONTRAST: u32 = 3;
pub const FX_CURVES: u32 = 4;
pub const FX_EXPOSURE: u32 = 5;
pub const FX_HIGHLIGHTS_SHADOWS: u32 = 6;
pub const FX_HUE_SATURATION: u32 = 7;
pub const FX_INVERT_ALPHA: u32 = 8;
pub const FX_INVERT_COLORS: u32 = 9;
pub const FX_LEVELS: u32 = 10;
pub const FX_POSTERIZE: u32 = 11;
pub const FX_SEPIA: u32 = 12;
pub const FX_TEMP_TINT: u32 = 13;
pub const FX_INK_SKETCH: u32 = 20; // Künstlerisch
pub const FX_OIL_PAINTING: u32 = 21;
pub const FX_PENCIL_SKETCH: u32 = 22;
pub const FX_BOKEH_BLUR: u32 = 30; // Weichzeichnen
pub const FX_FRAGMENT_BLUR: u32 = 31;
pub const FX_GAUSSIAN_BLUR: u32 = 32;
pub const FX_MEDIAN: u32 = 33;
pub const FX_MOTION_BLUR: u32 = 34;
pub const FX_RADIAL_BLUR: u32 = 35;
pub const FX_SKETCH_BLUR: u32 = 36;
pub const FX_SQUARE_BLUR: u32 = 37;
pub const FX_SURFACE_BLUR: u32 = 38;
pub const FX_UNFOCUS: u32 = 39;
pub const FX_ZOOM_BLUR: u32 = 40;
pub const FX_QUANTIZE: u32 = 50; // Farbe
pub const FX_BULGE: u32 = 60; // Verzerren
pub const FX_CRYSTALIZE: u32 = 61;
pub const FX_DENTS: u32 = 62;
pub const FX_FROSTED_GLASS: u32 = 63;
pub const FX_MORPHOLOGY: u32 = 64;
pub const FX_PIXELATE: u32 = 65;
pub const FX_POLAR_INVERSION: u32 = 66;
pub const FX_TILE_REFLECTION: u32 = 67;
pub const FX_TWIST: u32 = 68;
pub const FX_ADD_NOISE: u32 = 70; // Rauschen
pub const FX_REDUCE_NOISE: u32 = 71;
pub const FX_DROP_SHADOW: u32 = 80; // Objekt
pub const FX_GLOW: u32 = 90; // Foto
pub const FX_RED_EYE: u32 = 91;
pub const FX_SHARPEN: u32 = 92;
pub const FX_SOFTEN_PORTRAIT: u32 = 93;
pub const FX_STRAIGHTEN: u32 = 94;
pub const FX_VIGNETTE: u32 = 95;
pub const FX_CLOUDS: u32 = 100; // Rendern
pub const FX_JULIA: u32 = 101;
pub const FX_MANDELBROT: u32 = 102;
pub const FX_TURBULENCE: u32 = 103;
pub const FX_EDGE_DETECT: u32 = 110; // Stilisieren
pub const FX_EMBOSS: u32 = 111;
pub const FX_OUTLINE: u32 = 112;
pub const FX_RELIEF: u32 = 113;

pub const ERR_NOMEM: i32 = -8;
pub const ERR_DIMS: i32 = -12;
pub const ERR_CANCEL: i32 = -14;
pub const ERR_LIMIT: i32 = -15;

const FXG_MAX: usize = 16; // registered group tables

// ---------------------------------------------------------------- records

/// the result of prepare (read-only in render; FXD_FREE = its Drop)
pub type Aux = Box<dyn Any + Send + Sync>;
/// FXD_PREPARE: Ok(Some(aux)) / Ok(None) / Err(ERR_*)
pub type PrepareFn = fn(&FxArgs) -> Result<Option<Aux>, i32>;
/// FXD_RENDER: 0 / ERR_CANCEL / ERR_*
pub type RenderFn = fn(&FxArgs, &mut Dst) -> i32;

/// an effect (FXDESC)
pub struct FxDesc {
    pub id: u32,
    pub flags: u32,
    pub name: &'static str,
    pub prepare: Option<PrepareFn>,
    pub render: RenderFn,
    pub props: &'static [FxProp],
    pub category: u32,
    /// free for the code that registers the group (0 for the built-in effects)
    pub user: usize,
}

/// a property (FXPROP); slot i of the parameter block belongs to property i, except FP_PAN
/// (2 slots) and FP_BLOB (max slots)
pub struct FxProp {
    pub typ: u8,
    pub dec: u8,
    pub slot: u8,
    pub flags: u8,
    pub name: &'static str,
    pub min: f64,
    pub max: f64,
    /// the default (FP_COLOR: the bits of this f64 are the ARGB colour)
    pub def: f64,
    /// FP_PAN: the y default (FXP_CHOICES of the asm record)
    pub def_y: f64,
    pub choices: &'static [&'static str],
}

impl FxProp {
    /// an FP_INT / DEC / BOOL / ANGLE / SEED / BLOB property (FXPROP)
    pub const fn new(typ: u8, dec: u8, slot: u8, flags: u8, name: &'static str, min: f64, max: f64, def: f64) -> FxProp {
        FxProp { typ, dec, slot, flags, name, min, max, def, def_y: 0.0, choices: &[] }
    }
    /// an FP_CHOICE property
    pub const fn choice(slot: u8, name: &'static str, max: f64, def: f64, choices: &'static [&'static str]) -> FxProp {
        FxProp { typ: FP_CHOICE, dec: 0, slot, flags: 0, name, min: 0.0, max, def, def_y: 0.0, choices }
    }
    /// an FP_COLOR property (FXPROPC: the default's bits are the colour)
    pub const fn color(slot: u8, name: &'static str, argb: u32) -> FxProp {
        FxProp { typ: FP_COLOR, dec: 0, slot, flags: 0, name, min: 0.0, max: 0.0, def: f64::from_bits(argb as u64), def_y: 0.0, choices: &[] }
    }
    /// an FP_PAN property (default x, y)
    pub const fn pan(slot: u8, name: &'static str, dec: u8, min: f64, max: f64, x: f64, y: f64) -> FxProp {
        FxProp { typ: FP_PAN, dec, slot, flags: 0, name, min, max, def: x, def_y: y, choices: &[] }
    }
    /// the default colour of an FP_COLOR property
    pub fn def_color(&self) -> u32 {
        self.def.to_bits() as u32
    }
}

/// the parameter block (FX_NSLOTS 8-byte slots)
#[derive(Clone, Copy)]
pub struct Params(pub [u64; FX_NSLOTS]);

impl Default for Params {
    fn default() -> Self {
        Params([0; FX_NSLOTS])
    }
}

impl Params {
    /// slot as i64
    #[inline]
    pub fn i(&self, slot: usize) -> i64 {
        self.0[slot] as i64
    }
    /// slot as f64
    #[inline]
    pub fn f(&self, slot: usize) -> f64 {
        f64::from_bits(self.0[slot])
    }
    /// the i64 slot clamped to lo..=hi  [prm_int]
    #[inline]
    pub fn int(&self, slot: usize, lo: i32, hi: i32) -> i32 {
        self.i(slot).max(lo as i64).min(hi as i64) as i32
    }
    /// the f64 slot clamped to lo..=hi, NaN -> lo  [prm_f64]
    #[inline]
    pub fn dbl(&self, slot: usize, lo: f64, hi: f64) -> f64 {
        let v = self.f(slot);
        let v = if v.is_nan() { lo } else { v };
        rt::fx::minsd(rt::fx::maxsd(v, lo), hi)
    }
    /// byte i of the block counted from slot `slot` (FP_BLOB)
    #[inline]
    pub fn byte(&self, slot: usize, i: usize) -> u8 {
        let k = slot * 8 + i;
        (self.0[k / 8] >> ((k % 8) * 8)) as u8
    }
    pub fn set_i(&mut self, slot: usize, v: i64) {
        self.0[slot] = v as u64;
    }
    pub fn set_f(&mut self, slot: usize, v: f64) {
        self.0[slot] = v.to_bits();
    }
    pub fn set_byte(&mut self, slot: usize, i: usize, v: u8) {
        let k = slot * 8 + i;
        let sh = (k % 8) * 8;
        self.0[k / 8] = self.0[k / 8] & !(0xFFu64 << sh) | (v as u64) << sh;
    }
}

/// a cancel flag render / prepare poll (JOB_CANCEL of FXA_JOB): the pool's JobCtl, or any
/// flag of the caller's
pub trait Cancel {
    fn cancelled(&self) -> bool;
}

impl Cancel for JobCtl {
    #[inline]
    fn cancelled(&self) -> bool {
        JobCtl::cancelled(self)
    }
}

impl Cancel for core::sync::atomic::AtomicBool {
    #[inline]
    fn cancelled(&self) -> bool {
        self.load(core::sync::atomic::Ordering::Relaxed)
    }
}

impl Cancel for core::sync::atomic::AtomicU32 {
    #[inline]
    fn cancelled(&self) -> bool {
        self.load(core::sync::atomic::Ordering::Relaxed) != 0
    }
}

/// the arguments of prepare / render (FXARGS without the destination)
#[derive(Clone, Copy)]
pub struct FxArgs<'a> {
    pub src: &'a Surface,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
    pub params: &'a Params,
    pub aux: Option<&'a (dyn Any + Send + Sync)>,
    pub job: Option<&'a (dyn Cancel + Sync)>,
    pub sel: Option<&'a TMask>,
}

impl<'a> FxArgs<'a> {
    /// arguments for the ROI (x0, y0) - (x1, y1) without aux, job and selection
    pub fn new(src: &'a Surface, roi: [i32; 4], params: &'a Params) -> FxArgs<'a> {
        FxArgs { src, x0: roi[0], y0: roi[1], x1: roi[2], y1: roi[3], params, aux: None, job: None, sel: None }
    }
    /// the job's cancel flag is set (JOB_CANCEL)
    #[inline]
    pub fn cancelled(&self) -> bool {
        match self.job {
            Some(j) => j.cancelled(),
            None => false,
        }
    }
    /// the aux of prepare as T
    #[inline]
    pub fn aux<T: 'static>(&self) -> Option<&'a T> {
        self.aux?.downcast_ref::<T>()
    }
    /// source row y (w * 4 bytes)
    #[inline]
    pub fn src_row(&self, y: i32) -> &'a [u8] {
        row(self.src, y)
    }
}

/// row y of a surface (w * 4 bytes)
#[inline]
pub fn row(sf: &Surface, y: i32) -> &[u8] {
    let o = y as usize * sf.stride as usize;
    &sf.px[o..o + sf.w as usize * 4]
}

/// pixel i of a row
#[inline(always)]
pub fn px(b: &[u8], i: usize) -> u32 {
    let q: [u8; 4] = b[4 * i..4 * i + 4].try_into().unwrap();
    u32::from_le_bytes(q)
}

/// set pixel i of a row
#[inline(always)]
pub fn put(b: &mut [u8], i: usize, v: u32) {
    b[4 * i..4 * i + 4].copy_from_slice(&v.to_le_bytes());
}

/// the destination pixels of one render call: row k holds the pixels x0..x1 of row y0 + k
pub struct Dst<'a> {
    rows: Rows<'a>,
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

enum Rows<'a> {
    /// one slice per row
    List(Vec<&'a mut [u8]>),
    /// one slice per row, in a table shared by several destinations (tiles)
    Table(&'a mut [&'a mut [u8]]),
    /// row k at k * stride of one buffer
    Strided(&'a mut [u8], usize),
}

impl<'a> Dst<'a> {
    /// a destination from row slices (each (x1 - x0) * 4 bytes, y1 - y0 of them)
    pub fn new(roi: [i32; 4], rows: Vec<&'a mut [u8]>) -> Dst<'a> {
        Dst { rows: Rows::List(rows), x0: roi[0], y0: roi[1], x1: roi[2], y1: roi[3] }
    }

    /// a destination in one buffer: row y0 + k at k * stride (e.g. a tile buffer with stride
    /// (x1 - x0) * 4)
    pub fn strided(roi: [i32; 4], buf: &'a mut [u8], stride: usize) -> Dst<'a> {
        Dst { rows: Rows::Strided(buf, stride), x0: roi[0], y0: roi[1], x1: roi[2], y1: roi[3] }
    }

    /// the rectangle of a surface (clamped to it) as one destination
    pub fn of_surface(sf: &'a mut Surface, roi: [i32; 4]) -> Dst<'a> {
        let (w, h) = (sf.w, sf.h);
        let x0 = roi[0].clamp(0, w);
        let y0 = roi[1].clamp(0, h);
        let (x1, y1) = (roi[2].clamp(x0, w), roi[3].clamp(y0, h));
        let stride = sf.stride as usize;
        if x1 <= x0 || y1 <= y0 {
            return Dst { rows: Rows::List(Vec::new()), x0, y0, x1, y1 };
        }
        let start = y0 as usize * stride + x0 as usize * 4;
        let end = (y1 - 1) as usize * stride + x1 as usize * 4;
        Dst { rows: Rows::Strided(&mut sf.px[start..end], stride), x0, y0, x1, y1 }
    }

    /// the pixels x0..x1 of row y (y0 <= y < y1)
    #[inline]
    pub fn row(&mut self, y: i32) -> &mut [u8] {
        let k = (y - self.y0) as usize;
        match &mut self.rows {
            Rows::List(v) => v[k],
            Rows::Table(t) => t[k],
            Rows::Strided(b, st) => {
                let n = (self.x1 - self.x0) as usize * 4;
                &mut b[k * *st..k * *st + n]
            }
        }
    }
}

/// disjoint rectangles of a surface as destinations (clamped to the surface; empty ones are
/// empty destinations); panics when two rectangles overlap.  `store` holds the row slices of
/// all destinations (one allocation, cleared first; pass a new or reused empty Vec):
///     let mut store = Vec::new();
///     let dsts = fx::tiles(&mut surface, &rects, &mut store);
pub fn tiles<'a>(sf: &'a mut Surface, rects: &[[i32; 4]], store: &'a mut Vec<&'a mut [u8]>) -> Vec<Dst<'a>> {
    let (w, h) = (sf.w, sf.h);
    let stride = sf.stride as usize;
    let cl: Vec<[i32; 4]> = rects
        .iter()
        .map(|r| {
            let x0 = r[0].clamp(0, w);
            let y0 = r[1].clamp(0, h);
            [x0, y0, r[2].clamp(x0, w), r[3].clamp(y0, h)]
        })
        .collect();
    // an empty rectangle has no rows
    let nrows = |r: &[i32; 4]| if r[2] > r[0] && r[3] > r[1] { (r[3] - r[1]) as usize } else { 0 };
    // the rows of rectangle i at store[base[i]..]
    let mut base = Vec::with_capacity(cl.len() + 1);
    let mut total = 0usize;
    for r in &cl {
        base.push(total);
        total += nrows(r);
    }
    base.push(total);
    store.clear();
    store.resize_with(total, Default::default);
    // the non-empty rectangles by first row; the active ones of a row are kept sorted by x
    let mut order: Vec<usize> = (0..cl.len()).filter(|&i| nrows(&cl[i]) > 0).collect();
    if !order.is_empty() && stride != 0 {
        order.sort_by_key(|&i| (cl[i][1], cl[i][0]));
        let ymin = cl[order[0]][1];
        let ymax = order.iter().map(|&i| cl[i][3]).max().unwrap_or(0);
        let mut active: Vec<usize> = Vec::new();
        let mut next = 0usize;
        let lines = sf.px.chunks_mut(stride).take(ymax as usize).skip(ymin as usize);
        for (k, line) in lines.enumerate() {
            let y = ymin + k as i32;
            active.retain(|&i| y < cl[i][3]);
            while next < order.len() && cl[order[next]][1] == y {
                let i = order[next];
                let at = active.partition_point(|&j| cl[j][0] < cl[i][0]);
                active.insert(at, i);
                next += 1;
            }
            let (mut rest, _) = line.split_at_mut(w as usize * 4);
            let mut at = 0i32; // column of rest[0]
            for &i in &active {
                let (x0, x1) = (cl[i][0], cl[i][2]);
                assert!(x0 >= at, "fx::tiles: overlapping rectangles");
                let tmp = core::mem::take(&mut rest);
                let (_, tail) = tmp.split_at_mut((x0 - at) as usize * 4);
                let (mine, tail) = tail.split_at_mut((x1 - x0) as usize * 4);
                store[base[i] + (y - cl[i][1]) as usize] = mine;
                rest = tail;
                at = x1;
            }
        }
    }
    let mut rest: &'a mut [&'a mut [u8]] = &mut store[..];
    let mut out: Vec<Dst<'a>> = Vec::with_capacity(cl.len());
    for (i, r) in cl.iter().enumerate() {
        let (mine, tail) = core::mem::take(&mut rest).split_at_mut(base[i + 1] - base[i]);
        rest = tail;
        out.push(Dst { rows: Rows::Table(mine), x0: r[0], y0: r[1], x1: r[2], y1: r[3] });
    }
    out
}

/// fx_rows: f(dst_row, src_row, x0, y) for every row of the ROI (rows = the pixels x0..x1);
/// polls the cancel flag before every row  -> 0 | ERR_CANCEL
#[inline]
pub fn rows(a: &FxArgs, dst: &mut Dst, mut f: impl FnMut(&mut [u8], &[u8], i32, i32)) -> i32 {
    if a.x1 - a.x0 <= 0 {
        return 0;
    }
    let (b0, b1) = (a.x0 as usize * 4, a.x1 as usize * 4);
    for y in a.y0..a.y1 {
        if a.cancelled() {
            return ERR_CANCEL;
        }
        let s = &a.src_row(y)[b0..b1];
        f(dst.row(y), s, a.x0, y);
    }
    0
}

// ---------------------------------------------------------------- registry (fx_common.s)

static REG: SpinLock<Vec<&'static [FxDesc]>> = SpinLock::new(Vec::new());

/// add a group of effects; a group already registered is ignored -> 0 | ERR_LIMIT
pub fn register(group: &'static [FxDesc]) -> i32 {
    let mut r = REG.lock();
    if r.iter().any(|g| core::ptr::eq(*g, group)) {
        return 0;
    }
    if r.len() >= FXG_MAX {
        return ERR_LIMIT;
    }
    r.push(group);
    0
}

/// register every built-in group in menu order (adjustments first)  [fx_use_all]
pub fn use_all() {
    for g in [
        &adj::FXTAB as &'static [FxDesc],
        &artistic::FXTAB,
        &blur::FXTAB,
        &color::FXTAB,
        &distort::FXTAB,
        &noise::FXTAB,
        &object::FXTAB,
        &photo::FXTAB,
        &render::FXTAB,
        &stylize::FXTAB,
        &enhance::FXTAB,
    ] {
        register(g);
    }
}

/// the number of registered effects
pub fn count() -> usize {
    REG.lock().iter().map(|g| g.len()).sum()
}

/// the i-th registered effect (group order, then table order)
pub fn get(i: usize) -> Option<&'static FxDesc> {
    let r = REG.lock();
    let mut i = i;
    for g in r.iter() {
        if i < g.len() {
            return Some(&g[i]);
        }
        i -= g.len();
    }
    None
}

/// the effect with this ASCII key
pub fn find(name: &str) -> Option<&'static FxDesc> {
    (0..).map_while(get).find(|d| d.name == name)
}

/// the effect with this FX_* id
pub fn find_id(id: u32) -> Option<&'static FxDesc> {
    (0..).map_while(get).find(|d| d.id == id)
}

/// every property's default into its slot(s); slots without a property are 0
pub fn defaults(desc: &FxDesc, p: &mut Params) {
    p.0 = [0; FX_NSLOTS];
    for pr in desc.props {
        let s = pr.slot as usize;
        if s >= FX_NSLOTS {
            continue;
        }
        match pr.typ {
            FP_DEC | FP_ANGLE => p.0[s] = pr.def.to_bits(),
            FP_PAN => {
                p.0[s] = pr.def.to_bits();
                if s + 1 < FX_NSLOTS {
                    p.0[s + 1] = pr.def_y.to_bits();
                }
            }
            FP_COLOR => p.0[s] = pr.def.to_bits() & 0xFFFF_FFFF,
            _ => p.0[s] = rt::fx::cvttd64(pr.def) as u64,
        }
    }
}

/// deterministic per-pixel random bits: a murmur3-style finaliser over the inputs  [fx_hash32]
#[inline]
pub fn hash32(x: u32, y: u32, seed: u32, stream: u32) -> u32 {
    let mut h = x.wrapping_mul(0x8DA6B343)
        ^ y.wrapping_mul(0xD8163841)
        ^ seed.wrapping_mul(0xCB1AB31F)
        ^ stream.wrapping_mul(0x165667B1);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85EBCA6B);
    h ^= h >> 13;
    h = h.wrapping_mul(0xC2B2AE35);
    h ^ (h >> 16)
}
