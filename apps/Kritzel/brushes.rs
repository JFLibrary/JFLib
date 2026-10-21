// brushes -- the brush variants of the brush tool ("Pinsel") and their picker
//                                                         [apps/paint/src/brushes.s]
//
//   brush_input(p, ui, inp) -> true   canvas input while TOOL_BRUSH is active (takes every stroke)
//   brush_abort(p)                    drop a running stroke without recording it
//   brush_overlay(p, ui)              outline of the tip under the pointer (circle, nib, chisel)
//   brush_ribbon(p, ui)               drop-down with a preview stroke of every variant
//   brush_cmd(p, ui, id, arg) -> bool UIC_TIMER of the airbrush; automation commands
//                                     BR_CMD_SET (arg variant), BR_CMD_SIZE, BR_CMD_OPACITY
//
// Variants (B_*): Pinsel (soft round), Kalligrafiepinsel / Kalligrafiestift (flat nib at
// 45 / 135 degrees), Airbrush (random dots per distance and per time), Ölpinsel (bristle
// streaks, lighter valleys), Buntstift (paper grain), Marker (flat translucent chisel, no
// build-up), Natürlicher Bleistift (thin, grainy), Aquarellpinsel (translucent wash with a
// darker wet rim).
//
// Engine: a doc-sized A8 stroke buffer holds a field value per pixel.  Every resampled path
// segment sweeps the tip (a capsule = moving disc, or a moving rotated box: exact Minkowski
// sum, no stamp gaps) into it with max(), so a stroke never builds up over itself.  Each
// frame the touched rectangle of the layer is restored from a row backup (taken lazily since
// the stroke start) and the buffer is blended over it through the lut, 256 premultiplied
// colours that encode the variant's alpha curve, wet rim or streak lightening (blit_over).
// Undo: hist_begin at the press, hist_end with the stroke's box at the release.  The same
// engine renders the preview strokes of the picker into small surfaces.
//
// Float code keeps the asm's SSE operation order (rt::fx: cvtt / cvt / sqrt / minss / maxss).

use alloc::rc::Rc;
use alloc::vec::Vec;

use gfx::{blend, Surface, FMT_A8, FMT_PARGB};
use rt::fx::{cvt, cvtt, maxss, minss, sqrt};
use ui::*;

use crate::consts::*;
use crate::doc::{doc_cur, doc_h, doc_mark, doc_w};
use crate::history::{hist_begin, hist_cancel, hist_end};
use crate::tools::tool_set;
use crate::view::{view_to_doc_fx, view_zoom};
use crate::Kritzel;

const BTN_L_MASK: u32 = 1 << BTN_LEFT;
const BTN_R_MASK: u32 = 1 << BTN_RIGHT;
const ID_BRUSH_ITEM0: u32 = ID_BRUSH + 1; // flyout rows (+ variant)
const BR_CMD_SET: u32 = 0x4250; // command: arg = variant (automation)
const BR_CMD_SIZE: u32 = 0x4251; // arg = size px 1..100
const BR_CMD_OPACITY: u32 = 0x4252; // arg = opacity 1..100
const TIMER_AIR: u32 = 0x0B12_5401; // airbrush: spray while the pointer rests
const AIR_MS: u32 = 30;
const PV_W: i32 = 56; // preview stroke (DIP)
const PV_H: i32 = 22;
const BTN_W: i32 = 64; // drop-down button (DIP)
const BTN_H: i32 = 60;
const ROW_W: i32 = 236; // flyout row (DIP)
const ROW_H: i32 = 40;
// tip shapes, textures, alpha curves, edge modes
const SHP_CAPSULE: u32 = 0;
const SHP_BOX: u32 = 1;
const SHP_SPRAY: u32 = 2;
const TEX_NONE: u32 = 0;
const TEX_CRAYON: u32 = 1;
const TEX_PENCIL: u32 = 2;
const TEX_BRISTLE: u32 = 3;
const LUT_LINEAR: u32 = 0;
const LUT_SMOOTH: u32 = 1;
const LUT_OIL: u32 = 2;
const LUT_WATER: u32 = 3;
const KM_HARD: u32 = 0; // 1 px anti-aliased edge
const KM_SOFT: u32 = 1; // soft edge over SOFT * R px
const KM_DEPTH: u32 = 2; // field = depth inside the tip (wet rim)

const F_INV255: f32 = 0.003921569;
const F_INV256: f32 = 0.00390625;
const F_EPS: f32 = 0.0001;
const F_BIG: f32 = 1.0e9;
const F_OILC: f32 = 0.72;
const F_OILK: f32 = 1.2;
const F_OILMIN: f32 = -0.25;
const F_WEDGE: f32 = 0.62;
const F_WIN: f32 = 0.28;
const F_RIMK: f32 = 0.12;
const F_RIMMIN: f32 = 0.8;
const F_RIMMAX: f32 = 3.0;
const F_FALLK: f32 = 0.3;
const F_FALLMAX: f32 = 10.0;
const F_RND: f32 = 1.1920929e-7; // 2 / 2^24
const F_1_20: f32 = 0.05;
const F_0256: f32 = 0.256;
const F_001: f32 = 0.01;

/// variant record (g_btab): shape, texture, alpha curve, edge mode; R = max(RMIN, size / 2 *
/// RF), nib axis (UX, UY), half thickness = max(TMIN, R * TF), SOFT, maximum alpha AMAX
struct Bp {
    shape: u32,
    tex: u32,
    lut: u32,
    kmode: u32,
    rf: f32,
    rmin: f32,
    ux: f32,
    uy: f32,
    tf: f32,
    tmin: f32,
    soft: f32,
    amax: f32,
}

const fn bp(shape: u32, tex: u32, lut: u32, kmode: u32, f: [f32; 8]) -> Bp {
    Bp { shape, tex, lut, kmode, rf: f[0], rmin: f[1], ux: f[2], uy: f[3], tf: f[4], tmin: f[5], soft: f[6], amax: f[7] }
}

