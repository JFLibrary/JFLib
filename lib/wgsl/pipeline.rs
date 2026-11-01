// pipeline -- what WebGPU checks when it creates a pipeline from a compiled entry point
// (docs/WGSL.md "Overrides" and "Pipeline-Prüfungen")
//
//   output.constants(entry, &[(key, value)]) -> Result<Vec<(u32, f64)>, String>
//        the pipeline constants for lib/gpu (gpu::Constant { id, value }): keys as in WebGPU's
//        GPUProgrammableStage.constants (the override's name, or its @id as a decimal number
//        when it has one), values converted the way WebGPU converts them; every override the
//        entry point uses is then evaluated on the CPU (initializers with the front end's
//        constant evaluator) and every specialization constant of the module gets its final
//        value, the hidden ones (workgroup sizes, array counts) too.  Errors as WebGPU
//        reports them: an unknown key, a value out of the type's range, an override without
//        initializer left unset, an evaluation error (division by zero ...), a workgroup size
//        or array count of 0
//   output.workgroup_size(entry, constants) -> Result<[u32; 3], String>
//   output.check_pipeline(entry, constants, &Limits) -> Result<(), String>   the above plus the
//        limits (workgroup size and invocations, workgroup memory, colour targets, inter-stage
//        locations, immediate bytes), the storage texture formats WebGPU's core allows and the
//        function bodies the entry point reaches: with the overrides' values an expression may
//        become a constant that is an error (1 / a, sqrt(a), v[a], x << a, clamp(x, a, b) ...)
//   check_interface(vertex, fragment) -> Result<(), String>   every fragment input has a vertex
//        output at its location with the same component type and count (at least)
//   Limits::WEBGPU   WebGPU's default limits (what a browser gives without requiredLimits)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::builtins;
use crate::consteval::{self, check_float, comp, comps, CR};
use crate::ast::BinOp;
use crate::ir::*;
use crate::spirv::{EntryOut, Output, ResourceKind, SpecSource};
use crate::types::Types;
use crate::Module;

/// the limits a pipeline is checked against
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_compute_invocations: u32,
    pub max_workgroup_size: [u32; 3],
    pub max_workgroup_storage: u32,
    pub max_color_attachments: u32,
    pub max_inter_stage_variables: u32,
    pub max_immediate_size: u32,
    /// the device offers bgra8unorm-storage
    pub bgra8unorm_storage: bool,
    /// texture-formats-tier2: read_write storage textures of more formats
    pub texture_formats_tier2: bool,
    /// the sizes @subgroup_size may ask for (subgroupMinSize, subgroupMaxSize of the adapter)
    pub subgroup_min_size: u32,
    pub subgroup_max_size: u32,
}

impl Limits {
    /// WebGPU's default limits (Chrome 156 without requiredLimits)
    pub const WEBGPU: Limits = Limits {
        max_compute_invocations: 256,
        max_workgroup_size: [256, 256, 64],
        max_workgroup_storage: 16384,
        max_color_attachments: 8,
        max_inter_stage_variables: 16,
        max_immediate_size: 64,
        bgra8unorm_storage: false,
        texture_formats_tier2: false,
        subgroup_min_size: 4,
        subgroup_max_size: 128,
    };
}

/// the evaluator of module-scope override-expressions
struct Ev<'a> {
    m: &'a Module,
    t: Types,
    vals: Vec<Option<Value>>,
    busy: Vec<bool>,
}

