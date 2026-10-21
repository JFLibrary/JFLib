// check -- name resolution over the syntax tree, with the scope rules of ast.rs
//
//   resolve(&program, globals) -> Vec<Diagnostic>
//     errors:   a name declared twice in one block (also parameters, the two for
//               variables, top-level declarations), break / continue outside a loop (a
//               function body starts outside of any loop), return outside a function, two
//               variables over a range a..b
//     warnings: a name that is neither declared nor in globals (with a suggestion for a
//               typing error), a top-level let read by top-level code before it ran
//   globals: the predeclared names (crate::GLOBALS for check())

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::*;
use crate::Diagnostic;

struct Resolver<'a> {
    globals: &'a [&'a str],
    /// top-level lets and fns: name, the byte from which top-level code may read it, line
    top: Vec<(&'a str, u32, u32)>,
    /// the block scopes inside the top level, innermost last
    scopes: Vec<Vec<&'a Name>>,
    fns: u32,
    loops: u32,
    out: Vec<Diagnostic>,
}

/// see the module comment
pub fn resolve(prog: &Program, globals: &[&str]) -> Vec<Diagnostic> {
    let mut r = Resolver { globals, top: Vec::new(), scopes: Vec::new(), fns: 0, loops: 0, out: Vec::new() };
    for s in &prog.body {
        match &s.kind {
            StmtKind::Let { name, .. } => r.declare_top(name, s.span.end),
            StmtKind::Fn(FnDef { name: Some(name), .. }) => r.declare_top(name, 0),
            _ => {}
        }
    }
    for s in &prog.body {
        r.stmt(s);
    }
    r.out
}

/// edit distance of two names (characters)
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for j in 0..b.len() {
            let up = row[j + 1];
            row[j + 1] = if ca == b[j] { diag } else { 1 + diag.min(up).min(row[j]) };
            diag = up;
        }
    }
    row[b.len()]
}

impl<'a> Resolver<'a> {
    fn report(&mut self, sp: Span, len: usize, error: bool, msg: String) {
        let end_col = sp.col + (len as u32).max(1);
        self.out.push(Diagnostic { line: sp.line, col: sp.col, end_col, error, msg });
    }

    fn name_error(&mut self, n: &Name, msg: String) {
        self.report(n.span, n.text.chars().count(), true, msg);
    }

    fn declare_top(&mut self, n: &'a Name, from: u32) {
        if let Some(&(_, _, line)) = self.top.iter().find(|d| d.0 == n.text) {
            self.name_error(n, format!("{} ist schon deklariert (Zeile {})", n.text, line));
        } else {
            self.top.push((n.text.as_str(), from, n.span.line));
        }
    }

    /// a name in the innermost scope; param: the scope holds parameters
    fn declare(&mut self, n: &'a Name, param: bool) {
        let Some(scope) = self.scopes.last_mut() else { return };
        if let Some(prev) = scope.iter().find(|p| p.text == n.text) {
            let line = prev.span.line;
            let msg = if param {
                format!("Der Parameter {} kommt zweimal vor", n.text)
            } else {
                format!("{} ist in diesem Block schon deklariert (Zeile {})", n.text, line)
            };
            self.name_error(n, msg);
        } else {
            scope.push(n);
        }
    }

    /// the closest visible or global name to a misspelt one (none for names of 1 or 2
    /// characters)
    fn similar(&self, name: &str) -> Option<String> {
        let max = match name.chars().count() {
            0..=2 => return None,
            3..=4 => 1,
            _ => 2,
        };
        let mut best: Option<(usize, &str)> = None;
        let scoped = self.scopes.iter().flatten().map(|n| n.text.as_str());
        let top = self.top.iter().map(|d| d.0);
        for c in scoped.chain(top).chain(self.globals.iter().copied()) {
            let d = distance(name, c);
            if d <= max && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, c));
            }
        }
        best.map(|(_, c)| String::from(c))
    }

    /// a name that is read (assign: written)
    fn use_name(&mut self, name: &str, sp: Span, assign: bool) {
        if self.scopes.iter().any(|s| s.iter().any(|n| n.text == name)) {
            return;
        }
        if let Some(&(_, from, line)) = self.top.iter().find(|d| d.0 == name) {
            if self.fns == 0 && sp.start < from && !assign {
                let msg = format!("{} wird benutzt, bevor es mit let angelegt wird (Zeile {})", name, line);
                self.report(sp, name.chars().count(), false, msg);
            }
            return;
        }
        if self.globals.contains(&name) {
            return;
        }
        let mut msg = format!("Unbekannter Name {}", name);
        if let Some(s) = self.similar(name) {
            msg.push_str(&format!(" – meinst du {}?", s));
        } else if assign {
            msg.push_str(&format!(" – eine neue Variable wird mit let {} = … angelegt", name));
        }
        self.report(sp, name.chars().count(), false, msg);
    }

    fn block(&mut self, b: &'a Block) {
        self.scopes.push(Vec::new());
        for s in &b.stmts {
            if let StmtKind::Fn(FnDef { name: Some(n), .. }) = &s.kind {
                self.declare(n, false);
            }
        }
        for s in &b.stmts {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    fn func(&mut self, f: &'a FnDef) {
        let loops = core::mem::replace(&mut self.loops, 0);
        self.fns += 1;
        self.scopes.push(Vec::new());
        for p in &f.params {
            self.declare(p, true);
        }
        self.block(&f.body);
        self.scopes.pop();
        self.fns -= 1;
        self.loops = loops;
    }

    fn stmt(&mut self, s: &'a Stmt) {
        match &s.kind {
            StmtKind::Let { name, init } => {
                if let Some(e) = init {
                    self.expr(e);
                }
                self.declare(name, false);
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
                self.loops += 1;
                self.block(body);
                self.loops -= 1;
            }
            StmtKind::For { a, b, src, body } => {
                match src {
                    ForSrc::Each(e) => self.expr(e),
                    ForSrc::Range(lo, hi) => {
                        self.expr(lo);
                        self.expr(hi);
                    }
                }
                self.scopes.push(Vec::new());
                self.declare(a, false);
                if let Some(b) = b {
                    self.declare(b, false);
                    if matches!(src, ForSrc::Range(..)) {
                        self.name_error(b, String::from("Ein Bereich a..b hat nur eine Laufvariable"));
                    }
                }
                self.loops += 1;
                self.block(body);
                self.loops -= 1;
                self.scopes.pop();
            }
            StmtKind::Break | StmtKind::Continue => {
                if self.loops == 0 {
                    let (w, len) = if s.kind == StmtKind::Break { ("break", 5) } else { ("continue", 8) };
                    self.report(s.span, len, true, format!("{} steht außerhalb einer Schleife", w));
                }
            }
            StmtKind::Return(v) => {
                if self.fns == 0 {
                    self.report(s.span, 6, true, String::from("return steht außerhalb einer Funktion"));
                }
                if let Some(e) = v {
                    self.expr(e);
                }
            }
            StmtKind::Try { body, var, catch } => {
                self.block(body);
                self.scopes.push(Vec::new());
                if let Some(v) = var {
                    self.declare(v, false);
                }
                self.block(catch);
                self.scopes.pop();
            }
            StmtKind::Throw(e) | StmtKind::Expr(e) => self.expr(e),
            StmtKind::Assign { target, value, .. } => {
                match &target.kind {
                    ExprKind::Var(n) => self.use_name(n, target.span, true),
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
            ExprKind::Var(n) => self.use_name(n, e.span, false),
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
