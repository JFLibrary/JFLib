// "04 Dateien & Zwischenablage": the system's picker (open / save as / folder: Android's
// Storage Access Framework, content:// names), a note kept in config:// across restarts, a
// start counter in data://, copy and paste through the system clipboard.
// With jfgui_SELFTEST=phase1 / phase2 (the start intent's extras of
// android/tests/GalleryFilesInstrumentation.java) the app checks its files, the documents the
// picker hands out and the clipboard, and writes data://selftest.txt ("ok NAME" / "FAIL NAME:
// detail" lines, "done PASSED/TOTAL" last).

use alloc::{format, string::String, vec::Vec};
use ui::*;

const NOTE: &str = "config://note.txt";
const STARTS: &str = "data://starts.txt";
const RESULTS: &str = "data://selftest.txt";
const ID_FD: u32 = 0x4000;

pub struct Files {
    loaded: bool,
    note: String,
    saved: String,
    starts: u32,
    open: Shared<i32>,
    mode: u32,
    path: String,
    chosen: String,
    info: String,
    test: Option<SelfTest>,
}

impl Default for Files {
    fn default() -> Self {
        Self {
            loaded: false, note: String::new(), saved: String::new(), starts: 0, open: Shared::new(0),
            mode: FD_OPEN, path: String::new(), chosen: String::new(), info: String::new(), test: None,
        }
    }
}

fn text_of(path: &str) -> Option<String> {
    plat::read_file(path).and_then(|b| String::from_utf8(b).ok())
}

// "name · 123 Bytes" of a file or folder
fn describe(path: &str) -> String {
    let name = plat::display_name(path);
    match plat::vfs::stat(path) {
        Some(s) if s.kind == 2 => {
            let mut n = 0;
            plat::dir_list(path, &mut |_, _, _| n += 1);
            format!("{name} · Ordner, {n} Einträge")
        }
        Some(s) => format!("{name} · {} Bytes", s.size),
        None => name,
    }
}

impl Files {
    // the note and the start counter, once (not in headless script runs: no files of the user)
    fn load(&mut self, ui: &mut Ui) {
        self.loaded = true;
        if let Some(t) = plat::env("jfgui_SELFTEST").filter(|t| t.starts_with("phase")) {
            self.test = Some(SelfTest::new(&t));
        }
        if ui.headless() {
            return;
        }
        self.note = text_of(NOTE).unwrap_or_default();
        self.saved = self.note.clone();
        self.starts = text_of(STARTS).and_then(|t| t.trim().parse().ok()).unwrap_or(0) + 1;
        plat::write_file(STARTS, format!("{}\n", self.starts).as_bytes());
    }

    pub fn card(&mut self, ui: &mut Ui) {
        if !self.loaded {
            self.load(ui);
        }
        if let Some(t) = &mut self.test {
            t.step(ui);
        }
        ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 10);
        ui.text(TS_SUBTITLE, "04  Dateien & Zwischenablage");
        ui.box_begin(0, BOX_H | BOX_WRAP, 8);
        for (id, label, mode) in [(400, "Öffnen…", FD_OPEN), (401, "Speichern…", FD_SAVE), (402, "Ordner…", FD_FOLDER)] {
            ui.next_size(0, 44);
            if ui.button(id, label) & EV_ACT != 0 {
                self.mode = mode;
                self.path = if mode == FD_SAVE { String::from("Notiz.txt") } else { String::new() };
                self.open.set(1);
            }
        }
        ui.box_end();
        if ui.filedlg(ID_FD, self.mode, &self.open, &mut self.path, 4096, &["txt=Text"]) & EV_ACT != 0 {
            self.chosen = self.path.clone();
            self.info = match self.mode {
                FD_OPEN => match text_of(&self.chosen) {
                    Some(t) => {
                        self.note = t.chars().take(400).collect();
                        format!("Geöffnet: {}", describe(&self.chosen))
                    }
                    None => format!("Nicht lesbar: {}", plat::display_name(&self.chosen)),
                },
                FD_SAVE if plat::write_file(&self.chosen, self.note.as_bytes()) => format!("Gespeichert: {}", describe(&self.chosen)),
                FD_SAVE => format!("Nicht gespeichert: {}", plat::display_name(&self.chosen)),
                _ => format!("Ordner: {}", describe(&self.chosen)),
            };
        }
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, if self.info.is_empty() { "Noch keine Datei gewählt." } else { &self.info });
        ui.text(TS_BODY, "Notiz (bleibt nach dem Neustart):");
        ui.next_size(SIZE_FILL, 44);
        ui.textbox(410, Some("Etwas notieren"), &mut self.note, 400);
        if self.note != self.saved && !ui.headless() && plat::write_file(NOTE, self.note.as_bytes()) {
            self.saved = self.note.clone();
        }
        ui.box_begin(0, BOX_H, 8);
        ui.next_size(0, 44);
        if ui.button(420, "Kopieren") & EV_ACT != 0 {
            ui.clip_set(CLIP_TEXT, self.note.as_bytes());
        }
        ui.next_size(0, 44);
        if ui.button(421, "Einfügen") & EV_ACT != 0 {
            if let Some(t) = ui.clip_get(CLIP_TEXT).and_then(|b| String::from_utf8(b).ok()) {
                self.note = t.chars().take(400).collect();
            }
        }
        ui.box_end();
        if self.starts > 0 {
            ui.text(TS_CAPTION | TX_SECONDARY, &format!("App-Starts: {}", self.starts));
        }
        if let Some(t) = &self.test {
            ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, &t.status());
        }
        ui.box_end();
    }
}

