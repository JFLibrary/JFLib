// jfs -- the standalone runner of the script language (lib/script, .jfs): runs a script on the
// console, like `python file.py`
//
//   jfs [OPTIONS] FILE.jfs [ARGS ...]   compile and run a script; ARGS are config.args
//   jfs [OPTIONS] -e CODE [ARGS ...]    a one-liner
//   jfs [OPTIONS] - [ARGS ...]          the script from standard input
//   jfs --check FILE.jfs | -e CODE | -  only the diagnostics: "FILE:LINE:COL: Fehler: ..." and
//                                       the warnings on stdout; exit code 1 when one is an error
//   jfs --selftest DIR                  the example cases DIR/*.test (selftest.rs; build_rs.py
//                                       test runs apps/jfs/tests/cases through tests/cases.uis)
//   options: --virtual-time, --until MS, --seed N, --disk FILE, -c / --config K=V or JSON,
//   --env NAME, --fuel N, --heap N, --depth N, --stack N (usage() has the details); every
//   option comes before the script, everything after it belongs to the script
//
// The script runs in the console host of host.rs: print -> stdout, log -> stderr, now, rand,
// config, timer (an event loop on the real clock or a virtual one), disk (in a JSON file or in
// memory); net, dns, http and metrics belong to JFNetworker and throw a catchable error.  The
// script gets no other access to files or the network.  Names are checked against the host's
// before the run (script::compile_with: a typing error is an error, not a warning).  An error
// of the compiler or an uncaught one of the run goes to stderr with file, line and column, the
// source line and the calls: exit code 1; a wrong command line: 2.
//
// Build and test (Windows and Linux):
//   python build_rs.py run jfs -- -e "print(1 + 2)"
//   python build_rs.py test jfs
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod host;
mod selftest;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use script::{Diagnostic, Limits};

/// the app's own version: version= of apps/jfs/PACKAGE (the library's is plat::VERSION)
const APP_VERSION: &str = rt::app_version!();

/// where the script comes from
pub enum Source {
    File(String),
    Stdin,
    Code(String),
}

/// what the command line asks for
pub struct Opts {
    pub check: bool,
    pub virtual_time: bool,
    /// --until in nanoseconds of the clock
    pub until: Option<u64>,
    pub seed: Option<u64>,
    pub disk: Option<String>,
    /// the --config values in their order
    pub config: Vec<String>,
    /// the --env names
    pub env: Vec<String>,
    pub limits: Limits,
    pub source: Source,
    /// the script's arguments (config.args)
    pub args: Vec<String>,
}

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

pub fn err(s: &str) {
    plat::write_stderr(s.as_bytes());
}

/// a wrong command line: the message and exit code 2
fn bad(msg: &str) -> i32 {
    err(&format!("jfs: {}\n(jfs --help zeigt die Optionen)\n", msg));
    2
}

const USAGE: &str = concat!(
    "Aufruf: jfs [OPTIONEN] DATEI.jfs [ARGUMENTE ...]\n",
    "        jfs [OPTIONEN] -e CODE [ARGUMENTE ...]\n",
    "        jfs [OPTIONEN] - [ARGUMENTE ...]       (das Skript von der Standardeingabe)\n",
    "Führt ein Skript der Sprache von JFNetworker (.jfs) aus; die Argumente stehen in config.args.\n",
    "\n",
    "Optionen (vor dem Skript):\n",
    "  --check              nur prüfen: Fehler und Warnungen mit Zeile und Spalte auf stdout\n",
    "  --virtual-time       virtuelle Uhr: Timer laufen ohne Warten, now() beginnt bei 0\n",
    "  --until MS           die Ereignisschleife endet, wenn die Uhr MS Millisekunden erreicht\n",
    "  --seed N             rand() mit festem Startwert (sonst zufällig)\n",
    "  --disk DATEI         disk.get/put/delete/scan in einer JSON-Datei (sonst im Speicher)\n",
    "  -c, --config K=WERT  config.K setzen (WERT als JSON, sonst als Text); auch ein JSON-Objekt\n",
    "  --env NAME           die Umgebungsvariable NAME als config.env.NAME\n",
    "  --fuel N             Rechenschritte je Aufruf (0: unbegrenzt; Vorgabe 10000000)\n",
    "  --heap N             Heap-Grenze in Bytes, auch mit k, M, G (Vorgabe 16M)\n",
    "  --depth N            verschachtelte Aufrufe (Vorgabe 500)\n",
    "  --stack N            Werte auf dem Stapel (Vorgabe 100000)\n",
    "  --selftest DIR       die Beispielfälle DIR/*.test (für build_rs.py test)\n",
    "  -h, --help           diese Hilfe\n",
    "  -V, --version        die Version\n",
    "Umgebung: JFS_CONFIG ist ein JSON-Objekt, auf das --config aufsetzt.\n",
    "Im Skript: print, log (stderr), now, rand, config, timer, disk und die Standardbibliothek;\n",
    "net, dns, http und metrics gibt es nur in JFNetworker (ein fangbarer Fehler).\n",
    "Exit-Code: 0 fertig, 1 Fehler im Skript, 2 falscher Aufruf.\n",
);

