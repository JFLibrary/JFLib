// check_expr -- type checking of expressions: literals, names, the Load Rule, references and
// pointers, composite access and swizzles (incl. swizzle views), operators with overload
// resolution (spec 6.1.3), value constructors and conversions, bitcast, user function calls
// (with the alias analysis of 11.4.1), built-in calls (builtins.rs) and constant folding
//
//   expr(e) -> R<TE>            a checked expression (may be a reference)
//   rvalue(te)                  the Load Rule: references and swizzle views read
//   convert(te, to)             a feasible automatic conversion (abstract values checked)
//   concretize(te)              rvalue + the concretization of an abstract type
//   node(te)                    the IR node of a checked expression (folds constants)
//   resolve(args, cands)        overload resolution over fully elaborated candidates
//   call(...)                   calls; a call without result gives ty = NONE (statements)
//
// Constants are not put into the arena until something non-constant needs them (TE::id is
// NONE until then), so a const-expression ends up as one Const node.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::{self, BinOp, Expr, ExprKind, Span, UnOp};
use crate::builtins::{self, Sig, TP, TR};
use crate::check::*;
use crate::consteval::{self, comp, comps};
use crate::feature;
use crate::ir::{self, *};
use crate::lex::Suffix;
use crate::types::INF;

impl<'a> Checker<'a> {
    // ---- arena

    pub fn push(&mut self, kind: ir::ExprKind, ty: TypeId, span: Span) -> ExprId {
        let e = ir::Expr { kind, ty, span };
        match self.f.as_mut() {
            Some(f) if !self.module_arena => {
                f.exprs.push(e);
                (f.exprs.len() - 1) as ExprId
            }
            _ => {
                self.m.exprs.push(e);
                (self.m.exprs.len() - 1) as ExprId
            }
        }
    }

    pub fn arena(&self) -> &[ir::Expr] {
        match self.f.as_ref() {
            Some(f) if !self.module_arena => &f.exprs,
            _ => &self.m.exprs,
        }
    }

    pub fn mk(&mut self, kind: ir::ExprKind, ty: TypeId, stage: Phase, span: Span) -> TE {
        let id = self.push(kind, ty, span);
        TE { id, ty, stage, val: None, span }
    }

    pub fn cst(&self, ty: TypeId, val: Option<Value>, span: Span) -> TE {
        TE { id: NONE, ty, stage: Phase::Const, val, span }
    }

    /// the node of a checked expression; constants become Const nodes (concretized)
    pub fn node(&mut self, te: &TE) -> ExprId {
        if te.id != NONE {
            return te.id;
        }
        let mut ty = te.ty;
        let mut v = te.val.clone().unwrap_or(Value::Zero);
        if self.t.is_abstract(ty) {
            let c = self.t.concretize(ty);
            v = consteval::convert(&mut self.t, &v, ty, c, false).unwrap_or(Value::Zero);
            ty = c;
        }
        self.push(ir::ExprKind::Const(v), ty, te.span)
    }

    fn kind_of(&self, id: ExprId) -> Option<&ir::ExprKind> {
        if id == NONE {
            return None;
        }
        self.arena().get(id as usize).map(|e| &e.kind)
    }

    // ---- loads and conversions

    pub fn rvalue(&mut self, te: TE) -> R<TE> {
        match self.t.get(te.ty) {
            Type::Reference { store, access, .. } => {
                if !access.readable() {
                    return Err(self.err(te.span, String::from("aus diesem Speicher darf nicht gelesen werden (Zugriffsmodus write)")));
                }
                if let Some(r) = self.root(te.id) {
                    if let Some(f) = self.f.as_mut() {
                        f.reads.insert(r);
                    }
                }
                Ok(self.mk(ir::ExprKind::Load(te.id), store, te.stage, te.span))
            }
            Type::SwizzleView { elem, k, .. } => {
                if let Some(r) = self.root(te.id) {
                    if let Some(f) = self.f.as_mut() {
                        f.reads.insert(r);
                    }
                }
                let vt = self.t.vec(k, elem);
                Ok(self.mk(ir::ExprKind::Load(te.id), vt, te.stage, te.span))
            }
            _ => Ok(te),
        }
    }

    /// a feasible automatic conversion of te to `to` (error otherwise)
    pub fn convert(&mut self, te: TE, to: TypeId) -> R<TE> {
        if te.ty == to {
            return Ok(te);
        }
        if self.t.rank(te.ty, to) == INF {
            let (a, b) = (self.tn(te.ty), self.tn(to));
            let a = if let Type::Reference { store, .. } = self.t.get(te.ty) { self.tn(store) } else { a };
            return Err(self.err(te.span, format!("Typ `{}` passt nicht zu `{}`", a, b)));
        }
        let te = self.rvalue(te)?;
        if te.ty == to {
            return Ok(te);
        }
        // only abstract constants convert from here on
        if te.stage != Phase::Const {
            return Err(self.err(te.span, format!("Typ `{}` passt nicht zu `{}`", self.tn(te.ty), self.tn(to))));
        }
        let val = match &te.val {
            Some(v) => match consteval::convert(&mut self.t, v, te.ty, to, false) {
                Ok(v) => Some(v),
                Err(m) => return Err(self.err(te.span, m)),
            },
            None => None,
        };
        Ok(TE { id: NONE, ty: to, stage: Phase::Const, val, span: te.span })
    }

    pub fn concretize(&mut self, te: TE) -> R<TE> {
        let te = self.rvalue(te)?;
        if !self.t.is_abstract(te.ty) {
            return Ok(te);
        }
        let c = self.t.concretize(te.ty);
        self.convert(te, c)
    }

    /// a value in a context that needs one (no void call, no type, no reference)
    pub fn value(&mut self, e: &Expr) -> R<TE> {
        let te = self.expr(e)?;
        self.rvalue(te)
    }

    // ---- roots (alias analysis)

    pub fn root(&self, id: ExprId) -> Option<Root> {
        let f = self.f.as_ref()?;
        if self.module_arena {
            return None;
        }
        let mut cur = id;
        for _ in 0..10_000 {
            let e = f.exprs.get(cur as usize)?;
            match &e.kind {
                ir::ExprKind::Global(g) => return Some(Root::Global(*g)),
                ir::ExprKind::Param(k) => return if self.t.is_ptr(e.ty) { Some(Root::Param(*k)) } else { None },
                ir::ExprKind::Local(l) => {
                    let loc = f.locals.get(*l as usize)?;
                    if loc.kind == LocalKind::Var {
                        return Some(Root::Local(*l));
                    }
                    cur = (*f.local_init.get(*l as usize)?)?;
                }
                ir::ExprKind::AddrOf(b) | ir::ExprKind::Deref(b) | ir::ExprKind::Index(b, _) | ir::ExprKind::Member(b, _) | ir::ExprKind::Swizzle(b, ..) => cur = *b,
                _ => return None,
            }
        }
        None
    }

    fn note_global(&mut self, g: u32) {
        if let Some(f) = self.f.as_mut() {
            f.info.globals.insert(g);
        }
    }

    // ---- expressions

    pub fn expr(&mut self, e: &Expr) -> R<TE> {
        let sp = e.span;
        match &e.kind {
            ExprKind::Bool(b) => Ok(self.cst(BOOL, Some(Value::Bool(*b)), sp)),
            ExprKind::Int(_, v, s) => Ok(match s {
                Suffix::I => self.cst(I32, Some(Value::I32(*v as i32)), sp),
                Suffix::U => self.cst(U32, Some(Value::U32(*v as u32)), sp),
                _ => self.cst(AINT, Some(Value::AInt(*v)), sp),
            }),
            ExprKind::Float(_, v, s) => Ok(match s {
                Suffix::F => self.cst(F32, Some(Value::F32(*v as f32)), sp),
                Suffix::H => {
                    if !self.has(feature::F16) {
                        return Err(self.err(sp, String::from("f16-Literale (Suffix h) brauchen `enable f16;`")));
                    }
                    self.cst(F16, Some(Value::F16(*v as f32)), sp)
                }
                _ => self.cst(AFLOAT, Some(Value::AFloat(*v)), sp),
            }),
            ExprKind::Paren(x) => {
                let mut te = self.expr(x)?;
                te.span = sp;
                Ok(te)
            }
            ExprKind::Ident(id, targs) => self.ident(id, targs.as_deref(), sp),
            ExprKind::Call(id, targs, args) => {
                let te = self.call(id, targs.as_deref(), args, sp)?;
                if te.ty == builtins::NONE {
                    return Err(self.err(sp, format!("`{}` liefert keinen Wert", id.name)));
                }
                Ok(te)
            }
            ExprKind::Unary(op, x) => self.unary(*op, x, sp),
            ExprKind::Binary(op, a, b) => self.binary(*op, a, b, sp),
            ExprKind::Index(b, i) => self.index(b, i, sp),
            ExprKind::Member(b, m) => self.member(b, m, sp),
            ExprKind::Error => Err(E),
        }
    }

