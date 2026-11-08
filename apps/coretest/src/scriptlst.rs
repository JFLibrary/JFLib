// scriptlst -- checks of apps/jfs/crates/script's language services (SK6, script::ls) and formatter
// (script::fmt): definitions and references with shadowing, closures, loops and catch, rename
// edits and refusals, completion in scope, after a dot, of a module and on broken code, hover
// texts, signature help, folding ranges, the outline, the formatter on every syntax form,
// idempotence and the compiled code of formatted sources

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use script::ls::{self, CompleteCx, FoldKind, HostItem, ItemKind, SymKind};

use crate::T;

const SRC: &str = r#"/// zählt Aufrufe
let hits = 0
/// begrüßt
/// jemanden
fn greet(name, greeting) {
  let n = len(name)
  if n > 3 {
    let n = n * 2
    log(n)
  }
  return "${greeting} ${name} ${n}"
}
for i in 0..3 {
  let f = fn(x) { return x + i + hits }
  print(f(i))
}
try { greet("a", "b") } catch e { print(e) }
let s = "abc"
let parts = s.split(",")
"#;

/// the byte offset of the k-th (from 0) occurrence of needle, plus `off`
fn at(src: &str, needle: &str, k: usize, off: usize) -> u32 {
    let mut from = 0;
    for _ in 0..k {
        from = src[from..].find(needle).map_or(src.len(), |i| from + i + 1);
    }
    (src[from..].find(needle).map_or(src.len(), |i| from + i) + off) as u32
}

/// the lines of spans
fn lines(v: &[script::Span]) -> Vec<u32> {
    v.iter().map(|s| s.line).collect()
}

/// the labels of a completion
fn labels(c: &ls::Completion) -> Vec<&str> {
    c.items.iter().map(|i| i.label.as_str()).collect()
}

const HOST: &[HostItem] = &[
    HostItem { name: "log", params: Some("x, .."), doc: "Eine Zeile ins Log" },
    HostItem { name: "log.warn", params: Some("x, .."), doc: "Eine Warnung ins Log" },
    HostItem { name: "config", params: None, doc: "Die Einstellungen" },
    HostItem { name: "net.listen", params: Some("port, handlers"), doc: "Wartet auf TCP-Verbindungen" },
    HostItem { name: "net.udp", params: Some("port, f, opts"), doc: "Ein UDP-Port" },
];

pub fn run(t: &mut T) {
    index(t);
    rename(t);
    complete(t);
    hover(t);
    signature(t);
    folding(t);
    symbols(t);
    format(t);
    library(t);
}

