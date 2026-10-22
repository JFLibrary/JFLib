// cmd -- per-layer draw command lists, recorded during a UI pass and replayed into the
// framebuffer afterwards (popups drawn late still end up on top, passes that are thrown
// away cost no pixels)                                                 [src/ui/ui_cmd.s]
//
// Drawing calls for apps (current layer, physical pixels):
//   draw_color(argb)                     colour for the next draw_* calls
//   draw_rect(x, y, w, h)                filled rectangle
//   draw_rrect(x, y, w, h, r)            filled rounded rectangle
//   draw_frame(x, y, w, h, r, t)         rounded border of thickness t
//   draw_text(x, y, s, font_size)        text with baseline y; font_size = FONT_* | size_fx << 8
//   draw_icon(cp, x, y, size_fx)         icon, em box top-left at x, y
//   icon_hook(f)                         an app draws chosen icons itself (render time)
//   draw_circle(cx, cy, r, t)            24.8 fixed; t = 0 fills
//   draw_line(x0, y0, x1, y1, width)     24.8 fixed, round caps
//   draw_image(sf, x, y, w, h)           surface scaled to w x h (nearest) or 1:1
//   draw_clip(x, y, w, h) / draw_unclip()
// Internal: cmd_reset, cmd_layer(l) -> previous, cmd_rrect, cmd_text, cmd_custom,
// cmd_shadow, render_all, render_popup(dx, dy).
//
// Every record keeps the byte size the asm gives it (cmd_alloc rounds to 8): a layer holds
// at most its capacity (CAPS_MAIN / CAPS_SMALL), further commands are dropped as in the asm.
// The dirty regions (dirty.rs) hash every record's content (cmd_hash) and its box (cmd_bbox).

use alloc::rc::Rc;
use alloc::vec::Vec;

use super::consts::*;
use super::Ui;
use gfx::shapes::Shapes;
use gfx::{Dc, Surface};
use text::Text;

/// layer capacities in bytes of the main window / of secondary windows  [cmd_caps_*]
pub(crate) const CAPS_MAIN: [u32; L_COUNT] = [1024 * 1024, 256 * 1024, 256 * 1024, 256 * 1024];
#[allow(dead_code)] // ui-win: secondary windows
pub(crate) const CAPS_SMALL: [u32; L_COUNT] = [256 * 1024, 64 * 1024, 128 * 1024, 16 * 1024];

/// the gfx target of a custom draw call (render time): the draw context (clip, origin,
/// colour), the surface and the text engine (its path and rasterizer serve Shapes)
pub struct Gfx<'a> {
    pub dc: &'a mut Dc,
    pub sf: &'a mut Surface,
    pub text: &'a mut Text,
}

impl Gfx<'_> {
    /// the anti-aliased shape calls (fill_rrect, circles, lines) on this target
    pub fn shapes(&mut self) -> Shapes<'_> {
        Shapes { dc: &mut *self.dc, sf: &mut *self.sf, path: &mut self.text.path, ras: &mut self.text.ras }
    }
}

/// a custom draw function (cmd_custom: canvases, custom tracks): draw(g, rect) at render
/// time with the clip set to rect (absolute px).  Its identity (the Rc) and the version of
/// the command decide whether it is redrawn.
pub trait Draw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]);
}

pub type DrawRef = Rc<dyn Draw>;

/// icon_hook: f(g, cp, x, y, size_fx, argb) -> true when it drew icon cp itself (x, y =
/// target pixels with the origin applied), false = the icon font draws it
pub type IconHook = Rc<dyn Fn(&mut Gfx, u32, i32, i32, i32, u32) -> bool>;

