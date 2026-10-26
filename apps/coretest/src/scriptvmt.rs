// scriptvmt -- checks of lib/script's compiler and VM (SK2): semantics of the language,
// closures, recursion, exceptions across calls, error positions and traces, map order,
// bit-identical float arithmetic (num against the hardware, exact square roots, vectors from
// Python), the host interface, the API, the collector under pressure, fuel and the limits,
// yield / resume at every instruction against an uninterrupted run, compile errors, the
// deepest nesting on a small stack

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::sync::atomic::{AtomicU32, Ordering as AtOrd};

use script::{num, Cx, ErrorKind, Fail, Host, Limits, Pin, Status, Value, Vm};

use crate::T;

/// natives for the checks (names of the standard library, such as list.map or str.upper,
/// never get here: the library answers them first, scriptlibt checks it)
#[derive(Default)]
struct TestHost {
    log: Vec<String>,
    pins: Vec<Pin>,
}

impl Host for TestHost {
    fn call(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        let arg = |i: usize| args.get(i).copied().unwrap_or(Value::Nil);
        match name {
            "add" => Ok(Value::Int(args.iter().map(|a| if let Value::Int(i) = a { *i } else { 0 }).sum())),
            "fail" => {
                let m = format!("host: {}", cx.display(arg(0)));
                Err(cx.error(&m))
            }
            "fatal" => Err(Fail::Fatal(String::from("Host-Abbruch"))),
            "bad" => Ok(Value::Native(9999)),
            "pause" => {
                cx.yield_now();
                Ok(Value::Int(7))
            }
            "burn" => {
                cx.charge(1000);
                Ok(Value::Nil)
            }
            "log" => {
                self.log.push(cx.display(arg(0)));
                Ok(Value::Nil)
            }
            "mk" => {
                let m = cx.new_map()?;
                let s = cx.new_str("eins")?;
                cx.map_set(m, s, Value::Int(1))?;
                let l = cx.new_list(vec![Value::Int(1), s])?;
                let k = cx.new_str("liste")?;
                cx.map_set(m, k, l)?;
                Ok(m)
            }
            "http.serve" => {
                self.pins.push(cx.pin(arg(1)));
                Ok(Value::Nil)
            }
            "list.sum" => {
                let l = cx.list(arg(0)).unwrap_or(&[]);
                Ok(Value::Int(l.iter().map(|a| if let Value::Int(i) = a { *i } else { 0 }).sum()))
            }
            "map.remove" => Ok(cx.map_remove(arg(0), arg(1)).unwrap_or(Value::Nil)),
            "list.collect" => {
                // map through the host interface (the library's list.map is checked elsewhere)
                let items: Vec<Value> = cx.list(arg(0)).map(|l| l.to_vec()).unwrap_or_default();
                let out = cx.new_list(Vec::new())?;
                for x in items {
                    let y = cx.call(self, arg(1), &[x])?;
                    cx.list_push(out, y)?;
                }
                Ok(out)
            }
            "list.sort_by" => {
                // insertion sort; f(a, b) > 0 puts b first
                let mut items: Vec<Value> = cx.list(arg(0)).map(|l| l.to_vec()).unwrap_or_default();
                for i in 1..items.len() {
                    let mut j = i;
                    while j > 0 && matches!(cx.call(self, arg(1), &[items[j - 1], items[j]])?, Value::Int(n) if n > 0) {
                        items.swap(j - 1, j);
                        j -= 1;
                    }
                }
                cx.list_replace(arg(0), items)?;
                Ok(arg(0))
            }
            "twice" => {
                let a = cx.call(self, arg(0), &[Value::Int(1)])?;
                cx.call(self, arg(0), &[a])
            }
            _ => Err(Fail::Unknown),
        }
    }

    fn knows(&self, name: &str) -> bool {
        matches!(name, "add" | "log" | "http.serve")
    }
}

/// compile src and run it to the end: (output, result as text or the error)
fn exec_with(src: &str, lim: Limits, host: &mut TestHost) -> (String, Result<String, script::Error>) {
    let prog = match script::compile(src) {
        Ok(p) => p,
        Err(d) => {
            let msg = d.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("; ");
            let e = script::Error { kind: ErrorKind::Usage, msg: format!("compile: {}", msg), line: 0, col: 0, trace: Vec::new() };
            return (String::new(), Err(e));
        }
    };
    let mut vm = Vm::new(&prog, lim);
    loop {
        match vm.run(host, u64::MAX) {
            Status::Done(v) => return (vm.take_output(), Ok(vm.display(v))),
            Status::Yielded => {}
            Status::Error(e) => return (vm.take_output(), Err(e)),
        }
    }
}

/// the output of src; an error appended as "ERR line:col message"
fn out(src: &str) -> String {
    let (o, r) = exec_with(src, Limits::default(), &mut TestHost::default());
    match r {
        Ok(_) => o,
        Err(e) => format!("{}ERR {}:{} {}", o, e.line, e.col, e.msg),
    }
}

fn cases(t: &mut T, list: &[(&str, &str)]) {
    for (src, want) in list {
        t.eq(src, out(src).as_str(), *want);
    }
}

pub fn run(t: &mut T) {
    basics(t);
    closures(t);
    recursion(t);
    exceptions(t);
    errors(t);
    maps(t);
    floats(t);
    softfloat(t);
    host(t);
    callbacks(t);
    example(t);
    api(t);
    gc(t);
    fuel(t);
    limits(t);
    yields(t);
    compile_errors(t);
    deep(t);
}