fn index(t: &mut T) {
    let def = |pos: u32| ls::definition(SRC, pos, &CompleteCx::default()).map(|l| (l.span.line, l.span.col));
    // shadowing: the inner n in log(n), the outer one in the interpolation and on the right of
    // the inner let (its value is computed before it exists)
    t.eq("def inner n", def(at(SRC, "log(n)", 0, 4)), Some((8, 9)));
    t.eq("def outer n", def(at(SRC, "${n}", 0, 2)), Some((6, 7)));
    t.eq("def n in its own init", def(at(SRC, "n * 2", 0, 0)), Some((6, 7)));
    t.eq("def at a declaration", def(at(SRC, "let n", 0, 4)), Some((6, 7)));
    // closures and loops: the parameter, the loop variable, the global
    t.eq("def closure param", def(at(SRC, "x + i", 0, 0)), Some((14, 14)));
    t.eq("def loop var in closure", def(at(SRC, "i + hits", 0, 0)), Some((13, 5)));
    t.eq("def global in closure", def(at(SRC, "hits }", 0, 2)), Some((2, 5)));
    t.eq("def catch var", def(at(SRC, "print(e)", 0, 6)), Some((17, 31)));
    t.eq("def fn from top level", def(at(SRC, "greet(\"a\"", 0, 3)), Some((5, 4)));
    t.eq("def end of word", def(at(SRC, "greet(\"a\"", 0, 5)), Some((5, 4)));
    t.eq("def host name", def(at(SRC, "log(n)", 0, 1)), None);
    t.eq("def in comment", def(at(SRC, "zählt", 0, 1)), None);
    // references: the outer n (declaration, condition, inner init, interpolation)
    let r = ls::references(SRC, at(SRC, "let n", 0, 4));
    t.eq("refs outer n", lines(&r), alloc::vec![6, 7, 8, 11]);
    t.eq("refs inner n", lines(&ls::references(SRC, at(SRC, "log(n)", 0, 4))), alloc::vec![8, 9]);
    t.eq("refs greet", lines(&ls::references(SRC, at(SRC, "fn greet", 0, 4))), alloc::vec![5, 17]);
    t.eq("refs i", lines(&ls::references(SRC, at(SRC, "for i", 0, 4))), alloc::vec![13, 14, 15]);
    t.eq("refs none", ls::references(SRC, at(SRC, "len(", 0, 1)).len(), 0);
    // the index itself
    let idx = ls::index(SRC);
    let kinds: Vec<(&str, SymKind)> = idx.symbols.iter().map(|s| (s.name.as_str(), s.kind)).collect();
    t.check("index kinds", kinds.contains(&("hits", SymKind::Global)) && kinds.contains(&("greet", SymKind::Fn)) && kinds.contains(&("name", SymKind::Param)) && kinds.contains(&("i", SymKind::Loop)) && kinds.contains(&("e", SymKind::Catch)) && kinds.contains(&("f", SymKind::Local)));
    let greet = idx.symbols.iter().find(|s| s.name == "greet");
    t.eq("index params", greet.map(|s| s.params.clone()), Some(alloc::vec![String::from("name"), String::from("greeting")]));
    t.eq("index doc", greet.map(|s| s.doc.clone()), Some(String::from("begrüßt\njemanden")));
    let f = idx.symbols.iter().find(|s| s.name == "f");
    t.eq("index fn literal", f.map(|s| (s.ty, s.params.len())), Some(("fn", 1)));
    let x = idx.symbols.iter().find(|s| s.name == "x").and_then(|s| s.parent).map(|p| idx.symbols[p].name.as_str());
    t.eq("index parent of closure param", x, Some("f"));
    let parts = idx.symbols.iter().find(|s| s.name == "parts").map(|s| s.ty);
    t.eq("index type from split", parts, Some("list"));
    t.eq("index use write", idx.uses.iter().filter(|u| u.write).count(), 0);
    let w = ls::index("let a = 1\nfn f() { a = 2\n a += 1 }");
    t.eq("index writes", w.uses.iter().filter(|u| u.write && u.sym == Some(0)).count(), 2);
    // a name in parentheses: the use is the name itself
    let p = ls::index("let a = 1\nprint((a))");
    t.eq("use in parens", p.uses.iter().find(|u| u.sym == Some(0)).map(|u| (u.span.line, u.span.col, u.span.end - u.span.start)), Some((2, 8, 1)));
    // broken code: what parsed
    let b = "fn f(a) {\n  let bb = 1\n  print(bb +)\n  return a\n}";
    t.eq("def on broken code", ls::definition(b, at(b, "return a", 0, 7), &CompleteCx::default()).map(|l| l.span.line), Some(1));
}

fn rename(t: &mut T) {
    let n = at(SRC, "let n", 0, 4);
    match ls::rename(SRC, n, "count") {
        Ok(e) => {
            t.eq("rename edits", e.len(), 4);
            let out = ls::apply(SRC, &e);
            t.check("rename result", out.contains("let count = len(name)") && out.contains("if count > 3") && out.contains("let n = count * 2") && out.contains("${count}") && out.contains("log(n)"));
            t.check("rename compiles", script::compile(&out).is_ok());
        }
        Err(m) => t.check(&format!("rename: {}", m), false),
    }
    let g = ls::rename(SRC, at(SRC, "greet(\"a\"", 0, 1), "hello").map(|e| ls::apply(SRC, &e));
    t.check("rename fn", g.as_ref().is_ok_and(|s| s.contains("fn hello(name") && s.contains("try { hello(\"a\"")));
    t.eq("rename same", ls::rename(SRC, n, "n").map(|e| e.len()), Ok(0));
    let err = |pos: u32, new: &str| ls::rename(SRC, pos, new).err().unwrap_or_default();
    t.check("rename keyword", err(n, "while").contains("Schlüsselwort"));
    t.check("rename invalid", err(n, "1x").contains("kein gültiger Name"));
    t.check("rename invalid empty", err(n, "").contains("kein gültiger Name"));
    // the same scope, a use that would mean the renamed one, a renamed use that would mean
    // another declaration
    t.check("rename clash scope", err(at(SRC, "name, greeting", 0, 0), "greeting").contains("schon"));
    t.check("rename clash hides host", err(at(SRC, "greeting)", 0, 0), "len").contains("schon vergeben"));
    t.check("rename clash captured", err(at(SRC, "let hits", 0, 4), "x").contains("schon vergeben"));
    t.check("rename clash top", err(at(SRC, "let s ", 0, 4), "parts").contains("schon"));
    t.check("rename no symbol", err(at(SRC, "log(n)", 0, 1), "y").contains("kein Name"));
    t.check("rename broken", ls::rename("let a = 1\nprint(a +)", 4, "b").err().unwrap_or_default().contains("Syntaxfehler"));
    // a loop variable and a catch variable
    let l = ls::rename(SRC, at(SRC, "for i", 0, 4), "k").map(|e| ls::apply(SRC, &e));
    t.check("rename loop var", l.as_ref().is_ok_and(|s| s.contains("for k in 0..3") && s.contains("x + k + hits") && s.contains("print(f(k))")));
    let c = ls::rename(SRC, at(SRC, "catch e", 0, 6), "err").map(|e| ls::apply(SRC, &e));
    t.check("rename catch var", c.as_ref().is_ok_and(|s| s.contains("catch err { print(err) }")));
}

