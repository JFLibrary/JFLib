// inspect_src -- the live inspector's sources: the file and line where the app called the
// selected widget, copied or opened in an editor (ROADMAP §11 "Hot-Reload und beschreibbarer
// Inspektor", stage 4; GUIDE.md §30)
//
// Only in development builds: build_rs.py passes --cfg jfgui_dev there; a --release build has
// inspect_src_off.rs (no #[track_caller], empty scopes that fold away, no source shown).
// Nothing of the app changes for it.  The library's widget calls carry
// #[cfg_attr(jfgui_dev, track_caller)] and open a scope as their first statement
// (`let _src = self.src_scope();`).  While the inspector is open the scope sets the call being
// built (core::panic::Location::caller(): through every call marked so, the app's own line):
//   * a call from outside the ui crate (the app, another library) is that call;
//   * a call from inside the ui crate (a composite widget's inner button or text box, a menu's
//     rows, a library dialog's buttons) keeps the call that encloses it, so a composite widget
//     reports the app's call and not its inner one; with none enclosing (the runtime's own
//     chrome, the menu bar of the AppDesc, the inspector's window) the library's call counts.
// The scope restores the enclosing call when it ends.  ui.sem stores the call being built with
// each semantic record (none: its own caller, the app's for a custom widget); a layout box
// takes the one of its box_begin.  Closed (the default) a scope tests one flag and nothing is
// stored; the frames stay those of an app without it (`inspect expect same` / `idle`).
//
// The Widgets tab shows "Quelle": the file as rustc got it (build_rs.py makes it relative to
// the repository, --remap-path-prefix) and the line, a copy button and "Im Editor öffnen".  The
// editor, the first that is set or found:
//   jfgui_EDITOR    a command line with {file}, {line} and {col} ("code -g {file}:{line}");
//                   without {file} the file is appended; "..." or '...' keep blanks in a word
//   VISUAL, EDITOR  a program with its words; a known editor gets the line (line_args):
//                   code / codium -g FILE:LINE, JetBrains IDEs --line N FILE, vi / vim / nano /
//                   emacs / gedit ... +N FILE, kate -l N FILE, subl FILE:LINE, notepad++ -nN
//                   FILE, others FILE
//   found           CLion (clion64.exe in PATH, the Toolbox's and the installer's folders;
//                   Linux: clion / clion.sh in PATH, the Toolbox's script, the snap), then VS
//                   Code (code.cmd in PATH -> Code.exe beside it, its install folders; Linux:
//                   code in PATH)
// The file is opened by its absolute path: below jfgui_SRC, else below the folder that holds the
// program's build folder (ROOT/build/<target>/NAME), else below the working folder.
// Starting the editor is the app starting a program (plat::process::launch): the app's manifest
// decides as for Command::spawn -- a `process` rule names the editor, an `ask process` line
// asks, an app without a manifest may.  A refusal is said in the inspector with the program's
// name, the source stays to be copied; a development build makes no exception for the
// inspector (it runs in the app's process with the app's rights).  A headless run (a script)
// never starts the user's editor: only one the script set with `inspect editor`.
//
// Hooks (inspect.rs, ctx.rs, layout boxes): src_scope (the widget calls), src_sem (ui.sem),
// src_cur (box_begin), src_of / src_clear / src_count (the tree, frame_begin, closing),
// isrc_props (tab "Widgets", below the selector), isrc_script / isrc_expect (script commands).
//
// Script commands (auto.rs `inspect ...`):
//   inspect editor "COMMAND"|none   the editor of this run (sets jfgui_EDITOR; a script's
//                                   opening takes no other editor)
//   inspect open [dry]              "Im Editor öffnen" for the selected widget (dry: the command
//                                   built and the permission checked, nothing started)
//   inspect expect source "PREFIX"|none   the selected widget's source ("apps/x/main.rs:12")
//                                   starts with PREFIX / is unknown
//   inspect expect sources "PREFIX" every record of the app's windows: its source starts with
//                                   PREFIX (no widget call of the library left unmarked)
//   inspect expect opened "TEXT"|none     the last opening's command line contains TEXT /
//                                   nothing was opened since the last check
//   inspect expect note "TEXT"|none       the note below the source contains TEXT / is empty

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::panic::Location;

