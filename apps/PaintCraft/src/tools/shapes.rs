// t_shapes -- PaintCraft: Linie / Kurve and Formen (M5, agent T4).  Docs: docs/SHAPES.md
//                                              [apps/PaintCraft/src/t_shapes.s]
//
// Both tools are LIVE (TF_LIVE): a drag draws, the result stays editable until Fertigstellen /
// Enter / another tool / image / document command (ONE history step "Linie / Kurve" /
// "Formen"); Esc / Strg+Z drop it (no step).  Everything is rendered through the tool layer
// (tl_*): the stroke of the centre line (gfx dash: width, Linienart, caps) and the fill (the
// shape's own rule) become coverage masks (raster_path; aliased: threshold 128), preview ==
// result, the selection clips.
//   Linie / Kurve: a drag draws a straight line (Umschalt: 15 degree steps, Alt: centred on
//     the press point) with 4 control points; dragging a point bends it (Spline-Pfad through
//     the points, right button = Bézierpfad: the inner points are Bézier controls); the
//     square handle after the end moves the line (right button: rotates it, Umschalt 15
//     degrees); a press elsewhere finishes it and starts a new one.  Left = primary colour,
//     right = secondary.
//   Formen: a drag draws the chosen shape into the rectangle (Umschalt: equal sides, Alt:
//     centred); then 8 handles resize it (Umschalt at a corner keeps the ratio, Alt about the
//     centre; dragging an edge over the opposite one mirrors), a drag inside moves it, a drag
//     just outside the frame (or a right drag on it) rotates it about its centre (Umschalt:
//     15 degree steps), Horizontal / Vertikal spiegeln mirror it.  Umriss = primary colour,
//     Füllung = secondary colour (Nur Füllung: primary; right button swaps).
//   Arrow keys move the live line / shape by 1 px (Strg: 10 px).
//
//   tshp_init()                      shape library + tool registration (main.rs mod_init)
//   tshp_cmd(id, arg) -> true handled   commands CMD_TSHP_FIRST..LAST (test hooks):
//       795 arg = sel << 16 | value: sel 0 Linientyp, 1 Linienart, 2 Linienanfang, 3 Linienende
//       (menu indices), 4 shape index, 5 Formfüllmodus, 6 corner radius, 7 mirror the live
//       shape (0 horizontal, 1 vertical), 8 load custom shapes (0: tests/fixtures/shapes of
//       the working directory, 1: <exe dir>/Shapes again, 2: the folder in the environment
//       variable PDN_SHAPES_DIR), 9 rotate the live line / shape by (i16) value degrees
//   Records (drawn with the tool options): #ID_TSHP_FIRST state (value = live), +1 custom
//       shape status (value = shapes), +2 current shape name (value = index),
//       +3 "dash,cap0,cap1,mode,radius,ctype"
//
// Rust model: the three coverage masks (stroke, fill, union) and the source surface of
// "Füllung und Umriss" live in TShapes; the one the tool layer shows is moved into the
// tool layer record (TlMask::Own / TlSrc::Own) at the end of ts_render and taken back at the
// start of the next one (tl_reclaim).

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use core::cell::Cell;

use gfx::dash::{dash_pattern, dash_stroke, DStyle, DASH_COUNT, LCAP_ARROW, LCAP_ARROW_FILLED, LCAP_FLAT, LCAP_ROUND};
use gfx::path::Path;
use gfx::pathdata::PData;
use gfx::raster::Raster;
use gfx::tmask::{TMask, TM_REPLACE};
use gfx::xform::{sincos, Matrix};
use gfx::{Surface, FMT_ARGB, JOIN_MITER, JOIN_ROUND, RULE_NONZERO};
use img_core::xform::{invert, mul, rotate};
use rt::fx::{cvtd, cvtt, cvttd, maxsd, minsd, minss, sqrtd};
use rt::x87::atan2;
use ui::*;

use crate::app::constants::*;
use crate::document::model::DocId;
use crate::workspace::nubs::{nub_set, nubs_damage, nubs_draw, nubs_hit, nubs_hover, Nub};
use crate::workspace::palette::pal_version;
use crate::document::selection::sel_line;
use crate::tools::shapes_library::{shl_count, shl_emit, shl_gallery, shl_init, shl_load_dir, shl_name, shl_radius, shl_rule, shl_status};
use crate::tools::*;
use crate::workspace::view::{view_damage_doc, view_doc_to_screen_f64, view_zoom, View};
use crate::app::Pn;

const OPT_CTYPE: u32 = ID_OPT_SHP0; // Linientyp (rows + 1 .. + 2)
const OPT_CAP0: u32 = ID_OPT_SHP0 + 3; // Linienanfang (rows + 4 .. + 7)
const OPT_DASH: u32 = ID_OPT_SHP0 + 8; // Linienart (rows + 9 .. + 13)
const OPT_CAP1: u32 = ID_OPT_SHP0 + 14; // Linienende (rows + 15 .. + 18)
const OPT_MODE: u32 = ID_OPT_SHP0 + 19; // Formfüllmodus (rows + 20 .. + 22)
const OPT_RADIUS: u32 = ID_OPT_SHP0 + 23; // Ecke Größe
const OPT_RDEC: u32 = ID_OPT_SHP0 + 24;
const OPT_RINC: u32 = ID_OPT_SHP0 + 25;
const OPT_FLIPH: u32 = ID_OPT_SHP0 + 26;
const OPT_FLIPV: u32 = ID_OPT_SHP0 + 27;
const REC_STATE: u32 = ID_TSHP_FIRST;
const REC_STATUS: u32 = ID_TSHP_FIRST + 1;
const REC_SHAPE: u32 = ID_TSHP_FIRST + 2;
const REC_OPTS: u32 = ID_TSHP_FIRST + 3;
const DR_NONE: u32 = 0;
const DR_DRAW: u32 = 1;
const DR_NUB: u32 = 2;
const DR_MOVE: u32 = 3;
const DR_SCALE: u32 = 4;
const DR_ROTATE: u32 = 5;
const NUB_N: usize = 8; // shape: 8 frame handles; line: 4 points + move handle
const MOVE_NUB: i32 = 4;
const MODE_OUTLINE: u32 = 0;
const MODE_INTERIOR: u32 = 1;
const MODE_BOTH: u32 = 2;
const CT_SPLINE: u32 = 0;
const CT_BEZIER: u32 = 1;
const RADIUS_MAX: i32 = 1000;
const ROT_DIP: i32 = 24; // rotation zone around the frame
const MOVE_DIP: i32 = 22; // distance of the line's move handle
const OB_PAD: i32 = 2; // image px around the overlay lines (damage)

const C_THIRD: f64 = 0.3333333333333333;
const C_INV256: f64 = 0.00390625;
const C_EPS: f64 = 1.0e-9;
const C_BIG: f64 = 1073741824.0;
const C_NBIG: f64 = -1073741824.0;
const C_STEP15: f64 = 0.2617993877991494; // 15 degrees
const C_DEG2RAD: f64 = 0.017453292519943295;
const C_RAD2DEG: f64 = 57.29577951308232;
const C_TIE: f64 = 0.015625;
const F_INV255: f32 = 0.003921568627;
const SIGN: u64 = 0x8000_0000_0000_0000;
const M_ZERO: Matrix = Matrix { a: 0.0, b: 0.0, c: 0.0, d: 0.0, e: 0.0, f: 0.0 };
const RANGE_RADIUS: Range = Range::new(0, RADIUS_MAX, 1, 0);

