// geom -- points and coordinate systems, NURBS and POLYLINE results, BOUND, text sizes and
// shape information (FX-07, FX-09, FX-11)
//
// A point is in the local coordinates of a shape: PNT(Sheet.5!Connections.X1, ...) is in shape
// 5's, PNT(Width, 0) in the evaluating shape's.  PAR turns it into the coordinates of the
// evaluating shape's parent (its group or the page), LOC into the evaluating shape's own.  A
// shape maps local to parent coordinates by moving its local pin to the origin, flipping,
// turning by Angle and moving to its pin:
//   parent = Pin + R(Angle) * Flip * (local - LocPin)
// and a group's subshape continues through the group up to the page.
//
// Text sizes need font metrics, which this crate does not have: the engine takes a
// TextMetrics from the app; without one an empty text is exact (the margins and one line) and
// any other text is an estimate.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use super::engine::cell_value;
use super::eval::{target_of, Ev, R};
use super::math;
use super::parse::{Func, Node};
use super::val::{fmt_num, Val};
use crate::cell::CellError;
use crate::document::{SheetRef, ShapeRef};
use crate::names;
use crate::shape::ShapeKind;
use crate::sheet::{CellAddr, RowKey, SecKey};

pub(crate) fn call(ev: &mut Ev, f: Func, a: &[u32]) -> R {
    let n = a.len();
    match f {
        Func::Pnt => {
            if n != 2 {
                return Err(CellError::Value);
            }
            let x = ev.eval(a[0])?.num(super::val::Axis::X)?;
            let y = ev.eval(a[1])?.num(super::val::Axis::Y)?;
            Ok(Val::Point(x, y))
        }
        Func::PntX | Func::PntY => {
            if n != 1 {
                return Err(CellError::Value);
            }
            match ev.eval(a[0])?.plain() {
                Val::Point(x, y) => Ok(Val::Num(if f == Func::PntX { *x } else { *y })),
                v => Ok(Val::Num(v.num(if f == Func::PntX { super::val::Axis::X } else { super::val::Axis::Y })?)),
            }
        }
        Func::Par | Func::Loc => {
            if n != 1 {
                return Err(CellError::Value);
            }
            let p = point(ev, a[0])?;
            let from = frame_of(ev, a[0]);
            let to = if f == Func::Par { parent_frame(ev, ev.sheet) } else { Some(ev.sheet) };
            let q = convert(ev, p, from, to)?;
            Ok(Val::Point(q.0, q.1))
        }
        Func::LocToLoc | Func::LocToPar => {
            if n != 3 {
                return Err(CellError::Value);
            }
            let p = point(ev, a[0])?;
            let from = target_of(ev, a[1])?.sheet;
            let to = target_of(ev, a[2])?.sheet;
            let to = if f == Func::LocToPar { parent_frame(ev, to) } else { Some(to) };
            let q = convert(ev, p, Some(from), to)?;
            Ok(Val::Point(q.0, q.1))
        }
        Func::AngleToLoc | Func::AngleToPar => {
            // an angle from one coordinate system into another: its unit vector through the
            // flips and turns of both chains ([MS-VSDX] ANGLETOLOC/ANGLETOPAR)
            if n != 1 && n != 3 {
                return Err(CellError::Value);
            }
            let angle = ev.num(a[0])?;
            let (from, to) = if n == 3 {
                (Some(target_of(ev, a[1])?.sheet), Some(target_of(ev, a[2])?.sheet))
            } else if f == Func::AngleToLoc {
                (parent_frame(ev, ev.sheet), Some(ev.sheet))
            } else {
                (Some(ev.sheet), Some(ev.sheet))
            };
            let to = if f == Func::AngleToPar { to.and_then(|t| parent_frame(ev, t)) } else { to };
            let (s, c) = math::sincos(angle);
            let o = convert(ev, (0.0, 0.0), from, to)?;
            let p = convert(ev, (c, s), from, to)?;
            Ok(Val::Num(math::atan2(p.1 - o.1, p.0 - o.0)))
        }
        Func::Nurbs | Func::Polyline => {
            let mut xs = Vec::with_capacity(n);
            for &x in a {
                xs.push(ev.num(x)?);
            }
            Ok(Val::Shape(Rc::from(shape_text_of(f == Func::Nurbs, &xs)?)))
        }
        Func::Bound => bound(ev, a),
        _ => Err(CellError::Value),
    }
}

