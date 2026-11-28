// instance::detach -- (JD21, SP-06) an instance of a master made plain shapes: what it inherits
// from its master shapes becomes its own, so that it can be taken apart (Visio's "Durch diese
// Aktion wird die Verknüpfung des Objekts mit dem Master getrennt")
//
//   detach(tx, shape)        the instance and its subshapes: every cell, section and row of their
//                            master shapes they lack, the text and the picture they show, the
//                            styles, then no Master / MasterShape any more
//
// Formulas taken from the master name the master's shapes by Sheet.N: they get the IDs of the
// instance's shapes that inherit from them.  Cells the instance has with F="Inh" (the master's
// formula, a value of its own) get the master's formula and keep their value.  Rows and sections
// the instance deletes (Del="1") stay away, and the markers go.  A master shape that is itself an
// instance of another master passes on only what it holds itself (Visio's masters seldom nest).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::vec::Vec;

use crate::cell::{Cell, Formula};
use crate::document::{Document, ShapeRef, SheetRef};
use crate::edit::{EditError, Op, Scope, Transaction};
use crate::ops::text;
use crate::sheet::{Row, Section, Sheet};

// a master formula with its Sheet.N renamed to the instance's IDs
fn renamed(doc: &Document, f: &str, ids: &BTreeMap<u32, u32>) -> Option<alloc::string::String> {
    let mut ed: Vec<((usize, usize), alloc::string::String)> = Vec::new();
    for item in text::items(f) {
        let text::Item::Sheet { text: t, range, foreign: false, quoted: false } = item else { continue };
        if let Some(n) = t.strip_prefix("Sheet.").and_then(|n| n.parse::<u32>().ok()) {
            if let Some(m) = ids.get(&n) {
                ed.push((range, format!("Sheet.{m}")));
            }
        }
    }
    let _ = doc;
    (!ed.is_empty()).then(|| text::replace(f, ed))
}

// a master cell for the instance: its formula renamed
fn take(tx: &mut Transaction, c: &Cell, ids: &BTreeMap<u32, u32>) -> Cell {
    let mut out = *c;
    if let Formula::Expr(f) = c.formula {
        let s = alloc::string::String::from(tx.doc().str(f));
        if let Some(n) = renamed(tx.doc(), &s, ids) {
            out.formula = Formula::Expr(tx.intern(&n));
        }
    }
    out
}

// the instance's own cells merged with the master's (own ones win; F="Inh" takes the master's
// formula with its own value)
fn merge_cells(tx: &mut Transaction, own: &[Cell], theirs: &[Cell], ids: &BTreeMap<u32, u32>) -> Vec<Cell> {
    let mut out: Vec<Cell> = Vec::with_capacity(own.len().max(theirs.len()));
    for c in own {
        let mut c = *c;
        if c.formula == Formula::Inherited {
            match theirs.iter().find(|m| m.name == c.name) {
                Some(m) => c.formula = take(tx, m, ids).formula,
                None => c.formula = Formula::Absent,
            }
        }
        out.push(c);
    }
    for m in theirs {
        if !own.iter().any(|c| c.name == m.name) {
            let c = take(tx, m, ids);
            out.push(c);
        }
    }
    out
}

fn merge_sheet(tx: &mut Transaction, own: &Sheet, theirs: &Sheet, ids: &BTreeMap<u32, u32>) -> (Vec<Cell>, Vec<Section>) {
    let cells = merge_cells(tx, &own.cells, &theirs.cells, ids);
    let mut secs: Vec<Section> = Vec::new();
    for ms in &theirs.sections {
        if ms.del {
            continue;
        }
        let key = ms.key();
        let local = own.sections.iter().find(|s| s.key() == key);
        if local.is_some_and(|s| s.del) {
            continue;
        }
        let mut sec = Section::new(key);
        sec.ix_written = ms.ix_written;
        sec.triggers = ms.triggers.clone();
        sec.extras = ms.extras.clone();
        sec.cells = merge_cells(tx, local.map_or(&[][..], |l| &l.cells), &ms.cells, ids);
        // the master's rows in its order, the instance's own rows after them
        for mr in &ms.rows {
            if mr.del {
                continue;
            }
            let lr = local.and_then(|l| l.rows.iter().find(|r| r.key == mr.key));
            if lr.is_some_and(|r| r.del) {
                continue;
            }
            let mut row = Row::new(mr.key);
            row.ix_written = mr.ix_written;
            row.local_name = lr.and_then(|r| r.local_name).or(mr.local_name);
            row.kind = lr.and_then(|r| r.kind).or(mr.kind);
            row.triggers = mr.triggers.clone();
            row.extras = mr.extras.clone();
            row.cells = merge_cells(tx, lr.map_or(&[][..], |r| &r.cells), &mr.cells, ids);
            sec.rows.push(row);
        }
        if let Some(l) = local {
            for r in &l.rows {
                if !r.del && !ms.rows.iter().any(|m| m.key == r.key) {
                    sec.rows.push(r.clone());
                }
            }
        }
        secs.push(sec);
    }
    for s in &own.sections {
        if !s.del && !theirs.sections.iter().any(|m| m.key() == s.key()) {
            let mut s = s.clone();
            s.rows.retain(|r| !r.del);
            secs.push(s);
        }
    }
    (cells, secs)
}

