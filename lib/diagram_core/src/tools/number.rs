// tools::number -- Shapes nummerieren (AS-11, RA-09): numbers in the shapes' texts
//
//   Order               Selection (as given), Rows (left to right, then top to bottom), Columns
//                       (top to bottom, then left to right), Connections (along the connectors)
//   Place               Before the text, After it, Instead of it
//   Numbering           start, step, prefix, place
//   order(doc, shapes, by) -> Vec<ShapeRef>     the shapes in the order they get their numbers
//   label(prefix, n)    the text a number shows: "S-" and 3 -> "S-3"
//   number(tx, eng, shapes, &numbering) -> next   the numbers written (one shape after the
//                       other, step by step) -> the number the next shape would get
//   set_number(tx, eng, s, text, place)     one shape's number (an earlier one replaced)
//   remove(tx, eng, s)  the number taken out of the text again
//   on_drop(doc, page) / keep_on_drop(tx, page, Some(&numbering) | None) / dropped(tx, eng, s)
//                       a page that numbers shapes as they are dropped: the settings and the next
//                       number in User cells of the page sheet; dropped() gives a dropped shape the
//                       next number and counts on
//
// Rules (Visio's add-on is a program; these are chosen and named here):
// * The number is plain text: before the text it is followed by a space ("3 Prüfen"), after it it
//   follows one ("Prüfen 3"), instead of it it is the whole text.  The characters' formatting
//   stays: the number goes into the first (last) run of characters.
// * User.JFD_Number keeps the number text written and User.JFD_NumberPlace where (0 before, 1
//   after, 2 instead): numbering again takes the old number out first, so a shape never carries
//   two.  A text the user changed so that the old number is gone is taken as it is.
// * Rows: the shapes are sorted by their middles from top to bottom; a shape whose middle lies
//   within the height of the first shape of the current row joins that row, else it starts the
//   next; each row runs from left to right.  Columns alike with the axes swapped.
// * Connections: the shapes in the order of the flow -- a shape gets its number once every shape
//   with a connector to it (Begin -> End) has one; among the shapes that are ready the one first in
//   rows comes first; in a loop the first shape in rows that is left goes on.  Shapes without
//   connectors are ready from the start.
// * A shape whose text is locked (LockTextEdit) is left out and takes no number.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::bbox;
use crate::cell::Value;
use crate::document::{Document, Owner, PageRef, ShapeRef, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::formula::Engine;
use crate::shape::{Text, TextItem};
use crate::units::Unit;
use crate::{glue, ops, structure};

/// User cell: the number text a shape got (prefix and number)
pub const NUMBER_CELL: &str = "JFD_Number";
/// User cell: where it went (Place::code)
pub const PLACE_CELL: &str = "JFD_NumberPlace";
/// User cells of the page sheet while the page numbers shapes as they are dropped: the next
/// number, the step, the prefix, the place
pub const NEXT_CELL: &str = "JFD_NumberNext";
pub const STEP_CELL: &str = "JFD_NumberStep";
pub const PREFIX_CELL: &str = "JFD_NumberPrefix";
pub const DROP_PLACE_CELL: &str = "JFD_NumberDropPlace";

/// the order the shapes get their numbers in
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Order {
    /// as given (the order the user selected them in)
    Selection,
    /// left to right, then top to bottom
    Rows,
    /// top to bottom, then left to right
    Columns,
    /// along the connectors (Begin -> End)
    Connections,
}

/// where the number goes in the text
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Place {
    /// in front of the text, a space between
    Before,
    /// after the text, a space between
    After,
    /// the number is the whole text
    Instead,
}

impl Place {
    /// the code of User.JFD_NumberPlace
    pub fn code(self) -> u8 {
        match self {
            Place::Before => 0,
            Place::After => 1,
            Place::Instead => 2,
        }
    }

    /// the place of a code (anything else: Before)
    pub fn from_code(c: f64) -> Place {
        match c as i64 {
            1 => Place::After,
            2 => Place::Instead,
            _ => Place::Before,
        }
    }
}

/// how shapes are numbered
#[derive(Clone, PartialEq, Debug)]
pub struct Numbering {
    /// the first number
    pub start: i64,
    /// added for every next shape (may be negative)
    pub step: i64,
    /// the text before the number ("S-", "Schritt ")
    pub prefix: String,
    pub place: Place,
}

impl Default for Numbering {
    fn default() -> Numbering {
        Numbering { start: 1, step: 1, prefix: String::new(), place: Place::Before }
    }
}

/// the text a number shows
pub fn label(prefix: &str, n: i64) -> String {
    format!("{prefix}{n}")
}

// ---------------------------------------------------------------- the order

/// the shapes in the order they get their numbers
pub fn order(doc: &Document, shapes: &[ShapeRef], by: Order) -> Vec<ShapeRef> {
    match by {
        Order::Selection => shapes.to_vec(),
        Order::Rows => lines(doc, shapes, false),
        Order::Columns => lines(doc, shapes, true),
        Order::Connections => flow(doc, shapes),
    }
}

// rows (or columns): see the file header
fn lines(doc: &Document, shapes: &[ShapeRef], columns: bool) -> Vec<ShapeRef> {
    let mut items: Vec<(usize, ShapeRef, [f64; 4])> = shapes.iter().enumerate().map(|(i, &s)| (i, s, bbox(doc, s))).collect();
    // across: from the top (rows) or from the left (columns); along: from the left or the top
    let across = |b: &[f64; 4]| if columns { (b[0] + b[2]) * 0.5 } else { -(b[1] + b[3]) * 0.5 };
    let along = |b: &[f64; 4]| if columns { -(b[1] + b[3]) * 0.5 } else { (b[0] + b[2]) * 0.5 };
    let span = |b: &[f64; 4]| if columns { [b[0], b[2]] } else { [b[1], b[3]] };
    let mid = |b: &[f64; 4]| if columns { (b[0] + b[2]) * 0.5 } else { (b[1] + b[3]) * 0.5 };
    items.sort_by(|a, b| across(&a.2).partial_cmp(&across(&b.2)).unwrap_or(core::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
    let mut out = Vec::with_capacity(items.len());
    let mut row: Vec<(usize, ShapeRef, [f64; 4])> = Vec::new();
    let mut band = [0.0, 0.0];
    let flush = |row: &mut Vec<(usize, ShapeRef, [f64; 4])>, out: &mut Vec<ShapeRef>| {
        row.sort_by(|a, b| along(&a.2).partial_cmp(&along(&b.2)).unwrap_or(core::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
        out.extend(row.drain(..).map(|x| x.1));
    };
    for it in items {
        let m = mid(&it.2);
        if row.is_empty() {
            band = span(&it.2);
        } else if m < band[0] - 1e-9 || m > band[1] + 1e-9 {
            flush(&mut row, &mut out);
            band = span(&it.2);
        }
        row.push(it);
    }
    flush(&mut row, &mut out);
    out
}

// the top-level shape a glued end stands for
fn top(doc: &Document, s: ShapeRef) -> ShapeRef {
    let mut cur = s;
    for _ in 0..64 {
        match doc.shape(cur).and_then(|x| x.parent()) {
            Some(p) => cur = p,
            None => break,
        }
    }
    cur
}

// along the connectors: see the file header
fn flow(doc: &Document, shapes: &[ShapeRef]) -> Vec<ShapeRef> {
    let ranked = lines(doc, shapes, false);
    let rank: BTreeMap<ShapeRef, usize> = ranked.iter().enumerate().map(|(i, &s)| (s, i)).collect();
    let n = ranked.len();
    let mut next: Vec<BTreeSet<usize>> = alloc::vec![BTreeSet::new(); n];
    let owners: BTreeSet<Owner> = shapes.iter().map(|s| s.owner).collect();
    for owner in owners {
        for c in doc.top_shapes(owner) {
            if doc.shape(c).is_none_or(|x| x.is_deleted()) || !glue::is_one_d(doc, c) {
                continue;
            }
            let end = |t: glue::Target| match t {
                glue::Target::Point { shape, .. } | glue::Target::Shape(shape) => rank.get(&top(doc, shape)).copied(),
                glue::Target::None => None,
            };
            let [a, b] = glue::glued(doc, c);
            if let (Some(a), Some(b)) = (end(a), end(b)) {
                if a != b {
                    next[a].insert(b);
                }
            }
        }
    }
    let mut indeg = alloc::vec![0usize; n];
    for set in &next {
        for &b in set {
            indeg[b] += 1;
        }
    }
    let mut done = alloc::vec![false; n];
    let mut ready: BTreeSet<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut out = Vec::with_capacity(n);
    while out.len() < n {
        // a loop: the first shape in rows that is left goes on
        let i = match ready.iter().next().copied() {
            Some(i) => i,
            None => match (0..n).find(|&i| !done[i]) {
                Some(i) => i,
                None => break,
            },
        };
        ready.remove(&i);
        if done[i] {
            continue;
        }
        done[i] = true;
        out.push(ranked[i]);
        for &b in &next[i] {
            indeg[b] = indeg[b].saturating_sub(1);
            if indeg[b] == 0 && !done[b] {
                ready.insert(b);
            }
        }
    }
    out
}

// ---------------------------------------------------------------- the text

// the shape's text without the number it got earlier
fn without(text: &Text, old: Option<&str>, place: Place) -> Text {
    let Some(old) = old.filter(|o| !o.is_empty()) else { return text.clone() };
    let mut t = text.clone();
    if place == Place::Instead {
        if t.chars() == old {
            return Text { items: Vec::new(), extras: t.extras };
        }
        return t;
    }
    let pos = if place == Place::Before {
        t.items.iter().position(|i| matches!(i, TextItem::Chars(_)))
    } else {
        t.items.iter().rposition(|i| matches!(i, TextItem::Chars(_)))
    };
    let Some(k) = pos else { return t };
    let TextItem::Chars(c) = &t.items[k] else { return t };
    let s: &str = c;
    let rest = if place == Place::Before {
        s.strip_prefix(old).and_then(|r| if r.is_empty() { Some(String::from(r)) } else { r.strip_prefix(' ').map(String::from) })
    } else {
        // before a closing line break
        let (body, tail) = match s.strip_suffix('\n') {
            Some(b) => (b, "\n"),
            None => (s, ""),
        };
        body.strip_suffix(old).and_then(|r| if r.is_empty() { Some(r) } else { r.strip_suffix(' ') }).map(|r| format!("{r}{tail}"))
    };
    if let Some(r) = rest {
        if r.is_empty() {
            t.items.remove(k);
        } else {
            t.items[k] = TextItem::Chars(r.into_boxed_str());
        }
    }
    t
}

// the text with the number put in
fn with(text: Text, num: &str, place: Place) -> Text {
    let mut t = text;
    let empty = t.chars().trim().is_empty() && !t.items.iter().any(|i| matches!(i, TextItem::Field(_)));
    if place == Place::Instead || empty {
        // the paragraph and character marks in front stay
        let lead: Vec<TextItem> = t.items.iter().take_while(|i| matches!(i, TextItem::Cp(_) | TextItem::Pp(_) | TextItem::Tp(_))).cloned().collect();
        let mut items = lead;
        items.push(TextItem::Chars(num.into()));
        return Text { items, extras: t.extras };
    }
    let content = |i: &TextItem| matches!(i, TextItem::Chars(_) | TextItem::Field(_));
    match place {
        Place::Before => {
            let k = t.items.iter().position(content).unwrap_or(t.items.len());
            match t.items.get(k) {
                Some(TextItem::Chars(c)) => {
                    let s = format!("{num} {c}");
                    t.items[k] = TextItem::Chars(s.into_boxed_str());
                }
                _ => t.items.insert(k, TextItem::Chars(format!("{num} ").into_boxed_str())),
            }
        }
        _ => {
            let k = t.items.iter().rposition(content);
            match k.map(|k| (k, &t.items[k])) {
                Some((k, TextItem::Chars(c))) => {
                    // before a closing line break, if the text ends with one
                    let s: &str = c;
                    let (body, tail) = match s.strip_suffix('\n') {
                        Some(b) => (b, "\n"),
                        None => (s, ""),
                    };
                    let s = format!("{body} {num}{tail}");
                    t.items[k] = TextItem::Chars(s.into_boxed_str());
                }
                Some((k, _)) => t.items.insert(k + 1, TextItem::Chars(format!(" {num}").into_boxed_str())),
                None => t.items.push(TextItem::Chars(format!(" {num}").into_boxed_str())),
            }
        }
    }
    t
}

fn user_sym(tx: &mut Transaction, sheet: SheetRef, name: &str, text: &str) -> Result<(), EditError> {
    let v = tx.intern(text);
    tx.set_user(sheet, name, Value::Text(v), Some(Unit::Str))
}

/// give shape `s` the number text `num` at `place`, an earlier number taken out first
pub fn set_number(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef, num: &str, place: Place) -> Result<(), EditError> {
    let (text, old, old_place) = {
        let doc = tx.doc();
        let sh = doc.shape(s).ok_or(EditError::NoShape)?;
        let text = sh.text().cloned().unwrap_or_default();
        let old = structure::user_text(doc, s, NUMBER_CELL);
        let old_place = structure::user_num(doc, s, PLACE_CELL).map_or(Place::Before, Place::from_code);
        (text, old, old_place)
    };
    let base = without(&text, old.as_deref(), old_place);
    ops::set_text(tx, eng, s, with(base, num, place))?;
    let sheet = SheetRef::Shape(s);
    user_sym(tx, sheet, NUMBER_CELL, num)?;
    tx.set_user(sheet, PLACE_CELL, Value::Num(place.code() as f64), None)
}

/// take a shape's number out of its text again (the User cells stay, empty)
pub fn remove(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef) -> Result<(), EditError> {
    let (text, old, place) = {
        let doc = tx.doc();
        let sh = doc.shape(s).ok_or(EditError::NoShape)?;
        let old = structure::user_text(doc, s, NUMBER_CELL);
        let place = structure::user_num(doc, s, PLACE_CELL).map_or(Place::Before, Place::from_code);
        (sh.text().cloned().unwrap_or_default(), old, place)
    };
    if old.as_deref().is_none_or(str::is_empty) {
        return Ok(());
    }
    ops::set_text(tx, eng, s, without(&text, old.as_deref(), place))?;
    user_sym(tx, SheetRef::Shape(s), NUMBER_CELL, "")
}

/// number `shapes` in the order given -> the number the next shape would get; shapes whose
/// text is locked are left out
pub fn number(tx: &mut Transaction, eng: &mut Engine, shapes: &[ShapeRef], n: &Numbering) -> Result<i64, EditError> {
    let mut k = n.start;
    for &s in shapes {
        match set_number(tx, eng, s, &label(&n.prefix, k), n.place) {
            Ok(()) => k += n.step,
            // LockTextEdit: no number here
            Err(EditError::Invalid) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(k)
}

// ---------------------------------------------------------------- numbering on drop

fn page_user(doc: &Document, page: PageRef, name: &str) -> Option<Value> {
    doc.user_cells(SheetRef::Page(page)).into_iter().find(|r| r.name().is_some_and(|n| n.eq_ignore_ascii_case(name))).map(|r| r.get("Value").value())
}

/// the page numbers shapes as they are dropped: its numbering (start = the next number)
pub fn on_drop(doc: &Document, page: PageRef) -> Option<Numbering> {
    let next = match page_user(doc, page, NEXT_CELL)? {
        Value::Num(x) if x.is_finite() => x as i64,
        _ => return None,
    };
    let step = match page_user(doc, page, STEP_CELL) {
        Some(Value::Num(x)) if x.is_finite() => x as i64,
        _ => 1,
    };
    let prefix = match page_user(doc, page, PREFIX_CELL) {
        Some(Value::Text(t)) => String::from(doc.str(t)),
        _ => String::new(),
    };
    let place = match page_user(doc, page, DROP_PLACE_CELL) {
        Some(Value::Num(x)) => Place::from_code(x),
        _ => Place::Before,
    };
    Some(Numbering { start: next, step, prefix, place })
}

/// the page numbers dropped shapes from `n.start` on (None: no more); the page sheet's User cells
pub fn keep_on_drop(tx: &mut Transaction, page: PageRef, n: Option<&Numbering>) -> Result<(), EditError> {
    let sheet = SheetRef::Page(page);
    match n {
        Some(n) => {
            tx.set_user(sheet, NEXT_CELL, Value::Num(n.start as f64), None)?;
            tx.set_user(sheet, STEP_CELL, Value::Num(n.step as f64), None)?;
            user_sym(tx, sheet, PREFIX_CELL, &n.prefix)?;
            tx.set_user(sheet, DROP_PLACE_CELL, Value::Num(n.place.code() as f64), None)
        }
        // off: the next number is no number (the row stays, Visio keeps it as it is)
        None if page_user(tx.doc(), page, NEXT_CELL).is_some() => user_sym(tx, sheet, NEXT_CELL, ""),
        None => Ok(()),
    }
}

/// a shape dropped on a page that numbers dropped shapes gets the next number, the page counts
/// on -> whether it got one
pub fn dropped(tx: &mut Transaction, eng: &mut Engine, s: ShapeRef) -> Result<bool, EditError> {
    let Owner::Page(page) = s.owner else { return Ok(false) };
    let Some(n) = on_drop(tx.doc(), page) else { return Ok(false) };
    if !super::plain(tx.doc(), s) {
        return Ok(false);
    }
    match set_number(tx, eng, s, &label(&n.prefix, n.start), n.place) {
        Ok(()) => {}
        Err(EditError::Invalid) => return Ok(false),
        Err(e) => return Err(e),
    }
    tx.set_user(SheetRef::Page(page), NEXT_CELL, Value::Num((n.start + n.step) as f64), None)?;
    Ok(true)
}
