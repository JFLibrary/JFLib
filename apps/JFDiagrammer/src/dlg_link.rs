// dlg_link -- (JD8 rest) hyperlinks of shapes (HL-01, RE-09, ZF-12, ZF-15, KB-13): the dialog
// Hyperlinks (Einfügen › Link, Ctrl+K, the shape's menu), following a link (Ctrl+click, the
// rows of the shape's menu), the link pointer and its QuickInfo
//
//   LinkDlg                     the dialog's state: the shapes, their links being edited
//   open(app, ui)               LINK, HYPERLINK: the first selected shape's links
//   dialog(app, ui)             every pass: address, sub-address (a page of the drawing and a
//                               shape on it), description; the list of links with Neu,
//                               Löschen, Als Standard; OK writes them into every selected
//                               shape (one undo step)
//   menu_rows(app) / follow_menu_row(app, ui, k)   the links of the selected shape as rows of
//                               its menu (cmds_text::LINK_ROWS)
//   tip(app, s) -> Option<String>   the QuickInfo over a shape with a link: its description,
//                               else its address or page
//   cursor(app, ui)             the link pointer: an arrow with a chain at its lower right
//   follow_key(app, ui, key)    Ctrl+click on a shape of the page shown: its default link
//   follow(app, ui, s, row)     a link: a page of the drawing (sub-address "Seite" or
//                               "Seite/Shape"): that page, the shape selected and in view; an
//                               address: http, https and mailto with the system (ui.open_url,
//                               PERMISSIONS "url https http mailto"), a file shown in the file
//                               manager (ui.open_folder, a path relative to the drawing's
//                               folder), anything else refused with a notice
//
// Visio's Hyperlink section ([MS-VSDX] Hyperlink_Type): named rows Row_1, Row_2 ... with the cells
// Description, Address, SubAddress, ExtraInfo, Frame, SortKey, NewWindow, Default, Invisible.
// A link to a page of the same drawing has an empty Address and the page's name as SubAddress,
// a shape on it after a slash ("Seite-2/Rechteck.3"; Visio's dialog writes a zoom after a
// comma, which is read and left out here -- Annahme).  Default = 1 marks the link a Ctrl+click
// follows (else the first).

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{RowKey, SecKey, ShapeRef, SheetRef, Unit, Value};
use ui::i18n::{t, tf};
use ui::*;

use crate::Diagrammer;

const ID: u32 = 0x7830_0000;
const ID_LIST: u32 = ID + 0x100;

/// a link being edited
#[derive(Clone, Default, Debug)]
struct Row {
    /// the row's name when the shape has it (Row_1 ...)
    name: Option<String>,
    address: String,
    /// the sub-address' page: 0 none, k a page (pages[k - 1]), pages.len() + 1 the raw text
    page: i32,
    /// a sub-address that is no page of the drawing (a bookmark of another file ...)
    raw: String,
    shape: String,
    description: String,
    default: i32,
}

/// the dialog's state
#[derive(Default)]
pub(crate) struct LinkDlg {
    pub open: Shared<i32>,
    shapes: Vec<ShapeRef>,
    rows: Vec<Row>,
    cur: i32,
    /// the page names offered
    pages: Vec<String>,
    /// the shape menu's rows: the shape and its links' rows
    menu: Vec<(ShapeRef, usize)>,
}

// the page names of the drawing (foreground and background pages, as the tabs show them)
fn page_names(app: &Diagrammer) -> Vec<String> {
    app.opened.as_ref().map_or(Vec::new(), |o| o.tabs.iter().map(|t| diagram_view::export::page_name(&o.doc, t.page)).collect())
}

// a row of the shape's Hyperlink section as the dialog edits it
fn row_of(r: &diagram_core::RowView, pages: &[String]) -> Row {
    let sub = r.text("SubAddress").unwrap_or("").trim();
    let (pg, rest) = match sub.split_once('/') {
        Some((p, s)) => (p, s),
        None => (sub, ""),
    };
    let shape = String::from(rest.split(',').next().unwrap_or("").trim());
    let pg = pg.split(',').next().unwrap_or("").trim();
    let (page, raw) = if sub.is_empty() {
        (0, String::new())
    } else {
        match pages.iter().position(|p| p.eq_ignore_ascii_case(pg)) {
            Some(k) => (k as i32 + 1, String::new()),
            None => (pages.len() as i32 + 1, String::from(sub)),
        }
    };
    Row {
        name: r.name().map(String::from),
        address: String::from(r.text("Address").unwrap_or("")),
        page,
        raw,
        shape: if page as usize > pages.len() { String::new() } else { shape },
        description: String::from(r.text("Description").unwrap_or("")),
        default: r.flag("Default") as i32,
    }
}

