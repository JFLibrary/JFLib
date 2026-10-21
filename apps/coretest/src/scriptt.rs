// scriptt -- checks of lib/script: the JFNetworker example, operator precedence, strings and
// interpolation, line-break rules, error recovery, check(), tokens(), outline(), robustness

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use script::ast::{Expr, ExprKind, StmtKind};
use script::{TokClass, TokKind};

use crate::T;

const EXAMPLE: &str = r#"let hits = 0
let cache = {}
fn key(id) { return "user/" + id }
http.serve(config.port ?? 80, fn(req, res) {
  hits += 1
  if req.path == "/user" {
    let id = req.query.id
    if cache[id] != nil { return res.json(200, cache[id]) }
    disk.get(key(id), fn(err, v) {
      if err { return res.send(500, "${err}") }
      cache[id] = v
      res.json(200, v)
    })
    return
  }
  res.send(200, "hello #${hits}")
})
"#;

/// the S-expression of a script, or "ERR" and its errors
fn sx(src: &str) -> String {
    match script::parse("t.jfs", src) {
        Ok(p) => p.sexpr(),
        Err(d) => {
            let v: Vec<String> = d.iter().map(|x| x.to_string()).collect();
            format!("ERR {}", v.join("; "))
        }
    }
}

/// (line, message) of the syntax errors
fn errs(src: &str) -> Vec<(u32, String)> {
    script::parse_partial("t.jfs", src).1.into_iter().map(|d| (d.line, d.msg)).collect()
}

/// the value of the string literal in `x = LIT`
fn str_of(lit: &str) -> String {
    let src = format!("x = {}", lit);
    let Ok(p) = script::parse("t.jfs", &src) else { return String::from("ERR") };
    match p.body.first().map(|s| &s.kind) {
        Some(StmtKind::Assign { value: Expr { kind: ExprKind::Str(s), .. }, .. }) => s.clone(),
        _ => String::from("?"),
    }
}

/// tokens() covers every byte of src exactly once, in order, without empty tokens
fn covers(src: &str) -> bool {
    let mut pos = 0;
    for (s, n, _) in script::tokens(src) {
        if s != pos || n == 0 {
            return false;
        }
        pos = s + n;
    }
    pos as usize == src.len()
}

/// the deepest kinds of nesting with n levels: brackets, lists, blocks, function literals in
/// calls (3 levels each), interpolations, unary operators
fn deep_srcs(n: usize) -> [String; 6] {
    [
        format!("x = {}1{}", "(".repeat(n), ")".repeat(n)),
        format!("x = {}{}", "[".repeat(n), "]".repeat(n)),
        format!("{}{}", "for i in a { ".repeat(n), "}".repeat(n)),
        format!("{}{}", "f(fn() { ".repeat(n / 3), "})".repeat(n / 3)),
        format!("x = {}1{}", "\"${".repeat(n), "}\"".repeat(n)),
        format!("x = {}1", "-".repeat(n)),
    ]
}

pub fn run(t: &mut T) {
    example(t);
    precedence(t);
    strings(t);
    newlines(t);
    recovery(t);
    checks(t);
    tokens(t);
    robustness(t);
}

