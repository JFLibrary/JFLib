// layers -- (JD8 rest) Start › Bearbeiten › Ebenen (RS-30, LY-01..04): Ebene zuweisen,
// Ebeneneigenschaften, and what the layers of a page do on the canvas
//
//   LayerState                    the two dialogs' rows while they are open
//   available(app, id)            LAYER_ASSIGN needs a selection, LAYER_PROPERTIES a drawing
//   open_assign / assign_dialog   Ebene zuweisen: the page's layers as check boxes for the
//                                 selection (a box is half set when only some of the shapes are
//                                 on that layer; it stays so until clicked), Alle, Keine, a new
//                                 layer by name; OK = one undo step
//   open_props / props_dialog     Ebeneneigenschaften: a table with Name, Anzahl (shapes on it),
//                                 Sichtbar, Drucken, Aktiv, Sperren, Einrasten, Kleben, Farbe;
//                                 the colour and its transparency of the chosen row; Neu,
//                                 Umbenennen, Löschen (LY-04: asked first; the shapes that are on
//                                 no other layer go with it); OK = one undo step
//   Flags::of(doc, page) / flags  the canvas' model (edit_canvas::model): a shape whose layers
//                                 are all hidden is left out (not drawn, not selectable), a
//                                 locked layer's shapes get SEF_NOSELECT, Kleben off SEF_NOGLUE,
//                                 Einrasten off SEF_NOSNAP (only other shapes are affected: the
//                                 layer's own shapes still glue and snap, LY-04)
//   active_before(doc) / assign_new   edit_doc::edit: shapes an edit adds to a page with active
//                                 layers and that are on no layer yet go onto the active ones
//
// Visio's cells (Layer section of the page sheet, a row per layer, IX its number; the shape's
// LayerMember "0;2"): Name, NameUniv, Visible, Print, Active, Lock, Snap, Glue, Color (a colour
// index; 255 = none), ColorTrans (0..1).  A shape on several layers shows when one of them is
// visible and prints when one of them prints (diagram_core::render); it is locked when one of
// them is locked and does not take glue or snapping when one of them refuses it (Annahme: Visio
// documents only the single-layer case).  Shapes that already are on a layer (a master's, LY-03)
// keep theirs instead of going onto the active ones (Annahme).  The colour list offers the first
// eight entries of the document's colour table (Visio's standard colours 0..7).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{glue, structure, CellAddr, Document, Owner, PageRef, RowKey, SecKey, ShapeKind, ShapeRef, SheetRef, Transaction, Value};
use ui::i18n::{t, tf};
use ui::*;

use crate::commands::*;
use crate::edit_canvas::selected;
use crate::edit_doc;
use crate::Diagrammer;

const ID_ASSIGN: u32 = 0x8600;
const ID_PROPS: u32 = 0x8800;

// i18n
const COLORS: [&str; 8] = ["Schwarz", "Weiß", "Rot", "Grün", "Blau", "Gelb", "Magenta", "Cyan"];
// i18n
const COLUMNS: [&str; 7] = ["Sichtbar", "Drucken", "Aktiv", "Sperren", "Einrasten", "Kleben", "Farbe"];
/// the Layer cells of the columns (Farbe: Color, 255 when off)
const CELLS: [&str; 6] = ["Visible", "Print", "Active", "Lock", "Snap", "Glue"];

/// a layer of the page with its values
#[derive(Clone, Debug, PartialEq)]
struct Row {
    /// its row (None: new)
    ix: Option<u32>,
    name: String,
    univ: String,
    count: usize,
    /// Visible, Print, Active, Lock, Snap, Glue, colour on (0 / 1)
    flags: [i32; 7],
    /// the colour index (0..7 offered) and its transparency in percent
    color: i32,
    trans: i32,
}

/// a row of Ebene zuweisen: the layer (None: new), its name, 0 off, 1 on, 2 some shapes
#[derive(Clone, Debug)]
struct Pick {
    ix: Option<u32>,
    name: String,
    state: i32,
}

