// jfguiLib Build Manager: native WinUI workspace, profiles, queue and release packages.
#![no_std]
#![no_main]
#![deny(unsafe_code)]
extern crate alloc;

mod client;
mod process_checks;
use alloc::{collections::BTreeMap, format, string::String, vec, vec::Vec};
use agent_core::{json, Value};
use ui::*;
use client::Client;

const DESC: AppDesc = AppDesc { title: "jfguiLib Build Manager", flags: AF_RESIZABLE | AF_CUSTOM_TITLEBAR,
    w: 1320, h: 900, min_w: 1000, min_h: 720, ..AppDesc::NONE };
const PACKAGE_KEYS: [&str; 11] = ["display", "version", "publisher", "summary", "description", "generic", "categories", "deb_name", "default_win", "default_linux", "styles"];
const PACKAGE_LABELS: [&str; 11] = ["Anzeigename", "Version", "Herausgeber", "Kurzbeschreibung", "Beschreibung", "Programmtyp", "Kategorien", "Debian-Paketname", "Windows-Standard", "Linux-Standard", "Paket-Styles"];
const TARGETS: [&str; 3] = ["win", "linux", "web"];
const TARGET_LABELS: [&str; 3] = ["Windows", "Linux", "Web"];
fn s<'a>(v: &'a Value, key: &str) -> &'a str { v.get(key).and_then(Value::as_str).unwrap_or("") }
fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] { v.get(key).and_then(Value::as_array).unwrap_or(&[]) }
fn list(values: impl Iterator<Item=String>) -> Value { Value::Array(values.map(Value::string).collect()) }
fn object() -> Value { Value::object([]) }
fn number(v: &Value, key: &str) -> u64 {
    match v.get(key) { Some(Value::Number(n)) => n.as_str().parse::<f64>().unwrap_or(0.0).max(0.0) as u64, _ => 0 }
}
fn status(value: &str) -> &str { match value { "queued" => "Wartend", "running" => "Läuft", "succeeded" => "Erfolgreich", "failed" => "Fehlgeschlagen", "cancelled" => "Abgebrochen", "interrupted" => "Unterbrochen", _ => "Noch kein Build" } }
fn test_status(value: &str) -> &str { match value { "passed" => "Bestanden", "failed" => "Fehlgeschlagen", "skipped" => "Ausstehend", "none" => "Keine App-Tests vorhanden", "not-applicable" => "Nicht anwendbar", _ => value } }
fn compact(value: &str, cap: usize) -> String {
    let mut end = value.len().min(cap); while !value.is_char_boundary(end) { end -= 1; }
    String::from(&value[..end])
}
fn find_workspace(start: &str) -> Option<String> {
    let mut path = start.replace('\\', "/");
    loop {
        if plat::file_exists(&format!("{path}/build_rs.py")) == 1 { return Some(path); }
        if plat::file_exists(&format!("{path}/RustLib/build_rs.py")) == 1 { return Some(format!("{path}/RustLib")); }
        if let Some((parent, _)) = path.rsplit_once('/') { path = parent.into(); } else { return None; }
        if path.is_empty() { return None; }
    }
}
fn edit(ui: &mut Ui, id: u32, label: &str, value: &mut String) {
    ui.text(TS_CAPTION | TX_SECONDARY, label);
    ui.next_size(-1, 32); ui.textbox(id, None, value, 8192);
}

