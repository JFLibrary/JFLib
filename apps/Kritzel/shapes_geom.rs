// shapes_geom -- Kritzel shape geometry: unit-square path programs and emitters
//                                                         [apps/paint/src/shapes_geom.s]
//
//   sg_xf(sg, num, den, ox, oy)             transform of the emitters: t = o + v * num / den
//                                           (v doc 24.8, t target 24.8)
//   sg_move(sg, path, x, y)   sg_line(sg, path, x, y)   sg_close(path)   path ops through
//                                           the transform (24.8, origin-relative)
//   sg_cubic(sg, path, c1x, c1y, c2x, c2y, x, y)  cubic Bezier
//   sg_prog(sg, path, prog, l, t, r, b)     run a unit program on the box
//   sg_shape(sg, path, kind, l, t, r, b)    path of a box shape SH_*
//   sg_icon_prog(kind) -> program of the gallery icon (None: use sg_shape)
//   IconDraw                                the draw callback of a gallery cell (sg_draw_icon):
//                                           outline of shape `kind` in colour `argb`
//
// A program is an i32 stream in 1/65536 of the box: 0 end, 1 move u v, 2 line u v,
// 3 cubic u1 v1 u2 v2 u3 v3, 4 close.  The tables are generated offline (regular
// polygons and stars need no trigonometry at run time).

use core::cell::Cell;

use gfx::path::Path;
use gfx::{CAP_ROUND, JOIN_MITER};
use ui::{dp_at, Draw, Gfx};

use crate::consts::*;

const OP_MOVE: i32 = 1;
const OP_LINE: i32 = 2;
const OP_CUBIC: i32 = 3;
const OP_CLOSE: i32 = 4;
const KAPPA: i32 = 141; // 0.5523 * 256 (quarter circle by one cubic)
const CALLOUT_BODY: i64 = 52429; // rounded callout: body = top 80 % of the box
const TAIL_X0: i32 = 14418; // rounded callout tail base 22 % .. 40 %, tip at 10 %
const TAIL_X1: i32 = 26214;
const TAIL_TIP: i32 = 6554;

/// the emitter transform t = o + v * num / den (the .data of shapes_geom.s: g_xf_num,
/// g_xf_den, g_xf_ox, g_xf_oy); Default = the identity
#[derive(Clone, Copy)]
pub(crate) struct Sg {
    num: i64,
    den: i64,
    ox: i32,
    oy: i32,
}

impl Default for Sg {
    fn default() -> Sg {
        Sg { num: 1, den: 1, ox: 0, oy: 0 }
    }
}

/// sg_xf(num, den, ox, oy): den 0 counts as 1
pub(crate) fn sg_xf(sg: &mut Sg, num: i32, den: i32, ox: i32, oy: i32) {
    sg.num = num as i64;
    sg.den = if den == 0 { 1 } else { den as i64 };
    sg.ox = ox;
    sg.oy = oy;
}

// xf_pt(x, y): 64-bit product, signed division (truncating), 32-bit sum
#[inline]
fn xf_pt(sg: &Sg, x: i32, y: i32) -> (i32, i32) {
    let tx = (x as i64).wrapping_mul(sg.num).wrapping_div(sg.den) as i32;
    let ty = (y as i64).wrapping_mul(sg.num).wrapping_div(sg.den) as i32;
    (tx.wrapping_add(sg.ox), ty.wrapping_add(sg.oy))
}

/// sg_move(x, y)
pub(crate) fn sg_move(sg: &Sg, path: &mut Path, x: i32, y: i32) {
    let (x, y) = xf_pt(sg, x, y);
    path.move_fx(x, y);
}

/// sg_line(x, y)
pub(crate) fn sg_line(sg: &Sg, path: &mut Path, x: i32, y: i32) {
    let (x, y) = xf_pt(sg, x, y);
    path.line_fx(x, y);
}

/// sg_close()
pub(crate) fn sg_close(path: &mut Path) {
    path.close();
}