use super::auto::{parse_qstring, skip_ws, word_eq};
use super::consts::*;
use super::inspect::Src;
use super::keys::*;
use super::{App, Shared, Ui};
use crate::i18n::{fill, tr};

/// the inspector shows sources (a development build)
pub(crate) const SRC: bool = true;

// widget ids inside the inspector's window (inspect.rs: 0x07FE_00..; hotreload.rs: 0x07FE_0030)
const ID_COPY: u32 = 0x07FE_0040;
const ID_OPEN: u32 = 0x07FE_0041;
const KEY_W: i32 = 140; // the property names (inspect.rs)

/// the sources of a pass (ui.m.isrc)
#[derive(Default)]
pub(crate) struct S {
    /// the call being built (src_scope), shared with the open scopes that restore it
    cur: Shared<Option<Src>>,
    /// the semantic records of this pass with their source: (record index, call)
    recs: Vec<(u32, Src)>,
    /// `inspect editor`: the script set the editor (a headless run takes no other)
    script_editor: bool,
    /// the last opening's command line (started or dry); taken by `inspect expect opened`
    opened: String,
    /// what the last copy or opening did, below the source; the source it was for (another
    /// selection clears it)
    note: String,
    note_of: Option<Src>,
}

/// a widget call's scope (src_scope): the enclosing call comes back when it ends
pub(crate) struct SrcScope(Option<(Shared<Option<Src>>, Option<Src>)>);

impl Drop for SrcScope {
    fn drop(&mut self) {
        if let Some((cur, prev)) = &self.0 {
            cur.set(*prev);
        }
    }
}

// the call comes from the ui crate itself (its files share this file's folder as rustc got it)
fn in_ui(at: Src) -> bool {
    let here = file!();
    at.file().starts_with(&here[..here.len() - "inspect_src.rs".len()])
}

/// a source as the inspector shows it: "apps/x/main.rs:12" ('/' separators)
pub(crate) fn src_text(at: Src) -> String {
    format!("{}:{}", at.file().replace('\\', "/"), at.line())
}

// the folder the relative source paths start in: jfgui_SRC, else the folder holding the
// program's build folder (ROOT/build/<target>/NAME, ROOT/build/release/<target>/NAME), else the
// working folder
fn src_root() -> String {
    if let Some(r) = plat::env("jfgui_SRC").filter(|r| !r.trim().is_empty()) {
        return r;
    }
    exe_root().unwrap_or_else(plat::cwd)
}

#[cfg(any(windows, target_os = "linux"))]
fn exe_root() -> Option<String> {
    let exe = plat::current_exe()?;
    let k = exe.replace('\\', "/").rfind("/build/")?;
    Some(String::from(&exe[..k]))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn exe_root() -> Option<String> {
    None
}

// the absolute path of a source file in the separators of this system
fn src_path(at: Src) -> String {
    let (sep, other) = if cfg!(windows) { ('\\', '/') } else { ('/', '\\') };
    let f = at.file().replace(other, &format!("{}", sep));
    let b = f.as_bytes();
    if f.starts_with(sep) || (b.len() > 2 && b[1] == b':') {
        return f; // (rustc got an absolute path)
    }
    let root = src_root().replace(other, &format!("{}", sep));
    format!("{}{}{}", root.trim_end_matches(sep), sep, f)
}

// the words of a command line: blanks separate them, "..." and '...' keep blanks (nothing is
// escaped: Windows paths keep their backslashes)
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut w = String::new();
    let (mut quote, mut any) = (None, false);
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => w.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                any = true;
            }
            None if c == ' ' || c == '\t' => {
                if any || !w.is_empty() {
                    out.push(core::mem::take(&mut w));
                }
                any = false;
            }
            None => w.push(c),
        }
    }
    if any || !w.is_empty() {
        out.push(w);
    }
    out
}

// a program's name in lower case without its folder and .exe / .cmd / .bat / .sh
fn base_name(p: &str) -> String {
    let n = p.rsplit(['/', '\\']).next().unwrap_or(p).to_ascii_lowercase();
    for ext in [".exe", ".cmd", ".bat", ".sh"] {
        if let Some(s) = n.strip_suffix(ext) {
            return String::from(s);
        }
    }
    n
}

