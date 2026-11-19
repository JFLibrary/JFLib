// engine -- recalculation over the cells of a document (FX-02, FX-04)
//
//   let mut eng = Engine::new();                 one engine per document
//   let res = eng.calculate(&doc);               every formula, in dependency order; nothing
//                                                written (res.write(&mut tx) does)
//   eng.recalc(&mut tx, &sheets)                 after an edit in tx: the cells of these sheets
//                                                (stale ones included) and all that depend on
//                                                them, results written into tx (undo keeps them)
//   eng.recalc_cells(&mut tx, &cells)            the same for single cells
//   eng.refresh(&doc, &change.sheets())          after undo/redo: dependencies again
//   eng.user_set(&mut tx, sheet, addr, input)    a user's change: GUARD, SETATREF redirection
//   eng.run_action(&mut tx, sheet, addr)         an action or event cell: SETF and effects
//   eng.evaluate(&doc, sheet, addr)              one cell with stored inputs
//
// Nothing is recalculated on its own: opening a file computes nothing (PLAN.md, "Fallen").  A
// formula that cannot be computed here -- an unknown function, a glue helper, a cycle -- keeps
// its stored value and is reported (Keep); a value is written only with the formula it
// belongs to.
//
// Dependencies are recorded while formulas run: every cell a formula reads (also the ones
// THEMEVAL, PAR or TEXTWIDTH read behind the scenes, and the master or style cell an
// inherited formula comes from) becomes an edge.  The first recalc builds the graph with a
// full calculation; later ones follow the edges from the changed sheets.  Formulas are pulled
// in: a cell that reads another cell not computed yet computes that one first (depth-limited;
// a cell met again while it is being computed is a cycle and gives its stored value).

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use super::eval::{Ev, Keep};
use super::keymap::KeyMap;
use super::parse::{parse, Func, Node, ParseError, Parsed};
use super::theme::Themes;
use super::val::{Axis, Stored, Target, Val};
use crate::cell::{CellError, Formula, Origin, Value};
use crate::document::{Document, Owner, SheetRef};
use crate::edit::{EditError, Transaction};
use crate::sheet::{CellAddr, RowKey, SecKey, Sheet};
use crate::strings::Sym;
use crate::units::Unit;

const MAX_DEPTH: u32 = 48;

/// a cell of a sheet as a sortable key
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct NodeKey {
    /// the sheet
    pub sheet: SheetRef,
    /// the section (None: a single cell)
    pub sec: Option<SecKey>,
    /// the row (None: a single cell or a cell of the section itself)
    pub row: Option<RowKey>,
    /// the cell name
    pub cell: Sym,
}

impl NodeKey {
    /// the key of a cell
    pub fn new(sheet: SheetRef, addr: &CellAddr) -> NodeKey {
        NodeKey { sheet, sec: addr.section, row: addr.row, cell: addr.cell }
    }

    /// the cell address
    pub fn addr(&self) -> CellAddr {
        CellAddr { section: self.sec, row: self.row, cell: self.cell }
    }

    fn first(sheet: SheetRef) -> NodeKey {
        NodeKey { sheet, sec: None, row: None, cell: Sym(0) }
    }

    fn last(sheet: SheetRef) -> NodeKey {
        let s = Sym(u32::MAX);
        NodeKey { sheet, sec: Some(SecKey { name: s, index: u32::MAX }), row: Some(RowKey::Named(s)), cell: s }
    }
}

/// what an action asks for besides changing formulas (SETF is done by the engine itself);
/// the app carries them out -- or not: a RUNADDON the app does not know, a macro, CALLTHIS
/// are reported, never run
#[derive(Clone, PartialEq, Debug)]
pub enum Effect {
    /// SETF: give the cell this formula (the engine applies it)
    SetFormula(Target, String),
    /// OPENTEXTWIN(): edit the shape's text
    OpenTextWindow(SheetRef),
    /// OPENSHEETWIN(): show the shape's ShapeSheet
    OpenShapeSheet(SheetRef),
    /// OPENGROUPWIN(): open the group in its own window
    OpenGroup(SheetRef),
    /// GOTOPAGE(name)
    GotoPage(String),
    /// HYPERLINK(address, sub-address)
    Hyperlink(String, String),
    /// PLAYSOUND(file or sound name)
    PlaySound(String),
    /// DOCMD(n): one of Visio's command numbers, for the app to map to its own commands
    Command(i64),
    /// RUNADDON(name): the app runs its own counterpart when it has one (e.g. the shape data
    /// dialog), else reports it
    AddOn(String),
    /// RUNMACRO, CALLTHIS: foreign code, never run (FX-15)
    Refused(String),
}

/// how a cell came out
#[derive(Clone, PartialEq, Debug)]
pub enum Outcome {
    /// computed
    Value(Val),
    /// not computed: the stored value stays
    Kept(Keep),
}

/// the size of text in the app's fonts
pub trait TextMetrics {
    /// (width, height) in inches of `text` in `font` at `size` (inches); lines break at
    /// `max_width` when given, and at line breaks
    fn measure(&self, text: &str, font: &str, size: f64, max_width: Option<f64>) -> (f64, f64);
}

/// the estimate used without the app's fonts: 0.45 em a character, lines of 1.2 em
pub struct Estimate;

