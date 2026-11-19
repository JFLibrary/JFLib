// dlg_find -- (JD8 rest) the dialog Suchen und Ersetzen (TX-08, KB-02: Ctrl+F, Ctrl+H; Start ›
// Bearbeiten › Suchen): text in shapes, optionally in shape data, shape names and master names,
// in the selection, the page shown or all pages
//
//   FindDlg                     the dialog's state: what, with what, where, how, the last hit
//   open(app, ui, replace)      FIND / REPLACE
//   dialog(app, ui)             every pass: Weitersuchen selects the next hit (another page is
//                               shown, the shape comes into view), Ersetzen replaces the hit
//                               and finds the next, Alle ersetzen replaces every hit of the
//                               range (one undo step)
//
// The order: the pages from the one shown on (all pages; foreground before background as the
// tabs), on a page the shapes in their order with the subshapes of a group after it; per shape
// its text (fields count with the text they show), its shape data (the values as they show),
// its name, its master's name.  After the last hit the search starts again at the beginning.
// Replacing works on shape texts (as Visio's): a hit inside a run of characters is replaced, the
// marks and fields around it stay; a hit across a field or a format mark is found but not
// replaced.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{field, ShapeRef, SheetRef, Text, TextItem};
use ui::i18n::{t, tf};
use ui::*;

use crate::Diagrammer;

const ID: u32 = 0x7840_0000;

/// a hit: the page tab, the shape, the part (0 text, 1 shape data row `row`, 2 name, 3 master's
/// name), where in it (characters) and how long
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Hit {
    tab: usize,
    shape: ShapeRef,
    part: u8,
    row: usize,
    at: usize,
    len: usize,
}

/// the dialog's state
pub(crate) struct FindDlg {
    pub open: Shared<i32>,
    replace: bool,
    what: String,
    with: String,
    /// 0 the selection, 1 the page shown, 2 all pages
    scope: i32,
    case: i32,
    whole: i32,
    in_data: i32,
    in_names: i32,
    in_masters: i32,
    sel: Vec<ShapeRef>,
    sel_tab: usize,
    last: Option<Hit>,
    status: String,
}

impl Default for FindDlg {
    fn default() -> FindDlg {
        FindDlg {
            open: Shared::new(0),
            replace: false,
            what: String::new(),
            with: String::new(),
            scope: 1,
            case: 0,
            whole: 0,
            in_data: 0,
            in_names: 0,
            in_masters: 0,
            sel: Vec::new(),
            sel_tab: 0,
            last: None,
            status: String::new(),
        }
    }
}

/// Suchen (replace = false) / Ersetzen
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui, replace: bool) {
    if app.text.is_some() {
        crate::edit_text::finish(app, ui);
    }
    let sel = crate::edit_canvas::selected(app);
    let tab = app.page_index();
    let d = &mut app.txt.find;
    d.replace = replace;
    d.scope = if sel.len() > 1 { 0 } else if d.scope == 0 { 1 } else { d.scope };
    d.sel = sel;
    d.sel_tab = tab;
    d.last = None;
    d.status.clear();
    d.open.set(1);
    ui.repass();
}

// ---------------------------------------------------------------- matching

fn same(a: char, b: char, case: bool) -> bool {
    if case { a == b } else { a == b || a.to_lowercase().eq(b.to_lowercase()) }
}

fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// the first hit of `needle` in `hay` at or after `from` (characters)
fn find_in(hay: &[char], needle: &[char], from: usize, case: bool, whole: bool) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| {
        hay[i..i + needle.len()].iter().zip(needle).all(|(a, b)| same(*a, *b, case))
            && (!whole || ((i == 0 || !word(hay[i - 1])) && (i + needle.len() == hay.len() || !word(hay[i + needle.len()]))))
    })
}

