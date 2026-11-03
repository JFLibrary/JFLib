// experiments -- the panel Experimente (SA3, dock panel P_EXPERIMENTS, bottom, hidden until asked for):
// parameter sweeps without JSON, runs on worker threads, runs kept by their names, comparisons
//
//   Exp::new()                  nothing chosen, nothing running
//   panel(n, ui)                the list of the project's experiments and kept runs, the pages Aufbau
//                               (define.rs), Lauf (this file) and Vergleich (compare.rs)
//   states(n, ui) / command(n, ui, id, arg)   CMD_EXPERIMENTS, the file dialog's CMD_EXP_FILE_DONE
//   tick(n, ui)                 every frame: a run's progress and end, a typed change into the project,
//                               the file dialog, the question before a run goes
//   start(n) / cancel(n)        Starten, Abbrechen
//   keep_run(doc, exp, &sweep, name, date) -> Result<(), String>   a run into Project::runs
//   parse_numbers(text) -> Result<Vec<u64>, String>, parse_values(text) -> Vec<Value>   what a list field takes
//   import_text(n, text) / export_text(exp, file)   .jfnex files
//
// A change of the experiment in a field goes into the draft at once (Experiment::problems shows it
// live) and into the project as one undo step when the field loses the keyboard; a choice at once.
//
// A run: Starten hands jfn_run::SweepJob copies of the experiment and the project, so edits during a
// run do not touch it and the running simulation is not touched either.  The page polls it by a
// timer (TIMER_EXP): runs done of runs total.  Cancel leaves nothing: no half result gets a name.
//
// What is saved: running or finishing changes nothing in the project, no undo step, the file is not
// dirty; the result stays in this session only.  "Behalten" under a name puts it into Project::runs:
// the file has changed then (Doc::file_changed: the title's dot, the question on closing, autosave), but
// there is no undo step, as for removing one, which asks first.  A kept run has no details of each
// single run (Sweep::to_value); its CSV and table are the same bytes.  At most KEEP_RUNS of at most
// KEEP_POINTS points (jfn_model::experiment), and the page says so when that is reached.
//
// --exp-golden FILE (tests): a finished run's table is compared with this file, and the page says how.

mod compare;
mod define;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use agent_core::jmap::Value;
use agent_core::json::Number;
use jfn_model::experiment::{Experiment, LoadAxis, SavedRun, Slo, Timing, FORMAT, KEEP_POINTS, KEEP_RUNS, VERSION};
use jfn_run::{SweepJob, SweepOut};
use jfn_sim::experiment::{Status, Sweep};
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;
use crate::doc::{edits, Doc};
use crate::workspace::icons::JN_METRICS;

/// the height CMD_EXPERIMENTS gives the bottom of the dock at least, the width of the list (DIP)
const PANEL_H: i32 = 460;
const LIST_W: i32 = 230;
/// how often a run's progress reaches the window (ms)
const POLL_MS: u32 = 100;
/// the threads a run may ask for
const RG_THREADS: Range = Range::new(1, 256, 1, 0);
/// points the table of a run shows at most (its row ids)
const POINT_ROWS: usize = 1500;
/// the width of that table (DIP)
const POINTS_W: i32 = 1120;
/// below this width of the panel the list stands above the pages (DIP)
const STACK_W: i32 = 640;
/// the height of the list then (DIP)
const STACK_H: i32 = 170;
/// the file types of the dialogs (German; shown through t())
const EXT_JFNEX: &str = "jfnex=Experiment (jfnex)";
const EXT_CSV: &str = "csv=Tabelle (CSV)";

/// what the file dialog is open for
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileFor {
    Import,
    Export,
    Csv,
}

/// a run that has ended, not kept yet
pub(crate) struct Done {
    pub sweep: Sweep,
    pub exp: Experiment,
    pub date: String,
    /// the table against --exp-golden
    golden: Option<bool>,
}

/// a run under way
struct Running {
    job: SweepJob,
    exp: Experiment,
    date: String,
}