impl TextMetrics for Estimate {
    fn measure(&self, text: &str, _font: &str, size: f64, max_width: Option<f64>) -> (f64, f64) {
        let em = 0.45 * size;
        let mut lines = 0usize;
        let mut widest: f64 = 0.0;
        for para in text.split('\n') {
            let w = para.chars().count() as f64 * em;
            match max_width {
                // a little slack: a width computed from the same estimate fits on one line
                Some(m) if m > 0.0 && w > m * (1.0 + 1e-9) + 1e-12 => {
                    let k = super::math::ceil(w / m) as usize;
                    lines += k.max(1);
                    widest = widest.max(m);
                }
                _ => {
                    lines += 1;
                    widest = widest.max(w);
                }
            }
        }
        (widest, lines.max(1) as f64 * 1.2 * size)
    }
}

enum Slot {
    Busy,
    // computed in this run, and the value readers get (the stored one when kept)
    Done(Outcome, Val),
    // read as stored, and the sheet the value is inherited from
    Read(Val, Option<SheetRef>),
    // computed with an input cut off by the depth limit: computed again later
    Retry,
}

type ParseCache = BTreeMap<Sym, Result<Rc<Parsed>, ParseError>>;

/// the state of one calculation (crate-internal; Ev works on it)
pub(crate) struct Run {
    cache: ParseCache,
    memo: KeyMap<Slot>,
    // None: compute every formula; Some: only these, the rest is read as stored
    dirty: Option<BTreeSet<NodeKey>>,
    depth: u32,
    // the cells read by the formulas being computed (innermost last)
    reads: Vec<(NodeKey, Vec<NodeKey>)>,
    // what each computed cell read
    recorded: Vec<(NodeKey, Vec<NodeKey>)>,
    cycles: BTreeSet<NodeKey>,
    // how often the depth limit cut a computation, and the cells it cut (computed later from
    // the top, deepest first)
    deep: u64,
    frontier: Vec<Node3>,
    pub(crate) effects: Option<Vec<Effect>>,
    pub(crate) set_value: Option<Val>,
    pub(crate) themes: Themes,
    metrics: Rc<dyn TextMetrics>,
}

impl Run {
    pub(crate) fn push_effect(&mut self, e: Effect) {
        if let Some(v) = self.effects.as_mut() {
            v.push(e);
        }
    }

    pub(crate) fn measure(&self, text: &str, font: &str, size: f64, max: Option<f64>) -> (f64, f64) {
        if text.is_empty() {
            // an empty text: no width, one line
            return (0.0, 1.2 * size);
        }
        self.metrics.measure(text, font, size, max)
    }

    fn parsed(&mut self, doc: &Document, f: Sym) -> Result<Rc<Parsed>, ParseError> {
        cached(&mut self.cache, doc, f)
    }

    fn record(&mut self, k: NodeKey) {
        if let Some((_, v)) = self.reads.last_mut() {
            v.push(k);
        }
    }

    pub(crate) fn record_key(&mut self, k: NodeKey) {
        self.record(k);
    }

    // how many reads the innermost formula has recorded so far
    pub(crate) fn reads_len(&self) -> usize {
        self.reads.last().map_or(0, |(_, v)| v.len())
    }

    // the reads recorded since `mark`
    pub(crate) fn reads_since(&self, mark: usize) -> Vec<NodeKey> {
        self.reads.last().map_or(Vec::new(), |(_, v)| v.get(mark..).map_or(Vec::new(), |s| s.to_vec()))
    }
}

fn first_unknown(p: &Parsed) -> Option<String> {
    p.unknown().next().map(String::from)
}

fn cached(cache: &mut ParseCache, doc: &Document, f: Sym) -> Result<Rc<Parsed>, ParseError> {
    cache
        .entry(f)
        .or_insert_with(|| {
            parse(doc.str(f)).map(|mut p| {
                p.addrs = p.refs.iter().map(|r| super::eval::bind(doc, &r.cell)).collect();
                Rc::new(p)
            })
        })
        .clone()
}

// the stored value of a resolved cell
fn stored_of(doc: &Document, r: &crate::inherit::Resolved, addr: &CellAddr) -> Val {
    match r.origin {
        Origin::Deleted => Val::Err(CellError::Ref),
        Origin::Default if r.cached.is_none() => default_of(doc, addr),
        _ => Val::from_value(r.value(), r.error().map(|e| doc.str(e)), doc.strings()),
    }
}

// the stored value of a cell for a sheet
fn stored(doc: &Document, sheet: SheetRef, addr: &CellAddr) -> Val {
    stored_of(doc, &doc.resolve(sheet, addr), addr)
}

// a cell no sheet on the way has: single cells have their default (mostly 0), rows do not
// exist
fn default_of(doc: &Document, addr: &CellAddr) -> Val {
    if addr.section.is_some() {
        return Val::Err(CellError::Ref);
    }
    Val::Num(default_single(doc.str(addr.cell)))
}

/// (crate) the default of a single cell no sheet has
pub(crate) fn default_single(name: &str) -> f64 {
    match name {
        "DropOnPageScale" | "FontScale" | "ShapeShdwScaleFactor" | "PageScale" | "DrawingScale" | "LinePattern" | "FillPattern" => 1.0,
        _ => 0.0,
    }
}

