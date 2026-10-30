// selftest -- `jfs --selftest DIR`: the example cases of DIR, each run by this program started
// anew, against their expected output
//
//   DIR/NAME.test   one case: one or more runs of jfs and what they must give
//     # a comment
//     args: ARGS          the command line ("double" quotes with \" and \\ inside, or 'single'
//                         quotes, keep blanks together)
//     env: NAME=VALUE     an environment variable of the run (repeatable; JFS_CONFIG is
//                         removed unless a line sets it)
//     exit: N             the exit code (default 0)
//     --- stdin           the lines up to the next section: the standard input
//     --- stdout          ... the standard output (default: none)
//     --- stderr          ... the standard error (default: none)
//     --- file PATH       ... what PATH holds after the run
//     ===                 the next run of the case (args, env, exit and sections of its own)
//   {dir} in args, env, paths and texts is DIR, {tmp} a scratch folder of the case
//   ($TEMP/jfs-*/NAME, removed afterwards; PERMISSIONS allows it).  Texts are compared
//   without \r and without blank lines at the end.  Prints "ok NAME" or "FAIL NAME ..." per
//   case, then the count; exit code 1 when a case failed.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::out;

#[derive(Default)]
struct Run {
    args: Vec<String>,
    env: Vec<(String, String)>,
    exit: i32,
    stdin: String,
    stdout: String,
    stderr: String,
    files: Vec<(String, String)>,
}

/// a command line split like a shell's (no variables, no globs)
fn split(s: &str) -> Result<Vec<String>, String> {
    let mut v = Vec::new();
    let mut cur = String::new();
    let mut has = false;
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            ' ' | '\t' => {
                if has {
                    v.push(core::mem::take(&mut cur));
                    has = false;
                }
            }
            '"' => {
                has = true;
                loop {
                    match it.next() {
                        None => return Err(String::from("ein \" ist nicht geschlossen")),
                        Some('"') => break,
                        Some('\\') => match it.next() {
                            Some(e @ ('"' | '\\')) => cur.push(e),
                            Some(e) => {
                                cur.push('\\');
                                cur.push(e);
                            }
                            None => return Err(String::from("ein \" ist nicht geschlossen")),
                        },
                        Some(x) => cur.push(x),
                    }
                }
            }
            '\'' => {
                has = true;
                loop {
                    match it.next() {
                        None => return Err(String::from("ein ' ist nicht geschlossen")),
                        Some('\'') => break,
                        Some(x) => cur.push(x),
                    }
                }
            }
            c => {
                has = true;
                cur.push(c);
            }
        }
    }
    if has {
        v.push(cur);
    }
    Ok(v)
}

/// the runs of a case file; {dir} and {tmp} replaced
fn parse(text: &str, dir: &str, tmp: &str) -> Result<Vec<Run>, String> {
    let sub = |s: &str| s.replace("{dir}", dir).replace("{tmp}", tmp);
    let mut runs = Vec::new();
    let mut r = Run::default();
    // the open section: its kind and lines
    let mut sec: Option<(String, Vec<String>)> = None;
    let close = |r: &mut Run, sec: &mut Option<(String, Vec<String>)>| -> Result<(), String> {
        let Some((kind, lines)) = sec.take() else { return Ok(()) };
        let mut body = String::new();
        for l in &lines {
            body.push_str(&sub(l));
            body.push('\n');
        }
        match kind.as_str() {
            "stdin" => r.stdin = body,
            "stdout" => r.stdout = body,
            "stderr" => r.stderr = body,
            k => match k.strip_prefix("file ") {
                Some(p) => r.files.push((sub(p.trim()), body)),
                None => return Err(format!("unbekannter Abschnitt --- {}", k)),
            },
        }
        Ok(())
    };
    for (n, line) in text.split('\n').enumerate() {
        let line = line.trim_end_matches('\r');
        if line == "===" {
            close(&mut r, &mut sec)?;
            runs.push(core::mem::take(&mut r));
            continue;
        }
        if let Some(k) = line.strip_prefix("--- ") {
            close(&mut r, &mut sec)?;
            sec = Some((String::from(k.trim()), Vec::new()));
            continue;
        }
        if let Some((_, lines)) = sec.as_mut() {
            lines.push(String::from(line));
            continue;
        }
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let at = |m: String| format!("Zeile {}: {}", n + 1, m);
        if let Some(a) = t.strip_prefix("args:") {
            r.args = split(&sub(a)).map_err(at)?;
        } else if let Some(e) = t.strip_prefix("env:") {
            let Some((k, v)) = e.trim().split_once('=') else { return Err(at(String::from("env: NAME=WERT erwartet"))) };
            r.env.push((String::from(k), sub(v)));
        } else if let Some(x) = t.strip_prefix("exit:") {
            r.exit = x.trim().parse().map_err(|_| at(format!("exit: {} ist keine Zahl", x.trim())))?;
        } else {
            return Err(at(format!("unbekannte Zeile {}", t)));
        }
    }
    close(&mut r, &mut sec)?;
    runs.push(r);
    Ok(runs)
}