static NUB_FX: [f64; 8] = [0.0, 0.5, 1.0, 1.0, 1.0, 0.5, 0.0, 0.0];
static NUB_FY: [f64; 8] = [0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 0.5];
static NUB_EDGES: [u8; 8] = [3, 2, 6, 4, 12, 8, 9, 1];
static NUB_CURSORS: [u32; 8] = [CUR_SIZE_NWSE, CUR_SIZE_NS, CUR_SIZE_NESW, CUR_SIZE_WE, CUR_SIZE_NWSE, CUR_SIZE_NS, CUR_SIZE_NESW, CUR_SIZE_WE];
static CORNER_X: [usize; 4] = [0, 2, 2, 0];
static CORNER_Y: [usize; 4] = [1, 1, 3, 3];
/// Linienanfang / Linienende menu -> LCAP_*: Flach, Pfeil, Ausgefüllter Pfeil, Abgerundet
static CAP_MAP: [u32; 4] = [LCAP_FLAT, LCAP_ARROW, LCAP_ARROW_FILLED, LCAP_ROUND];
static CTYPE_LABELS: [&str; 2] = ["Kurvenzug (Spline)", "Bézierpfad"];
static CAP_LABELS: [&str; 4] = ["Flach", "Pfeil", "Ausgefüllter Pfeil", "Abgerundet"];
static DASH_LABELS: [&str; 5] = ["Durchgezogen", "Striche", "Punktiert", "Strich-Punkt", "Strich-Punkt-Punkt"];
static MODE_LABELS: [&str; 3] = ["Umriss zeichnen", "Nur Füllung", "Füllung und Umriss"];
static CTYPE_ICONS: [u32; 2] = [IC_CURVE, IC_INKING_TOOL];
static CAP0_ICONS: [u32; 4] = [IC_REMOVE, IC_BACK, IC_CARET_LEFT, IC_CIRCLE_RING];
static CAP1_ICONS: [u32; 4] = [IC_REMOVE, IC_FORWARD, IC_CARET_RIGHT, IC_CIRCLE_RING];
static MODE_ICONS: [u32; 3] = [IC_CIRCLE_RING, IC_CIRCLE_FILL, IC_RADIO_ON];

/// the drawn Linienart of the drop-down button (cmd_custom: ts_draw_dash)
pub(crate) struct DashIcon {
    dash: Cell<u32>,
    argb: Cell<u32>,
    dpi: Cell<i32>,
}

impl Draw for DashIcon {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        ts_draw_dash(g, self.dash.get(), self.argb.get(), self.dpi.get(), r);
    }
}

/// the state of t_shapes.s (its g_* globals)
pub(crate) struct TShapes {
    /// the line's centre line
    pd: PData,
    /// the path and rasterizer of the masks (the asm's current path)
    path: Path,
    ras: Raster,
    /// Füllung und Umriss: source pixels (kept while the size fits)
    src: Surface,
    /// TMASK: stroke, fill, union
    mask: [Option<TMask>; 3],
    /// the mask that lives in the tool layer record (TL_MASK) / the source is there
    in_tl: Option<usize>,
    src_in_tl: bool,
    nubs: [Nub; NUB_N],
    /// line: 4 points (f64 x, y) and their drag-start copy
    pts: [[f64; 2]; 4],
    pts0: [[f64; 2]; 4],
    /// shape: frame box (local, f64; any order) and its drag-start copy
    b: [f64; 4],
    b0: [f64; 4],
    /// shape: rigid transform local -> image, its inverse, the drag-start copies
    m: Matrix,
    mi: Matrix,
    m0: Matrix,
    mi0: Matrix,
    /// M + tie-break offset (render); line: identity + tie-break offset
    xm: Matrix,
    xl: Matrix,
    /// total rotation (radians), at the drag start; start angle of a rotation drag
    rot: f64,
    rot0: f64,
    a0: f64,
    /// rotation centre (image), pointer at the drag start / of the last drag step
    c0: [f64; 2],
    st: [f64; 2],
    lp: [f64; 2],
    doc: Option<DocId>,
    /// image box of the overlay lines, valid
    ob: [i32; 4],
    obv: bool,
    live: bool,
    /// tool of the session; current tool (tool_cur of the last call)
    tool: i32,
    curtool: i32,
    btn: u32,
    /// curve type of the live line; shape of the live session
    lctype: u32,
    sidx: i32,
    /// DR_*
    drag: u32,
    dnub: i32,
    dbtn: u32,
    lmods: u32,
    /// hover: 1 inside the frame, 2 rotate zone
    zone: u32,
    /// rendered with TC_VER / pal_version / options / selection
    cver: u32,
    palv: u32,
    oseen: u32,
    vsel: u32,
    /// bumped by every change of an option below
    over: u32,
    ochg: bool,
    /// Linientyp, Linienanfang, Linienende (menu indices), Linienart, Art der Form,
    /// Formfüllmodus, Ecke Größe (px)
    ctype: u32,
    cap0: u32,
    cap1: u32,
    dash: u32,
    shape: i32,
    mode: u32,
    radius: i32,
    dash_icon: Option<Rc<DashIcon>>,
}

impl Default for TShapes {
    fn default() -> TShapes {
        TShapes {
            pd: PData::new(),
            path: Path::default(),
            ras: Raster::default(),
            src: Surface::default(),
            mask: [None, None, None],
            in_tl: None,
            src_in_tl: false,
            nubs: [Nub::default(); NUB_N],
            pts: [[0.0; 2]; 4],
            pts0: [[0.0; 2]; 4],
            b: [0.0; 4],
            b0: [0.0; 4],
            m: M_ZERO, // (.bss)
            mi: M_ZERO,
            m0: M_ZERO,
            mi0: M_ZERO,
            xm: M_ZERO,
            xl: M_ZERO,
            rot: 0.0,
            rot0: 0.0,
            a0: 0.0,
            c0: [0.0; 2],
            st: [0.0; 2],
            lp: [0.0; 2],
            doc: None,
            ob: [0; 4],
            obv: false,
            live: false,
            tool: 0,
            curtool: 0,
            btn: 0,
            lctype: 0,
            sidx: 0,
            drag: DR_NONE,
            dnub: 0,
            dbtn: 0,
            lmods: 0,
            zone: 0,
            cver: 0,
            palv: 0,
            oseen: 0,
            vsel: 0,
            over: 0,
            ochg: false,
            ctype: 0,
            cap0: 0,
            cap1: 0,
            dash: 0,
            shape: 0,
            mode: 0,
            radius: 20, // Ecke Größe (px)
            dash_icon: None,
        }
    }
}

static DESC_LINE: ToolDesc = ToolDesc {
    flags: (TF_LIVE | TF_CLIP_SEL | TF_USES_WIDTH | TF_AA | TF_BLEND | TF_FILL) as u32,
    activate: Some(ts_activate),
    deactivate: Some(ts_activate),
    input: Some(ts_input),
    options: Some(ts_options),
    key: Some(ts_key),
    commit: Some(ts_commit),
    cancel: Some(ts_cancel),
    cursor: Some(ts_cursor),
    overlay: Some(ts_overlay),
    tick: Some(ts_tick),
    ..ToolDesc::NONE
};

static DESC_SHAPES: ToolDesc = ToolDesc {
    flags: (TF_LIVE | TF_CLIP_SEL | TF_USES_WIDTH | TF_AA | TF_BLEND | TF_FILL) as u32,
    activate: Some(ts_activate),
    deactivate: Some(ts_activate),
    input: Some(ts_input),
    options: Some(ts_options),
    key: Some(ts_key),
    commit: Some(ts_commit),
    cancel: Some(ts_cancel),
    cursor: Some(ts_cursor),
    overlay: Some(ts_overlay),
    tick: Some(ts_tick),
    ..ToolDesc::NONE
};

/// tshp_init() -- the shape library, both tools
pub(crate) fn tshp_init(p: &mut Pn, ui: &mut Ui) {
    shl_init(p);
    tool_register(&mut p.tool, TOOL_LINE, Some(&DESC_LINE));
    tool_register(&mut p.tool, TOOL_SHAPES, Some(&DESC_SHAPES));
    ts_nubs_hide_all(&p.view, ui, &mut p.tshp.nubs);
}

/// tshp_cmd(id, arg) -> 1 handled
pub(crate) fn tshp_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    if id != CMD_TSHP_FIRST {
        return false;
    }
    let sel = arg >> 16;
    let v = arg as u16 as i16 as i32; // value (i16)
    let t = &mut p.tshp;
    match sel {
        0 => {
            t.ctype = (v & 1) as u32;
            if t.live {
                t.lctype = t.ctype;
            }
            t.over = t.over.wrapping_add(1);
        }
        1 => {
            if (v as u32) < DASH_COUNT {
                t.dash = v as u32;
                t.over = t.over.wrapping_add(1);
            }
        }
        2 => {
            t.cap0 = (v & 3) as u32;
            t.over = t.over.wrapping_add(1);
        }
        3 => {
            t.cap1 = (v & 3) as u32;
            t.over = t.over.wrapping_add(1);
        }
        4 => {
            if (v as u32) < shl_count(&p.shl) as u32 {
                ts_set_shape(&mut p.tshp, v);
            }
        }
        5 => {
            if (v as u32) <= 2 {
                t.mode = v as u32;
                t.over = t.over.wrapping_add(1);
            }
        }
        6 => {
            if (0..=RADIUS_MAX).contains(&v) {
                t.radius = v;
                t.over = t.over.wrapping_add(1);
            }
        }
        7 => ts_flip(p, ui, v & 1 != 0),
        8 => {
            if v == 2 {
                let Some(dir) = plat::getenv("PDN_SHAPES_DIR") else { return true };
                shl_load_dir(p, &dir);
            } else if v == 0 {
                shl_load_dir(p, "apps/PaintCraft/tests/fixtures/shapes");
            } else {
                shl_init(p);
            }
            if p.tshp.shape as u32 >= shl_count(&p.shl) as u32 {
                p.tshp.shape = 0;
            }
        }
        9 => ts_rotate_by(p, ui, v as f64 * C_DEG2RAD),
        _ => {}
    }
    true
}