/// (crate) the value of a cell for a sheet during a calculation: computed now when it has a
/// formula the run computes, else as stored
pub(crate) fn cell_value(doc: &Document, run: &mut Run, sheet: SheetRef, addr: &CellAddr) -> Val {
    let key = NodeKey::new(sheet, addr);
    run.record(key);
    match run.memo.get(&key) {
        // a themed value read by another formula is the value behind it
        Some(Slot::Done(_, v)) => return v.plain().clone(),
        Some(Slot::Read(v, src)) => {
            let (v, src) = (v.clone(), *src);
            if let Some(src) = src {
                run.record(NodeKey { sheet: src, ..key });
            }
            return v;
        }
        Some(Slot::Busy) => {
            run.cycles.insert(key);
            return stored(doc, sheet, addr);
        }
        Some(Slot::Retry) | None => {}
    }
    let r = doc.resolve(sheet, addr);
    let src = r.source.filter(|&s| s != sheet);
    if let Some(src) = src {
        run.record(NodeKey { sheet: src, ..key });
    }
    let value = stored_of(doc, &r, addr);
    let retry = matches!(run.memo.get(&key), Some(Slot::Retry));
    let f = match r.formula() {
        Formula::Expr(f) => {
            let themed = matches!(&value, Val::Str(s) if &**s == "Themed");
            if retry || themed || run.dirty.as_ref().is_none_or(|d| d.contains(&key)) { Some(f) } else { None }
        }
        _ => None,
    };
    let Some(f) = f else {
        run.memo.insert(key, Slot::Read(value.clone(), src));
        return value;
    };
    match compute(doc, run, key, f, r.source.unwrap_or(sheet)) {
        Outcome::Value(v) => v.plain().clone(),
        Outcome::Kept(_) => value,
    }
}

// compute one cell's formula (the result goes into the memo)
fn compute(doc: &Document, run: &mut Run, key: NodeKey, f: Sym, src: SheetRef) -> Outcome {
    if run.depth >= MAX_DEPTH {
        // not now: the caller gets the stored value and is computed again once this cell is
        // (compute_run starts from here)
        run.deep += 1;
        run.frontier.push((key, f, src));
        return Outcome::Kept(Keep::Deep);
    }
    let deep_before = run.deep;
    run.memo.insert(key, Slot::Busy);
    let parsed = run.parsed(doc, f);
    run.reads.push((key, Vec::new()));
    if src != key.sheet {
        run.record(NodeKey { sheet: src, ..key });
    }
    let out = match parsed {
        Err(e) => Outcome::Kept(Keep::Parse(e)),
        Ok(p) => match first_unknown(&p) {
            // an unknown function: not evaluated at all (its arguments could have effects)
            Some(name) => Outcome::Kept(Keep::Unknown(name)),
            None => {
                run.depth += 1;
                let root = p.root;
                let mut ev = Ev {
                    doc,
                    run,
                    sheet: key.sheet,
                    cell: key.cell,
                    addr: key.addr(),
                    axis: Axis::of(doc.str(key.cell)),
                    p,
                    src,
                    keep: None,
                };
                let r = ev.eval(root);
                let keep = ev.keep.take();
                run.depth -= 1;
                match (keep, r) {
                    (Some(k), _) => Outcome::Kept(k),
                    (None, Ok(v)) => Outcome::Value(v),
                    (None, Err(e)) => Outcome::Value(Val::Err(e)),
                }
            }
        },
    };
    if let Some((k, reads)) = run.reads.pop() {
        run.recorded.push((k, reads));
    }
    if run.deep > deep_before {
        // an input was cut off: this result is not final
        run.memo.insert(key, Slot::Retry);
        return Outcome::Kept(Keep::Deep);
    }
    let readers = match &out {
        Outcome::Value(v) => v.clone(),
        Outcome::Kept(_) => stored(doc, key.sheet, &key.addr()),
    };
    run.memo.insert(key, Slot::Done(out.clone(), readers));
    out
}

fn done(run: &Run, k: &NodeKey) -> bool {
    matches!(run.memo.get(k), Some(Slot::Done(..)))
}


// compute the given cells in a run -> their results
fn compute_run(doc: &Document, run: &mut Run, nodes: &[Node3]) -> Results {
    let mut out = Results::default();
    for &(k, f, src) in nodes {
        if !run.memo.contains_key(&k) {
            compute(doc, run, k, f, src);
        }
    }
    // chains deeper than the depth limit: the cells where they were cut first, deepest first
    // (a cell cut again goes back under the deeper ones it found), then the cells that read
    // them; each round reaches MAX_DEPTH further
    let mut budget = 4 * nodes.len() + 1024;
    loop {
        while let Some(n) = run.frontier.pop() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            if done(run, &n.0) {
                continue;
            }
            let len = run.frontier.len();
            compute(doc, run, n.0, n.1, n.2);
            if !done(run, &n.0) {
                run.frontier.insert(len, n);
            }
        }
        let mut again = false;
        for &(k, f, src) in nodes {
            if matches!(run.memo.get(&k), Some(Slot::Retry)) && budget > 0 {
                budget -= 1;
                compute(doc, run, k, f, src);
                again = true;
            }
        }
        if !again || budget == 0 || (run.frontier.is_empty() && nodes.iter().all(|n| !matches!(run.memo.get(&n.0), Some(Slot::Retry)))) {
            break;
        }
    }
    for (k, slot) in run.memo.iter() {
        match slot {
            Slot::Done(o, _) => out.cells.push((k.sheet, k.addr(), o.clone())),
            Slot::Retry => out.cells.push((k.sheet, k.addr(), Outcome::Kept(Keep::Deep))),
            _ => {}
        }
    }
    out.cycles = run.cycles.iter().map(|k| (k.sheet, k.addr())).collect();
    out
}