/// a count with an optional k, M or G (binary units): "64k", "16M"
fn number(s: &str) -> Option<u64> {
    let (digits, mul) = match s.as_bytes().last()? {
        b'k' | b'K' => (&s[..s.len() - 1], 1u64 << 10),
        b'm' | b'M' => (&s[..s.len() - 1], 1 << 20),
        b'g' | b'G' => (&s[..s.len() - 1], 1 << 30),
        _ => (s, 1),
    };
    let digits = digits.replace('_', "");
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u64>().ok()?.checked_mul(mul)
}

fn run() -> i32 {
    let argv: Vec<String> = plat::args().into_iter().skip(1).collect();
    let mut o = Opts {
        check: false,
        virtual_time: false,
        until: None,
        seed: None,
        disk: None,
        config: Vec::new(),
        env: Vec::new(),
        // print output is written after every slice; a large cap keeps it all
        limits: Limits { output: 1 << 28, ..Limits::default() },
        source: Source::Stdin,
        args: Vec::new(),
    };
    let mut source = None;
    let mut selftest = None;
    let mut i = 0;
    while i < argv.len() {
        let a = argv[i].as_str();
        i += 1;
        if a == "-" {
            source = Some(Source::Stdin);
            break;
        }
        if a == "--" {
            match argv.get(i) {
                Some(f) => source = Some(Source::File(f.clone())),
                None => return bad("nach -- fehlt die Skriptdatei"),
            }
            i += 1;
            break;
        }
        if !a.starts_with('-') {
            source = Some(Source::File(String::from(a)));
            break;
        }
        // --name=value or --name value
        let (name, inline) = match a.split_once('=') {
            Some((n, v)) if a.starts_with("--") => (n, Some(String::from(v))),
            _ => (a, None),
        };
        let takes = matches!(name, "-e" | "--until" | "--seed" | "--disk" | "-c" | "--config" | "--env" | "--fuel" | "--heap" | "--depth" | "--stack" | "--selftest" | "--script");
        let value = if takes {
            match inline {
                Some(v) => v,
                None => match argv.get(i) {
                    Some(v) => {
                        i += 1;
                        v.clone()
                    }
                    None => return bad(&format!("{} braucht einen Wert", name)),
                },
            }
        } else if inline.is_some() {
            return bad(&format!("{} nimmt keinen Wert", name));
        } else {
            String::new()
        };
        let num = |v: &str| number(v).ok_or_else(|| format!("{} braucht eine Zahl, nicht \u{201e}{}\u{201c}", name, v));
        let r: Result<(), String> = match name {
            "-h" | "--help" => {
                out(USAGE);
                return 0;
            }
            "-V" | "--version" => {
                out(&format!("jfs {} (JFLib {})\n", APP_VERSION, plat::VERSION));
                return 0;
            }
            "--check" => {
                o.check = true;
                Ok(())
            }
            "--virtual-time" => {
                o.virtual_time = true;
                Ok(())
            }
            "-e" => {
                source = Some(Source::Code(value));
                break;
            }
            "--until" => num(&value).map(|n| o.until = Some(n.saturating_mul(1_000_000))),
            "--seed" => num(&value).map(|n| o.seed = Some(n)),
            "--disk" => {
                o.disk = Some(value);
                Ok(())
            }
            "-c" | "--config" => {
                o.config.push(value);
                Ok(())
            }
            "--env" => {
                o.env.push(value);
                Ok(())
            }
            "--fuel" => num(&value).map(|n| o.limits.fuel = if n == 0 { u64::MAX } else { n }),
            "--heap" => num(&value).map(|n| o.limits.heap = n),
            "--depth" => num(&value).and_then(|n| u32::try_from(n).map_err(|_| format!("{} ist zu groß", name))).map(|n| o.limits.depth = n),
            "--stack" => num(&value).and_then(|n| u32::try_from(n).map_err(|_| format!("{} ist zu groß", name))).map(|n| o.limits.stack = n),
            "--selftest" => {
                selftest = Some(value);
                Ok(())
            }
            "--script" => Ok(()), // build_rs.py test passes its .uis file: nothing to do with it
            _ => Err(format!("unbekannte Option {}", a)),
        };
        if let Err(m) = r {
            return bad(&m);
        }
    }
    if let Some(dir) = selftest {
        return selftest::run(&dir);
    }
    let Some(source) = source else {
        if argv.is_empty() {
            err(USAGE);
            return 2;
        }
        return bad("es fehlt das Skript: eine Datei, -e CODE oder - für die Standardeingabe");
    };
    o.args = argv[i..].to_vec();
    o.source = source;
    let (name, src) = match load(&o.source) {
        Ok(x) => x,
        Err(m) => {
            err(&format!("jfs: {}\n", m));
            return 2;
        }
    };
    if o.check {
        return check(&name, &src);
    }
    let prog = match script::compile_with(&name, &src, script::GLOBALS) {
        Ok(p) => p,
        Err(diags) => {
            let mut s = String::new();
            for d in &diags {
                s.push_str(&format!("{}:{}\n", name, d));
                excerpt(&mut s, &src, d.line, d.col, d.end_col);
            }
            err(&s);
            return 1;
        }
    };
    host::run(&prog, &o, &name, &src)
}

