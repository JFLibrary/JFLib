// Native process contract checks, invoked explicitly by the integration harness.
use alloc::{format, string::String, vec::Vec};
use plat::process::{Child, Command};

fn drain(child: &mut Child, timeout: u64) -> Result<(i32, Vec<u8>, Vec<u8>), String> {
    let end = plat::time_us() + timeout;
    let mut out = Vec::new(); let mut err = Vec::new(); let mut buf = [0u8; 8192];
    let mut eof = [false; 2]; let mut code = None;
    while plat::time_us() < end {
        for (i, target) in [&mut out, &mut err].into_iter().enumerate() {
            let result = if i == 0 { child.read_stdout(&mut buf) } else { child.read_stderr(&mut buf) };
            match result {
                Ok(Some(0)) => eof[i] = true,
                Ok(Some(n)) => target.extend_from_slice(&buf[..n]),
                Ok(None) => {},
                Err(e) => return Err(format!("read: {e}")),
            }
            if target.len() > 2_000_000 { return Err("unbounded output".into()); }
        }
        if code.is_none() { code = child.try_wait().map_err(|e| format!("wait: {e}"))?; }
        if eof.iter().all(|v| *v) && code.is_some() { return Ok((code.unwrap_or(-1), out, err)); }
        plat::sleep_ms(2);
    }
    Err("process timeout".into())
}
fn check(python: &str, scratch: &str) -> Result<(), String> {
    let code = "import os,sys; assert sys.argv[1:]==['', 'hello world', 'quote\"slash\\\\', 'Grüße']; assert os.environ['jfgui_PROCESS_TEST']=='Grüße'; assert 'jfgui_PROCESS_REMOVE' not in os.environ; assert os.getcwd()==sys.stdin.readline().strip(); print('ready',flush=True); sys.stdout.write('A'*200000); sys.stderr.write('B'*200000); sys.exit(7)";
    plat::setenv("jfgui_PROCESS_REMOVE", Some("remove"));
    let mut child = Command { program: python, args: &["-u", "-c", code, "", "hello world", "quote\"slash\\", "Grüße"],
        cwd: Some(scratch), env: &[("PYTHONIOENCODING", Some("utf-8")), ("jfgui_PROCESS_TEST", Some("Grüße")), ("jfgui_PROCESS_REMOVE", None)] }.spawn().map_err(|e| format!("spawn: {e}"))?;
    let input = format!("{}\n", if cfg!(windows) { scratch.replace('/', "\\") } else { String::from(scratch) });
    let mut offset = 0;
    while offset < input.len() { if let Some(n) = child.write_stdin(&input.as_bytes()[offset..]).map_err(|e| format!("stdin: {e}"))? { offset += n; } }
    child.close_stdin();
    let (status, out, err) = drain(&mut child, 10_000_000)?;
    if status != 7 || !out.starts_with(b"ready") || out.iter().filter(|b| **b == b'A').count() != 200000 || err.len() != 200000 {
        return Err(format!("IO mismatch: {status}, {} stdout, {} stderr: {}", out.len(), err.len(), String::from_utf8_lossy(&err[..err.len().min(500)])));
    }
    drop(child);
    if (Command { program: "jfgui-definitely-missing-program", args: &[], cwd: None, env: &[] }).spawn().is_ok() { return Err("missing program succeeded".into()); }
    if (Command { program: python, args: &[], cwd: Some("jfgui-definitely-missing-directory"), env: &[] }).spawn().is_ok() { return Err("missing cwd succeeded".into()); }
    let marker = format!("{scratch}/tree.txt");
    let probe = format!("{}/apps/buildManager/tests/process_probe.py", plat::cwd());
    let mut child = Command { program: python, args: &["-u", &probe, "parent", &marker], cwd: Some(scratch), env: &[] }.spawn().map_err(|e| format!("tree spawn: {e}"))?;
    child.write_stdin(b"go\n").map_err(|e| format!("tree stdin: {e}"))?;
    let end = plat::time_us() + 5_000_000;
    while plat::file_exists(&marker) != 1 && plat::time_us() < end { plat::sleep_ms(20); }
    if plat::file_exists(&marker) != 1 { return Err("descendant did not start".into()); }
    child.terminate();
    let before = plat::read_file(&marker);
    plat::sleep_ms(150);
    if before != plat::read_file(&marker) { return Err("descendant survived cancellation".into()); }
    Ok(())
}
pub(crate) fn run(python: &str, scratch: &str) -> i32 {
    match check(python, scratch) {
        Ok(()) => { plat::write_stdout(b"PASS native process: argv, unicode, environment, cwd, pipes, exit, tree cancellation\n"); 0 },
        Err(e) => { plat::write_stderr(format!("FAIL native process: {e}\n").as_bytes()); 1 },
    }
}
