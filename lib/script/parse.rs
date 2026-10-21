// parse -- the parser of .jfs scripts: statements by recursive descent, expressions by
// precedence climbing (Pratt), with error recovery
//
//   parse_program(name, src) -> (Program, Vec<Diagnostic>)   the tree of everything that parsed
//                               and the errors of lexer and parser (the first of each line)
//   Line breaks: a statement ends at a line break once its expression is complete; `;` is
//   optional.  Inside ( ) [ ] map { } and ${ } line breaks are ignored; after an operator,
//   `=` or `,` the expression goes on.  A line that starts with `.` or a binary operator other
//   than `-` continues the line above; `-x`, `(..)` and `[..]` at the start of a line begin a
//   new statement (inside brackets they continue).  `else` and `catch` may start a line.
//   Recovery: an error ends the statement; the parser skips to the next line break outside
//   the statement's brackets (or to the `}` of its block) and goes on there.  A closing
//   bracket missing at a line end is reported and taken as given when the next line starts
//   at or left of the statement's column, or when the token closes an outer bracket.
//   Stack: nesting deeper than MAX_DEPTH (blocks, brackets, operands; the links of an operator
//   or call chain count too, since every pass over the tree recurses) is an error.  The
//   parser needs up to about 2 KB of stack per level in development builds, so the deepest
//   script fits in a 256 KB thread (coretest checks that on Linux).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::ast::*;
use crate::lex::{self, Tok, TokKind};
use crate::Diagnostic;

/// nesting limit of blocks and expressions (the links of operator and call chains count)
pub const MAX_DEPTH: u32 = 100;

const BP_EQ: u8 = 4;
const BP_CMP: u8 = 5;

struct Fail;
type PResult<T> = Result<T, Fail>;

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Tok>,
    pos: usize,
    diags: Vec<Diagnostic>,
    /// line breaks end statements here (blocks), not inside brackets
    nl_sig: bool,
    /// column and token index of the current statement's first token
    stmt_col: u32,
    stmt_start: usize,
    /// token indices of the open brackets
    open: Vec<usize>,
    depth: u32,
    /// a let / fn whose name was read when its statement failed (keeps the name declared)
    pending: Option<Box<Stmt>>,
}

/// see the module comment
pub fn parse_program(name: &str, src: &str) -> (Program, Vec<Diagnostic>) {
    let (all, diags) = lex::lex(src);
    let toks = lex::significant(&all, src);
    let mut p = Parser { src, toks, pos: 0, diags, nl_sig: true, stmt_col: 1, stmt_start: 0, open: Vec::new(), depth: 0, pending: None };
    let body = p.stmt_list(true);
    let mut d = p.diags;
    d.sort_by_key(|x| (x.line, x.col));
    let mut out: Vec<Diagnostic> = Vec::with_capacity(d.len());
    for x in d {
        if !out.last().is_some_and(|o| o.line == x.line) {
            out.push(x);
        }
    }
    (Program { name: name.to_string(), body }, out)
}

fn closer(k: TokKind) -> TokKind {
    match k {
        TokKind::LParen => TokKind::RParen,
        TokKind::LBracket => TokKind::RBracket,
        TokKind::LBrace => TokKind::RBrace,
        _ => TokKind::Eof,
    }
}

fn closer_name(k: TokKind) -> &'static str {
    match k {
        TokKind::RParen => "eine schließende Klammer )",
        TokKind::RBracket => "eine schließende eckige Klammer ]",
        _ => "eine schließende geschweifte Klammer }",
    }
}

fn is_closer(k: TokKind) -> bool {
    matches!(k, TokKind::RParen | TokKind::RBracket | TokKind::RBrace)
}