// ------------------------------------------------------------------ small helpers

// ts_round(x) -> nearest integer (clamped to +-2^30)
fn ts_round(x: f64) -> i32 {
    cvtd(minsd(maxsd(x, C_NBIG), C_BIG))
}

// ts_floor(x) -> floor (clamped)
fn ts_floor(x: f64) -> i32 {
    let x = minsd(maxsd(x, C_NBIG), C_BIG);
    let t = cvttd(x);
    if (t as f64) > x { t - 1 } else { t }
}

#[inline]
fn fabs(v: f64) -> f64 {
    f64::from_bits(v.to_bits() & !SIGN)
}

// (v & sign) | s
#[inline]
fn with_sign(s: f64, v: f64) -> f64 {
    f64::from_bits((v.to_bits() & SIGN) | s.to_bits())
}

// ts_is_line() -> 1 while the session (or the current tool) is Linie / Kurve
fn ts_is_line(t: &TShapes) -> bool {
    t.tool == TOOL_LINE
}

// ts_img_per_dip(dip) -> image px of dip DIP at the current zoom
fn ts_img_per_dip(p: &Pn, ui: &Ui, dip: i32) -> f64 {
    let d = ui.dp(dip);
    let z = (view_zoom(p) as i32).max(1);
    d as f64 * 10000.0 / z as f64
}

// ts_set_shape(i) -- the shape option (and the live shape)
fn ts_set_shape(t: &mut TShapes, i: i32) {
    t.shape = i;
    if t.live && t.tool == TOOL_SHAPES {
        t.sidx = i;
    }
    t.over = t.over.wrapping_add(1);
}

// ts_nubs_hide_all(nubs) -- every nub hidden (damaged where it was drawn)
fn ts_nubs_hide_all(v: &View, ui: &mut Ui, nubs: &mut [Nub; NUB_N]) {
    for n in nubs.iter_mut() {
        n.flags |= NBF_HIDDEN as u32;
        n.flags &= !((NBF_HOVER | NBF_DRAG) as u32);
    }
    nubs_damage(v, ui, nubs);
}

// ts_nub_at(i, x, y, kind) -- nub i visible at the image point
fn ts_nub_at(v: &View, ui: &mut Ui, nubs: &mut [Nub; NUB_N], i: usize, x: f64, y: f64, kind: i32) {
    let n = &mut nubs[i & (NUB_N - 1)];
    n.kind = kind as u32;
    n.flags &= !(NBF_HIDDEN as u32);
    let a = ts_round(x * 256.0);
    let b = ts_round(y * 256.0);
    nub_set(v, ui, n, a, b);
}

// ------------------------------------------------------------------ geometry

// ts_minv() -- mi = M^-1 (M is rigid: always invertible)
fn ts_minv(t: &mut TShapes) {
    if let Some(i) = invert(&t.m) {
        t.mi = i;
    }
}

// ts_line_pd() -- pd = the centre line from the 4 points (spline through them / Bézier)
fn ts_line_pd(t: &mut TShapes) {
    t.pd.clear();
    let q = t.pts;
    t.pd.move_to(q[0][0], q[0][1]);
    if t.lctype == CT_BEZIER {
        t.pd.cubic(&[q[1][0], q[1][1], q[2][0], q[2][1], q[3][0], q[3][1]]);
        return;
    }
    // Catmull-Rom: b1 = P(i) + (P(i+1) - P(i-1)) / 6, b2 = P(i+1) - (P(i+2) - P(i)) / 6
    for i in 0..3usize {
        let i0 = i.saturating_sub(1);
        let i3 = (i + 2).min(3);
        let mut c = [0.0f64; 6];
        for k in 0..2 {
            c[k] = (q[i + 1][k] - q[i0][k]) / 6.0 + q[i][k];
            c[2 + k] = q[i + 1][k] - (q[i3][k] - q[i][k]) / 6.0;
            c[4 + k] = q[i + 1][k];
        }
        t.pd.cubic(&c);
    }
}

// ts_xforms(aa) -- xl (line) / xm (shape: M) with the tie-break offset: aliased output samples
// the pixel centres, so outlines lying exactly on a pixel centre are moved by 1/64 px to +x /
// +y (a top-left rule: exactly w pixels)
fn ts_xforms(t: &mut TShapes, aa: bool) {
    t.xl = Matrix::IDENTITY;
    t.xm = t.m;
    if !aa {
        t.xl.c = C_TIE;
        t.xl.f = C_TIE;
        t.xm.c = t.xm.c + C_TIE;
        t.xm.f = t.xm.f + C_TIE;
    }
}

// tl_reclaim() -- the mask and the source shown by the tool layer back into TShapes
fn tl_reclaim(p: &mut Pn) {
    let tl = &mut p.tool.tls[TL_SHAPES];
    let t = &mut p.tshp;
    if let Some(i) = t.in_tl.take() {
        if let TlMask::Own(m) = core::mem::take(&mut tl.mask) {
            t.mask[i] = Some(m);
        }
    }
    if t.src_in_tl {
        t.src_in_tl = false;
        if let TlSrc::Own(s) = core::mem::take(&mut tl.src) {
            t.src = s;
        }
    }
}

// ts_masks(doc) -> 1 when the three masks exist with the document's size
fn ts_masks(p: &mut Pn, doc: DocId) -> bool {
    let (w, h) = (p.doc.d(doc).w, p.doc.d(doc).h);
    for m in p.tshp.mask.iter_mut() {
        if m.as_ref().is_some_and(|m| m.w == w as u32 && m.h == h as u32) {
            continue;
        }
        *m = TMask::new(w as u32, h as u32);
        if m.is_none() {
            return false;
        }
    }
    true
}

// ts_masks_free()
fn ts_masks_free(p: &mut Pn) {
    tl_reclaim(p);
    let t = &mut p.tshp;
    t.mask = [None, None, None];
    t.src = Surface::default();
    let tl = &mut p.tool.tls[TL_SHAPES];
    tl.mask = TlMask::None;
    tl.src = TlSrc::None;
}

// ts_style() -- width, Linienart, caps, join of the session
fn ts_style(p: &Pn) -> DStyle<'static> {
    let t = &p.tshp;
    let mut st = DStyle { width: p.tool.cfg.width, pat: dash_pattern(t.dash), join: 0, cap0: 0, cap1: 0, dcap: 0, offset: 0.0, miter: 0.0 };
    if ts_is_line(t) {
        st.join = JOIN_ROUND;
        st.cap0 = CAP_MAP[(t.cap0 & 3) as usize];
        st.cap1 = CAP_MAP[(t.cap1 & 3) as usize];
    } else {
        st.join = JOIN_MITER;
        st.miter = 10.0;
    }
    st
}

// ts_mode() -> MODE_* of the session (the line: outline)
fn ts_mode(t: &TShapes) -> u32 {
    if t.tool == TOOL_LINE { MODE_OUTLINE } else { t.mode }
}

// ts_shape_path() -- path += the live shape (B, xm = M + the tie-break offset)
fn ts_shape_path(s: &crate::tools::shapes_library::Shl, t: &mut TShapes) {
    let r = t.radius as f64;
    shl_emit(s, t.sidx, &t.b, &t.xm, r, &mut t.path);
}

// ts_aa_box(k, box) -- aliased: threshold 128; box grows by the mask's bounds
fn ts_aa_box(t: &mut TShapes, k: usize, aa: bool, bx: &mut [i32; 4]) {
    let Some(m) = t.mask[k].as_mut() else { return };
    if !aa {
        m.threshold(128);
    }
    if let Some(r) = m.bbox() {
        bx[0] = bx[0].min(r[0]);
        bx[1] = bx[1].min(r[1]);
        bx[2] = bx[2].max(r[2]);
        bx[3] = bx[3].max(r[3]);
    }
}

