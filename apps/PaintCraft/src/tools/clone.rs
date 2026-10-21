// t_clone -- PaintCraft: Klonen (Stempel) (L) and Neu färben (R) (agent T6).  Both paint with
// the brush engine (brush_engine.rs: one coverage mask per stroke; width, hardness, spacing,
// antialiasing, pen pressure) through one TLK_SOURCE tool layer (tool.rs, the single write
// path, clipped by the selection).  A stroke is a pending edit from the press to the release
// = one history step ("Klonen (Stempel)" / "Neu färben"); Esc cancels it.
//                                              [apps/PaintCraft/src/t_clone.s]
//
//   Klonen (Stempel)  Strg+click sets the anchor (image pixel + the current layer, kept by its
//                     LY_UID: the source may be another layer, DRAW-017).  The first stroke
//                     after it fixes the offset anchor - press point; every stroke copies the
//                     source layer at pointer + offset with the Füllmethode (TC_BLEND, incl.
//                     Überschreiben).  The source is the layer as it was before the stroke (the
//                     preview reads the unchanged layer, the commit a snapshot of the source
//                     area when source = target layer).  A crosshair marks the source point
//                     (the anchor until the first stroke, then pointer + offset).  Without an
//                     anchor a press shows an error text in the error bar (fio_error).
//   Neu färben        left: pixels similar to the secondary colour ("Probennahme:
//                     Sekundärfarbe") or to the layer colour under the press point
//                     ("Quelle: erster Klick") take the primary colour; right: the roles of
//                     the colours are swapped.  Similar = flood CheckColor with Toleranz
//                     (TC_TOL) on straight ("Gerade") or premultiplied ("Vormultipliziert",
//                     default) colours; a matching pixel is shifted by (new - old) per colour
//                     channel (its shading stays, alpha kept) and written with Überschreiben
//                     (lerp by the brush coverage), so non-matching pixels stay bit-exact.
//
//   tclone_init()                        registers both tools (tool_register)
//   tclone_cmd(id, arg) -> 1             test hooks, id = CMD_TCLONE_FIRST + k:
//       0 x | y << 16   anchor at (x, y) on the current layer of the active image (= Strg+click)
//       1 what          0 = anchor and offset cleared, 1 = only the offset (next stroke fixes it)
//       2 mode          Neu färben: Probennahme 0 einmalig / 1 Sekundärfarbe
//       3 mode          Neu färben: Alpha-Modus der Toleranz 0 Vormultipliziert / 1 Gerade
//   Hidden records (in the tool row of the current tool):
//       ID_TCLONE_FIRST + 0  text "x,y,layer" anchor (layer index, -1 = none / gone), value 1 =
//                            set for the active image
//       ID_TCLONE_FIRST + 1  text "dx,dy" offset, value 1 = fixed
//       ID_TCLONE_FIRST + 2  text "sx,sy" crosshair (window px), value 1 = shown
//       ID_TCLONE_FIRST + 3  text "sampling,alpha" (Neu färben), value 1 = stroke running
//       ID_TCLONE_FIRST + 4  text "OLD,NEW" colours of the last recolor stroke (AARRGGBB)
//
// Rust model: the source layer is kept by its uid (TlSrc::Layer); the recoloured copy / the
// clone snapshot is p.tclone.sf (TlSrc::Clone).

use alloc::format;

use gfx::blendmode::{BM_COUNT, BM_OPAC_SHIFT, BM_OVERWRITE};
use gfx::{Surface, FMT_ARGB};
use ui::*;

use crate::tools::brushes::{be_begin, be_dab, be_dirty, be_end, be_to, be_ui_pressure, Bep};
use crate::app::constants::*;
use crate::document::model::{doc_active, doc_cur_layer, Doc, DocId};
use crate::integrations::file_io::fio_error;
use crate::document::layers::lay_find_uid;
use crate::tools::brush::ui_pt_fx;
use crate::tools::*;
use crate::workspace::view::view_doc_to_screen_fx;
use crate::app::Pn;

