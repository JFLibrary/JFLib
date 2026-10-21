// shapes -- Kritzel shape tool ("Formen"): gallery, live editing, commit  [apps/paint/src/shapes.s]
//
//   shape_input(p, ui, in) -> true     one frame of canvas input (UiInput) for TOOL_SHAPE
//   shape_overlay(p, ui)               live preview, dashed box and handles (cmd_custom);
//                                      commits the shape when another tool was chosen
//   shape_ribbon(p, ui)                gallery (3 x 8 shapes), outline / fill style drop-downs
//   shape_cmd(p, ui, id, arg) -> false commands of the tool (none yet)
//   shape_pre_cmd(p, ui, id) -> bool   called by pt_cmd before any command: commits the
//                                      shape being edited (Delete discards it: true)
//   shape_cursor(p, ui) -> bits        canvas cursor bits while the shape tool is active
//
// State for other modules: kind (g_shape_kind, SH_* chosen in the gallery).
//
// A new shape is dragged out on the canvas (Shift: square / circle / 45 degree line).
// It stays editable -- not yet in the layer -- until the user clicks outside it,
// presses Enter, picks another tool or runs a command: dragging inside moves it,
// the handles resize it (line / curve: end and control points).  Esc / Delete
// discard it.  Curve: after the line, two more drags bend it (cubic control
// points).  Polygon: drag the first side, click further corners, finish with a
// double click, a click on the first corner or Enter.
// Outline: colour 1 with width size, fill: colour 2 (right button: swapped);
// colours, size, opacity and the styles stay live until the commit.
//
// Geometry (doc 24.8): box shapes keep two corner pixels P0 / P1 (inclusive);
// line / curve points are pixel centres (P + 0.5); the finished polygon keeps
// its corners in 1/65536 of its box (so it scales with the handles).
//
// The preview is drawn at render time from a snapshot (`Ren`) taken when the overlay is
// recorded: the shape, its styles, the colours and the view of that pass.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use gfx::path::Path;
use gfx::raster::Raster;
use gfx::{Dc, Surface, CAP_ROUND, JOIN_MITER, JOIN_ROUND, RULE_NONZERO};
use rt::fx::{cvt, maxss, minss};
use ui::*;

use crate::consts::*;
use crate::doc::{doc_layer, doc_mark};
use crate::history::{hist_begin, hist_end};
use crate::shapes_geom::{sg_close, sg_cubic, sg_line, sg_move, sg_shape, sg_xf, IconDraw, Sg};
use crate::tools::tool_set;
use crate::view::view_to_doc;
use crate::Kritzel;

const S_IDLE: u32 = 0;
const S_CREATE: u32 = 1; // dragging out a new shape
const S_ACTIVE: u32 = 2; // finished, editable (handles)
const S_MOVE: u32 = 3; // dragging the whole shape
const S_SIZE: u32 = 4; // dragging handle hnd
const S_BEND1W: u32 = 5; // curve: waiting for the first bend
const S_BEND1: u32 = 6;
const S_BEND2W: u32 = 7; // curve: waiting for the second bend
const S_BEND2: u32 = 8;
const S_POLYD: u32 = 9; // polygon: dragging the last corner
const S_POLYW: u32 = 10; // polygon: waiting for the next corner
const MAX_POLY: usize = 256;
const GAL_COLS: i32 = 8;
const ID_SH_OUTLINE: u32 = ID_SHAPE0 + 30; // outline drop-down (+2 none, +3 solid)
const ID_SH_FILL: u32 = ID_SHAPE0 + 40; // fill drop-down (+2 none, +3 solid)
// p: P0, P1, C1, C2 (x, y doc 24.8)
const P0X: usize = 0;
const P0Y: usize = 1;
const P1X: usize = 2;
const P1Y: usize = 3;
const C1X: usize = 4;
const C1Y: usize = 5;
const C2X: usize = 6;
const C2Y: usize = 7;
const FNV_PRIME: u32 = 16777619;
const HANDLE_COL: u32 = 0xFF50_5050;
const DASH_COL: u32 = 0xFF40_4040;

/// the shape tool's state (the .data / .bss of shapes.s)
pub(crate) struct Shape {
    /// SH_* chosen in the gallery (g_shape_kind)
    pub(crate) kind: i32,
    pub(crate) outline: i32,
    pub(crate) fill: i32,
    // everything the preview depends on (g_sh_state .. g_sh_state_end)
    p: [i32; 8],
    orig: [i32; 8],
    st: u32,
    kact: i32,
    btn: u32,
    swap: u32,
    hnd: u32,
    npts: usize,
    anchor: [i32; 2],
    pts: [[i32; 2]; MAX_POLY],
    // the rest
    can: [i32; 4],
    lastp: [i32; 2],
    tol: i32,
    draw: Rc<ShDraw>,
    icons: Vec<Rc<IconDraw>>,
}

impl Default for Shape {
    fn default() -> Shape {
        Shape {
            kind: SH_RECT,
            outline: 1,
            fill: 0,
            p: [0; 8],
            orig: [0; 8],
            st: S_IDLE,
            kact: 0,
            btn: 0,
            swap: 0,
            hnd: 0,
            npts: 0,
            anchor: [0; 2],
            pts: [[0; 2]; MAX_POLY],
            can: [0; 4],
            lastp: [0; 2],
            tol: 0,
            draw: Rc::new(ShDraw::default()),
            icons: Vec::new(),
        }
    }
}

// ------------------------------------------------------------------ helpers

// is_box(kind): shapes defined by their bounding box
fn is_box(kind: i32) -> bool {
    !(kind == SH_LINE || kind == SH_CURVE || kind == SH_POLYGON)
}

#[inline]
fn iabs(v: i32) -> i32 {
    v.wrapping_abs()
}

// near_pt(ax, ay, bx, by, tol): both axes differ by at most tol px
fn near_pt(ax: i32, ay: i32, bx: i32, by: i32, tol: i32) -> bool {
    iabs(ax.wrapping_sub(bx)) <= tol && iabs(ay.wrapping_sub(by)) <= tol
}