impl Row {
    fn sub(&self, pages: &[String]) -> String {
        match self.page {
            k if k >= 1 && (k as usize) <= pages.len() => {
                let p = &pages[k as usize - 1];
                if self.shape.trim().is_empty() { p.clone() } else { format!("{p}/{}", self.shape.trim()) }
            }
            k if k as usize == pages.len() + 1 => self.raw.clone(),
            _ => String::new(),
        }
    }

    fn empty(&self, pages: &[String]) -> bool {
        self.address.trim().is_empty() && self.sub(pages).is_empty()
    }

    // the list's line: description, else address, else sub-address
    fn line(&self, pages: &[String]) -> String {
        let s = [self.description.trim(), self.address.trim()].into_iter().find(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| self.sub(pages));
        if s.is_empty() { String::from(t("(neuer Link)")) } else { s }
    }
}

/// Link, Hyperlink: the dialog with the first selected shape's links
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    let tg = crate::cmds_text::target(app, ui);
    let Some(&first) = tg.shapes.first() else { return };
    let pages = page_names(app);
    let Some(o) = app.opened.as_ref() else { return };
    let mut rows: Vec<Row> = o.doc.hyperlinks(SheetRef::Shape(first)).iter().map(|r| row_of(r, &pages)).collect();
    if rows.is_empty() {
        rows.push(Row::default());
    }
    let cur = rows.iter().position(|r| r.default != 0).unwrap_or(0) as i32;
    let d = &mut app.txt.link;
    d.shapes = tg.shapes;
    d.rows = rows;
    d.cur = cur;
    d.pages = pages;
    d.open.set(1);
    ui.repass();
}

fn label(ui: &mut Ui, s: &str) {
    ui.next_size(130, 0);
    ui.text(TS_BODY, s);
}

/// the dialog (every pass)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.txt.link.open.clone();
    if !ui.dialog_begin(ID, t("Hyperlinks"), &open) {
        return;
    }
    let d = &mut app.txt.link;
    let cur = (d.cur.max(0) as usize).min(d.rows.len().saturating_sub(1));
    let pages = d.pages.clone();
    if let Some(row) = d.rows.get_mut(cur) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        label(ui, t("Adresse"));
        ui.next_size(340, 0);
        ui.textbox(ID + 10, Some(t("https://…, mailto:…, Datei")), &mut row.address, 2048);
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        label(ui, t("Unteradresse"));
        let mut names: Vec<String> = alloc::vec![String::from(t("(keine)"))];
        names.extend(pages.iter().cloned());
        if !row.raw.is_empty() {
            names.push(row.raw.clone());
        }
        let names: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        ui.next_size(200, 0);
        ui.combobox(ID + 11, &mut row.page, &names);
        ui.next_size(132, 0);
        let page_link = row.page >= 1 && row.page as usize <= pages.len();
        ui.next_flags(if page_link { 0 } else { ST_DISABLED });
        ui.textbox(ID + 12, Some(t("Shape (optional)")), &mut row.shape, 256);
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        label(ui, t("Beschreibung"));
        ui.next_size(340, 0);
        ui.textbox(ID + 13, Some(t("QuickInfo")), &mut row.description, 1024);
        ui.box_end();
    }
    ui.spacer(6);
    ui.text(TS_BODY_STRONG, t("Links des Shapes"));
    ui.box_begin(0, BOX_H, 8);
    let lines: Vec<String> = d.rows.iter().map(|r| {
        let l = r.line(&pages);
        if r.default != 0 { format!("{l}  ({})", t("Standard")) } else { l }
    }).collect();
    let mut sel = cur as i32;
    ui.next_size(340, 130);
    let (a, b) = ui.list_begin(ID_LIST, &mut sel, lines.len() as i32, 26);
    for i in a..b {
        ui.list_item(i, &lines[i as usize]);
    }
    ui.list_end(&mut sel);
    d.cur = sel;
    ui.box_begin(0, BOX_V, 6);
    ui.next_size(130, 0);
    if ui.button(ID + 20, t("Neu")) & EV_ACT != 0 {
        d.rows.push(Row::default());
        d.cur = d.rows.len() as i32 - 1;
        ui.repass();
    }
    ui.next_size(130, 0);
    if ui.button(ID + 21, t("Löschen")) & EV_ACT != 0 && !d.rows.is_empty() {
        d.rows.remove(cur);
        if d.rows.is_empty() {
            d.rows.push(Row::default());
        }
        d.cur = (cur as i32).min(d.rows.len() as i32 - 1);
        ui.repass();
    }
    ui.next_size(130, 0);
    if ui.button(ID + 22, t("Als Standard")) & EV_ACT != 0 {
        for (k, r) in d.rows.iter_mut().enumerate() {
            r.default = (k == cur) as i32;
        }
        ui.repass();
    }
    ui.box_end();
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        apply(app, ui);
    }
    if r != DLG_NONE {
        app.txt.link.open.set(0);
        ui.repass();
    }
}