/// one recorded draw command  [C_*]
#[derive(Clone)]
pub enum Cmd {
    Rect { x: i32, y: i32, w: i32, h: i32, c: u32 },
    /// fill (alpha 0 = none), border t px in `stroke`, its bottom band in `stroke2`; r and t
    /// are kept as u16
    RRect { x: i32, y: i32, w: i32, h: i32, r: u16, t: u16, fill: u32, stroke: u32, stroke2: u32 },
    /// Fractional rounded panel; geometry is 24.8 pixels and composited once.
    PanelFx { x: i32, y: i32, w: i32, h: i32, r: i32, t: i32, fill: u32, stroke: u32 },
    /// text with baseline y; maxw != 0 clips it to x .. x + maxw
    Text { x: i32, y: i32, font: u16, size: i32, c: u32, maxw: i32, s: Vec<u8> },
    /// icon, em box top-left at x, y
    Icon { x: i32, y: i32, cp: u32, size: i32, c: u32 },
    Image { x: i32, y: i32, w: i32, h: i32, sf: Rc<Surface>, ver: u32 },
    Clip { x: i32, y: i32, w: i32, h: i32 },
    Unclip,
    /// 24.8; t = 0 fills
    Circle { x: i32, y: i32, r: i32, t: i32, c: u32 },
    /// 24.8, round caps
    Line { x0: i32, y0: i32, x1: i32, y1: i32, w: i32, c: u32 },
    Custom { x: i32, y: i32, w: i32, h: i32, f: Option<DrawRef>, user: usize, ver: u32 },
    Shadow { x: i32, y: i32, w: i32, h: i32, r: i32, depth: i32, c: u32 },
}

impl Cmd {
    /// the record size of the asm (cmd_alloc: rounded up to 8 bytes)
    fn size(&self) -> u32 {
        let n = match self {
            Cmd::Rect { .. } | Cmd::Icon { .. } | Cmd::Circle { .. } | Cmd::Clip { .. } => 24,
            Cmd::RRect { .. } | Cmd::Image { .. } => 40,
            Cmd::PanelFx { .. } => 40,
            Cmd::Text { s, .. } => 28 + s.len() as u32,
            Cmd::Unclip => 8,
            Cmd::Line { .. } | Cmd::Shadow { .. } => 32,
            Cmd::Custom { .. } => 48,
        };
        (n + 7) & !7
    }
}

/// the per-window part of the command lists (ui_cmd_wb0 .. ui_cmd_wb1)
pub(crate) struct W {
    pub(crate) layers: [Vec<Cmd>; L_COUNT],
    /// bytes used / capacity per layer (cmd_init_caps)
    pub(crate) len: [u32; L_COUNT],
    pub(crate) cap: [u32; L_COUNT],
    pub(crate) layer: usize,
    pub(crate) dcol: u32,
    pub(crate) overflow: bool,
}

impl W {
    /// cmd_init_caps(caps): empty lists with these capacities
    pub(crate) fn new(caps: &[u32; L_COUNT]) -> W {
        W {
            layers: Default::default(),
            len: [0; L_COUNT],
            cap: *caps,
            layer: 0,
            dcol: 0xFF00_0000,
            overflow: false,
        }
    }
}

// FNV-1a 64 over little-endian fields (hash_fnv64)
struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xCBF2_9CE4_8422_2325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn i32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
}