// constrain(ax, ay, bx, by, mode) -> x, y -- Shift: mode 0 square (equal |dx|, |dy|),
// mode 1 snap to 0 / 45 / 90 degrees
fn constrain(ax: i32, ay: i32, bx: i32, by: i32, mode: i32) -> (i32, i32) {
    let dx = bx.wrapping_sub(ax);
    let dy = by.wrapping_sub(ay);
    let adx = iabs(dx);
    let ady = iabs(dy);
    if mode != 0 {
        if ady.wrapping_mul(1000) < adx.wrapping_mul(414) {
            return (bx, ay); // horizontal
        }
        if adx.wrapping_mul(1000) < ady.wrapping_mul(414) {
            return (ax, by); // vertical
        }
    }
    let m = if adx < ady { ady } else { adx };
    let x = ax.wrapping_add(if dx < 0 { m.wrapping_neg() } else { m });
    let y = ay.wrapping_add(if dy < 0 { m.wrapping_neg() } else { m });
    (x, y)
}

// handle positions (0..2 halves of the box) and their pointer shapes
const HND_HX: [i32; 8] = [0, 1, 2, 0, 2, 0, 1, 2];
const HND_HY: [i32; 8] = [0, 0, 0, 1, 1, 2, 2, 2];
const HND_CUR: [u32; 8] =
    [CUR_SIZE_NWSE, CUR_SIZE_NS, CUR_SIZE_NESW, CUR_SIZE_WE, CUR_SIZE_WE, CUR_SIZE_NESW, CUR_SIZE_NS, CUR_SIZE_NWSE];

// Bernstein weights (1-t)^3, 3(1-t)^2 t, 3(1-t) t^2, t^3 for t = i / 16
#[rustfmt::skip]
static BEZ16: [[f32; 4]; 17] = [
    [1.0, 0.0, 0.0, 0.0],
    [0.823974609, 0.164794922, 0.010986328, 0.000244141],
    [0.669921875, 0.287109375, 0.041015625, 0.001953125],
    [0.536376953, 0.372509766, 0.086242676, 0.006591797],
    [0.421875, 0.421875, 0.140625, 0.015625],
    [0.324951172, 0.442382813, 0.200683594, 0.030517578],
    [0.244140625, 0.439453125, 0.263671875, 0.052734375],
    [0.177978516, 0.418945313, 0.328857422, 0.086242676],
    [0.125, 0.375, 0.375, 0.125],
    [0.086242676, 0.328857422, 0.418945313, 0.177978516],
    [0.052734375, 0.263671875, 0.439453125, 0.244140625],
    [0.030517578, 0.200683594, 0.442382813, 0.324951172],
    [0.015625, 0.140625, 0.421875, 0.421875],
    [0.006591797, 0.086242676, 0.372509766, 0.536376953],
    [0.001953125, 0.041015625, 0.287109375, 0.669921875],
    [0.000244141, 0.010986328, 0.164794922, 0.823974609],
    [0.0, 0.0, 0.0, 1.0],
];

static SH_NAMES: [&str; SH_COUNT as usize] = [
    "Linie",
    "Kurve",
    "Oval",
    "Rechteck",
    "Abgerundetes Rechteck",
    "Polygon",
    "Dreieck",
    "Rechtwinkliges Dreieck",
    "Raute",
    "Fünfeck",
    "Sechseck",
    "Pfeil nach rechts",
    "Pfeil nach links",
    "Pfeil nach oben",
    "Pfeil nach unten",
    "Stern mit vier Zacken",
    "Stern mit fünf Zacken",
    "Stern mit sechs Zacken",
    "Rechteckige Sprechblase",
    "Ovale Sprechblase",
    "Denkblase",
    "Herz",
    "Blitz",
];

// seg_near(ax, ay, bx, by, m) -> m (float x, y) is within sqrt(tol2) of the segment
// (window px)
fn seg_near(ax: i32, ay: i32, bx: i32, by: i32, mx: f32, my: f32, tol2: f32) -> bool {
    let x0 = ax as f32;
    let y0 = ay as f32;
    let abx = bx as f32 - x0;
    let aby = by as f32 - y0;
    let mut amx = mx - x0;
    let mut amy = my - y0;
    // t = clamp(am . ab / |ab|^2, 0, 1)
    let dot = abx * amx + aby * amy;
    let len2 = abx * abx + aby * aby;
    let mut t = 0.0f32;
    if len2 > 0.0 {
        t = dot / len2;
        t = maxss(t, 0.0);
        t = minss(t, 1.0);
    }
    // d = am - t * ab
    amx -= abx * t;
    amy -= aby * t;
    let d2 = amx * amx + amy * amy;
    !(d2 > tol2)
}

// ------------------------------------------------------------------ render snapshot

/// what sh_render, sh_frame and sh_handles read: the shape, its styles, the colours and the
/// view (a copy: the preview is drawn from it at render time)
#[derive(Clone, Default)]
struct Ren {
    p: [i32; 8],
    st: u32,
    kact: i32,
    swap: u32,
    pts: Vec<[i32; 2]>,
    outline: i32,
    fill: i32,
    col1: u32,
    col2: u32,
    size: i32,
    opacity: i32,
    zoom: i32,
    cx: i32,
    cy: i32,
    hair: i32,
    dp4: i32,
    dp7: i32,
}

impl Ren {
    // d2s(x, y) -> sx, sy -- doc 24.8 -> window px
    fn d2s(&self, x: i32, y: i32) -> (i32, i32) {
        let z = self.zoom as i64;
        let sx = ((x as i64).wrapping_mul(z).wrapping_add(128000) / 256000) as i32;
        let sy = ((y as i64).wrapping_mul(z).wrapping_add(128000) / 256000) as i32;
        (sx.wrapping_add(self.cx), sy.wrapping_add(self.cy))
    }

