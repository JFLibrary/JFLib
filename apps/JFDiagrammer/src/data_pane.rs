// data_pane -- (JD18, AB-11, DA-04) the pane Externe Daten: a tab per recordset, its rows with a
// link mark, the menus of the pane and of a tab, rows dragged onto shapes or the empty page; the
// source's dialogs (entfernen, umbenennen, Spalteneinstellungen)
//
//   ExtState                  the tab shown (app state only: switching tabs is no edit of the
//                             drawing, so it does not go through data::set_active and the undo
//                             history; the file keeps the ActiveRecordsetID it had), the grid's
//                             selection and sort, the dialogs
//   pane(app, ui)             panes::content: the rows of the recordset shown, from the drawing
//                             alone (no file is read): the first column marks the rows bound to
//                             shapes (Recordset::shapes_of), the headers are the columns' labels
//                             in their display order (DisplayOrder); a click on a header sorts
//                             (in the pane only), a double click on a linked row selects its
//                             shapes; the tabs below; the menus (right click, menu key)
//   drop_target(app, ui)      (edit_canvas::canvas, after the Shapes panel's) rows dropped on the
//                             canvas: onto a shape -> the first row bound to it (data::link; a
//                             shape bound to the recordset before gets the new row); onto the
//                             empty page -> one shape per row from the master armed in the Shapes
//                             panel (Visio's selected master; else a rectangle), side by side from
//                             the drop point, each bound to its row; one undo step either way.
//                             While rows are dragged over a shape it is outlined and the pointer
//                             says "link", over the page "copy".
//   remove_source / rename_source / columns_open   the dialogs of the source shown
//   select_linked / unlink_rows   the shapes of the rows selected in the pane (the pane's menu)
//   unlink / selection_linked     the selected shapes (the shape's menu › Daten)
//   available(app, id)        whether DATA_SELECT_LINKED / DATA_UNLINK_ROWS can run
//
// Spalteneinstellungen: the label (Column::label), the data type (Column::data_type, the types a
// column can have: text, number, Boolean, date, duration, currency) and the order
// (Column::display_order, Visio's DisplayOrder: the source's order stays, so a refresh keeps the
// order the user chose).  Hidden columns are not offered: neither the model nor Visio's file
// format as we know it has a field for them.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

use diagram_core::data::{self, kind, DataValue, Recordset};
use diagram_core::{instance, ops, Document, Owner, ShapeRef, SheetRef};
use ui::i18n::{self, t, tf};
use ui::*;

use crate::commands::{self, *};
use crate::edit_canvas::{self, key_of};
use crate::{edit_doc, Diagrammer};

/// the drag kind of the pane's rows: the recordset's ID, then the rows' IDs (u32 LE each)
pub(crate) const DT_ROWS: u32 = DT_USER0 << 3;

const ID: u32 = 0x7C78_0000;
/// the rows (ID_GRID + 1 + i)
const ID_GRID: u32 = 0x6C78_0000;
const ID_TABS: u32 = ID + 0x10;
/// the canvas as the rows' drop target
const ID_DROP: u32 = ID + 0x20;
const ID_REMOVE: u32 = 0x7C7C_0000;
const ID_RENAME: u32 = 0x7C7D_0000;
const ID_COLS: u32 = 0x7C7E_0000;

// the data types a column can have (Visio's codes) and their names
const TYPE_CODES: [u16; 6] = [kind::TEXT, kind::NUMBER, kind::BOOL, kind::DATE, kind::DURATION, kind::CURRENCY];
// i18n
const TYPE_NAMES: [&str; 6] = ["Zeichenfolge", "Zahl", "Boolesch", "Datum", "Dauer", "Währung"];

// the pane's menu (right click on a row or the pane, the menu key)
// i18n
static PANE_MENU: [Mi; 10] = [
    Mi::Cmd(DATA_SELECT_LINKED),
    Mi::Cmd(DATA_UNLINK_ROWS),
    Mi::Sep,
    Mi::Cmd(DATA_COLUMNS),
    Mi::Sub("Datenquelle", &[Mi::Cmd(DATA_CUSTOM_IMPORT), Mi::Cmd(DATA_SOURCE_REMOVE), Mi::Cmd(DATA_SOURCE_RENAME)]),
    Mi::Sep,
    Mi::Cmd(DATA_REFRESH),
    Mi::Cmd(DATA_REFRESH_SETTINGS),
    Mi::Sep,
    Mi::Cmd(PANE_DG_FIELDS),
];
// a tab's menu
static TAB_MENU: [Mi; 7] = [
    Mi::Cmd(DATA_SOURCE_RENAME),
    Mi::Cmd(DATA_SOURCE_REMOVE),
    Mi::Sep,
    Mi::Cmd(DATA_REFRESH),
    Mi::Cmd(DATA_REFRESH_SETTINGS),
    Mi::Sep,
    Mi::Cmd(DATA_CUSTOM_IMPORT),
];