/// hash of a record's content in the asm's record layout (header: type, flags, size,
/// colour; then the fields; text bytes; pointers by identity) -- any visible change of a
/// command changes it
pub(crate) fn cmd_hash(c: &Cmd) -> u64 {
    let mut h = Fnv::new();
    let size = c.size();
    let head = |h: &mut Fnv, t: u8, argb: u32| {
        h.bytes(&[t, 0]);
        h.bytes(&(size as u16).to_le_bytes());
        h.u32(argb);
    };
    match c {
        Cmd::Rect { x, y, w, h: hh, c } => {
            head(&mut h, 1, *c);
            for v in [*x, *y, *w, *hh] {
                h.i32(v);
            }
        }
        Cmd::RRect { x, y, w, h: hh, r, t, fill, stroke, stroke2 } => {
            head(&mut h, 2, *fill);
            for v in [*x, *y, *w, *hh] {
                h.i32(v);
            }
            h.bytes(&r.to_le_bytes());
            h.bytes(&t.to_le_bytes());
            h.u32(*stroke);
            h.u32(*stroke2);
        }
        Cmd::PanelFx { x, y, w, h: hh, r, t, fill, stroke } => {
            head(&mut h, 12, *fill);
            for v in [*x, *y, *w, *hh, *r, *t] { h.i32(v); }
            h.u32(*stroke);
        }
        Cmd::Text { x, y, font, size: sz, c, maxw, s } => {
            head(&mut h, 3, *c);
            h.i32(*x);
            h.i32(*y);
            h.bytes(&font.to_le_bytes());
            h.bytes(&(s.len() as u16).to_le_bytes());
            h.i32(*sz);
            h.i32(*maxw);
            h.bytes(s);
        }
        Cmd::Icon { x, y, cp, size: sz, c } => {
            head(&mut h, 4, *c);
            h.i32(*x);
            h.i32(*y);
            h.u32(*cp);
            h.i32(*sz);
        }
        Cmd::Image { x, y, w, h: hh, sf, ver } => {
            head(&mut h, 5, 0);
            for v in [*x, *y, *w, *hh] {
                h.i32(v);
            }
            h.u64(Rc::as_ptr(sf) as usize as u64);
            h.u32(*ver);
        }
        Cmd::Clip { x, y, w, h: hh } => {
            head(&mut h, 6, 0);
            for v in [*x, *y, *w, *hh] {
                h.i32(v);
            }
        }
        Cmd::Unclip => head(&mut h, 7, 0),
        Cmd::Circle { x, y, r, t, c } => {
            head(&mut h, 8, *c);
            for v in [*x, *y, *r, *t] {
                h.i32(v);
            }
        }
        Cmd::Line { x0, y0, x1, y1, w, c } => {
            head(&mut h, 9, *c);
            for v in [*x0, *y0, *x1, *y1, *w] {
                h.i32(v);
            }
        }
        Cmd::Custom { x, y, w, h: hh, f, user, ver } => {
            head(&mut h, 10, 0);
            for v in [*x, *y, *w, *hh] {
                h.i32(v);
            }
            h.u64(f.as_ref().map_or(0, |f| Rc::as_ptr(f) as *const () as usize as u64));
            h.u64(*user as u64);
            h.u32(*ver);
        }
        Cmd::Shadow { x, y, w, h: hh, r, depth, c } => {
            head(&mut h, 11, *c);
            for v in [*x, *y, *w, *hh, *r, *depth] {
                h.i32(v);
            }
        }
    }
    h.0
}

/// cmd_bbox -> the command's box x0, y0, x1, y1 (px, exclusive), None when it has no area
/// (clip records)
pub(crate) fn cmd_bbox(c: &Cmd) -> Option<[i32; 4]> {
    Some(match *c {
        Cmd::PanelFx { x, y, w, h, .. } => [x >> 8, y >> 8,
            ((x as i64 + w as i64 + 255) >> 8) as i32,
            ((y as i64 + h as i64 + 255) >> 8) as i32],
        Cmd::Text { x, y, size, maxw, ref s, .. } => {
            // x, baseline, size: a generous box
            let px = ((size as u32) >> 8).wrapping_add(1) as i32;
            let x0 = x.wrapping_sub(2);
            let tw = if maxw != 0 { maxw } else { (s.len() as i32).wrapping_mul(px) };
            let half = ((px as u32) >> 1) as i32;
            [x0, y.wrapping_sub(half.wrapping_add(px)), x0.wrapping_add(tw).wrapping_add(4), y.wrapping_add(half).wrapping_add(1)]
        }
        Cmd::Icon { x, y, size, .. } => {
            let s = ((size as u32) >> 8).wrapping_add(2) as i32; // size px + margin
            let (x0, y0) = (x.wrapping_sub(1), y.wrapping_sub(1));
            [x0, y0, x0.wrapping_add(s), y0.wrapping_add(s)]
        }
        Cmd::Circle { x, y, r, t, .. } => {
            // centre and radius in 24.8, ring width
            let d = (r.wrapping_add(t) >> 8).wrapping_add(2);
            let (cx, cy) = (x >> 8, y >> 8);
            [cx.wrapping_sub(d), cy.wrapping_sub(d), cx.wrapping_add(d).wrapping_add(1), cy.wrapping_add(d).wrapping_add(1)]
        }
        Cmd::Line { x0, y0, x1, y1, w, .. } => {
            let d = (w >> 8).wrapping_add(2); // half width + margin (generous)
            [
                (x0.min(x1) >> 8).wrapping_sub(d),
                (y0.min(y1) >> 8).wrapping_sub(d),
                (x0.max(x1) >> 8).wrapping_add(d).wrapping_add(1),
                (y0.max(y1) >> 8).wrapping_add(d).wrapping_add(1),
            ]
        }
        Cmd::Shadow { x, y, w, h, depth: d, .. } => {
            // the shadow reaches `depth` around the box and depth/3 further down
            [x.wrapping_sub(d), y.wrapping_sub(d), x.wrapping_add(w).wrapping_add(d), y.wrapping_add(h).wrapping_add(d).wrapping_add(d)]
        }
        Cmd::Clip { .. } | Cmd::Unclip => return None,
        Cmd::Rect { x, y, w, h, .. }
        | Cmd::RRect { x, y, w, h, .. }
        | Cmd::Image { x, y, w, h, .. }
        | Cmd::Custom { x, y, w, h, .. } => [x, y, x.wrapping_add(w), y.wrapping_add(h)],
    })
}

