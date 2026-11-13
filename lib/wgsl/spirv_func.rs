// spirv_func -- the back end's function bodies (spirv.rs has the module level): the entry
// point wrapper, statements with structured control flow, expressions, memory, calls
//
//   lower_entry(m, ep, ...) -> EntryOut   one SPIR-V module: the wrapper (a void function
//        without parameters: workgroup memory zeroed, private initializers that are
//        override-expressions stored, the stage inputs loaded, the WGSL entry function called,
//        its result stored to the outputs), then every function the call tree needs
//   Fx            the lowering of one arena (a function's, or the module's in the wrapper)
//
// Values (V): an SSA id; a pointer / reference as its root memory object plus an access chain
// (PtrVal; the chain is emitted where the pointer is used); a swizzle view (swizzle
// assignment); a handle (texture or sampler: the module-scope variable, loaded where it is
// used, in the block of its use as Vulkan wants for OpSampledImage).
//
// Pointer and handle parameters: SPIR-V's logical addressing takes only memory object
// declarations as pointer arguments, so a function is specialized per call shape (FnKey):
// a pointer into a module-scope variable is fixed in the specialization and its dynamic
// indices become u32 parameters; a pointer into a function variable passes the variable and
// the indices (Tint's DirectVariableAccess does the same).  WGSL has no recursion, so the
// specializations are finite.
//
// WGSL's runtime semantics where SPIR-V's differ: integer division and remainder by 0 and
// MIN / -1 (the divisor becomes 1), shift amounts modulo the bit width, saturating float to
// integer conversions, short-circuit && and ||, zeroed function variables at their
// declaration and private / workgroup memory.  Robustness: indices clamped (spirv.rs).

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use shader::{op, spv, Bi, Dec, Id, Mode, Model, Sc};

use crate::ast::BinOp;
use crate::ir::*;
use crate::spirv::*;
use crate::Module;

/// a lowered value (module comment)
#[derive(Clone, Debug)]
pub(crate) enum V {
    Id(Id),
    Ptr(PtrVal),
    Swz(PtrVal, [u8; 4], u8),
    Handle(u32),
    None,
}

/// the lowering of one expression arena and its statements
pub(crate) struct Fx<'g, 'a> {
    pub g: &'g mut Gen<'a>,
    pub exprs: &'a [Expr],
    pub locals: &'a [Local],
    pub params: Vec<V>,
    lets: Vec<Option<V>>,
    vars: Vec<Option<Id>>,
    breaks: Vec<Id>,
    conts: Vec<Id>,
    reached: BTreeSet<u32>,
}

/// the store type of a reference or pointer type (other types: themselves)
pub(crate) fn store_of(m: &Module, t: TypeId) -> TypeId {
    match *m.ty(t) {
        Type::Reference { store, .. } | Type::Pointer { store, .. } => store,
        _ => t,
    }
}

impl<'g, 'a> Fx<'g, 'a> {
    pub fn new(g: &'g mut Gen<'a>, exprs: &'a [Expr], locals: &'a [Local], params: Vec<V>) -> Fx<'g, 'a> {
        let n = locals.len();
        Fx { g, exprs, locals, params, lets: alloc::vec![None; n], vars: alloc::vec![None; n], breaks: Vec::new(), conts: Vec::new(),
             reached: BTreeSet::new() }
    }