    // sh_opac(argb): alpha scaled by the opacity slider
    fn opac(&self, argb: u32) -> u32 {
        let a = (argb >> 24).wrapping_mul(self.opacity as u32) / 100;
        a << 24 | (argb & 0x00FF_FFFF)
    }

    // .sr_span(a, c): two corner pixels (24.8) -> lo, hi of the box, inset by half the
    // outline width when there is an outline
    fn span(&self, a: i32, c: i32) -> (i32, i32) {
        let (mut lo, c) = if a > c { (c, a) } else { (a, c) };
        let mut hi = c.wrapping_add(256);
        if self.outline != 0 {
            let half = self.size << 7; // half width (doc 24.8)
            lo = lo.wrapping_add(half);
            hi = hi.wrapping_sub(half);
            if lo > hi {
                lo = lo.wrapping_add(hi) >> 1; // too small: collapse to the middle
                hi = lo;
            }
        }
        (lo, hi)
    }

    // sh_render(preview): paint the shape into the target; preview maps doc coordinates
    // to the window with the view transform
    fn render(&self, preview: bool, path: &mut Path, ras: &mut Raster, dc: &mut Dc, sf: &mut Surface) {
        // colours (right button: swapped), opacity
        let (c1, c2) = if self.swap != 0 { (self.col2, self.col1) } else { (self.col1, self.col2) };
        let outc = self.opac(c1);
        let fillc = self.opac(c2);
        // transform and stroke width
        let mut width = self.size << 8;
        let mut sg = Sg::default();
        if preview {
            width = width.wrapping_mul(self.zoom) / 1000;
            sg_xf(&mut sg, self.zoom, 1000, self.cx << 8, self.cy << 8);
        }
        let sg = &sg;
        path.begin(dc);
        let p = &self.p;
        let open;
        if self.st == S_POLYD || self.st == S_POLYW {
            // polygon under construction: open polyline through the corner centres
            for (i, q) in self.pts.iter().enumerate() {
                let (x, y) = (q[0] + 128, q[1] + 128);
                if i == 0 { sg_move(sg, path, x, y) } else { sg_line(sg, path, x, y) }
            }
            open = true;
        } else if self.kact == SH_LINE {
            sg_move(sg, path, p[P0X] + 128, p[P0Y] + 128);
            sg_line(sg, path, p[P1X] + 128, p[P1Y] + 128);
            open = true;
        } else if self.kact == SH_CURVE {
            sg_move(sg, path, p[P0X] + 128, p[P0Y] + 128);
            let (c1x, c1y, c2x, c2y) = (p[C1X] + 128, p[C1Y] + 128, p[C2X] + 128, p[C2Y] + 128);
            sg_cubic(sg, path, c1x, c1y, c2x, c2y, p[P1X] + 128, p[P1Y] + 128);
            open = true;
        } else if self.kact == SH_POLYGON {
            // finished polygon: unit corners on the box of the corner pixel centres
            let ox = p[P0X] + 128;
            let oy = p[P0Y] + 128;
            let w = (p[P1X] + 128).wrapping_sub(ox); // may be negative: mirrored
            let h = (p[P1Y] + 128).wrapping_sub(oy);
            for (i, q) in self.pts.iter().enumerate() {
                let x = (((q[0] as i64).wrapping_mul(w as i64) >> 16) as i32).wrapping_add(ox);
                let y = (((q[1] as i64).wrapping_mul(h as i64) >> 16) as i32).wrapping_add(oy);
                if i == 0 { sg_move(sg, path, x, y) } else { sg_line(sg, path, x, y) }
            }
            sg_close(path);
            open = false;
        } else {
            // box shape: [min, max + 1 px), inset by half the outline
            let (l, r) = self.span(p[P0X], p[P1X]);
            let (t, b) = self.span(p[P0Y], p[P1Y]);
            sg_shape(sg, path, self.kact, l, t, r, b);
            open = false;
        }
        if open {
            dc.set_color(outc);
            path.stroke(ras, dc, sf, width, JOIN_ROUND | CAP_ROUND << 4);
            return;
        }
        if self.fill != 0 {
            dc.set_color(fillc);
            path.fill(ras, dc, sf, RULE_NONZERO);
        }
        if self.outline != 0 {
            dc.set_color(outc);
            path.stroke(ras, dc, sf, width, JOIN_MITER);
        }
    }

    // sh_frame() -> window px lines of the dashed box (x0, y0, x1, y1), just outside the
    // shape's box
    fn frame(&self) -> [i32; 4] {
        let p = &self.p;
        let (mut x0, mut x1) = if p[P0X] > p[P1X] { (p[P1X], p[P0X]) } else { (p[P0X], p[P1X]) };
        x1 = x1.wrapping_add(256);
        let (mut y0, mut y1) = if p[P0Y] > p[P1Y] { (p[P1Y], p[P0Y]) } else { (p[P0Y], p[P1Y]) };
        y1 = y1.wrapping_add(256);
        // polygon: its corners are the outline's centre line
        if self.kact == SH_POLYGON && self.outline != 0 {
            let a = self.size << 7;
            x0 = x0.wrapping_sub(a);
            y0 = y0.wrapping_sub(a);
            x1 = x1.wrapping_add(a);
            y1 = y1.wrapping_add(a);
        }
        let (ax, ay) = self.d2s(x0, y0);
        let (bx, by) = self.d2s(x1, y1);
        [ax - 1, ay - 1, bx, by]
    }

