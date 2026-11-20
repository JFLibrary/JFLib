// edit -- changing a document in transactions that can be undone and redone (DM-02)
//
//   let mut tx = doc.transaction("Verschieben");      every edit goes through tx
//   tx.set_length(sheet, addr, 25.0, Unit::Millimeters)?;
//   tx.set_formula(sheet, addr, "Width*0.5")?;          ... add_shape, delete_row, add_page ...
//   let change = tx.commit();                           Option<Change>: None when nothing changed
//   (dropping tx without commit takes all its edits back)
//   change.undo(&mut doc) / change.redo(&mut doc)       -> bool
//   change.label() / merging(key) / merge(&next) / sheets()
//
// With ui::history (lib/ui/history.rs) the app wraps a Change in an Edit of its own (the orphan
// rule keeps the impl in the app; diagram_core does not depend on ui):
//
//   struct Step(Change);
//   impl Edit<Document> for Step {
//       fn apply(&mut self, d: &mut Document) { self.0.redo(d); }
//       fn revert(&mut self, d: &mut Document) { self.0.undo(d); }
//       fn label(&self) -> &str { self.0.label() }
//       fn merge(&mut self, next: &mut dyn Edit<Document>) -> bool {
//           next.value().and_then(|v| v.downcast_ref::<Step>()).is_some_and(|n| self.0.merge(&n.0))
//       }
//       fn value(&self) -> Option<&dyn core::any::Any> { Some(self) }
//   }
//   history.record(Box::new(Step(change)));            the change is already applied
//
// How undo works: inverse operations, not snapshots.  Each edit is broken into primitive steps
// on the smallest part it touches -- one cell, one row, one section, one shape's attributes,
// one shape subtree, one page, master or style -- and each step, when it runs, hands back the
// step that undoes it (a cell's old content, a removed subtree, an old position).  Undo runs the
// inverse steps backwards and collects their inverses for redo.  So a step costs memory and time
// in proportion to what it changes, whatever the size of the drawing: moving 1000 shapes keeps
// 2000 cells, not 1000 sheets or the document.  Handles stay valid because slots are never
// reused and deleted shapes come back into their slots.  The steps address cells and rows by
// position, which holds because undo and redo replay them in exact reverse order on the state
// they left; an untracked change in between (the *_mut functions) breaks that, a step that no
// longer fits is refused (undo returns false) and the steps run before it are taken back; when
// even that fails the change is_broken and refuses from then on.  IDs, like slots, are never
// handed out twice (NoIds once a file used the largest one, 4294967294; NO_ID is none); a step that would give two pages,
// masters or styles one ID is refused.
//
// Structure keeps inheritance: a subshape of an instance stays in its instance, the master
// shape of a master in use stays its first top-level shape (InUse), an instance never goes into
// a master it depends on (Cycle).  delete_shapes deletes a selection in one pass.
//
// Formula and value stay a pair: set_formula keeps the old value and marks the cell stale for
// the engine (JD6), set_constant writes a value whose formula is the value itself, set_result
// is the engine's way to store a formula's result; GUARD(...) formulas refuse both unless
// forced (force_formula, force_constant).  Deleting an inherited row, section or subshape writes
// a Del="1" mark; restore_row and restore_section remove it again.

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;
use core::mem;

use crate::cell::{Cell, CellError, Formula, Value};
use crate::document::{DocSettings, Document, MasterRef, Owner, Part, PageRef, ShapeRef, SheetRef, StyleRef, Theme};
use crate::names::{self, Category};
use crate::page::{Connect, Master, MasterHeader, Page, PageHeader, Style, StyleHeader};
use crate::shape::{ForeignData, Node, ShapeData, ShapeHeader, ShapeKind, StyleRefs, Text};
use crate::sheet::{CellAddr, Row, RowKey, SecKey, Section};
use crate::strings::Sym;
use crate::units::Unit;

/// why an edit was refused (the transaction keeps its earlier edits)
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum EditError {
    /// the sheet does not exist
    NoSheet,
    /// the shape does not exist
    NoShape,
    /// the page does not exist
    NoPage,
    /// the master does not exist
    NoMaster,
    /// the style does not exist
    NoStyle,
    /// the section does not exist for the sheet
    NoSection,
    /// the row does not exist for the sheet
    NoRow,
    /// the row or section is deleted here (Del="1"); restore it first
    Deleted,
    /// GUARD protects the formula; force_formula / force_constant override it
    Guarded,
    /// it exists already
    Exists,
    /// something still refers to it (instances of a master, users of a style)
    InUse,
    /// the change would make a cycle (background pages, styles, groups)
    Cycle,
    /// the page is not a background page
    NotBackground,
    /// the request does not fit the object
    Invalid,
    /// the document no longer matches (changed outside transactions)
    Stale,
    /// no free ID is left (a file used the largest ID, 4294967294)
    NoIds,
}

// ---- primitive steps

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum Scope {
    Top,
    Section(u32),
    Row(u32, u32),
}

#[derive(Clone, Debug)]
pub(crate) enum Op {
    CellSet { sheet: SheetRef, scope: Scope, pos: u32, cell: Cell },
    CellInsert { sheet: SheetRef, scope: Scope, pos: u32, cell: Cell },
    CellRemove { sheet: SheetRef, scope: Scope, pos: u32 },
    RowInsert { sheet: SheetRef, sec: u32, pos: u32, row: Box<Row> },
    RowRemove { sheet: SheetRef, sec: u32, pos: u32 },
    SecInsert { sheet: SheetRef, pos: u32, sec: Box<Section> },
    SecRemove { sheet: SheetRef, pos: u32 },
    ShapeInsert { owner: Owner, parent: Option<u32>, pos: u32, nodes: Vec<(u32, Box<Node>)> },
    ShapeRemove { owner: Owner, slot: u32 },
    ShapeHeader { shape: ShapeRef, header: Box<ShapeHeader> },
    ShapeMove { shape: ShapeRef, parent: Option<u32>, pos: u32 },
    ShapeText { shape: ShapeRef, text: Option<Box<Text>> },
    ShapeForeign { shape: ShapeRef, foreign: Option<Box<ForeignData>> },
    PageInsert { slot: u32, pos: u32, page: Box<Page> },
    PageRemove { slot: u32 },
    PageHeader { page: PageRef, header: Box<PageHeader> },
    PageMove { page: PageRef, pos: u32 },
    PageTheme { page: PageRef, theme: Option<u32> },
    MasterInsert { slot: u32, pos: u32, master: Box<Master> },
    MasterRemove { slot: u32 },
    MasterHeader { master: MasterRef, header: Box<MasterHeader> },
    StyleInsert { slot: u32, pos: u32, style: Box<Style> },
    StyleRemove { slot: u32 },
    StyleHeader { style: StyleRef, header: Box<StyleHeader> },
    ConnectInsert { owner: Owner, pos: u32, connect: Box<Connect> },
    ConnectRemove { owner: Owner, pos: u32 },
    Settings { settings: Box<DocSettings> },
    Themes { themes: Box<Vec<Theme>> },
    PartInsert { pos: u32, part: Box<Part> },
    PartRemove { pos: u32 },
}

fn cells_mut(doc: &mut Document, sheet: SheetRef, scope: Scope) -> Option<&mut Vec<Cell>> {
    let sh = doc.sheet_mut(sheet)?;
    match scope {
        Scope::Top => Some(&mut sh.cells),
        Scope::Section(s) => sh.sections.get_mut(s as usize).map(|x| &mut x.cells),
        Scope::Row(s, r) => sh.sections.get_mut(s as usize)?.rows.get_mut(r as usize).map(|x| &mut x.cells),
    }
}

fn rows_mut(doc: &mut Document, sheet: SheetRef, sec: u32) -> Option<&mut Vec<Row>> {
    doc.sheet_mut(sheet)?.sections.get_mut(sec as usize).map(|s| &mut s.rows)
}

fn connects_mut(doc: &mut Document, owner: Owner) -> Option<&mut Vec<Connect>> {
    match owner {
        Owner::Page(p) => doc.pages.get_mut(p.0).map(|x| &mut x.connects),
        Owner::Master(m) => doc.masters.get_mut(m.0).map(|x| &mut x.connects),
    }
}

fn remove_at<T>(v: &mut Vec<T>, pos: u32) -> Option<T> {
    ((pos as usize) < v.len()).then(|| v.remove(pos as usize))
}

impl Op {
    // run the step -> the step that undoes it (Err: it does not fit the document, nothing done)
    fn exec(self, doc: &mut Document) -> Result<Op, Op> {
        match self.exec_inner(doc) {
            Ok(inv) => Ok(inv),
            Err(op) => Err(*op),
        }
    }

