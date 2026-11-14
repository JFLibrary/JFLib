// gfx -- surfaces and the draw context (clip, origin, colour), fills, blits  [src/gfx/surface.s]
//
//   Surface::new(w, h, fmt)            zeroed pixels, stride 64-byte aligned (sides 0..65536)
//   Dc::new(w, h) / dc.begin(&sf) / dc.end()      the draw context of a target; begin saves
//                                      clip, origin and opacity and resets them for another
//                                      target, end restores them (gfx_begin / gfx_end)
//   dc.begin_rows(&sf, row0, h)        sf holds rows row0 .. row0 + sf.h of a target h rows
//                                      high (a row band drawn on a thread of its own): the
//                                      coordinates stay the target's, only the pixel rows move
//   dc: set_color(argb), set_opacity(op), clip_push(x, y, w, h) / clip_pop(), origin(x, y),
//       translate(dx, dy), fill / clear / hline / vline / frame / blit / blit_scaled (sf, ...)
//   set_parallel(f) / parallel()       the loop large copies run on (the UI runtime passes the
//                                      worker pool's): a blit that copies XRGB rows from
//                                      COPY_PAR_MIN bytes on is cut into row bands on it, from
//                                      COPY_STREAM_MIN bytes on past the caches (a 4K canvas)
//   path (Path, 24.8 and float API), raster (Raster), stroke (Path::stroke), shapes (Shapes),
//   blend (row kernels), straight (straight-alpha rows), blendmode (paint.net blend modes),
//   tmask (tiled masks), flood (flood fill), gradient, pathdata (SVG path data), dash
//
// Colours are straight 0xAARRGGBB; Dc keeps the premultiplied form for the kernels.  The
// surface is passed to every drawing call: Dc holds no pointer to it.

#![no_std]
#![forbid(unsafe_code)]

// in a module's build (--cfg jflib_module, docs/CRATES.md) the module runtime goes by the same name
#[cfg(jflib_module)]
extern crate modrt as rt;

extern crate alloc;

pub mod blend;
pub mod blendmode;
pub mod dash;
pub mod flood;
pub mod gradient;
pub mod path;
mod panel;
pub mod pathdata;
pub mod raster;
pub mod shapes;
pub mod straight;
pub mod stroke;
pub mod tmask;
pub mod xform;

use alloc::vec;
use alloc::vec::Vec;

pub use rt::fx::div255;

pub const FMT_XRGB: u8 = 0; // 0x??RRGGBB, alpha byte ignored (window framebuffer)
pub const FMT_PARGB: u8 = 1; // premultiplied 0xAARRGGBB (images, layers)
pub const FMT_A8: u8 = 2; // 8-bit coverage / mask
pub const FMT_ARGB: u8 = 3; // straight 0xAARRGGBB = bytes B, G, R, A
pub const SFF_OWNED: u8 = 1;
pub const SFF_OPAQUE: u8 = 2; // hint: every alpha is 255
pub const FX_ONE: i32 = 256; // 24.8 fixed point: 1 pixel
pub const RULE_NONZERO: u32 = 0;
pub const RULE_EVENODD: u32 = 1;
pub const JOIN_MITER: u32 = 0; // stroke style = join | cap << 4
pub const JOIN_ROUND: u32 = 1;
pub const JOIN_BEVEL: u32 = 2;
pub const CAP_BUTT: u32 = 0;
pub const CAP_ROUND: u32 = 1;
pub const CAP_SQUARE: u32 = 2;
pub const ERR_FAIL: i32 = -1;
pub const ERR_NOMEM: i32 = -8;
pub const ERR_LIMIT: i32 = -15;
const SURF_MAX: i32 = 65536; // largest surface side (px)
/// a blit's plain copy (XRGB source) from this many bytes on runs in row bands on the loop of
/// set_parallel (gfxtest --bench-blit: a 1 MB copy takes about a fifth on 12 workers)
pub const COPY_PAR_MIN: usize = 1 << 20;
/// ... and from this many on with streaming stores (gfx::blend::copy_stream): source and target
/// together no longer fit the caches, the target is not read first.  A 4K frame (32 MB): 1.9 ms on
/// 12 workers and the calling thread, 2.7 ms with plain stores, 5 ms on one thread (Ryzen 5 3600,
/// DDR4-3200, a loaded machine; the memory's limit for such a copy is about 1.7 ms there)
pub const COPY_STREAM_MIN: usize = 12 << 20;
// bytes of a band of such a copy (the pool hands the bands out as threads get free)
const COPY_BAND: usize = 256 * 1024;

