// gfx -- surfaces and the draw context (clip, origin, colour), fills, blits  [src/gfx/surface.s]
//
//   Surface::new(w, h, fmt)            zeroed pixels, stride 64-byte aligned (sides 0..65536)
//   Dc::new(w, h) / dc.begin(&sf) / dc.end()      the draw context of a target; begin saves
//                                      clip, origin and opacity and resets them for another
//                                      target, end restores them (gfx_begin / gfx_end)
//   dc: set_color(argb), set_opacity(op), clip_push(x, y, w, h) / clip_pop(), origin(x, y),
//       translate(dx, dy), fill / clear / hline / vline / frame / blit / blit_scaled (sf, ...)
//   path (Path, 24.8 and float API), raster (Raster), stroke (Path::stroke), shapes (Shapes),
//   blend (row kernels), straight (straight-alpha rows), blendmode (paint.net blend modes),
//   tmask (tiled masks), flood (flood fill), gradient, pathdata (SVG path data), dash
//
// Colours are straight 0xAARRGGBB; Dc keeps the premultiplied form for the kernels.  The
// surface is passed to every drawing call: Dc holds no pointer to it.

#![no_std]
#![forbid(unsafe_code)]

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

impl Surface {
    /// zeroed pixels, stride 64-byte aligned; sides outside 0..65536 give an empty surface
    /// (surf_alloc fails). Byte lengths beyond the target's addressable allocation size
    /// also give an empty surface, before allocating pixels.
    pub fn new(w: i32, h: i32, fmt: u8) -> Surface {
        if !(0..=SURF_MAX).contains(&w) || !(0..=SURF_MAX).contains(&h) {
            return Surface { px: Vec::new(), w: 0, h: 0, stride: 0, fmt, flags: 0, opac: 255, blend: 0 };
        }
        let row = if fmt == FMT_A8 { w } else { w * 4 };
        let stride = (row + 63) & !63;
        let Some(bytes) = (stride as usize).checked_mul(h as usize).filter(|&n| n <= isize::MAX as usize) else {
            return Surface { px: Vec::new(), w: 0, h: 0, stride: 0, fmt, flags: 0, opac: 255, blend: 0 };
        };
        let bytes = bytes.max(64);
        Surface { px: vec![0u8; bytes], w, h, stride, fmt, flags: SFF_OWNED, opac: 255, blend: 0 }
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
        };
        dc.set_color(0xFF00_0000);
        dc
    }

    /// draw into another target: clip, origin and opacity of the current one are saved; the
    /// clip becomes the whole new target, the origin 0  [gfx_begin]
    pub fn begin(&mut self, sf: &Surface) {
        self.targets.push(Saved {
            w: self.w,
            h: self.h,
            clip: self.clip,
            csp: self.clip_stack.len(),
            ox: self.ox,
            oy: self.oy,
            opac: self.opac,
        });
        self.w = sf.w;
        self.h = sf.h;
        self.ox = 0;
        self.oy = 0;
        self.clip = [0, 0, sf.w, sf.h];
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
                let o = yy as usize * stride;
                sf.px[o + x0 as usize..o + x1 as usize].fill(a);
            }
            return;
        }
        if self.ia != 0 && self.pcol32 == 0 {
            return; // fully transparent
        }
        for yy in y0..y1 {
            let o = yy as usize * stride + x0 as usize * 4;
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
            let o = yy as usize * stride;
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

    /// draw a surface at x, y: premultiplied over (x opacity), XRGB sources are copied  [gfx_blit]
    pub fn blit(&self, sf: &mut Surface, src: &Surface, x: i32, y: i32) {
        if sf.fmt == FMT_A8 || src.fmt == FMT_A8 {
            return;
        }
        let Some([x0, y0, x1, y1]) = self.clip_rect(x, y, src.w, src.h) else { return };
        let sx = (x0 - x.wrapping_add(self.ox)) as usize;
        let sy = (y0 - y.wrapping_add(self.oy)) as usize;
        let n = (x1 - x0) as usize;
        for r in 0..(y1 - y0) as usize {
            let d = (y0 as usize + r) * sf.stride as usize + x0 as usize * 4;
            let s = (sy + r) * src.stride as usize + sx * 4;
            let dst = &mut sf.px[d..d + n * 4];
            let srow = &src.px[s..s + n * 4];
            if src.fmt == FMT_XRGB {
                dst.copy_from_slice(srow);
            } else {
                blend::blit_over(dst, srow, self.opac);
            }
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
            let d = (y0 + r) as usize * sf.stride as usize + x0 as usize * 4;
            let dst = &mut sf.px[d..d + vw * 4];
            if src.fmt == FMT_XRGB {
                dst.copy_from_slice(&row);
            } else {
                blend::blit_over(dst, &row, self.opac);
            }
        }
    }
}
