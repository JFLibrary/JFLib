// data_link -- (JD18, RD-09, AS-04, DA-04) Daten verknüpfen: the wizard „Automatisch
// verknüpfen“ -- which recordset, which shapes (the selection or every shape of the page), pairs
// "Datenspalte = Shape-Feld", whether links a shape has are replaced -- then
// diagram_core::data::auto_link in one undo step; the last page says how many shapes were linked
//
//   LinkState                 the wizard's page and choices
//   open(app, ui)             Daten › Daten verknüpfen: the recordset the pane Externe Daten
//                             shows (data_cmds::recordset_shown), the selection when there is one
//   dialog(app, ui)           every pass
//   comparisons(doc, rs)      the pairs a recordset keeps (its AutoLinkComparison elements): Nach
//                             Aktualisierung automatisch verknüpfen links new rows with them
//                             (data_refresh.rs)
//
// Shape fields: the shape's text, its name, every shape data field one of the shapes has (shown
// by its label; the row name only where two fields share a label).  A shape is linked when
// every pair matches (compared as text, trimmed, ignoring case; diagram_core::data::auto_link).
// The pairs go into the recordset
// as Visio's AutoLinkComparison with the ContextType codes of VisAutoLinkFieldTypes: 1 the
// shape's text, 9 its name, 14 a shape data row name (the label read from a file, 2, is found
// on the shapes).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::data::{self, AutoLink, Recordset, ShapeField};
use diagram_core::{Document, Owner, PageRef, ShapeRef, SheetRef};
use ui::i18n::{t, tf};
use ui::*;

use crate::commands::{self, DATA_LINK};
use crate::{edit_canvas, edit_doc, Diagrammer};

const ID: u32 = 0x7C20_0000;

/// VisAutoLinkFieldTypes: the shape's text, a shape data label, the shape's name (local,
/// universal), a shape data row name (universal)
const CTX_TEXT: u32 = 1;
const CTX_LABEL: u32 = 2;
const CTX_NAME: u32 = 9;
const CTX_NAME_U: u32 = 17;
const CTX_ROW: u32 = 14;

/// the wizard's state
pub(crate) struct LinkState {
    pub open: Shared<i32>,
    /// 0 the recordset and the shapes, 1 the pairs, 2 the result
    step: u8,
    /// the recordsets in the list's order, the one chosen
    sets: Vec<u32>,
    set_pick: i32,
    /// 0 the selected shapes, 1 every shape of the page
    scope: i32,
    sel: Vec<ShapeRef>,
    page: Option<PageRef>,
    /// the shape fields offered (label, field)
    fields: Vec<(String, ShapeField)>,
    /// (column, field) by index
    pairs: Vec<(i32, i32)>,
    replace: i32,
    /// what the last page says
    result: String,
}

impl Default for LinkState {
    fn default() -> LinkState {
        LinkState { open: Shared::new(0), step: 0, sets: Vec::new(), set_pick: 0, scope: 1, sel: Vec::new(), page: None, fields: Vec::new(), pairs: Vec::new(), replace: 0, result: String::new() }
    }
}

/// Daten › Daten verknüpfen
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    let shown = crate::data_cmds::recordset_shown(app);
    let sel = edit_canvas::selected(app);
    let page = app.model.page;
    let Some(o) = app.opened.as_ref() else { return };
    let sets: Vec<u32> = o.doc.data.recordsets.iter().map(|r| r.id).collect();
    if sets.is_empty() || page.is_none() {
        return;
    }
    let st = &mut app.dlink;
    st.set_pick = shown.and_then(|s| sets.iter().position(|&i| i == s)).unwrap_or(0) as i32;
    st.sets = sets;
    st.scope = if sel.is_empty() { 1 } else { 0 };
    st.sel = sel;
    st.page = page;
    st.step = 0;
    st.fields.clear();
    st.pairs.clear();
    st.replace = 0;
    st.result.clear();
    st.open.set(1);
    ui.repass();
}