struct Manager {
    client: Option<Client>, fixture: bool, local: base::ini::Ini, config: String,
    workspace: String, python: String, distro: String, search: String, message: String,
    catalog: Value, apps: Vec<Value>, selected: Vec<i32>, active: usize,
    styles: Vec<Value>, style_flags: Vec<i32>, targets: [i32; 3], view: i32,
    optimization: i32, parallel: i32, tests: i32, output: String,
    profiles: Value, profile_revision: String, profile_name: String, profile_index: i32,
    start_name: String, start_index: i32, theme: i32, scale: i32, window_size: String, arguments: String,
    start_drafts: BTreeMap<String, Value>,
    package_fields: Vec<String>, package_revision: String,
    jobs: Vec<Value>, job_index: i32, artifact_index: i32, log: String, bottom: i32,
    prepared: Value, launch_pending: Option<Value>, preview: String, preview_open: Shared<i32>, close_open: Shared<i32>, settings: Shared<i32>,
    file_open: Shared<i32>, selected_file: String,
}
impl Manager {
    fn new() -> Self {
        let config = plat::arg_value("--config").unwrap_or_else(|| format!("{}/jfguiLib/BuildManager.ini", plat::config_dir().unwrap_or_else(|| String::from("."))));
        let local = base::ini::Ini::load(&config);
        let workspace = plat::arg_value("--workspace").or_else(|| local.get("host", "workspace").and_then(find_workspace))
            .or_else(|| find_workspace(&plat::cwd())).or_else(|| plat::args().first().and_then(|p| find_workspace(p))).unwrap_or_else(plat::cwd);
        let python = local.get("host", "python").unwrap_or("").into();
        let distro = local.get("host", "wsl").unwrap_or("Ubuntu-24.04").into();
        Self { client: None, fixture: false, local, config, workspace, python, distro, search: String::new(), message: String::new(),
            catalog: object(), apps: Vec::new(), selected: Vec::new(), active: 0, styles: Vec::new(), style_flags: Vec::new(),
            targets: [if cfg!(windows) {1} else {0}, if cfg!(windows) {0} else {1}, 0], view: 0,
            optimization: 0, parallel: plat::cpu_count().clamp(1, 256) as i32, tests: 0, output: "build/manager".into(),
            profiles: object(), profile_revision: String::new(), profile_name: "Mein Buildprofil".into(), profile_index: 0,
            start_name: "Standard".into(), start_index: 0, theme: 0, scale: 100, window_size: String::new(), arguments: String::new(),
            start_drafts: BTreeMap::new(),
            package_fields: vec![String::new(); 11], package_revision: String::new(), jobs: Vec::new(), job_index: -1,
            artifact_index: 0, log: String::new(), bottom: 0, prepared: object(), launch_pending: None, preview: String::new(),
            preview_open: Shared::new(0), close_open: Shared::new(0), settings: Shared::new(0), file_open: Shared::new(0), selected_file: String::new() }
    }
    fn send(&mut self, method: &str, value: Value) {
        if let Some(c) = &mut self.client { c.send(method, value); }
        else { self.message = "Offline-Vorschau: keine Projektdateien oder Builds werden verändert.".into(); }
    }
    fn busy(&self) -> bool { self.jobs.iter().any(|j| matches!(s(j, "status"), "queued" | "running")) }
    fn connect(&mut self) {
        if self.busy() { self.message = "Vor dem Projektwechsel laufende Aufträge beenden.".into(); return; }
        self.workspace = self.workspace.replace('\\', "/").trim_end_matches('/').into();
        if plat::file_exists(&format!("{}/RustLib/build_rs.py", self.workspace)) == 1 { self.workspace.push_str("/RustLib"); }
        if plat::file_exists(&format!("{}/build_rs.py", self.workspace)) != 1 {
            self.message = "Bitte den RustLib-Projektordner auswählen.".into(); self.settings.set(1); return;
        }
        self.client = None;
        match Client::start(self.workspace.clone(), self.python.clone(), self.distro.clone()) {
            Ok(client) => { self.client = Some(client); self.message = "Apps und Werkzeuge werden erkannt …".into(); self.send("catalog", object()); }
            Err(e) => self.message = e,
        }
    }
    fn load_catalog(&mut self, data: Value) {
        let old: Vec<String> = self.apps.iter().zip(&self.selected).filter(|(_, flag)| **flag != 0).map(|(app, _)| s(app, "id").into()).collect();
        self.apps = array(&data, "apps").to_vec();
        self.selected = self.apps.iter().map(|a| i32::from(old.iter().any(|id| id == s(a, "id")))).collect();
        self.styles = array(&data, "styles").to_vec(); self.style_flags = vec![0; self.styles.len()];
        self.profiles = data.get("profiles").cloned().unwrap_or_else(object);
        self.profile_revision = s(&data, "profile_revision").into();
        self.jobs = array(&data, "jobs").to_vec();
        if !self.jobs.is_empty() { self.job_index = self.jobs.len() as i32 - 1; }
        self.active = self.active.min(self.apps.len().saturating_sub(1));
        self.catalog = data; self.load_package(); self.message.clear();
    }
    fn load_package(&mut self) {
        let app = self.apps.get(self.active).cloned().unwrap_or_else(object);
        let package = app.get("package").cloned().unwrap_or_else(object);
        let values = package.get("values").cloned().unwrap_or_else(object);
        self.package_fields = PACKAGE_KEYS.iter().map(|k| s(&values, k).into()).collect();
        self.package_revision = s(&package, "revision").into();
    }
    fn select_app(&mut self, index: usize) {
        if let Some(app) = self.apps.get(self.active) {
            let draft = Value::object([("name", Value::string(self.start_name.clone())), ("argv", self.argv())]);
            self.start_drafts.insert(s(app, "id").into(), draft);
        }
        self.active = index; self.load_package(); self.start_index = 0;
        let id = self.apps.get(index).map(|a| s(a, "id")).unwrap_or("");
        let draft = self.start_drafts.get(id).cloned().or_else(|| array(&self.profiles, "starts").iter().find(|p| s(p, "app") == id).cloned())
            .unwrap_or_else(|| Value::object([("name", Value::string("Standard")), ("argv", Value::Array(Vec::new()))]));
        self.apply_start(&draft);
    }
    fn argv_for(&self, app: &str) -> Value {
        if self.apps.get(self.active).is_some_and(|a| s(a, "id") == app) { return self.argv(); }
        self.start_drafts.get(app).or_else(|| array(&self.profiles, "starts").iter().find(|p| s(p, "app") == app))
            .and_then(|p| p.get("argv")).cloned().unwrap_or_else(|| Value::Array(Vec::new()))
    }
    fn build_profile(&self) -> Value {
        Value::object([("name", Value::string(self.profile_name.clone())),
            ("apps", list(self.apps.iter().zip(&self.selected).filter(|(_, f)| **f != 0).map(|(a, _)| s(a, "id").into()))),
            ("styles", list(self.styles.iter().zip(&self.style_flags).filter(|(_, f)| **f != 0).map(|(a, _)| s(a, "id").into()))),
            ("targets", list(TARGETS.iter().zip(self.targets).filter(|(_, f)| *f != 0).map(|(t, _)| String::from(*t)))),
            ("mode", Value::string(if self.view == 1 { "release" } else { "development" })),
            ("optimization", Value::string(["size", "release", "o2"][self.optimization as usize])),
            ("jobs", Value::i64(self.parallel as i64)), ("tests", Value::Bool(self.tests != 0)),
            ("output", Value::string(self.output.clone()))])
    }
    fn apply_profile(&mut self, p: &Value) {
        self.profile_name = s(p, "name").into();
        self.view = i32::from(s(p, "mode") == "release");
        self.optimization = ["size", "release", "o2"].iter().position(|v| *v == s(p, "optimization")).unwrap_or(0) as i32;
        self.parallel = p.get("jobs").and_then(Value::as_i64).unwrap_or(1).clamp(1, 256) as i32;
        self.tests = i32::from(p.get("tests").and_then(Value::as_bool).unwrap_or(false));
        self.output = s(p, "output").into();
        for (i, a) in self.apps.iter().enumerate() { self.selected[i] = i32::from(array(p, "apps").iter().any(|v| v.as_str() == Some(s(a, "id")))); }
        for (i, a) in self.styles.iter().enumerate() { self.style_flags[i] = i32::from(array(p, "styles").iter().any(|v| v.as_str() == Some(s(a, "id")))); }
        for (i, target) in TARGETS.iter().enumerate() { self.targets[i] = i32::from(array(p, "targets").iter().any(|v| v.as_str() == Some(*target))); }
    }
    fn argv(&self) -> Value {
        let mut args = Vec::new();
        if self.theme > 0 { args.extend([Value::string("--theme"), Value::string(if self.theme == 1 { "light" } else { "dark" })]); }
        if self.scale != 100 { args.extend([Value::string("--scale"), Value::string(format!("{}", self.scale))]); }
        if !self.window_size.is_empty() { args.extend([Value::string("--size"), Value::string(self.window_size.clone())]); }
        args.extend(self.arguments.lines().filter(|v| !v.is_empty()).map(Value::string)); Value::Array(args)
    }
    fn apply_start(&mut self, p: &Value) {
        self.start_name = s(p, "name").into(); self.theme = 0; self.scale = 100; self.window_size.clear(); self.arguments.clear();
        let args = array(p, "argv"); let mut i = 0;
        while i < args.len() {
            let arg = args[i].as_str().unwrap_or("");
            if i + 1 < args.len() && matches!(arg, "--theme" | "--scale" | "--size") {
                let v = args[i + 1].as_str().unwrap_or("");
                match arg { "--theme" => self.theme = if v == "dark" {2} else {1}, "--scale" => self.scale = v.parse().unwrap_or(100), _ => self.window_size = v.into() }
                i += 2;
            } else { if !self.arguments.is_empty() { self.arguments.push('\n'); } self.arguments.push_str(arg); i += 1; }
        }
    }
    fn update_job(&mut self, mut job: Value) {
        job.insert("seen_us", Value::u64(plat::time_us()));
        if let Some(i) = self.jobs.iter().position(|j| s(j, "id") == s(&job, "id")) { self.jobs[i] = job; }
        else { self.jobs.push(job); self.job_index = self.jobs.len() as i32 - 1; self.log.clear(); self.artifact_index = 0; }
    }
    fn poll(&mut self) {
        let messages = self.client.as_ref().map(Client::drain).unwrap_or_default();
        for msg in messages {
            if let Some(error) = msg.get("error").and_then(Value::as_str) {
                if error.starts_with("Build-Backend beendet") {
                    for j in &mut self.jobs { if matches!(s(j, "status"), "running" | "queued") { j.insert("status", Value::string("interrupted")); } }
                    if !self.message.is_empty() && !self.message.starts_with("Apps und Werkzeuge") { self.message.push_str(" Backend beendet."); }
                    else { self.message = compact(error, 450); }
                } else {
                    self.message = compact(error, 450);
                }
                continue;
            }
            if let Some(result) = msg.get("result") {
                match s(&msg, "method") {
                    "catalog" => self.load_catalog(result.clone()),
                    "save_profile" => { self.profiles = result.get("profiles").cloned().unwrap_or_else(object); self.profile_revision = s(result, "revision").into(); self.message = "Profil gespeichert.".into(); },
                    "save_package" => {
                        if let Some(index) = self.apps.iter().position(|a| s(a, "id") == s(result, "app")) {
                            self.apps[index].insert("package", result.clone());
                            if index == self.active { self.load_package(); }
                        }
                        self.message = "Paketdaten gespeichert.".into();
                    },
                    "preview" => {
                        self.launch_pending = None;
                        if let Some(profile) = result.get("profile") { self.prepared = profile.clone(); }
                        self.preview.clear();
                        for job in array(result, "jobs") {
                            if let Some(spec) = job.get("spec") { self.preview.push_str(&format!("{} · {}\n", s(spec, "app"), s(spec, "mode"))); }
                            for cmd in array(job, "commands") {
                                for arg in cmd.as_array().unwrap_or(&[]) { self.preview.push_str(&json::stringify(arg).unwrap_or_default()); self.preview.push(' '); }
                                self.preview.push('\n');
                            }
                            self.preview.push('\n');
                        }
                        self.preview_open.set(1); self.message.clear();
                    },
                    "job" => { self.log = s(result, "log").into(); self.update_job(result.clone()); },
                    "enqueue" | "retry" => self.message = "Aufträge wurden eingereiht.".into(),
                    _ => {},
                }
            }
            match s(&msg, "event") {
                "job" => { if let Some(j) = msg.get("job") { self.update_job(j.clone()); } },
                "log" => {
                    if self.jobs.get(self.job_index as usize).is_some_and(|j| s(j, "id") == s(&msg, "job_id")) {
                        self.log.push_str(s(&msg, "text"));
                        if self.log.len() > 65536 { let mut cut = self.log.len() - 65536; while !self.log.is_char_boundary(cut) { cut += 1; } self.log.drain(..cut); }
                    }
                },
                "phase" => { if let Some(j) = self.jobs.iter_mut().find(|j| s(j, "id") == s(&msg, "job_id")) { j.insert("phase", Value::string(s(&msg, "phase"))); } },
                "test" => {
                    if let Some(j) = self.jobs.iter_mut().find(|j| s(j, "id") == s(&msg, "job_id")) {
                        let mut tests = array(j, "tests").to_vec(); tests.push(msg.clone()); j.insert("tests", Value::Array(tests));
                    }
                },
                _ => {},
            }
        }
    }
    fn save_local(&mut self, ui: &Ui) {
        if self.fixture { return; }
        self.local.set("host", "workspace", &self.workspace); self.local.set("host", "python", &self.python); self.local.set("host", "wsl", &self.distro);
        let (w, h) = ui.client_size(); self.local.set_int("window", "width", w as i64); self.local.set_int("window", "height", h as i64);
        self.local.set_int("window", "dark", i64::from(ui.theme_dark()));
        let temporary = format!("{}.{}.tmp", self.config, plat::time_us());
        if self.local.save(&temporary) {
            if !plat::rename(&temporary, &self.config) { plat::delete(&temporary); }
        }
    }
}