fn complete(t: &mut T) {
    let cx = CompleteCx::default();
    // in scope inside greet (the empty word on the line of log): the names of greet, the
    // globals, the library, statement keywords; not the loop's or the closure's
    let src = "let hits = 0\nfn greet(name, greeting) {\n  let n = len(name)\n  \n}\nfor i in 0..3 { }\n";
    let c = ls::complete(src, at(src, "  \n}", 0, 2), &cx);
    let l = labels(&c);
    t.check("complete scope", ["name", "greeting", "n", "hits", "greet", "print", "len", "str", "math", "let", "return", "nil"].iter().all(|x| l.contains(x)));
    t.check("complete scope excludes", !l.contains(&"i") && !l.contains(&"x"));
    t.check("complete locals first", l.iter().position(|x| *x == "n").unwrap_or(99) < l.iter().position(|x| *x == "hits").unwrap_or(0) && l.iter().position(|x| *x == "hits").unwrap_or(99) < l.iter().position(|x| *x == "print").unwrap_or(0) && l.iter().position(|x| *x == "print").unwrap_or(99) < l.iter().position(|x| *x == "let").unwrap_or(0));
    // a prefix: replace range, filter, exact case first, rank by kind
    let src2 = "fn greet(gname) {\n  gr\n}\nlet Grund = 1";
    let c = ls::complete(src2, at(src2, "gr\n", 0, 2), &cx);
    t.eq("complete prefix range", (c.start, c.end), (at(src2, "gr\n", 0, 0), at(src2, "gr\n", 0, 2)));
    t.eq("complete prefix", labels(&c), alloc::vec!["greet", "Grund"]);
    let c = ls::complete(src2, at(src2, "gr\n", 0, 1), &cx);
    t.check("complete prefix inside word", labels(&c).contains(&"gname") && labels(&c).contains(&"greet"));
    let item = c.items.iter().find(|i| i.label == "greet");
    t.eq("complete fn item", item.map(|i| (i.kind, i.detail.as_str())), Some((ItemKind::Function, "fn greet(gname)")));
    // after a dot: by the type of a literal, unknown types: all, a module, the host
    let d = |src: &str, cx: &CompleteCx| -> Vec<String> { ls::complete(src, src.len() as u32, cx).items.into_iter().map(|i| i.label).collect() };
    let s = d("let s = \"abc\"\ns.", &cx);
    t.check("complete str methods", s.contains(&String::from("split")) && s.contains(&String::from("upper")) && !s.contains(&String::from("push")));
    let l2 = d("let p = \"a,b\".split(\",\")\np.", &cx);
    t.check("complete list methods", l2.contains(&String::from("push")) && l2.contains(&String::from("map")) && !l2.contains(&String::from("upper")));
    let u = d("fn f(x) { x.", &cx);
    t.check("complete unknown type", u.contains(&String::from("push")) && u.contains(&String::from("split")) && u.contains(&String::from("keys")) && u.contains(&String::from("hex")));
    let item = ls::complete("fn f(x) { x.sl", 14, &cx).items.into_iter().find(|i| i.label == "slice");
    t.check("complete merged detail", item.is_some_and(|i| i.kind == ItemKind::Method && i.detail.contains("s.slice(start, end)") && i.detail.contains("l.slice(start, end)")));
    let m = d("math.", &cx);
    t.check("complete module", m.contains(&String::from("sqrt")) && m.contains(&String::from("pi")) && !m.contains(&String::from("split")));
    let k = ls::complete("math.p", 6, &cx);
    t.eq("complete module prefix", labels(&k), alloc::vec!["pi", "pow"]);
    t.eq("complete constant kind", k.items.first().map(|i| i.kind), Some(ItemKind::Constant));
    let hcx = CompleteCx { host: HOST, load: None };
    let n = ls::complete("net.", 4, &hcx);
    t.eq("complete host members", labels(&n), alloc::vec!["listen", "udp"]);
    t.eq("complete host detail", n.items.first().map(|i| (i.detail.as_str(), i.doc.as_str())), Some(("net.listen(port, handlers)", "Wartet auf TCP-Verbindungen")));
    let top = ls::complete("", 0, &hcx);
    let kind = |l: &str| top.items.iter().find(|i| i.label == l).map(|i| i.kind);
    t.eq("complete host kinds", (kind("net"), kind("config"), kind("log")), (Some(ItemKind::Module), Some(ItemKind::Value), Some(ItemKind::Function)));
    t.check("complete log members", d("log.", &hcx).contains(&String::from("warn")));
    // a script's name hides the host's and the library's
    let sh = ls::complete("let net = [1]\nfn f() { ne", 25, &hcx);
    t.eq("complete shadowed host", sh.items.iter().filter(|i| i.label == "net").map(|i| i.kind).collect::<Vec<_>>(), alloc::vec![ItemKind::Variable]);
    let shm = d("let list = \"x\"\nlist.", &cx);
    t.check("complete shadowed module", shm.contains(&String::from("upper")) && !shm.contains(&String::from("push")));
    // broken code: what parsed still gives the scope
    let b = "fn f(a) {\n  let bb = 1\n  b";
    t.check("complete broken scope", labels(&ls::complete(b, b.len() as u32, &cx)).contains(&"bb"));
    let b2 = "fn f(a) {\n  let bb = [1, 2\n  a";
    t.check("complete broken bracket", labels(&ls::complete(b2, b2.len() as u32, &cx)).contains(&"a"));
    let b3 = "fn f(req) {\n  if req.path == \"/\" {\n    req.";
    t.check("complete broken member", d(b3, &cx).contains(&String::from("len")));
    // nothing in comments and strings, but in ${..}
    t.eq("complete comment", ls::complete("// pr", 5, &cx).items.len(), 0);
    t.eq("complete block comment", ls::complete("/* pr */", 5, &cx).items.len(), 0);
    t.eq("complete string", ls::complete("let a = \"pr", 11, &cx).items.len(), 0);
    t.eq("complete string closed", ls::complete("let a = \"pr\"", 11, &cx).items.len(), 0);
    t.check("complete interp", labels(&ls::complete("let x = 1\nprint(\"${x", 20, &cx)).contains(&"x"));
    // keywords: statements at a statement's start, values elsewhere, else after }, in after for x
    let kw = |src: &str| -> Vec<String> { ls::complete(src, src.len() as u32, &cx).items.into_iter().filter(|i| i.kind == ItemKind::Keyword).map(|i| i.label).collect() };
    t.check("complete stmt keywords", kw("let a = 1\n").contains(&String::from("while")));
    t.check("complete expr keywords", { let k = kw("let a = "); k.contains(&String::from("true")) && !k.contains(&String::from("while")) });
    t.check("complete continuation", !kw("let a = 1 +\n").contains(&String::from("while")));
    t.check("complete else", kw("if a { } ").contains(&String::from("else")));
    t.check("complete in", kw("for k, v ").contains(&String::from("in")) && kw("for x ").contains(&String::from("in")));
    // an imported module (cx.load reads it)
    let lib = "/// hilft\nfn helper(a) { return a }\nlet VERSION = 2\nfn _inner() { fn deep() {} }\n";
    let load = |p: &str| if p == "lib.jfs" { Some(String::from(lib)) } else { None };
    let mcx = CompleteCx { host: &[], load: Some(&load) };
    let main = "let m = import(\"lib.jfs\")\nm.";
    let mm = d(main, &mcx);
    t.check("complete import", mm.contains(&String::from("helper")) && mm.contains(&String::from("VERSION")) && !mm.contains(&String::from("deep")));
    let main2 = "let m = import(\"lib.jfs\")\nm.helper(1)";
    let loc = ls::definition(main2, at(main2, "helper", 0, 2), &mcx);
    t.eq("definition import", loc.map(|l| (l.path, l.span.line, l.span.col)), Some((Some(String::from("lib.jfs")), 2, 4)));
    let hv = ls::hover(main2, at(main2, "helper", 0, 2), &mcx);
    t.eq("hover import", hv.map(|h| h.text()), Some(String::from("fn helper(a)\nFunktion aus lib.jfs\nhilft")));
    t.eq("hover module", ls::hover(main2, at(main2, "m.h", 0, 0), &mcx).map(|h| h.sig), Some(String::from("let m = import(\"lib.jfs\")")));
    t.eq("import missing", ls::complete("let m = import(\"x.jfs\")\nm.", 26, &mcx).items.len(), 0);
}