/// make the instance `shape` (a shape with a Master) and its subshapes plain shapes: what they
/// inherit from their master shapes becomes their own (see the head of this file); a shape that
/// is no instance is left alone
pub fn detach(tx: &mut Transaction, shape: ShapeRef) -> Result<(), EditError> {
    tx.atomic(|tx| {
        // the instance's shapes and the master shape IDs they inherit from
        let mut all = alloc::vec![shape];
        let mut k = 0;
        while k < all.len() {
            let kids: Vec<ShapeRef> = tx.doc().shape(all[k]).map(|s| s.children().collect()).unwrap_or_default();
            all.extend(kids);
            k += 1;
        }
        let mut ids: BTreeMap<u32, u32> = BTreeMap::new();
        let mut pairs: Vec<(ShapeRef, ShapeRef)> = Vec::new();
        {
            let doc = tx.doc();
            if doc.shape(shape).is_none_or(|s| s.header().master.is_none()) {
                return Ok(());
            }
            for &s in &all {
                let Some(ms) = doc.master_shape_of(s) else { continue };
                let (Some(mid), Some(sid)) = (doc.shape(ms).map(|x| x.id()), doc.shape(s).map(|x| x.id())) else { continue };
                ids.insert(mid, sid);
                pairs.push((s, ms));
            }
        }
        for (s, ms) in pairs {
            let (own, theirs, text, foreign, mhead) = {
                let doc = tx.doc();
                let (Some(own), Some(theirs)) = (doc.sheet(SheetRef::Shape(s)).cloned(), doc.sheet(SheetRef::Shape(ms)).cloned()) else { continue };
                let msh = doc.shape(ms).ok_or(EditError::NoShape)?;
                let sh = doc.shape(s).ok_or(EditError::NoShape)?;
                let text = if sh.data().text.is_none() { msh.data().text.clone() } else { None };
                let foreign = if sh.data().foreign.is_none() { msh.data().foreign.clone() } else { None };
                (own, theirs, text, foreign, msh.header().clone())
            };
            let (cells, secs) = merge_sheet(tx, &own, &theirs, &ids);
            let sheet = SheetRef::Shape(s);
            // the sections anew, the single cells added
            for pos in (0..own.sections.len()).rev() {
                tx.run(Op::SecRemove { sheet, pos: pos as u32 })?;
            }
            for (pos, sec) in secs.into_iter().enumerate() {
                tx.run(Op::SecInsert { sheet, pos: pos as u32, sec: Box::new(sec) })?;
            }
            for (pos, c) in cells.into_iter().enumerate() {
                match own.cells.iter().position(|o| o.name == c.name) {
                    Some(p) => {
                        if own.cells[p] != c {
                            tx.run(Op::CellSet { sheet, scope: Scope::Top, pos: p as u32, cell: c })?;
                        }
                    }
                    None => {
                        let n = tx.doc().sheet(sheet).map_or(pos, |x| x.cells.len());
                        tx.run(Op::CellInsert { sheet, scope: Scope::Top, pos: n as u32, cell: c })?;
                    }
                }
            }
            if let Some(t) = text {
                tx.set_text(s, Some(*t))?;
            }
            if let Some(f) = foreign {
                tx.set_foreign(s, Some(*f))?;
            }
            // the header: no master, the master shape's kind and styles where the shape has none
            let mut h = tx.doc().shape(s).ok_or(EditError::NoShape)?.header().clone();
            h.master = None;
            h.master_shape = None;
            if matches!(h.kind, crate::shape::ShapeKind::Unspecified) {
                h.kind = mhead.kind;
            }
            h.styles.line = h.styles.line.or(mhead.styles.line);
            h.styles.fill = h.styles.fill.or(mhead.styles.fill);
            h.styles.text = h.styles.text.or(mhead.styles.text);
            tx.set_shape_header(s, h)?;
        }
        // subshapes the instance deleted (Del="1") go; the root has no master any more either
        let dead: Vec<ShapeRef> = all.iter().copied().filter(|&s| tx.doc().shape(s).is_some_and(|x| x.header().del)).collect();
        if !dead.is_empty() {
            tx.delete_shapes(&dead)?;
        }
        if let Some(mut h) = tx.doc().shape(shape).map(|x| x.header().clone()) {
            if h.master.is_some() || h.master_shape.is_some() {
                h.master = None;
                h.master_shape = None;
                tx.set_shape_header(shape, h)?;
            }
        }
        Ok(())
    })
}