/// start `exe` with the run's command line, environment and input -> (exit code, stdout,
/// stderr)
fn exec(exe: &str, r: &Run) -> Result<(i32, Vec<u8>, Vec<u8>), String> {
    let args: Vec<&str> = r.args.iter().map(|s| s.as_str()).collect();
    let mut env: Vec<(&str, Option<&str>)> = Vec::new();
    if !r.env.iter().any(|(k, _)| k == "JFS_CONFIG") {
        env.push(("JFS_CONFIG", None));
    }
    for (k, v) in &r.env {
        env.push((k.as_str(), Some(v.as_str())));
    }
    let cmd = plat::process::Command { program: exe, args: &args, cwd: None, env: &env };
    let mut c = cmd.spawn().map_err(|e| format!("{} startet nicht (Fehler {})", exe, e))?;
    let input = r.stdin.as_bytes();
    let mut sent = 0;
    if input.is_empty() {
        c.close_stdin();
    }
    let (mut so, mut se) = (Vec::new(), Vec::new());
    let mut b = [0u8; 4096];
    let deadline = plat::time_us() + 60_000_000;
    let mut code = None;
    loop {
        let mut busy = false;
        if sent < input.len() {
            match c.write_stdin(&input[sent..]) {
                Ok(Some(n)) => {
                    sent += n;
                    busy = true;
                }
                Ok(None) => {}
                Err(_) => sent = input.len(),
            }
            if sent == input.len() {
                c.close_stdin();
            }
        }
        while let Ok(Some(n)) = c.read_stdout(&mut b) {
            if n == 0 {
                break;
            }
            so.extend_from_slice(&b[..n]);
            busy = true;
        }
        while let Ok(Some(n)) = c.read_stderr(&mut b) {
            if n == 0 {
                break;
            }
            se.extend_from_slice(&b[..n]);
            busy = true;
        }
        if code.is_some() && !busy {
            break;
        }
        if code.is_none() {
            // after the end, one more round drains the pipes
            code = c.try_wait().ok().flatten();
        }
        if plat::time_us() > deadline {
            c.terminate();
            return Err(String::from("läuft länger als 60 s"));
        }
        if !busy && code.is_none() {
            plat::sleep_ms(2);
        }
    }
    Ok((code.unwrap_or(-1), so, se))
}

/// a text for comparing: without \r and blank lines at the end
fn norm(s: &str) -> String {
    let s = s.replace('\r', "");
    String::from(s.trim_end_matches('\n'))
}

/// None when equal, else where they differ: "Zeile 2: erwartet „..“, bekommen „..“"
fn differ(want: &str, got: &str) -> Option<String> {
    let (w, g) = (norm(want), norm(got));
    if w == g {
        return None;
    }
    let (wl, gl): (Vec<&str>, Vec<&str>) = (w.split('\n').collect(), g.split('\n').collect());
    let i = (0..wl.len().max(gl.len())).find(|&i| wl.get(i) != gl.get(i)).unwrap_or(0);
    let show = |x: Option<&&str>| x.map_or(String::from("(nichts)"), |s| format!("\u{201e}{}\u{201c}", s));
    Some(format!("Zeile {}: erwartet {}, bekommen {}", i + 1, show(wl.get(i)), show(gl.get(i))))
}