/// the binary operator of a token and its binding power (higher binds tighter)
fn binop(k: TokKind) -> Option<(BinOp, u8)> {
    Some(match k {
        TokKind::Coalesce => (BinOp::Coalesce, 1),
        TokKind::OrOr => (BinOp::Or, 2),
        TokKind::AndAnd => (BinOp::And, 3),
        TokKind::EqEq => (BinOp::Eq, BP_EQ),
        TokKind::NotEq => (BinOp::Ne, BP_EQ),
        TokKind::Lt => (BinOp::Lt, BP_CMP),
        TokKind::Le => (BinOp::Le, BP_CMP),
        TokKind::Gt => (BinOp::Gt, BP_CMP),
        TokKind::Ge => (BinOp::Ge, BP_CMP),
        TokKind::Plus => (BinOp::Add, 6),
        TokKind::Minus => (BinOp::Sub, 6),
        TokKind::Star => (BinOp::Mul, 7),
        TokKind::Slash => (BinOp::Div, 7),
        TokKind::Percent => (BinOp::Rem, 7),
        _ => return None,
    })
}

fn span(t: Tok) -> Span {
    Span { start: t.start, end: t.end, line: t.line, col: t.col }
}

impl<'a> Parser<'a> {
    // ---- tokens

    fn peek(&self) -> Tok {
        self.toks[self.pos]
    }

    fn peek_at(&self, k: usize) -> Tok {
        self.toks[(self.pos + k).min(self.toks.len() - 1)]
    }

    fn at(&self, k: TokKind) -> bool {
        self.toks[self.pos].kind == k
    }

    fn advance(&mut self) -> Tok {
        let t = self.toks[self.pos];
        if t.kind != TokKind::Eof {
            self.pos += 1;
        }
        t
    }

    fn prev(&self) -> Tok {
        self.toks[self.pos.saturating_sub(1)]
    }

