// cursors -- PaintCraft's own pointer cursors (like paint.net's tool cursors): a small black
// crosshair with a white rim at the hot spot and the tool's colourful icon (pn_icons.rs)
// below right with a white halo, so it reads on light and dark images; plus a rotate cursor
// for the move tools outside the selection.  The library draws them at the window's dpi
// (ui.cursor_new); without custom cursors (headless runs) the canvas shows the crosshair.
//                                                                [apps/PaintCraft/src/cursors.s]
//
//   pn_cursor(kind) -> ui cursor handle (0 = none), made on first use: kind 0..TOOL_COUNT-1
//                      = that tool's cursor, PNC_ROTATE = rotate
//   pn_cursor_shot(path) -> 0 / ERR_*   (test hook, cmd 899) every cursor at 100 / 150 /
//                      200 % side by side on grey and dark stripes into a PNG
// Tool cursor functions (TD_CURSOR) may return PNC_BASE + kind; tool_cursor turns every
// crosshair and PNC_* into CUR_CUSTOM with ui.cursor_custom.

use alloc::rc::Rc;

use gfx::{Dc, Surface, CAP_ROUND, FMT_PARGB, JOIN_ROUND, RULE_NONZERO};
use rt::fx::cvtd;
use ui::*;

use crate::app::constants::*;
use crate::workspace::icons::{pn_icon_paint, IconSet};
use crate::app::Pn;

const PC_W: u32 = 32; // cursor box (DIP)
const PC_H: u32 = 32;
const PC_HOT: u32 = 7; // crosshair centre = hot spot (DIP)
const PC_ARM0: u32 = 1; // crosshair from 1 to 14 DIP
const PC_ARM1: u32 = 14;
const PC_ICON: u32 = 14; // icon box at 14, 14 (DIP)
const PC_ICON_DIP: u32 = 16;
const PC_KINDS: usize = 32;
const RC_HOT: u32 = 12; // rotate cursor: centre of the arc (DIP)
const SHOT_CELL: i32 = 72; // pn_cursor_shot: px per cursor cell
const SHOT_ROWS: i32 = 3; // 100 / 150 / 200 %
static ROT_WIDTH: [f64; 2] = [3.5, 1.5]; // rim, line (DIP)
// three quarter circles of radius 7 around (12, 12) from the top, counter-clockwise to the
// right side (k = 0.5523 * 7 = 3.866): start, then c1, c2, end per quarter
static ROT_ARC: [f64; 20] = [
    12.0, 5.0, 8.134, 5.0, 5.0, 8.134, 5.0, 12.0, 5.0, 15.866, 8.134, 19.0, 12.0, 19.0, 15.866, 19.0, 19.0, 15.866,
    19.0, 12.0,
];
// arrow head at the end (pointing up at 19, 12 -> 19, 8)
static ROT_HEAD: [f64; 6] = [15.5, 12.0, 22.5, 12.0, 19.0, 8.0];
static HALO_DX: [i8; 8] = [-1, 0, 1, -1, 1, -1, 0, 1];
static HALO_DY: [i8; 8] = [-1, -1, -1, 0, 0, 1, 1, 1];

/// the state of cursors.s (the handles made so far)
#[derive(Default)]
pub(crate) struct Cursors {
    /// g_pc_handle
    handle: [u32; PC_KINDS],
}

/// pn_cursor(kind) -> handle (0 = none)
pub(crate) fn pn_cursor(p: &mut Pn, ui: &mut Ui, kind: u32) -> u32 {
    let k = kind as usize;
    if k >= PC_KINDS {
        return 0;
    }
    if p.cur.handle[k] != 0 {
        return p.cur.handle[k];
    }
    let h = if kind == PNC_ROTATE as u32 {
        ui.cursor_new(Rc::new(pc_draw_rotate), PC_W, PC_H, RC_HOT | RC_HOT << 16)
    } else {
        if kind >= TOOL_COUNT as u32 {
            return 0;
        }
        // the tool's icon (its command record)
        let id = CMD_TOOL0 + kind;
        let icon = p.cmds.iter().find(|c| c.id == id).map_or(0, |c| c.icon);
        if icon == 0 {
            return 0;
        }
        let set = p.icons.set.clone();
        ui.cursor_new(Rc::new(move |g: &mut Gfx, dpi: i32| pc_draw_tool(&set, g, dpi, icon)), PC_W, PC_H, PC_HOT | PC_HOT << 16)
    };
    p.cur.handle[k] = h;
    h
}

// pc_px(dip, dpi) -> round(dip * dpi / 96)
fn pc_px(dip: u32, dpi: u32) -> i32 {
    (dip.wrapping_mul(dpi).wrapping_add(48) / 96) as i32
}

// pc_unit(dpi) -> max(1, round(dpi / 96)) -- line width / rim of 1 DIP
fn pc_unit(dpi: u32) -> i32 {
    (pc_px(1, dpi) as u32).max(1) as i32
}