/// cmd_visible: the command's box touches the current gfx clip (no box: always)
fn cmd_visible(c: &Cmd, dc: &Dc) -> bool {
    let Some(b) = cmd_bbox(c) else { return true };
    let k = dc.clip;
    !(b[0] >= k[2] || b[2] <= k[0] || b[1] >= k[3] || b[3] <= k[1])
}

/// cmd_render_layer: replay one layer into the current gfx target
pub(crate) fn render_layer(list: &[Cmd], dc: &mut Dc, sf: &mut Surface, text: &mut Text, hook: &Option<IconHook>,
                          icon_style: (&dyn crate::Style, u32, bool, u32, u32)) {
    let mut depth = 0; // clip depth opened by this layer
    for c in list {
        if !matches!(c, Cmd::Clip { .. } | Cmd::Unclip) && !cmd_visible(c, dc) {
            continue;
        }
        match c {
            &Cmd::PanelFx { x, y, w, h, r, t, fill, stroke } => {
                let mut sh = Shapes { dc: &mut *dc, sf: &mut *sf, path: &mut text.path, ras: &mut text.ras };
                sh.panel_fx(x, y, w, h, r, t, fill, stroke);
            }
            &Cmd::Rect { x, y, w, h, c } => {
                dc.set_color(c);
                dc.fill(sf, x, y, w, h);
            }
            &Cmd::RRect { x, y, w, h, r, t, fill, stroke, stroke2 } => {
                let (r, t) = (r as i32, t as i32);
                let mut sh = Shapes { dc: &mut *dc, sf: &mut *sf, path: &mut text.path, ras: &mut text.ras };
                if fill & 0xFF00_0000 != 0 {
                    sh.dc.set_color(fill);
                    sh.fill_rrect(x, y, w, h, r);
                }
                if t == 0 {
                    continue;
                }
                if stroke & 0xFF00_0000 != 0 {
                    sh.dc.set_color(stroke);
                    sh.stroke_rrect(x, y, w, h, r, t);
                }
                // different bottom edge colour: overdraw the lower corner band of the border
                if stroke2 != stroke && stroke2 & 0xFF00_0000 != 0 {
                    sh.dc.set_color(stroke2);
                    let band = r.max(t);
                    sh.dc.clip_push(x, y.wrapping_add(h).wrapping_sub(band), w, band);
                    sh.stroke_rrect(x, y, w, h, r, t);
                    sh.dc.clip_pop();
                }
            }
            Cmd::Text { x, y, font, size, c, maxw, s } => {
                dc.set_color(*c);
                text.set_font(*font as u32, *size);
                if *maxw != 0 {
                    dc.clip_push(*x, -100000, *maxw, 200000);
                }
                text.draw_n(dc, sf, *x, *y, s, s.len());
                if *maxw != 0 {
                    dc.clip_pop();
                }
            }
            &Cmd::Icon { x, y, cp, size, c } => {
                if let Some(f) = hook {
                    // the app draws this icon itself?
                    let (tx, ty) = (x.wrapping_add(dc.ox), y.wrapping_add(dc.oy));
                    let mut g = Gfx { dc: &mut *dc, sf: &mut *sf, text: &mut *text };
                    if f(&mut g, cp, tx, ty, size, c) {
                        continue;
                    }
                }
                let (style, accent, dark, disabled, dpi) = icon_style;
                let (tx, ty) = (x.wrapping_add(dc.ox), y.wrapping_add(dc.oy));
                let mut g = Gfx { dc: &mut *dc, sf: &mut *sf, text: &mut *text };
                if style.paint_icon(&mut g, cp, tx, ty, size, c, accent, dark, c == disabled, dpi) {
                    continue;
                }
                dc.set_color(c);
                text.icon_draw(dc, sf, cp, x, y, size);
            }
            Cmd::Image { x, y, w, h, sf: src, .. } => {
                dc.blit_scaled(sf, src, *x, *y, *w, *h);
            }
            &Cmd::Clip { x, y, w, h } => {
                dc.clip_push(x, y, w, h);
                depth += 1;
            }
            Cmd::Unclip => {
                if depth > 0 {
                    dc.clip_pop();
                    depth -= 1;
                }
            }
            &Cmd::Circle { x, y, r, t, c } => {
                dc.set_color(c);
                let mut sh = Shapes { dc: &mut *dc, sf: &mut *sf, path: &mut text.path, ras: &mut text.ras };
                if t != 0 {
                    sh.stroke_circle_fx(x, y, r, t);
                } else {
                    sh.fill_circle_fx(x, y, r);
                }
            }
            &Cmd::Line { x0, y0, x1, y1, w, c } => {
                dc.set_color(c);
                let mut sh = Shapes { dc: &mut *dc, sf: &mut *sf, path: &mut text.path, ras: &mut text.ras };
                sh.line_fx(x0, y0, x1, y1, w);
            }
            Cmd::Custom { x, y, w, h, f, .. } => {
                // clip to the widget, call f(g, rect)
                dc.clip_push(*x, *y, *w, *h);
                if let Some(f) = f {
                    let mut g = Gfx { dc: &mut *dc, sf: &mut *sf, text: &mut *text };
                    f.draw(&mut g, [*x, *y, *w, *h]);
                }
                dc.clip_pop();
            }
            &Cmd::Shadow { x, y, w, h, r, depth: d, c } => {
                // soft shadow: `depth` expanding rounded rectangles with fading alpha,
                // offset downwards by depth / 3
                if d <= 0 {
                    continue;
                }
                let base = c >> 24;
                let down = (d as u32 / 3) as i32;
                let mut i = d;
                while i != 0 {
                    // alpha_i = base * (depth - i + 1) / (depth * 2)
                    let a = base.wrapping_mul((d - i + 1) as u32) / ((d + d) as u32);
                    let mut sh = Shapes { dc: &mut *dc, sf: &mut *sf, path: &mut text.path, ras: &mut text.ras };
                    sh.dc.set_color((c & 0xFF_FFFF) | a << 24);
                    sh.fill_rrect(x - i, y - i + down, w + i * 2, h + i * 2, r + i);
                    i -= 1;
                }
            }
        }
    }
    while depth > 0 {
        dc.clip_pop();
        depth -= 1;
    }
}