const CX_ARM: i32 = 8; // crosshair arm (window px)
const CX_PAD: i32 = CX_ARM + 3; // damage margin around the centre
const RC_ONCE: u32 = 0; // Quelle: erster Klick
const RC_SECONDARY: u32 = 1; // Quelle: Sekundärfarbe
const TA_PREMUL: u32 = 0; // Alpha-Modus der Toleranz: Vormultipliziert
const ID_TC_ANCHOR: u32 = ID_TCLONE_FIRST; // records
const ID_TC_OFFSET: u32 = ID_TCLONE_FIRST + 1;
const ID_TC_CROSS: u32 = ID_TCLONE_FIRST + 2;
const ID_TC_RCOPT: u32 = ID_TCLONE_FIRST + 3;
const ID_TC_RCCOL: u32 = ID_TCLONE_FIRST + 4;
const ID_RC_SAMP: u32 = (TOOLOPT_BASE + TOOL_RECOLOR * TOOLOPT_STRIDE) as u32; // Probennahme (rows +1..+2)
const ID_RC_ALPHA: u32 = ID_RC_SAMP + 5; // Alpha-Modus (rows +1..+2)
const S_NOANCHOR: &str = "Zum Klonen fehlt noch die Quelle. Legen Sie sie zuerst mit Strg+Klick fest.";
static SAMP_LABELS: [&str; 2] = ["Quelle: erster Klick", "Quelle: Sekundärfarbe"];
static ALPHA_LABELS: [&str; 2] = ["Vormultipliziert", "Gerade"];
static SAMP_ICONS: [u32; 2] = [IC_EYEDROPPER, IC_COLOR];
static ALPHA_ICONS: [u32; 2] = [IC_CONTRAST, IC_DROP];

/// the state of t_clone.s (its g_* globals; the tool layer is p.tool.tls[TL_CLONE])
pub(crate) struct TClone {
    /// recoloured copy / clone snapshot (g_tc_sf; None = g_tc_have 0)
    sf: Option<Surface>,
    doc: Option<DocId>,
    /// Klonen: uid of the source layer of the stroke
    sly: u64,
    /// Neu färben: uid of the target layer of the stroke
    rc_ly: u64,
    /// anchor: document, layer uid, pixel
    an_doc: Option<DocId>,
    an_uid: u64,
    an_x: i32,
    an_y: i32,
    an_on: bool,
    /// offset (source - target) fixed
    off_on: bool,
    off_dx: i32,
    off_dy: i32,
    /// crosshair drawn at (sx, sy)
    cx_vis: bool,
    cx_sx: i32,
    cx_sy: i32,
    /// 1 while a stroke is drawn
    on: bool,
    btn: u32,
    tool: i32,
    /// Probennahme (default: Sekundärfarbe), TA_*
    rc_samp: u32,
    rc_alpha: u32,
    /// colour replaced (straight), replacement colour
    rc_old: u32,
    rc_new: u32,
    /// comparison colour (premultiplied in TA_PREMUL)
    rc_cmp: u32,
    /// tol * tol * 4
    rc_thr: u32,
    /// new - old per channel B, G, R
    rc_d: [i32; 3],
}

impl Default for TClone {
    fn default() -> TClone {
        TClone {
            sf: None,
            doc: None,
            sly: 0,
            rc_ly: 0,
            an_doc: None,
            an_uid: 0,
            an_x: 0,
            an_y: 0,
            an_on: false,
            off_on: false,
            off_dx: 0,
            off_dy: 0,
            cx_vis: false,
            cx_sx: 0,
            cx_sy: 0,
            on: false,
            btn: 0,
            tool: 0,
            rc_samp: RC_SECONDARY,
            rc_alpha: 0,
            rc_old: 0,
            rc_new: 0,
            rc_cmp: 0,
            rc_thr: 0,
            rc_d: [0; 3],
        }
    }
}