// a column in the dialog Spalteneinstellungen
#[derive(Clone)]
struct ColEdit {
    /// its index in Recordset::columns
    index: usize,
    label: String,
    /// index into TYPE_CODES
    kind: i32,
}

// the rows' display order, kept while nothing it depends on changes
#[derive(Default)]
struct Sorted {
    key: (u32, Option<(usize, bool)>, usize, usize, Vec<usize>),
    /// positions in Recordset::rows
    idx: Vec<usize>,
}

/// the pane's state
pub(crate) struct ExtState {
    /// the recordset whose tab is shown (data_cmds::recordset_shown)
    pub tab: Option<u32>,
    grid: GridState,
    /// the recordset the grid's selection belongs to
    grid_rs: Option<u32>,
    sorted: Sorted,
    remove: Shared<i32>,
    remove_id: Option<u32>,
    rename: Shared<i32>,
    rename_id: Option<u32>,
    rename_text: String,
    rename_focus: u8,
    cols: Shared<i32>,
    cols_id: Option<u32>,
    col_list: Vec<ColEdit>,
    col_sel: i32,
    /// what could not be done (shown in the pane)
    error: Option<String>,
}

impl Default for ExtState {
    fn default() -> ExtState {
        ExtState {
            tab: None,
            grid: GridState::default(),
            grid_rs: None,
            sorted: Sorted::default(),
            remove: Shared::new(0),
            remove_id: None,
            rename: Shared::new(0),
            rename_id: None,
            rename_text: String::new(),
            rename_focus: 0,
            cols: Shared::new(0),
            cols_id: None,
            col_list: Vec::new(),
            col_sel: 0,
            error: None,
        }
    }
}

// the columns in their display order (DisplayOrder, else the source's)
fn display_order(rs: &Recordset) -> Vec<usize> {
    let mut v: Vec<usize> = (0..rs.columns.len()).collect();
    v.sort_by_key(|&i| (rs.columns[i].display_order.unwrap_or(i as u32), i));
    v
}

