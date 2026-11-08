// cargo jflib watch [-p PKG] [-- ARGS]: hot reload (tools/hotreload.py of the JFLib repository, the
// runtime part lib/ui/hotreload.rs, docs/GUIDE.md section 30) for an app outside it.
//
// Watches every file of the project's own packages (the workspace's and those it reaches by path;
// target/ and hidden folders but .cargo left out), polled; builds a development build when files
// were saved and starts the app again in its session DIR = target/jflib-watch/NAME:
//   * the running app is asked to end (DIR/ctl "quit"; killed after STOP_WAIT): it writes the end
//     of its input log, its window placement (jfgui_STATE) and the carried state
//   * the new build runs as a copy NAME.hot[.exe] next to the build with jfgui_HOTRELOAD=DIR: it
//     replays the log -- the app's state comes back through its own code -- and goes on recording
//   * the replay diverged (exit code 3): the log is put aside, the app starts afresh
//   * the build failed: the old version keeps running; the error goes to the console and to
//     DIR/build.txt, which the app shows at the top of its inspector
//   * the app ended by itself: its log is put aside, it starts afresh after the next change
// The machine's own target only (Windows or Linux).  Ctrl+C ends watch and the app.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime};

use crate::cargo::{self, Project};
use crate::json::{self, Value};
use crate::package::{LINUX, WIN};

const EXIT_DIVERGED: i32 = 3; // lib/ui/hotreload.rs HR_EXIT_DIVERGED
const POLL: Duration = Duration::from_millis(300);
const QUIET: Duration = Duration::from_millis(300);
const STOP_WAIT: Duration = Duration::from_secs(5);

static STOP: AtomicBool = AtomicBool::new(false);

/// Ctrl+C (and Ctrl+Break, SIGTERM) sets STOP instead of ending this program at once: the app is
/// asked to end first, as at a change
#[allow(unsafe_code)]
mod interrupt {
    use super::STOP;
    use std::sync::atomic::Ordering;

    #[cfg(windows)]
    pub fn catch() {
        type Handler = unsafe extern "system" fn(u32) -> i32;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetConsoleCtrlHandler(handler: Option<Handler>, add: i32) -> i32;
        }
        unsafe extern "system" fn on_ctrl(_kind: u32) -> i32 {
            STOP.store(true, Ordering::SeqCst);
            1
        }
        // SAFETY: the handler only stores into an atomic
        unsafe {
            SetConsoleCtrlHandler(Some(on_ctrl), 1);
        }
    }

    #[cfg(unix)]
    pub fn catch() {
        unsafe extern "C" {
            fn signal(sig: i32, handler: usize) -> usize;
        }
        extern "C" fn on_signal(_sig: i32) {
            STOP.store(true, Ordering::SeqCst);
        }
        // SAFETY: SIGINT and SIGTERM get a handler that only stores into an atomic
        unsafe {
            signal(2, on_signal as extern "C" fn(i32) as usize);
            signal(15, on_signal as extern "C" fn(i32) as usize);
        }
    }

    #[cfg(not(any(windows, unix)))]
    pub fn catch() {}
}

/// the project's own packages: those without a registry source (the workspace, path dependencies)
fn local_dirs(p: &Project) -> Result<Vec<PathBuf>, String> {
    let out = cargo::command().args(["metadata", "--format-version", "1"]).current_dir(&p.package.dir).stderr(Stdio::inherit())
        .output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err("cargo metadata failed".to_string());
    }
    let meta = json::parse(&String::from_utf8_lossy(&out.stdout))?;
    let mut dirs: Vec<PathBuf> = meta.get("packages").map(Value::arr).unwrap_or_default().iter()
        .filter(|pk| pk.get("source").is_none_or(Value::is_null))
        .filter_map(|pk| Path::new(pk.s("manifest_path")).parent().map(Path::to_path_buf)).collect();
    dirs.push(p.workspace_root.clone()); // Cargo.lock, .cargo/config.toml
    dirs.sort();
    dirs.dedup();
    Ok(dirs)
}

