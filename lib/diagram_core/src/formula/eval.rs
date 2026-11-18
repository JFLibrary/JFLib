// eval -- evaluating a parsed formula for one cell of one sheet (FX-02, FX-03)
//
// Ev holds what a formula needs while it runs: the document, the engine's run (results so far,
// themes, text metrics, the effects an action collects), the sheet and cell it is evaluated
// for, and the sheet whose cell holds the formula text.  The last two differ for inherited
// formulas: a master shape's formula runs in the instance's context (Width is the instance's
// Width), and its Sheet.N references name shapes of the master, which the instance has under
// other IDs -- they are mapped to the instance's subshape with MasterShape = N.
//
// Arithmetic works on internal units (the parser converted "10 mm"), so units need no work
// here.  Errors propagate through operators and functions; IF, AND, OR and the like evaluate
// only what they need.

use alloc::rc::Rc;
use alloc::string::String;

use super::color::Color;
use super::engine::Run;
use super::parse::{BinOp, CellPart, CellRef, Node, Parsed, SheetName};
use super::val::{Axis, Target, Val};
use crate::cell::CellError;
use crate::document::{Document, Owner, SheetRef};
use crate::sheet::{CellAddr, RowKey, SecKey};
use crate::strings::Sym;

/// why a formula's value cannot be computed here (the stored value stays and the cell is
/// marked)
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Keep {
    /// the formula does not parse
    Parse(super::parse::ParseError),
    /// it calls a function the engine does not know (the first such name)
    Unknown(String),
    /// it calls a function the engine knows but does not compute here (glue helpers,
    /// add-ons, macros, values that depend on the machine: fonts, random numbers)
    NotComputed(&'static str),
    /// it names a section the engine does not know
    Reference,
    /// its inputs nest deeper than the engine follows at once
    Deep,
}

pub(crate) type R = Result<Val, CellError>;

pub(crate) struct Ev<'a, 'd> {
    pub doc: &'d Document,
    pub run: &'a mut Run,
    pub sheet: SheetRef,
    pub cell: Sym,
    pub addr: CellAddr,
    pub axis: Axis,
    pub p: Rc<Parsed>,
    pub src: SheetRef,
    pub keep: Option<Keep>,
}

/// the address of the cell being evaluated (THEMEVAL() needs its section and row)
pub(crate) fn host_addr(ev: &Ev) -> Option<CellAddr> {
    Some(ev.addr)
}

/// the page or master whose shape tree holds a sheet
pub(crate) fn owner_of(s: SheetRef) -> Option<Owner> {
    match s {
        SheetRef::Shape(r) => Some(r.owner),
        SheetRef::Page(p) => Some(Owner::Page(p)),
        SheetRef::Master(m) => Some(Owner::Master(m)),
        _ => None,
    }
}

/// ThePage! of a sheet: its page sheet (in a master: the master's page sheet)
pub(crate) fn page_of(s: SheetRef) -> Option<SheetRef> {
    match owner_of(s)? {
        Owner::Page(p) => Some(SheetRef::Page(p)),
        Owner::Master(m) => Some(SheetRef::Master(m)),
    }
}

impl<'a, 'd> Ev<'a, 'd> {
    pub(crate) fn node(&self, i: u32) -> Node {
        self.p.nodes[i as usize]
    }

    pub(crate) fn args(&self, start: u32, n: u32) -> &[u32] {
        &self.p.args[start as usize..(start + n) as usize]
    }

    pub(crate) fn keep(&mut self, k: Keep) -> CellError {
        if self.keep.is_none() {
            self.keep = Some(k);
        }
        CellError::NotAvailable
    }

    /// evaluate node i
    pub(crate) fn eval(&mut self, i: u32) -> R {
        match self.node(i) {
            Node::Num(x) => Ok(Val::Num(x)),
            Node::Str(s) => Ok(Val::Str(Rc::from(&*self.p.strs[s as usize]))),
            Node::Bool(b) => Ok(Val::bool(b)),
            Node::Err(e) => Err(e),
            Node::Missing => Ok(Val::Missing),
            Node::Ref(r) => self.reference(r),
            Node::Neg(a) => {
                let v = self.eval(a)?;
                match v.plain() {
                    Val::Point(x, y) => Ok(Val::Point(-x, -y)),
                    v => Ok(Val::Num(-v.num(self.axis)?)),
                }
            }
            Node::Pct(a) => Ok(Val::Num(self.num(a)? / 100.0)),
            Node::Bin(op, a, b) => self.binary(op, a, b),
            Node::Call(f, start, n, sheet) => super::funcs::call(self, f, start, n, sheet),
        }
    }

    /// evaluate node i as a number
    pub(crate) fn num(&mut self, i: u32) -> Result<f64, CellError> {
        let v = self.eval(i)?;
        v.num(self.axis)
    }

    /// evaluate node i as a truth value
    pub(crate) fn truth(&mut self, i: u32) -> Result<bool, CellError> {
        Ok(self.num(i)? != 0.0)
    }

    /// evaluate node i as text
    pub(crate) fn text(&mut self, i: u32) -> Result<String, CellError> {
        let v = self.eval(i)?;
        v.text()
    }

    /// the RGB of a colour value (indices through the document's colour table)
    pub(crate) fn rgb(&self, v: &Val) -> Result<(u8, u8, u8), CellError> {
        match v.plain() {
            Val::Color(Color::Rgb(r, g, b)) => Ok((*r, *g, *b)),
            Val::Color(Color::Index(i)) => Ok(self.index_color(*i)),
            Val::Num(x) if *x >= 0.0 => Ok(self.index_color(*x as u32)),
            Val::Str(s) => Color::parse(s).map(|c| self.rgb(&Val::Color(c))).unwrap_or(Err(CellError::Value)),
            Val::Missing => Ok((0, 0, 0)),
            Val::Err(e) => Err(*e),
            _ => Err(CellError::Value),
        }
    }

    pub(crate) fn index_color(&self, i: u32) -> (u8, u8, u8) {
        if let Some(e) = self.doc.colors.iter().find(|c| c.ix == i) {
            if let Some(Color::Rgb(r, g, b)) = Color::parse(self.doc.str(e.rgb)) {
                return (r, g, b);
            }
        }
        super::color::DEFAULT_COLORS.get(i as usize).copied().unwrap_or((0, 0, 0))
    }

    fn binary(&mut self, op: BinOp, a: u32, b: u32) -> R {
        let x = self.eval(a)?;
        let y = self.eval(b)?;
        let ax = self.axis;
        match op {
            BinOp::Concat => {
                let mut s = x.text()?;
                s.push_str(&y.text()?);
                Ok(Val::Str(Rc::from(s)))
            }
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                let ord = compare(&x, &y, ax)?;
                Ok(Val::bool(match op {
                    BinOp::Eq => ord == core::cmp::Ordering::Equal,
                    BinOp::Ne => ord != core::cmp::Ordering::Equal,
                    BinOp::Lt => ord == core::cmp::Ordering::Less,
                    BinOp::Gt => ord == core::cmp::Ordering::Greater,
                    BinOp::Le => ord != core::cmp::Ordering::Greater,
                    _ => ord != core::cmp::Ordering::Less,
                }))
            }
            _ => {
                // an RGB colour plus or minus a number stays the colour (THEMEVAL("x")+1)
                if matches!(op, BinOp::Add | BinOp::Sub) {
                    match (x.plain(), y.plain()) {
                        (Val::Color(c @ Color::Rgb(..)), Val::Num(_) | Val::Missing) => return Ok(Val::Color(*c)),
                        (Val::Num(_) | Val::Missing, Val::Color(c @ Color::Rgb(..))) if op == BinOp::Add => return Ok(Val::Color(*c)),
                        (Val::Point(x1, y1), Val::Point(x2, y2)) => {
                            let s = if op == BinOp::Add { 1.0 } else { -1.0 };
                            return Ok(Val::Point(x1 + s * x2, y1 + s * y2));
                        }
                        _ => {}
                    }
                }
                let (p, q) = (x.num(ax)?, y.num(ax)?);
                let r = match op {
                    BinOp::Add => p + q,
                    BinOp::Sub => p - q,
                    BinOp::Mul => p * q,
                    BinOp::Div => {
                        if q == 0.0 {
                            return Err(CellError::DivZero);
                        }
                        p / q
                    }
                    _ => super::math::pow(p, q),
                };
                if r.is_finite() { Ok(Val::Num(r)) } else { Err(CellError::Num) }
            }
        }
    }

    // ---- references

    /// where a reference points for this evaluation: the sheet and, unless it names a sheet
    /// only, the cell
    pub(crate) fn target(&mut self, r: &CellRef) -> Result<(SheetRef, Option<CellAddr>), CellError> {
        let sheet = self.sheet_of(&r.sheet).ok_or(CellError::Ref)?;
        let addr = match &r.cell {
            CellPart::None => return Ok((sheet, None)),
            CellPart::Unknown(_) => {
                self.keep(Keep::Reference);
                return Err(CellError::Ref);
            }
            part => bind(self.doc, part),
        };
        Ok((sheet, addr))
    }

    fn reference(&mut self, i: u32) -> R {
        let p = self.p.clone();
        let r = &p.refs[i as usize];
        let sheet = self.sheet_of(&r.sheet).ok_or(CellError::Ref)?;
        let addr = match (&r.cell, p.addrs.get(i as usize)) {
            (CellPart::Unknown(_), _) => {
                self.keep(Keep::Reference);
                return Err(CellError::Ref);
            }
            (CellPart::None, _) => None,
            (_, Some(Some(a))) => Some(*a),
            // not bound yet, or a name the document did not have then
            (part, _) => bind(self.doc, part),
        };
        let Some(addr) = addr else {
            // a single cell whose name no sheet of the document has: its default
            return match &r.cell {
                CellPart::Single(name) => Ok(Val::Num(super::engine::default_single(name))),
                _ => Err(CellError::Ref),
            };
        };
        let v = super::engine::cell_value(self.doc, self.run, sheet, &addr);
        match v {
            Val::Err(e) => Err(e),
            // a cell holds a number: a point computed for it (PAR(PNT()) in EndY) is the
            // coordinate of that cell's axis, whoever reads it (Width reading EndY gets y)
            Val::Point(x, y) => Ok(Val::Num(if Axis::of(self.doc.str(addr.cell)) == Axis::Y { y } else { x })),
            v => Ok(v),
        }
    }

    /// the sheet a sheet name stands for in this evaluation
    pub(crate) fn sheet_of(&self, s: &SheetName) -> Option<SheetRef> {
        let doc = self.doc;
        match s {
            SheetName::Local => Some(self.sheet),
            SheetName::TheDoc => Some(SheetRef::Document),
            SheetName::ThePage => page_of(self.sheet),
            SheetName::Id(n) => self.shape_id(*n),
            SheetName::Named(name) => {
                let owner = owner_of(self.sheet)?;
                doc.all_shapes(owner).find(|&r| doc.shape(r).is_some_and(|s| s.name_u() == **name)).map(SheetRef::Shape)
            }
            SheetName::Page(name, sub) => {
                let p = doc.page_by_name_u(name).or_else(|| {
                    let sym = doc.sym(name)?;
                    doc.pages().find(|&p| doc.page(p).is_some_and(|pg| pg.header.name == Some(sym)))
                })?;
                match sub {
                    Some(id) => doc.shape_by_id(Owner::Page(p), *id).map(SheetRef::Shape),
                    None => Some(SheetRef::Page(p)),
                }
            }
            SheetName::Master(name, sub) => {
                let m = doc.master_by_name_u(name).or_else(|| {
                    let sym = doc.sym(name)?;
                    doc.masters().find(|&m| doc.master(m).is_some_and(|ms| ms.header.name == Some(sym)))
                })?;
                match sub {
                    Some(id) => doc.shape_by_id(Owner::Master(m), *id).map(SheetRef::Shape),
                    None => Some(SheetRef::Master(m)),
                }
            }
            SheetName::Style(name) => {
                let st = doc.style_by_name_u(name).or_else(|| {
                    let sym = doc.sym(name)?;
                    doc.styles().find(|&s| doc.style(s).is_some_and(|st| st.header.name == Some(sym)))
                })?;
                Some(SheetRef::Style(st))
            }
        }
    }

    // Sheet.N!: a shape of the same page or master; an inherited formula's N names a shape of
    // its master, which the instance holds as the subshape with MasterShape = N
    fn shape_id(&self, n: u32) -> Option<SheetRef> {
        let doc = self.doc;
        let owner = owner_of(self.sheet)?;
        let src_owner = owner_of(self.src);
        if let (Some(Owner::Master(m)), SheetRef::Shape(ctx)) = (src_owner, self.sheet) {
            if src_owner != Some(owner) {
                if let Some(r) = instance_shape(doc, ctx, m, n) {
                    return Some(SheetRef::Shape(r));
                }
            }
        }
        doc.shape_by_id(owner, n).map(SheetRef::Shape)
    }
}