/// the panel's state
pub(crate) struct Exp {
    /// the chosen experiment (an index of Project::experiments)
    pub sel: i32,
    /// the page: 0 Aufbau, 1 Lauf, 2 Vergleich
    pub page: i32,
    /// the width of the list
    split: i32,
    /// the chosen experiment as the fields have it
    pub(crate) draft: Option<Experiment>,
    /// the texts of the fields by widget id (they fill from the draft)
    pub(crate) bufs: BTreeMap<u32, String>,
    /// what a field's text cannot be
    pub(crate) errs: BTreeMap<u32, String>,
    /// the field being typed in (its change reaches the project when it loses the keyboard)
    pub(crate) typing: u32,
    /// a choice changed the draft: into the project now
    pub(crate) chosen: bool,
    /// the expander of the prices
    pub(crate) prices: i32,
    /// Experiment::problems of the draft and what they were read for
    found: Vec<String>,
    found_for: Option<(Experiment, u32)>,
    run: Option<Running>,
    pub(crate) done: Option<Done>,
    /// counts what Vergleich reads from (a run done, kept, gone)
    pub(crate) runs_ver: u32,
    /// the threads of the next run
    threads: i32,
    /// what the last run or file said: an error, or that all is well
    note: String,
    note_bad: bool,
    /// the name a run is kept under
    keep_name: String,
    /// Vergleich: A and B (compare.rs)
    pub(crate) cmp: compare::Cmp,
    points: GridState,
    fd: Shared<i32>,
    fd_path: String,
    fd_for: FileFor,
    /// the kept run that goes after its question
    ask: Shared<i32>,
    ask_run: usize,
}

impl Exp {
    pub(crate) fn new() -> Exp {
        Exp {
            sel: 0,
            page: 0,
            split: LIST_W,
            draft: None,
            bufs: BTreeMap::new(),
            errs: BTreeMap::new(),
            typing: 0,
            chosen: false,
            prices: 0,
            found: Vec::new(),
            found_for: None,
            run: None,
            done: None,
            runs_ver: 0,
            threads: (plat::cpu_count().saturating_sub(1).max(1)) as i32,
            note: String::new(),
            note_bad: false,
            keep_name: String::new(),
            cmp: compare::Cmp::new(),
            points: GridState::default(),
            fd: Shared::new(0),
            fd_path: String::new(),
            fd_for: FileFor::Import,
            ask: Shared::new(0),
            ask_run: 0,
        }
    }

    /// a run is under way
    pub(crate) fn running(&self) -> bool {
        self.run.is_some()
    }

    // what is said below the buttons
    fn say(&mut self, text: String, bad: bool) {
        self.note = text;
        self.note_bad = bad;
    }

    // the fields start from the draft again
    pub(crate) fn fresh_fields(&mut self) {
        self.bufs.clear();
        self.errs.clear();
    }
}

// ---------------------------------------------------------------- what the fields take

/// numbers separated by commas or spaces
pub(crate) fn parse_numbers(text: &str) -> Result<Vec<u64>, String> {
    let mut out = Vec::new();
    for w in text.split([',', ' ', ';']).map(str::trim).filter(|w| !w.is_empty()) {
        out.push(w.parse::<u64>().map_err(|_| tf("„{}“ ist keine ganze Zahl.", &[w]))?);
    }
    Ok(out)
}

/// a word of a field as a value: a number, true or false, else a text
pub(crate) fn value_of(w: &str) -> Value {
    let w = w.trim();
    if w == "true" || w == "false" {
        return Value::Bool(w == "true");
    }
    match Number::parse(&w.replace(',', ".")) {
        Some(n) => Value::Number(n),
        None => Value::string(w),
    }
}

/// the values of an axis: separated by semicolons, or commas when no value has a decimal comma
pub(crate) fn parse_values(text: &str) -> Vec<Value> {
    let sep = if text.contains(';') { ';' } else { ',' };
    text.split(sep).map(str::trim).filter(|w| !w.is_empty()).map(value_of).collect()
}

