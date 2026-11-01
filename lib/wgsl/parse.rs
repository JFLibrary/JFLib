// parse -- the WGSL parser: recursive descent over the tokens of lex.rs (the grammar of spec
// section 18), with error recovery
//
//   parse(src) -> (Ast, Vec<Diagnostic>)   the tree of everything that parsed and the errors
//                                          of lexer and parser (the first error of a
//                                          statement or declaration; at most MAX_ERRORS)
//   Expressions follow the grammar exactly, including what it forbids: `&`, `|` and `^` take
//   unary operands and do not mix with each other or with other operators; a shift takes
//   unary operands; comparisons do not chain; `&&` and `||` do not mix.  Each of these gets
//   its own message ("`&` und `|` dürfen nicht ohne Klammern gemischt werden").
//   Tokens are split where the grammar needs the shorter one: `a--b` is `a - (-b)`, `&&x` is
//   `&(&x)` (spec 3.1: the longest token valid in the parser state).
//   Recovery: an error ends the statement; the parser skips to the next `;` or `}` of the
//   same brace level (inside a function) or to the next declaration keyword at brace level 0.
//   Stack: nesting deeper than MAX_DEPTH (blocks, statements, parentheses, template lists,
//   unary operators, and the links of operator, postfix and else-if chains -- every later
//   pass recurses over them) is an error, never a stack overflow.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::ast::*;
use crate::lex::{self, Suffix, Tok, Token};
use crate::{Diagnostic, LineMap};

/// nesting limit (see the module comment)
pub const MAX_DEPTH: u32 = 128;
/// braces inside a function (the function body counts; WGSL guarantees 127)
pub const MAX_BRACES: u32 = 127;
/// the height of an expression tree (operator chains included)
pub const MAX_EXPR_DEPTH: u32 = 256;
/// the parser stops after this many errors
pub const MAX_ERRORS: usize = 25;

struct Fail;
type PResult<T> = Result<T, Fail>;

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    diags: Vec<Diagnostic>,
    lm: LineMap,
    depth: u32,
    braces: u32,
    /// set once MAX_ERRORS is reached
    stop: bool,
}

/// see the module comment
pub fn parse(src: &str) -> (Ast, Vec<Diagnostic>) {
    let (toks, diags) = lex::tokens(src);
    let mut p = Parser { src, toks, pos: 0, diags, lm: LineMap::new(src), depth: 0, braces: 0, stop: false };
    let ast = p.unit();
    let mut diags = p.diags;
    diags.sort_by_key(|d| (d.start, d.end));
    diags.dedup_by_key(|d| d.start);
    (ast, diags)
}

fn bin_of(t: Tok) -> Option<BinOp> {
    Some(match t {
        Tok::Plus => BinOp::Add,
        Tok::Minus | Tok::MinusMinus => BinOp::Sub,
        Tok::Star => BinOp::Mul,
        Tok::Slash => BinOp::Div,
        Tok::Percent => BinOp::Rem,
        Tok::And => BinOp::And,
        Tok::Or => BinOp::Or,
        Tok::Xor => BinOp::Xor,
        Tok::Shl => BinOp::Shl,
        Tok::Shr => BinOp::Shr,
        Tok::AndAnd => BinOp::LogAnd,
        Tok::OrOr => BinOp::LogOr,
        Tok::EqEq => BinOp::Eq,
        Tok::NotEq => BinOp::Ne,
        Tok::Lt => BinOp::Lt,
        Tok::Le => BinOp::Le,
        Tok::Gt => BinOp::Gt,
        Tok::Ge => BinOp::Ge,
        _ => return None,
    })
}

fn assign_of(t: Tok) -> Option<AssignOp> {
    Some(match t {
        Tok::Eq => AssignOp::Assign,
        Tok::PlusEq => AssignOp::Add,
        Tok::MinusEq => AssignOp::Sub,
        Tok::StarEq => AssignOp::Mul,
        Tok::SlashEq => AssignOp::Div,
        Tok::PercentEq => AssignOp::Rem,
        Tok::AndEq => AssignOp::And,
        Tok::OrEq => AssignOp::Or,
        Tok::XorEq => AssignOp::Xor,
        Tok::ShrEq => AssignOp::Shr,
        Tok::ShlEq => AssignOp::Shl,
        _ => return None,
    })
}

/// the operator group of the grammar (for the "requires parentheses" messages)
fn group(op: BinOp) -> &'static str {
    match op {
        BinOp::Mul | BinOp::Div | BinOp::Rem | BinOp::Add | BinOp::Sub => "arith",
        BinOp::Shl | BinOp::Shr => "shift",
        BinOp::And | BinOp::Or | BinOp::Xor => "bit",
        BinOp::LogAnd | BinOp::LogOr => "logic",
        _ => "cmp",
    }
}

impl<'a> Parser<'a> {
    // ---- tokens

    fn tok(&self) -> Token {
        self.toks[self.pos.min(self.toks.len() - 1)]
    }
    fn kind(&self) -> Tok {
        self.tok().kind
    }
    fn peek(&self, k: usize) -> Tok {
        self.toks[(self.pos + k).min(self.toks.len() - 1)].kind
    }
    fn next(&mut self) -> Token {
        let t = self.tok();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn eat(&mut self, k: Tok) -> bool {
        if self.kind() == k {
            self.next();
            true
        } else {
            false
        }
    }
    /// the first character of a two-character token as a token of its own (`--` -> `-`)
    fn split(&mut self, first: Tok, rest: Tok) -> Token {
        let t = self.tok();
        self.toks[self.pos] = Token { kind: rest, start: t.start + 1, end: t.end };
        Token { kind: first, start: t.start, end: t.start + 1 }
    }
    fn text(&self, t: Token) -> &'a str {
        self.src.get(t.start as usize..t.end as usize).unwrap_or("")
    }
    fn span_from(&self, start: u32) -> Span {
        let prev = if self.pos > 0 { self.toks[self.pos - 1].end } else { start };
        Span::new(start, prev.max(start))
    }
    fn describe(&self, t: Token) -> String {
        match t.kind {
            Tok::Eof => String::from("das Ende des Texts"),
            Tok::Ident | Tok::Int | Tok::Float | Tok::Reserved | Tok::Error => format!("`{}`", self.text(t)),
            k => format!("`{}`", k.text()),
        }
    }

