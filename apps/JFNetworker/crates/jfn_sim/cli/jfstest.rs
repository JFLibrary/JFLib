// jfstest -- `jfs --test FILE ...` in the shell of a host: the test functions of jfs files of
// the device, one after another in one process of the shell (IDE1)
//
//   Engine::sh_jfs_test(ni, args)    the files' tests as a console program (JfsTest); the shell's
//                                    `jfs` (cli/shrun.rs) hands it the words after --test
//   Engine::jfs_tests(dev) -> Option<&TestRun>   the device's last run: the files, the tests
//                                    found, the results so far, whether it ended (the IDE's
//                                    Tests pane reads it)
//   TestRun / TestResult             what a run found and what each test did
//
// Every top-level fn whose name starts with test_ is a test, in the order of its file.  Each
// runs in a program of its own: the file as a service of the device would run it
// (ScriptService::new_file, so its relative imports start in its directory) with stop_on_error,
// its top-level code first -- every test has fresh globals and modules --, then the test, then
// its timers and callbacks on the virtual clock.  A test passes when nothing is left to do (as
// `jfs FILE` ends) or when its time is up (TEST_LIMIT of virtual time: a test that starts a
// server ends there), and fails with its first error (assert and assert_eq throw one with both
// values) or an error of the file's top-level code.  Between two tests the program goes and what
// it opened closes (ScriptService::release).  A file that cannot be read or does not compile
// counts as one failure.
//
// The report in the terminal says what the runner apps/jfs says (testrun.rs): "ok      NAME
// (FILE)", "FEHLER  NAME (FILE:LINE:COL): message" with what the test printed ("        | " in
// front), then "N Tests: P bestanden, F fehlgeschlagen"; the exit code is 1 when a test failed,
// a file does not load or compile or no test was found, else 0.  Ctrl+C ends the run (130), the
// record keeps what ran.  The record (Engine::jfs_tests) is the engine's, per device, and a
// new run replaces it; it is not part of the project.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

use script::ls::SymKind;

use crate::kernel::{Engine, Ns, SEC};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::scriptsvc::{ScriptCfg, ScriptService, ScriptState};

/// the virtual time a test may take at most
pub const TEST_LIMIT: Ns = 10 * SEC;
/// the timer key of JfsTest itself (the script's service uses the keys 0 to 2)
const K_TEST: u32 = 3;
/// bytes of a test's output a result keeps
const OUTPUT_KEEP: usize = 4096;

/// what one test did
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestResult {
    /// the file as absolute path, the test's name and the line of its fn (from 1; 0 for a file
    /// that did not load or compile)
    pub file: String,
    pub name: String,
    pub line: u32,
    pub ok: bool,
    /// where it failed: file (a module's path when the error is in one), line, column (0: no
    /// place)
    pub at: Option<(String, u32, u32)>,
    /// why it failed (German, as apps/jfs/crates/script words it)
    pub msg: String,
    /// what it printed (the last OUTPUT_KEEP bytes)
    pub output: String,
}

/// a run of `jfs --test` on a device
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TestRun {
    /// the files as typed and as absolute paths
    pub files: Vec<(String, String)>,
    /// the tests found: file (absolute), name, line
    pub tests: Vec<(String, String, u32)>,
    pub results: Vec<TestResult>,
    /// every test ran (or Ctrl+C ended the run)
    pub done: bool,
    /// when it started (virtual time)
    pub started: Ns,
}

impl TestRun {
    /// (passed, failed)
    pub fn counts(&self) -> (usize, usize) {
        let p = self.results.iter().filter(|r| r.ok).count();
        (p, self.results.len() - p)
    }
    fn footprint(&self) -> usize {
        let strs = |v: &[(String, String)]| v.iter().map(|x| x.0.len() + x.1.len() + 48).sum::<usize>();
        strs(&self.files) + self.tests.iter().map(|t| t.0.len() + t.1.len() + 56).sum::<usize>() + self.results.iter().map(|r| 128 + r.file.len() + r.name.len() + r.msg.len() + r.output.len()).sum::<usize>()
    }
}

/// the bytes the engine's test records hold (keyframe.rs counts them)
pub(crate) fn tests_footprint(e: &Engine) -> usize {
    e.jfs_tests.values().map(|r| r.footprint() + 64).sum()
}

/// a test still to run
#[derive(Clone)]
struct Planned {
    abs: String,
    shown: String,
    src: Arc<String>,
    name: String,
    line: u32,
}

/// the test that runs
#[derive(Clone)]
struct Cur {
    svc: ScriptService,
    p: Planned,
    since: Ns,
}

/// the console program of a run (see the module comment)
#[derive(Clone)]
pub struct JfsTest {
    todo: VecDeque<Planned>,
    cur: Option<Cur>,
    passed: u32,
    failed: u32,
    done: bool,
}

