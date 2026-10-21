// scaffoldtest-cli -- the command line program of scaffoldtest
//
// The lines, words and characters of files, counted by the same crate as the window
// (crates/scaffoldtest_core); a program of apps/scaffoldtest/BINS.
//   scaffoldtest-cli FILE ...   one line per file, then the sum (exit code 1 when a file
//                               cannot be read)
//   scaffoldtest-cli --version | --help
// Build and start it:
//   python build_rs.py run scaffoldtest --bin scaffoldtest-cli -- apps/scaffoldtest/README.md
//
// The files named on the command line are the user's: `fs.user` in apps/scaffoldtest/PERMISSIONS
// lets the program read them and nothing else (docs/PERMISSIONS.md).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use scaffoldtest_core::Counts;

/// the app's own version: version= of apps/scaffoldtest/PACKAGE (the library's is plat::VERSION)
const APP_VERSION: &str = rt::app_version!();

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

fn run() -> i32 {
    let args: Vec<String> = plat::args().into_iter().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        out(&format!("scaffoldtest-cli {} (JFLib {})\n", APP_VERSION, plat::VERSION));
        return 0;
    }
    let files: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    if files.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        out("Aufruf: scaffoldtest-cli DATEI ... | --version | --help\n");
        return if files.is_empty() { 2 } else { 0 };
    }
    let (mut sum, mut failed) = (Counts::default(), 0);
    for f in &files {
        let Some(bytes) = plat::read_file(f) else {
            plat::write_stderr(format!("scaffoldtest-cli: {} kann nicht gelesen werden\n", f).as_bytes());
            failed += 1;
            continue;
        };
        let c = scaffoldtest_core::count(&String::from_utf8_lossy(&bytes));
        out(&format!("{}: {}\n", f, scaffoldtest_core::summary(&c)));
        sum.add(&c);
    }
    if files.len() > 1 {
        out(&format!("zusammen: {}\n", scaffoldtest_core::summary(&sum)));
    }
    if failed == 0 { 0 } else { 1 }
}

rt::main!(run());
