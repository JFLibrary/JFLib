// pdftest -- self-checks of lib/pdf, the vector PDF writer: every file it writes is read back by
// reader.rs, a small PDF reader of its own that shares no code with the writer (header, xref
// offsets, objects complete, references resolved, stream lengths, FlateDecode), and its pages,
// fonts, images, links and outlines are checked against what was drawn.
// Console program; exit code 1 when a check fails.
//
//   python build_rs.py app pdftest
//   build/win/pdftest.exe [--only NAME]      the checks (build_rs.py test runs them through
//                                            tests/checks.uis; --script is ignored)
//   build/win/pdftest.exe --sample FILE      the sample document with every feature -> FILE
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

mod checks;
mod reader;
mod sample;

/// the result collector of a check group
pub struct T {
    pub group: &'static str,
    pub ok: u32,
    pub failed: u32,
}

impl T {
    pub fn check(&mut self, name: &str, cond: bool) {
        if cond {
            self.ok += 1;
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {}: {}\n", self.group, name).as_bytes());
        }
    }
    pub fn eq<A: PartialEq + core::fmt::Debug>(&mut self, name: &str, got: A, want: A) {
        if got == want {
            self.ok += 1;
        } else {
            self.failed += 1;
            plat::write_stdout(format!("FAIL {}: {}: got {:?}, want {:?}\n", self.group, name, got, want).as_bytes());
        }
    }
    /// a reader error is a failed check with its text
    pub fn ok<V>(&mut self, name: &str, r: Result<V, String>) -> Option<V> {
        match r {
            Ok(v) => {
                self.ok += 1;
                Some(v)
            }
            Err(e) => {
                self.failed += 1;
                plat::write_stdout(format!("FAIL {}: {}: {}\n", self.group, name, e).as_bytes());
                None
            }
        }
    }
}

fn run() -> i32 {
    if let Some(path) = plat::arg_value("--sample") {
        let bytes = match sample::build() {
            Ok(b) => b,
            Err(e) => {
                plat::write_stdout(format!("pdftest: the sample failed: {e:?}\n").as_bytes());
                return 1;
            }
        };
        if !plat::write_file(&path, &bytes) {
            plat::write_stdout(format!("pdftest: cannot write {path}\n").as_bytes());
            return 1;
        }
        plat::write_stdout(format!("pdftest: {} bytes -> {path}\n", bytes.len()).as_bytes());
        return 0;
    }
    let only = plat::arg_value("--only").map(String::from);
    let mut failed = 0;
    let mut total = 0;
    for (group, f) in checks::GROUPS {
        if only.as_ref().is_some_and(|o| !group.starts_with(o.as_str())) {
            continue;
        }
        let mut t = T { group, ok: 0, failed: 0 };
        f(&mut t);
        plat::write_stdout(format!("{group}: {} of {} passed\n", t.ok, t.ok + t.failed).as_bytes());
        failed += t.failed;
        total += t.ok + t.failed;
    }
    plat::write_stdout(format!("pdftest: {} of {} checks passed\n", total - failed, total).as_bytes());
    if failed == 0 && total > 0 { 0 } else { 1 }
}

rt::main!(run());