/// sg_cubic(c1x, c1y, c2x, c2y, x, y)
#[allow(clippy::too_many_arguments)]
pub(crate) fn sg_cubic(sg: &Sg, path: &mut Path, c1x: i32, c1y: i32, c2x: i32, c2y: i32, x: i32, y: i32) {
    let (ax, ay) = xf_pt(sg, c1x, c1y);
    let (bx, by) = xf_pt(sg, c2x, c2y);
    let (px, py) = xf_pt(sg, x, y);
    path.cubic_fx(ax, ay, bx, by, px, py);
}

// .sp_map: program coordinates u, v -> l + u * w / 65536, t + v * h / 65536
#[inline]
fn map(u: i32, v: i32, l: i32, t: i32, w: i32, h: i32) -> (i32, i32) {
    let x = ((u as i64).wrapping_mul(w as i64) >> 16) as i32;
    let y = ((v as i64).wrapping_mul(h as i64) >> 16) as i32;
    (x.wrapping_add(l), y.wrapping_add(t))
}

/// sg_prog(prog, l, t, r, b) -- program coordinates u, v map to l + u * (r - l) / 65536
#[allow(clippy::too_many_arguments)]
pub(crate) fn sg_prog(sg: &Sg, path: &mut Path, prog: &[i32], l: i32, t: i32, r: i32, b: i32) {
    let w = r.wrapping_sub(l);
    let h = b.wrapping_sub(t);
    let mut i = 0;
    while i < prog.len() {
        match prog[i] {
            OP_MOVE => {
                let (x, y) = map(prog[i + 1], prog[i + 2], l, t, w, h);
                sg_move(sg, path, x, y);
                i += 3;
            }
            OP_LINE => {
                let (x, y) = map(prog[i + 1], prog[i + 2], l, t, w, h);
                sg_line(sg, path, x, y);
                i += 3;
            }
            OP_CUBIC => {
                let (ax, ay) = map(prog[i + 1], prog[i + 2], l, t, w, h);
                let (bx, by) = map(prog[i + 3], prog[i + 4], l, t, w, h);
                let (px, py) = map(prog[i + 5], prog[i + 6], l, t, w, h);
                sg_cubic(sg, path, ax, ay, bx, by, px, py);
                i += 7;
            }
            OP_CLOSE => {
                path.close();
                i += 1;
            }
            _ => break,
        }
    }
}

/// sg_shape(kind, l, t, r, b) -- path of a box shape
#[allow(clippy::too_many_arguments)]
pub(crate) fn sg_shape(sg: &Sg, path: &mut Path, kind: i32, l: i32, t: i32, r: i32, b: i32) {
    match kind {
        SH_RRECT => gen_rrect(sg, path, l, t, r, b, false),
        SH_CALLOUT_R => gen_rrect(sg, path, l, t, r, b, true),
        k if (k as u32) < SH_COUNT as u32 => {
            if let Some(p) = SHAPE_PROGS[k as usize] {
                sg_prog(sg, path, p, l, t, r, b);
            }
        }
        _ => {}
    }
}

// gen_rrect(l, t, r, b, callout) -- rounded rectangle, radius min(w, h) / 6; with callout
// the body takes the top 80 % and a tail points to the bottom left
#[allow(clippy::too_many_arguments)]
fn gen_rrect(sg: &Sg, path: &mut Path, l: i32, t: i32, r: i32, b: i32, callout: bool) {
    let w = r.wrapping_sub(l);
    let mut h = b.wrapping_sub(t);
    if callout {
        h = ((h as i64).wrapping_mul(CALLOUT_BODY) >> 16) as i32;
    }
    let yb = t.wrapping_add(h); // body bottom
    let m = if h > w { w } else { h };
    let rad = (m / 6).max(0);
    let q = rad - (rad.wrapping_mul(KAPPA) >> 8); // radius - kappa * radius
    let gx = |f: i32| (((w as i64).wrapping_mul(f as i64) >> 16) as i32).wrapping_add(l);
    // top edge, top right corner
    sg_move(sg, path, l + rad, t);
    sg_line(sg, path, r - rad, t);
    sg_cubic(sg, path, r - q, t, r, t + q, r, t + rad);
    // right edge, bottom right corner
    sg_line(sg, path, r, yb - rad);
    sg_cubic(sg, path, r, yb - q, r - q, yb, r - rad, yb);
    if callout {
        // tail: base right, tip, base left
        sg_line(sg, path, gx(TAIL_X1), yb);
        sg_line(sg, path, gx(TAIL_TIP), b);
        sg_line(sg, path, gx(TAIL_X0), yb);
    }
    sg_line(sg, path, l + rad, yb);
    // bottom left corner, left edge, top left corner
    sg_cubic(sg, path, l + q, yb, l, yb - q, l, yb - rad);
    sg_line(sg, path, l, t + rad);
    sg_cubic(sg, path, l, t + q, l + q, t, l + rad, t);
    path.close();
}