/// remove a scratch folder and what is in it (one level of files)
fn remove(dir: &str) {
    let mut names = Vec::new();
    plat::dir_list(dir, &mut |n, _, _| names.push(String::from(n)));
    for n in names {
        plat::delete(&format!("{}/{}", dir, n));
    }
    plat::delete(dir);
}

/// run every case of `dir` -> the exit code
pub fn run(dir: &str) -> i32 {
    let dir = dir.replace('\\', "/");
    let dir = dir.trim_end_matches('/');
    let exe = plat::current_exe().unwrap_or_default().replace('\\', "/");
    let mut cases = Vec::new();
    plat::dir_list(dir, &mut |n, is_dir, _| {
        if !is_dir && n.ends_with(".test") {
            cases.push(String::from(n));
        }
    });
    cases.sort();
    if cases.is_empty() {
        out(&format!("FAIL keine Fälle (*.test) in {}\n", dir));
        return 1;
    }
    let temp = plat::temp_dir().unwrap_or_default().replace('\\', "/");
    let base = format!("{}/jfs-{:x}", temp.trim_end_matches('/'), plat::time_us() & 0xFF_FFFF);
    if !plat::mkdir(&base) {
        out(&format!("FAIL der Ordner {} lässt sich nicht anlegen\n", base));
        return 1;
    }
    let mut failed = 0;
    for case in &cases {
        let tmp = format!("{}/{}", base, case.trim_end_matches(".test"));
        plat::mkdir(&tmp);
        let fails = run_case(&exe, dir, &tmp, case);
        remove(&tmp);
        if fails.is_empty() {
            out(&format!("ok   {}\n", case));
        } else {
            failed += 1;
            let mut s = String::new();
            for f in fails {
                s.push_str(&format!("FAIL {}{}\n", case, f));
            }
            out(&s);
        }
    }
    plat::delete(&base);
    out(&format!("jfs: {} von {} Fällen bestanden\n", cases.len() - failed, cases.len()));
    if failed == 0 { 0 } else { 1 }
}

/// the failures of one case
fn run_case(exe: &str, dir: &str, tmp: &str, case: &str) -> Vec<String> {
    let text = match plat::read_file(&format!("{}/{}", dir, case)).map(String::from_utf8) {
        Some(Ok(t)) => t,
        _ => return alloc::vec![String::from(": lässt sich nicht lesen")],
    };
    let runs = match parse(&text, dir, tmp) {
        Ok(r) => r,
        Err(m) => return alloc::vec![format!(": {}", m)],
    };
    let mut fails = Vec::new();
    for (i, r) in runs.iter().enumerate() {
        let tag = if runs.len() > 1 { format!(" (Lauf {})", i + 1) } else { String::new() };
        let (code, so, se) = match exec(exe, r) {
            Ok(x) => x,
            Err(m) => {
                fails.push(format!("{}: {}", tag, m));
                continue;
            }
        };
        let (so, se) = (String::from_utf8_lossy(&so), String::from_utf8_lossy(&se));
        if code != r.exit {
            let first = norm(&se);
            let first = first.split('\n').next().unwrap_or("");
            fails.push(format!("{}: Exit-Code {} statt {} (stderr: {})", tag, code, r.exit, first));
        }
        if let Some(d) = differ(&r.stdout, &so) {
            fails.push(format!("{}: stdout {}", tag, d));
        }
        if let Some(d) = differ(&r.stderr, &se) {
            fails.push(format!("{}: stderr {}", tag, d));
        }
        for (path, want) in &r.files {
            match plat::read_file(path) {
                Some(b) => {
                    if let Some(d) = differ(want, &String::from_utf8_lossy(&b)) {
                        fails.push(format!("{}: {} {}", tag, path, d));
                    }
                }
                None => fails.push(format!("{}: {} fehlt", tag, path)),
            }
        }
    }
    fails
}