/// the V text of NURBS(...) / POLYLINE(...) from its numbers
pub(crate) fn shape_text_of(nurbs: bool, xs: &[f64]) -> Result<String, CellError> {
    let (head, group) = if nurbs { (4, 4) } else { (2, 2) };
    if xs.len() < head || (xs.len() - head) % group != 0 {
        return Err(CellError::Value);
    }
    let mut s = String::from(if nurbs { "NURBS(" } else { "POLYLINE(" });
    for (i, x) in xs.iter().enumerate() {
        if i > 0 {
            let in_group = i > head && (i - head) % group != 0;
            s.push_str(if in_group { "," } else { ", " });
        }
        s.push_str(&fmt_num(*x));
    }
    s.push(')');
    Ok(s)
}

fn point(ev: &mut Ev, i: u32) -> Result<(f64, f64), CellError> {
    match ev.eval(i)?.plain() {
        Val::Point(x, y) => Ok((*x, *y)),
        _ => Err(CellError::Value),
    }
}

// the sheet whose coordinates a point expression is in: for PNT(...) the sheet of the first
// reference in its x argument, else the evaluating sheet
fn frame_of(ev: &mut Ev, i: u32) -> Option<SheetRef> {
    let start = match ev.node(i) {
        Node::Call(Func::Pnt, s, n, _) if n > 0 => ev.args(s, n)[0],
        _ => return Some(ev.sheet),
    };
    let mut stack = alloc::vec![start];
    while let Some(k) = stack.pop() {
        match ev.node(k) {
            Node::Ref(r) => {
                let cr = ev.p.refs[r as usize].clone();
                return ev.target(&cr).ok().map(|t| t.0);
            }
            Node::Neg(x) | Node::Pct(x) => stack.push(x),
            Node::Bin(_, x, y) => {
                stack.push(y);
                stack.push(x);
            }
            Node::Call(_, s, n, _) => {
                let args: Vec<u32> = ev.args(s, n).to_vec();
                stack.extend(args.into_iter().rev());
            }
            _ => {}
        }
    }
    Some(ev.sheet)
}

// the frame of a sheet's parent: its group, else its page (None: page coordinates)
fn parent_frame(ev: &Ev, s: SheetRef) -> Option<SheetRef> {
    match s {
        SheetRef::Shape(r) => ev.doc.shape(r).and_then(|sh| sh.parent()).map(SheetRef::Shape),
        _ => None,
    }
}

struct Xf {
    pin: (f64, f64),
    loc: (f64, f64),
    angle: f64,
    flip: (bool, bool),
}

fn num_cell(ev: &mut Ev, s: SheetRef, cell: crate::strings::Sym) -> Result<f64, CellError> {
    cell_value(ev.doc, ev.run, s, &CellAddr::single(cell)).num(super::val::Axis::X)
}

fn xf(ev: &mut Ev, r: ShapeRef) -> Result<Xf, CellError> {
    let s = SheetRef::Shape(r);
    Ok(Xf {
        pin: (num_cell(ev, s, names::PIN_X)?, num_cell(ev, s, names::PIN_Y)?),
        loc: (num_cell(ev, s, names::LOC_PIN_X)?, num_cell(ev, s, names::LOC_PIN_Y)?),
        angle: num_cell(ev, s, names::ANGLE)?,
        flip: (num_cell(ev, s, names::FLIP_X)? != 0.0, num_cell(ev, s, names::FLIP_Y)? != 0.0),
    })
}