// a formula cell: its key, its formula and the sheet that holds the formula
type Node3 = (NodeKey, Sym, SheetRef);

// the formula and its sheet for these cells (cells without an expression drop out)
fn nodes_of(doc: &Document, keys: &[NodeKey]) -> Vec<Node3> {
    keys.iter()
        .filter_map(|&k| {
            let r = doc.resolve(k.sheet, &k.addr());
            match r.formula() {
                Formula::Expr(f) => Some((k, f, r.source.unwrap_or(k.sheet))),
                _ => None,
            }
        })
        .collect()
}

/// the result of a calculation
#[derive(Clone, Debug, Default)]
pub struct Results {
    /// every computed or kept cell in the order computed
    pub cells: Vec<(SheetRef, CellAddr, Outcome)>,
    /// cells met while being computed (cycles); they used their stored value
    pub cycles: Vec<(SheetRef, CellAddr)>,
}

impl Results {
    /// the outcome of one cell
    pub fn get(&self, sheet: SheetRef, addr: &CellAddr) -> Option<&Outcome> {
        self.cells.iter().find(|(s, a, _)| *s == sheet && a == addr).map(|(_, _, o)| o)
    }

    /// write every result that differs from the stored value into the transaction (cells
    /// that inherit their formula get an F="Inh" cell); -> the number of cells written
    pub fn write(&self, tx: &mut Transaction) -> Result<usize, EditError> {
        let mut n = 0;
        for (s, a, o) in &self.cells {
            if let Outcome::Value(v) = o {
                n += write_one(tx, *s, a, v)? as usize;
            }
        }
        Ok(n)
    }
}

// write one result when it differs from what is stored (or the cell is stale)
fn write_one(tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr, v: &Val) -> Result<bool, EditError> {
    let doc = tx.doc();
    let r = doc.resolve(sheet, addr);
    if !r.exists() {
        return Ok(false);
    }
    let cell_name = doc.str(addr.cell);
    let st = v.stored(cell_name);
    let stale = r.cached.is_some_and(|c| c.stale);
    let same = st.matches(r.value(), r.error().map(|e| doc.str(e)), doc.strings(), 0.0) && r.cached.is_some();
    if same && !stale {
        return Ok(false);
    }
    let (value, error) = match st {
        Stored::Num(x) => (Value::Num(x), None),
        Stored::Text(t) => (Value::Text(tx.intern(&t)), None),
        Stored::Error(e) => (r.value(), Some(e)),
    };
    match tx.set_result(sheet, addr, value, error) {
        Ok(()) => Ok(true),
        Err(EditError::Invalid | EditError::Deleted) => Ok(false),
        Err(e) => Err(e),
    }
}

/// what a recalculation did
#[derive(Clone, Debug, Default)]
pub struct Recalc {
    /// cells computed
    pub computed: usize,
    /// cells whose stored value changed
    pub written: usize,
    /// cells that kept their stored value, and why
    pub kept: Vec<(SheetRef, CellAddr, Keep)>,
    /// cells in cycles
    pub cycles: usize,
}

/// the protection cells ([MS-VSDX] Protection section, SS-15)
pub const LOCKS: &[&str] = &[
    "LockWidth", "LockHeight", "LockAspect", "LockMoveX", "LockMoveY", "LockRotate", "LockBegin", "LockEnd", "LockDelete",
    "LockSelect", "LockFormat", "LockTextEdit", "LockVtxEdit", "LockCrop", "LockGroup", "LockCalcWH", "LockFromGroupFormat",
    "LockCustProp", "LockReplace", "LockThemeColors", "LockThemeEffects", "LockThemeConnectors", "LockThemeFonts",
    "LockThemeIndex", "LockVariation",
];

/// a user's change of a cell (Engine::user_set)
#[derive(Copy, Clone, Debug)]
pub enum Input<'a> {
    /// a value in internal units, shown in the unit
    Value(Value, Option<Unit>),
    /// a length or angle in a unit (25 mm)
    Length(f64, Unit),
    /// a formula
    Formula(&'a str),
}

/// the formula engine of one document
pub struct Engine {
    cache: ParseCache,
    // what each computed cell read, and who read each cell
    fwd: BTreeMap<NodeKey, Vec<NodeKey>>,
    rev: BTreeMap<NodeKey, Vec<NodeKey>>,
    // dependencies recorded by calculate, made into fwd/rev when a recalc needs them
    pending: Vec<(NodeKey, Vec<NodeKey>)>,
    built: bool,
    themes: Themes,
    metrics: Rc<dyn TextMetrics>,
}

impl Default for Engine {
    fn default() -> Self {
        Engine::new()
    }
}

impl Engine {
    /// an engine with the text estimate
    pub fn new() -> Engine {
        Engine::with_metrics(Rc::new(Estimate))
    }

    /// an engine that measures text with the app's fonts
    pub fn with_metrics(metrics: Rc<dyn TextMetrics>) -> Engine {
        Engine { cache: BTreeMap::new(), fwd: BTreeMap::new(), rev: BTreeMap::new(), pending: Vec::new(), built: false, themes: Themes::default(), metrics }
    }

    /// forget everything (the document was replaced, or its themes changed)
    pub fn reset(&mut self) {
        *self = Engine::with_metrics(self.metrics.clone());
    }

    /// forget the parsed theme parts (JD11: a design was applied or a part replaced); the
    /// caller recalculates the sheets that use them
    pub fn themes_changed(&mut self) {
        self.themes = Themes::default();
    }