// the shapes of the range in search order: (tab, shape)
fn order(app: &Diagrammer) -> Vec<(usize, ShapeRef)> {
    let d = &app.txt.find;
    let Some(o) = app.opened.as_ref() else { return Vec::new() };
    let doc = &o.doc;
    let tree = |tab: usize, tops: Vec<ShapeRef>, out: &mut Vec<(usize, ShapeRef)>| {
        let mut stack: Vec<ShapeRef> = tops.into_iter().rev().collect();
        while let Some(s) = stack.pop() {
            let Some(sh) = doc.shape(s) else { continue };
            if sh.is_deleted() || sh.header().kind == diagram_core::ShapeKind::Guide {
                continue;
            }
            out.push((tab, s));
            let kids: Vec<ShapeRef> = sh.children().collect();
            stack.extend(kids.into_iter().rev());
        }
    };
    let mut out = Vec::new();
    let shown = app.page_index();
    match d.scope {
        0 => tree(d.sel_tab, d.sel.clone(), &mut out),
        1 => {
            if let Some(tb) = o.tabs.get(shown) {
                tree(shown, doc.top_shapes(diagram_core::Owner::Page(tb.page)).collect(), &mut out);
            }
        }
        _ => {
            let n = o.tabs.len();
            for k in 0..n {
                let tab = (shown + k) % n;
                tree(tab, doc.top_shapes(diagram_core::Owner::Page(o.tabs[tab].page)).collect(), &mut out);
            }
        }
    }
    out
}

// the texts of a shape that are searched: (part, row, text)
fn parts(app: &Diagrammer, s: ShapeRef) -> Vec<(u8, usize, String)> {
    let d = &app.txt.find;
    let Some(o) = app.opened.as_ref() else { return Vec::new() };
    let doc = &o.doc;
    let mut out = alloc::vec![(0u8, 0usize, field::plain(doc, s))];
    if d.in_data != 0 {
        for (k, r) in doc.shape_data(SheetRef::Shape(s)).iter().enumerate() {
            let v = r.get("Value");
            let text = match v.value() {
                diagram_core::Value::Text(x) => String::from(doc.str(x)),
                diagram_core::Value::Num(x) => field::format(&field::Shown::Num(x, v.unit()), "", &doc.fields),
                diagram_core::Value::Absent => String::new(),
            };
            out.push((1, k, text));
        }
    }
    if let Some(sh) = doc.shape(s) {
        if d.in_names != 0 {
            out.push((2, 0, sh.name()));
        }
        if d.in_masters != 0 {
            if let Some(m) = doc.master_of(s).and_then(|m| doc.master(m)) {
                out.push((3, 0, String::from(m.header.name.or(m.header.name_u).map_or("", |n| doc.str(n)))));
            }
        }
    }
    out
}

// the next hit after `after` (None: from the beginning), wrapping around once -> (hit, wrapped)
fn next(app: &Diagrammer, after: Option<Hit>) -> Option<(Hit, bool)> {
    let d = &app.txt.find;
    let needle: Vec<char> = d.what.chars().collect();
    let (case, whole) = (d.case != 0, d.whole != 0);
    let list = order(app);
    let start = after.and_then(|h| list.iter().position(|&(t, s)| t == h.tab && s == h.shape)).unwrap_or(0);
    for round in 0..2 {
        for (i, &(tab, s)) in list.iter().enumerate() {
            if (round == 0 && i < start) || (round == 1 && i > start) {
                continue;
            }
            for (part, row, text) in parts(app, s) {
                let hay: Vec<char> = text.chars().collect();
                let from = match after {
                    Some(h) if round == 0 && i == start && h.tab == tab && h.shape == s => {
                        if (part, row) < (h.part, h.row) {
                            continue;
                        }
                        if (part, row) == (h.part, h.row) { h.at + 1 } else { 0 }
                    }
                    _ => 0,
                };
                if let Some(at) = find_in(&hay, &needle, from, case, whole) {
                    return Some((Hit { tab, shape: s, part, row, at, len: needle.len() }, round == 1));
                }
            }
        }
        if after.is_none() {
            break;
        }
    }
    None
}