    pub fn m(&self) -> &'a Module {
        self.g.m
    }

    pub fn tyof(&self, e: ExprId) -> TypeId {
        self.exprs[e as usize].ty
    }

    pub fn sty(&mut self, t: TypeId) -> Id {
        self.g.ty(t)
    }

    pub fn scalar(&self, t: TypeId) -> Scalar {
        scalar_of(self.g.m, t)
    }

    /// component count of a scalar (1) or vector
    pub fn width(&self, t: TypeId) -> u8 {
        match *self.g.m.ty(t) {
            Type::Vector(n, _) => n,
            _ => 1,
        }
    }

    pub fn cu(&mut self, x: u32) -> Id {
        self.g.b.const_u32(x)
    }

    /// a scalar constant broadcast to t's shape (t scalar: the constant itself)
    pub fn splat_c(&mut self, c: Id, t: TypeId) -> Id {
        let n = self.width(t);
        if n == 1 {
            return c;
        }
        let vt = self.sty(t);
        self.g.b.const_composite(vt, &alloc::vec![c; n as usize])
    }

    /// a scalar value broadcast to t's shape
    pub fn splat_v(&mut self, v: Id, t: TypeId) -> Id {
        let n = self.width(t);
        if n == 1 {
            return v;
        }
        let vt = self.sty(t);
        self.g.b.construct(vt, &alloc::vec![v; n as usize])
    }

    /// the constant of scalar s with value x (ints wrap) in t's shape
    pub fn num(&mut self, t: TypeId, x: f64) -> Id {
        let c = match self.scalar(t) {
            Scalar::I32 => self.g.b.const_i32(x as i64 as i32),
            Scalar::U32 => self.g.b.const_u32(x as i64 as u32),
            Scalar::F16 => {
                let h = self.g.scalar_ty(Scalar::F16);
                self.g.b.const_scalar(h, &[f16_bits(x as f32) as u32])
            }
            Scalar::Bool => self.g.b.const_bool(x != 0.0),
            _ => self.g.b.const_f32(x as f32),
        };
        self.splat_c(c, t)
    }

    /// the bool type of t's shape
    pub fn bool_t(&mut self, t: TypeId) -> TypeId {
        let n = self.width(t);
        self.g.vec_tid(n, Scalar::Bool)
    }

    // ---- control flow helpers

    /// branch to target from an open block (and note that target is reached)
    fn br(&mut self, target: Id) {
        if self.g.b.block_open() {
            self.g.b.branch(target);
            self.reached.insert(target.0);
        }
    }

    /// start a merge block; unreachable ones end at once with OpUnreachable
    fn begin_merge(&mut self, l: Id) {
        self.g.b.begin_block(l);
        if !self.reached.contains(&l.0) {
            self.g.b.unreachable();
        }
    }

    /// if c { then } as a selection construct (c: an open block's bool)
    pub fn if_then(&mut self, c: Id, then: impl FnOnce(&mut Self)) {
        let (t, merge) = (self.g.b.label(), self.g.b.label());
        self.g.b.selection_merge(merge, 0);
        self.g.b.branch_cond(c, t, merge);
        self.reached.insert(merge.0);
        self.g.b.begin_block(t);
        then(self);
        self.br(merge);
        self.begin_merge(merge);
    }

    /// run f unless the invocation was discarded (Basis: Tint's DemoteToHelper for stores,
    /// atomics and texture stores)
    pub fn guarded(&mut self, f: impl FnOnce(&mut Self)) {
        match self.g.discard_flag {
            Some(flag) => {
                let d = self.g.b.load(flag);
                let nd = self.g.b.lnot(d);
                self.if_then(nd, f);
            }
            None => f(self),
        }
    }

    /// like guarded for an operation with a result (a zero result when skipped)
    pub fn guarded_value(&mut self, ty: Id, f: impl FnOnce(&mut Self) -> Id) -> Id {
        match self.g.discard_flag {
            Some(flag) => {
                let z = self.g.b.const_null(ty);
                let tmp = self.g.b.local(ty, None);
                self.g.b.store(tmp, z);
                let d = self.g.b.load(flag);
                let nd = self.g.b.lnot(d);
                self.if_then(nd, |fx| {
                    let r = f(fx);
                    fx.g.b.store(tmp, r);
                });
                self.g.b.load(tmp)
            }
            None => f(self),
        }
    }

    // ---- memory

    /// the SPIR-V pointer of p (an access chain unless the chain is empty)
    pub fn ptr_id(&mut self, p: &PtrVal) -> Id {
        if p.chain.is_empty() {
            return p.base;
        }
        let idx: Vec<Id> = p.chain.iter().map(|s| match *s {
            Step::Const(c) => self.g.b.const_u32(c),
            Step::Dyn(id) => id,
        }).collect();
        self.g.b.access(p.base, &idx)
    }

    pub fn load_ptr(&mut self, p: &PtrVal) -> Id {
        let ptr = self.ptr_id(p);
        let v = self.g.b.load(ptr);
        match self.g.relayout(p.sc, p.ty) {
            Relayout::Copy => {
                let vt = self.g.ty(p.ty);
                self.g.b.emit(op::CopyLogical, Some(vt), &[v.0])
            }
            Relayout::Manual => self.manual_layout(v, p.ty, true),
            Relayout::None => v,
        }
    }

    /// v of WGSL type t rebuilt member by member as the laid-out type (to_lay) or the plain one
    pub fn manual_layout(&mut self, v: Id, t: TypeId, to_lay: bool) -> Id {
        let m = self.m();
        let rt = if to_lay { self.g.ty(t) } else { self.g.ty_plain(t) };
        if rt == self.g.b.type_of(v) {
            return v;
        }
        let parts: Vec<Id> = match *m.ty(t) {
            Type::Array { elem, count: ArrayCount::Fixed(n), .. } => (0..n).map(|k| {
                let e = self.g.b.extract(v, &[k]);
                self.manual_layout(e, elem, to_lay)
            }).collect(),
            Type::Struct(i) => {
                let tys: Vec<TypeId> = m.structs[i as usize].members.iter().map(|mm| mm.ty).collect();
                tys.iter().enumerate().map(|(k, &mt)| {
                    let e = self.g.b.extract(v, &[k as u32]);
                    self.manual_layout(e, mt, to_lay)
                }).collect()
            }
            _ => return v,
        };
        self.g.b.construct(rt, &parts)
    }

    pub fn store_ptr(&mut self, p: &PtrVal, v: Id) {
        let v = match self.g.relayout(p.sc, p.ty) {
            Relayout::Copy => {
                let mt = self.g.ty_mem(p.ty);
                self.g.b.emit(op::CopyLogical, Some(mt), &[v.0])
            }
            Relayout::Manual => self.manual_layout(v, p.ty, false),
            Relayout::None => v,
        };
        if p.sc == Sc::StorageBuffer && self.g.discard_flag.is_some() {
            let p = p.clone();
            self.guarded(|fx| {
                let ptr = fx.ptr_id(&p);
                fx.g.b.store(ptr, v);
            });
            return;
        }
        let ptr = self.ptr_id(p);
        self.g.b.store(ptr, v);
    }

    /// an index value as u32 (i32 indices reinterpreted: negative ones become large and are
    /// clamped)
    pub fn index_u32(&mut self, e: ExprId) -> (Id, Option<u32>) {
        let c = match &self.exprs[e as usize].kind {
            ExprKind::Const(Value::I32(x)) => Some(*x as u32),
            ExprKind::Const(Value::U32(x)) => Some(*x),
            _ => None,
        };
        let v = self.val(e);
        if self.scalar(self.tyof(e)) == Scalar::I32 {
            let u = self.g.b.ty_u32();
            return (self.g.b.bitcast(u, v), c);
        }
        (v, c)
    }

    /// UMin(i, n - 1) for a constant bound n
    fn clamp_const(&mut self, i: Id, n: u32) -> Id {
        let hi = self.cu(n.saturating_sub(1));
        let u = self.g.b.ty_u32();
        self.g.b.ext(u, spv::glsl::UMin, &[i, hi])
    }

    /// UMin(i, n - 1) for an u32 value n
    fn clamp_dyn(&mut self, i: Id, n: Id) -> Id {
        let one = self.cu(1);
        let hi = self.g.b.isub(n, one);
        let u = self.g.b.ty_u32();
        self.g.b.ext(u, spv::glsl::UMin, &[i, hi])
    }

    /// the element count of a runtime-sized array reference (OpArrayLength)
    pub fn array_length(&mut self, p: &PtrVal) -> Id {
        let member = match p.chain.last() {
            Some(Step::Const(k)) => *k,
            _ => 0,
        };
        let u = self.g.b.ty_u32();
        self.g.b.emit(op::ArrayLength, Some(u), &[p.base.0, member])
    }

    /// an index into a reference: the chain grows by the (clamped) index
    fn index_ref(&mut self, mut p: PtrVal, i: ExprId, rty: TypeId) -> PtrVal {
        let m = self.m();
        let (iv, ci) = self.index_u32(i);
        let robust = self.g.robust;
        let step = match *m.ty(p.ty) {
            Type::Vector(n, _) | Type::Matrix(n, _, _) => match ci {
                Some(c) if c < n as u32 => Step::Const(c),
                _ => Step::Dyn(if robust { self.clamp_const(iv, n as u32) } else { iv }),
            },
            Type::Array { count, .. } => match count {
                ArrayCount::Fixed(n) => match ci {
                    Some(c) if c < n => Step::Const(c),
                    _ => Step::Dyn(if robust { self.clamp_const(iv, n) } else { iv }),
                },
                ArrayCount::Runtime => {
                    if robust {
                        let len = self.array_length(&p);
                        Step::Dyn(self.clamp_dyn(iv, len))
                    } else {
                        Step::Dyn(iv)
                    }
                }
                ArrayCount::Override(o) => {
                    if robust {
                        let n = self.g.override_value(o);
                        let n = if self.scalar(m.overrides[o as usize].ty) == Scalar::I32 {
                            let u = self.g.b.ty_u32();
                            self.g.b.bitcast(u, n)
                        } else {
                            n
                        };
                        Step::Dyn(self.clamp_dyn(iv, n))
                    } else {
                        Step::Dyn(iv)
                    }
                }
                ArrayCount::Expr(x) => {
                    if robust {
                        let n = self.g.hidden(x);
                        Step::Dyn(self.clamp_dyn(iv, n))
                    } else {
                        Step::Dyn(iv)
                    }
                }
            },
            _ => Step::Dyn(iv),
        };
        p.chain.push(step);
        p.ty = store_of(m, rty);
        p
    }

    /// an index into a value: vectors with OpVectorExtractDynamic, arrays and matrices through
    /// a function variable
    fn index_value(&mut self, v: Id, bt: TypeId, i: ExprId) -> Id {
        let m = self.m();
        let (iv, ci) = self.index_u32(i);
        let n = match *m.ty(bt) {
            Type::Vector(n, _) | Type::Matrix(n, _, _) => n as u32,
            Type::Array { count: ArrayCount::Fixed(n), .. } => n,
            _ => 1,
        };
        if let Some(c) = ci {
            if c < n {
                return self.g.b.extract(v, &[c]);
            }
        }
        let iv = if self.g.robust { self.clamp_const(iv, n) } else { iv };
        if let Type::Vector(..) = *m.ty(bt) {
            return self.g.b.extract_dynamic(v, iv);
        }
        let vt = self.sty(bt);
        let tmp = self.g.b.local(vt, None);
        self.g.b.store(tmp, v);
        let p = self.g.b.access(tmp, &[iv]);
        self.g.b.load(p)
    }

    /// the OpVariable of a function-scope var (created at its first use, in the entry block)
    fn var(&mut self, l: u32) -> Id {
        if let Some(v) = self.vars[l as usize] {
            return v;
        }
        let t = self.locals[l as usize].ty;
        let vt = self.sty(t);
        let v = self.g.b.local(vt, None);
        let name = self.locals[l as usize].name.clone();
        self.g.b.name(v, &name);
        self.vars[l as usize] = Some(v);
        v
    }

    // ---- expressions

    /// an expression as an SSA value (V::Id)
    pub fn val(&mut self, e: ExprId) -> Id {
        match self.expr(e) {
            V::Id(id) => id,
            V::Ptr(p) => self.load_ptr(&p),
            V::Swz(p, cs, n) => {
                let v = self.load_ptr(&p);
                let comps: Vec<u32> = cs[..n as usize].iter().map(|&c| c as u32).collect();
                self.g.b.shuffle(v, v, &comps)
            }
            V::Handle(g) => self.handle(g),
            V::None => {
                let b = self.g.b.ty_bool();
                self.g.b.const_null(b)
            }
        }
    }

    /// a texture or sampler loaded from its variable
    pub fn handle(&mut self, g: u32) -> Id {
        let gv = self.g.global(g);
        self.g.b.load(gv.var)
    }

    /// the module-scope variable of a handle expression
    pub fn handle_of(&mut self, e: ExprId) -> u32 {
        match self.expr(e) {
            V::Handle(g) => g,
            V::Ptr(p) => p.global.unwrap_or(0),
            _ => 0,
        }
    }

    pub fn expr(&mut self, e: ExprId) -> V {
        let m = self.m();
        let ex = &self.exprs[e as usize];
        let ty = ex.ty;
        match &ex.kind {
            ExprKind::Const(v) => V::Id(self.g.constant(v, ty)),
            ExprKind::Override(o) => V::Id(self.g.override_value(*o)),
            ExprKind::Global(g) => V::Ptr(self.g.global_ptr(*g)),
            ExprKind::Local(l) => {
                let loc = &self.locals[*l as usize];
                match loc.kind {
                    LocalKind::Var => {
                        let lt = loc.ty;
                        let v = self.var(*l);
                        V::Ptr(PtrVal { base: v, global: None, sc: Sc::Function, root: lt, chain: Vec::new(), ty: lt })
                    }
                    LocalKind::Let => self.lets[*l as usize].clone().unwrap_or(V::None),
                }
            }
            ExprKind::Param(k) => self.params.get(*k as usize).cloned().unwrap_or(V::None),
            ExprKind::Load(a) => match self.expr(*a) {
                V::Ptr(p) => {
                    if p.sc == Sc::UniformConstant {
                        V::Handle(p.global.unwrap_or(0))
                    } else {
                        V::Id(self.load_ptr(&p))
                    }
                }
                V::Swz(p, cs, n) => {
                    let v = self.load_ptr(&p);
                    let comps: Vec<u32> = cs[..n as usize].iter().map(|&c| c as u32).collect();
                    V::Id(self.g.b.shuffle(v, v, &comps))
                }
                other => other,
            },
            ExprKind::AddrOf(a) | ExprKind::Deref(a) => self.expr(*a),
            ExprKind::Index(a, i) => {
                let at = self.tyof(*a);
                match self.expr(*a) {
                    V::Ptr(p) => V::Ptr(self.index_ref(p, *i, ty)),
                    V::Id(v) => V::Id(self.index_value(v, at, *i)),
                    other => other,
                }
            }
            ExprKind::Member(a, k) => match self.expr(*a) {
                V::Ptr(mut p) => {
                    p.chain.push(Step::Const(*k));
                    p.ty = store_of(m, ty);
                    V::Ptr(p)
                }
                V::Id(v) => V::Id(self.g.b.extract(v, &[*k])),
                other => other,
            },
            ExprKind::Swizzle(a, cs, n) => match self.expr(*a) {
                V::Ptr(p) => V::Swz(p, *cs, *n),
                V::Id(v) => {
                    let comps: Vec<u32> = cs[..*n as usize].iter().map(|&c| c as u32).collect();
                    V::Id(self.g.b.shuffle(v, v, &comps))
                }
                other => other,
            },
            ExprKind::Unary(op, a) => {
                let v = self.val(*a);
                let r = match op {
                    UnaryOp::Neg => {
                        if self.scalar(ty).is_float() {
                            self.g.b.fneg(v)
                        } else {
                            self.g.b.ineg(v)
                        }
                    }
                    UnaryOp::Not => self.g.b.lnot(v),
                    UnaryOp::BitNot => self.g.b.bnot(v),
                };
                V::Id(r)
            }
            ExprKind::Binary(op, a, b) => {
                let (a, b, op) = (*a, *b, *op);
                if matches!(op, BinOp::LogAnd | BinOp::LogOr) {
                    return V::Id(self.short_circuit(op, a, b));
                }
                let (ta, tb) = (self.tyof(a), self.tyof(b));
                let va = self.val(a);
                let vb = self.val(b);
                V::Id(self.binop(op, va, ta, vb, tb, ty))
            }
            ExprKind::Construct(args) => {
                let vt = self.sty(ty);
                if args.is_empty() {
                    return V::Id(self.g.b.const_null(vt));
                }
                let ids: Vec<Id> = args.iter().map(|&x| self.val(x)).collect();
                V::Id(self.g.b.construct(vt, &ids))
            }
            ExprKind::Splat(a) => {
                let v = self.val(*a);
                V::Id(self.splat_v(v, ty))
            }
            ExprKind::Convert(a) => {
                let from = self.tyof(*a);
                let v = self.val(*a);
                V::Id(self.convert(v, from, ty))
            }
            ExprKind::Bitcast(a) => {
                let from = self.tyof(*a);
                let v = self.val(*a);
                if from == ty {
                    return V::Id(v);
                }
                let vt = self.sty(ty);
                V::Id(self.g.b.bitcast(vt, v))
            }
            ExprKind::Call(f, args) => self.call(*f, args, ty),
            ExprKind::Builtin(b, args) => crate::spirv_builtin::builtin(self, *b, args, ty),
        }
    }

    /// a && b, a || b: b is evaluated only when needed
    fn short_circuit(&mut self, op: BinOp, a: ExprId, b: ExprId) -> Id {
        let bt = self.g.b.ty_bool();
        let tmp = self.g.b.local(bt, None);
        let va = self.val(a);
        self.g.b.store(tmp, va);
        let (rhs, merge) = (self.g.b.label(), self.g.b.label());
        self.g.b.selection_merge(merge, 0);
        if op == BinOp::LogAnd {
            self.g.b.branch_cond(va, rhs, merge);
        } else {
            self.g.b.branch_cond(va, merge, rhs);
        }
        self.reached.insert(merge.0);
        self.g.b.begin_block(rhs);
        let vb = self.val(b);
        self.g.b.store(tmp, vb);
        self.br(merge);
        self.begin_merge(merge);
        self.g.b.load(tmp)
    }

    /// a binary operator on lowered operands (ta, tb: their WGSL types; rt: the result's)
    pub fn binop(&mut self, op: BinOp, va: Id, ta: TypeId, vb: Id, tb: TypeId, rt: TypeId) -> Id {
        let m = self.m();
        let s = self.scalar(ta);
        let float = s.is_float();
        let signed = s == Scalar::I32;
        let b = &mut self.g.b;
        match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => {}
            BinOp::And | BinOp::Or | BinOp::Xor => {
                return if s == Scalar::Bool {
                    match op {
                        BinOp::And => b.land(va, vb),
                        BinOp::Or => b.lor(va, vb),
                        _ => b.binary(op::LogicalNotEqual, va, vb),
                    }
                } else {
                    match op {
                        BinOp::And => b.band(va, vb),
                        BinOp::Or => b.bor(va, vb),
                        _ => b.bxor(va, vb),
                    }
                };
            }
            BinOp::Shl | BinOp::Shr => {
                let mask = self.num(tb, 31.0);
                let n = self.g.b.band(vb, mask);
                return match (op, signed) {
                    (BinOp::Shl, _) => self.g.b.shl(va, n),
                    (_, true) => self.g.b.sar(va, n),
                    _ => self.g.b.shr(va, n),
                };
            }
            BinOp::LogAnd => return b.land(va, vb),
            BinOp::LogOr => return b.lor(va, vb),
            _ => {
                let opc = match (op, s) {
                    (BinOp::Eq, Scalar::Bool) => op::LogicalEqual,
                    (BinOp::Ne, Scalar::Bool) => op::LogicalNotEqual,
                    (BinOp::Eq, _) if float => op::FOrdEqual,
                    (BinOp::Ne, _) if float => op::FUnordNotEqual,
                    (BinOp::Lt, _) if float => op::FOrdLessThan,
                    (BinOp::Le, _) if float => op::FOrdLessThanEqual,
                    (BinOp::Gt, _) if float => op::FOrdGreaterThan,
                    (BinOp::Ge, _) if float => op::FOrdGreaterThanEqual,
                    (BinOp::Eq, _) => op::IEqual,
                    (BinOp::Ne, _) => op::INotEqual,
                    (BinOp::Lt, Scalar::I32) => op::SLessThan,
                    (BinOp::Le, Scalar::I32) => op::SLessThanEqual,
                    (BinOp::Gt, Scalar::I32) => op::SGreaterThan,
                    (BinOp::Ge, Scalar::I32) => op::SGreaterThanEqual,
                    (BinOp::Lt, _) => op::ULessThan,
                    (BinOp::Le, _) => op::ULessThanEqual,
                    (BinOp::Gt, _) => op::UGreaterThan,
                    _ => op::UGreaterThanEqual,
                };
                return b.cmp(opc, va, vb);
            }
        }
        // arithmetic
        let (ka, kb) = (*m.ty(ta), *m.ty(tb));
        match (ka, kb) {
            (Type::Matrix(..), Type::Matrix(..)) if op == BinOp::Mul => return self.g.b.mat_mat(va, vb),
            (Type::Matrix(c, ..), Type::Matrix(..)) => {
                // + and - column by column
                let mut cols = Vec::new();
                for k in 0..c as u32 {
                    let x = self.g.b.extract(va, &[k]);
                    let y = self.g.b.extract(vb, &[k]);
                    cols.push(if op == BinOp::Add { self.g.b.fadd(x, y) } else { self.g.b.fsub(x, y) });
                }
                let mt = self.sty(rt);
                return self.g.b.construct(mt, &cols);
            }
            (Type::Matrix(..), Type::Vector(..)) => return self.g.b.mat_vec(va, vb),
            (Type::Vector(..), Type::Matrix(..)) => return self.g.b.vec_mat(va, vb),
            (Type::Matrix(..), Type::Scalar(_)) => return self.g.b.mat_scale(va, vb),
            (Type::Scalar(_), Type::Matrix(..)) => return self.g.b.mat_scale(vb, va),
            // (only multiply: the checker splats the scalar of every other mixed operator)
            (Type::Vector(..), Type::Scalar(_)) if float && op == BinOp::Mul => return self.g.b.vec_scale(va, vb),
            (Type::Scalar(_), Type::Vector(..)) if float && op == BinOp::Mul => return self.g.b.vec_scale(vb, va),
            _ => {}
        }
        if float {
            let b = &mut self.g.b;
            return match op {
                BinOp::Add => b.fadd(va, vb),
                BinOp::Sub => b.fsub(va, vb),
                BinOp::Mul => b.fmul(va, vb),
                BinOp::Div => b.fdiv(va, vb),
                _ => b.binary(op::FRem, va, vb),
            };
        }
        match op {
            BinOp::Add => self.g.b.iadd(va, vb),
            BinOp::Sub => self.g.b.isub(va, vb),
            BinOp::Mul => self.g.b.imul(va, vb),
            _ => {
                // WGSL: x / 0 = x, x % 0 = 0, MIN / -1 = MIN, MIN % -1 = 0: the divisor becomes 1
                let zero = self.num(tb, 0.0);
                let one = self.num(tb, 1.0);
                let mut bad = self.g.b.ieq(vb, zero);
                if signed {
                    let min = self.num(ta, -2147483648.0);
                    let neg1 = self.num(tb, -1.0);
                    let a_min = self.g.b.ieq(va, min);
                    let b_neg = self.g.b.ieq(vb, neg1);
                    let ovf = self.g.b.land(a_min, b_neg);
                    bad = self.g.b.lor(bad, ovf);
                }
                let d = self.g.b.select(bad, one, vb);
                match (op, signed) {
                    (BinOp::Div, true) => self.g.b.sdiv(va, d),
                    (BinOp::Div, false) => self.g.b.udiv(va, d),
                    // a - (a / d) * d: the truncated remainder without OpSRem, which a
                    // driver got wrong for negative divisors (NVIDIA: 1 % -1 = 1), as Tint does
                    (_, true) => {
                        let q = self.g.b.sdiv(va, d);
                        let p = self.g.b.imul(q, d);
                        self.g.b.isub(va, p)
                    }
                    _ => self.g.b.umod(va, d),
                }
            }
        }
    }

    /// a numeric conversion from type `from` to `to` (same shape)
    pub fn convert(&mut self, v: Id, from: TypeId, to: TypeId) -> Id {
        if from == to {
            return v;
        }
        let m = self.m();
        if let (Type::Matrix(c, ..), Type::Matrix(..)) = (*m.ty(from), *m.ty(to)) {
            let (cf, ct) = (self.g.comp_type(from, 0), self.g.comp_type(to, 0));
            let cols: Vec<Id> = (0..c as u32).map(|k| {
                let x = self.g.b.extract(v, &[k]);
                self.convert(x, cf, ct)
            }).collect();
            let mt = self.sty(to);
            return self.g.b.construct(mt, &cols);
        }
        let (sf, st) = (self.scalar(from), self.scalar(to));
        let tt = self.sty(to);
        match (sf, st) {
            (a, b) if a == b => v,
            (Scalar::Bool, _) => {
                let one = self.num(to, 1.0);
                let zero = self.num(to, 0.0);
                self.g.b.select(v, one, zero)
            }
            (_, Scalar::Bool) => {
                let zero = self.num(from, 0.0);
                if sf.is_float() { self.g.b.fne(v, zero) } else { self.g.b.ine(v, zero) }
            }
            (Scalar::I32, Scalar::U32) | (Scalar::U32, Scalar::I32) => self.g.b.bitcast(tt, v),
            (Scalar::I32, _) => self.g.b.s2f(tt, v),
            (Scalar::U32, _) => self.g.b.u2f(tt, v),
            (_, Scalar::I32) | (_, Scalar::U32) => {
                // saturating: clamp to the range representable in the source type, then
                // truncate (NMax / NMin: NaN becomes the lower bound)
                let (lo, hi) = match (sf, st) {
                    (Scalar::F16, Scalar::I32) => (-65504.0, 65504.0),
                    (Scalar::F16, _) => (0.0, 65504.0),
                    (_, Scalar::I32) => (-2147483648.0, 2147483520.0),
                    _ => (0.0, 4294967040.0),
                };
                let (l, h) = (self.num(from, lo), self.num(from, hi));
                let ft = self.sty(from);
                let x = self.g.b.ext(ft, 80, &[v, l]);
                let x = self.g.b.ext(ft, 79, &[x, h]);
                if st == Scalar::I32 { self.g.b.f2s(tt, x) } else { self.g.b.f2u(tt, x) }
            }
            _ => self.g.b.convert(op::FConvert, tt, v),
        }
    }

    // ---- calls

    fn call(&mut self, f: u32, args: &[ExprId], ty: TypeId) -> V {
        let mut keys = Vec::with_capacity(args.len());
        let mut ids: Vec<Id> = Vec::new();
        for &a in args {
            match self.expr(a) {
                V::Id(id) => {
                    keys.push(ArgKey::Value);
                    ids.push(id);
                }
                V::Handle(g) => keys.push(ArgKey::Handle(g)),
                V::Ptr(p) => {
                    let shape: Vec<Option<u32>> = p.chain.iter().map(|s| match *s {
                        Step::Const(c) => Some(c),
                        Step::Dyn(_) => None,
                    }).collect();
                    match p.global {
                        Some(g) => keys.push(ArgKey::Global(g, shape)),
                        None => {
                            keys.push(ArgKey::Local(p.sc as u32, p.root, shape));
                            ids.push(p.base);
                        }
                    }
                    for s in &p.chain {
                        if let Step::Dyn(id) = s {
                            ids.push(*id);
                        }
                    }
                }
                V::Swz(..) | V::None => keys.push(ArgKey::Value),
            }
        }
        let fid = self.g.func(FnKey { f, args: keys });
        let rt = if ty == VOID { self.g.b.ty_void() } else { self.sty(ty) };
        let mut w = alloc::vec![fid.0];
        w.extend(ids.iter().map(|i| i.0));
        let r = self.g.b.emit(op::FunctionCall, Some(rt), &w);
        if ty == VOID { V::None } else { V::Id(r) }
    }

    // ---- statements

    pub fn stmts(&mut self, list: &[Stmt]) {
        for s in list {
            if !self.g.b.block_open() {
                return;
            }
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Block(b) => self.stmts(b),
            Stmt::Var { local, init } => {
                let t = self.locals[*local as usize].ty;
                let v = match init {
                    Some(e) => self.val(*e),
                    None => {
                        let vt = self.sty(t);
                        self.g.b.const_null(vt)
                    }
                };
                let var = self.var(*local);
                self.g.b.store(var, v);
            }
            Stmt::Let { local, init } => {
                let v = self.expr(*init);
                let v = match v {
                    V::Swz(..) => V::Id(match v {
                        V::Swz(p, cs, n) => {
                            let x = self.load_ptr(&p);
                            let comps: Vec<u32> = cs[..n as usize].iter().map(|&c| c as u32).collect();
                            self.g.b.shuffle(x, x, &comps)
                        }
                        _ => unreachable!(),
                    }),
                    other => other,
                };
                self.lets[*local as usize] = Some(v);
            }
            Stmt::Assign { lhs, rhs } => {
                let l = self.expr(*lhs);
                let r = self.val(*rhs);
                self.assign(l, r);
            }
            Stmt::Compound { lhs, op, rhs } => {
                let l = self.expr(*lhs);
                let (old, lt) = match &l {
                    V::Ptr(p) => (self.load_ptr(p), p.ty),
                    V::Swz(p, cs, n) => {
                        let v = self.load_ptr(p);
                        let comps: Vec<u32> = cs[..*n as usize].iter().map(|&c| c as u32).collect();
                        let s = self.scalar(p.ty);
                        (self.g.b.shuffle(v, v, &comps), self.g.vec_tid(*n, s))
                    }
                    _ => return,
                };
                let rt = self.tyof(*rhs);
                let r = self.val(*rhs);
                let x = self.binop(*op, old, lt, r, rt, lt);
                self.assign(l, x);
            }
            Stmt::Phony(e) | Stmt::Call(e) => {
                let _ = self.expr(*e);
            }
            Stmt::If { cond, then, else_ } => {
                let c = self.val(*cond);
                let (t, e, merge) = (self.g.b.label(), self.g.b.label(), self.g.b.label());
                self.g.b.selection_merge(merge, 0);
                if else_.is_empty() {
                    self.g.b.branch_cond(c, t, merge);
                    self.reached.insert(merge.0);
                } else {
                    self.g.b.branch_cond(c, t, e);
                }
                self.g.b.begin_block(t);
                self.stmts(then);
                self.br(merge);
                if !else_.is_empty() {
                    self.g.b.begin_block(e);
                    self.stmts(else_);
                    self.br(merge);
                }
                self.begin_merge(merge);
            }
            Stmt::Switch { selector, cases } => {
                let sel = self.val(*selector);
                let merge = self.g.b.label();
                let labels: Vec<Id> = cases.iter().map(|_| self.g.b.label()).collect();
                let default = cases.iter().position(|c| c.default).map(|i| labels[i]).unwrap_or(merge);
                if default == merge {
                    self.reached.insert(merge.0);
                }
                let mut targets = Vec::new();
                for (c, &l) in cases.iter().zip(labels.iter()) {
                    for &v in &c.selectors {
                        targets.push((v as i32 as u32, l));
                    }
                }
                self.g.b.selection_merge(merge, 0);
                self.g.b.switch(sel, default, &targets);
                for (c, &l) in cases.iter().zip(labels.iter()) {
                    self.g.b.begin_block(l);
                    self.breaks.push(merge);
                    self.stmts(&c.body);
                    self.breaks.pop();
                    self.br(merge);
                }
                self.begin_merge(merge);
            }
            Stmt::Loop { body, continuing, break_if } => {
                let (header, bl, cont, merge) = (self.g.b.label(), self.g.b.label(), self.g.b.label(), self.g.b.label());
                self.br(header);
                self.g.b.begin_block(header);
                self.g.b.loop_merge(merge, cont, 0);
                self.g.b.branch(bl);
                self.g.b.begin_block(bl);
                self.breaks.push(merge);
                self.conts.push(cont);
                self.stmts(body);
                self.br(cont);
                self.breaks.pop();
                self.conts.pop();
                self.g.b.begin_block(cont);
                if self.reached.contains(&cont.0) {
                    self.stmts(continuing);
                    if self.g.b.block_open() {
                        match break_if {
                            Some(c) => {
                                let v = self.val(*c);
                                self.g.b.branch_cond(v, merge, header);
                                self.reached.insert(merge.0);
                            }
                            None => self.g.b.branch(header),
                        }
                    }
                } else {
                    self.g.b.branch(header);
                }
                self.begin_merge(merge);
            }
            Stmt::Break => {
                if let Some(&t) = self.breaks.last() {
                    self.br(t);
                }
            }
            Stmt::Continue => {
                if let Some(&t) = self.conts.last() {
                    self.br(t);
                }
            }
            Stmt::Return(e) => match e {
                Some(e) => {
                    let v = self.val(*e);
                    self.g.b.ret_value(v);
                }
                None => self.g.b.ret(),
            },
            Stmt::Discard => {
                if let Some(flag) = self.g.discard_flag {
                    let t = self.g.b.const_bool(true);
                    self.g.b.store(flag, t);
                } else {
                    self.g.b.demote();
                }
            }
        }
    }

    /// store r through l (a swizzle view writes its components: the whole vector, read,
    /// modified, written back)
    fn assign(&mut self, l: V, r: Id) {
        match l {
            V::Ptr(p) => self.store_ptr(&p, r),
            V::Swz(p, cs, n) => {
                let old = self.load_ptr(&p);
                let w = self.width(p.ty) as u32;
                let mut comps: Vec<u32> = (0..w).collect();
                for (i, &c) in cs[..n as usize].iter().enumerate() {
                    comps[c as usize] = w + i as u32;
                }
                let v = self.g.b.shuffle(old, r, &comps);
                self.store_ptr(&p, v);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- functions and entry points

/// lower one specialized function (FnKey) into g
fn lower_fn(g: &mut Gen, key: &FnKey) {
    let m = g.m;
    let f = &m.functions[key.f as usize];
    let fid = g.funcs[key];
    let u = g.b.ty_u32();
    let mut ptys = Vec::new();
    // per WGSL parameter: what it becomes (the index of its first SPIR-V parameter)
    let mut plan: Vec<(usize, &ArgKey)> = Vec::new();
    for (k, p) in f.params.iter().enumerate() {
        let a = &key.args[k];
        plan.push((ptys.len(), a));
        match a {
            ArgKey::Value => ptys.push(g.ty(p.ty)),
            ArgKey::Handle(_) => {}
            ArgKey::Global(_, shape) => ptys.extend(shape.iter().filter(|s| s.is_none()).map(|_| u)),
            ArgKey::Local(sc, root, shape) => {
                let sc = sc_of(*sc);
                let rt = g.ty_in(*root, sc);
                ptys.push(g.b.ty_ptr(sc, rt));
                ptys.extend(shape.iter().filter(|s| s.is_none()).map(|_| u));
            }
        }
    }
    let ret = match f.ret {
        Some(t) => g.ty(t),
        None => g.b.ty_void(),
    };
    let (_, pids) = g.b.begin_function_id(fid, ret, &ptys);
    g.b.name(fid, &f.name);
    let mut params = Vec::new();
    for (k, (first, a)) in plan.iter().enumerate() {
        let pt = f.params[k].ty;
        let store = store_of(m, pt);
        let chain = |shape: &Vec<Option<u32>>, mut next: usize| -> Vec<Step> {
            shape.iter().map(|s| match s {
                Some(c) => Step::Const(*c),
                None => {
                    let id = pids[next];
                    next += 1;
                    Step::Dyn(id)
                }
            }).collect()
        };
        let v = match a {
            ArgKey::Value => V::Id(pids[*first]),
            ArgKey::Handle(gi) => V::Handle(*gi),
            ArgKey::Global(gi, shape) => {
                let base = g.global_ptr(*gi);
                V::Ptr(PtrVal { chain: chain(shape, *first), ty: store, ..base })
            }
            ArgKey::Local(sc, root, shape) => V::Ptr(PtrVal { base: pids[*first], global: None, sc: sc_of(*sc), root: *root,
                                                             chain: chain(shape, *first + 1), ty: store }),
        };
        params.push(v);
    }
    let mut fx = Fx::new(g, &f.exprs, &f.locals, params);
    fx.stmts(&f.body);
    if fx.g.b.block_open() {
        if f.ret.is_none() {
            fx.g.b.ret();
        } else {
            fx.g.b.unreachable();
        }
    }
    g.b.end_function();
}

/// a storage class from its number (the ones pointers of WGSL can have)
fn sc_of(n: u32) -> Sc {
    match n {
        x if x == Sc::Private as u32 => Sc::Private,
        x if x == Sc::Workgroup as u32 => Sc::Workgroup,
        x if x == Sc::Uniform as u32 => Sc::Uniform,
        x if x == Sc::StorageBuffer as u32 => Sc::StorageBuffer,
        x if x == Sc::PushConstant as u32 => Sc::PushConstant,
        x if x == Sc::UniformConstant as u32 => Sc::UniformConstant,
        _ => Sc::Function,
    }
}

/// the storage class of a WGSL address space
fn space_sc(s: AddressSpace) -> Sc {
    match s {
        AddressSpace::Private => Sc::Private,
        AddressSpace::Workgroup => Sc::Workgroup,
        AddressSpace::Uniform => Sc::Uniform,
        AddressSpace::Storage => Sc::StorageBuffer,
        AddressSpace::Immediate => Sc::PushConstant,
        AddressSpace::Handle => Sc::UniformConstant,
        AddressSpace::Function => Sc::Function,
    }
}

/// a library module (spirv::compile_library): every function, uncalled, beside an empty
/// compute entry point
pub(crate) fn lower_library(m: &Module, opt: &CompileOptions, values: Vec<Option<Value>>, overrides: &[OverrideInfo]) -> Result<EntryOut, String> {
    let ep = EntryPoint {
        name: String::from("library"),
        func: 0,
        stage: Stage::Compute,
        workgroup_size: Some([WgSize::Const(1); 3]),
        subgroup_size: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        globals: Vec::new(),
        resources: Vec::new(),
        overrides: Vec::new(),
        immediate: None,
    };
    let ids = overrides.iter().map(|o| o.id).collect();
    let mut g = Gen::new(m, &ep, opt, values, false, ids);
    let void = g.b.ty_void();
    let (wid, _) = g.b.begin_function(void, &[]);
    g.b.ret();
    g.b.end_function();
    g.b.entry_point(Model::GLCompute, wid, "library");
    g.b.exec_mode(wid, Mode::LocalSize, &[1, 1, 1]);
    for (fi, f) in m.functions.iter().enumerate() {
        if f.stage.is_some() {
            continue;
        }
        let mut args = Vec::new();
        let mut ok = true;
        for p in &f.params {
            match *m.ty(p.ty) {
                Type::Pointer { space, store, .. } => args.push(ArgKey::Local(space_sc(space) as u32, store, Vec::new())),
                Type::Texture(..) | Type::Sampler { .. } => ok = false,
                _ => args.push(ArgKey::Value),
            }
        }
        if ok {
            g.func(FnKey { f: fi as u32, args });
        }
    }
    let mut i = 0;
    while i < g.todo.len() {
        let k = g.todo[i].clone();
        i += 1;
        lower_fn(&mut g, &k);
    }
    if !g.errors.is_empty() {
        return Err(g.errors.join("; "));
    }
    let words = g.b.finish();
    Ok(EntryOut {
        name: ep.name.clone(),
        stage: Stage::Compute,
        words,
        workgroup_size: Some([1, 1, 1]),
        subgroup_size: None,
        resources: Vec::new(),
        immediate_size: 0,
        inputs: Vec::new(),
        outputs: Vec::new(),
        needs: g.needs,
        spec: g.spec.clone(),
        overrides: Vec::new(),
    })
}

/// a stage input or output variable (with the conversion its value needs)
struct IoV {
    var: Id,
    /// f16 values go through f32 variables
    f16: bool,
    /// sample_mask: an array<u32, 1>
    array1: bool,
    /// computed in the wrapper (workgroup_index, global_invocation_index)
    computed: Option<BuiltinValue>,
}

fn io_var(g: &mut Gen, io: &IoVar, input: bool) -> IoV {
    let m = g.m;
    let sc = if input { Sc::Input } else { Sc::Output };
    let mut f16 = false;
    let mut array1 = false;
    let vt = match *m.ty(io.ty) {
        Type::Scalar(Scalar::F16) | Type::Vector(_, Scalar::F16) => {
            f16 = true;
            let n = match *m.ty(io.ty) {
                Type::Vector(n, _) => n,
                _ => 1,
            };
            let t = g.vec_tid(n, Scalar::F32);
            g.ty(t)
        }
        _ => {
            g.ty_plain(io.ty)
        }
    };
    let var = match io.kind {
        IoKind::Builtin(b) => {
            let bi = match b {
                BuiltinValue::VertexIndex => Bi::VertexIndex,
                BuiltinValue::InstanceIndex => Bi::InstanceIndex,
                BuiltinValue::ClipDistances => {
                    g.needs.clip_distance = true;
                    Bi::ClipDistance
                }
                BuiltinValue::Position => {
                    if input { Bi::FragCoord } else { Bi::Position }
                }
                BuiltinValue::FrontFacing => Bi::FrontFacing,
                BuiltinValue::FragDepth => Bi::FragDepth,
                BuiltinValue::PrimitiveIndex => {
                    g.needs.primitive_index = true;
                    g.b.capability(shader::Cap::Geometry);
                    Bi::PrimitiveId
                }
                BuiltinValue::SampleIndex => {
                    g.needs.sample_rate_shading = true;
                    Bi::SampleId
                }
                BuiltinValue::SampleMask => {
                    array1 = true;
                    Bi::SampleMask
                }
                BuiltinValue::LocalInvocationId => Bi::LocalInvocationId,
                BuiltinValue::LocalInvocationIndex => Bi::LocalInvocationIndex,
                BuiltinValue::GlobalInvocationId => Bi::GlobalInvocationId,
                BuiltinValue::WorkgroupId => Bi::WorkgroupId,
                BuiltinValue::NumWorkgroups => Bi::NumWorkgroups,
                BuiltinValue::GlobalInvocationIndex | BuiltinValue::WorkgroupIndex => {
                    return IoV { var: Id(0), f16, array1, computed: Some(b) };
                }
                BuiltinValue::SubgroupInvocationId => {
                    g.b.capability(shader::Cap::GroupNonUniform);
                    Bi::SubgroupLocalInvocationId
                }
                BuiltinValue::SubgroupSize => {
                    g.b.capability(shader::Cap::GroupNonUniform);
                    Bi::SubgroupSize
                }
                BuiltinValue::SubgroupId => {
                    g.b.capability(shader::Cap::GroupNonUniform);
                    Bi::SubgroupId
                }
                BuiltinValue::NumSubgroups => {
                    g.b.capability(shader::Cap::GroupNonUniform);
                    Bi::NumSubgroups
                }
            };
            let t = if array1 {
                let u = g.b.ty_u32();
                g.b.ty_array(u, 1)
            } else {
                vt
            };
            let v = g.b.builtin(sc, t, bi);
            if io.invariant {
                g.b.decorate(v, Dec::Invariant, &[]);
            }
            if matches!(b, BuiltinValue::SampleIndex | BuiltinValue::PrimitiveIndex | BuiltinValue::SubgroupInvocationId | BuiltinValue::SubgroupSize) {
                if input && g.stage == Stage::Fragment {
                    g.b.decorate(v, Dec::Flat, &[]);
                }
            }
            v
        }
        IoKind::Location { location, blend_src, interp } => {
            let v = g.b.global(sc, vt, None);
            g.b.decorate(v, Dec::Location, &[location]);
            if let Some(i) = blend_src {
                g.b.decorate(v, Dec::Index, &[i]);
            }
            if let Some((ty, samp)) = interp {
                match ty {
                    InterpType::Flat => g.b.decorate(v, Dec::Flat, &[]),
                    InterpType::Linear => g.b.decorate(v, Dec::NoPerspective, &[]),
                    InterpType::Perspective => {}
                }
                if ty != InterpType::Flat {
                    match samp {
                        InterpSampling::Centroid => g.b.decorate(v, Dec::Centroid, &[]),
                        InterpSampling::Sample => {
                            g.needs.sample_rate_shading = true;
                            g.b.capability(shader::Cap::SampleRateShading);
                            g.b.decorate(v, Dec::Sample, &[]);
                        }
                        _ => {}
                    }
                }
            }
            v
        }
    };
    g.b.name(var, &io.name);
    IoV { var, f16, array1, computed: None }
}

/// compile one entry point (spirv.rs: compile_module)
pub(crate) fn lower_entry(m: &Module, ep: &EntryPoint, opt: &CompileOptions, values: Vec<Option<Value>>, subst: bool,
                          overrides: &[OverrideInfo]) -> Result<EntryOut, String> {
    let ids = overrides.iter().map(|o| o.id).collect();
    let mut g = Gen::new(m, ep, opt, values, subst, ids);
    let f = &m.functions[ep.func as usize];
    if ep.stage == Stage::Fragment && f.uses_discard {
        if g.minor >= 6 {
            g.needs.demote = true;
        } else {
            let bt = g.b.ty_bool();
            let fl = g.b.const_bool(false);
            let v = g.b.global(Sc::Private, bt, Some(fl));
            g.b.name(v, "discarded");
            g.discard_flag = Some(v);
        }
    }
    let ins: Vec<IoV> = ep.inputs.iter().map(|io| io_var(&mut g, io, true)).collect();
    let outs: Vec<IoV> = ep.outputs.iter().map(|io| io_var(&mut g, io, false)).collect();
    // the workgroup size
    let mut wg_ids = [Id(0); 3];
    let mut wg_default = Some([1u32; 3]);
    let mut wg_const = true;
    if let Some(ws) = ep.workgroup_size {
        for (k, w) in ws.iter().enumerate() {
            match *w {
                WgSize::Const(n) => {
                    wg_ids[k] = g.b.const_u32(n);
                    if let Some(d) = wg_default.as_mut() {
                        d[k] = n;
                    }
                }
                WgSize::Override(e) => {
                    wg_const = false;
                    wg_ids[k] = g.hidden(e);
                    match crate::pipeline::eval_u32(m, e, &g.values) {
                        Ok(n) => {
                            if let Some(d) = wg_default.as_mut() {
                                d[k] = n;
                            }
                        }
                        Err(_) => wg_default = None,
                    }
                }
            }
        }
        if g.subst {
            wg_const = true;
        }
    }
    let void = g.b.ty_void();
    let (wid, _) = g.b.begin_function(void, &[]);
    g.b.name(wid, &format!("{}_entry", ep.name));
    {
        let mut fx = Fx::new(&mut g, &m.exprs, &[], Vec::new());
        // workgroup memory is zeroed (WGSL): one invocation stores, then a barrier
        let wg_vars: Vec<u32> = ep.globals.iter().copied().filter(|&gi| m.globals[gi as usize].space == AddressSpace::Workgroup).collect();
        if !wg_vars.is_empty() {
            let lii = match ep.inputs.iter().position(|io| io.kind == IoKind::Builtin(BuiltinValue::LocalInvocationIndex)) {
                Some(i) => ins[i].var,
                None => {
                    let u = fx.g.b.ty_u32();
                    fx.g.b.builtin(Sc::Input, u, Bi::LocalInvocationIndex)
                }
            };
            let li = fx.g.b.load(lii);
            let z = fx.cu(0);
            let first = fx.g.b.ieq(li, z);
            fx.if_then(first, |fx| {
                for &gi in &wg_vars {
                    let gv = fx.g.global(gi);
                    let t = fx.m().globals[gi as usize].ty;
                    zero_memory(fx, gv.var, t);
                }
            });
            fx.g.b.control_barrier(spv::scope::Workgroup, spv::scope::Workgroup, spv::sem::AcquireRelease | spv::sem::WorkgroupMemory);
        }
        // private variables whose initializer is an override-expression
        for &gi in &ep.globals {
            let gl = &m.globals[gi as usize];
            if gl.space != AddressSpace::Private {
                continue;
            }
            if let Some(e) = gl.init {
                if !matches!(m.exprs[e as usize].kind, ExprKind::Const(_) | ExprKind::Override(_)) {
                    let v = fx.val(e);
                    let gv = fx.g.global(gi);
                    fx.g.b.store(gv.var, v);
                }
            }
        }
        // the inputs
        let mut loaded: Vec<Option<Id>> = alloc::vec![None; ins.len()];
        for (i, io) in ep.inputs.iter().enumerate() {
            loaded[i] = Some(load_input(&mut fx, io, &ins[i], &ins, ep, &wg_ids));
        }
        let mut args = Vec::new();
        for (k, p) in f.params.iter().enumerate() {
            if p.io.builtin.is_some() || p.io.location.is_some() {
                let i = ep.inputs.iter().position(|io| io.param == Some(k as u32) && io.member.is_none()).unwrap_or(0);
                args.push(loaded[i].unwrap_or(Id(0)));
            } else {
                let mut parts: Vec<(u32, Id)> = Vec::new();
                for (i, io) in ep.inputs.iter().enumerate() {
                    if io.param == Some(k as u32) {
                        parts.push((io.member.unwrap_or(0), loaded[i].unwrap_or(Id(0))));
                    }
                }
                parts.sort_by_key(|p| p.0);
                let st = fx.sty(p.ty);
                let ids: Vec<Id> = parts.iter().map(|p| p.1).collect();
                args.push(fx.g.b.construct(st, &ids));
            }
        }
        let key = FnKey { f: ep.func, args: alloc::vec![ArgKey::Value; f.params.len()] };
        let fid = fx.g.func(key);
        let rt = match f.ret {
            Some(t) => fx.sty(t),
            None => void,
        };
        let mut w = alloc::vec![fid.0];
        w.extend(args.iter().map(|a| a.0));
        let r = fx.g.b.emit(op::FunctionCall, Some(rt), &w);
        if let Some(flag) = fx.g.discard_flag {
            // a discarded fragment ends here: nothing it wrote after discard took effect
            let d = fx.g.b.load(flag);
            let (kill, go) = (fx.g.b.label(), fx.g.b.label());
            fx.g.b.selection_merge(go, 0);
            fx.g.b.branch_cond(d, kill, go);
            fx.g.b.begin_block(kill);
            fx.g.b.discard();
            fx.g.b.begin_block(go);
        }
        for (i, io) in ep.outputs.iter().enumerate() {
            let v = match io.member {
                Some(k) => fx.g.b.extract(r, &[k]),
                None => r,
            };
            let o = &outs[i];
            let v = if o.f16 {
                let n = fx.width(io.ty);
                let t = fx.g.vec_tid(n, Scalar::F32);
                let ft = fx.sty(t);
                fx.g.b.convert(op::FConvert, ft, v)
            } else {
                v
            };
            let v = if fx.g.minor < 4 && matches!(*m.ty(io.ty), Type::Array { .. }) { fx.manual_layout(v, io.ty, false) } else { v };
            if o.array1 {
                let p = fx.g.b.access_c(o.var, &[0]);
                fx.g.b.store(p, v);
            } else {
                fx.g.b.store(o.var, v);
            }
        }
        if ep.stage == Stage::Vertex {
            // WebGPU's points are 1 pixel; Vulkan wants PointSize written for point lists
            let f = fx.g.b.ty_f32();
            let ps = fx.g.b.builtin(Sc::Output, f, Bi::PointSize);
            let one = fx.g.b.const_f32(1.0);
            fx.g.b.store(ps, one);
        }
        fx.g.b.ret();
    }
    g.b.end_function();
    let model = match ep.stage {
        Stage::Vertex => Model::Vertex,
        Stage::Fragment => Model::Fragment,
        Stage::Compute => Model::GLCompute,
    };
    g.b.entry_point(model, wid, &ep.name);
    match ep.stage {
        Stage::Fragment => {
            g.b.exec_mode(wid, Mode::OriginUpperLeft, &[]);
            if let Some(io) = ep.outputs.iter().find(|io| io.kind == IoKind::Builtin(BuiltinValue::FragDepth)) {
                g.b.exec_mode(wid, Mode::DepthReplacing, &[]);
                match io.depth_mode {
                    Some(DepthMode::Less) => g.b.exec_mode(wid, Mode::DepthLess, &[]),
                    Some(DepthMode::Greater) => g.b.exec_mode(wid, Mode::DepthGreater, &[]),
                    None => {}
                }
            }
        }
        Stage::Compute => {
            if wg_const {
                let d = if g.subst {
                    let mut d = [1u32; 3];
                    if let Some(ws) = ep.workgroup_size {
                        for (k, w) in ws.iter().enumerate() {
                            d[k] = match *w {
                                WgSize::Const(n) => n,
                                WgSize::Override(e) => crate::pipeline::eval_u32(m, e, &g.values).unwrap_or(1),
                            };
                        }
                    }
                    d
                } else {
                    wg_default.unwrap_or([1; 3])
                };
                g.b.exec_mode(wid, Mode::LocalSize, &d);
            } else {
                g.b.workgroup_size(wid, wg_ids);
            }
        }
        Stage::Vertex => {}
    }
    // the functions of the call tree
    let mut i = 0;
    while i < g.todo.len() {
        let k = g.todo[i].clone();
        i += 1;
        lower_fn(&mut g, &k);
    }
    if !g.errors.is_empty() {
        return Err(g.errors.join("; "));
    }
    let words = g.b.finish();
    let immediate_size = ep.immediate.map(|gi| g.layout(m.globals[gi as usize].ty).1.div_ceil(4) * 4).unwrap_or(0);
    let resources = resources(&g);
    let io = |list: &[IoVar]| -> Vec<StageVar> { list.iter().map(|v| StageVar { name: v.name.clone(), kind: v.kind, ty: v.ty }).collect() };
    Ok(EntryOut {
        name: ep.name.clone(),
        stage: ep.stage,
        words,
        workgroup_size: if ep.stage == Stage::Compute { wg_default } else { None },
        subgroup_size: match ep.subgroup_size {
            Some(WgSize::Const(n)) => Some(n),
            Some(WgSize::Override(e)) => crate::pipeline::eval_u32_default(m, e),
            None => None,
        },
        resources,
        immediate_size,
        inputs: io(&ep.inputs),
        outputs: io(&ep.outputs),
        needs: g.needs,
        spec: g.spec.clone(),
        overrides: ep.overrides.clone(),
    })
}

/// store the zero value of t into var (override-sized arrays element by element)
fn zero_memory(fx: &mut Fx, var: Id, t: TypeId) {
    let m = fx.m();
    match *m.ty(t) {
        Type::Array { elem, count: ArrayCount::Override(_) | ArrayCount::Expr(_), .. } => {
            let n = match *m.ty(t) {
                Type::Array { count: ArrayCount::Override(o), .. } => {
                    let v = fx.g.override_value(o);
                    if fx.scalar(m.overrides[o as usize].ty) == Scalar::I32 {
                        let u = fx.g.b.ty_u32();
                        fx.g.b.bitcast(u, v)
                    } else {
                        v
                    }
                }
                Type::Array { count: ArrayCount::Expr(e), .. } => fx.g.hidden(e),
                _ => fx.cu(1),
            };
            // for (var i = 0u; i < n; i++) { var[i] = zero }
            let u = fx.g.b.ty_u32();
            let i = fx.g.b.local(u, None);
            let z = fx.cu(0);
            fx.g.b.store(i, z);
            let (header, check, body, cont, merge) = (fx.g.b.label(), fx.g.b.label(), fx.g.b.label(), fx.g.b.label(), fx.g.b.label());
            fx.g.b.branch(header);
            fx.g.b.begin_block(header);
            fx.g.b.loop_merge(merge, cont, 0);
            fx.g.b.branch(check);
            fx.g.b.begin_block(check);
            let iv = fx.g.b.load(i);
            let lt = fx.g.b.ult(iv, n);
            fx.g.b.branch_cond(lt, body, merge);
            fx.g.b.begin_block(body);
            let p = fx.g.b.access(var, &[iv]);
            let et = fx.g.ty_plain(elem);
            let ez = fx.g.b.const_null(et);
            fx.g.b.store(p, ez);
            fx.g.b.branch(cont);
            fx.g.b.begin_block(cont);
            let one = fx.cu(1);
            let nx = fx.g.b.iadd(iv, one);
            fx.g.b.store(i, nx);
            fx.g.b.branch(header);
            fx.g.b.begin_block(merge);
        }
        _ => {
            let vt = fx.g.ty_plain(t);
            let z = fx.g.b.const_null(vt);
            fx.g.b.store(var, z);
        }
    }
}

/// the value of a stage input in the wrapper
fn load_input(fx: &mut Fx, io: &IoVar, v: &IoV, ins: &[IoV], ep: &EntryPoint, wg: &[Id; 3]) -> Id {
    if let Some(b) = v.computed {
        // linear_indexing: workgroup_index = wid.x + nwg.x * (wid.y + nwg.y * wid.z),
        // global_invocation_index = gid.x + sx * (gid.y + sy * gid.z) with sx = nwg.x * wg.x ...
        let u = fx.g.b.ty_u32();
        let uv3 = fx.g.b.ty_vec(u, 3);
        let find = |fx: &mut Fx, bv: BuiltinValue, bi: Bi| -> Id {
            match ep.inputs.iter().position(|x| x.kind == IoKind::Builtin(bv)) {
                Some(i) => fx.g.b.load(ins[i].var),
                None => {
                    let var = match fx.g.builtins.get(&(bi as u32)) {
                        Some(&v) => v,
                        None => {
                            let v = fx.g.b.builtin(Sc::Input, uv3, bi);
                            fx.g.builtins.insert(bi as u32, v);
                            v
                        }
                    };
                    fx.g.b.load(var)
                }
            }
        };
        let nwg = find(fx, BuiltinValue::NumWorkgroups, Bi::NumWorkgroups);
        let (id, sx, sy) = if b == BuiltinValue::WorkgroupIndex {
            let wid = find(fx, BuiltinValue::WorkgroupId, Bi::WorkgroupId);
            let nx = fx.g.b.extract(nwg, &[0]);
            let ny = fx.g.b.extract(nwg, &[1]);
            (wid, nx, ny)
        } else {
            let gid = find(fx, BuiltinValue::GlobalInvocationId, Bi::GlobalInvocationId);
            let nx = fx.g.b.extract(nwg, &[0]);
            let ny = fx.g.b.extract(nwg, &[1]);
            let sx = fx.g.b.imul(nx, wg[0]);
            let sy = fx.g.b.imul(ny, wg[1]);
            (gid, sx, sy)
        };
        let x = fx.g.b.extract(id, &[0]);
        let y = fx.g.b.extract(id, &[1]);
        let z = fx.g.b.extract(id, &[2]);
        let t = fx.g.b.imul(sy, z);
        let t = fx.g.b.iadd(y, t);
        let t = fx.g.b.imul(sx, t);
        return fx.g.b.iadd(x, t);
    }
    let x = if v.array1 {
        let p = fx.g.b.access_c(v.var, &[0]);
        fx.g.b.load(p)
    } else {
        fx.g.b.load(v.var)
    };
    if v.f16 {
        let t = fx.sty(io.ty);
        return fx.g.b.convert(op::FConvert, t, x);
    }
    x
}