// two values for sorting: numbers, dates and money by value, texts without case, empty ones last
fn cmp_values(a: &DataValue, b: &DataValue) -> Ordering {
    use DataValue::*;
    match (a, b) {
        (Empty, Empty) => Ordering::Equal,
        (Empty, _) => Ordering::Greater,
        (_, Empty) => Ordering::Less,
        (Num(x) | Currency(x) | Date(x), Num(y) | Currency(y) | Date(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        _ => a.text().to_lowercase().cmp(&b.text().to_lowercase()),
    }
}

/// the pane's content (panes::content)
pub(crate) fn pane(app: &mut Diagrammer, ui: &mut Ui) {
    let shown = crate::data_cmds::recordset_shown(app);
    let Some(rs_id) = shown else {
        empty(app, ui);
        return;
    };
    let Some(o) = app.opened.as_ref() else { return };
    let Some(rs) = o.doc.recordset(rs_id) else { return };
    let ext = &mut app.ext;
    let order = display_order(rs);
    if ext.grid_rs != Some(rs_id) {
        // another tab: its own selection; the recordset's sort (SortColumn, SortAsc) to start with
        ext.grid = GridState::default();
        ext.grid_rs = Some(rs_id);
        ext.grid.sort = rs.sort_column.as_ref().and_then(|c| order.iter().position(|&k| rs.columns[k].id == *c)).map(|p| (p + 1, rs.sort_asc == Some(false)));
    }
    // the rows' order (sorted once per change)
    let skey = (rs_id, ext.grid.sort, alloc::sync::Arc::as_ptr(&rs.rows) as usize, rs.rows.len(), order.clone());
    if ext.sorted.key != skey {
        let mut idx: Vec<usize> = (0..rs.rows.len()).collect();
        if let Some((c, desc)) = ext.grid.sort.filter(|s| s.0 >= 1) {
            if let Some(&k) = order.get(c - 1) {
                idx.sort_by(|&a, &b| {
                    let o = cmp_values(rs.rows[a].values.get(k).unwrap_or(&DataValue::Empty), rs.rows[b].values.get(k).unwrap_or(&DataValue::Empty));
                    if desc { o.reverse() } else { o }
                });
            }
        }
        ext.sorted = Sorted { key: skey, idx };
    }
    let idx = &ext.sorted.idx;
    let keys: Vec<u64> = idx.iter().map(|&p| rs.rows[p].id as u64).collect();
    let linked: BTreeSet<u32> = rs.links.iter().map(|l| l.row).collect();
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    let mut cols = vec![GridCol::new("", 28).min(28).flags(GC_NOSORT)];
    cols.extend(order.iter().map(|&k| GridCol::new(rs.columns[k].label.as_str(), 120)));
    ui.next_size(-1, -1);
    let (first, end) = ui.grid_begin(ID_GRID, &mut ext.grid, &cols, &keys);
    // the chain of a link
    let icon = crate::icons::JD_LINK;
    for i in first..end {
        let row = &rs.rows[idx[i as usize]];
        let ev = ui.grid_row(&mut ext.grid, i);
        // a row dragged: the selected rows when it is one of them, else this one
        match ui.drag_source(ID_GRID.wrapping_add(1).wrapping_add(i as u32), ev) {
            DS_START => {
                let key = row.id as u64;
                if !ext.grid.sel.contains(&key) {
                    ext.grid.select_only(Some(key));
                }
                let rows: Vec<u32> = ext.grid.selected_in_order().into_iter().map(|k| k as u32).collect();
                let mut bytes = rs_id.to_le_bytes().to_vec();
                for r in &rows {
                    bytes.extend_from_slice(&r.to_le_bytes());
                }
                let mut d = DragData::of_app(DT_ROWS, &bytes);
                d.label = if rows.len() == 1 {
                    order.first().and_then(|&k| row.values.get(k)).map(DataValue::text).filter(|s| !s.is_empty()).unwrap_or_else(|| tf("Zeile {}", &[&format!("{}", row.id)]))
                } else {
                    tf("{} Zeilen", &[&format!("{}", rows.len())])
                };
                d.effects = DE_COPY | DE_LINK;
                ui.drag_set(d);
            }
            DS_END => ui.repass(),
            _ => {}
        }
        if linked.contains(&row.id) {
            ui.grid_cell(&ext.grid, 0);
            ui.icon(icon, 16);
            ui.sem(K_ICON, 0, Some(&tf("Zeile {} ist verknüpft", &[&format!("{}", row.id)])), 0);
        }
        for (c, &k) in order.iter().enumerate() {
            ui.grid_text(&ext.grid, c + 1, &row.values.get(k).map(DataValue::text).unwrap_or_default());
        }
    }
    let gev = ui.grid_end(&mut ext.grid);
    let area = ui.last_rect();
    let activated = (gev.ev & EV_DOUBLE != 0 && gev.row >= 0).then(|| keys.get(gev.row as usize).map(|&k| k as u32)).flatten();
    if gev.ev & EV_RCLICK != 0 {
        ui.menu_popup(i18n::menu(&PANE_MENU));
    }
    // a right click on the pane that no row took: the same menu
    if ui.released() & (1 << BTN_RIGHT) != 0 && ui.popup_count() == 0 && inside(ui, area) {
        ui.menu_popup(i18n::menu(&PANE_MENU));
    }
    // the tabs: one per recordset (Visio's below the rows)
    let tabs: Vec<(u32, &str)> = o.doc.data.recordsets.iter().map(|r| (r.id, r.name.as_str())).collect();
    let names: Vec<&str> = tabs.iter().map(|t| t.1).collect();
    let mut sel = tabs.iter().position(|t| t.0 == rs_id).unwrap_or(0) as i32;
    let before = sel;
    ui.next_size(-1, 26);
    let ev = ui.tabs_scrolling(ID_TABS, &mut sel, &names);
    let strip = ui.last_rect();
    if sel != before {
        ext.tab = tabs.get(sel as usize).map(|t| t.0);
        ui.repass();
    }
    let rename = ev & EV_DOUBLE != 0;
    if ui.context_menu(i18n::menu(&TAB_MENU)) == 0 && ui.released() & (1 << BTN_RIGHT) != 0 && ui.popup_count() == 0 && inside(ui, strip) {
        ui.menu_popup(i18n::menu(&TAB_MENU));
    }
    if let Some(e) = ext.error.clone() {
        ui.next_size(-1, 0);
        if ui.infobar(ID + 0x30, 0, t("Hinweis"), &e) & EV_ACT != 0 {
            ext.error = None;
            ui.repass();
        }
    }
    ui.box_end();
    if let Some(row) = activated {
        select_shapes(app, ui, rs_id, &[row]);
    }
    if rename {
        rename_source(app, ui);
    }
}

// whether the pointer is inside a rectangle
fn inside(ui: &Ui, r: [i32; 4]) -> bool {
    let (mx, my) = ui.mouse();
    mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3]
}

// no recordset yet: what the pane is for, and the way to a source
fn empty(app: &mut Diagrammer, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.box_begin(ID + 1, BOX_V | BOX_XCENTER | pad(10), 8);
    ui.empty_state(crate::icons::JD_EXTERNAL_DATA, t("Keine Datenquelle"), t("Benutzerdefinierter Import holt die Zeilen einer Excel-Arbeitsmappe oder einer Textdatei hierher. Auf die Seite gezogen, werden sie mit Shapes verknüpft."));
    ui.next_size(220, 0);
    ui.cmd_button(DATA_CUSTOM_IMPORT, 0);
    ui.box_end();
    let area = ui.last_rect();
    if ui.released() & (1 << BTN_RIGHT) != 0 && ui.popup_count() == 0 && inside(ui, area) {
        ui.menu_popup(i18n::menu(&PANE_MENU));
    }
    let _ = app;
}

// ---------------------------------------------------------------- rows onto the canvas

/// (edit_canvas::canvas, right after the Shapes panel's drop target; the canvas' rectangle is the
/// last one) rows of the pane dropped onto the page
pub(crate) fn drop_target(app: &mut Diagrammer, ui: &mut Ui) {
    match ui.drop_target(ID_DROP, DT_ROWS) {
        DS_HOVER => {
            let (_, x, y) = ui.drag_over_px();
            let at = app.canvas.vp.to_world(x as f32, y as f32);
            let shape = edit_canvas::shape_at(app, ui, at);
            ui.drop_effect_set(if shape.is_some() { DE_LINK } else { DE_COPY });
            if let Some(s) = shape {
                outline(app, ui, s);
            }
        }
        DS_DROP => {
            let d = ui.drop_data();
            if d.app & DT_ROWS != 0 {
                if let Some((rs, rows)) = decode(&d.bytes) {
                    let (x, y) = ui.drop_pos_px();
                    let at = app.canvas.vp.to_world(x as f32, y as f32);
                    dropped(app, ui, at, rs, &rows);
                }
            }
        }
        _ => {}
    }
}

// the bytes of a row drag: the recordset, the rows
fn decode(b: &[u8]) -> Option<(u32, Vec<u32>)> {
    if b.len() < 8 || b.len() % 4 != 0 {
        return None;
    }
    let mut it = b.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
    let rs = it.next()?;
    Some((rs, it.collect()))
}

// the shape rows would be bound to, outlined as Visio does
fn outline(app: &Diagrammer, ui: &mut Ui, s: ShapeRef) {
    let (Some(geo), Some(o)) = (app.model.geo, app.opened.as_ref()) else { return };
    let Some(b) = ops::bounds(&o.doc, &[s]) else { return };
    let vp = app.canvas.vp;
    let (a, z) = (vp.to_screen(geo.world([b[0], b[3]])), vp.to_screen(geo.world([b[2], b[1]])));
    let r = vp.rect;
    ui.clip_push(r[0], r[1], r[2], r[3]);
    ui.draw_color(ui.color(T_ACCENT));
    let pad = ui.dp(2);
    ui.draw_frame(a[0] as i32 - pad, a[1] as i32 - pad, (z[0] - a[0]) as i32 + 2 * pad, (z[1] - a[1]) as i32 + 2 * pad, 0, ui.dp(3).max(2));
    ui.clip_pop();
}

// a shape and its subshapes as sheets (for the engine)
fn sheets_of(doc: &Document, s: ShapeRef, out: &mut Vec<SheetRef>) {
    out.push(SheetRef::Shape(s));
    if let Some(sh) = doc.shape(s) {
        for c in sh.children() {
            sheets_of(doc, c, out);
        }
    }
}

// rows dropped at world point `at`: onto a shape, or new shapes on the empty page
fn dropped(app: &mut Diagrammer, ui: &mut Ui, at: [f32; 2], rs: u32, rows: &[u32]) {
    let Some(&first) = rows.first() else { return };
    if app.opened.as_ref().and_then(|o| o.doc.recordset(rs)).is_none() {
        return;
    }
    if let Some(s) = edit_canvas::shape_at(app, ui, at) {
        let Some(o) = app.opened.as_mut() else { return };
        let r = edit_doc::edit(o, t("Zeile mit Shape verknüpfen"), true, |tx, eng| {
            let w = data::link(tx, s, rs, first)?;
            eng.recalc_cells(tx, &w)?;
            Ok(())
        });
        match r {
            Ok(()) => {
                let name = app.opened.as_ref().map(|o| edit_canvas::shape_label(&o.doc, s)).unwrap_or_default();
                app.se.select(&[key_of(s)]);
                ui.announce(&tf("Zeile mit „{}“ verknüpft", &[&name]), LIVE_POLITE);
            }
            Err(e) => app.ext.error = Some(tf("Die Zeile lässt sich nicht verknüpfen: {}", &[&format!("{e:?}")])),
        }
        ui.repass();
        return;
    }
    let (Some(geo), Some(page)) = (app.model.geo, app.model.page) else { return };
    let [x, y] = geo.page(at);
    let owner = Owner::Page(page);
    let lang = crate::stencils::lang();
    // Visio's selected master: the tile armed in the Shapes panel (it stays armed)
    let st = &app.stencils;
    let entry = st.armed.and_then(|(key, k)| st.entry(key, k));
    let masters = crate::structures::masters(st);
    // a rectangle of Visio's default size on the paper (40 x 20 mm, 1.5 x 0.75 in)
    let paper = if edit_canvas::metric(app) { [40.0 / 25.4, 20.0 / 25.4] } else { [1.5, 0.75] };
    let (w, h) = (paper[0] / geo.scale, paper[1] / geo.scale);
    let gap = 0.25 / geo.scale;
    let Some(o) = app.opened.as_mut() else { return };
    let label = if rows.len() == 1 { t("Shape aus Zeile erstellen") } else { t("Shapes aus Zeilen erstellen") };
    let made = edit_doc::edit(o, label, true, |tx, eng| {
        let mut shapes = Vec::new();
        let mut cx = x;
        for &row in rows {
            let s = match entry {
                Some((l, e)) => {
                    let m = instance::import_master(tx, &l.doc, e.master)?;
                    let s = instance::place(tx, page, m, cx, y, lang)?;
                    let mut sheets = Vec::new();
                    sheets_of(tx.doc(), s, &mut sheets);
                    eng.recalc(tx, &sheets)?;
                    edit_canvas::follow(tx, eng, owner, &[s])?;
                    crate::structures::dropped(tx, eng, &masters, owner, s)?;
                    s
                }
                None => {
                    let s = ops::new_shape(tx, eng, owner, ops::Basic::Rectangle, [cx - w * 0.5, y - h * 0.5], [cx + w * 0.5, y + h * 0.5])?;
                    crate::structures::joined(tx, eng, owner, &[s])?;
                    s
                }
            };
            let written = data::link(tx, s, rs, row)?;
            eng.recalc_cells(tx, &written)?;
            // the next one to the right of this one
            let b = ops::bounds(tx.doc(), &[s]).unwrap_or([cx - w * 0.5, y, cx + w * 0.5, y]);
            cx = b[2] + gap + (cx - b[0]);
            shapes.push(s);
        }
        Ok(shapes)
    });
    match made {
        Ok(shapes) => {
            let keys: Vec<u64> = shapes.iter().map(|&s| key_of(s)).collect();
            app.se.select(&keys);
            let said = if shapes.len() == 1 { String::from(t("Shape aus Zeile erstellt")) } else { tf("{} Shapes aus Zeilen erstellt", &[&format!("{}", shapes.len())]) };
            ui.announce(&said, LIVE_POLITE);
        }
        Err(e) => app.ext.error = Some(tf("Die Shapes lassen sich nicht erstellen: {}", &[&format!("{e:?}")])),
    }
    ui.repass();
}

// ---------------------------------------------------------------- rows and shapes

// the rows selected in the pane's grid (of the recordset shown), else its current row
fn rows_selected(app: &Diagrammer) -> Option<(u32, Vec<u32>)> {
    let rs = crate::data_cmds::recordset_shown(app)?;
    if app.ext.grid_rs != Some(rs) {
        return None;
    }
    let mut rows: Vec<u32> = app.ext.grid.selected_in_order().into_iter().map(|k| k as u32).collect();
    if rows.is_empty() {
        rows.extend(app.ext.grid.cur.map(|k| k as u32));
    }
    (!rows.is_empty()).then_some((rs, rows))
}

// the shapes bound to these rows (that still exist)
fn shapes_of_rows(doc: &Document, rs: u32, rows: &[u32]) -> Vec<ShapeRef> {
    let Some(set) = doc.recordset(rs) else { return Vec::new() };
    let mut out: Vec<ShapeRef> = Vec::new();
    for &r in rows {
        for (p, s) in set.shapes_of(r) {
            if let Some(x) = doc.data_shape(p, s) {
                if !out.contains(&x) {
                    out.push(x);
                }
            }
        }
    }
    out
}

/// whether a pane command can run now: Verknüpfte Shapes auswählen and Verknüpfung aufheben
/// need a selected row bound to a shape
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    match id {
        DATA_SELECT_LINKED | DATA_UNLINK_ROWS => match (app.opened.as_ref(), rows_selected(app)) {
            (Some(o), Some((rs, rows))) => !shapes_of_rows(&o.doc, rs, &rows).is_empty(),
            _ => false,
        },
        _ => true,
    }
}