// Weitersuchen
fn find_next(app: &mut Diagrammer, ui: &mut Ui) {
    if app.txt.find.what.is_empty() {
        return;
    }
    let after = app.txt.find.last;
    match next(app, after) {
        Some((h, wrapped)) => {
            app.txt.find.last = Some(h);
            crate::cmds_text::show_shape(app, ui, h.tab, Some(h.shape));
            let label = app.opened.as_ref().map(|o| crate::edit_canvas::shape_label(&o.doc, h.shape)).unwrap_or_default();
            let part = [t("Text"), t("Shape-Daten"), t("Shape-Name"), t("Master-Shape")][h.part.min(3) as usize];
            let mut s = tf("Gefunden: {} ({})", &[&label, part]);
            if wrapped {
                s.push_str(" · ");
                s.push_str(t("Die Suche hat wieder am Anfang begonnen."));
            }
            app.txt.find.status = s;
        }
        None => {
            app.txt.find.last = None;
            let w = app.txt.find.what.clone();
            app.txt.find.status = tf("„{}“ wurde nicht gefunden.", &[&w]);
        }
    }
    ui.repass();
}

// the text with every hit inside one run of characters replaced (from character `from` of the
// shown text on; once: only the first) -> (text, count)
fn replaced(doc: &diagram_core::Document, s: ShapeRef, text: &Text, d: &FindDlg, from: usize, once: bool) -> (Text, usize) {
    let needle: Vec<char> = d.what.chars().collect();
    let (case, whole) = (d.case != 0, d.whole != 0);
    let mut out = text.clone();
    let mut pos = 0usize;
    let mut n = 0;
    let sheet = SheetRef::Shape(s);
    for it in out.items.iter_mut() {
        match it {
            TextItem::Chars(c) => {
                let hay: Vec<char> = c.chars().collect();
                let mut res = String::new();
                let mut i = 0;
                let mut search = from.saturating_sub(pos);
                while let Some(at) = (!(once && n > 0)).then(|| find_in(&hay, &needle, search.max(i), case, whole)).flatten() {
                    res.extend(&hay[i..at]);
                    res.push_str(&d.with);
                    i = at + needle.len();
                    search = i;
                    n += 1;
                }
                if i > 0 {
                    res.extend(&hay[i..]);
                    *c = res.into_boxed_str();
                }
                pos += hay.len();
            }
            TextItem::Field(ix) => pos += field::shown(doc, sheet, *ix).chars().count(),
            TextItem::Other(x) => pos += field::cached(doc.str(*x)).map_or(0, |t| t.chars().count()),
            _ => {}
        }
    }
    (out, n)
}

// replace the hits of shapes (`only`: the one hit) -> how many, one undo step
fn replace_in(app: &mut Diagrammer, ui: &mut Ui, shapes: &[ShapeRef], only: Option<Hit>) -> usize {
    let d = &app.txt.find;
    let Some(o) = app.opened.as_mut() else { return 0 };
    // the new texts first (the shown text of fields needs the document as it is)
    let mut work: Vec<(ShapeRef, Text)> = Vec::new();
    let mut count = 0;
    for &s in shapes {
        let Some(text) = o.doc.shape(s).and_then(|x| x.text()) else { continue };
        let (from, once) = match only {
            Some(h) => (h.at, true),
            None => (0, false),
        };
        let (new, n) = replaced(&o.doc, s, text, d, from, once);
        if n > 0 {
            // the single hit must be the one found (not a later one in the same run)
            if let Some(h) = only {
                let before: String = field::plain(&o.doc, s).chars().take(h.at).collect();
                let after = field::segments_of(&o.doc, SheetRef::Shape(s), &new).iter().map(|g| g.text()).collect::<String>();
                if !after.starts_with(&before) || after.chars().skip(h.at).take(d.with.chars().count()).collect::<String>() != d.with {
                    continue;
                }
            }
            count += n;
            work.push((s, new));
        }
    }
    if work.is_empty() {
        return 0;
    }
    let label = if only.is_some() { t("Ersetzen") } else { t("Alle ersetzen") };
    let r = crate::edit_doc::edit(o, label, true, |tx, eng| {
        for (s, text) in &work {
            diagram_core::ops::set_text(tx, eng, *s, text.clone())?;
            crate::edit_canvas::follow(tx, eng, s.owner, &[*s])?;
        }
        Ok(())
    });
    ui.repass();
    if r.is_ok() { count } else { 0 }
}