fn hover(t: &mut T) {
    let cx = CompleteCx { host: HOST, load: None };
    let h = |pos: u32| ls::hover(SRC, pos, &cx).map(|x| x.text());
    t.eq("hover fn", h(at(SRC, "greet(\"a\"", 0, 2)), Some(String::from("fn greet(name, greeting)\nFunktion\nbegrüßt\njemanden")));
    t.eq("hover global", h(at(SRC, "hits }", 0, 1)), Some(String::from("let hits\nglobale Variable\nzählt Aufrufe")));
    t.eq("hover param", h(at(SRC, "len(name)", 0, 5)), Some(String::from("name\nParameter von greet")));
    t.eq("hover local", h(at(SRC, "${n}", 0, 2)), Some(String::from("let n\nlokale Variable in greet")));
    t.eq("hover loop", h(at(SRC, "f(i)", 0, 2)), Some(String::from("for i\nSchleifenvariable")));
    t.eq("hover catch", h(at(SRC, "print(e)", 0, 6)), Some(String::from("catch e\nFehlervariable")));
    t.eq("hover fn literal", h(at(SRC, "print(f(", 0, 6)), Some(String::from("let f = fn(x)\nlokale Variable")));
    t.eq("hover closure param", h(at(SRC, "x + i", 0, 0)), Some(String::from("x\nParameter von f")));
    let split = ls::hover(SRC, at(SRC, "split", 0, 2), &cx);
    t.eq("hover method", split.as_ref().map(|x| (x.sig.as_str(), x.kind.as_str())), Some(("s.split(sep, n)", "Standardbibliothek")));
    t.check("hover method doc", split.is_some_and(|x| x.doc.starts_with("Zerlegt") && x.span.line == 19));
    t.eq("hover builtin", ls::hover(SRC, at(SRC, "len(", 0, 1), &cx).map(|x| (x.sig, x.kind)), Some((String::from("len(x)"), String::from("eingebaut"))));
    t.eq("hover host", ls::hover(SRC, at(SRC, "log(n)", 0, 1), &cx).map(|x| x.sig), Some(String::from("log(x, ..)")));
    let src = "net.listen(80, {})\nmath.pi\nlet l = []\nl.push(1)\nstr.from_code(65)";
    t.eq("hover host member", ls::hover(src, 6, &cx).map(|x| x.text()), Some(String::from("net.listen(port, handlers)\nName des Hosts\nWartet auf TCP-Verbindungen")));
    t.eq("hover constant", ls::hover(src, at(src, "pi", 0, 1), &cx).map(|x| x.sig), Some(String::from("math.pi")));
    t.eq("hover list method", ls::hover(src, at(src, "push", 0, 1), &cx).map(|x| x.sig), Some(String::from("l.push(x, ..)")));
    t.eq("hover module fn", ls::hover(src, at(src, "from_code", 0, 1), &cx).map(|x| x.sig), Some(String::from("str.from_code(n, ..)")));
    t.eq("hover namespace", ls::hover(src, 1, &cx).map(|x| x.kind), Some(String::from("Name des Hosts")));
    t.eq("hover module", ls::hover(src, at(src, "math", 0, 1), &cx).map(|x| x.kind), Some(String::from("Modul der Standardbibliothek")));
    t.eq("hover keyword", ls::hover(SRC, at(SRC, "return", 0, 1), &cx), None);
    t.eq("hover unknown", ls::hover("pirnt(1)", 1, &cx), None);
}

