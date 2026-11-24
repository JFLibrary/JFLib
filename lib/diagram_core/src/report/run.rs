// run -- a report definition run against a drawing: the shapes it takes, their values, the
// table with groups, summaries and the grand total
//
//   run(doc, &def, &Ctx) -> Table
//   Ctx { page, selection, labels }    the page shown, the selected shapes, the words of the
//                                      summary rows (the app's language; English by default)
//   Table { title, columns, rows, shapes }   rows of RowKind Item, Group (a group's heading),
//                                      Summary(f) (a group's subtotal), Grand(f) (over all);
//                                      None instead of a function: a row with several
//
// The layouts:
// * no grouping: the shapes' rows (sorted, duplicates left out when the definition says so),
//   then one row per summary function any column has (Grand), the function's name in the first
//   empty cell
// * grouping with the rows shown: per group a heading row (the group's value in its column),
//   its rows, one subtotal row per function; with GrandSummary the summaries over all at the end
// * grouping, summaries only (ItemDisplay 0): one row per group -- the group's value, then a
//   column per (field, function): "Anzahl" for the grouping field itself, else
//   "<field> (<function>)" -- and with GrandSummary a last row over all (Visio's inventory:
//   Master-Name and the count of each)
//
// Values: a number in the definition's precision with the drawing's decimal sign
// (Document::fields), with the unit's abbreviation when ShowUnits is on and the unit has one
// (lengths in the unit of their cell, per cent, angles, durations); dates in the general date
// format; Booleans as Labels::yes / no; texts as they are.  Each cell also carries its value
// typed (Val): the number in the shown unit, rounded to the precision, for CSV and XLSX.
// Sorting compares numbers as numbers and texts by their characters (case as the sort key
// says); an empty or missing value comes first.  Criteria: =, <>, <, <=, >, >= compare numbers
// when both sides are numbers (the criterion's value in its Unit, else in the shape's unit),
// else texts (case as the definition says); exists / does not exist ask whether the shape has
// the field.  Every criterion must hold, one with AndOr 0 is an "or" with the ones before.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use super::{Builtin, Condition, Filter, Func, Property, ReportDef, Scope, FUNCS};
use crate::cell::Value;
use crate::document::{Document, Owner, PageRef, ShapeRef, SheetRef};
use crate::field::{self, Env, Shown};
use crate::formula::math;
use crate::inherit::Resolved;
use crate::names;
use crate::shape::ShapeKind;
use crate::sheet::CellAddr;
use crate::units::{Dimension, Unit};
use crate::views::RowView;

/// the words of the summary rows and of truth values (the app's language)
#[derive(Clone, PartialEq, Debug)]
pub struct Labels {
    /// Func::Total
    pub total: String,
    /// Func::Average
    pub average: String,
    /// Func::Count
    pub count: String,
    /// Func::Min
    pub min: String,
    /// Func::Max
    pub max: String,
    /// the rows over all groups
    pub grand: String,
    /// a true value
    pub yes: String,
    /// a false value
    pub no: String,
}

impl Default for Labels {
    fn default() -> Labels {
        let s = String::from;
        Labels { total: s("Total"), average: s("Average"), count: s("Count"), min: s("Minimum"), max: s("Maximum"), grand: s("Grand total"), yes: s("TRUE"), no: s("FALSE") }
    }
}

impl Labels {
    /// the word of a function
    pub fn func(&self, f: Func) -> &str {
        match f {
            Func::Total => &self.total,
            Func::Average => &self.average,
            Func::Count => &self.count,
            Func::Min => &self.min,
            Func::Max => &self.max,
        }
    }
}

/// where a report runs
#[derive(Clone, Debug, Default)]
pub struct Ctx<'a> {
    /// the page shown (Scope::Page)
    pub page: Option<PageRef>,
    /// the selected shapes (Scope::Selection)
    pub selection: &'a [ShapeRef],
    /// the words of the summary rows
    pub labels: Labels,
}

/// a cell's value, typed
#[derive(Clone, PartialEq, Debug)]
pub enum Val {
    /// nothing
    Empty,
    /// a number in the unit shown
    Num(f64),
    /// a text
    Text(String),
    /// a truth value
    Bool(bool),
    /// a date (days since 30 December 1899)
    Date(f64),
}

/// a cell of the table: its text and its value
#[derive(Clone, PartialEq, Debug)]
pub struct Cell {
    /// as shown
    pub text: String,
    /// typed
    pub value: Val,
}