// the shapes the wizard links: the selection, or every shape of the page (members of groups too)
fn shapes(doc: &Document, st: &LinkState) -> Vec<ShapeRef> {
    if st.scope == 0 {
        return st.sel.iter().copied().filter(|&s| doc.shape(s).is_some()).collect();
    }
    match st.page {
        Some(p) if doc.page(p).is_some() => doc.all_shapes(Owner::Page(p)).collect(),
        _ => Vec::new(),
    }
}

// the shape fields: text, name, the shape data fields the shapes have (by row name, once)
fn fields_of(doc: &Document, shapes: &[ShapeRef]) -> Vec<(String, ShapeField)> {
    let mut out: Vec<(String, ShapeField)> = alloc::vec![(String::from(t("Shape-Text")), ShapeField::Text), (String::from(t("Shape-Name")), ShapeField::Name)];
    // (name, label); a field shows its label, its row name only where two fields share a label
    // (linked columns have Visio's row names "_VisDM_...", which say nothing to the user)
    let mut named: Vec<(String, String)> = Vec::new();
    for &s in shapes {
        for r in doc.shape_data(SheetRef::Shape(s)) {
            let Some(name) = r.name() else { continue };
            if named.iter().any(|(n, _)| n == name) {
                continue;
            }
            named.push((String::from(name), String::from(r.text("Label").unwrap_or(""))));
        }
    }
    let mut data: Vec<(String, String)> = Vec::new();
    for (name, label) in &named {
        let twice = named.iter().filter(|(_, l)| !label.is_empty() && l.eq_ignore_ascii_case(label)).count() > 1;
        let shown = if label.is_empty() || label == name {
            name.clone()
        } else if twice {
            format!("{label} ({name})")
        } else {
            label.clone()
        };
        data.push((name.clone(), shown));
    }
    data.sort_by_key(|a| a.1.to_lowercase());
    out.extend(data.into_iter().map(|(n, shown)| (shown, ShapeField::Data(n))));
    out
}

// the first pair offered: a column whose label or name a shape data field has, else the first
// column against the shape's text
fn first_pair(rs: &Recordset, fields: &[(String, ShapeField)]) -> (i32, i32) {
    for (c, col) in rs.columns.iter().enumerate() {
        let hit = fields.iter().position(|(shown, f)| match f {
            ShapeField::Data(n) => n.eq_ignore_ascii_case(&col.name) || shown.split(" (").next().is_some_and(|l| l.eq_ignore_ascii_case(&col.label)),
            _ => false,
        });
        if let Some(k) = hit {
            return (c as i32, k as i32);
        }
    }
    (0, 0)
}

/// a recordset's comparisons as pairs for auto_link (columns by source name; a label read from
/// a file is looked up on the drawing's shapes)
pub(crate) fn comparisons(doc: &Document, rs: &Recordset) -> Vec<(usize, ShapeField)> {
    let mut out = Vec::new();
    for a in &rs.auto_link {
        let Some(c) = rs.column_by_id(&a.column).or_else(|| rs.column_by_name(&a.column)) else { continue };
        let field = match a.context {
            CTX_TEXT => ShapeField::Text,
            CTX_NAME | CTX_NAME_U => ShapeField::Name,
            CTX_ROW => match &a.label {
                Some(n) => ShapeField::Data(n.clone()),
                None => continue,
            },
            CTX_LABEL => match a.label.as_deref().and_then(|l| row_of_label(doc, l)) {
                Some(n) => ShapeField::Data(n),
                None => continue,
            },
            _ => continue,
        };
        out.push((c, field));
    }
    out
}

// the row name of a shape data field with this label on some shape of the drawing
fn row_of_label(doc: &Document, label: &str) -> Option<String> {
    for p in doc.pages() {
        for s in doc.all_shapes(Owner::Page(p)) {
            for r in doc.shape_data(SheetRef::Shape(s)) {
                if r.text("Label").is_some_and(|l| l.eq_ignore_ascii_case(label)) {
                    return r.name().map(String::from);
                }
            }
        }
    }
    None
}