    fn run(&mut self, dirty: Option<BTreeSet<NodeKey>>) -> Run {
        Run {
            cache: core::mem::take(&mut self.cache),
            memo: KeyMap::new(),
            dirty,
            depth: 0,
            reads: Vec::new(),
            recorded: Vec::new(),
            cycles: BTreeSet::new(),
            deep: 0,
            frontier: Vec::new(),
            effects: None,
            set_value: None,
            themes: {
                let mut t = core::mem::take(&mut self.themes);
                t.new_run();
                t
            },
            metrics: self.metrics.clone(),
        }
    }

    // take back what a run kept, and its dependencies
    fn finish(&mut self, run: &mut Run) {
        self.cache = core::mem::take(&mut run.cache);
        self.themes = core::mem::take(&mut run.themes);
        self.graph();
        for (k, reads) in run.recorded.drain(..) {
            self.set_deps(k, reads);
        }
    }

    // make the dependencies a full calculation recorded into the graph (once, when needed)
    fn graph(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        {
            // a whole graph at once: sorted pairs instead of one insert per edge
            let mut pairs: Vec<(NodeKey, NodeKey)> = Vec::new();
            let mut fwd: Vec<(NodeKey, Vec<NodeKey>)> = Vec::with_capacity(self.pending.len());
            for (k, mut reads) in core::mem::take(&mut self.pending) {
                reads.sort_unstable();
                reads.dedup();
                reads.retain(|r| *r != k);
                pairs.extend(reads.iter().map(|r| (*r, k)));
                fwd.push((k, reads));
            }
            fwd.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            fwd.dedup_by(|a, b| a.0 == b.0);
            // sorted input: the maps are built in one pass
            self.fwd = fwd.into_iter().collect();
            pairs.sort_unstable();
            pairs.dedup();
            let mut rev: Vec<(NodeKey, Vec<NodeKey>)> = Vec::new();
            for (r, k) in pairs {
                match rev.last_mut() {
                    Some((last, v)) if *last == r => v.push(k),
                    _ => rev.push((r, alloc::vec![k])),
                }
            }
            self.rev = rev.into_iter().collect();
        }
    }

    fn set_deps(&mut self, k: NodeKey, mut reads: Vec<NodeKey>) {
        reads.sort_unstable();
        reads.dedup();
        reads.retain(|r| *r != k);
        if let Some(old) = self.fwd.remove(&k) {
            for r in old {
                if let Some(v) = self.rev.get_mut(&r) {
                    if let Ok(p) = v.binary_search(&k) {
                        v.remove(p);
                    }
                    if v.is_empty() {
                        self.rev.remove(&r);
                    }
                }
            }
        }
        for r in &reads {
            let v = self.rev.entry(*r).or_default();
            if let Err(p) = v.binary_search(&k) {
                v.insert(p, k);
            }
        }
        self.fwd.insert(k, reads);
    }

    // compute the given cells (all formula cells when None) -> results
    fn compute_all(&mut self, doc: &Document, keys: &[NodeKey], dirty: Option<BTreeSet<NodeKey>>) -> Results {
        let mut run = self.run(dirty);
        let out = compute_run(doc, &mut run, &nodes_of(doc, keys));
        self.finish(&mut run);
        out
    }



    /// compute every formula of the document (nothing is written; Results::write does);
    /// this also records the dependencies later recalculations follow
    pub fn calculate(&mut self, doc: &Document) -> Results {
        let nodes = formula_nodes(doc);
        self.fwd.clear();
        self.rev.clear();
        self.pending.clear();
        let mut run = self.run(None);
        let r = compute_run(doc, &mut run, &nodes);
        // the graph is made from these when a recalculation needs it
        let recorded = core::mem::take(&mut run.recorded);
        self.finish(&mut run);
        self.pending = recorded;
        self.built = true;
        r
    }

    /// compute one cell from the stored values of its inputs (nothing else is recomputed)
    pub fn evaluate(&mut self, doc: &Document, sheet: SheetRef, addr: &CellAddr) -> Outcome {
        let key = NodeKey::new(sheet, addr);
        let mut set = BTreeSet::new();
        set.insert(key);
        let mut run = self.run(Some(set));
        let r = doc.resolve(sheet, addr);
        let o = match r.formula() {
            Formula::Expr(f) => compute(doc, &mut run, key, f, r.source.unwrap_or(sheet)),
            _ => Outcome::Value(stored(doc, sheet, addr)),
        };
        run.recorded.clear();
        self.finish(&mut run);
        o
    }

    /// evaluate a formula text for a cell without storing anything (the formula editor's
    /// preview); inputs as stored
    pub fn evaluate_text(&mut self, doc: &Document, sheet: SheetRef, addr: &CellAddr, text: &str) -> Result<Outcome, ParseError> {
        let p = parse(text)?;
        let mut run = self.run(Some(BTreeSet::new()));
        let key = NodeKey::new(sheet, addr);
        run.reads.push((key, Vec::new()));
        let o = match first_unknown(&p) {
            Some(name) => Outcome::Kept(Keep::Unknown(name)),
            None => {
                let root = p.root;
                let mut ev = Ev { doc, run: &mut run, sheet, cell: addr.cell, addr: *addr, axis: Axis::of(doc.str(addr.cell)), p: Rc::new(p), src: sheet, keep: None };
                let r = ev.eval(root);
                match (ev.keep.take(), r) {
                    (Some(k), _) => Outcome::Kept(k),
                    (None, Ok(v)) => Outcome::Value(v),
                    (None, Err(e)) => Outcome::Value(Val::Err(e)),
                }
            }
        };
        run.reads.clear();
        run.recorded.clear();
        self.finish(&mut run);
        Ok(o)
    }