impl<'a> Ev<'a> {
    fn new(m: &'a Module, vals: Vec<Option<Value>>) -> Ev<'a> {
        Ev { m, t: Types::from_module(&m.types, &m.structs), vals, busy: alloc::vec![false; m.overrides.len()] }
    }

    fn over(&mut self, o: u32) -> CR<Value> {
        let i = o as usize;
        if let Some(v) = self.vals.get(i).cloned().flatten() {
            return Ok(v);
        }
        let ov = &self.m.overrides[i];
        let Some(init) = ov.init else {
            return Err(format!("das Override `{}` hat keinen Wert: es hat keinen Initialisierer, die Pipeline muss es setzen", ov.name));
        };
        if self.busy[i] {
            return Err(format!("das Override `{}` hängt von sich selbst ab", ov.name));
        }
        self.busy[i] = true;
        let v = self.expr(init);
        self.busy[i] = false;
        let v = v.map_err(|e| format!("`{}`: {}", self.m.overrides[i].name, e))?;
        self.vals[i] = Some(v.clone());
        Ok(v)
    }

    fn binary(&mut self, op: BinOp, va: &Value, ta: TypeId, vb: &Value, tb: TypeId) -> CR<Value> {
        let (ka, kb) = (self.t.get(ta), self.t.get(tb));
        let mat = matches!(ka, Type::Matrix(..)) || matches!(kb, Type::Matrix(..));
        if op == BinOp::Mul && mat && !(self.t.is_scalar(ta) || self.t.is_scalar(tb)) {
            return consteval::mat_mul(&self.t, va, ta, vb, tb);
        }
        if self.t.is_scalar(ta) && !self.t.is_scalar(tb) && !op.is_comparison() && !matches!(op, BinOp::Shl | BinOp::Shr) {
            // scalar * vector / matrix: component-wise with the scalar first
            let n = consteval::shape(&self.t, tb).map(|s| s.0).unwrap_or(0);
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let ct = consteval::comp_type(&self.t, tb, i);
                let c = comp(&self.t, vb, tb, i);
                out.push(if self.t.is_scalar(ct) { consteval::binary_scalar(op, va, &c)? } else {
                    let n2 = consteval::shape(&self.t, ct).map(|s| s.0).unwrap_or(0);
                    let mut o2 = Vec::with_capacity(n2);
                    for j in 0..n2 {
                        o2.push(consteval::binary_scalar(op, va, &comp(&self.t, &c, ct, j))?);
                    }
                    Value::Comp(o2)
                });
            }
            return Ok(Value::Comp(out));
        }
        consteval::binary(&self.t, op, va, vb, ta, tb)
    }

    fn construct(&self, ty: TypeId, vals: Vec<(Value, TypeId)>) -> Value {
        let mut out = Vec::with_capacity(vals.len());
        let vector = matches!(self.t.get(ty), Type::Vector(..));
        for (v, at) in vals {
            if vector && matches!(self.t.get(at), Type::Vector(..)) {
                out.extend(comps(&self.t, &v, at));
            } else {
                out.push(v);
            }
        }
        Value::Comp(out)
    }

    fn expr(&mut self, e: ExprId) -> CR<Value> {
        let m = self.m;
        let ex = &m.exprs[e as usize];
        let ty = ex.ty;
        let tyof = |x: ExprId| m.exprs[x as usize].ty;
        match &ex.kind {
            ExprKind::Const(v) => Ok(v.clone()),
            ExprKind::Override(o) => self.over(*o),
            ExprKind::Unary(op, a) => {
                let v = self.expr(*a)?;
                consteval::unary(&self.t, *op, &v, tyof(*a))
            }
            ExprKind::Binary(op, a, b) => {
                let va = self.expr(*a)?;
                if matches!(op, BinOp::LogAnd | BinOp::LogOr) {
                    if va == Value::Bool(*op == BinOp::LogOr) {
                        return Ok(va);
                    }
                    return self.expr(*b);
                }
                let vb = self.expr(*b)?;
                self.binary(*op, &va, tyof(*a), &vb, tyof(*b))
            }
            ExprKind::Construct(args) => {
                let mut vals = Vec::with_capacity(args.len());
                for &a in args {
                    vals.push((self.expr(a)?, tyof(a)));
                }
                Ok(self.construct(ty, vals))
            }
            ExprKind::Splat(a) => {
                let v = self.expr(*a)?;
                let n = match self.t.get(ty) {
                    Type::Vector(n, _) => n as usize,
                    _ => 1,
                };
                Ok(Value::Comp(alloc::vec![v; n]))
            }
            ExprKind::Convert(a) => {
                let v = self.expr(*a)?;
                consteval::convert(&mut self.t, &v, tyof(*a), ty, true)
            }
            ExprKind::Bitcast(a) => {
                let v = self.expr(*a)?;
                consteval::bitcast(&self.t, &v, tyof(*a), ty)
            }
            ExprKind::Builtin(b, args) => {
                let mut vs = Vec::with_capacity(args.len());
                for &a in args {
                    vs.push((self.expr(a)?, tyof(a)));
                }
                builtins::eval(&mut self.t, *b, &vs)
            }
            ExprKind::Index(a, i) => {
                let v = self.expr(*a)?;
                let k = self.expr(*i)?;
                let k = consteval::as_int(&k).unwrap_or(-1);
                let n = consteval::shape(&self.t, tyof(*a)).map(|s| s.0).unwrap_or(0);
                if k < 0 || k as usize >= n {
                    return Err(format!("der Index {} liegt außerhalb von 0..{}", k, n.saturating_sub(1)));
                }
                Ok(comp(&self.t, &v, tyof(*a), k as usize))
            }
            ExprKind::Member(a, k) => {
                let v = self.expr(*a)?;
                Ok(comp(&self.t, &v, tyof(*a), *k as usize))
            }
            ExprKind::Swizzle(a, cs, n) => {
                let v = self.expr(*a)?;
                let at = tyof(*a);
                Ok(Value::Comp(cs[..*n as usize].iter().map(|&c| comp(&self.t, &v, at, c as usize)).collect()))
            }
            _ => Err(String::from("das ist kein Override-Ausdruck")),
        }
    }
}

