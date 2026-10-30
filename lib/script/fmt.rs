// fmt -- the formatter of .jfs scripts
//
//   format(src) -> Result<String, Vec<Diagnostic>>   the script laid out anew, two blanks per
//                                level; the syntax errors when it does not parse (it is left
//                                as it is then)
//   format_with(src, unit)       the same with another unit of indentation ("    ", "\t")
//
// The layout: one statement per line (`;` between statements becomes a line break, the
// statements of a one-line block get lines of their own, `}` a line of its own; an empty
// block stays `{}`), the `{` of a block on the line of what it belongs to, `else` and `catch`
// after the `}` before them; indentation by the open brackets (one level per line that leaves
// brackets open, a closing bracket at a line's start back at the level of the line that
// opened it) and one more level for a line that continues a statement (`.map(f)`, `&& b`,
// the value after `=`).  Blanks: around binary operators and `=`, after `,` and the `:` of a
// map, between words; none inside ( ) [ ] and map braces, around `.` and `..`, after a unary
// operator, before the ( of a call and the [ of an index.  Inside expressions the line
// breaks of the source stay (they are where the author broke a long list or chain).
// Comments stay where they are: a comment after code on its line one blank after it, a
// comment on its own line at the level of the code around it, /* */ over several lines as
// written; blank lines stay, at most one in a row (none at the start of a block or the file,
// none before a `}`).  Strings and numbers keep their text.  The result ends with a line
// break when the source did.  Inside ${..} of a string nothing is broken into lines.
// Safety: the formatted source is parsed again and its tree compared with the original's
// (ast::Program::sexpr); a difference is an error instead of a changed program, so the
// compiled code of the result always equals the original's.  format is idempotent.

use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::*;
use crate::lex::{self, Tok, TokKind};
use crate::Diagnostic;

/// see the module comment
pub fn format(src: &str) -> Result<String, Vec<Diagnostic>> {
    format_with(src, "  ")
}

/// what an open bracket is
#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    Block,
    Paren,
    Bracket,
    Map,
    Interp,
}

/// the places the tree tells: statement starts, block braces
struct Marks {
    stmts: Vec<u32>,
    /// (the `{`, the `}`) of every block, by the `{`
    blocks: Vec<(u32, u32)>,
    closes: Vec<u32>,
}

impl Marks {
    fn stmt(&self, at: u32) -> bool {
        self.stmts.binary_search(&at).is_ok()
    }
    fn open(&self, at: u32) -> bool {
        self.blocks.binary_search_by_key(&at, |b| b.0).is_ok()
    }
    fn close(&self, at: u32) -> bool {
        self.closes.binary_search(&at).is_ok()
    }
    /// the `{` of the block whose `}` is at `at`
    fn open_of(&self, at: u32) -> Option<u32> {
        self.blocks.iter().find(|b| b.1 == at).map(|b| b.0)
    }
}

fn mark_block(b: &Block, m: &mut Marks) {
    m.blocks.push((b.span.start, b.span.end.saturating_sub(1)));
    m.closes.push(b.span.end.saturating_sub(1));
    for s in &b.stmts {
        mark_stmt(s, m);
    }
}

fn mark_stmt(s: &Stmt, m: &mut Marks) {
    m.stmts.push(s.span.start);
    match &s.kind {
        StmtKind::Let { init: Some(e), .. } | StmtKind::Throw(e) | StmtKind::Expr(e) | StmtKind::Return(Some(e)) => mark_expr(e, m),
        StmtKind::Fn(f) => mark_block(&f.body, m),
        StmtKind::If { arms, els } => {
            for (c, b) in arms {
                mark_expr(c, m);
                mark_block(b, m);
            }
            if let Some(b) = els {
                mark_block(b, m);
            }
        }
        StmtKind::While { cond, body } => {
            mark_expr(cond, m);
            mark_block(body, m);
        }
        StmtKind::For { src, body, .. } => {
            match src {
                ForSrc::Each(e) => mark_expr(e, m),
                ForSrc::Range(a, b) => {
                    mark_expr(a, m);
                    mark_expr(b, m);
                }
            }
            mark_block(body, m);
        }
        StmtKind::Try { body, catch, .. } => {
            mark_block(body, m);
            mark_block(catch, m);
        }
        StmtKind::Assign { target, value, .. } => {
            mark_expr(target, m);
            mark_expr(value, m);
        }
        StmtKind::Block(b) => mark_block(b, m),
        _ => {}
    }
}

