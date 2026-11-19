// inherit -- reading cells with inheritance: shape -> master shape -> styles -> default
//
//   doc.resolve(sheet, &addr) -> Resolved   the formula's origin and source cell, the value
//   doc.rows(sheet, sec) -> Vec<EffRow>     the rows a section really has: inherited ones,
//                                           overridden ones and own ones, minus deleted ones
//   doc.section_indices(sheet, name)        which Geometry sections (indices) a shape has
//   doc.parent_sheet(sheet, category)       the next sheet on the way up
//
// The way up from a sheet (FX-03, MS-10, [MS-VSDX] Style-to-Shape Inheritance):
// * a shape asks first its own style of the cell's category when it names one itself
//   (LineStyle for line cells, FillStyle for fill and effect cells, TextStyle for text);
// * otherwise the master shape (instances and their subshapes); the master shape then follows
//   the same rules, so its styles come after it;
// * a shape without master asks its style of the category, cells without category end there;
// * a style asks the style it is based on for that category; pages, masters' pages and the
//   document ask their LineStyle/FillStyle/TextStyle;
// * at the end the cell has its default (Origin::Default; the document's "No Style" style is
//   normally the last style of every chain and holds real values).
// A cell with F="Inh" on the way gives the value (its sheet's own result) but not the formula;
// the search goes on for the formula.  A row or section with Del="1" on the way ends the
// search: Origin::Deleted.  Rows are matched by IX or N, sections by name and index.  The way
// is at most 64 sheets long (a cycle of styles or masters in a broken file ends there).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use crate::cell::{Cell, Formula, Origin, Value};
use crate::document::{Document, SheetRef};
use crate::names::{self, Category};
use crate::sheet::{CellAddr, Local, RowKey, SecKey};
use crate::strings::Sym;
use crate::units::Unit;

const MAX_DEPTH: usize = 64;

/// a cell read with inheritance
#[derive(Copy, Clone, Debug)]
pub struct Resolved<'d> {
    /// where the formula comes from
    pub origin: Origin,
    /// the sheet whose cell holds the formula (None: default or deleted)
    pub source: Option<SheetRef>,
    /// that cell
    pub cell: Option<&'d Cell>,
    /// the nearest cell on the way, which holds the value for the asked sheet (an F="Inh"
    /// cell of the sheet itself, else the source cell)
    pub cached: Option<&'d Cell>,
}

impl<'d> Resolved<'d> {
    const NONE: Resolved<'static> = Resolved { origin: Origin::Default, source: None, cell: None, cached: None };

    /// the value for the asked sheet
    pub fn value(&self) -> Value {
        self.cached.or(self.cell).map_or(Value::Absent, |c| c.value)
    }

    /// the value as a number
    pub fn num(&self) -> Option<f64> {
        self.value().num()
    }

    /// the formula (Formula::Absent when the source cell is a constant or there is none)
    pub fn formula(&self) -> Formula {
        self.cell.map_or(Formula::Absent, |c| c.formula)
    }

    /// the unit the value is shown in
    pub fn unit(&self) -> Option<Unit> {
        self.cached.and_then(|c| c.unit).or(self.cell.and_then(|c| c.unit))
    }

    /// the error state of the value
    pub fn error(&self) -> Option<Sym> {
        self.cached.or(self.cell).and_then(|c| c.error)
    }

    /// whether the cell exists for the sheet (not default, not deleted)
    pub fn exists(&self) -> bool {
        !matches!(self.origin, Origin::Default | Origin::Deleted)
    }
}

/// a row a section really has (Document::rows)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct EffRow {
    /// IX or N
    pub key: RowKey,
    /// the row type (T), from the nearest sheet that names it
    pub kind: Option<Sym>,
    /// where the row comes from: Local when only the asked sheet has it
    pub origin: Origin,
    /// whether the asked sheet has an entry for it (own row or override)
    pub local: bool,
}

fn origin_of(r: SheetRef) -> Origin {
    match r {
        SheetRef::Shape(_) => Origin::Master,
        _ => Origin::Style,
    }
}

impl Document {
    /// the category of the cell at `addr` (which style it inherits from)
    pub fn category(&self, addr: &CellAddr) -> Option<Category> {
        match addr.section {
            Some(s) => Category::of_section(self.str(s.name)),
            None => Category::of(None, self.str(addr.cell)),
        }
    }

    /// the next sheet up from `sheet` for a cell of this category
    pub fn parent_sheet(&self, sheet: SheetRef, cat: Option<Category>) -> Option<SheetRef> {
        let styles = self.sheet_styles(sheet);
        if let Some(c) = cat {
            if let Some(id) = styles.get(c) {
                let st = self.style_by_id(id)?;
                return (SheetRef::Style(st) != sheet).then_some(SheetRef::Style(st));
            }
        }
        match sheet {
            SheetRef::Shape(s) => self.master_shape_of(s).map(SheetRef::Shape),
            _ => None,
        }
    }