/// a value of an axis as a field shows it
pub(crate) fn value_text(v: &Value) -> String {
    match v {
        Value::Number(n) => String::from(n.as_str()),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// a name no experiment of the project has
fn free_name(p: &jfn_model::Project) -> String {
    let free = |n: &str| !p.experiments.iter().any(|x| x.name.eq_ignore_ascii_case(n));
    (1..).map(|k| format!("experiment{}", k)).find(|n| free(n)).unwrap_or_default()
}

/// a new experiment on what the project has: its first load generator, three levels
fn fresh(p: &jfn_model::Project) -> Experiment {
    let gen_dev = p.devices.iter().find_map(|d| d.services.iter().find(|s| s.kind == "http-load").map(|s| (d.name.clone(), s.name.clone())));
    let (dev, svc) = gen_dev.unwrap_or_default();
    let mut slo = Slo::default();
    slo.p99_ms = Value::u64(200);
    slo.errors_pct = Value::u64(1);
    Experiment {
        format: String::from(FORMAT),
        version: VERSION,
        name: free_name(p),
        load: LoadAxis { dev, svc, rps: alloc::vec![100, 200, 400], ..LoadAxis::default() },
        timing: Timing { warmup_s: 5, settle_s: 3, measure_s: 10, seeds: Vec::new() },
        slo,
        ..Experiment::default()
    }
}

/// a kept run goes into the project: the file changes, there is no undo step
pub(crate) fn keep_run(doc: &mut Doc, exp: &Experiment, sweep: &Sweep, name: &str, date: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(String::from(t("Ein Lauf braucht einen Namen.")));
    }
    if doc.proj.runs.iter().any(|r| r.name.eq_ignore_ascii_case(name)) {
        return Err(tf("Einen Lauf „{}“ gibt es schon.", &[name]));
    }
    if doc.proj.runs.len() >= KEEP_RUNS {
        return Err(tf("Das Projekt hält schon {} Läufe; entfernen Sie erst einen.", &[&KEEP_RUNS.to_string()]));
    }
    if sweep.points.len() > KEEP_POINTS {
        return Err(tf("Dieser Lauf hat {} Punkte; das Projekt hält höchstens {} je Lauf. Die CSV-Datei hält alles.", &[&sweep.points.len().to_string(), &KEEP_POINTS.to_string()]));
    }
    doc.proj.runs.push(SavedRun { name: String::from(name), date: String::from(date), experiment: exp.clone(), result: sweep.to_value() });
    doc.file_changed();
    Ok(())
}

/// the text of a .jfnex file for this experiment; `file`: the project's file it belongs to
pub(crate) fn export_text(exp: &Experiment, file: &str) -> String {
    let mut x = exp.clone();
    x.project = String::from(file);
    x.format = String::from(FORMAT);
    x.version = VERSION;
    x.to_text()
}

/// an experiment from a .jfnex file into the project's list, one undo step (it may not fit: its
/// findings show)
pub(crate) fn import_text(n: &mut Net, text: &str) -> Result<(), String> {
    let mut x = Experiment::from_text(text).map_err(|m| tr::msg(&m))?;
    x.project.clear();
    if x.name.is_empty() || n.doc.proj.experiments.iter().any(|y| y.name.eq_ignore_ascii_case(&x.name)) {
        x.name = free_name(&n.doc.proj);
    }
    let mut list = n.doc.proj.experiments.clone();
    list.push(x);
    n.exp.sel = list.len() as i32 - 1;
    n.doc.apply(edits::experiments(&n.doc.proj, "Experiment einlesen", list));
    n.exp.draft = None;
    Ok(())
}

// ---------------------------------------------------------------- commands and the frame

/// enabled / checked of CMD_EXPERIMENTS (every frame)
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    ui.cmd_state(CMD_EXPERIMENTS, if n.shell.dock.visible(P_EXPERIMENTS) { CF_CHECKED } else { 0 });
}

/// the commands of the panel
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32, _arg: u32) -> bool {
    match id {
        CMD_EXPERIMENTS => {
            let dock = &mut n.shell.dock;
            dock.show(P_EXPERIMENTS);
            dock.activate(P_EXPERIMENTS);
            // (the page wants room: a good part of the window)
            let want = PANEL_H.max(ui.px2dip(ui.client_size().1) * 3 / 5);
            let dock = &mut n.shell.dock;
            if dock.side(P_EXPERIMENTS) == Some(DOCK_BOTTOM) && dock.size(DOCK_BOTTOM) < want {
                dock.set_size(DOCK_BOTTOM, want);
            }
            ui.focus(ID_EX_LIST);
        }
        CMD_EXP_FILE_DONE => file_chosen(n),
        _ => return false,
    }
    ui.repass();
    true
}

// the file dialog chose fd_path
fn file_chosen(n: &mut Net) {
    let path = n.exp.fd_path.clone();
    match n.exp.fd_for {
        FileFor::Import => match plat::read_file(&path).map(|b| String::from_utf8_lossy(&b).into_owned()) {
            Some(text) => {
                if let Err(e) = import_text(n, &text) {
                    n.exp.say(e, true);
                }
            }
            None => n.exp.say(tf("{} lässt sich nicht lesen.", &[&path]), true),
        },
        FileFor::Export => {
            let Some(x) = n.exp.draft.clone() else { return };
            let text = export_text(&x, &n.doc.file_name());
            save_file(n, &path, text.as_bytes());
        }
        FileFor::Csv => {
            let Some(csv) = compare::csv_shown(n) else { return };
            save_file(n, &path, csv.as_bytes());
        }
    }
}

