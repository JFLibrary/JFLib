// check_stmt -- function bodies: every statement of spec section 9, scopes and shadowing,
// the behavior analysis of 9.7 (missing return, loops that never end), break / continue /
// break-if placement, `continue` bypassing a declaration its continuing block uses,
// @must_use, assignments (simple, swizzle, compound, ++ / --, phony) -- producing ir::Stmt
// with for and while desugared to Loop
//
//   fn_body(&body, name) -> R<(returns, Vec<ir::Stmt>)>

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::{self, AssignOp, BinOp, ExprKind, Span, StmtKind, VarKind};
use crate::builtins;
use crate::check::*;
use crate::consteval;
use crate::ir::{self, *};

pub const NEXT: u8 = 1;
pub const RETURN: u8 = 2;
pub const BREAK: u8 = 4;
pub const CONTINUE: u8 = 8;

/// a statement's behavior and IR
type Out = (u8, Option<ir::Stmt>);

impl<'a> Checker<'a> {
    pub fn fn_body(&mut self, body: &ast::Block, name: Span) -> R<(bool, Vec<ir::Stmt>)> {
        self.only_diag(&body.attrs, body.span);
        let (beh, stmts) = self.stmts(&body.stmts);
        let had_error = self.f.as_ref().is_none_or(|f| f.had_error);
        if had_error {
            return Ok((true, stmts));
        }
        let ret = self.f.as_ref().and_then(|f| f.ret);
        let end = Span::new(body.span.end.saturating_sub(1), body.span.end);
        if ret.is_some() && beh & NEXT != 0 {
            let fname = self.f.as_ref().map(|f| f.info.name.clone()).unwrap_or_default();
            return Err(self.err_note(end, format!("die Funktion `{}` muss auf jedem Weg mit `return` einen Wert liefern", fname), name, String::from("die Funktion")));
        }
        Ok((beh & (NEXT | RETURN) != 0, stmts))
    }

    fn push_scope(&mut self) {
        if let Some(f) = self.f.as_mut() {
            f.scopes.push(Vec::new());
        }
    }

    fn pop_scope(&mut self) {
        if let Some(f) = self.f.as_mut() {
            f.scopes.pop();
        }
    }

    fn block(&mut self, b: &ast::Block) -> (u8, Vec<ir::Stmt>) {
        self.only_diag(&b.attrs, b.span);
        self.push_scope();
        let r = self.stmts(&b.stmts);
        self.pop_scope();
        r
    }

    fn stmts(&mut self, v: &[ast::Stmt]) -> (u8, Vec<ir::Stmt>) {
        let mut acc = NEXT;
        let mut out = Vec::with_capacity(v.len());
        let mut warned = false;
        for s in v {
            if self.errors >= MAX_ERRORS {
                break;
            }
            // code after return / break / continue is checked, but never runs
            if acc & NEXT == 0 && !warned && !matches!(s.kind, StmtKind::Empty) {
                warned = true;
                self.warn(Span::new(s.span.start, s.span.start + 1), String::from("unerreichbarer Code"));
            }
            let (b, st) = match self.stmt(s) {
                Ok(x) => x,
                Err(_) => (NEXT, None),
            };
            if acc & NEXT != 0 {
                acc = (acc & !NEXT) | b;
            }
            if let Some(st) = st {
                out.push(st);
            }
        }
        (acc, out)
    }

    /// declare a local name in the innermost scope (same scope twice: error)
    fn declare(&mut self, name: &ast::Ident, r: LocalRef) -> R<()> {
        let dup = self.f.as_ref().and_then(|f| f.scopes.last()).and_then(|s| s.iter().find(|e| e.0 == name.name).map(|e| e.3));
        if let Some(psp) = dup {
            return Err(self.err_note(name.span, format!("`{}` ist in diesem Block schon deklariert", name.name), psp, String::from("hier zuerst")));
        }
        let Some(f) = self.f.as_mut() else { return Err(E) };
        let uid = f.next_uid;
        f.next_uid += 1;
        let depth = f.scopes.len();
        if let Some(s) = f.scopes.last_mut() {
            s.push((name.name.clone(), r, uid, name.span));
        }
        // a declaration at the top level of a loop body (for the continue check)
        if let Some(c) = f.ctl.last_mut() {
            if c.kind == CtlKind::Loop && c.depth == depth {
                c.decls.push((uid, name.span, name.name.clone()));
            }
        }
        Ok(())
    }