fn signature(t: &mut T) {
    let cx = CompleteCx { host: HOST, load: None };
    let sig = |src: &str| ls::signature(src, src.len() as u32, &cx).map(|s| {
        let act = String::from(s.params.get(s.active).map_or("-", |&(a, b)| &s.label[a as usize..b as usize]));
        (s.label, act)
    });
    let base = "fn greet(name, greeting) { }\nlet s = \"a\"\n";
    let w = |x: &str| format!("{}{}", base, x);
    t.eq("sig fn first", sig(&w("greet(")), Some((String::from("greet(name, greeting)"), String::from("name"))));
    t.eq("sig fn second", sig(&w("greet(\"a\", ")), Some((String::from("greet(name, greeting)"), String::from("greeting"))));
    t.eq("sig too many", sig(&w("greet(1, 2, ")), Some((String::from("greet(name, greeting)"), String::from("-"))));
    t.eq("sig method", sig(&w("s.split(")), Some((String::from("s.split(sep, n)"), String::from("sep"))));
    t.eq("sig module", sig(&w("math.pow(2, ")), Some((String::from("math.pow(x, y)"), String::from("y"))));
    t.eq("sig variadic", sig(&w("print(1, 2, ")), Some((String::from("print(x, ..)"), String::from("x"))));
    t.eq("sig host", sig(&w("net.listen(80, ")), Some((String::from("net.listen(port, handlers)"), String::from("handlers"))));
    t.eq("sig nested", sig(&w("greet(len(")), Some((String::from("len(x)"), String::from("x"))));
    t.eq("sig after inner call", sig(&w("greet(len(s), ")), Some((String::from("greet(name, greeting)"), String::from("greeting"))));
    t.eq("sig list arg", sig(&w("greet([1, 2], ")), Some((String::from("greet(name, greeting)"), String::from("greeting"))));
    t.eq("sig in list", sig(&w("greet([1, ")), None);
    t.eq("sig interp arg", sig(&w("greet(\"${s}, ${s}\", ")), Some((String::from("greet(name, greeting)"), String::from("greeting"))));
    t.eq("sig outside", sig(&w("greet(1)")), None);
    t.eq("sig in string", sig(&w("greet(\"a, ")), None);
    t.eq("sig no fn", sig(&w("let q = (")), None);
    t.eq("sig doc", ls::signature(&w("s.find("), (base.len() + 7) as u32, &cx).map(|s| s.doc.starts_with("Wo sub")), Some(true));
    let p = ls::signature(&w("greet("), (base.len() + 6) as u32, &cx).map(|s| s.params);
    t.eq("sig ranges", p, Some(alloc::vec![(6, 10), (12, 20)]));
}

