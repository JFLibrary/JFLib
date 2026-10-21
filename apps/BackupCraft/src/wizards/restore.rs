// restore -- the restore wizard: restore point, content (everything or a selection from the
// point's file tree), target (original place or another folder on a host), conflicts,
// summary; the restore runs as an operation

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::{self, Method};
use bc_proto::ids;
use bc_proto::j::Json;
use bc_proto::model::*;
use ui::*;

use crate::app::Console;
use crate::icons::*;
use crate::net::After;
use crate::tr::{t, tf};
use crate::widgets::picker::Picker;
use crate::wizards::{page_begin, page_end, tile, ID_WRESTORE};
use crate::{fmt, widgets};

pub struct RestoreWiz {
    step: usize,
    job: String,
    job_name: String,
    point: Option<String>,
    grid: GridState,
    all: i32,
    picker: Option<Picker>,
    original: i32,
    host: String,
    target: String,
    conflict: i32,
    pub busy: bool,
    pub error: String,
}

const STEPS: usize = 5;

impl RestoreWiz {
    pub fn new(job: &Job, point: Option<String>) -> RestoreWiz {
        let mut grid = GridState::default();
        if let Some(p) = &point {
            grid.select_only(Some(ids::key(p)));
        }
        RestoreWiz {
            step: if point.is_some() { 1 } else { 0 },
            job: job.id.clone(),
            job_name: job.name.clone(),
            picker: point.as_ref().map(|p| Picker::point(p)),
            point,
            grid,
            all: 0,
            original: 0,
            host: job.host_id.clone(),
            target: String::new(),
            conflict: 0,
            busy: false,
            error: String::new(),
        }
    }

    fn items(&self) -> Vec<String> {
        if self.all != 0 {
            return Vec::new();
        }
        self.picker.as_ref().map(|p| p.selected()).unwrap_or_default()
    }