    fn exec_inner(self, doc: &mut Document) -> Result<Op, Box<Op>> {
        macro_rules! fail {
            ($op:expr) => {
                return Err(Box::new($op))
            };
        }
        Ok(match self {
            Op::CellSet { sheet, scope, pos, cell } => {
                let Some(slot) = cells_mut(doc, sheet, scope).and_then(|v| v.get_mut(pos as usize)) else {
                    fail!(Op::CellSet { sheet, scope, pos, cell })
                };
                Op::CellSet { sheet, scope, pos, cell: mem::replace(slot, cell) }
            }
            Op::CellInsert { sheet, scope, pos, cell } => match cells_mut(doc, sheet, scope) {
                Some(v) if (pos as usize) <= v.len() => {
                    v.insert(pos as usize, cell);
                    Op::CellRemove { sheet, scope, pos }
                }
                _ => fail!(Op::CellInsert { sheet, scope, pos, cell }),
            },
            Op::CellRemove { sheet, scope, pos } => match cells_mut(doc, sheet, scope).and_then(|v| remove_at(v, pos)) {
                Some(cell) => Op::CellInsert { sheet, scope, pos, cell },
                None => fail!(Op::CellRemove { sheet, scope, pos }),
            },
            Op::RowInsert { sheet, sec, pos, row } => match rows_mut(doc, sheet, sec) {
                Some(v) if (pos as usize) <= v.len() => {
                    v.insert(pos as usize, *row);
                    Op::RowRemove { sheet, sec, pos }
                }
                _ => fail!(Op::RowInsert { sheet, sec, pos, row }),
            },
            Op::RowRemove { sheet, sec, pos } => match rows_mut(doc, sheet, sec).and_then(|v| remove_at(v, pos)) {
                Some(row) => Op::RowInsert { sheet, sec, pos, row: Box::new(row) },
                None => fail!(Op::RowRemove { sheet, sec, pos }),
            },
            Op::SecInsert { sheet, pos, sec } => match doc.sheet_mut(sheet) {
                Some(sh) if (pos as usize) <= sh.sections.len() => {
                    sh.sections.insert(pos as usize, *sec);
                    Op::SecRemove { sheet, pos }
                }
                _ => fail!(Op::SecInsert { sheet, pos, sec }),
            },
            Op::SecRemove { sheet, pos } => match doc.sheet_mut(sheet).and_then(|sh| remove_at(&mut sh.sections, pos)) {
                Some(sec) => Op::SecInsert { sheet, pos, sec: Box::new(sec) },
                None => fail!(Op::SecRemove { sheet, pos }),
            },
            Op::ShapeInsert { owner, parent, pos, nodes } => {
                let fits = doc.tree(owner).is_some_and(|t| {
                    !nodes.is_empty()
                        && parent.is_none_or(|p| t.node(p).is_some())
                        && (pos as usize) <= t.siblings(parent).len()
                        && nodes.iter().all(|(s, n)| t.node(*s).is_none() && t.by_id(n.id).is_none())
                });
                if !fits {
                    fail!(Op::ShapeInsert { owner, parent, pos, nodes });
                }
                let tree = doc.tree_mut(owner).expect("checked");
                let root = nodes[0].0;
                tree.put_subtree(nodes);
                tree.siblings_mut(parent).expect("checked").insert(pos as usize, root);
                Op::ShapeRemove { owner, slot: root }
            }
            Op::ShapeRemove { owner, slot } => {
                let found = doc.tree(owner).and_then(|t| {
                    let parent = t.node(slot)?.parent;
                    let pos = t.siblings(parent).iter().rposition(|&s| s == slot)?;
                    Some((parent, pos))
                });
                let Some((parent, pos)) = found else { fail!(Op::ShapeRemove { owner, slot }) };
                let tree = doc.tree_mut(owner).expect("checked");
                tree.siblings_mut(parent).expect("checked").remove(pos);
                let nodes = tree.take_subtree(slot);
                Op::ShapeInsert { owner, parent, pos: pos as u32, nodes }
            }
            Op::ShapeHeader { shape, header } => match doc.tree_mut(shape.owner).and_then(|t| t.data_mut(shape.slot)) {
                Some(d) => Op::ShapeHeader { shape, header: Box::new(mem::replace(&mut d.header, *header)) },
                None => fail!(Op::ShapeHeader { shape, header }),
            },
            Op::ShapeMove { shape, parent, pos } => {
                let found = doc.tree(shape.owner).and_then(|t| {
                    let old_parent = t.node(shape.slot)?.parent;
                    if let Some(p) = parent {
                        t.node(p)?;
                        if t.is_within(p, shape.slot) {
                            return None;
                        }
                    }
                    let old_pos = t.siblings(old_parent).iter().position(|&s| s == shape.slot)?;
                    Some((old_parent, old_pos))
                });
                let Some((old_parent, old_pos)) = found else { fail!(Op::ShapeMove { shape, parent, pos }) };
                let tree = doc.tree_mut(shape.owner).expect("checked");
                tree.siblings_mut(old_parent).expect("checked").remove(old_pos);
                let sib = tree.siblings_mut(parent).expect("checked");
                let at = (pos as usize).min(sib.len());
                sib.insert(at, shape.slot);
                tree.node_mut(shape.slot).expect("checked").parent = parent;
                Op::ShapeMove { shape, parent: old_parent, pos: old_pos as u32 }
            }
            Op::ShapeText { shape, text } => match doc.tree_mut(shape.owner).and_then(|t| t.data_mut(shape.slot)) {
                Some(d) => Op::ShapeText { shape, text: mem::replace(&mut d.text, text) },
                None => fail!(Op::ShapeText { shape, text }),
            },
            Op::ShapeForeign { shape, foreign } => match doc.tree_mut(shape.owner).and_then(|t| t.data_mut(shape.slot)) {
                Some(d) => Op::ShapeForeign { shape, foreign: mem::replace(&mut d.foreign, foreign) },
                None => fail!(Op::ShapeForeign { shape, foreign }),
            },
            Op::PageInsert { slot, pos, page } => {
                if doc.pages.get(slot).is_some() || doc.pages.by_id(page.header.id).is_some() {
                    fail!(Op::PageInsert { slot, pos, page });
                }
                doc.pages.put(slot, pos, page);
                Op::PageRemove { slot }
            }
            Op::PageRemove { slot } => match doc.pages.take(slot) {
                Some((pos, page)) => Op::PageInsert { slot, pos, page },
                None => fail!(Op::PageRemove { slot }),
            },
            Op::PageHeader { page, header } => match doc.pages.id_free_for(page.0, header.id).then(|| doc.pages.get_mut(page.0)).flatten() {
                Some(pg) => {
                    let old_id = pg.header.id;
                    let old = mem::replace(&mut pg.header, *header);
                    doc.pages.reindex(page.0, old_id);
                    Op::PageHeader { page, header: Box::new(old) }
                }
                None => fail!(Op::PageHeader { page, header }),
            },
            Op::PageMove { page, pos } => match doc.pages.position(page.0) {
                Some(old) => {
                    doc.pages.order.remove(old);
                    let at = (pos as usize).min(doc.pages.order.len());
                    doc.pages.order.insert(at, page.0);
                    Op::PageMove { page, pos: old as u32 }
                }
                None => fail!(Op::PageMove { page, pos }),
            },
            Op::PageTheme { page, theme } => match doc.pages.get_mut(page.0) {
                Some(pg) => Op::PageTheme { page, theme: mem::replace(&mut pg.theme, theme) },
                None => fail!(Op::PageTheme { page, theme }),
            },
            Op::MasterInsert { slot, pos, master } => {
                if doc.masters.get(slot).is_some() || doc.masters.by_id(master.header.id).is_some() {
                    fail!(Op::MasterInsert { slot, pos, master });
                }
                doc.masters.put(slot, pos, master);
                Op::MasterRemove { slot }
            }
            Op::MasterRemove { slot } => match doc.masters.take(slot) {
                Some((pos, master)) => Op::MasterInsert { slot, pos, master },
                None => fail!(Op::MasterRemove { slot }),
            },
            Op::MasterHeader { master, header } => match doc.masters.id_free_for(master.0, header.id).then(|| doc.masters.get_mut(master.0)).flatten() {
                Some(m) => {
                    let old_id = m.header.id;
                    let old = mem::replace(&mut m.header, *header);
                    doc.masters.reindex(master.0, old_id);
                    Op::MasterHeader { master, header: Box::new(old) }
                }
                None => fail!(Op::MasterHeader { master, header }),
            },
            Op::StyleInsert { slot, pos, style } => {
                if doc.styles.get(slot).is_some() || doc.styles.by_id(style.header.id).is_some() {
                    fail!(Op::StyleInsert { slot, pos, style });
                }
                doc.styles.put(slot, pos, style);
                Op::StyleRemove { slot }
            }
            Op::StyleRemove { slot } => match doc.styles.take(slot) {
                Some((pos, style)) => Op::StyleInsert { slot, pos, style },
                None => fail!(Op::StyleRemove { slot }),
            },
            Op::StyleHeader { style, header } => match doc.styles.id_free_for(style.0, header.id).then(|| doc.styles.get_mut(style.0)).flatten() {
                Some(s) => {
                    let old_id = s.header.id;
                    let old = mem::replace(&mut s.header, *header);
                    doc.styles.reindex(style.0, old_id);
                    Op::StyleHeader { style, header: Box::new(old) }
                }
                None => fail!(Op::StyleHeader { style, header }),
            },
            Op::ConnectInsert { owner, pos, connect } => match connects_mut(doc, owner) {
                Some(v) if (pos as usize) <= v.len() => {
                    v.insert(pos as usize, *connect);
                    Op::ConnectRemove { owner, pos }
                }
                _ => fail!(Op::ConnectInsert { owner, pos, connect }),
            },
            Op::ConnectRemove { owner, pos } => match connects_mut(doc, owner).and_then(|v| remove_at(v, pos)) {
                Some(c) => Op::ConnectInsert { owner, pos, connect: Box::new(c) },
                None => fail!(Op::ConnectRemove { owner, pos }),
            },
            Op::Settings { settings } => Op::Settings { settings: Box::new(mem::replace(&mut doc.settings, *settings)) },
            Op::Themes { themes } => Op::Themes { themes: Box::new(mem::replace(&mut doc.themes, *themes)) },
            Op::PartInsert { pos, part } => {
                if pos as usize > doc.parts.len() {
                    fail!(Op::PartInsert { pos, part })
                }
                doc.parts.insert(pos as usize, *part);
                Op::PartRemove { pos }
            }
            Op::PartRemove { pos } => match remove_at(&mut doc.parts, pos) {
                Some(part) => Op::PartInsert { pos, part: Box::new(part) },
                None => fail!(Op::PartRemove { pos }),
            },
        })
    }

    fn sheets(&self, out: &mut BTreeSet<SheetRef>) {
        let owner_sheet = |o: Owner| match o {
            Owner::Page(p) => SheetRef::Page(p),
            Owner::Master(m) => SheetRef::Master(m),
        };
        match self {
            Op::CellSet { sheet, .. }
            | Op::CellInsert { sheet, .. }
            | Op::CellRemove { sheet, .. }
            | Op::RowInsert { sheet, .. }
            | Op::RowRemove { sheet, .. }
            | Op::SecInsert { sheet, .. }
            | Op::SecRemove { sheet, .. } => {
                out.insert(*sheet);
            }
            Op::ShapeHeader { shape, .. } | Op::ShapeMove { shape, .. } | Op::ShapeText { shape, .. } | Op::ShapeForeign { shape, .. } => {
                out.insert(SheetRef::Shape(*shape));
            }
            Op::ShapeRemove { owner, slot } => {
                out.insert(SheetRef::Shape(ShapeRef { owner: *owner, slot: *slot }));
            }
            Op::ShapeInsert { owner, nodes, .. } => {
                out.extend(nodes.iter().map(|(slot, _)| SheetRef::Shape(ShapeRef { owner: *owner, slot: *slot })));
            }
            Op::ConnectInsert { owner, .. } | Op::ConnectRemove { owner, .. } => {
                out.insert(owner_sheet(*owner));
            }
            Op::PageInsert { slot, .. } | Op::PageRemove { slot } => {
                out.insert(SheetRef::Page(PageRef(*slot)));
            }
            Op::PageHeader { page, .. } | Op::PageMove { page, .. } | Op::PageTheme { page, .. } => {
                out.insert(SheetRef::Page(*page));
            }
            Op::MasterInsert { slot, .. } | Op::MasterRemove { slot } => {
                out.insert(SheetRef::Master(MasterRef(*slot)));
            }
            Op::MasterHeader { master, .. } => {
                out.insert(SheetRef::Master(*master));
            }
            Op::StyleInsert { slot, .. } | Op::StyleRemove { slot } => {
                out.insert(SheetRef::Style(StyleRef(*slot)));
            }
            Op::StyleHeader { style, .. } => {
                out.insert(SheetRef::Style(*style));
            }
            Op::Settings { .. } | Op::Themes { .. } | Op::PartInsert { .. } | Op::PartRemove { .. } => {
                out.insert(SheetRef::Document);
            }
        }
    }
}

// why a run of steps stopped
struct Refused {
    // the steps as they were before the run (complete when `clean`)
    ops: Vec<Op>,
    // whether the steps already run could be taken back (false: the document is half changed)
    clean: bool,
}

// take back the steps of a run that stopped: `done` holds the inverses of the steps that ran,
// each gives the original step back onto `ops`
fn restore(doc: &mut Document, ops: &mut Vec<Op>, mut done: Vec<Op>) -> bool {
    while let Some(inv) = done.pop() {
        match inv.exec(doc) {
            Ok(orig) => ops.push(orig),
            Err(_) => return false,
        }
    }
    true
}