/// the text a result keeps of an output (its end)
fn tail(s: &str) -> String {
    let mut cut = s.len().saturating_sub(OUTPUT_KEEP);
    while !s.is_char_boundary(cut) {
        cut += 1;
    }
    String::from(&s[cut..])
}

/// "FEHLER  NAME (FILE:LINE:COL): message" with the output below it
fn failure_text(r: &TestResult, shown: &str) -> String {
    let at = match &r.at {
        Some((f, l, c)) if *l > 0 => format!("{}:{}:{}", if *f == r.file { shown } else { f.as_str() }, l, c),
        Some((f, _, _)) if *f != r.file => f.clone(),
        _ => String::from(shown),
    };
    let mut s = format!("FEHLER  {} ({}): {}\n", r.name, at, r.msg);
    for l in r.output.lines() {
        s.push_str("        | ");
        s.push_str(l);
        s.push('\n');
    }
    s
}

/// "N Tests: P bestanden, F fehlgeschlagen"
fn summary(passed: u32, failed: u32) -> String {
    let n = passed + failed;
    format!("{} {}: {} bestanden, {} fehlgeschlagen\n", n, if n == 1 { "Test" } else { "Tests" }, passed, failed)
}

impl Engine {
    /// the last run of `jfs --test` on device `dev`
    pub fn jfs_tests(&self, dev: u32) -> Option<&TestRun> {
        self.jfs_tests.get(&dev)
    }

    /// jfs --test FILE ...: the test functions of the files, one after another
    pub(crate) fn sh_jfs_test(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        if args.is_empty() {
            self.sh_set_code(ni, 2);
            return self.sh_err(ni, "jfs --test: welche Dateien? Aufruf: jfs --test DATEI.jfs ...\n");
        }
        let dev = self.dev_of(ni);
        let mut run = TestRun { started: self.now, ..TestRun::default() };
        let mut todo = VecDeque::new();
        let mut text = String::new();
        let mut failed = 0u32;
        for f in args {
            let abs = self.sh_abs(ni, f);
            let abs = self.store(ni).and_then(|s| s.norm(&abs).ok()).unwrap_or(abs);
            run.files.push((String::from(*f), abs.clone()));
            let bad = |msg: String, at: Option<(String, u32, u32)>| TestResult { file: abs.clone(), name: String::new(), line: 0, ok: false, at, msg, output: String::new() };
            let data = match self.sh_read(ni, &abs) {
                Ok(d) => d,
                Err(e) => {
                    text.push_str(&format!("FEHLER  {}: {}\n", f, e));
                    run.results.push(bad(String::from(e), None));
                    failed += 1;
                    continue;
                }
            };
            let Ok(src) = String::from_utf8(data) else {
                text.push_str(&format!("FEHLER  {}: das ist kein Text in UTF-8\n", f));
                run.results.push(bad(String::from("das ist kein Text in UTF-8"), None));
                failed += 1;
                continue;
            };
            if let Err(diags) = script::compile_named(&abs, &src) {
                text.push_str(&format!("FEHLER  {} lässt sich nicht übersetzen\n", f));
                for d in diags.iter().filter(|d| d.error) {
                    text.push_str(&format!("{}:{}\n", f, d));
                }
                let first = diags.iter().find(|d| d.error);
                run.results.push(bad(first.map_or_else(|| String::from("lässt sich nicht übersetzen"), |d| d.msg.clone()), first.map(|d| (abs.clone(), d.line, d.col))));
                failed += 1;
                continue;
            }
            let src = Arc::new(src);
            let tests: Vec<(String, u32)> = script::ls::symbols(&src).into_iter().filter(|s| s.kind == SymKind::Fn && s.name.starts_with("test_")).map(|s| (s.name, s.span.line)).collect();
            if tests.is_empty() {
                text.push_str(&format!("{}: keine Tests (Funktionen test_…)\n", f));
            }
            for (name, line) in tests {
                run.tests.push((abs.clone(), name.clone(), line));
                todo.push_back(Planned { abs: abs.clone(), shown: String::from(*f), src: src.clone(), name, line });
            }
        }
        self.out(ni, &text);
        if todo.is_empty() {
            run.done = true;
            self.jfs_tests.insert(dev, run);
            self.out(ni, &summary(0, failed));
            self.sh_set_code(ni, 1);
            return Ok(());
        }
        self.jfs_tests.insert(dev, run);
        let p = JfsTest { todo, cur: None, passed: 0, failed, done: false };
        self.run_program(ni, "jfs", Box::new(p), false)
    }
}

impl JfsTest {
    /// the device's record of the run
    fn record(os: &mut OsApi, f: impl FnOnce(&mut TestRun)) {
        let dev = os.dev();
        if let Some(r) = os.eng.jfs_tests.get_mut(&dev) {
            f(r);
        }
    }

