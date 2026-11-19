// edit_text -- (JD7) the text of a shape edited in place (TX-01): a ui.richtext box over the
// shape, started by a double click, F2, Enter, a typed character (it replaces the text, as in
// Visio) or the text tool; the text goes into the shape when the box loses the focus (a click
// elsewhere, Esc, Tab), as one undo step, and the formula engine computes what depends on it
// (TxtHeight of a text box that grows with its text)
//
//   begin(app, key, ch)     start editing the shape `key` of the page shown
//   frame(app, ui)          the box, every pass while editing; ends the editing
//   finish(app, ui)         (JD8 rest) end the editing now (a dialog opens over it)
//   caret_of(doc) -> usize  the caret as a count of shown characters (lines joined by LF)
//
// Paragraphs are the text's lines; the shape's first character and paragraph formats stay
// (formatted runs in the editor are written as plain text in P1, TX-02 keeps what the file had
// as long as the text is not edited).
//
// (JD8 rest) Fields (TX-06) show their text in the editor and stay fields as long as their text
// is not touched: the text before the first and after the last change keeps its fields, a field
// in the changed part becomes the characters it showed.  The caret at the end goes to
// TextCmds::last, so that a field or a link asked for next goes there.  A click on the register
// row or on a command of the register Textwerkzeuge does not end the editing: the register stays,
// the command runs with the text being edited, the editor gets the focus back; the File register
// ends it (cmds_text::frame).  TextCmds::caret places the caret of the next start.  A vertical text
// (TextDirection 1) is edited in a box across the shape.

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{field, ops, CellAddr, SheetRef, Text, TextItem};
use ui::i18n::t;
use ui::*;

use crate::commands::*;
use crate::edit_canvas::{self, shape_of};
use crate::edit_doc;
use crate::Diagrammer;

const ID_TEXT: u32 = 0x5C00;

/// a piece of the text being edited: characters, or a field shown as its text (kept whole)
#[derive(Clone, Debug)]
enum Atom {
    Chars(String),
    Field(TextItem, String),
}

impl Atom {
    fn text(&self) -> &str {
        match self {
            Atom::Chars(s) | Atom::Field(_, s) => s,
        }
    }
}

/// the text being edited
pub(crate) struct TextEdit {
    pub key: u64,
    pub doc: RichDoc,
    /// the text it started with (unchanged: nothing is written)
    before: Vec<Atom>,
    /// a text box the text tool just made: left empty, it goes again (Visio's)
    pub fresh: bool,
    focused: bool,
}

/// start editing the text of shape `key` (ch: a typed character that replaces the text)
pub(crate) fn begin(app: &mut Diagrammer, ui: &mut Ui, key: u64, ch: Option<char>) {
    begin_with(app, ui, key, ch, false);
}

// the shape's text as atoms in the editor's form (U+2028 and U+2029 as LF, without Visio's
// end mark)
fn atoms(doc: &diagram_core::Document, s: diagram_core::ShapeRef) -> Vec<Atom> {
    let Some(text) = doc.shape(s).and_then(|x| x.text()) else { return Vec::new() };
    let lf = |t: &str| t.replace(['\u{2028}', '\u{2029}'], "\n");
    let mut out: Vec<Atom> = field::segments_of(doc, SheetRef::Shape(s), text)
        .into_iter()
        .map(|g| match g {
            field::Seg::Text(t) => Atom::Chars(lf(&t)),
            field::Seg::Field { item, text: shown } => Atom::Field(text.items[item].clone(), lf(&shown)),
        })
        .collect();
    if let Some(Atom::Chars(last)) = out.last_mut() {
        if last.ends_with('\n') {
            last.pop();
        }
        if last.is_empty() {
            out.pop();
        }
    }
    out
}

fn joined(a: &[Atom]) -> String {
    a.iter().map(|x| x.text()).collect()
}

