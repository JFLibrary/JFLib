// ast -- the syntax tree of a .jfs script (parse() builds it, the compiler reads it)
//
//   Program { name, body }   the top-level statements
//   Stmt { span, kind }      StmtKind: Let, Fn, If, While, For, Break, Continue, Return, Try,
//                            Throw, Expr, Assign, Block
//   Expr { span, kind }      ExprKind: literals, Interp, List, Map, Fn, Var, Unary, Binary,
//                            Call, Method, Field, Index
//   Span                     bytes start..end plus line and column of the start (1-based,
//                            columns count characters); a name's span.start is unique per
//                            source, a key for side tables of later passes
//   x.sexpr()                the tree as S-expressions: (+ 1 (* 2 3)), (call f a), (. m k) ..
//
// Scopes (check::resolve and the compiler follow the same rules):
//   * top-level let and fn are the script's globals: visible in every function wherever they
//     are declared (functions run later, as callbacks); top-level code itself reads a let
//     only after it ran (check warns otherwise)
//   * a block { } is a scope: its fn declarations are visible in the whole block (local
//     functions may call each other), a let from the next statement on; a second let of a
//     name in one block is an error, an inner block may shadow it
//   * parameters form a scope around the function body; the for variables and the catch
//     variable a scope around their block
//   * fn declarations and fn literals are closures: they capture the locals they use
//   * && || ?? evaluate their right side only when needed; a..b exists only in for and
//     counts a, a + 1 .. b - 1

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// a source range: bytes start..end, line and column of start (1-based; columns count characters)
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub col: u32,
}

impl Span {
    /// from the start of self to the end of other
    pub fn to(self, other: Span) -> Span {
        Span { end: other.end.max(self.end), ..self }
    }
}

/// a name where it is written: declarations, parameters, field and method names
#[derive(Clone, PartialEq, Debug)]
pub struct Name {
    pub text: String,
    pub span: Span,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Program {
    /// the name given to parse() (a file name for messages)
    pub name: String,
    pub body: Vec<Stmt>,
}

/// { statements }
#[derive(Clone, PartialEq, Debug)]
pub struct Block {
    pub span: Span,
    pub stmts: Vec<Stmt>,
}

/// a function: fn name(params) { body }, or the literal fn(params) { body } without a name
#[derive(Clone, PartialEq, Debug)]
pub struct FnDef {
    pub span: Span,
    pub name: Option<Name>,
    pub params: Vec<Name>,
    pub body: Block,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Stmt {
    pub span: Span,
    pub kind: StmtKind,
}

#[derive(Clone, PartialEq, Debug)]
pub enum StmtKind {
    /// let name = init (no init: nil)
    Let { name: Name, init: Option<Expr> },
    /// fn name(params) { } (the FnDef has a name)
    Fn(FnDef),
    /// if c { } else if c { } else { }: the (condition, block) arms in order
    If { arms: Vec<(Expr, Block)>, els: Option<Block> },
    While { cond: Expr, body: Block },
    /// for a in src / for a, b in src (key and value of a map, index and item of a list)
    For { a: Name, b: Option<Name>, src: ForSrc, body: Block },
    Break,
    Continue,
    Return(Option<Expr>),
    /// try { body } catch var { catch }
    Try { body: Block, var: Option<Name>, catch: Block },
    Throw(Expr),
    Expr(Expr),
    /// target = value, or target op= value (op Some(Add) for +=); target is Var, Field or Index
    Assign { op: Option<BinOp>, target: Expr, value: Expr },
    /// a bare { } block
    Block(Block),
}

/// what a for loop runs over
#[derive(Clone, PartialEq, Debug)]
pub enum ForSrc {
    /// a list, map, str or bytes value
    Each(Expr),
    /// lo..hi
    Range(Expr, Expr),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Expr {
    pub span: Span,
    pub kind: ExprKind,
}

#[derive(Clone, PartialEq, Debug)]
pub enum ExprKind {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// "a${x}b": the pieces in order, Str for the text (empty pieces left out)
    Interp(Vec<Expr>),
    List(Vec<Expr>),
    /// {k: v, "s": v, 1: v, [e]: v}: (key, value) in order; a bare name key is a Str
    Map(Vec<(Expr, Expr)>),
    Fn(Box<FnDef>),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// f(args)
    Call(Box<Expr>, Vec<Expr>),
    /// obj.name(args)
    Method(Box<Expr>, Name, Vec<Expr>),
    /// obj.name
    Field(Box<Expr>, Name),
    /// obj[index]
    Index(Box<Expr>, Box<Expr>),
}

/// the unary operators: minus and not (!)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnOp {
    Neg,
    Not,
}

/// binary operators; And, Or and Coalesce (??) short-circuit
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    Mul, Div, Rem, Add, Sub, Lt, Le, Gt, Ge, Eq, Ne, And, Or, Coalesce,
}

impl BinOp {
    /// the operator as written: "*", "&&", "??" ..
    pub fn text(self) -> &'static str {
        match self {
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::Coalesce => "??",
        }
    }
}

// ---- S-expressions

impl Program {
    /// one S-expression per top-level statement, one per line
    pub fn sexpr(&self) -> String {
        let v: Vec<String> = self.body.iter().map(Stmt::sexpr).collect();
        v.join("\n")
    }
}

impl Stmt {
    /// (let x 1), (fn f (a) (block ..)), (if c (block ..) else (block ..)), (= x 1) ..
    pub fn sexpr(&self) -> String {
        let mut o = String::new();
        stmt(&mut o, self);
        o
    }
}