/// the dialogs' state
pub(crate) struct LayerState {
    pub assign_dlg: Shared<i32>,
    picks: Vec<Pick>,
    pub props_dlg: Shared<i32>,
    rows: Vec<Row>,
    /// rows deleted in the dialog (their layer indices)
    deleted: Vec<u32>,
    /// the chosen row of the table
    sel: i32,
    /// the name box (a new layer, a new name)
    name: String,
    /// Löschen asked for this row: the warning shows
    ask: Option<usize>,
    page: Option<PageRef>,
    shapes: Vec<ShapeRef>,
}

impl Default for LayerState {
    fn default() -> LayerState {
        LayerState {
            assign_dlg: Shared::new(0),
            picks: Vec::new(),
            props_dlg: Shared::new(0),
            rows: Vec::new(),
            deleted: Vec::new(),
            sel: 0,
            name: String::new(),
            ask: None,
            page: None,
            shapes: Vec::new(),
        }
    }
}

/// whether a command of this module can run now
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    match id {
        LAYER_ASSIGN => app.opened.is_some() && !app.se.sel.is_empty(),
        LAYER_PROPERTIES => app.opened.is_some() && app.model.page.is_some(),
        _ => true,
    }
}

// the layers of a page as rows of the table (counts: every shape on it, subshapes too)
fn rows_of(doc: &Document, page: PageRef) -> Vec<Row> {
    let mut rows: Vec<Row> = doc
        .layers(page)
        .into_iter()
        .filter_map(|l| {
            let RowKey::Index(ix) = l.key() else { return None };
            let name = String::from(l.text("Name").or_else(|| l.text("NameUniv")).unwrap_or(""));
            let univ = String::from(l.text("NameUniv").unwrap_or(&name));
            let mut flags = [0; 7];
            for (k, c) in CELLS.iter().enumerate() {
                // missing Visible, Print, Snap and Glue cells mean on (Visio's defaults)
                let dflt = matches!(*c, "Visible" | "Print" | "Snap" | "Glue");
                flags[k] = l.num(c).map_or(dflt, |v| v != 0.0) as i32;
            }
            let (on, color) = match l.get("Color").value() {
                Value::Num(n) if n >= 0.0 && n != 255.0 => (1, n as i32),
                Value::Text(_) => (1, 2),
                _ => (0, 2),
            };
            flags[6] = on;
            let trans = (l.num("ColorTrans").unwrap_or(0.0) * 100.0 + 0.5) as i32;
            Some(Row { ix: Some(ix), name, univ, count: 0, flags, color, trans })
        })
        .collect();
    for s in doc.all_shapes(Owner::Page(page)) {
        for m in doc.layer_members(s) {
            if let Some(r) = rows.iter_mut().find(|r| r.ix == Some(m)) {
                r.count += 1;
            }
        }
    }
    rows
}

// a toggle of the table: a check box look, its name for scripts and screen readers
fn cell_toggle(ui: &mut Ui, id: u32, name: &str, v: &mut i32) -> bool {
    ui.next_icon(if *v != 0 { IC_CHECKBOX_COMPOSITE } else { IC_CHECKBOX });
    ui.next_flags(ST_SUBTLE | ST_ICON_ONLY | ST_COMPACT);
    let ev = ui.toggle_button(id, name, v);
    ui.tooltip(name);
    ev & EV_ACT != 0
}

// ---------------------------------------------------------------- Ebene zuweisen

/// Ebene zuweisen: the dialog for the selection
pub(crate) fn open_assign(app: &mut Diagrammer, ui: &mut Ui) {
    let shapes = selected(app);
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return };
    if shapes.is_empty() {
        return;
    }
    let doc = &o.doc;
    let members: Vec<Vec<u32>> = shapes.iter().map(|&s| doc.layer_members(s)).collect();
    app.layers.picks = rows_of(doc, page)
        .into_iter()
        .map(|r| {
            let ix = r.ix.unwrap_or(0);
            let on = members.iter().filter(|m| m.contains(&ix)).count();
            let state = if on == 0 { 0 } else if on == members.len() { 1 } else { 2 };
            Pick { ix: r.ix, name: r.name, state }
        })
        .collect();
    app.layers.page = Some(page);
    app.layers.shapes = shapes;
    app.layers.name.clear();
    app.layers.assign_dlg.set(1);
    ui.repass();
}

