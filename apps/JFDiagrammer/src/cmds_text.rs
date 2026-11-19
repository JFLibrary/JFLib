// cmds_text -- (JD8 rest) the text, field, link and search commands around their dialogs:
// Text (dlg_text.rs), Feld (dlg_field.rs), Hyperlinks (dlg_link.rs), Suchen und Ersetzen
// (dlg_find.rs) and the vertical text box (RS-09, RE-09, RE-10, RE-13, TX-05, TX-06, TX-08,
// HL-01, ZF-12, ZF-15, KB-10, KB-13)
//
//   TextCmds                    the dialogs' state, the text edited last (a field or a link
//                               goes to its caret), a caret for the next editing start
//   available(app, id)          the commands want an open drawing, shapes or the text being
//                               edited
//   frame(app, ui)              every pass: the fields' locale and clock (Document::fields),
//                               the dialogs, the shape menu's rows of links
//   command(app, ui, id)        a row of links in the shape's menu (LINK_ROWS + k): follow it
//   target(app) -> Target       where a field or a link goes: the text being edited (it ends
//                               and comes back after the dialog), the text edited last when
//                               nothing happened since (the ribbon's button took the focus),
//                               else the selected shapes
//   vertical(tx, eng, s)        a text box of the text tool turned into a vertical one
//                               (TextDirection 1; its width follows the text)
//   show_shape(app, ui, k, s)   page tab k, shape s selected and in the middle of the view
//
// Fields show values in the user's locale: Document::fields carries the decimal sign, the
// digit groups, month and day names and the date picture of the language, the local time
// (NOW) and the file's path, made again when the language, the drawing or its file changes;
// the clock follows every pass.

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::{CellAddr, EditError, ShapeRef, SheetRef, Transaction, Unit, Value};
use ui::i18n::t;
use ui::*;

use crate::commands::*;
use crate::edit_canvas::{key_of, shape_of};
use crate::Diagrammer;

/// the first command id of the shape menu's rows of links (Mi::List, at most LINK_ROWS_MAX)
pub(crate) const LINK_ROWS: u32 = 0x1830;
pub(crate) const LINK_ROWS_MAX: u16 = 8;

/// the state of the text commands
pub(crate) struct TextCmds {
    pub text: crate::dlg_text::TextDlg,
    pub field: crate::dlg_field::FieldDlg,
    pub link: crate::dlg_link::LinkDlg,
    pub find: crate::dlg_find::FindDlg,
    /// the text edited last: shape key, caret (shown characters), the edit revision after it
    pub last: Option<(u64, usize, u32)>,
    /// the caret of the next editing start (a field put in while editing), the register shown
    /// when the editing stopped for the dialog
    pub caret: Option<usize>,
    reg: Option<crate::ribbon::Reg>,
    /// what Document::fields was made for: language column, drawing serial, file
    env_for: (usize, u32, Option<String>),
    /// the link pointer (ui.cursor_new; 0: not made yet)
    pub link_cursor: u32,
}

impl Default for TextCmds {
    fn default() -> TextCmds {
        TextCmds {
            text: Default::default(),
            field: Default::default(),
            link: Default::default(),
            find: Default::default(),
            last: None,
            caret: None,
            reg: None,
            env_for: (usize::MAX, 0, None),
            link_cursor: 0,
        }
    }
}

/// whether a text command can run now
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    let open = app.opened.is_some();
    let sel = !app.se.sel.is_empty() || app.text.is_some();
    match id {
        TEXT_DIALOG | FORMAT_TEXT | FIELD | LINK | HYPERLINK => open && sel,
        TEXT_BOX_V | FIND | REPLACE => open,
        _ => true,
    }
}

/// every pass: the fields' locale and clock, the dialogs, the shape menu's links
pub(crate) fn frame(app: &mut Diagrammer, ui: &mut Ui) {
    env(app);
    crate::dlg_text::dialog(app, ui);
    crate::dlg_field::dialog(app, ui);
    crate::dlg_link::dialog(app, ui);
    crate::dlg_find::dialog(app, ui);
    // the links of the selected shape as rows of its menu (ZF-12)
    let rows: Vec<String> = crate::dlg_link::menu_rows(app);
    ui.menu_list(LINK_ROWS, &rows);
}