/// every file below the project's folders -> (modification time, size)
fn scan(dirs: &[PathBuf], target: &Path) -> BTreeMap<PathBuf, (SystemTime, u64)> {
    fn walk(d: &Path, target: &Path, top: bool, out: &mut BTreeMap<PathBuf, (SystemTime, u64)>) {
        let Ok(entries) = fs::read_dir(d) else { return };
        for e in entries.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                let hidden = name.starts_with('.') && name != ".cargo";
                if !hidden && p != target && !(top && name == "target") {
                    walk(&p, target, false, out);
                }
            } else if let Ok(m) = e.metadata() {
                out.insert(p, (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), m.len()));
            }
        }
    }
    let mut out = BTreeMap::new();
    for d in dirs {
        walk(d, target, true, &mut out);
    }
    out
}

struct Session<'a> {
    p: &'a Project,
    triple: &'static str,
    dir: PathBuf,
    args: Vec<String>,
    /// the build's program and the copy that runs
    exe: Option<PathBuf>,
    run_exe: Option<PathBuf>,
    child: Option<Child>,
    replay_seen: Option<(SystemTime, u64)>,
}

impl Session<'_> {
    fn path(&self, n: &str) -> PathBuf {
        self.dir.join(n)
    }

    /// the session's log out of the way: the next start is fresh
    fn put_aside(&self, why: &str) {
        let log = self.path("events.log");
        if log.is_file() {
            let _ = fs::rename(&log, self.path(&format!("events.{why}.log")));
        }
    }

    /// replay.txt when it changed since the last call
    fn replay_report(&mut self) -> Option<String> {
        let m = fs::metadata(self.path("replay.txt")).ok()?;
        let key = (m.modified().ok()?, m.len());
        if Some(key) == self.replay_seen {
            return None;
        }
        self.replay_seen = Some(key);
        fs::read_to_string(self.path("replay.txt")).ok().map(|t| t.trim().to_string())
    }

    /// a development build -> it worked; build.txt says the error (the app shows it)
    fn build(&mut self) -> bool {
        let t0 = Instant::now();
        let mut cmd = cargo::command();
        cmd.current_dir(&self.p.package.dir).args(["build", "-p", &self.p.package.name, "--color", "never", "--message-format=json-render-diagnostics"]);
        // as `cargo run` builds here (its build is taken again): Windows for the machine itself,
        // Linux always with --target (the template's rustflags must not reach the build scripts)
        if self.triple == LINUX {
            cmd.args(["--target", LINUX]);
        }
        cmd
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let out = match cmd.output() {
            Ok(o) => o,
            Err(e) => {
                println!("watch: cargo: {e}");
                return false;
            }
        };
        let errors = String::from_utf8_lossy(&out.stderr).to_string();
        let mut exe = None;
        for line in String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.starts_with('{')) {
            if let Ok(m) = json::parse(line)
                && m.s("reason") == "compiler-artifact"
                && m.s("package_id") == self.p.package.id
                && let Some(e) = m.get("executable").and_then(Value::str)
            {
                exe = Some(PathBuf::from(e));
            }
        }
        let ok = out.status.success() && exe.is_some();
        let shown: String = errors.lines().filter(|l| !["Compiling", "Finished", "Blocking", "Locking"].iter().any(|w| l.trim_start().starts_with(w)))
            .collect::<Vec<_>>().join("\n");
        let _ = fs::write(self.path("build.txt"), if ok { String::new() } else { shown.trim_end().to_string() + "\n" });
        println!("watch: build {} ({:.1} s)", if ok { "done" } else { "FAILED" }, t0.elapsed().as_secs_f32());
        if !ok {
            println!("{}", shown.trim_end());
        } else {
            self.exe = exe;
        }
        ok
    }

    /// the build copied to NAME.hot[.exe] next to it (the ended program's file may stay locked a moment)
    fn install(&mut self) -> Result<(), String> {
        let exe = self.exe.clone().ok_or("no build")?;
        let name = format!("{}.hot{}", self.p.package.name, if self.triple == WIN { ".exe" } else { "" });
        let run = exe.with_file_name(name);
        let mut last = String::new();
        for _ in 0..50 {
            match fs::copy(&exe, &run) {
                Ok(_) => {
                    self.run_exe = Some(run);
                    return Ok(());
                }
                Err(e) => last = e.to_string(),
            }
            sleep(Duration::from_millis(100));
        }
        Err(format!("{}: {last}", run.display()))
    }

    fn start(&mut self) -> Result<(), String> {
        let _ = fs::remove_file(self.path("ctl"));
        let run = self.run_exe.clone().ok_or("nothing built")?;
        let mut cmd = Command::new(&run);
        cmd.args(&self.args).current_dir(&self.p.package.dir).stdin(Stdio::null())
            .env("jfgui_HOTRELOAD", &self.dir).env("jfgui_STATE", self.path("window.ini"));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0); // Ctrl+C reaches watch, which asks the app to end
        }
        self.child = Some(cmd.spawn().map_err(|e| format!("{}: {e}", run.display()))?);
        Ok(())
    }

    /// ask the app to end, kill it after STOP_WAIT -> its exit code
    fn stop(&mut self) -> Option<i32> {
        let mut child = self.child.take()?;
        if child.try_wait().ok().flatten().is_none() {
            let _ = fs::write(self.path("ctl"), "quit\n");
            let t0 = Instant::now();
            while child.try_wait().ok().flatten().is_none() && t0.elapsed() < STOP_WAIT {
                sleep(Duration::from_millis(50));
            }
            if child.try_wait().ok().flatten().is_none() {
                println!("watch: the app did not end; killed");
                let _ = child.kill();
            }
        }
        let code = child.wait().ok().and_then(|s| s.code());
        let _ = fs::remove_file(self.path("ctl"));
        code
    }

    /// the app ended by itself? diverged: started again afresh; else its log put aside
    fn check(&mut self) -> Result<(), String> {
        let Some(child) = self.child.as_mut() else { return Ok(()) };
        let Some(status) = child.try_wait().map_err(|e| e.to_string())? else { return Ok(()) };
        self.child = None;
        if status.code() == Some(EXIT_DIVERGED) {
            let why = self.replay_report().unwrap_or_else(|| "diverged".to_string());
            println!("watch: the replay {why}; the app starts afresh");
            self.put_aside("diverged");
            return self.start();
        }
        println!("watch: the app ended ({status}); it starts afresh after the next change");
        self.put_aside("ended");
        Ok(())
    }

    /// a build; it worked: the app started again in its session; it failed: the old one runs on
    fn on_change(&mut self) -> Result<(), String> {
        if self.build() {
            self.stop();
            self.install()?;
            self.start()?;
        } else if self.child.is_some() {
            println!("watch: the running version stays (the error is in its inspector)");
        }
        Ok(())
    }
}

