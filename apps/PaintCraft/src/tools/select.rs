// t_select -- PaintCraft: Rechteckige Auswahl, Lasso-Auswahl, Elliptische Auswahl (agent T2).
// Sub-module of sel.rs (tsel_init is called from sel_init).
//                                              [apps/PaintCraft/src/t_select.s]
//
// Dragging draws the new shape as a preview op of the selection (sel_set_preview: the ants
// and every clip show it at once); releasing records one history step named like the tool
// and combines the shape with the mode of the options (Auswahlmodus) or the mouse / keys
// like paint.net: Strg + left = Hinzufügen, right = Subtrahieren, Strg + right = Invertieren
// ("xor"), Alt + left = Schneiden.  A click without dragging deselects (Ersetzen).
// Rectangle / ellipse: corners on the pixel grid; Umschalt = square / circle; Zeichenmodus
// Alle Größen / Festes Seitenverhältnis / Feste Größe (Breite, Höhe).  After drawing, the
// shape keeps 8 handles (paint.net 5) until the selection changes otherwise or another tool
// is chosen; dragging a handle edits the shape in place (part of the same history step).
// Lasso: the freehand polygon of every pointer move point (sub-pixel), even-odd filled.
//
//   tsel_init()                register the three tools
//   tsel_mode(ui, btn) -> SM_* for a press (options + modifiers; also the magic wand)
//   tsel_state() -> text       test record: "x0,y0,x1,y1" of the editable shape or ""

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

use crate::app::constants::*;
use crate::document::model::{doc_active, doc_index, DocId};
use crate::document::history::hist_push_sel;
use crate::workspace::nubs::{nubs_damage, nubs_draw, nubs_frame, nubs_hit, nubs_hover, Nub};
use crate::document::selection::{sel_clear, sel_combine, sel_edit_last, sel_poly_ellipse, sel_poly_new, sel_poly_rect, sel_poly_unref, sel_set_preview, PolyRef};
use crate::tools::*;
use crate::workspace::view::view_screen_to_doc_fx;
use crate::app::Pn;

const DM_NORMAL: u32 = 0; // Alle Größen
const DM_RATIO: u32 = 1; // Festes Seitenverhältnis
const DM_SIZE: u32 = 2; // Feste Größe
const ID_TS_RECT: u32 = (TOOLOPT_BASE + TOOL_RECT_SELECT * TOOLOPT_STRIDE) as u32;
const ID_TS_ELL: u32 = (TOOLOPT_BASE + TOOL_ELLIPSE_SELECT * TOOLOPT_STRIDE) as u32;
const TSO_DM: u32 = 0; // option ids relative to the tool's block
const TSO_W: u32 = 10;
const TSO_H: u32 = 11;
const NFC: usize = NF_COUNT as usize;
const C_INV256: f32 = 0.00390625;
static DM_LABELS: [&str; 3] = ["Alle Größen", "Festes Seitenverhältnis", "Feste Größe"];
/// edges moved by each frame handle (NF_* order): 1 left, 2 top, 4 right, 8 bottom
static NUB_EDGES: [u8; 8] = [3, 2, 6, 4, 12, 8, 9, 1];
static NUB_CURSORS: [u32; 8] = [CUR_SIZE_NWSE, CUR_SIZE_NS, CUR_SIZE_NESW, CUR_SIZE_WE, CUR_SIZE_NWSE, CUR_SIZE_NS, CUR_SIZE_NESW, CUR_SIZE_WE];
const RANGE_RATIO: Range = Range::new(1, 1000, 1, 0);
const RANGE_SIZE: Range = Range::new(1, 65535, 1, 0);