fn save_file(n: &mut Net, path: &str, bytes: &[u8]) {
    if plat::write_file(path, bytes) {
        n.exp.say(tf("Gespeichert: {}", &[path]), false);
    } else {
        n.exp.say(tf("{} lässt sich nicht schreiben.", &[path]), true);
    }
}

// the file dialog opens
fn ask_file(n: &mut Net, what: FileFor, name: &str) {
    let dir = n.doc.path.as_deref().and_then(|p| p.rfind(['/', '\\']).map(|k| String::from(&p[..k])));
    n.exp.fd_path = match dir {
        Some(d) => format!("{}/{}", d.replace('\\', "/"), name),
        None => String::from(name),
    };
    n.exp.fd_for = what;
    n.exp.fd.set(1);
}

/// every frame, panel or not: a run's end, a typed change into the project, the dialogs
pub(crate) fn tick(n: &mut Net, ui: &mut Ui) {
    // the run's progress and end
    if n.exp.run.is_some() {
        ui.timer(TIMER_EXP, POLL_MS);
        poll(n, ui);
    }
    // the field gave the keyboard up
    if n.exp.typing != 0 && ui.focused() != n.exp.typing {
        commit(n);
    }
    if n.exp.chosen {
        commit(n);
    }
    let open = n.exp.fd.clone();
    let (flags, ext) = match n.exp.fd_for {
        FileFor::Import => (FD_OPEN, EXT_JFNEX),
        FileFor::Export => (FD_SAVE, EXT_JFNEX),
        FileFor::Csv => (FD_SAVE, EXT_CSV),
    };
    if ui.filedlg(ID_EX_FD, flags, &open, &mut n.exp.fd_path, 1040, &[t(ext)]) & EV_ACT != 0 {
        ui.cmd_run(CMD_EXP_FILE_DONE, 0);
    }
    // the question before a kept run goes
    let ask = n.exp.ask.clone();
    if ui.dialog_begin(ID_EX_ASK, t("Lauf entfernen?"), &ask) {
        let name = n.doc.proj.runs.get(n.exp.ask_run).map_or(String::new(), |r| r.name.clone());
        ui.next_size(380, 0);
        ui.text(TS_BODY | TX_WRAP, &tf("„{}“ geht aus dem Projekt, mit seinen Ergebnissen. Das lässt sich nicht rückgängig machen.", &[&name]));
        if ui.dialog_buttons(Some(t("Entfernen")), None, Some(t("Abbrechen"))) == DLG_PRIMARY {
            let k = n.exp.ask_run;
            if k < n.doc.proj.runs.len() {
                n.doc.proj.runs.remove(k);
                n.doc.file_changed();
                n.exp.runs_ver = n.exp.runs_ver.wrapping_add(1);
                n.exp.cmp.forget();
            }
            ui.repass();
        }
    }
    ui.dialog_end();
}

// a typed or chosen change of the draft into the project: one undo step
fn commit(n: &mut Net) {
    n.exp.typing = 0;
    n.exp.chosen = false;
    let Some(x) = n.exp.draft.clone() else { return };
    let k = n.exp.sel as usize;
    if n.doc.proj.experiments.get(k).is_none_or(|y| *y == x) {
        return;
    }
    let mut list = n.doc.proj.experiments.clone();
    list[k] = x;
    n.doc.apply(edits::experiments(&n.doc.proj, "Experiment ändern", list));
}

// the run has moved on, or ended
fn poll(n: &mut Net, ui: &mut Ui) {
    let Some(r) = n.exp.run.as_mut() else { return };
    // (the timer brings the next pass)
    let Some(out) = r.job.take() else { return };
    let Some(r) = n.exp.run.take() else { return };
    match out {
        SweepOut::Done(mut sweep) => {
            // (what is left is what a kept run stores)
            sweep.details.clear();
            let golden = plat::arg_value("--exp-golden").and_then(|f| plat::read_file(&f)).map(|b| String::from_utf8_lossy(&b).replace("\r\n", "\n") == sweep.table());
            n.exp.say(String::new(), false);
            n.exp.keep_name = String::from(r.exp.name.as_str());
            n.exp.done = Some(Done { sweep, exp: r.exp, date: r.date, golden });
            n.exp.runs_ver = n.exp.runs_ver.wrapping_add(1);
            n.exp.cmp.forget();
        }
        SweepOut::Refused(found) => {
            let lines: Vec<String> = found.iter().map(tr::msg).collect();
            n.exp.say(lines.join("\n"), true);
        }
        SweepOut::Cancelled => n.exp.say(String::from(t("Abgebrochen: von diesem Lauf bleibt nichts.")), false),
    }
    ui.repass();
}

