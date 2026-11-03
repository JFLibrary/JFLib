// compare -- the page Vergleich of the panel Experimente: one run or two side by side, its table and
// its charts, with the reading help at the place it matters
//
//   Cmp::new() / forget() / choose_a(src)     what is chosen; a run's data is read again
//   Src                       the last run or a kept one
//   page(n, ui)               A and B, the table per configuration, a row per run (knee, maximum, what limits
//                             it, cost per hour and per 1000), the charts of ui.line_chart with LX_NUM: the saturation
//                             curve of every configuration, the latencies over throughput and the use of
//                             what limits it for the configuration chosen, with the knee marked and the
//                             SLO as a threshold
//   warnings(ui, &sweep)      a throughput that breaks down, levels that did not end well
//   curve_x(values) -> Vec<f64>   x values that never fall (SeriesBuf takes no others)
//   csv_shown(n) -> Option<String>   the CSV of what the page shows (Sweep::csv)
//
// A point's throughput can fall as the offered load rises: the latency chart then keeps it at the highest
// x so far, and warnings() says what happened.  Costs and the figures of the model are marked as
// assumptions; the labels of the configurations are the words of the tables (German, as in a report).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_sim::experiment::{Point, Sweep};
use ui::*;

use super::{level_text, places};
use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;

/// the height of a chart with its legend, the least width of one (DIP)
const CHART_H: i32 = 240;
const CHART_MIN_W: i32 = 340;
/// configurations the saturation chart draws at most
const CURVES_MAX: usize = 16;

/// where a run's figures are
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Src {
    /// the last run, not kept
    Done,
    /// a kept one: an index of Project::runs
    Kept(usize),
}

/// a chart's series: its label, its points
type Curve = (String, SeriesBuf);

/// what the charts draw, read when the choice changes (they keep their drawings per version)
struct Curves {
    sat: Vec<Curve>,
    lat: Vec<Curve>,
    util: Vec<Curve>,
    /// x of the knee on the latency chart and on the others
    knee_lat: Option<f64>,
    knee_off: Option<f64>,
    /// the limits in seconds, with their names
    limits: Vec<(f64, String)>,
    limit_text: String,
}

/// the page's state
pub(crate) struct Cmp {
    pub a: Option<Src>,
    pub b: Option<Src>,
    /// the configuration of the detail charts (an index of A's)
    cfg: i32,
    /// A and B as read, for this version of the panel's runs and this choice
    read: Option<(u32, Option<Src>, Option<Src>)>,
    pub a_sweep: Option<Sweep>,
    pub b_sweep: Option<Sweep>,
    curves: Option<(i32, Curves)>,
    grid: GridState,
    /// the next CSV file is the last run's (the page Lauf), else A's
    pub csv_of_done: bool,
    /// the reading help is open
    help: i32,
}

impl Cmp {
    pub(crate) fn new() -> Cmp {
        Cmp { a: None, b: None, cfg: 0, read: None, a_sweep: None, b_sweep: None, curves: None, grid: GridState::default(), csv_of_done: false, help: 0 }
    }

    /// the runs changed: read them again; what is gone is not chosen any more
    pub(crate) fn forget(&mut self) {
        self.read = None;
        self.curves = None;
        self.a = None;
        self.b = None;
    }

    pub(crate) fn choose_a(&mut self, s: Src) {
        self.a = Some(s);
        self.read = None;
        self.curves = None;
    }
}

/// x values that never fall
pub(crate) fn curve_x(values: &[f64]) -> Vec<f64> {
    let mut top = f64::NEG_INFINITY;
    values
        .iter()
        .map(|&v| {
            top = top.max(v);
            top
        })
        .collect()
}

/// a series from x and y values
fn buf_of(x: &[f64], y: &[f64]) -> SeriesBuf {
    let mut b = SeriesBuf::new(x.len().max(1));
    for (a, c) in x.iter().zip(y.iter()) {
        b.push(*a, *c);
    }
    b
}

/// the sweep of a source
fn sweep_of(n: &Net, s: Src) -> Option<Sweep> {
    match s {
        Src::Done => n.exp.done.as_ref().map(|d| d.sweep.clone()),
        Src::Kept(k) => n.doc.proj.runs.get(k).and_then(|r| Sweep::from_value(&r.result)),
    }
}