    // ---- errors

    fn error_at(&mut self, start: u32, end: u32, msg: String) {
        if self.diags.iter().filter(|d| d.error).count() >= MAX_ERRORS {
            if !self.stop {
                self.stop = true;
                let d = self.lm.diag(self.src, start, end, true, String::from("zu viele Fehler, der Rest wird nicht mehr geprüft"));
                self.diags.push(d);
            }
            return;
        }
        let d = self.lm.diag(self.src, start, end, true, msg);
        self.diags.push(d);
    }
    fn error(&mut self, msg: String) -> Fail {
        let t = self.tok();
        // the lexer already reported a bad token
        if t.kind != Tok::Error {
            self.error_at(t.start, t.end, msg);
        }
        Fail
    }
    fn expect(&mut self, k: Tok, ctx: &str) -> PResult<Token> {
        if self.kind() == k {
            return Ok(self.next());
        }
        let found = self.describe(self.tok());
        Err(self.error(format!("erwartet `{}` {}, gefunden {}", k.text(), ctx, found)))
    }
    fn ident(&mut self, what: &str) -> PResult<Ident> {
        let t = self.tok();
        match t.kind {
            Tok::Ident | Tok::Reserved => {
                self.next();
                Ok(Ident { name: self.text(t).to_string(), span: Span::new(t.start, t.end) })
            }
            k if k.is_keyword() => Err(self.error(format!("`{}` ist ein Schlüsselwort und kann nicht {} sein", k.text(), what))),
            _ => {
                let found = self.describe(t);
                Err(self.error(format!("erwartet {}, gefunden {}", what, found)))
            }
        }
    }
    fn enter(&mut self) -> PResult<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            return Err(self.error(format!("zu tief verschachtelt (höchstens {} Ebenen)", MAX_DEPTH)));
        }
        Ok(())
    }
    fn leave(&mut self) {
        self.depth -= 1;
    }
    /// links of a chain are bounded by the height of the tree they build
    fn link(&mut self, _links: u32) -> PResult<()> {
        Ok(())
    }
    /// an expression node; Err when the tree gets higher than MAX_EXPR_DEPTH
    fn mk(&mut self, kind: ExprKind, span: Span) -> PResult<Expr> {
        let e = Expr::new(kind, span);
        if e.depth > MAX_EXPR_DEPTH {
            let msg = format!("der Ausdruck ist zu tief verschachtelt oder zu lang verkettet (höchstens {} Ebenen) – teile ihn auf", MAX_EXPR_DEPTH);
            self.error_at(span.start, span.start + 1, msg);
            return Err(Fail);
        }
        Ok(e)
    }

    /// skip to the end of the broken statement: past the next `;` or before the `}` of this
    /// brace level
    fn recover_stmt(&mut self) {
        let mut depth = 0i32;
        loop {
            match self.kind() {
                Tok::Eof => return,
                Tok::Semi if depth == 0 => {
                    self.next();
                    return;
                }
                Tok::LBrace => depth += 1,
                Tok::RBrace => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    if depth == 0 && self.peek(1) != Tok::Else {
                        self.next();
                        return;
                    }
                }
                _ => {}
            }
            self.next();
        }
    }
    /// skip to the next module-scope declaration
    fn recover_decl(&mut self) {
        let mut depth = 0i32;
        let start = self.pos;
        loop {
            let k = self.kind();
            if k == Tok::Eof {
                return;
            }
            if depth <= 0
                && self.pos > start
                && matches!(k, Tok::Fn | Tok::Var | Tok::Const | Tok::Override | Tok::Alias | Tok::Struct | Tok::ConstAssert | Tok::At)
            {
                return;
            }
            match k {
                Tok::LBrace => depth += 1,
                Tok::RBrace => depth -= 1,
                _ => {}
            }
            self.next();
        }
    }

    // ---- module

    fn unit(&mut self) -> Ast {
        let mut ast = Ast::default();
        loop {
            if self.stop {
                break;
            }
            match self.kind() {
                Tok::Enable | Tok::Requires => match self.directive() {
                    Ok(d) => ast.directives.push(d),
                    Err(_) => self.recover_decl(),
                },
                Tok::Diagnostic if self.peek(1) == Tok::LParen => match self.directive() {
                    Ok(d) => ast.directives.push(d),
                    Err(_) => self.recover_decl(),
                },
                _ => break,
            }
        }
        while !self.stop && self.kind() != Tok::Eof {
            if self.eat(Tok::Semi) {
                continue;
            }
            if matches!(self.kind(), Tok::Enable | Tok::Requires) || (self.kind() == Tok::Diagnostic && self.peek(1) == Tok::LParen) {
                let t = self.tok();
                self.error_at(t.start, t.end, format!("die Direktive `{}` muss vor allen Deklarationen stehen", self.text(t)));
                let _ = self.directive();
                continue;
            }
            let before = self.pos;
            match self.global_decl() {
                Ok(d) => ast.decls.push(d),
                Err(_) => {
                    self.recover_decl();
                    if self.pos == before {
                        self.next();
                    }
                }
            }
        }
        ast
    }

    fn name_list(&mut self, what: &str) -> PResult<Vec<Ident>> {
        let mut v = Vec::new();
        loop {
            v.push(self.ident(what)?);
            if !self.eat(Tok::Comma) || self.kind() == Tok::Semi {
                break;
            }
        }
        Ok(v)
    }

    fn directive(&mut self) -> PResult<Directive> {
        let start = self.tok().start;
        let kind = match self.next().kind {
            Tok::Enable => DirectiveKind::Enable(self.name_list("den Namen einer Erweiterung")?),
            Tok::Requires => DirectiveKind::Requires(self.name_list("den Namen einer Spracherweiterung")?),
            _ => DirectiveKind::Diagnostic(self.diagnostic_control()?),
        };
        self.expect(Tok::Semi, "nach der Direktive")?;
        Ok(Directive { kind, span: self.span_from(start) })
    }

    fn diagnostic_control(&mut self) -> PResult<DiagnosticControl> {
        self.expect(Tok::LParen, "nach `diagnostic`")?;
        let severity = self.ident("einen Schweregrad (error, warning, info, off)")?;
        self.expect(Tok::Comma, "nach dem Schweregrad")?;
        let rule = self.ident("den Namen einer Diagnose-Regel")?;
        let rule2 = if self.eat(Tok::Dot) { Some(self.ident("den zweiten Teil des Regelnamens")?) } else { None };
        self.eat(Tok::Comma);
        self.expect(Tok::RParen, "nach der Diagnose-Regel")?;
        Ok(DiagnosticControl { severity, rule, rule2 })
    }

    fn attrs(&mut self) -> PResult<Vec<Attr>> {
        let mut v = Vec::new();
        while self.kind() == Tok::At {
            v.push(self.attr()?);
        }
        Ok(v)
    }

    fn attr(&mut self) -> PResult<Attr> {
        let start = self.next().start;
        let t = self.tok();
        let name = match t.kind {
            Tok::Ident | Tok::Reserved | Tok::Const | Tok::Diagnostic => {
                self.next();
                Ident { name: self.text(t).to_string(), span: Span::new(t.start, t.end) }
            }
            _ => {
                let found = self.describe(t);
                return Err(self.error(format!("erwartet den Namen eines Attributs nach `@`, gefunden {}", found)));
            }
        };
        if t.kind == Tok::Diagnostic {
            let c = self.diagnostic_control()?;
            return Ok(Attr { name, kind: AttrKind::Diagnostic(c), span: self.span_from(start) });
        }
        let mut args = Vec::new();
        let parens = self.kind() == Tok::LParen;
        if parens {
            self.next();
            while self.kind() != Tok::RParen {
                args.push(self.expression()?);
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
            self.expect(Tok::RParen, "nach den Argumenten des Attributs")?;
        }
        Ok(Attr { name, kind: AttrKind::Plain { args, parens }, span: self.span_from(start) })
    }

    fn global_decl(&mut self) -> PResult<Decl> {
        let start = self.tok().start;
        let attrs = self.attrs()?;
        let kind = match self.kind() {
            Tok::Fn => DeclKind::Fn(self.function(attrs)?),
            Tok::Var | Tok::Const | Tok::Override | Tok::Let => {
                let v = self.var_decl(attrs, true)?;
                self.expect(Tok::Semi, "nach der Deklaration")?;
                DeclKind::Var(v)
            }
            Tok::Alias => {
                self.no_attrs(&attrs, "`alias`");
                self.next();
                let name = self.ident("den Namen des Alias")?;
                self.expect(Tok::Eq, "nach dem Namen des Alias")?;
                let ty = self.type_spec()?;
                self.expect(Tok::Semi, "nach dem Alias")?;
                DeclKind::Alias { name, ty }
            }
            Tok::Struct => {
                self.no_attrs(&attrs, "`struct`");
                self.next();
                let name = self.ident("den Namen der Struktur")?;
                self.expect(Tok::LBrace, "nach dem Namen der Struktur")?;
                let mut members = Vec::new();
                while self.kind() != Tok::RBrace {
                    let ms = self.tok().start;
                    let attrs = self.attrs()?;
                    let mname = self.ident("den Namen eines Members")?;
                    self.expect(Tok::Colon, "nach dem Namen des Members")?;
                    let ty = self.type_spec()?;
                    members.push(Member { attrs, name: mname, ty, span: self.span_from(ms) });
                    if !self.eat(Tok::Comma) {
                        break;
                    }
                }
                self.expect(Tok::RBrace, "am Ende der Struktur (Member werden mit `,` getrennt)")?;
                self.eat(Tok::Semi);
                DeclKind::Struct { name, members }
            }
            Tok::ConstAssert => {
                self.no_attrs(&attrs, "`const_assert`");
                self.next();
                let e = self.expression()?;
                self.expect(Tok::Semi, "nach `const_assert`")?;
                DeclKind::ConstAssert(e)
            }
            _ => {
                let found = self.describe(self.tok());
                return Err(self.error(format!("erwartet eine Deklaration (fn, var, const, override, struct, alias, const_assert), gefunden {}", found)));
            }
        };
        Ok(Decl { kind, span: self.span_from(start) })
    }

    fn no_attrs(&mut self, attrs: &[Attr], what: &str) {
        if let Some(a) = attrs.first() {
            self.error_at(a.span.start, a.span.end, format!("vor {} sind keine Attribute erlaubt", what));
        }
    }

    fn function(&mut self, attrs: Vec<Attr>) -> PResult<FnDecl> {
        self.next();
        let name = self.ident("den Namen der Funktion")?;
        self.expect(Tok::LParen, "nach dem Namen der Funktion")?;
        let mut params = Vec::new();
        while self.kind() != Tok::RParen {
            let ps = self.tok().start;
            let pattrs = self.attrs()?;
            let pname = self.ident("den Namen eines Parameters")?;
            self.expect(Tok::Colon, "nach dem Namen des Parameters")?;
            let ty = self.type_spec()?;
            params.push(Param { attrs: pattrs, name: pname, ty, span: self.span_from(ps) });
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen, "nach den Parametern")?;
        let ret = if self.eat(Tok::Arrow) {
            let rattrs = self.attrs()?;
            Some((rattrs, self.type_spec()?))
        } else {
            None
        };
        if self.kind() != Tok::LBrace && self.kind() != Tok::At {
            let found = self.describe(self.tok());
            return Err(self.error(format!("erwartet `{{` für den Rumpf der Funktion, gefunden {}", found)));
        }
        let body = self.block()?;
        Ok(FnDecl { attrs, name, params, ret, body })
    }

    /// var / let / const / override (global: whether at module scope)
    fn var_decl(&mut self, attrs: Vec<Attr>, global: bool) -> PResult<VarDecl> {
        let start = attrs.first().map(|a| a.span.start).unwrap_or(self.tok().start);
        let t = self.next();
        let kind = match t.kind {
            Tok::Var => VarKind::Var,
            Tok::Let => VarKind::Let,
            Tok::Const => VarKind::Const,
            _ => VarKind::Override,
        };
        if kind == VarKind::Override && !global {
            self.error_at(t.start, t.end, String::from("`override` ist nur auf Modulebene erlaubt"));
        }
        let templ = if kind == VarKind::Var && self.kind() == Tok::TemplateStart { Some(self.template_list()?) } else { None };
        let name = self.ident("den Namen der Deklaration")?;
        let ty = if self.eat(Tok::Colon) { Some(self.type_spec()?) } else { None };
        let init = if self.eat(Tok::Eq) {
            Some(self.expression()?)
        } else {
            if matches!(kind, VarKind::Let | VarKind::Const) {
                let found = self.describe(self.tok());
                return Err(self.error(format!("erwartet `=` mit einem Initialisierer für `{}`, gefunden {}", t.kind.text(), found)));
            }
            None
        };
        Ok(VarDecl { kind, attrs, templ, name, ty, init, span: self.span_from(start) })
    }

    fn type_spec(&mut self) -> PResult<Expr> {
        let id = self.ident("einen Typ")?;
        let start = id.span.start;
        let args = if self.kind() == Tok::TemplateStart { Some(self.template_list()?) } else { None };
        Ok(self.mk(ExprKind::Ident(id, args), self.span_from(start))?)
    }

    fn template_list(&mut self) -> PResult<Vec<Expr>> {
        self.next();
        self.enter()?;
        let mut v = Vec::new();
        loop {
            if self.kind() == Tok::TemplateEnd && !v.is_empty() {
                break;
            }
            let e = self.expression();
            match e {
                Ok(e) => v.push(e),
                Err(f) => {
                    self.leave();
                    return Err(f);
                }
            }
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.leave();
        if self.kind() != Tok::TemplateEnd {
            let found = self.describe(self.tok());
            return Err(self.error(format!("erwartet `>` am Ende der Template-Liste, gefunden {}", found)));
        }
        self.next();
        Ok(v)
    }

    // ---- statements

    fn block(&mut self) -> PResult<Block> {
        let start = self.tok().start;
        let attrs = self.attrs()?;
        self.block_with(attrs, start)
    }

    fn block_with(&mut self, attrs: Vec<Attr>, start: u32) -> PResult<Block> {
        self.expect(Tok::LBrace, "am Anfang des Blocks")?;
        self.braces += 1;
        if self.braces > MAX_BRACES {
            self.braces -= 1;
            let prev = self.toks[self.pos.saturating_sub(1)];
            self.error_at(prev.start, prev.end, format!("zu viele verschachtelte Blöcke (höchstens {})", MAX_BRACES));
            return Err(Fail);
        }
        let entered = self.enter();
        if entered.is_err() {
            self.braces -= 1;
            return Err(Fail);
        }
        let mut stmts = Vec::new();
        let r = self.stmts_until_brace(&mut stmts, false);
        self.leave();
        self.braces -= 1;
        r?;
        self.expect(Tok::RBrace, "am Ende des Blocks")?;
        Ok(Block { attrs, stmts, span: self.span_from(start) })
    }

    /// statements up to the closing brace (not consumed); in_loop: stop before `continuing`
    fn stmts_until_brace(&mut self, out: &mut Vec<Stmt>, in_loop: bool) -> PResult<()> {
        loop {
            if self.stop {
                return Err(Fail);
            }
            match self.kind() {
                Tok::RBrace => return Ok(()),
                Tok::Eof => return Err(self.error(String::from("erwartet `}`, der Block endet nicht"))),
                Tok::Continuing if in_loop => return Ok(()),
                _ => {}
            }
            let before = self.pos;
            match self.statement() {
                Ok(s) => out.push(s),
                Err(_) => {
                    self.recover_stmt();
                    if self.pos == before {
                        self.next();
                    }
                }
            }
        }
    }

    fn statement(&mut self) -> PResult<Stmt> {
        let start = self.tok().start;
        let attrs = self.attrs()?;
        let kind = match self.kind() {
            Tok::LBrace => StmtKind::Block(self.block_with(attrs, start)?),
            Tok::If => self.if_stmt(attrs)?,
            Tok::Switch => self.switch_stmt(attrs)?,
            Tok::Loop => self.loop_stmt(attrs)?,
            Tok::For => self.for_stmt(attrs)?,
            Tok::While => {
                self.next();
                let cond = self.expression()?;
                let body = self.block()?;
                StmtKind::While { attrs, cond, body }
            }
            _ => {
                if let Some(a) = attrs.first() {
                    self.error_at(a.span.start, a.span.end, String::from("Attribute sind nur vor Blöcken, if, switch, loop, for und while erlaubt"));
                }
                self.simple_stmt(true)?
            }
        };
        Ok(Stmt { kind, span: self.span_from(start) })
    }

    /// the statements that end with `;` (semi: expect it here)
    fn simple_stmt(&mut self, semi: bool) -> PResult<StmtKind> {
        let t = self.tok();
        let kind = match t.kind {
            Tok::Semi => {
                if semi {
                    self.next();
                }
                return Ok(StmtKind::Empty);
            }
            Tok::Return => {
                self.next();
                if self.kind() == Tok::Semi { StmtKind::Return(None) } else { StmtKind::Return(Some(self.expression()?)) }
            }
            Tok::Break => {
                self.next();
                if self.kind() == Tok::If {
                    let end = self.tok().end;
                    self.error_at(t.start, end, String::from("`break if` ist nur als letzte Anweisung eines `continuing`-Blocks erlaubt"));
                    return Err(Fail);
                }
                StmtKind::Break
            }
            Tok::Continue => {
                self.next();
                StmtKind::Continue
            }
            Tok::Discard => {
                self.next();
                StmtKind::Discard
            }
            Tok::ConstAssert => {
                self.next();
                StmtKind::ConstAssert(self.expression()?)
            }
            Tok::Var | Tok::Let | Tok::Const | Tok::Override => StmtKind::Decl(self.var_decl(Vec::new(), false)?),
            Tok::Underscore => {
                self.next();
                self.expect(Tok::Eq, "nach `_`")?;
                StmtKind::Phony(self.expression()?)
            }
            _ => self.updating_or_call()?,
        };
        if semi {
            self.expect(Tok::Semi, "nach der Anweisung")?;
        }
        Ok(kind)
    }

    /// an assignment, ++ / --, or a function call statement
    fn updating_or_call(&mut self) -> PResult<StmtKind> {
        let lhs = self.lhs_expr()?;
        if let ExprKind::Call(..) = lhs.kind {
            if !matches!(self.kind(), Tok::PlusPlus | Tok::MinusMinus) && assign_of(self.kind()).is_none() {
                return Ok(StmtKind::Call(lhs));
            }
        }
        let t = self.tok();
        if let Some(op) = assign_of(t.kind) {
            self.next();
            let rhs = self.expression()?;
            return Ok(StmtKind::Assign { lhs, op, rhs });
        }
        match t.kind {
            Tok::PlusPlus => {
                self.next();
                Ok(StmtKind::Incr(lhs))
            }
            Tok::MinusMinus => {
                self.next();
                Ok(StmtKind::Decr(lhs))
            }
            _ => {
                let found = self.describe(t);
                if bin_of(t.kind).is_some() || t.kind == Tok::TemplateEnd || t.kind == Tok::TemplateStart {
                    Err(self.error(format!("ein Ausdruck allein ist keine Anweisung (erwartet `=`, `+=`, `++` … oder einen Funktionsaufruf), gefunden {}", found)))
                } else {
                    Err(self.error(format!("erwartet `=` oder einen anderen Zuweisungsoperator nach der linken Seite, gefunden {}", found)))
                }
            }
        }
    }

    /// lhs_expression: (`*` | `&`)* (ident [call] | `(` lhs `)`) postfix*
    fn lhs_expr(&mut self) -> PResult<Expr> {
        self.enter()?;
        let r = self.lhs_inner();
        self.leave();
        r
    }

    fn lhs_inner(&mut self) -> PResult<Expr> {
        let t = self.tok();
        let op = match t.kind {
            Tok::Star => Some(UnOp::Deref),
            Tok::And => Some(UnOp::AddrOf),
            Tok::AndAnd => {
                self.split(Tok::And, Tok::And);
                Some(UnOp::AddrOf)
            }
            _ => None,
        };
        if let Some(op) = op {
            if t.kind != Tok::AndAnd {
                self.next();
            }
            let e = self.lhs_expr()?;
            let span = Span::new(t.start, e.span.end);
            return Ok(self.mk(ExprKind::Unary(op, Box::new(e)), span)?);
        }
        let core = match t.kind {
            Tok::LParen => {
                self.next();
                let e = self.lhs_expr()?;
                self.expect(Tok::RParen, "nach dem geklammerten Ausdruck")?;
                self.mk(ExprKind::Paren(Box::new(e)), self.span_from(t.start))?
            }
            Tok::Ident | Tok::Reserved => {
                let id = self.ident("einen Namen")?;
                let targs = if self.kind() == Tok::TemplateStart { Some(self.template_list()?) } else { None };
                if self.kind() == Tok::LParen {
                    let args = self.call_args()?;
                    self.mk(ExprKind::Call(id, targs, args), self.span_from(t.start))?
                } else {
                    self.mk(ExprKind::Ident(id, targs), self.span_from(t.start))?
                }
            }
            _ => {
                let found = self.describe(t);
                return Err(self.error(format!("erwartet eine Anweisung, gefunden {}", found)));
            }
        };
        self.postfix(core)
    }

    fn call_args(&mut self) -> PResult<Vec<Expr>> {
        self.expect(Tok::LParen, "")?;
        self.enter()?;
        let mut args = Vec::new();
        let r: PResult<()> = (|| {
            while self.kind() != Tok::RParen {
                args.push(self.expression()?);
                if !self.eat(Tok::Comma) {
                    break;
                }
            }
            Ok(())
        })();
        self.leave();
        r?;
        self.expect(Tok::RParen, "am Ende der Argumentliste")?;
        Ok(args)
    }

    fn if_stmt(&mut self, attrs: Vec<Attr>) -> PResult<StmtKind> {
        self.next();
        let cond = self.expression()?;
        let then = self.block()?;
        let mut else_ifs = Vec::new();
        let mut else_ = None;
        let mut links = 0;
        let r: PResult<()> = (|| {
            while self.kind() == Tok::Else {
                let es = self.next().start;
                if self.eat(Tok::If) {
                    self.enter()?;
                    links += 1;
                    let c = self.expression()?;
                    let b = self.block()?;
                    else_ifs.push(ElseIf { cond: c, body: b, span: self.span_from(es) });
                } else {
                    else_ = Some(self.block()?);
                    break;
                }
            }
            Ok(())
        })();
        self.depth -= links;
        r?;
        Ok(StmtKind::If { attrs, cond, then, else_ifs, else_ })
    }

    fn switch_stmt(&mut self, attrs: Vec<Attr>) -> PResult<StmtKind> {
        self.next();
        let selector = self.expression()?;
        let bs = self.tok().start;
        let body_attrs = self.attrs()?;
        self.expect(Tok::LBrace, "nach dem Ausdruck von `switch`")?;
        self.enter()?;
        let mut clauses = Vec::new();
        let r: PResult<()> = (|| {
            while self.kind() != Tok::RBrace {
                let cs = self.tok().start;
                let (sels, alone) = match self.kind() {
                    Tok::Case => {
                        self.next();
                        let mut sels = Vec::new();
                        loop {
                            if self.kind() == Tok::Default {
                                let t = self.next();
                                sels.push(CaseSel::Default(Span::new(t.start, t.end)));
                            } else {
                                sels.push(CaseSel::Expr(self.expression()?));
                            }
                            if !self.eat(Tok::Comma) || matches!(self.kind(), Tok::Colon | Tok::LBrace | Tok::At) {
                                break;
                            }
                        }
                        (sels, false)
                    }
                    Tok::Default => {
                        let t = self.next();
                        (alloc::vec![CaseSel::Default(Span::new(t.start, t.end))], true)
                    }
                    _ => {
                        let found = self.describe(self.tok());
                        return Err(self.error(format!("erwartet `case` oder `default` im `switch`, gefunden {}", found)));
                    }
                };
                self.eat(Tok::Colon);
                let body = self.block()?;
                clauses.push(Clause { sels, default_alone: alone, body, span: self.span_from(cs) });
            }
            Ok(())
        })();
        self.leave();
        r?;
        self.expect(Tok::RBrace, "am Ende von `switch`")?;
        let body_span = self.span_from(bs);
        if clauses.is_empty() {
            self.error_at(body_span.start, body_span.end, String::from("`switch` braucht mindestens einen `case`- oder `default`-Zweig"));
        }
        Ok(StmtKind::Switch { attrs, selector, body_attrs, clauses, body_span })
    }

    fn loop_stmt(&mut self, attrs: Vec<Attr>) -> PResult<StmtKind> {
        self.next();
        let bs = self.tok().start;
        let body_attrs = self.attrs()?;
        self.expect(Tok::LBrace, "nach `loop`")?;
        self.enter()?;
        let mut stmts = Vec::new();
        let mut continuing = None;
        let r: PResult<()> = (|| {
            self.stmts_until_brace(&mut stmts, true)?;
            if self.kind() == Tok::Continuing {
                let cs = self.next().start;
                let cbs = self.tok().start;
                let cattrs = self.attrs()?;
                self.expect(Tok::LBrace, "nach `continuing`")?;
                self.enter()?;
                let mut cstmts = Vec::new();
                let mut break_if = None;
                let r: PResult<()> = (|| {
                    loop {
                        if self.kind() == Tok::Break && self.peek(1) == Tok::If {
                            let kw = self.tok();
                            self.next();
                            self.next();
                            break_if = Some(self.expression()?);
                            self.expect(Tok::Semi, "nach `break if`")?;
                            if self.kind() != Tok::RBrace {
                                self.error_at(kw.start, kw.end, String::from("`break if` muss die letzte Anweisung von `continuing` sein"));
                                return Err(Fail);
                            }
                            break;
                        }
                        match self.kind() {
                            Tok::RBrace => break,
                            Tok::Eof => return Err(self.error(String::from("erwartet `}`, `continuing` endet nicht"))),
                            _ => {}
                        }
                        let before = self.pos;
                        match self.statement() {
                            Ok(s) => cstmts.push(s),
                            Err(_) => {
                                self.recover_stmt();
                                if self.pos == before {
                                    self.next();
                                }
                            }
                        }
                    }
                    Ok(())
                })();
                self.leave();
                r?;
                self.expect(Tok::RBrace, "am Ende von `continuing`")?;
                let body = Block { attrs: cattrs, stmts: cstmts, span: self.span_from(cbs) };
                continuing = Some(Continuing { body, break_if, span: self.span_from(cs) });
                if self.kind() != Tok::RBrace {
                    return Err(self.error(String::from("`continuing` muss die letzte Anweisung der Schleife sein")));
                }
            }
            Ok(())
        })();
        self.leave();
        r?;
        self.expect(Tok::RBrace, "am Ende von `loop`")?;
        let body = Block { attrs: body_attrs, stmts, span: self.span_from(bs) };
        Ok(StmtKind::Loop { attrs, body, continuing })
    }

    fn for_stmt(&mut self, attrs: Vec<Attr>) -> PResult<StmtKind> {
        self.next();
        self.expect(Tok::LParen, "nach `for`")?;
        let init = if self.kind() == Tok::Semi {
            None
        } else {
            let s = self.tok().start;
            let k = match self.kind() {
                Tok::Var | Tok::Let | Tok::Const => StmtKind::Decl(self.var_decl(Vec::new(), false)?),
                Tok::Underscore => self.simple_stmt(false)?,
                _ => self.updating_or_call()?,
            };
            Some(Box::new(Stmt { kind: k, span: self.span_from(s) }))
        };
        self.expect(Tok::Semi, "nach dem Initialisierer von `for`")?;
        let cond = if self.kind() == Tok::Semi { None } else { Some(self.expression()?) };
        self.expect(Tok::Semi, "nach der Bedingung von `for`")?;
        let update = if self.kind() == Tok::RParen {
            None
        } else {
            let s = self.tok().start;
            let k = match self.kind() {
                Tok::Underscore => self.simple_stmt(false)?,
                _ => self.updating_or_call()?,
            };
            Some(Box::new(Stmt { kind: k, span: self.span_from(s) }))
        };
        self.expect(Tok::RParen, "nach dem Kopf von `for`")?;
        let body = self.block()?;
        Ok(StmtKind::For { attrs, init, cond, update, body })
    }

    // ---- expressions

    /// (the nesting is counted in unary(): one level per parenthesis)
    pub(crate) fn expression(&mut self) -> PResult<Expr> {
        let u = self.unary()?;
        self.expression_after(u)
    }

    /// at (when known): the place of the first operator
    fn mixing(&mut self, a: BinOp, b: BinOp, at: Option<u32>) -> Fail {
        let msg = if a.is_comparison() && b.is_comparison() {
            format!("Vergleiche lassen sich nicht verketten (`{}` nach `{}`) – setze Klammern", b.text(), a.text())
        } else {
            format!("`{}` und `{}` dürfen nicht ohne Klammern gemischt werden", a.text(), b.text())
        };
        match at {
            Some(s) => {
                let end = self.tok().end;
                self.error_at(s, end, msg);
                Fail
            }
            None => self.error(msg),
        }
    }

    /// after an expression: an operator that may not follow is a mixing error
    fn check_end(&mut self, last: BinOp, at: u32) -> PResult<()> {
        if let Some(op) = bin_of(self.kind()) {
            return Err(self.mixing(last, op, Some(at)));
        }
        Ok(())
    }

    fn expression_after(&mut self, u: Expr) -> PResult<Expr> {
        let k = self.kind();
        if matches!(k, Tok::And | Tok::Or | Tok::Xor) {
            let op = bin_of(k).unwrap_or(BinOp::And);
            let at = self.tok().start;
            let mut e = u;
            let mut links = 0;
            while self.kind() == k {
                links += 1;
                self.link(links)?;
                self.next();
                let r = self.unary()?;
                let span = e.span.to(r.span);
                e = self.mk(ExprKind::Binary(op, Box::new(e), Box::new(r)), span)?;
            }
            self.check_end(op, at)?;
            return Ok(e);
        }
        let mut e = self.relational_after(u)?;
        let k = self.kind();
        if matches!(k, Tok::AndAnd | Tok::OrOr) {
            let op = if k == Tok::AndAnd { BinOp::LogAnd } else { BinOp::LogOr };
            let at = self.tok().start;
            let mut links = 0;
            while self.kind() == k {
                links += 1;
                self.link(links)?;
                self.next();
                let u2 = self.unary()?;
                let r = self.relational_after(u2)?;
                let span = e.span.to(r.span);
                e = self.mk(ExprKind::Binary(op, Box::new(e), Box::new(r)), span)?;
            }
            self.check_end(op, at)?;
        } else if let ExprKind::Binary(op, ..) = e.kind {
            if matches!(self.kind(), Tok::And | Tok::Or | Tok::Xor) {
                let b = bin_of(self.kind()).unwrap_or(BinOp::And);
                return Err(self.mixing(op, b, None));
            }
        }
        Ok(e)
    }

    fn relational_after(&mut self, u: Expr) -> PResult<Expr> {
        let s = self.shift_after(u)?;
        let k = self.kind();
        let op = match k {
            Tok::Lt | Tok::Gt | Tok::Le | Tok::Ge | Tok::EqEq | Tok::NotEq => bin_of(k).unwrap_or(BinOp::Eq),
            _ => return Ok(s),
        };
        let at = self.tok().start;
        self.next();
        let u2 = self.unary()?;
        let r = self.shift_after(u2)?;
        if let Some(b) = bin_of(self.kind()) {
            if b.is_comparison() || matches!(group(b), "bit") {
                return Err(self.mixing(op, b, Some(at)));
            }
        }
        let span = s.span.to(r.span);
        Ok(self.mk(ExprKind::Binary(op, Box::new(s), Box::new(r)), span)?)
    }

    fn shift_after(&mut self, u: Expr) -> PResult<Expr> {
        let k = self.kind();
        if matches!(k, Tok::Shl | Tok::Shr) {
            let op = bin_of(k).unwrap_or(BinOp::Shl);
            let at = self.tok().start;
            self.next();
            let r = self.unary()?;
            if let Some(b) = bin_of(self.kind()) {
                if matches!(group(b), "arith" | "shift" | "bit") {
                    return Err(self.mixing(op, b, Some(at)));
                }
            }
            let span = u.span.to(r.span);
            return Ok(self.mk(ExprKind::Binary(op, Box::new(u), Box::new(r)), span)?);
        }
        let mut e = self.mult_after(u)?;
        let mut links = 0;
        let mut last = None;
        loop {
            let op = match self.kind() {
                Tok::Plus => {
                    self.next();
                    BinOp::Add
                }
                Tok::Minus => {
                    self.next();
                    BinOp::Sub
                }
                Tok::MinusMinus => {
                    // `a--b` is `a - -b`: the first `-` is the operator, the second starts b
                    self.split(Tok::Minus, Tok::Minus);
                    BinOp::Sub
                }
                _ => break,
            };
            links += 1;
            self.link(links)?;
            let u2 = self.unary()?;
            let r = self.mult_after(u2)?;
            let span = e.span.to(r.span);
            e = self.mk(ExprKind::Binary(op, Box::new(e), Box::new(r)), span)?;
            last = Some(op);
        }
        if matches!(self.kind(), Tok::Shl | Tok::Shr) {
            let b = bin_of(self.kind()).unwrap_or(BinOp::Shl);
            let a = match (&last, &e.kind) {
                (Some(op), _) => *op,
                (None, ExprKind::Binary(op, ..)) => *op,
                _ => BinOp::Add,
            };
            return Err(self.mixing(a, b, None));
        }
        Ok(e)
    }

    fn mult_after(&mut self, u: Expr) -> PResult<Expr> {
        let mut e = u;
        let mut links = 0;
        loop {
            let op = match self.kind() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Rem,
                _ => return Ok(e),
            };
            links += 1;
            self.link(links)?;
            self.next();
            let r = self.unary()?;
            let span = e.span.to(r.span);
            e = self.mk(ExprKind::Binary(op, Box::new(e), Box::new(r)), span)?;
        }
    }

    fn unary(&mut self) -> PResult<Expr> {
        self.enter()?;
        let r = self.unary_inner();
        self.leave();
        r
    }

    fn unary_inner(&mut self) -> PResult<Expr> {
        let t = self.tok();
        let op = match t.kind {
            Tok::Minus => UnOp::Neg,
            Tok::Bang => UnOp::Not,
            Tok::Tilde => UnOp::BitNot,
            Tok::Star => UnOp::Deref,
            Tok::And => UnOp::AddrOf,
            Tok::AndAnd => {
                let first = self.split(Tok::And, Tok::And);
                let e = self.unary()?;
                let span = Span::new(first.start, e.span.end);
                return Ok(self.mk(ExprKind::Unary(UnOp::AddrOf, Box::new(e)), span)?);
            }
            Tok::MinusMinus => {
                let first = self.split(Tok::Minus, Tok::Minus);
                let e = self.unary()?;
                let span = Span::new(first.start, e.span.end);
                return Ok(self.mk(ExprKind::Unary(UnOp::Neg, Box::new(e)), span)?);
            }
            _ => return self.singular(),
        };
        self.next();
        let e = self.unary()?;
        let span = Span::new(t.start, e.span.end);
        Ok(self.mk(ExprKind::Unary(op, Box::new(e)), span)?)
    }

    fn singular(&mut self) -> PResult<Expr> {
        let p = self.primary()?;
        self.postfix(p)
    }

    fn postfix(&mut self, mut e: Expr) -> PResult<Expr> {
        let mut links = 0;
        loop {
            match self.kind() {
                Tok::LBracket => {
                    links += 1;
                    self.link(links)?;
                    self.next();
                    let i = self.expression()?;
                    self.expect(Tok::RBracket, "nach dem Index")?;
                    let span = self.span_from(e.span.start);
                    e = self.mk(ExprKind::Index(Box::new(e), Box::new(i)), span)?;
                }
                Tok::Dot => {
                    links += 1;
                    self.link(links)?;
                    self.next();
                    let m = self.ident("den Namen eines Members oder einen Swizzle nach `.`")?;
                    let span = self.span_from(e.span.start);
                    e = self.mk(ExprKind::Member(Box::new(e), m), span)?;
                }
                _ => return Ok(e),
            }
        }
    }

    fn primary(&mut self) -> PResult<Expr> {
        let t = self.tok();
        let sp = Span::new(t.start, t.end);
        match t.kind {
            Tok::True | Tok::False => {
                self.next();
                Ok(self.mk(ExprKind::Bool(t.kind == Tok::True), sp)?)
            }
            Tok::Int => {
                self.next();
                let text = self.text(t);
                match lex::int_value(text) {
                    Ok((v, s)) => Ok(self.mk(ExprKind::Int(text.to_string(), v, s), sp)?),
                    Err(m) => {
                        self.error_at(t.start, t.end, m);
                        Ok(self.mk(ExprKind::Int(text.to_string(), 0, Suffix::None), sp)?)
                    }
                }
            }
            Tok::Float => {
                self.next();
                let text = self.text(t);
                match lex::float_value(text) {
                    Ok((v, s)) => Ok(self.mk(ExprKind::Float(text.to_string(), v, s), sp)?),
                    Err(m) => {
                        self.error_at(t.start, t.end, m);
                        Ok(self.mk(ExprKind::Float(text.to_string(), 0.0, Suffix::None), sp)?)
                    }
                }
            }
            Tok::LParen => {
                self.next();
                let e = self.expression()?;
                self.expect(Tok::RParen, "nach dem geklammerten Ausdruck")?;
                Ok(self.mk(ExprKind::Paren(Box::new(e)), self.span_from(t.start))?)
            }
            Tok::Ident | Tok::Reserved => {
                let id = self.ident("einen Namen")?;
                let targs = if self.kind() == Tok::TemplateStart { Some(self.template_list()?) } else { None };
                if self.kind() == Tok::LParen {
                    let args = self.call_args()?;
                    Ok(self.mk(ExprKind::Call(id, targs, args), self.span_from(t.start))?)
                } else {
                    Ok(self.mk(ExprKind::Ident(id, targs), self.span_from(t.start))?)
                }
            }
            Tok::Underscore => Err(self.error(String::from("`_` ist kein Wert; es steht nur links in `_ = Ausdruck;`"))),
            _ => {
                let found = self.describe(t);
                Err(self.error(format!("erwartet einen Ausdruck, gefunden {}", found)))
            }
        }
    }
}