/// every pass: Ebene zuweisen while it is open
pub(crate) fn assign_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.layers.assign_dlg.clone();
    if !ui.dialog_begin(ID_ASSIGN, t("Ebene zuweisen"), &dlg) {
        return;
    }
    let st = &mut app.layers;
    ui.next_size(380, 0);
    ui.box_begin(ID_ASSIGN + 1, BOX_V, 6);
    ui.text(TS_BODY, t("Auf Ebene(n):"));
    if st.picks.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("Diese Seite hat keine Ebenen."));
    }
    for (k, p) in st.picks.iter_mut().enumerate() {
        ui.checkbox(ID_ASSIGN + 16 + k as u32 % 0x1F0, &p.name, &mut p.state);
    }
    ui.box_begin(0, BOX_H, 6);
    if ui.button(ID_ASSIGN + 2, t("Alle")) & EV_ACT != 0 {
        st.picks.iter_mut().for_each(|p| p.state = 1);
    }
    if ui.button(ID_ASSIGN + 3, t("Keine")) & EV_ACT != 0 {
        st.picks.iter_mut().for_each(|p| p.state = 0);
    }
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    ui.next_size(-1, 0);
    ui.textbox(ID_ASSIGN + 4, Some(t("Name der neuen Ebene")), &mut st.name, 128);
    let name = String::from(st.name.trim());
    let fresh = !name.is_empty() && !st.picks.iter().any(|p| p.name == name);
    ui.next_flags(if fresh { 0 } else { ST_DISABLED });
    if ui.button(ID_ASSIGN + 5, t("Neu…")) & EV_ACT != 0 && fresh {
        st.picks.push(Pick { ix: None, name, state: 1 });
        st.name.clear();
    }
    ui.box_end();
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        apply_assign(app, ui);
    }
    if r != DLG_NONE {
        app.layers.assign_dlg.set(0);
        ui.repass();
    }
}

// Ebene zuweisen, OK: new layers, then every shape's layers (half set boxes keep each shape's)
fn apply_assign(app: &mut Diagrammer, ui: &mut Ui) {
    let st = &app.layers;
    let (Some(page), shapes, picks) = (st.page, st.shapes.clone(), st.picks.clone()) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let _ = edit_doc::edit(o, t("Ebene zuweisen"), true, |tx, _eng| {
        let mut ixs: Vec<(u32, i32)> = Vec::new();
        for p in &picks {
            let ix = match p.ix {
                Some(ix) => ix,
                None => tx.add_layer(page, &p.name, &p.name)?,
            };
            ixs.push((ix, p.state));
        }
        for &s in &shapes {
            let now = tx.doc().layer_members(s);
            let mut want: Vec<u32> = now.iter().copied().filter(|m| !ixs.iter().any(|(ix, st)| ix == m && *st == 0)).collect();
            for (ix, st) in &ixs {
                if *st == 1 && !want.contains(ix) {
                    want.push(*ix);
                }
            }
            want.sort_unstable();
            if want != now {
                tx.set_layers(s, &want)?;
            }
        }
        Ok(())
    });
    ui.repass();
}

// ---------------------------------------------------------------- Ebeneneigenschaften

/// Ebeneneigenschaften: the dialog for the page shown
pub(crate) fn open_props(app: &mut Diagrammer, ui: &mut Ui) {
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return };
    let st = &mut app.layers;
    st.rows = rows_of(&o.doc, page);
    st.deleted.clear();
    st.sel = 0;
    st.ask = None;
    st.name.clear();
    st.page = Some(page);
    st.props_dlg.set(1);
    ui.repass();
}