static DESC_CLONE: ToolDesc = ToolDesc {
    flags: (TF_CLIP_SEL | TF_USES_WIDTH | TF_AA | TF_BLEND | TF_HARD | TF_OUTLINE) as u32,
    activate: Some(tc_act),
    deactivate: Some(tc_deact),
    input: Some(tc_input),
    options: Some(clone_options),
    commit: Some(tc_commit),
    cancel: Some(tc_cancel),
    overlay: Some(tc_overlay),
    tick: Some(tc_tick),
    ..ToolDesc::NONE
};
static DESC_RECOLOR: ToolDesc = ToolDesc {
    flags: (TF_CLIP_SEL | TF_USES_WIDTH | TF_AA | TF_HARD | TF_TOL | TF_OUTLINE) as u32,
    input: Some(tc_input),
    options: Some(recolor_options),
    commit: Some(tc_commit),
    cancel: Some(tc_cancel),
    ..ToolDesc::NONE
};

/// tclone_init() -- register Klonen (Stempel) and Neu färben
pub(crate) fn tclone_init(p: &mut Pn, _ui: &mut Ui) {
    tool_register(&mut p.tool, TOOL_CLONE, Some(&DESC_CLONE));
    tool_register(&mut p.tool, TOOL_RECOLOR, Some(&DESC_RECOLOR));
}

/// the private surface of a stroke (TlSrc::Clone)
pub(crate) fn tc_surface(t: &TClone) -> Option<&Surface> {
    t.sf.as_ref()
}

/// tclone_cmd(id, arg) -> 1 handled -- test hooks (see the file header)
pub(crate) fn tclone_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    let k = id.wrapping_sub(CMD_TCLONE_FIRST);
    if k > CMD_TCLONE_LAST - CMD_TCLONE_FIRST {
        return false;
    }
    match k {
        0 => {
            let d = doc_active(&p.doc);
            an_set(p, d, (arg & 0xFFFF) as i32, (arg >> 16) as i32);
        }
        1 => {
            p.tclone.off_on = false;
            if arg == 0 {
                p.tclone.an_on = false;
            }
        }
        2 => {
            p.tclone.rc_samp = arg & 1;
            tool_cfg_changed(p);
        }
        3 => {
            p.tclone.rc_alpha = arg & 1;
            tool_cfg_changed(p);
        }
        _ => {}
    }
    cx_update(p, ui);
    ui.repass();
    true
}

// ================================================================== anchor (Klonen)

// an_set(doc, x, y): anchor at image pixel (x, y) on doc's current layer; the next stroke
// fixes the offset again
fn an_set(p: &mut Pn, doc: Option<DocId>, x: i32, y: i32) {
    let Some(doc) = doc else { return };
    let Some(l) = p.doc.get(doc).and_then(doc_cur_layer) else { return };
    let t = &mut p.tclone;
    t.an_uid = l.uid;
    t.an_doc = Some(doc);
    t.an_x = x;
    t.an_y = y;
    t.an_on = true;
    t.off_on = false;
}

// an_valid(doc) -> the anchor belongs to doc
fn an_valid(t: &TClone, doc: Option<DocId>) -> bool {
    doc.is_some() && t.an_on && doc == t.an_doc
}

// an_find(doc) -> index of the anchor's layer (found by LY_UID) | -1
fn an_find(t: &TClone, d: &Doc) -> i32 {
    lay_find_uid(d, t.an_uid).unwrap_or(-1)
}

// an_layer(doc) -> uid of the source layer (the anchor's layer; gone: the current layer)
fn an_layer(t: &TClone, d: &Doc) -> Option<u64> {
    if lay_find_uid(d, t.an_uid).is_some() {
        return Some(t.an_uid);
    }
    doc_cur_layer(d).map(|l| l.uid)
}

// ================================================================== crosshair (Klonen)

// cx_damage(sx, sy): redraw the crosshair area around the window point
fn cx_damage(p: &Pn, ui: &mut Ui, x: i32, y: i32) {
    tool_damage(&p.view, ui, x - CX_PAD, y - CX_PAD, x + CX_PAD + 1, y + CX_PAD + 1);
}

// cx_hide(): the crosshair disappears (another tool)
fn cx_hide(p: &mut Pn, ui: &mut Ui) {
    if p.tclone.cx_vis {
        p.tclone.cx_vis = false;
        cx_damage(p, ui, p.tclone.cx_sx, p.tclone.cx_sy);
    }
}