    // sh_handles() -> handle centres (window px): box shapes / polygon: 8 on the dashed
    // box (TL, T, TR, L, R, BL, B, BR); line: P0, P1; curve: P0, P1, C1, C2
    fn handles(&self) -> ([[i32; 2]; 8], usize) {
        let mut out = [[0; 2]; 8];
        let n = match self.kact {
            SH_LINE => 2,
            SH_CURVE => 4,
            _ => {
                let fr = self.frame();
                for i in 0..8 {
                    out[i][0] = (fr[2].wrapping_sub(fr[0]).wrapping_mul(HND_HX[i]) >> 1).wrapping_add(fr[0]);
                    out[i][1] = (fr[3].wrapping_sub(fr[1]).wrapping_mul(HND_HY[i]) >> 1).wrapping_add(fr[1]);
                }
                return (out, 8);
            }
        };
        for (i, o) in out.iter_mut().enumerate().take(n) {
            let (x, y) = self.d2s(self.p[i * 2] + 128, self.p[i * 2 + 1] + 128);
            *o = [x, y];
        }
        (out, n)
    }

    // sh_hit(mx, my, tol) -> handle index or -1
    fn hit(&self, mx: i32, my: i32, tol: i32) -> i32 {
        let (h, n) = self.handles();
        for (i, q) in h.iter().enumerate().take(n) {
            if near_pt(mx, my, q[0], q[1], tol) {
                return i as i32;
            }
        }
        -1
    }

    // sh_inside(mx, my, tol): a press there grabs the whole shape
    fn inside(&self, mx: i32, my: i32, tol: i32) -> bool {
        if self.kact != SH_LINE && self.kact != SH_CURVE {
            let fr = self.frame();
            return mx >= fr[0] && mx <= fr[2] && my >= fr[1] && my <= fr[3];
        }
        // distance to the (flattened) curve <= tolerance + half the stroke
        let fx = mx as f32;
        let fy = my as f32;
        let r = (self.size.wrapping_mul(self.zoom) / 2000).wrapping_add(tol) as f32;
        let tol2 = r * r;
        // control points P0, C1, C2, P1 (a line: C1 = P0, C2 = P1)
        let p = &self.p;
        let (b, c) = if self.kact == SH_LINE { (P0X, P1X) } else { (C1X, C2X) };
        let q = [
            self.d2s(p[P0X] + 128, p[P0Y] + 128),
            self.d2s(p[b] + 128, p[b + 1] + 128),
            self.d2s(p[c] + 128, p[c + 1] + 128),
            self.d2s(p[P1X] + 128, p[P1Y] + 128),
        ];
        let mut prev = q[0];
        // 16 segments with Bernstein weights from BEZ16
        for w in BEZ16.iter().skip(1) {
            let x = cvt(q[0].0 as f32 * w[0] + q[1].0 as f32 * w[1] + q[2].0 as f32 * w[2] + q[3].0 as f32 * w[3]);
            let y = cvt(q[0].1 as f32 * w[0] + q[1].1 as f32 * w[1] + q[2].1 as f32 * w[2] + q[3].1 as f32 * w[3]);
            if seg_near(prev.0, prev.1, x, y, fx, fy, tol2) {
                return true;
            }
            prev = (x, y);
        }
        false
    }
}

// dash(x, y, len, vert) -- dashed line one hair thick (white with dark dashes)
fn dash(dc: &mut Dc, sf: &mut Surface, x: i32, y: i32, len: i32, vert: bool, hair: i32, d: i32) {
    if len <= 0 {
        return;
    }
    let piece = |dc: &mut Dc, sf: &mut Surface, off: i32, l: i32| {
        if vert {
            dc.fill(sf, x, y + off, hair, l);
        } else {
            dc.fill(sf, x + off, y, l, hair);
        }
    };
    dc.set_color(0xFFFF_FFFF);
    piece(dc, sf, 0, len);
    dc.set_color(DASH_COL);
    let mut off = 0;
    while off < len {
        let l = (len - off).min(d);
        piece(dc, sf, off, l);
        off += d * 2;
    }
}

/// sh_draw_cb: the cmd_custom callback of the preview -- the shape, its dashed box,
/// handles (from the snapshot of the pass that recorded it)
#[derive(Default)]
struct ShDraw {
    snap: RefCell<Ren>,
}

impl Draw for ShDraw {
    fn draw(&self, g: &mut Gfx, _r: [i32; 4]) {
        let s = self.snap.borrow();
        let text = &mut *g.text;
        s.render(true, &mut text.path, &mut text.ras, g.dc, g.sf);
        let handles = match s.st {
            S_CREATE => false,
            S_ACTIVE | S_MOVE | S_SIZE => true,
            _ => return,
        };
        if is_box(s.kact) || s.kact == SH_POLYGON {
            let fr = s.frame();
            let hair = s.hair;
            // top, bottom, left, right
            dash(g.dc, g.sf, fr[0], fr[1], fr[2] - fr[0] + hair, false, hair, s.dp4);
            dash(g.dc, g.sf, fr[0], fr[3], fr[2] - fr[0] + hair, false, hair, s.dp4);
            dash(g.dc, g.sf, fr[0], fr[1], fr[3] - fr[1], true, hair, s.dp4);
            dash(g.dc, g.sf, fr[2], fr[1], fr[3] - fr[1], true, hair, s.dp4);
        }
        if !handles {
            return;
        }
        let (h, n) = s.handles();
        let hs = s.dp7; // handle size
        let bd = s.hair; // border
        for q in h.iter().take(n) {
            let x = q[0] - ((hs as u32) >> 1) as i32;
            let y = q[1] - ((hs as u32) >> 1) as i32;
            g.dc.set_color(HANDLE_COL);
            g.dc.fill(g.sf, x, y, hs, hs);
            g.dc.set_color(0xFFFF_FFFF);
            g.dc.fill(g.sf, x + bd, y + bd, hs - bd - bd, hs - bd - bd);
        }
    }
}

// ------------------------------------------------------------------ geometry state

impl Shape {
    // the snapshot of sh_render & co. (the view and the colours from paint-core)
    fn ren(&self, p: &Kritzel, ui: &Ui) -> Ren {
        let t = &p.tools;
        let (col1, col2, size, opacity) = (t.col1, t.col2, t.size, t.opacity);
        let (zoom, cx, cy) = (p.view.zoom, p.view.cx, p.view.cy);
        Ren {
            p: self.p,
            st: self.st,
            kact: self.kact,
            swap: self.swap,
            pts: self.pts[..self.npts].to_vec(),
            outline: self.outline,
            fill: self.fill,
            col1,
            col2,
            size,
            opacity,
            zoom,
            cx,
            cy,
            hair: ui.hair(),
            dp4: ui.dp(4),
            dp7: ui.dp(7),
        }
    }