/// the state of t_select.s (its g_* globals)
pub(crate) struct TSel {
    /// image of the drag / of the editable shape
    doc: Option<DocId>,
    eddoc: Option<DocId>,
    /// lasso points (f32 x, y)
    pts: Vec<[f32; 2]>,
    nubs: [Nub; NFC],
    /// shape rectangle (normalized) / editable rectangle
    r: [i32; 4],
    er: [i32; 4],
    /// 0 none, 1 new shape, 2 handle
    drag: u32,
    tool: i32,
    btn: u32,
    mode: u32,
    qual: u32,
    shift: u32,
    /// Umschalt at the last preview
    lshift: u32,
    /// last grid point of a handle drag
    hx: i32,
    hy: i32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    lx: f32,
    ly: f32,
    /// the handles apply
    ed: bool,
    edtool: i32,
    /// DOC_VER_SEL after the last edit
    edver: u32,
    /// handle being dragged
    nub: i32,
    /// Zeichenmodus; Festes Seitenverhältnis; Feste Größe
    dm: u32,
    rw: i32,
    rh: i32,
    fw: i32,
    fh: i32,
}

impl Default for TSel {
    fn default() -> TSel {
        TSel {
            doc: None,
            eddoc: None,
            pts: Vec::new(),
            nubs: Default::default(),
            r: [0; 4],
            er: [0; 4],
            drag: 0,
            tool: 0,
            btn: 0,
            mode: 0,
            qual: 0,
            shift: 0,
            lshift: 0,
            hx: 0,
            hy: 0,
            x0: 0,
            y0: 0,
            x1: 0,
            y1: 0,
            lx: 0.0,
            ly: 0.0,
            ed: false,
            edtool: 0,
            edver: 0,
            nub: 0,
            dm: 0,
            rw: 0,
            rh: 0,
            fw: 0,
            fh: 0,
        }
    }
}

static DESC_SEL: ToolDesc = ToolDesc {
    flags: (TF_SELMODE | TF_SELQUAL) as u32,
    deactivate: Some(ts_deactivate),
    input: Some(ts_input),
    options: Some(ts_options),
    cursor: Some(ts_cursor),
    docswitch: Some(ts_docswitch),
    overlay: Some(ts_overlay),
    tick: Some(ts_tick),
    ..ToolDesc::NONE
};

/// tsel_init() -- the three selection tools, default option values
pub(crate) fn tsel_init(p: &mut Pn, _ui: &mut Ui) {
    let t = &mut p.tsel;
    t.rw = 4;
    t.rh = 3;
    t.fw = 100;
    t.fh = 100;
    tool_register(&mut p.tool, TOOL_RECT_SELECT, Some(&DESC_SEL));
    tool_register(&mut p.tool, TOOL_LASSO, Some(&DESC_SEL));
    tool_register(&mut p.tool, TOOL_ELLIPSE_SELECT, Some(&DESC_SEL));
}

/// tsel_mode(ui, btn) -> SM_* of a press: right = Subtrahieren (Strg: Invertieren),
/// Strg + left = Hinzufügen, Alt + left = Schneiden, else the Auswahlmodus option
pub(crate) fn tsel_mode(p: &Pn, inp: &UiInput, btn: u32) -> u32 {
    let m = inp.mods;
    if btn == BTN_RIGHT {
        return if m & MOD_CTRL != 0 { SM_XOR as u32 } else { SM_EXCLUDE as u32 };
    }
    if m & MOD_CTRL != 0 {
        return SM_UNION as u32;
    }
    if m & MOD_ALT != 0 {
        return SM_INTERSECT as u32;
    }
    let v = p.tool.cfg.selmode;
    if v < crate::app::constants::SM_COUNT as u32 { v } else { 0 }
}

/// tsel_state() -> "x0,y0,x1,y1" of the editable rectangle / ellipse, "" when none
pub(crate) fn tsel_state(p: &Pn) -> String {
    let d = doc_active(&p.doc);
    if !ts_editable(p, d) {
        return String::new();
    }
    let e = p.tsel.er;
    format!("{},{},{},{}", e[0], e[1], e[2], e[3])
}

// ------------------------------------------------------------------ helpers

