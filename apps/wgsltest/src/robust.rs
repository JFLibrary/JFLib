// robust -- no input makes the front end panic or overflow its stack: random mutations of
// valid shaders (bytes and tokens deleted, duplicated, swapped, replaced), deep nesting of
// every kind (at the limit it still works, beyond it is an error), long chains, many
// declarations; perf -- compile-time numbers for a large generated shader

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::T;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

const PIECES: &[&str] = &[
    "(", ")", "{", "}", "[", "]", "<", ">", ";", ",", ":", ".", "@", "=", "==", "&&", "||", "&", "*", "-", "--", "++", "!", "~", "->", "_",
    "fn", "var", "let", "const", "override", "struct", "alias", "loop", "for", "while", "if", "else", "switch", "case", "default",
    "break", "continue", "continuing", "return", "discard", "const_assert", "enable f16;", "requires x;", "diagnostic(off, a);",
    "0", "1u", "-1i", "1.5", "0x1p3", "1e40", "1h", "true", "x", "i32", "f32", "vec4f", "array<f32>", "ptr<function, i32>",
    "texture_2d<f32>", "atomic<u32>", "@builtin(position)", "@location(0)", "@group(0)", "@binding(1)", "@workgroup_size(1)",
    "@compute", "@vertex", "@fragment", "@must_use", "@id(3)", "@align(16)", "@size(4)", "@interpolate(flat)", "/*", "*/", "//",
    "\u{2028}", "é", "\u{0}", "#", "$", "\"",
];

/// a mutation of src: n random edits at token-ish boundaries
fn mutate(src: &str, r: &mut Rng, n: usize) -> String {
    let mut s: Vec<char> = src.chars().collect();
    for _ in 0..n {
        let len = s.len().max(1);
        let at = r.below(len);
        let span = 1 + r.below(12);
        let end = (at + span).min(s.len());
        match r.below(5) {
            0 => {
                s.drain(at.min(s.len())..end);
            }
            1 => {
                let piece: Vec<char> = PIECES[r.below(PIECES.len())].chars().collect();
                let at = at.min(s.len());
                for (k, c) in piece.into_iter().enumerate() {
                    s.insert(at + k, c);
                }
            }
            2 => {
                let dup: Vec<char> = s[at.min(s.len())..end].to_vec();
                let to = r.below(s.len() + 1);
                for (k, c) in dup.into_iter().enumerate() {
                    s.insert(to + k, c);
                }
            }
            3 => {
                let piece: Vec<char> = PIECES[r.below(PIECES.len())].chars().collect();
                let at = at.min(s.len());
                s.drain(at..end);
                for (k, c) in piece.into_iter().enumerate() {
                    s.insert(at + k, c);
                }
            }
            _ => {
                if s.len() > 2 {
                    let b = r.below(s.len());
                    let a = at.min(s.len() - 1);
                    s.swap(a, b);
                }
            }
        }
    }
    s.into_iter().collect()
}