    // sh_begin(btn, px, py) -- start a new shape at the doc pixel px, py
    fn begin(&mut self, btn: u32, px: i32, py: i32) {
        self.btn = btn;
        self.swap = (btn == BTN_RIGHT) as u32;
        self.anchor = [px, py];
        let fx = px << 8;
        let fy = py << 8;
        for i in 0..4 {
            self.p[i * 2] = fx;
            self.p[i * 2 + 1] = fy;
        }
        self.kact = self.kind;
        if self.kact == SH_POLYGON {
            self.pts[0] = [fx, fy];
            self.pts[1] = [fx, fy];
            self.npts = 2;
            self.st = S_POLYD;
        } else {
            self.st = S_CREATE;
        }
    }

    // sh_normalize() -- box shapes: P0 top-left, P1 bottom-right; a mirrored polygon keeps
    // its look by mirroring its unit coordinates
    fn normalize(&mut self) {
        if self.kact == SH_LINE || self.kact == SH_CURVE {
            return;
        }
        for axis in 0..2 {
            let a = self.p[P0X + axis];
            let b = self.p[P1X + axis];
            if a > b {
                self.p[P0X + axis] = b;
                self.p[P1X + axis] = a;
                if self.kact == SH_POLYGON {
                    for q in self.pts[..self.npts].iter_mut() {
                        q[axis] = 65536i32.wrapping_sub(q[axis]);
                    }
                }
            }
        }
    }