// the top-level shape a shape belongs to (the edit layer selects those)
fn top(doc: &Document, s: ShapeRef) -> ShapeRef {
    let mut t = s;
    while let Some(p) = doc.shape(t).and_then(|x| x.parent()) {
        t = p;
    }
    t
}

// select the shapes bound to rows: those on the page shown, else those on the page of the first
fn select_shapes(app: &mut Diagrammer, ui: &mut Ui, rs: u32, rows: &[u32]) {
    let Some(o) = app.opened.as_ref() else { return };
    let shapes = shapes_of_rows(&o.doc, rs, rows);
    let Some(&first) = shapes.first() else { return };
    let k = app.page_index();
    let here = o.tabs.get(k).map(|t| Owner::Page(t.page));
    let owner = if shapes.iter().any(|s| Some(s.owner) == here) { here } else { Some(first.owner) };
    let Some(kk) = o.tabs.iter().position(|t| Some(Owner::Page(t.page)) == owner) else { return };
    let mut keys: Vec<u64> = Vec::new();
    for s in shapes.iter().filter(|s| Some(s.owner) == owner) {
        let key = key_of(top(&o.doc, *s));
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let on = shapes.iter().copied().find(|s| Some(s.owner) == owner);
    if kk != k {
        crate::cmds_text::show_shape(app, ui, kk, on);
    }
    app.se.select(&keys);
    ui.repass();
}

/// Verknüpfte Shapes auswählen (the rows selected in the pane)
pub(crate) fn select_linked(app: &mut Diagrammer, ui: &mut Ui) {
    let Some((rs, rows)) = rows_selected(app) else { return };
    select_shapes(app, ui, rs, &rows);
}

/// Verknüpfung aufheben (the pane's menu): the shapes of the rows selected lose their binding to
/// the recordset shown; their shape data stays
pub(crate) fn unlink_rows(app: &mut Diagrammer, ui: &mut Ui) {
    let Some((rs, rows)) = rows_selected(app) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let shapes = shapes_of_rows(&o.doc, rs, &rows);
    if shapes.is_empty() {
        return;
    }
    let _ = edit_doc::edit(o, commands::label(DATA_UNLINK_ROWS), true, |tx, _| {
        for &s in &shapes {
            data::unlink(tx, s, Some(rs))?;
        }
        Ok(())
    });
    ui.repass();
}

/// Verknüpfung mit Zeile aufheben (the selected shapes; every row they are bound to)
pub(crate) fn unlink(app: &mut Diagrammer, ui: &mut Ui) {
    let sel = edit_canvas::selected(app);
    let Some(o) = app.opened.as_mut() else { return };
    let shapes: Vec<ShapeRef> = sel.into_iter().filter(|&s| !o.doc.data_links(s).is_empty()).collect();
    if shapes.is_empty() {
        return;
    }
    let _ = edit_doc::edit(o, commands::label(DATA_UNLINK), true, |tx, _| {
        for &s in &shapes {
            data::unlink(tx, s, None)?;
        }
        Ok(())
    });
    ui.repass();
}

/// whether a selected shape is bound to a row
pub(crate) fn selection_linked(app: &Diagrammer) -> bool {
    let Some(o) = app.opened.as_ref() else { return false };
    edit_canvas::selected(app).into_iter().any(|s| !o.doc.data_links(s).is_empty())
}

// ---------------------------------------------------------------- the source's dialogs

/// Datenquelle entfernen (the recordset shown): asks first
pub(crate) fn remove_source(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(rs) = crate::data_cmds::recordset_shown(app) else { return };
    app.ext.remove_id = Some(rs);
    app.ext.remove.set(1);
    ui.repass();
}

/// Datenquelle umbenennen
pub(crate) fn rename_source(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(rs) = crate::data_cmds::recordset_shown(app) else { return };
    let Some(name) = app.opened.as_ref().and_then(|o| o.doc.recordset(rs)).map(|r| r.name.clone()) else { return };
    let e = &mut app.ext;
    e.rename_id = Some(rs);
    e.rename_text = name;
    e.rename_focus = 2;
    e.rename.set(1);
    ui.repass();
}

/// Spalteneinstellungen
pub(crate) fn columns_open(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(id) = crate::data_cmds::recordset_shown(app) else { return };
    let Some(rs) = app.opened.as_ref().and_then(|o| o.doc.recordset(id)) else { return };
    let list: Vec<ColEdit> = display_order(rs)
        .into_iter()
        .map(|i| {
            let c = &rs.columns[i];
            let kind = TYPE_CODES.iter().position(|&k| k == c.data_type).unwrap_or(0) as i32;
            ColEdit { index: i, label: c.label.clone(), kind }
        })
        .collect();
    let e = &mut app.ext;
    e.col_list = list;
    e.col_sel = 0;
    e.cols_id = Some(id);
    e.cols.set(1);
    ui.repass();
}

/// every pass: the pane's dialogs (remove, rename, column settings)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    remove_dialog(app, ui);
    rename_dialog(app, ui);
    columns_dialog(app, ui);
}