    fn declared_here(&self, name: &str) -> bool {
        self.f.as_ref().and_then(|f| f.scopes.last()).is_some_and(|s| s.iter().any(|e| e.0 == name))
    }

    fn new_local(&mut self, name: &ast::Ident, ty: TypeId, kind: LocalKind, init: Option<ExprId>, sp: Span) -> u32 {
        let Some(f) = self.f.as_mut() else { return 0 };
        f.locals.push(ir::Local { name: name.name.clone(), ty, kind, span: sp });
        f.local_init.push(init);
        (f.locals.len() - 1) as u32
    }

    fn stmt(&mut self, s: &ast::Stmt) -> R<Out> {
        let sp = s.span;
        match &s.kind {
            StmtKind::Empty => Ok((NEXT, None)),
            StmtKind::Block(b) => {
                let (beh, v) = self.block(b);
                Ok((beh, Some(ir::Stmt::Block(v))))
            }
            StmtKind::Return(e) => {
                if self.f.as_ref().is_some_and(|f| f.in_continuing > 0) {
                    return Err(self.err(sp, String::from("`return` ist in `continuing` nicht erlaubt")));
                }
                let ret = self.f.as_ref().and_then(|f| f.ret);
                let v = match (e, ret) {
                    (Some(e), Some(t)) => {
                        let te = self.value(e)?;
                        let te = self.convert(te, t)?;
                        Some(self.node(&te))
                    }
                    (None, None) => None,
                    (Some(e), None) => return Err(self.err(e.span, String::from("die Funktion hat keinen Rückgabetyp; `return` ohne Wert"))),
                    (None, Some(t)) => return Err(self.err(sp, format!("`return` braucht einen Wert vom Typ `{}`", self.tn(t)))),
                };
                Ok((RETURN, Some(ir::Stmt::Return(v))))
            }
            StmtKind::If { attrs, cond, then, else_ifs, else_ } => {
                self.only_diag(attrs, sp);
                let c = self.condition(cond, "if")?;
                let (mut beh, t) = self.block(then);
                // else-if chain: built from the back
                let mut parts = Vec::with_capacity(else_ifs.len());
                for ei in else_ifs {
                    let c2 = self.condition(&ei.cond, "else if")?;
                    let (b2, t2) = self.block(&ei.body);
                    beh |= b2;
                    parts.push((c2, t2));
                }
                let mut tail = match else_ {
                    Some(b) => {
                        let (b3, e) = self.block(b);
                        beh |= b3;
                        e
                    }
                    None => {
                        beh |= NEXT;
                        Vec::new()
                    }
                };
                while let Some((c2, t2)) = parts.pop() {
                    tail = alloc::vec![ir::Stmt::If { cond: c2, then: t2, else_: tail }];
                }
                Ok((beh, Some(ir::Stmt::If { cond: c, then: t, else_: tail })))
            }
            StmtKind::Switch { attrs, selector, body_attrs, clauses, body_span } => {
                self.only_diag(attrs, sp);
                self.only_diag(body_attrs, *body_span);
                self.switch(selector, clauses, sp)
            }
            StmtKind::Loop { attrs, body, continuing } => {
                self.only_diag(attrs, sp);
                self.only_diag(&body.attrs, body.span);
                self.loop_(&body.stmts, continuing.as_ref(), sp)
            }
            StmtKind::For { attrs, init, cond, update, body } => {
                self.only_diag(attrs, sp);
                self.for_(init.as_deref(), cond.as_ref(), update.as_deref(), body, sp)
            }
            StmtKind::While { attrs, cond, body } => {
                self.only_diag(attrs, sp);
                self.for_(None, Some(cond), None, body, sp)
            }
            StmtKind::Break => {
                let kind = self.f.as_ref().and_then(|f| f.ctl.last().map(|c| c.kind));
                match kind {
                    Some(CtlKind::Loop) | Some(CtlKind::Switch) => Ok((BREAK, Some(ir::Stmt::Break))),
                    Some(CtlKind::Continuing) => Err(self.err(sp, String::from("`break` darf `continuing` nicht verlassen; benutze `break if` als letzte Anweisung"))),
                    None => Err(self.err(sp, String::from("`break` steht außerhalb einer Schleife und eines `switch`"))),
                }
            }
            StmtKind::Continue => {
                let Some(f) = self.f.as_mut() else { return Err(E) };
                let target = f.ctl.iter_mut().rev().find(|c| c.kind != CtlKind::Switch);
                match target {
                    Some(c) if c.kind == CtlKind::Loop => {
                        let n = c.decls.len();
                        c.continues.push((n, sp));
                        Ok((CONTINUE, Some(ir::Stmt::Continue)))
                    }
                    Some(_) => Err(self.err(sp, String::from("`continue` darf nicht zum Anfang des eigenen `continuing`-Blocks springen"))),
                    None => Err(self.err(sp, String::from("`continue` steht außerhalb einer Schleife"))),
                }
            }
            StmtKind::Discard => {
                if let Some(f) = self.f.as_mut() {
                    f.info.uses_discard = true;
                }
                Ok((NEXT, Some(ir::Stmt::Discard)))
            }
            StmtKind::Call(e) => self.call_stmt(e),
            StmtKind::Decl(v) => {
                let r = self.local_decl(v);
                if r.is_err() && v.kind != VarKind::Override && !self.declared_here(&v.name.name) {
                    let _ = self.declare(&v.name, LocalRef::Error);
                }
                r
            }
            StmtKind::Assign { lhs, op, rhs } => self.assign(lhs, *op, rhs, sp),
            StmtKind::Phony(e) => {
                let te = self.expr(e)?;
                let te = self.rvalue(te)?;
                let ok = self.t.is_constructible(te.ty) || self.t.is_ptr(te.ty) || self.t.is_handle(te.ty);
                if !ok {
                    let hint = if self.t.contains_runtime_array(te.ty) || self.t.contains_atomic(te.ty) { " (nimm einen Zeiger: `_ = &x;`)" } else { "" };
                    return Err(self.err(e.span, format!("`_ =` braucht einen konstruierbaren Wert, einen Zeiger, eine Textur oder einen Sampler, nicht `{}`{}", self.tn(te.ty), hint)));
                }
                let te = self.concretize(te)?;
                let id = self.node(&te);
                Ok((NEXT, Some(ir::Stmt::Phony(id))))
            }
            StmtKind::Incr(e) | StmtKind::Decr(e) => {
                let r = self.expr(e)?;
                let Some((_, store, access)) = self.t.store_of(r.ty).filter(|_| self.t.is_ref(r.ty)) else {
                    return Err(self.err(e.span, String::from("`++` / `--` brauchen eine Variable oder einen Speicherbezug")));
                };
                if access != Access::ReadWrite {
                    return Err(self.err(e.span, String::from("`++` / `--` brauchen Lese- und Schreibzugriff")));
                }
                if store != I32 && store != U32 {
                    return Err(self.err(e.span, format!("`++` / `--` gibt es nur für i32 und u32, nicht für `{}`", self.tn(store))));
                }
                self.note_rw(r.id);
                let one = if store == I32 { Value::I32(1) } else { Value::U32(1) };
                let c = self.push(ir::ExprKind::Const(one), store, e.span);
                let op = if matches!(s.kind, StmtKind::Incr(_)) { BinOp::Add } else { BinOp::Sub };
                Ok((NEXT, Some(ir::Stmt::Compound { lhs: r.id, op, rhs: c })))
            }
            StmtKind::ConstAssert(e) => {
                self.const_assert(e)?;
                Ok((NEXT, None))
            }
        }
    }