    fn check(&self, step: usize) -> &'static str {
        match step {
            0 if self.point.is_none() => t("Bitte einen Restore-Point wählen."),
            1 if self.all == 0 && self.items().is_empty() => t("Bitte auswählen, was wiederhergestellt wird – oder „Alles“."),
            2 if self.original == 0 && (self.host.is_empty() || self.target.trim().is_empty()) => t("Bitte Host und Zielordner angeben."),
            _ => "",
        }
    }

    pub fn frame(&mut self, app: &mut Console, ui: &mut Ui) -> bool {
        let id = ID_WRESTORE;
        let steps = [t("Restore-Point"), t("Inhalt"), t("Ziel"), t("Optionen"), t("Zusammenfassung")];
        let states: Vec<i32> = (0..STEPS).map(|i| if i < self.step { if self.check(i).is_empty() { WS_DONE } else { WS_ERROR } } else { WS_TODO }).collect();
        let (clicked, close) = page_begin(ui, id, &tf("Wiederherstellen: {}", &[&self.job_name]), t("Vorhandene Dateien werden nie stillschweigend überschrieben."), &steps, self.step, &states);
        if let Some(s) = clicked {
            self.step = s;
        }
        match self.step {
            0 => self.step_point(app, ui, id),
            1 => self.step_content(app, ui, id),
            2 => self.step_target(app, ui, id),
            3 => self.step_options(ui, id),
            _ => self.step_summary(app, ui),
        }
        let err = self.check(self.step);
        let r = page_end(ui, id + 0x100, self.step == 0, self.step + 1 == STEPS, err.is_empty(), self.busy, &self.error.clone(), t("Wiederherstellen"));
        match r {
            1 => self.step = self.step.saturating_sub(1),
            2 if self.step + 1 < STEPS => self.step += 1,
            2 => {
                if let Some(bad) = (0..STEPS).find(|s| !self.check(*s).is_empty()) {
                    self.step = bad;
                } else if let Some(p) = &self.point {
                    self.busy = true;
                    let body = api::RestoreRequest {
                        items: self.items(),
                        original: self.original != 0,
                        host_id: self.host.clone(),
                        target: String::from(self.target.trim()),
                        conflict: [api::Conflict::Skip, api::Conflict::Rename, api::Conflict::Overwrite][self.conflict.clamp(0, 2) as usize],
                        passphrase: String::new(),
                    }
                    .to_json();
                    app.net.action(Method::Post, &format!("/api/points/{}/restore", p), Some(body), "restore.start", &["/api/operations"], After::CloseWizard);
                }
            }
            3 => return false,
            _ => {}
        }
        !close
    }

    fn step_point(&mut self, app: &mut Console, ui: &mut Ui, id: u32) {
        let now = ui.now();
        let path = format!("/api/jobs/{}/points", self.job);
        app.net.want(&path, 15_000, now);
        let pts = app.net.data.points.get(&self.job).cloned().unwrap_or_default();
        ui.text(TS_SUBTITLE, t("Welcher Stand?"));
        let keys: Vec<u64> = pts.iter().map(|p| ids::key(&p.id)).collect();
        // the newest point unless the user chose one
        if self.grid.sel.is_empty() {
            if let Some(first) = keys.first() {
                self.grid.select_only(Some(*first));
            }
        }
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_LAYER | BOX_CLIP | BOX_FIXED, 0);
        let cols = [GridCol::new(t("Zeitpunkt"), 200).flags(GC_FILL), GridCol::new(t("Art"), 120), GridCol::new(t("Dateien"), 100).flags(GC_RIGHT), GridCol::new(t("Größe"), 100).flags(GC_RIGHT)];
        let (a, b) = ui.grid_begin(id + 0x8000, &mut self.grid, &cols, &keys);
        for i in a..b {
            let p = &pts[i as usize];
            ui.grid_row(&mut self.grid, i);
            ui.grid_text(&self.grid, 0, &fmt::when(p.created, &app.zone));
            ui.grid_text_ts(&self.grid, 1, TS_BODY | TX_SECONDARY, if p.full { t("Vollständig") } else { t("Inkrementell") });
            ui.grid_text_ts(&self.grid, 2, TS_BODY | TX_SECONDARY, &fmt::count(p.files));
            ui.grid_text(&self.grid, 3, &fmt::size(p.size));
        }
        let ev = ui.grid_end(&mut self.grid);
        ui.box_end();
        if ev.ev & EV_ACT != 0 || self.point.is_none() {
            if let Some(k) = self.grid.cur.or_else(|| self.grid.sel.iter().next().copied()) {
                if let Some(p) = pts.iter().find(|p| ids::key(&p.id) == k) {
                    if self.point.as_deref() != Some(p.id.as_str()) {
                        self.point = Some(p.id.clone());
                        self.picker = Some(Picker::point(&p.id));
                    }
                }
            }
        }
        if ev.ev & EV_DOUBLE != 0 {
            self.step = 1;
        }
    }

    fn step_content(&mut self, app: &mut Console, ui: &mut Ui, id: u32) {
        ui.text(TS_SUBTITLE, t("Was wird wiederhergestellt?"));
        if tile(ui, id + 0x200, IC_SELECT_ALL, t("Alles"), t("Den kompletten Stand dieses Restore-Points."), self.all != 0, true, None) {
            self.all = 1;
        }
        if tile(ui, id + 0x201, IC_FOLDER_OPEN, t("Auswahl"), t("Einzelne Ordner und Dateien aus dem Restore-Point."), self.all == 0, true, None) {
            self.all = 0;
        }
        if self.all == 0 {
            let now = ui.now();
            let n = self.items().len();
            ui.text(TS_CAPTION | TX_SECONDARY, &tf("{} ausgewählt", &[&format!("{}", n)]));
            ui.next_size(-1, 320);
            ui.box_begin(0, BOX_V | BOX_LAYER | BOX_CLIP, 0);
            if let Some(p) = &mut self.picker {
                ui.next_size(-1, -1);
                p.frame(ui, &mut app.net, id + 0x400, now);
            }
            ui.box_end();
        }
    }

    fn step_target(&mut self, app: &mut Console, ui: &mut Ui, id: u32) {
        ui.text(TS_SUBTITLE, t("Wohin?"));
        let host = app.net.data.host(&self.host).map_or(String::new(), |h| h.name.clone());
        if tile(ui, id + 0x300, BC_RESTORE, t("Originalort"), &tf("Zurück an die ursprünglichen Pfade auf {}.", &[&host]), self.original != 0, true, None) {
            self.original = 1;
        }
        if tile(ui, id + 0x301, IC_FOLDER, t("Anderer Ordner"), t("In einen neuen Ordner auf einem Host – sicher zum Vergleichen."), self.original == 0, true, None) {
            self.original = 0;
        }
        if self.original == 0 {
            let hosts: Vec<Host> = app.net.data.hosts.clone();
            let names: Vec<&str> = hosts.iter().map(|h| h.name.as_str()).collect();
            let mut sel = hosts.iter().position(|h| h.id == self.host).map_or(-1, |p| p as i32);
            widgets::label(ui, t("Host"));
            ui.next_size(420, 0);
            if ui.combobox(id + 0x310, &mut sel, &names) & EV_ACT != 0 && sel >= 0 {
                self.host = hosts[sel as usize].id.clone();
            }
            let h = hosts.iter().find(|h| h.id == self.host).cloned();
            widgets::label(ui, t("Zielordner"));
            ui.next_size(420, 0);
            let hint = h.as_ref().and_then(|h| h.restore_roots.first().cloned()).unwrap_or_default();
            if self.target.is_empty() && !hint.is_empty() {
                let sep = if hint.contains('\\') { "\\" } else { "/" };
                self.target = format!("{}{}{}", hint, sep, self.job_name.replace([' ', '–'], "_"));
            }
            ui.textbox(id + 0x311, Some(hint.as_str()), &mut self.target, 1024);
            if let Some(h) = h {
                if h.restore_roots.is_empty() {
                    ui.infobar(id + 0x312, IB_WARNING, t("Kein Wiederherstellungsziel freigegeben"), t("Der Host nimmt keine Wiederherstellungen an. Ziele lassen sich unter „Server & Agenten“ freigeben."));
                } else {
                    widgets::hint(ui, &tf("Erlaubt: {}", &[&h.restore_roots.join(", ")]));
                }
            }
        }
    }

    fn step_options(&mut self, ui: &mut Ui, id: u32) {
        ui.text(TS_SUBTITLE, t("Wenn eine Datei schon existiert"));
        ui.radio(id + 0x500, t("Überspringen (vorhandene Datei bleibt)"), &mut self.conflict, 0);
        ui.radio(id + 0x501, t("Unter neuem Namen ablegen (Datei (wiederhergestellt).ext)"), &mut self.conflict, 1);
        ui.radio(id + 0x502, t("Überschreiben"), &mut self.conflict, 2);
        if self.conflict == 2 {
            ui.infobar(id + 0x503, IB_WARNING, t("Vorsicht"), t("Überschriebene Dateien lassen sich nur aus einem anderen Restore-Point zurückholen."));
        }
        widgets::hint(ui, t("Berechtigungen, Zeitstempel und symbolische Verknüpfungen werden mit wiederhergestellt; Verknüpfungen, die aus dem Ziel hinauszeigen, nicht."));
    }

    fn step_summary(&mut self, app: &mut Console, ui: &mut Ui) {
        ui.text(TS_SUBTITLE, t("Zusammenfassung"));
        let pts = app.net.data.points.get(&self.job).cloned().unwrap_or_default();
        let p = self.point.as_ref().and_then(|id| pts.iter().find(|p| &p.id == id).cloned()).unwrap_or_default();
        let what = if self.all != 0 { String::from(t("Alles")) } else { tf("{} Einträge", &[&format!("{}", self.items().len())]) };
        let host = app.net.data.host(&self.host).map_or(String::from("–"), |h| h.name.clone());
        let target = if self.original != 0 { tf("Originalort auf {}", &[&host]) } else { format!("{}: {}", host, self.target) };
        let conflict = [t("Überspringen"), t("Neuer Name"), t("Überschreiben")][self.conflict.clamp(0, 2) as usize];
        widgets::props(ui, &[
            (t("Job"), &self.job_name),
            (t("Restore-Point"), &fmt::when(p.created, &app.zone)),
            (t("Inhalt"), &what),
            (t("Ziel"), &target),
            (t("Bei Konflikten"), conflict),
        ]);
        for it in self.items().iter().take(10) {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.icon(IC_DOCUMENT, 16);
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, it);
            ui.box_end();
        }
        ui.infobar(ID_WRESTORE + 0x600, IB_INFO, t("Läuft im Hintergrund"), t("Den Fortschritt zeigt die Aktivitätsleiste; das Protokoll steht unter Monitoring."));
    }
}
