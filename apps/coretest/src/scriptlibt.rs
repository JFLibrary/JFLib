// scriptlibt -- checks of lib/script's standard library (SK3): str, list, map, bytes, math,
// json, int / float.parse and type() from scripts, Unicode, empty lists, nan and inf, JSON
// round trips and errors with positions, sort stability, the fuel of library calls (fixed
// numbers, a stop in the middle of a big sort), the heap limit for results, the collector
// while callbacks run, yields against an uninterrupted run, the host API (Vm::json_*,
// Cx::spend / keep / json_*), and math bit for bit (vectors and a sweep whose hash must be
// the same on Windows and Linux; tools/script_math_check.py checks the accuracy)
//
//   coretest --math FILE   answers "fn x_hex [y_hex]" lines with the result bits (for
//                          tools/script_math_check.py)

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use script::{math, num, Cx, ErrorKind, Fail, Host, Limits, Status, Value, Vm};

use crate::T;

/// compile and run src to its end: (output, result as text or the error, fuel, collections)
fn exec(src: &str, lim: Limits, host: &mut dyn Host) -> (String, Result<String, script::Error>, u64, u32) {
    let prog = match script::compile(src) {
        Ok(p) => p,
        Err(d) => {
            let msg = d.iter().map(|x| x.to_string()).collect::<Vec<_>>().join("; ");
            let e = script::Error { kind: ErrorKind::Usage, msg: format!("compile: {}", msg), line: 0, col: 0, trace: Vec::new() };
            return (String::new(), Err(e), 0, 0);
        }
    };
    let mut vm = Vm::new(&prog, lim);
    loop {
        match vm.run(host, u64::MAX) {
            Status::Done(v) => return (vm.take_output(), Ok(vm.display(v)), vm.call_fuel(), vm.collections()),
            Status::Yielded => {}
            Status::Error(e) => return (vm.take_output(), Err(e), vm.call_fuel(), vm.collections()),
        }
    }
}