    fn note_rw(&mut self, id: ExprId) {
        if let Some(r) = self.root(id) {
            if let Some(f) = self.f.as_mut() {
                f.reads.insert(r);
                f.writes.insert(r);
            }
        }
    }

    fn condition(&mut self, e: &ast::Expr, what: &str) -> R<ExprId> {
        let te = self.value(e)?;
        if te.ty != BOOL {
            return Err(self.err(e.span, format!("die Bedingung von `{}` muss bool sein, nicht `{}`", what, self.tn(te.ty))));
        }
        Ok(self.node(&te))
    }

    fn switch(&mut self, selector: &ast::Expr, clauses: &[ast::Clause], sp: Span) -> R<Out> {
        let sel = self.value(selector)?;
        if !matches!(sel.ty, I32 | U32 | AINT) {
            return Err(self.err(selector.span, format!("der `switch`-Ausdruck muss i32 oder u32 sein, nicht `{}`", self.tn(sel.ty))));
        }
        // the case values (const-expressions)
        let mut cases: Vec<(TE, usize)> = Vec::new();
        let mut defaults: Vec<Span> = Vec::new();
        let mut ok = true;
        for (k, c) in clauses.iter().enumerate() {
            let mut seen_default = false;
            for s in &c.sels {
                match s {
                    ast::CaseSel::Default(dsp) => {
                        if seen_default {
                            let _ = self.err(*dsp, String::from("`default` steht zweimal in derselben Liste"));
                            ok = false;
                        }
                        seen_default = true;
                        defaults.push(*dsp);
                    }
                    ast::CaseSel::Expr(e) => match self.value(e) {
                        Ok(te) => {
                            if te.stage != Phase::Const {
                                let _ = self.err(e.span, String::from("ein case-Wert muss ein const-Ausdruck sein"));
                                ok = false;
                            } else {
                                cases.push((te, k));
                            }
                        }
                        Err(_) => ok = false,
                    },
                }
            }
        }
        if defaults.is_empty() {
            let _ = self.err(sp, String::from("`switch` braucht genau einen `default`-Zweig"));
            ok = false;
        } else if defaults.len() > 1 {
            let _ = self.err_note(defaults[1], String::from("`switch` hat mehr als einen `default`-Zweig"), defaults[0], String::from("der erste"));
            ok = false;
        }
        // the common type: a concrete one if any, otherwise i32
        let concrete = core::iter::once(&sel).chain(cases.iter().map(|c| &c.0)).find(|t| !self.t.is_abstract(t.ty)).map(|t| t.ty);
        let ty = concrete.unwrap_or(I32);
        if ty != I32 && ty != U32 {
            return Err(self.err(selector.span, format!("der `switch`-Ausdruck muss i32 oder u32 sein, nicht `{}`", self.tn(ty))));
        }
        let sel = self.convert(sel, ty)?;
        let mut values: Vec<(i64, Span, usize)> = Vec::new();
        for (te, k) in cases {
            let tsp = te.span;
            let Ok(cv) = self.convert(te, ty) else {
                ok = false;
                continue;
            };
            let v = cv.val.as_ref().and_then(consteval::as_int).unwrap_or(0) as i64;
            if let Some(p) = values.iter().find(|p| p.0 == v) {
                let psp = p.1;
                let _ = self.err_note(tsp, format!("der case-Wert {} kommt zweimal vor", v), psp, String::from("hier zuerst"));
                ok = false;
            }
            values.push((v, tsp, k));
        }
        let sid = self.node(&sel);
        let mut beh = 0u8;
        let mut out = Vec::with_capacity(clauses.len());
        if let Some(f) = self.f.as_mut() {
            f.ctl.push(Ctl { kind: CtlKind::Switch, depth: usize::MAX, decls: Vec::new(), continues: Vec::new() });
        }
        for (k, c) in clauses.iter().enumerate() {
            let (b, body) = self.block(&c.body);
            beh |= b;
            let selectors: Vec<i64> = values.iter().filter(|v| v.2 == k).map(|v| v.0).collect();
            let default = c.sels.iter().any(|s| matches!(s, ast::CaseSel::Default(_)));
            out.push(ir::Case { selectors, default, body });
        }
        if let Some(f) = self.f.as_mut() {
            f.ctl.pop();
        }
        if !ok {
            return Err(E);
        }
        if beh & BREAK != 0 {
            beh = (beh | NEXT) & !BREAK;
        }
        Ok((beh, Some(ir::Stmt::Switch { selector: sid, cases: out })))
    }

