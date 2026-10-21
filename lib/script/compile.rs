// compile -- from the syntax tree to bytecode (code.rs)
//
//   generate(&program, src) -> Code   (lib.rs's compile() parses and resolves first, so the
//                                      tree has no errors here)
//
// Two passes.  analyze() resolves every name with the scope rules of ast.rs, in the same walk
// as check::resolve: a use of a name (keyed by the byte where it starts) refers to a local
// declaration (keyed by the byte of the declared name), a global (top-level let and fn) or the
// host; a local that a nested function uses is marked captured.
//
// The generator gives every declaration of a block a slot when the block starts (Reserve):
// captured ones get a fresh cell right there, so closures can capture a let that runs later in
// the block; the block's fn declarations are created next (hoisting: they may call each
// other), top-level ones before the first statement of the script.  The block's slots are
// popped at its end; break and continue pop down to their loop.  for loops keep their state in
// hidden slots below the loop variables; a captured loop variable gets a new cell each round,
// so closures keep the value of their round.  try adds an entry to the function's exception
// table (protected range, handler, stack height); entries of inner trys come first.  Every
// instruction gets the line and column of its source: operators the position of the operator,
// fields and methods the position of the name, calls the start of the callee.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::ast::*;
use crate::code::{Code, Const, Handler, HostName, Op, Proto, Up};

// ---- analysis

#[derive(Clone, Copy, PartialEq, Debug)]
enum Res {
    /// a local declaration (byte of its name)
    Local(u32),
    Global(u32),
    Host,
}

#[derive(Default)]
struct Analysis {
    /// use (byte of the name) -> what it refers to
    uses: BTreeMap<u32, Res>,
    /// local declarations used by nested functions
    captured: BTreeSet<u32>,
    globals: Vec<String>,
    /// declaration of a top-level let / fn -> its global
    gdecl: BTreeMap<u32, u32>,
}

