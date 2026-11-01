// ast -- the syntax tree of a WGSL module, every node with its byte span
//
//   Ast { directives, decls }      the module in source order
//   Decl                           fn, var, const, override, alias, struct, const_assert
//   Stmt / StmtKind                every statement of spec section 9 (for and while stay as
//                                  written; the IR desugars them)
//   Expr / ExprKind                expressions; a type is an expression too (an identifier
//                                  with an optional template list: `array<vec4<f32>, 4>`)
//   Attr                           `@name(args)`; @diagnostic keeps its two names
//
// Literals keep their text (the printer writes it back unchanged) and their decoded value.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::lex::Suffix;

/// a byte range in the source
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// start..end
    pub fn new(start: u32, end: u32) -> Span {
        Span { start, end }
    }
    /// from the start of self to the end of b
    pub fn to(self, b: Span) -> Span {
        Span { start: self.start, end: b.end.max(self.start) }
    }
    /// b lies inside self
    pub fn contains(self, b: Span) -> bool {
        self.start <= b.start && b.end <= self.end
    }
}

/// a name with its place
#[derive(Clone, PartialEq, Debug)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

/// a parsed module: directives, then declarations in source order
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Ast {
    pub directives: Vec<Directive>,
    pub decls: Vec<Decl>,
}

/// enable / requires / diagnostic
#[derive(Clone, PartialEq, Debug)]
pub enum DirectiveKind {
    Enable(Vec<Ident>),
    Requires(Vec<Ident>),
    Diagnostic(DiagnosticControl),
}

/// a directive with its range
#[derive(Clone, PartialEq, Debug)]
pub struct Directive {
    pub kind: DirectiveKind,
    pub span: Span,
}

/// `(severity, rule)` or `(severity, a.b)`
#[derive(Clone, PartialEq, Debug)]
pub struct DiagnosticControl {
    pub severity: Ident,
    pub rule: Ident,
    pub rule2: Option<Ident>,
}

/// the arguments of an attribute
#[derive(Clone, PartialEq, Debug)]
pub enum AttrKind {
    /// @name or @name(args)
    Plain { args: Vec<Expr>, parens: bool },
    Diagnostic(DiagnosticControl),
}

/// an attribute `@name(args)`
#[derive(Clone, PartialEq, Debug)]
pub struct Attr {
    pub name: Ident,
    pub kind: AttrKind,
    pub span: Span,
}

impl Attr {
    /// the arguments (none for @diagnostic)
    pub fn args(&self) -> &[Expr] {
        match &self.kind {
            AttrKind::Plain { args, .. } => args,
            AttrKind::Diagnostic(_) => &[],
        }
    }
}

/// a function parameter
#[derive(Clone, PartialEq, Debug)]
pub struct Param {
    pub attrs: Vec<Attr>,
    pub name: Ident,
    pub ty: Expr,
    pub span: Span,
}

/// a function declaration
#[derive(Clone, PartialEq, Debug)]
pub struct FnDecl {
    pub attrs: Vec<Attr>,
    pub name: Ident,
    pub params: Vec<Param>,
    /// the return type and its attributes
    pub ret: Option<(Vec<Attr>, Expr)>,
    pub body: Block,
}

/// a struct member
#[derive(Clone, PartialEq, Debug)]
pub struct Member {
    pub attrs: Vec<Attr>,
    pub name: Ident,
    pub ty: Expr,
    pub span: Span,
}

/// var, let, const and override declarations
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VarKind {
    Var,
    Let,
    Const,
    Override,
}

/// a var / let / const / override declaration
#[derive(Clone, PartialEq, Debug)]
pub struct VarDecl {
    pub kind: VarKind,
    pub attrs: Vec<Attr>,
    /// var<...>: address space and access mode as written
    pub templ: Option<Vec<Expr>>,
    pub name: Ident,
    pub ty: Option<Expr>,
    pub init: Option<Expr>,
    pub span: Span,
}

/// the module-scope declarations
#[derive(Clone, PartialEq, Debug)]
pub enum DeclKind {
    Fn(FnDecl),
    Var(VarDecl),
    Alias { name: Ident, ty: Expr },
    Struct { name: Ident, members: Vec<Member> },
    ConstAssert(Expr),
}

/// a module-scope declaration with its range
#[derive(Clone, PartialEq, Debug)]
pub struct Decl {
    pub kind: DeclKind,
    pub span: Span,
}

impl Decl {
    /// the declared name (None for const_assert)
    pub fn name(&self) -> Option<&Ident> {
        match &self.kind {
            DeclKind::Fn(f) => Some(&f.name),
            DeclKind::Var(v) => Some(&v.name),
            DeclKind::Alias { name, .. } | DeclKind::Struct { name, .. } => Some(name),
            DeclKind::ConstAssert(_) => None,
        }
    }
}

/// a compound statement (attributes, statements)
#[derive(Clone, PartialEq, Debug)]
pub struct Block {
    pub attrs: Vec<Attr>,
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

/// a case selector
#[derive(Clone, PartialEq, Debug)]
pub enum CaseSel {
    Default(Span),
    Expr(Expr),
}

/// a switch clause
#[derive(Clone, PartialEq, Debug)]
pub struct Clause {
    pub sels: Vec<CaseSel>,
    /// `default:` alone (no `case`)
    pub default_alone: bool,
    pub body: Block,
    pub span: Span,
}

/// an else-if clause
#[derive(Clone, PartialEq, Debug)]
pub struct ElseIf {
    pub cond: Expr,
    pub body: Block,
    pub span: Span,
}

/// the continuing block of a loop, with its optional break-if
#[derive(Clone, PartialEq, Debug)]
pub struct Continuing {
    pub body: Block,
    pub break_if: Option<Expr>,
    pub span: Span,
}

/// = and the compound assignment operators
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shr,
    Shl,
}