    fn loop_combine(&mut self, b1: u8, b2: u8, sp: Span) -> R<u8> {
        let r = if b1 == RETURN {
            RETURN
        } else if (b1 | b2) & BREAK == 0 {
            (b1 | b2) & !(CONTINUE | NEXT)
        } else {
            (b1 | b2 | NEXT) & !(BREAK | CONTINUE)
        };
        if r == 0 {
            return Err(self.err(Span::new(sp.start, sp.start + 4), String::from("die Schleife endet nie (kein `break`, kein `return`)")));
        }
        Ok(r)
    }

    fn loop_(&mut self, body: &[ast::Stmt], cont: Option<&ast::Continuing>, sp: Span) -> R<Out> {
        if let Some(f) = self.f.as_mut() {
            let depth = f.scopes.len() + 1;
            f.ctl.push(Ctl { kind: CtlKind::Loop, depth, decls: Vec::new(), continues: Vec::new() });
        }
        self.push_scope();
        let (b1, stmts) = self.stmts(body);
        let mut b2 = NEXT;
        let mut cstmts = Vec::new();
        let mut break_if = None;
        let mut uses = BTreeMap::new();
        let mut err = false;
        if let Some(c) = cont {
            if let Some(f) = self.f.as_mut() {
                f.in_continuing += 1;
                f.ctl.push(Ctl { kind: CtlKind::Continuing, depth: usize::MAX, decls: Vec::new(), continues: Vec::new() });
                f.cont_uses.push(BTreeMap::new());
            }
            self.only_diag(&c.body.attrs, c.body.span);
            self.push_scope();
            let (b, v) = self.stmts(&c.body.stmts);
            b2 = b;
            cstmts = v;
            if let Some(e) = &c.break_if {
                match self.condition(e, "break if") {
                    Ok(id) => break_if = Some(id),
                    Err(_) => err = true,
                }
                if b2 & NEXT != 0 {
                    b2 = (b2 & !NEXT) | BREAK | NEXT;
                }
            }
            self.pop_scope();
            if let Some(f) = self.f.as_mut() {
                f.in_continuing -= 1;
                f.ctl.pop();
                uses = f.cont_uses.pop().unwrap_or_default();
            }
        }
        self.pop_scope();
        let ctl = self.f.as_mut().and_then(|f| f.ctl.pop());
        // a continue must not skip a declaration the continuing block uses
        if let Some(ctl) = ctl {
            for &(n, csp) in &ctl.continues {
                for (uid, dsp, name) in ctl.decls.iter().skip(n) {
                    if let Some(&usp) = uses.get(uid) {
                        let _ = self.err_note(csp, format!("`continue` springt über die Deklaration von `{}`, die `continuing` benutzt", name), *dsp, String::from("hier deklariert"));
                        if let Some(d) = self.diags.last_mut() {
                            let n = self.lm.note(self.src, usp.start, usp.end, String::from("hier in `continuing` benutzt"));
                            d.notes.push(n);
                        }
                        err = true;
                        break;
                    }
                }
            }
        }
        if err {
            return Err(E);
        }
        let beh = self.loop_combine(b1, b2 & !(CONTINUE | RETURN), sp)?;
        Ok((beh, Some(ir::Stmt::Loop { body: stmts, continuing: cstmts, break_if })))
    }