// ts_render() -- masks, box and tool layer from the current geometry and options
fn ts_render(p: &mut Pn) {
    if !p.tshp.live {
        return;
    }
    let Some(doc) = p.tshp.doc else { return };
    tl_reclaim(p);
    if !ts_masks(p, doc) {
        return;
    }
    let aa = p.tool.cfg.aa != 0;
    ts_xforms(&mut p.tshp, aa);
    let mode = ts_mode(&p.tshp);
    // empty box
    let mut bx = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
    // ---- the stroke (outline)
    if mode != MODE_INTERIOR {
        let st = ts_style(p);
        let Pn { tshp: t, shl, .. } = p;
        t.path.reset();
        if ts_is_line(t) {
            ts_line_pd(t);
            let xl = t.xl;
            t.pd.emit(&mut t.path, Some(&xl));
        } else {
            ts_shape_path(shl, t);
        }
        dash_stroke(&mut t.path, &st);
        if let Some(m) = t.mask[0].as_mut() {
            m.raster_path(&t.path, &mut t.ras, RULE_NONZERO, TM_REPLACE);
        }
        ts_aa_box(t, 0, aa, &mut bx);
    }
    if mode != MODE_OUTLINE {
        let Pn { tshp: t, shl, .. } = p;
        t.path.reset();
        ts_shape_path(shl, t);
        let rule = shl_rule(shl, t.sidx);
        if let Some(m) = t.mask[1].as_mut() {
            m.raster_path(&t.path, &mut t.ras, rule, TM_REPLACE);
        }
        ts_aa_box(t, 1, aa, &mut bx);
    }
    // the tool layer: paint of the button, mask / source by the mode
    let btn = p.tshp.btn;
    tl_set_paint(p, TL_SHAPES, btn);
    match mode {
        MODE_BOTH => ts_build_both(p, &mut bx),
        _ => {
            // (Nur Füllung with the right button: the secondary colour, tl_set_paint)
            let k = if mode == MODE_OUTLINE { 0 } else { 1 };
            if let Some(m) = p.tshp.mask[k].take() {
                p.tool.tls[TL_SHAPES].mask = TlMask::Own(m);
                p.tshp.in_tl = Some(k);
            }
        }
    }
    // an empty result: an empty box
    if bx[0] >= bx[2] {
        bx = [0; 4];
    }
    tl_set_box(p, TL_SHAPES, bx[0], bx[1], bx[2], bx[3]);
}

// ts_build_both(&box) -- Füllung und Umriss: source S (straight ARGB over the box) and
// union coverage U so that blending S through U equals "outline over fill":
//   o = a * Ao, f = b * Af (a, b coverages of stroke / fill, A* colour alphas),
//   L = o + f (1 - o), U = a + b - a b, S.a = L / U, S.rgb = (Co o + Cf f (1 - o)) / L
// The outline may be a hatch pattern (TC_FILL: fg = outline colour, bg = fill colour).
fn ts_build_both(p: &mut Pn, bx: &mut [i32; 4]) {
    let co = tool_color(p, p.tshp.btn);
    let cf = tool_color(p, if p.tshp.btn == BTN_RIGHT { BTN_LEFT } else { BTN_RIGHT });
    let fs = p.tool.cfg.fill;
    let ok = {
        let t = &mut p.tshp;
        let [Some(m0), Some(m1), Some(mu)] = &mut t.mask else { return };
        mu.clear(0);
        build_both(m0, m1, mu, &mut t.src, bx, co, cf, fs)
    };
    let t = &mut p.tshp;
    if let Some(m) = t.mask[2].take() {
        p.tool.tls[TL_SHAPES].mask = TlMask::Own(m);
        t.in_tl = Some(2);
    }
    let tl = &mut p.tool.tls[TL_SHAPES];
    tl.kind = TLK_SOURCE as u32;
    match ok {
        Some(true) => {
            tl.src = TlSrc::Own(core::mem::take(&mut t.src));
            t.src_in_tl = true;
            tl.sx = bx[0];
            tl.sy = bx[1];
        }
        Some(false) => {}
        None => {
            // no memory: nothing is shown
            if let TlMask::Own(m) = &mut tl.mask {
                m.clear(0);
            }
            *bx = [0; 4];
        }
    }
}

// the pixels of ts_build_both -> Some(true) source written, Some(false) empty box, None = out
// of memory
#[allow(clippy::too_many_arguments)]
fn build_both(m0: &TMask, m1: &TMask, mu: &mut TMask, src: &mut Surface, bx: &[i32; 4], co: u32, cf: u32, fs: u32) -> Option<bool> {
    let w = bx[2].wrapping_sub(bx[0]);
    if w <= 0 {
        return Some(false);
    }
    let h = bx[3].wrapping_sub(bx[1]);
    if h <= 0 {
        return Some(false);
    }
    // the source surface (kept while the size fits)
    if w != src.w || h != src.h || src.px.is_empty() {
        if w as u32 > 65536 || h as u32 > 65536 {
            *src = Surface::default();
            return None;
        }
        *src = Surface::new(w, h, FMT_ARGB);
    }
    let ss = src.stride as usize;
    let af = (cf >> 24) as f32 * F_INV255;
    let mut ba = [0u8; 64];
    let mut bb = [0u8; 64];
    let mut pat = [0u8; 256];
    for row in 0..h {
        let y = bx[1] + row;
        let mut col = 0;
        while col < w {
            // n = up to the next tile column (64 aligned in image x), <= the rest of the row
            let x = bx[0] + col;
            let n = (64 - (x & 63)).min(w - col);
            let nu = n as usize;
            let (ra, _) = m0.row(x, y, n, &mut ba);
            let (rb, _) = m1.row(x, y, n, &mut bb);
            fill_pattern_row(&mut pat, n, x, y, fs, co, cf);
            let u = mu.row_w(x, y)?;
            if u.len() < nu {
                return None;
            }
            let so = row as usize * ss + col as usize * 4;
            let srow = &mut src.px[so..so + nu * 4];
            for i in 0..nu {
                let a = ra[i] as f32 * F_INV255;
                let b = rb[i] as f32 * F_INV255;
                // U = a + b - a b
                let uu = a + b - a * b;
                let uv = cvtt(uu * 255.0 + 0.5);
                u[i] = uv as u8;
                let mut s = 0u32;
                if uv != 0 {
                    // o = a Ao, f = b Af, L = o + f (1 - o)
                    let oc = u32::from_le_bytes([pat[i * 4], pat[i * 4 + 1], pat[i * 4 + 2], pat[i * 4 + 3]]);
                    let ao = (oc >> 24) as f32 * F_INV255;
                    let o = a * ao;
                    let f = b * af;
                    let f = f * (1.0 - o);
                    let l = o + f;
                    if l > 1.0e-6 {
                        let sa = minss(l / uu, 1.0) * 255.0 + 0.5;
                        s = (cvtt(sa) as u32) << 24;
                        // channels: (Co o + Cf f (1 - o)) / L
                        for sh in [0u32, 8, 16] {
                            let c1 = ((oc >> sh) & 255) as f32 * o;
                            let c2 = ((cf >> sh) & 255) as f32 * f;
                            let v = minss((c1 + c2) / l, 255.0) + 0.5;
                            s |= (cvtt(v) as u32) << sh;
                        }
                    }
                }
                srow[i * 4..i * 4 + 4].copy_from_slice(&s.to_le_bytes());
            }
            col += n;
        }
    }
    Some(true)
}

// ------------------------------------------------------------------ handles / overlay

// ts_ov_point(i) -> overlay point i (0..3): the frame corners M(B) of a shape, the control
// points of a line
fn ts_ov_point(t: &TShapes, i: usize) -> [f64; 2] {
    let i = i & 3;
    if ts_is_line(t) {
        return t.pts[i];
    }
    let (x, y) = t.m.apply(t.b[CORNER_X[i]], t.b[CORNER_Y[i]]);
    [x, y]
}

// ts_ob_update() -- ob = image box of the overlay lines (frame / Bézier arms) + margin
fn ts_ob_update(t: &mut TShapes) {
    let mut bx = [C_BIG, C_BIG, C_NBIG, C_NBIG];
    for i in 0..4 {
        let q = ts_ov_point(t, i);
        bx[0] = minsd(q[0], bx[0]);
        bx[1] = minsd(q[1], bx[1]);
        bx[2] = maxsd(q[0], bx[2]);
        bx[3] = maxsd(q[1], bx[3]);
    }
    t.ob = [ts_floor(bx[0]) - OB_PAD, ts_floor(bx[1]) - OB_PAD, ts_floor(bx[2]) + OB_PAD + 1, ts_floor(bx[3]) + OB_PAD + 1];
}