/// sg_icon_prog(kind) -> program of the gallery icon, None = draw with sg_shape
pub(crate) fn sg_icon_prog(kind: i32) -> Option<&'static [i32]> {
    if (kind as u32) < SH_COUNT as u32 { ICON_PROGS[kind as usize] } else { None }
}

/// sg_draw_icon: the draw callback of a gallery cell -- outline of shape `kind` in colour
/// `argb` (set when the cell is recorded: the asm's user = kind | argb << 32; dpi = the
/// window's dpi then)
#[derive(Default)]
pub(crate) struct IconDraw {
    pub(crate) kind: Cell<i32>,
    pub(crate) argb: Cell<u32>,
    pub(crate) dpi: Cell<i32>,
}

impl Draw for IconDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let kind = self.kind.get();
        let dpi = self.dpi.get();
        g.dc.set_color(self.argb.get());
        let sg = Sg::default();
        let inset = dp_at(3, dpi);
        // box on pixel centres: l = (x + in) + 0.5, r = (x + w - in) - 0.5
        let l = (r[0] + inset).wrapping_shl(8) + 128;
        let t = (r[1] + inset).wrapping_shl(8) + 128;
        let rr = (r[0] + r[2] - inset).wrapping_shl(8) - 128;
        let b = (r[1] + r[3] - inset).wrapping_shl(8) - 128;
        let text = &mut *g.text;
        text.path.begin(g.dc);
        match sg_icon_prog(kind) {
            Some(p) => sg_prog(&sg, &mut text.path, p, l, t, rr, b),
            None => sg_shape(&sg, &mut text.path, kind, l, t, rr, b),
        }
        // 1 px at 100 %, scaled with the dpi
        let width = ((dpi as u32) << 8) / 96;
        text.path.stroke(&mut text.ras, g.dc, g.sf, width as i32, JOIN_MITER | CAP_ROUND << 4);
    }
}

// ------------------------------------------------------------------ programs

// programs of the box shapes (None = generated in code / not a box shape)
static SHAPE_PROGS: [Option<&[i32]>; SH_COUNT as usize] = [
    None,
    None,
    Some(&SP_OVAL),
    Some(&SP_RECT),
    None,
    None,
    Some(&SP_TRI),
    Some(&SP_RTRI),
    Some(&SP_DIAMOND),
    Some(&SP_PENT),
    Some(&SP_HEX),
    Some(&SP_ARR_R),
    Some(&SP_ARR_L),
    Some(&SP_ARR_U),
    Some(&SP_ARR_D),
    Some(&SP_STAR4),
    Some(&SP_STAR5),
    Some(&SP_STAR6),
    None,
    Some(&SP_CALL_O),
    Some(&SP_CALL_C),
    Some(&SP_HEART),
    Some(&SP_BOLT),
];

// gallery icons
static ICON_PROGS: [Option<&[i32]>; SH_COUNT as usize] = [
    Some(&SP_I_LINE),
    Some(&SP_I_CURVE),
    Some(&SP_OVAL),
    Some(&SP_RECT),
    None,
    Some(&SP_I_POLY),
    Some(&SP_TRI),
    Some(&SP_RTRI),
    Some(&SP_DIAMOND),
    Some(&SP_PENT),
    Some(&SP_HEX),
    Some(&SP_ARR_R),
    Some(&SP_ARR_L),
    Some(&SP_ARR_U),
    Some(&SP_ARR_D),
    Some(&SP_STAR4),
    Some(&SP_STAR5),
    Some(&SP_STAR6),
    None,
    Some(&SP_CALL_O),
    Some(&SP_CALL_C),
    Some(&SP_HEART),
    Some(&SP_BOLT),
];

