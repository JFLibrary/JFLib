// labels -- slim texts on the canvas (H3): notes of the style "label" and the input they are
// typed in, right on the canvas without a flyout
//
//   Input                       what is being typed: a new label at a place, or a label's new text
//   open_at(n, ui, at)          a double click on free canvas: the input with its first line at
//                               the world point
//   open_centre(n, ui)          Beschriftung einfügen (the keyboard's way): at the middle of the
//                               view, below what lies there
//   Input::edit(note)           canvas.open_rename of a label (a double click, Enter, F2)
//   frame(n, ui)                the input over the canvas: its keys, a click elsewhere
//   flush(n, ui)                what is being typed counts now (before a command acts)
//   box_of(ui, text)            the box of a text in world units
//   set_text(n, ui, id, text)   a label's new text with its box, one step (the inspector)
//   refit(n, ui) -> changed     the boxes after another font or text size
//
// A label is text only: its box is the text with a hair around it (LABEL_PAD), measured where
// the text is set -- Scene::build has no fonts -- and stored in the note's w and h.  World units
// are DIP at 100 % and text widths scale with the font, so the box holds at any zoom and any
// display scale; another font or text size measures the labels anew (refit, also after a file
// was opened).  That only moves the boxes, it is no change of the project.
//
// Nothing exists until the text is taken: Enter or a click elsewhere create the label with its
// text and box in one undo step, Esc leaves nothing behind.  An empty text is never stored: on a
// label that exists it deletes the label.  Shift+Enter gives a new line.  While typing the label
// is not drawn and not selected, so nothing shows behind the input; the input shows the text as
// it will be, from 100 % upwards (below that it stays at the size of 100 %, to be read).

use alloc::string::String;

use jfn_model::{Note, NoteStyle};
use ui::*;

use super::paint::{fl, line_h, LABEL_PAD, LABEL_TS};
use crate::app::constants::*;
use crate::app::tr::t;
use crate::app::Net;
use crate::doc::edits;

/// what is being typed
pub(crate) struct Input {
    /// the label being edited (None: a new one)
    pub id: Option<u32>,
    /// the top left corner of its box (world units)
    pub at: [i32; 2],
    pub text: String,
    /// the focus was asked for; the text is to be selected; the press that opened it has ended
    opened: bool,
    select: bool,
    armed: bool,
    /// opened with the pointer: the keyboard goes back without a focus ring
    quiet: bool,
}

impl Input {
    fn new(at: [i32; 2]) -> Input {
        Input { id: None, at, text: String::new(), opened: false, select: false, armed: false, quiet: true }
    }

    /// the input over a label, its text selected
    pub(crate) fn edit(n: &Note) -> Input {
        Input { id: Some(n.id), text: n.text.clone(), select: true, ..Input::new([n.x, n.y]) }
    }
}

/// the box w, h of a text in world units (a hair around it)
pub(crate) fn box_of(ui: &mut Ui, text: &str) -> (i32, i32) {
    let old = ui.text.get_font();
    ui.ts_font(LABEL_TS);
    let (mut w, mut lines) = (0, 0);
    for line in text.split('\n') {
        w = w.max(ui.text.measure(line));
        lines += 1;
    }
    let lh = line_h(ui.text.ascent(), ui.text.descent());
    ui.text.set_font(old.0, old.1);
    let k = 96.0 / ui.dp(96).max(1) as f32;
    let pad = (LABEL_PAD * 2.0) as i32;
    (up(w as f32 / 256.0 * k) + pad, up((lines * lh) as f32 * k) + pad)
}

// round up to whole world units
fn up(v: f32) -> i32 {
    -fl(-v)
}

// round to whole world units
fn rnd(v: f32) -> i32 {
    fl(v + 0.5)
}

/// a double click on free canvas: the input with its first line at the world point
pub(crate) fn open_at(n: &mut Net, ui: &mut Ui, at: [f32; 2]) {
    let (_, h) = box_of(ui, "x");
    n.canvas.label = Some(Input::new([rnd(at[0] - LABEL_PAD), rnd(at[1]) - h / 2]));
    n.canvas.rename = None;
    ui.repass();
}