    // sh_poly_finish() -- polygon corners -> unit coordinates of their box; S_ACTIVE (fewer
    // than two corners: discarded)
    fn poly_finish(&mut self) {
        let n = self.npts;
        if n < 2 {
            self.st = S_IDLE;
            return;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for q in &self.pts[..n] {
            x0 = x0.min(q[0]);
            x1 = x1.max(q[0]);
            y0 = y0.min(q[1]);
            y1 = y1.max(q[1]);
        }
        self.p[P0X] = x0;
        self.p[P0Y] = y0;
        self.p[P1X] = x1;
        self.p[P1Y] = y1;
        let w = x1.wrapping_sub(x0);
        let h = y1.wrapping_sub(y0);
        // offset * 65536 / extent (0 if the extent is 0)
        let div = |off: i32, ext: i32| if ext == 0 { 0 } else { ((off as i64) << 16).wrapping_div(ext as i64) as i32 };
        for q in self.pts[..n].iter_mut() {
            q[0] = div(q[0].wrapping_sub(x0), w);
            q[1] = div(q[1].wrapping_sub(y0), h);
        }
        self.kact = SH_POLYGON;
        self.st = S_ACTIVE;
    }

    // sh_bbox() -- doc pixel bounds of the shape's points (x0, y0, x1, y1 exclusive)
    fn bbox(&self) -> [i32; 4] {
        let mut pts: [[i32; 2]; 4] = [[0; 2]; 4];
        let list: &[[i32; 2]] = if self.st == S_POLYD || self.st == S_POLYW {
            // polygon under construction: its corners
            &self.pts[..self.npts.max(1)]
        } else {
            let n = if self.kact == SH_CURVE { 4 } else { 2 };
            for (i, q) in pts.iter_mut().enumerate().take(n) {
                *q = [self.p[i * 2], self.p[i * 2 + 1]];
            }
            &pts[..n]
        };
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for q in list {
            x0 = x0.min(q[0]);
            x1 = x1.max(q[0]);
            y0 = y0.min(q[1]);
            y1 = y1.max(q[1]);
        }
        [x0 >> 8, y0 >> 8, (x1 >> 8) + 1, (y1 >> 8) + 1]
    }

    // sh_hash(env) -> hash of everything the preview depends on (cmd_custom version)
    fn hash(&self, p: &Kritzel) -> u32 {
        let mut h: u32 = 2166136261;
        let mut put = |v: u32| {
            h ^= v;
            h = h.wrapping_mul(FNV_PRIME);
        };
        for v in self.p.iter().chain(self.orig.iter()) {
            put(*v as u32);
        }
        put(self.st);
        put(self.kact as u32);
        put(self.btn);
        put(self.swap);
        put(self.hnd);
        put(self.npts as u32);
        put(self.anchor[0] as u32);
        put(self.anchor[1] as u32);
        for q in self.pts.iter() {
            put(q[0] as u32);
            put(q[1] as u32);
        }
        let (t, v) = (&p.tools, &p.view);
        for v in [t.col1, t.col2, t.size as u32, t.opacity as u32, v.zoom as u32, v.cx as u32, v.cy as u32] {
            put(v);
        }
        put(self.outline as u32);
        put(self.fill as u32);
        h
    }
}

/// ------------------------------------------------------------------ commit

// sh_commit() -- rasterize the shape into the current layer (undoable), end editing
fn sh_commit(p: &mut Kritzel, ui: &mut Ui) {
    let s = &mut p.shape;
    if s.st == S_IDLE {
        return;
    }
    if s.st == S_POLYD || s.st == S_POLYW {
        s.poly_finish();
        if s.st == S_IDLE {
            return;
        }
    }
    let bb = s.bbox();
    let m = p.tools.size.wrapping_mul(2).wrapping_add(4); // margin: miter joins, anti-aliasing
    let r = [bb[0] - m, bb[1] - m, bb[2] + m, bb[3] + m];
    let ren = p.shape.ren(p, ui);
    let layer = p.doc.cur_layer;
    hist_begin(p, layer);
    {
        let Ui { dc, text, .. } = ui;
        let sf = doc_layer(&mut p.doc, layer);
        dc.begin(sf);
        ren.render(false, &mut text.path, &mut text.ras, dc, sf);
        dc.end();
    }
    doc_mark(&mut p.doc, r[0], r[1], r[2], r[3]);
    hist_end(p, r[0], r[1], r[2], r[3]);
    p.shape.st = S_IDLE;
}

// ------------------------------------------------------------------ input

/// shape_input(in) -> true: one frame of canvas input
pub(crate) fn shape_input(p: &mut Kritzel, ui: &mut Ui, inp: &UiInput) -> bool {
    p.shape.can = inp.rect;
    p.shape.tol = ui.dp(6);
    let mx = inp.x.wrapping_add(inp.rect[0]);
    let my = inp.y.wrapping_add(inp.rect[1]);
    let (px, py) = view_to_doc(&p.view, mx, my);
    // keys while a shape is being edited
    if p.shape.st != S_IDLE {
        if ui.key(VK_ESCAPE) || ui.key(VK_DELETE) {
            p.shape.st = S_IDLE;
            return true;
        }
        if ui.key(VK_RETURN) {
            sh_commit(p, ui);
            return true;
        }
    }
    // pressed = button pressed over the canvas this frame (0 = none)
    let mut pressed = 0;
    if inp.ev & EV_HOVER != 0 {
        if inp.pressed & (1 << BTN_LEFT) != 0 {
            pressed = BTN_LEFT;
        } else if inp.pressed & (1 << BTN_RIGHT) != 0 {
            pressed = BTN_RIGHT;
        }
    }
    // a press on the canvas takes the keyboard focus from ribbon buttons, so Enter
    // reaches the shape
    if pressed != 0 && ui.focused() != ID_CANVAS {
        ui.focus(ID_CANVAS);
    }
    let tol = p.shape.tol;
    let s = &mut p.shape;
    // held while the drag button is down
    let mut held = (1u32 << (s.btn & 31)) & inp.btn != 0;
    // dbl = double click (EV_DOUBLE close to the previous press)
    let mut dbl = false;
    if pressed != 0 {
        if inp.ev & EV_DOUBLE != 0 {
            dbl = near_pt(mx, my, s.lastp[0], s.lastp[1], tol);
        }
        s.lastp = [mx, my];
    }
    match s.st {
        S_IDLE => {
            // nothing active: a press starts a shape
            if pressed != 0 {
                s.begin(pressed, px, py);
            }
        }
        S_CREATE => {
            // dragging out a new shape
            let (mut x, mut y) = (px, py);
            if inp.mods & MOD_SHIFT != 0 {
                // box shapes: square; line / curve: 45 degree steps
                let mode = if s.kact > SH_CURVE { 0 } else { 1 };
                (x, y) = constrain(s.anchor[0], s.anchor[1], px, py, mode);
            }
            s.p[P1X] = x << 8;
            s.p[P1Y] = y << 8;
            // curve: control points on the line (1/3, 2/3)
            let t = s.p[P1X].wrapping_sub(s.p[P0X]) / 3;
            s.p[C1X] = s.p[P0X].wrapping_add(t);
            s.p[C2X] = s.p[C1X].wrapping_add(t);
            let t = s.p[P1Y].wrapping_sub(s.p[P0Y]) / 3;
            s.p[C1Y] = s.p[P0Y].wrapping_add(t);
            s.p[C2Y] = s.p[C1Y].wrapping_add(t);
            if !held {
                // released: a click without a drag makes nothing
                if s.p[P0X] == s.p[P1X] && s.p[P0Y] == s.p[P1Y] {
                    s.st = S_IDLE;
                } else {
                    s.normalize();
                    s.st = if s.kact == SH_CURVE { S_BEND1W } else { S_ACTIVE };
                }
            }
        }
        S_ACTIVE => {
            // editable shape: handles, move, or commit + new shape
            if pressed != 0 {
                let ren = p.shape.ren(p, ui);
                let h = ren.hit(mx, my, tol);
                let grab = if h >= 0 {
                    p.shape.hnd = h as u32;
                    S_SIZE
                } else if ren.inside(mx, my, tol) {
                    S_MOVE
                } else {
                    sh_commit(p, ui);
                    p.shape.begin(pressed, px, py);
                    return true;
                };
                let s = &mut p.shape;
                s.st = grab;
                s.btn = pressed;
                s.orig = s.p;
                s.anchor = [px, py];
            }
        }
        S_MOVE => {
            // moving: every point by the pointer's offset
            let dx = px.wrapping_sub(s.anchor[0]) << 8;
            let dy = py.wrapping_sub(s.anchor[1]) << 8;
            for i in 0..4 {
                s.p[i * 2] = s.orig[i * 2].wrapping_add(dx);
                s.p[i * 2 + 1] = s.orig[i * 2 + 1].wrapping_add(dy);
            }
            if !held {
                s.st = S_ACTIVE;
            }
        }
        S_SIZE => {
            // a handle
            if s.kact == SH_LINE || s.kact == SH_CURVE {
                // line / curve: the handle's point follows the pointer
                let i = s.hnd as usize & 3;
                s.p[i * 2] = px << 8;
                s.p[i * 2 + 1] = py << 8;
            } else {
                // box / polygon: move the edges the handle touches
                let o = s.orig;
                s.p[..4].copy_from_slice(&o[..4]);
                let dx = px.wrapping_sub(s.anchor[0]) << 8;
                let dy = py.wrapping_sub(s.anchor[1]) << 8;
                let i = s.hnd as usize & 7;
                let (hx, hy) = (HND_HX[i], HND_HY[i]);
                if hx == 0 {
                    s.p[P0X] = s.p[P0X].wrapping_add(dx);
                }
                if hx == 2 {
                    s.p[P1X] = s.p[P1X].wrapping_add(dx);
                }
                if hy == 0 {
                    s.p[P0Y] = s.p[P0Y].wrapping_add(dy);
                }
                if hy == 2 {
                    s.p[P1Y] = s.p[P1Y].wrapping_add(dy);
                }
            }
            if !held {
                s.normalize();
                s.st = S_ACTIVE;
            }
        }
        S_BEND1W | S_BEND1 => {
            // curve bends: the first drag moves both control points
            if s.st == S_BEND1W {
                if pressed == 0 {
                    return true;
                }
                s.btn = pressed;
                s.st = S_BEND1;
                held = true;
            }
            s.p[C1X] = px << 8;
            s.p[C1Y] = py << 8;
            s.p[C2X] = px << 8;
            s.p[C2Y] = py << 8;
            if !held {
                s.st = S_BEND2W;
            }
        }
        S_BEND2W | S_BEND2 => {
            // the second drag moves C2
            if s.st == S_BEND2W {
                if pressed == 0 {
                    return true;
                }
                s.btn = pressed;
                s.st = S_BEND2;
                held = true;
            }
            s.p[C2X] = px << 8;
            s.p[C2Y] = py << 8;
            if !held {
                s.st = S_ACTIVE;
            }
        }
        S_POLYD => {
            // polygon: the last corner follows the pointer
            let last = s.npts.wrapping_sub(1) & (MAX_POLY - 1);
            let (mut x, mut y) = (px, py);
            if inp.mods & MOD_SHIFT != 0 && last != 0 {
                let q = s.pts[last - 1];
                (x, y) = constrain(q[0] >> 8, q[1] >> 8, px, py, 1);
            }
            s.pts[last] = [x << 8, y << 8];
            if !held {
                // released: drop a corner that repeats the previous one
                if last != 0 && s.pts[last] == s.pts[last - 1] {
                    s.npts -= 1;
                }
                s.st = S_POLYW;
            }
        }
        S_POLYW => {
            if pressed == 0 {
                return true;
            }
            let mut done = dbl; // double click finishes
            if !done && s.npts >= 3 {
                // a click on the first corner closes the polygon
                let q = s.pts[0];
                let (fx, fy) = d2s(&p.view, q[0] + 128, q[1] + 128);
                done = near_pt(mx, my, fx, fy, tol);
            }
            let s = &mut p.shape;
            if !done && s.npts >= MAX_POLY {
                done = true;
            }
            if done {
                s.poly_finish();
            } else {
                let n = s.npts;
                s.pts[n] = [px << 8, py << 8];
                s.npts += 1;
                s.btn = pressed;
                s.st = S_POLYD;
            }
        }
        _ => {}
    }
    true
}

// d2s(x, y) -> sx, sy -- doc 24.8 -> window px with the live view
fn d2s(v: &crate::view::View, x: i32, y: i32) -> (i32, i32) {
    let r = Ren { zoom: v.zoom, cx: v.cx, cy: v.cy, ..Ren::default() };
    r.d2s(x, y)
}

/// shape_cursor() -> cursor bits (CUR_* << HB_CURSOR_SHIFT) for the canvas: resize arrows
/// over handles, the move cross inside the edited shape
pub(crate) fn shape_cursor(p: &mut Kritzel, ui: &mut Ui) -> u32 {
    let handle = |kact: i32, h: usize| if kact <= SH_CURVE { CUR_HAND } else { HND_CUR[h & 7] };
    let s = &p.shape;
    let mut cur = CUR_CROSS;
    match s.st {
        S_MOVE => cur = CUR_MOVE,
        S_SIZE => cur = handle(s.kact, s.hnd as usize),
        S_ACTIVE => {
            p.shape.tol = ui.dp(6);
            let (mx, my) = ui.mouse();
            if mx >= 0 {
                let tol = p.shape.tol;
                let ren = p.shape.ren(p, ui);
                let h = ren.hit(mx, my, tol);
                if h >= 0 {
                    cur = handle(ren.kact, h as usize);
                } else if ren.inside(mx, my, tol) {
                    cur = CUR_MOVE;
                }
            }
        }
        _ => {}
    }
    cur << HB_CURSOR_SHIFT
}

// ------------------------------------------------------------------ overlay

/// shape_overlay() -- preview of the shape being edited (drawn at render time)
pub(crate) fn shape_overlay(p: &mut Kritzel, ui: &mut Ui) {
    if p.shape.st == S_IDLE {
        return;
    }
    if p.tools.tool != TOOL_SHAPE {
        sh_commit(p, ui); // another tool was chosen
        return;
    }
    // only the shape's window rectangle (plus stroke, miters and handles) is redrawn when
    // it changes: its box, grown by 2 x the width + 10 DIP
    let s = &p.shape;
    let bb = s.bbox();
    let ren = s.ren(p, ui);
    let m = (ren.size.wrapping_mul(ren.zoom) / 500).wrapping_add(ui.dp(10)); // margin (px)
    let (mut x0, mut y0) = ren.d2s(bb[0] << 8, bb[1] << 8);
    x0 -= m;
    y0 -= m;
    let (mut x1, mut y1) = ren.d2s(bb[2] << 8, bb[3] << 8);
    x1 += m;
    y1 += m;
    // clipped to the canvas
    let can = s.can;
    x0 = x0.max(can[0]);
    x1 = x1.min(can[0].wrapping_add(can[2]));
    y0 = y0.max(can[1]);
    y1 = y1.min(can[1].wrapping_add(can[3]));
    let w = x1.wrapping_sub(x0);
    let h = y1.wrapping_sub(y0);
    if w <= 0 || h <= 0 {
        return;
    }
    let ver = s.hash(p);
    *s.draw.snap.borrow_mut() = ren;
    let f: DrawRef = s.draw.clone();
    ui.cmd_custom(x0, y0, w, h, Some(f), 0, ver);
}

// ------------------------------------------------------------------ ribbon

// sh_pick_kind(k) -- gallery click: select the shape tool with shape k; an edited box shape
// changes its kind, anything else is committed first
fn sh_pick_kind(p: &mut Kritzel, ui: &mut Ui, k: i32) {
    let s = &mut p.shape;
    if s.st != S_IDLE {
        if s.st == S_ACTIVE && is_box(s.kact) && is_box(k) {
            s.kact = k;
        } else {
            sh_commit(p, ui);
        }
    }
    p.shape.kind = k;
    tool_set(&mut p.tools, TOOL_SHAPE);
}

// gal_cell(i) -- one gallery cell: a subtle 20 DIP button showing shape i
fn gal_cell(p: &mut Kritzel, ui: &mut Ui, i: i32) {
    let id = ID_SHAPE0 + i as u32;
    let d = ui.dp(20);
    ui.place(d, d);
    let ev = ui.behave(id, 0); // no focus: Enter stays with the canvas
    if ev & EV_ACT != 0 {
        sh_pick_kind(p, ui, i);
    }
    let checked = p.tools.tool == TOOL_SHAPE && p.shape.kind == i;
    // state: 0 rest, 1 hover, 2 pressed
    let mut si = 0;
    if ev & EV_HOVER != 0 {
        si = 1;
        if ev & EV_HELD != 0 {
            si = 2;
        }
    }
    let (fill, stroke, border) = if checked {
        let tok = if si == 0 { T_CTRL_HOVER } else { T_CTRL + si * 4 };
        (ui.color(tok), ui.color(T_CTRL_STROKE), ui.hair())
    } else {
        (ui.color(T_SUBTLE + si * 4), 0, 0)
    };
    let r = ui.last_rect();
    let rad = ui.dp(4);
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, border, fill, stroke, stroke);
    // the icon: drawn with the path API at render time
    let tc = ui.color(if si == 2 { T_TEXT2 } else { T_TEXT });
    let s = &mut p.shape;
    while s.icons.len() < SH_COUNT as usize {
        let d = IconDraw::default();
        d.kind.set(s.icons.len() as i32);
        s.icons.push(Rc::new(d));
    }
    let icon = &s.icons[i as usize];
    icon.argb.set(tc);
    icon.dpi.set(ui.dpi());
    let f: DrawRef = icon.clone();
    #[cfg(target_pointer_width = "64")]
    let (user, version) = ((i as usize) | (tc as usize) << 32, 0);
    // The draw object already owns the color. Split its change key across the two
    // renderer fields on wasm32, preserving the native command hash unchanged.
    #[cfg(target_pointer_width = "32")]
    let (user, version) = (i as usize, tc);
    ui.cmd_custom(r[0], r[1], r[2], r[3], Some(f), user, version);
    // semantic record (tests: click #13xx, expect checked)
    let name = SH_NAMES[i as usize];
    let sem = ui.sem(K_BUTTON, id, Some(name), 0);
    if checked {
        sem.state |= SS_CHECKED;
    }
    ui.tooltip(name);
}