    fn text(&self, t: Tok) -> &'a str {
        &self.src[t.start as usize..t.end as usize]
    }

    /// from the start of t to the end of the last token read
    fn since(&self, t: Tok) -> Span {
        Span { start: t.start, end: self.prev().end.max(t.start), line: t.line, col: t.col }
    }

    // ---- errors

    fn error(&mut self, sp: Span, msg: impl Into<String>) {
        self.diags.push(crate::diag(self.src, sp, true, msg.into()));
    }

    /// whether error_here() reports at the end of the previous token: the current one starts
    /// a later line of the statement (the line above is incomplete)
    fn after_prev(&self) -> bool {
        let t = self.peek();
        (t.nl || t.kind == TokKind::Eof) && self.pos > self.stmt_start
    }

    fn error_here(&mut self, msg: impl Into<String>) {
        if self.after_prev() {
            let p = self.prev();
            let col = p.col + self.text(p).chars().count() as u32;
            self.diags.push(Diagnostic { line: p.line, col, end_col: col + 1, error: true, msg: msg.into() });
        } else {
            self.error(span(self.peek()), msg);
        }
    }

    fn fail<T>(&mut self, msg: impl Into<String>) -> PResult<T> {
        self.error_here(msg);
        Err(Fail)
    }

    /// "Unerwarteter Name x", "Unerwartete Klammer )" ..
    fn unexpected(&self, t: Tok) -> String {
        let s = self.text(t);
        match t.kind {
            TokKind::Ident => format!("Unerwarteter Name {}", s),
            TokKind::Int | TokKind::Float => format!("Unerwartete Zahl {}", s),
            TokKind::Str | TokKind::StrHead | TokKind::StrMid | TokKind::StrTail => String::from("Unerwartete Zeichenkette"),
            TokKind::LParen | TokKind::RParen | TokKind::LBracket | TokKind::RBracket | TokKind::LBrace | TokKind::RBrace => {
                format!("Unerwartete Klammer {}", s)
            }
            TokKind::Comma => String::from("Unerwartetes Komma"),
            TokKind::Dot => String::from("Unerwarteter Punkt"),
            TokKind::Colon => String::from("Unerwarteter Doppelpunkt"),
            TokKind::Semi => String::from("Unerwartetes Semikolon"),
            TokKind::Eof => String::from("Unerwartetes Ende des Skripts"),
            TokKind::Error => format!("Unerwartetes Zeichen {}", s),
            k if k.is_keyword() => format!("Unerwartetes Schlüsselwort {}", s),
            _ => format!("Unerwarteter Operator {}", s),
        }
    }

    // ---- brackets and recovery

    /// the current token begins a new statement: the end of the script, or a later line at or
    /// left of the statement's column
    fn boundary(&self) -> bool {
        let t = self.peek();
        t.kind == TokKind::Eof || (t.nl && t.col <= self.stmt_col && self.pos > self.stmt_start)
    }

    /// the current token closes a bracket opened before the innermost one
    fn closes_outer(&self) -> bool {
        let k = self.peek().kind;
        let n = self.open.len().saturating_sub(1);
        is_closer(k) && self.open[..n].iter().any(|&i| closer(self.toks[i].kind) == k)
    }

    fn open_bracket(&mut self) -> Tok {
        self.open.push(self.pos);
        self.advance()
    }

    /// the closing bracket of the innermost open one; when it is missing at a statement
    /// boundary or before an outer closing bracket, report it and take it as given
    fn close(&mut self) -> PResult<()> {
        let Some(&oi) = self.open.last() else { return Ok(()) };
        let o = self.toks[oi];
        let want = closer(o.kind);
        if self.at(want) {
            self.open.pop();
            self.advance();
            return Ok(());
        }
        let line = if self.after_prev() { self.prev().line } else { self.peek().line };
        let mut msg = format!("Hier fehlt {}", closer_name(want));
        if o.line != line {
            msg.push_str(&format!(" (geöffnet in Zeile {})", o.line));
        }
        if self.boundary() || self.closes_outer() {
            self.error_here(msg);
            self.open.pop();
            return Ok(());
        }
        self.fail(msg)
    }

    /// skip the rest of a failed statement (see the module comment); base: the open brackets
    /// before it
    fn sync(&mut self, base: usize) {
        let mut depth = self.open.len().saturating_sub(base);
        loop {
            let t = self.peek();
            match t.kind {
                TokKind::Eof => return,
                TokKind::Semi if depth == 0 => {
                    self.advance();
                    return;
                }
                TokKind::RBrace if depth == 0 => return,
                _ => {}
            }
            if t.nl && self.pos > self.stmt_start {
                if depth == 0 || t.col < self.stmt_col || (t.col == self.stmt_col && !is_closer(t.kind)) {
                    return;
                }
            }
            match t.kind {
                TokKind::LParen | TokKind::LBracket | TokKind::LBrace => depth += 1,
                TokKind::RParen | TokKind::RBracket | TokKind::RBrace => depth = depth.saturating_sub(1),
                _ => {}
            }
            self.advance();
        }
    }

    /// one level deeper; chain: the next link of an operator or call chain (it keeps a level
    /// free for its operand, so an overlong chain is reported as such)
    fn enter(&mut self, chain: bool) -> PResult<()> {
        self.depth += 1;
        if self.depth + chain as u32 > MAX_DEPTH {
            let msg = if chain {
                format!("Der Ausdruck ist zu lang (höchstens {} Ebenen) – teile ihn mit let auf", MAX_DEPTH)
            } else {
                format!("Zu tief verschachtelt (höchstens {} Ebenen)", MAX_DEPTH)
            };
            return self.fail(msg);
        }
        Ok(())
    }

    // ---- statements

    /// statements up to the `}` of a block (top: up to the end, a stray `}` is an error)
    fn stmt_list(&mut self, top: bool) -> Vec<Stmt> {
        let mut out = Vec::new();
        loop {
            self.nl_sig = true;
            let t = self.peek();
            match t.kind {
                TokKind::Eof => break,
                TokKind::RBrace if !top => break,
                TokKind::RBrace => {
                    self.error(span(t), "Unerwartete Klammer }: hier ist kein Block offen");
                    self.advance();
                    continue;
                }
                TokKind::Semi => {
                    self.advance();
                    continue;
                }
                _ => {}
            }
            let (start, base, depth) = (self.pos, self.open.len(), self.depth);
            self.stmt_start = start;
            self.stmt_col = t.col;
            match self.statement() {
                Ok(s) => out.push(s),
                Err(Fail) => {
                    out.extend(self.pending.take().map(|b| *b));
                    self.sync(base);
                    self.open.truncate(base);
                    self.depth = depth;
                    if self.pos == start {
                        self.advance();
                    }
                }
            }
            self.pending = None;
        }
        out
    }

    fn block(&mut self) -> PResult<Block> {
        let t = self.peek();
        if t.kind != TokKind::LBrace {
            let msg = if t.kind == TokKind::Assign {
                "Zum Vergleichen schreibe == statt ="
            } else {
                "Hier fehlt eine öffnende geschweifte Klammer { für den Block"
            };
            return self.fail(msg);
        }
        self.enter(false)?;
        self.open_bracket();
        let saved = (self.nl_sig, self.stmt_col, self.stmt_start, self.pending.take());
        let stmts = self.stmt_list(false);
        (self.nl_sig, self.stmt_col, self.stmt_start, self.pending) = saved;
        self.close()?;
        self.depth -= 1;
        Ok(Block { span: self.since(t), stmts })
    }

    /// one statement; the kinds are parsed by functions that are never inlined, so the frames
    /// on the recursion path (block, stmt_list, statement) stay small
    fn statement(&mut self) -> PResult<Stmt> {
        match self.peek().kind {
            TokKind::Let => self.let_stmt(),
            TokKind::Fn if self.peek_at(1).kind != TokKind::LParen => self.fn_decl(),
            TokKind::If => self.if_stmt(),
            TokKind::While => self.while_stmt(),
            TokKind::For => self.for_stmt(),
            TokKind::Try => self.try_stmt(),
            TokKind::Break | TokKind::Continue | TokKind::Return | TokKind::Throw => self.jump_stmt(),
            TokKind::LBrace => self.block_stmt(),
            TokKind::Else => self.fail("else ohne vorheriges if"),
            TokKind::Catch => self.fail("catch ohne vorheriges try"),
            _ => self.expr_stmt(),
        }
    }

    #[inline(never)]
    fn while_stmt(&mut self) -> PResult<Stmt> {
        let kw = self.advance();
        let cond = self.expr()?;
        let body = self.block()?;
        Ok(Stmt { span: self.since(kw), kind: StmtKind::While { cond, body } })
    }

    /// break, continue, return [e], throw e
    #[inline(never)]
    fn jump_stmt(&mut self) -> PResult<Stmt> {
        let kw = self.advance();
        let kind = match kw.kind {
            TokKind::Break => StmtKind::Break,
            TokKind::Continue => StmtKind::Continue,
            TokKind::Return if self.stmt_ends() => StmtKind::Return(None),
            TokKind::Return => StmtKind::Return(Some(self.expr()?)),
            _ if self.stmt_ends() => return self.fail("Nach throw fehlt ein Wert (in derselben Zeile)"),
            _ => StmtKind::Throw(self.expr()?),
        };
        let sp = self.since(kw);
        self.end_stmt()?;
        Ok(Stmt { span: sp, kind })
    }

    #[inline(never)]
    fn block_stmt(&mut self) -> PResult<Stmt> {
        let b = self.block()?;
        Ok(Stmt { span: b.span, kind: StmtKind::Block(b) })
    }

    /// the statement may end here: a line break, `;`, `}` or the end
    fn stmt_ends(&self) -> bool {
        let t = self.peek();
        (t.nl && self.pos > self.stmt_start) || matches!(t.kind, TokKind::Semi | TokKind::RBrace | TokKind::Eof)
    }

    fn end_stmt(&mut self) -> PResult<()> {
        if self.at(TokKind::Semi) {
            self.advance();
            return Ok(());
        }
        if self.stmt_ends() {
            return Ok(());
        }
        let t = self.peek();
        let msg = match t.kind {
            TokKind::DotDot => String::from("Bereiche a..b gibt es nur in for-Schleifen"),
            _ => format!("{} – fehlt ein Operator oder ein Zeilenumbruch?", self.unexpected(t)),
        };
        self.fail(msg)
    }

    fn name(&mut self, msg: &str) -> PResult<Name> {
        let t = self.peek();
        if t.kind == TokKind::Ident {
            self.advance();
            return Ok(Name { text: self.text(t).to_string(), span: span(t) });
        }
        if t.kind.is_keyword() && !self.after_prev() {
            return self.fail(format!("{} ist ein Schlüsselwort und kann kein Name sein", self.text(t)));
        }
        self.fail(msg)
    }

    #[inline(never)]
    fn let_stmt(&mut self) -> PResult<Stmt> {
        let kw = self.advance();
        let name = self.name("Nach let fehlt ein Name")?;
        self.pending = Some(Box::new(Stmt { span: self.since(kw), kind: StmtKind::Let { name: name.clone(), init: None } }));
        let init = if self.at(TokKind::Assign) {
            self.advance();
            Some(self.expr()?)
        } else if self.stmt_ends() {
            None
        } else {
            return self.fail("Hier fehlt = nach dem Namen");
        };
        let sp = self.since(kw);
        self.end_stmt()?;
        Ok(Stmt { span: sp, kind: StmtKind::Let { name, init } })
    }

    #[inline(never)]
    fn fn_decl(&mut self) -> PResult<Stmt> {
        let kw = self.advance();
        let name = self.name("Nach fn fehlt ein Name")?;
        let stub = FnDef { span: self.since(kw), name: Some(name.clone()), params: Vec::new(), body: Block { span: name.span, stmts: Vec::new() } };
        self.pending = Some(Box::new(Stmt { span: stub.span, kind: StmtKind::Fn(stub) }));
        let f = self.fn_rest(kw, Some(name))?;
        Ok(Stmt { span: f.span, kind: StmtKind::Fn(f) })
    }

    /// (params) { body } of a declaration or a literal
    #[inline(never)]
    fn fn_rest(&mut self, kw: Tok, name: Option<Name>) -> PResult<FnDef> {
        if !self.at(TokKind::LParen) {
            return self.fail("Hier fehlt ( mit der Parameterliste");
        }
        self.open_bracket();
        let saved = self.nl_sig;
        self.nl_sig = false;
        let mut params = Vec::new();
        while !self.at(TokKind::RParen) && !self.at(TokKind::LBrace) && !self.at(TokKind::Eof) {
            params.push(self.name("Hier fehlt ein Parametername")?);
            if self.at(TokKind::Comma) {
                self.advance();
                continue;
            }
            if self.at(TokKind::RParen) || self.at(TokKind::LBrace) || self.boundary() || self.closes_outer() {
                break;
            }
            return self.fail("Hier fehlt ein Komma , oder eine schließende Klammer )");
        }
        self.nl_sig = saved;
        if self.at(TokKind::LBrace) {
            // fn f(a {: the body follows
            self.error_here("Hier fehlt eine schließende Klammer )");
            self.open.pop();
        } else {
            self.close()?;
        }
        let body = self.block()?;
        Ok(FnDef { span: self.since(kw), name, params, body })
    }

    #[inline(never)]
    fn if_stmt(&mut self) -> PResult<Stmt> {
        let kw = self.advance();
        let mut arms = Vec::new();
        let mut els = None;
        loop {
            let cond = self.expr()?;
            let body = self.block()?;
            arms.push((cond, body));
            if !self.at(TokKind::Else) {
                break;
            }
            self.advance();
            if self.at(TokKind::If) {
                self.advance();
                continue;
            }
            els = Some(self.block()?);
            break;
        }
        Ok(Stmt { span: self.since(kw), kind: StmtKind::If { arms, els } })
    }

    #[inline(never)]
    fn for_stmt(&mut self) -> PResult<Stmt> {
        let kw = self.advance();
        let a = self.name("Nach for fehlt ein Variablenname")?;
        let b = if self.at(TokKind::Comma) {
            self.advance();
            Some(self.name("Nach dem Komma fehlt ein zweiter Variablenname")?)
        } else {
            None
        };
        if !self.at(TokKind::In) {
            return self.fail("Hier fehlt in, z. B. for x in liste { … }");
        }
        self.advance();
        let first = self.expr()?;
        let src = if self.at(TokKind::DotDot) {
            self.advance();
            ForSrc::Range(first, self.expr()?)
        } else {
            ForSrc::Each(first)
        };
        let body = self.block()?;
        Ok(Stmt { span: self.since(kw), kind: StmtKind::For { a, b, src, body } })
    }

    #[inline(never)]
    fn try_stmt(&mut self) -> PResult<Stmt> {
        let kw = self.advance();
        let body = self.block()?;
        if !self.at(TokKind::Catch) {
            return self.fail("Nach dem try-Block fehlt catch");
        }
        self.advance();
        let var = if self.at(TokKind::Ident) { Some(self.name("")?) } else { None };
        let catch = self.block()?;
        Ok(Stmt { span: self.since(kw), kind: StmtKind::Try { body, var, catch } })
    }

    /// an expression statement or an assignment
    #[inline(never)]
    fn expr_stmt(&mut self) -> PResult<Stmt> {
        let e = self.expr()?;
        let t = self.peek();
        let op = match t.kind {
            TokKind::Assign => None,
            TokKind::PlusEq => Some(BinOp::Add),
            TokKind::MinusEq => Some(BinOp::Sub),
            TokKind::StarEq => Some(BinOp::Mul),
            TokKind::SlashEq => Some(BinOp::Div),
            TokKind::PercentEq => Some(BinOp::Rem),
            _ => {
                let sp = e.span;
                self.end_stmt()?;
                return Ok(Stmt { span: sp, kind: StmtKind::Expr(e) });
            }
        };
        if !matches!(e.kind, ExprKind::Var(_) | ExprKind::Field(..) | ExprKind::Index(..)) {
            let msg = format!("Vor {} muss eine Variable, ein Feld oder ein Index stehen", self.text(t));
            self.error(e.span, msg);
            return Err(Fail);
        }
        self.advance();
        let value = self.expr()?;
        let sp = e.span.to(value.span);
        self.end_stmt()?;
        Ok(Stmt { span: sp, kind: StmtKind::Assign { op, target: e, value } })
    }

    // ---- expressions

    fn expr(&mut self) -> PResult<Expr> {
        self.binary(0)
    }

    /// operators binding at least as tight as min (left-associative; comparisons do not chain)
    fn binary(&mut self, min: u8) -> PResult<Expr> {
        self.enter(false)?;
        let d0 = self.depth;
        let mut lhs = self.unary()?;
        let mut last = 0;
        loop {
            let t = self.peek();
            let Some((op, bp)) = binop(t.kind) else { break };
            if bp < min || (t.nl && self.nl_sig && op == BinOp::Sub) {
                break;
            }
            if (bp == BP_EQ || bp == BP_CMP) && last == bp {
                return self.fail("Vergleiche lassen sich nicht verketten – schreibe z. B. a < b && b < c");
            }
            self.advance();
            self.enter(true)?;
            let rhs = self.binary(bp + 1)?;
            lhs = Expr { span: lhs.span.to(rhs.span), kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)) };
            last = bp;
        }
        self.depth = d0 - 1;
        Ok(lhs)
    }

    fn unary(&mut self) -> PResult<Expr> {
        let t = self.peek();
        let op = match t.kind {
            TokKind::Minus => UnOp::Neg,
            TokKind::Not => UnOp::Not,
            _ => return self.postfix(),
        };
        self.advance();
        self.enter(false)?;
        let e = self.unary()?;
        self.depth -= 1;
        Ok(Expr { span: span(t).to(e.span), kind: ExprKind::Unary(op, Box::new(e)) })
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let e = self.primary()?;
        self.postfix_ops(e)
    }

    /// calls, .field, .method(..) and [index] after a primary expression (a function of its
    /// own, like the recursive parts of primary: small frames on the recursion path)
    #[inline(never)]
    fn postfix_ops(&mut self, mut e: Expr) -> PResult<Expr> {
        let d0 = self.depth;
        loop {
            let t = self.peek();
            let same_line = !(t.nl && self.nl_sig);
            let sp = e.span;
            let kind = match t.kind {
                TokKind::LParen if same_line => {
                    let args = self.exprs()?;
                    ExprKind::Call(Box::new(e), args)
                }
                TokKind::LBracket if same_line => {
                    self.open_bracket();
                    let saved = self.nl_sig;
                    self.nl_sig = false;
                    let i = self.expr()?;
                    self.nl_sig = saved;
                    self.close()?;
                    ExprKind::Index(Box::new(e), Box::new(i))
                }
                TokKind::Dot => {
                    self.advance();
                    let n = self.peek();
                    if n.kind != TokKind::Ident && !(n.kind.is_keyword() && !self.after_prev()) {
                        return self.fail("Nach dem Punkt fehlt ein Name");
                    }
                    self.advance();
                    let name = Name { text: self.text(n).to_string(), span: span(n) };
                    if self.at(TokKind::LParen) && !(self.peek().nl && self.nl_sig) {
                        let args = self.exprs()?;
                        ExprKind::Method(Box::new(e), name, args)
                    } else {
                        ExprKind::Field(Box::new(e), name)
                    }
                }
                _ => break,
            };
            e = Expr { span: Span { end: self.prev().end, ..sp }, kind };
            self.enter(true)?;
        }
        self.depth = d0;
        Ok(e)
    }

    fn primary(&mut self) -> PResult<Expr> {
        let t = self.peek();
        let text = self.text(t);
        let kind = match t.kind {
            TokKind::Int => ExprKind::Int(lex::int_value(text).unwrap_or(0)),
            TokKind::Float => ExprKind::Float(lex::float_value(text).unwrap_or(0.0)),
            TokKind::Str => ExprKind::Str(lex::str_value(t.kind, text)),
            TokKind::StrHead => return self.interp(),
            TokKind::True => ExprKind::Bool(true),
            TokKind::False => ExprKind::Bool(false),
            TokKind::Nil => ExprKind::Nil,
            TokKind::Ident => ExprKind::Var(text.to_string()),
            TokKind::LParen => return self.paren(),
            TokKind::LBracket => return self.list(),
            TokKind::LBrace => return self.map(),
            TokKind::Fn if self.peek_at(1).kind == TokKind::Ident => {
                if self.after_prev() {
                    // a declaration on the next line: the expression above is missing
                    self.error_here("Hier fehlt ein Ausdruck");
                    return Ok(Expr { span: Span { end: t.start, ..span(t) }, kind: ExprKind::Nil });
                }
                return self.fail("Eine Funktion als Wert hat keinen Namen – schreibe fn(…) { … }");
            }
            TokKind::Fn => return self.fn_lit(),
            TokKind::DotDot => return self.fail("Bereiche a..b gibt es nur in for-Schleifen"),
            _ => {
                let after = self.pos > self.stmt_start;
                if after && (self.boundary() || matches!(t.kind, TokKind::RParen | TokKind::RBracket | TokKind::RBrace
                    | TokKind::Comma | TokKind::Semi | TokKind::StrMid | TokKind::StrTail)) {
                    // the expression is missing, the rest still parses
                    self.error_here("Hier fehlt ein Ausdruck");
                    return Ok(Expr { span: Span { end: t.start, ..span(t) }, kind: ExprKind::Nil });
                }
                let msg = if after { String::from("Hier fehlt ein Ausdruck") } else { self.unexpected(t) };
                return self.fail(msg);
            }
        };
        self.advance();
        Ok(Expr { span: span(t), kind })
    }

    /// ( e ): the expression, its span with the brackets
    #[inline(never)]
    fn paren(&mut self) -> PResult<Expr> {
        let o = self.open_bracket();
        let saved = self.nl_sig;
        self.nl_sig = false;
        let e = self.expr()?;
        self.nl_sig = saved;
        self.close()?;
        Ok(Expr { span: self.since(o), kind: e.kind })
    }

    /// [ a, b ]
    #[inline(never)]
    fn list(&mut self) -> PResult<Expr> {
        let o = self.peek();
        let items = self.exprs()?;
        Ok(Expr { span: self.since(o), kind: ExprKind::List(items) })
    }

    /// fn(params) { body }
    #[inline(never)]
    fn fn_lit(&mut self) -> PResult<Expr> {
        let kw = self.advance();
        let f = self.fn_rest(kw, None)?;
        Ok(Expr { span: f.span, kind: ExprKind::Fn(Box::new(f)) })
    }

    /// ( a, b ) or [ a, b ]: the current token opens the list; a trailing comma is allowed
    #[inline(never)]
    fn exprs(&mut self) -> PResult<Vec<Expr>> {
        let o = self.open_bracket();
        let close = closer(o.kind);
        let saved = self.nl_sig;
        self.nl_sig = false;
        let mut v = Vec::new();
        while !self.at(close) && !self.at(TokKind::Eof) {
            v.push(self.expr()?);
            if self.at(TokKind::Comma) {
                self.advance();
                continue;
            }
            if self.at(close) || self.boundary() || self.closes_outer() {
                break;
            }
            return self.fail(format!("Hier fehlt ein Komma , oder {}", closer_name(close)));
        }
        self.nl_sig = saved;
        self.close()?;
        Ok(v)
    }

    /// {k: v, "s": v, 1: v, [e]: v}
    #[inline(never)]
    fn map(&mut self) -> PResult<Expr> {
        let o = self.open_bracket();
        let saved = self.nl_sig;
        self.nl_sig = false;
        let mut kv = Vec::new();
        while !self.at(TokKind::RBrace) && !self.at(TokKind::Eof) {
            let t = self.peek();
            let word = t.kind == TokKind::Ident
                || (t.kind.is_keyword() && !matches!(t.kind, TokKind::True | TokKind::False | TokKind::Nil) && !self.boundary());
            let key = match t.kind {
                _ if word => {
                    self.advance();
                    Expr { span: span(t), kind: ExprKind::Str(self.text(t).to_string()) }
                }
                TokKind::Int | TokKind::Float | TokKind::Str | TokKind::StrHead | TokKind::True | TokKind::False | TokKind::Nil => {
                    self.primary()?
                }
                TokKind::LBracket => {
                    self.open_bracket();
                    let e = self.expr()?;
                    self.close()?;
                    e
                }
                _ => {
                    if self.boundary() || self.closes_outer() {
                        break;
                    }
                    return self.fail("Hier fehlt ein Schlüssel: ein Name, eine Zeichenkette, eine Zahl oder [Ausdruck]");
                }
            };
            if !self.at(TokKind::Colon) {
                return self.fail("Hier fehlt ein Doppelpunkt : nach dem Schlüssel");
            }
            self.advance();
            let v = self.expr()?;
            kv.push((key, v));
            if self.at(TokKind::Comma) {
                self.advance();
                continue;
            }
            if self.at(TokKind::RBrace) || self.boundary() || self.closes_outer() {
                break;
            }
            return self.fail("Hier fehlt ein Komma , oder eine schließende geschweifte Klammer }");
        }
        self.nl_sig = saved;
        self.close()?;
        Ok(Expr { span: self.since(o), kind: ExprKind::Map(kv) })
    }

    /// "a${x}b${y}c": StrHead, expression, StrMid, expression, StrTail
    #[inline(never)]
    fn interp(&mut self) -> PResult<Expr> {
        let head = self.advance();
        let mut parts = Vec::new();
        self.piece(&mut parts, head);
        let saved = self.nl_sig;
        self.nl_sig = false;
        loop {
            let t = self.peek();
            if t.nl || t.kind == TokKind::Eof {
                // the string ended at the line end (lex reported it)
                self.error_here("Hier fehlt } am Ende von ${…}");
                break;
            }
            if matches!(t.kind, TokKind::StrMid | TokKind::StrTail) {
                self.error(span(t), "Leeres ${}: hier fehlt ein Ausdruck");
            } else {
                parts.push(self.expr()?);
            }
            let t = self.peek();
            match t.kind {
                TokKind::StrMid => {
                    self.advance();
                    self.piece(&mut parts, t);
                }
                TokKind::StrTail => {
                    self.advance();
                    self.piece(&mut parts, t);
                    break;
                }
                _ => {
                    self.error_here("Hier fehlt } am Ende von ${…}");
                    if t.nl || t.kind == TokKind::Eof {
                        break;
                    }
                    return Err(Fail);
                }
            }
        }
        self.nl_sig = saved;
        Ok(Expr { span: self.since(head), kind: ExprKind::Interp(parts) })
    }

    fn piece(&self, parts: &mut Vec<Expr>, t: Tok) {
        let s = lex::str_value(t.kind, self.text(t));
        if !s.is_empty() {
            parts.push(Expr { span: span(t), kind: ExprKind::Str(s) });
        }
    }
}