/// the output of src; an error appended as "ERR line:col message"
fn out(src: &str) -> String {
    let (o, r, _, _) = exec(src, Limits::default(), &mut ());
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

/// the kind of the error that ends src under lim (None when it runs through)
fn kind(src: &str, lim: Limits) -> Option<ErrorKind> {
    exec(src, lim, &mut ()).1.err().map(|e| e.kind)
}

/// the fuel of src's main code
fn fuel_of(src: &str) -> u64 {
    exec(src, Limits::default(), &mut ()).2
}

/// the example of docs/GUIDE.md §29
const GUIDE_SRC: &str = r#"let parts = "/users/42/posts".split("/").filter(fn(p) { return len(p) > 0 })
let id = int.parse(parts[1]) ?? 0
let kv = "Content-Type:  text/html".split(":", 2)
print(kv[0].lower(), kv[1].trim())
let jobs = [{n: "b", prio: 2}, {n: "a", prio: 1}, {n: "c", prio: 2}]
jobs.sort(fn(x, y) { return x.prio - y.prio })
print(jobs.map(fn(j) { return j.n }).join(", "))
let t = json.encode({id: 7, tags: ["x"], load: 0.5})
let v = json.decode(t)
print("{:>4}|{:.2}|{:#x}".format(v.id, v.load, 255))
let backoff = math.min(30.0, 0.1 * math.pow(2, 5))
let load = 100 + 50 * math.sin(2 * math.pi * 3 / 24)
let shard = "user:42".hash() % 8
print(id, int.parse("abc") ?? 0, kv, t, backoff, load, shard)
"#;

pub fn run(t: &mut T) {
    t.eq("the example of the guide", out(GUIDE_SRC).as_str(),
        "content-type text/html\na, b, c\n   7|0.50|0xff\n42 0 [\"Content-Type\", \"  text/html\"] {\"id\":7,\"tags\":[\"x\"],\"load\":0.5} 3.2 135.35533905932738 2\n");
    strs(t);
    formats(t);
    lists(t);
    sorting(t);
    maps(t);
    bytes(t);
    maths(t);
    math_bits(t);
    parsing(t);
    json(t);
    json_errors(t);
    errors(t);
    fuel(t);
    heap(t);
    collector(t);
    yields(t);
    host_api(t);
    names(t);
}

fn strs(t: &mut T) {
    cases(t, &[
        ("let s = \"Grüße, Welt\"\nprint(s.len(), len(s), s.slice(0, 5), s.slice(-4), s.slice(7, 100), \"[${s.slice(3, 1)}]\", s.slice(nil, 2))",
         "11 11 Grüße Welt Welt [] Gr\n"),
        ("print(str.len(\"€uro\"), str.slice(\"€uro\", 1, 3), \"abc\".slice(-10, -1), \"\".slice(0), \"äbc\".slice(0, 99))", "4 ur ab  äbc\n"),
        ("print(\"a,b,,c\".split(\",\"), \"a,b,c\".split(\",\", 2), \"\".split(\",\"), \"\".split(), \"abc\".split(\",\"))",
         "[\"a\", \"b\", \"\", \"c\"] [\"a\", \"b,c\"] [\"\"] [] [\"abc\"]\n"),
        ("print(\"  eins  zwei\\tdrei \\n\".split(), \"  eins  zwei drei \".split(nil, 2), \" x \".split(nil, 1))",
         "[\"eins\", \"zwei\", \"drei\"] [\"eins\", \"zwei drei \"] [\"x \"]\n"),
        ("print(\"aß€\".split(\"\"), \"abc\".split(\"\", 2), \"a€b€c\".split(\"€\"), \"a--b\".split(\"--\"), \"--\".split(\"--\"))",
         "[\"a\", \"ß\", \"€\"] [\"a\", \"bc\"] [\"a\", \"b\", \"c\"] [\"a\", \"b\"] [\"\", \"\"]\n"),
        ("print(\", \".join([\"x\", \"y\"]), [\"a\", 1, nil, [2], 2.5].join(\"-\"), [].join(\",\"), [\"a\", \"b\"].join(), str.join(\"+\", [1, 2]))",
         "x, y a-1-nil-[2]-2.5  ab 1+2\n"),
        ("print(\"[${\"  hi \\n\".trim()}]\", \"xxhixx\".trim(\"x\"), \"--a-b--\".trim_start(\"-\"), \"--a-b--\".trim_end(\"-\"), \"[${\"\\u{3000}x\\u{2003}\".trim()}]\", \"ab\".trim(\"ba\") == \"\")",
         "[hi] hi a-b-- --a-b [x] true\n"),
        ("print(\"Größe\".find(\"ö\"), \"abcabc\".find(\"c\", 3), \"abc\".find(\"x\"), \"abc\".find(\"\"), \"aßc\".find(\"c\"), \"abcabc\".find(\"a\", -3), \"abc\".find(\"a\", 99))",
         "2 5 nil 0 2 3 nil\n"),
        ("print(\"€uro€\".find_last(\"€\"), \"abc\".find_last(\"z\"), \"aaa\".find_last(\"a\"), \"x\".find_last(\"\"))", "4 nil 2 1\n"),
        ("print(\"Hallo\".contains(\"all\"), \"Hallo\".contains(\"\"), \"Hallo\".starts_with(\"Ha\"), \"Hallo\".ends_with(\"lo\"), \"Hallo\".starts_with(\"lo\"))",
         "true true true true false\n"),
        ("print(\"banana\".count(\"an\"), \"aaa\".count(\"aa\"), \"abc\".count(\"\"), \"äää\".count(\"ä\"), \"\".count(\"x\"))", "2 1 4 3 0\n"),
        ("print(\"a-b-c\".replace(\"-\", \"+\"), \"a-b-c\".replace(\"-\", \"\", 1), \"äöü\".replace(\"ö\", \"oe\"), \"abc\".replace(\"\", \"-\"), \"abc\".replace(\"x\", \"y\"), \"aaa\".replace(\"a\", \"b\", 0))",
         "a+b+c ab-c äoeü -a-b-c- abc aaa\n"),
        ("print(\"straße\".upper(), \"ÄÖÜ Ab\".lower(), \"ǆ\".upper(), \"ΣΑΣ\".lower(), \"abc\".upper().lower())", "STRASSE äöü ab Ǆ σας abc\n"),
        ("print(\"ab\".repeat(3), \"[${\"x\".repeat(0)}]\", \"€\".repeat(2), \"\".repeat(1000000000000))", "ababab [] €€ \n"),
        ("print(\"aß€\".chars(), \"\".chars(), \"aß\".bytes(), \"€\".code(), \"aß\".code(1), str.from_code(72, 105, 0x1F600), str.from_code())",
         "[\"a\", \"ß\", \"€\"] [] bytes(61 c3 9f) 8364 223 Hi😀 \n"),
        ("print(\"\".hash(), \"a\".hash(), \"foobar\".hash(), bytes(\"a\").hash() == \"a\".hash())", "2166136261 3826002220 3214735720 true\n"),
        ("let s = \"x,y\"\nprint(s.split(\",\") == str.split(s, \",\"), str.split(s, \",\"), s.upper() == str.upper(s))", "false [\"x\", \"y\"] true\n"),
        ("let p = \"/users/42/posts\"\nlet parts = p.split(\"/\").filter(fn(x) { return len(x) > 0 })\nprint(parts, int.parse(parts[1]) + 1, p.starts_with(\"/users/\"), p.slice(p.find_last(\"/\") + 1))",
         "[\"users\", \"42\", \"posts\"] 43 true posts\n"),
        ("let h = \"Content-Type:  text/html \"\nlet kv = h.split(\":\", 2)\nprint(\"[${kv[0].lower()}]\", \"[${kv[1].trim()}]\")", "[content-type] [text/html]\n"),
    ]);
}

fn formats(t: &mut T) {
    cases(t, &[
        ("print(\"{} + {} = {}\".format(1, 2, 3), \"{1}{0}{1}\".format(\"a\", \"b\"), \"{name}:{port}\".format({name: \"web\", port: 80}))",
         "1 + 2 = 3 bab web:80\n"),
        ("print(\"{:>5}|{:<5}|{:^5}|{:^6}|\".format(1, 2, 3, \"ab\"))", "    1|2    |  3  |  ab  |\n"),
        ("print(\"{:05}|{:+}|{:+}|{:05}|{:<05}|{:x>4}|{:06.2}\".format(-42, 5, -5, \"ab\", 7, \"z\", 3.14159))",
         "-0042|+5|-5|ab000|70000|xxxz|003.14\n"),
        ("print(\"{:.2}|{:.2}|{:.0}|{:.3}|{:.1}|{:.1}|{:.1}\".format(3.14159, 2, 2.5, \"abcdef\", 0.25, 0.35, -0.04))",
         "3.14|2.00|2|abc|0.2|0.3|-0.0\n"),
        ("print(\"{:x} {:X} {:#x} {:b} {:#o} {:x} {:#06x} {:d}\".format(255, 255, 255, 5, 8, -255, 255, 42))",
         "ff FF 0xff 101 0o10 -ff 0x00ff 42\n"),
        ("print(\"{:e}|{:.2e}|{:e}|{:f}|{:.3f}|{:e}\".format(1234.5, 1234.5, 0.00012, 1.5, 2, 7))", "1.2345e3|1.23e3|1.2e-4|1.500000|2.000|7e0\n"),
        ("print(\"{:*^9}|{:-<4}|{:>4}|\".format(\"mitte\", \"a\", \"äö\"))", "**mitte**|a---|  äö|\n"),
        ("print(\"{{}} {} {{x}}\".format(1), \"{}\".format([1, \"a\"]), \"{:5}|{:>5}\".format(true, nil), \"{:s}\".format(1.0))",
         "{} 1 {x} [1, \"a\"] true |  nil 1.0\n"),
        ("print(\"{:.2}|{:8.1}|{:+}|{:5}|{}|{:.1}\".format(math.nan, -math.inf, math.inf, -0.0, -0.0, 1e21))",
         "nan|    -inf|+inf| -0.0|-0.0|1000000000000000000000.0\n"),
        ("print(str.format(\"{}-{}\", \"a\", \"b\"), \"\".format(), \"ohne\".format(1, 2), \"{0}{0}\".format(\"€\"))", "a-b  ohne €€\n"),
        ("print(\"{} {}\".format(1))", "ERR 1:15 str.format: Es gibt kein Argument 1"),
        ("print(\"{3}\".format(1))", "ERR 1:13 str.format: Es gibt kein Argument 3"),
        ("print(\"a{\".format())", "ERR 1:12 str.format: Nach { fehlt die schließende }"),
        ("print(\"}\".format())", "ERR 1:11 str.format: Eine } ohne öffnende {"),
        ("print(\"{:q}\".format(1))", "ERR 1:14 str.format: Ungültige Formatangabe {:q}"),
        ("print(\"{:10001}\".format(1))", "ERR 1:18 str.format: Ungültige Formatangabe {:10001}"),
        ("print(\"{:x}\".format(1.5))", "ERR 1:14 str.format: {:x} braucht int, nicht float"),
        ("print(\"{:e}\".format(\"a\"))", "ERR 1:14 str.format: {:e} braucht eine Zahl, nicht str"),
        ("print(\"{nope}\".format({a: 1}))", "ERR 1:16 str.format: Die Map hat kein Feld nope"),
        ("print(\"{a}\".format(5))", "ERR 1:13 str.format: Für {a} muss das erste Argument eine Map sein, nicht int"),
        ("print(\"{a}\".format())", "ERR 1:13 str.format: Für {a} muss das erste Argument eine Map sein, nicht nichts"),
    ]);
}

fn lists(t: &mut T) {
    cases(t, &[
        ("let l = [1, 2, 3]\nl.insert(0, 0)\nl.insert(4, 4)\nprint(l)\nprint(l.remove(2))\nprint(l, l.len(), list.len(l))", "[0, 1, 2, 3, 4]\n2\n[0, 1, 3, 4] 4 4\n"),
        ("let l = []\nlist.push(l, 1, 2)\nl.push(3)\nprint(l, list.pop(l), l.pop(), l, [].pop())", "[1] 3 2 [1] nil\n"),
        ("print([1, 2, 3].slice(1), [1, 2, 3].slice(-2, -1), [].slice(0, 5), [1, 2].slice(5), [1, 2, 3].slice(2, 1), [1].slice())", "[2, 3] [2] [] [] [] [1]\n"),
        ("let l = [1, 2, 3]\nprint(l.reverse(), l, [].reverse(), l.reverse() == l)", "[1, 2, 3] [1, 2, 3] [] true\n"),
        ("print([1, 2, 3].map(fn(x) { return x * x }), [1, 2, 3, 4].filter(fn(x) { return x % 2 == 0 }), [].map(fn(x) { return x }), [].filter(print))",
         "[1, 4, 9] [2, 4] [] []\n"),
        ("print([\" a \", \"b \"].map(str.trim), [\"1\", \"x\", \"3\"].map(int.parse), [1, 2, 3].map(str))", "[\"a\", \"b\"] [1, nil, 3] [\"1\", \"2\", \"3\"]\n"),
        ("print([1, 2, 3, 4].reduce(fn(a, b) { return a + b }), [].reduce(fn(a, b) { return a + b }, 0), [\"a\", \"b\"].reduce(fn(a, b) { return a + b }, \">\"), [7].reduce(fn(a, b) { return 0 }))",
         "10 0 >ab 7\n"),
        ("print([1, \"a\", nil].contains(nil), [1, 2].contains(1.0), [[1]].contains([1]), [\"a\"].index(\"a\"), [].index(1), [1, 2, 1].index(1), [].contains(nil))",
         "true true false 0 nil 0 false\n"),
        ("let l = [1, 2, 3]\nlet m = l.map(fn(x) { l.push(x); return x })\nprint(m, l)", "[1, 2, 3] [1, 2, 3, 1, 2, 3]\n"),
        ("[].reduce(fn(a, b) { return a })", "ERR 1:4 list.reduce: Eine leere Liste braucht einen Startwert (init)"),
        ("[].remove(0)", "ERR 1:4 list.remove: Index 0 liegt außerhalb der Liste (Länge 0)"),
        ("[1].insert(-1, 0)", "ERR 1:5 list.insert: Index -1 liegt außerhalb der Liste (Länge 1)"),
        ("[1].insert(2, 0)", "ERR 1:5 list.insert: Index 2 liegt außerhalb der Liste (Länge 1)"),
        ("[1].remove(\"a\")", "ERR 1:5 list.remove: i muss int sein, nicht str"),
        ("[1].map(fn(x) { throw \"innen ${x}\" })", "ERR 1:17 innen 1"),
        ("print([1].filter(\"x\"))", "ERR 1:11 list.filter: f muss eine Funktion sein, nicht str"),
        ("list.sort(5)", "ERR 1:6 list.sort: l muss list sein, nicht int"),
    ]);
}

fn sorting(t: &mut T) {
    cases(t, &[
        ("print([3, 1, 2].sort(), [\"b\", \"a\", \"C\", \"ä\"].sort(), [].sort(), [5].sort(), [2.5, 1, math.nan, -math.inf, 2, -0.0, 0].sort())",
         "[1, 2, 3] [\"C\", \"a\", \"b\", \"ä\"] [] [5] [-inf, -0.0, 0, 1, 2, 2.5, nan]\n"),
        ("print([1.0, 1, 0, 1.0, -0.0, 0].sort(), [bytes(2), bytes(1, 0), bytes(1)].sort())", "[0, -0.0, 0, 1.0, 1, 1.0] [bytes(01), bytes(01 00), bytes(02)]\n"),
        ("let people = [{n: \"a\", age: 30}, {n: \"b\", age: 25}, {n: \"c\", age: 30}, {n: \"d\", age: 25}, {n: \"e\", age: 30}]\npeople.sort(fn(x, y) { return x.age - y.age })\nprint(people.map(fn(p) { return p.n }).join())\npeople.sort(fn(x, y) { return x.age > y.age })\nprint(people.map(fn(p) { return p.n }).join())\nlist.sort(people, fn(x, y) { return x.n < y.n })\nprint(people.map(fn(p) { return p.n }).join())",
         "bdace\nacebd\nabcde\n"),
        ("let l = []\nfor i in 0..1000 { l.push({k: (i * 7919) % 13, i: i}) }\nl.sort(fn(a, b) { return a.k - b.k })\nlet ok = true\nfor j in 1..1000 {\n  let a = l[j - 1]\n  let b = l[j]\n  if a.k > b.k || (a.k == b.k && a.i > b.i) { ok = false }\n}\nprint(ok, l[0].k, l[999].k)",
         "true 0 12\n"),
        ("let l = []\nfor i in 0..500 { l.push((i * 7919) % 1000 * 0.5) }\nlet s = l.slice().sort()\nlet ok = true\nfor j in 1..500 { if s[j - 1] > s[j] { ok = false } }\nprint(ok, s[0], s[499], len(s), l[1])", "true 0.0 499.5 500 459.5\n"),
        ("let l = [3, 1, 2]\ntry { l.sort(fn(a, b) { if a == 2 { throw \"halt\" } return a - b }) } catch e { print(e) }\nprint(l)", "halt\n[3, 1, 2]\n"),
        ("let l = [2, 1]\ntry { l.sort(fn(a, b) { return \"x\" }) } catch e { print(e) }\nprint(l)", "list.sort: cmp muss eine Zahl oder bool zurückgeben, nicht str\n[2, 1]\n"),
        ("let l = [1, \"a\", 2]\ntry { l.sort() } catch e { print(e) }\nprint(l)", "list.sort: str und int lassen sich nicht vergleichen – gib eine Vergleichsfunktion an\n[1, \"a\", 2]\n"),
        ("print([nil, nil].sort())", "ERR 1:18 list.sort: nil und nil lassen sich nicht vergleichen – gib eine Vergleichsfunktion an"),
        ("print([3, 1, 2].sort(fn(a, b) { return b - a }), [0.5, 0.25].sort(fn(a, b) { return a - b }), [\"bb\", \"a\", \"ccc\"].sort(fn(a, b) { return len(a) - len(b) }))",
         "[3, 2, 1] [0.25, 0.5] [\"a\", \"bb\", \"ccc\"]\n"),
        ("let calls = 0\nlet l = []\nfor i in 0..100 { l.push(i) }\nl.sort(fn(a, b) { calls += 1; return a - b })\nprint(calls)", "99\n"),
    ]);
}

fn maps(t: &mut T) {
    cases(t, &[
        ("let m = {z: 1, a: 2, m: 3}\nprint(m.keys(), m.values(), m.entries(), m.len(), map.len(m))",
         "[\"z\", \"a\", \"m\"] [1, 2, 3] [[\"z\", 1], [\"a\", 2], [\"m\", 3]] 3 3\n"),
        ("let m = {z: 1, a: 2, m: 3}\nprint(m.has(\"a\"), m.has(\"q\"), m.get(\"q\", 0), m.get(\"z\"), m.get(\"q\"))\nprint(m.delete(\"a\"), m.delete(\"a\"), m)\nm.a = 9\nprint(m.keys())",
         "true false 0 1 nil\ntrue false {z: 1, m: 3}\n[\"z\", \"m\", \"a\"]\n"),
        ("let d = {host: \"x\", port: 80}\nprint(d.merge({port: 8080, tls: true}), d)\nprint({}.merge(), {a: 1}.merge({b: 2}, {a: 3}), map.merge({1: \"i\"}, {true: \"b\"}))",
         "{host: \"x\", port: 8080, tls: true} {host: \"x\", port: 80}\n{} {a: 3, b: 2} {1: \"i\", true: \"b\"}\n"),
        ("let m = {}\nfor i in 0..100 { m[i] = i * i }\nfor i in 0..100 { if i % 3 != 0 { m.delete(i) } }\nprint(m.len(), m.keys().slice(0, 4), m.values().slice(-2), m.has(99), m.has(98))",
         "34 [0, 3, 6, 9] [9216, 9801] true false\n"),
        ("let m = {nil_wert: nil}\nprint(m.has(\"nil_wert\"), m.get(\"nil_wert\", 5), m.nil_wert ?? 5, {}.keys(), {}.entries())", "true nil 5 [] []\n"),
        ("print(map.keys({keys: 1}), {a: 1}.keys())", "[\"keys\"] [\"a\"]\n"),
        ("print({keys: 1}.keys())", "ERR 1:17 Das Feld keys ist keine Funktion, sondern int"),
        ("print({a: 1}.has(1.5))", "ERR 1:14 map.has: Ein Map-Schlüssel muss str, int oder bool sein, nicht float"),
        ("print({a: 1}.merge(5))", "ERR 1:14 map.merge: other muss map sein, nicht int"),
        ("print(map.merge())", "ERR 1:11 map.merge: m muss map sein, nicht nil"),
    ]);
}

fn bytes(t: &mut T) {
    cases(t, &[
        ("let b = bytes(\"Grüß\")\nprint(b, len(b), b.len(), b.hex(), b.text(), b[2], type(b))", "bytes(47 72 c3 bc c3 9f) 6 6 4772c3bcc39f Grüß 195 bytes\n"),
        ("let b = bytes(\"x\")\nprint(bytes([1, 2, 255]), bytes(65, 66), bytes(), bytes(b) == b, bytes(7), bytes([]).hex(), bytes(\"\").text() == \"\")",
         "bytes(01 02 ff) bytes(41 42) bytes() true bytes(07)  true\n"),
        ("print(bytes.from_hex(\"00ff10\").slice(1), bytes.from_hex(\"ABcd\"), bytes.from_hex(\"\"), bytes(\"Grüß\").slice(-2), bytes(\"ab\").slice(10), bytes(\"abc\").slice(1, 2).text())",
         "bytes(ff 10) bytes(ab cd) bytes() bytes(c3 9f) bytes() b\n"),
        ("print(bytes([0x41, 0xFF, 0x42]).text(true), bytes([0xE2, 0x82]).text(true).len(), bytes(\"€\").text(), bytes(\"a\") + bytes(\"b\"))", "A\u{FFFD}B 1 € bytes(61 62)\n"),
        ("print(bytes.from_hex(\"0g\"))", "ERR 1:13 bytes.from_hex: g ist keine Hex-Ziffer (Index 1)"),
        ("print(bytes.from_hex(\"abc\"))", "ERR 1:13 bytes.from_hex: Die Zahl der Hex-Ziffern ist ungerade"),
        ("print(bytes([0x61, 0xC3]).text())", "ERR 1:27 bytes.text: Die Bytes sind kein gültiges UTF-8 (ab Index 1)"),
        ("print(bytes([256]))", "ERR 1:7 bytes: 256 ist kein Byte (0 bis 255)"),
        ("print(bytes(1, \"a\"))", "ERR 1:7 bytes: Ein Byte muss int sein, nicht str"),
        ("print(bytes({}))", "ERR 1:7 bytes braucht str, list, bytes oder Bytes als int, nicht map"),
        ("print(bytes.hex(\"ab\"))", "ERR 1:13 bytes.hex: b muss bytes sein, nicht str"),
    ]);
}

fn maths(t: &mut T) {
    cases(t, &[
        ("print(math.abs(-3), math.abs(-2.5), math.abs(int.min), math.min(3, 1.5, 2), math.max([4, 9, 2]), math.max(1, 1.0), math.min(1.0, 1), math.min([7]))",
         "3 2.5 -9223372036854775808 1.5 9 1 1.0 7\n"),
        ("print(math.floor(2.7), math.floor(-2.5), math.ceil(2.1), math.ceil(-0.5), math.round(2.5), math.round(-2.5), math.round(0.49999999999999994), math.round(-0.4), math.trunc(-2.7), math.floor(7), math.round(-3))",
         "2.0 -3.0 3.0 -0.0 3.0 -3.0 0.0 -0.0 -2.0 7 -3\n"),
        ("print(math.sqrt(16), math.sqrt(2), math.pow(2, 10), math.pow(2, 0.5), math.pow(-2, 3), math.pow(10, -2), math.exp(0), math.exp(1), math.log(1), math.log(math.e))",
         "4.0 1.4142135623730951 1024.0 1.4142135623730951 -8.0 0.01 1.0 2.7182818284590455 0.0 1.0\n"),
        ("print(math.log2(1024), math.log2(0.125), math.log2(3), math.log10(1000), math.log10(0.001), math.log10(2), math.log(10))",
         "10.0 -3.0 1.584962500721156 3.0 -3.0 0.3010299956639812 2.302585092994046\n"),
        ("print(math.sin(0), math.cos(0), math.sin(math.pi / 2), math.cos(math.pi), math.sin(math.pi), math.sin(-0.0), math.cos(1e300), math.sin(1e22))",
         "0.0 1.0 1.0 -1.0 1.2246467991473532e-16 -0.0 -0.5753861119575491 -0.8522008497671888\n"),
        ("print(math.pi, math.e, math.tau, math.inf, -math.inf, math.nan, int.max, int.min, int.max + 1 == int.min)",
         "3.141592653589793 2.718281828459045 6.283185307179586 inf -inf nan 9223372036854775807 -9223372036854775808 true\n"),
        ("print(math.sqrt(-1), math.log(0), math.log(-1), math.exp(1000), math.exp(-1000), math.pow(0, -1), math.pow(-8, 1.0 / 3.0), math.sin(math.inf), math.cos(-math.inf))",
         "nan -inf nan inf 0.0 inf nan nan nan\n"),
        ("print(math.pow(math.nan, 0), math.pow(1, math.nan), math.pow(-1, math.inf), math.pow(0.5, -math.inf), math.pow(-0.0, -1), math.pow(-math.inf, 3), math.pow(2, 1024), math.pow(2, -1075))",
         "1.0 1.0 1.0 inf -inf -inf inf 0.0\n"),
        ("print(math.min(1, math.nan, 0), math.max(math.nan), math.clamp(15, 0, 10), math.clamp(-1.5, 0, 10), math.clamp(5, 0, 10.0), math.clamp(math.nan, 0, 1))",
         "nan nan 10 0 5 nan\n"),
        ("print(math.is_nan(math.nan), math.is_nan(1), math.is_nan(math.inf), math.is_finite(math.inf), math.is_finite(math.nan), math.is_finite(3), math.is_finite(-1e308))",
         "true false false false false true true\n"),
        ("print(math.nan == math.nan, math.nan != math.nan, math.inf > 1e308, -math.inf < int.min, 0.0 == -0.0, 1 / math.inf)", "false true true true true 0.0\n"),
        ("print(math.max())", "ERR 1:12 math.max braucht mindestens einen Wert"),
        ("print(math.max([]))", "ERR 1:12 math.max braucht mindestens einen Wert"),
        ("print(math.sqrt(\"4\"))", "ERR 1:12 math.sqrt: x muss eine Zahl sein, nicht str"),
        ("print(math.min(1, \"a\"))", "ERR 1:12 math.min: x muss eine Zahl sein, nicht str"),
        ("print(math.clamp(1, 5, 0))", "ERR 1:12 math.clamp: lo ist größer als hi"),
        ("print(math.clamp(1, math.nan, 2))", "ERR 1:12 math.clamp: lo muss eine Zahl (nicht nan) sein, nicht float"),
        ("print(math.pi())", "ERR 1:12 math.pi ist keine Funktion"),
        ("print(math.nope(1))", "ERR 1:12 Unbekannte Funktion math.nope"),
    ]);
}

/// (function, input bits, second input bits, result bits): results of script::math checked
/// against Python's decimal module with tools/script_math_check.py (below 1 ulp; exp and pow
/// are also the bits of fdlibm / Java's StrictMath)
const MATH_BITS: [(&str, u64, u64, u64); 47] = [
    ("exp", 0x3FF0000000000000, 0x0000000000000000, 0x4005BF0A8B14576A),
    ("exp", 0xBFF0000000000000, 0x0000000000000000, 0x3FD78B56362CEF38),
    ("exp", 0x3FE0000000000000, 0x0000000000000000, 0x3FFA61298E1E069C),
    ("exp", 0x4024000000000000, 0x0000000000000000, 0x40D5829DCF950560),
    ("exp", 0xC024000000000000, 0x0000000000000000, 0x3F07CD79B5647C9A),
    ("exp", 0x4085E00000000000, 0x0000000000000000, 0x7F0D945DF4F8EC8E),
    ("exp", 0xC087200000000000, 0x0000000000000000, 0x0000000000000055),
    ("exp", 0x3DDB7CDFD9D7BDBB, 0x0000000000000000, 0x3FF000000006DF38),
    ("exp", 0x40562CCCCCCCCCCD, 0x0000000000000000, 0x47EF4705BBFFAE5C),
    ("log", 0x4000000000000000, 0x0000000000000000, 0x3FE62E42FEFA39EF),
    ("log", 0x4024000000000000, 0x0000000000000000, 0x40026BB1BBB55516),
    ("log", 0x3FE0000000000000, 0x0000000000000000, 0xBFE62E42FEFA39EF),
    ("log", 0x01A56E1FC2F8F359, 0x0000000000000000, 0xC085963447F87FB5),
    ("log", 0x0000000000000001, 0x0000000000000000, 0xC0874385446D71C3),
    ("log", 0x7E37E43C8800759C, 0x0000000000000000, 0x4085963447F87FB5),
    ("log", 0x3FF000001AD7F29B, 0x0000000000000000, 0x3E7AD7F2847B6492),
    ("log", 0x401C000000000000, 0x0000000000000000, 0x3FFF2272AE325A57),
    ("log2", 0x4008000000000000, 0x0000000000000000, 0x3FF95C01A39FBD68),
    ("log2", 0x4024000000000000, 0x0000000000000000, 0x400A934F0979A371),
    ("log2", 0x000012688B70E62B, 0x0000000000000000, 0xC0901730DABCA5F6),
    ("log2", 0x3FB999999999999A, 0x0000000000000000, 0xC00A934F0979A371),
    ("log10", 0x4000000000000000, 0x0000000000000000, 0x3FD34413509F79FF),
    ("log10", 0x401C000000000000, 0x0000000000000000, 0x3FEB0B0B0B78CC3F),
    ("log10", 0x3EE4F8B588E368F1, 0x0000000000000000, 0xC014000000000000),
    ("log10", 0x40C81CD6C8B43958, 0x0000000000000000, 0x40105DB618083A9E),
    ("sin", 0x3FF0000000000000, 0x0000000000000000, 0x3FEAED548F090CEE),
    ("sin", 0x3FE0000000000000, 0x0000000000000000, 0x3FDEAEE8744B05F0),
    ("sin", 0x4008000000000000, 0x0000000000000000, 0x3FC210386DB6D55B),
    ("sin", 0x4059000000000000, 0x0000000000000000, 0xBFE03425B78C4DB8),
    ("sin", 0x412E848000000000, 0x0000000000000000, 0xBFD6664B2568D867),
    ("sin", 0x4480F0CF064DD592, 0x0000000000000000, 0xBFEB453AB76BF397),
    ("sin", 0xC004000000000000, 0x0000000000000000, 0xBFE326AF0DCFCAB0),
    ("sin", 0x3E112E0BE826D695, 0x0000000000000000, 0x3E112E0BE826D695),
    ("cos", 0x3FF0000000000000, 0x0000000000000000, 0x3FE14A280FB5068C),
    ("cos", 0x4000000000000000, 0x0000000000000000, 0xBFDAA22657537205),
    ("cos", 0x3FE8000000000000, 0x0000000000000000, 0x3FE769FEC655211F),
    ("cos", 0x4202A05F20000000, 0x0000000000000000, 0x3FEBF098901C931A),
    ("cos", 0x7E37E43C8800759C, 0x0000000000000000, 0xBFE2699022ADC4C1),
    ("cos", 0xC059000000000000, 0x0000000000000000, 0x3FEB981DBF665FDF),
    ("pow", 0x4000000000000000, 0x3FE0000000000000, 0x3FF6A09E667F3BCD),
    ("pow", 0x4024000000000000, 0xC008000000000000, 0x3F50624DD2F1A9FC),
    ("pow", 0x3FF8000000000000, 0x4059200000000000, 0x439BA4104F641D8C),
    ("pow", 0xC004000000000000, 0x4008000000000000, 0xC02F400000000000),
    ("pow", 0x3FB999999999999A, 0x3FB999999999999A, 0x3FE96B230BCDC434),
    ("pow", 0x401C000000000000, 0x3FD5555555555555, 0x3FFE9B5DBA58189D),
    ("pow", 0x4202A05F20000000, 0x403E000000000000, 0x7E37E43C8800759C),
    ("pow", 0x4000000000000000, 0xC090C80000000000, 0x0000000000000001),
];

fn next(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn apply(f: &str, x: f64, y: f64) -> f64 {
    match f {
        "exp" => math::exp(x),
        "log" => math::log(x),
        "log2" => math::log2(x),
        "log10" => math::log10(x),
        "sin" => math::sin(x),
        "cos" => math::cos(x),
        "pow" => math::pow(x, y),
        "round" => math::round(x),
        _ => num::sqrt(x),
    }
}

fn math_bits(t: &mut T) {
    for (f, x, y, want) in MATH_BITS {
        let got = apply(f, f64::from_bits(x), f64::from_bits(y)).to_bits();
        t.eq(&format!("{}({:016x}, {:016x})", f, x, y), got, want);
    }
    // a sweep over random bits and the ranges that matter: the hash is the same on every target
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut hash = 0xCBF2_9CE4_8422_2325u64;
    for f in ["exp", "log", "log2", "log10", "sin", "cos", "pow", "round", "sqrt"] {
        for i in 0..4000 {
            let r = next(&mut seed);
            let x = match i % 4 {
                0 => f64::from_bits(r),
                1 => f64::from_bits((r & 0x800F_FFFF_FFFF_FFFF) | ((1023 - 30 + (r >> 52) % 60) << 52)),
                2 => num::div(num::from_i64((r % 2_000_000) as i64 - 1_000_000), 1000.0),
                _ => f64::from_bits((r & 0x000F_FFFF_FFFF_FFFF) | ((1023 + (r >> 52) % 9) << 52)),
            };
            let y = num::div(num::from_i64((next(&mut seed) % 20_000) as i64 - 10_000), 100.0);
            hash = (hash ^ apply(f, x, y).to_bits()).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    t.eq("math sweep hash", format!("{:016x}", hash), String::from("f881eabf6fcbbf80"));
}

fn parsing(t: &mut T) {
    cases(t, &[
        ("print(int.parse(\"42\"), int.parse(\" -17 \"), int.parse(\"0x1F\"), int.parse(\"ff\", 16), int.parse(\"0b101\"), int.parse(\"z\", 36), int.parse(\"+5\"), int.parse(\"0o17\"), int.parse(\"0x1f\", 16))",
         "42 -17 31 255 5 35 5 15 31\n"),
        ("print(int.parse(\"12abc\"), int.parse(\"\"), int.parse(\"-\"), int.parse(\"0x\"), int.parse(\"1_000\"), int.parse(\"9223372036854775808\"), int.parse(\"1.5\"), int.parse(\"٣\"), int.parse(\"0b2\"))",
         "nil nil nil nil nil nil nil nil nil\n"),
        ("print(int.parse(\"9223372036854775807\"), int.parse(\"-9223372036854775808\"), int.parse(\"0xFFFFFFFFFFFFFFFF\"), int.parse(\"-0x8000000000000000\"), int.parse(\"0x10000000000000000\"), int.parse(\"11\", 2))",
         "9223372036854775807 -9223372036854775808 -1 -9223372036854775808 nil 3\n"),
        ("print(float.parse(\"1.5\"), float.parse(\"-1e3\"), float.parse(\"inf\"), float.parse(\"-Infinity\"), float.parse(\"nan\"), float.parse(\".5\"), float.parse(\"1,5\"), float.parse(\"\"), float.parse(\" 2 \"), float.parse(\"0x10\"))",
         "1.5 -1000.0 inf -inf nan 0.5 nil nil 2.0 nil\n"),
        ("print(float.parse(\"0.1\") == 0.1, float.parse(\"1e400\"), float.parse(\"1e-400\"), float.parse(\"-0\"), float.parse(\"4.9e-324\"))", "true inf 0.0 -0.0 5e-324\n"),
        ("print(int(\"0x10\"), int(\" 7 \"), float(\"2\"), int(2.9), int(-2.9), float(3), int(\"-0b11\"), str(1.0), str(1e21))", "16 7 2.0 2 -2 3.0 -3 1.0 1e21\n"),
        ("print(type(1), type(1.5), type(\"s\"), type(bytes()), type([]), type({}), type(nil), type(true), type(print), type(str.split), type(fn() {}), type(math.pi))",
         "int float str bytes list map nil bool fn fn fn float\n"),
        ("int.parse(\"7\", 1)", "ERR 1:5 int.parse: base muss zwischen 2 und 36 liegen, nicht 1"),
        ("int.parse(7)", "ERR 1:5 int.parse: s muss str sein, nicht int"),
        ("float.parse(nil)", "ERR 1:7 float.parse: s muss str sein, nicht nil"),
        ("int(\"0x\")", "ERR 1:1 int(…): \"0x\" ist keine ganze Zahl"),
    ]);
}

const JSON_SRC: &str = r#"let v = {name: "Grüße 😀", n: 3, x: 1.5, neg: -0.0, ok: true, none: nil, list: [1, [2, {}], []], "a b": "q\"\\\n\t", big: 9223372036854775807}
let t = json.encode(v)
print(t)
let back = json.decode(t)
print(back)
print(json.encode(back) == t, back.keys() == nil, back.keys())
print(json.encode({a: [1, 2], b: {}, c: []}, 2))
print(json.encode([]), json.encode({}), json.encode("ä\u{1}"), json.encode(nil), json.encode(0.1), json.encode(1e21), json.encode(5e-324), json.encode({1: "i", true: "b"}))
let x = [1]
print(json.encode([x, x]), json.encode(-9223372036854775807 - 1), json.encode(1.0), json.encode(-1.5e-7))
"#;

const JSON_OUT: &str = "{\"name\":\"Grüße 😀\",\"n\":3,\"x\":1.5,\"neg\":-0.0,\"ok\":true,\"none\":null,\"list\":[1,[2,{}],[]],\"a b\":\"q\\\"\\\\\\n\\t\",\"big\":9223372036854775807}
{name: \"Grüße 😀\", n: 3, x: 1.5, neg: -0.0, ok: true, none: nil, list: [1, [2, {}], []], \"a b\": \"q\\\"\\\\\\n\\t\", big: 9223372036854775807}
true false [\"name\", \"n\", \"x\", \"neg\", \"ok\", \"none\", \"list\", \"a b\", \"big\"]
{
  \"a\": [
    1,
    2
  ],
  \"b\": {},
  \"c\": []
}
[] {} \"ä\\u0001\" null 0.1 1e21 5e-324 {\"1\":\"i\",\"true\":\"b\"}
[[1],[1]] -9223372036854775808 1.0 -1.5e-7\n";

fn json(t: &mut T) {
    t.eq("json round trip", out(JSON_SRC).as_str(), JSON_OUT);
    cases(t, &[
        ("print(json.decode(\"{\\\"z\\\":1,\\\"a\\\":2,\\\"m\\\":{\\\"y\\\":0,\\\"b\\\":1}}\").keys(), json.decode(\"{\\\"z\\\":1,\\\"a\\\":2,\\\"m\\\":{\\\"y\\\":0,\\\"b\\\":1}}\").m)",
         "[\"z\", \"a\", \"m\"] {y: 0, b: 1}\n"),
        ("print(json.decode(\"[1, 1.0, 1e2, -0, 12345678901234567890, 9223372036854775807, -9223372036854775808, 0.1, 1E-2, -0.0]\"))",
         "[1, 1.0, 100.0, 0, 1.2345678901234567e19, 9223372036854775807, -9223372036854775808, 0.1, 0.01, -0.0]\n"),
        ("print(json.decode(\"\\\"\\\\u00e4\\\\ud83d\\\\ude00\\\\/\\\\b\\\"\") == \"ä😀/\\u{8}\", json.decode(\" true \"), json.decode(\"null\"), json.decode(\"[]\"), json.decode(\"{}\"), json.decode(\"1e400\"))",
         "true true nil [] {} inf\n"),
        ("print(json.decode(bytes(\"[1,\\\"€\\\"]\")), json.encode(json.decode(\"[1.5, 2, \\\"x\\\"]\")))", "[1, \"€\"] [1.5,2,\"x\"]\n"),
        ("let d = []\nfor i in 0..64 { d = [d] }\nlet t = json.encode(d)\nprint(len(t), json.encode(json.decode(t)) == t)\ntry { json.encode([d]) } catch e { print(e) }\ntry { json.decode(\"[\" + t + \"]\") } catch e { print(e) }",
         "130 true\njson.encode: Die Daten sind zu tief verschachtelt (mehr als 64 Ebenen)\njson.decode: Die Daten sind zu tief verschachtelt (mehr als 64 Ebenen) (Zeile 1, Spalte 66)\n"),
        ("let m = {}\nfor i in 0..5000 { m[\"k${i}\"] = i }\nlet t = json.encode(m)\nlet back = json.decode(t)\nprint(len(back), back.k4999, back.keys()[0], json.encode(back) == t)\ntry { json.decode(t.slice(0, -1) + \",\\\"k17\\\":1}\") } catch e { print(e.slice(0, 44)) }",
         "5000 4999 k0 true\njson.decode: Der Schlüssel kommt doppelt vor\n"),
        ("let l = []\nfor i in 0..3000 { l.push({id: i, tags: [\"t${i % 3}\"], score: i / 7.0}) }\nlet back = json.decode(json.encode(l))\nprint(len(back), back[2999].tags, back[7].score, json.encode(back) == json.encode(l))",
         "3000 [\"t2\"] 1.0 true\n"),
    ]);
}

fn json_errors(t: &mut T) {
    cases(t, &[
        ("json.decode(\"{\\\"a\\\" 1}\")", "ERR 1:6 json.decode: Hier fehlt ein Doppelpunkt (Zeile 1, Spalte 6)"),
        ("json.decode(\"[1, 2\")", "ERR 1:6 json.decode: Hier fehlt ein Komma oder eine schließende Klammer (Zeile 1, Spalte 6)"),
        ("json.decode(\"{\\\"a\\\": 1, \\\"a\\\": 2}\")", "ERR 1:6 json.decode: Der Schlüssel kommt doppelt vor (Zeile 1, Spalte 13)"),
        ("json.decode(\"nul\")", "ERR 1:6 json.decode: Unbekanntes Wort (erlaubt sind true, false und null) (Zeile 1, Spalte 1)"),
        ("json.decode(\"1 2\")", "ERR 1:6 json.decode: Nach dem Wert folgt noch etwas (Zeile 1, Spalte 3)"),
        ("json.decode(\"\")", "ERR 1:6 json.decode: Hier fehlt ein Wert (Zeile 1, Spalte 1)"),
        ("json.decode(\"{\\n  \\\"ä\\\": tru\\n}\")", "ERR 1:6 json.decode: Unbekanntes Wort (erlaubt sind true, false und null) (Zeile 2, Spalte 8)"),
        ("json.decode(\"[01]\")", "ERR 1:6 json.decode: Hier fehlt ein Komma oder eine schließende Klammer (Zeile 1, Spalte 3)"),
        ("json.decode(\"\\\"a\\nb\\\"\")", "ERR 1:6 json.decode: Ein Steuerzeichen im Text muss als Escape-Sequenz stehen (Zeile 1, Spalte 3)"),
        ("json.decode(\"\\\"\\\\ud800\\\"\")", "ERR 1:6 json.decode: Ungültiges \\u-Zeichen (ein Surrogat ohne Partner) (Zeile 1, Spalte 8)"),
        ("json.decode(\"\\\"\\\\q\\\"\")", "ERR 1:6 json.decode: Ungültige Escape-Sequenz (Zeile 1, Spalte 3)"),
        ("json.decode(\"[1.]\")", "ERR 1:6 json.decode: Ungültige Zahl (Zeile 1, Spalte 4)"),
        ("json.decode(\"{1: 2}\")", "ERR 1:6 json.decode: Hier fehlt ein Schlüssel in Anführungszeichen (Zeile 1, Spalte 2)"),
        ("json.decode(\"\\\"abc\")", "ERR 1:6 json.decode: Der Text endet ohne schließendes \" (Zeile 1, Spalte 5)"),
        ("json.decode(bytes([0x22, 0xFF, 0x22]))", "ERR 1:6 json.decode: Der Text ist kein gültiges UTF-8 (Zeile 1, Spalte 2)"),
        ("json.decode(5)", "ERR 1:6 json.decode: text muss str oder bytes sein, nicht int"),
        ("json.encode(math.nan)", "ERR 1:6 json.encode: nan lässt sich nicht als JSON darstellen"),
        ("json.encode([1, -math.inf])", "ERR 1:6 json.encode: -inf lässt sich nicht als JSON darstellen"),
        ("json.encode({b: bytes(1)})", "ERR 1:6 json.encode: bytes lassen sich nicht als JSON darstellen – erst b.hex() oder b.text()"),
        ("json.encode([print])", "ERR 1:6 json.encode: Funktionen lassen sich nicht als JSON darstellen"),
        ("let a = []\na.push(a)\njson.encode(a)", "ERR 3:6 json.encode: Die Daten enthalten sich selbst"),
        ("let m = {}\nm.self = {up: m}\njson.encode(m)", "ERR 3:6 json.encode: Die Daten enthalten sich selbst"),
        ("json.encode({1: \"a\", \"1\": \"b\"})", "ERR 1:6 json.encode: Der Schlüssel \"1\" kommt doppelt vor"),
        ("json.encode(1, 9)", "ERR 1:6 json.encode: indent muss zwischen 0 und 8 liegen, nicht 9"),
        ("try { json.decode(\"[\") } catch e { print(\"gefangen:\", e) }", "gefangen: json.decode: Hier fehlt ein Wert (Zeile 1, Spalte 2)\n"),
    ]);
}

fn errors(t: &mut T) {
    cases(t, &[
        ("\"x\".split(1)", "ERR 1:5 str.split: sep muss str sein, nicht int"),
        ("\"x\".split(\",\", 0)", "ERR 1:5 str.split: n muss mindestens 1 sein, nicht 0"),
        ("\"x\".repeat(-1)", "ERR 1:5 str.repeat: n muss mindestens 0 sein, nicht -1"),
        ("\"x\".repeat(\"2\")", "ERR 1:5 str.repeat: n muss int sein, nicht str"),
        ("\"ab\".code(5)", "ERR 1:6 str.code: Index 5 liegt außerhalb des Textes (Länge 2)"),
        ("str.from_code(0xD800)", "ERR 1:5 str.from_code: 55296 ist kein gültiges Zeichen"),
        ("str.from_code(-1)", "ERR 1:5 str.from_code: -1 ist kein gültiges Zeichen"),
        ("\"a\".replace(\"a\", 1)", "ERR 1:5 str.replace: to muss str sein, nicht int"),
        ("\"a\".replace(\"a\", \"b\", -1)", "ERR 1:5 str.replace: n muss mindestens 0 sein, nicht -1"),
        ("\"a\".trim(5)", "ERR 1:5 str.trim: chars muss str sein, nicht int"),
        ("\"a\".slice(\"0\")", "ERR 1:5 str.slice: start muss int sein, nicht str"),
        ("\"a\".find(nil)", "ERR 1:5 str.find: sub muss str sein, nicht nil"),
        ("str.split(5, \",\")", "ERR 1:5 str.split: s muss str sein, nicht int"),
        ("(5).split(\",\")", "ERR 1:5 int hat keine Methode split"),
        ("[1].join(5)", "ERR 1:5 list.join: sep muss str sein, nicht int"),
        ("\"a\".nope()", "ERR 1:5 str hat keine Methode nope"),
        ("try { \"x\".repeat(-1) } catch e { print(\"gefangen:\", e) }\nprint(\"weiter\")", "gefangen: str.repeat: n muss mindestens 0 sein, nicht -1\nweiter\n"),
        ("fn f(l) { return l.sort(fn(a, b) { return a.x - b.x }) }\nf([{x: 1}, {y: 2}])", "ERR 1:47 Der Operator - passt nicht zu nil und int"),
    ]);
    // the stack trace of an error in a callback goes through the library call
    let (_, r, _, _) = exec("fn key(a, b) {\n  return a.k - b.k\n}\nlet l = [{k: 1}, {k: \"x\"}]\nl.sort(key)", Limits::default(), &mut ());
    match r {
        Err(e) => {
            let tr: Vec<(&str, u32)> = e.trace.iter().map(|x| (x.func.as_str(), x.line)).collect();
            t.eq("trace through sort", (e.msg.as_str(), tr), ("Der Operator - passt nicht zu str und int", vec![("key", 2), ("", 5)]));
        }
        Ok(_) => t.check("error in a sort callback", false),
    }
}

fn fuel(t: &mut T) {
    // costs that grow with the work: the fuel of a program minus that of the same program
    // without the work
    let base = fuel_of("let s = \"x\".repeat(0)");
    t.eq("repeat: 1 per 64 bytes", fuel_of("let s = \"x\".repeat(64000)") - base, 1000);
    // one part of 6400 bytes: the text 100, the part 1 + 100, the list 1; 3201 parts: the text
    // 100, a part 1 each, the list 1 + 3201 / 8
    let split0 = fuel_of("let s = \"x\".repeat(6400)\nlet p = s.split(\",\")");
    let split1 = fuel_of("let s = \"x,\".repeat(3200)\nlet p = s.split(\",\")");
    t.eq("split: 1 per part, 1 per 8 list values", split1 - split0, (100 + 3201 + 401) - 202);
    let sorting = |n: u32| {
        let src = |op: &str| format!("let l = []\nfor i in 0..{} {{ l.push((i * 7919) % 10007) }}\nlet s = l.slice()\ns.{}()", n, op);
        fuel_of(&src("sort")) - fuel_of(&src("len"))
    };
    let (s1, s2) = (sorting(1000), sorting(2000));
    t.check(&format!("sort: about n log2 n comparisons ({} and {})", s1, s2), (8000..9966).contains(&s1) && (17000..21932).contains(&s2));
    t.eq("fuel of sorting 1000 and 2000 numbers", (s1, s2), (9525, 21067));
    let len_cost = |n: u32| fuel_of(&format!("let s = \"x\".repeat({})\nlet n = s.len()", n)) - fuel_of(&format!("let s = \"x\".repeat({})\nlet n = 0", n));
    t.eq("len costs the call only", (len_cost(10), len_cost(100_000)), (1, 1));
    // encode: 2001 values / 8 + 9891 bytes / 64 + 1 for the str; decode: 9891 / 64, 1000 keys
    // and 1000 maps, the list 1 + 1000 / 8; and the two calls
    let rt = |a: &str, b: &str| fuel_of(&format!("let l = []\nfor i in 0..1000 {{ l.push({{a: i}}) }}\nlet t = {}\nlet b = {}", a, b));
    t.eq("fuel of a json round trip", rt("json.encode(l)", "json.decode(t)") - rt("l", "t"), (250 + 154 + 1) + (154 + 2000 + 126) + 2);
    // the fuel limit stops a big sort in the middle (a callback, the default order) and the list
    // stays as it was
    let src = "let l = []\nfor i in 0..3000 { l.push((i * 7919) % 3001) }\nlet first = [l[0], l[1], l[2]]\nl.sort(fn(a, b) { return a - b })";
    let prog = script::compile(src).ok();
    if let Some(prog) = prog {
        let mut vm = Vm::new(&prog, Limits { fuel: 100_000, ..Limits::default() });
        let st = vm.run(&mut (), u64::MAX);
        let l = vm.global("l").unwrap_or(Value::Nil);
        let head: Vec<Value> = vm.list(l).map(|x| x[..3].to_vec()).unwrap_or_default();
        let ok = matches!(&st, Status::Error(e) if e.kind == ErrorKind::Fuel);
        t.check(&format!("sort with a callback stopped by the fuel limit: {:?}", st), ok);
        t.eq("list unchanged after the stop", head, vec![Value::Int(0), Value::Int(7919 % 3001), Value::Int(2 * 7919 % 3001)]);
        t.check("fuel used up to the limit", vm.call_fuel() >= 100_000 && vm.call_fuel() < 101_100);
        t.check("vm idle and usable", !vm.busy() && vm.call_value(Value::Nil, &[]).is_err());
    }
    let Ok(prog) = script::compile("big.sort()") else { return t.check("sort program", false) };
    let mut vm = Vm::new(&prog, Limits { fuel: 500_000, heap: 64 << 20, ..Limits::default() });
    let items: Vec<Value> = (0..200_000u64).map(|i| Value::Int(((i * 2_654_435_761) % 1_000_003) as i64)).collect();
    let big = vm.new_list(items).unwrap_or(Value::Nil);
    vm.set_global("big", big);
    let st = vm.run(&mut (), u64::MAX);
    t.check("default sort stopped in the middle", matches!(&st, Status::Error(e) if e.kind == ErrorKind::Fuel) && vm.call_fuel() < 501_100);
    t.eq("big list unchanged", vm.list(big).map(|l| (l.len(), l[1], l[199_999])), Some((200_000, Value::Int(2_654_435_761 % 1_000_003), Value::Int(((199_999u64 * 2_654_435_761) % 1_000_003) as i64))));
    // a stop before the work: repeat, json.decode of a long text, split
    let lim = Limits { fuel: 50_000, heap: 1 << 30, ..Limits::default() };
    t.eq("repeat stops before it builds", kind("let s = \"x\".repeat(10000000)", lim), Some(ErrorKind::Fuel));
    t.eq("json.decode pays the text first", kind("let s = \"[\" + \"0,\".repeat(1000000) + \"0]\"\nlet v = json.decode(s)", Limits { fuel: 40_000, heap: 1 << 30, ..Limits::default() }), Some(ErrorKind::Fuel));
    t.eq("split pays per part", kind("let s = \",\".repeat(100000)\nlet p = s.split(\",\")", lim), Some(ErrorKind::Fuel));
    t.eq("json.encode counts values", kind("let a = [1]\nfor i in 0..30 { a = [a, a, a] }\nlet t = json.encode(a)", Limits { fuel: 100_000, heap: 1 << 30, ..Limits::default() }), Some(ErrorKind::Fuel));
    t.eq("fuel stops are not catchable", kind("try { let s = \"x\".repeat(10000000) } catch e { print(e) }", lim), Some(ErrorKind::Fuel));
}

fn heap(t: &mut T) {
    let lim = Limits { heap: 1 << 20, ..Limits::default() };
    t.eq("repeat beyond the heap", kind("let s = \"x\".repeat(2000000)", lim), Some(ErrorKind::Heap));
    t.eq("repeat far beyond the heap", kind("let s = \"x\".repeat(1099511627776)", lim), Some(ErrorKind::Heap));
    t.eq("repeat overflowing", kind("let s = \"ab\".repeat(4611686018427387904)", lim), Some(ErrorKind::Heap));
    t.eq("split into too many parts", kind("let s = \"x\".repeat(400000)\nlet p = s.split(\"\")", lim), Some(ErrorKind::Heap));
    t.eq("replace growing too big", kind("let s = \"x\".repeat(100000)\nlet p = s.replace(\"x\", \"yyyyyyyyyyyyyyyy\")", lim), Some(ErrorKind::Heap));
    t.eq("join too big", kind("let l = []\nfor i in 0..100 { l.push(\"x\".repeat(10000)) }\nlet s = l.join(\"-\".repeat(1000))\nlet t = l.join(\"\")\nlet u = [s, s, s, s, s, s, s, s].join()", lim), Some(ErrorKind::Heap));
    t.eq("format too big", kind("let s = \"{0:10000}\".repeat(200).format(1)", lim), Some(ErrorKind::Heap));
    t.eq("map results too big", kind("let l = []\nfor i in 0..2000 { l.push(i) }\nlet r = l.map(fn(x) { return \"y\".repeat(1000) })", lim), Some(ErrorKind::Heap));
    t.eq("json.decode with too many values", kind("let s = \"[\" + \"0,\".repeat(5000) + \"0]\"\nlet v = json.decode(s)", Limits { heap: 64 << 10, ..Limits::default() }), Some(ErrorKind::Heap));
    t.eq("json.encode of a shared tree too big", kind("let a = [1]\nfor i in 0..30 { a = [a, a] }\nlet t = json.encode(a)", lim), Some(ErrorKind::Heap));
    t.eq("heap stops are not catchable", kind("try { let s = \"x\".repeat(2000000) } catch e { print(e) }", lim), Some(ErrorKind::Heap));
    t.eq("chars of a long text", kind("let s = \"ab\".repeat(100000)\nlet c = s.chars()", lim), Some(ErrorKind::Heap));
    t.eq("within the heap", kind("let s = \"x\".repeat(300000)\nlet p = s.split(\"x\", 3)\nlet u = s.upper()", lim), None);
}

/// the collector runs while library callbacks hold values
fn collector(t: &mut T) {
    let lim = Limits { heap: 64 << 10, ..Limits::default() };
    let src = "let l = []\nfor i in 0..300 { l.push(\"v${(i * 37) % 300}\") }\nl.sort(fn(a, b) { let junk = [a, b, \"${a}${b}\"]; if a < b { return -1 } if a > b { return 1 } return 0 })\nprint(l[0], l[1], l[299], len(l))";
    let (o, r, _, gcs) = exec(src, lim, &mut ());
    t.eq("sort under collector pressure", (o.as_str(), r.is_ok(), gcs >= 3), ("v0 v1 v99 300\n", true, true));
    let src = "let l = [\"b${1}\", \"a${1}\", \"c${1}\"]\nlet first = true\nl.sort(fn(x, y) {\n  if first {\n    first = false\n    while len(l) > 0 { l.pop() }\n    for i in 0..3000 { let junk = \"${i}\" }\n  }\n  return x < y\n})\nprint(l)";
    let (o, r, _, gcs) = exec(src, lim, &mut ());
    t.eq("sort keeps its items while the callback empties the list", (o.as_str(), r.is_ok(), gcs >= 1), ("[\"a1\", \"b1\", \"c1\"]\n", true, true));
    let src = "let l = []\nfor i in 0..50 { l.push(\"i${i}\") }\nlet r = l.map(fn(x) { while len(l) > 0 { l.pop() } let g = []; for i in 0..200 { g.push(\"${i}\") } return x + \"!\" })\nprint(len(r), r[0], r[49], len(l))";
    let (o, r, _, gcs) = exec(src, lim, &mut ());
    t.eq("map keeps its items while the callback empties the list", (o.as_str(), r.is_ok(), gcs >= 1), ("50 i0! i49! 0\n", true, true));
    let src = "let l = []\nfor i in 0..400 { l.push(\"k${i}\") }\nlet f = l.filter(fn(x) { let g = [x, x + x]; return len(x) == 3 })\nlet m = {}\nfor x in f { m[x] = [x] }\nlet t = json.encode(m)\nlet back = json.decode(t)\nprint(len(f), f[0], f[89], len(back), back.k99)";
    let (o, r, _, gcs) = exec(src, lim, &mut ());
    t.eq("filter and json under collector pressure", (o.as_str(), r.is_ok(), gcs >= 1), ("90 k10 k99 90 [\"k99\"]\n", true, true));
}

const YIELD_SRC: &str = r#"let words = "der schnelle braune fuchs springt über den faulen hund".split()
let sorted = words.slice().sort(fn(a, b) { return len(a) - len(b) })
let lens = sorted.map(fn(w) { return w.len() })
let total = lens.reduce(fn(a, b) { return a + b }, 0)
let m = {}
for w in words { m[w.slice(0, 1)] = (m.get(w.slice(0, 1)) ?? 0) + 1 }
let t = json.encode({sorted: sorted, total: total, m: m})
let back = json.decode(t)
print(sorted.join(" "), total, "{:08.3}".format(math.sqrt(total)), back.m, t.len(), math.sin(total), math.pow(total, 1.5))
"#;

/// run src in runs of `slice` fuel: (output, total fuel, collections)
fn sliced(prog: &script::Program, slice: u64) -> (String, u64, u32) {
    let mut vm = Vm::new(prog, Limits { heap: 16 << 10, ..Limits::default() });
    loop {
        match vm.run(&mut (), slice) {
            Status::Yielded => {}
            Status::Done(_) => return (vm.take_output(), vm.call_fuel(), vm.collections()),
            Status::Error(e) => return (format!("{}{}", vm.take_output(), e), vm.call_fuel(), vm.collections()),
        }
    }
}

fn yields(t: &mut T) {
    let Ok(prog) = script::compile(YIELD_SRC) else { return t.check("yield program compiles", false) };
    let reference = sliced(&prog, u64::MAX);
    t.check(&format!("library program runs: {}", reference.0), reference.0.starts_with("der den über hund fuchs braune faulen springt schnelle 46 0006.782"));
    for slice in [1, 2, 7, 50, 333] {
        t.eq(&format!("library calls in runs of {} fuel", slice), sliced(&prog, slice), reference.clone());
    }
}

/// natives that use the library through Cx
#[derive(Default)]
struct JsonHost {
    sent: Vec<String>,
}

impl Host for JsonHost {
    fn call(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        let arg = |i: usize| args.get(i).copied().unwrap_or(Value::Nil);
        match name {
            "send" => {
                let s = cx.json_encode(arg(0), 0)?;
                self.sent.push(s);
                Ok(Value::Nil)
            }
            "receive" => {
                let text = String::from(cx.str(arg(0)).unwrap_or(""));
                cx.json_decode(&text)
            }
            "work" => {
                for _ in 0..1000 {
                    cx.spend(100)?;
                }
                Ok(Value::Int(1))
            }
            "hold" => {
                // keeps the items, empties the list, calls back (allocating), reads them
                let items: Vec<Value> = cx.list(arg(0)).map(|l| l.to_vec()).unwrap_or_default();
                for v in &items {
                    cx.keep(*v);
                }
                cx.list_replace(arg(0), Vec::new())?;
                cx.call(self, arg(1), &[])?;
                let texts: Vec<String> = items.iter().map(|v| String::from(cx.str(*v).unwrap_or("?"))).collect();
                cx.new_str(&texts.join(","))
            }
            _ => Err(Fail::Unknown),
        }
    }
}

fn host_api(t: &mut T) {
    let src = "send({a: [1, 2.5, \"x\"], b: nil})\nlet r = receive(\"{\\\"k\\\": [true, {\\\"z\\\": 1}]}\")\nprint(r, r.k[1].z)\ntry { send(math.nan) } catch e { print(e) }\ntry { receive(\"{\") } catch e { print(e) }\nlet l = [\"a${1}\", \"b${2}\"]\nprint(hold(l, fn() { for i in 0..3000 { let junk = \"${i}\" } }), l)";
    let mut h = JsonHost::default();
    let (o, r, _, gcs) = exec(src, Limits { heap: 32 << 10, ..Limits::default() }, &mut h);
    t.eq("Cx json and keep", (o.as_str(), r.is_ok(), gcs >= 1),
        ("{k: [true, {z: 1}]} 1\njson.encode: nan lässt sich nicht als JSON darstellen\njson.decode: Hier fehlt ein Schlüssel in Anführungszeichen (Zeile 1, Spalte 2)\na1,b2 []\n", true, true));
    t.eq("sent", h.sent.clone(), vec![String::from("{\"a\":[1,2.5,\"x\"],\"b\":null}")]);
    t.eq("Cx::spend stops a long native", kind_with("work()", Limits { fuel: 50_000, ..Limits::default() }, &mut JsonHost::default()), Some(ErrorKind::Fuel));
    t.eq("Cx::spend within the limit", kind_with("work()", Limits::default(), &mut JsonHost::default()), None);
    // Vm::json_encode / json_decode for hosts, also between runs
    let Ok(prog) = script::compile("let cfg = {port: 80, names: [\"a\", \"ü\"]}") else { return t.check("cfg compiles", false) };
    let mut vm = Vm::new(&prog, Limits::default());
    let _ = vm.run(&mut (), u64::MAX);
    let cfg = vm.global("cfg").unwrap_or(Value::Nil);
    t.eq("Vm::json_encode", vm.json_encode(cfg, 0).ok(), Some(String::from("{\"port\":80,\"names\":[\"a\",\"ü\"]}")));
    t.eq("Vm::json_encode pretty", vm.json_encode(cfg, 1).ok(), Some(String::from("{\n \"port\": 80,\n \"names\": [\n  \"a\",\n  \"ü\"\n ]\n}")));
    let v = vm.json_decode("{\"a\": [1, 2], \"b\": \"x\"}").unwrap_or(Value::Nil);
    t.eq("Vm::json_decode", (vm.display(v), vm.map_get_str(v, "a").and_then(|l| vm.list(l).map(|x| x.len()))), (String::from("{a: [1, 2], b: \"x\"}"), Some(2)));
    let e = vm.json_decode("[1,").err();
    t.eq("Vm::json_decode error", e.map(|e| (e.kind, e.msg)), Some((ErrorKind::Uncaught, String::from("json.decode: Hier fehlt ein Wert (Zeile 1, Spalte 4)"))));
    let e = vm.json_encode(Value::Float(num::NAN), 0).err();
    t.eq("Vm::json_encode error", e.map(|e| e.kind), Some(ErrorKind::Uncaught));
}

fn kind_with(src: &str, lim: Limits, host: &mut dyn Host) -> Option<ErrorKind> {
    exec(src, lim, host).1.err().map(|e| e.kind)
}

/// every name of library() can be read as a value; check() knows the modules
fn names(t: &mut T) {
    let names = script::library();
    t.check("library names", names.len() > 80 && names.contains(&"str.split") && names.contains(&"math.pi") && names.contains(&"json.decode"));
    let mut src = String::new();
    for n in &names {
        src.push_str(&format!("print(\"{}\", type({}))\n", n, n));
    }
    let o = out(&src);
    let bad: Vec<&str> = o.lines().filter(|l| !(l.ends_with(" fn") || l.ends_with(" float") || l.ends_with(" int"))).collect();
    t.eq("every library name is a value", bad, Vec::<&str>::new());
    t.eq("lines", o.lines().count(), names.len());
    let d = script::check("let p = str.split(\"a,b\", \",\")\nprint(list.len(p), map.keys({}), math.pi, json.encode(p), bytes(\"x\"), int.parse(\"1\"), float.parse(\"1\"))");
    t.eq("check knows the library", d, Vec::new());
    let d = script::check("let list = [1]\nlet map = {}\nprint(list.len(), map)");
    t.eq("modules can be shadowed", d, Vec::new());
}

/// one result of `fn` for the math file mode
fn math_value(f: &str, x: f64, y: f64) -> Option<f64> {
    Some(match f {
        "exp" | "log" | "log2" | "log10" | "sin" | "cos" | "pow" | "round" | "sqrt" => apply(f, x, y),
        _ => return None,
    })
}

/// coretest --math FILE: a line "fn x_hex [y_hex]" in, the result bits (or "-") out
pub fn math_file(path: &str) -> i32 {
    let Some(text) = plat::read_file(path) else {
        plat::write_stderr(b"coretest: cannot read the math file\n");
        return 2;
    };
    let text = String::from_utf8_lossy(&text);
    let mut out = String::new();
    for line in text.lines() {
        let mut f = line.split(' ');
        let name = f.next().unwrap_or("");
        let mut arg = || f.next().and_then(|h| u64::from_str_radix(h, 16).ok()).map_or(0.0, f64::from_bits);
        let (x, y) = (arg(), arg());
        match math_value(name, x, y) {
            Some(r) => out.push_str(&format!("{:016x}\n", r.to_bits())),
            None => out.push_str("-\n"),
        }
    }
    plat::write_stdout(out.as_bytes());
    0
}