// ts_grid(v_fx) -> nearest pixel grid line (ties go down)
fn ts_grid(v: i32) -> i32 {
    v.wrapping_add(127) >> 8
}

// ts_editable(doc) -> the handles of the last rectangle / ellipse apply
fn ts_editable(p: &Pn, doc: Option<DocId>) -> bool {
    let t = &p.tsel;
    let Some(d) = doc else { return false };
    t.ed && doc == t.eddoc && p.doc.get(d).is_some_and(|x| x.ver_sel == t.edver) && tool_cur(&p.tool) == t.edtool
}

// ts_hide(): the handles disappear
fn ts_hide(p: &mut Pn, ui: &mut Ui) {
    if p.tsel.ed {
        p.tsel.ed = false;
        nubs_damage(&p.view, ui, &mut p.tsel.nubs);
    }
}

// ts_nubs_update(): the 8 handles on the editable rectangle
fn ts_nubs_update(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.tsel;
    let e = t.er;
    nubs_frame(&p.view, ui, &mut t.nubs, e[0] << 8, e[1] << 8, e[2] << 8, e[3] << 8);
    nubs_damage(&p.view, ui, &mut t.nubs); // (shown again at an unchanged place)
}

// ts_constrain(): r = the shape rectangle from the press / current grid points: Feste Größe,
// Festes Seitenverhältnis, Umschalt = square (rectangle and ellipse only)
fn ts_constrain(t: &mut TSel) {
    let mut dx = t.x1.wrapping_sub(t.x0);
    let mut dy = t.y1.wrapping_sub(t.y0);
    if t.tool != TOOL_LASSO && (t.dm == DM_SIZE || t.dm == DM_RATIO || t.shift != 0) {
        let (w, h);
        if t.dm == DM_SIZE {
            w = t.fw;
            h = t.fh;
        } else if t.dm == DM_RATIO {
            // w = |dx|, h = |dy|; the larger one wins, the other follows rw : rh
            let aw = dx.wrapping_abs() as u32 as u64;
            let ah = dy.wrapping_abs() as u32 as u64;
            let rw = t.rw as u32 as u64;
            let rh = t.rh as u32 as u64;
            if ((aw * rh) as i64) < ((ah * rw) as i64) {
                h = ah as i32;
                w = ((ah * rw + (rh >> 1)) / rh) as i32;
            } else {
                // h = round(w * rh / rw)
                w = aw as i32;
                h = ((aw * rh + (rw >> 1)) / rw) as i32;
            }
        } else {
            // square: the larger extent in both directions
            let m = dx.wrapping_abs().max(dy.wrapping_abs());
            w = m;
            h = m;
        }
        // dx = sign(dx) * w, dy = sign(dy) * h (sign(0) = +)
        dx = if dx < 0 { w.wrapping_neg() } else { w };
        dy = if dy < 0 { h.wrapping_neg() } else { h };
    }
    let (ax, bx) = (t.x0, t.x0.wrapping_add(dx));
    let (ay, by) = (t.y0, t.y0.wrapping_add(dy));
    t.r = [ax.min(bx), ay.min(by), ax.max(bx), ay.max(by)];
}

// ts_poly_of(tool, &rect) -> POLY of a rectangle / ellipse (None when it has no area)
fn ts_poly_of(tool: i32, r: &[i32; 4]) -> Option<PolyRef> {
    if r[0] == r[2] || r[1] == r[3] {
        return None;
    }
    if tool == TOOL_ELLIPSE_SELECT { sel_poly_ellipse(r[0], r[1], r[2], r[3]) } else { sel_poly_rect(r[0], r[1], r[2], r[3]) }
}

// ts_shape() -> POLY of the shape being drawn | None (no area yet)
fn ts_shape(t: &mut TSel) -> Option<PolyRef> {
    if t.tool == TOOL_LASSO {
        if t.pts.len() < 3 {
            return None;
        }
        return sel_poly_new(&t.pts);
    }
    ts_constrain(t);
    ts_poly_of(t.tool, &t.r)
}