/// the same for a text box the text tool just made (`fresh`: it goes again when left empty)
pub(crate) fn begin_with(app: &mut Diagrammer, ui: &mut Ui, key: u64, ch: Option<char>, fresh: bool) {
    let caret = app.txt.caret.take();
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return };
    let s = shape_of(page, key);
    if o.doc.shape(s).is_none() {
        return;
    }
    let before = atoms(&o.doc, s);
    let shown = joined(&before);
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
    let sel = match (ch, caret) {
        (Some(_), _) => TextSel { anchor: end, caret: end },
        (None, Some(k)) => {
            let p = pos_of(&doc, k);
            TextSel { anchor: p, caret: p }
        }
        (None, None) => TextSel { anchor: TextPos { line: 0, byte: 0 }, caret: end },
    };
    doc.set_selection(sel);
    doc.clear_history();
    app.text = Some(TextEdit { key, doc, before, fresh, focused: false });
    app.se.select(&[key]);
    ui.repass();
}

/// the caret of an editor as shown characters (lines joined by LF)
pub(crate) fn caret_of(doc: &RichDoc) -> usize {
    let c = doc.selection().caret;
    let paras = doc.paras();
    let mut n = 0;
    for (i, p) in paras.iter().enumerate() {
        let t = p.text();
        if i == c.line {
            return n + t.get(..c.byte.min(t.len())).map_or(0, |x| x.chars().count());
        }
        n += t.chars().count() + 1;
    }
    n.saturating_sub(1)
}

// the editor position of a count of shown characters
fn pos_of(doc: &RichDoc, mut k: usize) -> TextPos {
    let paras = doc.paras();
    for (i, p) in paras.iter().enumerate() {
        let t = p.text();
        let n = t.chars().count();
        if k <= n || i + 1 == paras.len() {
            let byte = t.char_indices().nth(k).map_or(t.len(), |(b, _)| b);
            return TextPos { line: i, byte };
        }
        k -= n + 1;
    }
    doc.end()
}

/// the edit box over the shape (every pass while editing)
pub(crate) fn frame(app: &mut Diagrammer, ui: &mut Ui) {
    let vertical = app.text.as_ref().is_some_and(|te| is_vertical(app, te.key));
    if app.text.is_none() {
        return;
    }
    // (JD8 rest) the shortcuts of Link and Text (Ctrl+K, F11 in German; KB-10, KB-13) reach their
    // commands while the box has the keys
    let k = ui.key_first();
    if k != 0 {
        let (vk, mods) = (k & 0xFFFF, k >> 16);
        let keys = crate::commands::shortcuts(crate::cmds_text::lang_col(), app.opts.ini());
        if let Some(&(id, _, _)) = keys.iter().find(|s| matches!(s.0, LINK | TEXT_DIALOG) && s.1 == vk && s.2 == mods) {
            ui.key_take();
            ui.cmd_run(id, 0);
        }
    }
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
    // (JD8 rest) a vertical text: the box across the shape, the lines as long as the shape is high
    if vertical {
        let (cx, cy) = (x + w / 2, y + h / 2);
        core::mem::swap(&mut w, &mut h);
        x = cx - w / 2;
        y = cy - h / 2;
    }
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
    // (JD8 rest) the register row and the commands of the register Textwerkzeuge (Feld, Link,
    // Symbol, QuickInfo, the dialog Text) take the focus without ending the editing: the register
    // stays, the command runs with the text being edited (Feld at the caret), then the editor has
    // the focus again (open menus keep it while they are used); every other widget ends it
    let f = ui.focused();
    let ribbon = f == crate::shell::ID_TABS || matches!(f, FIELD | LINK | SYMBOL | SCREEN_TIP | TEXT_DIALOG);
    // (the box commits when it loses the focus: to the ribbon that is no end)
    if ev & EV_COMMIT != 0 && !(ribbon && f != ID_TEXT) {
        end(app, ui);
    } else if f != ID_TEXT && ev & EV_FOCUS == 0 {
        if !ribbon {
            end(app, ui);
        } else if ui.mbtn() & (1 << BTN_LEFT) == 0 && ui.popup_count() == 0 {
            ui.focus(ID_TEXT);
            ui.repass();
        }
    }
}