// run steps backwards (undo, rollback) -> their inverses in forward order; on a step that does
// not fit, the steps already run are taken back and Err gives the steps as they were
fn run_backwards(doc: &mut Document, mut ops: Vec<Op>) -> Result<Vec<Op>, Refused> {
    let mut done = Vec::with_capacity(ops.len());
    while let Some(op) = ops.pop() {
        match op.exec(doc) {
            Ok(inv) => done.push(inv),
            Err(op) => {
                ops.push(op);
                let clean = restore(doc, &mut ops, done);
                return Err(Refused { ops, clean });
            }
        }
    }
    done.reverse();
    Ok(done)
}

// run steps forwards (redo) -> their inverses; as run_backwards on failure
fn run_forwards(doc: &mut Document, mut ops: Vec<Op>) -> Result<Vec<Op>, Refused> {
    ops.reverse();
    let mut done = Vec::with_capacity(ops.len());
    while let Some(op) = ops.pop() {
        match op.exec(doc) {
            Ok(inv) => done.push(inv),
            Err(op) => {
                ops.push(op);
                let clean = restore(doc, &mut ops, done);
                ops.reverse();
                return Err(Refused { ops, clean });
            }
        }
    }
    Ok(done)
}

// ---- changes

/// a committed transaction: one undo step
#[derive(Clone, Debug)]
pub struct Change {
    label: String,
    // applied: the inverse steps in the order of the edits; undone: the steps that redo them
    ops: Vec<Op>,
    undone: bool,
    key: u32,
    // a refused step could not be taken back cleanly: the change does nothing any more
    broken: bool,
}

impl Change {
    /// the label of the transaction ("Verschieben")
    pub fn label(&self) -> &str {
        &self.label
    }

    /// the number of primitive steps
    pub fn steps(&self) -> usize {
        self.ops.len()
    }

    /// whether it is undone now
    pub fn is_undone(&self) -> bool {
        self.undone
    }

    /// whether it stopped working: a step no longer fitted and what ran before could not be
    /// taken back (only after untracked changes); undo and redo then refuse
    pub fn is_broken(&self) -> bool {
        self.broken
    }

    /// changes with the same non-zero key merge into one step (a drag)
    pub fn merging(mut self, key: u32) -> Change {
        self.key = key;
        self
    }

    /// the merge key (0: none)
    pub fn merge_key(&self) -> u32 {
        self.key
    }

    /// take the following change into this one when both have the same non-zero key and
    /// both are applied -> whether it did (the next change is then part of this step)
    pub fn merge(&mut self, next: &Change) -> bool {
        if self.key == 0 || self.key != next.key || self.undone || next.undone || self.broken || next.broken {
            return false;
        }
        self.ops.extend(next.ops.iter().cloned());
        true
    }

    /// take the change back -> false when it is undone already or no longer fits the document
    /// (then nothing changes)
    pub fn undo(&mut self, doc: &mut Document) -> bool {
        if self.undone || self.broken {
            return false;
        }
        match run_backwards(doc, mem::take(&mut self.ops)) {
            Ok(redo) => {
                self.ops = redo;
                self.undone = true;
                true
            }
            Err(r) => {
                self.ops = r.ops;
                self.broken = !r.clean;
                false
            }
        }
    }

    /// do the change again -> false when it is applied or no longer fits
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if !self.undone || self.broken {
            return false;
        }
        match run_forwards(doc, mem::take(&mut self.ops)) {
            Ok(undo) => {
                self.ops = undo;
                self.undone = false;
                true
            }
            Err(r) => {
                self.ops = r.ops;
                self.broken = !r.clean;
                false
            }
        }
    }

    /// the sheets it touches, for recalculation: whose cells, rows, sections or attributes it
    /// changes, shapes it adds or removes (of a removed group only the group), the page or
    /// master sheet of changed Connect entries, added or removed pages, masters and styles, the
    /// document sheet for the settings
    pub fn sheets(&self) -> Vec<SheetRef> {
        let mut set = BTreeSet::new();
        for op in &self.ops {
            op.sheets(&mut set);
        }
        set.into_iter().collect()
    }
}

// ---- transactions

/// edits that become one undo step (Document::transaction)
pub struct Transaction<'d> {
    pub(crate) doc: &'d mut Document,
    label: String,
    ops: Vec<Op>,
    open: bool,
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        if self.open {
            let _ = run_backwards(self.doc, mem::take(&mut self.ops));
        }
    }
}

impl Document {
    /// start a transaction; its edits become one undo step when committed
    pub fn transaction(&mut self, label: &str) -> Transaction<'_> {
        Transaction { doc: self, label: String::from(label), ops: Vec::new(), open: true }
    }
}

fn text(sym: Sym) -> Value {
    Value::Text(sym)
}

impl<'d> Transaction<'d> {
    /// the document as it is now (with the edits so far)
    pub fn doc(&self) -> &Document {
        self.doc
    }

    /// the handle of a text (cell names for addresses, values)
    pub fn intern(&mut self, s: &str) -> Sym {
        self.doc.strings.intern(s)
    }

    /// the address of a single cell by name
    pub fn single(&mut self, cell: &str) -> CellAddr {
        CellAddr::single(self.intern(cell))
    }

    /// the address of a cell of a named row (User, Property, Hyperlink ...)
    pub fn named(&mut self, section: &str, row: &str, cell: &str) -> CellAddr {
        let (s, r, c) = (self.intern(section), self.intern(row), self.intern(cell));
        CellAddr::row(SecKey::one(s), RowKey::Named(r), c)
    }

    /// end the transaction -> the change to keep for undo (None: nothing changed)
    pub fn commit(mut self) -> Option<Change> {
        self.open = false;
        let ops = mem::take(&mut self.ops);
        (!ops.is_empty()).then(|| Change { label: mem::take(&mut self.label), ops, undone: false, key: 0, broken: false })
    }

    /// take every edit of the transaction back
    pub fn rollback(self) {}

    /// the sheets the edits so far touch, as Change::sheets names them once committed (an app
    /// that follows a gesture with more work in the same step asks which pages it changed)
    pub fn sheets(&self) -> Vec<SheetRef> {
        let mut set = BTreeSet::new();
        for op in &self.ops {
            op.sheets(&mut set);
        }
        set.into_iter().collect()
    }

    pub(crate) fn run(&mut self, op: Op) -> Result<(), EditError> {
        match op.exec(self.doc) {
            Ok(inv) => {
                self.ops.push(inv);
                Ok(())
            }
            Err(_) => Err(EditError::Stale),
        }
    }