/// the function bodies with the overrides' values: what the substitution of constants would
/// make a const-expression error at pipeline creation (Tint substitutes them and checks again)
struct Body<'a> {
    ev: Ev<'a>,
    f: &'a Function,
}

impl<'a> Body<'a> {
    fn ty(&self, e: ExprId) -> TypeId {
        self.f.exprs.get(e as usize).map_or(BOOL, |x| x.ty)
    }

    fn scalar_int(&self, t: TypeId) -> bool {
        matches!(self.ev.t.scalar_of(t), Some(Scalar::I32 | Scalar::U32))
    }

    /// the element count of what Index(b, _) indexes (None: decided at run time)
    fn index_bound(&mut self, b: ExprId) -> Option<i128> {
        let m = self.ev.m;
        let t = crate::spirv_func::store_of(m, self.ty(b));
        match *m.ty(t) {
            Type::Vector(n, _) => Some(n as i128),
            Type::Matrix(c, ..) => Some(c as i128),
            Type::SwizzleView { k, .. } => Some(k as i128),
            Type::Array { count: ArrayCount::Fixed(n), .. } => Some(n as i128),
            Type::Array { count: ArrayCount::Override(o), .. } => self.ev.over(o).ok().as_ref().and_then(consteval::as_int),
            Type::Array { count: ArrayCount::Expr(x), .. } => self.ev.expr(x).ok().as_ref().and_then(consteval::as_int),
            _ => None,
        }
    }

