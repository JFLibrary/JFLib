// formula -- the formula engine: parser, evaluation, functions, recalculation (JD6, FX-01..15)
//
//   formula::parse("Width*0.5") -> Parsed            the universal syntax (parse.rs)
//   Engine::new() / with_metrics(..)                 one per document (engine.rs)
//   eng.calculate(&doc) -> Results                   every formula; results.write(&mut tx)
//   eng.recalc(&mut tx, &sheets) / recalc_cells      after edits, results into the transaction
//   eng.refresh(&doc, &sheets)                       after undo/redo
//   eng.user_set(&mut tx, sheet, addr, Input)        a user's change (GUARD, SETATREF)
//   eng.run_action(&mut tx, sheet, addr) -> Effects  actions and events (SETF, DOCMD ...)
//   eng.evaluate / evaluate_text                     one cell or a formula text, stored inputs
//   eng.guarded / theme_guarded / locks              GUARD, THEMEGUARD and the Lock cells
//   Val, Stored, Outcome, Keep, Effect, TextMetrics
//   the structure functions (FX-12: CONTAINERSHEETREF, LISTORDER ...) read the Relationships
//   cells of structure::rel (sfuncs.rs)
//
// The engine follows the file's own rules (decision V1): a cell's formula runs in the context
// of the sheet that has the cell -- also when the formula is inherited from a master shape or a
// style -- and its result is stored next to the formula (Transaction::set_result; an inherited
// formula gets an F="Inh" cell when its result differs from the inherited value).  Opening a
// file computes nothing.  A formula the engine cannot compute keeps its stored value and is
// reported, never turned into an error.
//
// Not here: the routing behind _WALKGLUE and the glue triggers (JD7), font metrics (the app
// passes a TextMetrics), applying themes (JD11).

mod color;
mod engine;
mod eval;
mod funcs;
mod geom;
mod keymap;
pub mod math;
mod parse;
mod sfuncs;
mod theme;
mod val;
mod xml;

pub use color::{hsl_to_rgb, rgb_to_hsl, Color};
pub use engine::{
    all_sheets, formula_cells, sheet_formula_cells, Effect, Engine, Estimate, Input, NodeKey, Outcome, Recalc, Results, LOCKS,
    TextMetrics,
};
pub use eval::Keep;
pub use parse::{cell_part, parse, unit_code, BinOp, CellPart, CellRef, Func, ParseError, ParseErrorKind, Parsed, SheetName};
pub use theme::Theme;
pub use val::{close, Axis, Stored, Target, Val};
pub use xml::{Elem, Tree};

#[cfg(test)]
mod tests;