// whether a shape's text runs downwards (TextDirection 1)
fn is_vertical(app: &Diagrammer, key: u64) -> bool {
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return false };
    let doc = &o.doc;
    doc.sym("TextDirection").and_then(|c| doc.resolve(SheetRef::Shape(shape_of(page, key)), &CellAddr::single(c)).num()) == Some(1.0)
}

/// end the editing now: the text goes into the shape
pub(crate) fn finish(app: &mut Diagrammer, ui: &mut Ui) {
    end(app, ui);
}

// write the text into the shape (when it changed) and stop editing
fn end(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(te) = app.text.take() else { return };
    let (Some(o), Some(page)) = (app.opened.as_mut(), app.model.page) else { return };
    let caret = caret_of(&te.doc);
    let text: String = te.doc.paras().iter().map(|p| p.text()).collect::<Vec<_>>().join("\n");
    // an empty text box of the text tool: its making is taken back
    if te.fresh && text.trim().is_empty() {
        edit_doc::undo(o);
        app.se.select(&[]);
        app.txt.last = None;
        ui.repass();
        return;
    }
    if text != joined(&te.before) {
        let s = shape_of(page, te.key);
        let owner = diagram_core::Owner::Page(page);
        let items = rebuild(&te.before, &text);
        let _ = edit_doc::edit(o, t("Text bearbeiten"), true, |tx, eng| {
            let new = keep_marks(tx.doc().shape(s).and_then(|x| x.text().cloned()), items);
            ops::set_text(tx, eng, s, new)?;
            edit_canvas::follow(tx, eng, owner, &[s])
        });
    }
    app.txt.last = Some((te.key, caret, o.ed.rev));
    ui.repass();
}

// the items of the new text: the atoms before the first and after the last change keep their
// form (fields stay fields), the changed part is plain characters; Visio's end mark at the end
fn rebuild(before: &[Atom], new: &str) -> Vec<TextItem> {
    let old: Vec<char> = joined(before).chars().collect();
    let nw: Vec<char> = new.chars().collect();
    let mut p = 0;
    while p < old.len() && p < nw.len() && old[p] == nw[p] {
        p += 1;
    }
    let mut s = 0;
    while s < old.len() - p && s < nw.len() - p && old[old.len() - 1 - s] == nw[nw.len() - 1 - s] {
        s += 1;
    }
    let tail = old.len() - s;
    let mut out: Vec<TextItem> = Vec::new();
    let push = |out: &mut Vec<TextItem>, cs: &[char]| {
        if cs.is_empty() {
            return;
        }
        let add: String = cs.iter().collect();
        match out.last_mut() {
            Some(TextItem::Chars(c)) => {
                let mut m = String::from(&**c);
                m.push_str(&add);
                *c = m.into_boxed_str();
            }
            _ => out.push(TextItem::Chars(add.into_boxed_str())),
        }
    };
    // the part before the change
    let mut a = 0;
    for at in before {
        let n = at.text().chars().count();
        let b = a + n;
        if b <= p {
            match at {
                Atom::Field(item, _) => out.push(item.clone()),
                Atom::Chars(_) => push(&mut out, &old[a..b]),
            }
        } else if a < p {
            push(&mut out, &old[a..p]);
        }
        a = b;
    }
    // the change
    push(&mut out, &nw[p..nw.len() - s]);
    // the part after it
    let mut a = 0;
    for at in before {
        let n = at.text().chars().count();
        let b = a + n;
        if a >= tail {
            match at {
                Atom::Field(item, _) => out.push(item.clone()),
                Atom::Chars(_) => push(&mut out, &old[a..b]),
            }
        } else if b > tail {
            push(&mut out, &old[tail..b]);
        }
        a = b;
    }
    if !out.is_empty() {
        push(&mut out, &['\n']);
    }
    out
}

// the new text with the first character and paragraph marks of the old one
fn keep_marks(old: Option<Text>, mut text: Vec<TextItem>) -> Text {
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
    let mut t = Text::default();
    items.append(&mut text);
    t.items = items;
    t
}