// pc_draw_tool(sf, dpi, icon) -- crosshair at the hot spot, the icon with a white halo
fn pc_draw_tool(set: &IconSet, g: &mut Gfx, dpi: i32, icon: u32) {
    let d = dpi as u32;
    let u = pc_unit(d);
    let c = pc_px(PC_HOT, d);
    let a0 = pc_px(PC_ARM0, d);
    let len = pc_px(PC_ARM1, d) - a0;
    // white rim
    g.dc.set_color(0xFFFF_FFFF);
    g.dc.fill(g.sf, a0 - u, c - u, len + u * 2, u + u * 2);
    g.dc.fill(g.sf, c - u, a0 - u, u + u * 2, len + u * 2);
    // black cross
    g.dc.set_color(0xFF00_0000);
    g.dc.fill(g.sf, a0, c, len, u);
    g.dc.fill(g.sf, c, a0, u, len);
    // the icon: halo (silhouette at 8 offsets), then the icon itself
    let ip = pc_px(PC_ICON, d);
    let isz = pc_px(PC_ICON_DIP, d) << 8;
    for k in 0..8 {
        let x = HALO_DX[k] as i32 * u + ip;
        let y = HALO_DY[k] as i32 * u + ip;
        pn_icon_paint(set, g, icon, x, y, isz, 0xFFFF_FFFF, 0xFFFF_FFFF);
    }
    pn_icon_paint(set, g, icon, ip, ip, isz, 0xFF1F_1F1F, 0);
}

// rc_fx(x, y) -> 24.8: DIP point * scale
fn rc_fx(s: f64, x: f64, y: f64) -> (i32, i32) {
    let k = s * 256.0;
    (cvtd(x * k), cvtd(y * k))
}

// the arrow head of pc_draw_rotate as a closed path
fn rot_head(g: &mut Gfx, s: f64) {
    g.text.path.begin(g.dc);
    let (x, y) = rc_fx(s, ROT_HEAD[0], ROT_HEAD[1]);
    g.text.path.move_fx(x, y);
    let (x, y) = rc_fx(s, ROT_HEAD[2], ROT_HEAD[3]);
    g.text.path.line_fx(x, y);
    let (x, y) = rc_fx(s, ROT_HEAD[4], ROT_HEAD[5]);
    g.text.path.line_fx(x, y);
    g.text.path.close();
}

// pc_draw_rotate(sf, dpi) -- three quarters of a circle around the hot spot with an arrow
// head, black over a white rim
fn pc_draw_rotate(g: &mut Gfx, dpi: i32) {
    let s = dpi as f64 / 96.0; // px per DIP
    let style = JOIN_ROUND | (CAP_ROUND << 4);
    // pass 0: white, 3.5 DIP wide; pass 1: black, 1.5 DIP
    for pass in 0..2 {
        g.dc.set_color(if pass == 0 { 0xFFFF_FFFF } else { 0xFF00_0000 });
        g.text.path.begin(g.dc);
        let (x, y) = rc_fx(s, ROT_ARC[0], ROT_ARC[1]);
        g.text.path.move_fx(x, y);
        for q in 0..3 {
            let a = &ROT_ARC[2 + q * 6..8 + q * 6];
            let k = s * 256.0;
            let v: [i32; 6] = core::array::from_fn(|i| cvtd(a[i] * k));
            g.text.path.cubic_fx(v[0], v[1], v[2], v[3], v[4], v[5]);
        }
        let w = cvtd(ROT_WIDTH[pass] * s * 256.0);
        g.text.path.stroke(&mut g.text.ras, g.dc, g.sf, w, style);
        // arrow head (a filled triangle; the rim pass draws it grown by its stroke)
        rot_head(g, s);
        if pass == 0 {
            let w = cvtd((ROT_WIDTH[1] + ROT_WIDTH[1]) * s * 256.0);
            g.text.path.stroke(&mut g.text.ras, g.dc, g.sf, w, style);
            rot_head(g, s); // fill the same triangle too
        }
        g.text.path.fill(&mut g.text.ras, g.dc, g.sf, RULE_NONZERO);
    }
}

/// pn_cursor_shot(path) -> 0 / ERR_* -- test hook: every cursor (tools, rotate) at 100, 150 and
/// 200 % in cells of SHOT_CELL px, the left half of each cell light grey, the right half dark,
/// into a PNG
pub(crate) fn pn_cursor_shot(p: &mut Pn, ui: &mut Ui, path: &str) -> i32 {
    let n = TOOL_COUNT + 1;
    let mut sf = Surface::new(n * SHOT_CELL, SHOT_ROWS * SHOT_CELL, FMT_PARGB);
    if sf.px.is_empty() {
        return gfx::ERR_NOMEM;
    }
    let mut dc = Dc::new(sf.w, sf.h);
    dc.clear(&mut sf, 0xFFD0_D0D0);
    for k in 0..n {
        dc.set_color(0xFF30_3030);
        dc.fill(&mut sf, k * SHOT_CELL + SHOT_CELL / 2, 0, SHOT_CELL / 2, SHOT_ROWS * SHOT_CELL);
    }
    for row in 0..SHOT_ROWS {
        let dpi = (row + 2) * 48; // dpi 96, 144, 192
        for k in 0..n {
            let kind = if k < TOOL_COUNT { k as u32 } else { PNC_ROTATE as u32 };
            let h = pn_cursor(p, ui, kind);
            if h == 0 {
                continue;
            }
            if let Some(c) = ui.cursor_render(h, dpi) {
                dc.blit(&mut sf, &c, k * SHOT_CELL + 4, row * SHOT_CELL + 4);
            }
        }
    }
    img_core::io::save(path, &sf)
}