/// the CSV of what the page shows (the page Lauf: the last run)
pub(crate) fn csv_shown(n: &Net) -> Option<String> {
    if n.exp.cmp.csv_of_done {
        return n.exp.done.as_ref().map(|d| d.sweep.csv());
    }
    n.exp.cmp.a_sweep.as_ref().map(|s| s.csv())
}

// a source's name in a list
fn source_name(n: &Net, s: Src) -> String {
    match s {
        Src::Done => n.exp.done.as_ref().map_or(String::new(), |d| tf("Letzter Lauf ({})", &[&d.exp.name])),
        Src::Kept(k) => n.doc.proj.runs.get(k).map_or(String::new(), |r| format!("{} ({})", r.name, r.date)),
    }
}

// the runs to choose from
fn sources(n: &Net) -> Vec<Src> {
    let done = n.exp.done.is_some().then_some(Src::Done);
    done.into_iter().chain((0..n.doc.proj.runs.len()).map(Src::Kept)).collect()
}

// a run's figures, read again when the choice or the runs changed
fn refresh(n: &mut Net) {
    let list = sources(n);
    let c = &mut n.exp.cmp;
    if c.a.is_none_or(|a| !list.contains(&a)) {
        c.a = list.first().copied();
        c.read = None;
    }
    if c.b.is_some_and(|b| !list.contains(&b)) {
        c.b = None;
        c.read = None;
    }
    let key = (n.exp.runs_ver, c.a, c.b);
    if c.read == Some(key) {
        return;
    }
    let (a, b) = (c.a, c.b);
    let (x, y) = (a.and_then(|s| sweep_of(n, s)), b.and_then(|s| sweep_of(n, s)));
    let c = &mut n.exp.cmp;
    c.a_sweep = x;
    c.b_sweep = y;
    c.curves = None;
    c.read = Some(key);
    c.cfg = 0;
}

/// a throughput that breaks down, with what it means
pub(crate) fn warnings(ui: &mut Ui, s: &Sweep) {
    let found = s.overloads();
    if found.is_empty() {
        return;
    }
    ui.box_begin(0, BOX_V | BOX_CARD | pad(8), 4);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.status_dot(BADGE_WARN);
    ui.text(TS_BODY_STRONG, t("Der Durchsatz bricht unter Überlast ein"));
    ui.box_end();
    for m in &found {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, &tr::msg(m));
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Das Modell rechnet hier richtig: Abgelaufene Anfragen beschäftigen den Server weiter, ihre Verbindungen schließen, und die Ports des Clients gehen aus. Um die Kapazität zu sehen, geben Sie dem Lastgenerator eine lange Zeitgrenze und begrenzen seine Verbindungen je Ziel."));
    ui.box_end();
}