    /// the expression's value when the overrides make it a constant; Err: it is an error then
    fn expr(&mut self, e: ExprId) -> CR<Option<Value>> {
        let f = self.f;
        let Some(ex) = f.exprs.get(e as usize) else { return Ok(None) };
        let ty = ex.ty;
        Ok(match &ex.kind {
            ExprKind::Const(v) => Some(v.clone()),
            ExprKind::Override(o) => Some(self.ev.over(*o)?),
            ExprKind::Global(_) | ExprKind::Local(_) | ExprKind::Param(_) => None,
            ExprKind::Load(a) | ExprKind::AddrOf(a) | ExprKind::Deref(a) => {
                self.expr(*a)?;
                None
            }
            ExprKind::Unary(op, a) => match self.expr(*a)? {
                Some(v) => Some(consteval::unary(&self.ev.t, *op, &v, self.ty(*a))?),
                None => None,
            },
            ExprKind::Binary(op, a, b) => {
                let va = self.expr(*a)?;
                if matches!(op, BinOp::LogAnd | BinOp::LogOr) {
                    // the right side of a decided short-circuit is not evaluated
                    if va == Some(Value::Bool(*op == BinOp::LogOr)) {
                        return Ok(va);
                    }
                }
                let vb = self.expr(*b)?;
                let (ta, tb) = (self.ty(*a), self.ty(*b));
                match (va, vb) {
                    (Some(x), Some(y)) if matches!(op, BinOp::LogAnd | BinOp::LogOr) => {
                        let _ = x;
                        Some(y)
                    }
                    (Some(x), Some(y)) => Some(self.ev.binary(*op, &x, ta, &y, tb)?),
                    (_, Some(y)) => {
                        self.right_operand(*op, &y, tb)?;
                        None
                    }
                    _ => None,
                }
            }
            ExprKind::Construct(args) => {
                let mut vals = Vec::with_capacity(args.len());
                let mut known = true;
                for &a in args {
                    match self.expr(a)? {
                        Some(v) => vals.push((v, self.ty(a))),
                        None => known = false,
                    }
                }
                if known { Some(self.ev.construct(ty, vals)) } else { None }
            }
            ExprKind::Splat(a) => {
                let n = match self.ev.t.get(ty) {
                    Type::Vector(n, _) => n as usize,
                    _ => 1,
                };
                self.expr(*a)?.map(|v| Value::Comp(alloc::vec![v; n]))
            }
            ExprKind::Convert(a) => match self.expr(*a)? {
                Some(v) => {
                    let from = self.ty(*a);
                    Some(consteval::convert(&mut self.ev.t, &v, from, ty, true)?)
                }
                None => None,
            },
            ExprKind::Bitcast(a) => match self.expr(*a)? {
                Some(v) => Some(consteval::bitcast(&self.ev.t, &v, self.ty(*a), ty)?),
                None => None,
            },
            ExprKind::Member(a, k) => self.expr(*a)?.map(|v| comp(&self.ev.t, &v, self.ty(*a), *k as usize)),
            ExprKind::Swizzle(a, cs, n) => {
                let at = self.ty(*a);
                self.expr(*a)?.map(|v| Value::Comp(cs[..*n as usize].iter().map(|&c| comp(&self.ev.t, &v, at, c as usize)).collect()))
            }
            ExprKind::Index(a, i) => {
                let va = self.expr(*a)?;
                let vi = self.expr(*i)?;
                let k = vi.as_ref().and_then(consteval::as_int);
                if let (Some(k), Some(n)) = (k, self.index_bound(*a)) {
                    if k < 0 || k >= n {
                        return Err(format!("der Index {} liegt außerhalb von 0..{}", k, n - 1));
                    }
                }
                match (va, k) {
                    (Some(v), Some(k)) => Some(comp(&self.ev.t, &v, self.ty(*a), k as usize)),
                    _ => None,
                }
            }
            ExprKind::Call(_, args) => {
                for &a in args {
                    self.expr(a)?;
                }
                None
            }
            ExprKind::Builtin(b, args) => {
                let mut vals = Vec::with_capacity(args.len());
                for &a in args {
                    vals.push((self.expr(a)?, self.ty(a)));
                }
                builtins::arg_checks(&self.ev.t, *b, &vals).map_err(|x| x.2)?;
                let is_const = builtins::lookup(builtins::name(*b)).is_some_and(|x| x.1 & builtins::CONST != 0);
                if is_const && vals.iter().all(|v| v.0.is_some()) {
                    let a: Vec<(Value, TypeId)> = vals.into_iter().map(|(v, t)| (v.unwrap_or(Value::Zero), t)).collect();
                    Some(builtins::eval(&mut self.ev.t, *b, &a)?)
                } else {
                    None
                }
            }
        })
    }

    /// `x / a`, `x % a`, `x << a`, `x >> a` with a runtime x and a constant right side
    fn right_operand(&self, op: BinOp, v: &Value, t: TypeId) -> CR<()> {
        if !self.scalar_int(t) {
            return Ok(());
        }
        let list = if self.ev.t.is_scalar(t) { alloc::vec![v.clone()] } else { comps(&self.ev.t, v, t) };
        let ints = || list.iter().filter_map(consteval::as_int);
        if matches!(op, BinOp::Div | BinOp::Rem) && ints().any(|x| x == 0) {
            return Err(String::from("ganzzahlige Division durch null"));
        }
        if matches!(op, BinOp::Shl | BinOp::Shr) && ints().any(|x| x >= 32) {
            return Err(String::from("die Schiebeweite muss kleiner als 32 sein"));
        }
        Ok(())
    }

    fn stmts(&mut self, list: &[Stmt]) -> CR<()> {
        for s in list {
            match s {
                Stmt::Block(v) => self.stmts(v)?,
                Stmt::Var { init, .. } => {
                    if let Some(e) = init {
                        self.expr(*e)?;
                    }
                }
                Stmt::Let { init, .. } | Stmt::Phony(init) | Stmt::Call(init) => {
                    self.expr(*init)?;
                }
                Stmt::Assign { lhs, rhs } => {
                    self.expr(*lhs)?;
                    self.expr(*rhs)?;
                }
                Stmt::Compound { lhs, op, rhs } => {
                    self.expr(*lhs)?;
                    if let Some(v) = self.expr(*rhs)? {
                        self.right_operand(*op, &v, self.ty(*rhs))?;
                    }
                }
                Stmt::If { cond, then, else_ } => {
                    self.expr(*cond)?;
                    self.stmts(then)?;
                    self.stmts(else_)?;
                }
                Stmt::Switch { selector, cases } => {
                    self.expr(*selector)?;
                    for c in cases {
                        self.stmts(&c.body)?;
                    }
                }
                Stmt::Loop { body, continuing, break_if } => {
                    self.stmts(body)?;
                    self.stmts(continuing)?;
                    if let Some(e) = break_if {
                        self.expr(*e)?;
                    }
                }
                Stmt::Return(e) => {
                    if let Some(e) = e {
                        self.expr(*e)?;
                    }
                }
                Stmt::Break | Stmt::Continue | Stmt::Discard => {}
            }
        }
        Ok(())
    }
}