impl Cell {
    fn empty() -> Cell {
        Cell { text: String::new(), value: Val::Empty }
    }

    fn text(s: &str) -> Cell {
        Cell { text: String::from(s), value: if s.is_empty() { Val::Empty } else { Val::Text(String::from(s)) } }
    }
}

/// what a row of the table is
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RowKind {
    /// a shape's row (summaries only: a group's row)
    Item,
    /// a group's heading
    Group,
    /// a group's subtotal (None: several functions in one row)
    Summary(Option<Func>),
    /// the summary over all (None: several functions in one row)
    Grand(Option<Func>),
}

/// a row of the table
#[derive(Clone, PartialEq, Debug)]
pub struct Row {
    /// what it is
    pub kind: RowKind,
    /// a cell per column
    pub cells: Vec<Cell>,
}

/// a column of the table
#[derive(Clone, PartialEq, Debug)]
pub struct Column {
    /// its heading
    pub name: String,
    /// its values are numbers (right-aligned)
    pub numeric: bool,
}

/// a report's result
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Table {
    /// the title (the definition's Title, else its Name)
    pub title: String,
    /// the columns
    pub columns: Vec<Column>,
    /// the rows
    pub rows: Vec<Row>,
    /// how many shapes the report took
    pub shapes: usize,
}

// ---------------------------------------------------------------- values of a shape

/// a property's value on a shape
#[derive(Clone, PartialEq, Debug)]
enum Raw {
    /// the shape does not have the property
    Missing,
    /// it has it, without a value
    Empty,
    /// a number in internal units and its cell's unit
    Num(f64, Option<Unit>),
    Text(String),
    Bool(bool),
    Date(f64),
}

impl Raw {
    fn present(&self) -> bool {
        !matches!(self, Raw::Missing)
    }
}

// a shape data row by its name, else by its label
fn data_row<'d>(doc: &'d Document, sheet: SheetRef, name: &str) -> Option<RowView<'d>> {
    let rows = doc.shape_data(sheet);
    rows.iter().find(|r| r.name() == Some(name)).copied().or_else(|| rows.iter().find(|r| r.text("Label").is_some_and(|l| !l.is_empty() && l.eq_ignore_ascii_case(name))).copied())
}

fn truth(s: &str) -> Option<bool> {
    match s.trim().to_ascii_uppercase().as_str() {
        "TRUE" | "WAHR" | "1" => Some(true),
        "FALSE" | "FALSCH" | "0" => Some(false),
        _ => None,
    }
}

// a cell's value as a Raw; `ty` the shape data Type (3 Boolean, 5 date)
fn cell_raw(doc: &Document, r: Resolved, ty: Option<f64>) -> Raw {
    let unit = r.unit();
    let boolean = ty == Some(3.0) || unit == Some(Unit::Bool);
    match r.value() {
        Value::Absent => Raw::Empty,
        Value::Text(s) => {
            let t = doc.str(s);
            if boolean {
                if let Some(b) = truth(t) {
                    return Raw::Bool(b);
                }
            }
            if t.is_empty() { Raw::Empty } else { Raw::Text(String::from(t)) }
        }
        Value::Num(x) if boolean => Raw::Bool(x != 0.0),
        Value::Num(x) if unit == Some(Unit::Date) || (ty == Some(5.0) && unit.is_none()) => Raw::Date(x),
        Value::Num(x) => Raw::Num(x, unit),
    }
}

