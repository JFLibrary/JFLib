// t_brush -- PaintCraft: Farbpinsel, Radierer, Zeichenstift (agent T1).  Sub-module of
// tool.rs (tool_init calls t_brush_init).  A stroke accumulates its coverage in the brush
// engine's mask (max of the dabs) and is shown / written through one tool layer (TL):
//   Farbpinsel   TLK_SOLID: colour of the button (left = primary, right = secondary; its
//                alpha = stroke opacity), fill style, blend mode; width, hardness, spacing,
//                path smoothing, antialiasing, pen pressure scales the width
//   Radierer     TLK_ERASE: alpha *= 1 - coverage * (alpha of the button's colour)
//   Zeichenstift TLK_SOLID, 1 px aliased lines (Bresenham), blend mode incl. Überschreiben
// The stroke is a pending edit (tool_live_begin) from the press to the release: one history
// step ("Farbpinsel" / "Radierer" / "Zeichenstift"); Esc cancels it.
//                                              [apps/PaintCraft/src/t_brush.s]

use ui::*;

use crate::tools::brushes::{be_begin, be_dab, be_dirty, be_end, be_finish, be_to, be_ui_pressure, Bep};
use crate::app::constants::*;
use crate::document::model::DocId;
use crate::tools::*;
use crate::workspace::view::view_screen_to_doc_fx;
use crate::app::Pn;

/// the state of t_brush.s (its g_* globals; the tool layer is p.tool.tls[TL_BRUSH])
#[derive(Default)]
pub(crate) struct TBrush {
    doc: Option<DocId>,
    /// 1 while a stroke is drawn
    on: bool,
    /// BTN_LEFT / BTN_RIGHT
    btn: u32,
    /// TOOL_BRUSH / ERASER / PENCIL
    tool: i32,
}

static DESC_BRUSH: ToolDesc = ToolDesc {
    flags: (TF_CLIP_SEL | TF_USES_WIDTH | TF_AA | TF_BLEND | TF_FILL | TF_HARD | TF_OUTLINE) as u32,
    input: Some(br_input),
    options: Some(brush_options),
    commit: Some(br_commit),
    cancel: Some(br_cancel),
    ..ToolDesc::NONE
};
static DESC_ERASER: ToolDesc = ToolDesc {
    flags: (TF_CLIP_SEL | TF_USES_WIDTH | TF_AA | TF_HARD | TF_OUTLINE) as u32,
    input: Some(br_input),
    options: Some(eraser_options),
    commit: Some(br_commit),
    cancel: Some(br_cancel),
    ..ToolDesc::NONE
};
static DESC_PENCIL: ToolDesc = ToolDesc {
    flags: (TF_CLIP_SEL | TF_BLEND) as u32,
    input: Some(br_input),
    options: Some(pencil_options),
    commit: Some(br_commit),
    cancel: Some(br_cancel),
    ..ToolDesc::NONE
};

/// t_brush_init() -- register the three tools
pub(crate) fn t_brush_init(p: &mut Pn, _ui: &mut Ui) {
    tool_register(&mut p.tool, TOOL_BRUSH, Some(&DESC_BRUSH));
    tool_register(&mut p.tool, TOOL_ERASER, Some(&DESC_ERASER));
    tool_register(&mut p.tool, TOOL_PENCIL, Some(&DESC_PENCIL));
}

/// ui_pt_fx(ui, x, y) -> (x_fx, y_fx): canvas-relative screen point -> image 24.8
pub(crate) fn ui_pt_fx(p: &Pn, inp: &UiInput, x: i32, y: i32) -> (i32, i32) {
    let sx = ((inp.rect[0] + x) << 8) + 128;
    let sy = ((inp.rect[1] + y) << 8) + 128;
    view_screen_to_doc_fx(&p.view, sx, sy)
}

// br_begin(doc, btn, ui): start a stroke of the current tool at the press point
fn br_begin(p: &mut Pn, doc: DocId, btn: u32, inp: &UiInput) {
    p.tbrush.btn = btn;
    let tool = tool_cur(&p.tool);
    p.tbrush.tool = tool;
    let c = p.tool.cfg;
    let mut fl = 0;
    if c.aa != 0 {
        fl |= BEF_AA as u32;
    }
    if c.pressure != 0 {
        fl |= BEF_PRESSURE as u32;
    }
    if c.smooth != 0 {
        fl |= BEF_SMOOTH as u32;
    }
    if tool == TOOL_PENCIL {
        fl = BEF_PENCIL as u32;
    }
    let bep = Bep { width: c.width, hard: c.hard, spacing: c.spacing, flags: fl };
    if !be_begin(p, doc, &bep) {
        return;
    }
    if tool != TOOL_PENCIL { p.be.configure(&p.custom_brush.dynamics); }
    tl_init(p, TL_BRUSH, doc);
    tl_set_paint(p, TL_BRUSH, btn);
    let t = &mut p.tool.tls[TL_BRUSH];
    t.mask = TlMask::Be;
    if tool == TOOL_PENCIL {
        t.fill = FS_SOLID as u32;
    }
    if tool == TOOL_ERASER {
        t.kind = TLK_ERASE as u32;
        t.strength = t.argb >> 24;
    }
    // the first dab at the press position
    let (x, y) = ui_pt_fx(p, inp, inp.press_x, inp.press_y);
    be_dab(&mut p.be, x, y, be_ui_pressure(inp, -1));
    p.tbrush.on = true;
    p.tbrush.doc = Some(doc);
    br_mark(p);
    tl_show(p, TL_BRUSH);
    tool_live_begin(p, doc);
}

