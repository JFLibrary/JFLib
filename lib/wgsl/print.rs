// print -- the AST as WGSL text: four-space indentation, one statement per line, literals as
// written, parentheses exactly where the tree has Paren nodes (so parse(print(ast)) gives
// the same tree, spans aside; comments are not kept)
//
//   print(&ast) -> String

use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::*;

/// see the module comment
pub fn print(ast: &Ast) -> String {
    let mut p = P { out: String::new(), ind: 0 };
    for d in &ast.directives {
        match &d.kind {
            DirectiveKind::Enable(v) => {
                p.out.push_str("enable ");
                p.names(v);
                p.out.push_str(";\n");
            }
            DirectiveKind::Requires(v) => {
                p.out.push_str("requires ");
                p.names(v);
                p.out.push_str(";\n");
            }
            DirectiveKind::Diagnostic(c) => {
                p.out.push_str("diagnostic");
                p.control(c);
                p.out.push_str(";\n");
            }
        }
    }
    for (i, d) in ast.decls.iter().enumerate() {
        if i > 0 || !ast.directives.is_empty() {
            p.out.push('\n');
        }
        p.decl(d);
    }
    p.out
}

struct P {
    out: String,
    ind: usize,
}

impl P {
    fn s(&mut self, t: &str) {
        self.out.push_str(t);
    }
    fn nl(&mut self) {
        self.out.push('\n');
        for _ in 0..self.ind {
            self.out.push_str("    ");
        }
    }
    fn names(&mut self, v: &[Ident]) {
        for (i, n) in v.iter().enumerate() {
            if i > 0 {
                self.s(", ");
            }
            self.s(&n.name);
        }
    }
    fn control(&mut self, c: &DiagnosticControl) {
        self.s("(");
        self.s(&c.severity.name);
        self.s(", ");
        self.s(&c.rule.name);
        if let Some(r) = &c.rule2 {
            self.s(".");
            self.s(&r.name);
        }
        self.s(")");
    }
    fn attrs(&mut self, v: &[Attr], sep: &str) {
        for a in v {
            self.s("@");
            self.s(&a.name.name);
            match &a.kind {
                AttrKind::Diagnostic(c) => self.control(c),
                AttrKind::Plain { args, parens } => {
                    if *parens {
                        self.s("(");
                        self.list(args);
                        self.s(")");
                    }
                }
            }
            self.s(sep);
        }
    }
    fn list(&mut self, v: &[Expr]) {
        for (i, e) in v.iter().enumerate() {
            if i > 0 {
                self.s(", ");
            }
            self.expr(e);
        }
    }
    fn decl(&mut self, d: &Decl) {
        match &d.kind {
            DeclKind::Fn(f) => {
                self.attrs(&f.attrs, "\n");
                self.s("fn ");
                self.s(&f.name.name);
                self.s("(");
                for (i, prm) in f.params.iter().enumerate() {
                    if i > 0 {
                        self.s(", ");
                    }
                    self.attrs(&prm.attrs, " ");
                    self.s(&prm.name.name);
                    self.s(": ");
                    self.expr(&prm.ty);
                }
                self.s(")");
                if let Some((a, t)) = &f.ret {
                    self.s(" -> ");
                    self.attrs(a, " ");
                    self.expr(t);
                }
                self.s(" ");
                self.block(&f.body);
                self.s("\n");
            }
            DeclKind::Var(v) => {
                self.var(v);
                self.s(";\n");
            }
            DeclKind::Alias { name, ty } => {
                self.s("alias ");
                self.s(&name.name);
                self.s(" = ");
                self.expr(ty);
                self.s(";\n");
            }
            DeclKind::Struct { name, members } => {
                self.s("struct ");
                self.s(&name.name);
                self.s(" {");
                self.ind += 1;
                for m in members {
                    self.nl();
                    self.attrs(&m.attrs, " ");
                    self.s(&m.name.name);
                    self.s(": ");
                    self.expr(&m.ty);
                    self.s(",");
                }
                self.ind -= 1;
                self.nl();
                self.s("}\n");
            }
            DeclKind::ConstAssert(e) => {
                self.s("const_assert ");
                self.expr(e);
                self.s(";\n");
            }
        }
    }
    fn var(&mut self, v: &VarDecl) {
        self.attrs(&v.attrs, " ");
        self.s(match v.kind {
            VarKind::Var => "var",
            VarKind::Let => "let",
            VarKind::Const => "const",
            VarKind::Override => "override",
        });
        if let Some(t) = &v.templ {
            self.s("<");
            self.list(t);
            self.s(">");
        }
        self.s(" ");
        self.s(&v.name.name);
        if let Some(t) = &v.ty {
            self.s(": ");
            self.expr(t);
        }
        if let Some(e) = &v.init {
            self.s(" = ");
            self.expr(e);
        }
    }
    fn block(&mut self, b: &Block) {
        self.attrs(&b.attrs, " ");
        self.stmts_block(&b.stmts, None);
    }
    fn stmts_block(&mut self, v: &[Stmt], cont: Option<&Continuing>) {
        self.s("{");
        self.ind += 1;
        for st in v {
            self.nl();
            self.stmt(st);
        }
        if let Some(c) = cont {
            self.nl();
            self.s("continuing ");
            self.attrs(&c.body.attrs, " ");
            self.s("{");
            self.ind += 1;
            for st in &c.body.stmts {
                self.nl();
                self.stmt(st);
            }
            if let Some(e) = &c.break_if {
                self.nl();
                self.s("break if ");
                self.expr(e);
                self.s(";");
            }
            self.ind -= 1;
            self.nl();
            self.s("}");
        }
        self.ind -= 1;
        self.nl();
        self.s("}");
    }
    /// a statement without its `;` (for headers)
    fn simple(&mut self, st: &Stmt) {
        match &st.kind {
            StmtKind::Decl(v) => self.var(v),
            StmtKind::Assign { lhs, op, rhs } => {
                self.expr(lhs);
                self.s(" ");
                self.s(op.text());
                self.s(" ");
                self.expr(rhs);
            }
            StmtKind::Phony(e) => {
                self.s("_ = ");
                self.expr(e);
            }
            StmtKind::Incr(e) => {
                self.expr(e);
                self.s("++");
            }
            StmtKind::Decr(e) => {
                self.expr(e);
                self.s("--");
            }
            StmtKind::Call(e) => self.expr(e),
            _ => {}
        }
    }
    fn stmt(&mut self, st: &Stmt) {
        match &st.kind {
            StmtKind::Empty => self.s(";"),
            StmtKind::Block(b) => self.block(b),
            StmtKind::Return(e) => {
                self.s("return");
                if let Some(e) = e {
                    self.s(" ");
                    self.expr(e);
                }
                self.s(";");
            }
            StmtKind::If { attrs, cond, then, else_ifs, else_ } => {
                self.attrs(attrs, " ");
                self.s("if ");
                self.expr(cond);
                self.s(" ");
                self.block(then);
                for ei in else_ifs {
                    self.s(" else if ");
                    self.expr(&ei.cond);
                    self.s(" ");
                    self.block(&ei.body);
                }
                if let Some(b) = else_ {
                    self.s(" else ");
                    self.block(b);
                }
            }
            StmtKind::Switch { attrs, selector, body_attrs, clauses, .. } => {
                self.attrs(attrs, " ");
                self.s("switch ");
                self.expr(selector);
                self.s(" ");
                self.attrs(body_attrs, " ");
                self.s("{");
                self.ind += 1;
                for c in clauses {
                    self.nl();
                    if c.default_alone {
                        self.s("default ");
                    } else {
                        self.s("case ");
                        for (i, sel) in c.sels.iter().enumerate() {
                            if i > 0 {
                                self.s(", ");
                            }
                            match sel {
                                CaseSel::Default(_) => self.s("default"),
                                CaseSel::Expr(e) => self.expr(e),
                            }
                        }
                        self.s(": ");
                    }
                    self.block(&c.body);
                }
                self.ind -= 1;
                self.nl();
                self.s("}");
            }
            StmtKind::Loop { attrs, body, continuing } => {
                self.attrs(attrs, " ");
                self.s("loop ");
                self.attrs(&body.attrs, " ");
                self.stmts_block(&body.stmts, continuing.as_ref());
            }
            StmtKind::For { attrs, init, cond, update, body } => {
                self.attrs(attrs, " ");
                self.s("for (");
                if let Some(i) = init {
                    self.simple(i);
                }
                self.s(";");
                if let Some(c) = cond {
                    self.s(" ");
                    self.expr(c);
                }
                self.s(";");
                if let Some(u) = update {
                    self.s(" ");
                    self.simple(u);
                }
                self.s(") ");
                self.block(body);
            }
            StmtKind::While { attrs, cond, body } => {
                self.attrs(attrs, " ");
                self.s("while ");
                self.expr(cond);
                self.s(" ");
                self.block(body);
            }
            StmtKind::Break => self.s("break;"),
            StmtKind::Continue => self.s("continue;"),
            StmtKind::Discard => self.s("discard;"),
            StmtKind::ConstAssert(e) => {
                self.s("const_assert ");
                self.expr(e);
                self.s(";");
            }
            _ => {
                self.simple(st);
                self.s(";");
            }
        }
    }
    fn targs(&mut self, t: &Option<Vec<Expr>>) {
        if let Some(t) = t {
            self.s("<");
            self.list(t);
            self.s(">");
        }
    }
    fn expr(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Bool(b) => self.s(if *b { "true" } else { "false" }),
            ExprKind::Int(t, ..) | ExprKind::Float(t, ..) => self.s(t),
            ExprKind::Ident(id, t) => {
                self.s(&id.name);
                self.targs(t);
            }
            ExprKind::Call(id, t, args) => {
                self.s(&id.name);
                self.targs(t);
                self.s("(");
                self.list(args);
                self.s(")");
            }
            ExprKind::Paren(x) => {
                self.s("(");
                self.expr(x);
                self.s(")");
            }
            ExprKind::Unary(op, x) => {
                self.s(op.text());
                self.expr(x);
            }
            ExprKind::Binary(op, a, b) => {
                self.expr(a);
                self.s(" ");
                self.s(op.text());
                self.s(" ");
                self.expr(b);
            }
            ExprKind::Index(a, i) => {
                self.expr(a);
                self.s("[");
                self.expr(i);
                self.s("]");
            }
            ExprKind::Member(a, m) => {
                self.expr(a);
                self.s(".");
                self.s(&m.name);
            }
            ExprKind::Error => self.s("/*error*/"),
        }
    }
}
