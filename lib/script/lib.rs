// script -- the JFNetworker scripting language (.jfs): lexer, parser, syntax tree, editor
// diagnostics, compiler, VM with fuel and a garbage collector
//
//   parse(name, src) -> Result<ast::Program, Vec<Diagnostic>>   the syntax tree, or the syntax
//                                errors (the parser goes on at the next statement after an
//                                error and reports the first error of each line)
//   parse_partial(name, src)     the tree of what parsed plus the errors (editor features)
//   check(src)                   syntax errors plus name resolution against GLOBALS: unknown
//                                names (warnings), duplicate declarations, break / continue
//                                outside a loop, return outside a function (errors);
//                                check_with(src, globals) with other predeclared names
//   resolve(&program, globals)   the resolution part alone (for the compiler)
//   tokens(src)                  (byte start, length, TokKind) of every token, covering every
//                                byte, for syntax highlighting; never fails
//   outline(src)                 the top-level fns and lets with their lines
//   compile(src) -> Result<Program, Vec<Diagnostic>>   bytecode (code.rs, compile.rs), or the
//                                errors of parse() and check() (warnings do not stop it);
//                                compile_named(name, src) names the file; compile_with(name,
//                                src, globals) for a host with other names (the standalone
//                                runner apps/jfs): unknown names are errors there
//   Vm::new(&program, limits)    a VM (vm.rs): run(host, fuel) -> Done | Yielded | Error,
//                                call(name, args), call_value, global / set_global, pin,
//                                output; Value (16 bytes, heap.rs), Host and Cx for native
//                                functions; num: float arithmetic in software
//   library()                    the standard library every host gets (stdlib.rs: str, list,
//                                map, bytes, math, json, int / float.parse; math.rs: exp, log,
//                                pow, sin, cos .. in software), found by name before the host
//
// The language: small, dynamically typed, JS/Lua-like.  Values: nil, bool, int (i64), float
// (f64), str (immutable UTF-8), bytes, list, map (insertion ordered; keys str, int, bool),
// fn (closures).  Statements: let x = e, fn name(a, b) { }, if c { } else if c { } else { },
// while c { }, for x in list { }, for k, v in map { }, for i in 0..n { }, break, continue,
// return [e], try { } catch e { }, throw e, assignments = += -= *= /= %= to names, fields
// (m.k) and indexes (a[i]), expression statements.  Expressions: literals (1_000, 0xFF,
// 0b101, 1.5e3, "text" with \n \t \r \0 \" \\ \$ \u{..} and ${expr}, true, false, nil),
// [a, b], {k: v, "s": v, 1: v, [e]: v}, fn(a) { }, calls, .field, [index], methods
// s.split(","), unary - !, binary operators from loose to tight: ?? || && (== !=)
// (< <= > >=) (+ -) (* / %).  Comments // and /* */.  A line break ends a statement once
// its expression is complete (parse.rs has the details); a string stays on one line.
// Positions: lines and columns from 1, columns count characters, end_col is exclusive.
// The messages are German.  The scope rules are in ast.rs.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod ast;
pub mod check;
pub mod lex;
mod parse;
mod code;
mod compile;
mod heap;
pub mod num;
pub mod math;
pub mod vm;
mod stdlib;
mod strlib;
mod listlib;
mod jsonlib;

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

pub use ast::Span;
pub use check::resolve;
pub use lex::{TokClass, TokKind};
pub use parse::MAX_DEPTH;
pub use code::Program;
pub use heap::{Ref, Value};
pub use vm::{Cx, Error, ErrorKind, Fail, Host, Limits, Pin, Status, TraceLine, Vm};

/// a message for the editor: position (1-based; columns count characters, end_col is
/// exclusive and on the same line), error or warning, German text
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub line: u32,
    pub col: u32,
    pub end_col: u32,
    pub error: bool,
    pub msg: String,
}

impl core::fmt::Display for Diagnostic {
    /// "3:5: Fehler: Hier fehlt eine schließende Klammer )"
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "{}:{}: {}: {}", self.line, self.col, if self.error { "Fehler" } else { "Warnung" }, self.msg)
    }
}

/// the names a script may use without declaring them: the host API of the simulator and the
/// standard library (check warns about other unknown names)
pub const GLOBALS: &[&str] = &[
    "print", "log", "len", "str", "int", "float", "type", "math", "json", "bytes", "now", "rand", "config", "timer", "net",
    "dns", "http", "disk", "metrics", "list", "map",
];

/// the functions and constants of the standard library ("str.split", "math.pi", "bytes"),
/// for completion in an editor; methods are listed under their type (s.split(",") is
/// str.split(s, ","))
pub fn library() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = Vec::with_capacity(stdlib::NAMES.len() + stdlib::CONSTANTS.len() + 7);
    v.extend_from_slice(&["print", "len", "type", "str", "int", "float", "bytes"]);
    v.extend_from_slice(stdlib::NAMES);
    v.extend_from_slice(stdlib::CONSTANTS);
    v
}