// ---------------------------------------------------------------- the self-test

struct SelfTest {
    phase2: bool,
    step: u32,
    log: Vec<(bool, String)>,
    open: Shared<i32>,
    picking: bool,
    path: String,
    tree: String,
    doc: String,
    saved: String,
    waits: u32,
    done: bool,
}

const TREE_FILES: &str = "hello.txt";
const HELLO: &str = "Hallo vom Host\n";

impl SelfTest {
    fn new(phase: &str) -> Self {
        Self {
            phase2: phase == "phase2", step: 0, log: Vec::new(), open: Shared::new(0), picking: false, path: String::new(),
            tree: String::new(), doc: String::new(), saved: String::new(), waits: 0, done: false,
        }
    }

    fn status(&self) -> String {
        let bad = self.log.iter().filter(|l| !l.0).count();
        format!("Selbsttest: {} Prüfungen, {} Fehler{}", self.log.len(), bad, if self.done { ", fertig" } else { " …" })
    }

    fn check(&mut self, ok: bool, name: &str, detail: impl FnOnce() -> String) {
        let line = if ok { format!("ok {name}") } else { format!("FAIL {name}: {}", detail()) };
        plat::write_stdout(format!("SELFTEST {line}\n").as_bytes());
        self.log.push((ok, if ok { String::from(name) } else { line }));
    }

    fn eq(&mut self, name: &str, got: Option<String>, want: &str) {
        let ok = got.as_deref() == Some(want);
        self.check(ok, name, || format!("{got:?} != {want:?}"));
    }

    fn finish(&mut self) {
        self.done = true;
        let mut out = String::new();
        for (ok, l) in &self.log {
            out.push_str(&if *ok { format!("ok {l}\n") } else { format!("{l}\n") });
        }
        let passed = self.log.iter().filter(|l| l.0).count();
        out.push_str(&format!("done {passed}/{}\n", self.log.len()));
        plat::write_file(RESULTS, out.as_bytes());
        plat::write_stdout(format!("SELFTEST done {passed}/{}\n", self.log.len()).as_bytes());
    }

    // one picker through the UI's file dialog -> the chosen name once the user is done
    fn pick(&mut self, ui: &mut Ui, flags: u32, start: &str) -> Option<Option<String>> {
        if !self.picking {
            self.picking = true;
            self.path = String::from(start);
            self.open.set(1);
        }
        let r = ui.filedlg(ID_FD + 0x100, flags, &self.open, &mut self.path, 4096, &["txt=Text"]);
        if r & EV_ACT != 0 {
            self.picking = false;
            return Some(Some(core::mem::take(&mut self.path)));
        }
        if self.open.get() == 0 {
            self.picking = false;
            return Some(None);
        }
        None
    }