pub(crate) fn run(args: &[String]) -> Result<(), String> {
    let (ours, theirs) = match args.iter().position(|a| a == "--") {
        Some(i) => (&args[..i], args[i + 1..].to_vec()),
        None => (args, Vec::new()),
    };
    let o = crate::opts(ours)?;
    if o.release || o.target.is_some() || o.modes.is_some() || o.serve.is_some() || !o.free.is_empty() {
        return Err("watch takes -p PKG and -- ARGS for the app (a development build for this machine)".to_string());
    }
    let triple = if cfg!(windows) { WIN } else if cfg!(target_os = "linux") { LINUX } else {
        return Err("watch runs the app on Windows or Linux".to_string());
    };
    let p = cargo::project(o.package.as_deref())?;
    let dir = p.target_dir.join("jflib-watch").join(&p.package.name);
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let dirs = local_dirs(&p)?;
    let mut files = scan(&dirs, &p.target_dir);
    interrupt::catch();
    let mut s = Session { p: &p, triple, dir, args: theirs, exe: None, run_exe: None, child: None, replay_seen: None };
    println!("watch: {} [{triple}], {} files, session {}; Ctrl+C ends", p.package.name, files.len(), s.dir.display());
    s.put_aside("old"); // a new session: no replay of an earlier one
    if s.build() {
        s.install()?;
        s.start()?;
    }
    while !STOP.load(Ordering::SeqCst) {
        sleep(POLL);
        s.check()?;
        if let Some(r) = s.replay_report() {
            println!("watch: start: {r}");
        }
        let mut now = scan(&dirs, &p.target_dir);
        if now == files {
            continue;
        }
        // until the saving is over
        loop {
            sleep(QUIET);
            let more = scan(&dirs, &p.target_dir);
            if more == now {
                break;
            }
            now = more;
        }
        let changed: Vec<String> = now.iter().filter(|(f, v)| files.get(*f) != Some(*v)).map(|(f, _)| f).chain(files.keys().filter(|f| !now.contains_key(*f)))
            .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().to_string())).collect();
        files = now;
        let mut shown = changed.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
        if changed.len() > 6 {
            shown += " ...";
        }
        println!("watch: changed {shown}");
        s.on_change()?;
    }
    println!("watch: ending");
    s.stop();
    Ok(())
}