impl AssignOp {
    /// the spelling
    pub fn text(self) -> &'static str {
        match self {
            AssignOp::Assign => "=",
            AssignOp::Add => "+=",
            AssignOp::Sub => "-=",
            AssignOp::Mul => "*=",
            AssignOp::Div => "/=",
            AssignOp::Rem => "%=",
            AssignOp::And => "&=",
            AssignOp::Or => "|=",
            AssignOp::Xor => "^=",
            AssignOp::Shr => ">>=",
            AssignOp::Shl => "<<=",
        }
    }
    /// the binary operator of a compound assignment
    pub fn binary(self) -> Option<BinOp> {
        Some(match self {
            AssignOp::Assign => return None,
            AssignOp::Add => BinOp::Add,
            AssignOp::Sub => BinOp::Sub,
            AssignOp::Mul => BinOp::Mul,
            AssignOp::Div => BinOp::Div,
            AssignOp::Rem => BinOp::Rem,
            AssignOp::And => BinOp::And,
            AssignOp::Or => BinOp::Or,
            AssignOp::Xor => BinOp::Xor,
            AssignOp::Shr => BinOp::Shr,
            AssignOp::Shl => BinOp::Shl,
        })
    }
}

/// the statements
#[derive(Clone, PartialEq, Debug)]
pub enum StmtKind {
    /// `;`
    Empty,
    Block(Block),
    Return(Option<Expr>),
    /// if / else if / else (the else-if clauses in order)
    If { attrs: Vec<Attr>, cond: Expr, then: Block, else_ifs: Vec<ElseIf>, else_: Option<Block> },
    Switch { attrs: Vec<Attr>, selector: Expr, body_attrs: Vec<Attr>, clauses: Vec<Clause>, body_span: Span },
    Loop { attrs: Vec<Attr>, body: Block, continuing: Option<Continuing> },
    For { attrs: Vec<Attr>, init: Option<Box<Stmt>>, cond: Option<Expr>, update: Option<Box<Stmt>>, body: Block },
    While { attrs: Vec<Attr>, cond: Expr, body: Block },
    Break,
    Continue,
    Discard,
    /// a function call statement (the expression is a Call)
    Call(Expr),
    Decl(VarDecl),
    Assign { lhs: Expr, op: AssignOp, rhs: Expr },
    /// `_ = e`
    Phony(Expr),
    Incr(Expr),
    Decr(Expr),
    ConstAssert(Expr),
}

/// a statement with its range
#[derive(Clone, PartialEq, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

/// unary operators
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
    Deref,
    AddrOf,
}

impl UnOp {
    /// the spelling
    pub fn text(self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Not => "!",
            UnOp::BitNot => "~",
            UnOp::Deref => "*",
            UnOp::AddrOf => "&",
        }
    }
}

/// binary operators
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    LogAnd,
    LogOr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl BinOp {
    /// the spelling
    pub fn text(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::And => "&",
            BinOp::Or => "|",
            BinOp::Xor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::LogAnd => "&&",
            BinOp::LogOr => "||",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
        }
    }
    /// == != < <= > >=
    pub fn is_comparison(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
    }
}

/// the expressions
#[derive(Clone, PartialEq, Debug)]
pub enum ExprKind {
    Bool(bool),
    /// text, value, suffix
    Int(String, i64, Suffix),
    Float(String, f64, Suffix),
    /// a name with an optional template list (also a type)
    Ident(Ident, Option<Vec<Expr>>),
    /// callee (a function, a type or a type-generator), template args, arguments
    Call(Ident, Option<Vec<Expr>>, Vec<Expr>),
    Paren(Box<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Index(Box<Expr>, Box<Expr>),
    /// `.name` (a member or a swizzle)
    Member(Box<Expr>, Ident),
    /// what the parser put where an expression failed
    Error,
}

/// an expression (a type is one too) with its range and height
#[derive(Clone, PartialEq, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    /// the height of the tree (a leaf is 1); every pass over it recurses this deep
    pub depth: u32,
}

impl Expr {
    /// the placeholder of a failed expression
    pub fn error(span: Span) -> Expr {
        Expr { kind: ExprKind::Error, span, depth: 1 }
    }
    /// a node with its height computed from the children
    pub fn new(kind: ExprKind, span: Span) -> Expr {
        let d = |v: &[Expr]| v.iter().map(|e| e.depth).max().unwrap_or(0);
        let depth = 1 + match &kind {
            ExprKind::Bool(_) | ExprKind::Int(..) | ExprKind::Float(..) | ExprKind::Error => 0,
            ExprKind::Ident(_, t) => t.as_deref().map(d).unwrap_or(0),
            ExprKind::Call(_, t, a) => t.as_deref().map(d).unwrap_or(0).max(d(a)),
            ExprKind::Paren(x) | ExprKind::Unary(_, x) | ExprKind::Member(x, _) => x.depth,
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => a.depth.max(b.depth),
        };
        Expr { kind, span, depth }
    }
}