// cmd_translate(layer, dx, dy): move every command of a layer by (dx, dy) px (circles and
// lines in 24.8)
#[allow(dead_code)] // ui-win: OS popup windows (U3)
fn translate(list: &mut [Cmd], dx: i32, dy: i32) {
    for c in list.iter_mut() {
        match c {
            Cmd::Unclip => {}
            Cmd::Circle { x, y, .. } | Cmd::PanelFx { x, y, .. } => {
                *x = x.wrapping_add(dx << 8);
                *y = y.wrapping_add(dy << 8);
            }
            Cmd::Line { x0, y0, x1, y1, .. } => {
                *x1 = x1.wrapping_add(dx << 8);
                *y1 = y1.wrapping_add(dy << 8);
                *x0 = x0.wrapping_add(dx << 8);
                *y0 = y0.wrapping_add(dy << 8);
            }
            Cmd::Rect { x, y, .. }
            | Cmd::RRect { x, y, .. }
            | Cmd::Text { x, y, .. }
            | Cmd::Icon { x, y, .. }
            | Cmd::Image { x, y, .. }
            | Cmd::Clip { x, y, .. }
            | Cmd::Custom { x, y, .. }
            | Cmd::Shadow { x, y, .. } => {
                *x = x.wrapping_add(dx);
                *y = y.wrapping_add(dy);
            }
        }
    }
}

impl Ui {
    /// cmd_reset: empty every layer, current layer = base
    pub(crate) fn cmd_reset(&mut self) {
        let w = &mut self.wp.cmd;
        for l in w.layers.iter_mut() {
            l.clear();
        }
        w.len = [0; L_COUNT];
        w.layer = 0;
        w.dcol = 0xFF00_0000;
        self.wp.rtl.reset();
    }