/// runs f(i) for every i in 0..n, possibly on several threads (img_core::ParallelFor)
pub type ParallelFor = fn(usize, &(dyn Fn(usize) + Sync));

static PAR: rt::sync::SpinLock<Option<ParallelFor>> = rt::sync::SpinLock::new(None);

/// the loop large copies run on (Dc::blit, COPY_PAR_MIN); the UI runtime passes a wrapper of
/// plat::parallel_for -- gfx does not depend on plat.  Without it they run on the calling thread
/// (the same bytes)
pub fn set_parallel(f: ParallelFor) {
    *PAR.lock() = Some(f);
}

/// the loop set by set_parallel
pub fn parallel() -> Option<ParallelFor> {
    *PAR.lock()
}

/// rows rows of n bytes: from src (stride ss) at row sy, byte sx into dst (stride ds) at row dy,
/// byte dx.  From COPY_PAR_MIN bytes on in bands on the loop of set_parallel, from
/// COPY_STREAM_MIN on past the caches (fenced before it returns)
#[allow(clippy::too_many_arguments)]
fn copy_rows(dst: &mut [u8], ds: usize, dy: usize, dx: usize, src: &[u8], ss: usize, sy: usize, sx: usize, n: usize, rows: usize) {
    let bytes = rows.saturating_mul(n);
    let stream = bytes >= COPY_STREAM_MIN;
    let copy = |d: &mut [u8], s: &[u8]| {
        if stream {
            blend::copy_stream(d, s);
        } else {
            d.copy_from_slice(s);
        }
    };
    let par = if bytes >= COPY_PAR_MIN && rows >= 2 { parallel() } else { None };
    let Some(pf) = par else {
        for r in 0..rows {
            let (d, s) = ((dy + r) * ds + dx, (sy + r) * ss + sx);
            copy(&mut dst[d..d + n], &src[s..s + n]);
        }
        if stream {
            blend::stream_fence();
        }
        return;
    };
    // bands of bh rows: chunks of the target's rows from row dy on (each band locks its own)
    let bh = rows.div_ceil((bytes / COPY_BAND).clamp(2, rows));
    let chunks: Vec<rt::sync::SpinLock<&mut [u8]>> = dst[dy * ds..].chunks_mut(bh * ds).take(rows.div_ceil(bh)).map(rt::sync::SpinLock::new).collect();
    pf(chunks.len(), &|i| {
        let Some(cell) = chunks.get(i) else { return };
        let mut c = cell.lock();
        for r in i * bh..((i + 1) * bh).min(rows) {
            let (d, s) = ((r - i * bh) * ds + dx, (sy + r) * ss + sx);
            copy(&mut c[d..d + n], &src[s..s + n]);
        }
        if stream {
            blend::stream_fence(); // (this thread's stores, before the loop reports it done)
        }
    });
}

/// pixels of one image: rows of `stride` bytes (4 per pixel, A8: 1)
#[derive(Default)]
pub struct Surface {
    pub px: Vec<u8>,
    pub w: i32,
    pub h: i32,
    pub stride: i32,
    pub fmt: u8,
    pub flags: u8,
    pub opac: u8,  // layer opacity 0..255
    pub blend: u8, // layer blend mode
}

/// n zeroed bytes | None when the memory is not there (instead of an abort). A probe
/// reservation first, then vec![0; n]: that is alloc_zeroed, whose fresh pages need no
/// clearing pass.
pub fn try_zeroed(n: usize) -> Option<Vec<u8>> {
    let mut probe: Vec<u8> = Vec::new();
    probe.try_reserve_exact(n).ok()?;
    drop(probe);
    Some(vec![0u8; n])
}

impl Surface {
    /// zeroed pixels, stride 64-byte aligned; sides outside 0..65536 give an empty surface
    /// (surf_alloc fails). Byte lengths beyond the target's addressable allocation size
    /// also give an empty surface, before allocating pixels.
    pub fn new(w: i32, h: i32, fmt: u8) -> Surface {
        let Some((stride, bytes)) = Surface::layout(w, h, fmt) else {
            return Surface { px: Vec::new(), w: 0, h: 0, stride: 0, fmt, flags: 0, opac: 255, blend: 0 };
        };
        Surface { px: vec![0u8; bytes], w, h, stride, fmt, flags: SFF_OWNED, opac: 255, blend: 0 }
    }

    /// like `new`, but None (instead of an empty surface or an abort) when the size is
    /// invalid or the memory is not there (try_zeroed). For sizes that come from a file or
    /// from the user.
    pub fn try_new(w: i32, h: i32, fmt: u8) -> Option<Surface> {
        let (stride, bytes) = Surface::layout(w, h, fmt)?;
        Some(Surface { px: try_zeroed(bytes)?, w, h, stride, fmt, flags: SFF_OWNED, opac: 255, blend: 0 })
    }

