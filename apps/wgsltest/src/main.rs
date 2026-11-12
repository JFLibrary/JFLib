// wgsltest -- self-checks of lib/wgsl (the WGSL front end, docs/WGSL.md): the lexer (tokens,
// literals, template lists), the parser (statements, precedence, error recovery, depth
// limit), print -> parse round trips, positive and negative checker cases for every rule
// (each negative case asserts the message's position and a key part of its text), constant
// evaluation against i64 / f64 computations and f32 rounding, the uniformity analysis and the
// subgroup front end (unif.rs), the IR of a few shaders (types,
// layouts, entry point interfaces), the six benchmark shaders, robustness (mutations of valid
// shaders, deep nesting: errors, never a panic) and compile-time numbers for a large shader.
// Console program; exit code 1 when a check fails.
//
//   python build_rs.py test wgsltest
//   build/win/wgsltest.exe [--check] [--only GROUP] [--verbose]
//   build/win/wgsltest.exe --file F.wgsl     check one file and print the diagnostics
//   build/win/wgsltest.exe --verdict F.wgsl  the corpus group's verdict on one file (valid, error,
//                                            pipeline-error) and why
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

mod backend;
mod backend_api;
mod bench;
mod checks;
mod consts;
mod coverage;
mod irtest;
mod lexparse;
mod robust;
mod unif;

/// the result collector of a check group
pub struct T {
    pub group: &'static str,
    pub ok: u32,
    pub failed: u32,
    pub verbose: bool,
}

impl T {
    pub fn check(&mut self, name: &str, cond: bool) {
        if cond {
            self.ok += 1;
        } else {
            self.failed += 1;
            out(&format!("FAIL {}: {}\n", self.group, name));
        }
    }
    pub fn eq<A: PartialEq + core::fmt::Debug>(&mut self, name: &str, got: A, want: A) {
        if got == want {
            self.ok += 1;
        } else {
            self.failed += 1;
            out(&format!("FAIL {}: {}: got {:?}, want {:?}\n", self.group, name, got, want));
        }
    }
    /// the source must pass the checker
    pub fn pass(&mut self, name: &str, src: &str) -> Option<wgsl::Module> {
        match wgsl::check(src) {
            Ok(m) => {
                self.ok += 1;
                Some(m)
            }
            Err(d) => {
                self.failed += 1;
                out(&format!("FAIL {}: {}: rejected\n", self.group, name));
                for x in d.iter().take(6) {
                    out(&format!("    {}\n", x));
                }
                None
            }
        }
    }
    /// the source must pass the checker with these options
    pub fn pass_with(&mut self, name: &str, src: &str, opt: &wgsl::Options) -> Option<wgsl::Module> {
        match wgsl::check_with(src, opt) {
            Ok(m) => {
                self.ok += 1;
                Some(m)
            }
            Err(d) => {
                self.failed += 1;
                out(&format!("FAIL {}: {}: rejected\n", self.group, name));
                for x in d.iter().take(6) {
                    out(&format!("    {}\n", x));
                }
                None
            }
        }
    }
    /// the source (with `$` at the expected error position) must fail with an error there
    /// whose message contains `part`
    pub fn fail(&mut self, name: &str, marked: &str, part: &str) {
        self.fail_with(name, marked, part, &wgsl::Options::default());
    }
    /// fail() with these options
    pub fn fail_with(&mut self, name: &str, marked: &str, part: &str, opt: &wgsl::Options) {
        let (src, line, col) = strip_marker(marked);
        let r = wgsl::check_with(&src, opt);
        let diags = match r {
            Ok(_) => {
                self.failed += 1;
                out(&format!("FAIL {}: {}: accepted, want an error at {}:{} containing {:?}\n", self.group, name, line, col, part));
                return;
            }
            Err(d) => d,
        };
        let hit = diags.iter().any(|d| d.error && d.line == line && d.col == col && d.msg.contains(part));
        if hit {
            self.ok += 1;
        } else {
            self.failed += 1;
            out(&format!("FAIL {}: {}: want {}:{} {:?}, got:\n", self.group, name, line, col, part));
            for x in diags.iter().take(6) {
                out(&format!("    {}\n", x));
            }
        }
    }
    /// like fail(), for syntax errors from wgsl::parse
    pub fn parse_fail(&mut self, name: &str, marked: &str, part: &str) {
        let (src, line, col) = strip_marker(marked);
        let (_, diags) = wgsl::parse(&src);
        let hit = diags.iter().any(|d| d.error && d.line == line && d.col == col && d.msg.contains(part));
        if hit {
            self.ok += 1;
        } else {
            self.failed += 1;
            out(&format!("FAIL {}: {}: want syntax error {}:{} {:?}, got:\n", self.group, name, line, col, part));
            for x in diags.iter().take(6) {
                out(&format!("    {}\n", x));
            }
        }
    }
}