// the shape of the instance around `ctx` that stands for shape `n` of master `m`
fn instance_shape(doc: &Document, ctx: crate::document::ShapeRef, m: crate::document::MasterRef, n: u32) -> Option<crate::document::ShapeRef> {
    let mid = doc.master(m)?.header.id;
    let tree = doc.tree(ctx.owner)?;
    // the instance root: ctx or the nearest group above it with Master = m
    let mut cur = Some(ctx.slot);
    let mut root = None;
    let mut depth = 0;
    while let (Some(s), true) = (cur, depth < 64) {
        let d = tree.get(s)?;
        if d.header.master == Some(mid) {
            root = Some(s);
            break;
        }
        cur = tree.parent(s);
        depth += 1;
    }
    let root = root?;
    let ms = doc.master(m)?;
    if ms.master_shape().and_then(|slot| ms.shapes.id(slot)) == Some(n) {
        return Some(crate::document::ShapeRef { owner: ctx.owner, slot: root });
    }
    // a subshape of the instance with MasterShape = n (groups before their subshapes)
    let mut stack = alloc::vec![root];
    while let Some(s) = stack.pop() {
        if s != root && tree.get(s).is_some_and(|d| d.header.master_shape == Some(n)) {
            return Some(crate::document::ShapeRef { owner: ctx.owner, slot: s });
        }
        stack.extend(tree.children(s).iter().copied());
    }
    None
}