// the pairs as AutoLinkComparison elements
fn to_auto_link(rs: &Recordset, pairs: &[(usize, ShapeField)]) -> Vec<AutoLink> {
    pairs
        .iter()
        .filter_map(|(c, f)| {
            let column = rs.columns.get(*c)?.id.clone();
            let (context, label) = match f {
                ShapeField::Text => (CTX_TEXT, None),
                ShapeField::Name => (CTX_NAME, None),
                ShapeField::Data(n) => (CTX_ROW, Some(n.clone())),
            };
            Some(AutoLink { column, context, label })
        })
        .collect()
}

/// every pass: the wizard
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.dlink.open.clone();
    if open.get() == 0 {
        return;
    }
    // the drawing closed or its data went away: the wizard goes too
    let rs = app.opened.as_ref().and_then(|o| {
        let id = *app.dlink.sets.get(app.dlink.set_pick.max(0) as usize)?;
        o.doc.recordset(id).cloned()
    });
    let Some(rs) = rs else {
        open.set(0);
        ui.repass();
        return;
    };
    if !ui.dialog_begin(ID, t("Automatisch verknüpfen"), &open) {
        return;
    }
    let step = app.dlink.step;
    ui.next_size(460, 0);
    ui.box_begin(ID + 1, BOX_V, 8);
    match step {
        0 => page_shapes(app, ui),
        1 => page_pairs(app, ui, &rs),
        _ => {
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_WRAP, &app.dlink.result);
        }
    }
    ui.box_end();
    let r = match step {
        0 => ui.dialog_buttons(Some(t("Weiter")), None, Some(t("Abbrechen"))),
        1 => ui.dialog_buttons(Some(t("Fertig stellen")), Some(t("Zurück")), Some(t("Abbrechen"))),
        _ => ui.dialog_buttons(None, None, Some(t("Schließen"))),
    };
    ui.dialog_end();
    match (step, r) {
        (0, DLG_PRIMARY) => {
            let fields = app.opened.as_ref().map(|o| fields_of(&o.doc, &shapes(&o.doc, &app.dlink))).unwrap_or_default();
            let st = &mut app.dlink;
            if st.pairs.is_empty() || st.fields != fields {
                st.pairs = alloc::vec![first_pair(&rs, &fields)];
            }
            st.fields = fields;
            st.step = 1;
            open.set(1);
        }
        (1, DLG_SECONDARY) => {
            app.dlink.step = 0;
            open.set(1);
        }
        (1, DLG_PRIMARY) => {
            finish(app, ui, &rs);
            app.dlink.step = 2;
            open.set(1);
        }
        _ => {}
    }
    if r != DLG_NONE {
        ui.repass();
    }
}

// page 1: the recordset, the shapes
fn page_shapes(app: &mut Diagrammer, ui: &mut Ui) {
    let names: Vec<String> = app.opened.as_ref().map(|o| app.dlink.sets.iter().filter_map(|&id| o.doc.recordset(id).map(|r| r.name.clone())).collect()).unwrap_or_default();
    let st = &mut app.dlink;
    ui.text(TS_BODY_STRONG, t("Datenquelle"));
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    ui.next_size(-1, 0);
    let before = st.set_pick;
    ui.combobox(ID + 2, &mut st.set_pick, &refs);
    if st.set_pick != before {
        // other columns: the pairs start again
        st.pairs.clear();
    }
    ui.spacer(4);
    ui.text(TS_BODY_STRONG, t("Welche Shapes sollen verknüpft werden?"));
    ui.next_flags(if st.sel.is_empty() { ST_DISABLED } else { 0 });
    ui.radio(ID + 3, t("Ausgewählte Shapes"), &mut st.scope, 0);
    ui.radio(ID + 4, t("Alle Shapes dieser Seite"), &mut st.scope, 1);
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Ein Shape wird mit der Zeile verknüpft, deren Werte zu seinen Feldern passen."));
}