// the charts' data of A's configuration c, with B's p99 of the same name
fn curves_of(a: &Sweep, b: Option<&Sweep>, c: usize) -> Curves {
    let pts = |s: &Sweep, k: usize| -> Vec<Point> { s.points_of(k).cloned().collect() };
    let offered = |p: &[Point]| curve_x(&p.iter().map(|x| x.offered_m as f64 / 1000.0).collect::<Vec<_>>());
    let achieved = |p: &[Point]| p.iter().map(|x| x.achieved_m as f64 / 1000.0).collect::<Vec<_>>();
    // the saturation curves, with what was offered as the line they fall away from
    let mut sat: Vec<Curve> = Vec::new();
    let mine = pts(a, c);
    let line = offered(&mine);
    sat.push((String::from(t("Angeboten")), buf_of(&line, &line)));
    let mut add = |s: &Sweep, tag: &str| {
        for k in 0..s.configs.len().min(CURVES_MAX) {
            let p = pts(s, k);
            sat.push((format!("{}{}", s.configs[k].label, tag), buf_of(&offered(&p), &achieved(&p))));
        }
    };
    add(a, "");
    if let Some(s) = b {
        add(s, " (B)");
    }
    // the latencies over the throughput
    let x = curve_x(&achieved(&mine));
    let secs = |f: &dyn Fn(&Point) -> Option<u64>, p: &[Point]| -> Vec<f64> { p.iter().map(|q| f(q).map_or(f64::NAN, |ns| ns as f64 * 1e-9)).collect() };
    let mut lat: Vec<Curve> = alloc::vec![
        (String::from("p50"), buf_of(&x, &secs(&|q| q.p50, &mine))),
        (String::from("p95"), buf_of(&x, &secs(&|q| q.p95, &mine))),
        (String::from("p99"), buf_of(&x, &secs(&|q| q.p99, &mine))),
        (String::from("p99.9"), buf_of(&x, &secs(&|q| q.p999, &mine))),
    ];
    if let Some(s) = b {
        if let Some(k) = s.configs.iter().position(|o| o.label == a.configs[c].label) {
            let p = pts(s, k);
            lat.push((String::from("p99 (B)"), buf_of(&curve_x(&achieved(&p)), &secs(&|q| q.p99, &p))));
        }
    }
    // the use of what can limit it
    let pct = |f: &dyn Fn(&Point) -> u64| -> Vec<f64> { mine.iter().map(|q| f(q) as f64 / 10.0).collect() };
    let name = |what: &str, who: &str| if who.is_empty() { String::from(what) } else { format!("{} {}", what, who) };
    let top = mine.iter().max_by_key(|q| q.offered_m);
    let util: Vec<Curve> = alloc::vec![
        (name(t("CPU"), top.map_or("", |q| q.cpu_dev.as_str())), buf_of(&line, &pct(&|q| q.cpu_max_pm))),
        (name(t("Sperre"), top.map_or("", |q| q.lock_name.as_str())), buf_of(&line, &pct(&|q| q.lock_max_pm))),
        (name(t("Disk"), top.map_or("", |q| q.disk_dev.as_str())), buf_of(&line, &pct(&|q| q.disk_max_pm))),
        (name(t("Kabel"), top.map_or("", |q| q.link_name.as_str())), buf_of(&line, &pct(&|q| q.link_max_pm))),
    ];
    let knee = a.configs[c].knee;
    let knee_lat = knee.and_then(|k| x.get(k).copied());
    let knee_off = knee.and_then(|k| line.get(k).copied());
    let ns = |v: Option<u64>| v.map(|x| x as f64 * 1e-9);
    let l = &a.slo;
    let limits: Vec<(f64, String)> = [("p50", ns(l.p50)), ("p95", ns(l.p95)), ("p99", ns(l.p99)), ("p99.9", ns(l.p999))].into_iter().filter_map(|(n, v)| v.map(|s| (s, tf("SLO {}", &[n])))).collect();
    Curves { sat, lat, util, knee_lat, knee_off, limits, limit_text: a.configs[c].limit.clone() }
}

// the row of the table for a configuration of a run
fn cells(s: &Sweep, c: usize, sep: u32) -> [String; 5] {
    let o = &s.configs[c];
    let at_knee = o.knee.and_then(|k| s.points_of(c).nth(k));
    [
        places(o.cost_uh, 1_000_000, 3, sep),
        o.knee.and_then(|k| s.levels.get(k)).map_or(String::from("–"), level_text),
        places(o.max_achieved_m, 1000, 1, sep),
        at_knee.map_or(String::from("–"), |p| places(p.cost_per_1000_u, 1_000_000, 6, sep)),
        o.limit.clone(),
    ]
}