/// every function an entry point reaches, with the overrides' values
fn check_bodies(m: &Module, ep: &EntryPoint, vals: &[Option<Value>]) -> Result<(), String> {
    let mut seen = alloc::vec![false; m.functions.len()];
    let mut work = alloc::vec![ep.func];
    while let Some(fi) = work.pop() {
        let Some(f) = m.functions.get(fi as usize) else { continue };
        if core::mem::replace(&mut seen[fi as usize], true) {
            continue;
        }
        work.extend(f.calls.iter().copied());
        if f.overrides.is_empty() {
            continue;
        }
        let mut b = Body { ev: Ev::new(m, vals.to_vec()), f };
        b.stmts(&f.body).map_err(|e| format!("`{}`: {}", f.name, e))?;
    }
    Ok(())
}

/// the defaults of every override (None: no initializer, or it does not evaluate)
pub(crate) fn default_values(m: &Module) -> Vec<Option<Value>> {
    let mut ev = Ev::new(m, alloc::vec![None; m.overrides.len()]);
    for o in 0..m.overrides.len() as u32 {
        let _ = ev.over(o);
    }
    ev.vals
}

/// a WebGPU pipeline constant converted to the override's type (WebIDL: bool = not 0 and not
/// NaN; integers [EnforceRange]; floats finite and in range, rounded to nearest)
fn convert_constant(name: &str, x: f64, s: Scalar) -> Result<Value, String> {
    let bad = || format!("die Pipeline-Konstante `{}` = {} lässt sich nicht als {} darstellen", name, x, s.name());
    match s {
        Scalar::Bool => Ok(Value::Bool(x != 0.0 && !x.is_nan())),
        Scalar::I32 | Scalar::U32 => {
            if !x.is_finite() {
                return Err(bad());
            }
            let t = crate::math::trunc(x);
            if s == Scalar::I32 {
                if !(-2147483648.0..=2147483647.0).contains(&t) {
                    return Err(bad());
                }
                Ok(Value::I32(t as i32))
            } else {
                if !(0.0..=4294967295.0).contains(&t) {
                    return Err(bad());
                }
                Ok(Value::U32(t as u32))
            }
        }
        _ => check_float(x, s).ok_or_else(bad),
    }
}

/// the overrides given as WebGPU pipeline constants (by name, or by @id when there is one)
fn given_values(m: &Module, given: &[(&str, f64)]) -> Result<Vec<Option<Value>>, String> {
    let mut vals: Vec<Option<Value>> = alloc::vec![None; m.overrides.len()];
    for &(key, x) in given {
        let hit = m.overrides.iter().position(|o| match o.id {
            Some(n) => key.parse::<u32>().ok() == Some(n as u32) && key.bytes().all(|c| c.is_ascii_digit()),
            None => o.name == key,
        });
        let Some(i) = hit else {
            return Err(format!("die Pipeline-Konstante `{}` gibt es im Modul nicht (Overrides mit @id werden über die Zahl gesetzt)", key));
        };
        if vals[i].is_some() {
            return Err(format!("die Pipeline-Konstante `{}` steht zweimal", key));
        }
        let s = crate::spirv::scalar_of(m, m.overrides[i].ty);
        vals[i] = Some(convert_constant(key, x, s)?);
    }
    Ok(vals)
}