fn folding(t: &mut T) {
    let src = "/* a\n   b */\nfn f(a) {\n  if a {\n    print(1)\n  }\n  let m = {\n    k: 1\n  }\n}\n// one\n// two\nprint(f(\n  1\n))\nlet x = [1] // x\n// alone\n";
    let f: Vec<(u32, u32, FoldKind)> = ls::folding(src).iter().map(|x| (x.line, x.end, x.kind)).collect();
    t.eq(
        "folding",
        f,
        alloc::vec![(1, 2, FoldKind::Comment), (3, 9, FoldKind::Block), (4, 5, FoldKind::Block), (7, 8, FoldKind::Literal), (11, 12, FoldKind::Comment), (13, 14, FoldKind::Literal)],
    );
    // a closer that does not start its line folds with the rest; one-line blocks do not fold
    let g: Vec<(u32, u32)> = ls::folding("if a {\n  b() }\nwhile c { d() }").iter().map(|x| (x.line, x.end)).collect();
    t.eq("folding closer inline", g, alloc::vec![(1, 2)]);
    // else blocks, broken code
    let h: Vec<(u32, u32)> = ls::folding("if a {\n  x()\n} else {\n  y()\n}").iter().map(|x| (x.line, x.end)).collect();
    t.eq("folding else", h, alloc::vec![(1, 2), (3, 4)]);
    let b: Vec<(u32, u32)> = ls::folding("fn f() {\n  let a = (\n  b()\n}").iter().map(|x| (x.line, x.end)).collect();
    t.check("folding broken", b.first() == Some(&(1, 3)));
}

fn symbols(t: &mut T) {
    let src = "let a = 1\nfn outer(x) {\n  fn inner(y) { return y }\n  if x { fn deeper() {} }\n  return inner(x)\n}\nlet h = fn(r) {\n  fn deep() {}\n}\nprint(fn() { fn anon_inner() {} })\n";
    let s = ls::symbols(src);
    let top: Vec<(&str, SymKind, &str)> = s.iter().map(|d| (d.name.as_str(), d.kind, d.detail.as_str())).collect();
    t.eq("symbols top", top, alloc::vec![("a", SymKind::Global, "let a"), ("outer", SymKind::Fn, "fn outer(x)"), ("h", SymKind::Global, "let h = fn(r)"), ("anon_inner", SymKind::Fn, "fn anon_inner()")]);
    let kids = |i: usize| -> Vec<&str> { s.get(i).map_or(Vec::new(), |d| d.children.iter().map(|c| c.name.as_str()).collect()) };
    t.eq("symbols nested", kids(1), alloc::vec!["inner", "deeper"]);
    t.eq("symbols in fn literal", kids(2), alloc::vec!["deep"]);
    t.eq("symbols positions", s.get(1).map(|d| (d.span.line, d.span.col, d.range.line, d.range.end as usize)), Some((2, 4, 2, src.find("}\nlet h").unwrap_or(0) + 1)));
    let broken = ls::symbols("fn ok() {}\nfn bad( {\n}\nlet z = 1");
    t.eq("symbols broken", broken.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), alloc::vec!["ok", "bad", "z"]);
}