/// the source without its `$` marker, and the 1-based line and column of the marker
pub fn strip_marker(marked: &str) -> (String, u32, u32) {
    let at = marked.find('$').unwrap_or(0);
    let mut src = String::from(&marked[..at]);
    src.push_str(&marked[(at + 1).min(marked.len())..]);
    let before = &marked[..at];
    let line = before.matches('\n').count() as u32 + 1;
    let col = before.rsplit('\n').next().map(|l| l.chars().count()).unwrap_or(0) as u32 + 1;
    (src, line, col)
}

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

fn check_file(path: &str) -> i32 {
    let Some(bytes) = plat::read_file(path) else {
        out(&format!("cannot read {}\n", path));
        return 2;
    };
    let src = String::from_utf8_lossy(&bytes);
    match wgsl::check(&src) {
        Ok(m) => {
            for w in &m.warnings {
                out(&format!("{}: {}", path, w.render(&src)));
            }
            out(&format!("{}: ok, {} functions, {} entry points\n", path, m.functions.len(), m.entry_points.len()));
            0
        }
        Err(d) => {
            for x in &d {
                out(&format!("{}: {}", path, x.render(&src)));
            }
            1
        }
    }
}

/// --stack KIND N KB: check a nesting of N in a thread with KB KB of stack (measures the
/// stack a nesting level costs; a crash ends the process)
fn stack_probe() -> i32 {
    let a = plat::args();
    let i = a.iter().position(|x| x == "--stack").unwrap_or(0);
    let kind = a.get(i + 1).cloned().unwrap_or_default();
    let n: usize = a.get(i + 2).and_then(|x| x.parse().ok()).unwrap_or(100);
    let kb: usize = a.get(i + 3).and_then(|x| x.parse().ok()).unwrap_or(1024);
    let src = robust::nesting(&kind, n);
    let th = plat::thread_spawn(kb * 1024, alloc::boxed::Box::new(move || {
        let r = wgsl::check(&src);
        let first = r.as_ref().err().and_then(|d| d.first()).map(|d| format!(" ({})", d)).unwrap_or_default();
        out(&format!("stack {} {} {} KB: {}{}\n", kind, n, kb, if r.is_ok() { "ok" } else { "error" }, first));
    }));
    if let Some(t) = th {
        t.join();
    }
    0
}

fn run() -> i32 {
    if let Some(f) = plat::arg_value("--file") {
        return check_file(&f);
    }
    if let Some(f) = plat::arg_value("--spv") {
        return backend::spv_file(&f);
    }
    if let Some(f) = plat::arg_value("--verdict") {
        let Some(bytes) = plat::read_file(&f) else {
            out(&format!("cannot read {}\n", f));
            return 2;
        };
        let (v, why) = backend::verdict(&String::from_utf8_lossy(&bytes));
        out(&format!("{} {}\n", v, why.lines().next().unwrap_or("")));
        return 0;
    }
    if plat::args().iter().any(|a| a == "--stack") {
        return stack_probe();
    }
    let only = plat::arg_value("--only").map(String::from);
    let verbose = plat::args().iter().any(|a| a == "--verbose");
    let groups: [(&'static str, fn(&mut T)); 15] = [
        ("lex", lexparse::lex),
        ("parse", lexparse::parse),
        ("print", lexparse::print),
        ("check", checks::run),
        ("unif", unif::run),
        ("consts", consts::run),
        ("ir", irtest::run),
        ("bench", bench::run),
        ("robust", robust::run),
        ("perf", robust::perf),
        ("spirv", backend::spirv),
        ("corpus", backend::corpus),
        ("backend", backend_api::run),
        ("perf_backend", backend_api::perf_backend),
        ("msl", backend::msl),
    ];
    let mut failed = 0;
    let mut total = 0;
    for (name, f) in groups {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let mut t = T { group: name, ok: 0, failed: 0, verbose };
        f(&mut t);
        out(&format!("{:<10} {} ok, {} failed\n", name, t.ok, t.failed));
        failed += t.failed;
        total += t.ok + t.failed;
    }
    out(&format!("wgsltest: {} of {} checks passed\n", total - failed, total));
    if failed == 0 { 0 } else { 1 }
}

rt::main!(run());