/// when a run started, as a kept run says it (a fixed date in test runs)
fn date_now(headless: bool) -> String {
    if headless {
        return String::from("2000-01-01 00:00");
    }
    let (d, _) = plat::date();
    format!("{:04}-{:02}-{:02} {:02}:{:02}", d.year, d.month, d.day, d.hour, d.min)
}

/// Starten: copies of the experiment and the project go to a job of their own
pub(crate) fn start(n: &mut Net, headless: bool) {
    if n.exp.run.is_some() {
        return;
    }
    commit(n);
    let Some(x) = n.exp.draft.clone() else { return };
    let name = format!("{}.jfnex", x.name);
    let threads = n.exp.threads.max(1) as usize;
    // (what is kept of earlier runs would only be copied with every configuration)
    let mut p = n.doc.proj.clone();
    p.runs.clear();
    match SweepJob::start(x.clone(), p, name, threads) {
        Some(job) => {
            n.exp.say(String::new(), false);
            n.exp.run = Some(Running { job, exp: x, date: date_now(headless) });
        }
        None => n.exp.say(String::from(t("Es lässt sich kein Thread starten; hier läuft kein Experiment.")), true),
    }
}

/// Abbrechen
pub(crate) fn cancel(n: &mut Net) {
    if let Some(r) = n.exp.run.as_ref() {
        r.job.cancel();
    }
}

// the draft follows the project and the choice in the list
fn follow(n: &mut Net) {
    let list = &n.doc.proj.experiments;
    if list.is_empty() {
        n.exp.draft = None;
        n.exp.sel = 0;
        return;
    }
    n.exp.sel = n.exp.sel.clamp(0, list.len() as i32 - 1);
    let now = &list[n.exp.sel as usize];
    let typing = n.exp.typing != 0;
    if !typing && n.exp.draft.as_ref() != Some(now) {
        n.exp.draft = Some(now.clone());
        n.exp.fresh_fields();
    }
}

/// the panel
pub(crate) fn panel(n: &mut Net, ui: &mut Ui) {
    follow(n);
    let stacked = ui.px2dip(ui.box_rect()[2]) < STACK_W;
    let mut split = if stacked { STACK_H } else { n.exp.split };
    ui.next_size(-1, -1);
    ui.split_begin(ID_EX_SPLIT, split, stacked);
    side(n, ui);
    ui.split_next();
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | pad(6), 6);
    if n.doc.proj.experiments.is_empty() && n.doc.proj.runs.is_empty() {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(12), 0);
        ui.empty_state(JN_METRICS, t("Noch kein Experiment"), t("Ein Experiment vergleicht Konfigurationen desselben Projekts unter steigender Last: größer oder mehr, wie weit trägt es, was begrenzt es, was kostet es."));
        ui.box_end();
    } else {
        let names = [t("Aufbau"), t("Lauf"), t("Vergleich")];
        let mut page = n.exp.page;
        ui.next_flags(ST_COMPACT);
        if ui.segmented(ID_EX_PAGE, &mut page, &names) & EV_ACT != 0 {
            n.exp.page = page;
            ui.repass();
        }
        match n.exp.page {
            1 => run_page(n, ui),
            2 => compare::page(n, ui),
            _ => define::page(n, ui),
        }
    }
    ui.box_end();
    if ui.split_end(&mut split) && !stacked {
        n.exp.split = split;
    }
}