    /// for and while (while: no init, no update)
    fn for_(&mut self, init: Option<&ast::Stmt>, cond: Option<&ast::Expr>, update: Option<&ast::Stmt>, body: &ast::Block, sp: Span) -> R<Out> {
        self.push_scope();
        let r = (|| {
            let mut pre = Vec::new();
            if let Some(i) = init {
                let (_, st) = self.stmt(i)?;
                pre.extend(st);
            }
            let c = match cond {
                Some(c) => Some(self.condition(c, if init.is_some() || update.is_some() { "for" } else { "while" })?),
                None => None,
            };
            // the update is checked before the body: it cannot see the body's declarations
            let mut cont = Vec::new();
            if let Some(u) = update {
                if let StmtKind::Decl(_) = u.kind {
                    return Err(self.err(u.span, String::from("im dritten Teil von `for` ist keine Deklaration erlaubt")));
                }
                let (_, st) = self.stmt(u)?;
                cont.extend(st);
            }
            if let Some(f) = self.f.as_mut() {
                // the update cannot see the body's declarations: no continue check needed
                f.ctl.push(Ctl { kind: CtlKind::Loop, depth: usize::MAX, decls: Vec::new(), continues: Vec::new() });
            }
            self.only_diag(&body.attrs, body.span);
            let (b, stmts) = self.stmts(&body.stmts);
            if let Some(f) = self.f.as_mut() {
                f.ctl.pop();
            }
            let mut lbody = Vec::with_capacity(stmts.len() + 1);
            let mut b1 = b;
            if let Some(c) = c {
                lbody.push(ir::Stmt::If { cond: c, then: Vec::new(), else_: alloc::vec![ir::Stmt::Break] });
                b1 = BREAK | b;
            }
            lbody.extend(stmts);
            let beh = self.loop_combine(b1, NEXT, sp)?;
            let lp = ir::Stmt::Loop { body: lbody, continuing: cont, break_if: None };
            let st = if pre.is_empty() {
                lp
            } else {
                pre.push(lp);
                ir::Stmt::Block(pre)
            };
            Ok((beh, Some(st)))
        })();
        self.pop_scope();
        r
    }