// cx_update() -> changed: the crosshair of Klonen: at the anchor, once the offset is fixed at
// pointer + offset; the old and the new place are redrawn when it moved or appeared /
// disappeared
fn cx_update(p: &mut Pn, ui: &mut Ui) -> bool {
    let (mut vis, mut sx, mut sy) = (false, 0, 0);
    let t = &p.tclone;
    if tool_cur(&p.tool) == TOOL_CLONE && an_valid(t, doc_active(&p.doc)) {
        let mut x = (t.an_x << 8) + 128;
        let mut y = (t.an_y << 8) + 128;
        if t.off_on {
            let (px, py, ok) = tool_pointer(&p.tool);
            if ok {
                x = (t.off_dx << 8).wrapping_add(px);
                y = (t.off_dy << 8).wrapping_add(py);
            }
        }
        let (a, b) = view_doc_to_screen_fx(&p.view, x, y);
        sx = a >> 8;
        sy = b >> 8;
        vis = true;
    }
    if vis == t.cx_vis && (!vis || (sx == t.cx_sx && sy == t.cx_sy)) {
        return false;
    }
    if t.cx_vis {
        cx_damage(p, ui, t.cx_sx, t.cx_sy);
    }
    let t = &mut p.tclone;
    t.cx_vis = vis;
    t.cx_sx = sx;
    t.cx_sy = sy;
    if vis {
        cx_damage(p, ui, sx, sy);
    }
    true
}

// tc_overlay(doc, &canvas_rect): TD_OVERLAY of Klonen: the crosshair (dark bars, light lines)
fn tc_overlay(p: &mut Pn, g: &mut Gfx, _doc: DocId, _rect: [i32; 4]) {
    let t = &p.tclone;
    if !t.cx_vis {
        return;
    }
    let (x, y) = (t.cx_sx, t.cx_sy);
    g.dc.set_color(0x9000_0000);
    g.dc.fill(g.sf, x - CX_ARM - 1, y - 1, 2 * CX_ARM + 3, 3);
    g.dc.fill(g.sf, x - 1, y - CX_ARM - 1, 3, CX_ARM);
    g.dc.fill(g.sf, x - 1, y + 2, 3, CX_ARM);
    g.dc.set_color(0xFFFF_FFFF);
    g.dc.hline(g.sf, x - CX_ARM, y, 2 * CX_ARM + 1);
    g.dc.vline(g.sf, x, y - CX_ARM, 2 * CX_ARM + 1);
}

// tc_act(doc) / tc_tick(doc): crosshair state; tc_deact(doc): hide it
fn tc_act(p: &mut Pn, ui: &mut Ui, _doc: Option<DocId>) {
    cx_update(p, ui);
}

fn tc_tick(p: &mut Pn, ui: &mut Ui, _doc: Option<DocId>) {
    cx_update(p, ui);
}

fn tc_deact(p: &mut Pn, ui: &mut Ui, _doc: Option<DocId>) {
    cx_hide(p, ui);
}

// ================================================================== strokes (both tools)