fn example(t: &mut T) {
    let p = script::parse("server.jfs", EXAMPLE);
    t.check("example parses", p.is_ok());
    let Ok(p) = p else { return };
    t.eq("example name", p.name.as_str(), "server.jfs");
    t.eq("example statements", p.body.len(), 4);
    t.eq("example fn key", p.body[2].sexpr().as_str(), r#"(fn key (id) (block (return (+ "user/" id))))"#);
    let serve = p.body[3].sexpr();
    t.check("example serve", serve.starts_with("(method http serve (?? (. config port) 80) (fn (req res) (block (+= hits 1) (if (== (. req path) \"/user\")"));
    t.check("example callback", serve.contains("(method disk get (call key id) (fn (err v) (block (if err (block (return (method res send 500 (interp err))))) (= (index cache id) v) (method res json 200 v))))"));
    t.check("example bare return", serve.contains("(return))"));
    t.check("example interpolation", serve.ends_with("(method res send 200 (interp \"hello #\" hits)))))"));
    // spans: line and column (characters) of the nodes
    if let StmtKind::Expr(Expr { kind: ExprKind::Method(_, m, args), .. }) = &p.body[3].kind {
        t.eq("span method name", (m.span.line, m.span.col), (4, 6));
        if let Some(ExprKind::Fn(f)) = args.get(1).map(|a| &a.kind) {
            t.eq("span fn literal", (f.span.line, f.span.col), (4, 31));
            t.eq("fn body statements", f.body.stmts.len(), 3);
            t.eq("span first statement", f.body.stmts.first().map(|s| (s.span.line, s.span.col)), Some((5, 3)));
            t.eq("span fn end", f.span.end as usize, EXAMPLE.len() - 2);
        } else {
            t.check("example fn literal", false);
        }
    } else {
        t.check("example method call", false);
    }
    t.eq("example check", script::check(EXAMPLE), Vec::new());
    let o = script::outline(EXAMPLE);
    let want = vec![(String::from("let hits"), 1), (String::from("let cache"), 2), (String::from("fn key(id)"), 3)];
    t.eq("outline", o, want);
}

fn precedence(t: &mut T) {
    let cases = [
        ("1 + 2 * 3", "(+ 1 (* 2 3))"),
        ("(1 + 2) * 3", "(* (+ 1 2) 3)"),
        ("a - b - c", "(- (- a b) c)"),
        ("a / b * c % d", "(% (* (/ a b) c) d)"),
        ("-a * b", "(* (- a) b)"),
        ("-a.b", "(- (. a b))"),
        ("!a == b", "(== (! a) b)"),
        ("!!a", "(! (! a))"),
        ("a + b < c * d", "(< (+ a b) (* c d))"),
        ("a < b == c >= d", "(== (< a b) (>= c d))"),
        ("a == b && c != d", "(&& (== a b) (!= c d))"),
        ("a || b && c", "(|| a (&& b c))"),
        ("a && b || c && d", "(|| (&& a b) (&& c d))"),
        ("a ?? b || c", "(?? a (|| b c))"),
        ("a ?? b ?? c", "(?? (?? a b) c)"),
        ("config.port ?? 80 + 1", "(?? (. config port) (+ 80 1))"),
        ("(a < b) < c", "(< (< a b) c)"),
        ("a.b.c(1)[2]", "(index (method (. a b) c 1) 2)"),
        ("f(a)(b)", "(call (call f a) b)"),
        ("f(a, g(b, c), [d])", "(call f a (call g b c) (list d))"),
        ("m[k].x(y, z,)", "(method (index m k) x y z)"),
        ("obj.if.for", "(. (. obj if) for)"),
        ("x = a + b", "(= x (+ a b))"),
        ("m.k += 1", "(+= (. m k) 1)"),
        ("a[i] -= 2 * 3", "(-= (index a i) (* 2 3))"),
        ("x *= 2; x /= 2; x %= 2", "(*= x 2)\n(/= x 2)\n(%= x 2)"),
        (r#"let m = {a: 1, "b": 2, [k]: 3, 4: true, if: nil}"#, r#"(let m (map ("a" 1) ("b" 2) (k 3) (4 true) ("if" nil)))"#),
        (r#"[1, 2.5, "s", nil, false,]"#, r#"(list 1 2.5 "s" nil false)"#),
        ("fn(x) { return x * 2 }", "(fn (x) (block (return (* x 2))))"),
        ("1_000 + 0xFF + 0b101 + 1.5e3 + 2e-1", "(+ (+ (+ (+ 1000 255) 5) 1500.0) 0.2)"),
        ("0xFFFF_FFFF_FFFF_FFFF", "-1"),
        ("9223372036854775807", "9223372036854775807"),
        ("for i in 0..n + 1 { }", "(for i (.. 0 (+ n 1)) (block))"),
    ];
    for (src, want) in cases {
        t.eq(src, sx(src).as_str(), want);
    }
    t.check("comparisons do not chain", sx("a < b < c").contains("nicht verketten"));
    t.check("equality does not chain", sx("a == b != c").contains("nicht verketten"));
    t.check("assign to a call", sx("f() = 1").contains("Vor = muss eine Variable"));
    t.check("assign in a condition", sx("if x = 1 { }").contains("schreibe == statt ="));
    t.check("range outside for", sx("let r = 0..3").contains("nur in for-Schleifen"));
    t.check("named fn value", sx("let f = fn g() { }").contains("keinen Namen"));
    t.check("int too big", sx("9223372036854775808").contains("zu groß"));
    t.check("bad number", sx("12abc").contains("Ungültige Zahl 12abc"));
}

fn strings(t: &mut T) {
    t.eq("escapes", str_of(r#""a\tb\n\"q\" \\ \u{e4}\u{1F600} \$ \r\0""#).as_str(), "a\tb\n\"q\" \\ ä😀 $ \r\0");
    t.eq("literal ${", str_of(r#""\${x}""#).as_str(), "${x}");
    t.eq("empty string", str_of(r#""""#).as_str(), "");
    t.eq("unicode text", str_of("\"größe € 😀\"").as_str(), "größe € 😀");
    let cases = [
        (r#"x = "x=${x + 1}!""#, r#"(= x (interp "x=" (+ x 1) "!"))"#),
        (r#"x = "${a}${b}""#, "(= x (interp a b))"),
        (r#"x = "a ${x} b ${y} c""#, r#"(= x (interp "a " x " b " y " c"))"#),
        (r#"x = "${"in${a}"}""#, r#"(= x (interp (interp "in" a)))"#),
        (r#"x = "${ {k: 1}["k"] }""#, r#"(= x (interp (index (map ("k" 1)) "k")))"#),
        (r#"x = "${f(", ")}""#, r#"(= x (interp (call f ", ")))"#),
        (r#"x = "\u{24}{y}""#, r#"(= x "${y}")"#),
    ];
    for (src, want) in cases {
        t.eq(src, sx(src).as_str(), want);
    }
    let e = errs("let s = \"abc\nlet t = 1");
    t.eq("unterminated string", e.len(), 1);
    t.check("unterminated message", e.first().is_some_and(|x| x.0 == 1 && x.1.starts_with("Zeichenkette ohne Ende")));
    t.check("bad escape", sx(r#"x = "a\qb""#).contains("Unbekannte Escape-Sequenz \\q"));
    t.check("bad unicode escape", sx(r#"x = "\u{110000}""#).contains("\\u{…}"));
    t.check("open interpolation", sx(r#"x = "a ${b""#).contains("Zeichenkette ohne Ende"));
    t.check("empty interpolation", sx(r#"x = "${}""#).contains("Leeres ${}"));
    let e = errs("x = \"a\n  b\"\ny = 2");
    t.eq("string over two lines", e.iter().map(|x| x.0).collect::<Vec<_>>(), vec![1, 2]);
}

fn newlines(t: &mut T) {
    let cases = [
        ("let a = 1\nlet b = 2", "(let a 1)\n(let b 2)"),
        ("let a = 1; let b = 2;", "(let a 1)\n(let b 2)"),
        ("let a = 1\r\nlet b = 2\r\n", "(let a 1)\n(let b 2)"),
        ("let a = 1 +\n  2", "(let a (+ 1 2))"),
        ("let a =\n  2", "(let a 2)"),
        ("f(1,\n  2\n)", "(call f 1 2)"),
        ("f(\n1,\n2)", "(call f 1 2)"),
        ("let m = {\n  a: 1,\n  b: [\n    2,\n  ],\n}", r#"(let m (map ("a" 1) ("b" (list 2))))"#),
        ("x\n-1", "x\n(- 1)"),
        ("(x\n-1)", "(- x 1)"),
        ("x\n.y()\n.z", "(. (method x y) z)"),
        ("a\n?? b\n|| c", "(?? a (|| b c))"),
        ("f\n(1)", "f\n1"),
        ("a\n[1]", "a\n(list 1)"),
        ("if a {\n  b\n}\nelse {\n  c\n}", "(if a (block b) else (block c))"),
        ("if a { b } else if c { d } else { e }", "(if a (block b) c (block d) else (block e))"),
        ("fn f() {\n  return\n  x\n}", "(fn f () (block (return) x))"),
        ("try { a }\ncatch e { b }", "(try (block a) e (block b))"),
        ("try { a } catch { b }", "(try (block a) _ (block b))"),
        ("/* a\n b */ x /* c */ + 1 // d", "(+ x 1)"),
        ("x /* line\nbreak */ -1", "x\n(- 1)"),
        ("for k, v in m { continue }", "(for (k v) m (block continue))"),
        ("while x < 3 { x += 1; break }", "(while (< x 3) (block (+= x 1) break))"),
        ("{ let a = 1 }", "(block (let a 1))"),
        ("throw \"e\"", "(throw \"e\")"),
        ("let a", "(let a)"),
        ("x = fn(a) {\n  return a\n}(5)", "(= x (call (fn (a) (block (return a))) 5))"),
        ("if a\n{\n}", "(if a (block))"),
        ("", ""),
    ];
    for (src, want) in cases {
        t.eq(src, sx(src).as_str(), want);
    }
    t.check("two statements on a line", sx("let a = 1 let b = 2").contains("Unerwartetes Schlüsselwort let"));
    t.check("throw without value", sx("throw\nx").contains("Nach throw fehlt ein Wert"));
}

fn recovery(t: &mut T) {
    // three broken lines between good ones: one error each, the good statements stay
    let src = "let a = 1\nlet b = (2 +\nlet c = 3\nlet = 4\nfn ok() { return 1 }\nprint(a b)\nlet d = 5\n";
    let (p, d) = script::parse_partial("t.jfs", src);
    t.eq("recovery lines", d.iter().map(|x| x.line).collect::<Vec<_>>(), vec![2, 4, 6]);
    t.eq("recovery columns", d.iter().map(|x| x.col).collect::<Vec<_>>(), vec![13, 5, 9]);
    t.check("recovery all errors", d.iter().all(|x| x.error));
    t.eq("recovery messages", d.iter().map(|x| x.msg.as_str()).collect::<Vec<_>>(),
         vec!["Hier fehlt ein Ausdruck", "Nach let fehlt ein Name", "Hier fehlt ein Komma , oder eine schließende Klammer )"]);
    t.eq("recovery tree", p.sexpr().as_str(), "(let a 1)\n(let b (+ 2 nil))\n(let c 3)\n(fn ok () (block (return 1)))\n(let d 5)");
    t.check("parse fails", script::parse("t.jfs", src).is_err());

    // errors inside the callbacks of the example: the rest still parses
    let broken = EXAMPLE.replace("req.query.id", "req.query.").replace("cache[id] = v", "cache[id] = = v");
    let (p, d) = script::parse_partial("t.jfs", &broken);
    t.eq("callback error lines", d.iter().map(|x| x.line).collect::<Vec<_>>(), vec![7, 11]);
    t.eq("callback statements", p.body.len(), 4);
    t.eq("callback outline", script::outline(&broken).len(), 3);
    t.check("callback let kept", p.sexpr().contains("(let id)"));

    // lexer errors
    let e = errs("let x = 1 @ 2\nlet y = \"abc\nlet z = 3");
    t.eq("lexer error lines", e.iter().map(|x| x.0).collect::<Vec<_>>(), vec![1, 2]);
    t.check("unexpected character", e.first().is_some_and(|x| x.1 == "Unerwartetes Zeichen @"));
    t.check("hint for &", sx("a & b").contains("schreibe &&"));

    // missing closing brackets
    let e = errs("fn f() {\n  let a = 1\n");
    t.check("missing } at the end", e.len() == 1 && e[0].1 == "Hier fehlt eine schließende geschweifte Klammer } (geöffnet in Zeile 1)");
    let e = errs("let a = [1, 2\nlet b = 3\nfoo(1\n}\n");
    t.eq("missing ] and ) lines", e.iter().map(|x| x.0).collect::<Vec<_>>(), vec![1, 3, 4]);
    t.check("missing ]", e.first().is_some_and(|x| x.1 == "Hier fehlt eine schließende eckige Klammer ]"));
    let (p, e) = script::parse_partial("t.jfs", "fn f(a, b {\n  return a\n}\n");
    t.eq("fn f(a, b {", e.len(), 1);
    t.eq("fn f(a, b { tree", p.sexpr().as_str(), "(fn f (a b) (block (return a)))");
    let (p, e) = script::parse_partial("t.jfs", "if x { foo(1 }\nlet y = 2\n");
    t.eq("missing ) before }", e.iter().map(|x| x.msg.as_str()).collect::<Vec<_>>(), vec!["Hier fehlt eine schließende Klammer )"]);
    t.eq("missing ) before } tree", p.sexpr().as_str(), "(if x (block (call foo 1)))\n(let y 2)");
    let e = errs("let m = {\n  a: 1\n  b: 2\n}\nlet z = 1\n");
    t.eq("missing comma in a map", e.iter().map(|x| x.0).collect::<Vec<_>>(), vec![2]);
    t.eq("outline with errors", script::outline("let a = (\nfn f(x, y) {\n"),
         vec![(String::from("let a"), 1), (String::from("fn f(x, y)"), 2)]);
    t.check("stray }", sx("}\nlet a = 1").contains("kein Block offen"));
    t.check("else without if", sx("else { }").contains("else ohne vorheriges if"));
}

fn checks(t: &mut T) {
    let lines = [
        "let a = 1",
        "let a = 2",
        "fn f(x, x) { return x }",
        "break",
        "return 5",
        "print(unknwn)",
        "while true { fn g() { continue } }",
        "for i in 0..3 { if i > 1 { break } }",
        "fn h() { return later }",
        "let later = 0",
        "print(late2)",
        "let late2 = 1",
        "let n = pirnt",
        "x = 5",
    ];
    let d = script::check(&lines.join("\n"));
    let got: Vec<(u32, bool)> = d.iter().map(|x| (x.line, x.error)).collect();
    let want = vec![(2, true), (3, true), (4, true), (5, true), (6, false), (7, true), (11, false), (13, false), (14, false)];
    t.eq("check findings", got, want);
    let msg = |line: u32| d.iter().find(|x| x.line == line).map(|x| x.msg.clone()).unwrap_or_default();
    t.eq("check duplicate", msg(2).as_str(), "a ist schon deklariert (Zeile 1)");
    t.eq("check parameter", msg(3).as_str(), "Der Parameter x kommt zweimal vor");
    t.eq("check break", msg(4).as_str(), "break steht außerhalb einer Schleife");
    t.eq("check return", msg(5).as_str(), "return steht außerhalb einer Funktion");
    t.eq("check unknown", msg(6).as_str(), "Unbekannter Name unknwn");
    t.eq("check continue in fn", msg(7).as_str(), "continue steht außerhalb einer Schleife");
    t.eq("check before let", msg(11).as_str(), "late2 wird benutzt, bevor es mit let angelegt wird (Zeile 12)");
    t.eq("check suggestion", msg(13).as_str(), "Unbekannter Name pirnt – meinst du print?");
    t.eq("check assign unknown", msg(14).as_str(), "Unbekannter Name x – eine neue Variable wird mit let x = … angelegt");
    t.eq("check positions", d.iter().find(|x| x.line == 6).map(|x| (x.col, x.end_col)), Some((7, 13)));

    let clean = [
        "fn a() { return b() }\nfn b() { return a() }",
        "fn outer() {\n  fn a() { return b() }\n  fn b() { return a() }\n  return a()\n}",
        "let v = 1\nfn f(v) {\n  let v = 2\n  return v\n}",
        "try { throw 1 } catch e { print(e) }",
        "for k, v in {a: 1} { print(k, v) }",
        "let a = 1\nif a { let a = 2 }",
        "let f = fn(n) { if n < 2 { return n } return f(n - 1) + f(n - 2) }",
        "while true { for i in 0..3 { break } continue }",
    ];
    for src in clean {
        t.eq(src, script::check(src), Vec::new());
    }
    let one = |src: &str| -> Vec<(u32, bool)> { script::check(src).iter().map(|x| (x.line, x.error)).collect() };
    t.eq("local before let", one("fn f() {\n  print(y)\n  let y = 1\n}"), vec![(2, false)]);
    t.eq("duplicate for variables", one("for k, k in m { }"), vec![(1, true), (1, false)]);
    t.eq("duplicate in a block", one("if true {\n  let q = 1\n  let q = 2\n}"), vec![(3, true)]);
    t.eq("syntax and names", one("let = 1\nprint(zz)"), vec![(1, true), (2, false)]);
    t.eq("check_with", script::check_with("host.ping()", &["host"]), Vec::new());
    t.eq("catch scope", one("try { } catch e { }\nprint(e)"), vec![(2, false)]);
}

fn tokens(t: &mut T) {
    t.check("tokens cover the example", covers(EXAMPLE));
    let tricky = "let s = \"a${b + \"c${d}\"}e\" /* x\ny */ @ 0x1G 1.5e3 \u{a0}é_1 \"open\r\n// end\n\"${ {} }\"";
    t.check("tokens cover errors", covers(tricky));
    use TokKind::*;
    let kinds: Vec<TokKind> = script::tokens("let x = \"a${b}c\" // hi").iter().map(|k| k.2).collect();
    t.eq("token kinds", kinds, vec![Let, Space, Ident, Space, Assign, Space, Str, Interp, Ident, Interp, Str, Space, Comment]);
    let kinds: Vec<TokKind> = script::tokens("x@1\n\"${a}${b}\"").iter().map(|k| k.2).collect();
    t.eq("token kinds 2", kinds, vec![Ident, Error, Int, Newline, Str, Interp, Ident, Interp, Interp, Ident, Interp, Str]);
    let ops: Vec<TokKind> = script::tokens("+=-=*=/=%===!=<=>=&&||??..").iter().map(|k| k.2).collect();
    t.eq("operators", ops, vec![PlusEq, MinusEq, StarEq, SlashEq, PercentEq, EqEq, NotEq, Le, Ge, AndAnd, OrOr, Coalesce, DotDot]);
    t.eq("classes", [Let.class(), True.class(), Str.class(), Interp.class(), Error.class(), AndAnd.class(), Ident.class(), Comment.class()],
         [TokClass::Keyword, TokClass::Constant, TokClass::Str, TokClass::Punct, TokClass::Error, TokClass::Op, TokClass::Ident, TokClass::Comment]);
    t.eq("unicode name", script::tokens("größe = 1").first().map(|k| (k.1, k.2)), Some((7, Ident)));
    let mut all = true;
    for src in [EXAMPLE, tricky] {
        for i in 0..=src.len() {
            if src.is_char_boundary(i) {
                all &= covers(&src[..i]);
            }
        }
    }
    t.check("tokens cover every prefix", all);
}

fn robustness(t: &mut T) {
    // every prefix of the example and random token soup: no panic, no endless loop
    let mut n = 0;
    for i in 0..=EXAMPLE.len() {
        n += script::check(&EXAMPLE[..i]).len();
    }
    t.check("prefixes checked", n > 0);
    let parts = ["let", "fn", "(", ")", "{", "}", "[", "]", "\"a${", "}\"", "\"", "x", "1", "+", "\n", ",", ".", "=",
                 "if", "else", "for", "in", "..", "return", "@", "/*", "*/", "//", " ", ":", "-", "try", "catch", "break"];
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut ok = true;
    for _ in 0..400 {
        let mut s = String::new();
        for _ in 0..48 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            s.push_str(parts[(seed % parts.len() as u64) as usize]);
            s.push(' ');
        }
        let _ = script::check(&s);
        let _ = script::outline(&s);
        ok &= covers(&s);
    }
    t.check("token soup", ok);
    // nesting: deep is an error, not a stack overflow
    let deep = format!("x = {}1{}", "(".repeat(5000), ")".repeat(5000));
    let e = errs(&deep);
    t.check("deep nesting", e.len() == 1 && e[0].1.starts_with("Zu tief verschachtelt"));
    let ok90 = format!("x = {}1{}", "(".repeat(90), ")".repeat(90));
    t.check("90 levels", script::parse("t.jfs", &ok90).is_ok());
    let blocks = format!("{}{}", "if a { ".repeat(3000), "} ".repeat(3000));
    t.check("deep blocks", errs(&blocks).first().is_some_and(|x| x.1.starts_with("Zu tief verschachtelt")));
    let chain = format!("x = 1{}", " + 1".repeat(5000));
    t.check("long chain", errs(&chain).first().is_some_and(|x| x.1.starts_with("Der Ausdruck ist zu lang")));
    let calls = format!("x = f{}", "()".repeat(5000));
    t.check("long call chain", errs(&calls).first().is_some_and(|x| x.1.starts_with("Der Ausdruck ist zu lang")));
    let unary = format!("x = {}1", "-".repeat(5000));
    t.check("long unary chain", errs(&unary).first().is_some_and(|x| x.1.starts_with("Zu tief verschachtelt")));
    // the deepest nesting allowed, parsed and checked on a thread with 256 KB of stack (exact
    // on Linux; Windows commits that much and reserves the default)
    let res = Arc::new(AtomicU32::new(0));
    let r2 = res.clone();
    let h = plat::thread_spawn(256 << 10, Box::new(move || {
        let srcs = deep_srcs(script::MAX_DEPTH as usize - 1);
        let mut bits = 1 << 31;
        for (i, s) in srcs.iter().enumerate() {
            if script::parse_partial("t.jfs", s).1.is_empty() && script::check(s).iter().all(|d| !d.error) {
                bits |= 1 << i;
            }
        }
        r2.store(bits, Ordering::SeqCst);
    }));
    if let Some(h) = h {
        h.join();
    }
    t.eq("deepest nesting on 256 KB", res.load(Ordering::SeqCst), (1 << 31) | 0b11_1111);
}