    fn call_stmt(&mut self, e: &ast::Expr) -> R<Out> {
        let ExprKind::Call(id, targs, args) = &e.kind else {
            return Err(self.err(e.span, String::from("erwartet einen Funktionsaufruf")));
        };
        // what the callee is: must_use functions and constructors may not stand alone
        let local = self.lookup_local(&id.name).is_some();
        if !local {
            match self.lookup_decl(&id.name) {
                Ok(Some((_, DeclInfo::Fn(fi)))) => {
                    if self.fns[fi as usize].must_use {
                        let fsp = self.fns[fi as usize].span;
                        let _ = self.call(id, targs.as_deref(), args, e.span);
                        return Err(self.err_note(e.span, format!("das Ergebnis von `{}` muss benutzt werden (@must_use)", id.name), fsp, String::from("hier deklariert")));
                    }
                }
                Ok(Some((_, DeclInfo::Alias(_) | DeclInfo::Struct(_)))) => {
                    return Err(self.err(e.span, String::from("ein Wert-Konstruktor wird ausgewertet, aber nicht benutzt")));
                }
                Ok(None) => {
                    if let Some((_, flags)) = builtins::lookup(&id.name) {
                        if flags & builtins::MUST_USE != 0 {
                            let _ = self.call(id, targs.as_deref(), args, e.span);
                            return Err(self.err(e.span, format!("das Ergebnis von `{}` muss benutzt werden", id.name)));
                        }
                    } else if id.name == "bitcast" || predeclared_kind(&id.name).is_some_and(|k| k == "ein Typ" || k == "ein Typ-Generator") {
                        return Err(self.err(e.span, String::from("ein Wert-Konstruktor wird ausgewertet, aber nicht benutzt")));
                    }
                }
                _ => {}
            }
        }
        let te = self.call(id, targs.as_deref(), args, e.span)?;
        let id = self.node(&te);
        Ok((NEXT, Some(ir::Stmt::Call(id))))
    }