impl Expr {
    /// (+ 1 (* 2 3)), (call f a), (method s split ","), (. m k), (index a 0), (interp "a" x) ..
    pub fn sexpr(&self) -> String {
        let mut o = String::new();
        expr(&mut o, self);
        o
    }
}

fn block(o: &mut String, b: &Block) {
    o.push_str("(block");
    for s in &b.stmts {
        o.push(' ');
        stmt(o, s);
    }
    o.push(')');
}

fn func(o: &mut String, f: &FnDef) {
    o.push_str("(fn ");
    if let Some(n) = &f.name {
        o.push_str(&n.text);
        o.push(' ');
    }
    let params: Vec<&str> = f.params.iter().map(|p| p.text.as_str()).collect();
    o.push_str(&format!("({}) ", params.join(" ")));
    block(o, &f.body);
    o.push(')');
}

fn stmt(o: &mut String, s: &Stmt) {
    match &s.kind {
        StmtKind::Let { name, init } => {
            o.push_str("(let ");
            o.push_str(&name.text);
            if let Some(e) = init {
                o.push(' ');
                expr(o, e);
            }
            o.push(')');
        }
        StmtKind::Fn(f) => func(o, f),
        StmtKind::If { arms, els } => {
            o.push_str("(if");
            for (c, b) in arms {
                o.push(' ');
                expr(o, c);
                o.push(' ');
                block(o, b);
            }
            if let Some(b) = els {
                o.push_str(" else ");
                block(o, b);
            }
            o.push(')');
        }
        StmtKind::While { cond, body } => {
            o.push_str("(while ");
            expr(o, cond);
            o.push(' ');
            block(o, body);
            o.push(')');
        }
        StmtKind::For { a, b, src, body } => {
            o.push_str("(for ");
            match b {
                Some(b) => o.push_str(&format!("({} {})", a.text, b.text)),
                None => o.push_str(&a.text),
            }
            o.push(' ');
            match src {
                ForSrc::Each(e) => expr(o, e),
                ForSrc::Range(lo, hi) => {
                    o.push_str("(.. ");
                    expr(o, lo);
                    o.push(' ');
                    expr(o, hi);
                    o.push(')');
                }
            }
            o.push(' ');
            block(o, body);
            o.push(')');
        }
        StmtKind::Break => o.push_str("break"),
        StmtKind::Continue => o.push_str("continue"),
        StmtKind::Return(v) => {
            o.push_str("(return");
            if let Some(e) = v {
                o.push(' ');
                expr(o, e);
            }
            o.push(')');
        }
        StmtKind::Try { body, var, catch } => {
            o.push_str("(try ");
            block(o, body);
            o.push(' ');
            o.push_str(var.as_ref().map_or("_", |v| v.text.as_str()));
            o.push(' ');
            block(o, catch);
            o.push(')');
        }
        StmtKind::Throw(e) => {
            o.push_str("(throw ");
            expr(o, e);
            o.push(')');
        }
        StmtKind::Expr(e) => expr(o, e),
        StmtKind::Assign { op, target, value } => {
            o.push('(');
            o.push_str(op.map_or("", BinOp::text));
            o.push_str("= ");
            expr(o, target);
            o.push(' ');
            expr(o, value);
            o.push(')');
        }
        StmtKind::Block(b) => block(o, b),
    }
}

fn list(o: &mut String, head: &str, items: &[Expr]) {
    o.push('(');
    o.push_str(head);
    for e in items {
        o.push(' ');
        expr(o, e);
    }
    o.push(')');
}

fn expr(o: &mut String, e: &Expr) {
    match &e.kind {
        ExprKind::Nil => o.push_str("nil"),
        ExprKind::Bool(b) => o.push_str(if *b { "true" } else { "false" }),
        ExprKind::Int(v) => o.push_str(&format!("{}", v)),
        ExprKind::Float(v) => o.push_str(&format!("{:?}", v)),
        ExprKind::Str(s) => quote(o, s),
        ExprKind::Interp(parts) => list(o, "interp", parts),
        ExprKind::List(items) => list(o, "list", items),
        ExprKind::Map(kv) => {
            o.push_str("(map");
            for (k, v) in kv {
                o.push_str(" (");
                expr(o, k);
                o.push(' ');
                expr(o, v);
                o.push(')');
            }
            o.push(')');
        }
        ExprKind::Fn(f) => func(o, f),
        ExprKind::Var(n) => o.push_str(n),
        ExprKind::Unary(op, a) => {
            o.push_str(if *op == UnOp::Neg { "(- " } else { "(! " });
            expr(o, a);
            o.push(')');
        }
        ExprKind::Binary(op, a, b) => {
            o.push('(');
            o.push_str(op.text());
            o.push(' ');
            expr(o, a);
            o.push(' ');
            expr(o, b);
            o.push(')');
        }
        ExprKind::Call(f, args) => {
            o.push_str("(call ");
            expr(o, f);
            for a in args {
                o.push(' ');
                expr(o, a);
            }
            o.push(')');
        }
        ExprKind::Method(obj, name, args) => {
            o.push_str("(method ");
            expr(o, obj);
            o.push(' ');
            o.push_str(&name.text);
            for a in args {
                o.push(' ');
                expr(o, a);
            }
            o.push(')');
        }
        ExprKind::Field(obj, name) => {
            o.push_str("(. ");
            expr(o, obj);
            o.push(' ');
            o.push_str(&name.text);
            o.push(')');
        }
        ExprKind::Index(obj, i) => {
            o.push_str("(index ");
            expr(o, obj);
            o.push(' ');
            expr(o, i);
            o.push(')');
        }
    }
}

/// a string in double quotes with \" \\ \n \t and \u{..} for other control characters
fn quote(o: &mut String, s: &str) {
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if c.is_control() => o.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
}