/// the dialog (every pass)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.txt.find.open.clone();
    let title = if app.txt.find.replace { crate::commands::label(crate::commands::REPLACE) } else { crate::commands::label(crate::commands::FIND) };
    if !ui.dialog_begin(ID, title, &open) {
        return;
    }
    let row = |ui: &mut Ui, s: &str| {
        ui.next_size(140, 0);
        ui.text(TS_BODY, s);
    };
    let mut go = false;
    let d = &mut app.txt.find;
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row(ui, t("Suchen nach"));
    ui.next_size(300, 0);
    let before = d.what.clone();
    // Enter (the box keeps the focus; leaving it commits too) looks for the next hit
    if ui.textbox(ID + 1, None, &mut d.what, 512) & EV_COMMIT != 0 && ui.focused() == ID + 1 {
        go = true;
    }
    if d.what != before {
        d.last = None;
    }
    ui.box_end();
    if d.replace {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        row(ui, t("Ersetzen durch"));
        ui.next_size(300, 0);
        ui.textbox(ID + 2, None, &mut d.with, 512);
        ui.box_end();
    }
    ui.spacer(4);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    row(ui, t("Suchen in"));
    let scope = d.scope;
    ui.next_flags(if d.sel.is_empty() { ST_DISABLED } else { 0 });
    ui.radio(ID + 3, t("Auswahl"), &mut d.scope, 0);
    ui.radio(ID + 4, crate::tc("Seite|Bereich"), &mut d.scope, 1);
    ui.radio(ID + 5, t("Alle Seiten"), &mut d.scope, 2);
    if d.scope != scope {
        d.last = None;
    }
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    row(ui, t("Auch in"));
    ui.checkbox(ID + 6, t("Shape-Daten"), &mut d.in_data);
    ui.checkbox(ID + 7, t("Shape-Namen"), &mut d.in_names);
    ui.checkbox(ID + 8, t("Namen der Master-Shapes"), &mut d.in_masters);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    row(ui, t("Optionen"));
    ui.checkbox(ID + 9, t("Groß-/Kleinschreibung beachten"), &mut d.case);
    ui.checkbox(ID + 10, t("Nur ganzes Wort"), &mut d.whole);
    ui.box_end();
    ui.spacer(6);
    ui.box_begin(0, BOX_H, 8);
    let has = !d.what.is_empty();
    ui.next_flags(if has { 0 } else { ST_DISABLED });
    if ui.button(ID + 20, t("Weitersuchen")) & EV_ACT != 0 {
        go = true;
    }
    let (mut one, mut all) = (false, false);
    if d.replace {
        ui.next_flags(if has { 0 } else { ST_DISABLED });
        one = ui.button(ID + 21, t("Ersetzen")) & EV_ACT != 0;
        ui.next_flags(if has { 0 } else { ST_DISABLED });
        all = ui.button(ID + 22, t("Alle ersetzen")) & EV_ACT != 0;
    }
    ui.box_end();
    if !d.status.is_empty() {
        ui.spacer(4);
        ui.next_size(460, 0);
        ui.text(TS_CAPTION | TX_WRAP, &d.status.clone());
    }
    ui.dialog_buttons(None, None, Some(t("Schließen")));
    ui.dialog_end();
    if go && has {
        find_next(app, ui);
    }
    if one && has {
        // the hit found last is replaced, then the next is looked for
        match app.txt.find.last {
            Some(h) if h.part == 0 => {
                let n = replace_in(app, ui, &[h.shape], Some(h));
                if n > 0 {
                    let with = app.txt.find.with.chars().count();
                    app.txt.find.last = Some(Hit { at: (h.at + with).saturating_sub(1), ..h });
                }
                find_next(app, ui);
            }
            _ => find_next(app, ui),
        }
    }
    if all && has {
        let shapes: Vec<ShapeRef> = order(app).into_iter().map(|(_, s)| s).collect();
        let n = replace_in(app, ui, &shapes, None);
        app.txt.find.last = None;
        app.txt.find.status = if n == 1 { String::from(t("1 Stelle ersetzt.")) } else { tf("{} Stellen ersetzt.", &[&format!("{n}")]) };
        ui.repass();
    }
}