fn mark_expr(e: &Expr, m: &mut Marks) {
    match &e.kind {
        ExprKind::Interp(v) | ExprKind::List(v) => v.iter().for_each(|x| mark_expr(x, m)),
        ExprKind::Map(kv) => kv.iter().for_each(|(k, v)| {
            mark_expr(k, m);
            mark_expr(v, m);
        }),
        ExprKind::Fn(f) => mark_block(&f.body, m),
        ExprKind::Unary(_, a) | ExprKind::Field(a, _) => mark_expr(a, m),
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
            mark_expr(a, m);
            mark_expr(b, m);
        }
        ExprKind::Call(f, args) | ExprKind::Method(f, _, args) => {
            mark_expr(f, m);
            args.iter().for_each(|x| mark_expr(x, m));
        }
        _ => {}
    }
}

/// a token that may end an operand (a `-` after it is binary)
fn operand_end(k: TokKind) -> bool {
    matches!(k, TokKind::Ident | TokKind::Int | TokKind::Float | TokKind::Str | TokKind::StrTail | TokKind::True | TokKind::False | TokKind::Nil | TokKind::RParen | TokKind::RBracket | TokKind::RBrace)
}

struct Out<'a> {
    src: &'a str,
    unit: &'a str,
    s: String,
    /// at the start of a line (nothing written on it yet)
    bol: bool,
    /// the level of the current line
    level: usize,
}

impl Out<'_> {
    /// end the current line (nothing at the start of one)
    fn newline(&mut self) {
        if self.bol {
            return;
        }
        while self.s.ends_with(' ') {
            self.s.pop();
        }
        self.s.push('\n');
        self.bol = true;
    }

    fn blank(&mut self) {
        if !self.s.is_empty() && !self.s.ends_with("\n\n") {
            self.s.push('\n');
        }
    }

    fn start(&mut self, level: usize) {
        if self.bol {
            for _ in 0..level {
                self.s.push_str(self.unit);
            }
            self.level = level;
            self.bol = false;
        }
    }

    fn put(&mut self, t: &str) {
        self.s.push_str(t);
    }

    fn text(&self, t: &Tok) -> &str {
        &self.src[t.start as usize..t.end as usize]
    }
}

