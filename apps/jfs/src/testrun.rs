// testrun -- `jfs --test FILE ...`: the test functions of jfs files
//
// Every top-level function whose name starts with test_ is a test, in the order of the file.
// Each runs in a fresh VM (host::run_test): the file's top-level code first, so every test has
// fresh globals and modules, then the function, then its timers and disk callbacks on the
// virtual clock (up to --until, 60 s without it).  An exception nobody catches fails the test:
// assert and assert_eq throw one with both values.  The report: "ok      NAME (FILE)" or
// "FEHLER  NAME (FILE:LINE:COL): message" with what the test printed and logged ("        | "
// in front), then "N Tests: P bestanden, F fehlgeschlagen".  Exit code 1 when a test failed,
// a file does not load or compile, or there was no test at all; else 0.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::{excerpt, host, load, out, Opts, Source};

/// see the module comment
pub fn run(files: &[String], o: &Opts) -> i32 {
    let (mut passed, mut failed) = (0u32, 0u32);
    for f in files {
        let (name, src) = match load(&Source::File(f.clone())) {
            Ok(x) => x,
            Err(m) => {
                out(&format!("FEHLER  {}\n", m));
                failed += 1;
                continue;
            }
        };
        let prog = match script::compile_with(&name, &src, script::GLOBALS) {
            Ok(p) => p,
            Err(diags) => {
                let mut s = format!("FEHLER  {} lässt sich nicht übersetzen\n", name);
                for d in &diags {
                    s.push_str(&format!("{}:{}\n", name, d));
                    excerpt(&mut s, &src, d.line, d.col, d.end_col);
                }
                out(&s);
                failed += 1;
                continue;
            }
        };
        let tests: Vec<String> = prog.globals().into_iter().filter(|g| g.starts_with("test_")).map(String::from).collect();
        let mut found = 0;
        for t in &tests {
            match host::run_test(&prog, o, &name, t) {
                Ok(false) => {}
                Ok(true) => {
                    found += 1;
                    passed += 1;
                    out(&format!("ok      {} ({})\n", t, name));
                }
                Err(x) => {
                    found += 1;
                    failed += 1;
                    let mut s = format!("FEHLER  {} ({}): {}\n", t, x.at, x.msg);
                    for l in x.output.lines() {
                        s.push_str("        | ");
                        s.push_str(l);
                        s.push('\n');
                    }
                    out(&s);
                }
            }
        }
        if found == 0 {
            out(&format!("{}: keine Tests (Funktionen test_…)\n", name));
        }
    }
    let n = passed + failed;
    out(&format!("{} {}: {} bestanden, {} fehlgeschlagen\n", n, if n == 1 { "Test" } else { "Tests" }, passed, failed));
    if failed > 0 || passed == 0 { 1 } else { 0 }
}