/// the page
pub(super) fn page(n: &mut Net, ui: &mut Ui) {
    refresh(n);
    let list = sources(n);
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_EX_CMP_SCROLL, SC_V | pad(8));
    ui.box_begin(0, BOX_V, 8);
    if list.is_empty() {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Noch nichts zu vergleichen: Der Vergleich zeigt den letzten Lauf und die behaltenen Läufe."));
        ui.box_end();
        ui.scroll_end();
        return;
    }
    // A and B
    let names: Vec<String> = list.iter().map(|s| source_name(n, *s)).collect();
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.text(TS_BODY | TX_SECONDARY, "A");
    let mut a = list.iter().position(|s| Some(*s) == n.exp.cmp.a).unwrap_or(0) as i32;
    ui.next_size(-1, 0);
    if ui.combobox(ID_EX_CMP_A, &mut a, &refs) & EV_ACT != 0 {
        n.exp.cmp.a = list.get(a as usize).copied();
        ui.repass();
    }
    ui.text(TS_BODY | TX_SECONDARY, "B");
    let mut items: Vec<&str> = alloc::vec![t("– keiner –")];
    items.extend(refs.iter().copied());
    let mut b = n.exp.cmp.b.and_then(|x| list.iter().position(|s| *s == x)).map_or(0, |k| k as i32 + 1);
    ui.next_size(-1, 0);
    if ui.combobox(ID_EX_CMP_B, &mut b, &items) & EV_ACT != 0 {
        n.exp.cmp.b = if b <= 0 { None } else { list.get(b as usize - 1).copied() };
        ui.repass();
    }
    if ui.button(ID_EX_CMP_CSV, t("CSV von A …")) & EV_ACT != 0 {
        n.exp.cmp.csv_of_done = false;
        let name = n.exp.cmp.a_sweep.as_ref().map_or(String::new(), |s| format!("{}.csv", s.name.trim_end_matches(".jfnex")));
        super::ask_file(n, super::FileFor::Csv, &name);
        ui.repass();
    }
    ui.box_end();
    refresh(n);
    let Some(sa) = n.exp.cmp.a_sweep.clone() else {
        ui.box_end();
        ui.scroll_end();
        return;
    };
    let sb = n.exp.cmp.b_sweep.clone();
    warnings(ui, &sa);
    table(n, ui, &sa, sb.as_ref());
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Das Knie ist die höchste Laststufe, bis zu der alle Stufen das SLO halten. „Begrenzt durch“ nennt, was an der ersten Stufe danach am stärksten belegt war."));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Kosten, Hardware und Dienstzeiten sind Annahmen des Modells, keine Preise und Messungen echter Systeme: Das Ergebnis zeigt die Form, nicht die Zahl."));
    charts(n, ui, &sa, sb.as_ref());
    // what it does not cover
    let mut open = n.exp.cmp.help;
    let shown = ui.expander_begin(ID_EX_CMP_HELP, t("Was das Modell nicht abdeckt"), &mut open) != 0;
    n.exp.cmp.help = open;
    if shown {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, t("Das Modell rechnet mit festen Kosten je Anfrage, CPU-Kernen mit Warteschlange, Sperren, Speicher, Platten und Kabeln. Es kennt keine Speicherbereinigung, keine CPU-Caches und NUMA, kein Thermal-Throttling, keine Abfragepläne und keine Ausfälle durch Fehler im Code. Ein Seed ist ein möglicher Verlauf, mehrere zeigen die Streuung. Ein echtes System knickt früher oder später ab: Vertrauen Sie der Reihenfolge der Varianten, nicht ihren absoluten Werten."));
    }
    ui.expander_end();
    ui.box_end();
    ui.scroll_end();
}

// the table per configuration: A, and under it B for the same name, the numbers one above the other
fn table(n: &mut Net, ui: &mut Ui, a: &Sweep, b: Option<&Sweep>) {
    let sep = ui.decimal_sep();
    let mut labels: Vec<String> = a.configs.iter().map(|c| c.label.clone()).collect();
    if let Some(s) = b {
        for c in &s.configs {
            if !labels.contains(&c.label) {
                labels.push(c.label.clone());
            }
        }
    }
    // the rows: the configuration, its run, its figures
    let row_of = |s: &Sweep, l: &str| s.configs.iter().position(|c| c.label == l).map(|c| cells(s, c, sep));
    let mut rows: Vec<(String, &str, [String; 5])> = Vec::new();
    for l in &labels {
        if let Some(r) = row_of(a, l) {
            rows.push((l.clone(), "A", r));
        }
        if let Some(r) = b.and_then(|s| row_of(s, l)) {
            rows.push((l.clone(), "B", r));
        }
    }
    rows.truncate(180);
    let right = GC_NOSORT | GC_RIGHT;
    let cols = [
        GridCol::new(t("Konfiguration"), 200).flags(GC_FILL | GC_NOSORT),
        GridCol::new(t("Lauf"), 44).flags(GC_NOSORT),
        GridCol::new(t("€/h"), 64).flags(right),
        GridCol::new(t("Knie"), 76).flags(right),
        GridCol::new(t("Maximum/s"), 84).flags(right),
        GridCol::new(t("€/1000 am Knie"), 110).flags(right),
        GridCol::new(t("begrenzt durch"), 130).flags(GC_NOSORT),
    ];
    let keys: Vec<u64> = (0..rows.len() as u64).collect();
    let st = &mut n.exp.cmp.grid;
    ui.next_size(-1, 34 + 26 * rows.len().clamp(2, 12) as i32);
    let (first, end) = ui.grid_begin(ID_EX_CMP_GRID, st, &cols, &keys);
    for i in first..end {
        let (l, run, r) = &rows[i as usize];
        ui.grid_row(st, i);
        ui.grid_text(st, 0, l);
        ui.grid_text(st, 1, run);
        for (k, x) in r.iter().enumerate() {
            ui.grid_text(st, 2 + k, x);
        }
    }
    let _ = ui.grid_end(st);
}