// ts_damage() -- redraw the overlay lines and the handles (where they were and are)
fn ts_damage(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.tshp;
    if t.obv {
        if let Some(d) = t.doc {
            view_damage_doc(&p.view, ui, d, t.ob[0], t.ob[1], t.ob[2], t.ob[3]);
        }
    }
    nubs_damage(&p.view, ui, &mut t.nubs);
}

// ts_dir(a, b) -> unit(b - a), (1, 0) for a = b
fn ts_dir(a: &[f64; 2], b: &[f64; 2]) -> [f64; 2] {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let l = sqrtd(dx * dx + dy * dy);
    if l > C_EPS {
        [dx / l, dy / l]
    } else {
        [1.0, 0.0]
    }
}

// ts_nubs_update() -- handles and overlay box from the geometry
fn ts_nubs_update(p: &mut Pn, ui: &mut Ui) {
    if ts_is_line(&p.tshp) {
        // the 4 control points
        let t = &mut p.tshp;
        for i in 0..4 {
            let q = t.pts[i];
            ts_nub_at(&p.view, ui, &mut t.nubs, i, q[0], q[1], NK_CIRCLE);
        }
        // the move handle: after the end, along P0 -> P3
        let d = ts_dir(&p.tshp.pts[0], &p.tshp.pts[3]);
        let k = ts_img_per_dip(p, ui, MOVE_DIP);
        let t = &mut p.tshp;
        let x = d[0] * k + t.pts[3][0];
        let y = d[1] * k + t.pts[3][1];
        ts_nub_at(&p.view, ui, &mut t.nubs, MOVE_NUB as usize, x, y, NK_SQUARE);
        // nubs 5..7 unused
        for n in t.nubs[MOVE_NUB as usize + 1..].iter_mut() {
            n.flags |= NBF_HIDDEN as u32;
        }
    } else {
        let t = &mut p.tshp;
        let b = t.b;
        for i in 0..NF_COUNT as usize {
            let x = (b[2] - b[0]) * NUB_FX[i] + b[0];
            let y = (b[3] - b[1]) * NUB_FY[i] + b[1];
            let (x, y) = t.m.apply(x, y);
            ts_nub_at(&p.view, ui, &mut t.nubs, i, x, y, NK_SQUARE);
        }
    }
    ts_ob_update(&mut p.tshp);
    p.tshp.obv = true;
}

// ts_changed() -- the geometry changed: handles, overlay, render
fn ts_changed(p: &mut Pn, ui: &mut Ui) {
    ts_damage(p, ui);
    ts_nubs_update(p, ui);
    ts_damage(p, ui);
    ts_render(p);
}

// ts_overlay(doc, &rect) -- the frame of a shape / the Bézier arms of a line + the handles
fn ts_overlay(p: &mut Pn, g: &mut Gfx, doc: DocId, _rect: [i32; 4]) {
    let t = &mut p.tshp;
    if !t.live || t.doc != Some(doc) || t.drag == DR_DRAW {
        return; // (nothing while the first drag draws)
    }
    let mut c = [[0i32; 2]; 4];
    for (i, ci) in c.iter_mut().enumerate() {
        let mut q = ts_ov_point(t, i);
        view_doc_to_screen_f64(&p.view, &mut q);
        *ci = [ts_floor(q[0]), ts_floor(q[1])];
    }
    if ts_is_line(t) {
        if t.lctype == CT_BEZIER {
            // Bézier: P0 - P1 and P3 - P2
            sel_line(g, c[0][0], c[0][1], c[1][0], c[1][1], 0xFF80_8080);
            sel_line(g, c[3][0], c[3][1], c[2][0], c[2][1], 0xFF80_8080);
        }
    } else {
        for i in 0..4 {
            let j = (i + 1) & 3;
            sel_line(g, c[i][0], c[i][1], c[j][0], c[j][1], 0xFF50_5050);
        }
    }
    nubs_draw(&p.view, g, &mut t.nubs);
}

// ------------------------------------------------------------------ session

// ts_begin(doc, btn, tool) -- a new live line / shape (not yet rendered)
fn ts_begin(p: &mut Pn, doc: DocId, btn: u32, tool: i32) {
    let t = &mut p.tshp;
    t.tool = tool;
    t.doc = Some(doc);
    t.btn = btn;
    t.live = true;
    t.obv = false;
    t.lctype = t.ctype;
    t.sidx = t.shape;
    t.m = Matrix::IDENTITY;
    t.mi = Matrix::IDENTITY;
    t.rot = 0.0;
    t.in_tl = None;
    t.src_in_tl = false;
    tl_init(p, TL_SHAPES, doc);
    tl_set_paint(p, TL_SHAPES, btn);
    tl_show(p, TL_SHAPES);
    ts_versions(p);
    tool_live_begin(p, doc);
}

// ts_versions() -- remember the option / colour / selection state that was rendered
fn ts_versions(p: &mut Pn) {
    let t = &mut p.tshp;
    t.cver = p.tool.cfg.ver;
    t.palv = pal_version(&p.pal);
    t.oseen = t.over;
    if let Some(d) = t.doc {
        t.vsel = p.doc.d(d).ver_sel;
    }
}

// ts_end() -- the session is over: handles hidden, masks freed
fn ts_end(p: &mut Pn, ui: &mut Ui) {
    ts_damage(p, ui);
    let t = &mut p.tshp;
    t.live = false;
    t.drag = DR_NONE;
    t.obv = false;
    ts_nubs_hide_all(&p.view, ui, &mut t.nubs);
    ts_masks_free(p);
    p.tool.tls[TL_SHAPES].shown = false;
}

// ts_commit(doc) -> 1 step -- the line / shape into the layer (one history step)
fn ts_commit(p: &mut Pn, ui: &mut Ui, _doc: DocId) -> bool {
    if !p.tshp.live {
        return false;
    }
    let tl = &p.tool.tls[TL_SHAPES];
    let r = if tl.bx[0] >= tl.bx[2] {
        // nothing drawn: no step
        tl_hide(p, TL_SHAPES);
        false
    } else if p.tshp.tool == TOOL_LINE {
        tl_commit(p, TL_SHAPES, "Linie / Kurve", IC_CURVE)
    } else {
        tl_commit(p, TL_SHAPES, "Formen", IC_SHAPES)
    };
    ts_end(p, ui);
    r
}

// ts_cancel(doc) -- drop it (no history; doc None: the image is gone)
fn ts_cancel(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    if !p.tshp.live {
        return;
    }
    if doc.is_none() {
        // the image is gone: nothing of it may be damaged any more
        p.tshp.obv = false;
        p.tshp.doc = None;
    } else {
        tl_hide(p, TL_SHAPES);
    }
    ts_end(p, ui);
}

// ts_session_ok(doc) -> 1 when a live session of this tool on doc exists
fn ts_session_ok(t: &TShapes, doc: DocId) -> bool {
    t.live && t.doc == Some(doc) && t.tool == t.curtool
}

// ------------------------------------------------------------------ transforms

// ts_translate(dx, dy) -- move the live line / shape
fn ts_translate(p: &mut Pn, ui: &mut Ui, dx: f64, dy: f64) {
    let t = &mut p.tshp;
    if ts_is_line(t) {
        for q in t.pts.iter_mut() {
            q[0] = q[0] + dx;
            q[1] = q[1] + dy;
        }
    } else {
        t.m.c = dx + t.m.c;
        t.m.f = dy + t.m.f;
        ts_minv(t);
    }
    ts_changed(p, ui);
}

// ts_center() -> rotation centre: the centroid of the line's points / the frame centre
fn ts_center(t: &TShapes) -> [f64; 2] {
    if ts_is_line(t) {
        let q = &t.pts;
        return [(q[0][0] + q[1][0] + q[2][0] + q[3][0]) * 0.25, (q[0][1] + q[1][1] + q[2][1] + q[3][1]) * 0.25];
    }
    let (x, y) = t.m.apply((t.b[0] + t.b[2]) * 0.5, (t.b[1] + t.b[3]) * 0.5);
    [x, y]
}

// ts_rotate_from(angle, center) -- the drag-start state rotated by angle about center:
// line: pts = R pts0, shape: M = R M0
fn ts_rotate_from(t: &mut TShapes, angle: f64, c: [f64; 2]) {
    let mut r = Matrix::IDENTITY;
    rotate(&mut r, angle, c[0], c[1]);
    if ts_is_line(t) {
        t.pts = t.pts0;
        for q in t.pts.iter_mut() {
            let (x, y) = r.apply(q[0], q[1]);
            *q = [x, y];
        }
    } else {
        t.m = mul(&r, &t.m0);
        ts_minv(t);
    }
}