// how a known editor (VISUAL / EDITOR) takes the file at a line
fn line_args(base: &str, file: &str, line: u32) -> Vec<String> {
    const JETBRAINS: [&str; 10] = ["clion", "idea", "rustrover", "pycharm", "webstorm", "goland", "phpstorm", "rider", "fleet", "studio"];
    const PLUS: [&str; 14] =
        ["vi", "vim", "nvim", "gvim", "nano", "emacs", "emacsclient", "micro", "gedit", "gnome-text-editor", "mousepad", "xed", "pluma", "joe"];
    let (f, n) = (String::from(file), format!("{}", line));
    match base {
        "code" | "code-insiders" | "codium" | "vscodium" | "cursor" => alloc::vec![String::from("-g"), format!("{}:{}", file, line)],
        "kate" | "kwrite" => alloc::vec![String::from("-l"), n, f],
        "subl" | "sublime_text" => alloc::vec![format!("{}:{}", file, line)],
        "notepad++" => alloc::vec![format!("-n{}", line), f],
        b if JETBRAINS.iter().any(|j| b.starts_with(j)) => alloc::vec![String::from("--line"), n, f],
        b if PLUS.contains(&b) => alloc::vec![format!("+{}", line), f],
        _ => alloc::vec![f],
    }
}

// a command line as the inspector shows it: words with blanks in "..."
fn cmd_text(prog: &str, args: &[String]) -> String {
    let mut s = String::new();
    for w in core::iter::once(prog).chain(args.iter().map(|a| a.as_str())) {
        if !s.is_empty() {
            s.push(' ');
        }
        if w.is_empty() || w.contains([' ', '\t']) {
            s.push('"');
            s.push_str(w);
            s.push('"');
        } else {
            s.push_str(w);
        }
    }
    s
}

// the editors found on this system: CLion, then VS Code (see the module header)
#[cfg(windows)]
fn found_editor(file: &str, line: u32) -> Option<(String, Vec<String>)> {
    let exists = |p: &String| plat::file_exists(p) == 1;
    let env = |n: &str| plat::env(n).filter(|v| !v.is_empty());
    // CLion: in PATH, the Toolbox's folder, the installer's ProgramFiles\JetBrains\CLion
    // <version> (the newest first)
    let clion = plat::process::find("clion64.exe").or_else(|| {
        let mut clion: Vec<String> = Vec::new();
        if let Some(d) = env("LOCALAPPDATA") {
            clion.push(format!("{}\\Programs\\CLion\\bin\\clion64.exe", d));
        }
        if let Some(pf) = env("ProgramFiles") {
            let mut dirs: Vec<String> = Vec::new();
            let jb = format!("{}\\JetBrains", pf);
            plat::dir_list(&jb, &mut |name: &str, is_dir: bool, _: u32| {
                if is_dir && name.starts_with("CLion") {
                    dirs.push(String::from(name));
                }
            });
            dirs.sort_unstable_by(|a, b| b.cmp(a));
            clion.extend(dirs.iter().map(|d| format!("{}\\{}\\bin\\clion64.exe", jb, d)));
        }
        clion.into_iter().find(exists)
    });
    if let Some(p) = clion {
        return Some((p, alloc::vec![String::from("--line"), format!("{}", line), String::from(file)]));
    }
    // VS Code: bin\code.cmd in PATH starts Code.exe beside bin (a batch file is not started)
    let mut code: Vec<String> = Vec::new();
    if let Some(c) = plat::process::find("code.cmd") {
        if let Some((bin, _)) = c.rsplit_once('\\') {
            if let Some((dir, _)) = bin.rsplit_once('\\') {
                code.push(format!("{}\\Code.exe", dir));
            }
        }
    }
    if let Some(d) = env("LOCALAPPDATA") {
        code.push(format!("{}\\Programs\\Microsoft VS Code\\Code.exe", d));
    }
    if let Some(pf) = env("ProgramFiles") {
        code.push(format!("{}\\Microsoft VS Code\\Code.exe", pf));
    }
    let p = code.into_iter().find(exists)?;
    Some((p, alloc::vec![String::from("-g"), format!("{}:{}", file, line)]))
}