// ts_preview(): the shape as the selection's preview op
fn ts_preview(p: &mut Pn) {
    let poly = ts_shape(&mut p.tsel);
    let Some(doc) = p.tsel.doc else { return };
    let (m, q) = (p.tsel.mode, p.tsel.qual);
    sel_set_preview(p, doc, poly.as_ref(), m, q);
    sel_poly_unref(poly);
}

// ts_lasso_add(x_fx, y_fx): append a lasso point (image px, f32), duplicates skipped
fn ts_lasso_add(t: &mut TSel, x: i32, y: i32) {
    t.lx = x as f32 * C_INV256;
    t.ly = y as f32 * C_INV256;
    if let Some(l) = t.pts.last() {
        if l[0].to_bits() == t.lx.to_bits() && l[1].to_bits() == t.ly.to_bits() {
            return;
        }
    }
    t.pts.push([t.lx, t.ly]);
}

// ts_lasso_moves(ui, x_fx, y_fx): every move point of this frame, then the pointer
fn ts_lasso_moves(p: &mut Pn, inp: &UiInput, x: i32, y: i32) {
    for pt in inp.pts.iter() {
        let sx = ((pt[0] + inp.rect[0]) << 8) | 128;
        let sy = ((pt[1] + inp.rect[1]) << 8) | 128;
        let (a, b) = view_screen_to_doc_fx(&p.view, sx, sy);
        ts_lasso_add(&mut p.tsel, a, b);
    }
    ts_lasso_add(&mut p.tsel, x, y);
}

// ts_abort(): a drag in progress is dropped (preview removed)
fn ts_abort(p: &mut Pn) {
    if p.tsel.drag == 0 {
        return;
    }
    p.tsel.drag = 0;
    for n in p.tsel.nubs.iter_mut() {
        n.flags &= !(NBF_DRAG as u32);
    }
    let Some(doc) = p.tsel.doc else { return };
    if doc_index(&p.doc, doc) >= 0 {
        sel_set_preview(p, doc, None, 0, 0);
    }
}

// ts_finish(): the button was released: record and combine the shape
fn ts_finish(p: &mut Pn, ui: &mut Ui) {
    p.tsel.drag = 0;
    let Some(doc) = p.tsel.doc else { return };
    let poly = ts_shape(&mut p.tsel);
    sel_set_preview(p, doc, None, 0, 0);
    let Some(poly) = poly else {
        // a click: Ersetzen deselects
        if p.tsel.mode == SM_REPLACE as u32 && p.doc.d(doc).sel.nops != 0 && hist_push_sel(p, doc, "Auswahl aufheben", IC_CLEAR_SELECTION) {
            sel_clear(p, doc);
        }
        return;
    };
    let (name, icon) = match p.tsel.tool {
        TOOL_LASSO => ("Lasso-Auswahl", IC_LASSO),
        TOOL_ELLIPSE_SELECT => ("Elliptische Auswahl", IC_CIRCLE_RING),
        _ => ("Rechteckige Auswahl", IC_SELECT_RECT),
    };
    if hist_push_sel(p, doc, name, icon) {
        let (m, q) = (p.tsel.mode, p.tsel.qual);
        sel_combine(p, doc, &poly, m, q);
        // rectangle / ellipse: editable with handles
        if p.tsel.tool != TOOL_LASSO && p.doc.d(doc).sel.nops != 0 {
            let ver = p.doc.d(doc).ver_sel;
            let t = &mut p.tsel;
            t.edtool = t.tool;
            t.eddoc = Some(doc);
            t.edver = ver;
            t.er = t.r;
            t.ed = true;
            ts_nubs_update(p, ui);
        }
    }
    sel_poly_unref(Some(poly));
}