// ts_rotate_by(angle) -- test hook: rotate the live line / shape about its centre
fn ts_rotate_by(p: &mut Pn, ui: &mut Ui, angle: f64) {
    let t = &mut p.tshp;
    if !t.live {
        return;
    }
    ts_save0(t);
    let c = ts_center(t);
    t.rot = angle + t.rot;
    ts_rotate_from(t, angle, c);
    ts_changed(p, ui);
}

// ts_save0() -- the drag-start copies of the geometry
fn ts_save0(t: &mut TShapes) {
    t.pts0 = t.pts;
    t.b0 = t.b;
    t.m0 = t.m;
    t.mi0 = t.mi;
    t.rot0 = t.rot;
}

// ts_flip(vertical) -- mirror the live shape inside its frame (swap the box sides)
fn ts_flip(p: &mut Pn, ui: &mut Ui, vertical: bool) {
    let t = &mut p.tshp;
    if !t.live || t.tool != TOOL_SHAPES {
        return;
    }
    let k = vertical as usize;
    t.b.swap(k, k + 2);
    ts_changed(p, ui);
}

// ts_drag_to(x, y, mods) -- the drag follows the pointer (image px)
fn ts_drag_to(p: &mut Pn, ui: &mut Ui, x: f64, y: f64, mods: u32) {
    let t = &mut p.tshp;
    match t.drag {
        DR_DRAW => {
            if ts_is_line(t) {
                // line: P3 = pointer (Umschalt: 15 degree steps), Alt: P0 mirrors it about the
                // start
                let mut dx = x - t.st[0];
                let mut dy = y - t.st[1];
                if mods & MOD_SHIFT != 0 {
                    let len = sqrtd(dx * dx + dy * dy);
                    let a = atan2(dy, dx);
                    let a = ts_round(a / C_STEP15) as f64 * C_STEP15;
                    let (s, c) = sincos(a);
                    dx = c * len;
                    dy = s * len;
                }
                // d = (dx, dy); P3 = S + d, P0 = S (Alt: S - d)
                let (mut sx, mut sy) = (t.st[0], t.st[1]);
                t.pts[3] = [sx + dx, sy + dy];
                if mods & MOD_ALT != 0 {
                    sx = sx - dx;
                    sy = sy - dy;
                }
                t.pts[0] = [sx, sy];
                // P1, P2 at 1/3 and 2/3
                let ex = (t.pts[3][0] - t.pts[0][0]) * C_THIRD;
                let ey = (t.pts[3][1] - t.pts[0][1]) * C_THIRD;
                let x1 = t.pts[0][0] + ex;
                let y1 = t.pts[0][1] + ey;
                t.pts[1] = [x1, y1];
                t.pts[2] = [x1 + ex, y1 + ey];
            } else {
                // whole pixels: q = floor(pointer), s = floor(start) (the corners of the pixels)
                let qx = ts_floor(x) as f64;
                let qy = ts_floor(y) as f64;
                let mut sx = ts_floor(t.st[0]) as f64;
                let mut sy = ts_floor(t.st[1]) as f64;
                let mut w = qx - sx;
                let mut h = qy - sy;
                if mods & MOD_SHIFT != 0 {
                    // equal sides: m = max(|w|, |h|), signs kept
                    let m = maxsd(fabs(w), fabs(h));
                    w = with_sign(m, w);
                    h = with_sign(m, h);
                }
                t.b[2] = sx + w;
                t.b[3] = sy + h;
                if mods & MOD_ALT != 0 {
                    sx = sx - w;
                    sy = sy - h;
                }
                t.b[0] = sx;
                t.b[1] = sy;
            }
            ts_changed(p, ui);
        }
        DR_NUB => {
            t.pts[(t.dnub & 3) as usize] = [x, y];
            ts_changed(p, ui);
        }
        DR_MOVE => {
            // from the drag start: line pts0 + d, shape M = T(round(d)) M0
            let mut dx = x - t.st[0];
            let mut dy = y - t.st[1];
            if !ts_is_line(t) {
                dx = ts_round(dx) as f64;
                dy = ts_round(dy) as f64;
                t.m = t.m0;
            } else {
                t.pts = t.pts0;
            }
            ts_translate(p, ui, dx, dy);
        }
        DR_SCALE => {
            // the pointer in the frame's own coordinates (pixel corners without rotation)
            let (mut x, mut y) = (x, y);
            if !(t.m0.b < 0.0 || t.m0.b > 0.0) {
                x = ts_floor(x) as f64;
                y = ts_floor(y) as f64;
            }
            let (px, py) = t.mi0.apply(x, y);
            let b0 = t.b0;
            let mut r = b0;
            let e = NUB_EDGES[(t.dnub & (NF_COUNT - 1)) as usize]; // 1 left, 2 top, 4 right, 8 bottom
            if e & 1 != 0 {
                r[0] = px;
            }
            if e & 2 != 0 {
                r[1] = py;
            }
            if e & 4 != 0 {
                r[2] = px;
            }
            if e & 8 != 0 {
                r[3] = py;
            }
            // Alt: the opposite side mirrors the moved one about the centre
            if mods & MOD_ALT != 0 {
                let cx2 = b0[0] + b0[2];
                let cy2 = b0[1] + b0[3];
                if e & 1 != 0 {
                    r[2] = cx2 - r[0];
                }
                if e & 4 != 0 {
                    r[0] = cx2 - r[2];
                }
                if e & 2 != 0 {
                    r[3] = cy2 - r[1];
                }
                if e & 8 != 0 {
                    r[1] = cy2 - r[3];
                }
            }
            // Umschalt at a corner: keep the ratio of B0 (the larger factor, signs kept)
            if mods & MOD_SHIFT != 0 && matches!(e, 3 | 6 | 12 | 9) {
                let w0 = b0[2] - b0[0];
                let h0 = b0[3] - b0[1];
                if (w0 < 0.0 || w0 > 0.0) && (h0 < 0.0 || h0 > 0.0) {
                    let sx = (r[2] - r[0]) / w0;
                    let sy = (r[3] - r[1]) / h0;
                    let s = maxsd(fabs(sx), fabs(sy));
                    let w2 = w0 * with_sign(s, sx);
                    let h2 = h0 * with_sign(s, sy);
                    if e & 1 != 0 {
                        r[0] = r[2] - w2;
                    } else {
                        r[2] = r[0] + w2;
                    }
                    if e & 2 != 0 {
                        r[1] = r[3] - h2;
                    } else {
                        r[3] = r[1] + h2;
                    }
                }
            }
            t.b = r;
            ts_changed(p, ui);
        }
        DR_ROTATE => {
            // the angle about the centre at the drag start; Umschalt: 15 degree steps of the total
            let mut a = atan2(y - t.c0[1], x - t.c0[0]) - t.a0 + t.rot0;
            if mods & MOD_SHIFT != 0 {
                a = ts_round(a / C_STEP15) as f64 * C_STEP15;
            }
            t.rot = a;
            let c0 = t.c0;
            let turn = a - t.rot0;
            ts_rotate_from(t, turn, c0);
            ts_changed(p, ui);
        }
        _ => {}
    }
}

// ts_inside(x, y, margin) -> 1 when the image point lies inside the shape's frame grown by
// margin (image px)
fn ts_inside(t: &TShapes, x: f64, y: f64, margin: f64) -> bool {
    let (x, y) = t.mi.apply(x, y);
    let q = [x, y];
    for axis in 0..2 {
        let a = t.b[axis];
        let c = t.b[axis + 2];
        let lo = minsd(a, c) - margin;
        let hi = maxsd(c, a) + margin;
        if !(q[axis] >= lo) || !(hi >= q[axis]) {
            return false;
        }
    }
    true
}

// ------------------------------------------------------------------ tool callbacks