static BTAB: [Bp; 9] = [
    // B_BRUSH: soft round
    bp(SHP_CAPSULE, TEX_NONE, LUT_SMOOTH, KM_SOFT, [1.0, 0.5, 1.0, 0.0, 1.0, 0.5, 0.35, 1.0]),
    // B_CALLI_BRUSH: flat nib "/" with slightly soft edges
    bp(SHP_BOX, TEX_NONE, LUT_LINEAR, KM_SOFT, [1.0, 0.5, 0.70710678, -0.70710678, 0.22, 0.6, 0.06, 1.0]),
    // B_CALLI_PEN: thin hard nib "\"
    bp(SHP_BOX, TEX_NONE, LUT_LINEAR, KM_HARD, [1.0, 0.5, 0.70710678, 0.70710678, 0.08, 0.5, 0.0, 1.0]),
    // B_AIRBRUSH: spray dots
    bp(SHP_SPRAY, TEX_NONE, LUT_LINEAR, KM_HARD, [1.0, 1.0, 1.0, 0.0, 1.0, 0.5, 0.0, 1.0]),
    // B_OIL: bristle streaks
    bp(SHP_CAPSULE, TEX_BRISTLE, LUT_OIL, KM_SOFT, [1.0, 0.5, 1.0, 0.0, 1.0, 0.5, 0.12, 1.0]),
    // B_CRAYON: paper grain
    bp(SHP_CAPSULE, TEX_CRAYON, LUT_LINEAR, KM_HARD, [1.0, 0.5, 1.0, 0.0, 1.0, 0.5, 0.0, 1.0]),
    // B_MARKER: flat chisel, half transparent
    bp(SHP_BOX, TEX_NONE, LUT_LINEAR, KM_HARD, [1.0, 0.5, 1.0, 0.0, 0.6, 0.5, 0.0, 0.5]),
    // B_PENCIL2: thin and grainy
    bp(SHP_CAPSULE, TEX_PENCIL, LUT_LINEAR, KM_HARD, [0.4, 0.6, 1.0, 0.0, 1.0, 0.5, 0.0, 0.85]),
    // B_WATERCOLOR: translucent with a wet rim
    bp(SHP_CAPSULE, TEX_NONE, LUT_WATER, KM_DEPTH, [1.0, 1.0, 1.0, 0.0, 1.0, 0.5, 0.0, 1.0]),
];

static BS_NAMES: [&str; 9] = [
    "Pinsel",
    "Kalligrafiepinsel",
    "Kalligrafiestift",
    "Airbrush",
    "Ölpinsel",
    "Buntstift",
    "Marker",
    "Natürlicher Bleistift",
    "Aquarellpinsel",
];

/// stroke parameters (g_pr)
#[derive(Default, Clone, Copy)]
struct Pr {
    /// radius / nib half length
    r: f32,
    hl: f32,
    /// nib half thickness
    ht: f32,
    /// field = (depth + OFF) * K
    k: f32,
    off: f32,
    /// nib axes u, v
    ux: f32,
    uy: f32,
    vx: f32,
    vy: f32,
    invr: f32,
    /// resample distance
    step: f32,
    /// distance between spray bursts
    airstep: f32,
    amax: f32,
    /// smoothed travel direction
    dirx: f32,
    diry: f32,
    /// distance since the last burst
    airacc: f32,
    shape: u32,
    tex: u32,
    lutm: u32,
    /// dots per burst
    airn: i32,
    dirset: bool,
}

/// sweep block (g_sw)
#[derive(Default, Clone, Copy)]
struct Sw {
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
    dx: f32,
    dy: f32,
    /// 1 / |d|^2 (0 for a dot)
    il2: f32,
    /// unit normal of the segment / bristle axis
    wx: f32,
    wy: f32,
    /// box half-plane offsets
    e1: f32,
    e2: f32,
    e3: f32,
    e4: f32,
    hw: f32,
    hasw: bool,
}

/// the rasterizer: parameters, sweep block, pending box, noise, colour table (shared by
/// strokes and previews; the target surface is passed to each call: g_tg)
struct Eng {
    pr: Pr,
    sw: Sw,
    /// changed since the last composite
    pend: [i32; 4],
    rng: u32,
    bristle: [u8; 64],
    lut: [u32; 256],
    color: u32,
    opac: i32,
}

impl Default for Eng {
    fn default() -> Eng {
        Eng {
            pr: Pr::default(),
            sw: Sw::default(),
            pend: [0; 4],
            rng: 0,
            bristle: [0; 64],
            lut: [0; 256],
            color: 0,
            opac: 0,
        }
    }
}

/// the brush engine's state (the .bss of brushes.s)
#[derive(Default)]
pub(crate) struct Brushes {
    e: Eng,
    /// stroke buffer (doc sized, A8)
    buf: Surface,
    /// preview coverage scratch
    pvbuf: Surface,
    /// preview strokes (PARGB) and their content versions (SF_AUX)
    pv: [Option<Rc<Surface>>; 9],
    pv_aux: [u32; 9],
    /// one composited row
    tmp: Vec<u8>,
    /// row backup of the layer
    bk: Vec<u8>,
    bk_stride: usize,
    bk_y0: i32,
    bk_y1: i32,
    /// whole stroke
    sbox: [i32; 4],
    active: bool,
    btn: u32,
    /// last emitted point (f32 doc px)
    lx: f32,
    ly: f32,
    /// pointer (f32 doc px)
    cx: f32,
    cy: f32,
    /// pointer (screen px) for the outline
    bo_x: i32,
    bo_y: i32,
    bo_hover: bool,
    pv_dpi: i32,
    pv_col: u32,
    pv_ver: u32,
}

// ------------------------------------------------------------------ tool input

/// brush_input(in) -> true -- every TOOL_BRUSH stroke is drawn here
pub(crate) fn brush_input(p: &mut Kritzel, ui: &mut Ui, inp: &UiInput) -> bool {
    let b = &mut p.br;
    // pointer for the tip outline
    b.bo_x = inp.x.wrapping_add(inp.rect[0]);
    b.bo_y = inp.y.wrapping_add(inp.rect[1]);
    b.bo_hover = inp.ev & EV_HOVER != 0;
    if !b.active {
        // a press starts a stroke (left: colour 1, right: colour 2)
        let pr = inp.pressed;
        if pr & (BTN_L_MASK | BTN_R_MASK) == 0 || inp.ev & EV_HOVER == 0 {
            return true;
        }
        let btn = if pr & BTN_L_MASK != 0 { BTN_LEFT } else { BTN_RIGHT };
        let (x, y) = fx_point(p, inp.press_x.wrapping_add(inp.rect[0]), inp.press_y.wrapping_add(inp.rect[1]));
        if bs_begin(p, ui, btn, x, y) != 0 {
            return true;
        }
    }
    // every recorded move point continues the stroke
    for pt in inp.pts.iter() {
        let (x, y) = fx_point(p, pt[0].wrapping_add(inp.rect[0]), pt[1].wrapping_add(inp.rect[1]));
        bs_point(&mut p.br, x, y);
    }
    bs_flush(&mut p.br);
    bs_composite(p);
    // released: end the stroke
    if inp.btn & (1 << p.br.btn) == 0 {
        bs_end(p);
    }
    true
}