/// every override value an entry point needs (given or from its initializer); checks the
/// workgroup size and the override-sized arrays (> 0)
pub(crate) fn override_values(m: &Module, ep: &EntryPoint, given: &[(&str, f64)]) -> Result<Vec<Option<Value>>, String> {
    let vals = given_values(m, given)?;
    let mut ev = Ev::new(m, vals);
    for &o in &ep.overrides {
        ev.over(o)?;
    }
    if let Some(ws) = ep.workgroup_size {
        for w in ws {
            if let WgSize::Override(e) = w {
                let x = ev.expr(e)?;
                if consteval::as_int(&x).unwrap_or(0) <= 0 {
                    return Err(format!("die Workgroup-Größe {} muss größer als 0 sein", consteval::fmt(&x)));
                }
            }
        }
    }
    for g in &ep.globals {
        let gl = &m.globals[*g as usize];
        if let Some(e) = array_count_expr(m, gl.ty) {
            let x = match e {
                Err(o) => ev.over(o)?,
                Ok(e) => ev.expr(e)?,
            };
            if consteval::as_int(&x).unwrap_or(0) <= 0 {
                return Err(format!("die Länge {} des Arrays `{}` muss größer als 0 sein", consteval::fmt(&x), gl.name));
            }
        }
    }
    Ok(ev.vals)
}

/// the override-sized array count inside a workgroup variable's type: Ok(expression) or
/// Err(override)
fn array_count_expr(m: &Module, t: TypeId) -> Option<Result<ExprId, u32>> {
    match *m.ty(t) {
        Type::Array { count: ArrayCount::Override(o), .. } => Some(Err(o)),
        Type::Array { count: ArrayCount::Expr(e), .. } => Some(Ok(e)),
        Type::Array { elem, .. } => array_count_expr(m, elem),
        _ => None,
    }
}

/// a module-scope override-expression as u32 (workgroup sizes, array counts)
pub(crate) fn eval_u32(m: &Module, e: ExprId, vals: &[Option<Value>]) -> Result<u32, String> {
    let mut ev = Ev::new(m, vals.to_vec());
    let v = ev.expr(e)?;
    match consteval::as_int(&v) {
        Some(x) if x > 0 && x <= u32::MAX as i128 => Ok(x as u32),
        _ => Err(format!("{} ist keine gültige Größe", consteval::fmt(&v))),
    }
}

pub(crate) fn eval_i64(m: &Module, e: ExprId, vals: &[Option<Value>]) -> Result<i64, String> {
    let mut ev = Ev::new(m, vals.to_vec());
    let v = ev.expr(e)?;
    consteval::as_int(&v).map(|x| x as i64).ok_or_else(|| format!("{} ist keine Ganzzahl", consteval::fmt(&v)))
}

/// an override-expression's value with the overrides' defaults (None: one has no default)
pub(crate) fn eval_u32_default(m: &Module, e: ExprId) -> Option<u32> {
    let vals = given_values(m, &[]).ok()?;
    let v = eval_i64(m, e, &vals).ok()?;
    u32::try_from(v).ok()
}

fn value_f64(v: &Value) -> f64 {
    match *v {
        Value::Bool(b) => b as u8 as f64,
        Value::I32(x) => x as f64,
        Value::U32(x) => x as f64,
        Value::F32(x) | Value::F16(x) => x as f64,
        Value::AInt(x) => x as f64,
        Value::AFloat(x) => x,
        _ => 0.0,
    }
}

impl Output {
    fn entry_or(&self, entry: &str) -> Result<&EntryOut, String> {
        self.entry(entry).ok_or_else(|| format!("das Modul hat keinen Einstiegspunkt `{}`", entry))
    }

    /// the pipeline constants (SpecId, value) of an entry point for WebGPU-style constants (see
    /// the module comment); pass them to lib/gpu as Constant { id, value }
    pub fn constants(&self, entry: &str, given: &[(&str, f64)]) -> Result<Vec<(u32, f64)>, String> {
        let e = self.entry_or(entry)?;
        let m = &self.module;
        let ep = m.entry_point(entry).ok_or_else(|| String::from("?"))?;
        let vals = override_values(m, ep, given)?;
        let mut out = Vec::with_capacity(e.spec.len());
        for &(sid, src) in &e.spec {
            let x = match src {
                SpecSource::Override(o) => vals.get(o as usize).cloned().flatten().map(|v| value_f64(&v)).unwrap_or(0.0),
                SpecSource::Expr(x) => eval_u32(m, x, &vals)? as f64,
            };
            out.push((sid, x));
        }
        Ok(out)
    }