// the left side: the experiments, their buttons, the kept runs
fn side(n: &mut Net, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | pad(6), 6);
    ui.text(TS_BODY_STRONG, t("Experimente"));
    let names: Vec<String> = n.doc.proj.experiments.iter().map(|x| x.name.clone()).collect();
    let count = names.len().min(60);
    let mut sel = n.exp.sel;
    ui.next_size(-1, -1);
    let (first, end) = ui.list_begin(ID_EX_LIST, &mut sel, count as i32, 26);
    for i in first..end {
        ui.list_item_begin(i);
        ui.text(TS_BODY | TX_ELLIPSIS, &names[i as usize]);
        ui.list_item_end();
    }
    let (ev, s) = ui.list_end(&mut sel);
    if ev & EV_ACT != 0 && s != n.exp.sel {
        commit(n);
        n.exp.sel = s;
        n.exp.draft = None;
        ui.repass();
    }
    ui.box_begin(0, BOX_H | BOX_WRAP, 4);
    let past_end = names.len() >= 60;
    ui.next_flags(ST_COMPACT | if past_end { ST_DISABLED } else { 0 });
    ui.next_icon(IC_ADD);
    if ui.button(ID_EX_NEW, t("Neu")) & EV_ACT != 0 {
        commit(n);
        let mut list = n.doc.proj.experiments.clone();
        list.push(fresh(&n.doc.proj));
        n.exp.sel = list.len() as i32 - 1;
        n.exp.page = 0;
        n.exp.draft = None;
        n.doc.apply(edits::experiments(&n.doc.proj, "Experiment anlegen", list));
        ui.repass();
    }
    ui.next_flags(ST_COMPACT);
    if ui.button(ID_EX_IMPORT, t("Einlesen …")) & EV_ACT != 0 {
        ask_file(n, FileFor::Import, "");
        ui.repass();
    }
    let none = n.exp.draft.is_none();
    ui.next_flags(ST_COMPACT | if none { ST_DISABLED } else { 0 });
    if ui.button(ID_EX_EXPORT, t("Ausgeben …")) & EV_ACT != 0 {
        commit(n);
        let name = n.exp.draft.as_ref().map_or(String::new(), |x| format!("{}.jfnex", x.name));
        ask_file(n, FileFor::Export, &name);
        ui.repass();
    }
    ui.next_flags(ST_COMPACT | ST_ICON_ONLY | if none { ST_DISABLED } else { 0 });
    ui.next_icon(IC_DELETE);
    if ui.button(ID_EX_REMOVE, t("Experiment entfernen")) & EV_ACT != 0 {
        commit(n);
        let k = n.exp.sel as usize;
        let mut list = n.doc.proj.experiments.clone();
        if k < list.len() {
            list.remove(k);
            n.doc.apply(edits::experiments(&n.doc.proj, "Experiment entfernen", list));
            n.exp.draft = None;
        }
        ui.repass();
    }
    ui.box_end();
    ui.text(TS_BODY_STRONG, &tf("Behaltene Läufe ({})", &[&n.doc.proj.runs.len().to_string()]));
    let runs = n.doc.proj.runs.len();
    if runs == 0 {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Ein Lauf bleibt nur, wenn Sie ihn unter einem Namen behalten."));
    }
    for k in 0..runs.min(KEEP_RUNS) {
        let (name, date) = n.doc.proj.runs.get(k).map_or((String::new(), String::new()), |r| (r.name.clone(), r.date.clone()));
        ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
        ui.next_size(-1, 0);
        ui.next_flags(ST_SUBTLE | ST_COMPACT);
        if ui.button(ID_EX_RUNS + 2 * k as u32, &name) & EV_ACT != 0 {
            n.exp.cmp.choose_a(compare::Src::Kept(k));
            n.exp.page = 2;
            ui.repass();
        }
        ui.tooltip(&date);
        ui.next_flags(ST_SUBTLE | ST_COMPACT | ST_ICON_ONLY);
        ui.next_icon(IC_DELETE);
        if ui.button(ID_EX_RUNS + 2 * k as u32 + 1, &tf("Lauf {} entfernen", &[&name])) & EV_ACT != 0 {
            n.exp.ask_run = k;
            n.exp.ask.set(1);
            ui.repass();
        }
        ui.box_end();
    }
    ui.box_end();
}

/// a level as the window says it
pub(crate) fn level_text(l: &jfn_model::experiment::Level) -> String {
    match l {
        jfn_model::experiment::Level::Rps(r) => format!("{}/s", r),
        jfn_model::experiment::Level::Users(u) => tf("{} Nutzer", &[&u.to_string()]),
    }
}

/// a number of millionths with `places` decimals, in the language's way
pub(crate) fn places(v: u64, unit: u64, places: u32, sep: u32) -> String {
    let scale = 10u64.pow(places);
    let scaled = (v as u128 * scale as u128 / unit as u128) as u64;
    let (a, b) = (scaled / scale, scaled % scale);
    if places == 0 {
        return a.to_string();
    }
    format!("{}{}{:0w$}", a, char::from_u32(sep).unwrap_or(','), b, w = places as usize)
}

/// a point's verdict in words
pub(crate) fn verdict(p: &jfn_sim::experiment::Point) -> &'static str {
    match p.status {
        Status::Memory => t("Speichergrenze"),
        Status::Failed => t("Startet nicht"),
        Status::Ok if p.knee => t("Knie"),
        Status::Ok if p.slo_ok => t("hält"),
        Status::Ok => t("hält nicht"),
    }
}

