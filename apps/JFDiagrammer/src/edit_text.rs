// edit_text -- (JD7) the text of a shape edited in place (TX-01): a ui.richtext box over the
// shape, started by a double click, F2, Enter, a typed character (it replaces the text, as in
// Visio) or the text tool; the text goes into the shape when the box loses the focus (a click
// elsewhere, Esc, Tab), as one undo step, and the formula engine computes what depends on it
// (TxtHeight of a text box that grows with its text)
//
//   begin(app, key, ch)     start editing the shape `key` of the page shown
//   frame(app, ui)          the box, every pass while editing; ends the editing
//
// Paragraphs are the text's lines; the shape's first character and paragraph formats stay
// (formatted runs in the editor are written as plain text in P1, TX-02 keeps what the file had
// as long as the text is not edited).

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{ops, Text};
use ui::i18n::t;
use ui::*;

use crate::edit_canvas::{self, shape_of};
use crate::edit_doc;
use crate::Diagrammer;

const ID_TEXT: u32 = 0x5C00;

/// the text being edited
pub(crate) struct TextEdit {
    pub key: u64,
    pub doc: RichDoc,
    /// the text it started with (unchanged: nothing is written)
    pub before: String,
    focused: bool,
}

/// start editing the text of shape `key` (ch: a typed character that replaces the text)
pub(crate) fn begin(app: &mut Diagrammer, ui: &mut Ui, key: u64, ch: Option<char>) {
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return };
    let s = shape_of(page, key);
    let Some(shape) = o.doc.shape(s) else { return };
    // locked text (LockTextEdit): nothing
    let before = shape.text().map(|t| t.chars()).unwrap_or_default();
    // Visio ends a text with a line feed of its own: not a paragraph of the editor
    let shown = before.strip_suffix('\n').unwrap_or(&before).replace('\u{2028}', "\n").replace('\u{2029}', "\n");
    let text = match ch {
        Some(c) => {
            let mut s = String::new();
            s.push(c);
            s
        }
        None => shown,
    };
    let mut doc = RichDoc::from_plain(&text);
    let end = doc.end();
    doc.set_selection(TextSel { anchor: if ch.is_some() { end } else { TextPos { line: 0, byte: 0 } }, caret: end });
    doc.clear_history();
    app.text = Some(TextEdit { key, doc, before, focused: false });
    app.se.select(&[key]);
    ui.repass();
}

/// the edit box over the shape (every pass while editing)
pub(crate) fn frame(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(te) = app.text.as_mut() else { return };
    let Some(sh) = app.model.shapes.iter().find(|s| s.key == te.key) else {
        app.text = None;
        return;
    };
    // the shape's box on the screen, at least wide and tall enough to type in
    let vp = app.canvas.vp;
    let b = if sh.flags & SEF_1D != 0 {
        let (a, z) = (sh.ends[0], sh.ends[1]);
        let c = [(a[0] + z[0]) * 0.5, (a[1] + z[1]) * 0.5];
        [c[0] - 60.0, c[1] - 14.0, c[0] + 60.0, c[1] + 14.0]
    } else {
        sh.bx.aabb()
    };
    let p0 = vp.to_screen([b[0], b[1]]);
    let p1 = vp.to_screen([b[2], b[3]]);
    let (minw, minh) = (ui.dp(120), ui.dp(36));
    let (mut x, mut y, mut w, mut h) = (p0[0] as i32, p0[1] as i32, (p1[0] - p0[0]) as i32, (p1[1] - p0[1]) as i32);
    if w < minw {
        x -= (minw - w) / 2;
        w = minw;
    }
    if h < minh {
        y -= (minh - h) / 2;
        h = minh;
    }
    ui.next_rect_px(x, y, w, h);
    let ev = ui.richtext(ID_TEXT, &mut te.doc, RT_PLAIN_PASTE);
    if !te.focused {
        ui.focus(ID_TEXT);
        te.focused = true;
        ui.repass();
        return;
    }
    if ev & EV_COMMIT != 0 || (ui.focused() != ID_TEXT && ev & EV_FOCUS == 0) {
        end(app, ui);
    }
}

// write the text into the shape (when it changed) and stop editing
fn end(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(te) = app.text.take() else { return };
    let (Some(o), Some(page)) = (app.opened.as_mut(), app.model.page) else { return };
    let mut text: String = te.doc.paras().iter().map(|p| p.text()).collect::<Vec<_>>().join("\n");
    if text == te.before.strip_suffix('\n').unwrap_or(&te.before) {
        ui.repass();
        return;
    }
    if !text.is_empty() {
        text.push('\n');
    }
    let s = shape_of(page, te.key);
    let owner = diagram_core::Owner::Page(page);
    let _ = edit_doc::edit(o, t("Text bearbeiten"), true, |tx, eng| {
        let new = keep_marks(tx.doc().shape(s).and_then(|x| x.text().cloned()), &text);
        ops::set_text(tx, eng, s, new)?;
        edit_canvas::follow(tx, eng, owner, &[s])
    });
    ui.repass();
}

// the new text with the first character and paragraph marks of the old one
fn keep_marks(old: Option<Text>, text: &str) -> Text {
    use diagram_core::TextItem;
    let mut items = Vec::new();
    if let Some(old) = old {
        for it in old.items.iter() {
            match it {
                TextItem::Cp(_) | TextItem::Pp(_) | TextItem::Tp(_) => {
                    if !items.iter().any(|x: &TextItem| core::mem::discriminant(x) == core::mem::discriminant(it)) {
                        items.push(it.clone());
                    }
                }
                _ => {}
            }
        }
    }
    let mut t = Text::plain(text);
    items.append(&mut t.items);
    t.items = items;
    t
}
