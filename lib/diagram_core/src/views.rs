// views -- rows of a sheet as the user knows them: shape data, user cells, hyperlinks, layers
//
//   doc.section_rows(sheet, sec) -> Vec<RowView>   every row a section really has
//   doc.shape_data(sheet)        the Property rows (DA-01: Label, Type, Format, Value ...)
//   doc.user_cells(sheet)        the User rows (Value, Prompt)
//   doc.hyperlinks(sheet)        the Hyperlink rows (Address, SubAddress, Description ...)
//   doc.layers(page)             the Layer rows of a page sheet (Name, Visible, Print, Lock ...)
//   doc.layer_members(shape)     the layer indices of a shape (LayerMember "0;2")
//   row.get("Label") -> Resolved   row.text("Label") / num(..) / flag(..)
//
// The views read through inheritance (inherit.rs): an instance shows the shape data of its
// master with its own values over them.  Shape data, user cells and hyperlinks exist on shapes,
// pages and the document alike.

use alloc::vec::Vec;

use crate::cell::Value;
use crate::document::{Document, PageRef, ShapeRef, SheetRef};
use crate::inherit::{EffRow, Resolved};
use crate::names;
use crate::sheet::{CellAddr, RowKey, SecKey};
use crate::strings::Sym;

/// one row of a section with access to its cells
#[derive(Copy, Clone)]
pub struct RowView<'d> {
    doc: &'d Document,
    /// the sheet asked
    pub sheet: SheetRef,
    /// the section
    pub section: SecKey,
    /// the row
    pub row: EffRow,
}

impl<'d> RowView<'d> {
    /// IX or N
    pub fn key(&self) -> RowKey {
        self.row.key
    }

    /// the universal name of a named row (Prop.<name>, User.<name>)
    pub fn name(&self) -> Option<&'d str> {
        match self.row.key {
            RowKey::Named(n) => Some(self.doc.str(n)),
            RowKey::Index(_) => None,
        }
    }

    /// the address of one of its cells
    pub fn addr(&self, cell: Sym) -> CellAddr {
        CellAddr::row(self.section, self.row.key, cell)
    }

    /// a cell of the row
    pub fn cell(&self, cell: Sym) -> Resolved<'d> {
        self.doc.resolve(self.sheet, &self.addr(cell))
    }

    /// a cell of the row by name
    pub fn get(&self, cell: &str) -> Resolved<'d> {
        match self.doc.sym(cell) {
            Some(c) => self.cell(c),
            None => Resolved { origin: crate::cell::Origin::Default, source: None, cell: None, cached: None },
        }
    }

    /// a cell's value as text (None: a number or no value)
    pub fn text(&self, cell: &str) -> Option<&'d str> {
        match self.get(cell).value() {
            Value::Text(s) => Some(self.doc.str(s)),
            _ => None,
        }
    }

    /// a cell's value as a number
    pub fn num(&self, cell: &str) -> Option<f64> {
        self.get(cell).num()
    }

    /// a cell's value as a truth value (a number other than 0, or "TRUE")
    pub fn flag(&self, cell: &str) -> bool {
        match self.get(cell).value() {
            Value::Num(x) => x != 0.0,
            Value::Text(s) => self.doc.str(s).eq_ignore_ascii_case("TRUE"),
            Value::Absent => false,
        }
    }
}

impl Document {
    /// every row section `sec` really has for `sheet`
    pub fn section_rows(&self, sheet: SheetRef, sec: SecKey) -> Vec<RowView<'_>> {
        self.rows(sheet, sec).into_iter().map(|row| RowView { doc: self, sheet, section: sec, row }).collect()
    }

    /// the shape data fields (Property rows)
    pub fn shape_data(&self, sheet: SheetRef) -> Vec<RowView<'_>> {
        self.section_rows(sheet, SecKey::one(names::PROPERTY))
    }

    /// the user-defined cells (User rows)
    pub fn user_cells(&self, sheet: SheetRef) -> Vec<RowView<'_>> {
        self.section_rows(sheet, SecKey::one(names::USER))
    }

    /// the hyperlinks (Hyperlink rows)
    pub fn hyperlinks(&self, sheet: SheetRef) -> Vec<RowView<'_>> {
        self.section_rows(sheet, SecKey::one(names::HYPERLINK))
    }

    /// the layers of a page (Layer rows of its page sheet; the index is the layer's number)
    pub fn layers(&self, page: PageRef) -> Vec<RowView<'_>> {
        self.section_rows(SheetRef::Page(page), SecKey::one(names::LAYER))
    }

    /// the layer of a page with this universal name (NameUniv, else Name) -> its index
    pub fn layer_by_name(&self, page: PageRef, name: &str) -> Option<u32> {
        self.layers(page).into_iter().find_map(|l| {
            let n = l.text("NameUniv").or_else(|| l.text("Name"))?;
            match (n == name, l.key()) {
                (true, RowKey::Index(ix)) => Some(ix),
                _ => None,
            }
        })
    }

    /// the layers a shape is on (its LayerMember cell, with inheritance)
    pub fn layer_members(&self, shape: ShapeRef) -> Vec<u32> {
        match self.resolve(SheetRef::Shape(shape), &CellAddr::single(names::LAYER_MEMBER)).value() {
            Value::Text(s) => parse_members(self.str(s)),
            Value::Num(x) if x >= 0.0 => alloc::vec![x as u32],
            _ => Vec::new(),
        }
    }
}

/// a LayerMember text ("0;2") -> the layer indices
pub fn parse_members(text: &str) -> Vec<u32> {
    text.split(';').filter_map(|p| p.trim().parse().ok()).collect()
}
