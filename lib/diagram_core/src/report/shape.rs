// shape -- (NO-22, DA-13) a report's table on the page: a group of cell shapes instead of
// Visio's embedded Excel object, found again and built anew in place
//
//   report_layer(tx, page, name, name_univ) -> index   the layer of report tables (found by
//                                       its universal name, else added)
//   place(tx, eng, owner, top_left, &table, &Place) -> the group
//   report_shape(doc, shape) -> Option<ReportShape>      the table a shape is (or is part of)
//   refresh(tx, eng, shape, &table) -> the new group     the same place, marks and layers
//
// The table: a title row (no line, no fill, 11 pt bold), the heading row (8 pt bold on a light
// grey), the rows (8 pt; group headings, subtotals and the grand total bold, the totals on a
// lighter grey), each cell a rectangle with its text (numeric columns right-aligned), grouped
// with ops::group (the members scale with the group as in any group).  Sizes are on paper
// (rows 0.2 in, columns by their longest text, 0.5 to 3.5 in) and grow with the page's scale.
// The group carries the marks: User.JFD_Report = 1, JFD_ReportName (the definition's name),
// JFD_ReportDef (a copy of the definition's XML, Visio's "Kopie der Berichtsdefinition") or
// JFD_ReportLink (where the definition is: "" the drawing's definition of that name, else the
// path of a .vrd file, Visio's "Verknüpfung mit der Berichtsdefinition"); SelectMode 0 (a click
// takes the table, not a cell).  At most MAX_ROWS rows are drawn, then a row "…".

use alloc::string::String;
use alloc::vec::Vec;