    // the cells to recompute after changes to these sheets and cells: their formula cells
    // and everything that read them, transitively
    fn affected(&self, doc: &Document, sheets: &[SheetRef], cells: &[NodeKey]) -> Vec<NodeKey> {
        let mut seen: BTreeSet<NodeKey> = BTreeSet::new();
        let mut order = Vec::new();
        let mut queue: VecDeque<NodeKey> = VecDeque::new();
        for &s in sheets {
            if doc.sheet(s).is_some() {
                for k in sheet_formula_cells(doc, s) {
                    queue.push_back(k);
                }
            }
            for (k, _) in self.rev.range(NodeKey::first(s)..=NodeKey::last(s)) {
                queue.push_back(*k);
            }
            for (k, _) in self.fwd.range(NodeKey::first(s)..=NodeKey::last(s)) {
                queue.push_back(*k);
            }
        }
        queue.extend(cells.iter().copied());
        while let Some(k) = queue.pop_front() {
            if !seen.insert(k) {
                continue;
            }
            order.push(k);
            if let Some(users) = self.rev.get(&k) {
                queue.extend(users.iter().copied());
            }
        }
        order
    }

    fn ensure_built(&mut self, doc: &Document) {
        if !self.built {
            self.calculate(doc);
        }
        self.graph();
    }

    fn recalc_keys(&mut self, tx: &mut Transaction, sheets: &[SheetRef], cells: &[NodeKey]) -> Result<Recalc, EditError> {
        self.ensure_built(tx.doc());
        // sheets that are gone: their cells leave the graph
        for &s in sheets {
            if tx.doc().sheet(s).is_none() {
                let gone: Vec<NodeKey> = self.fwd.range(NodeKey::first(s)..=NodeKey::last(s)).map(|(k, _)| *k).collect();
                for k in gone {
                    self.set_deps(k, Vec::new());
                    self.fwd.remove(&k);
                }
            }
        }
        let keys = self.affected(tx.doc(), sheets, cells);
        let dirty: BTreeSet<NodeKey> = keys.iter().copied().collect();
        let res = self.compute_all(tx.doc(), &keys, Some(dirty));
        let mut rc = Recalc { cycles: res.cycles.len(), ..Recalc::default() };
        for (s, a, o) in &res.cells {
            match o {
                Outcome::Value(v) => {
                    rc.computed += 1;
                    rc.written += write_one(tx, *s, a, v)? as usize;
                }
                Outcome::Kept(k) => rc.kept.push((*s, *a, k.clone())),
            }
        }
        Ok(rc)
    }

    /// recompute after an edit: the formula cells of these sheets (Change::sheets(); stale
    /// cells among them) and every cell that depends on them; results go into `tx`
    pub fn recalc(&mut self, tx: &mut Transaction, sheets: &[SheetRef]) -> Result<Recalc, EditError> {
        self.recalc_keys(tx, sheets, &[])
    }

    /// recompute these cells and every cell that depends on them
    pub fn recalc_cells(&mut self, tx: &mut Transaction, cells: &[(SheetRef, CellAddr)]) -> Result<Recalc, EditError> {
        let keys: Vec<NodeKey> = cells.iter().map(|(s, a)| NodeKey::new(*s, a)).collect();
        self.recalc_keys(tx, &[], &keys)
    }

    /// after undo or redo (the stored values are right again): record the dependencies of
    /// the cells of these sheets anew
    pub fn refresh(&mut self, doc: &Document, sheets: &[SheetRef]) {
        if !self.built {
            return;
        }
        let mut keys = Vec::new();
        for &s in sheets {
            let old: Vec<NodeKey> = self.fwd.range(NodeKey::first(s)..=NodeKey::last(s)).map(|(k, _)| *k).collect();
            for k in old {
                self.set_deps(k, Vec::new());
                self.fwd.remove(&k);
            }
            if doc.sheet(s).is_some() {
                keys.extend(sheet_formula_cells(doc, s));
            }
        }
        let dirty = keys.iter().copied().collect();
        let _ = self.compute_all(doc, &keys, Some(dirty));
    }