    // (stride, pixel bytes) or None for sides outside 0..=SURF_MAX or a size beyond isize::MAX
    fn layout(w: i32, h: i32, fmt: u8) -> Option<(i32, usize)> {
        if !(0..=SURF_MAX).contains(&w) || !(0..=SURF_MAX).contains(&h) {
            return None;
        }
        let row = if fmt == FMT_A8 { w } else { w * 4 };
        let stride = (row + 63) & !63;
        let bytes = (stride as usize).checked_mul(h as usize).filter(|&n| n <= isize::MAX as usize)?;
        Some((stride, bytes.max(64)))
    }

    /// a tightly packed A8 surface (glyph masks: stride = w)
    pub fn new_a8_packed(w: i32, h: i32) -> Surface {
        Surface { px: vec![0u8; (w * h) as usize], w, h, stride: w, fmt: FMT_A8, flags: SFF_OWNED, opac: 255, blend: 0 }
    }

    /// new size, same format; the contents are NOT kept  [surf_resize]
    pub fn resize(&mut self, w: i32, h: i32) {
        *self = Surface::new(w, h, self.fmt);
    }

    /// bytes per pixel
    pub fn bpp(&self) -> usize {
        if self.fmt == FMT_A8 { 1 } else { 4 }
    }
}

// what begin saves of the previous target
#[derive(Clone, Copy)]
struct Saved {
    w: i32,
    h: i32,
    clip: [i32; 4],
    csp: usize,
    ox: i32,
    oy: i32,
    opac: u32,
    row0: i32,
}

/// the draw context of a target: clip (x0, y0, x1, y1 exclusive, in target pixels), origin,
/// colour (straight and premultiplied), opacity
pub struct Dc {
    pub clip: [i32; 4],
    clip_stack: Vec<[i32; 4]>,
    targets: Vec<Saved>,
    pub ox: i32,
    pub oy: i32,
    pub w: i32,
    pub h: i32,
    pub color: u32,
    pub opac: u32,
    pub pcol: [u32; 4], // premultiplied B, G, R, A
    pub pcol32: u32,
    pub ia: u32, // 255 - premultiplied alpha
    /// the target row in the surface's first row (begin_rows: a band of a taller target);
    /// 0 otherwise.  The fills and blits here, the rasterizer (paths, shapes, panels) and the
    /// text engine's glyph blits honour it; other row writers (tmask, gradient, flood) expect 0.
    pub row0: i32,
}

impl Dc {
    /// a context for a w x h target: clip = the whole target, colour opaque black  [gfx_init]
    pub fn new(w: i32, h: i32) -> Dc {
        let mut dc = Dc {
            clip: [0, 0, w, h],
            clip_stack: Vec::new(),
            targets: Vec::new(),
            ox: 0,
            oy: 0,
            w,
            h,
            color: 0,
            opac: 255,
            pcol: [0; 4],
            pcol32: 0,
            ia: 255,
            row0: 0,
        };
        dc.set_color(0xFF00_0000);
        dc
    }

    /// draw into another target: clip, origin and opacity of the current one are saved; the
    /// clip becomes the whole new target, the origin 0  [gfx_begin]
    pub fn begin(&mut self, sf: &Surface) {
        self.begin_rows(sf, 0, sf.h);
    }

    /// begin for a band: sf holds rows row0 .. row0 + sf.h of a target h rows high.  Drawing
    /// keeps the target's coordinates (clip, origin, paths, glyph positions), so a band gets
    /// the bytes the whole target would get there; the clip starts as the band's rows.
    pub fn begin_rows(&mut self, sf: &Surface, row0: i32, h: i32) {
        self.targets.push(Saved {
            w: self.w,
            h: self.h,
            clip: self.clip,
            csp: self.clip_stack.len(),
            ox: self.ox,
            oy: self.oy,
            opac: self.opac,
            row0: self.row0,
        });
        self.w = sf.w;
        self.h = h;
        self.ox = 0;
        self.oy = 0;
        self.row0 = row0;
        let y1 = row0.saturating_add(sf.h).min(h);
        self.clip = [0, row0.clamp(0, y1.max(0)), sf.w, y1.max(0)];
    }

    /// back to the previous target with its clip, origin and opacity  [gfx_end]
    pub fn end(&mut self) {
        let Some(s) = self.targets.pop() else { return };
        self.w = s.w;
        self.h = s.h;
        self.clip = s.clip;
        self.clip_stack.truncate(s.csp);
        self.ox = s.ox;
        self.oy = s.oy;
        self.row0 = s.row0;
        self.clamp_clip();
        self.set_opacity(s.opac);
    }