// ts_input(doc, ui, x_fx, y_fx) -> 1 changed
fn ts_input(p: &mut Pn, ui: &mut Ui, doc: DocId, inp: &UiInput, x_fx: i32, y_fx: i32) -> bool {
    p.tshp.curtool = tool_cur(&p.tool);
    let sx = inp.x + inp.rect[0];
    let sy = inp.y + inp.rect[1];
    let x = x_fx as f64 * C_INV256;
    let y = y_fx as f64 * C_INV256;
    if p.tshp.drag != DR_NONE {
        // a drag in progress
        let t = &mut p.tshp;
        if Some(doc) == t.doc && inp.btn & (1 << t.dbtn) != 0 {
            if x == t.lp[0] && y == t.lp[1] && inp.mods == t.lmods {
                return false;
            }
            t.lp = [x, y];
            t.lmods = inp.mods;
            ts_drag_to(p, ui, x, y, inp.mods);
            return true;
        }
        // released
        let old = t.drag;
        t.drag = DR_NONE;
        for n in t.nubs.iter_mut() {
            n.flags &= !(NBF_DRAG as u32);
        }
        if old == DR_DRAW {
            // the first drag ended: nothing drawn (a click) = no line / shape
            let drop = if ts_is_line(t) {
                let dx = t.pts[3][0] - t.pts[0][0];
                let dy = t.pts[3][1] - t.pts[0][1];
                !(dx * dx + dy * dy >= 0.25)
            } else {
                !(fabs(t.b[2] - t.b[0]) >= 0.5) || !(fabs(t.b[3] - t.b[1]) >= 0.5)
            };
            if drop {
                tool_cancel_live(p, ui);
                return true;
            }
        }
        ts_changed(p, ui); // the handles appear
        return true;
    }
    let btn = if inp.pressed & (1 << BTN_LEFT) != 0 {
        BTN_LEFT
    } else if inp.pressed & (1 << BTN_RIGHT) != 0 {
        BTN_RIGHT
    } else {
        // hover over the handles
        if !ts_session_ok(&p.tshp, doc) {
            return false;
        }
        let r = nubs_hover(&p.view, ui, &mut p.tshp.nubs, sx, sy);
        // the cursor zone of a shape (inside / rotate)
        if !ts_is_line(&p.tshp) {
            let z = if ts_inside(&p.tshp, x, y, 0.0) {
                1
            } else {
                let m = ts_img_per_dip(p, ui, ROT_DIP);
                if ts_inside(&p.tshp, x, y, m) { 2 } else { 0 }
            };
            p.tshp.zone = z;
        }
        return r;
    };
    let mut what = DR_DRAW;
    let mut hit = -1;
    let mut new = true;
    if ts_session_ok(&p.tshp, doc) {
        hit = nubs_hit(&p.view, ui, &p.tshp.nubs, sx, sy);
        let mut commit = false;
        if ts_is_line(&p.tshp) {
            // line: a control point (right = Bézier), the move handle (right = rotate)
            if hit < 0 || hit > MOVE_NUB {
                commit = true;
            } else if hit == MOVE_NUB {
                what = if btn == BTN_RIGHT { DR_ROTATE } else { DR_MOVE };
                new = false;
            } else {
                if btn == BTN_RIGHT && p.tshp.lctype != CT_BEZIER {
                    p.tshp.lctype = CT_BEZIER;
                    ts_changed(p, ui); // the Bézier arms appear, the curve changes
                }
                what = DR_NUB;
                new = false;
            }
        } else if hit >= 0 {
            what = DR_SCALE;
            new = false;
        } else if ts_inside(&p.tshp, x, y, 0.0) {
            what = if btn == BTN_RIGHT { DR_ROTATE } else { DR_MOVE };
            new = false;
        } else {
            let m = ts_img_per_dip(p, ui, ROT_DIP);
            if ts_inside(&p.tshp, x, y, m) {
                what = DR_ROTATE;
                new = false;
            } else {
                commit = true;
            }
        }
        if commit {
            tool_commit_live(p, ui);
        }
    }
    if new {
        // a new line / shape from the press point
        let ct = p.tshp.curtool;
        ts_begin(p, doc, btn, ct);
        let t = &mut p.tshp;
        t.pts = [[x, y]; 4];
        let fx = ts_floor(x) as f64;
        let fy = ts_floor(y) as f64;
        t.b = [fx, fy, fx, fy];
        what = DR_DRAW;
        hit = -1;
    }
    let t = &mut p.tshp;
    t.drag = what;
    t.dnub = hit;
    t.dbtn = btn;
    ts_save0(t);
    t.st = [x, y];
    t.lp = [x, y];
    t.lmods = inp.mods;
    // rotation: the centre and the start angle
    t.c0 = ts_center(t);
    t.a0 = atan2(y - t.c0[1], x - t.c0[0]);
    if hit >= 0 {
        t.nubs[(hit as usize) & (NUB_N - 1)].flags |= NBF_DRAG as u32;
        nubs_damage(&p.view, ui, &mut t.nubs);
    }
    true
}

// ts_key(vk, mods) -> 1 handled -- arrow keys move the live line / shape
fn ts_key(p: &mut Pn, ui: &mut Ui, vk: u32, mods: u32) -> bool {
    if !p.tshp.live || p.tshp.drag != DR_NONE {
        return false;
    }
    let step = if mods & MOD_CTRL != 0 { 10 } else { 1 };
    let (dx, dy) = match vk {
        VK_LEFT => (-step, 0),
        VK_RIGHT => (step, 0),
        VK_UP => (0, -step),
        VK_DOWN => (0, step),
        _ => return false,
    };
    ts_translate(p, ui, dx as f64, dy as f64);
    true
}

// ts_tick(doc) -- option / colour / selection changes of the live line / shape
fn ts_tick(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    p.tshp.curtool = tool_cur(&p.tool);
    let t = &p.tshp;
    if !t.live || doc.is_none() || doc != t.doc || t.drag == DR_DRAW {
        return;
    }
    let d = p.doc.d(doc.unwrap());
    if p.tool.cfg.ver != t.cver || pal_version(&p.pal) != t.palv || t.over != t.oseen || d.ver_sel != t.vsel {
        ts_versions(p);
        ts_changed(p, ui);
    }
}

// ts_cursor(x_fx, y_fx) -> CUR_*
fn ts_cursor(p: &mut Pn, _x: i32, _y: i32) -> u32 {
    let t = &p.tshp;
    if !t.live {
        return CUR_CROSS;
    }
    match t.drag {
        DR_MOVE | DR_NUB => return CUR_MOVE,
        DR_ROTATE => return CUR_HAND,
        DR_SCALE => return NUB_CURSORS[(t.dnub & (NUB_N as i32 - 1)) as usize],
        _ => {}
    }
    // hovered handle
    for (i, n) in t.nubs.iter().enumerate() {
        if n.flags & NBF_HOVER as u32 != 0 && n.flags & NBF_HIDDEN as u32 == 0 {
            return if ts_is_line(t) { CUR_MOVE } else { NUB_CURSORS[i] };
        }
    }
    if ts_is_line(t) {
        return CUR_CROSS;
    }
    match t.zone {
        1 => CUR_MOVE,
        2 => CUR_HAND,
        _ => CUR_CROSS,
    }
}

// ts_activate(doc) / ts_deactivate(doc)
fn ts_activate(p: &mut Pn, _ui: &mut Ui, _doc: Option<DocId>) {
    p.tshp.curtool = tool_cur(&p.tool);
}

// ------------------------------------------------------------------ options + records

// ts_is_line_cur() -> 1 when the current tool is Linie / Kurve
fn ts_is_line_cur(p: &Pn) -> bool {
    tool_cur(&p.tool) == TOOL_LINE
}

// ts_opt_style() -- "Stil:" (line: Linienanfang, Linienart, Linienende; shape: Linienart)
fn ts_opt_style(p: &mut Pn, ui: &mut Ui) {
    opt_label(ui, "Stil:");
    if ts_is_line_cur(p) {
        let mut v = p.tshp.cap0;
        let r = opt_menu(p, ui, OPT_CAP0, &mut v, &CAP_LABELS, &CAP0_ICONS, Some("Form des Linienanfangs"), OMF_ICONONLY as u32);
        p.tshp.cap0 = v;
        p.tshp.ochg |= r;
    }
    let r = ts_dash_menu(p, ui);
    p.tshp.ochg |= r;
    if ts_is_line_cur(p) {
        let mut v = p.tshp.cap1;
        let r = opt_menu(p, ui, OPT_CAP1, &mut v, &CAP_LABELS, &CAP1_ICONS, Some("Form des Linienendes"), OMF_ICONONLY as u32);
        p.tshp.cap1 = v;
        p.tshp.ochg |= r;
    }
}