    /// the protection cells of a sheet (SS-15) that are set, with inheritance: "LockWidth",
    /// "LockMoveX", "LockThemeColors" ... -- gestures (JD7) and applying a theme (JD11) ask
    /// them; a direct change in the ShapeSheet does not
    pub fn locks(&mut self, doc: &Document, sheet: SheetRef) -> Vec<&'static str> {
        LOCKS
            .iter()
            .copied()
            .filter(|n| {
                doc.sym(n).is_some_and(|c| {
                    let a = CellAddr::single(c);
                    match self.evaluate(doc, sheet, &a) {
                        Outcome::Value(v) => v.num(Axis::X).is_ok_and(|x| x != 0.0),
                        Outcome::Kept(_) => doc.resolve(sheet, &a).num().is_some_and(|x| x != 0.0),
                    }
                })
            })
            .collect()
    }

    /// whether the effective formula of a cell is protected against the user's edits
    /// (GUARD around it)
    pub fn guarded(&mut self, doc: &Document, sheet: SheetRef, addr: &CellAddr) -> bool {
        self.top_call(doc, sheet, addr, Func::Guard)
    }

    /// whether the formula keeps its value when a theme or style is applied (THEMEGUARD
    /// around it, or GUARD)
    pub fn theme_guarded(&mut self, doc: &Document, sheet: SheetRef, addr: &CellAddr) -> bool {
        self.top_call(doc, sheet, addr, Func::ThemeGuard) || self.top_call(doc, sheet, addr, Func::Guard)
    }

    fn top_call(&mut self, doc: &Document, sheet: SheetRef, addr: &CellAddr, f: Func) -> bool {
        let Formula::Expr(s) = doc.resolve(sheet, addr).formula() else { return false };
        matches!(cached(&mut self.cache, doc, s), Ok(p) if p.top_is(f))
    }

    /// a user's change of a cell (a gesture or a dialog): a guarded formula refuses it
    /// (EditError::Guarded); a formula with SETATREF keeps its formula and the change goes to
    /// the cells SETATREF names; then everything that depends on it is recomputed
    pub fn user_set(&mut self, tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr, input: Input) -> Result<Recalc, EditError> {
        let doc = tx.doc();
        let formula = match doc.resolve(sheet, addr).formula() {
            Formula::Expr(s) => Some(s),
            _ => None,
        };
        let redirect = formula.and_then(|s| cached(&mut self.cache, doc, s).ok()).filter(|p| p.calls(Func::SetAtRef));
        if let Some(p) = redirect {
            let targets = self.redirect(tx, sheet, addr, &p, input)?;
            let mut cells = targets;
            cells.push((sheet, *addr));
            return self.recalc_cells(tx, &cells);
        }
        match input {
            Input::Value(v, u) => tx.set_constant(sheet, addr, v, u)?,
            Input::Length(x, u) => tx.set_length(sheet, addr, x, u)?,
            Input::Formula(f) => tx.set_formula(sheet, addr, f)?,
        }
        self.recalc_cells(tx, &[(sheet, *addr)])
    }

    // SETATREF(ref, [set_expr], [ignoreEval]): the new value goes to ref -- as the value of
    // set_expr when given (SETATREFEXPR() in it stands for the new value), else as entered
    fn redirect(&mut self, tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr, p: &Rc<Parsed>, input: Input) -> Result<Vec<(SheetRef, CellAddr)>, EditError> {
        let new_val = match input {
            Input::Value(Value::Num(x), _) => Val::Num(x),
            Input::Value(Value::Text(t), _) => Val::str(tx.doc().str(t)),
            Input::Value(Value::Absent, _) => Val::Num(0.0),
            Input::Length(x, u) => Val::Num(u.to_internal(x)),
            Input::Formula(f) => match self.evaluate_text(tx.doc(), sheet, addr, f) {
                Ok(Outcome::Value(v)) => v,
                _ => return Err(EditError::Invalid),
            },
        };
        let calls: Vec<(u32, u32)> = p
            .nodes
            .iter()
            .filter_map(|n| match *n {
                Node::Call(Func::SetAtRef, s, k, _) => Some((s, k)),
                _ => None,
            })
            .collect();
        let mut targets = Vec::new();
        for (s, k) in calls {
            let args: Vec<u32> = p.args[s as usize..(s + k) as usize].to_vec();
            let mut run = self.run(Some(BTreeSet::new()));
            run.set_value = Some(new_val.clone());
            run.reads.push((NodeKey::new(sheet, addr), Vec::new()));
            let (target, value) = {
                let doc = tx.doc();
                let mut ev = Ev { doc, run: &mut run, sheet, cell: addr.cell, addr: *addr, axis: Axis::of(doc.str(addr.cell)), p: p.clone(), src: sheet, keep: None };
                let t = super::eval::target_of(&mut ev, args[0]);
                let v = match args.get(1) {
                    Some(&x) if ev.node(x) != Node::Missing => ev.eval(x),
                    _ => Ok(new_val.clone()),
                };
                (t, v)
            };
            run.reads.clear();
            run.recorded.clear();
            self.finish(&mut run);
            let (Ok(t), Ok(v)) = (target, value) else { continue };
            let Some(ta) = t.addr else { continue };
            let st = v.stored(tx.doc().str(ta.cell));
            let value = match st {
                Stored::Num(x) => Value::Num(x),
                Stored::Text(s) => Value::Text(tx.intern(&s)),
                Stored::Error(_) => continue,
            };
            let unit = tx.doc().resolve(t.sheet, &ta).unit();
            tx.set_constant(t.sheet, &ta, value, unit)?;
            targets.push((t.sheet, ta));
        }
        Ok(targets)
    }

    /// run an action or event cell (Actions.Row_1.Action, EventDblClick ...): SETF changes
    /// formulas in `tx`, also guarded ones (then dependents are recomputed); the other effects
    /// are for the app
    pub fn run_action(&mut self, tx: &mut Transaction, sheet: SheetRef, addr: &CellAddr) -> Result<Vec<Effect>, EditError> {
        let key = NodeKey::new(sheet, addr);
        let mut run = self.run(Some(BTreeSet::new()));
        run.effects = Some(Vec::new());
        {
            let doc = tx.doc();
            let r = doc.resolve(sheet, addr);
            if let Formula::Expr(f) = r.formula() {
                let mut set = BTreeSet::new();
                set.insert(key);
                run.dirty = Some(set);
                compute(doc, &mut run, key, f, r.source.unwrap_or(sheet));
            }
        }
        let effects = run.effects.take().unwrap_or_default();
        run.recorded.clear();
        self.finish(&mut run);
        let mut rest = Vec::new();
        let mut changed = Vec::new();
        for e in effects {
            match e {
                Effect::SetFormula(t, text) => {
                    if let Some(a) = t.addr {
                        // GUARD protects against the user, not against SETF (ShapeSheet reference)
                        tx.force_formula(t.sheet, &a, &text)?;
                        changed.push((t.sheet, a));
                    }
                }
                e => rest.push(e),
            }
        }
        if !changed.is_empty() {
            self.recalc_cells(tx, &changed)?;
        }
        Ok(rest)
    }

    /// the number of cells in the dependency graph (for tests and statistics)
    pub fn graph_size(&mut self) -> (usize, usize) {
        self.graph();
        (self.fwd.len(), self.fwd.values().map(|v| v.len()).sum())
    }
}