// ts_handle(gx, gy): a handle was dragged to the grid point (gx, gy): the editable shape
// follows (the last op's geometry, no new history step)
fn ts_handle(p: &mut Pn, ui: &mut Ui, gx: i32, gy: i32) {
    let t = &mut p.tsel;
    let e = NUB_EDGES[(t.nub & (NF_COUNT - 1)) as usize];
    if e & 1 != 0 {
        t.er[0] = gx;
    }
    if e & 2 != 0 {
        t.er[1] = gy;
    }
    if e & 4 != 0 {
        t.er[2] = gx;
    }
    if e & 8 != 0 {
        t.er[3] = gy;
    }
    let r = [t.er[0].min(t.er[2]), t.er[1].min(t.er[3]), t.er[0].max(t.er[2]), t.er[1].max(t.er[3])];
    if let (Some(poly), Some(doc)) = (ts_poly_of(t.edtool, &r), t.eddoc) {
        // (no area: keep the last geometry)
        sel_edit_last(p, doc, &poly);
        sel_poly_unref(Some(poly));
        p.tsel.edver = p.doc.d(doc).ver_sel;
    }
    ts_nubs_update(p, ui);
}

// ------------------------------------------------------------------ tool callbacks

// ts_input(doc, ui, x_fx, y_fx) -> changed
fn ts_input(p: &mut Pn, ui: &mut Ui, doc: DocId, inp: &UiInput, x: i32, y: i32) -> bool {
    let sx = inp.x + inp.rect[0];
    let sy = inp.y + inp.rect[1];
    p.tsel.shift = inp.mods & MOD_SHIFT;
    if p.tsel.drag != 0 {
        if Some(doc) != p.tsel.doc {
            ts_abort(p);
            return true;
        }
        if inp.btn & (1 << p.tsel.btn) == 0 {
            // released
            if p.tsel.drag == 2 {
                let t = &mut p.tsel;
                t.drag = 0;
                t.nubs[(t.nub & (NF_COUNT - 1)) as usize].flags &= !(NBF_DRAG as u32);
                nubs_damage(&p.view, ui, &mut t.nubs);
            } else {
                ts_finish(p, ui);
            }
            return true;
        }
        if p.tsel.drag == 2 {
            let gx = ts_grid(x);
            let gy = ts_grid(y);
            if gx == p.tsel.hx && gy == p.tsel.hy {
                return false;
            }
            p.tsel.hx = gx;
            p.tsel.hy = gy;
            ts_handle(p, ui, gx, gy);
            return true;
        }
        if p.tsel.tool == TOOL_LASSO {
            let n = p.tsel.pts.len();
            ts_lasso_moves(p, inp, x, y);
            if n == p.tsel.pts.len() {
                return false;
            }
            ts_preview(p);
            return true;
        }
        let gx = ts_grid(x);
        let gy = ts_grid(y);
        let t = &mut p.tsel;
        // only a new grid point or Umschalt changes the shape
        if gx == t.x1 && gy == t.y1 && t.shift == t.lshift {
            return false;
        }
        t.x1 = gx;
        t.y1 = gy;
        t.lshift = t.shift;
        ts_preview(p);
        return true;
    }
    let btn = if inp.pressed & (1 << BTN_LEFT) != 0 {
        BTN_LEFT
    } else if inp.pressed & (1 << BTN_RIGHT) != 0 {
        BTN_RIGHT
    } else {
        // hover: the handles light up
        if !ts_editable(p, Some(doc)) {
            return false;
        }
        return nubs_hover(&p.view, ui, &mut p.tsel.nubs, sx, sy);
    };
    if btn == BTN_LEFT && ts_editable(p, Some(doc)) {
        let k = nubs_hit(&p.view, ui, &p.tsel.nubs, sx, sy);
        if k >= 0 {
            // drag a handle
            let t = &mut p.tsel;
            t.nub = k;
            t.hx = i32::MIN;
            t.hy = i32::MIN;
            t.drag = 2;
            t.btn = btn;
            t.doc = Some(doc);
            t.nubs[(k & (NF_COUNT - 1)) as usize].flags |= NBF_DRAG as u32;
            nubs_damage(&p.view, ui, &mut t.nubs);
            return true;
        }
    }
    ts_hide(p, ui);
    let mode = tsel_mode(p, inp, btn);
    let qual = p.tool.cfg.selqual;
    let tool = tool_cur(&p.tool);
    let t = &mut p.tsel;
    t.doc = Some(doc);
    t.btn = btn;
    t.tool = tool;
    t.mode = mode;
    t.qual = qual;
    t.drag = 1;
    t.x0 = ts_grid(x);
    t.x1 = t.x0;
    t.y0 = ts_grid(y);
    t.y1 = t.y0;
    t.lshift = t.shift;
    t.pts.clear();
    if t.tool == TOOL_LASSO {
        ts_lasso_add(t, x, y);
    }
    ts_preview(p);
    true
}