    // Android lets only the focused app read the clipboard (after the picker the focus comes
    // back a moment after the answer): a clip readable, else again in 100 ms (at most 5 s)
    fn clip_ready(&mut self, ui: &mut Ui) -> bool {
        if plat::clip_has(CLIP_TEXT) || plat::clip_has(CLIP_IMAGE) || self.waits >= 50 {
            self.waits = 0;
            return true;
        }
        self.waits += 1;
        ui.timer(0x5E1F, 100);
        false
    }

    fn step(&mut self, ui: &mut Ui) {
        if self.done {
            return;
        }
        if self.phase2 {
            self.phase2_checks(ui);
            self.finish();
            return;
        }
        match self.step {
            0 => {
                if plat::env("jfgui_SELFTEST_CLIP").is_some() && !self.clip_ready(ui) {
                    return;
                }
                self.clipboard(ui);
                self.storage();
                self.step = 1;
            }
            // the folder the instrumentation chooses (Documents/jfgui-test)
            1 => {
                let start = plat::env("jfgui_SELFTEST_DIR").unwrap_or_default();
                if let Some(r) = self.pick(ui, FD_FOLDER, &start) {
                    self.check(r.as_deref().is_some_and(|p| p.starts_with("content://")), "picker: folder", || format!("{r:?}"));
                    self.tree = r.unwrap_or_default();
                    if !self.tree.is_empty() {
                        self.tree_checks();
                    }
                    self.step = 2;
                }
            }
            // a document to open (hello.txt; the picker starts beside it)
            2 => {
                let start = if self.tree.is_empty() { String::new() } else { format!("{}/{TREE_FILES}", self.tree) };
                if let Some(r) = self.pick(ui, FD_OPEN, &start) {
                    self.check(r.is_some(), "picker: open", || String::from("cancelled"));
                    self.doc = r.unwrap_or_default();
                    if !self.doc.is_empty() {
                        self.doc_checks();
                    }
                    self.step = 3;
                }
            }
            // save as gespeichert.txt in the chosen folder
            3 => {
                let start = if self.tree.is_empty() { String::from("gespeichert.txt") } else { format!("{}/gespeichert.txt", self.tree) };
                if let Some(r) = self.pick(ui, FD_SAVE, &start) {
                    self.check(r.is_some(), "picker: save as", || String::from("cancelled"));
                    self.saved = r.unwrap_or_default();
                    if !self.saved.is_empty() {
                        self.save_checks();
                    }
                    self.step = 4;
                }
            }
            4 => {
                let uris = format!("{}\n{}\n{}\n", self.tree, self.doc, self.saved);
                self.check(plat::write_file("config://selftest/uris.txt", uris.as_bytes()), "uris kept", String::new);
                self.grants(&[self.tree.clone(), self.doc.clone(), self.saved.clone()]);
                self.step = 5;
            }
            _ => {
                if !self.clip_ready(ui) {
                    return;
                }
                // the clip the instrumentation checks last: an image (GalleryFiles)
                let mut sf = gfx::Surface::new(3, 2, gfx::FMT_ARGB);
                for p in sf.px.chunks_exact_mut(4) {
                    p.copy_from_slice(&[0xC0, 0x80, 0x20, 0xFF]); // B, G, R, A
                }
                self.check(ui.clip_set_image(&sf) == 0, "clipboard: image set", String::new);
                let back = ui.clip_get_image();
                self.check(back.as_ref().is_ok_and(|s| s.w == 3 && s.h == 2), "clipboard: image back", || format!("{:?}", back.as_ref().err()));
                self.finish();
            }
        }
    }