fn to_parent(t: &Xf, p: (f64, f64)) -> (f64, f64) {
    let (mut dx, mut dy) = (p.0 - t.loc.0, p.1 - t.loc.1);
    if t.flip.0 {
        dx = -dx;
    }
    if t.flip.1 {
        dy = -dy;
    }
    let (s, c) = math::sincos(t.angle);
    (t.pin.0 + dx * c - dy * s, t.pin.1 + dx * s + dy * c)
}

fn to_local(t: &Xf, q: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = (q.0 - t.pin.0, q.1 - t.pin.1);
    let (s, c) = math::sincos(t.angle);
    let (mut x, mut y) = (dx * c + dy * s, -dx * s + dy * c);
    if t.flip.0 {
        x = -x;
    }
    if t.flip.1 {
        y = -y;
    }
    (x + t.loc.0, y + t.loc.1)
}

// the shapes from `s` up to the top of its tree (s first)
fn chain(ev: &Ev, s: Option<SheetRef>) -> Vec<ShapeRef> {
    let mut v = Vec::new();
    let mut cur = match s {
        Some(SheetRef::Shape(r)) => Some(r),
        _ => None,
    };
    while let (Some(r), true) = (cur, v.len() < 64) {
        v.push(r);
        cur = ev.doc.shape(r).and_then(|sh| sh.parent());
    }
    v
}

// a point from the local coordinates of `from` into those of `to` (None: the page)
fn convert(ev: &mut Ev, p: (f64, f64), from: Option<SheetRef>, to: Option<SheetRef>) -> Result<(f64, f64), CellError> {
    let mut q = p;
    for r in chain(ev, from) {
        let t = xf(ev, r)?;
        q = to_parent(&t, q);
    }
    for r in chain(ev, to).into_iter().rev() {
        let t = xf(ev, r)?;
        q = to_local(&t, q);
    }
    Ok(q)
}

// BOUND(value, type, ignore1, a1, b1 [, ignore2, a2, b2 ...]) ([MS-VSDX] 2.5.3.17): type 0
// keeps the value inside one of the ranges that are not ignored (else the nearest end, the
// first on a tie), type 1 outside all of them (else the nearest end of the range it is in),
// type 2 leaves it alone
fn bound(ev: &mut Ev, a: &[u32]) -> R {
    if a.len() < 5 || (a.len() - 2) % 3 != 0 {
        return Err(CellError::Value);
    }
    let v = ev.num(a[0])?;
    let kind = ev.num(a[1])?;
    if kind == 2.0 {
        return Ok(Val::Num(v));
    }
    if kind != 0.0 && kind != 1.0 {
        return Err(CellError::Value);
    }
    let mut ranges = Vec::new();
    for k in (2..a.len()).step_by(3) {
        let ignore = ev.node(a[k]) != Node::Missing && ev.truth(a[k])?;
        let (p, q) = (ev.num(a[k + 1])?, ev.num(a[k + 2])?);
        if !ignore {
            ranges.push((p.min(q), p.max(q)));
        }
    }
    if ranges.is_empty() {
        return Ok(Val::Num(v));
    }
    let inside = ranges.iter().find(|(lo, hi)| v >= *lo && v <= *hi).copied();
    let nearest = |rs: &[(f64, f64)]| {
        let mut best = v;
        let mut dist = f64::INFINITY;
        for (lo, hi) in rs {
            for b in [*lo, *hi] {
                if (b - v).abs() < dist {
                    dist = (b - v).abs();
                    best = b;
                }
            }
        }
        best
    };
    Ok(Val::Num(if kind == 0.0 {
        if inside.is_some() { v } else { nearest(&ranges) }
    } else {
        match inside {
            Some((lo, hi)) => {
                if v - lo < hi - v {
                    lo
                } else {
                    hi
                }
            }
            None => v,
        }
    }))
}

// ---- text