    /// the workgroup size of a compute entry point with these pipeline constants
    pub fn workgroup_size(&self, entry: &str, given: &[(&str, f64)]) -> Result<[u32; 3], String> {
        let m = &self.module;
        let ep = m.entry_point(entry).ok_or_else(|| format!("das Modul hat keinen Einstiegspunkt `{}`", entry))?;
        let Some(ws) = ep.workgroup_size else { return Err(format!("`{}` ist kein Compute-Shader", entry)) };
        let vals = override_values(m, ep, given)?;
        let mut out = [1u32; 3];
        for (i, w) in ws.iter().enumerate() {
            out[i] = match *w {
                WgSize::Const(n) => n,
                WgSize::Override(e) => eval_u32(m, e, &vals)?,
            };
        }
        Ok(out)
    }

    /// the @subgroup_size of a compute entry point with these pipeline constants (None: it has
    /// none)
    pub fn subgroup_size(&self, entry: &str, given: &[(&str, f64)]) -> Result<Option<u32>, String> {
        let m = &self.module;
        let ep = m.entry_point(entry).ok_or_else(|| format!("das Modul hat keinen Einstiegspunkt `{}`", entry))?;
        match ep.subgroup_size {
            None => Ok(None),
            Some(WgSize::Const(n)) => Ok(Some(n)),
            Some(WgSize::Override(e)) => {
                let vals = override_values(m, ep, given)?;
                let v = eval_i64(m, e, &vals)?;
                if v < 1 || v > u32::MAX as i64 {
                    return Err(format!("`@subgroup_size` muss mindestens 1 sein, nicht {}", v));
                }
                Ok(Some(v as u32))
            }
        }
    }

    /// what WebGPU checks when it creates a pipeline from this entry point (module comment)
    pub fn check_pipeline(&self, entry: &str, given: &[(&str, f64)], lim: &Limits) -> Result<(), String> {
        let e = self.entry_or(entry)?;
        let m = &self.module;
        let ep = m.entry_point(entry).ok_or_else(|| String::from("?"))?;
        let vals = override_values(m, ep, given)?;
        check_bodies(m, ep, &vals)?;
        if ep.stage == Stage::Compute {
            let ws = self.workgroup_size(entry, given)?;
            for (i, (&w, &max)) in ws.iter().zip(lim.max_workgroup_size.iter()).enumerate() {
                if w > max {
                    return Err(format!("die Workgroup-Größe {:?} überschreitet in {} das Limit {}", ws, ["x", "y", "z"][i], max));
                }
            }
            let n = ws[0] as u64 * ws[1] as u64 * ws[2] as u64;
            if n > lim.max_compute_invocations as u64 {
                return Err(format!("die Workgroup hat {} Aufrufe, das Limit ist {}", n, lim.max_compute_invocations));
            }
            let mut bytes = 0u64;
            for g in &ep.globals {
                let gl = &m.globals[*g as usize];
                if gl.space == AddressSpace::Workgroup {
                    bytes += size_with(m, gl.ty, &vals).div_ceil(16) * 16;
                }
            }
            if bytes > lim.max_workgroup_storage as u64 {
                return Err(format!("die Workgroup-Variablen brauchen {} Byte, das Limit ist {}", bytes, lim.max_workgroup_storage));
            }
            if let Some(sz) = self.subgroup_size(entry, given)? {
                if sz.count_ones() != 1 {
                    return Err(format!("`@subgroup_size` muss eine Zweierpotenz sein, nicht {}", sz));
                }
                if ws[0] % sz != 0 {
                    return Err(format!("die Workgroup-Größe in x ({}) ist kein Vielfaches von `@subgroup_size` ({})", ws[0], sz));
                }
                if sz < lim.subgroup_min_size || sz > lim.subgroup_max_size {
                    return Err(format!("`@subgroup_size` ({}) liegt außerhalb der erlaubten {} bis {}", sz, lim.subgroup_min_size, lim.subgroup_max_size));
                }
            }
        }
        for io in &e.outputs {
            if let IoKind::Location { location, .. } = io.kind {
                let max = if ep.stage == Stage::Fragment { lim.max_color_attachments } else { lim.max_inter_stage_variables };
                if location >= max {
                    return Err(format!("die Ausgabe `{}` hat @location({}), erlaubt sind 0 bis {}", io.name, location, max - 1));
                }
            }
        }
        if ep.stage == Stage::Fragment {
            for io in &e.inputs {
                if let IoKind::Location { location, .. } = io.kind {
                    if location >= lim.max_inter_stage_variables {
                        return Err(format!("die Eingabe `{}` hat @location({}), erlaubt sind 0 bis {}", io.name, location, lim.max_inter_stage_variables - 1));
                    }
                }
            }
        }
        if e.immediate_size > lim.max_immediate_size {
            return Err(format!("die immediate-Daten haben {} Byte, das Limit ist {}", e.immediate_size, lim.max_immediate_size));
        }
        for r in &e.resources {
            if let ResourceKind::StorageTexture { format, access, .. } = r.kind {
                if format == TexelFormat::Bgra8unorm && !lim.bgra8unorm_storage {
                    return Err(format!("`{}`: bgra8unorm als Speichertextur braucht das Gerätefeature bgra8unorm-storage", r.name));
                }
                if access == Access::ReadWrite && !read_write_format(format, lim.texture_formats_tier2) {
                    return Err(format!("`{}`: das Format {} erlaubt keinen read_write-Zugriff (nur r32float, r32sint, r32uint)", r.name, format.name()));
                }
            }
        }
        Ok(())
    }
}