    fn clipboard(&mut self, ui: &mut Ui) {
        // the instrumentation's text, from the system's clipboard
        if let Some(want) = plat::env("jfgui_SELFTEST_CLIP") {
            let got = ui.clip_get(CLIP_TEXT).and_then(|b| String::from_utf8(b).ok());
            self.eq("clipboard: system text in", got, &want);
            self.check(ui.clip_get(CLIP_RTF).is_none(), "clipboard: no rich form of another app", String::new);
        }
        let text = "jfgui → Android ✓";
        self.check(ui.clip_set(CLIP_TEXT, text.as_bytes()) == 0, "clipboard: text set", String::new);
        let got = ui.clip_get(CLIP_TEXT).and_then(|b| String::from_utf8(b).ok());
        self.eq("clipboard: text round trip", got, text);
        let rich: [(u32, &[u8]); 3] = [(CLIP_TEXT, b"fett"), (CLIP_HTML, b"<b>fett</b>"), (CLIP_RTF, b"{\\rtf1 {\\b fett}}")];
        self.check(ui.clip_set_many(&rich) == 0, "clipboard: rich set", String::new);
        let html = ui.clip_get(CLIP_HTML).and_then(|b| String::from_utf8(b).ok());
        self.eq("clipboard: html", html, "<b>fett</b>");
        let rtf = ui.clip_get(CLIP_RTF).and_then(|b| String::from_utf8(b).ok());
        self.eq("clipboard: own rtf", rtf, "{\\rtf1 {\\b fett}}");
        let t = ui.clip_get(CLIP_TEXT).and_then(|b| String::from_utf8(b).ok());
        self.eq("clipboard: rich text form", t, "fett");
    }

    fn storage(&mut self) {
        let token = format!("{:x}", plat::time_us());
        // config:// data:// cache:// on internal storage
        self.check(plat::write_file("config://selftest/a.txt", b"eins"), "write config", String::new);
        self.eq("read config", text_of("config://selftest/a.txt"), "eins");
        self.check(plat::write_file("config://selftest/a.txt", b"zwei!"), "replace config", String::new);
        self.eq("read replaced", text_of("config://selftest/a.txt"), "zwei!");
        let mut names = Vec::new();
        plat::dir_list("config://selftest", &mut |n, d, s| names.push((String::from(n), d, s)));
        self.check(names.iter().any(|(n, d, s)| n == "a.txt" && !d && *s == 5), "list with size", || format!("{names:?}"));
        self.check(!names.iter().any(|(n, _, _)| n.contains(".tmp-")), "no siblings left", || format!("{names:?}"));
        self.check(plat::rename("config://selftest/a.txt", "config://selftest/b.txt"), "rename", String::new);
        self.check(plat::file_exists("config://selftest/a.txt") == 0 && plat::file_exists("config://selftest/b.txt") == 1, "renamed", String::new);
        self.check(plat::mkdir("data://selftest/tief/er"), "mkdir", String::new);
        self.check(plat::file_exists("data://selftest/tief/er") == 2, "folder exists", String::new);
        self.check(!plat::delete("data://selftest/tief"), "non-empty folder stays", String::new);
        self.check(plat::delete("data://selftest/tief/er") && plat::delete("data://selftest/tief"), "delete folders", String::new);
        // streams
        let f = plat::File::open("data://selftest/strom.bin", plat::FO_WRITE);
        let ok = f.as_ref().is_some_and(|f| f.write_at(0, b"abc") == Some(3) && f.write_at(5, b"xy") == Some(2) && f.size() == Some(7) && f.sync());
        drop(f);
        self.check(ok, "stream write", String::new);
        let f = plat::File::open("data://selftest/strom.bin", plat::FO_READ);
        let mut buf = [9u8; 8];
        let n = f.as_ref().and_then(|f| f.read_at(0, &mut buf));
        self.check(n == Some(7) && &buf[..7] == b"abc\0\0xy", "stream read", || format!("{n:?} {buf:?}"));
        drop(f);
        // AtomicFile: nothing without commit, everything with it
        if let Some(a) = plat::AtomicFile::create("data://selftest/atom.txt") {
            a.write_at(0, b"halb");
        }
        self.check(plat::file_exists("data://selftest/atom.txt") == 0, "atomic: dropped", String::new);
        let ok = plat::AtomicFile::create("data://selftest/atom.txt").is_some_and(|a| a.write_at(0, b"ganz") == Some(4) && a.commit());
        self.check(ok, "atomic: commit", String::new);
        self.eq("atomic: read", text_of("data://selftest/atom.txt"), "ganz");
        let l1 = plat::FileLock::acquire("data://selftest/sperre");
        let l2 = plat::FileLock::acquire("data://selftest/sperre");
        self.check(l1.is_some() && l2.is_none(), "file lock", String::new);
        drop(l1);
        self.check(plat::FileLock::acquire("data://selftest/sperre").is_some(), "file lock again", String::new);
        let t = plat::File::open_temp(None);
        self.check(t.is_some_and(|t| t.write_at(0, b"tmp") == Some(3)), "temp file", String::new);
        self.check(plat::write_file("cache://selftest/c.txt", b"cache"), "write cache", String::new);
        self.check(plat::write_file("/tmp/selftest.txt", b"weg"), "write /tmp", String::new);
        self.check(plat::write_file("config://selftest/persist.txt", token.as_bytes())
            && plat::write_file("data://selftest/persist.txt", token.as_bytes()), "write persist token", String::new);
        #[cfg(target_os = "android")]
        {
            let os = plat::vfs::os_path("data://selftest/persist.txt").unwrap_or_default();
            let id = plat::web::persist(&os);
            let st = id.and_then(plat::web::request_status);
            self.check(st == Some(plat::web::RequestStatus::Durable), "persist durable", || format!("{os} {st:?}"));
        }
    }