/// a row of links in the shape's menu -> handled
pub(crate) fn command(app: &mut Diagrammer, ui: &mut Ui, id: u32) -> bool {
    if (LINK_ROWS..LINK_ROWS + LINK_ROWS_MAX as u32).contains(&id) {
        crate::dlg_link::follow_menu_row(app, ui, (id - LINK_ROWS) as usize);
        ui.repass();
        return true;
    }
    false
}

// ---------------------------------------------------------------- the fields' locale and clock

// the local time as days since 30 Dec 1899 and the zone's offset in days
fn local_now() -> (f64, f64) {
    let ns = plat::unix_time_ns();
    let secs = ns.div_euclid(1_000_000_000);
    let zone = plat::fs::local_zone_id()
        .and_then(|id| match id.strip_prefix("win:") {
            Some(k) => datetime::Zone::from_windows(k),
            None => datetime::Zone::by_name(&id),
        })
        .unwrap_or_else(datetime::Zone::utc);
    let off = zone.offset_at(secs) as f64;
    let days = (ns as f64 / 1e9 + off) / 86_400.0 + 25_569.0;
    (days, off / 86_400.0)
}

/// the day names of the current language, Sunday first
fn day_names() -> [&'static str; 7] {
    [t("Sonntag"), t("Montag"), t("Dienstag"), t("Mittwoch"), t("Donnerstag"), t("Freitag"), t("Samstag")]
}

/// the short date picture of a language column (de en es it fr)
pub(crate) fn date_picture(col: usize) -> &'static str {
    match col {
        0 => "dd.MM.yyyy",
        1 => "M/d/yyyy",
        _ => "dd/MM/yyyy",
    }
}

/// the language column shown now
pub(crate) fn lang_col() -> usize {
    (i18n::language() as usize).min(crate::lang::LANGS.len() - 1)
}

// Document::fields up to date: the locale when the language, the drawing or its file changed
// (the pages are built again), the clock every pass
fn env(app: &mut Diagrammer) {
    let col = lang_col();
    let Some(o) = app.opened.as_mut() else { return };
    let (now, off) = local_now();
    let key = (col, o.serial, o.ed.path.clone());
    let f = &mut o.doc.fields;
    f.now = now;
    f.utc_offset = off;
    if app.txt.env_for == key {
        return;
    }
    let loc = i18n::locale();
    f.decimal = loc.decimal();
    f.group = loc.group();
    let mut months = [""; 12];
    for (k, m) in months.iter_mut().enumerate() {
        *m = loc.month_name(k as u32 + 1);
    }
    f.months = months;
    f.days = day_names();
    f.date = date_picture(col);
    f.path = o.ed.path.clone().unwrap_or_default();
    // a new drawing was made now; a file that does not say when keeps its time unknown
    if app.txt.env_for.1 != o.serial {
        f.created = if o.ed.path.is_none() { now } else { f64::NAN };
        f.saved = f64::NAN;
        f.edited = f64::NAN;
    }
    if o.ed.path.is_some() && app.txt.env_for.2 != o.ed.path && app.txt.env_for.1 == o.serial {
        f.saved = now;
    }
    // the same drawing in another language: its fields show anew
    if app.txt.env_for.1 == o.serial {
        o.ed.rebuild = true;
    }
    app.txt.env_for = key;
}

// ---------------------------------------------------------------- where a field or link goes

/// where a field or a link goes
pub(crate) struct Target {
    /// the shapes of the page shown
    pub shapes: Vec<ShapeRef>,
    /// the caret in the first shape's shown text (None: at the end)
    pub caret: Option<usize>,
    /// the text was being edited: editing comes back after the dialog
    pub resume: bool,
}

/// where a field or a link goes now (see the file header); the text being edited ends here
pub(crate) fn target(app: &mut Diagrammer, ui: &mut Ui) -> Target {
    app.txt.reg = Some(app.reg);
    if app.text.is_some() {
        crate::edit_text::finish(app, ui);
    }
    let page = app.model.page;
    if let (Some((key, caret, rev)), Some(o), Some(page)) = (app.txt.last, app.opened.as_ref(), page) {
        let only = app.se.sel.len() == 1 && app.se.sel.contains(&key);
        if only && o.ed.rev == rev {
            return Target { shapes: alloc::vec![shape_of(page, key)], caret: Some(caret), resume: true };
        }
    }
    Target { shapes: crate::edit_canvas::selected(app), caret: None, resume: false }
}