// the links into every shape of the dialog: rows kept by name, new ones as Row_n, the others
// deleted (one undo step)
fn apply(app: &mut Diagrammer, ui: &mut Ui) {
    let d = &app.txt.link;
    let pages = d.pages.clone();
    let rows: Vec<Row> = d.rows.iter().filter(|r| !r.empty(&pages)).cloned().collect();
    let shapes = d.shapes.clone();
    let Some(o) = app.opened.as_mut() else { return };
    let sec = SecKey::one(o.doc.intern("Hyperlink"));
    let _ = crate::edit_doc::edit(o, t("Hyperlinks"), true, |tx, _eng| {
        for (k, &s) in shapes.iter().enumerate() {
            let sheet = SheetRef::Shape(s);
            // the rows the shape has: the ones not kept go
            let have: Vec<RowKey> = tx.doc().rows(sheet, sec).iter().map(|r| r.key).collect();
            let kept: Vec<String> = if k == 0 { rows.iter().filter_map(|r| r.name.clone()).collect() } else { Vec::new() };
            for key in &have {
                let name = match key {
                    RowKey::Named(n) => String::from(tx.doc().str(*n)),
                    RowKey::Index(_) => String::new(),
                };
                if !kept.contains(&name) {
                    tx.delete_row(sheet, sec, *key)?;
                }
            }
            let mut next = 1;
            for r in &rows {
                let name = match (&r.name, k) {
                    (Some(n), 0) => n.clone(),
                    _ => loop {
                        let n = format!("Row_{next}");
                        next += 1;
                        let taken = rows.iter().any(|x| x.name.as_deref() == Some(n.as_str()) && k == 0) || {
                            let doc = tx.doc();
                            doc.sym(&n).is_some_and(|sym| doc.has_row(sheet, sec, RowKey::Named(sym)))
                        };
                        if !taken {
                            break n;
                        }
                    },
                };
                tx.set_hyperlink(sheet, &name, r.address.trim(), &r.sub(&pages), r.description.trim())?;
                let a = tx.named("Hyperlink", &name, "Default");
                tx.set_constant(sheet, &a, Value::Num(r.default as f64), Some(Unit::Bool))?;
            }
        }
        Ok(())
    });
    ui.repass();
}

// ---------------------------------------------------------------- following links

// the row a Ctrl+click follows: the default one, else the first with an address or a page
fn default_row(doc: &diagram_core::Document, s: ShapeRef) -> Option<usize> {
    let rows = doc.hyperlinks(SheetRef::Shape(s));
    let useful = |r: &diagram_core::RowView| !r.text("Address").unwrap_or("").trim().is_empty() || !r.text("SubAddress").unwrap_or("").trim().is_empty();
    rows.iter().position(|r| r.flag("Default") && useful(r)).or_else(|| rows.iter().position(useful))
}

// what a link shows: description, else address, else sub-address
fn link_text(r: &diagram_core::RowView) -> String {
    ["Description", "Address", "SubAddress"].iter().filter_map(|c| r.text(c)).map(str::trim).find(|s| !s.is_empty()).map(String::from).unwrap_or_default()
}