    /// the cell at `addr` for `sheet`, with inheritance
    pub fn resolve(&self, sheet: SheetRef, addr: &CellAddr) -> Resolved<'_> {
        let cat = self.category(addr);
        let mut cached: Option<&Cell> = None;
        let mut cur = sheet;
        for _ in 0..MAX_DEPTH {
            let Some(sh) = self.sheet(cur) else { break };
            match sh.find(addr) {
                Local::Deleted => return Resolved { origin: Origin::Deleted, source: None, cell: None, cached },
                Local::Cell(c) => {
                    if cached.is_none() {
                        cached = Some(c);
                    }
                    if c.formula != Formula::Inherited {
                        let origin = if cur == sheet { Origin::Local } else { origin_of(cur) };
                        return Resolved { origin, source: Some(cur), cell: Some(c), cached };
                    }
                }
                Local::Missing => {}
            }
            match self.parent_sheet(cur, cat) {
                Some(p) => cur = p,
                None => break,
            }
        }
        Resolved { cached, ..Resolved::NONE }
    }

    /// the value of a single cell (PinX, Width ...) for `sheet`, with inheritance
    pub fn value_of(&self, sheet: SheetRef, cell: &str) -> Value {
        match self.sym(cell) {
            Some(c) => self.resolve(sheet, &CellAddr::single(c)).value(),
            None => Value::Absent,
        }
    }

    /// the rows section `sec` really has for `sheet`: inherited, overridden and own rows
    /// without the deleted ones; indexed rows by rising index, named rows in the order the
    /// master shape or style has them, then the own ones
    pub fn rows(&self, sheet: SheetRef, sec: SecKey) -> Vec<EffRow> {
        let mut rows = self.rows_at(sheet, sec, 0);
        if rows.iter().all(|r| matches!(r.key, RowKey::Index(_))) {
            rows.sort_by_key(|r| r.key);
        }
        rows
    }

    fn rows_at(&self, sheet: SheetRef, sec: SecKey, depth: usize) -> Vec<EffRow> {
        let cat = Category::of_section(self.str(sec.name));
        let mut rows = match self.parent_sheet(sheet, cat) {
            Some(p) if depth < MAX_DEPTH => {
                let mut v = self.rows_at(p, sec, depth + 1);
                for r in &mut v {
                    r.local = false;
                    if r.origin == Origin::Local {
                        r.origin = origin_of(p);
                    }
                }
                v
            }
            _ => Vec::new(),
        };
        let Some(section) = self.sheet(sheet).and_then(|s| s.section(sec)) else { return rows };
        if section.del {
            return Vec::new();
        }
        // merge the own rows: a map from key to position, removed rows become None
        let mut index: BTreeMap<RowKey, usize> = rows.iter().enumerate().map(|(i, r)| (r.key, i)).collect();
        let mut merged: Vec<Option<EffRow>> = rows.drain(..).map(Some).collect();
        for row in &section.rows {
            match index.get(&row.key) {
                Some(&i) => {
                    if row.del {
                        merged[i] = None;
                    } else if let Some(r) = merged[i].as_mut() {
                        r.local = true;
                        if row.kind.is_some() {
                            r.kind = row.kind;
                        }
                    }
                }
                None if !row.del => {
                    index.insert(row.key, merged.len());
                    merged.push(Some(EffRow { key: row.key, kind: row.kind, origin: Origin::Local, local: true }));
                }
                None => {}
            }
        }
        merged.into_iter().flatten().collect()
    }

    /// whether section `sec` has row `key` for `sheet` (inherited or own, not deleted)
    pub fn has_row(&self, sheet: SheetRef, sec: SecKey, key: RowKey) -> bool {
        self.rows(sheet, sec).iter().any(|r| r.key == key)
    }

    /// the indices of the sections named `name` that `sheet` really has (Geometry1 is 0)
    pub fn section_indices(&self, sheet: SheetRef, name: Sym) -> Vec<u32> {
        self.sections_at(sheet, name, 0).into_iter().collect()
    }

    fn sections_at(&self, sheet: SheetRef, name: Sym, depth: usize) -> BTreeSet<u32> {
        let cat = Category::of_section(self.str(name));
        let mut set = match self.parent_sheet(sheet, cat) {
            Some(p) if depth < MAX_DEPTH => self.sections_at(p, name, depth + 1),
            _ => BTreeSet::new(),
        };
        if let Some(sh) = self.sheet(sheet) {
            for s in sh.sections.iter().filter(|s| s.name == name) {
                if s.del {
                    set.remove(&s.index);
                } else {
                    set.insert(s.index);
                }
            }
        }
        set
    }

    /// whether `sheet` has section `sec` (inherited or own, not deleted)
    pub fn has_section(&self, sheet: SheetRef, sec: SecKey) -> bool {
        self.sections_at(sheet, sec.name, 0).contains(&sec.index)
    }

    /// whether the effective formula at `addr` is guarded (GUARD(...) at its top): edits of
    /// the user leave it alone (FX-04; the engine of JD6 knows the rest)
    pub fn is_guarded(&self, sheet: SheetRef, addr: &CellAddr) -> bool {
        match self.resolve(sheet, addr).formula() {
            Formula::Expr(f) => guarded(self.str(f)),
            _ => false,
        }
    }

    /// the Geometry sections of a sheet as keys
    pub fn geometry(&self, sheet: SheetRef) -> Vec<SecKey> {
        self.section_indices(sheet, names::GEOMETRY).into_iter().map(|index| SecKey { name: names::GEOMETRY, index }).collect()
    }
}

/// whether a formula text is GUARD(...) as a whole
pub fn guarded(formula: &str) -> bool {
    let f = formula.trim_start();
    f.len() >= 6 && f.as_bytes()[..6].eq_ignore_ascii_case(b"GUARD(")
}