/// start editing the text again with the caret at `caret` (after a field went in)
pub(crate) fn resume(app: &mut Diagrammer, s: ShapeRef, caret: usize) {
    app.txt.caret = Some(caret);
    app.text_req = Some((key_of(s), None, false));
    if let Some(r) = app.txt.reg.take() {
        app.reg = r;
    }
}

// ---------------------------------------------------------------- the vertical text box

/// make a new text box vertical (TEXT_BOX_V): TextDirection 1, the lines run downwards, so the
/// box's height is the length of a line and its width follows the text
/// (TEXTHEIGHT(TheText,Height)).  Visio's vertical text box: TextDirection = 1 (Annahme for the
/// width formula: the horizontal box's TEXTHEIGHT(TheText,Width) turned a quarter)
pub(crate) fn vertical(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef, height: f64) -> Result<(), EditError> {
    let sheet = SheetRef::Shape(s);
    let unit = {
        let doc = tx.doc();
        doc.sym("Width").and_then(|c| doc.resolve(sheet, &CellAddr::single(c)).unit())
    };
    let (wa, ha, ta) = (tx.single("Width"), tx.single("Height"), tx.single("TextDirection"));
    tx.set_constant(sheet, &ta, Value::Num(1.0), None)?;
    tx.set_constant(sheet, &ha, Value::Num(height), unit.or(Some(Unit::Inches)))?;
    tx.set_formula(sheet, &wa, "TEXTHEIGHT(TheText,Height)")?;
    eng.recalc(tx, &[sheet])?;
    Ok(())
}

// ---------------------------------------------------------------- a shape into view

/// page tab k shown, shape s selected and brought into view: in the middle of the view on
/// another page or when it is not fully visible (Find, a link's shape)
pub(crate) fn show_shape(app: &mut Diagrammer, ui: &mut Ui, k: usize, s: Option<ShapeRef>) {
    let other_page = app.page_tab != k as i32;
    if other_page {
        app.page_tab = k as i32;
        app.text = None;
        if s.is_none() {
            app.canvas.request_fit();
            app.fit_again = true;
        }
    }
    let Some(s) = s else {
        app.se.select(&[]);
        ui.repass();
        return;
    };
    // select the top-level shape (a subshape: its group)
    let top = {
        let Some(o) = app.opened.as_ref() else { return };
        let mut top = s;
        let mut cur = o.doc.shape(s).and_then(|x| x.parent());
        while let Some(c) = cur {
            top = c;
            cur = o.doc.shape(c).and_then(|x| x.parent());
        }
        top
    };
    app.se.select(&[key_of(top)]);
    // the middle of its box in world units
    if let Some(p) = crate::viewer::paint(app, k) {
        let d = p.view();
        let dr = d.drawing();
        let geo = crate::edit_canvas::Geo { w: dr.width, h: dr.height, scale: if dr.scale > 0.0 { dr.scale } else { 1.0 } };
        drop(d);
        if let Some(o) = app.opened.as_ref() {
            let b = diagram_core::ops::frame(&o.doc, top).bounds();
            let (p0, p1) = (geo.world([b[0], b[3]]), geo.world([b[2], b[1]]));
            let vp = &mut app.canvas.vp;
            let (a, z) = (vp.to_screen(p0), vp.to_screen(p1));
            let r = vp.rect;
            let seen = a[0] >= r[0] as f32 && a[1] >= r[1] as f32 && z[0] <= (r[0] + r[2]) as f32 && z[1] <= (r[1] + r[3]) as f32;
            if other_page || !seen {
                vp.cx = ((p0[0] + p1[0]) * 0.5) as f64;
                vp.cy = ((p0[1] + p1[1]) * 0.5) as f64;
            }
            app.fit_again = false;
        }
    }
    ui.repass();
}