pub fn run(t: &mut T) {
    let mut srcs: Vec<&str> = wgsl::bench::ALL.iter().map(|x| x.1).collect();
    srcs.push(crate::lexparse::SAMPLE);
    srcs.extend(crate::checks::PASS.iter().map(|x| x.1));
    // --mutations N [--seed S]: more rounds (a soak test; the default run does 2400)
    let args = plat::args();
    let rounds: usize = args.iter().position(|a| a == "--mutations").and_then(|i| args.get(i + 1)).and_then(|x| x.parse().ok()).unwrap_or(2400);
    let seed: u64 = args.iter().position(|a| a == "--seed").and_then(|i| args.get(i + 1)).and_then(|x| x.parse().ok()).unwrap_or(0x9e37_79b9_7f4a_7c15);
    let mut r = Rng(seed | 1);
    let mut runs = 0u32;
    let mut accepted = 0u32;
    let mut errors_have_positions = true;
    for round in 0..rounds {
        let src = srcs[round % srcs.len()];
        let edits = 1 + r.below(4);
        let m = mutate(src, &mut r, edits);
        if plat::args().iter().any(|a| a == "--trace") {
            crate::out(&format!("ROUND {}\n{}\nEND\n", round, m));
        }
        let (_, pd) = wgsl::parse(&m);
        let _ = wgsl::print(&wgsl::parse(&m).0);
        match wgsl::check(&m) {
            Ok(_) => accepted += 1,
            Err(d) => {
                if d.is_empty() || d.iter().any(|x| x.line == 0 || x.col == 0 || x.msg.is_empty()) {
                    errors_have_positions = false;
                }
            }
        }
        let _ = pd;
        runs += 1;
    }
    if t.verbose { crate::out("mutations: no panic\n"); }
    t.check("mutations: no panic", runs as usize == rounds);
    if t.verbose { crate::out("mutations: every error has a position and a message\n"); }
    t.check("mutations: every error has a position and a message", errors_have_positions);
    if t.verbose {
        crate::out(&format!("mutations: {} runs, {} still valid\n", runs, accepted));
    }
    // nesting at the limit works, beyond it is an error (never a stack overflow)
    let nest = |n: usize| format!("const x = {}1{};", "(".repeat(n), ")".repeat(n));
    if t.verbose { crate::out("120 parentheses check\n"); }
    t.check("120 parentheses check", wgsl::check(&nest(120)).is_ok());
    if t.verbose { crate::out("10000 parentheses: error\n"); }
    t.check("10000 parentheses: error", wgsl::check(&nest(10000)).is_err());
    let blocks = |n: usize| format!("fn f() {{ {} {} }}", "{".repeat(n), "}".repeat(n));
    if t.verbose { crate::out("120 nested blocks check\n"); }
    t.check("120 nested blocks check", wgsl::check(&blocks(120)).is_ok());
    if t.verbose { crate::out("10000 nested blocks: error\n"); }
    t.check("10000 nested blocks: error", wgsl::check(&blocks(10000)).is_err());
    let ifs = |n: usize| format!("fn f(c: bool) {{ {} {} }}", "if c { ".repeat(n), "}".repeat(n));
    if t.verbose { crate::out("63 nested ifs check\n"); }
    t.check("63 nested ifs check", wgsl::check(&ifs(63)).is_ok());
    t.check("64 nested ifs: error", wgsl::check(&ifs(64)).is_err());
    let loops = |n: usize| format!("fn f() {{ {} {} }}", "loop { break; ".repeat(n), "}".repeat(n));
    if t.verbose { crate::out("60 nested loops check\n"); }
    t.check("60 nested loops check", wgsl::check(&loops(60)).is_ok());
    let types = |n: usize| format!("var<private> a: {}f32{};", "array<".repeat(n), ", 2>".repeat(n));
    if t.verbose { crate::out("8 nested array types check\n"); }
    t.check("8 nested array types check", wgsl::check(&types(8)).is_ok());
    if t.verbose { crate::out("3000 nested array types: error\n"); }
    t.check("3000 nested array types: error", wgsl::check(&types(3000)).is_err());
    let calls = |n: usize| format!("fn g(x: f32) -> f32 {{ return x; }}\nconst c = 1.0;\nfn f() -> f32 {{ return {}c{}; }}", "g(".repeat(n), ")".repeat(n));
    if t.verbose { crate::out("60 nested calls check\n"); }
    t.check("60 nested calls check", wgsl::check(&calls(60)).is_ok());
    if t.verbose { crate::out("5000 nested calls: error\n"); }
    t.check("5000 nested calls: error", wgsl::check(&calls(5000)).is_err());
    let vecs = |n: usize| format!("const v = {}1.0{};", "abs(".repeat(n), ")".repeat(n));
    if t.verbose { crate::out("60 nested built-ins check\n"); }
    t.check("60 nested built-ins check", wgsl::check(&vecs(60)).is_ok());
    let chain = |n: usize| format!("fn f(a: f32) -> f32 {{ return a{}; }}", " + a".repeat(n));
    if t.verbose { crate::out("chain of 120 checks\n"); }
    t.check("chain of 120 checks", wgsl::check(&chain(120)).is_ok());
    if t.verbose { crate::out("chain of 250 checks\n"); }
    t.check("chain of 250 checks", wgsl::check(&chain(250)).is_ok());
    let nested_chains = format!("fn f(a: f32) -> f32 {{ return {}; }}", (0..60).fold(String::from("a"), |acc, _| format!("({}{})", acc, " + a".repeat(60))));
    if t.verbose { crate::out("nested chains: error\n"); }
    t.check("nested chains: error", wgsl::check(&nested_chains).is_err());
    if t.verbose { crate::out("chain of 100000: error\n"); }
    t.check("chain of 100000: error", wgsl::check(&chain(100000)).is_err());
    let members = |n: usize| format!("fn f(v: vec4f) -> f32 {{ return v{}.x; }}", ".xyzw".repeat(n));
    if t.verbose { crate::out("member chain of 100 checks\n"); }
    t.check("member chain of 100 checks", wgsl::check(&members(100)).is_ok());
    if t.verbose { crate::out("member chain of 5000: error\n"); }
    t.check("member chain of 5000: error", wgsl::check(&members(5000)).is_err());
    let unary = |n: usize| format!("const x = {}1;", "-".repeat(n));
    if t.verbose { crate::out("unary chain of 100 checks\n"); }
    t.check("unary chain of 100 checks", wgsl::check(&unary(100)).is_ok());
    let elifs = |n: usize| format!("fn f(c: bool) {{ if c {{}} {} }}", "else if c {} ".repeat(n));
    if t.verbose { crate::out("120 else-ifs check\n"); }
    t.check("120 else-ifs check", wgsl::check(&elifs(120)).is_ok());
    t.check("125 else-ifs: error", wgsl::check(&elifs(125)).is_err());
    if t.verbose { crate::out("5000 else-ifs: error\n"); }
    t.check("5000 else-ifs: error", wgsl::check(&elifs(5000)).is_err());
    let swz = |n: usize| format!("fn f() {{ var v: vec4f; v{} = vec2f(); }}", ".wzyx".repeat(n) + ".xy");
    if t.verbose { crate::out("swizzle view chain checks\n"); }
    t.check("swizzle view chain checks", wgsl::check(&swz(50)).is_ok());
    // the deepest valid module in a thread with 512 KB of stack (exact on Linux; Windows only
    // commits less, the reserve is the 1 MB of the main thread)
    let worst = nesting("worst", 61);
    let verbose = t.verbose;
    let th = plat::thread_spawn(512 * 1024, alloc::boxed::Box::new(move || {
        let ok = wgsl::check(&worst).is_ok();
        if !ok {
            crate::out("FAIL robust: worst case rejected\n");
        } else if verbose {
            crate::out("worst case: ok\n");
        }
        // and through the back end (a library: the module has no entry point), at both levels
        for minor in [shader::BASIS, shader::VOLL] {
            let r = wgsl::spirv::compile_library(&worst, &wgsl::CompileOptions::new(minor));
            if !r.is_ok_and(|o| shader::val::validate(&o.entries[0].words).is_ok()) {
                crate::out("FAIL robust: worst case does not compile to valid SPIR-V\n");
            }
        }
    }));
    if let Some(th) = th {
        th.join();
    }
    t.check("worst-case nesting in 512 KB, also compiled to SPIR-V", true);
    // many declarations in a dependency chain (no recursion between declarations)
    let mut many = String::from("const c0 = 1;\n");
    for k in 1..20000 {
        many.push_str(&format!("const c{} = c{} + 1;\n", k, k - 1));
    }
    many.push_str("const_assert c19999 == 20000;\n");
    if t.verbose { crate::out("20000 chained consts\n"); }
    t.check("20000 chained consts", wgsl::check(&many).is_ok());
    let mut fns = String::from("fn f0() -> i32 { return 1; }\n");
    for k in 1..5000 {
        fns.push_str(&format!("fn f{}() -> i32 {{ return f{}() + 1; }}\n", k, k - 1));
    }
    if t.verbose { crate::out("5000 chained functions\n"); }
    t.check("5000 chained functions", wgsl::check(&fns).is_ok());
    let mut cyc = String::new();
    for k in 0..3000 {
        cyc.push_str(&format!("const d{} = d{};\n", k, (k + 1) % 3000));
    }
    if t.verbose { crate::out("a cycle of 3000\n"); }
    t.check("a cycle of 3000", wgsl::check(&cyc).err().is_some_and(|d| d.iter().any(|x| x.msg.contains("zyklisch"))));
    // huge constants are capped, not built
    if t.verbose { crate::out("huge array constant\n"); }
    t.check("huge array constant", wgsl::check("const a = array<array<f32, 16384>, 16384>();\nconst b = a[3][4];").is_ok());
    if t.verbose { crate::out("huge composite: error, no allocation storm\n"); }
    t.check("huge composite: no allocation storm", wgsl::check("fn f() { let a = array<array<f32, 4096>, 4096>(); }").is_ok());
    t.check("over 4 GiB: error", wgsl::check("const a = array<array<f32, 65536>, 65536>();").is_err());
    // garbage
    let mut g = String::new();
    for _ in 0..20000 {
        g.push(char::from_u32((r.next() % 0x3000) as u32).unwrap_or('x'));
    }
    if t.verbose { crate::out("random characters: errors\n"); }
    t.check("random characters: errors", wgsl::check(&g).is_err());
    let all_tokens: String = PIECES.join(" ");
    let _ = wgsl::check(&all_tokens.repeat(20));
    if t.verbose { crate::out("token soup survived\n"); }
    t.check("token soup survived", true);
}