/// the formats a read_write storage texture may have (core: r32*; tier2 adds more)
fn read_write_format(f: TexelFormat, tier2: bool) -> bool {
    use TexelFormat as T;
    matches!(f, T::R32float | T::R32sint | T::R32uint)
        || tier2 && matches!(f, T::Rgba8unorm | T::Rgba8uint | T::Rgba8sint | T::Rgba16uint | T::Rgba16sint | T::Rgba16float
            | T::Rgba32uint | T::Rgba32sint | T::Rgba32float | T::R8unorm | T::R8uint | T::R8sint | T::R16uint | T::R16sint
            | T::R16float | T::Rg8unorm | T::Rg8uint | T::Rg8sint | T::Rg16uint | T::Rg16sint | T::Rg16float | T::Rg32uint
            | T::Rg32sint | T::Rg32float | T::Rgb10a2uint | T::Rgb10a2unorm | T::Rg11b10ufloat)
}

/// the size of a type in memory with override-sized arrays evaluated
fn size_with(m: &Module, t: TypeId, vals: &[Option<Value>]) -> u64 {
    match *m.ty(t) {
        Type::Array { count, stride, .. } => {
            let n = match count {
                ArrayCount::Fixed(n) => n as u64,
                ArrayCount::Override(o) => vals.get(o as usize).cloned().flatten().and_then(|v| consteval::as_int(&v)).unwrap_or(0).max(0) as u64,
                ArrayCount::Expr(e) => eval_u32(m, e, vals).unwrap_or(0) as u64,
                ArrayCount::Runtime => 0,
            };
            n * stride as u64
        }
        Type::Struct(i) => m.structs[i as usize].size as u64,
        _ => {
            let t2 = Types::from_module(&m.types, &m.structs);
            t2.layout(t).map(|l| l.1 as u64).unwrap_or(4)
        }
    }
}

/// the scalar kind and component count of a stage variable's type
fn io_shape(m: &Module, t: TypeId) -> (Scalar, u8) {
    match *m.ty(t) {
        Type::Scalar(s) => (s, 1),
        Type::Vector(n, s) => (s, n),
        _ => (Scalar::Bool, 0),
    }
}

/// WebGPU's inter-stage rules for a vertex and a fragment entry point: every fragment input
/// at a location has a vertex output there with the same component type, at least as many
/// components and the same interpolation
pub fn check_interface(m: &Module, vs: &EntryOut, fs: &EntryOut) -> Result<(), String> {
    for i in &fs.inputs {
        let IoKind::Location { location, interp, .. } = i.kind else { continue };
        let Some(o) = vs.outputs.iter().find(|o| matches!(o.kind, IoKind::Location { location: l, .. } if l == location)) else {
            return Err(format!("die Fragment-Eingabe bei @location({}) hat keine Vertex-Ausgabe", location));
        };
        let (si, ni) = io_shape(m, i.ty);
        let (so, no) = io_shape(m, o.ty);
        if si != so || no < ni {
            return Err(format!("@location({}): die Vertex-Ausgabe `{}` ({}) passt nicht zur Fragment-Eingabe `{}` ({})", location,
                               o.name, m.type_name(o.ty), i.name, m.type_name(i.ty)));
        }
        if let IoKind::Location { interp: oi, .. } = o.kind {
            if oi != interp {
                return Err(format!("@location({}): die Interpolation der Vertex-Ausgabe und der Fragment-Eingabe unterscheidet sich", location));
            }
        }
    }
    Ok(())
}
