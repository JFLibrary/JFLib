// themed -- the values cells store as "Themed", worked out by the formula engine (JD11)
//
// Visio stores V="Themed" where a cell's result is the plain THEMEVAL() (the style "Theme"
// and what inherits from it): the value depends on the shape's QuickStyle cells and its
// page's design, and every reader works it out again.  The builder asks the engine for such
// a cell in the context of the sheet being drawn (Engine::evaluate: the cell's effective
// formula, inputs as stored), so the drawing shows exactly what THEMEVAL means -- the style
// lists with their colour transforms, the variant, Visio's line ends and fill patterns -- and
// a design applied (design::apply) shows at once.  One engine and one cache per build.

use alloc::collections::BTreeMap;
use alloc::string::String;
use core::cell::RefCell;

use crate::document::{Document, SheetRef};
use crate::formula::{Color, Engine, NodeKey, Outcome, Val};
use crate::sheet::CellAddr;

/// a themed value as the builder uses it
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Tv {
    Num(f64),
    Rgb(u32),
    Index(u32),
    Text(String),
}

#[derive(Default)]
pub(super) struct Themed {
    eng: RefCell<Option<Engine>>,
    cache: RefCell<BTreeMap<NodeKey, Option<Tv>>>,
}

fn convert(v: &Val) -> Option<Tv> {
    match v {
        Val::Themed(inner) => convert(inner),
        Val::Num(x) if x.is_finite() => Some(Tv::Num(*x)),
        Val::Color(Color::Rgb(r, g, b)) => Some(Tv::Rgb(((*r as u32) << 16) | ((*g as u32) << 8) | *b as u32)),
        Val::Color(Color::Index(i)) => Some(Tv::Index(*i)),
        Val::Str(s) | Val::Font(s) => Some(Tv::Text(String::from(&**s))),
        _ => None,
    }
}

impl Themed {
    /// the value of a "Themed" cell of a sheet
    pub(super) fn get(&self, doc: &Document, sheet: SheetRef, addr: &CellAddr) -> Option<Tv> {
        let key = NodeKey::new(sheet, addr);
        if let Some(v) = self.cache.borrow().get(&key) {
            return v.clone();
        }
        let v = {
            let mut eng = self.eng.borrow_mut();
            let eng = eng.get_or_insert_with(Engine::new);
            match eng.evaluate(doc, sheet, addr) {
                Outcome::Value(v) => convert(&v),
                Outcome::Kept(_) => None,
            }
        };
        self.cache.borrow_mut().insert(key, v.clone());
        v
    }

    pub(super) fn num(&self, doc: &Document, sheet: SheetRef, addr: &CellAddr) -> Option<f64> {
        match self.get(doc, sheet, addr)? {
            Tv::Num(x) => Some(x),
            Tv::Index(i) => Some(i as f64),
            _ => None,
        }
    }
}

/// whether a stored text is Visio's "Themed"
pub(super) fn is_themed(t: &str) -> bool {
    t.eq_ignore_ascii_case("Themed")
}