/// the script's name for messages and its text
fn load(s: &Source) -> Result<(String, String), String> {
    let (name, bytes) = match s {
        Source::Code(c) => return Ok((String::from("-e"), c.clone())),
        Source::File(f) => match plat::read_file(f) {
            Some(b) => (f.clone(), b),
            None => {
                let why = if plat::file_exists(f) == 0 { "gibt es nicht" } else { "lässt sich nicht lesen" };
                return Err(format!("{} {}", f, why));
            }
        },
        Source::Stdin => {
            let mut b = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                match plat::read_stdin(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => b.extend_from_slice(&buf[..n]),
                    Err(e) => return Err(format!("die Standardeingabe lässt sich nicht lesen (Fehler {})", e)),
                }
            }
            (String::from("<stdin>"), b)
        }
    };
    match String::from_utf8(bytes) {
        Ok(t) => Ok((name, t.strip_prefix('\u{feff}').map(String::from).unwrap_or(t))),
        Err(_) => Err(format!("{} ist kein UTF-8-Text", name)),
    }
}

/// --check: every diagnostic, with the host's names (an unknown one is an error as in a run)
fn check(name: &str, src: &str) -> i32 {
    let errors = match script::compile_with(name, src, script::GLOBALS) {
        Ok(_) => Vec::new(),
        Err(d) => d,
    };
    let mut all: Vec<Diagnostic> = script::check_with(src, script::GLOBALS);
    for d in all.iter_mut() {
        if errors.iter().any(|e| (e.line, e.col, &e.msg) == (d.line, d.col, &d.msg)) {
            d.error = true;
        }
    }
    let mut s = String::new();
    for d in &all {
        s.push_str(&format!("{}:{}\n", name, d));
    }
    out(&s);
    if all.iter().any(|d| d.error) { 1 } else { 0 }
}

/// the source line `line` and a mark under columns col .. end_col (characters, from 1)
pub fn excerpt(s: &mut String, src: &str, line: u32, col: u32, end_col: u32) {
    let Some(text) = src.split('\n').nth((line as usize).wrapping_sub(1)) else { return };
    let text = text.trim_end_matches('\r');
    if line == 0 || text.trim().is_empty() {
        return;
    }
    s.push_str("    ");
    s.push_str(text);
    s.push_str("\n    ");
    for c in text.chars().take(col.saturating_sub(1) as usize) {
        s.push(if c == '\t' { '\t' } else { ' ' });
    }
    for _ in 0..end_col.saturating_sub(col).max(1) {
        s.push('^');
    }
    s.push('\n');
}

rt::main!(run());
