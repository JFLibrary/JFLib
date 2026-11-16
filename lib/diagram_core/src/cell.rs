// cell -- one ShapeSheet cell as the file keeps it: name, formula, cached value, unit, error
//
//   Cell::new(name)                    no value, no formula attribute
//   Cell { name, value, formula, unit, error, stale }
//   Value::Num(f64) / Text(Sym) / Absent      the cached value V (internal units)
//   Formula::Absent / Inherited / NoFormula / Expr(Sym)
//   CellError::parse("#REF!")          the error state E
//   Origin                             where a cell's formula comes from (local, master shape,
//                                      style, default) or that its row was deleted
//
// Formula and value are a pair (PLAN.md, "Fallen"):
// * Formula::Absent -- no F attribute: the value itself is the formula, a constant shown in U.
// * Formula::Expr -- F holds an expression; V is its last result.  A formula the engine (JD6)
//   cannot evaluate keeps V as it is.
// * Formula::Inherited -- F="Inh": the formula belongs to the master shape or style, V is this
//   sheet's own result of it.
// * Formula::NoFormula -- F="No Formula": explicitly without formula, V is a plain value.
// `stale` says the formula changed and V is not its result yet; the engine clears it.  V holds
// numbers as f64 and everything else as text: "Themed" (theme inheritance), "#RRGGBB" colours,
// strings, NURBS(...) and POLYLINE(...) results, "1.#INF".

use crate::strings::{Strings, Sym};
use crate::units::Unit;

/// a cached value (V), numbers in internal units
#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub enum Value {
    /// no V attribute
    #[default]
    Absent,
    /// a number (lengths in inches, angles in radians ...)
    Num(f64),
    /// anything that is not a number, as written
    Text(Sym),
}

impl Value {
    /// V as the file writes it with its unit U: a finite number parses to Num, everything else
    /// stays text -- also every value of a STR or GUID cell and of an unknown unit ("007" is a
    /// serial number there, not 7)
    pub fn parse(text: &str, unit: Option<Unit>, strings: &mut Strings) -> Value {
        let textual = matches!(unit, Some(Unit::Str | Unit::Guid | Unit::Other(_)));
        match text.parse::<f64>() {
            Ok(x) if !textual && x.is_finite() && is_plain_number(text) => Value::Num(x),
            _ => Value::Text(strings.intern(text)),
        }
    }

    /// the number, if the value is one
    pub fn num(self) -> Option<f64> {
        match self {
            Value::Num(x) => Some(x),
            _ => None,
        }
    }

    /// the text, if the value is one
    pub fn text(self) -> Option<Sym> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
}

// what Rust parses but a cell would not write as a number: "inf", "NaN", "+1", " 1"
fn is_plain_number(t: &str) -> bool {
    let b = t.as_bytes();
    !b.is_empty() && (b[0] == b'-' || b[0] == b'.' || b[0].is_ascii_digit()) && b.iter().all(|c| {
        c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E')
    })
}

/// the formula state of a cell (F)
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum Formula {
    /// no F attribute: the value is the formula (a constant)
    #[default]
    Absent,
    /// F="Inh": the formula is inherited, the value is this sheet's own
    Inherited,
    /// F="No Formula": no formula
    NoFormula,
    /// F holds an expression in the universal syntax
    Expr(Sym),
}

impl Formula {
    /// F as the file writes it ("Inh", "No Formula" or an expression)
    pub fn parse(text: &str, strings: &mut Strings) -> Formula {
        match text {
            "Inh" => Formula::Inherited,
            "No Formula" => Formula::NoFormula,
            t => Formula::Expr(strings.intern(t)),
        }
    }

    /// the expression, if F holds one
    pub fn expr(self) -> Option<Sym> {
        match self {
            Formula::Expr(s) => Some(s),
            _ => None,
        }
    }
}

/// the error state of a cell (E), after [MS-VSDX] Cell_Type
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum CellError {
    /// #DIM!: a dimension out of range
    Dim,
    /// #DIV/0!
    DivZero,
    /// #VALUE!: an operand of the wrong type
    Value,
    /// #REF!: a reference to a cell that does not exist
    Ref,
    /// #NUM!: an invalid number
    Num,
    /// #N/A: no value available
    NotAvailable,
    /// #NAME?: an unknown name
    Name,
}

impl CellError {
    /// the error of an E text (None for texts the crate does not know: keep them as Sym)
    pub fn parse(text: &str) -> Option<CellError> {
        Some(match text {
            "#DIM!" => CellError::Dim,
            "#DIV/0!" => CellError::DivZero,
            "#VALUE!" => CellError::Value,
            "#REF!" => CellError::Ref,
            "#NUM!" => CellError::Num,
            "#N/A" => CellError::NotAvailable,
            "#NAME?" => CellError::Name,
            _ => return None,
        })
    }

    /// the text E holds
    pub fn as_str(self) -> &'static str {
        match self {
            CellError::Dim => "#DIM!",
            CellError::DivZero => "#DIV/0!",
            CellError::Value => "#VALUE!",
            CellError::Ref => "#REF!",
            CellError::Num => "#NUM!",
            CellError::NotAvailable => "#N/A",
            CellError::Name => "#NAME?",
        }
    }
}

/// where a cell's formula comes from
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Origin {
    /// the sheet itself
    Local,
    /// the master shape (of an instance or of a subshape of one)
    Master,
    /// a style sheet (LineStyle, FillStyle, TextStyle and the styles they are based on)
    Style,
    /// nowhere: the cell has its built-in default
    Default,
    /// its row or section was deleted in the sheet or on the way to the master (Del="1")
    Deleted,
}

/// one cell as the file keeps it
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Cell {
    /// the cell name (N): "PinX", "X", "Value" ...
    pub name: Sym,
    /// the cached value (V) in internal units
    pub value: Value,
    /// the formula state (F)
    pub formula: Formula,
    /// the unit it is shown in (U); None: no U attribute
    pub unit: Option<Unit>,
    /// the error state (E) as written; CellError::parse types it
    pub error: Option<Sym>,
    /// the formula changed and the value is not its result yet
    pub stale: bool,
}

impl Cell {
    /// a cell without value and formula attribute
    pub fn new(name: Sym) -> Cell {
        Cell { name, value: Value::Absent, formula: Formula::Absent, unit: None, error: None, stale: false }
    }

    /// a constant: the value is the formula
    pub fn constant(name: Sym, value: Value, unit: Option<Unit>) -> Cell {
        Cell { name, value, formula: Formula::Absent, unit, error: None, stale: false }
    }

    /// whether the cell keeps its own formula (not F="Inh")
    pub fn has_own_formula(&self) -> bool {
        self.formula != Formula::Inherited
    }
}
