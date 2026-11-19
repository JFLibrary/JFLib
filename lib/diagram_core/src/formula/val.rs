// val -- what a formula computes: typed values and how they meet cells (FX-02)
//
//   Val::Num(f64)              a number in internal units (inches, radians, days); TRUE is 1
//   Val::Str / Color / Point / Err / Themed / Shape / Font / Target / Missing
//   val.to_cell(cell name) -> (Value, Option<CellError>) text   what V stores for it
//   Val::from_value(value, error, strings)   a stored V read back as a Val
//
// The stored forms (as Visio writes V): numbers as numbers, an RGB colour "#rrggbb", a colour
// index as a number, a theme result "Themed", NURBS(...)/POLYLINE(...) as their text, a point
// as its X or Y depending on the cell (PNT in BeginX gives x), strings as they are.  An error
// leaves V alone and sets E.

use alloc::rc::Rc;
use alloc::string::{String, ToString};

use super::color::Color;
use crate::cell::{CellError, Value};
use crate::document::SheetRef;
use crate::sheet::CellAddr;
use crate::strings::Strings;

/// a cell or sheet as a value (GetRef, SheetRef(), SETATREF targets)
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Target {
    /// the sheet
    pub sheet: SheetRef,
    /// the cell (None: the sheet itself)
    pub addr: Option<CellAddr>,
}

/// a computed value
#[derive(Clone, PartialEq, Debug)]
pub enum Val {
    /// a number in internal units; also truth values (1, 0)
    Num(f64),
    /// a string
    Str(Rc<str>),
    /// a colour
    Color(Color),
    /// a point (PNT, PAR, LOC)
    Point(f64, f64),
    /// an error value
    Err(CellError),
    /// a value that comes from the theme: V stores "Themed", formulas use the value inside
    Themed(Rc<Val>),
    /// a NURBS(...) or POLYLINE(...) result as V writes it
    Shape(Rc<str>),
    /// a font (FONT("Calibri")): a Font cell stores the name
    Font(Rc<str>),
    /// a reference to a cell or sheet
    Target(Target),
    /// an empty argument
    Missing,
}

/// which coordinate of a point a cell takes
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Axis {
    /// X, BeginX, PinX, LocPinX, TxtPinX, XDyn ... (also the default)
    X,
    /// Y, BeginY, PinY, YDyn ...
    Y,
}

impl Axis {
    /// the axis of a cell name: Y when the name has a Y and no X
    pub fn of(cell: &str) -> Axis {
        if cell.contains('Y') && !cell.contains('X') { Axis::Y } else { Axis::X }
    }
}

// numbers as V writes them: the shortest form that reads back the same
pub(crate) fn fmt_num(x: f64) -> String {
    if x == 0.0 {
        return String::from("0");
    }
    if x == super::math::trunc(x) && x.abs() < 1e15 {
        return (x as i64).to_string();
    }
    let mut s = String::new();
    let _ = core::fmt::write(&mut s, format_args!("{x}"));
    s
}

impl Val {
    /// a truth value
    pub fn bool(b: bool) -> Val {
        Val::Num(if b { 1.0 } else { 0.0 })
    }

    /// a string value
    pub fn str(s: &str) -> Val {
        Val::Str(Rc::from(s))
    }

    /// the value without its Themed wrapper
    pub fn plain(&self) -> &Val {
        match self {
            Val::Themed(v) => v.plain(),
            v => v,
        }
    }

    /// whether it is an error value
    pub fn is_err(&self) -> bool {
        matches!(self.plain(), Val::Err(_))
    }

    /// the error, if it is one
    pub fn err(&self) -> Option<CellError> {
        match self.plain() {
            Val::Err(e) => Some(*e),
            _ => None,
        }
    }

    /// a stored value read back (E set: the error; "Themed" stays text here, the engine
    /// computes what is behind it)
    pub fn from_value(value: Value, error: Option<&str>, strings: &Strings) -> Val {
        if let Some(e) = error.and_then(CellError::parse) {
            return Val::Err(e);
        }
        match value {
            Value::Absent => Val::Num(0.0),
            Value::Num(x) => Val::Num(x),
            Value::Text(s) => {
                let t = strings.str(s);
                if let Some(c) = Color::parse(t) {
                    Val::Color(c)
                } else if t.starts_with("NURBS(") || t.starts_with("POLYLINE(") {
                    Val::Shape(Rc::from(t))
                } else {
                    Val::Str(Rc::from(t))
                }
            }
        }
    }