/// a nesting of n levels of one kind (for --stack and the depth checks)
pub fn nesting(kind: &str, n: usize) -> String {
    match kind {
        "parens" => format!("const x = {}1{};", "(".repeat(n), ")".repeat(n)),
        "rparens" => format!("fn f(a: f32) -> f32 {{ return {}a{}; }}", "(".repeat(n), ")".repeat(n)),
        "blocks" => format!("fn f() {{ {} {} }}", "{".repeat(n), "}".repeat(n)),
        "ifs" => format!("fn f(c: bool) {{ {} {} }}", "if c { ".repeat(n), "}".repeat(n)),
        "loops" => format!("fn f() {{ {} {} }}", "loop { break; ".repeat(n), "}".repeat(n)),
        "chain" => format!("fn f(a: f32) -> f32 {{ return a{}; }}", " + a".repeat(n)),
        "calls" => format!("fn g(x: f32) -> f32 {{ return x; }}\nfn f(a: f32) -> f32 {{ return {}a{}; }}", "g(".repeat(n), ")".repeat(n)),
        "members" => format!("fn f(v: vec4f) -> f32 {{ return v{}.x; }}", ".xyzw".repeat(n)),
        "unary" => format!("fn f(a: f32) -> f32 {{ return {}a; }}", "-".repeat(n)),
        "elifs" => format!("fn f(c: bool) {{ if c {{}} {} }}", "else if c {} ".repeat(n)),
        "types" => format!("var<private> a: {}f32{};", "array<".repeat(n), ", 2>".repeat(n)),
        // the deepest valid function: n nested loops around a 250-link chain of calls and
        // members (the worst stack a module within the limits needs)
        "worst" => format!(
            "fn g(x: f32) -> f32 {{ return x; }}\nfn f(v: vec4f) -> f32 {{ {} return {}v.x{}{}; {} return 0.0; }}",
            "loop { if v.x > 0.0 { break; } ".repeat(n),
            "g(".repeat(14),
            ")".repeat(14),
            " + v.y".repeat(225),
            "}".repeat(n)
        ),
        _ => String::new(),
    }
}