    fn ident(&mut self, id: &ast::Ident, targs: Option<&[Expr]>, sp: Span) -> R<TE> {
        if let Some((r, uid, dsp)) = self.lookup_local(&id.name) {
            if targs.is_some() {
                return Err(self.err(sp, format!("`{}` hat keine Template-Parameter", id.name)));
            }
            if let Some(f) = self.f.as_mut() {
                for m in f.cont_uses.iter_mut() {
                    m.entry(uid).or_insert(sp);
                }
            }
            let _ = dsp;
            return Ok(match r {
                LocalRef::Const(ty, v) => self.cst(ty, Some(v), sp),
                LocalRef::Let(l) => {
                    let ty = self.f.as_ref().map(|f| f.locals[l as usize].ty).unwrap_or(BOOL);
                    self.mk(ir::ExprKind::Local(l), ty, Phase::Runtime, sp)
                }
                LocalRef::Var(l) => {
                    let ty = self.f.as_ref().map(|f| f.locals[l as usize].ty).unwrap_or(BOOL);
                    let rt = self.t.reference(AddressSpace::Function, ty, Access::ReadWrite);
                    self.mk(ir::ExprKind::Local(l), rt, Phase::Runtime, sp)
                }
                LocalRef::Param(k) => {
                    let ty = self.f.as_ref().map(|f| f.params[k as usize].ty).unwrap_or(BOOL);
                    self.mk(ir::ExprKind::Param(k), ty, Phase::Runtime, sp)
                }
                LocalRef::Error => return Err(E),
            });
        }
        match self.lookup_decl(&id.name)? {
            Some((i, info)) => {
                if targs.is_some() && !matches!(info, DeclInfo::Alias(_) | DeclInfo::Struct(_)) {
                    return Err(self.err(sp, format!("`{}` hat keine Template-Parameter", id.name)));
                }
                let dsp = self.decl_span(i);
                match info {
                    DeclInfo::Const(ty, v) => Ok(self.cst(ty, Some(v), sp)),
                    DeclInfo::Override(o) => {
                        if let Some(f) = self.f.as_mut() {
                            f.info.overrides.insert(o);
                        }
                        let ty = self.m.overrides[o as usize].ty;
                        Ok(self.mk(ir::ExprKind::Override(o), ty, Phase::Override, sp))
                    }
                    DeclInfo::Var(g) => {
                        if self.f.is_none() || self.module_arena {
                            return Err(self.err_note(sp, format!("die Variable `{}` ist in const- und override-Ausdrücken nicht erlaubt", id.name), dsp, String::from("hier deklariert")));
                        }
                        self.note_global(g);
                        let gl = &self.m.globals[g as usize];
                        let (space, ty, access) = (gl.space, gl.ty, gl.access);
                        let rt = self.t.reference(space, ty, access);
                        Ok(self.mk(ir::ExprKind::Global(g), rt, Phase::Runtime, sp))
                    }
                    DeclInfo::Fn(_) => Err(self.err_note(sp, format!("`{}` ist eine Funktion; aufrufen mit `{}(…)`", id.name, id.name), dsp, String::from("hier deklariert"))),
                    DeclInfo::Alias(_) | DeclInfo::Struct(_) => Err(self.err_note(sp, format!("`{}` ist ein Typ und kein Wert", id.name), dsp, String::from("hier deklariert"))),
                    DeclInfo::Assert => Err(E),
                }
            }
            None => match predeclared_kind(&id.name) {
                Some(k) => Err(self.err(sp, format!("`{}` ist {} und kein Wert", id.name, k))),
                None => Err(self.err(sp, format!("`{}` ist hier nicht bekannt", id.name))),
            },
        }
    }

    fn auto_deref(&mut self, te: TE) -> R<TE> {
        if let Type::Pointer { space, store, access } = self.t.get(te.ty) {
            if !self.has(feature::POINTER_COMPOSITE_ACCESS) {
                return Err(self.err(te.span, String::from("ein Zeiger muss erst mit `*` dereferenziert werden: `(*p).x`")));
            }
            let rt = self.t.reference(space, store, access);
            return Ok(self.mk(ir::ExprKind::Deref(te.id), rt, te.stage, te.span));
        }
        Ok(te)
    }

    /// an index of type i32 / u32 (abstract: i32)
    fn index_value(&mut self, e: &Expr) -> R<TE> {
        let i = self.value(e)?;
        match i.ty {
            I32 | U32 => Ok(i),
            AINT => self.convert(i, I32),
            _ => Err(self.err(e.span, format!("ein Index muss i32 oder u32 sein, nicht `{}`", self.tn(i.ty)))),
        }
    }

    fn const_index(&self, i: &TE) -> Option<i64> {
        if i.stage != Phase::Const {
            return None;
        }
        match i.val {
            Some(Value::I32(x)) => Some(x as i64),
            Some(Value::U32(x)) => Some(x as i64),
            Some(Value::AInt(x)) => Some(x),
            _ => None,
        }
    }

    fn index(&mut self, be: &Expr, ie: &Expr, sp: Span) -> R<TE> {
        let base = self.expr(be)?;
        let base = self.auto_deref(base)?;
        let idx = self.index_value(ie)?;
        let ci = self.const_index(&idx);
        let bound = |c: &mut Self, n: u32| -> R<()> {
            if let Some(k) = ci {
                if k < 0 || k >= n as i64 {
                    return Err(c.err(ie.span, format!("der Index {} liegt außerhalb von 0..{}", k, n.saturating_sub(1))));
                }
            }
            Ok(())
        };
        match self.t.get(base.ty) {
            Type::Reference { space, store, access } => {
                let elem = match self.t.get(store) {
                    Type::Vector(n, s) => {
                        bound(self, n as u32)?;
                        s.id()
                    }
                    Type::Matrix(c, r, s) => {
                        bound(self, c as u32)?;
                        self.t.vec(r, s)
                    }
                    Type::Array { elem, count, .. } => {
                        match count {
                            ArrayCount::Fixed(n) => bound(self, n)?,
                            _ => {
                                if let Some(k) = ci {
                                    if k < 0 {
                                        return Err(self.err(ie.span, format!("der Index {} ist negativ", k)));
                                    }
                                }
                            }
                        }
                        elem
                    }
                    _ => return Err(self.err(be.span, format!("`{}` lässt sich nicht indizieren", self.tn(store)))),
                };
                let rt = self.t.reference(space, elem, access);
                let iid = self.node(&idx);
                Ok(self.mk(ir::ExprKind::Index(base.id, iid), rt, Phase::Runtime, sp))
            }
            Type::SwizzleView { space, elem, k, .. } => {
                bound(self, k as u32)?;
                let (vref, comps) = match self.kind_of(base.id) {
                    Some(ir::ExprKind::Swizzle(b, cs, _)) => (*b, *cs),
                    _ => return Err(E),
                };
                let rt = self.t.reference(space, elem.id(), Access::ReadWrite);
                let Some(c) = ci else {
                    // a dynamic index picks from the component list: v[array(c0, c1..)[i]]
                    let at = self.t.array(U32, ArrayCount::Fixed(k as u32));
                    let list = Value::Comp(comps[..k as usize].iter().map(|&c| Value::U32(c as u32)).collect());
                    let lid = self.push(ir::ExprKind::Const(list), at, ie.span);
                    let iid = self.node(&idx);
                    let mapped = self.push(ir::ExprKind::Index(lid, iid), U32, ie.span);
                    return Ok(self.mk(ir::ExprKind::Index(vref, mapped), rt, Phase::Runtime, sp));
                };
                Ok(self.mk(ir::ExprKind::Member(vref, comps[c as usize] as u32), rt, Phase::Runtime, sp))
            }
            _ => {
                let base = self.rvalue(base)?;
                let (elem, n) = match self.t.get(base.ty) {
                    Type::Vector(n, s) => (s.id(), n as u32),
                    Type::Matrix(c, r, s) => (self.t.vec(r, s), c as u32),
                    Type::Array { elem, count: ArrayCount::Fixed(n), .. } => (elem, n),
                    _ => return Err(self.err(be.span, format!("`{}` lässt sich nicht indizieren", self.tn(base.ty)))),
                };
                bound(self, n)?;
                if base.stage == Phase::Const && idx.stage == Phase::Const {
                    let val = match (&base.val, ci) {
                        (Some(v), Some(k)) => Some(comp(&self.t, v, base.ty, k as usize)),
                        _ => None,
                    };
                    return Ok(TE { id: NONE, ty: elem, stage: Phase::Const, val, span: sp });
                }
                // a dynamic index into an abstract constant: concretize the constant first
                let base = if self.t.is_abstract(base.ty) { self.concretize(base)? } else { base };
                let elem = consteval::comp_type(&self.t, base.ty, 0);
                let (bid, iid) = (self.node(&base), self.node(&idx));
                let stage = base.stage.max(idx.stage);
                Ok(self.mk(ir::ExprKind::Index(bid, iid), elem, stage, sp))
            }
        }
    }

    fn swizzle_comps(&mut self, name: &ast::Ident, n: u8) -> R<Vec<u8>> {
        let s = name.name.as_bytes();
        let xyzw = s.iter().all(|c| b"xyzw".contains(c));
        let rgba = s.iter().all(|c| b"rgba".contains(c));
        if s.is_empty() || s.len() > 4 || !(xyzw || rgba) {
            return Err(self.err(name.span, format!("`{}` ist kein gültiger Swizzle (x, y, z, w oder r, g, b, a, nicht gemischt, 1 bis 4 Buchstaben)", name.name)));
        }
        let mut v = Vec::new();
        for c in s {
            let k = match c {
                b'x' | b'r' => 0,
                b'y' | b'g' => 1,
                b'z' | b'b' => 2,
                _ => 3,
            };
            if k >= n {
                return Err(self.err(name.span, format!("`{}` greift über den Vektor mit {} Komponenten hinaus", name.name, n)));
            }
            v.push(k);
        }
        Ok(v)
    }