    fn local_decl(&mut self, v: &ast::VarDecl) -> R<Out> {
        if let Some(a) = v.attrs.first() {
            return Err(self.err(a.span, String::from("an lokalen Deklarationen sind keine Attribute erlaubt")));
        }
        match v.kind {
            VarKind::Const => {
                let (ty, val) = self.const_value(v)?;
                self.declare(&v.name, LocalRef::Const(ty, val))?;
                Ok((NEXT, None))
            }
            VarKind::Let => {
                let ie = v.init.as_ref().ok_or(E)?;
                let te = self.value(ie)?;
                let te = match &v.ty {
                    Some(t) => {
                        let t = self.ty(t)?;
                        self.convert(te, t)?
                    }
                    None => self.concretize(te)?,
                };
                let ty = te.ty;
                let ok = self.t.is_constructible(ty) || self.t.is_ptr(ty) || (self.t.is_handle(ty) && self.has(crate::feature::TEXTURE_AND_SAMPLER_LET));
                if !ok {
                    return Err(self.err(ie.span, format!("`let` kann keinen Wert vom Typ `{}` halten", self.tn(ty))));
                }
                let id = self.node(&te);
                let l = self.new_local(&v.name, ty, LocalKind::Let, Some(id), v.span);
                self.declare(&v.name, LocalRef::Let(l))?;
                Ok((NEXT, Some(ir::Stmt::Let { local: l, init: id })))
            }
            VarKind::Var => {
                if let Some(tl) = &v.templ {
                    let Some(t0) = tl.first() else { return Err(E) };
                    let s = self.address_space(t0)?;
                    if s != AddressSpace::Function {
                        return Err(self.err(t0.span, format!("in Funktionen gibt es nur `var` und `var<function>`, nicht `{}`", s.name())));
                    }
                    if let Some(a) = tl.get(1) {
                        return Err(self.err(a.span, String::from("im Adressraum function darf kein Zugriffsmodus stehen")));
                    }
                }
                let init = match &v.init {
                    Some(ie) => Some(self.value(ie)?),
                    None => None,
                };
                let ty = match (&v.ty, &init) {
                    (Some(t), _) => self.ty(t)?,
                    (None, Some(te)) => self.t.concretize(te.ty),
                    (None, None) => return Err(self.err(v.name.span, format!("`var {}` braucht einen Typ oder einen Initialisierer", v.name.name))),
                };
                let tsp = v.ty.as_ref().map(|t| t.span).unwrap_or(v.name.span);
                self.store_type_ok(AddressSpace::Function, Access::ReadWrite, ty, tsp)?;
                let init = match init {
                    Some(te) => {
                        let te = self.convert(te, ty)?;
                        Some(self.node(&te))
                    }
                    None => None,
                };
                let l = self.new_local(&v.name, ty, LocalKind::Var, None, v.span);
                self.declare(&v.name, LocalRef::Var(l))?;
                Ok((NEXT, Some(ir::Stmt::Var { local: l, init })))
            }
            VarKind::Override => Err(self.err(v.span, String::from("`override` ist nur auf Modulebene erlaubt"))),
        }
    }