/// the code of a program without the positions of its instructions
fn code_of(src: &str) -> Option<String> {
    let p = script::compile(src).ok()?;
    let mut out = String::new();
    for l in p.disasm().lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.first().is_some_and(|x| x.chars().all(|c| c.is_ascii_digit())) && f.len() > 2 {
            out.push_str(&format!("{} {}\n", f[0], f[2..].join(" ")));
        } else {
            out.push_str(l);
            out.push('\n');
        }
    }
    Some(out)
}

fn format(t: &mut T) {
    let cases: &[(&str, &str)] = &[
        ("let a=1;let b=2", "let a = 1\nlet b = 2"),
        ("fn f(a,b){return a+b}", "fn f(a, b) {\n  return a + b\n}"),
        ("if a{b()}else if c{d()}else{e()}", "if a {\n  b()\n} else if c {\n  d()\n} else {\n  e()\n}"),
        ("while x<3{x+=1}", "while x < 3 {\n  x += 1\n}"),
        ("for k,v in m{print(k,v)}", "for k, v in m {\n  print(k, v)\n}"),
        ("for i in 0..n{}", "for i in 0..n {}"),
        ("try{throw \"x\"}catch e{print(e)}", "try {\n  throw \"x\"\n} catch e {\n  print(e)\n}"),
        ("try { f() } catch { }", "try {\n  f()\n} catch {}"),
        ("let m={a:1,\"b\":[1,2,-3],[k]:nil, 5:true}", "let m = {a: 1, \"b\": [1, 2, -3], [k]: nil, 5: true}"),
        ("let x=-a*(b-c)+!d%2/e", "let x = -a * (b - c) + !d % 2 / e"),
        ("let y = a - -b", "let y = a - -b"),
        ("x . y.z( 1 )[ 2 ].w", "x.y.z(1)[2].w"),
        ("a[i]=b.c ;a[i] -= 1", "a[i] = b.c\na[i] -= 1"),
        ("print( \"a${ x+1 }b${y}\" )", "print(\"a${x + 1}b${y}\")"),
        ("net.udp(7,fn(s,m){s.send(m)})", "net.udp(7, fn(s, m) {\n  s.send(m)\n})"),
        ("let a = 1   // eins\n\n\n// zwei\nlet b = 2 /* drei */", "let a = 1 // eins\n\n// zwei\nlet b = 2 /* drei */"),
        ("let l = [\n1,\n2\n]", "let l = [\n  1,\n  2\n]"),
        ("let x = a +\nb", "let x = a +\n  b"),
        ("let y = l\n.map(f)\n.filter(g)", "let y = l\n  .map(f)\n  .filter(g)"),
        ("while true { if a { break } continue }", "while true {\n  if a {\n    break\n  }\n  continue\n}"),
        ("if a {\n}\nelse {\n}", "if a {} else {}"),
        ("fn f() {\n\n\n  let a = 1\n\n\n  let b = 2\n\n}", "fn f() {\n  let a = 1\n\n  let b = 2\n}"),
        ("let a = 1\n", "let a = 1\n"),
        ("\n\nlet a = 1\n\n\n", "let a = 1\n"),
        ("fn h() { return {status:200} }", "fn h() {\n  return {status: 200}\n}"),
        ("let z=a??b&&c||d==e", "let z = a ?? b && c || d == e"),
        ("fn f()\n{\n  return\n}", "fn f() {\n  return\n}"),
        ("{ let a = 1 }", "{\n  let a = 1\n}"),
        ("if a { // note\n  b()\n}", "if a { // note\n  b()\n}"),
        ("fn f() {\n  x()\n    // last\n}", "fn f() {\n  x()\n  // last\n}"),
        ("/* head\n   more */\nlet a = 1", "/* head\n   more */\nlet a = 1"),
        ("let s = \"${ {k: 1}.k }\"", "let s = \"${{k: 1}.k}\""),
        ("let f = fn() {}", "let f = fn() {}"),
        ("http.serve(80, fn(req, res) {\n  if req.path == \"/\" { return 404 }\n  return \"ok\"\n})\nlog(\"x\")", "http.serve(80, fn(req, res) {\n  if req.path == \"/\" {\n    return 404\n  }\n  return \"ok\"\n})\nlog(\"x\")"),
        ("let cfg = {\n  port: 80,\n  names: [\"a\",\n    \"b\"]\n}", "let cfg = {\n  port: 80,\n  names: [\"a\",\n    \"b\"]\n}"),
        ("foo(a,\nb)", "foo(a,\n  b)"),
        ("x = (1 +\n2) * 3", "x = (1 +\n  2) * 3"),
        ("print(1);;print(2);", "print(1)\nprint(2)"),
        ("let n = 0x1F + 1_000 + 2.5e-3", "let n = 0x1F + 1_000 + 2.5e-3"),
    ];
    for (i, (src, want)) in cases.iter().enumerate() {
        let got = script::fmt::format(src);
        t.eq(&format!("format {}", i), got.as_deref(), Ok(*want));
        if let Ok(g) = &got {
            t.eq(&format!("format idempotent {}", i), script::fmt::format(g).as_deref(), Ok(g.as_str()));
        }
    }
    // another unit
    t.eq("format unit", script::fmt::format_with("fn f() { if a { b() } }", "    ").as_deref(), Ok("fn f() {\n    if a {\n        b()\n    }\n}"));
    t.eq("format tabs", script::fmt::format_with("fn f() { b() }", "\t").as_deref(), Ok("fn f() {\n\tb()\n}"));
    // refused: syntax errors (the source as it is)
    t.check("format refuses", script::fmt::format("let = 1\nprint(").is_err());
    t.check("format refuses errors", script::fmt::format("print(1 +)").err().is_some_and(|d| d.first().is_some_and(|x| x.error)));
    // whole programs: idempotent, the same tree and the same code
    let progs = [SRC, BIG, SERVER];
    for (i, p) in progs.iter().enumerate() {
        match script::fmt::format(p) {
            Ok(f) => {
                t.eq(&format!("format prog idempotent {}", i), script::fmt::format(&f).as_deref(), Ok(f.as_str()));
                t.eq(&format!("format prog tree {}", i), script::parse("", &f).map(|x| x.sexpr()).ok(), script::parse("", p).map(|x| x.sexpr()).ok());
                t.check(&format!("format prog code {}", i), code_of(p).is_some() && code_of(&f) == code_of(p));
                t.check(&format!("format prog no tabs or trailing blanks {}", i), f.lines().all(|l| !l.ends_with(' ') && !l.contains('\t')));
            }
            Err(d) => t.check(&format!("format prog {}: {:?}", i, d.first().map(|x| x.msg.clone())), false),
        }
    }
    t.eq("format formatted unchanged", script::fmt::format(SERVER).as_deref(), Ok(SERVER));
    // the layout of BIG
    t.check("format big layout", script::fmt::format(BIG).is_ok_and(|f| f.contains("\n  for i in 0..3 {\n    log(\"i=${i}\")\n  }\n") && f.contains("return {status: 200, body: \"${hits} Aufrufe \\u{2713}\"}")));
}