#[rustfmt::skip]
static SP_OVAL: [i32; 33] = [
    1, 65536, 32768,
    3, 65536, 50865, 50865, 65536, 32768, 65536,
    3, 14671, 65536, 0, 50865, 0, 32768,
    3, 0, 14671, 14671, 0, 32768, 0,
    3, 50865, 0, 65536, 14671, 65536, 32768,
    4,
    0,
];

#[rustfmt::skip]
static SP_RECT: [i32; 14] = [
    1, 0, 0,
    2, 65536, 0,
    2, 65536, 65536,
    2, 0, 65536,
    4,
    0,
];

#[rustfmt::skip]
static SP_TRI: [i32; 11] = [
    1, 32768, 0,
    2, 65536, 65536,
    2, 0, 65536,
    4,
    0,
];

#[rustfmt::skip]
static SP_RTRI: [i32; 11] = [
    1, 0, 0,
    2, 65536, 65536,
    2, 0, 65536,
    4,
    0,
];

#[rustfmt::skip]
static SP_DIAMOND: [i32; 14] = [
    1, 32768, 0,
    2, 65536, 32768,
    2, 32768, 65536,
    2, 0, 32768,
    4,
    0,
];

#[rustfmt::skip]
static SP_PENT: [i32; 17] = [
    1, 32768, 0,
    2, 65536, 25033,
    2, 53020, 65536,
    2, 12516, 65536,
    2, 0, 25033,
    4,
    0,
];

#[rustfmt::skip]
static SP_HEX: [i32; 20] = [
    1, 0, 32768,
    2, 16384, 0,
    2, 49152, 0,
    2, 65536, 32768,
    2, 49152, 65536,
    2, 16384, 65536,
    4,
    0,
];

#[rustfmt::skip]
static SP_ARR_R: [i32; 23] = [
    1, 0, 16384,
    2, 32768, 16384,
    2, 32768, 0,
    2, 65536, 32768,
    2, 32768, 65536,
    2, 32768, 49152,
    2, 0, 49152,
    4,
    0,
];

#[rustfmt::skip]
static SP_ARR_L: [i32; 23] = [
    1, 65536, 16384,
    2, 32768, 16384,
    2, 32768, 0,
    2, 0, 32768,
    2, 32768, 65536,
    2, 32768, 49152,
    2, 65536, 49152,
    4,
    0,
];

#[rustfmt::skip]
static SP_ARR_U: [i32; 23] = [
    1, 16384, 65536,
    2, 16384, 32768,
    2, 0, 32768,
    2, 32768, 0,
    2, 65536, 32768,
    2, 49152, 32768,
    2, 49152, 65536,
    4,
    0,
];

#[rustfmt::skip]
static SP_ARR_D: [i32; 23] = [
    1, 16384, 0,
    2, 16384, 32768,
    2, 0, 32768,
    2, 32768, 65536,
    2, 65536, 32768,
    2, 49152, 32768,
    2, 49152, 0,
    4,
    0,
];

#[rustfmt::skip]
static SP_STAR4: [i32; 26] = [
    1, 32768, 0,
    2, 41573, 23963,
    2, 65536, 32768,
    2, 41573, 41573,
    2, 32768, 65536,
    2, 23963, 41573,
    2, 0, 32768,
    2, 23963, 23963,
    4,
    0,
];

#[rustfmt::skip]
static SP_STAR5: [i32; 32] = [
    1, 32768, 0,
    2, 40504, 25032,
    2, 65536, 25033,
    2, 45285, 40504,
    2, 53020, 65536,
    2, 32768, 50066,
    2, 12516, 65536,
    2, 20251, 40504,
    2, 0, 25033,
    2, 25032, 25032,
    4,
    0,
];