    fn assign(&mut self, lhs: &ast::Expr, op: AssignOp, rhs: &ast::Expr, sp: Span) -> R<Out> {
        let l = self.expr(lhs)?;
        let (store, writable, readable, view) = match self.t.get(l.ty) {
            Type::Reference { store, access, space } => (store, access.writable() && space != AddressSpace::Handle, access.readable(), None),
            Type::SwizzleView { elem, k, .. } => (self.t.vec(k, elem), true, true, Some(())),
            _ => return Err(self.err(lhs.span, String::from("die linke Seite einer Zuweisung muss eine Variable oder ein Speicherbezug sein"))),
        };
        if !writable {
            return Err(self.err(lhs.span, String::from("in diesen Speicher darf nicht geschrieben werden (nur lesbar)")));
        }
        if view.is_some() {
            // the components of a swizzle assignment must be distinct
            if let Some(ir::ExprKind::Swizzle(_, cs, n)) = self.arena().get(l.id as usize).map(|e| &e.kind) {
                let cs = &cs[..*n as usize];
                if (0..cs.len()).any(|i| cs[..i].contains(&cs[i])) {
                    return Err(self.err(lhs.span, String::from("ein Swizzle auf der linken Seite darf keine Komponente zweimal nennen")));
                }
            }
        }
        if !self.t.is_constructible(store) {
            return Err(self.err(lhs.span, format!("`{}` lässt sich nicht zuweisen (nicht konstruierbar)", self.tn(store))));
        }
        let r = self.value(rhs)?;
        match op.binary() {
            None => {
                let r = self.convert(r, store)?;
                if let Some(root) = self.root(l.id) {
                    if let Some(f) = self.f.as_mut() {
                        f.writes.insert(root);
                    }
                }
                let rid = self.node(&r);
                Ok((NEXT, Some(ir::Stmt::Assign { lhs: l.id, rhs: rid })))
            }
            Some(bop) => {
                if !readable {
                    return Err(self.err(lhs.span, String::from("eine zusammengesetzte Zuweisung braucht Lese- und Schreibzugriff")));
                }
                // type-check `lhs op rhs` with a stand-in for the loaded left side
                let stand_in = TE { id: NONE, ty: store, stage: Phase::Runtime, val: None, span: lhs.span };
                let checked = self.compound_types(bop, stand_in, r, sp)?;
                if checked.0 != store {
                    return Err(self.err(sp, format!("`{}` ergibt `{}`, die linke Seite ist `{}`", op.text(), self.tn(checked.0), self.tn(store))));
                }
                self.note_rw(l.id);
                Ok((NEXT, Some(ir::Stmt::Compound { lhs: l.id, op: bop, rhs: checked.1 })))
            }
        }
    }

    /// the result type of `lhs op rhs` and the converted right operand's node
    fn compound_types(&mut self, op: BinOp, l: TE, r: TE, sp: Span) -> R<(TypeId, ExprId)> {
        let cands = self.op_cands(op, l.ty, r.ty);
        let args = [l, r];
        let Ok(sig) = self.resolve(&args, &cands) else {
            let (a, b) = (self.tn(args[0].ty), self.tn(args[1].ty));
            return Err(self.err(sp, format!("`{}` ist für `{}` und `{}` nicht definiert", op.text(), a, b)));
        };
        let [_, r] = args;
        let r = self.convert(r, sig.p[1])?;
        if r.stage == Phase::Const && self.no_eval == 0 {
            if let Some(v) = &r.val {
                let cs = if self.t.is_scalar(r.ty) { alloc::vec![v.clone()] } else { consteval::comps(&self.t, v, r.ty) };
                let s = self.t.scalar_of(sig.p[0]);
                if matches!(op, BinOp::Div | BinOp::Rem) && matches!(s, Some(Scalar::I32 | Scalar::U32)) && cs.iter().any(|c| consteval::as_int(c) == Some(0)) {
                    return Err(self.err(r.span, String::from("ganzzahlige Division durch null")));
                }
                if matches!(op, BinOp::Shl | BinOp::Shr) && cs.iter().any(|c| consteval::as_int(c).is_some_and(|x| x >= 32)) {
                    return Err(self.err(r.span, String::from("die Schiebeweite muss kleiner als 32 sein")));
                }
            }
        }
        let id = self.node(&r);
        Ok((sig.ret, id))
    }
}