// fx_point(sx, sy) -> (x, y) -- screen px to doc px (f32, pixel centres)
fn fx_point(p: &Kritzel, sx: i32, sy: i32) -> (f32, f32) {
    let (x, y) = view_to_doc_fx(&p.view, sx, sy);
    (x as f32 * F_INV256, y as f32 * F_INV256)
}

// bs_surf_ensure(sf, w, h, fmt) -> 0 ok / -1 -- (re)allocate when the size differs
fn bs_surf_ensure(sf: &mut Surface, w: i32, h: i32, fmt: u8) -> i32 {
    if w == sf.w && h == sf.h && !sf.px.is_empty() {
        return 0;
    }
    *sf = Surface::new(w, h, fmt);
    if sf.w != w || sf.h != h { -1 } else { 0 }
}

// bs_begin(btn, x, y) -> 0 ok / -1 -- start a stroke at x, y (f32 doc px)
fn bs_begin(p: &mut Kritzel, ui: &mut Ui, btn: u32, x: f32, y: f32) -> i32 {
    let b = &mut p.br;
    b.btn = btn;
    let col = if btn == BTN_RIGHT { p.tools.col2 } else { p.tools.col1 };
    // stroke buffer and row backup sized like the document
    let (w, h) = (doc_w(&p.doc), doc_h(&p.doc));
    if bs_surf_ensure(&mut b.buf, w, h, FMT_A8) != 0 {
        return -1;
    }
    b.bk_stride = w as usize * 4;
    let need = b.bk_stride * h as usize;
    if need > b.bk.len() {
        b.bk = alloc::vec![0u8; need];
    }
    b.bk_y0 = 0;
    b.bk_y1 = 0;
    pend_reset(&mut b.e);
    box_reset(&mut b.sbox);
    // deterministic noise per stroke (reproducible tests)
    let mut r = cvtt(x).wrapping_mul(73856093) as u32;
    r ^= cvtt(y).wrapping_mul(19349663) as u32;
    r = r.wrapping_add(p.tools.brush as u32) | 1;
    b.e.rng = r;
    if p.tools.brush as u32 >= B_COUNT as u32 {
        p.tools.brush = 0;
    }
    bs_setup(&mut b.e, p.tools.brush as usize, p.tools.size, col, p.tools.opacity);
    let cur = p.doc.cur_layer;
    hist_begin(p, cur);
    let b = &mut p.br;
    b.active = true;
    // the press paints a dot
    b.lx = x;
    b.ly = y;
    b.cx = x;
    b.cy = y;
    bs_emit(&mut b.e, &mut b.buf, x, y, x, y);
    if b.e.pr.shape == SHP_SPRAY {
        ui.timer(TIMER_AIR, AIR_MS);
    }
    0
}

/// brush_abort() -- drop a running stroke without compositing or recording it (the
/// document changed size / was replaced under it); the stroke buffer is cleared
pub(crate) fn brush_abort(p: &mut Kritzel) {
    if !p.br.active {
        return;
    }
    p.br.active = false;
    hist_cancel(p);
    bs_clear_used(&mut p.br);
}

// bs_end() -- final composite, undo record, clear the used part of the buffer
fn bs_end(p: &mut Kritzel) {
    bs_composite(p);
    if !p.br.active {
        return; // bs_composite aborted the stroke
    }
    p.br.active = false;
    let s = p.br.sbox;
    hist_end(p, s[0], s[1], s[2], s[3]);
    bs_clear_used(&mut p.br);
}

// bs_clear_used() -- zero the part of the A8 stroke buffer the stroke touched (sbox)
fn bs_clear_used(b: &mut Brushes) {
    let [x0, y0, x1, y1] = b.sbox;
    let stride = b.buf.stride as usize;
    let mut y = y0;
    while y < y1 {
        let o = y as usize * stride;
        b.buf.px[o + x0 as usize..o + x1 as usize].fill(0);
        y += 1;
    }
}

// bs_point(x, y) -- a new pointer position (f32 doc px); emits a segment once it is at
// least one resample step away from the last emitted point
fn bs_point(b: &mut Brushes, x: f32, y: f32) {
    b.cx = x;
    b.cy = y;
    let dx = x - b.lx;
    let dy = y - b.ly;
    let d = sqrt(dx * dx + dy * dy);
    if d >= b.e.pr.step {
        bs_emit_cur(b);
    }
}

// bs_flush() -- emit the rest of the path up to the pointer
fn bs_flush(b: &mut Brushes) {
    if b.cx != b.lx || b.cy != b.ly {
        bs_emit_cur(b);
    }
}

// bs_emit_cur() -- segment from the last emitted point to the pointer
fn bs_emit_cur(b: &mut Brushes) {
    let (ax, ay) = (b.lx, b.ly);
    b.lx = b.cx;
    b.ly = b.cy;
    bs_emit(&mut b.e, &mut b.buf, ax, ay, b.cx, b.cy);
}

// bs_emit(ax, ay, bx, by) -- one path segment (f32 doc px) into the target
fn bs_emit(e: &mut Eng, tg: &mut Surface, ax: f32, ay: f32, bx: f32, by: f32) {
    e.sw.ax = ax;
    e.sw.ay = ay;
    e.sw.bx = bx;
    e.sw.by = by;
    if e.pr.shape == SHP_SPRAY {
        spray_seg(e, tg);
        return;
    }
    // smoothed travel direction (bristle axis of the oil brush)
    let mut dx = bx - ax;
    let mut dy = by - ay;
    let len = sqrt(dx * dx + dy * dy);
    if len >= 0.5 {
        dx /= len;
        dy /= len;
        let mut set = true;
        if !e.pr.dirset {
            e.pr.dirset = true;
        } else {
            // dir = normalize(dir * 2 step + d * length): long segments set it, short
            // jittery ones only nudge it
            dx *= len;
            dy *= len;
            let s2 = e.pr.step + e.pr.step;
            dx += e.pr.dirx * s2;
            dy += s2 * e.pr.diry;
            let l2 = sqrt(dx * dx + dy * dy);
            if l2 > F_EPS {
                dx /= l2;
                dy /= l2;
            } else {
                set = false;
            }
        }
        if set {
            e.pr.dirx = dx;
            e.pr.diry = dy;
        }
    }
    sweep(e, tg);
}