// ts_dash_menu() -> 1 changed -- Linienart: a drop-down showing the pattern (drawn), flyout
// rows OPT_DASH + 1 + i with the names
fn ts_dash_menu(p: &mut Pn, ui: &mut Ui) -> bool {
    let cur = if p.tshp.dash < DASH_COUNT { p.tshp.dash } else { 0 };
    let mut chosen = cur;
    ui.next_flags(ST_SUBTLE);
    ui.dropdown_button(OPT_DASH, crate::preferences::localization::tr("             "));
    let r = ui.last_rect();
    let inset = ui.dp(8);
    let chev = ui.dp(28);
    let tc = ui.color(T_TEXT);
    let dpi = ui.dpi();
    let ic = p.tshp.dash_icon.get_or_insert_with(|| Rc::new(DashIcon { dash: Cell::new(0), argb: Cell::new(0), dpi: Cell::new(96) })).clone();
    ic.dash.set(cur);
    ic.argb.set(tc);
    ic.dpi.set(dpi);
    let f: DrawRef = ic;
    #[cfg(target_pointer_width = "64")]
    let (user, version) = ((cur as usize) | (tc as usize) << 32, cur);
    #[cfg(target_pointer_width = "32")]
    let (user, version) = (cur as usize, tc);
    ui.cmd_custom(r[0] + inset, r[1], (r[2] - inset - chev).max(4), r[3], Some(f), user, version);
    ui.tooltip("Linienart ändern");
    if ui.flyout_begin(OPT_DASH, FL_BELOW) {
        for i in 0..DASH_COUNT {
            if i == p.tshp.dash {
                ui.next_flags(ST_CHECKED);
            }
            if ui.menu_item(OPT_DASH + 1 + i, DASH_LABELS[i as usize]) & EV_ACT != 0 {
                chosen = i;
                ui.popup_close();
            }
        }
    }
    ui.flyout_end();
    if chosen == p.tshp.dash {
        return false;
    }
    p.tshp.dash = chosen;
    true
}

// ts_draw_dash(dash, argb, dpi, rect) -- cmd_custom: a horizontal line of Linienart dash in the
// colour argb across rect {x, y, w, h} (render time)
fn ts_draw_dash(g: &mut Gfx, dash: u32, argb: u32, dpi: i32, r: [i32; 4]) {
    g.dc.set_color(argb);
    let st = DStyle { width: dpi as f64 / 48.0, pat: dash_pattern(dash & 0xFF), join: 0, cap0: 0, cap1: 0, dcap: 0, offset: 0.0, miter: 0.0 }; // 2 px at 96 dpi
    let Gfx { dc, sf, text } = g;
    let path = &mut text.path;
    path.begin(dc);
    let y = ((r[3] as u32 >> 1) as i32).wrapping_add(r[1]) as f32;
    path.move_f(r[0] as f32, y);
    path.line_f(r[0].wrapping_add(r[2]) as f32, y);
    dash_stroke(path, &st);
    path.fill(&mut text.ras, dc, sf, 0);
}

// ts_options() -- the tool row of both tools (+ the test records)
fn ts_options(p: &mut Pn, ui: &mut Ui) {
    p.tshp.ochg = false;
    if ts_is_line_cur(p) {
        // Linie / Kurve: Linientyp, Pinselgröße, Stil, Fülleffekt, Füllmethode, Antialiasing
        let mut v = p.tshp.ctype;
        let r = opt_menu(p, ui, OPT_CTYPE, &mut v, &CTYPE_LABELS, &CTYPE_ICONS, None, OMF_ICONONLY as u32);
        p.tshp.ctype = v;
        if r {
            let t = &mut p.tshp;
            t.lctype = t.ctype;
            t.over = t.over.wrapping_add(1);
        }
        opt_sep(ui);
        opt_width(p, ui);
        opt_sep(ui);
        ts_opt_style(p, ui);
    } else {
        // Formen: Art der Form, Pinselgröße, Formfüllmodus, Stil, Ecke Größe, ...
        let cur = p.tshp.shape;
        let i = shl_gallery(p, ui, cur);
        if i >= 0 {
            ts_set_shape(&mut p.tshp, i);
        }
        opt_sep(ui);
        opt_width(p, ui);
        opt_sep(ui);
        let mut v = p.tshp.mode;
        let r = opt_menu(p, ui, OPT_MODE, &mut v, &MODE_LABELS, &MODE_ICONS, Some("Formfüllmodus"), OMF_ICONONLY as u32);
        p.tshp.mode = v;
        p.tshp.ochg |= r;
        opt_sep(ui);
        ts_opt_style(p, ui);
        if shl_radius(&p.shl, p.tshp.shape) {
            opt_sep(ui);
            ts_opt_radius(p, ui);
        }
    }
    opt_sep(ui);
    opt_fill(p, ui);
    opt_sep(ui);
    opt_blend(p, ui);
    opt_aa(p, ui);
    if !ts_is_line_cur(p) {
        opt_sep(ui);
        if ui.icon_button(OPT_FLIPH, IC_SWITCH, "Horizontal spiegeln") & EV_ACT != 0 {
            ts_flip(p, ui, false);
        }
        if ui.icon_button(OPT_FLIPV, IC_CHEV_UP_DOWN, "Vertikal spiegeln") & EV_ACT != 0 {
            ts_flip(p, ui, true);
        }
    }
    if p.tshp.ochg {
        p.tshp.over = p.tshp.over.wrapping_add(1);
    }
    ts_records(p, ui);
}

// ts_opt_radius() -- "Ecke Größe:" [-] number [+]
fn ts_opt_radius(p: &mut Pn, ui: &mut Ui) {
    opt_label(ui, "Ecke Größe:");
    let t = &mut p.tshp;
    if ui.icon_button(OPT_RDEC, IC_REMOVE, "Ecke verkleinern") & EV_ACT != 0 && t.radius - 1 >= 0 {
        t.radius -= 1;
        t.ochg = true;
    }
    ui.next_size(44, 0);
    if ui.number_box(OPT_RADIUS, &mut t.radius, &RANGE_RADIUS) & EV_ACT != 0 {
        t.ochg = true;
    }
    if ui.icon_button(OPT_RINC, IC_ADD, "Ecke vergrößern") & EV_ACT != 0 && (t.radius as u32) < RADIUS_MAX as u32 {
        t.radius += 1;
        t.ochg = true;
    }
}

// ts_rec(id, text, value) -- a hidden semantic record (test scripts)
fn ts_rec(ui: &mut Ui, id: u32, text: &str, value: i32) {
    ui.last_reset();
    ui.sem(K_TEXT, id, Some(text), value);
}

// ts_fmt_pt(&s, point) -- append " x,y" (rounded)
fn ts_fmt_pt(s: &mut String, q: [f64; 2]) {
    let a = ts_round(q[0]);
    let b = ts_round(q[1]);
    s.push_str(&crate::app::formatting::fmt_cap(format!(" {},{}", a, b), 32));
}

// ts_records() -- #ID_TSHP_FIRST .. +3
fn ts_records(p: &mut Pn, ui: &mut Ui) {
    let t = &p.tshp;
    let mut txt = String::new();
    if t.live {
        if ts_is_line(t) {
            txt.push_str(if t.lctype == CT_BEZIER { "L B" } else { "L S" });
            for i in 0..4 {
                ts_fmt_pt(&mut txt, t.pts[i]);
            }
        } else {
            // "F i x0,y0 x1,y1 deg": the image points of the frame corners (x0, y0) and
            // (x1, y1) (M applied), the rotation in whole degrees
            let deg = ts_round(t.rot * C_RAD2DEG);
            txt = format!("F {}", t.sidx);
            let (x, y) = t.m.apply(t.b[0], t.b[1]);
            ts_fmt_pt(&mut txt, [x, y]);
            let (x, y) = t.m.apply(t.b[2], t.b[3]);
            ts_fmt_pt(&mut txt, [x, y]);
            txt.push_str(&format!(" {}", deg));
        }
    }
    let txt = crate::app::formatting::fmt_cap(txt, 256);
    ts_rec(ui, REC_STATE, &txt, t.live as i32);
    let (st, _) = shl_status(&p.shl);
    let st = String::from(st);
    ts_rec(ui, REC_STATUS, &st, shl_count(&p.shl));
    let name = String::from(shl_name(&p.shl, t.shape));
    ts_rec(ui, REC_SHAPE, &name, t.shape);
    let o = crate::app::formatting::fmt_cap(format!("{},{},{},{},{},{}", t.dash, t.cap0, t.cap1, t.mode, t.radius, t.ctype), 256);
    ts_rec(ui, REC_OPTS, &o, 0);
}