// tc_input(doc, ui, x_fx, y_fx) -> changed (TD_INPUT of both tools)
fn tc_input(p: &mut Pn, ui: &mut Ui, doc: DocId, inp: &UiInput, x: i32, y: i32) -> bool {
    if !p.tclone.on {
        let mut ch = false;
        if inp.pressed & ((1 << BTN_LEFT) | (1 << BTN_RIGHT)) != 0 {
            let mut start = None;
            if tool_cur(&p.tool) == TOOL_CLONE {
                // Klonen: Strg+click = anchor, a left press = stroke (needs the anchor)
                if inp.pressed & (1 << BTN_LEFT) != 0 {
                    if inp.mods & MOD_CTRL != 0 {
                        let (ax, ay) = ui_pt_fx(p, inp, inp.press_x, inp.press_y);
                        an_set(p, Some(doc), ax >> 8, ay >> 8);
                        ch = true;
                    } else if !an_valid(&p.tclone, Some(doc)) {
                        fio_error(p, None, S_NOANCHOR);
                        ch = true;
                    } else {
                        start = Some(BTN_LEFT);
                    }
                }
            } else {
                // Neu färben: left = primary, right = secondary
                start = Some(if inp.pressed & (1 << BTN_LEFT) != 0 { BTN_LEFT } else { BTN_RIGHT });
            }
            if let Some(btn) = start {
                tc_begin(p, doc, btn, inp);
                if p.tclone.on {
                    // the pointer may have moved in the press frame already
                    be_to(&mut p.be, x, y, be_ui_pressure(inp, -1));
                    tc_mark(p);
                    ch = true;
                }
            }
        }
        // (the records of the tool row show it in the next pass)
        return cx_update(p, ui) | ch;
    }
    if Some(doc) != p.tclone.doc {
        return false;
    }
    // every move point of this frame, then the pointer (repeated passes of a frame see the
    // same points: nothing changes, the result stays false)
    let mut ch = false;
    for (i, pt) in inp.pts.iter().enumerate() {
        let (px, py) = ui_pt_fx(p, inp, pt[0], pt[1]);
        ch |= be_to(&mut p.be, px, py, be_ui_pressure(inp, i as i32)); // (PN1) the point's own pressure
    }
    ch |= be_to(&mut p.be, x, y, be_ui_pressure(inp, -1));
    tc_mark(p);
    ch |= cx_update(p, ui);
    // released (or the button is no longer held): the stroke is finished
    let m = 1 << p.tclone.btn;
    if inp.btn & m != 0 && inp.released & m == 0 {
        return ch;
    }
    tool_commit_live(p, ui);
    true
}

// tc_begin(doc, btn, ui): start a stroke of the current tool at the press point
fn tc_begin(p: &mut Pn, doc: DocId, btn: u32, inp: &UiInput) {
    let tool = tool_cur(&p.tool);
    let Some(d) = p.doc.get(doc) else { return };
    let Some(target) = doc_cur_layer(d) else { return };
    if target.sf.px.is_empty() {
        return;
    }
    let target_uid = target.uid;
    let (pxf, pyf) = ui_pt_fx(p, inp, inp.press_x, inp.press_y);
    if tool == TOOL_CLONE {
        // Klonen: the source layer; the offset is fixed by the first stroke after Strg+click
        let Some(src) = an_layer(&p.tclone, p.doc.d(doc)) else { return };
        let t = &mut p.tclone;
        t.sly = src;
        if !t.off_on {
            t.off_dx = t.an_x - (pxf >> 8);
            t.off_dy = t.an_y - (pyf >> 8);
            t.off_on = true;
        }
    } else if !rc_setup(p, doc, target_uid, btn, pxf, pyf) {
        // Neu färben: colours, threshold, the recoloured copy (filled per dirty area)
        return;
    }
    let c = p.tool.cfg;
    let mut fl = 0;
    if c.aa != 0 {
        fl |= BEF_AA as u32;
    }
    if c.pressure != 0 {
        fl |= BEF_PRESSURE as u32;
    }
    let bep = Bep { width: c.width, hard: c.hard, spacing: c.spacing, flags: fl };
    if !be_begin(p, doc, &bep) {
        p.tclone.sf = None; // tc_free
        return;
    }
    tl_init(p, TL_CLONE, doc);
    let t = &mut p.tool.tls[TL_CLONE];
    t.kind = TLK_SOURCE as u32;
    t.mask = TlMask::Be;
    if tool == TOOL_CLONE {
        let m = if c.blend < BM_COUNT { c.blend } else { 0 };
        t.mode = m | 255 << BM_OPAC_SHIFT;
        t.src = TlSrc::Layer(p.tclone.sly);
        t.sx = p.tclone.off_dx.wrapping_neg();
        t.sy = p.tclone.off_dy.wrapping_neg();
    } else {
        t.mode = BM_OVERWRITE | 255 << BM_OPAC_SHIFT;
        t.src = TlSrc::Clone;
    }
    be_dab(&mut p.be, pxf, pyf, be_ui_pressure(inp, -1));
    let t = &mut p.tclone;
    t.on = true;
    t.doc = Some(doc);
    t.btn = btn;
    t.tool = tool;
    tc_mark(p);
    tl_show(p, TL_CLONE);
    tool_live_begin(p, doc);
}