// the shape whose text an argument names (TheText, Sheet.5!TheText; else the evaluating one)
fn text_shape(ev: &mut Ev, i: u32) -> Option<ShapeRef> {
    let s = match ev.node(i) {
        Node::Ref(r) => {
            let cr = ev.p.refs[r as usize].clone();
            ev.target(&cr).ok()?.0
        }
        _ => ev.sheet,
    };
    match s {
        SheetRef::Shape(r) => Some(r),
        _ => None,
    }
}

/// the text of the shape an argument names
pub(crate) fn shape_text(ev: &mut Ev, i: u32) -> Result<String, CellError> {
    let Some(r) = text_shape(ev, i) else { return Ok(String::new()) };
    // the text depends on the TheText cell (recorded as a dependency)
    if let Some(the_text) = ev.doc.sym("TheText") {
        let _ = cell_value(ev.doc, ev.run, SheetRef::Shape(r), &CellAddr::single(the_text));
    }
    // the end of the last paragraph is no character of the text (a file writes "Vertrieb\n",
    // the text is "Vertrieb": the lane title the flowchart's formulas read, JD10)
    // (TX-06) fields count with the text they show: their Value cells as computed in this run
    // (recorded as dependencies), in the format of their Format cells
    let doc = ev.doc;
    let mut t = String::new();
    if let Some(text) = doc.shape(r).and_then(|s| s.text()) {
        let sheet = SheetRef::Shape(r);
        for item in &text.items {
            match item {
                crate::shape::TextItem::Chars(c) => t.push_str(c),
                crate::shape::TextItem::Field(ix) => {
                    let a = CellAddr::row(SecKey::one(crate::names::FIELD), RowKey::Index(*ix), crate::names::VALUE);
                    let unit = doc.resolve(sheet, &a).unit();
                    let v = cell_value(doc, ev.run, sheet, &a);
                    let shown = match v.plain() {
                        Val::Num(x) => crate::field::Shown::Num(*x, unit),
                        Val::Str(s) | Val::Font(s) => crate::field::Shown::Text(s),
                        _ => crate::field::Shown::Nothing,
                    };
                    t.push_str(&crate::field::format(&shown, crate::field::picture(doc, sheet, *ix), &doc.fields));
                }
                crate::shape::TextItem::Other(x) => {
                    if let Some(c) = crate::field::cached(doc.str(*x)) {
                        t.push_str(&c);
                    }
                }
                _ => {}
            }
        }
    }
    for end in ["\r\n", "\n", "\u{2029}"] {
        if t.ends_with(end) {
            t.truncate(t.len() - end.len());
            break;
        }
    }
    Ok(t)
}

fn named_num(ev: &mut Ev, s: SheetRef, addr: Option<CellAddr>, default: f64) -> f64 {
    match addr {
        Some(a) => match cell_value(ev.doc, ev.run, s, &a) {
            Val::Err(_) => default,
            v => v.num(super::val::Axis::X).unwrap_or(default),
        },
        None => default,
    }
}