fn raw(doc: &Document, s: ShapeRef, p: &Property) -> Raw {
    let sheet = SheetRef::Shape(s);
    match p {
        Property::Builtin(b) => {
            let cell = match b {
                Builtin::Text => {
                    let t = field::plain(doc, s);
                    return if t.is_empty() { Raw::Empty } else { Raw::Text(t) };
                }
                Builtin::Master => {
                    let Some(m) = doc.shape(s).and_then(|x| x.header().master).and_then(|id| doc.master_by_id(id)).and_then(|m| doc.master(m)) else { return Raw::Missing };
                    return match m.header.name.or(m.header.name_u) {
                        Some(n) => Raw::Text(String::from(doc.str(n))),
                        None => Raw::Empty,
                    };
                }
                Builtin::Name => return doc.shape(s).map_or(Raw::Missing, |x| Raw::Text(x.name())),
                Builtin::NameId => return doc.shape(s).map_or(Raw::Missing, |x| Raw::Text(format!("Sheet.{}", x.id()))),
                Builtin::PinX => names::PIN_X,
                Builtin::PinY => names::PIN_Y,
                Builtin::Width => names::WIDTH,
                Builtin::Height => names::HEIGHT,
                Builtin::Other(_) => return Raw::Missing,
            };
            let r = doc.resolve(sheet, &CellAddr::single(cell));
            match r.num() {
                Some(x) => Raw::Num(x, Some(r.unit().filter(|u| u.dimension() == Dimension::Length).unwrap_or(Unit::Inches))),
                None => Raw::Missing,
            }
        }
        Property::Data(n) => match data_row(doc, sheet, n) {
            Some(row) => cell_raw(doc, row.get("Value"), row.num("Type")),
            None => Raw::Missing,
        },
        Property::User(n) => match doc.user_cells(sheet).into_iter().find(|r| r.name() == Some(n.as_str())) {
            Some(row) => cell_raw(doc, row.get("Value"), None),
            None => Raw::Missing,
        },
    }
}

// ---------------------------------------------------------------- the shapes

fn is_report(doc: &Document, s: ShapeRef) -> bool {
    doc.user_cells(SheetRef::Shape(s)).iter().any(|r| r.name() == Some(super::MARK))
}

fn expand(doc: &Document, s: ShapeRef, out: &mut Vec<ShapeRef>, depth: u32) {
    let Some(sh) = doc.shape(s) else { return };
    if depth > 32 || sh.is_deleted() || matches!(sh.header().kind, ShapeKind::Guide) || is_report(doc, s) {
        return;
    }
    let plain_group = sh.header().kind == ShapeKind::Group && sh.header().master.is_none() && doc.shape_data(SheetRef::Shape(s)).is_empty();
    if plain_group {
        for c in sh.children() {
            expand(doc, c, out, depth + 1);
        }
    } else if !out.contains(&s) {
        out.push(s);
    }
}

/// the shapes a definition takes (see the module head)
fn shapes(doc: &Document, def: &ReportDef, ctx: &Ctx) -> Vec<ShapeRef> {
    let mut tops: Vec<ShapeRef> = Vec::new();
    match def.scope {
        Scope::Page => {
            if let Some(p) = ctx.page.filter(|p| doc.page(*p).is_some()) {
                tops.extend(doc.top_shapes(Owner::Page(p)));
            }
        }
        Scope::Selection => tops.extend(ctx.selection.iter().copied()),
        _ => {
            let pages: Vec<PageRef> = doc.foreground_pages().collect();
            for p in pages {
                tops.extend(doc.top_shapes(Owner::Page(p)));
            }
        }
    }
    let mut out = Vec::new();
    for s in tops {
        expand(doc, s, &mut out, 0);
    }
    out
}

// ---------------------------------------------------------------- criteria