// tc_mark(): the area the engine changed since the last call: Neu färben fills its copy there
// first, then the tool layer recomposites it
fn tc_mark(p: &mut Pn) {
    let Some(r) = be_dirty(&mut p.be) else { return };
    if p.tclone.tool == TOOL_RECOLOR {
        rc_fill(p, r[0], r[1], r[2], r[3]);
    }
    tl_mark(p, TL_CLONE, r[0], r[1], r[2], r[3]);
}

// tc_commit(doc) -> step: the stroke into the layer (TD_COMMIT)
fn tc_commit(p: &mut Pn, ui: &mut Ui, _doc: DocId) -> bool {
    if !p.tclone.on {
        return false;
    }
    p.tclone.on = false;
    tc_mark(p);
    let (name, icon) = if p.tclone.tool == TOOL_CLONE {
        cl_snapshot(p);
        ("Klonen (Stempel)", IC_PICTURES)
    } else {
        ("Neu färben", IC_CRAYON)
    };
    let r = tl_commit(p, TL_CLONE, name, icon);
    be_end(&mut p.be);
    let t = &mut p.tool.tls[TL_CLONE];
    t.mask = TlMask::None;
    t.src = TlSrc::None;
    p.tclone.sf = None; // tc_free
    cx_update(p, ui);
    r
}

// tc_cancel(doc): drop the stroke (TD_CANCEL; doc None = the image is gone: its anchor too)
fn tc_cancel(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    if !p.tclone.on {
        return;
    }
    p.tclone.on = false;
    if doc.is_some() {
        tl_hide(p, TL_CLONE);
    } else if p.tclone.doc == p.tclone.an_doc {
        p.tclone.an_on = false;
    }
    be_end(&mut p.be);
    let t = &mut p.tool.tls[TL_CLONE];
    t.mask = TlMask::None;
    t.src = TlSrc::None;
    p.tclone.sf = None; // tc_free
    cx_update(p, ui);
}

// cl_snapshot(): Klonen onto its own source layer: tl_commit writes the layer in place (on
// several threads), so the source area of the stroke's box is copied first and TL_SRC points
// to the copy (pixels outside the layer stay outside: coverage 0)
fn cl_snapshot(p: &mut Pn) {
    let Some(doc) = p.tclone.doc else { return };
    let d = p.doc.d(doc);
    let Some(cur) = doc_cur_layer(d) else { return };
    if cur.uid != p.tclone.sly {
        return;
    }
    let t = &p.tclone;
    let b = p.tool.tls[TL_CLONE].bx;
    let sx0 = b[0].wrapping_add(t.off_dx).max(0);
    let sy0 = b[1].wrapping_add(t.off_dy).max(0);
    let sx1 = b[2].wrapping_add(t.off_dx).min(cur.sf.w);
    let sy1 = b[3].wrapping_add(t.off_dy).min(cur.sf.h);
    if sx0 >= sx1 || sy0 >= sy1 {
        p.tool.tls[TL_CLONE].src = TlSrc::None; // nothing to copy (source outside the layer)
        return;
    }
    let mut c = Surface::new(sx1 - sx0, sy1 - sy0, FMT_ARGB);
    if c.px.is_empty() {
        p.tool.tls[TL_CLONE].src = TlSrc::None;
        return;
    }
    let n = (sx1 - sx0) as usize * 4;
    for r in 0..(sy1 - sy0) as usize {
        let a = (sy0 as usize + r) * cur.sf.stride as usize + sx0 as usize * 4;
        let o = r * c.stride as usize;
        c.px[o..o + n].copy_from_slice(&cur.sf.px[a..a + n]);
    }
    let (odx, ody) = (t.off_dx, t.off_dy);
    p.tclone.sf = Some(c);
    let tl = &mut p.tool.tls[TL_CLONE];
    tl.src = TlSrc::Clone;
    tl.sx = sx0.wrapping_sub(odx);
    tl.sy = sy0.wrapping_sub(ody);
}