// ------------------------------------------------------------------ rasterization

// sweep() -- sweep the tip along sw a -> b into the target with max()
// Capsule: depth = R - distance to the segment.  Box: the moving box is the Minkowski sum
// of the nib rectangle and the segment, an intersection of six half planes (normals +-u,
// +-v, +-w); depth = smallest distance to them.
// field = clamp((depth + OFF) * K, 0, 255) * texture / 255
fn sweep(e: &mut Eng, tg: &mut Surface) {
    let pr = e.pr;
    let sw = &mut e.sw;
    let dx = sw.bx - sw.ax;
    sw.dx = dx;
    let dy = sw.by - sw.ay;
    sw.dy = dy;
    let l2 = dx * dx + dy * dy; // |d|^2
    sw.il2 = 0.0;
    sw.hasw = false;
    if l2 > F_EPS {
        sw.il2 = 1.0 / l2;
        let l = sqrt(l2);
        sw.wx = -dy / l; // w = (-dy, dx) / |d|
        sw.wy = dx / l;
        sw.hasw = true;
    }
    let shape = pr.shape;
    let tex = pr.tex;
    let (ex, ey);
    if shape != SHP_BOX {
        // capsule: the bristles follow the smoothed direction
        sw.wx = -pr.diry;
        sw.wy = pr.dirx;
        ex = pr.r + 1.5;
        ey = ex;
    } else {
        let du = dx * pr.ux + dy * pr.uy;
        sw.e1 = maxss(du, 0.0) + pr.hl; // hl + max(0, du)
        sw.e2 = maxss(-du, 0.0) + pr.hl; // hl + max(0, -du)
        let dv = dx * pr.vx + dy * pr.vy;
        sw.e3 = maxss(dv, 0.0) + pr.ht;
        sw.e4 = maxss(-dv, 0.0) + pr.ht;
        // hw = hl |w.u| + ht |w.v| (a dot has no w: unbounded)
        sw.hw = if !sw.hasw {
            F_BIG
        } else {
            let a = (sw.wx * pr.ux + sw.wy * pr.uy).abs() * pr.hl;
            let b = (sw.wx * pr.vx + sw.wy * pr.vy).abs() * pr.ht;
            a + b
        };
        // extents of the nib: hl |ux| + ht |vx|, hl |uy| + ht |vy| (+1.5)
        ex = pr.ux.abs() * pr.hl + pr.vx.abs() * pr.ht + 1.5;
        ey = pr.uy.abs() * pr.hl + pr.vy.abs() * pr.ht + 1.5;
    }
    // pixel box around both ends (ex / ey = extent in x / y)
    let x0 = cvtt(minss(sw.ax, sw.bx) - ex).max(0);
    let x1 = cvtt(maxss(sw.ax, sw.bx) + ex).wrapping_add(1).min(tg.w);
    let y0 = cvtt(minss(sw.ay, sw.by) - ey).max(0);
    let y1 = cvtt(maxss(sw.ay, sw.by) + ey).wrapping_add(1).min(tg.h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let sw = *sw;
    pend_add(e, tg, x0, y0, x1, y1);
    let r = [x0, y0, x1, y1];
    // one loop per tip shape and texture (the asm tests both per pixel; same arithmetic)
    match (shape == SHP_BOX, tex) {
        (false, TEX_NONE) => sweep_rows::<false, TEX_NONE>(&pr, &sw, &e.bristle, tg, r),
        (false, TEX_CRAYON) => sweep_rows::<false, TEX_CRAYON>(&pr, &sw, &e.bristle, tg, r),
        (false, TEX_PENCIL) => sweep_rows::<false, TEX_PENCIL>(&pr, &sw, &e.bristle, tg, r),
        (false, _) => sweep_rows::<false, TEX_BRISTLE>(&pr, &sw, &e.bristle, tg, r),
        (true, TEX_NONE) => sweep_rows::<true, TEX_NONE>(&pr, &sw, &e.bristle, tg, r),
        (true, TEX_CRAYON) => sweep_rows::<true, TEX_CRAYON>(&pr, &sw, &e.bristle, tg, r),
        (true, TEX_PENCIL) => sweep_rows::<true, TEX_PENCIL>(&pr, &sw, &e.bristle, tg, r),
        (true, _) => sweep_rows::<true, TEX_BRISTLE>(&pr, &sw, &e.bristle, tg, r),
    }
}

// the pixel loop of sweep over r = x0, y0, x1, y1 (inside the target)
#[inline(never)]
fn sweep_rows<const BOX: bool, const TEX: u32>(pr: &Pr, sw: &Sw, bristle: &[u8; 64], tg: &mut Surface, r: [i32; 4]) {
    let [x0, y0, x1, y1] = r;
    let stride = tg.stride as usize;
    for y in y0..y1 {
        let py = (y as f32 + 0.5) - sw.ay;
        // row constants: capsule py*dy | box py*uy, py*vy; both py*wy
        let r9 = py * sw.wy;
        let (r7, r8) = if BOX { (py * pr.uy, py * pr.vy) } else { (py * sw.dy, py) };
        let o = y as usize * stride;
        let row = &mut tg.px[o + x0 as usize..o + x1 as usize];
        for (i, dst) in row.iter_mut().enumerate() {
            let x = x0 + i as i32;
            let px = (x as f32 + 0.5) - sw.ax;
            let depth = if !BOX {
                // capsule: distance to the segment
                let mut t = px * sw.dx + r7;
                t *= sw.il2;
                t = minss(maxss(t, 0.0), 1.0);
                let qx = px - t * sw.dx;
                let qy = py - t * sw.dy;
                let d = sqrt(qx * qx + qy * qy);
                pr.r - d
            } else {
                let a = px * pr.ux + r7; // a = p.u
                let b = px * pr.vx + r8; // b = p.v
                let c = px * sw.wx + r9; // c = p.w
                let mut d = sw.e1 - a;
                d = minss(d, sw.e2 + a);
                d = minss(d, sw.e3 - b);
                d = minss(d, sw.e4 + b);
                d = minss(d, sw.hw - c);
                minss(d, sw.hw + c)
            };
            let mut f = (depth + pr.off) * pr.k;
            if !(f > 0.0) {
                continue;
            }
            f = minss(f, 255.0);
            if TEX != TEX_NONE {
                let g = if TEX == TEX_BRISTLE {
                    // streak value by the offset across the travel direction
                    let mut s = px * sw.wx + r9;
                    s *= pr.invr;
                    s += 1.0;
                    s *= 31.5;
                    let k = cvtt(s).clamp(0, 63);
                    bristle[k as usize] as i32
                } else {
                    bs_grain(x as u32, y as u32, TEX) as i32
                };
                f *= g as f32;
                f *= F_INV255;
            }
            let v = cvtt(f) as u8;
            if v > *dst {
                *dst = v;
            }
        }
    }
}

// spray_seg() -- airbrush bursts along sw a -> b, one per air step (the press: a single
// burst)
fn spray_seg(e: &mut Eng, tg: &mut Surface) {
    let dx = e.sw.bx - e.sw.ax;
    let dy = e.sw.by - e.sw.ay;
    let len = sqrt(dx * dx + dy * dy);
    let (ax, ay) = (e.sw.ax, e.sw.ay);
    if !(len > F_EPS) {
        let n = e.pr.airn;
        spray(e, tg, ax, ay, n);
        return;
    }
    let mut d = e.pr.airstep - e.pr.airacc; // distance to the next burst
    while !(d > len) {
        let t = d / len;
        let x = t * dx + ax;
        let y = t * dy + ay;
        let n = e.pr.airn;
        spray(e, tg, x, y, n);
        d += e.pr.airstep;
    }
    d -= len;
    e.pr.airacc = e.pr.airstep - d;
}

// spray(x, y, n) -- n random dots, uniform in the tip disc around x, y
fn spray(e: &mut Eng, tg: &mut Surface, x: f32, y: f32, n: i32) {
    let r = e.pr.r;
    let x0 = cvtt(x - r).wrapping_sub(1);
    let y0 = cvtt(y - r).wrapping_sub(1);
    let x1 = cvtt(x + r).wrapping_add(2);
    let y1 = cvtt(y + r).wrapping_add(2);
    pend_add(e, tg, x0, y0, x1, y1);
    let mut n = n;
    while n != 0 {
        let (a, b) = loop {
            let a = bs_randf(e);
            let b = bs_randf(e);
            if !(a * a + b * b > 1.0) {
                break (a, b);
            }
        };
        n -= 1;
        let fx = a * r + x;
        if !(fx >= 0.0) {
            continue;
        }
        let ix = cvtt(fx);
        let fy = b * r + y;
        if !(fy >= 0.0) {
            continue;
        }
        let iy = cvtt(fy);
        if ix as u32 >= tg.w as u32 || iy as u32 >= tg.h as u32 {
            continue;
        }
        let v = ((bs_rand(e) & 63) + 192) as u8;
        let o = iy as usize * tg.stride as usize + ix as usize;
        if v > tg.px[o] {
            tg.px[o] = v;
        }
    }
}

// bs_grain(x, y, mode) -> 0..255 -- paper texture in doc space
// crayon: wax clumps (bilinear value noise on a 4 px lattice) mixed with pixel noise,
// contrast stretched; pencil: fine noise in 70..255
#[inline]
fn bs_grain(x: u32, y: u32, mode: u32) -> u32 {
    if mode == TEX_PENCIL {
        let a = bs_hash(x, y);
        let b = bs_hash((x >> 1).wrapping_add(99), y >> 1);
        return ((a + b) * 93 >> 8) + 70; // 0..510 * 185 / 510
    }
    let (gx, gy) = (x >> 2, y >> 2);
    let h00 = bs_hash(gx, gy);
    let h10 = bs_hash(gx.wrapping_add(1), gy);
    let h01 = bs_hash(gx, gy.wrapping_add(1));
    let h11 = bs_hash(gx.wrapping_add(1), gy.wrapping_add(1));
    let fx = x & 3;
    let ifx = 4 - fx;
    let bottom = h01 * ifx + h11 * fx; // bottom edge * 4
    let top = h00 * ifx + h10 * fx; // top edge * 4
    let fy = y & 3;
    let ify = 4 - fy;
    let s = (top * ify + bottom * fy) >> 4; // smooth noise 0..255
    let pn = bs_hash(x, y); // pixel noise
    let v = ((s * 3 + pn * 2) * 102 >> 8) as i32 - 80; // 3 smooth + 2 pixel (0..1275) * 2 / 5
    v.clamp(0, 255) as u32
}

// bs_hash(x, y) -> 0..255
#[inline(always)]
fn bs_hash(x: u32, y: u32) -> u32 {
    let c = x.wrapping_mul(0x27D4_EB2D).wrapping_add(y.wrapping_mul(0x1656_67B1));
    let mut a = (c >> 15) ^ c;
    a = a.wrapping_mul(0x85EB_CA6B);
    a ^= a >> 13;
    (a >> 16) & 255
}

// bs_rand() -> next xorshift32 value
fn bs_rand(e: &mut Eng) -> u32 {
    let mut a = e.rng;
    a ^= a << 13;
    a ^= a >> 17;
    a ^= a << 5;
    e.rng = a;
    a
}

// bs_randf() -> uniform in [-1, 1)
fn bs_randf(e: &mut Eng) -> f32 {
    (bs_rand(e) >> 8) as f32 * F_RND - 1.0
}

// ------------------------------------------------------------------ dirty boxes

// pend_add(x0, y0, x1, y1) -- clamp to the target and grow the pending box
fn pend_add(e: &mut Eng, tg: &Surface, x0: i32, y0: i32, x1: i32, y1: i32) {
    let x0 = x0.max(0);
    let y0 = y0.max(0);
    let x1 = x1.min(tg.w);
    let y1 = y1.min(tg.h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    box_union(&mut e.pend, [x0, y0, x1, y1]);
}

// box_union(box, x0, y0, x1, y1)
fn box_union(b: &mut [i32; 4], r: [i32; 4]) {
    b[0] = b[0].min(r[0]);
    b[1] = b[1].min(r[1]);
    b[2] = b[2].max(r[2]);
    b[3] = b[3].max(r[3]);
}

// pend_reset() / box_reset(box) -- empty box
fn pend_reset(e: &mut Eng) {
    box_reset(&mut e.pend);
}

fn box_reset(b: &mut [i32; 4]) {
    *b = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
}

// ------------------------------------------------------------------ compositing

// bs_composite() -- restore the pending rectangle of the layer from the backup, blend the
// stroke buffer over it and mark the document
fn bs_composite(p: &mut Kritzel) {
    // the buffers were sized for the document at the press: if it changed since (undo of a
    // resize while the button is held), stop instead of writing past them
    if doc_w(&p.doc) != p.br.buf.w || doc_h(&p.doc) != p.br.buf.h {
        brush_abort(p);
        return;
    }
    let b = &mut p.br;
    let [x0, y0, x1, y1] = b.e.pend;
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    box_union(&mut b.sbox, b.e.pend);
    pend_reset(&mut b.e);
    let layer = doc_cur(&mut p.doc);
    bk_ensure(b, layer, y0, y1);
    let w = (x1 - x0) as usize;
    if b.tmp.len() < w * 4 {
        b.tmp.resize(w * 4, 0);
    }
    let ls = layer.stride as usize;
    let bs = b.buf.stride as usize;
    for y in y0 as usize..y1 as usize {
        let lo = y * ls + x0 as usize * 4;
        let dst = &mut layer.px[lo..lo + w * 4];
        let bo = y * b.bk_stride + x0 as usize * 4;
        dst.copy_from_slice(&b.bk[bo..bo + w * 4]);
        let ao = y * bs + x0 as usize;
        bs_gather(&mut b.tmp[..w * 4], &b.buf.px[ao..ao + w], &b.e.lut);
        blend::blit_over(dst, &b.tmp[..w * 4], 255);
    }
    doc_mark(&mut p.doc, x0, y0, x1, y1);
}

// bs_gather(dst, a8, n) -- dst[i] = lut[a8[i]]
#[inline]
fn bs_gather(dst: &mut [u8], a8: &[u8], lut: &[u32; 256]) {
    for (d, &a) in dst.chunks_exact_mut(4).zip(a8.iter()) {
        d.copy_from_slice(&lut[a as usize].to_le_bytes());
    }
}

// bk_ensure(y0, y1) -- the layer rows y0..y1 are in the backup (copied on first use; the
// saved rows stay one contiguous range)
fn bk_ensure(b: &mut Brushes, layer: &Surface, y0: i32, y1: i32) {
    if b.bk_y0 >= b.bk_y1 {
        bk_rows(b, layer, y0, y1);
        b.bk_y0 = y0;
        b.bk_y1 = y1;
        return;
    }
    if y0 < b.bk_y0 {
        bk_rows(b, layer, y0, b.bk_y0);
        b.bk_y0 = y0;
    }
    if y1 > b.bk_y1 {
        bk_rows(b, layer, b.bk_y1, y1);
        b.bk_y1 = y1;
    }
}

// bk_rows(y0, y1) -- copy whole rows of the current layer into the backup
fn bk_rows(b: &mut Brushes, layer: &Surface, y0: i32, y1: i32) {
    let n = layer.w as usize * 4;
    let ls = layer.stride as usize;
    for y in y0.max(0) as usize..y1.max(0) as usize {
        let d = y * b.bk_stride;
        b.bk[d..d + n].copy_from_slice(&layer.px[y * ls..y * ls + n]);
    }
}

// ------------------------------------------------------------------ parameters

// bs_setup(variant, size, colour, opacity) -- stroke parameters, colour table and bristle
// profile (rng must be seeded)
fn bs_setup(e: &mut Eng, variant: usize, size: i32, color: u32, opac: i32) {
    e.color = color;
    e.opac = opac;
    let bp = &BTAB[variant];
    let pr = &mut e.pr;
    pr.shape = bp.shape;
    pr.tex = bp.tex;
    pr.lutm = bp.lut;
    // R = max(RMIN, size / 2 * RF)
    let r = maxss(size as f32 * 0.5 * bp.rf, bp.rmin);
    pr.r = r;
    pr.hl = r;
    pr.invr = 1.0 / r;
    pr.ht = maxss(r * bp.tf, bp.tmin);
    // nib axes: u and v = (-uy, ux)
    pr.ux = bp.ux;
    pr.vy = bp.ux;
    pr.uy = bp.uy;
    pr.vx = -bp.uy;
    // field scale
    pr.off = 0.5;
    let mut k = 255.0f32;
    if bp.kmode == KM_SOFT {
        k /= maxss(r * bp.soft, 1.0);
    } else if bp.kmode == KM_DEPTH {
        k /= r + 0.5;
    }
    pr.k = k;
    pr.amax = bp.amax;
    // resample step clamp(size / 4, 1, 12)
    pr.step = minss(maxss(size as f32 * 0.25, 1.0), 12.0);
    // airbrush: a burst every max(1, R / 2), clamp(R^2 / 4, 2, 400) dots
    pr.airstep = maxss(r * 0.5, 1.0);
    pr.airn = cvtt(r * r * 0.25).clamp(2, 400);
    pr.airacc = 0.0;
    pr.diry = 0.0;
    pr.dirx = 1.0;
    pr.dirset = false;
    bs_bristles(e);
    bs_lut(e);
}

// bs_bristles() -- a fresh streak profile for the oil brush: 64 smoothed values 60..253
fn bs_bristles(e: &mut Eng) {
    let mut raw = [0u8; 66];
    for r in raw.iter_mut() {
        *r = bs_rand(e) as u8;
    }
    for i in 0..64 {
        let a = (raw[i] as u32 + raw[i + 1] as u32) >> 1;
        e.bristle[i] = ((a * 195 >> 8) + 60) as u8;
    }
}

// bs_lut() -- lut[v]: premultiplied colour of field value v for the stroke colour, opacity
// and the variant's alpha curve
fn bs_lut(e: &mut Eng) {
    let pr = e.pr;
    let a = (e.color >> 24).wrapping_mul(e.opac as u32) as i32;
    let ascale = a as f32 * F_001 * pr.amax; // alpha scale (0..255)
    // watercolour: rim width and fall-off (px)
    let rim = minss(maxss(pr.r * F_RIMK, F_RIMMIN), F_RIMMAX);
    let fall = minss(maxss(pr.r * F_FALLK, 1.0), F_FALLMAX);
    e.lut[0] = 0;
    for v in 1..256u32 {
        let mut x = v as f32 * F_INV255; // x = v / 255
        let mut light = 0.0f32; // > 0 lighter, < 0 darker
        match pr.lutm {
            LUT_SMOOTH => {
                let mut t = 3.0 - (x + x);
                t *= x;
                t *= x; // x^2 (3 - 2x)
                x = t;
            }
            LUT_OIL => {
                // thin paint (low bristle value) lighter, thick ridges darker
                light = minss(maxss((F_OILC - x) * F_OILK, F_OILMIN), 0.5);
                x = minss(v as f32 * F_001, 1.0);
            }
            LUT_WATER => {
                let d = v as f32 / pr.k - pr.off; // depth inside the tip (px)
                let edge = 0.5f32 + rim;
                if !(d >= 0.5) {
                    x = (d + 0.5) * F_WEDGE; // anti-aliased edge
                } else if !(d >= edge) {
                    x = F_WEDGE; // the wet rim
                } else {
                    let t = minss((d - edge) / fall, 1.0);
                    x = (F_WIN - F_WEDGE) * t + F_WEDGE; // towards the translucent inside
                }
            }
            _ => {}
        }
        let a8 = cvt(x * ascale).clamp(0, 255) as u32;
        let mut px = a8 << 24;
        let af = a8 as f32 * F_INV255;
        for ch in [0u32, 8, 16] {
            let mut c = ((e.color >> ch) & 255) as f32;
            if light >= 0.0 {
                c += (255.0 - c) * light; // towards white
            } else {
                c += c * light; // towards black
            }
            c *= af;
            let mut iv = cvt(c) as u32;
            if iv > a8 {
                iv = a8;
            }
            px |= iv << ch;
        }
        e.lut[v as usize] = px;
    }
}

// bs_air_tick() -> true -- the airbrush timer: spray at the resting pointer
fn bs_air_tick(p: &mut Kritzel, ui: &mut Ui) -> bool {
    let b = &mut p.br;
    if !b.active || b.e.pr.shape != SHP_SPRAY {
        return true;
    }
    let n = ((b.e.pr.airn as u32 >> 1) + 1) as i32;
    spray(&mut b.e, &mut b.buf, b.cx, b.cy, n);
    bs_composite(p);
    ui.timer(TIMER_AIR, AIR_MS);
    true
}

// ------------------------------------------------------------------ previews

// bs_previews() -- (re)render the preview strokes when the dpi or text colour changed
fn bs_previews(p: &mut Kritzel, ui: &mut Ui) {
    let b = &mut p.br;
    if b.active {
        return;
    }
    let d = ui.dp(96);
    let c = ui.color(T_TEXT);
    if d == b.pv_dpi && c == b.pv_col {
        return;
    }
    b.pv_dpi = d;
    b.pv_col = c;
    let w = ui.dp(PV_W);
    let h = ui.dp(PV_H);
    if bs_surf_ensure(&mut b.pvbuf, w, h, FMT_A8) != 0 {
        return;
    }
    for i in 0..B_COUNT as usize {
        let mut sf = Surface::new(w, h, FMT_PARGB);
        if sf.w != w || sf.h != h {
            continue;
        }
        bs_preview(b, i, &mut sf, c);
        b.pv[i] = Some(Rc::new(sf));
    }
}

// bs_preview(variant, sf, colour) -- a wavy sample stroke of the variant into sf
fn bs_preview(b: &mut Brushes, variant: usize, sf: &mut Surface, color: u32) {
    let n = b.pvbuf.stride as usize * b.pvbuf.h as usize;
    b.pvbuf.px[..n].fill(0);
    let e = &mut b.e;
    pend_reset(e);
    // tip size: 42 % of the height
    let size = ((sf.h as u32).wrapping_mul(42) / 100).max(3) as i32;
    e.rng = (variant as u32).wrapping_mul(0x2545_F491) | 1;
    bs_setup(e, variant, size, color, 100);
    // wave: x = pad .. w - pad, y = h/2 -+ amp * 4 s (1 - s)
    let pad = size as f32 * 0.5 + 2.0;
    let span = sf.w as f32 - pad - pad;
    let mid = sf.h as f32 * 0.5;
    let amp = maxss(mid - pad, 1.0);
    let (mut lx, mut ly) = (0.0f32, 0.0f32);
    for i in 0..=20 {
        let t = i as f32 * F_1_20;
        let mut s = t + t;
        let sign;
        if s >= 1.0 {
            sign = 1.0f32; // + in the second half
            s -= 1.0;
        } else {
            sign = -1.0;
        }
        let mut yv = 1.0 - s;
        yv *= s;
        yv *= 4.0; // 4 s (1 - s)
        yv *= amp;
        yv *= sign;
        yv += mid;
        let xv = t * span + pad;
        let (ax, ay) = if i == 0 { (xv, yv) } else { (lx, ly) };
        lx = xv;
        ly = yv;
        bs_emit(e, &mut b.pvbuf, ax, ay, xv, yv);
    }
    // colours into the preview surface (transparent background: the table as is)
    let ss = sf.stride as usize;
    let ps = b.pvbuf.stride as usize;
    let w = sf.w as usize;
    for y in 0..sf.h as usize {
        bs_gather(&mut sf.px[y * ss..y * ss + w * 4], &b.pvbuf.px[y * ps..y * ps + w], &b.e.lut);
    }
    b.pv_ver = b.pv_ver.wrapping_add(1);
    b.pv_aux[variant] = b.pv_ver;
}

// ------------------------------------------------------------------ ribbon

/// brush_ribbon() -- the variant picker: a drop-down showing the current variant's preview
/// stroke; its flyout lists every variant with a preview and its name
pub(crate) fn brush_ribbon(p: &mut Kritzel, ui: &mut Ui) {
    bs_previews(p, ui);
    let (w, h) = (ui.dp(BTN_W), ui.dp(BTN_H));
    let r = ui.place(w, h);
    let ev = ui.behave(ID_BRUSH, 0);
    let open = ui.popup_is_open(ID_BRUSH);
    // background: pressed, hovered or open
    let tok = if ev & (EV_HOVER | EV_HELD) == EV_HOVER | EV_HELD {
        Some(T_SUBTLE_PRESSED)
    } else if ev & EV_HOVER != 0 || open {
        Some(T_SUBTLE_HOVER)
    } else {
        None
    };
    if let Some(t) = tok {
        let c = ui.color(t);
        ui.draw_color(c);
        let rr = ui.dp(4);
        ui.draw_rrect(r[0], r[1], r[2], r[3], rr);
    }
    // preview of the current variant in the upper part
    let mut v = p.tools.brush as u32 as usize;
    if v >= B_COUNT as usize {
        v = 0;
    }
    if let Some(sf) = p.br.pv[v].clone() {
        let y = r[1].wrapping_add(ui.dp(10));
        let x = (r[2].wrapping_sub(sf.w) >> 1).wrapping_add(r[0]);
        ui.draw_image(&sf, p.br.pv_aux[v], x, y, 0, 0);
    }
    let c = ui.color(T_TEXT2);
    ui.draw_color(c);
    let s = ui.dp(12);
    let y = r[1].wrapping_add(r[3]).wrapping_sub(ui.dp(22));
    let x = (r[2].wrapping_sub(s) >> 1).wrapping_add(r[0]);
    ui.draw_icon(IC_CHEV_DOWN_SMALL, x, y, s << 8);
    ui.sem(K_COMBO, ID_BRUSH, Some("Pinselart"), p.tools.brush);
    // a click toggles the flyout
    if ev & EV_ACT != 0 {
        if open {
            ui.popup_close();
        } else {
            ui.popup_open(ID_BRUSH);
        }
    }
    if ui.flyout_begin(ID_BRUSH, FL_BELOW) {
        for i in 0..B_COUNT {
            bs_menu_row(p, ui, i);
        }
    }
    ui.flyout_end();
}

// bs_menu_row(i) -- one flyout row: preview stroke and name; a click selects variant i
fn bs_menu_row(p: &mut Kritzel, ui: &mut Ui, i: i32) {
    let (w, h) = (ui.dp(ROW_W), ui.dp(ROW_H));
    let r = ui.place(w, h);
    let id = ID_BRUSH_ITEM0 + i as u32;
    let ev = ui.behave(id, HB_FOCUS);
    let sel = i == p.tools.brush;
    // highlight
    let tok = if ev & (EV_HOVER | EV_HELD) == EV_HOVER | EV_HELD {
        Some(T_SUBTLE_PRESSED)
    } else if ev & (EV_HOVER | EV_FOCUS) != 0 || sel {
        Some(T_SUBTLE_HOVER)
    } else {
        None
    };
    if let Some(t) = tok {
        let c = ui.color(t);
        ui.draw_color(c);
        let rr = ui.dp(4);
        ui.draw_rrect(r[0], r[1], r[2], r[3], rr);
        // accent pill of the selected variant
        if sel {
            let c = ui.color(T_ACCENT);
            ui.draw_color(c);
            let ph = ui.dp(16);
            let pw = ui.dp(3);
            let y = (r[3].wrapping_sub(ph) >> 1).wrapping_add(r[1]);
            ui.draw_rrect(r[0], y, pw, ph, (pw as u32 >> 1) as i32);
        }
    }
    let pad = ui.dp(12); // left padding
    if let Some(sf) = p.br.pv[i as usize].clone() {
        let x = r[0].wrapping_add(pad);
        let y = (r[3].wrapping_sub(sf.h) >> 1).wrapping_add(r[1]);
        ui.draw_image(&sf, p.br.pv_aux[i as usize], x, y, 0, 0);
    }
    let c = ui.color(T_TEXT);
    ui.draw_color(c);
    let tx = (pad + pad).wrapping_add(ui.dp(PV_W)).wrapping_add(r[0]); // text x
    let line = ui.ts_line(TS_BODY);
    let base = (r[3].wrapping_sub(line) >> 1).wrapping_add(r[1]).wrapping_add(ui.ts_base(TS_BODY)); // baseline
    let fs = (ui.dp(14) as u32) << 16 | FONT_REG;
    let name = BS_NAMES[i as usize];
    ui.draw_text(tx, base, name, fs);
    ui.sem(K_MENUITEM, id, Some(name), 0);
    if ev & EV_ACT != 0 {
        p.tools.brush = i;
        tool_set(&mut p.tools, TOOL_BRUSH);
        ui.popup_close();
        ui.repass();
    }
}

// ------------------------------------------------------------------ overlay, commands

/// brush_overlay() -- outline of the tip under the pointer: a circle for round tips, the
/// nib for the calligraphy brushes, the chisel rectangle for the marker
pub(crate) fn brush_overlay(p: &mut Kritzel, ui: &mut Ui) {
    if p.tools.tool != TOOL_BRUSH {
        // the tool changed during a stroke: finish it
        if p.br.active {
            bs_end(p);
        }
        return;
    }
    if !p.br.bo_hover {
        return;
    }
    let v = p.tools.brush as u32;
    if v >= B_COUNT as u32 {
        return;
    }
    let bp = &BTAB[v as usize];
    let zs = view_zoom(&p.view) as f32 * F_0256; // doc px -> screen 24.8
    let r = maxss(p.tools.size as f32 * 0.5 * bp.rf, bp.rmin); // R (doc px)
    let ht = maxss(r * bp.tf, bp.tmin); // nib half thickness
    let hts = cvtt(ht * zs);
    let rs = cvtt(r * zs); // R (screen 24.8)
    if rs < 3 << 8 {
        return;
    }
    ui.draw_color(0xB070_7070);
    let hair = ui.hair(); // hairline px
    let (bx, by) = (p.br.bo_x, p.br.bo_y);
    let cx = (bx << 8).wrapping_add(128);
    let cy = (by << 8).wrapping_add(128);
    if bp.shape != SHP_BOX {
        ui.draw_circle(cx, cy, rs, hair << 8);
        return;
    }
    if bp.uy != 0.0 {
        // calligraphy: the nib as a line of its thickness
        let rf = rs as f32;
        let hx = cvtt(bp.ux * rf); // nib half vector
        let hy = cvtt(bp.uy * rf);
        let w = (hts + hts).max(hair << 8);
        ui.draw_line(cx - hx, cy - hy, cx + hx, cy + hy, w);
        return;
    }
    // marker: the chisel rectangle
    let hh = (hts >> 8).max(1); // half height px
    let hw = rs >> 8; // half width px
    ui.draw_frame(bx - hw, by - hh, hw + hw, hh + hh, 0, hair);
}

/// brush_cmd(id, arg) -> true if handled
pub(crate) fn brush_cmd(p: &mut Kritzel, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        UIC_TIMER => arg == TIMER_AIR && bs_air_tick(p, ui),
        BR_CMD_SET => {
            if arg < B_COUNT as u32 && !p.br.active {
                p.tools.brush = arg as i32;
                tool_set(&mut p.tools, TOOL_BRUSH);
            }
            true
        }
        BR_CMD_SIZE => {
            p.tools.size = (arg as i32).clamp(1, 100);
            true
        }
        BR_CMD_OPACITY => {
            p.tools.opacity = (arg as i32).clamp(1, 100);
            true
        }
        _ => false,
    }
}