    // run an edit made of several steps: on an error its steps so far are taken back
    pub(crate) fn atomic<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, EditError>) -> Result<T, EditError> {
        let mark = self.ops.len();
        let r = f(self);
        if r.is_err() && self.ops.len() > mark {
            let tail = self.ops.split_off(mark);
            let _ = run_backwards(self.doc, tail);
        }
        r
    }

    // ---- cells

    fn ensure_section(&mut self, sheet: SheetRef, sk: SecKey, must_exist: bool) -> Result<u32, EditError> {
        let sh = self.doc.sheet(sheet).ok_or(EditError::NoSheet)?;
        if let Some(pos) = sh.section_pos(sk) {
            return if sh.sections[pos].del { Err(EditError::Deleted) } else { Ok(pos as u32) };
        }
        if must_exist && !self.doc.has_section(sheet, sk) {
            return Err(EditError::NoSection);
        }
        let pos = sh.sections.len() as u32;
        let mut sec = Section::new(sk);
        sec.ix_written = sk.name == names::GEOMETRY;
        self.run(Op::SecInsert { sheet, pos, sec: Box::new(sec) })?;
        Ok(pos)
    }

    fn ensure_row(&mut self, sheet: SheetRef, sk: SecKey, sp: u32, rk: RowKey) -> Result<u32, EditError> {
        let sec = &self.doc.sheet(sheet).ok_or(EditError::NoSheet)?.sections[sp as usize];
        if let Some(pos) = sec.row_pos(rk) {
            return if sec.rows[pos].del { Err(EditError::Deleted) } else { Ok(pos as u32) };
        }
        let pos = sec.insert_pos(rk) as u32;
        if !self.doc.has_row(sheet, sk, rk) {
            return Err(EditError::NoRow);
        }
        self.run(Op::RowInsert { sheet, sec: sp, pos, row: Box::new(Row::new(rk)) })?;
        Ok(pos)
    }

    // write the local cell at addr: make gets the local cell (if any) and returns the new one
    fn write_cell(&mut self, sheet: SheetRef, addr: &CellAddr, make: impl FnOnce(Option<&Cell>) -> Cell) -> Result<(), EditError> {
        self.atomic(|tx| {
            let scope = match (addr.section, addr.row) {
                (None, _) => Scope::Top,
                (Some(sk), None) => Scope::Section(tx.ensure_section(sheet, sk, true)?),
                (Some(sk), Some(rk)) => {
                    let sp = tx.ensure_section(sheet, sk, true)?;
                    Scope::Row(sp, tx.ensure_row(sheet, sk, sp, rk)?)
                }
            };
            let sh = tx.doc.sheet(sheet).ok_or(EditError::NoSheet)?;
            let cells = match scope {
                Scope::Top => &sh.cells,
                Scope::Section(s) => &sh.sections[s as usize].cells,
                Scope::Row(s, r) => &sh.sections[s as usize].rows[r as usize].cells,
            };
            let pos = cells.iter().position(|c| c.name == addr.cell);
            let old = pos.map(|p| cells[p]);
            let new = make(old.as_ref());
            match pos {
                Some(_) if old == Some(new) => Ok(()),
                Some(p) => tx.run(Op::CellSet { sheet, scope, pos: p as u32, cell: new }),
                None => {
                    let p = cells.len() as u32;
                    tx.run(Op::CellInsert { sheet, scope, pos: p, cell: new })
                }
            }
        })
    }

    fn check_guard(&self, sheet: SheetRef, addr: &CellAddr) -> Result<(), EditError> {
        if self.doc.is_guarded(sheet, addr) { Err(EditError::Guarded) } else { Ok(()) }
    }

    /// give the cell a formula of its own; its value stays until the engine computes the new
    /// one (the cell is stale until then)
    pub fn set_formula(&mut self, sheet: SheetRef, addr: &CellAddr, formula: &str) -> Result<(), EditError> {
        self.check_guard(sheet, addr)?;
        self.force_formula(sheet, addr, formula)
    }

    /// set_formula also over a GUARD formula
    pub fn force_formula(&mut self, sheet: SheetRef, addr: &CellAddr, formula: &str) -> Result<(), EditError> {
        let f = self.intern(formula);
        let inherited = self.doc.resolve(sheet, addr);
        let (value, unit) = (inherited.value(), inherited.unit());
        self.write_cell(sheet, addr, |old| match old {
            Some(c) => Cell { formula: Formula::Expr(f), error: None, stale: true, ..*c },
            None => Cell { name: addr.cell, value, formula: Formula::Expr(f), unit, error: None, stale: true },
        })
    }

    /// set the cell to a constant: the value (internal units) is its formula, shown in `unit`
    pub fn set_constant(&mut self, sheet: SheetRef, addr: &CellAddr, value: Value, unit: Option<Unit>) -> Result<(), EditError> {
        self.check_guard(sheet, addr)?;
        self.force_constant(sheet, addr, value, unit)
    }

    /// set_constant also over a GUARD formula
    pub fn force_constant(&mut self, sheet: SheetRef, addr: &CellAddr, value: Value, unit: Option<Unit>) -> Result<(), EditError> {
        self.write_cell(sheet, addr, |_| Cell::constant(addr.cell, value, unit))
    }

    /// set a length or angle given in `unit` (converted to internal units, shown in `unit`)
    pub fn set_length(&mut self, sheet: SheetRef, addr: &CellAddr, value: f64, unit: Unit) -> Result<(), EditError> {
        self.set_constant(sheet, addr, Value::Num(unit.to_internal(value)), Some(unit))
    }

    /// set a string constant (shown as STR)
    pub fn set_string(&mut self, sheet: SheetRef, addr: &CellAddr, value: &str) -> Result<(), EditError> {
        let v = text(self.intern(value));
        self.set_constant(sheet, addr, v, Some(Unit::Str))
    }

    /// remove the sheet's own cell: the cell inherits again ("Formel löschen")
    pub fn inherit_cell(&mut self, sheet: SheetRef, addr: &CellAddr) -> Result<(), EditError> {
        let sh = self.doc.sheet(sheet).ok_or(EditError::NoSheet)?;
        let found = match (addr.section, addr.row) {
            (None, _) => sh.cells.iter().position(|c| c.name == addr.cell).map(|p| (Scope::Top, p)),
            (Some(sk), row) => sh.section_pos(sk).and_then(|sp| {
                let sec = &sh.sections[sp];
                match row {
                    None => sec.cells.iter().position(|c| c.name == addr.cell).map(|p| (Scope::Section(sp as u32), p)),
                    Some(rk) => sec.row_pos(rk).and_then(|rp| {
                        sec.rows[rp].cell_pos(addr.cell).map(|p| (Scope::Row(sp as u32, rp as u32), p))
                    }),
                }
            }),
        };
        match found {
            Some((scope, pos)) => self.run(Op::CellRemove { sheet, scope, pos: pos as u32 }),
            None => Ok(()),
        }
    }

    /// (the engine) store the result of the cell's formula for this sheet; a cell that
    /// inherits its formula gets an F="Inh" cell of its own
    pub fn set_result(&mut self, sheet: SheetRef, addr: &CellAddr, value: Value, error: Option<CellError>) -> Result<(), EditError> {
        let r = self.doc.resolve(sheet, addr);
        if !r.exists() {
            return Err(if r.origin == crate::cell::Origin::Deleted { EditError::Deleted } else { EditError::Invalid });
        }
        let unit = r.unit();
        let e = error.map(|e| self.intern(e.as_str()));
        self.write_cell(sheet, addr, |old| match old {
            Some(c) => Cell { value, error: e, stale: false, ..*c },
            None => Cell { name: addr.cell, value, formula: Formula::Inherited, unit, error: e, stale: false },
        })
    }

    // ---- sections and rows

    /// add a section of the sheet's own -> its key (Geometry: the next free index); Exists
    /// when the sheet has it already
    pub fn add_section(&mut self, sheet: SheetRef, name: &str) -> Result<SecKey, EditError> {
        let n = self.intern(name);
        let index = if n == names::GEOMETRY {
            // past every index the sheet has, also its own deleted ones (their Del marks stay)
            let own = self.doc.sheet(sheet).ok_or(EditError::NoSheet)?.sections.iter().filter(|s| s.name == n).map(|s| s.index);
            self.doc.section_indices(sheet, n).into_iter().chain(own).max().map_or(0, |i| i + 1)
        } else {
            0
        };
        let sk = SecKey { name: n, index };
        let sh = self.doc.sheet(sheet).ok_or(EditError::NoSheet)?;
        if sh.section_pos(sk).is_some() || self.doc.has_section(sheet, sk) {
            return Err(EditError::Exists);
        }
        self.ensure_section(sheet, sk, false)?;
        Ok(sk)
    }

    /// delete a section: an inherited one gets a Del="1" mark, an own one goes
    pub fn delete_section(&mut self, sheet: SheetRef, sk: SecKey) -> Result<(), EditError> {
        if !self.doc.has_section(sheet, sk) {
            return Err(EditError::NoSection);
        }
        let parent = self.doc.parent_sheet(sheet, Category::of_section(self.doc.str(sk.name)));
        let inherited = parent.is_some_and(|p| self.doc.has_section(p, sk));
        let local = self.doc.sheet(sheet).ok_or(EditError::NoSheet)?.section_pos(sk);
        self.atomic(|tx| {
            let mut pos = tx.doc.sheet(sheet).map_or(0, |s| s.sections.len() as u32);
            if let Some(p) = local {
                tx.run(Op::SecRemove { sheet, pos: p as u32 })?;
                pos = p as u32;
            }
            if inherited {
                let mut mark = Section::new(sk);
                mark.del = true;
                mark.ix_written = sk.name == names::GEOMETRY;
                tx.run(Op::SecInsert { sheet, pos, sec: Box::new(mark) })?;
            }
            Ok(())
        })
    }

    /// remove the Del="1" mark of an inherited section: it is inherited again
    pub fn restore_section(&mut self, sheet: SheetRef, sk: SecKey) -> Result<(), EditError> {
        let sh = self.doc.sheet(sheet).ok_or(EditError::NoSheet)?;
        match sh.section_pos(sk) {
            Some(p) if sh.sections[p].del => self.run(Op::SecRemove { sheet, pos: p as u32 }),
            _ => Err(EditError::NoSection),
        }
    }

    /// add a row of the sheet's own to a section (made when missing) -> its key; `key` None:
    /// the next index (Geometry from 1, other indexed sections from 0); `kind` the row type
    /// (Geometry: "LineTo" ...)
    pub fn add_row(&mut self, sheet: SheetRef, sk: SecKey, key: Option<RowKey>, kind: Option<&str>) -> Result<RowKey, EditError> {
        let rows = self.doc.rows(sheet, sk);
        let key = match key {
            Some(k) => k,
            None => {
                // past every index the section has, also the sheet's own deleted rows (Del marks)
                let own = self.doc.sheet(sheet).and_then(|s| s.section(sk)).map_or(&[][..], |s| s.rows.as_slice());
                let max = rows.iter().map(|r| r.key).chain(own.iter().map(|r| r.key)).filter_map(|k| match k {
                    RowKey::Index(i) => Some(i),
                    RowKey::Named(_) => None,
                }).max();
                RowKey::Index(max.map_or(if sk.name == names::GEOMETRY { 1 } else { 0 }, |m| m + 1))
            }
        };
        if rows.iter().any(|r| r.key == key) {
            return Err(EditError::Exists);
        }
        let kind = kind.map(|k| self.intern(k));
        self.atomic(|tx| {
            let sp = tx.ensure_section(sheet, sk, false)?;
            let sec = &tx.doc.sheet(sheet).ok_or(EditError::NoSheet)?.sections[sp as usize];
            if sec.row_pos(key).is_some() {
                return Err(EditError::Deleted);
            }
            let pos = sec.insert_pos(key) as u32;
            let mut row = Row::new(key);
            row.kind = kind;
            tx.run(Op::RowInsert { sheet, sec: sp, pos, row: Box::new(row) })?;
            Ok(key)
        })
    }

    /// delete a row: an inherited one gets a Del="1" mark (its own cells go), an own one goes
    pub fn delete_row(&mut self, sheet: SheetRef, sk: SecKey, key: RowKey) -> Result<(), EditError> {
        if !self.doc.has_row(sheet, sk, key) {
            return Err(EditError::NoRow);
        }
        let parent = self.doc.parent_sheet(sheet, Category::of_section(self.doc.str(sk.name)));
        let inherited = parent.is_some_and(|p| self.doc.has_row(p, sk, key));
        self.atomic(|tx| {
            let sp = tx.ensure_section(sheet, sk, true)?;
            let sec = &tx.doc.sheet(sheet).ok_or(EditError::NoSheet)?.sections[sp as usize];
            let mut pos = sec.insert_pos(key) as u32;
            if let Some(p) = sec.row_pos(key) {
                tx.run(Op::RowRemove { sheet, sec: sp, pos: p as u32 })?;
                pos = p as u32;
            }
            if inherited {
                let mut mark = Row::new(key);
                mark.del = true;
                tx.run(Op::RowInsert { sheet, sec: sp, pos, row: Box::new(mark) })?;
            }
            Ok(())
        })
    }

    /// remove the Del="1" mark of an inherited row: it is inherited again
    pub fn restore_row(&mut self, sheet: SheetRef, sk: SecKey, key: RowKey) -> Result<(), EditError> {
        let sh = self.doc.sheet(sheet).ok_or(EditError::NoSheet)?;
        let found = sh.section_pos(sk).and_then(|sp| {
            let sec = &sh.sections[sp];
            sec.row_pos(key).filter(|&rp| sec.rows[rp].del).map(|rp| (sp, rp))
        });
        match found {
            Some((sp, rp)) => self.run(Op::RowRemove { sheet, sec: sp as u32, pos: rp as u32 }),
            None => Err(EditError::NoRow),
        }
    }

    fn ensure_named_row(&mut self, sheet: SheetRef, section: Sym, name: &str) -> Result<(SecKey, RowKey), EditError> {
        let sk = SecKey::one(section);
        let rk = RowKey::Named(self.intern(name));
        if !self.doc.has_row(sheet, sk, rk) {
            self.add_row(sheet, sk, Some(rk), None)?;
        }
        Ok((sk, rk))
    }

    /// set a user-defined cell (User.<name>.Value) to a constant, adding the row when missing
    pub fn set_user(&mut self, sheet: SheetRef, name: &str, value: Value, unit: Option<Unit>) -> Result<(), EditError> {
        self.atomic(|tx| {
            let (sk, rk) = tx.ensure_named_row(sheet, names::USER, name)?;
            tx.set_constant(sheet, &CellAddr::row(sk, rk, names::VALUE), value, unit)
        })
    }

    /// give a user-defined cell a formula, adding the row when missing
    pub fn set_user_formula(&mut self, sheet: SheetRef, name: &str, formula: &str) -> Result<(), EditError> {
        self.atomic(|tx| {
            let (sk, rk) = tx.ensure_named_row(sheet, names::USER, name)?;
            tx.set_formula(sheet, &CellAddr::row(sk, rk, names::VALUE), formula)
        })
    }

    /// set a shape data field (Prop.<name>): its label and value, adding the row when missing
    pub fn set_shape_data(&mut self, sheet: SheetRef, name: &str, label: &str, value: Value) -> Result<(), EditError> {
        self.atomic(|tx| {
            let (sk, rk) = tx.ensure_named_row(sheet, names::PROPERTY, name)?;
            tx.set_string(sheet, &CellAddr::row(sk, rk, names::LABEL), label)?;
            let unit = matches!(value, Value::Text(_)).then_some(Unit::Str);
            tx.set_constant(sheet, &CellAddr::row(sk, rk, names::VALUE), value, unit)
        })
    }

    /// add or change a hyperlink (Hyperlink.<name>): address, sub-address, description
    pub fn set_hyperlink(&mut self, sheet: SheetRef, name: &str, address: &str, sub_address: &str, description: &str) -> Result<(), EditError> {
        self.atomic(|tx| {
            let (sk, rk) = tx.ensure_named_row(sheet, names::HYPERLINK, name)?;
            tx.set_string(sheet, &CellAddr::row(sk, rk, names::ADDRESS), address)?;
            tx.set_string(sheet, &CellAddr::row(sk, rk, names::SUB_ADDRESS), sub_address)?;
            tx.set_string(sheet, &CellAddr::row(sk, rk, names::DESCRIPTION), description)
        })
    }

    // ---- layers

    /// add a layer to a page -> its index; visible, printed, snapping and gluing, not locked
    pub fn add_layer(&mut self, page: PageRef, name: &str, name_univ: &str) -> Result<u32, EditError> {
        self.doc.page(page).ok_or(EditError::NoPage)?;
        let sheet = SheetRef::Page(page);
        let sk = SecKey::one(names::LAYER);
        self.atomic(|tx| {
            let RowKey::Index(ix) = tx.add_row(sheet, sk, None, None)? else { return Err(EditError::Invalid) };
            let rk = RowKey::Index(ix);
            let (n, nu) = (tx.intern(name), tx.intern(name_univ));
            tx.set_constant(sheet, &CellAddr::row(sk, rk, names::NAME), text(n), None)?;
            tx.set_constant(sheet, &CellAddr::row(sk, rk, names::NAME_UNIV), text(nu), None)?;
            let flags = [(names::VISIBLE, 1.0), (names::PRINT, 1.0), (names::ACTIVE, 0.0), (names::LOCK, 0.0),
                (names::SNAP, 1.0), (names::GLUE, 1.0), (names::COLOR, 255.0), (names::COLOR_TRANS, 0.0)];
            for (cell, v) in flags {
                tx.set_constant(sheet, &CellAddr::row(sk, rk, cell), Value::Num(v), None)?;
            }
            Ok(ix)
        })
    }

    /// put a shape on these layers (LayerMember; empty: on none)
    pub fn set_layers(&mut self, shape: ShapeRef, layers: &[u32]) -> Result<(), EditError> {
        let mut s = String::new();
        for (i, l) in layers.iter().enumerate() {
            if i > 0 {
                s.push(';');
            }
            s.push_str(&alloc::format!("{l}"));
        }
        let v = Value::parse(&s, None, &mut self.doc.strings);
        self.set_constant(SheetRef::Shape(shape), &CellAddr::single(names::LAYER_MEMBER), v, None)
    }

    /// delete a layer of a page; shapes on it leave it (their own LayerMember cells), the
    /// shapes themselves stay (LY-04 asks the user first and deletes them before).  The layers
    /// after it move one down (IX - 1, as Visio keeps a page's layers numbered without gaps) and
    /// the shapes' own LayerMember cells follow
    pub fn delete_layer(&mut self, page: PageRef, ix: u32) -> Result<(), EditError> {
        self.doc.page(page).ok_or(EditError::NoPage)?;
        let owner = Owner::Page(page);
        let members: Vec<(ShapeRef, Vec<u32>)> = self
            .doc
            .all_shapes(owner)
            .filter_map(|s| {
                let own = self.doc.sheet(SheetRef::Shape(s))?.cell(names::LAYER_MEMBER)?;
                let list = match own.value {
                    Value::Text(t) => crate::views::parse_members(self.doc.str(t)),
                    Value::Num(x) if x >= 0.0 => alloc::vec![x as u32],
                    _ => return None,
                };
                let new: Vec<u32> = list.iter().filter(|&&l| l != ix).map(|&l| if l > ix { l - 1 } else { l }).collect();
                (new != list).then_some((s, new))
            })
            .collect();
        self.atomic(|tx| {
            tx.remove_row(SheetRef::Page(page), SecKey::one(names::LAYER), RowKey::Index(ix))?;
            for (s, list) in &members {
                tx.set_layers(*s, list)?;
            }
            Ok(())
        })
    }

    // ---- shapes

    fn shape_header(&self, shape: ShapeRef) -> Result<ShapeHeader, EditError> {
        Ok(self.doc.shape(shape).ok_or(EditError::NoShape)?.header().clone())
    }

    fn put_header(&mut self, shape: ShapeRef, header: ShapeHeader) -> Result<(), EditError> {
        if self.doc.shape(shape).is_some_and(|s| *s.header() == header) {
            return Ok(());
        }
        self.run(Op::ShapeHeader { shape, header: Box::new(header) })
    }

    fn make_group(&mut self, group: ShapeRef) -> Result<(), EditError> {
        let mut h = self.shape_header(group)?;
        if h.kind != ShapeKind::Group {
            h.kind = ShapeKind::Group;
            self.put_header(group, h)?;
        }
        Ok(())
    }

    /// add an empty shape on a page or in a master, on top of its siblings -> its handle; a
    /// parent that is not a group becomes one
    pub fn add_shape(&mut self, owner: Owner, parent: Option<ShapeRef>, kind: ShapeKind) -> Result<ShapeRef, EditError> {
        self.add_shape_with(owner, parent, ShapeData::new(kind))
    }

    /// add a shape with its attributes and cells (the ID is the tree's next one)
    pub fn add_shape_with(&mut self, owner: Owner, parent: Option<ShapeRef>, data: ShapeData) -> Result<ShapeRef, EditError> {
        if let Some(p) = parent {
            if p.owner != owner || self.doc.shape(p).is_none() {
                return Err(EditError::NoShape);
            }
        }
        self.atomic(|tx| {
            if let Some(p) = parent {
                tx.make_group(p)?;
            }
            let tree = tx.doc.tree_mut(owner).ok_or(EditError::NoPage)?;
            let pslot = parent.map(|p| p.slot);
            let pos = tree.siblings(pslot).len() as u32;
            let (slot, id) = tree.alloc().ok_or(EditError::NoIds)?;
            let node = Box::new(Node { id, parent: pslot, children: Vec::new(), data });
            tx.run(Op::ShapeInsert { owner, parent: pslot, pos, nodes: alloc::vec![(slot, node)] })?;
            Ok(ShapeRef { owner, slot })
        })
    }

    /// drop an instance of a master on a page (MS-02): a shape that inherits everything from
    /// the master shape and sets only its pin, with subshapes for the master's subshapes
    /// -> the instance; its Name is the caller's (LG-04: the name in the current language)
    pub fn add_instance(&mut self, owner: Owner, master: MasterRef, pin_x: f64, pin_y: f64) -> Result<ShapeRef, EditError> {
        let m = self.doc.master(master).ok_or(EditError::NoMaster)?;
        if let Owner::Master(target) = owner {
            let tid = self.doc.master(target).ok_or(EditError::NoMaster)?.header.id;
            if target == master || self.doc.master_depends_on(m.header.id, tid) {
                return Err(EditError::Cycle);
            }
        }
        let mid = m.header.id;
        let ms = m.master_shape().ok_or(EditError::Invalid)?;
        // the master shape's subtree: (kind, master shape ID, index of the parent in the list)
        let mut plan: Vec<(ShapeKind, u32, Option<usize>)> = Vec::new();
        let mut stack = alloc::vec![(ms, None::<usize>)];
        while let Some((slot, parent)) = stack.pop() {
            let (Some(d), Some(id)) = (m.shapes.get(slot), m.shapes.id(slot)) else { continue };
            let i = plan.len();
            plan.push((d.header.kind, id, parent));
            stack.extend(m.shapes.children(slot).iter().rev().map(|&c| (c, Some(i))));
        }
        if plan.is_empty() {
            return Err(EditError::Invalid);
        }
        let tree = self.doc.tree_mut(owner).ok_or(EditError::NoPage)?;
        let pos = tree.top().len() as u32;
        let mut nodes: Vec<(u32, Box<Node>)> = Vec::with_capacity(plan.len());
        for (i, &(kind, msid, parent)) in plan.iter().enumerate() {
            let (slot, id) = tree.alloc().ok_or(EditError::NoIds)?;
            let mut data = ShapeData::new(kind);
            if i == 0 {
                data.header.master = Some(mid);
                data.sheet.cells.push(Cell::constant(names::PIN_X, Value::Num(pin_x), None));
                data.sheet.cells.push(Cell::constant(names::PIN_Y, Value::Num(pin_y), None));
            } else {
                data.header.master_shape = Some(msid);
            }
            let pslot = parent.map(|p| nodes[p].0);
            if let Some(p) = parent {
                nodes[p].1.children.push(slot);
            }
            nodes.push((slot, Box::new(Node { id, parent: pslot, children: Vec::new(), data })));
        }
        let root = nodes[0].0;
        self.run(Op::ShapeInsert { owner, parent: None, pos, nodes })?;
        Ok(ShapeRef { owner, slot: root })
    }

    /// delete a shape with its subshapes and the glue records of them; a subshape the master
    /// passes on gets a Del="1" mark instead
    pub fn delete_shape(&mut self, shape: ShapeRef) -> Result<(), EditError> {
        self.delete_shapes(&[shape])
    }

    /// delete several shapes in one go (a selection): the glue records are searched once, not
    /// once per shape; shapes inside another one of the list go with it.  The master shape of a
    /// master in use cannot be deleted (InUse: its instances would inherit from another shape)
    pub fn delete_shapes(&mut self, shapes: &[ShapeRef]) -> Result<(), EditError> {
        let mut marks: Vec<(ShapeRef, ShapeHeader)> = Vec::new();
        let mut roots: Vec<ShapeRef> = Vec::new();
        let mut ids: BTreeSet<(Owner, u32)> = BTreeSet::new();
        let mut seen: BTreeSet<ShapeRef> = BTreeSet::new();
        for &shape in shapes {
            let s = self.doc.shape(shape).ok_or(EditError::NoShape)?;
            if s.header().master_shape.is_some() && s.header().master.is_none() && s.master_shape().is_some() {
                let mut h = s.header().clone();
                h.del = true;
                marks.push((shape, h));
                continue;
            }
            if !seen.insert(shape) {
                continue;
            }
            self.check_master_shape(shape.owner, |top| top.retain(|&x| x != shape.slot))?;
            roots.push(shape);
            let tree = self.doc.tree(shape.owner).ok_or(EditError::NoPage)?;
            let mut stack = alloc::vec![shape.slot];
            while let Some(slot) = stack.pop() {
                if let Some(id) = tree.id(slot) {
                    ids.insert((shape.owner, id));
                }
                stack.extend(tree.children(slot));
            }
        }
        // a shape inside another root goes with it (walk up from each root: no pairwise test)
        let root_set: BTreeSet<(Owner, u32)> = roots.iter().map(|r| (r.owner, r.slot)).collect();
        let inner: Vec<bool> = roots
            .iter()
            .map(|r| {
                let Some(t) = self.doc.tree(r.owner) else { return false };
                let mut up = t.parent(r.slot);
                while let Some(p) = up {
                    if root_set.contains(&(r.owner, p)) {
                        return true;
                    }
                    up = t.parent(p);
                }
                false
            })
            .collect();
        let owners: BTreeSet<Owner> = roots.iter().map(|r| r.owner).collect();
        let mut gone: Vec<(Owner, u32)> = Vec::new();
        for owner in owners {
            let connects = match owner {
                Owner::Page(p) => self.doc.page(p).map(|x| &x.connects),
                Owner::Master(m) => self.doc.master(m).map(|x| &x.connects),
            };
            if let Some(cs) = connects {
                let hit = |id: u32| ids.contains(&(owner, id));
                gone.extend(cs.iter().enumerate().filter(|(_, c)| hit(c.from_sheet) || hit(c.to_sheet)).map(|(i, _)| (owner, i as u32)));
            }
        }
        // remove from the top of each sibling list down, so each removal finds its shape at once
        let outer: Vec<ShapeRef> = roots.iter().zip(inner).filter(|(_, inside)| !inside).map(|(r, _)| *r).collect();
        let mut groups: BTreeSet<(Owner, Option<u32>)> = BTreeSet::new();
        for r in &outer {
            groups.insert((r.owner, self.doc.tree(r.owner).and_then(|t| t.parent(r.slot))));
        }
        let outer_set: BTreeSet<ShapeRef> = outer.iter().copied().collect();
        let mut order: Vec<ShapeRef> = Vec::with_capacity(outer.len());
        for (owner, parent) in groups {
            let Some(t) = self.doc.tree(owner) else { continue };
            let sib = t.siblings(parent);
            order.extend(sib.iter().rev().map(|&slot| ShapeRef { owner, slot }).filter(|r| outer_set.contains(r)));
        }
        self.atomic(|tx| {
            for (shape, h) in marks {
                tx.put_header(shape, h)?;
            }
            for &(owner, pos) in gone.iter().rev() {
                tx.run(Op::ConnectRemove { owner, pos })?;
            }
            for r in order {
                tx.run(Op::ShapeRemove { owner: r.owner, slot: r.slot })?;
            }
            Ok(())
        })
    }

    // InUse when an edit of a master's top level (`change` applied to a copy of it) would give
    // the master's instances another master shape
    fn check_master_shape(&self, owner: Owner, change: impl FnOnce(&mut Vec<u32>)) -> Result<(), EditError> {
        let Owner::Master(m) = owner else { return Ok(()) };
        let Some(master) = self.doc.master(m) else { return Ok(()) };
        let mut top = master.shapes.top().to_vec();
        let before = top.first().copied();
        change(&mut top);
        if top.first().copied() != before && self.doc.master_in_use(master.header.id) {
            return Err(EditError::InUse);
        }
        Ok(())
    }

    // a subshape of an instance inherits by its MasterShape ID inside that instance: it stays
    // in its group (Invalid); within the group the z-order may change
    fn check_instance_part(&self, shape: ShapeRef, parent: Option<u32>) -> Result<(), EditError> {
        let s = self.doc.shape(shape).ok_or(EditError::NoShape)?;
        let h = s.header();
        if h.master_shape.is_some() && h.master.is_none() && s.parent().map(|p| p.slot) != parent {
            return Err(EditError::Invalid);
        }
        Ok(())
    }

    /// bring a deleted subshape of an instance back (remove its Del="1" mark)
    pub fn undelete_shape(&mut self, shape: ShapeRef) -> Result<(), EditError> {
        let mut h = self.shape_header(shape)?;
        h.del = false;
        self.put_header(shape, h)
    }

    /// move a shape in the z-order or into another group of the same tree: under `parent`
    /// (None: top level) at `pos` (None: on top); a parent that is not a group becomes one.
    /// Coordinates stay as they are (re-grouping with transforms is the canvas's work, JD7)
    pub fn move_shape(&mut self, shape: ShapeRef, parent: Option<ShapeRef>, pos: Option<usize>) -> Result<(), EditError> {
        let tree = self.doc.tree(shape.owner).ok_or(EditError::NoPage)?;
        tree.node(shape.slot).ok_or(EditError::NoShape)?;
        if let Some(p) = parent {
            if p.owner != shape.owner || tree.node(p.slot).is_none() {
                return Err(EditError::NoShape);
            }
            if tree.is_within(p.slot, shape.slot) {
                return Err(EditError::Cycle);
            }
        }
        let pslot = parent.map(|p| p.slot);
        self.check_instance_part(shape, pslot)?;
        let old_parent = tree.parent(shape.slot);
        self.check_master_shape(shape.owner, |top| {
            if old_parent.is_none() {
                top.retain(|&x| x != shape.slot);
            }
            if pslot.is_none() {
                let at = pos.unwrap_or(usize::MAX).min(top.len());
                top.insert(at, shape.slot);
            }
        })?;
        let pos = pos.map_or(u32::MAX, |p| p as u32);
        self.atomic(|tx| {
            if let Some(p) = parent {
                tx.make_group(p)?;
            }
            tx.run(Op::ShapeMove { shape, parent: pslot, pos })
        })
    }

    fn z_move(&mut self, shape: ShapeRef, to: impl FnOnce(usize, usize) -> usize) -> Result<(), EditError> {
        let s = self.doc.shape(shape).ok_or(EditError::NoShape)?;
        let parent = s.parent();
        let tree = self.doc.tree(shape.owner).ok_or(EditError::NoPage)?;
        let sib = tree.siblings(parent.map(|p| p.slot));
        let cur = sib.iter().position(|&x| x == shape.slot).ok_or(EditError::Stale)?;
        let at = to(cur, sib.len());
        if at == cur {
            return Ok(());
        }
        if parent.is_none() {
            self.check_master_shape(shape.owner, |top| {
                let x = top.remove(cur);
                top.insert(at, x);
            })?;
        }
        self.run(Op::ShapeMove { shape, parent: parent.map(|p| p.slot), pos: at as u32 })
    }

    /// put a shape on top of its siblings
    pub fn bring_to_front(&mut self, shape: ShapeRef) -> Result<(), EditError> {
        self.z_move(shape, |_, n| n - 1)
    }

    /// put a shape below its siblings
    pub fn send_to_back(&mut self, shape: ShapeRef) -> Result<(), EditError> {
        self.z_move(shape, |_, _| 0)
    }

    /// move a shape one step up among its siblings
    pub fn bring_forward(&mut self, shape: ShapeRef) -> Result<(), EditError> {
        self.z_move(shape, |c, n| (c + 1).min(n - 1))
    }

    /// move a shape one step down among its siblings
    pub fn send_backward(&mut self, shape: ShapeRef) -> Result<(), EditError> {
        self.z_move(shape, |c, _| c.saturating_sub(1))
    }

    /// set a shape's Name and NameU (None: derived again)
    pub fn set_shape_names(&mut self, shape: ShapeRef, name: Option<&str>, name_u: Option<&str>) -> Result<(), EditError> {
        let mut h = self.shape_header(shape)?;
        h.name = name.map(|n| self.intern(n));
        h.name_u = name_u.map(|n| self.intern(n));
        h.is_custom_name = h.name.map(|_| true);
        h.is_custom_name_u = h.name_u.map(|_| true);
        self.put_header(shape, h)
    }

    /// set the styles a shape inherits from (LineStyle, FillStyle, TextStyle)
    pub fn set_shape_styles(&mut self, shape: ShapeRef, styles: StyleRefs) -> Result<(), EditError> {
        for id in [styles.line, styles.fill, styles.text].into_iter().flatten() {
            self.doc.style_by_id(id).ok_or(EditError::NoStyle)?;
        }
        let mut h = self.shape_header(shape)?;
        h.styles = styles;
        self.put_header(shape, h)
    }

    /// set a shape's attributes as a whole (kind, names, references)
    pub fn set_shape_header(&mut self, shape: ShapeRef, header: ShapeHeader) -> Result<(), EditError> {
        self.doc.shape(shape).ok_or(EditError::NoShape)?;
        self.put_header(shape, header)
    }

    /// set a shape's own text (None: inherited from the master shape again)
    pub fn set_text(&mut self, shape: ShapeRef, text: Option<Text>) -> Result<(), EditError> {
        self.doc.shape(shape).ok_or(EditError::NoShape)?;
        self.run(Op::ShapeText { shape, text: text.map(Box::new) })
    }

    /// add a part the model keeps as bytes (a picture's media part) at the end of
    /// Document::parts -> its index; a name some part has already (ASCII case ignored, a
    /// leading '/' too) is refused (Exists)
    pub fn add_part(&mut self, name: &str, content_type: &str, rel_type: &str, data: Vec<u8>) -> Result<usize, EditError> {
        let bare = |n: &str| -> String { n.trim_start_matches('/').to_ascii_lowercase() };
        let want = bare(name);
        if want.is_empty() || self.doc.parts.iter().any(|p| bare(self.doc.str(p.name)) == want) {
            return Err(EditError::Exists);
        }
        let part = Part { name: self.intern(name), content_type: self.intern(content_type), rel_type: self.intern(rel_type), data };
        let pos = self.doc.parts.len();
        self.run(Op::PartInsert { pos: pos as u32, part: Box::new(part) })?;
        Ok(pos)
    }

    /// remove the part at `index` of Document::parts (shapes that name it show no picture)
    pub fn remove_part(&mut self, index: usize) -> Result<(), EditError> {
        self.run(Op::PartRemove { pos: index as u32 }).map_err(|_| EditError::Invalid)
    }

    /// set the picture data of a Foreign shape
    pub fn set_foreign(&mut self, shape: ShapeRef, foreign: Option<ForeignData>) -> Result<(), EditError> {
        self.doc.shape(shape).ok_or(EditError::NoShape)?;
        self.run(Op::ShapeForeign { shape, foreign: foreign.map(Box::new) })
    }

    // ---- glue records

    /// add a Connect entry on a page or in a master
    pub fn add_connect(&mut self, owner: Owner, connect: Connect) -> Result<(), EditError> {
        let n = match owner {
            Owner::Page(p) => self.doc.page(p).map(|x| x.connects.len()),
            Owner::Master(m) => self.doc.master(m).map(|x| x.connects.len()),
        };
        let pos = n.ok_or(EditError::NoPage)? as u32;
        self.run(Op::ConnectInsert { owner, pos, connect: Box::new(connect) })
    }

    /// remove the Connect entry at `index`
    pub fn remove_connect(&mut self, owner: Owner, index: usize) -> Result<(), EditError> {
        self.run(Op::ConnectRemove { owner, pos: index as u32 }).map_err(|_| EditError::Invalid)
    }

    // ---- pages

    /// add a page (foreground or background) at `pos` in document order (None: at the end)
    /// with the next free page ID -> its handle
    pub fn add_page(&mut self, name: &str, name_u: &str, background: bool, pos: Option<usize>) -> Result<PageRef, EditError> {
        let (n, nu) = (self.intern(name), self.intern(name_u));
        let len = self.doc.pages.len();
        let (slot, id) = self.doc.pages.alloc().ok_or(EditError::NoIds)?;
        let header = PageHeader { id, name: Some(n), name_u: Some(nu), background, ..PageHeader::default() };
        let pos = pos.unwrap_or(len).min(len) as u32;
        self.run(Op::PageInsert { slot, pos, page: Box::new(Page::new(header)) })?;
        Ok(PageRef(slot))
    }

    /// delete a page; pages that had it as background lose that background, TopPage moves to
    /// the first remaining foreground page
    pub fn delete_page(&mut self, page: PageRef) -> Result<(), EditError> {
        let id = self.doc.page(page).ok_or(EditError::NoPage)?.header.id;
        let users: Vec<PageRef> = self.doc.pages().filter(|&p| p != page && self.doc.page(p).is_some_and(|x| x.header.back_page == Some(id))).collect();
        self.atomic(|tx| {
            for p in users {
                let mut h = tx.doc.page(p).ok_or(EditError::NoPage)?.header.clone();
                h.back_page = None;
                tx.run(Op::PageHeader { page: p, header: Box::new(h) })?;
            }
            if tx.doc.settings.top_page == Some(id) {
                let next = tx.doc.foreground_pages().find(|&p| p != page).and_then(|p| tx.doc.page(p)).map(|x| x.header.id);
                let mut s = tx.doc.settings.clone();
                s.top_page = next;
                tx.run(Op::Settings { settings: Box::new(s) })?;
            }
            tx.run(Op::PageRemove { slot: page.0 })
        })
    }

    /// move a page to `pos` in document order
    pub fn move_page(&mut self, page: PageRef, pos: usize) -> Result<(), EditError> {
        let cur = self.doc.page_position(page).ok_or(EditError::NoPage)?;
        let pos = pos.min(self.doc.pages.len() - 1);
        if cur == pos {
            return Ok(());
        }
        self.run(Op::PageMove { page, pos: pos as u32 })
    }

    /// rename a page (Name and NameU)
    pub fn rename_page(&mut self, page: PageRef, name: &str, name_u: &str) -> Result<(), EditError> {
        let mut h = self.doc.page(page).ok_or(EditError::NoPage)?.header.clone();
        h.name = Some(self.intern(name));
        h.name_u = Some(self.intern(name_u));
        self.run(Op::PageHeader { page, header: Box::new(h) })
    }

    /// set the page's attributes as a whole
    pub fn set_page_header(&mut self, page: PageRef, header: PageHeader) -> Result<(), EditError> {
        self.doc.page(page).ok_or(EditError::NoPage)?;
        if self.doc.page_by_id(header.id).is_some_and(|p| p != page) {
            return Err(EditError::Exists);
        }
        self.run(Op::PageHeader { page, header: Box::new(header) })
    }

    /// give a page a background page (None: none); the background must be a background page
    /// and the chain of backgrounds must not lead back to the page
    pub fn set_background(&mut self, page: PageRef, back: Option<PageRef>) -> Result<(), EditError> {
        let mut h = self.doc.page(page).ok_or(EditError::NoPage)?.header.clone();
        h.back_page = match back {
            None => None,
            Some(b) => {
                let bp = self.doc.page(b).ok_or(EditError::NoPage)?;
                if !bp.header.background {
                    return Err(EditError::NotBackground);
                }
                let mut cur = Some(b);
                for _ in 0..=self.doc.pages.len() {
                    match cur {
                        Some(c) if c == page => return Err(EditError::Cycle),
                        Some(c) => cur = self.doc.background_of(c),
                        None => break,
                    }
                }
                Some(bp.header.id)
            }
        };
        self.run(Op::PageHeader { page, header: Box::new(h) })
    }

    /// set the theme of a page (an index into Document::themes, None: none)
    pub fn set_page_theme(&mut self, page: PageRef, theme: Option<u32>) -> Result<(), EditError> {
        self.doc.page(page).ok_or(EditError::NoPage)?;
        if theme.is_some_and(|t| t as usize >= self.doc.themes.len()) {
            return Err(EditError::Invalid);
        }
        self.run(Op::PageTheme { page, theme })
    }

    /// add an empty page with the settings of page `like` (PG-03: Visio gives a new page the
    /// properties of the active page -- size, scale, units, print setup, layout and routing,
    /// background page, design): its page sheet without the Layer section (layers belong to a
    /// page's shapes), the styles of its PageSheet element, its theme; a foreground page keeps
    /// `like`'s background page, a background page gets none.  At `pos` in document order (None:
    /// the end) with the next free page ID -> its handle
    pub fn add_page_like(&mut self, like: PageRef, name: &str, name_u: &str, background: bool, pos: Option<usize>) -> Result<PageRef, EditError> {
        let src = self.doc.page(like).ok_or(EditError::NoPage)?;
        let mut sheet = src.sheet.clone();
        sheet.sections.retain(|s| s.name != names::LAYER);
        let back_page = if background { None } else { src.header.back_page.filter(|&id| self.doc.page_by_id(id).is_some()) };
        let (sheet_styles, theme) = (src.header.sheet_styles, src.theme);
        let (n, nu) = (self.intern(name), self.intern(name_u));
        let len = self.doc.pages.len();
        let (slot, id) = self.doc.pages.alloc().ok_or(EditError::NoIds)?;
        let header = PageHeader { id, name: Some(n), name_u: Some(nu), background, back_page, sheet_styles, ..PageHeader::default() };
        let page = Page { sheet, theme, ..Page::new(header) };
        let pos = pos.unwrap_or(len).min(len) as u32;
        self.run(Op::PageInsert { slot, pos, page: Box::new(page) })?;
        Ok(PageRef(slot))
    }

    /// a copy of a page with its shapes, glue records and settings (PG-03 Duplizieren) at `pos`
    /// in document order (None: right after the original, as Visio's Page.Duplicate puts it)
    /// -> its handle.  The shapes keep their IDs (IDs count per page), so formulas that name
    /// Sheet.N, the Connect entries and the Relationships of containers and lists stay inside
    /// the copy; UniqueIDs are dropped (each names one shape of the document), the copy gets the
    /// next free page ID, the names given (not marked as custom names) and a part of its own
    /// when it is written.  Pages that use the original as their background keep it
    pub fn duplicate_page(&mut self, page: PageRef, name: &str, name_u: &str, pos: Option<usize>) -> Result<PageRef, EditError> {
        let mut copy = self.doc.page(page).ok_or(EditError::NoPage)?.clone();
        let at = self.doc.page_position(page).ok_or(EditError::NoPage)? + 1;
        let slots: Vec<u32> = copy.shapes.iter().collect();
        for s in slots {
            if let Some(d) = copy.shapes.data_mut(s) {
                d.header.unique_id = None;
            }
        }
        copy.part = None;
        let (n, nu) = (self.intern(name), self.intern(name_u));
        let len = self.doc.pages.len();
        let (slot, id) = self.doc.pages.alloc().ok_or(EditError::NoIds)?;
        copy.header.id = id;
        copy.header.name = Some(n);
        copy.header.name_u = Some(nu);
        copy.header.is_custom_name = None;
        copy.header.is_custom_name_u = None;
        let pos = pos.unwrap_or(at).min(len) as u32;
        self.run(Op::PageInsert { slot, pos, page: Box::new(copy) })?;
        Ok(PageRef(slot))
    }

    // ---- masters

    /// add an empty master with the next free master ID -> its handle (add shapes with
    /// add_shape(Owner::Master(..)))
    pub fn add_master(&mut self, name: &str, name_u: &str) -> Result<MasterRef, EditError> {
        let (n, nu) = (self.intern(name), self.intern(name_u));
        let pos = self.doc.masters.len() as u32;
        let (slot, id) = self.doc.masters.alloc().ok_or(EditError::NoIds)?;
        let header = MasterHeader { id, name: Some(n), name_u: Some(nu), ..MasterHeader::default() };
        self.run(Op::MasterInsert { slot, pos, master: Box::new(Master::new(header)) })?;
        Ok(MasterRef(slot))
    }

    /// delete a master that no shape uses (InUse otherwise)
    pub fn delete_master(&mut self, master: MasterRef) -> Result<(), EditError> {
        let id = self.doc.master(master).ok_or(EditError::NoMaster)?.header.id;
        if self.doc.master_in_use(id) {
            return Err(EditError::InUse);
        }
        self.run(Op::MasterRemove { slot: master.0 })
    }

    /// rename a master (Name and NameU)
    pub fn rename_master(&mut self, master: MasterRef, name: &str, name_u: &str) -> Result<(), EditError> {
        let mut h = self.doc.master(master).ok_or(EditError::NoMaster)?.header.clone();
        h.name = Some(self.intern(name));
        h.name_u = Some(self.intern(name_u));
        self.run(Op::MasterHeader { master, header: Box::new(h) })
    }

    /// set the master's attributes as a whole
    pub fn set_master_header(&mut self, master: MasterRef, header: MasterHeader) -> Result<(), EditError> {
        self.doc.master(master).ok_or(EditError::NoMaster)?;
        if self.doc.master_by_id(header.id).is_some_and(|m| m != master) {
            return Err(EditError::Exists);
        }
        self.run(Op::MasterHeader { master, header: Box::new(header) })
    }

    // ---- styles

    /// add a style with the next free style ID, based on `based_on` -> its handle
    pub fn add_style(&mut self, name: &str, name_u: &str, based_on: StyleRefs) -> Result<StyleRef, EditError> {
        for id in [based_on.line, based_on.fill, based_on.text].into_iter().flatten() {
            self.doc.style_by_id(id).ok_or(EditError::NoStyle)?;
        }
        let (n, nu) = (self.intern(name), self.intern(name_u));
        let pos = self.doc.styles.len() as u32;
        let (slot, id) = self.doc.styles.alloc().ok_or(EditError::NoIds)?;
        let header = StyleHeader { id, name: Some(n), name_u: Some(nu), based_on, ..StyleHeader::default() };
        let style = Style { header, ..Style::default() };
        self.run(Op::StyleInsert { slot, pos, style: Box::new(style) })?;
        Ok(StyleRef(slot))
    }

    /// delete a style nothing refers to (InUse otherwise)
    pub fn delete_style(&mut self, style: StyleRef) -> Result<(), EditError> {
        let id = self.doc.style(style).ok_or(EditError::NoStyle)?.header.id;
        if self.doc.style_in_use(id) {
            return Err(EditError::InUse);
        }
        self.run(Op::StyleRemove { slot: style.0 })
    }

    /// rename a style (Name and NameU)
    pub fn rename_style(&mut self, style: StyleRef, name: &str, name_u: &str) -> Result<(), EditError> {
        let mut h = self.doc.style(style).ok_or(EditError::NoStyle)?.header.clone();
        h.name = Some(self.intern(name));
        h.name_u = Some(self.intern(name_u));
        self.run(Op::StyleHeader { style, header: Box::new(h) })
    }

    /// set the styles a style is based on (Cycle when a chain would lead back to it)
    pub fn set_based_on(&mut self, style: StyleRef, based_on: StyleRefs) -> Result<(), EditError> {
        let mut h = self.doc.style(style).ok_or(EditError::NoStyle)?.header.clone();
        let me = h.id;
        for cat in [Category::Line, Category::Fill, Category::Text] {
            let mut cur = based_on.get(cat);
            for _ in 0..=self.doc.styles.len() {
                match cur {
                    Some(id) if id == me => return Err(EditError::Cycle),
                    Some(id) => {
                        let s = self.doc.style_by_id(id).and_then(|s| self.doc.style(s)).ok_or(EditError::NoStyle)?;
                        cur = s.header.based_on.get(cat);
                    }
                    None => break,
                }
            }
        }
        h.based_on = based_on;
        self.run(Op::StyleHeader { style, header: Box::new(h) })
    }

    // ---- document

    /// change the document settings (snap, glue, protection, default styles ...)
    pub fn update_settings(&mut self, f: impl FnOnce(&mut DocSettings)) -> Result<(), EditError> {
        let mut s = self.doc.settings.clone();
        f(&mut s);
        if s == self.doc.settings {
            return Ok(());
        }
        self.run(Op::Settings { settings: Box::new(s) })
    }

    /// replace the document's theme parts (JD11: a design applied brings its part); pages
    /// keep their Page::theme indices, so a caller that removes a part fixes those first
    pub fn set_themes(&mut self, themes: Vec<Theme>) -> Result<(), EditError> {
        if themes == self.doc.themes {
            return Ok(());
        }
        self.run(Op::Themes { themes: Box::new(themes) })
    }
}