// style_menu(id, &v, icon, label, none_label) -- a drop-down button whose flyout picks
// v = 0 (none) or 1 (solid); the rows have the ids id + 2 and id + 3
// TODO(shapes agent): more styles (crayon, marker, oil, pencil, watercolour) like Win11
fn style_menu(ui: &mut Ui, id: u32, v: &mut i32, icon: u32, label: &str, none_label: &str) {
    ui.next_size(0, 30);
    ui.next_icon(icon);
    ui.dropdown_button(id, label);
    if ui.flyout_begin(id, FL_BELOW) {
        if *v == 0 {
            ui.next_flags(ST_CHECKED);
        }
        ui.next_icon(IC_CANCEL);
        if ui.menu_item(id + 2, none_label) & EV_ACT != 0 {
            *v = 0;
        }
        if *v != 0 {
            ui.next_flags(ST_CHECKED);
        }
        ui.next_icon(icon);
        if ui.menu_item(id + 3, "Einfarbig") & EV_ACT != 0 {
            *v = 1;
        }
    }
    ui.flyout_end();
}

/// shape_ribbon() -- gallery (3 rows x 8) and the outline / fill style drop-downs
pub(crate) fn shape_ribbon(p: &mut Kritzel, ui: &mut Ui) {
    ui.box_begin(0, BOX_V | BOX_CENTER, 0);
    let mut i = 0;
    while i < SH_COUNT {
        ui.box_begin(0, BOX_H, 0);
        let mut col = 0;
        while i < SH_COUNT && col < GAL_COLS {
            gal_cell(p, ui, i);
            i += 1;
            col += 1;
        }
        ui.box_end();
    }
    ui.box_end();
    // outline / fill
    ui.box_begin(0, BOX_V | BOX_CENTER, 2);
    let s = &mut p.shape;
    style_menu(ui, ID_SH_OUTLINE, &mut s.outline, IC_CIRCLE_RING, "Kontur", "Keine Kontur");
    style_menu(ui, ID_SH_FILL, &mut s.fill, IC_CIRCLE_FILL, "Füllung", "Keine Füllung");
    ui.box_end();
}

// ------------------------------------------------------------------ commands

/// shape_pre_cmd(id) -> true if the command was consumed
pub(crate) fn shape_pre_cmd(p: &mut Kritzel, ui: &mut Ui, id: u32) -> bool {
    if p.shape.st == S_IDLE || id >= 0x7FFF_0000 {
        return false; // UIC_* (timers, theme, dpi, ...)
    }
    if (CMD_ZOOM_IN..=CMD_ZOOM_FIT).contains(&id) {
        return false; // zooming keeps the shape editable
    }
    if matches!(id, CMD_THEME | CMD_EDIT_COLOR | CMD_SWAP_COLORS | CMD_TOGGLE_LAYERS) {
        return false;
    }
    if id == CMD_DELETE {
        p.shape.st = S_IDLE; // Delete discards the shape
        return true;
    }
    sh_commit(p, ui);
    false
}

/// shape_cmd(id, arg) -> false
pub(crate) fn shape_cmd(p: &mut Kritzel, ui: &mut Ui, id: u32, arg: u32) -> bool {
    let _ = (p, ui, id, arg);
    false
}