fn basics(t: &mut T) {
    cases(t, &[
        ("print(1 + 2 * 3, 7 / 2, -7 / 2, 7 % 3, -7 % 3, 2 - 5)", "7 3 -3 1 -1 -3\n"),
        ("print(9223372036854775807 + 1, -9223372036854775807 - 2, 0x7FFF_FFFF_FFFF_FFFF * 2)", "-9223372036854775808 9223372036854775807 -2\n"),
        ("let m = -9223372036854775807 - 1\nprint(m / -1, m % -1, -m)", "-9223372036854775808 0 -9223372036854775808\n"),
        ("print(1 / 0)", "ERR 1:9 Division durch 0"),
        ("print(1 % 0)", "ERR 1:9 Division durch 0"),
        ("print(1 + 0.5, 1 / 2.0, 10 / 4, 10.0 / 4, 3 * 0.1)", "1.5 0.5 2 2.5 0.30000000000000004\n"),
        (r#"print("a" + "b", "x=${1 + 1}!", "${nil} ${true} ${[1, "s", 2.5, {k: "v", "a b": nil}]}")"#, "ab x=2! nil true [1, \"s\", 2.5, {k: \"v\", \"a b\": nil}]\n"),
        ("print(1 == 1.0, 1 != 1.0, \"a\" == \"a\", [1] == [1], nil == false, 0.1 + 0.2 == 0.3, 2 == 2.5)", "true false true false false false false\n"),
        ("let l = [1]\nprint(l == l, {} == {}, print == print, l != nil)", "true false true true\n"),
        ("print(nil ?? 5, 0 ?? 5, false ?? 5, nil || \"d\", 0 && \"x\", false && nothing)", "5 0 false d x false\n"),
        ("print(!nil, !0, !\"\", ![], !false)\nif 0 { print(\"0 ist wahr\") }", "true false false false true\n0 ist wahr\n"),
        ("print(len(\"größe\"), len([1, 2, 3]), len({a: 1}), type(1), type(1.0), type(\"s\"), type([]), type({}), type(nil), type(print), type(fn() {}), type(true))",
         "5 3 1 int float str list map nil fn fn bool\n"),
        ("print(str(12) + \"!\", int(\"42\") + 1, int(3.9), int(-3.9), float(2), int(true), float(\"2.5\"), str([1]))", "12! 43 3 -3 2.0 1 2.5 [1]\n"),
        ("int(\"x\")", "ERR 1:1 int(…): \"x\" ist keine ganze Zahl"),
        ("print(1 < 2, 2 <= 2, \"abc\" < \"abd\", 3 > 2.5, 2.5 >= 3, 9007199254740993 > 9007199254740992.0)", "true true true true false true\n"),
        ("print(1 < \"a\")", "ERR 1:9 Der Vergleich < passt nicht zu int und str"),
        ("let s = 0\nfor i in 0..10 { if i % 2 == 0 { continue } if i > 7 { break } s += i }\nlet w = 0\nwhile true { w += 1; if w >= 5 { break } }\nprint(s, w)", "16 5\n"),
        ("for i, c in \"aß€\" { print(i, c) }", "0 a\n1 ß\n2 €\n"),
        ("for i, x in [\"a\", \"b\"] { print(i, x) }\nfor x in [] { print(x) }", "0 a\n1 b\n"),
        ("let n = 3\nfor i in n - 1..n + 2 { print(i) }\nfor i in 5..2 { print(i) }", "2\n3\n4\n"),
        ("for i in 0..2.5 { }", "ERR 1:10 Ein Bereich a..b braucht ganze Zahlen, nicht float"),
        ("for x in 5 { }", "ERR 1:10 for braucht list, map, str oder bytes, nicht int"),
        ("let m = {a: 1}\nm.a += 5\nm.b = [1, 2]\nm.b[1] *= 10\nm[\"c\"] = \"x\"\nm.c += \"y\"\nlet l = [1, 2, 3]\nl[0] = l[2] * l[1]\nprint(m, l)",
         "{a: 6, b: [1, 20], c: \"xy\"} [6, 2, 3]\n"),
        ("let x = 1\n{ let x = 2; print(x) }\nprint(x)\nif true { let x = 3; { let x = 4; print(x) } print(x) }", "2\n1\n4\n3\n"),
        ("fn f() { let x = 10; { let x = x + 1; print(x) } print(x) }\nf()", "11\n10\n"),
        ("let l = []\nl.push(1, 2)\nl.push(3)\nprint(l.pop(), l, len(l), [].pop())", "3 [1, 2] 2 nil\n"),
        ("let a = [1, 2] + [3]\nprint(a, \"x\" + \"y\" + \"z\")", "[1, 2, 3] xyz\n"),
        ("let x\nprint(x, (fn(a, b) { return [a, b] })(1), (fn(a) { return a })(1, 2, 3))", "nil [1, nil] 1\n"),
        ("let i = 0\nwhile i < 3 { i += 1; let j = i * 2; if j == 4 { continue } print(j) }", "2\n6\n"),
        ("let a = [[1, 2], [3]]\nprint(a[0][1] + a[1][0], \"größe\"[1], \"abc\"[2])", "5 r c\n"),
    ]);
}

fn closures(t: &mut T) {
    cases(t, &[
        ("fn counter() { let n = 0; return fn() { n += 1; return n } }\nlet c1 = counter()\nlet c2 = counter()\nprint(c1(), c1(), c2(), c1())", "1 2 1 3\n"),
        ("let fs = []\nfor i in 0..3 { fs.push(fn() { return i * 10 }) }\nprint(fs[0](), fs[1](), fs[2]())", "0 10 20\n"),
        ("let fs = []\nfor k, v in {a: 1, b: 2} { fs.push(fn() { return k + str(v) }) }\nprint(fs[0](), fs[1]())", "a1 b2\n"),
        ("fn outer() {\n  let x = 1\n  fn mid() { fn inner() { x += 1; return x } return inner }\n  let f = mid()\n  f()\n  f()\n  return x\n}\nprint(outer())", "3\n"),
        ("fn eo(n) {\n  fn ev(n) { if n == 0 { return true } return od(n - 1) }\n  fn od(n) { if n == 0 { return false } return ev(n - 1) }\n  return ev(n)\n}\nprint(eo(10), eo(7))", "true false\n"),
        ("fn later() {\n  let a = get()\n  let y = 5\n  fn get() { return y }\n  return [a, get()]\n}\nprint(later())", "[nil, 5]\n"),
        ("fn adder(a) { return fn(b) { return fn(c) { return a + b + c } } }\nprint(adder(1)(2)(3))", "6\n"),
        ("fn shared() {\n  let v = 0\n  let inc = fn() { v += 1 }\n  let get = fn() { return v }\n  inc()\n  inc()\n  return get()\n}\nprint(shared())", "2\n"),
        ("let total = 0\nfn bump(n) { total += n }\nbump(2)\nbump(3)\nprint(total)", "5\n"),
        ("let fs = []\nlet i = 0\nwhile i < 3 { let j = i; fs.push(fn() { return j }); i += 1 }\nprint(fs[0](), fs[2]())", "0 2\n"),
        ("try { throw 5 } catch e { let f = fn() { return e * 2 }; print(f()) }", "10\n"),
        ("fn f(x) { let g = fn() { x += 1; return x }; g(); return x }\nprint(f(1))", "2\n"),
        ("let f = fn(n) { return n * 2 }\nlet m = {g: fn(x) { return x + 1 }}\nprint(f, m.g, fn() {}, f(4), m.g(4))", "<fn f> <fn g> <fn> 8 5\n"),
    ]);
}

fn recursion(t: &mut T) {
    cases(t, &[
        ("fn fib(n) { if n < 2 { return n } return fib(n - 1) + fib(n - 2) }\nprint(fib(20))", "6765\n"),
        ("fn ack(m, n) {\n  if m == 0 { return n + 1 }\n  if n == 0 { return ack(m - 1, 1) }\n  return ack(m - 1, ack(m, n - 1))\n}\nprint(ack(2, 3), ack(3, 3))", "9 61\n"),
        ("fn wrap() { fn f(n) { if n == 0 { return 0 } return n + f(n - 1) } return f(100) }\nprint(wrap())", "5050\n"),
        ("fn hanoi(n, a, b, c) { if n == 0 { return 0 } return hanoi(n - 1, a, c, b) + 1 + hanoi(n - 1, c, b, a) }\nprint(hanoi(10, 1, 2, 3))", "1023\n"),
    ]);
}

fn exceptions(t: &mut T) {
    cases(t, &[
        ("fn a(x) { if x > 2 { throw \"zu groß: ${x}\" } return x }\nfn b(x) { return a(x) * 2 }\nfn c(x) { try { return b(x) } catch e { return \"gefangen: \" + e } }\nprint(c(1), c(5))",
         "2 gefangen: zu groß: 5\n"),
        ("try { let z = 1 / 0 } catch e { print(e) }\ntry { let n = nil\nn.x } catch e { print(e) }", "Division durch 0\nnil hat kein Feld x\n"),
        ("try { try { throw 1 } catch e { throw e + 1 } } catch e { print(e) }", "2\n"),
        ("let r = []\nfor i in 0..5 { try { if i == 1 { continue } if i == 3 { break } r.push(i) } catch e { } }\nprint(r)", "[0, 2]\n"),
        ("fn f() { try { return \"t\" } catch e { return \"c\" } }\nprint(f())", "t\n"),
        ("try { throw {code: 7, msg: \"x\"} } catch e { print(e.code, e) }", "7 {code: 7, msg: \"x\"}\n"),
        ("fn deep(n) { if n == 0 { throw \"unten\" } for i in 0..1 { let q = [i]; deep(n - 1) } }\ntry { deep(5) } catch e { print(e) }\nprint(\"weiter\")", "unten\nweiter\n"),
        ("try { throw \"x\" } catch { print(\"ohne Variable\") }", "ohne Variable\n"),
        ("let n = 0\nfor i in 0..3 { try { n += 1; throw i } catch e { n += e } }\nprint(n)", "6\n"),
        ("fn g() { try { fail(\"innen\") } catch e { return e } }\nprint(g())", "host: innen\n"),
        ("try { [1][5] } catch e { print(e) }\ntry { undefined_fn() } catch e { print(e) }", "Index 5 liegt außerhalb der Liste (Länge 1)\nUnbekannte Funktion undefined_fn\n"),
        ("let x = 0\ntry { x = 1; throw nil } catch e { print(x, e) }", "1 nil\n"),
        ("throw \"boom\"", "ERR 1:1 boom"),
    ]);
}

fn errors(t: &mut T) {
    let (_, r) = exec_with("fn teile(a, b) {\n  return a / b\n}\nlet x = teile(1, 0)\n", Limits::default(), &mut TestHost::default());
    match r {
        Err(e) => {
            t.eq("error kind", e.kind, ErrorKind::Uncaught);
            t.eq("error position", (e.line, e.col), (2, 12));
            let tr: Vec<(&str, u32, u32)> = e.trace.iter().map(|x| (x.func.as_str(), x.line, x.col)).collect();
            t.eq("error trace", tr, vec![("teile", 2, 12), ("", 4, 9)]);
            t.eq("error display", e.to_string().as_str(), "2:12: Fehler: Division durch 0\n  in teile (2:12)\n  im Hauptprogramm (4:9)");
        }
        Ok(_) => t.check("division error", false),
    }
    let (_, r) = exec_with("fn a() { throw \"kaputt\" }\nfn b() { a() }\nb()", Limits::default(), &mut TestHost::default());
    let tr: Vec<(String, u32, u32)> = r.err().map_or(Vec::new(), |e| e.trace.iter().map(|x| (x.func.clone(), x.line, x.col)).collect());
    t.eq("throw trace", tr, vec![(String::from("a"), 1, 10), (String::from("b"), 2, 10), (String::new(), 3, 1)]);
    cases(t, &[
        ("let r = {}\nprint(r.a.b)", "ERR 2:11 nil hat kein Feld b"),
        ("let x = 5\nx()", "ERR 2:1 int ist keine Funktion"),
        ("prnt(1)", "ERR 1:1 Unbekannte Funktion prnt"),
        ("let a = foo + 1", "ERR 1:9 Unbekannter Name foo"),
        ("print(config.port)", "ERR 1:14 Unbekannter Name config.port"),
        ("let l = [1]\nprint(l[3])", "ERR 2:8 Index 3 liegt außerhalb der Liste (Länge 1)"),
        ("let l = [1]\nl[-1] = 0", "ERR 2:2 Index -1 liegt außerhalb der Liste (Länge 1)"),
        ("let s = \"a\" + 1", "ERR 1:13 Der Operator + passt nicht zu str und int"),
        ("[1].foo()", "ERR 1:5 list hat keine Methode foo"),
        ("let m = {a: 1}\nm.foo()", "ERR 2:3 Die Map hat keine Funktion foo"),
        ("let m = {a: 1}\nm.a()", "ERR 2:3 Das Feld a ist keine Funktion, sondern int"),
        ("y = 1", "ERR 1:1 y ist nicht deklariert – lege die Variable mit let y = … an"),
        ("let m = {}\nm[1.5] = 2", "ERR 2:2 Ein Map-Schlüssel muss str, int oder bool sein, nicht float"),
        ("let m = {[nil]: 1}", "ERR 1:9 Ein Map-Schlüssel muss str, int oder bool sein, nicht nil"),
        ("let m = {a: 1}\nm.a.b = 2", "ERR 2:5 Nur Maps haben Felder, nicht int (Feld b)"),
        ("let s = \"abc\"\ns[0] = \"x\"", "ERR 2:2 str lässt sich nicht ändern"),
        ("let v = -\"a\"", "ERR 1:9 Das Minus passt nicht zu str"),
        ("let x = 5\nx[0]", "ERR 2:2 int lässt sich nicht indizieren"),
        ("fatal()", "ERR 1:1 Host-Abbruch"),
        ("bad()", "ERR 1:1 bad gab einen ungültigen Wert zurück"),
        ("print(len(5))", "ERR 1:7 len braucht str, bytes, list oder map, nicht int"),
        ("print(\"x\"[\"a\"])", "ERR 1:10 Ein Index von str muss int sein, nicht str"),
        ("let a = 1\nlet b = a +\n  nil", "ERR 2:11 Der Operator + passt nicht zu int und nil"),
    ]);
    let (_, r) = exec_with("fatal()", Limits::default(), &mut TestHost::default());
    t.eq("host fatal kind", r.err().map(|e| e.kind), Some(ErrorKind::Host));
}

fn maps(t: &mut T) {
    cases(t, &[
        ("let m = {}\nm.z = 1\nm.a = 2\nm[\"m\"] = 3\nm[10] = 4\nm[true] = 5\nm.a = 20\nprint(m)\nlet ks = []\nfor k, v in m { ks.push(\"${k}=${v}\") }\nprint(ks)",
         "{z: 1, a: 20, m: 3, 10: 4, true: 5}\n[\"z=1\", \"a=20\", \"m=3\", \"10=4\", \"true=5\"]\n"),
        ("let big = {}\nfor i in 0..100 { big[\"k${(i * 37) % 100}\"] = i }\nlet first = []\nfor k in big { if len(first) < 5 { first.push(k) } }\nprint(first, len(big), big.k37, big[\"k99\"], big.nix)",
         "[\"k0\", \"k37\", \"k74\", \"k11\", \"k48\"] 100 1 27 nil\n"),
        ("let m = {a: 1, b: 2, c: 3}\nlet seen = []\nfor k in m { seen.push(k); if k == \"a\" { m.remove(\"b\"); m.d = 4 } }\nprint(seen, m, len(m))",
         "[\"a\", \"c\"] {a: 1, c: 3, d: 4} 3\n"),
        ("let m = {}\nfor i in 0..20 { m[i] = i }\nfor i in 0..20 { if i % 2 == 1 { m.remove(i) } }\nm[1] = \"neu\"\nlet s = 0\nfor k, v in m { if k != 1 { s += v } }\nprint(len(m), s, m[18], m[1], m[3])",
         "11 90 18 neu nil\n"),
        ("let m = {b: 1, a: 2}\nlet n = {[\"x\" + \"y\"]: 1, 2: \"zwei\", false: nil, \"if\": 3}\nprint(m, n, n.xy, n[2], n[false], n.if)",
         "{b: 1, a: 2} {xy: 1, 2: \"zwei\", false: nil, if: 3} 1 zwei nil 3\n"),
        ("let a = []\na.push(a)\nlet m = {}\nm.self = m\nprint(a, m)", "[[…]] {self: {…}}\n"),
        ("let m = {k: 1, k: 2}\nprint(m, len(m))", "{k: 2} 1\n"),
    ]);
}

/// expressions and the text of their float result, computed with Python (IEEE hardware)
const FLOAT_CASES: [(&str, &str); 31] = [
    ("0.1 + 0.2", "0.30000000000000004"),
    ("1.0 / 3.0", "0.3333333333333333"),
    ("2.0 / 3.0", "0.6666666666666666"),
    ("1e308 * 10.0", "inf"),
    ("5e-324 / 2.0", "0.0"),
    ("1.5e-323 / 2.0", "1e-323"),
    ("2.2250738585072014e-308 / 2.0", "1.1125369292536007e-308"),
    ("0.1 * 3.0", "0.30000000000000004"),
    ("1e16 + 1.0", "1e16"),
    ("9007199254740993 + 0.0", "9007199254740992.0"),
    ("123456789.0 * 987654321.0", "1.2193263111263526e17"),
    ("7.5 % 2.0", "1.5"),
    ("-7.5 % 2.0", "-1.5"),
    ("7.5 % -2.0", "1.5"),
    ("0.1 + 0.7", "0.7999999999999999"),
    ("3.0 - 2.9", "0.10000000000000009"),
    ("100.0 / 7.0", "14.285714285714286"),
    ("1e15", "1000000000000000.0"),
    ("1e16", "1e16"),
    ("0.0001", "0.0001"),
    ("0.00001", "1e-5"),
    ("1.7976931348623157e308 + 1e292", "inf"),
    ("1e-310 * 1e-10", "1e-320"),
    ("4.9e-324 * 0.6", "5e-324"),
    ("1e300 % 3.0", "0.0"),
    ("5.0 % 0.1", "0.09999999999999973"),
    ("2 / 0.5 + 1", "5.0"),
    ("0.1 + 0.2 - 0.3", "5.551115123125783e-17"),
    ("1.0 - 0.9999999999999999", "1.1102230246251565e-16"),
    ("1e-300 * 1e-300", "0.0"),
    ("-1e-300 * 1e-300", "-0.0"),
];

/// square roots (input bits, result bits) from Python's math.sqrt
const SQRT_CASES: [(u64, u64); 32] = [
    (0x4000000000000000, 0x3FF6A09E667F3BCD),
    (0x4008000000000000, 0x3FFBB67AE8584CAA),
    (0x3FE0000000000000, 0x3FE6A09E667F3BCD),
    (0x000012688B70E62B, 0x1FC1297872D9CBAE),
    (0x0000000000000001, 0x1E60000000000000),
    (0x7FEFFFFFFFFFFFFF, 0x5FEFFFFFFFFFFFFF),
    (0x3FB999999999999A, 0x3FD43D136248490F),
    (0x4024000000000000, 0x40094C583ADA5B53),
    (0x54B249AD2594C37D, 0x4A511B0EC57E649A),
    (0x40FE240C9FBE76C9, 0x4075F5D3B16948F6),
    (0x4113C3E30F306CAD, 0x4081C88124F2115D),
    (0x41026A096432877E, 0x40784649FFDAB23F),
    (0x4123DD6CF232479B, 0x40893670D974BBF2),
    (0x40F1AF449630B490, 0x4070D23D0986E389),
    (0x41205A9402347C83, 0x4086E05123F3EE0E),
    (0x411651E3AAEB2201, 0x4082E5C70724D627),
    (0x40EC51DD97C1203D, 0x406E1A8C918E759F),
    (0x411EF8AEEEC93517, 0x408642C30A553DCD),
    (0x40E24EF511F4ECEC, 0x4068346A2512E67A),
    (0x411A77B6BC11FE13, 0x4084942470A47254),
    (0x40F10DF6C6F62DFC, 0x407084D408B643DD),
    (0x40F6259036A80E80, 0x4072D2F9E6E1DF2C),
    (0x4119E91CC1AE933F, 0x40845C69353C3AE1),
    (0x41293BC83FD5036C, 0x408C6A83449DB036),
    (0x1CCF28C104724C60, 0x2E5F93A91C836B32),
    (0x0CE93BD04D2BCC16, 0x266C6A87CD469673),
    (0x6290CB1E29F3D7B6, 0x5140645483E26E26),
    (0x35A0BECD7B1D032E, 0x3AC726003EFA0BB6),
    (0x1F24A23D58D69964, 0x2F89B2275DE25F69),
    (0x21F8A6A63E3C4F44, 0x30F3DC2256968115),
    (0x4C08F6D055A1DB50, 0x45FC43933849A0D6),
    (0x2B51A61DBF29C77F, 0x35A0CDE23E938CE4),
];

fn floats(t: &mut T) {
    for (e, want) in FLOAT_CASES {
        let src = format!("print({})", e);
        t.eq(e, out(&src).as_str(), format!("{}\n", want).as_str());
    }
    cases(t, &[
        ("print(-0.0, 0.0 == -0.0, 0.0 / 0.0 == 0.0 / 0.0, -(0.0 / 0.0), 1.0 / 0.0, -1.0 / 0.0)", "-0.0 true false nan inf -inf\n"),
        ("let x = 0.0\nfor i in 0..10 { x += 0.1 }\nprint(x, x == 1.0, x < 1.0)", "0.9999999999999999 false true\n"),
        ("print(int(1e18), int(-0.5), float(\"1e400\"), float(\"-0\"))", "1000000000000000000 0 inf -0.0\n"),
        ("print(int(1e19))", "ERR 1:7 int(…): 1e19 passt nicht in int"),
        ("print(2.5 > 2, 2 < 2.5, 3 == 3.0000000000000004, 1 + 1.0)", "true true false 2.0\n"),
    ]);
    for (x, want) in SQRT_CASES {
        let r = num::sqrt(f64::from_bits(x)).to_bits();
        t.eq("sqrt vector", (x, r), (x, want));
    }
}

fn next(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

/// a float for the cross-checks: random bits, an exponent close to `near`, tiny or huge
/// exponents, or a special value
fn pick(seed: &mut u64, near: Option<u64>) -> f64 {
    let r = next(seed);
    let mant = next(seed) & ((1 << 52) - 1);
    let sign = (r >> 63) << 63;
    let exp = match (r >> 8) % 8 {
        0 | 1 => return f64::from_bits(next(seed)),
        2 | 3 => match near {
            Some(e) => (e + (r >> 16) % 7).saturating_sub(3).min(2046),
            None => (r >> 16) % 2047,
        },
        4 => (r >> 16) % 64,
        5 => 2046 - (r >> 16) % 64,
        6 => 1023 - 60 + (r >> 16) % 120,
        _ => {
            const SPECIAL: [u64; 8] = [0, 1 << 63, 0x7FF0 << 48, 0xFFF0 << 48, 0x7FF8 << 48, 1, 0x7FEF_FFFF_FFFF_FFFF, 0x0010 << 48];
            return f64::from_bits(SPECIAL[((r >> 16) % 8) as usize]);
        }
    };
    f64::from_bits(sign | exp << 52 | mant)
}

/// fmod the classic way (musl): one bit per step, an independent reference for num::rem
/// (Rust's % on f64 needs a libm)
fn fmod_ref(x: f64, y: f64) -> f64 {
    let (mut ux, mut uy) = (x.to_bits(), y.to_bits());
    let (mut ex, mut ey) = (((ux >> 52) & 0x7FF) as i64, ((uy >> 52) & 0x7FF) as i64);
    let sx = ux >> 63;
    if uy << 1 == 0 || y.is_nan() || ex == 0x7FF {
        return f64::NAN;
    }
    if ux << 1 <= uy << 1 {
        return if ux << 1 == uy << 1 { 0.0 * x } else { x };
    }
    if ex == 0 {
        let mut i = ux << 12;
        while i >> 63 == 0 {
            ex -= 1;
            i <<= 1;
        }
        ux <<= -ex + 1;
    } else {
        ux &= u64::MAX >> 12;
        ux |= 1 << 52;
    }
    if ey == 0 {
        let mut i = uy << 12;
        while i >> 63 == 0 {
            ey -= 1;
            i <<= 1;
        }
        uy <<= -ey + 1;
    } else {
        uy &= u64::MAX >> 12;
        uy |= 1 << 52;
    }
    while ex > ey {
        let i = ux.wrapping_sub(uy);
        if i >> 63 == 0 {
            if i == 0 {
                return 0.0 * x;
            }
            ux = i;
        }
        ux <<= 1;
        ex -= 1;
    }
    let i = ux.wrapping_sub(uy);
    if i >> 63 == 0 {
        if i == 0 {
            return 0.0 * x;
        }
        ux = i;
    }
    while ux >> 52 == 0 {
        ux <<= 1;
        ex -= 1;
    }
    if ex > 0 {
        ux -= 1 << 52;
        ux |= (ex as u64) << 52;
    } else {
        ux >>= -ex + 1;
    }
    f64::from_bits(ux | sx << 63)
}

/// whether soft equals the hardware's result (any NaN of the hardware is our one NaN)
fn same(soft: f64, hw: f64) -> bool {
    if hw.is_nan() { soft.to_bits() == num::NAN.to_bits() } else { soft.to_bits() == hw.to_bits() }
}

/// x = m * 2^e with m an integer (53 bits for normal numbers)
fn parts(x: f64) -> (u64, i32) {
    let b = x.to_bits();
    let e = ((b >> 52) & 0x7FF) as i32;
    let f = b & ((1 << 52) - 1);
    if e == 0 { (f, -1074) } else { (f | 1 << 52, e - 1075) }
}

/// a * 2^ea < b * 2^eb for integers a, b < 2^120
fn less(a: u128, ea: i32, b: u128, eb: i32) -> bool {
    let e = ea.min(eb);
    let (sa, sb) = ((ea - e) as u32, (eb - e) as u32);
    let fits = |v: u128, s: u32| s < 128 && (v == 0 || v.leading_zeros() > s);
    match (fits(a, sa), fits(b, sb)) {
        (true, true) => (a << sa) < (b << sb),
        (false, true) => false,
        (true, false) => true,
        _ => a < b,
    }
}

/// r is sqrt(x) correctly rounded: x lies strictly between the squares of the midpoints
/// to r's neighbours (x positive, finite)
fn sqrt_ok(x: f64, r: f64) -> bool {
    let (xm, xe) = parts(x);
    let (rm, re) = parts(r);
    if rm < 1 << 52 {
        return false;
    }
    let up = (2 * rm as u128 + 1) * (2 * rm as u128 + 1);
    let (down, de) = if rm == 1 << 52 { ((4 * rm as u128 - 1) * (4 * rm as u128 - 1), 2 * re - 4) } else { ((2 * rm as u128 - 1) * (2 * rm as u128 - 1), 2 * re - 2) };
    less(down, de, xm as u128, xe) && less(xm as u128, xe, up, 2 * re - 2)
}

fn softfloat(t: &mut T) {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut bad = [0u32; 8];
    let mut first: Vec<String> = Vec::new();
    for _ in 0..100_000 {
        let a = pick(&mut seed, None);
        let near = (a.to_bits() >> 52) & 0x7FF;
        let b = pick(&mut seed, Some(near));
        let checks = [
            same(num::add(a, b), a + b),
            same(num::sub(a, b), a - b),
            same(num::mul(a, b), a * b),
            same(num::div(a, b), a / b),
            same(num::rem(a, b), fmod_ref(a, b)),
            num::cmp(a, b) == a.partial_cmp(&b),
        ];
        for (i, ok) in checks.iter().enumerate() {
            if !ok {
                bad[i] += 1;
                if first.len() < 4 {
                    first.push(format!("op {} a {:016x} b {:016x}", i, a.to_bits(), b.to_bits()));
                }
            }
        }
        let i = next(&mut seed) as i64 >> (next(&mut seed) % 64);
        if num::from_i64(i).to_bits() != (i as f64).to_bits() {
            bad[6] += 1;
        }
        let x = num::abs(a);
        if x.is_finite() && x > 0.0 && !sqrt_ok(x, num::sqrt(x)) {
            bad[7] += 1;
            if first.len() < 4 {
                first.push(format!("sqrt {:016x}", x.to_bits()));
            }
        }
    }
    t.eq("soft float against the hardware (add sub mul div rem cmp from_i64 sqrt)", bad, [0; 8]);
    t.eq("first differences", first, Vec::new());
    let ints = [
        (0.0, Some(0)), (-0.9, Some(0)), (0.9, Some(0)), (-1.5, Some(-1)), (9223372036854775808.0, None), (-9223372036854775808.0, Some(i64::MIN)),
        (9223372036854774784.0, Some(9223372036854774784)), (4611686018427387904.0, Some(4611686018427387904)), (f64::INFINITY, None),
        (num::NAN, None), (1e300, None),
    ];
    for (x, want) in ints {
        t.eq("to_i64", (x.to_bits(), num::to_i64(x)), (x.to_bits(), want));
    }
    let rounding: [(f64, f64); 12] = [
        (num::floor(-0.5), -1.0), (num::floor(2.5), 2.0), (num::ceil(-0.5), -0.0), (num::ceil(2.1), 3.0), (num::trunc(-2.7), -2.0),
        (num::floor(-0.0), -0.0), (num::floor(1e300), 1e300), (num::ceil(4503599627370495.5), 4503599627370496.0),
        (num::floor(-4503599627370495.5), -4503599627370496.0), (num::trunc(5e-324), 0.0), (num::abs(-3.0), 3.0), (num::neg(0.0), -0.0),
    ];
    for (i, (got, want)) in rounding.iter().enumerate() {
        t.eq(&format!("rounding {}", i), got.to_bits(), want.to_bits());
    }
    let p53 = 9007199254740992.0;
    let cmp_cases = [
        (9007199254740993, p53, Some(Ordering::Greater)), (9007199254740992, p53, Some(Ordering::Equal)), (-1, -0.5, Some(Ordering::Less)),
        (i64::MAX, 9.2233720368547758e18, Some(Ordering::Less)), (i64::MIN, -9.2233720368547758e18, Some(Ordering::Equal)),
        (3, 3.0000000000000004, Some(Ordering::Less)), (0, -0.0, Some(Ordering::Equal)), (5, num::NAN, None), (i64::MAX, f64::INFINITY, Some(Ordering::Less)),
        (i64::MIN, f64::NEG_INFINITY, Some(Ordering::Greater)), (-3, -3.5, Some(Ordering::Greater)),
    ];
    for (i, x, want) in cmp_cases {
        t.eq("cmp_int", (i, x.to_bits(), num::cmp_int(i, x)), (i, x.to_bits(), want));
    }
    let mut s = String::new();
    num::fmt(num::div(num::NAN, 1.0), &mut s);
    s.push(' ');
    num::fmt(num::sub(f64::INFINITY, f64::INFINITY), &mut s);
    t.eq("one NaN", (s.as_str(), num::sub(f64::INFINITY, f64::INFINITY).to_bits()), ("nan nan", 0x7FF8_0000_0000_0000));
}

fn host(t: &mut T) {
    let src = r#"print(add(1, 2, 3))
print([1, 2, 3].sum(), "abc".upper())
try { fail("x") } catch e { print(e) }
let p = print
p("über p", 1)
let a = add
print(a(2, 2), type(add), add, http.serve)
let m = mk()
print(m, m.liste[1])
let b = bytes(1, 2, 255)
print(b, len(b), b[2], b + b, b == bytes(1, 2, 255))
for i, x in b { print(i, x) }
log("eins")
http.serve(80, fn(req) { return "Antwort auf ${req}" })
print(pause())
burn()
print(math.sqrt(2.0), config.port ?? 80, config.name)
"#;
    let prog = script::compile(src).ok();
    t.check("host script compiles", prog.is_some());
    let Some(prog) = prog else { return };
    let mut vm = Vm::new(&prog, Limits::default());
    let mut h = TestHost::default();
    let cfg = vm.new_map().ok();
    let math = vm.new_map().ok();
    if let (Some(cfg), Some(math)) = (cfg, math) {
        let k = vm.new_str("name").unwrap_or(Value::Nil);
        let v = vm.new_str("pc1").unwrap_or(Value::Nil);
        let _ = vm.map_set(cfg, k, v);
        let ks = vm.new_str("sqrt").unwrap_or(Value::Nil);
        let f = vm.native("math.sqrt");
        let _ = vm.map_set(math, ks, f);
        t.check("set host values", vm.set_global("config", cfg) && vm.set_global("math", math) && !vm.set_global("nothing", cfg));
    }
    let first = vm.run(&mut h, u64::MAX);
    t.eq("native yields", first, Status::Yielded);
    let before = vm.call_fuel();
    let second = vm.resume(&mut h, u64::MAX);
    t.eq("host run", second, Status::Done(Value::Nil));
    t.check("charged fuel", vm.last_fuel() >= 1000 && vm.call_fuel() == before + vm.last_fuel());
    let want = "6\n6 ABC\nhost: x\nüber p 1\n4 fn <fn add> <fn http.serve>\n{eins: 1, liste: [1, \"eins\"]} eins\nbytes(01 02 ff) 3 255 bytes(01 02 ff 01 02 ff) true\n0 1\n1 2\n2 255\n7\n1.4142135623730951 80 pc1\n";
    t.eq("host output", vm.take_output().as_str(), want);
    t.eq("host log", h.log.clone(), vec![String::from("eins")]);
    t.eq("pinned handler", h.pins.len(), 1);
    if let Some(&pin) = h.pins.first() {
        vm.collect();
        let f = vm.pinned(pin);
        let req = vm.new_str("GET /").unwrap_or(Value::Nil);
        t.check("call pinned", vm.call_value(f, &[req]).is_ok());
        let r = vm.run(&mut h, u64::MAX);
        let text = match r {
            Status::Done(v) => vm.str(v).map(String::from),
            _ => None,
        };
        t.eq("callback result", text.as_deref(), Some("Antwort auf GET /"));
        vm.unpin(pin);
        t.eq("unpinned", vm.pinned(pin), Value::Nil);
    }
}

/// the server of the plan's example (scriptt::EXAMPLE) against a fake simulator: requests,
/// a disk read that completes later, the cache
const SERVER: &str = r#"let hits = 0
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

/// the simulator side: the handler, pending disk reads, the responses sent
#[derive(Default)]
struct Sim {
    port: i64,
    handler: Option<Pin>,
    reads: Vec<(String, Pin)>,
    sent: Vec<(i64, i64, String)>,
}

impl Host for Sim {
    fn call(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        let arg = |i: usize| args.get(i).copied().unwrap_or(Value::Nil);
        match name {
            "http.serve" => {
                self.port = if let Value::Int(p) = arg(0) { p } else { -1 };
                self.handler = Some(cx.pin(arg(1)));
                Ok(Value::Nil)
            }
            "disk.get" => {
                let k = cx.str(arg(0)).unwrap_or("").to_string();
                let p = cx.pin(arg(1));
                self.reads.push((k, p));
                Ok(Value::Nil)
            }
            // res.send / res.json: res is a map {id: n} without such fields, so the method
            // comes here with the map first
            "map.send" | "map.json" => {
                let id = match cx.map_get_str(arg(0), "id") {
                    Some(Value::Int(i)) => i,
                    _ => return Err(cx.error("kein Antwort-Objekt")),
                };
                let code = if let Value::Int(c) = arg(1) { c } else { 0 };
                let body = if name == "map.json" { format!("json {}", cx.display(arg(2))) } else { cx.display(arg(2)) };
                self.sent.push((id, code, body));
                Ok(Value::Nil)
            }
            _ => Err(Fail::Unknown),
        }
    }
}

/// a request {path, query: {id}} and a response {id} for the handler
fn request(vm: &mut Vm, id: i64, path: &str, user: Option<&str>) -> [Value; 2] {
    let mut mk = || -> Option<[Value; 2]> {
        let req = vm.new_map().ok()?;
        let k = vm.new_str("path").ok()?;
        let p = vm.new_str(path).ok()?;
        vm.map_set(req, k, p).ok()?;
        let q = vm.new_map().ok()?;
        if let Some(u) = user {
            let k = vm.new_str("id").ok()?;
            let v = vm.new_str(u).ok()?;
            vm.map_set(q, k, v).ok()?;
        }
        let k = vm.new_str("query").ok()?;
        vm.map_set(req, k, q).ok()?;
        let res = vm.new_map().ok()?;
        let k = vm.new_str("id").ok()?;
        vm.map_set(res, k, Value::Int(id)).ok()?;
        Some([req, res])
    };
    mk().unwrap_or([Value::Nil, Value::Nil])
}

/// run the pending invocation to its end in slices of 50 fuel
fn finish(vm: &mut Vm, sim: &mut Sim) -> Status {
    loop {
        match vm.run(sim, 50) {
            Status::Yielded => {}
            s => return s,
        }
    }
}

fn example(t: &mut T) {
    let Ok(prog) = script::compile_named("server.jfs", SERVER) else {
        t.check("example compiles", false);
        return;
    };
    let mut vm = Vm::new(&prog, Limits::default());
    let mut sim = Sim::default();
    let cfg = vm.new_map().unwrap_or(Value::Nil);
    let k = vm.new_str("port").unwrap_or(Value::Nil);
    let _ = vm.map_set(cfg, k, Value::Int(8080));
    vm.set_global("config", cfg);
    t.eq("example main", finish(&mut vm, &mut sim), Status::Done(Value::Nil));
    t.eq("example port", sim.port, 8080);
    let Some(h) = sim.handler else {
        t.check("example handler", false);
        return;
    };
    // 1: hello
    let a = request(&mut vm, 1, "/", None);
    let f = vm.pinned(h);
    t.check("request 1", vm.call_value(f, &a).is_ok());
    t.eq("request 1 done", finish(&mut vm, &mut sim), Status::Done(Value::Nil));
    // 2: a user, not cached: a disk read the simulator answers later
    let a = request(&mut vm, 2, "/user", Some("7"));
    let f = vm.pinned(h);
    let _ = vm.call_value(f, &a);
    t.eq("request 2 done", finish(&mut vm, &mut sim), Status::Done(Value::Nil));
    t.eq("disk read", sim.reads.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), vec!["user/7"]);
    vm.collect();
    if let Some((_, p)) = sim.reads.pop() {
        let user = vm.new_map().unwrap_or(Value::Nil);
        let k = vm.new_str("name").unwrap_or(Value::Nil);
        let v = vm.new_str("Ada").unwrap_or(Value::Nil);
        let _ = vm.map_set(user, k, v);
        let cb = vm.pinned(p);
        vm.unpin(p);
        t.check("disk callback", vm.call_value(cb, &[Value::Nil, user]).is_ok());
        t.eq("disk callback done", finish(&mut vm, &mut sim), Status::Done(Value::Nil));
    }
    // 3: the same user from the cache
    let a = request(&mut vm, 3, "/user", Some("7"));
    let f = vm.pinned(h);
    let _ = vm.call_value(f, &a);
    t.eq("request 3 done", finish(&mut vm, &mut sim), Status::Done(Value::Nil));
    let sent: Vec<(i64, i64, &str)> = sim.sent.iter().map(|s| (s.0, s.1, s.2.as_str())).collect();
    t.eq("responses", sent, vec![(1, 200, "hello #1"), (2, 200, "json {name: \"Ada\"}"), (3, 200, "json {name: \"Ada\"}")]);
    t.eq("hits", vm.global("hits"), Some(Value::Int(3)));
}

/// natives that call script functions back (Cx::call)
fn callbacks(t: &mut T) {
    cases(t, &[
        ("let l = [3, 1, 2]\nprint(l.map(fn(x) { return x * 10 }), l.sort_by(fn(a, b) { return a - b }), l)", "[30, 10, 20] [1, 2, 3] [1, 2, 3]\n"),
        ("let w = [\"b\", \"a\", \"c\"]\nw.sort_by(fn(a, b) { if a < b { return -1 } if a > b { return 1 } return 0 })\nprint(w, twice(fn(x) { return x + 5 }))",
         "[\"a\", \"b\", \"c\"] 11\n"),
        ("try { [1, 2].map(fn(x) { if x == 2 { throw \"zwei\" } return x }) } catch e { print(e) }", "zwei\n"),
        ("print([1, 2].map(fn(x) { try { throw x } catch e { return e * 3 } }))", "[3, 6]\n"),
        ("print([[1, 2], [3]].map(fn(l) { return l.map(fn(x) { return x + 1 }) }))", "[[2, 3], [4]]\n"),
        ("print([1, 2].map(print))", "1\n2\n[nil, nil]\n"),
        ("let n = 0\n[1, 2, 3].map(fn(x) { n += pause() })\nprint(n)", "21\n"),
        ("fn deep(k) { return [k].map(fn(x) { return deep(x + 1) }) }\ndeep(0)", "ERR 1:25 Zu tief verschachtelte Rückrufe (mehr als 20)"),
        ("[1].map(5)", "ERR 1:5 list.map: f muss eine Funktion sein, nicht int"),
        ("[1].collect(5)", "ERR 1:5 int ist keine Funktion"),
        ("print([[1, 2], [3]].collect(fn(l) { return l.collect(fn(x) { return x + 1 }) }))", "[[2, 3], [4]]\n"),
        ("let big = []\nfor i in 0..2000 { big.push(i) }\nlet r = big.map(fn(x) { return {v: \"v${x}\"} })\nprint(len(r), r[1999].v, r[0])", "2000 v1999 {v: \"v0\"}\n"),
    ]);
    let (_, r) = exec_with("fn deep(k) { return [k].map(fn(x) { return deep(x + 1) }) }\ndeep(0)", Limits::default(), &mut TestHost::default());
    t.eq("nesting limit kind", r.err().map(|e| e.kind), Some(ErrorKind::Depth));
    let (_, r) = exec_with("[1].map(fn(x) { while true { } })", Limits { fuel: 5000, ..Limits::default() }, &mut TestHost::default());
    t.eq("fuel limit in a callback", r.err().map(|e| (e.kind, e.line)), Some((ErrorKind::Fuel, 1)));
    let src = "let big = []\nfor i in 0..1000 { big.push(i) }\nlet r = big.map(fn(x) { let g = {k: [x, \"${x}\"]}; return x * 2 })\nprint(len(r), r[999], r[0])";
    let prog = script::compile(src).ok();
    if let Some(prog) = prog {
        let mut vm = Vm::new(&prog, Limits { heap: 64 << 10, ..Limits::default() });
        let st = vm.run(&mut TestHost::default(), u64::MAX);
        t.eq("callbacks under collector pressure", (st, vm.take_output().as_str(), vm.collections() >= 3), (Status::Done(Value::Nil), "1000 1998 0\n", true));
    }
}

fn api(t: &mut T) {
    let src = "let total = 0\nfn add_to(x) { total += x\n  return total }\nfn boom() { return 1 / 0 }\nlet greeting = \"hallo\"\nfn echo(a, b) { return [b, a] }";
    let Ok(prog) = script::compile_named("api.jfs", src) else {
        t.check("api compiles", false);
        return;
    };
    t.eq("program name", prog.name(), "api.jfs");
    t.eq("program globals", prog.globals(), vec!["total", "add_to", "boom", "greeting", "echo"]);
    t.check("disasm", prog.disasm().contains("fn add_to #") && prog.instructions() > 10);
    let mut vm = Vm::new(&prog, Limits::default());
    let mut h = TestHost::default();
    t.check("busy before main", vm.busy() && vm.call("add_to", &[Value::Int(1)]).is_err_and(|e| e.kind == ErrorKind::Usage));
    t.eq("main", vm.run(&mut h, u64::MAX), Status::Done(Value::Nil));
    t.check("idle run", matches!(vm.run(&mut h, 10), Status::Error(e) if e.kind == ErrorKind::Usage));
    t.check("call", vm.call("add_to", &[Value::Int(5)]).is_ok());
    t.eq("call result", vm.run(&mut h, u64::MAX), Status::Done(Value::Int(5)));
    t.eq("global", vm.global("total"), Some(Value::Int(5)));
    t.check("set global", vm.set_global("total", Value::Int(100)));
    let _ = vm.call("add_to", &[Value::Int(1)]);
    t.eq("after set_global", vm.run(&mut h, u64::MAX), Status::Done(Value::Int(101)));
    t.check("unknown function", vm.call("nope", &[]).is_err_and(|e| e.kind == ErrorKind::Usage));
    t.check("not a function", vm.call("total", &[]).is_err());
    let _ = vm.call("boom", &[]);
    match vm.run(&mut h, u64::MAX) {
        Status::Error(e) => t.eq("boom", (e.kind, e.line, e.col, e.msg.as_str()), (ErrorKind::Uncaught, 4, 22, "Division durch 0")),
        s => t.check(&format!("boom gives an error, not {:?}", s), false),
    }
    t.check("idle after error", !vm.busy());
    let _ = vm.call("add_to", &[Value::Int(1)]);
    t.eq("works after an error", vm.run(&mut h, u64::MAX), Status::Done(Value::Int(102)));
    let _ = vm.call("add_to", &[Value::Int(1)]);
    t.check("busy", vm.call("add_to", &[Value::Int(1)]).is_err_and(|e| e.kind == ErrorKind::Usage));
    t.eq("finish", vm.run(&mut h, u64::MAX), Status::Done(Value::Int(103)));
    let g = vm.global("greeting").unwrap_or(Value::Nil);
    t.eq("str global", vm.str(g), Some("hallo"));
    let s = vm.new_str("x").unwrap_or(Value::Nil);
    let _ = vm.call("echo", &[s, Value::Float(1.5)]);
    match vm.run(&mut h, u64::MAX) {
        Status::Done(v) => t.eq("echo", vm.display(v).as_str(), "[1.5, \"x\"]"),
        s => t.check(&format!("echo: {:?}", s), false),
    }
    // a value that was not pinned is gone after a collection: None and errors, not a crash
    let stale = vm.new_list(Vec::new()).unwrap_or(Value::Nil);
    t.check("fresh value valid", vm.valid(stale) && vm.list(stale).is_some());
    vm.collect();
    t.eq("stale value", (vm.valid(stale), vm.list(stale).is_none(), vm.display(stale).as_str()), (false, true, "<ungültig>"));
    t.check("stale argument", vm.call("add_to", &[stale]).is_err_and(|e| e.kind == ErrorKind::Usage) && !vm.busy());
    t.check("stale global", !vm.set_global("total", stale) && !vm.valid(Value::Native(9999)));
    // cancel drops a yielded invocation
    let _ = vm.call("add_to", &[Value::Int(1)]);
    t.eq("yield before cancel", vm.run(&mut h, 2), Status::Yielded);
    vm.cancel();
    t.check("cancelled", !vm.busy() && vm.call("add_to", &[Value::Int(0)]).is_ok());
    t.eq("after cancel", vm.run(&mut h, u64::MAX), Status::Done(Value::Int(103)));
    // a clone is independent
    let mut twin = vm.clone();
    let _ = twin.call("add_to", &[Value::Int(1000)]);
    let _ = twin.run(&mut h, u64::MAX);
    t.eq("clone independent", (vm.global("total"), twin.global("total")), (Some(Value::Int(103)), Some(Value::Int(1103))));
    fn send_clone<X: Send + Clone>() {}
    send_clone::<Vm>();
    send_clone::<script::Program>();
}

const GC_SRC: &str = "let keep = []
for i in 0..20000 {
  let a = {name: \"a${i}\"}
  let b = {other: a}
  a.other = b
  if i % 1000 == 0 { keep.push(a) }
}
let s = \"\"
for i in 0..20000 { s = \"x${i}\" }
print(len(keep), s, keep[19].other.other.name)";

fn gc(t: &mut T) {
    let Ok(prog) = script::compile(GC_SRC) else {
        t.check("gc compiles", false);
        return;
    };
    let mut vm = Vm::new(&prog, Limits { heap: 256 << 10, ..Limits::default() });
    let st = vm.run(&mut (), u64::MAX);
    t.eq("gc run", st, Status::Done(Value::Nil));
    t.eq("gc output", vm.take_output().as_str(), "20 x19999 a19000\n");
    t.check("collected often", vm.collections() >= 10);
    t.eq("fuel and collections of the gc program", (vm.call_fuel(), vm.collections()), (620109, 22));
    vm.collect();
    t.eq("live objects", vm.heap_objects(), 62);
    t.check("live bytes", vm.heap_bytes() < 16 << 10);
    // pins keep a value, unpinned it goes
    let l = vm.new_list(vec![Value::Int(1)]).unwrap_or(Value::Nil);
    let p = vm.pin(l);
    vm.collect();
    t.eq("pinned survives", (vm.heap_objects(), vm.list(vm.pinned(p)).map(|x| x.len())), (63, Some(1)));
    vm.unpin(p);
    vm.collect();
    t.eq("unpinned freed", vm.heap_objects(), 62);
    // a cycle the script drops is freed
    let Ok(prog) = script::compile("let g = nil\nfn make() { let a = {}; let b = {a: a}; a.b = b; g = [a] }\nfn drop() { g = nil }") else { return };
    let mut vm = Vm::new(&prog, Limits::default());
    let _ = vm.run(&mut (), u64::MAX);
    let _ = vm.call("make", &[]);
    let _ = vm.run(&mut (), u64::MAX);
    vm.collect();
    let with = vm.heap_objects();
    let _ = vm.call("drop", &[]);
    let _ = vm.run(&mut (), u64::MAX);
    vm.collect();
    t.eq("cycle freed (the two closures stay)", (with, vm.heap_objects()), (5, 2));
}

fn fuel(t: &mut T) {
    let Ok(prog) = script::compile("let n = 0\nwhile true { n += 1 }") else { return };
    let mut vm = Vm::new(&prog, Limits { fuel: 10_000, ..Limits::default() });
    let mut yields = 0;
    let end = loop {
        match vm.run(&mut (), 1000) {
            Status::Yielded => yields += 1,
            s => break s,
        }
    };
    match end {
        Status::Error(e) => {
            t.eq("endless loop stopped", (e.kind, e.line), (ErrorKind::Fuel, 2));
            t.eq("fuel message", e.msg.as_str(), "Rechenzeit aufgebraucht: mehr als 10000 Schritte in einem Aufruf – eine Endlosschleife?");
        }
        s => t.check(&format!("endless loop: {:?}", s), false),
    }
    t.eq("yields before the limit", yields, 10);
    t.eq("fuel used", vm.call_fuel(), 10_000);
    t.check("n counted", matches!(vm.global("n"), Some(Value::Int(n)) if n > 1000));
    t.check("idle after fuel error", !vm.busy());
    let mut vm = Vm::new(&prog, Limits::default());
    t.eq("fuel 0 does nothing", (vm.run(&mut (), 0), vm.last_fuel()), (Status::Yielded, 0));
    t.eq("fuel 5", (vm.run(&mut (), 5), vm.last_fuel()), (Status::Yielded, 5));
    // Cx::fuel: the invocation's fuel when a native runs, the same however the runs are cut
    struct Stamp(Vec<u64>);
    impl Host for Stamp {
        fn call(&mut self, cx: &mut Cx, name: &str, _: &[Value]) -> Result<Value, Fail> {
            if name != "stamp" {
                return Err(Fail::Unknown);
            }
            self.0.push(cx.fuel());
            Ok(Value::Nil)
        }
    }
    let Ok(prog) = script::compile("stamp()\nlet i = 0\nwhile i < 100 { i += 1 }\nstamp()") else { return };
    let mut whole = Stamp(Vec::new());
    let mut vm = Vm::new(&prog, Limits::default());
    let _ = vm.run(&mut whole, u64::MAX);
    let total = vm.call_fuel();
    let mut sliced = Stamp(Vec::new());
    let mut vm = Vm::new(&prog, Limits::default());
    while vm.run(&mut sliced, 7) == Status::Yielded {}
    t.eq("Cx::fuel in slices", sliced.0.clone(), whole.0.clone());
    t.check(&format!("Cx::fuel counts up ({:?} of {})", whole.0, total), whole.0.len() == 2 && whole.0[0] >= 1 && whole.0[1] > whole.0[0] + 100 && whole.0[1] < total);
}

fn limits(t: &mut T) {
    let kind = |src: &str, lim: Limits| exec_with(src, lim, &mut TestHost::default()).1.err().map(|e| (e.kind, e.trace.len()));
    t.eq("depth limit", kind("fn r(n) { return r(n + 1) }\nr(0)", Limits::default()), Some((ErrorKind::Depth, 500)));
    let lim = Limits { stack: 300, ..Limits::default() };
    t.check("stack limit", kind("fn s(n) { let a = 1; let b = [a]; return s(n + 1) }\ns(0)", lim).is_some_and(|k| k.0 == ErrorKind::Stack));
    let lim = Limits { heap: 1 << 20, ..Limits::default() };
    t.eq("heap limit", kind("let s = \"x\"\nwhile true { s = s + s }", lim).map(|k| k.0), Some(ErrorKind::Heap));
    t.eq("heap limit list", kind("let l = []\nwhile true { l.push(l) }", lim).map(|k| k.0), Some(ErrorKind::Heap));
    let (o, r) = exec_with("print(\"0123456789abc\")\nprint(\"x\")", Limits { output: 10, ..Limits::default() }, &mut TestHost::default());
    t.eq("output limit", (o.as_str(), r.is_ok()), ("0123456789", true));
    t.eq("fuel per call message", kind("while true { }", Limits { fuel: 50, ..Limits::default() }), Some((ErrorKind::Fuel, 1)));
    // limits are not catchable
    t.eq("limit not caught", kind("fn r(n) { return r(n + 1) }\ntry { r(0) } catch e { print(\"gefangen\") }", Limits::default()).map(|k| k.0), Some(ErrorKind::Depth));
}

const YIELD_SRC: &str = r#"let log = []
fn fib(n) { if n < 2 { return n } return fib(n - 1) + fib(n - 2) }
fn make(k) { let acc = []; for i in 0..k { acc.push(fn() { return i * k }) } return acc }
let total = 0
for f in make(6) { total += f() }
let m = {}
for i in 0..40 { m["k${i % 7}"] = (m["k${i % 7}"] ?? 0) + i }
let caught = 0
for i in 0..20 {
  try { if i % 3 == 0 { throw "e${i}" } total += i } catch e { caught += 1; log.push(e) }
}
let s = ""
for c in "größe" { s = c + s }
let junk = 0
for i in 0..300 { let a = {x: [i, "${i}"]}; a.self = a; junk += len(a.x) }
print(fib(15), total, m, caught, log, s, junk, 0.1 * 3.0, 7 / 2)
"#;

const YIELD_OUT: &str = "610 217 {k0: 105, k1: 111, k2: 117, k3: 123, k4: 129, k5: 95, k6: 100} 7 [\"e0\", \"e3\", \"e6\", \"e9\", \"e12\", \"e15\", \"e18\"] eßörg 600 0.30000000000000004 3\n";

/// run to the end with runs of `slice` fuel: (output, result, total fuel, collections, runs)
fn sliced(vm: &mut Vm, slice: u64) -> (String, String, u64, u32, u32) {
    let mut runs = 0;
    loop {
        runs += 1;
        match vm.run(&mut (), slice) {
            Status::Yielded => {}
            Status::Done(v) => return (vm.take_output(), vm.display(v), vm.call_fuel(), vm.collections(), runs),
            Status::Error(e) => return (vm.take_output(), e.to_string(), vm.call_fuel(), vm.collections(), runs),
        }
    }
}

fn yields(t: &mut T) {
    let Ok(prog) = script::compile(YIELD_SRC) else {
        t.check("yield program compiles", false);
        return;
    };
    let lim = Limits { heap: 16 << 10, ..Limits::default() };
    let mut vm = Vm::new(&prog, lim);
    let reference = sliced(&mut vm, u64::MAX);
    t.eq("uninterrupted output", reference.0.as_str(), YIELD_OUT);
    t.eq("uninterrupted run collects", (reference.3 >= 2, reference.4), (true, 1));
    // fixed numbers: fuel and the collector's timing are the same on every target
    t.eq("fuel and collections of the program", (reference.2, reference.3), (29377, 4));
    for slice in [1, 2, 3, 5, 17, 100, 4096] {
        let mut vm = Vm::new(&prog, lim);
        let r = sliced(&mut vm, slice);
        t.eq(&format!("slices of {}", slice), (&r.0, &r.1, r.2, r.3), (&reference.0, &reference.1, reference.2, reference.3));
        if slice == 1 {
            t.check("a run per instruction", r.4 as u64 + 50 > reference.2 && r.4 as u64 <= reference.2 + 1);
        }
    }
    // a clone taken at a yield continues like the original
    let mut vm = Vm::new(&prog, lim);
    for _ in 0..(reference.2 / 2) {
        let _ = vm.run(&mut (), 1);
    }
    let mut twin = vm.clone();
    let a = sliced(&mut vm, 7);
    let b = sliced(&mut twin, u64::MAX);
    t.eq("clone at a yield", (&a.0, &a.1, a.2), (&b.0, &b.1, b.2));
    t.eq("clone output", format!("{}", a.0.len()), format!("{}", b.0.len()));
    // yields of natives in a loop
    let Ok(p2) = script::compile("let s = 0\nfor i in 0..5 { s += pause() }\nprint(s)") else { return };
    let mut vm = Vm::new(&p2, Limits::default());
    let mut h = TestHost::default();
    let mut n = 0;
    while vm.run(&mut h, u64::MAX) == Status::Yielded {
        n += 1;
    }
    t.eq("native yields", (n, vm.take_output().as_str()), (5, "35\n"));
}

fn compile_errors(t: &mut T) {
    let first = |src: &str| script::compile(src).err().and_then(|d| d.first().map(|x| (x.line, x.msg.clone())));
    t.eq("syntax error", first("let = 1"), Some((1, String::from("Nach let fehlt ein Name"))));
    t.eq("range with two variables", first("for i, x in 0..3 { }"), Some((1, String::from("Ein Bereich a..b hat nur eine Laufvariable"))));
    t.eq("break outside", first("let a = 1\nbreak"), Some((2, String::from("break steht außerhalb einer Schleife"))));
    t.eq("duplicate", first("let a = 1\nlet a = 2"), Some((2, String::from("a ist schon deklariert (Zeile 1)"))));
    t.check("warnings do not stop it", script::compile("print(unknown_name)").is_ok());
    // compile_with: the host's names, the built-ins and the library; an unknown name stops it
    let with = |src: &str| script::compile_with("t.jfs", src, &["log", "document"]).err().map(|d| d.iter().map(|x| (x.line, x.col, x.error, x.msg.clone())).collect::<Vec<_>>());
    t.eq("compile_with host and library names", with("log(document.title, math.sqrt(2), \"a\".upper(), len([]), json.encode({}))"), None);
    t.eq("compile_with unknown name", with("let a = 1\nlog(a)\nprnt(a)"), Some(vec![(3, 1, true, String::from("Unbekannter Name prnt – meinst du print?"))]));
    t.eq("compile_with JFNetworker names are unknown", with("timer.after(1, fn() { })").map(|d| d.len()), Some(1));
    t.check("compile_with: other warnings do not stop it", script::compile_with("", "fn f() { return late }\nprint(late2)\nlet late2 = 1\nlet late = 2", &[]).is_ok());
    t.eq("compile_with keeps syntax errors", with("let = 1").map(|d| d[0].3.clone()), Some(String::from("Nach let fehlt ein Name")));
    t.check("check reports the range", script::check("for i, x in 0..3 { }").iter().any(|d| d.error && d.msg.contains("nur eine Laufvariable")));
    let Ok(p) = script::compile("print(1)\nhttp.serve(1)\nconfig.port") else { return };
    t.eq("host names", p.host_names(), vec!["print", "http", "http.serve", "config", "config.port"]);
}

/// the deepest nesting the parser allows, compiled and run on a thread with 256 KB of stack
/// (exact on Linux)
fn deep(t: &mut T) {
    let res = Arc::new(AtomicU32::new(0));
    let r2 = res.clone();
    let h = plat::thread_spawn(256 << 10, Box::new(move || {
        let n = script::MAX_DEPTH as usize - 2;
        let k = n / 3;
        let srcs = [
            (format!("let x = {}1{}\nprint(x)", "(".repeat(n), ")".repeat(n)), "1\n"),
            (format!("let x = {}{}\nprint(len(x))", "[".repeat(n), "]".repeat(n)), "1\n"),
            (format!("{}print(1){}", "if true { ".repeat(n - 3), "}".repeat(n - 3)), "1\n"),
            (format!("let r = {}1{}\nprint(r)", "(fn() { return ".repeat(k), " })()".repeat(k)), "1\n"),
            (format!("let s = {}1{}\nprint(s)", "\"${".repeat(n), "}\"".repeat(n)), "1\n"),
            (format!("let v = {}1\nprint(v)", "-".repeat(n)), if n % 2 == 0 { "1\n" } else { "-1\n" }),
        ];
        let mut bits = 1 << 31;
        for (i, (s, want)) in srcs.iter().enumerate() {
            if out(s) == *want {
                bits |= 1 << i;
            }
        }
        r2.store(bits, AtOrd::SeqCst);
    }));
    if let Some(h) = h {
        h.join();
    }
    t.eq("deepest nesting compiled and run on 256 KB", res.load(AtOrd::SeqCst), (1 << 31) | 0b11_1111);
}