fn remove_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.ext.remove.clone();
    if !ui.dialog_begin(ID_REMOVE, commands::label(DATA_SOURCE_REMOVE), &open) {
        return;
    }
    let id = app.ext.remove_id;
    let name = app.opened.as_ref().zip(id).and_then(|(o, id)| o.doc.recordset(id)).map(|r| r.name.clone()).unwrap_or_default();
    ui.next_size(380, 0);
    ui.text(TS_BODY | TX_WRAP, &tf("Datenquelle „{}“ entfernen? Die verknüpften Shapes behalten ihre Shape-Daten.", &[&name]));
    let r = ui.dialog_buttons(Some(t("Entfernen")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        if let (Some(id), Some(o)) = (id, app.opened.as_mut()) {
            let _ = edit_doc::edit(o, commands::label(DATA_SOURCE_REMOVE), true, |tx, _| data::remove_recordset(tx, id));
            if app.ext.tab == Some(id) {
                app.ext.tab = None;
            }
        }
    }
    if r != DLG_NONE {
        app.ext.remove.set(0);
        ui.repass();
    }
}

// why a recordset cannot have this name (None: it can)
fn name_problem(doc: &Document, id: u32, name: &str) -> Option<&'static str> {
    let n = name.trim();
    if n.is_empty() {
        return Some(t("Der Name darf nicht leer sein."));
    }
    doc.data.recordsets.iter().any(|r| r.id != id && r.name.to_lowercase() == n.to_lowercase()).then(|| t("Eine andere Datenquelle hat diesen Namen schon."))
}