/// a diagnostic for the first line of a span
pub(crate) fn diag(src: &str, sp: Span, error: bool, msg: String) -> Diagnostic {
    let s = src.get(sp.start as usize..sp.end as usize).unwrap_or("");
    let w = s.split('\n').next().unwrap_or("").chars().count() as u32;
    Diagnostic { line: sp.line, col: sp.col, end_col: sp.col + w.max(1), error, msg }
}

/// the syntax tree of a script, or its syntax errors; name: a file name for later messages
pub fn parse(name: &str, src: &str) -> Result<ast::Program, Vec<Diagnostic>> {
    let (prog, diags) = parse::parse_program(name, src);
    if diags.is_empty() { Ok(prog) } else { Err(diags) }
}

/// the tree of everything that parsed and the syntax errors (empty when parse() succeeds)
pub fn parse_partial(name: &str, src: &str) -> (ast::Program, Vec<Diagnostic>) {
    parse::parse_program(name, src)
}

/// syntax errors and the findings of resolve() with GLOBALS, sorted by position
pub fn check(src: &str) -> Vec<Diagnostic> {
    check_with(src, GLOBALS)
}

/// check() with other predeclared names
pub fn check_with(src: &str, globals: &[&str]) -> Vec<Diagnostic> {
    let (prog, mut d) = parse::parse_program("", src);
    d.extend(check::resolve(&prog, globals));
    d.sort_by_key(|x| (x.line, x.col));
    d
}

/// (byte start, length, kind) of every token in source order, covering every byte; the
/// `${` and `}` of interpolations are TokKind::Interp, the text around them Str
pub fn tokens(src: &str) -> Vec<(u32, u32, TokKind)> {
    let (toks, _) = lex::lex(src);
    let mut out = Vec::with_capacity(toks.len() + 8);
    for t in &toks {
        let (s, e) = (t.start, t.end);
        match t.kind {
            TokKind::Eof => {}
            TokKind::StrHead => {
                out.push((s, e - 2 - s, TokKind::Str));
                out.push((e - 2, 2, TokKind::Interp));
            }
            TokKind::StrMid => {
                out.push((s, 1, TokKind::Interp));
                if e - 2 > s + 1 {
                    out.push((s + 1, e - 3 - s, TokKind::Str));
                }
                out.push((e - 2, 2, TokKind::Interp));
            }
            TokKind::StrTail => {
                out.push((s, 1, TokKind::Interp));
                if e > s + 1 {
                    out.push((s + 1, e - s - 1, TokKind::Str));
                }
            }
            k => out.push((s, e - s, k)),
        }
    }
    out
}

/// the top-level declarations with their lines: "fn key(id)", "let hits" (also while the
/// script has syntax errors)
pub fn outline(src: &str) -> Vec<(String, u32)> {
    let (prog, _) = parse::parse_program("", src);
    let mut out = Vec::new();
    for s in &prog.body {
        match &s.kind {
            ast::StmtKind::Let { name, .. } => out.push((format!("let {}", name.text), s.span.line)),
            ast::StmtKind::Fn(f) => {
                let params: Vec<&str> = f.params.iter().map(|p| p.text.as_str()).collect();
                let name = f.name.as_ref().map_or("", |n| n.text.as_str());
                out.push((format!("fn {}({})", name, params.join(", ")), s.span.line));
            }
            _ => {}
        }
    }
    out
}

/// compile a script for the VM; the errors of parse() and check() (not the warnings) when it
/// has some
pub fn compile(src: &str) -> Result<Program, Vec<Diagnostic>> {
    compile_named("", src)
}

/// compile() with a file name (Program::name)
pub fn compile_named(name: &str, src: &str) -> Result<Program, Vec<Diagnostic>> {
    let (prog, mut d) = parse::parse_program(name, src);
    d.extend(check::resolve(&prog, GLOBALS));
    finish(&prog, src, d)
}

/// compile_named() for a host with names of its own instead of GLOBALS: `globals` are the
/// names it answers (Host::call, set_global; "log", "timer", "config"), the built-ins and the
/// standard library are known without them.  Unlike compile(), a name that is neither
/// declared nor known is an error ("Unbekannter Name prnt – meinst du print?"), so a typing
/// error stops the script before it runs; the other warnings still do not
pub fn compile_with(name: &str, src: &str, globals: &[&str]) -> Result<Program, Vec<Diagnostic>> {
    let mut known: Vec<&str> = Vec::with_capacity(globals.len() + 16);
    for n in library() {
        let root = n.split('.').next().unwrap_or(n);
        if !known.contains(&root) {
            known.push(root);
        }
    }
    known.extend_from_slice(globals);
    let (prog, mut d) = parse::parse_program(name, src);
    d.extend(check::resolve_strict(&prog, &known));
    finish(&prog, src, d)
}

/// the program, or the errors among the diagnostics of parsing and resolving
fn finish(prog: &ast::Program, src: &str, mut d: Vec<Diagnostic>) -> Result<Program, Vec<Diagnostic>> {
    d.retain(|x| x.error);
    if !d.is_empty() {
        d.sort_by_key(|x| (x.line, x.col));
        return Err(d);
    }
    Ok(Program { code: Arc::new(compile::generate(prog, src)) })
}