    fn member(&mut self, be: &Expr, name: &ast::Ident, sp: Span) -> R<TE> {
        let base = self.expr(be)?;
        let base = self.auto_deref(base)?;
        // swizzle views: compose
        if let Type::SwizzleView { space, elem, n, k } = self.t.get(base.ty) {
            let cs = self.swizzle_comps(name, k)?;
            let (vref, old) = match self.kind_of(base.id) {
                Some(ir::ExprKind::Swizzle(b, c, _)) => (*b, *c),
                _ => return Err(E),
            };
            if cs.len() == 1 {
                let rt = self.t.reference(space, elem.id(), Access::ReadWrite);
                return Ok(self.mk(ir::ExprKind::Member(vref, old[cs[0] as usize] as u32), rt, Phase::Runtime, sp));
            }
            let mut nc = [0u8; 4];
            for (i, &c) in cs.iter().enumerate() {
                nc[i] = old[c as usize];
            }
            let st = self.t.intern(Type::SwizzleView { space, elem, n, k: cs.len() as u8 });
            return Ok(self.mk(ir::ExprKind::Swizzle(vref, nc, cs.len() as u8), st, Phase::Runtime, sp));
        }
        let (is_ref, store, space, access) = match self.t.get(base.ty) {
            Type::Reference { space, store, access } => (true, store, space, access),
            _ => (false, base.ty, AddressSpace::Function, Access::Read),
        };
        match self.t.get(store) {
            Type::Struct(si) => {
                let Some((k, mty)) = self.t.structs.get(si as usize).and_then(|s| s.members.iter().position(|m| m.name == name.name).map(|k| (k, s.members[k].ty))) else {
                    let sname = self.tn(store);
                    return Err(self.err(name.span, format!("die Struktur `{}` hat kein Member `{}`", sname, name.name)));
                };
                if is_ref {
                    let rt = self.t.reference(space, mty, access);
                    return Ok(self.mk(ir::ExprKind::Member(base.id, k as u32), rt, Phase::Runtime, sp));
                }
                if base.stage == Phase::Const {
                    let val = base.val.as_ref().map(|v| comp(&self.t, v, store, k));
                    return Ok(TE { id: NONE, ty: mty, stage: Phase::Const, val, span: sp });
                }
                let bid = self.node(&base);
                Ok(self.mk(ir::ExprKind::Member(bid, k as u32), mty, base.stage, sp))
            }
            Type::Vector(n, s) => {
                let cs = self.swizzle_comps(name, n)?;
                if cs.len() == 1 {
                    if is_ref {
                        let rt = self.t.reference(space, s.id(), access);
                        return Ok(self.mk(ir::ExprKind::Member(base.id, cs[0] as u32), rt, Phase::Runtime, sp));
                    }
                    if base.stage == Phase::Const {
                        let val = base.val.as_ref().map(|v| comp(&self.t, v, store, cs[0] as usize));
                        return Ok(TE { id: NONE, ty: s.id(), stage: Phase::Const, val, span: sp });
                    }
                    let bid = self.node(&base);
                    return Ok(self.mk(ir::ExprKind::Member(bid, cs[0] as u32), s.id(), base.stage, sp));
                }
                let mut c4 = [0u8; 4];
                for (i, &c) in cs.iter().enumerate() {
                    c4[i] = c;
                }
                let k = cs.len() as u8;
                if is_ref && access == Access::ReadWrite && self.has(feature::SWIZZLE_ASSIGNMENT) {
                    let st = self.t.intern(Type::SwizzleView { space, elem: s, n, k });
                    return Ok(self.mk(ir::ExprKind::Swizzle(base.id, c4, k), st, Phase::Runtime, sp));
                }
                let base = self.rvalue(base)?;
                let vt = self.t.vec(k, s);
                if base.stage == Phase::Const {
                    let val = base.val.as_ref().map(|v| Value::Comp(cs.iter().map(|&c| comp(&self.t, v, store, c as usize)).collect()));
                    return Ok(TE { id: NONE, ty: vt, stage: Phase::Const, val, span: sp });
                }
                let bid = self.node(&base);
                Ok(self.mk(ir::ExprKind::Swizzle(bid, c4, k), vt, base.stage, sp))
            }
            _ => Err(self.err(name.span, format!("`{}` hat kein Member `{}`", self.tn(store), name.name))),
        }
    }

    // ---- operators

    fn unary(&mut self, op: UnOp, x: &Expr, sp: Span) -> R<TE> {
        match op {
            UnOp::AddrOf => {
                let r = self.expr(x)?;
                match self.t.get(r.ty) {
                    Type::Reference { space, store, access } => {
                        if space == AddressSpace::Handle {
                            return Err(self.err(sp, String::from("von Texturen und Samplern lässt sich keine Adresse nehmen")));
                        }
                        if self.is_vector_component(r.id) {
                            return Err(self.err(sp, String::from("von einer Vektorkomponente lässt sich keine Adresse nehmen")));
                        }
                        let pt = self.t.ptr(space, store, access);
                        Ok(self.mk(ir::ExprKind::AddrOf(r.id), pt, Phase::Runtime, sp))
                    }
                    Type::SwizzleView { .. } => Err(self.err(sp, String::from("von einem Swizzle lässt sich keine Adresse nehmen"))),
                    _ => Err(self.err(sp, format!("`&` braucht eine Variable oder einen Speicherbezug, nicht einen Wert vom Typ `{}`", self.tn(r.ty)))),
                }
            }
            UnOp::Deref => {
                let p = self.value(x)?;
                match self.t.get(p.ty) {
                    Type::Pointer { space, store, access } => {
                        let rt = self.t.reference(space, store, access);
                        Ok(self.mk(ir::ExprKind::Deref(p.id), rt, Phase::Runtime, sp))
                    }
                    _ => Err(self.err(sp, format!("`*` braucht einen Zeiger, nicht `{}`", self.tn(p.ty)))),
                }
            }
            _ => {
                let a = self.value(x)?;
                let (uop, set): (UnaryOp, &[Scalar]) = match op {
                    UnOp::Neg => (UnaryOp::Neg, &[Scalar::AbstractInt, Scalar::AbstractFloat, Scalar::I32, Scalar::F32, Scalar::F16]),
                    UnOp::Not => (UnaryOp::Not, &[Scalar::Bool]),
                    _ => (UnaryOp::BitNot, &[Scalar::AbstractInt, Scalar::I32, Scalar::U32]),
                };
                let mut cands = Vec::new();
                for &s in set {
                    if s == Scalar::F16 && !self.has(feature::F16) {
                        continue;
                    }
                    for n in 1..=4u8 {
                        let t = self.t.vec_or_scalar(n, s);
                        cands.push(Sig::new(&[t], t));
                    }
                }
                let sig = match self.resolve(core::slice::from_ref(&a), &cands) {
                    Ok(s) => s,
                    Err(_) => return Err(self.err(sp, format!("`{}` ist für `{}` nicht definiert", op.text(), self.tn(a.ty)))),
                };
                let a = self.convert(a, sig.p[0])?;
                if a.stage == Phase::Const {
                    let val = match &a.val {
                        Some(v) if self.no_eval == 0 => match consteval::unary(&self.t, uop, v, a.ty) {
                            Ok(v) => Some(v),
                            Err(m) => return Err(self.err(sp, m)),
                        },
                        _ => None,
                    };
                    return Ok(TE { id: NONE, ty: sig.ret, stage: Phase::Const, val, span: sp });
                }
                Ok(self.mk(ir::ExprKind::Unary(uop, a.id), sig.ret, a.stage, sp))
            }
        }
    }

    fn is_vector_component(&self, id: ExprId) -> bool {
        match self.kind_of(id) {
            Some(ir::ExprKind::Member(b, _)) | Some(ir::ExprKind::Index(b, _)) => {
                let bt = self.arena()[*b as usize].ty;
                matches!(self.t.store_of(bt).map(|s| self.t.get(s.1)), Some(Type::Vector(..)))
            }
            _ => false,
        }
    }