fn rename_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.ext.rename.clone();
    if !ui.dialog_begin(ID_RENAME, commands::label(DATA_SOURCE_RENAME), &open) {
        return;
    }
    let e = &mut app.ext;
    let mut name = core::mem::take(&mut e.rename_text);
    ui.next_size(320, 0);
    ui.text(TS_BODY, t("Name der Datenquelle"));
    ui.next_size(320, 0);
    ui.textbox(ID_RENAME + 1, None, &mut name, 256);
    match e.rename_focus {
        2 => {
            ui.focus(ID_RENAME + 1);
            e.rename_focus = 1;
        }
        1 if ui.focused() == ID_RENAME + 1 => {
            ui.textbox_select(ID_RENAME + 1, &name, 0, name.len());
            e.rename_focus = 0;
        }
        _ => {}
    }
    let problem = match (app.opened.as_ref(), e.rename_id) {
        (Some(o), Some(id)) if o.doc.recordset(id).is_some() => name_problem(&o.doc, id, &name),
        _ => Some(t("Die Datenquelle gibt es nicht mehr.")),
    };
    if let Some(msg) = problem {
        ui.next_size(320, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, msg);
    }
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    app.ext.rename_text = name;
    if r == DLG_PRIMARY && problem.is_none() {
        let new = String::from(app.ext.rename_text.trim());
        if let (Some(id), Some(o)) = (app.ext.rename_id, app.opened.as_mut()) {
            if let Some(mut rs) = o.doc.recordset(id).cloned() {
                if rs.name != new {
                    rs.name = new;
                    let _ = edit_doc::edit(o, commands::label(DATA_SOURCE_RENAME), true, |tx, _| data::set_recordset(tx, rs));
                }
            }
        }
    }
    if r != DLG_NONE {
        // OK with a name that cannot be taken keeps the dialog open
        app.ext.rename.set((r == DLG_PRIMARY && problem.is_some()) as i32);
        ui.repass();
    }
}