    // below the folder the user chose (content://AUTH/tree/ID + names)
    fn tree_checks(&mut self) {
        let t = self.tree.clone();
        let at = |n: &str| format!("{t}/{n}");
        self.check(!plat::display_name(&t).is_empty() && !plat::display_name(&t).contains('%'), "tree: display name", || plat::display_name(&t));
        self.check(plat::file_exists(&t) == 2, "tree: is a folder", String::new);
        self.eq("tree: read", text_of(&at(TREE_FILES)), HELLO);
        self.check(plat::vfs::stat(&at(TREE_FILES)).is_some_and(|s| s.kind == 1 && s.size == HELLO.len() as u64), "tree: stat", String::new);
        self.check(plat::write_file(&at("neu/datei.txt"), b"neuer Inhalt"), "tree: write new (with folder)", String::new);
        self.eq("tree: read new", text_of(&at("neu/datei.txt")), "neuer Inhalt");
        self.check(plat::write_file(&at("neu/datei.txt"), b"kurz"), "tree: replace", String::new);
        self.eq("tree: replaced (truncated)", text_of(&at("neu/datei.txt")), "kurz");
        let mut names = Vec::new();
        plat::dir_list(&at("neu"), &mut |n, d, s| names.push((String::from(n), d, s)));
        self.check(names == [(String::from("datei.txt"), false, 4)], "tree: list", || format!("{names:?}"));
        self.eq("tree: file display name", Some(plat::display_name(&at("neu/datei.txt"))), "datei.txt");
        // AtomicFile writes in place on content://
        let ok = plat::AtomicFile::create(&at("neu/atom.txt")).is_some_and(|a| a.write_at(0, b"atomar") == Some(6) && a.commit());
        self.check(ok, "tree: atomic commit", String::new);
        self.eq("tree: atomic read", text_of(&at("neu/atom.txt")), "atomar");
        // a stream on a document: read, write, grow
        let f = plat::File::open(&at("neu/datei.txt"), plat::FO_RW);
        let ok = f.as_ref().is_some_and(|f| f.write_at(4, b"+mehr") == Some(5) && f.size() == Some(9) && f.sync());
        drop(f);
        self.check(ok, "tree: stream rw", String::new);
        self.eq("tree: stream result", text_of(&at("neu/datei.txt")), "kurz+mehr");
        // rename inside a folder, over an existing file, to another folder
        self.check(plat::rename(&at("neu/datei.txt"), &at("neu/umbenannt.txt")), "tree: rename", String::new);
        self.check(plat::file_exists(&at("neu/datei.txt")) == 0, "tree: old name gone", String::new);
        self.check(plat::rename(&at("neu/umbenannt.txt"), &at("neu/atom.txt")), "tree: rename over existing", String::new);
        self.eq("tree: replaced by rename", text_of(&at("neu/atom.txt")), "kurz+mehr");
        self.check(plat::rename(&at("neu/atom.txt"), &at("verschoben.txt")), "tree: move to other folder", String::new);
        self.eq("tree: moved", text_of(&at("verschoben.txt")), "kurz+mehr");
        self.check(plat::mkdir(&at("a/b/c")) && plat::file_exists(&at("a/b/c")) == 2, "tree: mkdir nested", String::new);
        self.check(!plat::delete(&at("a/b")), "tree: non-empty folder stays", String::new);
        self.check(plat::delete(&at("a/b/c")) && plat::delete(&at("a/b")) && plat::delete(&at("a")), "tree: delete folders", String::new);
        self.check(plat::delete(&at("verschoben.txt")) && plat::delete(&at("neu")), "tree: delete file and folder", String::new);
        self.check(!plat::delete(&t), "tree: the chosen folder stays", String::new);
        self.check(plat::file_exists(&at("fehlt.txt")) == 0 && plat::read_file(&at("fehlt.txt")).is_none(), "tree: missing", String::new);
    }