impl App for Manager {
    fn init(&mut self, ui: &mut Ui) -> bool {
        if let Some(path) = plat::arg_value("--fixture") {
            self.fixture = true;
            match plat::read_file(&path).and_then(|b| json::parse(&b).ok()) {
                Some(data) => self.load_catalog(data), None => self.message = "Vorschaudaten konnten nicht geladen werden.".into(),
            }
        } else { self.connect(); }
        if plat::arg_value("--theme").is_none() { ui.theme_set(self.local.get_int("window", "dark", 2) as u32); }
        ui.timer(2600, 500);
        true
    }
    fn frame(&mut self, ui: &mut Ui) {
        self.poll();
        let root_rect = ui.box_rect();
        ui.next_size(ui.px2dip(root_rect[2]), ui.px2dip(root_rect[3]));
        ui.box_begin(95, BOX_V | BOX_FIXED, 0);
        ui.titlebar_begin(DESC.title, 0);
        let title_height = ui.box_rect()[3];
        ui.titlebar_end();
        ui.next_size(-1, ui.px2dip(root_rect[3] - title_height));
        ui.box_begin(100, BOX_V | BOX_FIXED | pad(20), 12);
        ui.next_size(-1, 54);
        ui.box_begin(101, BOX_H | BOX_FIXED | BOX_XCENTER, 12);
        ui.next_size(-1, 0); ui.box_begin(102, BOX_V, 2);
        ui.text(TS_TITLE, "Build Manager");
        ui.text(TS_CAPTION | TX_SECONDARY, "jfguiLib · Apps, Builds und Releases"); ui.box_end();
        if ui.button(110, "Hell / Dunkel") & EV_ACT != 0 { ui.theme_set(u32::from(!ui.theme_dark())); }
        if ui.button(111, "Projekt & Werkzeuge") & EV_ACT != 0 { self.settings.set(1); }
        if ui.button(112, "Aktualisieren") & EV_ACT != 0 { self.send("catalog", object()); }
        ui.box_end();
        if !self.message.is_empty() {
            ui.next_size(-1, 0);
            if ui.infobar(120, 0, "Build Manager", &self.message) & EV_ACT != 0 { self.message.clear(); }
        }
        ui.next_size(-1, -1); ui.box_begin(130, BOX_H | BOX_FIXED, 16);
        self.sidebar(ui);
        ui.next_size(-1, -1); ui.box_begin(140, BOX_V | BOX_FIXED | BOX_CARD | pad(16), 12);
        ui.tabs(2000, &mut self.view, &["Entwicklung", "Release", "Starten", "Paketdaten"]);
        ui.next_size(-1, -1); ui.scroll_begin(145, 0);
        ui.box_begin(146, BOX_V | pad(4), 8);
        match self.view { 0 | 1 => self.build_view(ui), 2 => self.start_view(ui), _ => self.package_view(ui) }
        ui.box_end(); ui.scroll_end(); ui.box_end(); ui.box_end();
        self.jobs_view(ui);
        ui.box_end();
        ui.box_end();
        self.dialogs(ui);
    }
    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
        if id == UIC_WAKE { self.poll(); return true; }
        if id == UIC_TIMER { ui.timer(2600, 500); return true; }
        if id == UIC_WINCLOSE && self.busy() { self.close_open.set(1); return true; }
        if id == UIC_EXIT { self.save_local(ui); self.client = None; }
        false
    }
}