fn number(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let t = if t.contains(',') && !t.contains('.') { t.replace(',', ".") } else { String::from(t) };
    t.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn cmp_text(a: &str, b: &str, case: bool) -> Ordering {
    if case {
        a.cmp(b)
    } else {
        a.to_lowercase().cmp(&b.to_lowercase())
    }
}

fn holds(c: Condition, o: Ordering) -> bool {
    match c {
        Condition::Eq => o == Ordering::Equal,
        Condition::Ne => o != Ordering::Equal,
        Condition::Gt => o == Ordering::Greater,
        Condition::Ge => o != Ordering::Less,
        Condition::Lt => o == Ordering::Less,
        Condition::Le => o != Ordering::Greater,
        _ => false,
    }
}

fn cmp_num(a: f64, b: f64) -> Ordering {
    let tol = 1e-9 * a.abs().max(b.abs()).max(1.0);
    if (a - b).abs() <= tol {
        Ordering::Equal
    } else if a < b {
        Ordering::Less
    } else {
        Ordering::Greater
    }
}

fn matches(f: &Filter, r: &Raw, case: bool, env: &Env) -> bool {
    match f.condition {
        Condition::Exists => return r.present(),
        Condition::NotExists => return !r.present(),
        Condition::Other(_) => return false,
        _ => {}
    }
    let want = f.value.as_str();
    let o = match r {
        Raw::Missing => return false,
        Raw::Num(x, unit) => match number(want) {
            Some(v) => {
                let u = Unit::parse(f.unit.trim()).filter(|u| u.factor().is_some()).or(*unit);
                cmp_num(*x, u.map_or(v, |u| u.to_internal(v)))
            }
            None => cmp_text(&field::format(&Shown::Num(*x, *unit), "", env), want, case),
        },
        Raw::Date(x) => match number(want) {
            Some(v) => cmp_num(*x, v),
            None => cmp_text(&field::format(&Shown::Num(*x, Some(Unit::Date)), "", env), want, case),
        },
        Raw::Bool(b) => match truth(want) {
            Some(w) => (*b as u8).cmp(&(w as u8)),
            None => return f.condition == Condition::Ne,
        },
        Raw::Text(s) => match (number(s), number(want)) {
            (Some(a), Some(b)) => cmp_num(a, b),
            _ => cmp_text(s, want, case),
        },
        Raw::Empty => cmp_text("", want, case),
    };
    holds(f.condition, o)
}

fn passes(doc: &Document, s: ShapeRef, def: &ReportDef) -> bool {
    let mut ok = true;
    for (k, f) in def.filters.iter().enumerate() {
        let m = matches(f, &raw(doc, s, &f.property), def.case_sensitive, &doc.fields);
        ok = if k == 0 { m } else if f.and { ok && m } else { ok || m };
    }
    ok
}

// ---------------------------------------------------------------- formatting

fn pow10(n: u32) -> f64 {
    let mut p = 1.0;
    for _ in 0..n.min(15) {
        p *= 10.0;
    }
    p
}

fn rounded(x: f64, decimals: u32) -> f64 {
    let p = pow10(decimals);
    math::round(x * p) / p
}

// the unit a number shows in (lengths, angles, durations, areas, per cent: their unit)
fn shown_unit(unit: Option<Unit>) -> Option<Unit> {
    unit.filter(|u| matches!(u.dimension(), Dimension::Length | Dimension::Angle | Dimension::Duration | Dimension::Area) || *u == Unit::Percent)
}

fn picture(decimals: u32, units: bool) -> String {
    let mut p = String::from("0");
    if decimals > 0 {
        p.push('.');
        for _ in 0..decimals.min(15) {
            p.push('0');
        }
    }
    if units {
        p.push_str(" u");
    }
    p
}

fn num_cell(x: f64, unit: Option<Unit>, decimals: u32, units: bool, env: &Env) -> Cell {
    let u = shown_unit(unit);
    // rounded half away from zero first (the picture's formatting rounds half to even)
    let shown = rounded(u.map_or(x, |u| u.from_internal(x)), decimals);
    let with_unit = units && u.is_some_and(|u| !field::unit_abbr(u).is_empty());
    let text = field::format(&Shown::Num(u.map_or(shown, |u| u.to_internal(shown)), u), &picture(decimals, with_unit), env);
    Cell { text, value: Val::Num(shown) }
}

fn cell_of(r: &Raw, def: &ReportDef, env: &Env, labels: &Labels) -> Cell {
    match r {
        Raw::Missing | Raw::Empty => Cell::empty(),
        Raw::Num(x, unit) => num_cell(*x, *unit, def.precision, def.show_units, env),
        Raw::Text(s) => Cell::text(s),
        Raw::Bool(b) => Cell { text: String::from(if *b { labels.yes.as_str() } else { labels.no.as_str() }), value: Val::Bool(*b) },
        Raw::Date(x) => Cell { text: field::format(&Shown::Num(*x, Some(Unit::Date)), "", env), value: Val::Date(*x) },
    }
}

// ---------------------------------------------------------------- sorting and summaries

fn rank(r: &Raw) -> u8 {
    match r {
        Raw::Missing => 0,
        Raw::Empty => 1,
        Raw::Bool(_) => 2,
        Raw::Num(..) | Raw::Date(_) => 3,
        Raw::Text(_) => 4,
    }
}

fn cmp_raw(a: &Raw, b: &Raw, case: bool) -> Ordering {
    match (a, b) {
        (Raw::Num(x, _) | Raw::Date(x), Raw::Num(y, _) | Raw::Date(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Raw::Text(x), Raw::Text(y)) => match (number(x), number(y)) {
            (Some(p), Some(q)) => p.partial_cmp(&q).unwrap_or(Ordering::Equal),
            _ => cmp_text(x, y, case).then_with(|| x.cmp(y)),
        },
        (Raw::Bool(x), Raw::Bool(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

// a summary of one field over rows -> (value in internal units, unit); None: no number
fn summary(rows: &[&Vec<Raw>], k: usize, f: Func) -> Option<(f64, Option<Unit>)> {
    if f == Func::Count {
        return Some((rows.len() as f64, None));
    }
    let mut unit = None;
    let nums: Vec<f64> = rows
        .iter()
        .filter_map(|r| match &r[k] {
            Raw::Num(x, u) => {
                if unit.is_none() {
                    unit = *u;
                }
                Some(*x)
            }
            _ => None,
        })
        .collect();
    if nums.is_empty() {
        return None;
    }
    let v = match f {
        Func::Total | Func::Average => {
            let sum: f64 = nums.iter().sum();
            if f == Func::Total { sum } else { sum / nums.len() as f64 }
        }
        Func::Min => nums.iter().copied().fold(f64::INFINITY, f64::min),
        _ => nums.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    };
    Some((v, unit))
}

fn summary_cell(rows: &[&Vec<Raw>], k: usize, f: Func, def: &ReportDef, env: &Env) -> Cell {
    match summary(rows, k, f) {
        Some((v, _)) if f == Func::Count => num_cell(v, None, 0, false, env),
        Some((v, unit)) => num_cell(v, unit, def.precision, def.show_units, env),
        None => Cell::empty(),
    }
}

// the label into the first empty cell of a row
fn labeled(mut cells: Vec<Cell>, label: &str) -> Vec<Cell> {
    if let Some(c) = cells.iter_mut().find(|c| c.text.is_empty()) {
        *c = Cell::text(label);
    }
    cells
}

// ---------------------------------------------------------------- the table

/// run a definition against a drawing
pub fn run(doc: &Document, def: &ReportDef, ctx: &Ctx) -> Table {
    let env = &doc.fields;
    let labels = &ctx.labels;
    let case = def.case_sensitive;
    let taken: Vec<ShapeRef> = shapes(doc, def, ctx).into_iter().filter(|&s| passes(doc, s, def)).collect();
    let mut rows: Vec<Vec<Raw>> = taken.iter().map(|&s| def.fields.iter().map(|f| raw(doc, s, &f.property)).collect()).collect();
    // the sort keys (by field index), the first one first; a stable sort keeps the drawing's order
    let keys: Vec<(usize, bool, bool)> = def.sorts.iter().filter_map(|s| def.fields.iter().position(|f| f.id == s.field).map(|k| (k, s.ascending, s.case_sensitive))).take(3).collect();
    rows.sort_by(|a, b| {
        for &(k, asc, c) in &keys {
            let o = cmp_raw(&a[k], &b[k], c);
            if o != Ordering::Equal {
                return if asc { o } else { o.reverse() };
            }
        }
        Ordering::Equal
    });
    let cols: Vec<usize> = {
        let mut v: Vec<usize> = (0..def.fields.len()).filter(|&k| def.fields[k].display).collect();
        v.sort_by_key(|&k| def.fields[k].order);
        v
    };
    let grouping = def.grouping.clone().unwrap_or_default();
    let gk = (grouping.field != 0).then(|| def.fields.iter().position(|f| f.id == grouping.field)).flatten();
    let funcs_of = |k: usize| Func::of_mask(def.fields[k].summary);
    let used: Vec<Func> = FUNCS.iter().copied().filter(|f| cols.iter().any(|&k| def.fields[k].summary & f.bit() != 0)).collect();
    let mut table = Table { title: String::from(def.heading()), columns: Vec::new(), rows: Vec::new(), shapes: taken.len() };
    // duplicates: by the shown texts
    let mut seen: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut item = |r: &Vec<Raw>| -> Option<Row> {
        let cells: Vec<Cell> = cols.iter().map(|&k| cell_of(&r[k], def, env, labels)).collect();
        if grouping.exclude_duplicates && !seen.insert(cells.iter().map(|c| c.text.clone()).collect()) {
            return None;
        }
        Some(Row { kind: RowKind::Item, cells })
    };
    let summary_row = |set: &[&Vec<Raw>], f: Func, kind: RowKind, label: &str| -> Row {
        let cells: Vec<Cell> = cols.iter().map(|&k| if def.fields[k].summary & f.bit() != 0 { summary_cell(set, k, f, def, env) } else { Cell::empty() }).collect();
        Row { kind, cells: labeled(cells, label) }
    };
    let Some(gk) = gk else {
        table.columns = cols.iter().map(|&k| Column { name: String::from(def.fields[k].heading()), numeric: false }).collect();
        let kept: Vec<&Vec<Raw>> = rows.iter().collect();
        for r in &kept {
            if let Some(row) = item(r) {
                table.rows.push(row);
            }
        }
        for &f in &used {
            table.rows.push(summary_row(&kept, f, RowKind::Grand(Some(f)), labels.func(f)));
        }
        numeric_columns(&mut table);
        return table;
    };
    // the groups: by the group field's shown text (case as the definition says), in the order of
    // its sort key when it has one, else ascending
    let mut groups: Vec<(String, String, Vec<usize>)> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let text = cell_of(&r[gk], def, env, labels).text;
        let key = if case { text.clone() } else { text.to_lowercase() };
        match groups.iter_mut().find(|g| g.0 == key) {
            Some(g) => g.2.push(i),
            None => groups.push((key, text, alloc::vec![i])),
        }
    }
    let asc = keys.iter().find(|k| k.0 == gk).is_none_or(|k| k.1);
    groups.sort_by(|a, b| {
        let o = cmp_raw(&rows[a.2[0]][gk], &rows[b.2[0]][gk], case);
        if asc { o } else { o.reverse() }
    });
    let all: Vec<&Vec<Raw>> = rows.iter().collect();
    if grouping.show_items {
        table.columns = cols.iter().map(|&k| Column { name: String::from(def.fields[k].heading()), numeric: false }).collect();
        let at = cols.iter().position(|&k| k == gk).unwrap_or(0);
        for (_, text, members) in &groups {
            let mut cells: Vec<Cell> = cols.iter().map(|_| Cell::empty()).collect();
            if let Some(c) = cells.get_mut(at) {
                *c = Cell::text(text);
            }
            table.rows.push(Row { kind: RowKind::Group, cells });
            let set: Vec<&Vec<Raw>> = members.iter().map(|&i| &rows[i]).collect();
            for r in &set {
                if let Some(row) = item(r) {
                    table.rows.push(row);
                }
            }
            for &f in &used {
                table.rows.push(summary_row(&set, f, RowKind::Summary(Some(f)), labels.func(f)));
            }
        }
        if grouping.grand_summary {
            for &f in &used {
                let label = format!("{} – {}", labels.grand, labels.func(f));
                table.rows.push(summary_row(&all, f, RowKind::Grand(Some(f)), &label));
            }
        }
        numeric_columns(&mut table);
        return table;
    }
    // summaries only: the group's value, then a column per (field, function)
    let mut sums: Vec<(usize, Func)> = Vec::new();
    for &k in &cols {
        for f in funcs_of(k) {
            sums.push((k, f));
        }
    }
    if !cols.contains(&gk) {
        for f in funcs_of(gk) {
            sums.insert(0, (gk, f));
        }
    }
    table.columns.push(Column { name: String::from(def.fields[gk].heading()), numeric: false });
    for &(k, f) in &sums {
        let name = if k == gk { String::from(labels.func(f)) } else { format!("{} ({})", def.fields[k].heading(), labels.func(f)) };
        table.columns.push(Column { name, numeric: true });
    }
    for (_, text, members) in &groups {
        let set: Vec<&Vec<Raw>> = members.iter().map(|&i| &rows[i]).collect();
        let mut cells = alloc::vec![Cell::text(text)];
        cells.extend(sums.iter().map(|&(k, f)| summary_cell(&set, k, f, def, env)));
        table.rows.push(Row { kind: RowKind::Item, cells });
    }
    if grouping.grand_summary {
        let mut cells = alloc::vec![Cell::text(&labels.grand)];
        cells.extend(sums.iter().map(|&(k, f)| summary_cell(&all, k, f, def, env)));
        table.rows.push(Row { kind: RowKind::Grand(None), cells });
    }
    table
}

// a column is numeric when every value of its shapes' rows that is there is a number
fn numeric_columns(t: &mut Table) {
    for (k, c) in t.columns.iter_mut().enumerate() {
        let mut any = false;
        let mut all = true;
        for r in t.rows.iter().filter(|r| r.kind == RowKind::Item) {
            match r.cells.get(k).map(|c| &c.value) {
                Some(Val::Num(_)) | Some(Val::Date(_)) => any = true,
                Some(Val::Empty) | None => {}
                _ => all = false,
            }
        }
        c.numeric = any && all;
    }
}