// ts_tick(doc): handles go away when the selection changed otherwise
fn ts_tick(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    if p.tsel.ed && p.tsel.drag == 0 && !ts_editable(p, doc) {
        ts_hide(p, ui);
    }
}

// ts_deactivate(doc) / ts_docswitch(old, new): drop a drag and the handles
fn ts_deactivate(p: &mut Pn, ui: &mut Ui, _doc: Option<DocId>) {
    ts_abort(p);
    ts_hide(p, ui);
}

fn ts_docswitch(p: &mut Pn, ui: &mut Ui, _old: Option<DocId>, _new: Option<DocId>) {
    ts_abort(p);
    ts_hide(p, ui);
}

// ts_overlay(doc, &rect): the handles of the editable shape (until ts_tick hides them with
// damage, so a selection change of this frame keeps the screen consistent)
fn ts_overlay(p: &mut Pn, g: &mut Gfx, doc: DocId, _rect: [i32; 4]) {
    if p.tsel.ed && Some(doc) == p.tsel.eddoc {
        nubs_draw(&p.view, g, &mut p.tsel.nubs);
    }
}

// ts_cursor(x_fx, y_fx) -> resize cursors over / while dragging a handle, else the cross
fn ts_cursor(p: &mut Pn, _x: i32, _y: i32) -> u32 {
    if !ts_editable(p, doc_active(&p.doc)) {
        return CUR_CROSS;
    }
    for (i, n) in p.tsel.nubs.iter().enumerate() {
        if n.flags & (NBF_HOVER | NBF_DRAG) as u32 != 0 {
            return NUB_CURSORS[i];
        }
    }
    CUR_CROSS
}

// ts_options(): Auswahlmodus, Auswahl-Qualität (+ Zeichenmodus, Breite, Höhe)
fn ts_options(p: &mut Pn, ui: &mut Ui) {
    opt_selmode(p, ui);
    opt_selqual(p, ui);
    let id = match tool_cur(&p.tool) {
        TOOL_RECT_SELECT => ID_TS_RECT,
        TOOL_ELLIPSE_SELECT => ID_TS_ELL,
        _ => return,
    };
    opt_sep(ui);
    opt_label(ui, "Zeichenmodus:");
    let mut v = p.tsel.dm;
    opt_menu(p, ui, id + TSO_DM, &mut v, &DM_LABELS, &[], None, OMF_NOICON as u32);
    p.tsel.dm = v;
    if p.tsel.dm == DM_NORMAL {
        return;
    }
    let ratio = p.tsel.dm == DM_RATIO;
    let rg = if ratio { &RANGE_RATIO } else { &RANGE_SIZE };
    let t = &mut p.tsel;
    let (w, h) = if ratio { (&mut t.rw, &mut t.rh) } else { (&mut t.fw, &mut t.fh) };
    opt_label(ui, "Breite:");
    ui.next_size(64, 0);
    ui.number_box(id + TSO_W, w, rg);
    opt_label(ui, "Höhe:");
    ui.next_size(64, 0);
    ui.number_box(id + TSO_H, h, rg);
}