// shapes on a page that are on deleted layers only (LY-04)
fn doomed(doc: &Document, page: PageRef, deleted: &[u32]) -> Vec<ShapeRef> {
    doc.top_shapes(Owner::Page(page))
        .filter(|&s| {
            let m = doc.layer_members(s);
            !m.is_empty() && m.iter().all(|x| deleted.contains(x))
        })
        .collect()
}

/// every pass: Ebeneneigenschaften while it is open
pub(crate) fn props_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.layers.props_dlg.clone();
    if !ui.dialog_begin(ID_PROPS, t("Ebeneneigenschaften"), &dlg) {
        return;
    }
    let doomed_now = {
        let st = &app.layers;
        match (st.ask, app.opened.as_ref(), st.page) {
            (Some(k), Some(o), Some(page)) => {
                let mut del = st.deleted.clone();
                del.extend(st.rows.get(k).and_then(|r| r.ix));
                doomed(&o.doc, page, &del).len()
            }
            _ => 0,
        }
    };
    let st = &mut app.layers;
    ui.next_size(0, 0);
    ui.box_begin(ID_PROPS + 1, BOX_V, 8);
    if st.rows.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("Diese Seite hat keine Ebenen."));
    } else {
        let cols = [0, 0, 0, 0, 0, 0, 0, 0, 0];
        ui.gridbox_begin(ID_PROPS + 2, &cols, GRID_H_START, 12);
        ui.text(TS_BODY_STRONG, t("Name"));
        ui.text(TS_BODY_STRONG, t("Anzahl"));
        for c in COLUMNS {
            ui.text(TS_BODY_STRONG, t(c));
        }
        for (k, r) in st.rows.iter_mut().enumerate() {
            let base = ID_PROPS + 0x20 + (k as u32 % 120) * 16;
            ui.radio(base, &r.name, &mut st.sel, k as i32);
            ui.text(TS_BODY, &format!("{}", r.count));
            for (c, col) in COLUMNS.iter().enumerate() {
                let name = format!("{}: {}", r.name, t(col));
                cell_toggle(ui, base + 1 + c as u32, &name, &mut r.flags[c]);
            }
        }
        ui.gridbox_end();
    }
    // the chosen row: its colour
    let sel = st.sel.max(0) as usize;
    if let Some(r) = st.rows.get_mut(sel) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.text(TS_BODY, t("Farbe der Ebene"));
        let names: Vec<&str> = COLORS.iter().map(|c| t(c)).collect();
        ui.next_size(160, 0);
        ui.next_flags(if r.flags[6] != 0 { 0 } else { ST_DISABLED });
        let mut c = r.color.clamp(0, 7);
        if ui.combobox(ID_PROPS + 3, &mut c, &names) & EV_ACT != 0 {
            r.color = c;
        }
        ui.next_size(220, 0);
        ui.next_flags(if r.flags[6] != 0 { 0 } else { ST_DISABLED });
        ui.prop_slider(ID_PROPS + 4, t("Transparenz"), &mut r.trans, &Range::new(0, 100, 1, 0), 0);
        ui.box_end();
    }
    // new, rename, delete
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    ui.next_size(220, 0);
    ui.textbox(ID_PROPS + 5, Some(t("Name")), &mut st.name, 128);
    let name = String::from(st.name.trim());
    let fresh = !name.is_empty() && !st.rows.iter().any(|r| r.name == name);
    ui.next_flags(if fresh { 0 } else { ST_DISABLED });
    if ui.button(ID_PROPS + 6, t("Neu…")) & EV_ACT != 0 && fresh {
        st.rows.push(Row { ix: None, name: name.clone(), univ: name.clone(), count: 0, flags: [1, 1, 0, 0, 1, 1, 0], color: 2, trans: 0 });
        st.sel = st.rows.len() as i32 - 1;
        st.name.clear();
    }
    let has = st.rows.get(sel).is_some();
    ui.next_flags(if fresh && has { 0 } else { ST_DISABLED });
    if ui.button(ID_PROPS + 7, t("Umbenennen")) & EV_ACT != 0 && fresh && has {
        st.rows[sel].name = name;
        st.name.clear();
    }
    ui.next_flags(if has { 0 } else { ST_DISABLED });
    if ui.button(ID_PROPS + 8, t("Löschen")) & EV_ACT != 0 && has {
        st.ask = Some(sel);
    }
    ui.box_end();
    // LY-04: asked before
    if let Some(k) = st.ask {
        let lname = st.rows.get(k).map(|r| r.name.clone()).unwrap_or_default();
        let msg = tf("„{}“ löschen? {} Shapes, die auf keiner anderen Ebene liegen, werden mit gelöscht.", &[&lname, &format!("{doomed_now}")]);
        ui.text(TS_BODY | TX_WRAP, &msg);
        ui.box_begin(0, BOX_H, 6);
        if ui.button(ID_PROPS + 9, t("Ebene löschen")) & EV_ACT != 0 {
            if let Some(r) = (k < st.rows.len()).then(|| st.rows.remove(k)) {
                st.deleted.extend(r.ix);
            }
            st.ask = None;
            st.sel = st.sel.min(st.rows.len() as i32 - 1).max(0);
        }
        if ui.button(ID_PROPS + 10, t("Behalten")) & EV_ACT != 0 {
            st.ask = None;
        }
        ui.box_end();
    }
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        apply_props(app, ui);
    }
    if r != DLG_NONE {
        app.layers.props_dlg.set(0);
        app.layers.ask = None;
        ui.repass();
    }
}