struct Analyzer<'a> {
    a: Analysis,
    top: BTreeMap<&'a str, u32>,
    /// the scopes inside the top level, innermost last: names with their declaration, and the
    /// function depth the scope belongs to
    scopes: Vec<(Vec<(&'a str, u32)>, u32)>,
    depth: u32,
}

fn analyze(p: &Program) -> Analysis {
    let mut z = Analyzer { a: Analysis::default(), top: BTreeMap::new(), scopes: Vec::new(), depth: 0 };
    for s in &p.body {
        if let StmtKind::Let { name, .. } | StmtKind::Fn(FnDef { name: Some(name), .. }) = &s.kind {
            if !z.top.contains_key(name.text.as_str()) {
                let g = z.a.globals.len() as u32;
                z.a.globals.push(name.text.clone());
                z.top.insert(&name.text, g);
                z.a.gdecl.insert(name.span.start, g);
            }
        }
    }
    for s in &p.body {
        z.stmt(s);
    }
    z.a
}

impl<'a> Analyzer<'a> {
    fn declare(&mut self, n: &'a Name) {
        if let Some(s) = self.scopes.last_mut() {
            s.0.push((&n.text, n.span.start));
        }
    }

    fn use_name(&mut self, name: &str, key: u32) {
        for (names, d) in self.scopes.iter().rev() {
            if let Some(&(_, decl)) = names.iter().rev().find(|(n, _)| *n == name) {
                if *d < self.depth {
                    self.a.captured.insert(decl);
                }
                self.a.uses.insert(key, Res::Local(decl));
                return;
            }
        }
        let r = self.top.get(name).map_or(Res::Host, |&g| Res::Global(g));
        self.a.uses.insert(key, r);
    }

    fn block(&mut self, b: &'a Block) {
        self.scopes.push((Vec::new(), self.depth));
        for s in &b.stmts {
            if let StmtKind::Fn(FnDef { name: Some(n), .. }) = &s.kind {
                self.declare(n);
            }
        }
        for s in &b.stmts {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    fn func(&mut self, f: &'a FnDef) {
        self.depth += 1;
        self.scopes.push((Vec::new(), self.depth));
        for p in &f.params {
            self.declare(p);
        }
        self.block(&f.body);
        self.scopes.pop();
        self.depth -= 1;
    }

    fn stmt(&mut self, s: &'a Stmt) {
        match &s.kind {
            StmtKind::Let { name, init } => {
                if let Some(e) = init {
                    self.expr(e);
                }
                self.declare(name);
            }
            StmtKind::Fn(f) => self.func(f),
            StmtKind::If { arms, els } => {
                for (c, b) in arms {
                    self.expr(c);
                    self.block(b);
                }
                if let Some(b) = els {
                    self.block(b);
                }
            }
            StmtKind::While { cond, body } => {
                self.expr(cond);
                self.block(body);
            }
            StmtKind::For { a, b, src, body } => {
                match src {
                    ForSrc::Each(e) => self.expr(e),
                    ForSrc::Range(lo, hi) => {
                        self.expr(lo);
                        self.expr(hi);
                    }
                }
                self.scopes.push((Vec::new(), self.depth));
                self.declare(a);
                if let Some(b) = b {
                    self.declare(b);
                }
                self.block(body);
                self.scopes.pop();
            }
            StmtKind::Break | StmtKind::Continue => {}
            StmtKind::Return(v) => {
                if let Some(e) = v {
                    self.expr(e);
                }
            }
            StmtKind::Try { body, var, catch } => {
                self.block(body);
                self.scopes.push((Vec::new(), self.depth));
                if let Some(v) = var {
                    self.declare(v);
                }
                self.block(catch);
                self.scopes.pop();
            }
            StmtKind::Throw(e) | StmtKind::Expr(e) => self.expr(e),
            StmtKind::Assign { target, value, .. } => {
                match &target.kind {
                    ExprKind::Var(n) => self.use_name(n, target.span.start),
                    _ => self.expr(target),
                }
                self.expr(value);
            }
            StmtKind::Block(b) => self.block(b),
        }
    }

    fn expr(&mut self, e: &'a Expr) {
        match &e.kind {
            ExprKind::Nil | ExprKind::Bool(_) | ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) => {}
            ExprKind::Var(n) => self.use_name(n, e.span.start),
            ExprKind::Interp(items) | ExprKind::List(items) => {
                for x in items {
                    self.expr(x);
                }
            }
            ExprKind::Map(kv) => {
                for (k, v) in kv {
                    self.expr(k);
                    self.expr(v);
                }
            }
            ExprKind::Fn(f) => self.func(f),
            ExprKind::Unary(_, a) | ExprKind::Field(a, _) => self.expr(a),
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::Call(f, args) | ExprKind::Method(f, _, args) => {
                self.expr(f);
                for x in args {
                    self.expr(x);
                }
            }
        }
    }
}

// ---- code generation

struct Loop {
    /// where continue jumps to
    cont: u32,
    /// the slots in use inside the loop (break / continue pop down to them)
    height: u32,
    /// the jumps of break, patched at the loop's end
    breaks: Vec<usize>,
}

struct FnState {
    p: Proto,
    /// declaration -> slot, for the declarations of this function
    slots: BTreeMap<u32, u32>,
    /// declaration of each upvalue
    ups: Vec<u32>,
    /// slots of parameters and live block declarations (the height between statements)
    locals: u32,
    /// the height with temporaries, as the instructions so far leave it
    height: u32,
    loops: Vec<Loop>,
}

struct Gen<'a> {
    src: &'a str,
    /// byte offset of every line start
    lines: Vec<u32>,
    an: Analysis,
    consts: Vec<Const>,
    kstr: BTreeMap<String, u32>,
    kint: BTreeMap<i64, u32>,
    kflt: BTreeMap<u64, u32>,
    hosts: Vec<HostName>,
    hmap: BTreeMap<String, u32>,
    protos: Vec<Proto>,
    fns: Vec<FnState>,
}

/// see the module comment
pub(crate) fn generate(prog: &Program, src: &str) -> Code {
    let mut lines = vec![0u32];
    for (i, b) in src.bytes().enumerate() {
        if b == b'\n' {
            lines.push(i as u32 + 1);
        }
    }
    let mut g = Gen {
        src,
        lines,
        an: analyze(prog),
        consts: Vec::new(),
        kstr: BTreeMap::new(),
        kint: BTreeMap::new(),
        kflt: BTreeMap::new(),
        hosts: Vec::new(),
        hmap: BTreeMap::new(),
        protos: vec![Proto::default()],
        fns: Vec::new(),
    };
    g.main(prog);
    let globals = core::mem::take(&mut g.an.globals);
    Code { name: prog.name.clone(), protos: g.protos, consts: g.consts, globals, hosts: g.hosts }
}

fn new_fn(name: String, params: u32) -> FnState {
    let p = Proto { name, params, ..Proto::default() };
    FnState { p, slots: BTreeMap::new(), ups: Vec::new(), locals: 0, height: 0, loops: Vec::new() }
}

impl Gen<'_> {
    fn f(&mut self) -> &mut FnState {
        self.fns.last_mut().expect("a function")
    }

    /// line and column (1-based, columns count characters) of a byte offset
    fn pos(&self, at: u32) -> (u32, u32) {
        let line = match self.lines.binary_search(&at) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let start = self.lines[line] as usize;
        let col = self.src.get(start..(at as usize).min(self.src.len())).map_or(0, |s| s.chars().count());
        (line as u32 + 1, col as u32 + 1)
    }

    /// the first byte at or after `from` that is not white space or a comment (where an
    /// operator stands)
    fn skip(&self, from: u32) -> u32 {
        let b = self.src.as_bytes();
        let mut i = (from as usize).min(b.len());
        loop {
            while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r' | b'\n') {
                i += 1;
            }
            if b[i..].starts_with(b"//") {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            } else if b[i..].starts_with(b"/*") {
                i = match self.src[i + 2..].find("*/") {
                    Some(n) => i + 4 + n,
                    None => b.len(),
                };
            } else {
                return i as u32;
            }
        }
    }

    fn emit(&mut self, op: Op, at: u32) -> usize {
        let p = self.pos(at);
        let f = self.f();
        f.p.code.push(op);
        f.p.pos.push(p);
        f.height = (f.height as i32 + op.effect()).max(0) as u32;
        f.p.max_stack = f.p.max_stack.max(f.height);
        f.p.code.len() - 1
    }

    fn here(&mut self) -> u32 {
        self.f().p.code.len() as u32
    }

    /// point the jump at `at` to the next instruction
    fn patch(&mut self, at: usize) {
        let t = self.here();
        match &mut self.f().p.code[at] {
            Op::Jump(x) | Op::JumpIfFalse(x) | Op::And(x) | Op::Or(x) | Op::Coalesce(x) => *x = t,
            Op::IterNext(_, x) | Op::IterNext2(_, x) | Op::RangeNext(_, x) => *x = t,
            _ => {}
        }
    }

    // ---- constants and names

    fn kstr(&mut self, s: &str) -> u32 {
        if let Some(&k) = self.kstr.get(s) {
            return k;
        }
        let k = self.consts.len() as u32;
        self.consts.push(Const::Str(String::from(s)));
        self.kstr.insert(String::from(s), k);
        k
    }

    fn kint(&mut self, v: i64) -> u32 {
        if let Some(&k) = self.kint.get(&v) {
            return k;
        }
        let k = self.consts.len() as u32;
        self.consts.push(Const::Int(v));
        self.kint.insert(v, k);
        k
    }

    fn kflt(&mut self, v: f64) -> u32 {
        if let Some(&k) = self.kflt.get(&v.to_bits()) {
            return k;
        }
        let k = self.consts.len() as u32;
        self.consts.push(Const::Float(v));
        self.kflt.insert(v.to_bits(), k);
        k
    }

    fn host(&mut self, name: &str) -> u32 {
        if let Some(&h) = self.hmap.get(name) {
            return h;
        }
        let h = self.hosts.len() as u32;
        self.hosts.push(HostName { name: String::from(name), dotted: None });
        self.hmap.insert(String::from(name), h);
        h
    }

    /// the entry of base.field (http.serve, config.port)
    fn host_field(&mut self, base: &str, field: &str) -> u32 {
        let b = self.host(base);
        let full = format!("{}.{}", base, field);
        if let Some(&h) = self.hmap.get(&full) {
            return h;
        }
        let k = self.kstr(field);
        let h = self.hosts.len() as u32;
        self.hosts.push(HostName { name: full.clone(), dotted: Some((b, k)) });
        self.hmap.insert(full, h);
        h
    }

    fn is_host(&self, use_key: u32) -> bool {
        matches!(self.an.uses.get(&use_key), Some(Res::Host) | None)
    }

    // ---- variables

    fn captured(&self, decl: u32) -> bool {
        self.an.captured.contains(&decl)
    }

    /// the next slot for a declaration
    fn declare(&mut self, decl: u32) -> u32 {
        let f = self.f();
        let s = f.locals;
        f.locals += 1;
        f.slots.insert(decl, s);
        s
    }

    fn slot(&mut self, decl: u32) -> u32 {
        self.f().slots.get(&decl).copied().unwrap_or(0)
    }

    /// the upvalue of function fi for a declaration of an enclosing function
    fn upvalue(&mut self, fi: usize, decl: u32) -> u32 {
        if let Some(i) = self.fns[fi].ups.iter().position(|&d| d == decl) {
            return i as u32;
        }
        if fi == 0 {
            return 0;
        }
        let src = match self.fns[fi - 1].slots.get(&decl) {
            Some(&s) => Up::Local(s),
            None => Up::Up(self.upvalue(fi - 1, decl)),
        };
        let f = &mut self.fns[fi];
        f.ups.push(decl);
        f.p.ups.push(src);
        f.ups.len() as u32 - 1
    }

    /// store the top of the stack into a declaration (let, fn)
    fn store_decl(&mut self, decl: u32, at: u32) {
        if let Some(&g) = self.an.gdecl.get(&decl) {
            if self.fns.len() == 1 {
                self.emit(Op::SetGlobal(g), at);
                return;
            }
        }
        let s = self.slot(decl);
        let op = if self.captured(decl) { Op::SetCell(s) } else { Op::SetLocal(s) };
        self.emit(op, at);
    }

    fn load_var(&mut self, key: u32, name: &str, at: u32) {
        match self.an.uses.get(&key).copied() {
            Some(Res::Local(d)) => {
                let fi = self.fns.len() - 1;
                let op = match self.fns[fi].slots.get(&d) {
                    Some(&s) if self.captured(d) => Op::GetCell(s),
                    Some(&s) => Op::GetLocal(s),
                    None => Op::GetUp(self.upvalue(fi, d)),
                };
                self.emit(op, at);
            }
            Some(Res::Global(g)) => {
                self.emit(Op::GetGlobal(g), at);
            }
            _ => {
                let h = self.host(name);
                self.emit(Op::GetHost(h), at);
            }
        }
    }

    fn store_var(&mut self, key: u32, name: &str, at: u32) {
        match self.an.uses.get(&key).copied() {
            Some(Res::Local(d)) => {
                let fi = self.fns.len() - 1;
                let op = match self.fns[fi].slots.get(&d) {
                    Some(&s) if self.captured(d) => Op::SetCell(s),
                    Some(&s) => Op::SetLocal(s),
                    None => Op::SetUp(self.upvalue(fi, d)),
                };
                self.emit(op, at);
            }
            Some(Res::Global(g)) => {
                self.emit(Op::SetGlobal(g), at);
            }
            _ => {
                let h = self.host(name);
                self.emit(Op::AssignHost(h), at);
            }
        }
    }

    // ---- functions and blocks

    fn main(&mut self, prog: &Program) {
        self.fns.push(new_fn(String::new(), 0));
        for s in &prog.body {
            if let StmtKind::Fn(f) = &s.kind {
                self.declared_fn(f, s.span.start);
            }
        }
        for s in &prog.body {
            if !matches!(s.kind, StmtKind::Fn(_)) {
                self.stmt(s);
            }
        }
        let end = self.src.len() as u32;
        self.emit(Op::Nil, end);
        self.emit(Op::Return, end);
        let f = self.fns.pop().expect("main");
        self.protos[0] = f.p;
    }

    /// a fn declaration: its closure into its variable
    fn declared_fn(&mut self, f: &FnDef, at: u32) {
        let Some(n) = &f.name else { return };
        self.closure(f, n.text.clone());
        self.store_decl(n.span.start, at);
    }

    /// compile a function; its closure on the stack
    fn closure(&mut self, f: &FnDef, name: String) {
        let p = self.function(f, name);
        self.emit(Op::Closure(p), f.span.start);
    }

    /// compile a function -> its index in protos
    #[inline(never)]
    fn function(&mut self, f: &FnDef, name: String) -> u32 {
        self.fns.push(new_fn(name, f.params.len() as u32));
        for p in &f.params {
            self.declare(p.span.start);
        }
        let n = f.params.len() as u32;
        let st = self.f();
        st.height = n;
        st.p.max_stack = n;
        for p in &f.params {
            if self.captured(p.span.start) {
                let s = self.slot(p.span.start);
                self.emit(Op::BoxLocal(s), p.span.start);
            }
        }
        self.block(&f.body);
        let end = f.body.span.end.saturating_sub(1);
        self.emit(Op::Nil, end);
        self.emit(Op::Return, end);
        let st = self.fns.pop().expect("function");
        self.protos.push(st.p);
        self.protos.len() as u32 - 1
    }

    /// a block: slots for its declarations, cells for the captured ones, its fn declarations,
    /// its statements, popping the slots
    fn block(&mut self, b: &Block) {
        let base = self.f().locals;
        let mut decls: Vec<u32> = Vec::new();
        for s in &b.stmts {
            if let StmtKind::Let { name, .. } | StmtKind::Fn(FnDef { name: Some(name), .. }) = &s.kind {
                decls.push(name.span.start);
            }
        }
        for &d in &decls {
            self.declare(d);
        }
        let n = decls.len() as u32;
        if n > 0 {
            self.emit(Op::Reserve(n), b.span.start);
        }
        for &d in &decls {
            if self.captured(d) {
                let s = self.slot(d);
                self.emit(Op::NewCell(s), b.span.start);
            }
        }
        for s in &b.stmts {
            if let StmtKind::Fn(f) = &s.kind {
                self.declared_fn(f, s.span.start);
            }
        }
        for s in &b.stmts {
            if !matches!(s.kind, StmtKind::Fn(_)) {
                self.stmt(s);
            }
        }
        if n > 0 {
            self.emit(Op::PopN(n), b.span.end.saturating_sub(1));
        }
        self.f().locals = base;
    }

    // ---- statements

    fn stmt(&mut self, s: &Stmt) {
        let at = s.span.start;
        match &s.kind {
            StmtKind::Let { name, init } => {
                match init {
                    Some(e) => self.value(e, &name.text),
                    None => {
                        self.emit(Op::Nil, at);
                    }
                }
                self.store_decl(name.span.start, at);
            }
            StmtKind::Fn(_) => {}
            StmtKind::If { arms, els } => self.if_stmt(arms, els.as_ref()),
            StmtKind::While { cond, body } => self.while_stmt(cond, body),
            StmtKind::For { a, b, src, body } => self.for_stmt(a, b.as_ref(), src, body),
            StmtKind::Break | StmtKind::Continue => self.jump_out(s.kind == StmtKind::Break, at),
            StmtKind::Return(v) => {
                match v {
                    Some(e) => self.expr(e),
                    None => {
                        self.emit(Op::Nil, at);
                    }
                }
                self.emit(Op::Return, at);
            }
            StmtKind::Try { body, var, catch } => self.try_stmt(body, var.as_ref(), catch),
            StmtKind::Throw(e) => {
                self.expr(e);
                self.emit(Op::Throw, at);
            }
            StmtKind::Expr(e) => {
                self.expr(e);
                self.emit(Op::Pop, at);
            }
            StmtKind::Assign { op, target, value } => self.assign(*op, target, value),
            StmtKind::Block(b) => self.block(b),
        }
    }

    #[inline(never)]
    fn if_stmt(&mut self, arms: &[(Expr, Block)], els: Option<&Block>) {
        let mut ends = Vec::new();
        for (i, (c, b)) in arms.iter().enumerate() {
            self.expr(c);
            let j = self.emit(Op::JumpIfFalse(0), c.span.start);
            self.block(b);
            if i + 1 < arms.len() || els.is_some() {
                ends.push(self.emit(Op::Jump(0), b.span.end.saturating_sub(1)));
            }
            self.patch(j);
        }
        if let Some(b) = els {
            self.block(b);
        }
        for e in ends {
            self.patch(e);
        }
    }

    #[inline(never)]
    fn while_stmt(&mut self, cond: &Expr, body: &Block) {
        let top = self.here();
        self.expr(cond);
        let j = self.emit(Op::JumpIfFalse(0), cond.span.start);
        let height = self.f().locals;
        self.f().loops.push(Loop { cont: top, height, breaks: Vec::new() });
        self.block(body);
        self.emit(Op::Jump(top), body.span.end.saturating_sub(1));
        self.patch(j);
        let lp = self.f().loops.pop().expect("loop");
        for b in lp.breaks {
            self.patch(b);
        }
    }

    #[inline(never)]
    fn for_stmt(&mut self, a: &Name, b: Option<&Name>, src: &ForSrc, body: &Block) {
        let base = self.f().locals;
        let (hidden, at) = match src {
            ForSrc::Each(e) => {
                self.expr(e);
                self.emit(Op::Reserve(2), e.span.start);
                self.emit(Op::IterPrep(base), e.span.start);
                (3, e.span.start)
            }
            ForSrc::Range(lo, hi) => {
                self.expr(lo);
                self.expr(hi);
                self.emit(Op::RangePrep(base), lo.span.start);
                (2, lo.span.start)
            }
        };
        self.f().locals = base + hidden;
        let sa = self.declare(a.span.start);
        let sb = b.map(|b| self.declare(b.span.start));
        self.emit(Op::Reserve(1 + sb.is_some() as u32), a.span.start);
        let top = self.here();
        let next = match (src, sb) {
            (ForSrc::Range(..), _) => Op::RangeNext(base, 0),
            (_, Some(_)) => Op::IterNext2(base, 0),
            _ => Op::IterNext(base, 0),
        };
        let next = self.emit(next, at);
        if let Some(sb) = sb {
            self.emit(Op::SetLocal(sb), at);
        }
        self.emit(Op::SetLocal(sa), at);
        for (n, s) in [(Some(a), Some(sa)), (b, sb)] {
            if let (Some(n), Some(s)) = (n, s) {
                if self.captured(n.span.start) {
                    self.emit(Op::BoxLocal(s), n.span.start);
                }
            }
        }
        let height = self.f().locals;
        self.f().loops.push(Loop { cont: top, height, breaks: Vec::new() });
        self.block(body);
        self.emit(Op::Jump(top), body.span.end.saturating_sub(1));
        self.patch(next);
        let lp = self.f().loops.pop().expect("loop");
        for b in lp.breaks {
            self.patch(b);
        }
        let n = self.f().locals - base;
        self.emit(Op::PopN(n), body.span.end.saturating_sub(1));
        self.f().locals = base;
    }

    /// break (pop to the loop, jump to its end) or continue (pop, jump to its next round)
    fn jump_out(&mut self, brk: bool, at: u32) {
        let Some(lp) = self.f().loops.last() else { return };
        let (h, cont) = (lp.height, lp.cont);
        let (locals, saved) = (self.f().locals, self.f().height);
        if locals > h {
            self.emit(Op::PopN(locals - h), at);
        }
        if brk {
            let j = self.emit(Op::Jump(0), at);
            if let Some(lp) = self.f().loops.last_mut() {
                lp.breaks.push(j);
            }
        } else {
            self.emit(Op::Jump(cont), at);
        }
        self.f().height = saved;
    }

    #[inline(never)]
    fn try_stmt(&mut self, body: &Block, var: Option<&Name>, catch: &Block) {
        let h = self.f().locals;
        let start = self.here();
        self.block(body);
        let end = self.here();
        let j = self.emit(Op::Jump(0), body.span.end.saturating_sub(1));
        let target = self.here();
        // the VM cut the stack to h and pushed the exception: the catch variable's slot
        let f = self.f();
        f.height = h + 1;
        f.p.max_stack = f.p.max_stack.max(h + 1);
        f.locals = h + 1;
        if let Some(v) = var {
            self.f().slots.insert(v.span.start, h);
            if self.captured(v.span.start) {
                self.emit(Op::BoxLocal(h), v.span.start);
            }
        }
        self.block(catch);
        self.emit(Op::PopN(1), catch.span.end.saturating_sub(1));
        self.f().locals = h;
        self.patch(j);
        self.f().p.handlers.push(Handler { start, end, target, height: h });
    }

    #[inline(never)]
    fn assign(&mut self, op: Option<BinOp>, target: &Expr, value: &Expr) {
        let at_op = self.skip(target.span.end);
        match &target.kind {
            ExprKind::Var(n) => {
                let key = target.span.start;
                match op {
                    Some(op) => {
                        self.load_var(key, n, key);
                        self.expr(value);
                        self.binop(op, at_op);
                    }
                    None => self.value(value, n),
                }
                self.store_var(key, n, key);
            }
            ExprKind::Field(obj, name) => {
                self.expr(obj);
                let k = self.kstr(&name.text);
                match op {
                    Some(op) => {
                        self.emit(Op::Dup, name.span.start);
                        self.emit(Op::GetField(k), name.span.start);
                        self.expr(value);
                        self.binop(op, at_op);
                    }
                    None => self.value(value, &name.text),
                }
                self.emit(Op::SetField(k), name.span.start);
            }
            ExprKind::Index(obj, idx) => {
                let at = self.skip(obj.span.end);
                self.expr(obj);
                self.expr(idx);
                match op {
                    Some(op) => {
                        self.emit(Op::Dup2, at);
                        self.emit(Op::GetIndex, at);
                        self.expr(value);
                        self.binop(op, at_op);
                    }
                    None => self.expr(value),
                }
                self.emit(Op::SetIndex, at);
            }
            _ => {}
        }
    }

    // ---- expressions

    /// an expression; a fn literal gets `name` (let f = fn .., m.k = fn .., {k: fn ..})
    fn value(&mut self, e: &Expr, name: &str) {
        match &e.kind {
            ExprKind::Fn(f) => self.closure(f, String::from(name)),
            _ => self.expr(e),
        }
    }

    fn binop(&mut self, op: BinOp, at: u32) {
        let o = match op {
            BinOp::Add => Op::Add,
            BinOp::Sub => Op::Sub,
            BinOp::Mul => Op::Mul,
            BinOp::Div => Op::Div,
            BinOp::Rem => Op::Rem,
            BinOp::Lt => Op::Lt,
            BinOp::Le => Op::Le,
            BinOp::Gt => Op::Gt,
            BinOp::Ge => Op::Ge,
            BinOp::Eq => Op::Eq,
            BinOp::Ne => Op::Ne,
            BinOp::And | BinOp::Or | BinOp::Coalesce => return,
        };
        self.emit(o, at);
    }

    fn expr(&mut self, e: &Expr) {
        let at = e.span.start;
        match &e.kind {
            ExprKind::Nil => {
                self.emit(Op::Nil, at);
            }
            ExprKind::Bool(b) => {
                self.emit(if *b { Op::True } else { Op::False }, at);
            }
            ExprKind::Int(v) => {
                let op = match i32::try_from(*v) {
                    Ok(i) => Op::Int(i),
                    Err(_) => Op::Const(self.kint(*v)),
                };
                self.emit(op, at);
            }
            ExprKind::Float(v) => {
                let k = self.kflt(*v);
                self.emit(Op::Const(k), at);
            }
            ExprKind::Str(s) => {
                let k = self.kstr(s);
                self.emit(Op::Const(k), at);
            }
            ExprKind::Var(n) => self.load_var(at, n, at),
            ExprKind::Fn(f) => self.closure(f, String::from("fn")),
            ExprKind::Unary(op, a) => {
                self.expr(a);
                self.emit(if *op == UnOp::Neg { Op::Neg } else { Op::Not }, at);
            }
            ExprKind::Binary(op, a, b) => self.binary(*op, a, b),
            ExprKind::Interp(items) | ExprKind::List(items) => self.items(e, items),
            ExprKind::Map(kv) => self.map(e, kv),
            ExprKind::Call(f, args) => self.call(f, args),
            ExprKind::Method(obj, name, args) => self.method(obj, name, args),
            ExprKind::Field(obj, name) => self.field(obj, name),
            ExprKind::Index(obj, i) => {
                let at = self.skip(obj.span.end);
                self.expr(obj);
                self.expr(i);
                self.emit(Op::GetIndex, at);
            }
        }
    }

    #[inline(never)]
    fn binary(&mut self, op: BinOp, a: &Expr, b: &Expr) {
        let at = self.skip(a.span.end);
        self.expr(a);
        let jump = match op {
            BinOp::And => Some(Op::And(0)),
            BinOp::Or => Some(Op::Or(0)),
            BinOp::Coalesce => Some(Op::Coalesce(0)),
            _ => None,
        };
        match jump {
            Some(j) => {
                let j = self.emit(j, at);
                self.expr(b);
                self.patch(j);
            }
            None => {
                self.expr(b);
                self.binop(op, at);
            }
        }
    }

    /// a list literal or an interpolation
    #[inline(never)]
    fn items(&mut self, e: &Expr, items: &[Expr]) {
        for x in items {
            self.expr(x);
        }
        let n = items.len() as u32;
        let op = if matches!(e.kind, ExprKind::List(_)) { Op::MakeList(n) } else { Op::Concat(n) };
        self.emit(op, e.span.start);
    }

    #[inline(never)]
    fn map(&mut self, e: &Expr, kv: &[(Expr, Expr)]) {
        for (k, v) in kv {
            self.expr(k);
            match &k.kind {
                ExprKind::Str(s) => self.value(v, s),
                _ => self.expr(v),
            }
        }
        self.emit(Op::MakeMap(kv.len() as u32), e.span.start);
    }

    #[inline(never)]
    fn call(&mut self, f: &Expr, args: &[Expr]) {
        let argc = args.len() as u32;
        if let ExprKind::Var(n) = &f.kind {
            if self.is_host(f.span.start) {
                let h = self.host(n);
                for a in args {
                    self.expr(a);
                }
                self.emit(Op::CallHost(h, argc), f.span.start);
                return;
            }
        }
        self.expr(f);
        for a in args {
            self.expr(a);
        }
        self.emit(Op::Call(argc), f.span.start);
    }

    #[inline(never)]
    fn method(&mut self, obj: &Expr, name: &Name, args: &[Expr]) {
        let argc = args.len() as u32;
        if let ExprKind::Var(h) = &obj.kind {
            if self.is_host(obj.span.start) {
                let e = self.host_field(h, &name.text);
                for a in args {
                    self.expr(a);
                }
                self.emit(Op::CallHostMethod(e, argc), name.span.start);
                return;
            }
        }
        self.expr(obj);
        let k = self.kstr(&name.text);
        for a in args {
            self.expr(a);
        }
        self.emit(Op::CallMethod(k, argc), name.span.start);
    }

    #[inline(never)]
    fn field(&mut self, obj: &Expr, name: &Name) {
        if let ExprKind::Var(h) = &obj.kind {
            if self.is_host(obj.span.start) {
                let e = self.host_field(h, &name.text);
                self.emit(Op::GetHostField(e), name.span.start);
                return;
            }
        }
        self.expr(obj);
        let k = self.kstr(&name.text);
        self.emit(Op::GetField(k), name.span.start);
    }
}