    /// the number of a value for arithmetic (a point gives the axis' coordinate)
    pub fn num(&self, axis: Axis) -> Result<f64, CellError> {
        match self {
            Val::Num(x) => Ok(*x),
            Val::Color(Color::Index(i)) => Ok(*i as f64),
            Val::Color(Color::Rgb(r, g, b)) => Ok(*r as f64 + 256.0 * *g as f64 + 65536.0 * *b as f64),
            Val::Point(x, y) => Ok(if axis == Axis::Y { *y } else { *x }),
            Val::Err(e) => Err(*e),
            Val::Themed(v) => v.num(axis),
            Val::Str(s) => {
                let t = s.trim();
                if t.is_empty() {
                    Ok(0.0)
                } else if t.eq_ignore_ascii_case("TRUE") {
                    Ok(1.0)
                } else if t.eq_ignore_ascii_case("FALSE") {
                    Ok(0.0)
                } else {
                    t.parse::<f64>().map_err(|_| CellError::Value)
                }
            }
            Val::Missing | Val::Target(_) => Ok(0.0),
            Val::Shape(_) | Val::Font(_) => Err(CellError::Value),
        }
    }

    /// the text of a value (numbers in their shortest form, colours as "#rrggbb")
    pub fn text(&self) -> Result<String, CellError> {
        Ok(match self {
            Val::Str(s) | Val::Shape(s) | Val::Font(s) => String::from(&**s),
            Val::Num(x) => fmt_num(*x),
            Val::Color(Color::Rgb(r, g, b)) => Color::hex(*r, *g, *b),
            Val::Color(Color::Index(i)) => i.to_string(),
            Val::Point(x, y) => {
                let mut s = fmt_num(*x);
                s.push_str(", ");
                s.push_str(&fmt_num(*y));
                s
            }
            Val::Err(e) => return Err(*e),
            Val::Themed(v) => return v.text(),
            Val::Missing | Val::Target(_) => String::new(),
        })
    }

    /// what V stores for this result in cell `cell` -> (V as text or number, error); None
    /// value: keep V as it is
    pub fn stored(&self, cell: &str) -> Stored {
        match self {
            Val::Num(x) if x.is_finite() => Stored::Num(*x),
            Val::Num(_) => Stored::Error(CellError::Num),
            Val::Str(s) => Stored::Text(String::from(&**s)),
            Val::Color(Color::Index(i)) => Stored::Num(*i as f64),
            Val::Color(Color::Rgb(r, g, b)) => Stored::Text(Color::hex(*r, *g, *b)),
            Val::Point(x, y) => Stored::Num(if Axis::of(cell) == Axis::Y { *y } else { *x }),
            Val::Err(e) => Stored::Error(*e),
            Val::Themed(_) => Stored::Text(String::from("Themed")),
            Val::Shape(s) => Stored::Text(String::from(&**s)),
            Val::Font(s) => Stored::Text(String::from(&**s)),
            Val::Target(_) | Val::Missing => Stored::Num(0.0),
        }
    }
}

/// what a result stores in V
#[derive(Clone, PartialEq, Debug)]
pub enum Stored {
    /// a number
    Num(f64),
    /// a text ("#rrggbb", "Themed", a string, NURBS(...))
    Text(String),
    /// an error: V stays, E is set
    Error(CellError),
}

impl Stored {
    /// whether a stored V (with its E) equals this result: numbers within `tol` relative
    /// (and 1e-12 absolute), texts exactly (colours in any case)
    pub fn matches(&self, value: Value, error: Option<&str>, strings: &Strings, tol: f64) -> bool {
        match self {
            Stored::Error(e) => error == Some(e.as_str()),
            Stored::Num(x) => match value {
                Value::Num(y) => close(*x, y, tol),
                Value::Text(t) => strings.str(t).parse::<f64>().is_ok_and(|y| close(*x, y, tol)),
                Value::Absent => *x == 0.0,
            },
            Stored::Text(s) => match value {
                Value::Text(t) => {
                    let t = strings.str(t);
                    t == s || (s.starts_with('#') && t.eq_ignore_ascii_case(s)) || same_shape_text(s, t, tol)
                }
                Value::Num(y) => s.parse::<f64>().is_ok_and(|x| close(x, y, tol)),
                Value::Absent => s.is_empty(),
            },
        }
    }
}

/// whether two numbers agree within `tol` relative or 1e-12 absolute
pub fn close(a: f64, b: f64, tol: f64) -> bool {
    a == b || (a - b).abs() <= 1e-12 || (a - b).abs() <= tol * a.abs().max(b.abs())
}

// NURBS(...)/POLYLINE(...) texts: the same numbers
fn same_shape_text(a: &str, b: &str, tol: f64) -> bool {
    let (Some((fa, ra)), Some((fb, rb))) = (a.split_once('('), b.split_once('(')) else { return false };
    if fa != fb || !(fa == "NURBS" || fa == "POLYLINE") {
        return false;
    }
    let mut xs = ra.trim_end_matches(')').split(',').map(|t| t.trim().parse::<f64>());
    let mut ys = rb.trim_end_matches(')').split(',').map(|t| t.trim().parse::<f64>());
    loop {
        match (xs.next(), ys.next()) {
            (None, None) => return true,
            (Some(Ok(x)), Some(Ok(y))) if close(x, y, tol) => {}
            _ => return false,
        }
    }
}