/// the QuickInfo over a shape with a link (None: no link)
pub(crate) fn tip(app: &Diagrammer, s: ShapeRef) -> Option<String> {
    let o = app.opened.as_ref()?;
    let k = default_row(&o.doc, s)?;
    let rows = o.doc.hyperlinks(SheetRef::Shape(s));
    let text = link_text(rows.get(k)?);
    Some(format!("{text}\n{}", t("Strg+Klicken, um dem Link zu folgen")))
}

/// the shape menu's rows: the links of the single selected shape
pub(crate) fn menu_rows(app: &mut Diagrammer) -> Vec<String> {
    let sel = crate::edit_canvas::selected(app);
    let mut menu = Vec::new();
    let mut out = Vec::new();
    if let (Some(o), [s]) = (app.opened.as_ref(), sel.as_slice()) {
        for (k, r) in o.doc.hyperlinks(SheetRef::Shape(*s)).iter().enumerate().take(crate::cmds_text::LINK_ROWS_MAX as usize) {
            let text = link_text(r);
            if !text.is_empty() {
                out.push(text);
                menu.push((*s, k));
            }
        }
    }
    app.txt.link.menu = menu;
    out
}

/// a row of the shape's menu
pub(crate) fn follow_menu_row(app: &mut Diagrammer, ui: &mut Ui, k: usize) {
    if let Some(&(s, row)) = app.txt.link.menu.get(k) {
        follow(app, ui, s, row);
    }
}

/// Ctrl+click on a shape of the page shown: its link
pub(crate) fn follow_key(app: &mut Diagrammer, ui: &mut Ui, key: u64) {
    let Some(page) = app.model.page else { return };
    let s = crate::edit_canvas::shape_of(page, key);
    let row = app.opened.as_ref().and_then(|o| default_row(&o.doc, s));
    if let Some(row) = row {
        follow(app, ui, s, row);
    }
}

// a shape of a page by its name (Name or NameU, any case)
fn shape_named(doc: &diagram_core::Document, page: diagram_core::PageRef, name: &str) -> Option<ShapeRef> {
    let mut stack: Vec<ShapeRef> = doc.top_shapes(diagram_core::Owner::Page(page)).collect();
    while let Some(s) = stack.pop() {
        let sh = doc.shape(s)?;
        if sh.name().eq_ignore_ascii_case(name) || sh.name_u().eq_ignore_ascii_case(name) {
            return Some(s);
        }
        stack.extend(sh.children());
    }
    None
}

/// follow link `row` of shape `s`
pub(crate) fn follow(app: &mut Diagrammer, ui: &mut Ui, s: ShapeRef, row: usize) {
    let Some(o) = app.opened.as_ref() else { return };
    let rows = o.doc.hyperlinks(SheetRef::Shape(s));
    let Some(r) = rows.get(row) else { return };
    let address = String::from(r.text("Address").unwrap_or("").trim());
    let sub = String::from(r.text("SubAddress").unwrap_or("").trim());
    if address.is_empty() {
        // a page of this drawing, a shape on it
        let Some(page) = diagram_view::export::page_by_name(&o.doc, &sub) else {
            app.error = Some((String::from(t("Der Link lässt sich nicht öffnen.")), tf("Die Seite „{}“ gibt es nicht.", &[&sub])));
            ui.repass();
            return;
        };
        let Some(k) = o.tabs.iter().position(|t| t.page == page) else { return };
        let name = sub.split_once('/').map(|(_, n)| n.split(',').next().unwrap_or("").trim()).unwrap_or("");
        let target = if name.is_empty() { None } else { shape_named(&o.doc, page, name) };
        crate::cmds_text::show_shape(app, ui, k, target);
        return;
    }
    open_address(app, ui, &address);
}