/// the document's address of a cell part (None: a name the document's string table lacks,
/// so no sheet has the cell)
pub(crate) fn bind(doc: &Document, part: &CellPart) -> Option<CellAddr> {
    Some(match part {
        CellPart::None | CellPart::Unknown(_) => return None,
        CellPart::Single(c) => CellAddr::single(doc.sym(c)?),
        CellPart::Section { section, index, cell } => {
            CellAddr::section(SecKey { name: doc.sym(section)?, index: *index }, doc.sym(cell)?)
        }
        CellPart::Indexed { section, index, row, cell } => {
            CellAddr::row(SecKey { name: doc.sym(section)?, index: *index }, RowKey::Index(*row), doc.sym(cell)?)
        }
        CellPart::Named { section, row, cell } => {
            CellAddr::row(SecKey::one(doc.sym(section)?), RowKey::Named(doc.sym(row)?), doc.sym(cell)?)
        }
    })
}

/// compare two values: numbers (also colours, booleans) within a tiny tolerance, strings
/// exactly
pub(crate) fn compare(x: &Val, y: &Val, axis: Axis) -> Result<core::cmp::Ordering, CellError> {
    use core::cmp::Ordering;
    match (x.plain(), y.plain()) {
        (Val::Err(e), _) | (_, Val::Err(e)) => Err(*e),
        (Val::Str(a), Val::Str(b)) => Ok(a.cmp(b)),
        (Val::Color(Color::Rgb(..)), Val::Color(Color::Rgb(..))) => Ok(if x.plain() == y.plain() { Ordering::Equal } else { Ordering::Less }),
        (a, b) => {
            // fuzzy within 1e-9 ([MS-VSDX] 2.5.3.42 ff., FEQ/FGT ...)
            let (p, q) = (a.num(axis)?, b.num(axis)?);
            Ok(if (p - q).abs() <= 1e-9 {
                Ordering::Equal
            } else if p < q {
                Ordering::Less
            } else {
                Ordering::Greater
            })
        }
    }
}

/// a target of a reference node (GetRef, SETATREF, SETF): the node must be a reference
pub(crate) fn target_of(ev: &mut Ev, i: u32) -> Result<Target, CellError> {
    match ev.node(i) {
        Node::Ref(r) => {
            let cr = ev.p.refs[r as usize].clone();
            let (sheet, addr) = ev.target(&cr)?;
            Ok(Target { sheet, addr: Some(addr.ok_or(CellError::Ref)?) })
        }
        Node::Call(..) => match ev.eval(i)? {
            Val::Target(t) => Ok(t),
            _ => Err(CellError::Ref),
        },
        _ => Err(CellError::Ref),
    }
}