// a layer row's cells as the table has them
fn write_row(tx: &mut Transaction, page: PageRef, ix: u32, r: &Row) -> Result<(), diagram_core::EditError> {
    let sheet = SheetRef::Page(page);
    let sec = SecKey::one(tx.intern("Layer"));
    let key = RowKey::Index(ix);
    let set = |tx: &mut Transaction, cell: &str, v: Value| -> Result<(), diagram_core::EditError> {
        let c = tx.intern(cell);
        let a = CellAddr::row(sec, key, c);
        if tx.doc().resolve(sheet, &a).value() == v {
            return Ok(());
        }
        tx.set_constant(sheet, &a, v, None)
    };
    let name = tx.intern(&r.name);
    set(tx, "Name", Value::Text(name))?;
    for (k, c) in CELLS.iter().enumerate() {
        set(tx, c, Value::Num(r.flags[k] as f64))?;
    }
    let color = if r.flags[6] != 0 { r.color.clamp(0, 254) as f64 } else { 255.0 };
    set(tx, "Color", Value::Num(color))?;
    set(tx, "ColorTrans", Value::Num(r.trans.clamp(0, 100) as f64 / 100.0))?;
    Ok(())
}

// Ebeneneigenschaften, OK: deletions (with their shapes), changed and new rows; one undo step
fn apply_props(app: &mut Diagrammer, ui: &mut Ui) {
    let st = &app.layers;
    let (Some(page), rows, mut deleted) = (st.page, st.rows.clone(), st.deleted.clone()) else { return };
    let before = app.opened.as_ref().map(|o| rows_of(&o.doc, page)).unwrap_or_default();
    let owner = Owner::Page(page);
    let Some(o) = app.opened.as_mut() else { return };
    let r = edit_doc::edit(o, t("Ebeneneigenschaften"), true, |tx, eng| {
        // LY-04: the shapes on deleted layers only, then the layers (the highest first: the
        // others keep their numbers while they go)
        let gone = doomed(tx.doc(), page, &deleted);
        if !gone.is_empty() {
            // exactly these (a container's members on other layers stay, LY-04)
            structure::forget(tx, eng, &gone)?;
            glue::detach(tx, eng, owner, &gone)?;
            tx.delete_shapes(&gone)?;
        }
        deleted.sort_unstable();
        // the rows that stay, by their index now
        let mut keep: Vec<(u32, Row)> = rows.iter().filter_map(|r| r.ix.map(|ix| (ix, r.clone()))).collect();
        for &d in deleted.iter().rev() {
            tx.delete_layer(page, d)?;
            for (ix, _) in keep.iter_mut() {
                if *ix > d {
                    *ix -= 1;
                }
            }
        }
        for (ix, r) in &keep {
            let was = before.iter().find(|b| b.ix == r.ix);
            if was.is_none_or(|b| b.name != r.name || b.flags != r.flags || b.color != r.color || b.trans != r.trans) {
                write_row(tx, page, *ix, r)?;
            }
        }
        for r in rows.iter().filter(|r| r.ix.is_none()) {
            let ix = tx.add_layer(page, &r.name, &r.univ)?;
            write_row(tx, page, ix, r)?;
        }
        Ok(())
    });
    if r.is_ok() {
        // hidden or locked shapes leave the selection (the model drops them next pass)
        app.se.select(&[]);
    }
    ui.repass();
}