// ================================================================== Neu färben

#[inline]
fn int_scale(c: u32, f: u32) -> u32 {
    let t = c * f + 0x80;
    (t + (t >> 8)) >> 8
}

// pm_px(argb) -> premultiplied colour (alpha kept; INT_SCALE per channel)
fn pm_px(c: u32) -> u32 {
    let a = c >> 24;
    let mut r = c & 0xFF00_0000;
    for sh in [0, 8, 16] {
        r |= int_scale((c >> sh) & 255, a) << sh;
    }
    r
}

// rc_setup(layer, btn, x_fx, y_fx) -> true: old / new colour of the stroke, the comparison
// colour, the shift, the threshold and the recoloured copy of the layer's size (filled by
// rc_fill as the stroke grows)
fn rc_setup(p: &mut Pn, doc: DocId, layer: u64, btn: u32, x_fx: i32, y_fx: i32) -> bool {
    let new = tool_color(p, btn);
    let d = p.doc.d(doc);
    let Some(li) = lay_find_uid(d, layer) else { return false };
    let sf = &d.layers[li as usize].sf;
    let old = if p.tclone.rc_samp == RC_ONCE {
        // einmalig: the layer pixel under the press point (clamped into the layer)
        let x = (x_fx >> 8).max(0).min(sf.w - 1);
        let y = (y_fx >> 8).max(0).min(sf.h - 1);
        let o = y as usize * sf.stride as usize + x as usize * 4;
        u32::from_le_bytes([sf.px[o], sf.px[o + 1], sf.px[o + 2], sf.px[o + 3]])
    } else {
        // Sekundärfarbe: the other button's colour
        tool_color(p, if btn == BTN_RIGHT { BTN_LEFT } else { BTN_RIGHT })
    };
    let (w, h) = (sf.w, sf.h);
    let tol = p.tool.cfg.tol.min(100);
    let tol = (tol * 255 + 50) / 100;
    let t = &mut p.tclone;
    t.rc_ly = layer;
    t.rc_new = new;
    t.rc_old = old;
    t.rc_cmp = if t.rc_alpha == TA_PREMUL { pm_px(old) } else { old };
    // shift per channel: new - old (straight)
    for (k, sh) in [0u32, 8, 16].into_iter().enumerate() {
        t.rc_d[k] = ((new >> sh) & 255) as i32 - ((old >> sh) & 255) as i32;
    }
    // threshold: tol = round(pct * 255 / 100), sum <= tol * tol * 4
    t.rc_thr = tol * tol * 4;
    // the copy (every pixel the stroke covers is filled before it is shown)
    let c = Surface::new(w, h, FMT_ARGB);
    if c.px.is_empty() {
        return false;
    }
    t.sf = Some(c);
    true
}

// rc_px(argb): one pixel of the recoloured copy: shifted by (new - old) when it matches the
// comparison colour within the tolerance, else unchanged
fn rc_px(t: &TClone, c: u32) -> u32 {
    let mut f = c >> 24; // pixel alpha
    let ca = t.rc_cmp >> 24; // comparison alpha
    let da = ca.wrapping_sub(f);
    let mut sum = da.wrapping_mul(da); // (a.A - b.A)^2 + ...
    if t.rc_alpha != TA_PREMUL {
        f = 255; // Gerade: the channels are not scaled
    }
    for sh in [0, 8, 16] {
        let v = int_scale((c >> sh) & 255, f);
        let d = ((t.rc_cmp >> sh) & 255).wrapping_sub(v);
        sum = sum.wrapping_add(d.wrapping_mul(d).wrapping_add(1).wrapping_mul(ca) >> 8);
    }
    if sum > t.rc_thr {
        return c;
    }
    let mut r = c & 0xFF00_0000;
    for (k, sh) in [0u32, 8, 16].into_iter().enumerate() {
        r |= ((((c >> sh) & 255) as i32 + t.rc_d[k]).clamp(0, 255) as u32) << sh;
    }
    r
}