    /// the candidates of a binary operator for the operand types
    pub fn op_cands(&mut self, op: BinOp, ta: TypeId, tb: TypeId) -> Vec<Sig> {
        let f16 = self.has(feature::F16);
        let wa = self.t.width(ta);
        let wb = self.t.width(tb);
        let mut v = Vec::new();
        let nums = [Scalar::AbstractInt, Scalar::AbstractFloat, Scalar::I32, Scalar::U32, Scalar::F32, Scalar::F16];
        let set: Vec<Scalar> = match op {
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => nums.to_vec(),
            BinOp::Eq | BinOp::Ne => Scalar::ALL.to_vec(),
            BinOp::And | BinOp::Or => alloc::vec![Scalar::Bool, Scalar::AbstractInt, Scalar::I32, Scalar::U32],
            BinOp::Xor => alloc::vec![Scalar::AbstractInt, Scalar::I32, Scalar::U32],
            BinOp::Shl | BinOp::Shr => alloc::vec![Scalar::AbstractInt, Scalar::I32, Scalar::U32],
            BinOp::LogAnd | BinOp::LogOr => alloc::vec![Scalar::Bool],
        };
        for s in set {
            if s == Scalar::F16 && !f16 {
                continue;
            }
            for n in 1..=4u8 {
                let t = self.t.vec_or_scalar(n, s);
                if op.is_comparison() {
                    let b = self.t.vec_or_scalar(n, Scalar::Bool);
                    v.push(Sig::new(&[t, t], b));
                } else if matches!(op, BinOp::Shl | BinOp::Shr) {
                    let u = self.t.vec_or_scalar(n, Scalar::U32);
                    v.push(Sig::new(&[t, u], t));
                } else if matches!(op, BinOp::LogAnd | BinOp::LogOr) {
                    if n == 1 {
                        v.push(Sig::new(&[t, t], t));
                    }
                } else {
                    v.push(Sig::new(&[t, t], t));
                    if n > 1 && matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem) {
                        if wa == Some(1) || wb == Some(1) {
                            v.push(Sig::new(&[t, s.id()], t));
                            v.push(Sig::new(&[s.id(), t], t));
                        }
                    }
                }
            }
        }
        // matrices
        let is_mat = |t: Type| matches!(t, Type::Matrix(..));
        if is_mat(self.t.get(ta)) || is_mat(self.t.get(tb)) {
            for s in [Scalar::AbstractFloat, Scalar::F32, Scalar::F16] {
                if s == Scalar::F16 && !f16 {
                    continue;
                }
                for c in 2..=4u8 {
                    for r in 2..=4u8 {
                        let m = self.t.mat(c, r, s);
                        match op {
                            BinOp::Add | BinOp::Sub => v.push(Sig::new(&[m, m], m)),
                            BinOp::Mul => {
                                v.push(Sig::new(&[m, s.id()], m));
                                v.push(Sig::new(&[s.id(), m], m));
                                v.push(Sig::new(&[m, self.t.vec(c, s)], self.t.vec(r, s)));
                                v.push(Sig::new(&[self.t.vec(r, s), m], self.t.vec(c, s)));
                                for k in 2..=4u8 {
                                    // matKxR * matCxK
                                    let a = self.t.mat(k, r, s);
                                    let b = self.t.mat(c, k, s);
                                    v.push(Sig::new(&[a, b], m));
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
        v
    }

    fn binary(&mut self, op: BinOp, ae: &Expr, be: &Expr, sp: Span) -> R<TE> {
        let a = self.value(ae)?;
        // short-circuit: the right side of `false && x` / `true || x` is checked, not evaluated
        let short = matches!(op, BinOp::LogAnd | BinOp::LogOr)
            && a.stage == Phase::Const
            && a.ty == BOOL
            && a.val == Some(Value::Bool(op == BinOp::LogOr));
        if short {
            self.no_eval += 1;
        }
        let b = self.value(be);
        if short {
            self.no_eval -= 1;
        }
        let b = b?;
        self.binary_te(op, a, b, sp)
    }

    pub fn binary_te(&mut self, op: BinOp, a: TE, b: TE, sp: Span) -> R<TE> {
        let cands = self.op_cands(op, a.ty, b.ty);
        let args = [a, b];
        let sig = match self.resolve(&args, &cands) {
            Ok(s) => s,
            Err(_) => {
                let (x, y) = (self.tn(args[0].ty), self.tn(args[1].ty));
                return Err(self.err(sp, format!("`{}` ist für `{}` und `{}` nicht definiert", op.text(), x, y)));
            }
        };
        let [a, b] = args;
        let a = self.convert(a, sig.p[0])?;
        let b = self.convert(b, sig.p[1])?;
        let short_circuit = matches!(op, BinOp::LogAnd | BinOp::LogOr);
        if a.stage == Phase::Const && (b.stage == Phase::Const || short_circuit && a.val == Some(Value::Bool(op == BinOp::LogOr))) {
            if short_circuit && a.val == Some(Value::Bool(op == BinOp::LogOr)) {
                return Ok(TE { id: NONE, ty: BOOL, stage: Phase::Const, val: a.val.clone(), span: sp });
            }
            let val = match (&a.val, &b.val) {
                (Some(x), Some(y)) if self.no_eval == 0 => match self.fold_binary(op, x, a.ty, y, b.ty, sig.ret) {
                    Ok(v) => Some(v),
                    Err(m) => return Err(self.err(sp, m)),
                },
                _ => None,
            };
            return Ok(TE { id: NONE, ty: sig.ret, stage: Phase::Const, val, span: sp });
        }
        // a constant right operand of a concrete integer division / shift is checked now
        if b.stage == Phase::Const && self.no_eval == 0 {
            if let Some(v) = &b.val {
                let s = self.t.scalar_of(a.ty);
                let cs = if self.t.is_scalar(b.ty) { alloc::vec![v.clone()] } else { comps(&self.t, v, b.ty) };
                if matches!(op, BinOp::Div | BinOp::Rem) && matches!(s, Some(Scalar::I32 | Scalar::U32)) && cs.iter().any(|c| consteval::as_int(c) == Some(0)) {
                    return Err(self.err(b.span, String::from("ganzzahlige Division durch null")));
                }
                if matches!(op, BinOp::Shl | BinOp::Shr) && matches!(s, Some(Scalar::I32 | Scalar::U32)) && cs.iter().any(|c| consteval::as_int(c).is_some_and(|x| x >= 32)) {
                    return Err(self.err(b.span, String::from("die Schiebeweite muss kleiner als 32 sein")));
                }
            }
        }
        let stage = a.stage.max(b.stage);
        let (mut ai, mut bi) = (self.node(&a), self.node(&b));
        // mixed vector / scalar: splat the scalar (except float multiply)
        let (wa, wb) = (self.t.width(a.ty), self.t.width(b.ty));
        let float_mul = op == BinOp::Mul && self.t.scalar_of(a.ty).is_some_and(|s| s.is_float());
        if !matches!(op, BinOp::Shl | BinOp::Shr) && !float_mul {
            if wa == Some(1) && wb.is_some_and(|w| w > 1) {
                ai = self.push(ir::ExprKind::Splat(ai), b.ty, a.span);
            } else if wb == Some(1) && wa.is_some_and(|w| w > 1) {
                bi = self.push(ir::ExprKind::Splat(bi), a.ty, b.span);
            }
        }
        Ok(self.mk(ir::ExprKind::Binary(op, ai, bi), sig.ret, stage, sp))
    }

    fn broadcast(&self, v: &Value, n: usize) -> Value {
        Value::Comp((0..n).map(|_| v.clone()).collect())
    }

    fn fold_binary(&mut self, op: BinOp, a: &Value, ta: TypeId, b: &Value, tb: TypeId, ret: TypeId) -> Result<Value, String> {
        let (ka, kb) = (self.t.get(ta), self.t.get(tb));
        match (ka, kb) {
            (Type::Matrix(..), Type::Vector(..)) | (Type::Vector(..), Type::Matrix(..)) | (Type::Matrix(..), Type::Matrix(..)) if op == BinOp::Mul => {
                consteval::mat_mul(&self.t, a, ta, b, tb)
            }
            (Type::Matrix(c, r, _), Type::Scalar(_)) => {
                let col = self.broadcast(b, r as usize);
                let m = self.broadcast(&col, c as usize);
                consteval::binary(&self.t, op, a, &m, ta, ta)
            }
            (Type::Scalar(_), Type::Matrix(c, r, _)) => {
                let col = self.broadcast(a, r as usize);
                let m = self.broadcast(&col, c as usize);
                consteval::binary(&self.t, op, &m, b, tb, tb)
            }
            (Type::Vector(n, _), Type::Scalar(_)) if !matches!(op, BinOp::Shl | BinOp::Shr) => {
                let bv = self.broadcast(b, n as usize);
                consteval::binary(&self.t, op, a, &bv, ta, ta)
            }
            (Type::Scalar(_), Type::Vector(n, _)) => {
                let av = self.broadcast(a, n as usize);
                consteval::binary(&self.t, op, &av, b, tb, tb)
            }
            _ => {
                let _ = ret;
                consteval::binary(&self.t, op, a, b, ta, tb)
            }
        }
    }

    // ---- overload resolution (6.1.3)

    /// the most preferred candidate for these arguments (rvalue types), Err(()) without one
    pub fn resolve(&mut self, args: &[TE], cands: &[Sig]) -> Result<Sig, ()> {
        let mut viable: Vec<(Sig, Vec<u32>)> = Vec::new();
        'c: for c in cands {
            if c.n as usize != args.len() {
                continue;
            }
            let mut ranks = Vec::with_capacity(args.len());
            let mut abstract_param = false;
            for (a, &p) in args.iter().zip(c.params()) {
                let r = self.t.rank(a.ty, p);
                if r == INF {
                    continue 'c;
                }
                abstract_param |= self.t.is_abstract(p);
                ranks.push(r);
            }
            if abstract_param && args.iter().any(|a| a.stage != Phase::Const) {
                continue;
            }
            viable.push((*c, ranks));
        }
        if viable.is_empty() {
            return Err(());
        }
        'best: for i in 0..viable.len() {
            for j in 0..viable.len() {
                if i == j {
                    continue;
                }
                let (ri, rj) = (&viable[i].1, &viable[j].1);
                let le = ri.iter().zip(rj).all(|(x, y)| x <= y);
                let lt = ri.iter().zip(rj).any(|(x, y)| x < y);
                if !(le && lt) {
                    continue 'best;
                }
            }
            return Ok(viable[i].0);
        }
        Err(())
    }

    fn arg_list(&self, args: &[TE]) -> String {
        let v: Vec<String> = args.iter().map(|a| self.tn(a.ty)).collect();
        v.join(", ")
    }

    // ---- calls

    pub fn call(&mut self, id: &ast::Ident, targs: Option<&[Expr]>, args: &[Expr], sp: Span) -> R<TE> {
        if let Some((_, _, dsp)) = self.lookup_local(&id.name) {
            return Err(self.err_note(id.span, format!("`{}` ist keine Funktion", id.name), dsp, String::from("hier deklariert")));
        }
        match self.lookup_decl(&id.name)? {
            Some((i, info)) => {
                let dsp = self.decl_span(i);
                match info {
                    DeclInfo::Fn(fi) => {
                        if targs.is_some() {
                            return Err(self.err(id.span, format!("`{}` hat keine Template-Parameter", id.name)));
                        }
                        self.user_call(fi, id, args, sp)
                    }
                    DeclInfo::Alias(t) | DeclInfo::Struct(t) => {
                        if targs.is_some() {
                            return Err(self.err(id.span, format!("`{}` hat keine Template-Parameter", id.name)));
                        }
                        self.construct(t, args, sp)
                    }
                    _ => Err(self.err_note(id.span, format!("`{}` ist keine Funktion", id.name), dsp, String::from("hier deklariert"))),
                }
            }
            None => {
                if let Some((b, flags)) = builtins::lookup(&id.name) {
                    if targs.is_some() {
                        return Err(self.err(id.span, format!("`{}` hat keine Template-Parameter", id.name)));
                    }
                    return self.builtin_call(b, flags, id, args, sp);
                }
                if id.name == "bitcast" {
                    return self.bitcast(targs, args, sp);
                }
                let generic = matches!(id.name.as_str(), "vec2" | "vec3" | "vec4" | "array") || (id.name.len() == 6 && id.name.starts_with("mat") && alias_parts(&format!("{}f", id.name)).is_some());
                if generic && targs.is_none() {
                    return self.construct_inferred(&id.name, args, sp);
                }
                if predeclared_kind(&id.name).is_some_and(|k| k == "ein Typ" || k == "ein Typ-Generator") {
                    let te = Expr::new(ExprKind::Ident(id.clone(), targs.map(|t| t.to_vec())), Span::new(id.span.start, sp.end));
                    let t = self.ty(&te)?;
                    return self.construct(t, args, sp);
                }
                if builtins::SUBGROUP_NAMES.contains(&id.name.as_str()) {
                    return Err(self.err(id.span, format!("`{}` (subgroups / buffer_view) wird von diesem Compiler noch nicht unterstützt", id.name)));
                }
                match predeclared_kind(&id.name) {
                    Some(k) => Err(self.err(id.span, format!("`{}` ist {} und keine Funktion", id.name, k))),
                    None => Err(self.err(id.span, format!("die Funktion `{}` ist hier nicht bekannt", id.name))),
                }
            }
        }
    }

    fn user_call(&mut self, fi: u32, id: &ast::Ident, args: &[Expr], sp: Span) -> R<TE> {
        let info = self.fns[fi as usize].clone();
        if let Some(st) = info.stage {
            return Err(self.err_note(id.span, format!("`{}` ist ein {}-Einstiegspunkt und kann nicht aufgerufen werden", id.name, st.name()), info.span, String::from("hier deklariert")));
        }
        if args.len() != info.params.len() {
            return Err(self.err_note(
                sp,
                format!("`{}` erwartet {} Argument{}, nicht {}", id.name, info.params.len(), if info.params.len() == 1 { "" } else { "e" }, args.len()),
                info.span,
                String::from("hier deklariert"),
            ));
        }
        let mut ids = Vec::new();
        let mut roots: Vec<Option<Root>> = Vec::new();
        for (k, (a, &pt)) in args.iter().zip(&info.params).enumerate() {
            let te = self.value(a)?;
            let te = match self.convert(te, pt) {
                Ok(t) => t,
                Err(_) => {
                    if let Some(d) = self.diags.last_mut() {
                        d.msg = format!("Argument {} von `{}`: {}", k + 1, id.name, d.msg);
                    }
                    return Err(E);
                }
            };
            let nid = self.node(&te);
            let root = if self.t.is_ptr(pt) { self.root(nid) } else { None };
            if self.t.is_ptr(pt) && !self.has(feature::UNRESTRICTED_POINTER_PARAMETERS) && !self.whole_view(nid) {
                return Err(self.err(a.span, String::from("ein Zeiger-Argument muss auf eine ganze Variable zeigen (ohne unrestricted_pointer_parameters)")));
            }
            roots.push(root);
            ids.push(nid);
        }
        // alias analysis (11.4.1.2)
        for i in 0..roots.len() {
            let Some(ri) = roots[i] else { continue };
            for j in i + 1..roots.len() {
                if roots[j] == Some(ri) && (info.ptr_write[i] || info.ptr_write[j]) {
                    return Err(self.err(args[j].span, format!("die Argumente {} und {} von `{}` zeigen auf denselben Speicher, und einer davon wird beschrieben", i + 1, j + 1, id.name)));
                }
            }
            if let Root::Global(g) = ri {
                let clash = (info.ptr_write[i] && (info.g_read.contains(&g) || info.g_write.contains(&g))) || (info.ptr_read[i] && info.g_write.contains(&g));
                if clash {
                    let gname = self.m.globals[g as usize].name.clone();
                    return Err(self.err(args[i].span, format!("`{}` greift selbst auf `{}` zu, das hier als Zeiger übergeben wird (Aliasing)", id.name, gname)));
                }
            }
        }
        if let Some(f) = self.f.as_mut() {
            for (k, r) in roots.iter().enumerate() {
                if let Some(r) = r {
                    if info.ptr_read[k] {
                        f.reads.insert(*r);
                    }
                    if info.ptr_write[k] {
                        f.writes.insert(*r);
                    }
                }
            }
            for g in &info.g_read {
                f.reads.insert(Root::Global(*g));
            }
            for g in &info.g_write {
                f.writes.insert(Root::Global(*g));
            }
            f.calls.insert(fi);
            f.info.globals.extend(info.globals.iter().copied());
            f.info.overrides.extend(info.overrides.iter().copied());
            f.info.uses_discard |= info.uses_discard;
            if f.info.frag_use.is_none() {
                f.info.frag_use = info.frag_use.clone();
            }
            if f.info.compute_use.is_none() {
                f.info.compute_use = info.compute_use.clone();
            }
        }
        let ty = info.ret.unwrap_or(builtins::NONE);
        Ok(self.mk(ir::ExprKind::Call(fi, ids), ty, Phase::Runtime, sp))
    }

    /// a pointer to a whole variable (without unrestricted_pointer_parameters)
    fn whole_view(&self, id: ExprId) -> bool {
        let Some(f) = self.f.as_ref() else { return false };
        let mut cur = id;
        for _ in 0..1000 {
            match f.exprs.get(cur as usize).map(|e| &e.kind) {
                Some(ir::ExprKind::AddrOf(b)) => match f.exprs.get(*b as usize).map(|e| &e.kind) {
                    Some(ir::ExprKind::Global(_)) => return true,
                    Some(ir::ExprKind::Local(l)) => return f.locals.get(*l as usize).is_some_and(|x| x.kind == LocalKind::Var),
                    Some(ir::ExprKind::Deref(p)) => cur = *p,
                    _ => return false,
                },
                Some(ir::ExprKind::Param(_)) => return true,
                Some(ir::ExprKind::Local(l)) => match f.local_init.get(*l as usize) {
                    Some(Some(i)) => cur = *i,
                    _ => return false,
                },
                _ => return false,
            }
        }
        false
    }

    // ---- constructors and conversions

    fn args_of(&mut self, args: &[Expr]) -> R<Vec<TE>> {
        let mut v = Vec::with_capacity(args.len());
        for a in args {
            v.push(self.value(a)?);
        }
        Ok(v)
    }

    /// a constructor or conversion of a known type
    pub fn construct(&mut self, t: TypeId, args: &[Expr], sp: Span) -> R<TE> {
        let tname = self.tn(t);
        if !self.t.is_constructible(t) {
            return Err(self.err(sp, format!("`{}` ist nicht konstruierbar", tname)));
        }
        let vals = self.args_of(args)?;
        if vals.is_empty() {
            return Ok(TE { id: NONE, ty: t, stage: Phase::Const, val: Some(consteval::zero_of(&self.t, t)), span: sp });
        }
        match self.t.get(t) {
            Type::Scalar(s) => {
                if vals.len() != 1 {
                    return Err(self.err(sp, format!("`{}(…)` nimmt genau ein Argument", tname)));
                }
                let a = vals.into_iter().next().ok_or(E)?;
                let mut cands = Vec::new();
                for p in Scalar::ALL {
                    if p == Scalar::F16 && !self.has(feature::F16) {
                        continue;
                    }
                    if s == Scalar::F32 && p.is_abstract() {
                        continue;
                    }
                    cands.push(Sig::new(&[p.id()], t));
                }
                let Ok(sig) = self.resolve(core::slice::from_ref(&a), &cands) else {
                    return Err(self.err(sp, format!("`{}` lässt sich nicht aus `{}` erzeugen", tname, self.tn(a.ty))));
                };
                let a = self.convert(a, sig.p[0])?;
                self.conversion(a, t, sp)
            }
            Type::Vector(n, s) => {
                if vals.len() == 1 && self.t.width(vals[0].ty) == Some(n) {
                    // conversion from vecN<S> (S any scalar: the argument's own)
                    let a = vals.into_iter().next().ok_or(E)?;
                    return self.conversion(a, t, sp);
                }
                if vals.len() == 1 && self.t.width(vals[0].ty) == Some(1) {
                    let a = vals.into_iter().next().ok_or(E)?;
                    let a = self.convert(a, s.id())?;
                    return self.splat(a, t, sp);
                }
                self.components(vals, n, s, t, sp)
            }
            Type::Matrix(c, r, s) => {
                if vals.len() == 1 && matches!(self.t.get(vals[0].ty), Type::Matrix(c2, r2, _) if c2 == c && r2 == r) {
                    let a = vals.into_iter().next().ok_or(E)?;
                    return self.conversion(a, t, sp);
                }
                self.matrix_from(vals, c, r, s, t, sp)
            }
            Type::Array { elem, count: ArrayCount::Fixed(n), .. } => {
                if vals.len() != n as usize {
                    return Err(self.err(sp, format!("`{}` braucht {} Elemente, nicht {}", tname, n, vals.len())));
                }
                let mut conv = Vec::with_capacity(vals.len());
                for v in vals {
                    conv.push(self.convert(v, elem)?);
                }
                self.composite(conv, t, sp)
            }
            Type::Struct(si) => {
                let mtys: Vec<TypeId> = self.t.structs[si as usize].members.iter().map(|m| m.ty).collect();
                if vals.len() != mtys.len() {
                    return Err(self.err(sp, format!("`{}` hat {} Member, angegeben sind {}", tname, mtys.len(), vals.len())));
                }
                let mut conv = Vec::with_capacity(vals.len());
                for (v, mt) in vals.into_iter().zip(mtys) {
                    conv.push(self.convert(v, mt)?);
                }
                self.composite(conv, t, sp)
            }
            _ => Err(self.err(sp, format!("`{}` ist nicht konstruierbar", tname))),
        }
    }

    /// a value conversion (scalar, vector, matrix) to t; identity when the types are equal
    fn conversion(&mut self, a: TE, t: TypeId, sp: Span) -> R<TE> {
        if a.ty == t {
            let mut a = a;
            a.span = sp;
            return Ok(a);
        }
        let ok = match (self.t.get(a.ty), self.t.get(t)) {
            (Type::Scalar(_), Type::Scalar(_)) => true,
            (Type::Vector(n, _), Type::Vector(m, _)) => n == m,
            (Type::Matrix(c, r, x), Type::Matrix(c2, r2, y)) => c == c2 && r == r2 && x.is_float() && y.is_float(),
            _ => false,
        };
        if !ok {
            return Err(self.err(sp, format!("`{}` lässt sich nicht aus `{}` erzeugen", self.tn(t), self.tn(a.ty))));
        }
        if a.stage == Phase::Const {
            let val = match &a.val {
                Some(v) if self.no_eval == 0 => match consteval::convert(&mut self.t, v, a.ty, t, true) {
                    Ok(v) => Some(v),
                    Err(m) => return Err(self.err(sp, m)),
                },
                _ => None,
            };
            return Ok(TE { id: NONE, ty: t, stage: Phase::Const, val, span: sp });
        }
        let a = if self.t.is_abstract(a.ty) { self.concretize(a)? } else { a };
        if a.ty == t {
            return Ok(a);
        }
        Ok(self.mk(ir::ExprKind::Convert(a.id), t, a.stage, sp))
    }

    fn splat(&mut self, a: TE, t: TypeId, sp: Span) -> R<TE> {
        if a.stage == Phase::Const {
            let n = self.t.width(t).unwrap_or(1) as usize;
            let val = a.val.as_ref().map(|v| self.broadcast(v, n));
            return Ok(TE { id: NONE, ty: t, stage: Phase::Const, val, span: sp });
        }
        Ok(self.mk(ir::ExprKind::Splat(a.id), t, a.stage, sp))
    }

    /// a composite from converted parts (array, struct, vector components, matrix columns)
    fn composite(&mut self, parts: Vec<TE>, t: TypeId, sp: Span) -> R<TE> {
        if parts.iter().all(|p| p.stage == Phase::Const) {
            if consteval::elem_count(&self.t, t) > consteval::MAX_ELEMS {
                return Err(self.err(sp, String::from("die Konstante ist zu groß")));
            }
            let val = if parts.iter().all(|p| p.val.is_some()) { Some(Value::Comp(parts.into_iter().map(|p| p.val.unwrap_or(Value::Zero)).collect())) } else { None };
            return Ok(TE { id: NONE, ty: t, stage: Phase::Const, val, span: sp });
        }
        let stage = parts.iter().map(|p| p.stage).max().unwrap_or(Phase::Runtime);
        let ids: Vec<ExprId> = parts.iter().map(|p| self.node(p)).collect();
        Ok(self.mk(ir::ExprKind::Construct(ids), t, stage, sp))
    }

    /// vecN<s>(components): scalars and vectors of s adding up to n
    fn components(&mut self, vals: Vec<TE>, n: u8, s: Scalar, t: TypeId, sp: Span) -> R<TE> {
        let mut total = 0u32;
        let mut conv = Vec::with_capacity(vals.len());
        for v in vals {
            let w = match self.t.get(v.ty) {
                Type::Scalar(_) => 1,
                Type::Vector(k, _) => k,
                _ => return Err(self.err(v.span, format!("`{}` kann keine Komponente von `{}` sein", self.tn(v.ty), self.tn(t)))),
            };
            total += w as u32;
            let target = self.t.vec_or_scalar(w, s);
            conv.push(self.convert(v, target)?);
        }
        if total != n as u32 {
            return Err(self.err(sp, format!("`{}` braucht {} Komponenten, angegeben sind {}", self.tn(t), n, total)));
        }
        if conv.iter().all(|p| p.stage == Phase::Const) {
            let mut flat = Vec::new();
            let mut known = true;
            for p in &conv {
                match &p.val {
                    Some(v) => {
                        if self.t.is_scalar(p.ty) {
                            flat.push(v.clone());
                        } else {
                            flat.extend(comps(&self.t, v, p.ty));
                        }
                    }
                    None => known = false,
                }
            }
            return Ok(TE { id: NONE, ty: t, stage: Phase::Const, val: if known { Some(Value::Comp(flat)) } else { None }, span: sp });
        }
        let stage = conv.iter().map(|p| p.stage).max().unwrap_or(Phase::Runtime);
        let ids: Vec<ExprId> = conv.iter().map(|p| self.node(p)).collect();
        Ok(self.mk(ir::ExprKind::Construct(ids), t, stage, sp))
    }

    /// matCxR<s>(columns) or (scalars); scalars are grouped into column constructs
    fn matrix_from(&mut self, vals: Vec<TE>, c: u8, r: u8, s: Scalar, t: TypeId, sp: Span) -> R<TE> {
        let col = self.t.vec(r, s);
        if vals.len() == c as usize {
            let mut cols = Vec::with_capacity(c as usize);
            for v in vals {
                cols.push(self.convert(v, col)?);
            }
            return self.composite(cols, t, sp);
        }
        if vals.len() == c as usize * r as usize {
            let mut cols = Vec::with_capacity(c as usize);
            let mut it = vals.into_iter();
            for _ in 0..c {
                let mut comps_ = Vec::with_capacity(r as usize);
                for _ in 0..r {
                    let v = it.next().ok_or(E)?;
                    comps_.push(self.convert(v, s.id())?);
                }
                let cs = self.composite(comps_, col, sp)?;
                cols.push(cs);
            }
            return self.composite(cols, t, sp);
        }
        Err(self.err(sp, format!("`{}` braucht {} Spalten oder {} Skalare", self.tn(t), c, c as u32 * r as u32)))
    }

    /// vecN(…), matCxR(…), array(…) without template parameters: the component type is
    /// inferred by overload resolution
    fn construct_inferred(&mut self, generator: &str, args: &[Expr], sp: Span) -> R<TE> {
        let vals = self.args_of(args)?;
        let f16 = self.has(feature::F16);
        if generator == "array" {
            if vals.is_empty() {
                return Err(self.err(sp, String::from("`array()` braucht Elemente oder einen Elementtyp: `array<f32, 4>()`")));
            }
            // candidate element types: the argument types and what abstract ones convert to
            let mut cand_tys: Vec<TypeId> = Vec::new();
            for v in &vals {
                let t = v.ty;
                if !cand_tys.contains(&t) {
                    cand_tys.push(t);
                }
                if self.t.is_abstract(t) {
                    for s in [Scalar::AbstractFloat, Scalar::I32, Scalar::U32, Scalar::F32, Scalar::F16] {
                        if s == Scalar::F16 && !f16 {
                            continue;
                        }
                        let c = self.t.with_scalar(t, s);
                        if self.t.rank(t, c) != INF && !cand_tys.contains(&c) {
                            cand_tys.push(c);
                        }
                    }
                }
            }
            let n = vals.len();
            let cands: Vec<Sig> = cand_tys.iter().filter(|&&t| self.t.is_constructible(t)).map(|&t| Sig { p: [t; 8], n: 8, ret: t }).collect();
            let best = self.resolve_homogeneous(&vals, &cands);
            let Some(elem) = best else {
                return Err(self.err(sp, format!("die Elemente von `array(…)` haben keinen gemeinsamen Typ ({})", self.arg_list(&vals))));
            };
            let at = self.t.array(elem, ArrayCount::Fixed(n as u32));
            let mut conv = Vec::with_capacity(n);
            for v in vals {
                conv.push(self.convert(v, elem)?);
            }
            return self.composite(conv, at, sp);
        }
        let is_vec = generator.starts_with("vec");
        let (c, r) = (generator.as_bytes()[3] - b'0', if is_vec { 0 } else { generator.as_bytes()[5] - b'0' });
        if vals.is_empty() {
            if is_vec {
                let t = self.t.vec(c, Scalar::AbstractInt);
                return Ok(TE { id: NONE, ty: t, stage: Phase::Const, val: Some(Value::Zero), span: sp });
            }
            return Err(self.err(sp, format!("`{}()` braucht einen Komponententyp: `{}<f32>()`", generator, generator)));
        }
        // identity / conversion forms keep the argument's component type
        let set: Vec<Scalar> = if is_vec { Scalar::ALL.to_vec() } else { alloc::vec![Scalar::AbstractFloat, Scalar::F32, Scalar::F16] };
        if vals.len() == 1 {
            let at = vals[0].ty;
            match self.t.get(at) {
                Type::Vector(k, _) if is_vec && k == c => return Ok(vals.into_iter().next().ok_or(E)?),
                Type::Matrix(c2, r2, _) if !is_vec && c2 == c && r2 == r => return Ok(vals.into_iter().next().ok_or(E)?),
                Type::Scalar(s) if is_vec => {
                    let t = self.t.vec(c, s);
                    let a = vals.into_iter().next().ok_or(E)?;
                    return self.splat(a, t, sp);
                }
                _ => {}
            }
        }
        // the component scalar: the best of the set for every argument's components
        let mut best: Option<(Scalar, Vec<u32>)> = None;
        let mut tie = false;
        for &s in &set {
            if s == Scalar::F16 && !f16 {
                continue;
            }
            let mut ranks = Vec::new();
            let mut ok = true;
            for v in &vals {
                let target = match self.t.get(v.ty) {
                    Type::Scalar(_) => s.id(),
                    Type::Vector(k, _) => self.t.vec(k, s),
                    _ => {
                        ok = false;
                        break;
                    }
                };
                let rk = self.t.rank(v.ty, target);
                if rk == INF || (s.is_abstract() && v.stage != Phase::Const) {
                    ok = false;
                    break;
                }
                ranks.push(rk);
            }
            if !ok {
                continue;
            }
            match &best {
                None => best = Some((s, ranks)),
                Some((_, br)) => {
                    let le = ranks.iter().zip(br).all(|(x, y)| x <= y);
                    let lt = ranks.iter().zip(br).any(|(x, y)| x < y);
                    if le && lt {
                        best = Some((s, ranks));
                        tie = false;
                    } else if !(br.iter().zip(&ranks).all(|(x, y)| x <= y) && br.iter().zip(&ranks).any(|(x, y)| x < y)) {
                        tie = true;
                    }
                }
            }
        }
        let Some((s, _)) = best.filter(|_| !tie) else {
            return Err(self.err(sp, format!("`{}({})`: die Komponenten haben keinen gemeinsamen Typ", generator, self.arg_list(&vals))));
        };
        if is_vec {
            let t = self.t.vec(c, s);
            return self.components(vals, c, s, t, sp);
        }
        let t = self.t.mat(c, r, s);
        self.matrix_from(vals, c, r, s, t, sp)
    }

    /// overload resolution where every argument goes to the same type (array elements)
    fn resolve_homogeneous(&mut self, vals: &[TE], cands: &[Sig]) -> Option<TypeId> {
        let mut viable: Vec<(TypeId, Vec<u32>)> = Vec::new();
        for c in cands {
            let t = c.ret;
            let ranks: Vec<u32> = vals.iter().map(|v| self.t.rank(v.ty, t)).collect();
            if ranks.contains(&INF) {
                continue;
            }
            if self.t.is_abstract(t) && vals.iter().any(|v| v.stage != Phase::Const) {
                continue;
            }
            viable.push((t, ranks));
        }
        'b: for i in 0..viable.len() {
            for j in 0..viable.len() {
                if i == j {
                    continue;
                }
                let (ri, rj) = (&viable[i].1, &viable[j].1);
                if !(ri.iter().zip(rj).all(|(x, y)| x <= y) && ri.iter().zip(rj).any(|(x, y)| x < y)) {
                    continue 'b;
                }
            }
            return Some(viable[i].0);
        }
        None
    }

    fn bitcast(&mut self, targs: Option<&[Expr]>, args: &[Expr], sp: Span) -> R<TE> {
        let Some(ta) = targs.filter(|t| t.len() == 1) else {
            return Err(self.err(sp, String::from("`bitcast` braucht genau einen Zieltyp: `bitcast<u32>(x)`")));
        };
        let t = self.ty(&ta[0])?;
        if args.len() != 1 {
            return Err(self.err(sp, String::from("`bitcast` nimmt genau ein Argument")));
        }
        let a = self.value(&args[0])?;
        let tsize = |c: &Self, t: TypeId| -> Option<u32> {
            match c.t.get(t) {
                Type::Scalar(s) | Type::Vector(_, s) if matches!(s, Scalar::I32 | Scalar::U32 | Scalar::F32 | Scalar::F16) => Some(c.t.width(t)? as u32 * s.size()),
                _ => None,
            }
        };
        let Some(size) = tsize(self, t) else {
            return Err(self.err(ta[0].span, format!("`bitcast` kann nicht nach `{}` umwandeln", self.tn(t))));
        };
        // candidate source types of the same size
        let mut cands = alloc::vec![Sig::new(&[t], t)];
        for s in [Scalar::I32, Scalar::U32, Scalar::F32, Scalar::F16] {
            if s == Scalar::F16 && !self.has(feature::F16) {
                continue;
            }
            for n in 1..=4u8 {
                let src = self.t.vec_or_scalar(n, s);
                // 32-bit words: f16 only in pairs
                if src != t && n as u32 * s.size() == size && (s != Scalar::F16 || n % 2 == 0) {
                    cands.push(Sig::new(&[src], t));
                }
            }
        }
        if self.t.scalar_of(t) == Some(Scalar::U32) {
            let w = self.t.width(t).unwrap_or(1);
            cands.push(Sig::new(&[self.t.vec_or_scalar(w, Scalar::AbstractInt)], t));
        }
        let Ok(sig) = self.resolve(core::slice::from_ref(&a), &cands) else {
            return Err(self.err(sp, format!("`bitcast<{}>` geht nicht von `{}` (die Größe muss gleich sein)", self.tn(t), self.tn(a.ty))));
        };
        let from_abstract = self.t.is_abstract(sig.p[0]);
        let a = if from_abstract {
            let u = self.t.with_scalar(sig.p[0], Scalar::U32);
            let a = self.convert(a, sig.p[0])?;
            self.convert_explicit_abstract(a, u)?
        } else {
            self.convert(a, sig.p[0])?
        };
        if a.ty == t {
            return Ok(a);
        }
        if a.stage == Phase::Const {
            let val = match &a.val {
                Some(v) if self.no_eval == 0 => match self.bitcast_value(v, a.ty, t) {
                    Ok(v) => Some(v),
                    Err(m) => return Err(self.err(sp, m)),
                },
                _ => None,
            };
            return Ok(TE { id: NONE, ty: t, stage: Phase::Const, val, span: sp });
        }
        Ok(self.mk(ir::ExprKind::Bitcast(a.id), t, a.stage, sp))
    }

    /// AbstractInt -> u32 for bitcast (must fit)
    fn convert_explicit_abstract(&mut self, a: TE, to: TypeId) -> R<TE> {
        let val = match &a.val {
            Some(v) => match consteval::convert(&mut self.t, v, a.ty, to, false) {
                Ok(v) => Some(v),
                Err(m) => return Err(self.err(a.span, m)),
            },
            None => None,
        };
        Ok(TE { id: NONE, ty: to, stage: Phase::Const, val, span: a.span })
    }

    fn bitcast_value(&self, v: &Value, from: TypeId, to: TypeId) -> Result<Value, String> {
        // flatten to 32-bit words (f16 pairs packed), then rebuild
        let cs = if self.t.is_scalar(from) { alloc::vec![v.clone()] } else { comps(&self.t, v, from) };
        let mut words: Vec<u32> = Vec::new();
        let mut half: Option<u16> = None;
        for c in &cs {
            match *c {
                Value::I32(x) => words.push(x as u32),
                Value::U32(x) => words.push(x),
                Value::F32(x) => words.push(x.to_bits()),
                Value::F16(x) => {
                    let b = f16_bits(x);
                    match half.take() {
                        None => half = Some(b),
                        Some(lo) => words.push(lo as u32 | (b as u32) << 16),
                    }
                }
                _ => words.push(0),
            }
        }
        let s = self.t.scalar_of(to).unwrap_or(Scalar::U32);
        let mut out = Vec::new();
        for w in words {
            match s {
                Scalar::I32 => out.push(Value::I32(w as i32)),
                Scalar::U32 => out.push(Value::U32(w)),
                Scalar::F32 => {
                    let f = f32::from_bits(w);
                    if !f.is_finite() {
                        return Err(String::from("bitcast ergibt NaN oder unendlich, das ist in einem const-Ausdruck nicht erlaubt"));
                    }
                    out.push(Value::F32(f));
                }
                _ => {
                    for h in [w as u16, (w >> 16) as u16] {
                        let f = builtins::f16_from_bits(h);
                        if !f.is_finite() {
                            return Err(String::from("bitcast ergibt NaN oder unendlich, das ist in einem const-Ausdruck nicht erlaubt"));
                        }
                        out.push(Value::F16(f));
                    }
                }
            }
        }
        Ok(if self.t.is_scalar(to) { out.into_iter().next().unwrap_or(Value::Zero) } else { Value::Comp(out) })
    }

    // ---- built-in calls

    fn note_stage_use(&mut self, flags: u8, sp: Span, name: &str) {
        if let Some(f) = self.f.as_mut() {
            if flags & builtins::FRAG != 0 && f.info.frag_use.is_none() {
                f.info.frag_use = Some((sp, String::from(name)));
            }
            if flags & builtins::COMPUTE != 0 && f.info.compute_use.is_none() {
                f.info.compute_use = Some((sp, String::from(name)));
            }
        }
    }

    fn builtin_call(&mut self, b: Builtin, flags: u8, id: &ast::Ident, args: &[Expr], sp: Span) -> R<TE> {
        let feat = builtins::feature(b);
        if feat != 0 && !self.has(feat) {
            let what = if feat == feature::ATOMIC_VEC2U_MIN_MAX { "`enable atomic_vec2u_min_max;`" } else { "eine Spracherweiterung, die hier nicht verfügbar ist" };
            return Err(self.err(id.span, format!("`{}` braucht {}", id.name, what)));
        }
        if self.f.is_none() || self.module_arena {
            if flags & builtins::CONST == 0 {
                return Err(self.err(sp, format!("`{}` ist in const- und override-Ausdrücken nicht erlaubt", id.name)));
            }
        }
        self.note_stage_use(flags, sp, &id.name);
        if builtins::is_texture_fn(b) {
            return self.texture_call(b, id, args, sp);
        }
        use Builtin::*;
        match b {
            AtomicLoad | AtomicStore | AtomicAdd | AtomicSub | AtomicMax | AtomicMin | AtomicAnd | AtomicOr | AtomicXor | AtomicExchange
            | AtomicCompareExchangeWeak | AtomicStoreMin | AtomicStoreMax => return self.atomic_call(b, id, args, sp),
            ArrayLength => {
                let vals = self.args_of(args)?;
                if vals.len() != 1 {
                    return Err(self.err(sp, String::from("`arrayLength` nimmt genau ein Argument")));
                }
                let ok = matches!(self.t.get(vals[0].ty), Type::Pointer { space: AddressSpace::Storage, store, .. } if matches!(self.t.get(store), Type::Array { count: ArrayCount::Runtime, .. }));
                if !ok {
                    return Err(self.err(args[0].span, format!("`arrayLength` braucht einen Zeiger auf ein Laufzeit-Array im storage-Adressraum, nicht `{}`", self.tn(vals[0].ty))));
                }
                return Ok(self.mk(ir::ExprKind::Builtin(b, alloc::vec![vals[0].id]), U32, Phase::Runtime, sp));
            }
            WorkgroupUniformLoad => {
                let vals = self.args_of(args)?;
                if vals.len() != 1 {
                    return Err(self.err(sp, String::from("`workgroupUniformLoad` nimmt genau ein Argument")));
                }
                let ret = match self.t.get(vals[0].ty) {
                    Type::Pointer { space: AddressSpace::Workgroup, store, .. } => match self.t.get(store) {
                        Type::Atomic(inner) => Some(inner),
                        _ if self.t.is_constructible(store) => Some(store),
                        _ => None,
                    },
                    _ => None,
                };
                let Some(ret) = ret else {
                    return Err(self.err(args[0].span, format!("`workgroupUniformLoad` braucht einen Zeiger in den workgroup-Adressraum, nicht `{}`", self.tn(vals[0].ty))));
                };
                if let (Some(r), Some(f)) = (self.root(vals[0].id), self.f.as_mut()) {
                    f.reads.insert(r);
                }
                return Ok(self.mk(ir::ExprKind::Builtin(b, alloc::vec![vals[0].id]), ret, Phase::Runtime, sp));
            }
            _ => {}
        }
        let vals = self.args_of(args)?;
        let f16 = self.has(feature::F16);
        let cands = builtins::candidates(b, &mut self.t, f16);
        let Ok(sig) = self.resolve(&vals, &cands) else {
            return Err(self.err(sp, format!("keine passende Überladung von `{}({})`", id.name, self.arg_list(&vals))));
        };
        let mut conv = Vec::with_capacity(vals.len());
        for (v, &p) in vals.into_iter().zip(sig.params()) {
            conv.push(self.convert(v, p)?);
        }
        self.builtin_const_checks(b, &conv)?;
        let all_const = conv.iter().all(|c| c.stage == Phase::Const);
        if flags & builtins::CONST != 0 && all_const {
            let val = if self.no_eval == 0 && conv.iter().all(|c| c.val.is_some()) {
                let a: Vec<(Value, TypeId)> = conv.iter().map(|c| (c.val.clone().unwrap_or(Value::Zero), c.ty)).collect();
                match builtins::eval(&mut self.t, b, &a) {
                    Ok(v) => Some(v),
                    Err(m) => return Err(self.err(sp, m)),
                }
            } else {
                None
            };
            return Ok(TE { id: NONE, ty: sig.ret, stage: Phase::Const, val, span: sp });
        }
        let stage = if flags & builtins::CONST != 0 { conv.iter().map(|c| c.stage).max().unwrap_or(Phase::Runtime) } else { Phase::Runtime };
        let ids: Vec<ExprId> = conv.iter().map(|c| self.node(c)).collect();
        Ok(self.mk(ir::ExprKind::Builtin(b, ids), sig.ret, stage, sp))
    }

    /// errors a built-in reports when some of its arguments are constants
    fn builtin_const_checks(&mut self, b: Builtin, a: &[TE]) -> R<()> {
        if self.no_eval > 0 {
            return Ok(());
        }
        let c = |i: usize| a.get(i).filter(|x| x.stage == Phase::Const).and_then(|x| x.val.clone());
        let ints = |v: &Value| -> Vec<i128> { match v { Value::Comp(cs) => cs.iter().filter_map(consteval::as_int).collect(), _ => consteval::as_int(v).into_iter().collect() } };
        match b {
            Builtin::ExtractBits | Builtin::InsertBits => {
                let (oi, ci) = if b == Builtin::ExtractBits { (1, 2) } else { (2, 3) };
                if let (Some(o), Some(n)) = (c(oi), c(ci)) {
                    let (o, n) = (consteval::as_int(&o).unwrap_or(0), consteval::as_int(&n).unwrap_or(0));
                    if o + n > 32 {
                        return Err(self.err(a[oi].span.to(a[ci].span), format!("offset + count = {} ist größer als 32", o + n)));
                    }
                }
            }
            Builtin::Clamp => {
                if let (Some(lo), Some(hi)) = (c(1), c(2)) {
                    let (lv, hv) = (comps(&self.t, &lo, a[1].ty), comps(&self.t, &hi, a[2].ty));
                    let bad = if self.t.is_scalar(a[1].ty) { consteval::as_f64(&lo) > consteval::as_f64(&hi) } else { lv.iter().zip(&hv).any(|(x, y)| consteval::as_f64(x) > consteval::as_f64(y)) };
                    if bad {
                        return Err(self.err(a[1].span.to(a[2].span), String::from("clamp: die Untergrenze ist größer als die Obergrenze")));
                    }
                }
            }
            Builtin::Smoothstep => {
                if let (Some(e0), Some(e1)) = (c(0), c(1)) {
                    let eq = if self.t.is_scalar(a[0].ty) { consteval::as_f64(&e0) == consteval::as_f64(&e1) } else { comps(&self.t, &e0, a[0].ty).iter().zip(comps(&self.t, &e1, a[1].ty)).any(|(x, y)| consteval::as_f64(x) == consteval::as_f64(&y)) };
                    if eq {
                        return Err(self.err(a[0].span.to(a[1].span), String::from("smoothstep: edge0 und edge1 dürfen nicht gleich sein")));
                    }
                }
            }
            Builtin::Ldexp => {
                if let Some(e) = c(1) {
                    let bias = match self.t.scalar_of(a[0].ty) {
                        Some(Scalar::F32) => 127,
                        Some(Scalar::F16) => 15,
                        _ => 1023,
                    };
                    if ints(&if self.t.is_scalar(a[1].ty) { e.clone() } else { Value::Comp(comps(&self.t, &e, a[1].ty)) }).iter().any(|&x| x > bias + 1) {
                        return Err(self.err(a[1].span, format!("ldexp: der Exponent ist größer als {}", bias + 1)));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn texture_call(&mut self, b: Builtin, id: &ast::Ident, args: &[Expr], sp: Span) -> R<TE> {
        let vals = self.args_of(args)?;
        let ovs = builtins::texture_overloads(b);
        'o: for (ps, ret) in ovs {
            if ps.len() != vals.len() {
                continue;
            }
            let mut tex: Option<(TexDim, TexKind)> = None;
            for (v, p) in vals.iter().zip(ps.iter()) {
                if let builtins::P::T(tp) = p {
                    let Type::Texture(dim, kind) = self.t.get(v.ty) else { continue 'o };
                    if !tex_matches(*tp, dim, kind) {
                        continue 'o;
                    }
                    tex = Some((dim, kind));
                }
            }
            // the parameter types for this texture
            let mut targets = Vec::with_capacity(ps.len());
            for (v, p) in vals.iter().zip(ps.iter()) {
                let t = match p {
                    builtins::P::T(_) => v.ty,
                    builtins::P::Sampler => self.t.intern(Type::Sampler { comparison: false }),
                    builtins::P::SamplerCmp => self.t.intern(Type::Sampler { comparison: true }),
                    builtins::P::F32 => F32,
                    builtins::P::V(n) => self.t.vec(*n, Scalar::F32),
                    builtins::P::Offset(n) => self.t.vec(*n, Scalar::I32),
                    builtins::P::Int | builtins::P::Component => {
                        if v.ty == U32 {
                            U32
                        } else {
                            I32
                        }
                    }
                    builtins::P::IntV(n) => {
                        if self.t.scalar_of(v.ty) == Some(Scalar::U32) {
                            self.t.vec(*n, Scalar::U32)
                        } else {
                            self.t.vec(*n, Scalar::I32)
                        }
                    }
                    builtins::P::Texel => {
                        let ch = match tex {
                            Some((_, TexKind::Storage(f, _))) => f.channel(),
                            _ => Scalar::F32,
                        };
                        self.t.vec(4, ch)
                    }
                };
                if self.t.rank(v.ty, t) == INF {
                    continue 'o;
                }
                targets.push(t);
            }
            // matched: convert, check the constant parameters
            let mut ids = Vec::with_capacity(vals.len());
            for ((v, t), p) in vals.into_iter().zip(targets).zip(ps.iter()) {
                let cv = self.convert(v, t)?;
                match p {
                    builtins::P::Component => {
                        if cv.stage != Phase::Const {
                            return Err(self.err(cv.span, String::from("die Komponente von `textureGather` muss ein const-Ausdruck sein")));
                        }
                        if let Some(k) = cv.val.as_ref().and_then(consteval::as_int) {
                            if !(0..=3).contains(&k) {
                                return Err(self.err(cv.span, format!("die Komponente von `textureGather` muss 0 bis 3 sein, nicht {}", k)));
                            }
                        }
                    }
                    builtins::P::Offset(_) => {
                        if cv.stage != Phase::Const {
                            return Err(self.err(cv.span, String::from("der Offset muss ein const-Ausdruck sein")));
                        }
                        if let Some(v) = &cv.val {
                            if comps(&self.t, v, cv.ty).iter().any(|c| consteval::as_int(c).is_some_and(|x| !(-8..=7).contains(&x))) {
                                return Err(self.err(cv.span, String::from("jede Komponente des Offsets muss zwischen -8 und 7 liegen")));
                            }
                        }
                    }
                    _ => {}
                }
                ids.push(self.node(&cv));
            }
            let (dim, kind) = tex.unwrap_or((TexDim::D2, TexKind::External));
            let rt = match ret {
                TR::V4F => self.t.vec(4, Scalar::F32),
                TR::F32 => F32,
                TR::V4S => {
                    let s = match kind {
                        TexKind::Sampled(s) | TexKind::Multisampled(s) => s,
                        _ => Scalar::F32,
                    };
                    self.t.vec(4, s)
                }
                TR::V4C => {
                    let s = match kind {
                        TexKind::Storage(f, _) => f.channel(),
                        _ => Scalar::F32,
                    };
                    self.t.vec(4, s)
                }
                TR::U32 => U32,
                TR::Dims => self.t.vec_or_scalar(if dim == TexDim::D1 { 1 } else if matches!(dim, TexDim::D3) { 3 } else { 2 }, Scalar::U32),
                TR::Void => builtins::NONE,
            };
            return Ok(self.mk(ir::ExprKind::Builtin(b, ids), rt, Phase::Runtime, sp));
        }
        Err(self.err(sp, format!("keine passende Überladung von `{}({})`", id.name, self.arg_list(&vals))))
    }

    fn atomic_call(&mut self, b: Builtin, id: &ast::Ident, args: &[Expr], sp: Span) -> R<TE> {
        use Builtin::*;
        let vals = self.args_of(args)?;
        let want = match b {
            AtomicLoad => 1,
            AtomicCompareExchangeWeak => 3,
            _ => 2,
        };
        if vals.len() != want {
            return Err(self.err(sp, format!("`{}` nimmt {} Argument{}", id.name, want, if want == 1 { "" } else { "e" })));
        }
        let inner = match self.t.get(vals[0].ty) {
            Type::Pointer { space: space @ (AddressSpace::Storage | AddressSpace::Workgroup), store, access: Access::ReadWrite } => match self.t.get(store) {
                Type::Atomic(i) => Some((space, i)),
                _ => None,
            },
            _ => None,
        };
        let Some((space, inner)) = inner else {
            return Err(self.err(args[0].span, format!("`{}` braucht einen Zeiger auf ein atomic in storage (read_write) oder workgroup, nicht `{}`", id.name, self.tn(vals[0].ty))));
        };
        let v2u = self.t.vec(2, Scalar::U32);
        if matches!(b, AtomicStoreMin | AtomicStoreMax) != (inner == v2u) {
            return Err(self.err(args[0].span, format!("`{}` passt nicht zu `atomic<{}>`", id.name, self.tn(inner))));
        }
        if inner == v2u && space != AddressSpace::Storage {
            return Err(self.err(args[0].span, String::from("atomic<vec2<u32>> gibt es nur im storage-Adressraum")));
        }
        let mut ids = alloc::vec![vals[0].id];
        let ptr_root = self.root(vals[0].id);
        for v in vals.into_iter().skip(1) {
            let cv = self.convert(v, inner)?;
            ids.push(self.node(&cv));
        }
        if let (Some(r), Some(f)) = (ptr_root, self.f.as_mut()) {
            if b != AtomicStore {
                f.reads.insert(r);
            }
            if b != AtomicLoad {
                f.writes.insert(r);
            }
        }
        let rt = match b {
            AtomicStore | AtomicStoreMin | AtomicStoreMax => builtins::NONE,
            AtomicCompareExchangeWeak => {
                let s = self.t.scalar_of(inner).unwrap_or(Scalar::U32);
                self.t.builtin_struct(BuiltinStruct::AtomicCompareExchange(s))
            }
            _ => inner,
        };
        Ok(self.mk(ir::ExprKind::Builtin(b, ids), rt, Phase::Runtime, sp))
    }
}

fn tex_matches(tp: TP, dim: TexDim, kind: TexKind) -> bool {
    match tp {
        TP::Samp(d, s) => d == dim && matches!(kind, TexKind::Sampled(x) if s.is_none_or(|s| s == x)),
        TP::Ms => matches!(kind, TexKind::Multisampled(_)),
        TP::Depth(d) => d == dim && kind == TexKind::Depth,
        TP::DepthMs => kind == TexKind::DepthMultisampled,
        TP::External => kind == TexKind::External,
        TP::Storage(d, read) => d == dim && matches!(kind, TexKind::Storage(_, a) if if read { a.readable() } else { a.writable() }),
        TP::Query(mask) => builtins::tex_any(mask, dim, kind),
    }
}