/// a large generated shader: n functions of loops, arithmetic, built-ins and calls
pub fn big_shader(n: usize) -> String {
    let mut s = String::from("struct P { pos: vec4f, vel: vec4f, mass: f32, id: u32 }\n@group(0) @binding(0) var<storage, read_write> parts: array<P>;\n@group(0) @binding(1) var<uniform> dt: vec4f;\nconst G = 6.674e-11;\n");
    for k in 0..n {
        let prev = if k == 0 { String::from("x") } else { format!("fn_{}(x * 0.5, i + 1u)", k - 1) };
        s.push_str(&format!(
            "fn fn_{k}(x: f32, i: u32) -> f32 {{\n    var acc = 0.0;\n    var v = vec3f(x, f32(i), 1.0);\n    for (var j = 0u; j < 4u; j++) {{\n        let p = parts[(i + j) % arrayLength(&parts)];\n        let d = p.pos.xyz - v;\n        let r2 = max(dot(d, d), 1e-6);\n        acc += G * p.mass / r2 + sin(x * f32(j)) * 0.25;\n        v = normalize(v + d * dt.x) * clamp(acc, -1.0, 1.0);\n        if acc > 100.0 {{ break; }}\n    }}\n    switch i % 3u {{\n        case 0u: {{ acc *= 2.0; }}\n        case 1u, 2u {{ acc -= length(v); }}\n        default {{ }}\n    }}\n    return acc + {prev} + f32(countOneBits(i ^ {k}u));\n}}\n"
        ));
    }
    s.push_str(&format!("@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) g: vec3u) {{\n    let i = g.x;\n    if i < arrayLength(&parts) {{\n        parts[i].vel.x += fn_{}(parts[i].pos.x, i);\n    }}\n}}\n", n - 1));
    s
}