    /// store the colour and its premultiplied forms  [gfx_set_color]
    pub fn set_color(&mut self, argb: u32) {
        self.color = argb;
        let a = div255((argb >> 24) * self.opac);
        self.ia = 255 - a;
        self.pcol[3] = a;
        let mut packed = a << 24;
        for ch in 0..3 {
            let c = (argb >> (ch * 8)) & 255;
            let p = div255(c * a);
            self.pcol[ch] = p;
            packed |= p << (ch * 8);
        }
        self.pcol32 = packed;
    }

    pub fn get_color(&self) -> u32 {
        self.color
    }

    /// global opacity 0..255 (multiplies every alpha)
    pub fn set_opacity(&mut self, op: u32) {
        self.opac = op & 255;
        let c = self.color;
        self.set_color(c);
    }

    pub fn origin(&mut self, x: i32, y: i32) {
        self.ox = x;
        self.oy = y;
    }

    pub fn translate(&mut self, dx: i32, dy: i32) {
        self.ox = self.ox.wrapping_add(dx);
        self.oy = self.oy.wrapping_add(dy);
    }

    /// save the clip and intersect it with the rectangle (origin-relative)
    pub fn clip_push(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.clip_stack.push(self.clip);
        let x = x.wrapping_add(self.ox);
        let y = y.wrapping_add(self.oy);
        let x0 = x.max(self.clip[0]);
        let y0 = y.max(self.clip[1]);
        let x1 = x.wrapping_add(w).min(self.clip[2]).max(x0); // an empty clip stays empty
        let y1 = y.wrapping_add(h).min(self.clip[3]).max(y0);
        self.clip = [x0, y0, x1, y1];
    }

    pub fn clip_pop(&mut self) {
        if let Some(c) = self.clip_stack.pop() {
            self.clip = c;
            self.clamp_clip();
        }
    }

    /// 0 <= x0 <= x1 <= W, 0 <= y0 <= y1 <= H (a restored clip may belong to a larger target)
    fn clamp_clip(&mut self) {
        let x0 = self.clip[0].max(0).min(self.w);
        let x1 = self.clip[2].min(self.w).max(x0);
        let y0 = self.clip[1].max(0).min(self.h);
        let y1 = self.clip[3].min(self.h).max(y0);
        self.clip = [x0, y0, x1, y1];
    }

    /// origin-relative rectangle -> clipped target rectangle (x0, y0, x1, y1), or None
    pub fn clip_rect(&self, x: i32, y: i32, w: i32, h: i32) -> Option<[i32; 4]> {
        let x = x.wrapping_add(self.ox);
        let y = y.wrapping_add(self.oy);
        let x0 = x.max(self.clip[0]);
        let y0 = y.max(self.clip[1]);
        let x1 = x.wrapping_add(w).min(self.clip[2]);
        let y1 = y.wrapping_add(h).min(self.clip[3]);
        if x1 <= x0 || y1 <= y0 { None } else { Some([x0, y0, x1, y1]) }
    }

    /// blend the colour over a rectangle (A8 targets: store the alpha)  [gfx_fill]
    pub fn fill(&self, sf: &mut Surface, x: i32, y: i32, w: i32, h: i32) {
        let Some([x0, y0, x1, y1]) = self.clip_rect(x, y, w, h) else { return };
        let stride = sf.stride as usize;
        if sf.fmt == FMT_A8 {
            let a = (self.pcol32 >> 24) as u8;
            for yy in y0..y1 {
                let o = (yy - self.row0) as usize * stride;
                sf.px[o + x0 as usize..o + x1 as usize].fill(a);
            }
            return;
        }
        if self.ia != 0 && self.pcol32 == 0 {
            return; // fully transparent
        }
        for yy in y0..y1 {
            let o = (yy - self.row0) as usize * stride + x0 as usize * 4;
            let row = &mut sf.px[o..o + (x1 - x0) as usize * 4];
            if self.ia == 0 {
                blend::fill_solid(row, self.pcol32);
            } else {
                blend::fill_over(row, &self.pcol, self.ia);
            }
        }
    }