// an address with the system
fn open_address(app: &mut Diagrammer, ui: &mut Ui, address: &str) {
    let scheme = address.split_once(':').map_or("", |(s, _)| s).to_ascii_lowercase();
    let url = if address.to_ascii_lowercase().starts_with("www.") { format!("https://{address}") } else { String::from(address) };
    let fail = |app: &mut Diagrammer, ui: &mut Ui, why: String| {
        app.error = Some((String::from(t("Der Link lässt sich nicht öffnen.")), why));
        ui.repass();
    };
    if matches!(scheme.as_str(), "http" | "https" | "mailto") || url.starts_with("https://") {
        if ui.open_url(&url) != 0 {
            fail(app, ui, String::from(address));
        }
        return;
    }
    // a file: a drive letter or no scheme (relative to the drawing's folder), file:
    let is_file = scheme.is_empty() || scheme.len() == 1 || scheme == "file";
    if !is_file {
        fail(app, ui, tf("Links der Art „{}:“ öffnet JFDiagrammer nicht.", &[&scheme]));
        return;
    }
    let mut path = String::from(address.strip_prefix("file:///").or_else(|| address.strip_prefix("file:")).unwrap_or(address));
    let absolute = path.starts_with('/') || path.starts_with('\\') || path.get(1..2) == Some(":");
    if !absolute {
        let base = app.opened.as_ref().and_then(|o| o.ed.path.as_ref()).and_then(|p| p.rsplit_once(['/', '\\']).map(|x| String::from(x.0)));
        match base {
            Some(b) => path = format!("{b}/{path}"),
            None => {
                fail(app, ui, String::from(t("Ein relativer Link braucht eine gespeicherte Zeichnung.")));
                return;
            }
        }
    }
    if ui.open_folder(&path) != 0 {
        fail(app, ui, path);
    }
}

// ---------------------------------------------------------------- the link pointer

// the distance of p to the segment a..b
fn seg_dist(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    let (qx, qy) = (a[0] + dx * t - p[0], a[1] + dy * t - p[1]);
    let d2 = qx * qx + qy * qy;
    if d2 > 0.0 { rt::x87::pow(d2 as f64, 0.5) as f32 } else { 0.0 }
}

// the arrow (cursor units, 32 x 32)
const ARROW: [[f32; 2]; 7] = [[1.0, 1.0], [1.0, 21.0], [5.8, 16.6], [9.4, 24.4], [12.2, 23.2], [8.8, 15.4], [15.0, 15.4]];

fn inside(p: [f32; 2], poly: &[[f32; 2]]) -> bool {
    let mut c = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
        j = i;
    }
    c
}

/// the link pointer (made once): an arrow with a chain of two links at its lower right
pub(crate) fn cursor(app: &mut Diagrammer, ui: &mut Ui) -> u32 {
    if app.txt.link_cursor == 0 {
        let f: CursorDraw = Rc::new(|g: &mut Gfx, dpi: i32| {
            let px = (32 * dpi + 95) / 96;
            let k = 32.0 / px as f32;
            // the two links of the chain: their middle lines and radii
            let links = [([16.5f32, 27.0f32], [21.0f32, 22.5f32]), ([21.5, 28.5], [26.0, 24.0])];
            for y in 0..px {
                for x in 0..px {
                    let p = [(x as f32 + 0.5) * k, (y as f32 + 0.5) * k];
                    let mut c = 0u32;
                    if inside(p, &ARROW) {
                        let edge = (0..ARROW.len()).map(|i| seg_dist(p, ARROW[i], ARROW[(i + 1) % ARROW.len()])).fold(f32::MAX, f32::min);
                        c = if edge < 1.0 { 0xFF00_0000 } else { 0xFFFF_FFFF };
                    }
                    // the rings in the accent blue on a white halo, over the arrow
                    let d = links.iter().map(|(a, b)| seg_dist(p, *a, *b));
                    let (mut ring, mut halo) = (false, false);
                    for d in d {
                        ring |= (1.5..2.7).contains(&d);
                        halo |= d < 3.7;
                    }
                    if ring {
                        c = 0xFF1F_5FA8;
                    } else if halo {
                        c = 0xFFFF_FFFF;
                    }
                    if c != 0 {
                        g.dc.set_color(c);
                        g.dc.fill(g.sf, x, y, 1, 1);
                    }
                }
            }
        });
        app.txt.link_cursor = ui.cursor_new(f, 32, 32, 1 | 1 << 16);
    }
    ui.cursor_custom(app.txt.link_cursor);
    if app.txt.link_cursor != 0 { CUR_CUSTOM } else { CUR_HAND }
}
