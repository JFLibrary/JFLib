// store -- the report definitions of a document: the document sheet's User rows visPropRpt1,
// visPropRpt2 ... (Value: the definition's XML, U="STR"), as Visio keeps them
//
//   stored(doc) -> Vec<Stored>          every such row in the order of its number, read
//   find(doc, name) -> Option<Stored>   the one with this name (case ignored)
//   add(tx, &def) -> row name           a new row: the number after the largest one
//   replace(tx, row, &def)              the row's definition anew
//   rename(tx, row, name)               only the name (and the title, when it was the name)
//   remove(tx, row)                     the row goes; the definitions after it move up a row
//                                       (visPropRpt3 into visPropRpt2 ...), so the numbers
//                                       stay without gaps (Annahme: Visio reads them in order)
//
// A row whose text is no definition stays as it is (Stored::def is the error); replace and
// remove take it like any other.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::{parse, to_xml, ReportDef, ReportError};
use crate::cell::Value;
use crate::document::{Document, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::names;
use crate::sheet::{RowKey, SecKey};
use crate::units::Unit;

/// the row names' prefix in the document sheet's User section
pub const ROW_PREFIX: &str = "visPropRpt";

/// a definition kept in the document
#[derive(Clone, PartialEq, Debug)]
pub struct Stored {
    /// the row's name (visPropRpt3)
    pub row: String,
    /// its number (3)
    pub index: u32,
    /// the text of its Value cell
    pub xml: String,
    /// the definition read from it
    pub def: Result<ReportDef, ReportError>,
}

fn index_of(row: &str) -> Option<u32> {
    let n = row.strip_prefix(ROW_PREFIX)?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    n.parse().ok()
}

/// the definitions of the document sheet, by their numbers
pub fn stored(doc: &Document) -> Vec<Stored> {
    let mut v: Vec<Stored> = doc
        .user_cells(SheetRef::Document)
        .into_iter()
        .filter_map(|r| {
            let row = r.name()?;
            let index = index_of(row)?;
            let xml = match r.get("Value").value() {
                Value::Text(s) => String::from(doc.str(s)),
                _ => String::new(),
            };
            let def = parse(&xml);
            Some(Stored { row: String::from(row), index, xml, def })
        })
        .collect();
    v.sort_by_key(|s| s.index);
    v
}

/// the definition named `name` (case ignored)
pub fn find(doc: &Document, name: &str) -> Option<Stored> {
    stored(doc).into_iter().find(|s| s.def.as_ref().is_ok_and(|d| d.name.eq_ignore_ascii_case(name)))
}

fn write(tx: &mut Transaction, row: &str, xml: &str) -> Result<(), EditError> {
    let v = Value::Text(tx.intern(xml));
    tx.set_user(SheetRef::Document, row, v, Some(Unit::Str))
}

/// keep a new definition in the document -> its row's name
pub fn add(tx: &mut Transaction, def: &ReportDef) -> Result<String, EditError> {
    let next = stored(tx.doc()).iter().map(|s| s.index).max().unwrap_or(0) + 1;
    let row = format!("{ROW_PREFIX}{next}");
    write(tx, &row, &to_xml(def))?;
    Ok(row)
}

/// give a kept definition's row another definition
pub fn replace(tx: &mut Transaction, row: &str, def: &ReportDef) -> Result<(), EditError> {
    if !stored(tx.doc()).iter().any(|s| s.row == row) {
        return Err(EditError::NoRow);
    }
    write(tx, row, &to_xml(def))
}

/// rename a kept definition (its title too when it was its name); a row that is no
/// definition is refused (Invalid)
pub fn rename(tx: &mut Transaction, row: &str, name: &str) -> Result<(), EditError> {
    let s = stored(tx.doc()).into_iter().find(|s| s.row == row).ok_or(EditError::NoRow)?;
    let mut d = s.def.map_err(|_| EditError::Invalid)?;
    if d.title == d.name {
        d.title = String::from(name);
    }
    d.name = String::from(name);
    write(tx, row, &to_xml(&d))
}

/// remove a kept definition; the ones after it move up a row
pub fn remove(tx: &mut Transaction, row: &str) -> Result<(), EditError> {
    let all = stored(tx.doc());
    let at = all.iter().position(|s| s.row == row).ok_or(EditError::NoRow)?;
    for k in at..all.len() - 1 {
        let next = all[k + 1].xml.clone();
        write(tx, &all[k].row, &next)?;
    }
    let last = tx.intern(&all[all.len() - 1].row);
    tx.delete_row(SheetRef::Document, SecKey::one(names::USER), RowKey::Named(last))
}