/// a key-value server as JFNetworker's templates write them (already formatted)
const SERVER: &str = r#"let port = config.port ?? 80
http.serve(port, fn(req, res) {
  if !req.path.starts_with("/kv/") {
    return 404
  }
  let key = req.path.slice(4)
  if req.method == "GET" {
    disk.get(key, fn(err, v) {
      if v == nil {
        return res.send(404, "kein Wert\n")
      }
      res.send(200, "${v}\n")
    })
  } else {
    return 405
  }
})
log("Speicher bereit auf Port ${port}")"#;

/// a script with every construct, unevenly laid out
const BIG: &str = r#"// JFS: a request counter
let hits = 0
let names = ["web","db"]

fn greet(name) {
    let n = len(name)
    return "Hallo ${name}, ${n} Zeichen: ${ {k: n}.k }\n"
}

/* the handler counts the requests
   and answers in JSON */
fn handle(req) {
    hits += 1
    if req.path == "/count" && hits > 0x10 {
        return {status: 200, body: "${hits} Aufrufe \u{2713}"}
    }
    for i in 0..3 { log("i=${i}") }
    let big = 1_000_000 + 0b1010 + 1.5e-3
    while big > 0 { big -= 1; if big % 7 == 0 { continue } ; if big < 3 { break } }
    try { throw {code: -1} } catch e { log(e.code) }
    for k, v in {a: 1} { names.push(k) }
    let f = fn(x, y) { return x * y - -x }
    { let inner = f(2, 3) ; names[0] = "${inner}" }
    return nil ?? greet("du") || !true
}
let total = names
  .map(fn(n) { return len(n) })
  .reduce(fn(a, b) { return a + b }, 0)
print(handle({path: "/count"}), total)
"#;

fn library(t: &mut T) {
    let names = script::library();
    let missing: Vec<&str> = names.iter().copied().filter(|n| ls::library_doc(n).is_none()).collect();
    t.eq("library docs", missing, Vec::<&str>::new());
    t.check("library doc text", ls::library_doc("str.split").is_some_and(|(p, d)| p == Some("sep, n") && d.starts_with("Zerlegt")));
    t.eq("library constant", ls::library_doc("math.pi").map(|x| x.0), Some(None));
    t.check("is_name", ls::is_name("größe") && ls::is_name("_x1") && !ls::is_name("1x") && !ls::is_name("let") && !ls::is_name("a-b"));
}