#[cfg(target_os = "linux")]
fn found_editor(file: &str, line: u32) -> Option<(String, Vec<String>)> {
    let mut clion: Vec<String> = Vec::new();
    if let Some(h) = plat::env("HOME").filter(|h| !h.is_empty()) {
        clion.push(format!("{}/.local/share/JetBrains/Toolbox/scripts/clion", h));
    }
    clion.push(String::from("/snap/bin/clion"));
    let clion = plat::process::find("clion")
        .or_else(|| plat::process::find("clion.sh"))
        .or_else(|| clion.into_iter().find(|p| plat::file_exists(p) == 1));
    if let Some(p) = clion {
        return Some((p, alloc::vec![String::from("--line"), format!("{}", line), String::from(file)]));
    }
    let p = plat::process::find("code")?;
    Some((p, alloc::vec![String::from("-g"), format!("{}:{}", file, line)]))
}

// the browser and Android start no programs
#[cfg(not(any(windows, target_os = "linux")))]
fn found_editor(_file: &str, _line: u32) -> Option<(String, Vec<String>)> {
    None
}

// start the editor on its own (plat::process::launch; the browser and Android: not supported)
#[cfg(any(windows, target_os = "linux"))]
fn start(prog: &str, args: &[&str]) -> Result<(), u32> {
    plat::process::launch(prog, args)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn start(_prog: &str, _args: &[&str]) -> Result<(), u32> {
    Err(50)
}

impl Ui {
    // ------------------------------------------------------------------ recording

    /// (every widget call of the library, its first statement) while the inspector is open:
    /// this call becomes the one being built -- unless it comes from inside the ui crate while
    /// another encloses it; the enclosing one comes back when the scope ends
    #[track_caller]
    #[inline]
    pub(crate) fn src_scope(&self) -> SrcScope {
        if !self.m.ins.on {
            return SrcScope(None);
        }
        self.src_enter(Location::caller())
    }

    fn src_enter(&self, at: Src) -> SrcScope {
        let cur = &self.m.isrc.cur;
        let prev = cur.get();
        if prev.is_some() && in_ui(at) {
            return SrcScope(None); // the library inside the app's call: the app's
        }
        cur.set(Some(at));
        SrcScope(Some((cur.clone(), prev)))
    }

    /// (ui.sem, while the inspector is open) record k's source: the call being built, else
    /// the caller of ui.sem
    #[track_caller]
    #[inline]
    pub(crate) fn src_sem(&mut self, k: usize) {
        if self.m.ins.on {
            let at = match self.m.isrc.cur.get() {
                Some(c) => c,
                None => Location::caller(),
            };
            self.m.isrc.recs.push((k as u32, at));
        }
    }

    /// (a layout box begins, while the inspector is open) the call being built
    pub(crate) fn src_cur(&self) -> Option<Src> {
        self.m.isrc.cur.get()
    }

    /// the source of semantic record k of this pass
    pub(crate) fn src_of(&self, k: usize) -> Option<Src> {
        let r = &self.m.isrc.recs;
        r.binary_search_by_key(&(k as u32), |e| e.0).ok().map(|i| r[i].1)
    }

    /// (frame_begin, the inspector closed) the records' sources go
    pub(crate) fn src_clear(&mut self) {
        if !self.m.isrc.recs.is_empty() {
            self.m.isrc.recs.clear();
        }
    }

    /// the sources stored (`inspect expect idle`: none while closed)
    pub(crate) fn src_count(&self) -> usize {
        self.m.isrc.recs.len()
    }

    // ------------------------------------------------------------------ the tool window

    /// (inspect.rs, tab "Widgets", below the selector) the selected widget's source: copy,
    /// "Im Editor öffnen", the note of the last of them
    pub(crate) fn isrc_props(&mut self, src: Option<Src>) {
        if self.m.isrc.note_of != src {
            self.m.isrc.note.clear(); // (the note of another selection)
            self.m.isrc.note_of = src;
        }
        self.box_begin(0, BOX_H | BOX_XCENTER, 12);
        self.next_size(KEY_W, 0);
        self.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, tr("Quelle"));
        self.next_size(-1, 0);
        match src {
            None => self.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, tr("unbekannt")),
            Some(at) => {
                self.text(TS_BODY | TX_ELLIPSIS, &src_text(at));
                self.next_flags(ST_COMPACT);
                if self.icon_button(ID_COPY, IC_COPY, tr("Quelle kopieren")) & EV_ACT != 0 {
                    self.src_copy(at);
                }
            }
        }
        self.box_end();
        if let Some(at) = src {
            self.box_begin(0, BOX_H | BOX_XCENTER, 12);
            self.spacer(KEY_W);
            self.next_flags(ST_COMPACT);
            self.next_icon(IC_OPEN_FILE);
            if self.button(ID_OPEN, tr("Im Editor öffnen")) & EV_ACT != 0 {
                self.src_open(at, false);
            }
            self.box_end();
        }
        if !self.m.isrc.note.is_empty() {
            let n = self.m.isrc.note.clone();
            self.next_size(-1, 0);
            self.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &n);
        }
    }

    // the source to the clipboard ("apps/x/main.rs:12")
    fn src_copy(&mut self, at: Src) {
        let s = src_text(at);
        self.clip_set(CLIP_TEXT, s.as_bytes());
        self.src_note(at, String::from(tr("Kopiert")));
    }

    // the note below the source at
    fn src_note(&mut self, at: Src, note: String) {
        self.m.isrc.note = note;
        self.m.isrc.note_of = Some(at);
    }

    // the editor's command for file at line (see the module header) -> program, its words
    fn src_editor(&self, file: &str, line: u32, col: u32) -> Option<(String, Vec<String>)> {
        let script = self.headless; // a script: only the editor it set
        if !script || self.m.isrc.script_editor {
            if let Some(t) = plat::env("jfgui_EDITOR").filter(|t| !t.trim().is_empty()) {
                let (n, c) = (format!("{}", line), format!("{}", col));
                let w = words(&t);
                let has = w.iter().any(|x| x.contains("{file}"));
                let mut w: Vec<String> = w.iter().map(|x| x.replace("{file}", file).replace("{line}", &n).replace("{col}", &c)).collect();
                if !has {
                    w.push(String::from(file));
                }
                let prog = w.remove(0);
                return Some((prog, w));
            }
        }
        if script {
            return None;
        }
        for var in ["VISUAL", "EDITOR"] {
            let mut w = words(&plat::env(var).unwrap_or_default());
            if w.is_empty() {
                continue;
            }
            let prog = w.remove(0);
            w.extend(line_args(&base_name(&prog), file, line));
            return Some((prog, w));
        }
        found_editor(file, line)
    }

    /// "Im Editor öffnen": the editor at the source (dry: the command built and the permission
    /// checked, nothing started) -> it was opened; the note says what happened
    pub(crate) fn src_open(&mut self, at: Src, dry: bool) -> bool {
        let path = src_path(at);
        let Some((prog, args)) = self.src_editor(&path, at.line(), at.column()) else {
            self.src_note(at, String::from(tr("Kein Editor gefunden: jfgui_EDITOR, VISUAL oder EDITOR setzen")));
            return false;
        };
        let shown = cmd_text(&prog, &args);
        // the app's manifest decides (PERMISSIONS: process), as for every program it starts
        if !plat::policy::process_allowed(&prog) {
            // an `ask process` line: the question waits for the user (perm.rs shows it)
            let asked = plat::policy::questions().iter().any(|q| q.kind == "process" && q.target == prog);
            let t = if asked {
                tr("Die App fragt, ob sie {} starten darf: nach der Antwort noch einmal öffnen.")
            } else {
                tr("Die App darf {} nicht starten (PERMISSIONS: process). Quelle kopieren und selbst öffnen.")
            };
            self.src_note(at, fill(t, &[prog.as_str()]));
            return false;
        }
        if !dry {
            let a: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            if let Err(e) = start(&prog, &a) {
                let t = tr("Start fehlgeschlagen: {} (Fehler {})");
                self.src_note(at, fill(t, &[prog.as_str(), &format!("{}", e)]));
                return false;
            }
        }
        self.src_note(at, fill(tr("Geöffnet mit {}"), &[shown.as_str()]));
        self.m.isrc.opened = shown;
        true
    }

    // ------------------------------------------------------------------ script commands

    /// `inspect editor "COMMAND"|none`, `inspect open [dry]` -> false: not one of them
    pub(crate) fn isrc_script(&mut self, app: &mut dyn App, a: &[u8]) -> bool {
        if let Some(r) = word_eq(a, "editor") {
            let r = skip_ws(r);
            let v = if word_eq(r, "none").is_some() {
                None
            } else {
                match parse_qstring(r, 1024) {
                    Some((v, _)) => Some(String::from_utf8_lossy(&v).into_owned()),
                    None => {
                        self.fail(b"inspect editor \"COMMAND\"|none");
                        return true;
                    }
                }
            };
            plat::setenv("jfgui_EDITOR", v.as_deref());
            self.m.isrc.script_editor = true;
            return true;
        }
        let Some(r) = word_eq(a, "open") else { return false };
        if !self.m.ins.on {
            self.fail(b"the inspector is closed");
            return true;
        }
        let Some(at) = self.m.ins.sel_src else {
            self.fail(b"no source: nothing selected, or the selection has none");
            return true;
        };
        if !self.src_open(at, word_eq(skip_ws(r), "dry").is_some()) {
            let n = format!("not opened: {}", self.m.isrc.note);
            self.fail(n.as_bytes());
        }
        self.invalidate();
        self.run_passes(app);
        true
    }

    /// `inspect expect source|sources|opened|note ...` -> false: not one of them
    pub(crate) fn isrc_expect(&mut self, a: &[u8]) -> bool {
        if let Some(r) = word_eq(a, "sources") {
            self.isrc_expect_all(skip_ws(r));
            return true;
        }
        let (what, r) = if let Some(r) = word_eq(a, "source") {
            (0, skip_ws(r))
        } else if let Some(r) = word_eq(a, "opened") {
            (1, skip_ws(r))
        } else if let Some(r) = word_eq(a, "note") {
            (2, skip_ws(r))
        } else {
            return false;
        };
        let have = match what {
            0 => self.m.ins.sel_src.map(src_text).unwrap_or_default(),
            1 => core::mem::take(&mut self.m.isrc.opened),
            _ => self.m.isrc.note.clone(),
        };
        if word_eq(r, "none").is_some() {
            if !have.is_empty() {
                self.fail(format!("{:?}", have).as_bytes());
            }
            return true;
        }
        let Some((want, _)) = parse_qstring(r, 1024) else {
            self.fail(b"expected a quoted string");
            return true;
        };
        let want = String::from_utf8_lossy(&want);
        let ok = if what == 0 { have.starts_with(want.as_ref()) } else { have.contains(want.as_ref()) };
        if !ok {
            self.fail(format!("{:?}", have).as_bytes());
        }
        true
    }

    // `inspect expect sources "PREFIX"`: every record of the app's windows in the last pass has
    // a source that starts with PREFIX (a widget call of the library left unmarked shows a line
    // of lib/ui instead)
    fn isrc_expect_all(&mut self, r: &[u8]) {
        let Some((want, _)) = parse_qstring(r, 1024) else {
            self.fail(b"expected a quoted string");
            return;
        };
        if !self.m.ins.on {
            self.fail(b"the inspector is closed");
            return;
        }
        let want = String::from_utf8_lossy(&want).into_owned();
        let iw = self.uiw_index(super::inspect::INSPECT_WIN);
        let mut bad: Vec<String> = Vec::new();
        for (k, s) in self.sems.iter().enumerate() {
            let have = self.src_of(k).map(src_text).unwrap_or_default();
            if s.win as i32 != iw && !have.starts_with(want.as_str()) {
                let kind = KIND_NAMES[if s.kind < K_KIND_COUNT { s.kind as usize } else { 0 }];
                bad.push(format!("{} #{} {:?}: {}", kind, s.id, s.label.as_deref().unwrap_or(""), have));
            }
        }
        if !bad.is_empty() {
            let n = bad.len();
            bad.truncate(6);
            self.fail(format!("{} records elsewhere: {}", n, bad.join("; ")).as_bytes());
        }
    }
}