    /// replace every pixel inside the clip by argb (premultiplied, no blending)  [gfx_clear]
    pub fn clear(&self, sf: &mut Surface, argb: u32) {
        let [x0, y0, x1, y1] = self.clip;
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let a = argb >> 24;
        let mut c = argb;
        if a != 255 {
            c = argb & 0xFF00_0000;
            for ch in 0..3 {
                c |= div255(((argb >> (ch * 8)) & 255) * a) << (ch * 8);
            }
        }
        let stride = sf.stride as usize;
        for yy in y0..y1 {
            let o = (yy - self.row0) as usize * stride;
            if sf.fmt == FMT_A8 {
                sf.px[o + x0 as usize..o + x1 as usize].fill((c >> 24) as u8);
            } else {
                blend::fill_solid(&mut sf.px[o + x0 as usize * 4..o + x1 as usize * 4], c);
            }
        }
    }

    pub fn hline(&self, sf: &mut Surface, x: i32, y: i32, w: i32) {
        self.fill(sf, x, y, w, 1);
    }

    pub fn vline(&self, sf: &mut Surface, x: i32, y: i32, h: i32) {
        self.fill(sf, x, y, 1, h);
    }

    /// 1-pixel outline inside the rectangle  [gfx_frame]
    pub fn frame(&self, sf: &mut Surface, x: i32, y: i32, w: i32, h: i32) {
        if w <= 0 || h <= 0 {
            return;
        }
        self.hline(sf, x, y, w);
        if h == 1 {
            return;
        }
        self.hline(sf, x, y + h - 1, w);
        if h == 2 {
            return;
        }
        self.vline(sf, x, y + 1, h - 2);
        if w == 1 {
            return;
        }
        self.vline(sf, x + w - 1, y + 1, h - 2);
    }

    /// draw a surface at x, y: premultiplied over (x opacity), XRGB sources are copied (a large
    /// copy in row bands on the loop of set_parallel, COPY_PAR_MIN)  [gfx_blit]
    pub fn blit(&self, sf: &mut Surface, src: &Surface, x: i32, y: i32) {
        if sf.fmt == FMT_A8 || src.fmt == FMT_A8 {
            return;
        }
        let Some([x0, y0, x1, y1]) = self.clip_rect(x, y, src.w, src.h) else { return };
        let sx = (x0 - x.wrapping_add(self.ox)) as usize;
        let sy = (y0 - y.wrapping_add(self.oy)) as usize;
        let n = (x1 - x0) as usize;
        let (ds, ss) = (sf.stride as usize, src.stride as usize);
        if src.fmt == FMT_XRGB {
            copy_rows(&mut sf.px, ds, (y0 - self.row0) as usize, x0 as usize * 4, &src.px, ss, sy, sx * 4, n * 4, (y1 - y0) as usize);
            return;
        }
        for r in 0..(y1 - y0) as usize {
            let d = ((y0 - self.row0) as usize + r) * ds + x0 as usize * 4;
            let s = (sy + r) * ss + sx * 4;
            blend::blit_over(&mut sf.px[d..d + n * 4], &src.px[s..s + n * 4], self.opac);
        }
    }

    /// nearest-neighbour blit scaled to w x h (over)  [gfx_blit_scaled]
    pub fn blit_scaled(&self, sf: &mut Surface, src: &Surface, x: i32, y: i32, w: i32, h: i32) {
        if w == src.w && h == src.h {
            self.blit(sf, src, x, y);
            return;
        }
        if w <= 0 || h <= 0 || src.w <= 0 || src.h <= 0 || sf.fmt == FMT_A8 || src.fmt == FMT_A8 {
            return;
        }
        let xa = x.wrapping_add(self.ox);
        let ya = y.wrapping_add(self.oy);
        let Some([x0, y0, x1, y1]) = self.clip_rect(x, y, w, h) else { return };
        let vw = (x1 - x0) as usize;
        let xstep = ((src.w as u64) << 16) / w as u64;
        let ystep = ((src.h as u64) << 16) / h as u64;
        let mut row = vec![0u8; vw * 4];
        for r in 0..(y1 - y0) {
            let sy = (((y0 + r - ya) as u32 as u64 * ystep + (ystep >> 1)) >> 16) as usize;
            let srow = sy * src.stride as usize;
            let mut fx = (x0 - xa) as u32 as u64 * xstep + (xstep >> 1);
            for i in 0..vw {
                let s = srow + (fx >> 16) as usize * 4;
                row[i * 4..i * 4 + 4].copy_from_slice(&src.px[s..s + 4]);
                fx += xstep;
            }
            let d = (y0 + r - self.row0) as usize * sf.stride as usize + x0 as usize * 4;
            let dst = &mut sf.px[d..d + vw * 4];
            if src.fmt == FMT_XRGB {
                dst.copy_from_slice(&row);
            } else {
                blend::blit_over(dst, &row, self.opac);
            }
        }
    }
}