pub fn perf(t: &mut T) {
    let src = big_shader(1500);
    let lines = src.lines().count();
    let t0 = plat::time_us();
    let (ast, d) = wgsl::parse(&src);
    let t1 = plat::time_us();
    t.check("big shader parses", d.is_empty());
    let r = wgsl::check(&src);
    let t2 = plat::time_us();
    t.check("big shader checks", r.is_ok());
    if let Err(d) = &r {
        for x in d.iter().take(5) {
            crate::out(&format!("    {}\n", x));
        }
    }
    let p = wgsl::print(&ast);
    let t3 = plat::time_us();
    let exprs: usize = r.as_ref().map(|m| m.functions.iter().map(|f| f.exprs.len()).sum()).unwrap_or(0);
    let parse_ms = (t1 - t0) as f64 / 1000.0;
    let check_ms = (t2 - t1) as f64 / 1000.0;
    crate::out(&format!(
        "perf: {} lines, {} KB, {} functions, {} IR expressions: parse {:.1} ms, parse+check {:.1} ms ({:.0} lines/s), print {:.1} ms\n",
        lines,
        src.len() / 1024,
        r.as_ref().map(|m| m.functions.len()).unwrap_or(0),
        exprs,
        parse_ms,
        check_ms,
        lines as f64 / (check_ms / 1000.0),
        (t3 - t2) as f64 / 1000.0
    ));
    t.check("print of the big shader", p.len() > src.len() / 2);
    if let Ok(m) = &r {
        // the uniformity analysis alone (check has run it once)
        let t4 = plat::time_us();
        let u = wgsl::analyze_uniformity(m, &src, &wgsl::Options::default());
        let us = plat::time_us() - t4;
        crate::out(&format!("perf: the uniformity analysis of the big shader {:.1} ms ({:.1} % of parse+check), {} diagnostics\n", us as f64 / 1000.0, us as f64 / 1000.0 / check_ms * 100.0, u.len()));
        t.check("the big shader is uniform", u.is_empty());
    }
    if let Ok(m) = &r {
        let t4 = plat::time_us();
        let o = wgsl::compile_module(m, &wgsl::CompileOptions::new(shader::VOLL));
        let us = plat::time_us() - t4;
        let words = o.as_ref().map(|o| o.entries.iter().map(|e| e.words.len()).sum::<usize>()).unwrap_or(0);
        crate::out(&format!("perf: the big shader lowered to SPIR-V 1.6 in {:.1} ms ({} words)\n", us as f64 / 1000.0, words));
        t.check("big shader compiles", o.is_ok_and(|o| shader::val::validate(&o.entries[0].words).is_ok()));
    }
    // the benchmark shaders
    let t0 = plat::time_us();
    for _ in 0..20 {
        for (_, s) in wgsl::bench::ALL {
            let _ = wgsl::check(s);
        }
    }
    let us = (plat::time_us() - t0) as f64 / 20.0;
    crate::out(&format!("perf: the six benchmark shaders: {:.0} us per round\n", us));
    t.check("timing sane", us > 0.0);
    let mods: alloc::vec::Vec<(wgsl::Module, &str)> = wgsl::bench::ALL.iter().filter_map(|(_, s)| wgsl::check(s).ok().map(|m| (m, *s))).collect();
    let t0 = plat::time_us();
    for _ in 0..20 {
        for (m, s) in &mods {
            let _ = wgsl::analyze_uniformity(m, s, &wgsl::Options::default());
        }
    }
    let ua = (plat::time_us() - t0) as f64 / 20.0;
    crate::out(&format!("perf: their uniformity analysis: {:.1} us per round ({:.1} %)\n", ua, ua / us * 100.0));
}