// ---------------------------------------------------------------- the canvas

/// what the layers of a page do to its shapes on the canvas
pub(crate) struct Flags {
    /// per layer index: visible, locked, glue, snap
    layers: Vec<(u32, [bool; 4])>,
}

impl Flags {
    /// the page's layers (None: it has none)
    pub(crate) fn of(doc: &Document, page: PageRef) -> Option<Flags> {
        let layers: Vec<(u32, [bool; 4])> = doc
            .layers(page)
            .into_iter()
            .filter_map(|l| {
                let RowKey::Index(ix) = l.key() else { return None };
                let on = |c: &str, d: bool| l.num(c).map_or(d, |v| v != 0.0);
                Some((ix, [on("Visible", true), on("Lock", false), on("Glue", true), on("Snap", true)]))
            })
            .collect();
        (!layers.is_empty()).then_some(Flags { layers })
    }

    /// a shape's flags for the edit layer (None: all its layers are hidden, it is left out)
    pub(crate) fn flags(&self, doc: &Document, s: ShapeRef) -> Option<u32> {
        let members = doc.layer_members(s);
        let mut known = false;
        let mut visible = false;
        let mut f = 0;
        for m in members {
            let Some((_, l)) = self.layers.iter().find(|(ix, _)| *ix == m) else {
                visible = true;
                continue;
            };
            known = true;
            visible |= l[0];
            if l[1] {
                f |= SEF_NOSELECT;
            }
            if !l[2] {
                f |= SEF_NOGLUE;
            }
            if !l[3] {
                f |= SEF_NOSNAP;
            }
        }
        (visible || !known).then_some(f)
    }
}

/// edit_doc::edit: the pages with active layers and the ID their next new shape gets
pub(crate) fn active_before(doc: &Document) -> Vec<(PageRef, u32)> {
    let mut v = Vec::new();
    for p in doc.pages() {
        let active = doc.layers(p).iter().any(|l| l.num("Active").is_some_and(|x| x != 0.0));
        if active {
            if let Some(next) = doc.page(p).and_then(|pg| pg.shapes.next_id()) {
                v.push((p, next));
            }
        }
    }
    v
}

/// edit_doc::edit: the shapes this edit added to such a page that are on no layer go onto its
/// active layers (LY-02)
pub(crate) fn assign_new(tx: &mut Transaction, before: &[(PageRef, u32)]) -> Result<(), diagram_core::EditError> {
    for &(page, next) in before {
        let active: Vec<u32> = tx
            .doc()
            .layers(page)
            .into_iter()
            .filter(|l| l.num("Active").is_some_and(|x| x != 0.0))
            .filter_map(|l| match l.key() {
                RowKey::Index(ix) => Some(ix),
                _ => None,
            })
            .collect();
        if active.is_empty() {
            continue;
        }
        let new: Vec<ShapeRef> = {
            let doc = tx.doc();
            doc.top_shapes(Owner::Page(page))
                .filter(|&s| doc.shape(s).is_some_and(|x| x.id() >= next && x.header().kind != ShapeKind::Guide) && doc.layer_members(s).is_empty())
                .collect()
        };
        for s in new {
            tx.set_layers(s, &active)?;
        }
    }
    Ok(())
}