// the page Lauf: Starten, Abbrechen, how far, what the last run gave, Behalten
fn run_page(n: &mut Net, ui: &mut Ui) {
    refresh_found(n);
    let headless = ui.headless();
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_EX_RUN_SCROLL, SC_V | pad(8));
    ui.box_begin(0, BOX_V, 8);
    let running = n.exp.run.is_some();
    let blocked = !n.exp.found.is_empty() || n.exp.draft.is_none();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_flags(if running || blocked { ST_DISABLED } else { 0 });
    ui.next_icon(IC_PLAY);
    if ui.accent_button(ID_EX_START, t("Starten")) & EV_ACT != 0 {
        start(n, headless);
        ui.repass();
    }
    ui.next_flags(if running { 0 } else { ST_DISABLED });
    ui.next_icon(IC_STOP);
    if ui.button(ID_EX_CANCEL, t("Abbrechen")) & EV_ACT != 0 {
        cancel(n);
        ui.repass();
    }
    ui.text(TS_BODY | TX_SECONDARY, t("Threads"));
    ui.next_size(64, 0);
    ui.next_flags(if running { ST_DISABLED } else { 0 });
    let mut th = n.exp.threads;
    if ui.number_box(ID_EX_THREADS, &mut th, &RG_THREADS) & EV_ACT != 0 {
        n.exp.threads = th;
    }
    ui.box_end();
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Ein Lauf rechnet auf einer Kopie des Projekts. Was Sie währenddessen ändern, wirkt erst im nächsten Lauf, und die laufende Simulation bleibt unberührt."));
    if blocked && !running {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Starten geht, sobald der Aufbau keine Befunde mehr hat."));
    }
    if let Some(r) = n.exp.run.as_ref() {
        let (done, total) = (r.job.done(), r.job.total());
        ui.next_size(-1, 0);
        ui.progress(ID_EX_PROGRESS, done as i32, total.max(1) as i32);
        let what = if r.job.cancelled() { String::from(t("Wird abgebrochen …")) } else { tf("{} von {} Läufen fertig", &[&done.to_string(), &total.to_string()]) };
        ui.text(TS_BODY, &what);
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Ein Lauf ist eine Konfiguration mit einem Seed auf einer Laststufe."));
    }
    if !n.exp.note.is_empty() {
        let note = n.exp.note.clone();
        if n.exp.note_bad {
            crate::workspace::error_text(ui, &note, 0);
        } else {
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_WRAP, &note);
        }
    }
    done_part(n, ui);
    ui.box_end();
    ui.scroll_end();
}