// br_mark(): the area the engine changed goes to the tool layer
fn br_mark(p: &mut Pn) {
    if let Some(r) = be_dirty(&mut p.be) {
        tl_mark(p, TL_BRUSH, r[0], r[1], r[2], r[3]);
    }
}

// br_input(doc, ui, x_fx, y_fx) -> changed (TD_INPUT of all three tools)
fn br_input(p: &mut Pn, ui: &mut Ui, doc: DocId, inp: &UiInput, x: i32, y: i32) -> bool {
    if !p.tbrush.on {
        // a press starts a stroke (left = primary, right = secondary)
        let btn = if inp.pressed & (1 << BTN_LEFT) != 0 {
            BTN_LEFT
        } else if inp.pressed & (1 << BTN_RIGHT) != 0 {
            BTN_RIGHT
        } else {
            return false;
        };
        br_begin(p, doc, btn, inp);
        if !p.tbrush.on {
            return false;
        }
        // the pointer may have moved in the press frame already
        be_to(&mut p.be, x, y, be_ui_pressure(inp, -1));
        br_mark(p);
        return true;
    }
    if Some(doc) != p.tbrush.doc {
        return false;
    }
    // every move point of this frame, then the pointer (repeated passes of a frame see the
    // same points: nothing changes then, so the result stays false -- no endless repass)
    let mut ch = false;
    for (i, pt) in inp.pts.iter().enumerate() {
        let (px, py) = ui_pt_fx(p, inp, pt[0], pt[1]);
        ch |= be_to(&mut p.be, px, py, be_ui_pressure(inp, i as i32)); // (PN1) the point's own pressure
    }
    ch |= be_to(&mut p.be, x, y, be_ui_pressure(inp, -1));
    br_mark(p);
    // released (or the button is no longer held): the stroke is finished
    let m = 1 << p.tbrush.btn;
    if inp.btn & m != 0 && inp.released & m == 0 {
        return ch;
    }
    tool_commit_live(p, ui);
    true
}

// br_commit(doc) -> step: the stroke into the layer (TD_COMMIT)
fn br_commit(p: &mut Pn, _ui: &mut Ui, _doc: DocId) -> bool {
    if !p.tbrush.on {
        return false;
    }
    p.tbrush.on = false;
    be_finish(&mut p.be); // the last piece of a smoothed path
    br_mark(p);
    let (name, icon) = match p.tbrush.tool {
        TOOL_ERASER => ("Radierer", IC_ERASER),
        TOOL_PENCIL => ("Zeichenstift", IC_PENCIL),
        _ => ("Farbpinsel", IC_BRUSH),
    };
    let r = tl_commit(p, TL_BRUSH, name, icon);
    be_end(&mut p.be);
    p.tool.tls[TL_BRUSH].mask = TlMask::None;
    r
}

// br_cancel(doc): drop the stroke (TD_CANCEL; doc None = the image is gone)
fn br_cancel(p: &mut Pn, _ui: &mut Ui, doc: Option<DocId>) {
    if !p.tbrush.on {
        return;
    }
    p.tbrush.on = false;
    if doc.is_some() {
        tl_hide(p, TL_BRUSH);
    }
    be_end(&mut p.be);
    p.tool.tls[TL_BRUSH].mask = TlMask::None;
}

// options rows
fn brush_options(p: &mut Pn, ui: &mut Ui) {
    crate::tools::brushes::custom::button(p, ui);
    opt_width(p, ui);
    opt_sep(ui);
    opt_hardness(p, ui);
    opt_spacing(p, ui);
    opt_sep(ui);
    opt_fill(p, ui);
    opt_sep(ui);
    opt_smooth(p, ui);
    opt_blend(p, ui);
    opt_aa(p, ui);
    opt_pressure(p, ui); // (PN1) last: the row keeps its layout
}

fn eraser_options(p: &mut Pn, ui: &mut Ui) {
    crate::tools::brushes::custom::button(p, ui);
    opt_width(p, ui);
    opt_sep(ui);
    opt_hardness(p, ui);
    opt_spacing(p, ui);
    opt_sep(ui);
    opt_smooth(p, ui);
    opt_aa(p, ui);
    opt_pressure(p, ui); // (PN1) last: the row keeps its layout
}

fn pencil_options(p: &mut Pn, ui: &mut Ui) {
    opt_blend(p, ui);
}