    /// the result of the test that ran: the terminal and the record
    fn finish(&mut self, os: &mut OsApi, c: Cur, ok: bool) {
        let mut c = c;
        let (out, _) = c.svc.run_output(0);
        let output = tail(out);
        let err = if ok { None } else { c.svc.run_errors().last().cloned() };
        c.svc.release(os);
        let r = TestResult {
            file: c.p.abs.clone(),
            name: c.p.name.clone(),
            line: c.p.line,
            ok,
            at: err.as_ref().map(|e| (if e.file.is_empty() { c.p.abs.clone() } else { e.file.clone() }, e.line, e.col)),
            msg: err.map(|e| e.msg).unwrap_or_default(),
            output,
        };
        if ok {
            self.passed += 1;
            os.print(&format!("ok      {} ({})\n", r.name, c.p.shown));
        } else {
            self.failed += 1;
            os.print(&failure_text(&r, &c.p.shown));
        }
        JfsTest::record(os, |run| run.results.push(r));
    }

    /// on to the next test, or the end
    fn next(&mut self, os: &mut OsApi) {
        while self.cur.is_none() {
            let Some(p) = self.todo.pop_front() else {
                self.done = true;
                os.print(&summary(self.passed, self.failed));
                JfsTest::record(os, |r| r.done = true);
                os.exit(if self.failed > 0 || self.passed == 0 { 1 } else { 0 });
                return;
            };
            let cfg = ScriptCfg { stop_on_error: true, ..ScriptCfg::default() };
            let mut conf = agent_core::jmap::empty_object();
            conf.insert("args", agent_core::jmap::Value::Array(Vec::new()));
            conf.insert("file", agent_core::jmap::Value::string(&p.abs));
            conf.insert("test", agent_core::jmap::Value::string(&p.name));
            // the file's code, then the test (a line of its own after the last one: the
            // lines and columns stay the file's)
            let src = format!("{}\n{}()\n", p.src, p.name);
            match ScriptService::new_file(&p.abs, &src, cfg, &conf).map(ScriptService::hosted) {
                Ok(mut svc) => {
                    svc.start(os);
                    self.cur = Some(Cur { svc, p, since: os.now() });
                    self.check(os);
                }
                Err(d) => {
                    // (the file compiled; only a name clash with the call could do this)
                    let msg = d.first().map_or_else(String::new, |x| x.msg.clone());
                    let r = TestResult { file: p.abs.clone(), name: p.name.clone(), line: p.line, ok: false, at: None, msg, output: String::new() };
                    self.failed += 1;
                    os.print(&failure_text(&r, &p.shown));
                    JfsTest::record(os, |run| run.results.push(r));
                }
            }
        }
    }

    /// after every call: the test that runs failed, has nothing left to do or used its time
    fn check(&mut self, os: &mut OsApi) {
        let now = os.now();
        let Some(c) = self.cur.as_ref() else { return };
        let failed = c.svc.state() == ScriptState::Failed;
        let idle = c.svc.idle_at();
        let limit = c.since.saturating_add(TEST_LIMIT);
        if failed || idle.is_some_and(|t| t <= now) || now >= limit {
            if let Some(c) = self.cur.take() {
                self.finish(os, c, !failed);
            }
            self.next(os);
            return;
        }
        os.timer(K_TEST, idle.map_or(limit, |t| t.min(limit)));
    }
}

impl Service for JfsTest {
    fn kind(&self) -> &'static str {
        "jfs"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.next(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if self.done {
            return;
        }
        if ev != SvcEvent::Timer(K_TEST)
            && let Some(c) = self.cur.as_mut()
        {
            c.svc.on(ev, os);
        }
        self.check(os);
    }
    fn stop(&mut self, os: &mut OsApi) {
        // Ctrl+C or the device's power: the record keeps what ran
        if let Some(mut c) = self.cur.take() {
            c.svc.stop(os);
        }
        if !self.done {
            self.done = true;
            JfsTest::record(os, |r| r.done = true);
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        self.cur.as_ref().map(|c| c.svc.metrics()).unwrap_or_default()
    }
    fn status(&self) -> String {
        match &self.cur {
            Some(c) => format!("Test {} ({})", c.p.name, c.p.shown),
            None => String::from("Tests"),
        }
    }
    fn renamed(&mut self, name: &str) {
        if let Some(c) = self.cur.as_mut() {
            c.svc.renamed(name);
        }
    }
    fn footprint(&self) -> usize {
        let todo: usize = self.todo.iter().map(|p| 96 + p.abs.len() + p.shown.len() + p.name.len()).sum();
        1024 + todo + self.cur.as_ref().map_or(0, |c| c.svc.footprint())
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