// ---- rows in the middle of a section (JD7: vertices of a shape, SP-05)
//
// Indexed rows are numbered without gaps from 1 (Geometry) or 0 (every other indexed
// section), and formulas name them by that number (Geometry1.X3 is row IX 3, Connections.X1
// row IX 0).  So a row put in or taken out in the middle renumbers the rows after it, and the
// sheet's formulas follow: their own cells, and the formulas they inherit (those get a cell of
// the sheet's own with the new text).  A reference to a row taken out becomes the value it had
// (Visio would write #REF!; a value keeps the shape drawable).  A section the sheet inherits
// from its master shape is made the sheet's own first: every row it really has, with every
// cell as the sheet sees it (formula and value), and Del="1" marks for the master's rows it no
// longer has -- Visio does the same when the vertices of an instance are edited, since rows
// are matched with the master's by IX and a renumbered row would otherwise mix its cells with
// another master row's.  Formulas of the master that name other shapes by Sheet.N (mapped to
// the instance's subshapes while inherited) are copied as written.  Changed formulas keep their
// old values (stale) until the engine recomputes the sheet (Engine::recalc with it; ops does).
impl Transaction<'_> {
    /// insert a row of the sheet's own into an indexed section at `at` (an Index; past the
    /// last row: after it): the rows from there on move one down (IX + 1) and the references
    /// to them in the sheet's formulas follow; an inherited section is made the sheet's own
    /// first (see above) -> the key of the new row.  `kind` is the row type (Geometry:
    /// "LineTo" ...)
    pub fn insert_row(&mut self, sheet: SheetRef, sk: SecKey, at: RowKey, kind: Option<&str>) -> Result<RowKey, EditError> {
        let RowKey::Index(want) = at else { return Err(EditError::Invalid) };
        if names::named_rows(self.doc.str(sk.name)) {
            return Err(EditError::Invalid);
        }
        self.doc.sheet(sheet).ok_or(EditError::NoSheet)?;
        let (rows, parent_keys) = self.full_rows(sheet, sk);
        let first = if sk.name == names::GEOMETRY { 1 } else { 0 };
        let next = rows
            .iter()
            .filter_map(|r| match r.key {
                RowKey::Index(i) => Some(i + 1),
                RowKey::Named(_) => None,
            })
            .max()
            .unwrap_or(first);
        let ix = want.max(first).min(next);
        let texts = self.renumbered(sheet, sk, &|x| Some(if x >= ix { x + 1 } else { x }));
        let kind = kind.map(|k| self.intern(k));
        let mut new_rows: Vec<Row> = rows
            .into_iter()
            .map(|mut r| {
                if let RowKey::Index(x) = r.key {
                    if x >= ix {
                        r.key = RowKey::Index(x + 1);
                    }
                }
                r
            })
            .collect();
        let mut row = Row::new(RowKey::Index(ix));
        row.kind = kind;
        new_rows.push(row);
        self.atomic(|tx| {
            tx.replace_rows(sheet, sk, new_rows, &parent_keys)?;
            for (addr, text) in &texts {
                tx.force_formula(sheet, addr, text)?;
            }
            Ok(RowKey::Index(ix))
        })
    }

    /// take a row out of an indexed section: the rows after it move one up (IX - 1), the
    /// references to them follow and references to the row itself become its values; an
    /// inherited section is made the sheet's own first
    pub fn remove_row(&mut self, sheet: SheetRef, sk: SecKey, key: RowKey) -> Result<(), EditError> {
        let RowKey::Index(k) = key else { return Err(EditError::Invalid) };
        if names::named_rows(self.doc.str(sk.name)) {
            return Err(EditError::Invalid);
        }
        if !self.doc.has_row(sheet, sk, key) {
            return Err(EditError::NoRow);
        }
        let (rows, parent_keys) = self.full_rows(sheet, sk);
        let texts = self.renumbered(sheet, sk, &|x| match x {
            x if x == k => None,
            x if x > k => Some(x - 1),
            x => Some(x),
        });
        let new_rows: Vec<Row> = rows
            .into_iter()
            .filter(|r| r.key != key)
            .map(|mut r| {
                if let RowKey::Index(x) = r.key {
                    if x > k {
                        r.key = RowKey::Index(x - 1);
                    }
                }
                r
            })
            .collect();
        self.atomic(|tx| {
            tx.replace_rows(sheet, sk, new_rows, &parent_keys)?;
            for (addr, text) in &texts {
                tx.force_formula(sheet, addr, text)?;
            }
            Ok(())
        })
    }

    /// change the type (T) of a row the sheet has (Geometry: "MoveTo", "LineTo" ...); an
    /// inherited row gets an entry of the sheet's own that names the type
    pub fn set_row_kind(&mut self, sheet: SheetRef, sk: SecKey, key: RowKey, kind: &str) -> Result<(), EditError> {
        let eff = self.doc.rows(sheet, sk).into_iter().find(|r| r.key == key).ok_or(EditError::NoRow)?;
        let k = self.intern(kind);
        if eff.kind == Some(k) {
            return Ok(());
        }
        self.atomic(|tx| {
            let sp = tx.ensure_section(sheet, sk, true)?;
            let sec = &tx.doc.sheet(sheet).ok_or(EditError::NoSheet)?.sections[sp as usize];
            match sec.row_pos(key) {
                Some(p) => {
                    let mut row = sec.rows[p].clone();
                    row.kind = Some(k);
                    tx.run(Op::RowRemove { sheet, sec: sp, pos: p as u32 })?;
                    tx.run(Op::RowInsert { sheet, sec: sp, pos: p as u32, row: Box::new(row) })
                }
                None => {
                    let pos = sec.insert_pos(key) as u32;
                    let mut row = Row::new(key);
                    row.kind = Some(k);
                    tx.run(Op::RowInsert { sheet, sec: sp, pos, row: Box::new(row) })
                }
            }
        })
    }

    // the rows section `sk` really has for `sheet` as rows of its own (a row the parent passes
    // on: with every cell as the sheet sees it), sorted by key, and the keys of the rows the
    // parent sheet passes on
    fn full_rows(&self, sheet: SheetRef, sk: SecKey) -> (Vec<Row>, Vec<RowKey>) {
        let cat = Category::of_section(self.doc.str(sk.name));
        let parent = self.doc.parent_sheet(sheet, cat);
        let parent_keys: Vec<RowKey> = parent.map(|p| self.doc.rows(p, sk).iter().map(|r| r.key).collect()).unwrap_or_default();
        let own = self.doc.sheet(sheet).and_then(|s| s.section(sk));
        let mut out = Vec::new();
        for e in self.doc.rows(sheet, sk) {
            let mut row = own.and_then(|s| s.row(e.key)).cloned().unwrap_or_else(|| Row::new(e.key));
            row.del = false;
            row.kind = e.kind;
            row.ix_written = true;
            if parent_keys.contains(&e.key) {
                // every cell the way up names for this row, as the sheet sees it
                let mut cells: Vec<Sym> = row.cells.iter().map(|c| c.name).collect();
                let mut cur = parent;
                let mut depth = 0;
                while let (Some(p), true) = (cur, depth < 64) {
                    if let Some(r) = self.doc.sheet(p).and_then(|s| s.section(sk)).and_then(|s| s.row(e.key)) {
                        for c in &r.cells {
                            if !cells.contains(&c.name) {
                                cells.push(c.name);
                            }
                        }
                    }
                    cur = self.doc.parent_sheet(p, cat);
                    depth += 1;
                }
                row.cells = cells
                    .into_iter()
                    .filter_map(|n| {
                        let r = self.doc.resolve(sheet, &CellAddr::row(sk, e.key, n));
                        r.exists().then(|| Cell { name: n, value: r.value(), formula: r.formula(), unit: r.unit(), error: r.error(), stale: false })
                    })
                    .collect();
            }
            out.push(row);
        }
        out.sort_by_key(|r| r.key);
        (out, parent_keys)
    }

    // put `rows` in place of the sheet's own rows of section `sk`, with Del="1" marks for the
    // parent's rows they do not cover
    fn replace_rows(&mut self, sheet: SheetRef, sk: SecKey, rows: Vec<Row>, parent_keys: &[RowKey]) -> Result<(), EditError> {
        let sp = self.ensure_section(sheet, sk, false)?;
        let n = self.doc.sheet(sheet).ok_or(EditError::NoSheet)?.sections[sp as usize].rows.len();
        for pos in (0..n).rev() {
            self.run(Op::RowRemove { sheet, sec: sp, pos: pos as u32 })?;
        }
        let keys: BTreeSet<RowKey> = rows.iter().map(|r| r.key).collect();
        let mut all = rows;
        for &k in parent_keys {
            if !keys.contains(&k) {
                let mut mark = Row::new(k);
                mark.del = true;
                all.push(mark);
            }
        }
        all.sort_by_key(|r| r.key);
        for (i, row) in all.into_iter().enumerate() {
            self.run(Op::RowInsert { sheet, sec: sp, pos: i as u32, row: Box::new(row) })?;
        }
        Ok(())
    }

    // the formulas of `sheet` (own and inherited) whose references to rows of `sk` change when
    // the rows are renumbered by `map` (None: the row goes): the address each will have
    // afterwards and its new text
    fn renumbered(&self, sheet: SheetRef, sk: SecKey, map: &dyn Fn(u32) -> Option<u32>) -> Vec<(CellAddr, String)> {
        let doc = &*self.doc;
        let Some((prefix, form)) = crate::ops::text::row_form(doc.str(sk.name), sk.index) else { return Vec::new() };
        let own = match sheet {
            SheetRef::Shape(r) => doc.shape(r).map(|s| s.id()),
            _ => None,
        };
        let mut out = Vec::new();
        for key in crate::formula::sheet_formula_cells(doc, sheet) {
            let addr = key.addr();
            let Formula::Expr(f) = doc.resolve(sheet, &addr).formula() else { continue };
            let text = doc.str(f);
            let mut gone = |cell: &str, ix: u32| -> String {
                let v = doc.sym(cell).and_then(|c| doc.resolve(sheet, &CellAddr::row(sk, RowKey::Index(ix), c)).num());
                match v {
                    Some(x) => crate::ops::text::num(x),
                    None => String::from("#REF!"),
                }
            };
            let new = crate::ops::text::renumber_rows(text, own, &prefix, form, map, &mut gone);
            if new == text {
                continue;
            }
            let new_addr = match (addr.section, addr.row) {
                (Some(s), Some(RowKey::Index(x))) if s == sk => match map(x) {
                    Some(nx) => CellAddr::row(sk, RowKey::Index(nx), addr.cell),
                    None => continue,
                },
                _ => addr,
            };
            out.push((new_addr, new));
        }
        out
    }
}