// rc_fill(x0, y0, x1, y1): the recoloured copy of that area (clipped to the layer)
fn rc_fill(p: &mut Pn, x0: i32, y0: i32, x1: i32, y1: i32) {
    let Some(doc) = p.tclone.doc else { return };
    let d = p.doc.d(doc);
    let Some(li) = lay_find_uid(d, p.tclone.rc_ly) else { return };
    let l = &d.layers[li as usize].sf;
    let t = &mut p.tclone;
    let x0 = x0.max(0);
    let y0 = y0.max(0);
    let x1 = x1.min(l.w);
    let y1 = y1.min(l.h);
    if x0 >= x1 {
        return;
    }
    let Some(mut c) = t.sf.take() else { return };
    for y in y0..y1 {
        let lo = y as usize * l.stride as usize;
        let co = y as usize * c.stride as usize;
        for x in x0..x1 {
            let a = lo + x as usize * 4;
            let v = u32::from_le_bytes([l.px[a], l.px[a + 1], l.px[a + 2], l.px[a + 3]]);
            let b = co + x as usize * 4;
            c.px[b..b + 4].copy_from_slice(&rc_px(t, v).to_le_bytes());
        }
    }
    t.sf = Some(c);
}

// ================================================================== options, records

// clone_options(): Pinselgröße | Härte, Abstand | Füllmethode, Antialiasing
fn clone_options(p: &mut Pn, ui: &mut Ui) {
    opt_width(p, ui);
    opt_sep(ui);
    opt_hardness(p, ui);
    opt_spacing(p, ui);
    opt_sep(ui);
    opt_blend(p, ui);
    opt_aa(p, ui);
    opt_pressure(p, ui); // (PN1) last: the row keeps its layout
    cl_records(p, ui);
}

// recolor_options(): Pinselgröße | Härte, Abstand | Toleranz, Alpha-Modus der Toleranz,
// Probennahme | Antialiasing
fn recolor_options(p: &mut Pn, ui: &mut Ui) {
    opt_width(p, ui);
    opt_sep(ui);
    opt_hardness(p, ui);
    opt_spacing(p, ui);
    opt_sep(ui);
    opt_tolerance(p, ui);
    let mut v = p.tclone.rc_alpha;
    opt_menu(p, ui, ID_RC_ALPHA, &mut v, &ALPHA_LABELS, &ALPHA_ICONS, Some("Alpha-Modus der Toleranz"), OMF_ICONONLY as u32);
    p.tclone.rc_alpha = v;
    let mut v = p.tclone.rc_samp;
    opt_menu(p, ui, ID_RC_SAMP, &mut v, &SAMP_LABELS, &SAMP_ICONS, None, OMF_ICONONLY as u32);
    p.tclone.rc_samp = v;
    opt_sep(ui);
    opt_aa(p, ui);
    opt_pressure(p, ui); // (PN1) last: the row keeps its layout
    rc_records(p, ui);
}

// cl_records(): anchor "x,y,layer", offset "dx,dy", crosshair "sx,sy"
fn cl_records(p: &Pn, ui: &mut Ui) {
    let t = &p.tclone;
    let d = doc_active(&p.doc);
    let ok = an_valid(t, d);
    let li = match d {
        Some(d) if ok => an_find(t, p.doc.d(d)),
        _ => -1,
    };
    t1_sem(ui, ID_TC_ANCHOR, &format!("{},{},{}", t.an_x, t.an_y, li), ok as i32);
    t1_sem(ui, ID_TC_OFFSET, &format!("{},{}", t.off_dx, t.off_dy), t.off_on as i32);
    t1_sem(ui, ID_TC_CROSS, &format!("{},{}", t.cx_sx, t.cx_sy), t.cx_vis as i32);
}

// rc_records(): options "sampling,alpha" (value stroke running), colours "OLD,NEW"
fn rc_records(p: &Pn, ui: &mut Ui) {
    let t = &p.tclone;
    t1_sem(ui, ID_TC_RCOPT, &format!("{},{}", t.rc_samp as i32, t.rc_alpha as i32), t.on as i32);
    t1_sem(ui, ID_TC_RCCOL, &format!("{:08X},{:08X}", t.rc_old, t.rc_new), 0);
}