impl Manager {
    fn sidebar(&mut self, ui: &mut Ui) {
        ui.next_size(264, -1); ui.box_begin(150, BOX_V | BOX_FIXED | BOX_CARD | pad(12), 8);
        ui.text(TS_SUBTITLE, "Apps");
        ui.next_size(-1, 32); ui.textbox(151, Some("Apps suchen"), &mut self.search, 256);
        ui.box_begin(152, BOX_H, 8);
        if ui.button(153, "Alle") & EV_ACT != 0 { self.selected.fill(1); }
        if ui.button(154, "Keine") & EV_ACT != 0 { self.selected.fill(0); }
        ui.box_end();
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("{} von {} ausgewählt", self.selected.iter().filter(|n| **n != 0).count(), self.apps.len()));
        ui.next_size(-1, -1); ui.scroll_begin(155, 0); ui.box_begin(156, BOX_V, 6);
        let filter = self.search.to_lowercase(); let mut activate = None;
        for (i, app) in self.apps.iter().enumerate() {
            if !s(app, "name").to_lowercase().contains(&filter) && !s(app, "id").to_lowercase().contains(&filter) { continue; }
            ui.box_begin(3000 + i as u32 * 8, BOX_V, 2);
            ui.box_begin(3001 + i as u32 * 8, BOX_H | BOX_XCENTER, 4);
            ui.checkbox(3002 + i as u32 * 8, "", &mut self.selected[i]);
            ui.next_size(-1, 32);
            let ev = if i == self.active { ui.accent_button(3003 + i as u32 * 8, s(app, "name")) } else { ui.button(3003 + i as u32 * 8, s(app, "name")) };
            if ev & EV_ACT != 0 { activate = Some(i); }
            ui.box_end();
            let targets = array(app, "targets").iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" · ");
            let version = app.get("package").and_then(|p| p.get("values")).map(|p| s(p, "version")).unwrap_or("");
            let last = self.jobs.iter().rev().find(|j| s(j, "app") == s(app, "id") && s(j, "mode") != "run");
            ui.text(TS_CAPTION | TX_SECONDARY, &format!("{} {} · {}", targets, version, status(last.map(|j| s(j, "status")).unwrap_or(""))));
            ui.box_end();
        }
        if let Some(i) = activate { self.select_app(i); }
        ui.box_end(); ui.scroll_end(); ui.box_end();
    }
    fn build_view(&mut self, ui: &mut Ui) {
        ui.text(TS_SUBTITLE, if self.view == 0 { "Entwicklungs-Build" } else { "Geprüftes Release" });
        ui.text(TS_CAPTION | TX_SECONDARY, if self.view == 0 { "Inkrementell bauen und Ergebnisse direkt starten." } else { "Varianten bauen → App-Tests → Installer und Pakete." });
        let profiles = array(&self.profiles, "builds").to_vec();
        let mut labels = vec!["Aktuelle Auswahl"]; labels.extend(profiles.iter().map(|p| s(p, "name")));
        ui.box_begin(2100, BOX_H | BOX_XCENTER, 10);
        ui.next_size(-1, 32);
        if ui.combobox(2101, &mut self.profile_index, &labels) & EV_ACT != 0 && self.profile_index > 0 {
            if let Some(p) = profiles.get(self.profile_index as usize - 1) { self.apply_profile(p); }
        }
        ui.next_size(190, 32); ui.textbox(2102, Some("Profilname"), &mut self.profile_name, 128);
        if ui.button(2103, "Profil speichern") & EV_ACT != 0 { self.send("save_profile", Value::object([("kind", Value::string("builds")), ("profile", self.build_profile()), ("revision", Value::string(self.profile_revision.clone()))])); }
        ui.box_end();
        ui.text(TS_BODY_STRONG, "Zielplattformen");
        ui.box_begin(2110, BOX_H, 20);
        for i in 0..3 {
            let unavailable = self.catalog.get("capabilities").and_then(|c| c.get("targets")).and_then(|c| c.get(TARGETS[i]))
                .is_some_and(|c| c.get(if self.view == 1 { "package" } else { "build" }).and_then(Value::as_bool) == Some(false));
            if unavailable { ui.next_flags(ST_DISABLED); }
            ui.checkbox(2111 + i as u32, TARGET_LABELS[i], &mut self.targets[i]);
        }
        ui.box_end();
        if let Some(caps) = self.catalog.get("capabilities").and_then(|c| c.get("targets")) {
            for target in TARGETS {
                if let Some(cap) = caps.get(target) {
                    let reason = if self.view == 1 && !s(cap, "package_reason").is_empty() { s(cap, "package_reason") } else { s(cap, "reason") };
                    if !reason.is_empty() { ui.text(TS_CAPTION | TX_SECONDARY, reason); }
                }
            }
        }
        ui.text(TS_BODY_STRONG, "Styles");
        ui.box_begin(2120, BOX_H, 14);
        for (i, style) in self.styles.iter().enumerate() { ui.checkbox(2121 + i as u32, s(style, "id"), &mut self.style_flags[i]); }
        ui.box_end();
        ui.text(TS_CAPTION | TX_SECONDARY, "Ohne Auswahl: App-Standard; bei Release die Paket-Styles.");
        ui.box_begin(2140, BOX_H | BOX_XCENTER, 12);
        ui.text(TS_BODY, "Compiler-Prozesse"); ui.next_size(90, 32);
        ui.number_box(2141, &mut self.parallel, &Range::new(1, 256, 1, 0));
        if self.view == 1 {
            ui.next_size(-1, 32); ui.combobox(2142, &mut self.optimization, &["Kleine Pakete (size)", "Release mit LTO", "Optimiert ohne LTO (o2)"]);
        } else { ui.checkbox(2143, "App-Tests ausführen", &mut self.tests); }
        ui.box_end();
        edit(ui, 2150, "Ausgabebasis im Projekt", &mut self.output);
        ui.box_begin(2160, BOX_H | BOX_XCENTER, 10);
        if ui.accent_button(2161, "Aufträge vorbereiten") & EV_ACT != 0 {
            self.prepared = self.build_profile(); self.send("preview", Value::object([("profile", self.prepared.clone())]));
        }
        ui.text(TS_CAPTION | TX_SECONDARY, "Einstellungen und Aufrufe vor dem Start prüfen."); ui.box_end();
    }
    fn start_view(&mut self, ui: &mut Ui) {
        let app = self.apps.get(self.active).map(|a| String::from(s(a, "id"))).unwrap_or_default();
        ui.text(TS_SUBTITLE, &format!("Startprofil · {app}"));
        let profiles: Vec<Value> = array(&self.profiles, "starts").iter().filter(|p| s(p, "app") == app).cloned().collect();
        let mut labels = vec!["Aktuelle Argumente"]; labels.extend(profiles.iter().map(|p| s(p, "name")));
        ui.next_size(-1, 32);
        if ui.combobox(2200, &mut self.start_index, &labels) & EV_ACT != 0 && self.start_index > 0 {
            if let Some(p) = profiles.get(self.start_index as usize - 1) { self.apply_start(p); }
        }
        ui.box_begin(2201, BOX_H | BOX_XCENTER, 12);
        ui.next_size(-1, 32); ui.textbox(2202, Some("Profilname"), &mut self.start_name, 128);
        if ui.button(2203, "Startprofil speichern") & EV_ACT != 0 {
            let profile = Value::object([("name", Value::string(self.start_name.clone())), ("app", Value::string(app)), ("argv", self.argv())]);
            self.send("save_profile", Value::object([("kind", Value::string("starts")), ("profile", profile), ("revision", Value::string(self.profile_revision.clone()))]));
        }
        ui.box_end();
        ui.box_begin(2210, BOX_H | BOX_XCENTER, 12);
        ui.next_size(-1, 32); ui.combobox(2211, &mut self.theme, &["Theme der App", "Hell", "Dunkel"]);
        ui.text(TS_BODY, "Skalierung %"); ui.next_size(90, 32); ui.number_box(2212, &mut self.scale, &Range::new(50, 400, 25, 0)); ui.box_end();
        edit(ui, 2220, "Fenstergröße, z. B. 1280x800 (leer = App-Standard)", &mut self.window_size);
        ui.text(TS_CAPTION | TX_SECONDARY, "Weitere Argumente: ein Argument pro Zeile, ohne Shell-Anführungszeichen.");
        ui.next_size(-1, 100); ui.textbox(2221, Some("--dump"), &mut self.arguments, 32768 | TB_MULTI);
        ui.text(TS_CAPTION | TX_SECONDARY, "Unten ein erfolgreiches App-Ergebnis auswählen und starten.");
    }
    fn package_view(&mut self, ui: &mut Ui) {
        let app = self.apps.get(self.active).map(|a| String::from(s(a, "id"))).unwrap_or_default();
        ui.box_begin(2300, BOX_H | BOX_XCENTER, 12);
        ui.next_size(-1, 0); ui.text(TS_SUBTITLE, &format!("Paketdaten · {app}"));
        if ui.button(2301, "Paketdaten speichern") & EV_ACT != 0 {
            let mut values = object(); for (key, value) in PACKAGE_KEYS.iter().zip(&self.package_fields) { values.insert(key, Value::string(value.clone())); }
            self.send("save_package", Value::object([("app", Value::string(app)), ("values", values), ("revision", Value::string(self.package_revision.clone()))]));
        }
        ui.box_end();
        ui.text(TS_CAPTION | TX_SECONDARY, "Styles mit Leerzeichen trennen. Icons: res/app_48.png, app_128.png, app_256.png.");
        for i in 0..PACKAGE_KEYS.len() { edit(ui, 2310 + i as u32, PACKAGE_LABELS[i], &mut self.package_fields[i]); }
    }
    fn jobs_view(&mut self, ui: &mut Ui) {
        ui.next_size(-1, 250); ui.box_begin(2400, BOX_V | BOX_FIXED | BOX_CARD | pad(12), 8);
        ui.box_begin(2401, BOX_H | BOX_XCENTER, 10);
        ui.text(TS_BODY_STRONG, "Aufträge & Ergebnisse");
        let labels: Vec<String> = self.jobs.iter().map(|j| format!("{} · {} · {}", s(j, "app"), status(s(j, "status")), s(j, "id"))).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        ui.next_size(-1, 32);
        if ui.combobox(2402, &mut self.job_index, &refs) & EV_ACT != 0 {
            self.artifact_index = 0;
            if let Some(j) = self.jobs.get(self.job_index as usize) { self.send("job", Value::object([("id", Value::string(s(j, "id")))])); }
        }
        let job = self.jobs.get(self.job_index as usize).cloned().unwrap_or_else(object);
        if ui.button(2403, "Abbrechen") & EV_ACT != 0 { self.send("cancel", Value::object([("id", Value::string(s(&job, "id")))])); }
        if ui.button(2404, "Erneut ausführen") & EV_ACT != 0 { self.send("retry", Value::object([("id", Value::string(s(&job, "id")))])); }
        ui.box_end();
        let elapsed = number(&job, "duration").max(number(&job, "elapsed")) + if s(&job, "status") == "running" { plat::time_us().saturating_sub(number(&job, "seen_us")) / 1_000_000 } else { 0 };
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("{} · {} · {} s {}", status(s(&job, "status")), s(&job, "phase"), elapsed, compact(s(&job, "message"), 160)));
        ui.tabs(2410, &mut self.bottom, &["Protokoll", "Ergebnisse", "Tests"]);
        match self.bottom {
            1 => {
                let artifacts = array(&job, "artifacts");
                let labels: Vec<String> = artifacts.iter().map(|a| format!("{} · {} · {} · {}", s(a, "kind"), s(a, "target"), s(a, "style"), s(a, "path").rsplit(['/', '\\']).next().unwrap_or(""))).collect();
                let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
                ui.next_size(-1, 32); ui.combobox(2420, &mut self.artifact_index, &refs);
                if let Some(a) = artifacts.get(self.artifact_index as usize) {
                    ui.text(TS_CAPTION | TX_SECONDARY, &compact(s(a, "path"), 160));
                    ui.box_begin(2421, BOX_H, 12);
                    if ui.button(2422, "Ordner öffnen") & EV_ACT != 0 { ui.open_folder(s(a, "path")); }
                    if s(a, "kind") == "app" {
                        if s(&job, "status") != "succeeded" { ui.next_flags(ST_DISABLED); }
                        if ui.accent_button(2423, "App starten") & EV_ACT != 0 {
                            let argv = self.argv_for(s(a, "app"));
                            let mut command = vec![Value::string(s(a, "path"))];
                            command.extend(argv.as_array().unwrap_or(&[]).iter().cloned());
                            self.preview = format!("{} · {} · {}\n\n{}", s(a, "app"), s(a, "target"), s(a, "style"), json::stringify(&Value::Array(command)).unwrap_or_default());
                            self.launch_pending = Some(Value::object([("id", Value::string(s(&job, "id"))), ("artifact", Value::i64(self.artifact_index as i64)), ("argv", argv)]));
                            self.preview_open.set(1);
                        }
                    }
                    ui.box_end();
                }
            },
            2 => {
                let mut text = String::new();
                for test in array(&job, "tests") { text.push_str(&format!("{} / {} / {}: {} {}\n", s(test, "target"), s(test, "style"), s(test, "script"), test_status(s(test, "status")), s(test, "reason"))); }
                ui.next_size(-1, -1); ui.textbox(2430, Some("Noch keine Testergebnisse."), &mut text, 1048576 | TB_MULTI | TB_READONLY);
            },
            _ => { ui.next_size(-1, -1); ui.textbox(2431, Some("Build-Ausgabe erscheint hier. Vollständige Logs bleiben im Projekt gespeichert."), &mut self.log, 1048576 | TB_MULTI | TB_READONLY); },
        }
        ui.box_end();
    }
    fn dialogs(&mut self, ui: &mut Ui) {
        let launching = self.launch_pending.is_some();
        if ui.dialog_begin(2500, if launching { "App starten" } else { "Aufträge starten" }, &self.preview_open) {
            ui.text(TS_BODY, if launching { "Aufgelöster Programmaufruf:" } else { "Diese Aufträge werden nacheinander ausgeführt:" });
            ui.next_size(-1, 250); ui.textbox(2501, None, &mut self.preview, 1048576 | TB_MULTI | TB_READONLY | TB_WRAP);
            if ui.dialog_buttons(Some(if launching { "Programm starten" } else { "In Warteschlange einreihen" }), None, Some("Zurück")) == DLG_PRIMARY {
                if let Some(params) = self.launch_pending.take() { self.send("run", params); }
                else { self.send("enqueue", Value::object([("profile", self.prepared.clone())])); }
            }
            ui.dialog_end();
        }
        if ui.dialog_begin(2510, "Laufende Aufträge", &self.close_open) {
            ui.text(TS_BODY, "Beim Schließen werden laufende Builds und gestartete Apps beendet.");
            if ui.dialog_buttons(Some("Weiterarbeiten"), Some("Aufträge abbrechen und schließen"), None) == DLG_SECONDARY {
                self.save_local(ui); self.client = None; ui.quit();
            }
            ui.dialog_end();
        }
        if ui.dialog_begin(2520, "Projekt & Werkzeuge", &self.settings) {
            edit(ui, 2521, "Projektordner (Repository oder RustLib)", &mut self.workspace);
            if ui.button(2522, "build_rs.py auswählen …") & EV_ACT != 0 { self.selected_file = self.workspace.clone(); self.file_open.set(1); }
            edit(ui, 2523, "Python-Programm (leer = python / python3)", &mut self.python);
            edit(ui, 2524, "WSL-Distribution auf Windows", &mut self.distro);
            if let Some(caps) = self.catalog.get("capabilities") {
                ui.text(TS_CAPTION | TX_SECONDARY, if s(caps, "rust").is_empty() { "Rust-Compiler nicht gefunden." } else { s(caps, "rust") });
                for (i, target) in TARGETS.iter().enumerate() {
                    if let Some(cap) = caps.get("targets").and_then(|c| c.get(target)) {
                        let available = |key| if cap.get(key).and_then(Value::as_bool) == Some(true) { "ja" } else { "nein" };
                        ui.text(TS_CAPTION, &format!("{} · Bauen: {} · Starten: {} · Pakete: {}", TARGET_LABELS[i], available("build"), available("run"), available("package")));
                    }
                }
            }
            ui.text(TS_CAPTION | TX_SECONDARY, &compact(&self.message, 220));
            if ui.dialog_buttons(Some("Verbinden"), None, Some("Schließen")) == DLG_PRIMARY { self.connect(); self.save_local(ui); }
            ui.dialog_end();
        }
        if ui.filedlg(2530, FD_OPEN, &self.file_open, &mut self.selected_file, 4096, &["py=Python-Projektdatei"]) == DLG_PRIMARY {
            if let Some((dir, _)) = self.selected_file.rsplit_once(['/', '\\']) { self.workspace = dir.into(); }
        }
    }
}
fn launch() -> i32 {
    if let Some(python) = plat::arg_value("--process-check") {
        return process_checks::run(&python, &plat::arg_value("--scratch").unwrap_or_else(plat::cwd));
    }
    let app = Manager::new();
    let desc = AppDesc { w: app.local.get_int("window", "width", DESC.w as i64).clamp(1000, 3840) as u16,
        h: app.local.get_int("window", "height", DESC.h as i64).clamp(720, 2160) as u16, ..DESC };
    ui::launch(&desc, app, &style::STYLE)
}
ui::main!(launch());