#[rustfmt::skip]
static SP_STAR6: [i32; 38] = [
    1, 32768, 0,
    2, 43684, 16394,
    2, 65536, 16384,
    2, 54600, 32768,
    2, 65536, 49152,
    2, 43684, 49142,
    2, 32768, 65536,
    2, 21852, 49142,
    2, 0, 49152,
    2, 10936, 32768,
    2, 0, 16384,
    2, 21852, 16394,
    4,
    0,
];

#[rustfmt::skip]
static SP_CALL_O: [i32; 36] = [
    1, 12594, 46872,
    3, -536, 38665, -3877, 23883, 4829, 12517,
    3, 13535, 1152, 31515, -3177, 46616, 2456,
    3, 61717, 8089, 69130, 21892, 63843, 34532,
    3, 58556, 47173, 42253, 54627, 25955, 51856,
    2, 6554, 65536,
    4,
    0,
];

#[rustfmt::skip]
static SP_CALL_C: [i32; 171] = [
    1, 27810, 7683,
    3, 29702, -4148, 49173, -1561, 47281, 10270,
    3, 54740, 3188, 66395, 13386, 58936, 20467,
    3, 67945, 20659, 67630, 32975, 58621, 32784,
    3, 65624, 39674, 54283, 49247, 47281, 42356,
    3, 49331, 52733, 32253, 55536, 30203, 45159,
    3, 27019, 54502, 11642, 50148, 14826, 40805,
    3, 7808, 46239, -1135, 36643, 5883, 31210,
    3, -2431, 30768, -1705, 19402, 6610, 19843,
    3, 438, 14394, 9406, 5957, 15577, 11406,
    3, 12854, 3973, 25087, 250, 27810, 7683,
    4,
    1, 22514, 54059,
    3, 22514, 55626, 20841, 56897, 18778, 56897,
    3, 16715, 56897, 15042, 55626, 15042, 54059,
    3, 15042, 52492, 16715, 51222, 18778, 51222,
    3, 20841, 51222, 22514, 52492, 22514, 54059,
    4,
    1, 14516, 59735,
    3, 14516, 60849, 13327, 61752, 11859, 61752,
    3, 10392, 61752, 9202, 60849, 9202, 59735,
    3, 9202, 58620, 10392, 57717, 11859, 57717,
    3, 13327, 57717, 14516, 58620, 14516, 59735,
    4,
    1, 8151, 64149,
    3, 8151, 64915, 7333, 65536, 6324, 65536,
    3, 5315, 65536, 4498, 64915, 4498, 64149,
    3, 4498, 63383, 5315, 62761, 6324, 62761,
    3, 7333, 62761, 8151, 63383, 8151, 64149,
    4,
    0,
];

#[rustfmt::skip]
static SP_HEART: [i32; 47] = [
    1, 32768, 65536,
    3, 13107, 49516, 0, 34953, 0, 18933,
    3, 0, 7282, 7864, 0, 17695, 0,
    3, 24904, 0, 30802, 5097, 32768, 13107,
    3, 34734, 5097, 40632, 0, 47841, 0,
    3, 57672, 0, 65536, 7282, 65536, 18933,
    3, 65536, 34953, 52429, 49516, 32768, 65536,
    4,
    0,
];

#[rustfmt::skip]
static SP_BOLT: [i32; 35] = [
    1, 25705, 0,
    2, 39018, 18447,
    2, 33527, 20623,
    2, 50296, 36430,
    2, 44804, 39070,
    2, 65536, 65536,
    2, 30377, 45253,
    2, 37082, 42438,
    2, 15237, 29446,
    2, 23065, 25432,
    2, 0, 11803,
    4,
    0,
];

#[rustfmt::skip]
static SP_I_LINE: [i32; 7] = [
    1, 0, 65536,
    2, 65536, 0,
    0,
];

#[rustfmt::skip]
static SP_I_CURVE: [i32; 11] = [
    1, 0, 55706,
    3, 19661, -22938, 40632, 88474, 65536, 9830,
    0,
];

#[rustfmt::skip]
static SP_I_POLY: [i32; 20] = [
    1, 0, 16384,
    2, 39322, 0,
    2, 65536, 29491,
    2, 39322, 38011,
    2, 52429, 65536,
    2, 7864, 58982,
    4,
    0,
];