fn columns_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.ext.cols.clone();
    if !ui.dialog_begin(ID_COLS, commands::label(DATA_COLUMNS).trim_end_matches('.'), &open) {
        return;
    }
    let e = &mut app.ext;
    let names: Vec<String> = e.col_list.iter().map(|c| c.label.clone()).collect();
    let n = names.len() as i32;
    ui.next_size(420, 0);
    ui.text(TS_BODY, t("Spalten"));
    ui.box_begin(0, BOX_H, 12);
    ui.next_size(280, 220);
    let mut sel = e.col_sel.clamp(-1, n - 1);
    let (first, end) = ui.list_begin(ID_COLS + 1, &mut sel, n, 28 | LF_REORDER);
    for row in first..end {
        ui.list_item(row, &names[row as usize]);
    }
    let r = ui.list_end_full(&mut sel);
    if let Some((from, to)) = r.drop {
        if from < e.col_list.len() && to < e.col_list.len() {
            let c = e.col_list.remove(from);
            e.col_list.insert(to, c);
            sel = to as i32;
        }
    }
    ui.box_begin(0, BOX_V, 8);
    let (up, down) = (sel > 0 && sel < n, sel >= 0 && sel + 1 < n);
    ui.next_size(120, 0);
    ui.next_flags(if up { 0 } else { ST_DISABLED });
    if ui.button(ID_COLS + 0x100, t("Nach oben")) & EV_ACT != 0 && up {
        e.col_list.swap(sel as usize, sel as usize - 1);
        sel -= 1;
        ui.repass();
    }
    ui.next_size(120, 0);
    ui.next_flags(if down { 0 } else { ST_DISABLED });
    if ui.button(ID_COLS + 0x101, t("Nach unten")) & EV_ACT != 0 && down {
        e.col_list.swap(sel as usize, sel as usize + 1);
        sel += 1;
        ui.repass();
    }
    ui.box_end();
    ui.box_end();
    e.col_sel = sel;
    // the selected column's label and data type
    if let Some(c) = e.col_list.get_mut(sel.max(0) as usize).filter(|_| sel >= 0) {
        ui.text(TS_BODY, t("Name"));
        ui.next_size(420, 0);
        ui.textbox(ID_COLS + 0x200, None, &mut c.label, 256);
        ui.text(TS_BODY, t("Datentyp"));
        let types: Vec<&str> = TYPE_NAMES.iter().map(|s| t(s)).collect();
        ui.next_size(200, 0);
        ui.combobox(ID_COLS + 0x201, &mut c.kind, &types);
    }
    ui.next_size(420, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Name und Datentyp gelten für die Shape-Daten, die das Verknüpfen und das Aktualisieren schreiben."));
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        apply_columns(app);
    }
    if r != DLG_NONE {
        app.ext.cols.set(0);
        ui.repass();
    }
}

// the dialog's labels, types and order into the recordset (one undo step)
fn apply_columns(app: &mut Diagrammer) {
    let Some(id) = app.ext.cols_id else { return };
    let list = app.ext.col_list.clone();
    let Some(o) = app.opened.as_mut() else { return };
    let Some(mut rs) = o.doc.recordset(id).cloned() else { return };
    let before = rs.clone();
    for (pos, c) in list.iter().enumerate() {
        let Some(col) = rs.columns.get_mut(c.index) else { return };
        let label = c.label.trim();
        col.label = String::from(if label.is_empty() { col.id.as_str() } else { label });
        col.data_type = TYPE_CODES.get(c.kind.max(0) as usize).copied().unwrap_or(kind::TEXT);
        col.display_order = Some(pos as u32);
    }
    // the source's order is kept: no DisplayOrder needed
    if rs.columns.iter().enumerate().all(|(i, c)| c.display_order == Some(i as u32)) && before.columns.iter().all(|c| c.display_order.is_none()) {
        for c in &mut rs.columns {
            c.display_order = None;
        }
    }
    if rs != before {
        let _ = edit_doc::edit(o, commands::label(DATA_COLUMNS), true, |tx, _| data::set_recordset(tx, rs));
    }
}