// what the last run gave
fn done_part(n: &mut Net, ui: &mut Ui) {
    let Some(d) = n.exp.done.as_ref() else {
        if !n.exp.running() {
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Noch kein Lauf in dieser Sitzung; behaltene Läufe stehen im Vergleich."));
        }
        return;
    };
    let sep = ui.decimal_sep();
    let (points, configs, levels) = (d.sweep.points.clone(), d.sweep.configs.clone(), d.sweep.levels.clone());
    ui.text(TS_BODY_STRONG, &tf("Lauf von {}: {}", &[&d.date, &d.exp.name]));
    if let Some(same) = d.golden {
        ui.text(TS_CAPTION, if same { "golden: gleich" } else { "golden: verschieden" });
    }
    compare::warnings(ui, &d.sweep);
    // Behalten
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(240, 0);
    ui.textbox(ID_EX_KEEP_NAME, Some(t("Name des Laufs")), &mut n.exp.keep_name, 64);
    if ui.button(ID_EX_KEEP, t("Behalten")) & EV_ACT != 0 {
        keep_done(n);
        ui.repass();
    }
    if ui.button(ID_EX_CSV, t("CSV ausgeben …")) & EV_ACT != 0 {
        n.exp.cmp.csv_of_done = true;
        let name = n.exp.done.as_ref().map_or(String::new(), |d| format!("{}.csv", d.exp.name));
        ask_file(n, FileFor::Csv, &name);
        ui.repass();
    }
    ui.box_end();
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, &tf("Ohne Namen ist der Lauf weg, wenn das Projekt schließt. Das Projekt hält höchstens {} Läufe mit je {} Punkten, ohne die Einzelheiten je Gerät.", &[&KEEP_RUNS.to_string(), &KEEP_POINTS.to_string()]));
    // the points
    let shown = points.len().min(POINT_ROWS);
    let keys: Vec<u64> = (0..shown as u64).collect();
    let cols = [
        GridCol::new(t("Konfiguration"), 210).flags(GC_NOSORT),
        GridCol::new(t("Last"), 76).flags(GC_RIGHT | GC_NOSORT),
        GridCol::new(t("erreicht/s"), 84).flags(GC_RIGHT | GC_NOSORT),
        GridCol::new("p50", 72).flags(GC_RIGHT | GC_NOSORT),
        GridCol::new("p95", 72).flags(GC_RIGHT | GC_NOSORT),
        GridCol::new("p99", 72).flags(GC_RIGHT | GC_NOSORT),
        GridCol::new("p99.9", 72).flags(GC_RIGHT | GC_NOSORT),
        GridCol::new(t("Fehler"), 64).flags(GC_RIGHT | GC_NOSORT),
        GridCol::new(t("CPU"), 56).flags(GC_RIGHT | GC_NOSORT),
        GridCol::new(t("€/1000"), 84).flags(GC_RIGHT | GC_NOSORT),
        GridCol::new(t("SLO"), 96).flags(GC_NOSORT),
        GridCol::new(t("Engpass"), 130).flags(GC_NOSORT),
    ];
    // (wider than most panels: it scrolls sideways)
    let h = 34 + 26 * shown.clamp(3, 14) as i32;
    ui.next_size(-1, h + 8);
    ui.scroll_begin(ID_EX_POINTS_X, SC_H);
    ui.next_size(POINTS_W, h);
    let st = &mut n.exp.points;
    let (first, end) = ui.grid_begin(ID_EX_POINTS, st, &cols, &keys);
    let lat = |v: Option<u64>| v.map_or(String::from("–"), |ns| LcUnit::Secs.text(ns as f64 * 1e-9, sep));
    for i in first..end {
        let p = &points[i as usize];
        ui.grid_row(st, i);
        ui.grid_text(st, 0, configs.get(p.config).map_or("", |c| c.label.as_str()));
        ui.grid_text(st, 1, &levels.get(p.level).map_or(String::new(), level_text));
        ui.grid_text(st, 2, &places(p.achieved_m, 1000, 1, sep));
        ui.grid_text(st, 3, &lat(p.p50));
        ui.grid_text(st, 4, &lat(p.p95));
        ui.grid_text(st, 5, &lat(p.p99));
        ui.grid_text(st, 6, &lat(p.p999));
        ui.grid_text(st, 7, &format!("{} %", places(p.err_ppm, 10_000, 2, sep)));
        ui.grid_text(st, 8, &format!("{} %", places(p.cpu_max_pm, 10, 0, sep)));
        ui.grid_text(st, 9, &places(p.cost_per_1000_u, 1_000_000, 6, sep));
        ui.grid_text(st, 10, verdict(p));
        ui.grid_text(st, 11, &p.bottleneck);
    }
    let _ = ui.grid_end(st);
    ui.scroll_end();
    if points.iter().any(|p| p.status == Status::Memory) {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Speichergrenze: diese Stufe hielt mehr als 1 GiB und endete, wo sie war. Ein Generator ohne Grenze der Verbindungen öffnet unter Überlast je Anfrage eine neue; „Verbindungen je Ziel“ begrenzt das."));
    }
    if points.iter().any(|p| p.status == Status::Failed) {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Startet nicht: diese Konfiguration kam nicht hoch; ihre Stufen haben keine Zahlen."));
    }
}

// Behalten: the run under its name into the project
fn keep_done(n: &mut Net) {
    let Some(d) = n.exp.done.as_ref() else { return };
    let (x, sweep, date) = (d.exp.clone(), d.sweep.clone(), d.date.clone());
    let name = n.exp.keep_name.clone();
    match keep_run(&mut n.doc, &x, &sweep, &name, &date) {
        Ok(()) => {
            n.exp.say(tf("Behalten als „{}“. Das Projekt hat sich damit geändert; Speichern schreibt den Lauf in die Datei.", &[name.trim()]), false);
            n.exp.done = None;
            n.exp.runs_ver = n.exp.runs_ver.wrapping_add(1);
            let k = n.doc.proj.runs.len() - 1;
            n.exp.cmp.choose_a(compare::Src::Kept(k));
        }
        Err(e) => n.exp.say(e, true),
    }
}

// the findings of the draft, read again when it or the project changed
fn refresh_found(n: &mut Net) {
    let Some(x) = n.exp.draft.as_ref() else {
        n.exp.found.clear();
        return;
    };
    let key = (x.clone(), n.doc.rev);
    if n.exp.found_for.as_ref() == Some(&key) {
        return;
    }
    n.exp.found = x.problems(&n.doc.proj).iter().map(tr::msg).collect();
    n.exp.found_for = Some(key);
}