// the charts of a run
fn charts(n: &mut Net, ui: &mut Ui, a: &Sweep, b: Option<&Sweep>) {
    if a.configs.is_empty() {
        return;
    }
    // the configuration of the detail charts
    let names: Vec<&str> = a.configs.iter().map(|c| c.label.as_str()).collect();
    let mut cfg = n.exp.cmp.cfg.clamp(0, names.len() as i32 - 1);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.text(TS_BODY | TX_SECONDARY, t("Konfiguration der Latenzen"));
    ui.next_size(-1, 0);
    if ui.combobox(ID_EX_CMP_CFG, &mut cfg, &names) & EV_ACT != 0 {
        n.exp.cmp.cfg = cfg;
        ui.repass();
    }
    ui.box_end();
    if n.exp.cmp.curves.as_ref().is_none_or(|c| c.0 != cfg) {
        n.exp.cmp.curves = Some((cfg, curves_of(a, b, cfg as usize)));
    }
    let Some((_, c)) = n.exp.cmp.curves.as_ref() else { return };
    let w = ui.px2dip(ui.box_rect()[2]).max(CHART_MIN_W);
    let columns = (w / CHART_MIN_W).clamp(1, 3);
    let cw = ((w - 12 * (columns - 1)) / columns).max(CHART_MIN_W / 2);
    let knee_label = t("Knie");
    let marks_off: Vec<LcMarker> = c.knee_off.map(|x| LcMarker::line(x, knee_label)).into_iter().collect();
    let marks_lat: Vec<LcMarker> = c.knee_lat.map(|x| LcMarker::line(x, knee_label)).into_iter().collect();
    let crit = ui.color(T_CRIT);
    let limits: Vec<LcThreshold> = c.limits.iter().map(|(v, l)| LcThreshold::new(*v, l).color(crit)).collect();
    let open = a.levels.first().is_none_or(|l| matches!(l, jfn_model::experiment::Level::Rps(_)));
    let x_load = if open { t("x: angebotene Last (Anfragen/s)") } else { t("x: gesendete Anfragen/s") };
    let sat: Vec<LcSeries> = c.sat.iter().map(|(l, b)| LcSeries::new(l, b).unit(LcUnit::Num(""))).collect();
    let lat: Vec<LcSeries> = c.lat.iter().map(|(l, b)| LcSeries::new(l, b).unit(LcUnit::Secs)).collect();
    let util: Vec<LcSeries> = c.util.iter().map(|(l, b)| LcSeries::new(l, b).unit(LcUnit::Num("%"))).collect();
    let items: [(&str, &str, &str, LineChart); 3] = [
        (t("Sättigung"), x_load, t("Solange die Kurve der Linie „Angeboten“ folgt, kommt alles an. Wo sie flach wird, ist die Kapazität erreicht."), LineChart::new(&sat).x_axis(LX_NUM).name(t("Sättigung")).y_range(0.0, f64::NAN).markers(&marks_off)),
        (t("Latenz über Durchsatz"), t("x: erreichte Antworten/s"), t("Latenzen gelten nur für beantwortete Anfragen: Lesen Sie p99 zusammen mit den Fehlern, abgelaufene Anfragen haben keine Latenz."), LineChart::new(&lat).x_axis(LX_NUM).name(t("Latenz über Durchsatz")).y_range(0.0, f64::NAN).markers(&marks_lat).thresholds(&limits)),
        (t("Auslastung"), x_load, t("Was zuerst 100 % erreicht, begrenzt die Konfiguration."), LineChart::new(&util).x_axis(LX_NUM).name(t("Auslastung")).y_range(0.0, 100.0).markers(&marks_off)),
    ];
    let limit_text = c.limit_text.clone();
    for (r, row) in items.chunks(columns as usize).enumerate() {
        ui.box_begin(0, BOX_H, 12);
        for (j, (title, x, help, chart)) in row.iter().enumerate() {
            ui.next_size(cw, 0);
            ui.box_begin(0, BOX_V, 4);
            ui.text(TS_BODY_STRONG, title);
            ui.text(TS_CAPTION | TX_TERTIARY, x);
            ui.next_size(-1, CHART_H);
            let _ = ui.line_chart(ID_EX_CMP_CHART + (r * columns as usize + j) as u32, chart);
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, help);
            ui.box_end();
        }
        ui.box_end();
    }
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, &tf("Begrenzt durch: {}", &[&limit_text]));
}