/// TEXTWIDTH(TheText, [max]) and TEXTHEIGHT(TheText, width)
pub(crate) fn text_size(ev: &mut Ev, f: Func, a: &[u32]) -> R {
    if a.is_empty() || a.len() > 2 || (f == Func::TextHeight && a.len() != 2) {
        return Err(CellError::Value);
    }
    let width_arg = if a.len() == 2 { Some(ev.num(a[1])?) } else { None };
    let Some(r) = text_shape(ev, a[0]) else { return Ok(Val::Num(0.0)) };
    let text = shape_text(ev, a[0])?;
    let s = SheetRef::Shape(r);
    let doc = ev.doc;
    let cell = |n: &str| doc.sym(n).map(CellAddr::single);
    let left = named_num(ev, s, cell("LeftMargin"), 0.0);
    let right = named_num(ev, s, cell("RightMargin"), 0.0);
    let top = named_num(ev, s, cell("TopMargin"), 0.0);
    let bottom = named_num(ev, s, cell("BottomMargin"), 0.0);
    let size_addr = match (doc.sym("Size"), doc.sym("Character")) {
        (Some(c), Some(sec)) => Some(CellAddr::row(SecKey::one(sec), RowKey::Index(0), c)),
        _ => None,
    };
    let size = named_num(ev, s, size_addr, 12.0 / 72.0);
    let font = match (doc.sym("Font"), doc.sym("Character")) {
        (Some(c), Some(sec)) => match cell_value(ev.doc, ev.run, s, &CellAddr::row(SecKey::one(sec), RowKey::Index(0), c)) {
            Val::Font(f) | Val::Str(f) => String::from(&*f),
            Val::Themed(v) => v.text().unwrap_or_default(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    let inner_max = width_arg.map(|w| (w - left - right).max(0.0));
    let (w, h) = ev.run.measure(&text, &font, size, inner_max);
    Ok(Val::Num(if f == Func::TextWidth { w + left + right } else { h + top + bottom }))
}

// ---- information about shapes, pages and masters

pub(crate) fn info(ev: &mut Ev, f: Func, a: &[u32]) -> R {
    let doc = ev.doc;
    let shape = match ev.sheet {
        SheetRef::Shape(r) => doc.shape(r),
        _ => None,
    };
    let universal = match a.first() {
        Some(&x) if ev.node(x) != Node::Missing => ev.num(x)? == 750.0,
        _ => false,
    };
    let page = super::eval::page_of(ev.sheet);
    match f {
        Func::MasterName => {
            let m = match ev.sheet {
                SheetRef::Shape(r) => doc.master_of(r).and_then(|m| doc.master(m)),
                _ => None,
            };
            Ok(Val::str(&match m {
                Some(m) => {
                    let n = if universal { m.header.name_u.or(m.header.name) } else { m.header.name.or(m.header.name_u) };
                    String::from(n.map_or("", |n| doc.str(n)))
                }
                None => String::from("<no master>"),
            }))
        }
        Func::Name => match shape {
            Some(s) => Ok(Val::str(&if universal { s.name_u() } else { s.name() })),
            None => Err(CellError::Value),
        },
        Func::Id => Ok(Val::Num(shape.map_or(0.0, |s| s.id() as f64))),
        Func::Type | Func::TypeDesc => {
            let kind = shape.map(|s| s.header().kind);
            let (code, desc) = match kind {
                Some(ShapeKind::Group) => (2.0, "Group"),
                Some(ShapeKind::Shape) => (3.0, "Shape"),
                Some(ShapeKind::Foreign) => (4.0, "Foreign"),
                Some(ShapeKind::Guide) => (5.0, "Guide"),
                _ => (0.0, ""),
            };
            Ok(if f == Func::Type { Val::Num(code) } else { Val::str(desc) })
        }
        Func::Is1D => {
            let r = doc.resolve(ev.sheet, &CellAddr::single(names::BEGIN_X));
            Ok(Val::bool(r.exists()))
        }
        Func::Data1 | Func::Data2 | Func::Data3 => {
            let i = match f {
                Func::Data1 => 0,
                Func::Data2 => 1,
                _ => 2,
            };
            Ok(Val::str(shape.and_then(|s| s.data().data[i]).map_or("", |d| doc.str(d))))
        }
        Func::PageName => {
            let name = match page {
                Some(SheetRef::Page(p)) => doc.page(p).and_then(|pg| if universal { pg.header.name_u.or(pg.header.name) } else { pg.header.name.or(pg.header.name_u) }),
                Some(SheetRef::Master(m)) => doc.master(m).and_then(|ms| if universal { ms.header.name_u } else { ms.header.name.or(ms.header.name_u) }),
                _ => None,
            };
            Ok(Val::str(name.map_or("", |n| doc.str(n))))
        }
        Func::PageNumber => match page {
            Some(SheetRef::Page(p)) => Ok(Val::Num(doc.foreground_pages().position(|q| q == p).map_or(0.0, |i| (i + 1) as f64))),
            _ => Ok(Val::Num(0.0)),
        },
        Func::PageCount => Ok(Val::Num(doc.foreground_pages().count() as f64)),
        _ => Err(CellError::Value),
    }
}