/// Beschriftung einfügen: at the middle of the view, below what lies there (a label lies under
/// the devices and would hide behind one)
pub(crate) fn open_centre(n: &mut Net, ui: &mut Ui) {
    let vp = n.canvas.st.vp;
    let (w, h) = box_of(ui, t("Beschriftung"));
    let mut at = [rnd(vp.cx as f32 - LABEL_PAD), rnd(vp.cy as f32) - h / 2];
    for _ in 0..40 {
        let (x0, y0) = (at[0] as f32, at[1] as f32);
        let (x1, y1) = (x0 + w as f32, y0 + h as f32);
        let on = |r: [f32; 4]| x0 < r[0] + r[2] && r[0] < x1 && y0 < r[1] + r[3] && r[1] < y1;
        if !n.canvas.scene.nodes.iter().any(|x| x.flags & (DN_FRAME | DN_OUTLINE) == 0 && on(x.rect)) {
            break;
        }
        at[1] += h;
    }
    n.canvas.label = Some(Input::new(at));
    n.canvas.rename = None;
    ui.repass();
}

/// the text is taken now, the label stays selected when `keyed` (Enter); a click elsewhere leaves
/// the selection to the click
fn take(n: &mut Net, ui: &mut Ui, keyed: bool) {
    let Some(l) = n.canvas.label.take() else { return };
    let text = l.text.trim_end();
    match l.id {
        None if text.is_empty() => {}
        None => {
            let (w, h) = box_of(ui, text);
            let (id, e) = edits::add_label(&mut n.doc.proj, l.at[0], l.at[1], w, h, text);
            n.doc.record(e);
            if keyed {
                n.canvas.select_only(id);
            }
        }
        Some(id) => {
            set_text(n, ui, id, text);
            if keyed && !text.is_empty() {
                n.canvas.select_only(id);
            }
        }
    }
    back(ui, l.quiet);
}

/// a label's new text with its box, one step; an empty text deletes it
pub(crate) fn set_text(n: &mut Net, ui: &mut Ui, id: u32, text: &str) {
    if text.is_empty() {
        if let Some(e) = edits::delete(&n.doc.proj, &[id]) {
            n.doc.apply(e);
        }
    } else {
        let (w, h) = box_of(ui, text);
        if let Some(e) = edits::label_edit(&n.doc.proj, id, text, w, h) {
            n.doc.apply(e);
        }
    }
    ui.repass();
}

// the keyboard goes back to the canvas, unless a click gave it to something else
fn back(ui: &mut Ui, quiet: bool) {
    if ui.focused() == ID_LABEL_BOX {
        if quiet {
            ui.focus_quiet(ID_CANVAS);
        } else {
            ui.focus(ID_CANVAS);
        }
    }
    ui.repass();
}

/// what is being typed counts now (a command is about to act)
pub(crate) fn flush(n: &mut Net, ui: &mut Ui) {
    take(n, ui, false);
}