use super::run::{RowKind, Table};
use crate::cell::Value;
use crate::document::{Document, Owner, PageRef, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::names;
use crate::ops::{self, Basic};
use crate::sheet::{CellAddr, RowKey, SecKey};
use crate::shape::Text;
use crate::units::Unit;

/// User cell of a report table: 1
pub const MARK: &str = "JFD_Report";
/// User cell: the definition's name (STR)
pub const MARK_NAME: &str = "JFD_ReportName";
/// User cell: a copy of the definition's XML (STR)
pub const MARK_DEF: &str = "JFD_ReportDef";
/// User cell: where the definition is ("" the drawing, else a .vrd path; STR)
pub const MARK_LINK: &str = "JFD_ReportLink";
/// the most rows a table shape draws
pub const MAX_ROWS: usize = 2000;

const ROW_H: f64 = 0.2;
const TITLE_H: f64 = 0.32;
const CELL_PT: f64 = 8.0;
const TITLE_PT: f64 = 11.0;
const MARGIN_PT: f64 = 3.0;
const INK: &str = "#1D2228";
const GRID: &str = "#A3A8AF";
const HEAD: &str = "#E3E6EA";
const TOTAL: &str = "#F1F2F4";
const PAPER: &str = "#FFFFFF";

/// what a placed table carries besides its cells
#[derive(Clone, Debug, Default)]
pub struct Place {
    /// the definition's name
    pub name: String,
    /// a copy of the definition (its XML), or None when it is linked
    pub def_copy: Option<String>,
    /// the link when it is not copied: "" the drawing's definition of that name, else a .vrd path
    pub link: Option<String>,
    /// the layers of the page the table and its cells go on
    pub layers: Vec<u32>,
}

/// a table shape found on a page
#[derive(Clone, PartialEq, Debug)]
pub struct ReportShape {
    /// the group
    pub shape: ShapeRef,
    /// JFD_ReportName
    pub name: String,
    /// JFD_ReportDef (None: none or empty)
    pub def: Option<String>,
    /// JFD_ReportLink (None: none)
    pub link: Option<String>,
}

fn user_value(doc: &Document, s: ShapeRef, name: &str) -> Option<Value> {
    doc.user_cells(SheetRef::Shape(s)).into_iter().find(|r| r.name() == Some(name)).map(|r| r.get("Value").value())
}

fn user_text(doc: &Document, s: ShapeRef, name: &str) -> Option<String> {
    match user_value(doc, s, name)? {
        Value::Text(t) => Some(String::from(doc.str(t))),
        Value::Num(x) => Some(alloc::format!("{x}")),
        Value::Absent => Some(String::new()),
    }
}

/// the report table `shape` is, or belongs to (a cell of it)
pub fn report_shape(doc: &Document, shape: ShapeRef) -> Option<ReportShape> {
    let mut cur = Some(shape);
    let mut depth = 0;
    while let (Some(s), true) = (cur, depth < 64) {
        if user_value(doc, s, MARK).is_some_and(|v| v != Value::Num(0.0)) {
            return Some(ReportShape {
                shape: s,
                name: user_text(doc, s, MARK_NAME).unwrap_or_default(),
                def: user_text(doc, s, MARK_DEF).filter(|d| !d.trim().is_empty()),
                link: user_text(doc, s, MARK_LINK),
            });
        }
        cur = doc.shape(s)?.parent();
        depth += 1;
    }
    None
}

/// the layer report tables go on: the page's layer with this universal name, else a new one
pub fn report_layer(tx: &mut Transaction, page: PageRef, name: &str, name_univ: &str) -> Result<u32, EditError> {
    if let Some(ix) = tx.doc().layer_by_name(page, name_univ) {
        return Ok(ix);
    }
    tx.add_layer(page, name, name_univ)
}

// an estimate of a text's width on paper (inches) at `pt` points; bold a little wider
fn text_w(s: &str, pt: f64, bold: bool) -> f64 {
    let n = s.chars().count() as f64;
    n * pt * if bold { 0.6 } else { 0.55 } / 72.0
}

fn color(tx: &mut Transaction, rgb: &str) -> Value {
    Value::Text(tx.intern(rgb))
}

struct Look {
    fill: Option<&'static str>,
    line: bool,
    pt: f64,
    bold: bool,
    right: bool,
}

// a cell: a rectangle between two page points with its look and text
fn cell(tx: &mut Transaction, eng: &mut Engine, owner: Owner, a: [f64; 2], b: [f64; 2], text: &str, look: &Look) -> Result<ShapeRef, EditError> {
    let s = ops::new_shape(tx, eng, owner, Basic::Rectangle, a, b)?;
    let sheet = SheetRef::Shape(s);
    let set = |tx: &mut Transaction, cell: &str, v: Value, u: Option<Unit>| {
        let a = CellAddr::single(tx.intern(cell));
        tx.set_constant(sheet, &a, v, u)
    };
    match look.fill {
        Some(rgb) => {
            let c = color(tx, rgb);
            set(tx, "FillForegnd", c, None)?;
            set(tx, "FillPattern", Value::Num(1.0), None)?;
        }
        None => set(tx, "FillPattern", Value::Num(0.0), None)?,
    }
    if look.line {
        let c = color(tx, GRID);
        set(tx, "LineColor", c, None)?;
        set(tx, "LinePattern", Value::Num(1.0), None)?;
        set(tx, "LineWeight", Value::Num(0.5 / 72.0), Some(Unit::Points))?;
    } else {
        set(tx, "LinePattern", Value::Num(0.0), None)?;
    }
    set(tx, "ShdwPattern", Value::Num(0.0), None)?;
    set(tx, "VerticalAlign", Value::Num(1.0), None)?;
    for m in ["LeftMargin", "RightMargin"] {
        set(tx, m, Value::Num(MARGIN_PT / 72.0), Some(Unit::Points))?;
    }
    for m in ["TopMargin", "BottomMargin"] {
        set(tx, m, Value::Num(1.0 / 72.0), Some(Unit::Points))?;
    }
    let ch = SecKey::one(names::CHARACTER);
    let pa = SecKey::one(names::PARAGRAPH);
    let row0 = RowKey::Index(0);
    // (a drawing without styles has no Character or Paragraph row to write into)
    for sk in [ch, pa] {
        if !tx.doc().has_row(sheet, sk, row0) {
            tx.add_row(sheet, sk, Some(row0), None)?;
        }
    }
    let size = CellAddr::row(ch, row0, tx.intern("Size"));
    tx.set_constant(sheet, &size, Value::Num(look.pt / 72.0), Some(Unit::Points))?;
    let style = CellAddr::row(ch, row0, tx.intern("Style"));
    tx.set_constant(sheet, &style, Value::Num(if look.bold { 1.0 } else { 0.0 }), None)?;
    let ink = color(tx, INK);
    let col = CellAddr::row(ch, row0, tx.intern("Color"));
    tx.set_constant(sheet, &col, ink, None)?;
    let align = CellAddr::row(pa, row0, tx.intern("HorzAlign"));
    tx.set_constant(sheet, &align, Value::Num(if look.right { 2.0 } else { 0.0 }), None)?;
    let one_line: String = text.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
    if !one_line.is_empty() {
        tx.set_text(s, Some(Text::plain(&one_line)))?;
    }
    Ok(s)
}

/// put a report's table on a page or master: its top left corner at `at` (page coordinates)
/// -> the group
pub fn place(tx: &mut Transaction, eng: &mut Engine, owner: Owner, at: [f64; 2], table: &Table, p: &Place) -> Result<ShapeRef, EditError> {
    let k = crate::dock::reach(tx.doc(), owner, 1.0);
    let ncols = table.columns.len().max(1);
    // the columns' widths on paper by their longest text
    let mut widths: Vec<f64> = (0..ncols)
        .map(|c| {
            let head = table.columns.get(c).map_or(0.0, |col| text_w(&col.name, CELL_PT, true));
            let body = table.rows.iter().take(MAX_ROWS).filter_map(|r| r.cells.get(c)).map(|x| text_w(&x.text, CELL_PT, true)).fold(0.0, f64::max);
            (head.max(body) + 2.0 * MARGIN_PT / 72.0 + 0.06).clamp(0.5, 3.5)
        })
        .collect();
    let total: f64 = widths.iter().sum();
    let title_w = text_w(&table.title, TITLE_PT, true) + 0.2;
    if title_w > total && ncols > 0 {
        // a long title: the last column takes the rest
        widths[ncols - 1] += title_w - total;
    }
    let total: f64 = widths.iter().sum();
    let mut cells: Vec<ShapeRef> = Vec::new();
    let mut y = at[1];
    let x0 = at[0];
    if !table.title.is_empty() {
        let look = Look { fill: None, line: false, pt: TITLE_PT, bold: true, right: false };
        cells.push(cell(tx, eng, owner, [x0, y - TITLE_H * k], [x0 + total * k, y], &table.title, &look)?);
        y -= TITLE_H * k;
    }
    let mut row = |tx: &mut Transaction, eng: &mut Engine, y: f64, texts: &[&str], fill: Option<&'static str>, bold: bool, numeric: &[bool]| -> Result<(), EditError> {
        let mut x = x0;
        for (c, w) in widths.iter().enumerate() {
            let t = texts.get(c).copied().unwrap_or("");
            let look = Look { fill, line: true, pt: CELL_PT, bold, right: numeric.get(c).copied().unwrap_or(false) };
            cells.push(cell(tx, eng, owner, [x, y - ROW_H * k], [x + w * k, y], t, &look)?);
            x += w * k;
        }
        Ok(())
    };
    let heads: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
    row(tx, eng, y, &heads, Some(HEAD), true, &[])?;
    y -= ROW_H * k;
    let numeric: Vec<bool> = table.columns.iter().map(|c| c.numeric).collect();
    for r in table.rows.iter().take(MAX_ROWS) {
        let texts: Vec<&str> = r.cells.iter().map(|c| c.text.as_str()).collect();
        let (fill, bold) = match r.kind {
            RowKind::Item => (Some(PAPER), false),
            RowKind::Group => (Some(PAPER), true),
            RowKind::Summary(_) => (Some(TOTAL), true),
            RowKind::Grand(_) => (Some(HEAD), true),
        };
        row(tx, eng, y, &texts, fill, bold, &numeric)?;
        y -= ROW_H * k;
    }
    if table.rows.len() > MAX_ROWS {
        row(tx, eng, y, &["…"], Some(PAPER), false, &[])?;
    }
    drop(row);
    let g = ops::group(tx, eng, owner, &cells)?;
    let gs = SheetRef::Shape(g);
    tx.set_user(gs, MARK, Value::Num(1.0), None)?;
    let name = Value::Text(tx.intern(&p.name));
    tx.set_user(gs, MARK_NAME, name, Some(Unit::Str))?;
    match (&p.def_copy, &p.link) {
        (Some(xml), _) => {
            let v = Value::Text(tx.intern(xml));
            tx.set_user(gs, MARK_DEF, v, Some(Unit::Str))?;
        }
        (None, link) => {
            let v = Value::Text(tx.intern(link.as_deref().unwrap_or("")));
            tx.set_user(gs, MARK_LINK, v, Some(Unit::Str))?;
        }
    }
    let sm = CellAddr::single(tx.intern("SelectMode"));
    tx.set_constant(gs, &sm, Value::Num(0.0), None)?;
    let title = if table.title.is_empty() { p.name.clone() } else { table.title.clone() };
    tx.set_shape_names(g, Some(&title), None)?;
    if !p.layers.is_empty() {
        tx.set_layers(g, &p.layers)?;
        for &c in &cells {
            tx.set_layers(c, &p.layers)?;
        }
    }
    Ok(g)
}

/// build a report table anew where it is (its top left corner, its marks and layers stay), as
/// part of the caller's transaction -> the new group
pub fn refresh(tx: &mut Transaction, eng: &mut Engine, shape: ShapeRef, table: &Table) -> Result<ShapeRef, EditError> {
    let rs = report_shape(tx.doc(), shape).ok_or(EditError::NoShape)?;
    let g = rs.shape;
    let b = ops::bounds(tx.doc(), &[g]).ok_or(EditError::NoShape)?;
    let layers = tx.doc().layer_members(g);
    let p = Place { name: rs.name, def_copy: rs.def, link: if rs.link.is_some() { rs.link } else { Some(String::new()) }, layers };
    tx.delete_shapes(&[g])?;
    place(tx, eng, g.owner, [b[0], b[3]], table, &p)
}