// ---- which cells have formulas

fn walk_cells(sh: &Sheet, mut f: impl FnMut(CellAddr, &crate::cell::Cell)) {
    for c in &sh.cells {
        f(CellAddr::single(c.name), c);
    }
    for s in &sh.sections {
        if s.del {
            continue;
        }
        let sk = s.key();
        for c in &s.cells {
            f(CellAddr::section(sk, c.name), c);
        }
        for r in &s.rows {
            if r.del {
                continue;
            }
            for c in &r.cells {
                f(CellAddr::row(sk, r.key, c.name), c);
            }
        }
    }
}

// the sheets a sheet inherits from (master shapes and styles, any category)
fn ancestors(doc: &Document, s: SheetRef) -> Vec<SheetRef> {
    use crate::names::Category;
    let mut out: Vec<SheetRef> = Vec::new();
    let mut stack = alloc::vec![s];
    while let Some(x) = stack.pop() {
        if out.len() > 64 {
            break;
        }
        for cat in [None, Some(Category::Line), Some(Category::Fill), Some(Category::Text)] {
            if let Some(p) = doc.parent_sheet(x, cat) {
                if p != s && !out.contains(&p) {
                    out.push(p);
                    stack.push(p);
                }
            }
        }
    }
    out
}

// the cells of a sheet that have an expression (candidates for the sheets inheriting from it)
fn expr_cells(doc: &Document, s: SheetRef) -> Vec<CellAddr> {
    let mut v = Vec::new();
    if let Some(sh) = doc.sheet(s) {
        walk_cells(sh, |a, c| {
            if matches!(c.formula, Formula::Expr(_)) {
                v.push(a);
            }
        });
    }
    v
}

fn sheet_cells_with(doc: &Document, s: SheetRef, cache: &mut BTreeMap<SheetRef, Rc<Vec<CellAddr>>>) -> Vec<Node3> {
    let mut set: BTreeMap<NodeKey, (Sym, SheetRef)> = BTreeMap::new();
    let mut inh: Vec<NodeKey> = Vec::new();
    let Some(sh) = doc.sheet(s) else { return Vec::new() };
    walk_cells(sh, |a, c| match c.formula {
        Formula::Expr(f) => {
            set.insert(NodeKey::new(s, &a), (f, s));
        }
        Formula::Inherited => inh.push(NodeKey::new(s, &a)),
        _ => {}
    });
    let mut cand: BTreeSet<NodeKey> = inh.into_iter().collect();
    for p in ancestors(doc, s) {
        let list = cache.entry(p).or_insert_with(|| Rc::new(expr_cells(doc, p))).clone();
        cand.extend(list.iter().map(|a| NodeKey::new(s, a)));
    }
    for k in cand {
        if set.contains_key(&k) {
            continue;
        }
        let r = doc.resolve(s, &k.addr());
        if let Formula::Expr(f) = r.formula() {
            set.insert(k, (f, r.source.unwrap_or(s)));
        }
    }
    set.into_iter().map(|(k, (f, src))| (k, f, src)).collect()
}

/// the cells of one sheet whose value comes from a formula: its own cells with a formula
/// (also F="Inh") and the cells it inherits a formula for from master shapes and styles
pub fn sheet_formula_cells(doc: &Document, s: SheetRef) -> Vec<NodeKey> {
    sheet_cells_with(doc, s, &mut BTreeMap::new()).into_iter().map(|n| n.0).collect()
}

/// every sheet of the document: the document sheet, styles, pages, masters, the shapes of
/// masters and pages
pub fn all_sheets(doc: &Document) -> Vec<SheetRef> {
    let mut v = alloc::vec![SheetRef::Document];
    v.extend(doc.styles().map(SheetRef::Style));
    for m in doc.masters() {
        v.push(SheetRef::Master(m));
        v.extend(doc.all_shapes(Owner::Master(m)).map(SheetRef::Shape));
    }
    for p in doc.pages() {
        v.push(SheetRef::Page(p));
        v.extend(doc.all_shapes(Owner::Page(p)).map(SheetRef::Shape));
    }
    v
}

/// every cell of the document whose value comes from a formula (sheet_formula_cells of all
/// sheets)
pub fn formula_cells(doc: &Document) -> Vec<NodeKey> {
    formula_nodes(doc).into_iter().map(|n| n.0).collect()
}

fn formula_nodes(doc: &Document) -> Vec<Node3> {
    let mut out = Vec::new();
    let mut cache = BTreeMap::new();
    for s in all_sheets(doc) {
        out.extend(sheet_cells_with(doc, s, &mut cache));
    }
    out
}