/// the input over the canvas, after the canvas' events
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    // the label went away under it (undo, another project)
    if n.canvas.label.as_ref().and_then(|l| l.id).is_some_and(|id| n.doc.proj.note(id).is_none()) {
        n.canvas.label = None;
    }
    let vp = n.canvas.st.vp;
    let c = vp.rect;
    let Some(l) = &mut n.canvas.label else { return };
    // the font as it will be drawn, not smaller than at 100 %
    let old = ui.text.get_font();
    ui.ts_font(LABEL_TS);
    let (font, size) = ui.text.get_font();
    let size = (size as f32 * vp.zoom.max(1.0)) as i32;
    ui.text.set_font(font, size);
    let (asc, desc) = (ui.text.ascent(), ui.text.descent());
    let lh = line_h(asc, desc).max(1);
    let shown = if l.text.is_empty() { t("Beschriftung") } else { l.text.as_str() };
    let (mut tw, mut lines) = (0, 0);
    for line in shown.split('\n') {
        tw = tw.max(ui.text.width(line));
        lines += 1;
    }
    ui.text.set_font(old.0, old.1);
    // where the text lies as drawn (px), inside the canvas; the canvas is not mirrored, hit boxes
    // and records are logical
    let hair = ui.hair();
    let m = fl(LABEL_PAD * vp.scale() as f32 + 0.5).max(hair);
    let p = vp.to_screen([l.at[0] as f32 + LABEL_PAD, l.at[1] as f32 + LABEL_PAD]);
    let (x, y) = (fl(p[0] + 0.5), fl(p[1] + 0.5));
    let full = tw + ui.dp(2); // and the caret
    let w = full.min(c[2] - m * 2).max(ui.dp(8));
    let h = (lines * lh).min(c[3] - m * 2).max(lh);
    let lx = if ui.rtl() { ui.rtl_map(x + w - 1, c[0], c[2]) } else { x };
    let (bg, text, accent) = (ui.color(T_BG) | 0xFF00_0000, ui.color(T_TEXT), ui.color(T_ACCENT));
    if !l.opened {
        l.opened = true;
        l.quiet = !ui.focus_ring(EV_FOCUS);
        ui.focus(ID_LABEL_BOX);
    }
    // Enter (Ctrl+Enter, the text box's own way) takes it, Shift+Enter goes on to the text box as a
    // new line; Esc gives up (the text box would only drop the focus).  Not by its EV_COMMIT: a box
    // that held the edit state when another one took it reports that when it is drawn again.
    let mut act = 0; // 1 Enter, 2 Esc, 3 elsewhere
    if ui.focused() == ID_LABEL_BOX {
        if ui.capture_shortcut(VK_RETURN, 0) || ui.capture_shortcut(VK_RETURN, MOD_CTRL) {
            act = 1;
        } else if ui.capture_shortcut(VK_ESCAPE, 0) {
            act = 2;
        }
    }
    // a press elsewhere or the focus gone elsewhere take it, not the press that opened it
    l.armed = l.armed || ui.mbtn() & (1 << BTN_LEFT) == 0;
    let (mx, my) = ui.mouse();
    let over = mx >= lx - m && my >= y - m && mx < lx + w + m && my < y + h + m;
    if act == 0 && l.armed && ((ui.pressed() != 0 && !over) || ui.focused() != ID_LABEL_BOX) {
        act = 3;
    }
    ui.clip_push(c[0], c[1], c[2], c[3]);
    ui.cmd_rrect(lx - m, y - m, w + m * 2, h + m * 2, ui.dp(2), hair, bg, accent, accent);
    ui.next_rect_px(lx, y, w, h);
    // a character typed in this pass scrolled the box, now it fits: from its start again
    if w >= full {
        ui.slot(ID_LABEL_BOX).scrollx = 0;
    }
    let look = TextboxPresentation { font, size, line_height: lh, baseline: asc, color: text, background: bg, underline: false };
    ui.textbox_presented(ID_LABEL_BOX, Some(t("Beschriftung")), &mut l.text, 2000, look);
    ui.clip_pop();
    if l.select && ui.textbox_select(ID_LABEL_BOX, &l.text, 0, l.text.len()) {
        l.select = false;
    }
    match act {
        1 => take(n, ui, true),
        2 => {
            // a label that exists stays as it was, and selected
            let id = l.id;
            let quiet = l.quiet;
            n.canvas.label = None;
            if let Some(id) = id {
                n.canvas.select_only(id);
            }
            back(ui, quiet);
        }
        3 => take(n, ui, false),
        _ => {}
    }
}

/// the boxes after another font or text size: every label measured anew (not a step: the text is
/// the same, its box is only where the font puts it) -> a box changed
pub(crate) fn refit(n: &mut Net, ui: &mut Ui) -> bool {
    let dpi = ui.dp(96).max(1);
    let old = ui.text.get_font();
    ui.ts_font(LABEL_TS);
    let (font, size) = ui.text.get_font();
    ui.text.set_font(old.0, old.1);
    // the size at 100 %, a quarter px is no other size
    let key = (font, (size as i64 * 96 / dpi as i64 + 32) as i32 >> 6);
    if n.canvas.label_font == Some(key) {
        return false;
    }
    n.canvas.label_font = Some(key);
    let mut changed = false;
    for k in 0..n.doc.proj.notes.len() {
        let o = &n.doc.proj.notes[k];
        if o.style != NoteStyle::Label || o.text.is_empty() {
            continue;
        }
        let text = o.text.clone();
        let (w, h) = box_of(ui, &text);
        let o = &mut n.doc.proj.notes[k];
        // (a unit or two is rounding at another display scale)
        if (w - o.w).abs() > 2 || (h - o.h).abs() > 2 {
            o.w = w;
            o.h = h;
            changed = true;
        }
    }
    changed
}