/// see the module comment
pub fn format_with(src: &str, unit: &str) -> Result<String, Vec<Diagnostic>> {
    let prog = crate::parse("", src)?;
    let mut m = Marks { stmts: Vec::new(), blocks: Vec::new(), closes: Vec::new() };
    for s in &prog.body {
        mark_stmt(s, &mut m);
    }
    m.stmts.sort_unstable();
    m.stmts.dedup();
    m.blocks.sort_unstable();
    m.closes.sort_unstable();
    let (toks, _) = lex::lex(src);
    let mut o = Out { src, unit, s: String::with_capacity(src.len() + src.len() / 8), bol: true, level: 0 };
    let mut stack: Vec<(Open, usize)> = Vec::new();
    // the last significant token written and whether it was a unary operator, the line breaks
    // since what was written last, a // comment that ends the line, a comment written last
    let mut prev: Option<Tok> = None;
    let mut prev_unary = false;
    let mut nls = 0usize;
    let mut force_nl = false;
    let mut after_comment = false;
    for t in &toks {
        match t.kind {
            TokKind::Space | TokKind::Eof | TokKind::Semi => continue,
            TokKind::Newline => {
                nls += 1;
                continue;
            }
            _ => {}
        }
        let interp = stack.iter().any(|x| x.0 == Open::Interp);
        let inner = stack.last().map_or(0, |x| x.1 + 1);
        if t.kind == TokKind::Comment {
            let text = o.text(t).trim_end().replace("\r\n", "\n");
            let own = nls > 0 || o.s.is_empty();
            if own && !interp {
                o.newline();
                if nls >= 2 && prev.is_some_and(|p| !(p.kind == TokKind::LBrace && m.open(p.start))) {
                    o.blank();
                }
                o.start(inner);
            } else if o.bol {
                o.start(inner);
            } else {
                o.put(" ");
            }
            o.put(&text);
            force_nl = text.starts_with("//");
            after_comment = true;
            nls = 0;
            continue;
        }
        let k = t.kind;
        let is_stmt = m.stmt(t.start);
        let p_open = prev.is_some_and(|p| p.kind == TokKind::LBrace && m.open(p.start));
        let t_close = k == TokKind::RBrace && m.close(t.start);
        let empty = t_close && prev.is_some_and(|p| m.open_of(t.start) == Some(p.start));
        let joined = (k == TokKind::LBrace && m.open(t.start)) || (matches!(k, TokKind::Else | TokKind::Catch) && prev.is_some_and(|p| p.kind == TokKind::RBrace));
        // a line break before t?
        let nl = if force_nl {
            true
        } else if prev.is_none() {
            // (after comments at the start)
            nls > 0 && !o.s.is_empty()
        } else if interp || empty {
            false
        } else if is_stmt || p_open || t_close {
            true
        } else if joined {
            false
        } else {
            nls > 0
        };
        if nl {
            o.newline();
            if nls >= 2 && !p_open && !t_close {
                o.blank();
            }
        }
        if interp && prev.is_some() && is_stmt && !p_open {
            // statements inside ${..} stay on the line
            o.put(";");
        }
        // the level of a line that starts here
        if o.bol {
            let closer = matches!(k, TokKind::RParen | TokKind::RBracket | TokKind::RBrace);
            let level = if closer {
                stack.last().map_or(0, |x| x.1)
            } else {
                let stmt_cx = stack.last().is_none_or(|x| x.0 == Open::Block);
                let cont = stmt_cx && !is_stmt && !matches!(k, TokKind::Else | TokKind::Catch) && prev.is_some();
                inner + cont as usize
            };
            o.start(level);
        } else if after_comment || prev.is_some_and(|p| space(&p, t, &m, prev_unary, interp, empty)) {
            o.put(" ");
        }
        force_nl = false;
        after_comment = false;
        let text = String::from(o.text(t));
        o.put(&text);
        nls = 0;
        prev_unary = k == TokKind::Not || (k == TokKind::Minus && (is_stmt || prev.is_none_or(|p| !operand_end(p.kind))));
        // the brackets
        match k {
            TokKind::LBrace => stack.push((if m.open(t.start) { Open::Block } else { Open::Map }, o.level)),
            TokKind::LParen => stack.push((Open::Paren, o.level)),
            TokKind::LBracket => stack.push((Open::Bracket, o.level)),
            TokKind::StrHead => stack.push((Open::Interp, o.level)),
            TokKind::RBrace | TokKind::RParen | TokKind::RBracket | TokKind::StrTail => {
                stack.pop();
            }
            _ => {}
        }
        prev = Some(*t);
    }
    // the end: a line break when the source ended with one
    while o.s.ends_with(' ') || o.s.ends_with('\n') {
        o.s.pop();
    }
    if src.ends_with('\n') && !o.s.is_empty() {
        o.s.push('\n');
    }
    // the safety net: the same tree
    match crate::parse("", &o.s) {
        Ok(p2) if p2.sexpr() == prog.sexpr() => Ok(o.s),
        _ => Err(alloc::vec![Diagnostic {
            line: 1,
            col: 1,
            end_col: 2,
            error: true,
            msg: String::from("Der Formatierer hätte die Bedeutung des Skripts geändert; es bleibt, wie es ist"),
            file: String::new(),
        }]),
    }
}

/// a blank between token p and token t on one line (p_unary: p is a unary operator)
fn space(p: &Tok, t: &Tok, m: &Marks, p_unary: bool, interp: bool, empty: bool) -> bool {
    use TokKind::*;
    let (pk, tk) = (p.kind, t.kind);
    let p_block_open = pk == LBrace && m.open(p.start);
    let t_block_open = tk == LBrace && m.open(t.start);
    let t_close = tk == RBrace && m.close(t.start);
    if empty {
        return false;
    }
    // inside ${..}: a block keeps its statements on the line, blanks inside its braces
    if interp && (p_block_open || t_close) {
        return true;
    }
    if matches!(pk, StrHead | StrMid) || matches!(tk, StrMid | StrTail) {
        return false;
    }
    if matches!(tk, Comma | RParen | RBracket | Dot | Colon | DotDot) || (tk == RBrace && !t_close) {
        return false;
    }
    if matches!(pk, LParen | LBracket | Dot | DotDot) || (pk == LBrace && !p_block_open) || p_unary {
        return false;
    }
    if t_block_open {
        return true;
    }
    match tk {
        LParen => !matches!(pk, Ident | RParen | RBracket | Str | StrTail | Fn),
        LBracket => !matches!(pk, Ident | RParen | RBracket | Str | StrTail | RBrace),
        _ => true,
    }
}