// page 2: the pairs "Datenspalte = Shape-Feld", replace links
fn page_pairs(app: &mut Diagrammer, ui: &mut Ui, rs: &Recordset) {
    let st = &mut app.dlink;
    let cols: Vec<&str> = rs.columns.iter().map(|c| c.label.as_str()).collect();
    let fields: Vec<&str> = st.fields.iter().map(|(l, _)| l.as_str()).collect();
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_WRAP, t("Zeile automatisch mit dem Shape verknüpfen, wenn:"));
    ui.box_begin(0, BOX_H, 8);
    ui.next_size(190, 0);
    ui.text(TS_CAPTION | TX_SECONDARY, t("Datenspalte"));
    ui.next_size(24, 0);
    ui.text(TS_CAPTION, "");
    ui.next_size(190, 0);
    ui.text(TS_CAPTION | TX_SECONDARY, t("Shape-Feld"));
    ui.box_end();
    let mut remove: Option<usize> = None;
    let n = st.pairs.len();
    for (k, p) in st.pairs.iter_mut().enumerate() {
        let id = ID + 0x40 + k as u32 * 4;
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(190, 0);
        ui.combobox(id, &mut p.0, &cols);
        ui.next_size(24, 0);
        ui.text(TS_BODY | TX_CENTER, "=");
        ui.next_size(190, 0);
        ui.combobox(id + 1, &mut p.1, &fields);
        if n > 1 && ui.button(id + 2, t("Löschen")) & EV_ACT != 0 {
            remove = Some(k);
        }
        ui.box_end();
    }
    if let Some(k) = remove {
        st.pairs.remove(k);
        ui.repass();
    }
    ui.next_flags(ST_SUBTLE | ST_ALIGN_START);
    if st.pairs.len() < 8 && ui.button(ID + 5, t("Und")) & EV_ACT != 0 {
        st.pairs.push((0, 0));
        ui.repass();
    }
    ui.spacer(4);
    ui.checkbox(ID + 6, t("Vorhandene Verknüpfungen ersetzen"), &mut st.replace);
}

// Fertig stellen: link in one undo step, keep the pairs in the recordset
fn finish(app: &mut Diagrammer, ui: &mut Ui, rs: &Recordset) {
    let st = &app.dlink;
    let pairs: Vec<(usize, ShapeField)> = st
        .pairs
        .iter()
        .filter_map(|&(c, f)| {
            let c = usize::try_from(c).ok().filter(|&c| c < rs.columns.len())?;
            let f = st.fields.get(usize::try_from(f).ok()?)?.1.clone();
            Some((c, f))
        })
        .collect();
    let replace = st.replace != 0;
    let id = rs.id;
    let Some(o) = app.opened.as_mut() else { return };
    let shapes = shapes(&o.doc, &app.dlink);
    let keep = to_auto_link(rs, &pairs);
    let r = edit_doc::edit(o, commands::label(DATA_LINK), true, |tx, eng| {
        let (made, written) = data::auto_link(tx, &shapes, id, &pairs, replace)?;
        eng.recalc_cells(tx, &written)?;
        // the comparisons for Nach Aktualisierung automatisch verknüpfen
        if let Some(mut r) = tx.doc().recordset(id).cloned() {
            if r.auto_link != keep {
                r.auto_link = keep;
                data::set_recordset(tx, r)?;
            }
        }
        Ok(made.len())
    });
    let msg = match r {
        Ok(0) => String::from(t("Kein Shape wurde verknüpft: zu keinem Shape passt eine Zeile.")),
        Ok(1) => String::from(t("1 Shape wurde verknüpft.")),
        Ok(n) => tf("{} Shapes wurden verknüpft.", &[&format!("{n}")]),
        Err(e) => tf("Die Shapes lassen sich nicht verknüpfen: {}", &[&format!("{e:?}")]),
    };
    ui.announce(&msg, LIVE_POLITE);
    app.dlink.result = msg;
    ui.repass();
}