    /// cmd_layer(l) -> previous layer
    pub fn cmd_layer(&mut self, l: u32) -> u32 {
        let prev = self.wp.cmd.layer as u32;
        self.wp.cmd.layer = (l & 3) as usize;
        prev
    }

    // cmd_alloc: append a record to the current layer (dropped when the layer is full)
    fn push(&mut self, c: Cmd) {
        let w = &mut self.wp.cmd;
        let l = w.layer;
        let n = w.len[l] + c.size();
        if n > w.cap[l] {
            w.overflow = true;
            return;
        }
        w.len[l] = n;
        w.layers[l].push(c);
    }

    /// ui_draw_color(argb): colour for the following draw_* calls
    pub fn draw_color(&mut self, argb: u32) {
        self.wp.cmd.dcol = argb;
    }

    /// the colour of the following draw_* calls (g_dcol)
    pub fn draw_color_get(&self) -> u32 {
        self.wp.cmd.dcol
    }

    /// ui_draw_rect(x, y, w, h)
    pub fn draw_rect(&mut self, x: i32, y: i32, w: i32, h: i32) {
        let c = self.wp.cmd.dcol;
        self.push(Cmd::Rect { x, y, w, h, c });
    }

    /// ui_draw_rrect(x, y, w, h, r): filled rounded rectangle in the draw colour
    pub fn draw_rrect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32) {
        let c = self.wp.cmd.dcol;
        self.cmd_rrect(x, y, w, h, r, 0, c, 0, 0);
    }

    /// ui_draw_frame(x, y, w, h, r, t): border only, in the draw colour
    pub fn draw_frame(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, t: i32) {
        let c = self.wp.cmd.dcol;
        self.cmd_rrect(x, y, w, h, r, t, 0, c, c);
    }

    /// cmd_rrect(x, y, w, h, r, t, fill, stroke, stroke2): rounded rectangle with an
    /// optional fill and border (stroke2: the bottom edge)
    #[allow(clippy::too_many_arguments)]
    pub fn cmd_rrect(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, t: i32, fill: u32, stroke: u32, stroke2: u32) {
        let r = if self.look.rad_mode != 0 { self.look_radius(r, w, h) } else { r }; // (theme)
        self.push(Cmd::RRect { x, y, w, h, r: r as u16, t: t as u16, fill, stroke, stroke2 });
    }

    /// Rounded fill/border with 24.8 pixel geometry and a single coverage composite.
    /// Fractional bounds participate in dirty-region hashing and clipping.
    #[allow(clippy::too_many_arguments)]
    pub fn cmd_panel_fx(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, t: i32, fill: u32, stroke: u32) {
        self.push(Cmd::PanelFx { x, y, w, h, r, t, fill, stroke });
    }

    /// ui_draw_text(x, y, s, font_size): font_size = FONT_* | size_fx << 8
    pub fn draw_text(&mut self, x: i32, y: i32, s: &str, font_size: u32) {
        let c = self.wp.cmd.dcol;
        self.cmd_text(x, y, s.as_bytes(), font_size & 255, (font_size >> 8) as i32, c, 0);
    }

    /// cmd_text(x, y, s, font, size_fx, colour, maxw): at most 4000 bytes, never cut inside
    /// a UTF-8 sequence
    #[allow(clippy::too_many_arguments)]
    pub fn cmd_text(&mut self, x: i32, y: i32, s: &[u8], font: u32, size: i32, c: u32, maxw: i32) {
        let mut n = s.len();
        if n > 4000 {
            n = 4000;
            // do not cut inside a UTF-8 sequence: back off over continuation bytes
            while n > 0 && s[n] & 0xC0 == 0x80 {
                n -= 1;
            }
        }
        self.push(Cmd::Text { x, y, font: font as u16, size, c, maxw, s: s[..n].to_vec() });
    }

    /// emit_text(x, baseline, s, colour): text in the current text font (styles)
    pub fn emit_text(&mut self, x: i32, y: i32, s: &str, c: u32) {
        let (font, size) = self.text.get_font();
        let b = s.as_bytes();
        let n = b.iter().position(|&x| x == 0).unwrap_or(b.len());
        self.cmd_text(x, y, &b[..n], font, size, c, 0);
    }

    /// ui_draw_icon(cp, x, y, size_fx)
    pub fn draw_icon(&mut self, cp: u32, x: i32, y: i32, size: i32) {
        let c = self.wp.cmd.dcol;
        self.push(Cmd::Icon { x, y, cp, size, c });
    }

    /// ui_draw_circle(cx, cy, r, t): 24.8 fixed, t = 0 fills
    pub fn draw_circle(&mut self, x: i32, y: i32, r: i32, t: i32) {
        let c = self.wp.cmd.dcol;
        self.push(Cmd::Circle { x, y, r, t, c });
    }

    /// ui_draw_line(x0, y0, x1, y1, width): 24.8 fixed
    pub fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, w: i32) {
        let c = self.wp.cmd.dcol;
        self.push(Cmd::Line { x0, y0, x1, y1, w, c });
    }

    /// ui_draw_image(sf, x, y, w, h): w / h 0 = natural size; `ver` stands for the
    /// surface's SF_AUX version (bump it when the pixels change)
    pub fn draw_image(&mut self, sf: &Rc<Surface>, ver: u32, x: i32, y: i32, w: i32, h: i32) {
        let w = if w == 0 { sf.w } else { w };
        let h = if h == 0 { sf.h } else { h };
        self.push(Cmd::Image { x, y, w, h, sf: sf.clone(), ver });
    }

    /// ui_draw_clip(x, y, w, h)
    pub fn draw_clip(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.push(Cmd::Clip { x, y, w, h });
    }

    /// ui_draw_unclip
    pub fn draw_unclip(&mut self) {
        self.push(Cmd::Unclip);
    }

    /// ui_icon_hook(f): an app draws chosen icons itself (None removes the hook)
    pub fn icon_hook(&mut self, f: Option<IconHook>) {
        self.icon_hook = f;
    }

    /// cmd_custom(x, y, w, h, f, user, version): f.draw(g, rect) at render time, clipped
    #[allow(clippy::too_many_arguments)]
    pub fn cmd_custom(&mut self, x: i32, y: i32, w: i32, h: i32, f: Option<DrawRef>, user: usize, ver: u32) {
        self.push(Cmd::Custom { x, y, w, h, f, user, ver });
    }

    /// cmd_shadow(x, y, w, h, r, depth): soft shadow in the draw colour
    pub fn cmd_shadow(&mut self, x: i32, y: i32, w: i32, h: i32, r: i32, depth: i32) {
        let r = if self.look.rad_mode != 0 { self.look_radius(r, w, h) } else { r }; // (theme)
        let c = self.wp.cmd.dcol;
        self.push(Cmd::Shadow { x, y, w, h, r, depth, c });
    }

    /// cmd_render_all: replay every layer of the live window into the current gfx target
    /// (sf = the target the dc began)
    pub(crate) fn render_all_to(&mut self, shadow: bool) {
        let icon_style = (self.style, self.color(T_ACCENT), self.theme_dark, self.color(T_TEXT_DISABLED), self.wp.rt.dpi as u32);
        let Ui { wp, dc, text, icon_hook, .. } = self;
        let sf = if shadow { &mut wp.rt.shadow } else { &mut wp.rt.fb };
        for l in wp.cmd.layers.iter() {
            render_layer(l, dc, sf, text, icon_hook, icon_style);
        }
    }

    /// cmd_render_popup(dx, dy): (U3) replay the popup and tooltip layers into `sf` moved
    /// by (dx, dy) px (an OS popup window shows a part of its owner's popups)
    #[allow(dead_code)] // ui-win: OS popup windows (U3)
    pub(crate) fn render_popup(&mut self, sf: &mut Surface, dx: i32, dy: i32) {
        let icon_style = (self.style, self.color(T_ACCENT), self.theme_dark, self.color(T_TEXT_DISABLED), self.wp.rt.dpi as u32);
        let Ui { wp, dc, text, icon_hook, .. } = self;
        let layers = &mut wp.cmd.layers;
        translate(&mut layers[L_POPUP as usize], dx, dy);
        translate(&mut layers[L_TIP as usize], dx, dy);
        render_layer(&layers[L_POPUP as usize], dc, sf, text, icon_hook, icon_style);
        render_layer(&layers[L_TIP as usize], dc, sf, text, icon_hook, icon_style);
        translate(&mut layers[L_POPUP as usize], -dx, -dy);
        translate(&mut layers[L_TIP as usize], -dx, -dy);
    }
}