    // one document from ACTION_OPEN_DOCUMENT
    fn doc_checks(&mut self) {
        let d = self.doc.clone();
        self.eq("open: display name", Some(plat::display_name(&d)), TREE_FILES);
        self.eq("open: read", text_of(&d), HELLO);
        let f = plat::File::open(&d, plat::FO_READ);
        let mut buf = [0u8; 5];
        self.check(f.is_some_and(|f| f.read_at(6, &mut buf) == Some(5) && &buf == b"vom H"), "open: stream at offset", String::new);
    }

    // a document from ACTION_CREATE_DOCUMENT
    fn save_checks(&mut self) {
        let d = self.saved.clone();
        self.check(plat::display_name(&d).starts_with("gespeichert"), "save: display name", || plat::display_name(&d));
        self.check(plat::write_file(&d, b"gespeichert ueber SAF"), "save: write", String::new);
        self.eq("save: read", text_of(&d), "gespeichert ueber SAF");
        let ok = plat::AtomicFile::create(&d).is_some_and(|a| a.write_at(0, b"atomar ersetzt") == Some(14) && a.commit());
        self.check(ok, "save: atomic (in place)", String::new);
        self.eq("save: read after atomic", text_of(&d), "atomar ersetzt");
        self.check(plat::vfs::stat(&d).is_some_and(|s| s.size == 14), "save: stat size", String::new);
    }

    fn grants(&mut self, uris: &[String]) {
        #[cfg(target_os = "android")]
        {
            let g = plat::web::content_grants();
            for u in uris.iter().filter(|u| !u.is_empty()) {
                self.check(g.iter().any(|(x, r, w)| x == u && *r && *w), "grant kept", || format!("{u} not in {g:?}"));
            }
        }
        #[cfg(not(target_os = "android"))]
        let _ = uris;
    }

    // after a restart: the files and the grants are still there
    fn phase2_checks(&mut self, ui: &mut Ui) {
        let a = text_of("config://selftest/persist.txt");
        let b = text_of("data://selftest/persist.txt");
        self.check(a.is_some() && a == b, "restart: config and data kept", || format!("{a:?} {b:?}"));
        self.eq("restart: renamed file kept", text_of("config://selftest/b.txt"), "zwei!");
        self.eq("restart: atomic file kept", text_of("data://selftest/atom.txt"), "ganz");
        self.eq("restart: cache kept", text_of("cache://selftest/c.txt"), "cache");
        self.check(plat::file_exists("/tmp/selftest.txt") == 0, "restart: /tmp emptied", String::new);
        let starts = text_of(STARTS).and_then(|t| t.trim().parse::<u32>().ok()).unwrap_or(0);
        self.check(starts >= 2, "restart: start counter", || format!("{starts}"));
        let uris = text_of("config://selftest/uris.txt").unwrap_or_default();
        let u: Vec<String> = uris.lines().map(String::from).collect();
        self.check(u.len() == 3 && u.iter().all(|x| x.starts_with("content://")), "restart: uris", || uris.clone());
        if u.len() == 3 {
            self.eq("restart: tree still readable", text_of(&format!("{}/{TREE_FILES}", u[0])), HELLO);
            self.eq("restart: opened document readable", text_of(&u[1]), HELLO);
            self.eq("restart: saved document readable", text_of(&u[2]), "atomar ersetzt");
            self.grants(&u);
            #[cfg(target_os = "android")]
            {
                self.check(plat::web::content_release(&u[2]), "release grant", String::new);
                self.check(!plat::web::content_grants().iter().any(|(x, _, _)| *x == u[2]), "grant released", String::new);
            }
        }
        let _ = ui;
    }
}
