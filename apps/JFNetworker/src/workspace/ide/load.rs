// load -- the IDE's pane Lasttest (A2): T2's load generator on this host against a service, its
// numbers beside the form as the Metriken panel draws them, a summary at the end
//
//   LoadUi::new()              the form's choices and what the charts read
//   view(n, ui, k, cx, acts)   the form on the left (narrow: above); on the right the state, the
//                              tiles over the last 5 s (requests per second, p50 / p95 / p99, error
//                              rate), the charts of the series (D2) and, when the run ended, the
//                              summary
//   start(n, k, cx)            Starten: Action::Service with an http-load named NAME (it replaces
//                              the last one: fresh numbers)
//   stop(n, cx)                Stoppen: Action::ServiceRun
//   prepare(n, k, cx)          the first time it opens: the address of the project's load balancer,
//                              else of the hosts of the last deployment
//
// The load generator is a service of the engine only, not of the project: it is a run, not a part
// of the design, and it goes with a restart of the device.  It shows in the Metriken panel as well.
//
// Ids from a2 = ID_A20 + 0x800 k: + 0x200 the form's scroll, + 0x201 the URL, + 0x203 the rate,
// + 0x204 the users, + 0x205 the think time, + 0x206 the duration, + 0x207 Starten, + 0x208 Stoppen,
// + 0x209 and + 0x20B the split, + 0x20A the charts' scroll, + 0x210 the kind of load (.. + 0x212),
// + 0x220 ..
// the charts.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::ServiceDef;
use jfn_sim::{Action, Cmd, Kpi, LoadView, SEC};
use ui::*;

use super::{Act, Cx};
use crate::app::constants::*;
use crate::app::tr::{self, t, tc, tf};
use crate::app::Net;
use crate::workspace::metrics;

/// the service of a run
pub(crate) const NAME: &str = "lasttest";
const FORM_W: i32 = 300;
const LABEL_W: i32 = 96;
const TILE_W: i32 = 88;
const CHART_MIN_W: i32 = 260;
const CHART_H: i32 = 160;
/// the charts' span
const SPAN: u64 = 60;

/// what the charts show, and what it was read for
struct Data {
    key: (u32, u64),
    series: Vec<(&'static str, SeriesBuf)>,
    kpi: Option<Kpi>,
}

pub(crate) struct LoadUi {
    pub(crate) url: String,
    /// 0: open (a rate), 1: closed (users)
    pub(crate) mode: i32,
    pub(crate) rate: i32,
    pub(crate) users: i32,
    pub(crate) think_ms: i32,
    pub(crate) dur_s: i32,
    pub(crate) msg: Option<String>,
    inited: bool,
    /// the duration of the run shown
    run_s: i32,
    data: Option<Data>,
}

impl LoadUi {
    pub(crate) fn new() -> LoadUi {
        LoadUi { url: String::new(), mode: 0, rate: 100, users: 10, think_ms: 0, dur_s: 30, msg: None, inited: false, run_s: 0, data: None }
    }
}

/// the first time the pane opens: where the load should go
pub(crate) fn prepare(n: &mut Net, k: usize, cx: &Cx) {
    if n.devwin.get(k).is_none_or(|w| w.ide.load.inited) {
        return;
    }
    let e = n.doc.sim.engine();
    let lb = n.doc.proj.devices.iter().find_map(|d| d.services.iter().find(|s| s.kind == "lb").map(|s| (d.id, s.config.get("port").and_then(|p| p.as_u64()).unwrap_or(80))));
    let url = match lb {
        Some((d, port)) => e.deploy_addr(d).map(|a| format!("http://{}:{}/", a, port)),
        None => e.deployments(cx.dev).last().and_then(|r| r.hosts.first()).and_then(|h| e.deploy_addr(h.dev)).map(|a| format!("http://{}/", a)),
    };
    if let Some(w) = n.devwin.get(k) {
        w.ide.load.inited = true;
        w.ide.load.url = url.unwrap_or_else(|| String::from("http://"));
    }
}

/// Starten
pub(crate) fn start(n: &mut Net, k: usize, cx: &Cx) -> Result<(), String> {
    let Some(w) = n.devwin.get(k) else { return Ok(()) };
    let l = &mut w.ide.load;
    l.msg = None;
    let url = String::from(l.url.trim());
    if !url.starts_with("http://") || url.len() <= "http://".len() {
        return Err(String::from(t("Das Ziel ist eine URL wie http://10.0.1.1/.")));
    }
    let n_i = |v: i32| Value::i64(v.max(0) as i64);
    let mut config = Value::object([("targets", Value::Array(alloc::vec![Value::string(&url)])), ("duration_ms", n_i(l.dur_s * 1000))]);
    if l.mode == 0 {
        config.insert("mode", Value::string("open"));
        config.insert("rps", n_i(l.rate));
    } else {
        config.insert("mode", Value::string("closed"));
        config.insert("users", n_i(l.users));
        config.insert("think_ms", n_i(l.think_ms));
    }
    l.run_s = l.dur_s;
    let def = ServiceDef { kind: String::from("http-load"), name: String::from(NAME), disabled: false, config };
    n.doc.sim.send(Cmd::Apply(Action::Service { dev: cx.dev, def })).map(|_| ()).map_err(|e| tr::msg(&e.msg()))
}

/// Stoppen
pub(crate) fn stop(n: &mut Net, cx: &Cx) -> Result<(), String> {
    n.doc.sim.send(Cmd::Apply(Action::ServiceRun { dev: cx.dev, name: String::from(NAME), on: false })).map(|_| ()).map_err(|e| tr::msg(&e.msg()))
}

// a row: its label, then what follows up to box_end
fn row(ui: &mut Ui, label: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    ui.next_size(LABEL_W, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, label);
}

/// the pane
pub(crate) fn view(n: &mut Net, ui: &mut Ui, k: usize, cx: &Cx, acts: &mut Vec<Act>) {
    prepare(n, k, cx);
    let a2 = ID_A20 + 0x800 * k as u32;
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    let w = ui.px2dip(ui.box_rect()[2]);
    if w >= FORM_W + 320 || w == 0 {
        let mut pos = FORM_W;
        ui.next_size(-1, -1);
        ui.split_begin(a2 + 0x209, pos, false);
        form(n, ui, k, a2, cx, acts);
        ui.split_next();
        numbers(n, ui, k, a2, cx);
        ui.split_end(&mut pos);
    } else {
        // narrow: one above the other
        let mut pos = 170;
        ui.next_size(-1, -1);
        ui.split_begin(a2 + 0x20B, pos, true);
        form(n, ui, k, a2, cx, acts);
        ui.split_next();
        numbers(n, ui, k, a2, cx);
        ui.split_end(&mut pos);
    }
    ui.box_end();
}

// the form
fn form(n: &mut Net, ui: &mut Ui, k: usize, a2: u32, cx: &Cx, acts: &mut Vec<Act>) {
    let running = n.doc.sim.engine().http_load_view(cx.dev, NAME, 0).is_some_and(|v| v.running);
    let Some(w) = n.devwin.get(k) else { return };
    let l = &mut w.ide.load;
    ui.next_size(-1, -1);
    ui.scroll_begin(a2 + 0x200, SC_V);
    ui.box_begin(0, BOX_V | pad(4), 6);
    row(ui, t("Ziel"));
    ui.next_size(-1, 0);
    ui.textbox(a2 + 0x201, Some(t("z. B. http://10.0.1.1/")), &mut l.url, 400);
    ui.box_end();
    row(ui, t("Last"));
    ui.next_flags(ST_COMPACT);
    if ui.segmented(a2 + 0x210, &mut l.mode, &[t("Offen"), t("Geschlossen")]) & EV_ACT != 0 {
        ui.repass();
    }
    ui.tooltip(t("Offen: Anfragen kommen in fester Rate, egal wie schnell die Antworten sind. Geschlossen: Nutzer warten auf ihre Antwort und denken, bevor sie wieder fragen."));
    ui.box_end();
    if l.mode == 0 {
        row(ui, t("Anfragen/s"));
        ui.next_size(96, 0);
        ui.number_box(a2 + 0x203, &mut l.rate, &Range::new(1, 100_000, 10, 0));
        ui.box_end();
    } else {
        row(ui, t("Nutzer"));
        ui.next_size(96, 0);
        ui.number_box(a2 + 0x204, &mut l.users, &Range::new(1, 10_000, 1, 0));
        ui.box_end();
        row(ui, t("Denkzeit (ms)"));
        ui.next_size(96, 0);
        ui.number_box(a2 + 0x205, &mut l.think_ms, &Range::new(0, 600_000, 100, 0));
        ui.box_end();
    }
    row(ui, t("Dauer (s)"));
    ui.next_size(96, 0);
    ui.number_box(a2 + 0x206, &mut l.dur_s, &Range::new(1, 3600, 5, 0));
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_icon(IC_PLAY);
    ui.next_flags(if running || cx.d.off { ST_DISABLED } else { ST_ACCENT });
    if ui.button(a2 + 0x207, t("Starten")) & EV_ACT != 0 {
        acts.push(Act::LoadStart);
    }
    ui.next_icon(IC_STOP);
    ui.next_flags(if running { 0 } else { ST_DISABLED });
    if ui.button(a2 + 0x208, t("Stoppen")) & EV_ACT != 0 {
        acts.push(Act::LoadStop);
    }
    ui.box_end();
    if let Some(m) = &l.msg {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_WRAP, m);
    }
    ui.box_end();
    ui.scroll_end();
}

// what the charts and tiles show, read again when points came
fn refresh(n: &mut Net, k: usize, dev: u32) {
    let eng = n.doc.sim.engine();
    let key = (dev, eng.series_gen());
    if n.devwin.peek(k).is_some_and(|w| w.ide.load.data.as_ref().is_some_and(|d| d.key == key)) {
        return;
    }
    let mut series = Vec::new();
    for v in eng.series(dev, NAME) {
        let lat = matches!(v.kind, jfn_sim::SeriesKind::Lat(_)) && v.name.ends_with("_us");
        let mut b = SeriesBuf::new(v.points.len().max(1));
        for &(x, y) in &v.points {
            b.push(x as f64, if lat { y as f64 * 1e-6 } else { y as f64 });
        }
        series.push((v.name, b));
    }
    let kpi = eng.kpi(dev, NAME, metrics::KPI_SPAN);
    if let Some(w) = n.devwin.get(k) {
        w.ide.load.data = Some(Data { key, series, kpi });
    }
}

/// the summary of a run that ended
fn summary(v: &LoadView, secs: i32, sep: u32) -> String {
    let lat = |x: Option<u64>| x.map_or_else(|| String::from("–"), |ns| LcUnit::Secs.text(ns as f64 * 1e-9, sep));
    let errors = v.http_errors + v.failed;
    let per = if secs > 0 { LcUnit::Num("").text(v.responses as f64 / secs as f64, sep) } else { String::from("–") };
    tf("Fertig: {} Anfragen, {} Antworten ({}/s), {} Fehler; p50 {}, p95 {}, p99 {}", &[&v.sent.to_string(), &v.responses.to_string(), &per, &errors.to_string(), &lat(v.p50), &lat(v.p95), &lat(v.p99)])
}

// the state, the tiles, the charts
fn numbers(n: &mut Net, ui: &mut Ui, k: usize, a2: u32, cx: &Cx) {
    let dev = cx.dev;
    let view = n.doc.sim.engine().http_load_view(dev, NAME, 0);
    let run_s = n.devwin.peek(k).map_or(0, |w| w.ide.load.run_s);
    ui.next_size(-1, -1);
    let Some(v) = view else {
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(8), 0);
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP | TX_CENTER, t("Hier stehen Anfragen pro Sekunde, Latenzen und Fehler des Lasttests, sobald er läuft."));
        ui.box_end();
        return;
    };
    refresh(n, k, dev);
    let sep = ui.decimal_sep();
    ui.scroll_begin(a2 + 0x20A, SC_V | pad(4));
    ui.box_begin(0, BOX_V, 8);
    let w = ui.px2dip(ui.box_rect()[2]).max(TILE_W);
    // the state
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.status_dot(if v.running { BADGE_OK } else { BADGE_NEUTRAL });
    let what = if v.running { tf("läuft: {} Anfragen gesendet, {} Antworten", &[&v.sent.to_string(), &v.responses.to_string()]) } else { summary(&v, run_s, sep) };
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_WRAP, &what);
    ui.box_end();
    let Some(dw) = n.devwin.peek(k) else {
        ui.box_end();
        ui.scroll_end();
        return;
    };
    let Some(data) = dw.ide.load.data.as_ref() else {
        ui.box_end();
        ui.scroll_end();
        return;
    };
    // the tiles
    let tiles = data.kpi.as_ref().map(|k| metrics::kpi_tiles(k, None, sep)).unwrap_or_default();
    let per = (w / (TILE_W + 8)).max(1) as usize;
    for chunk in tiles.chunks(per) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        for (label, value) in chunk {
            ui.next_size(TILE_W, 0);
            ui.box_begin(0, BOX_V | BOX_CARD | pad(6), 0);
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, label);
            ui.text(TS_BODY_STRONG, value);
            ui.box_end();
        }
        ui.box_end();
    }
    // the charts, as the Metriken panel draws them
    let groups = metrics::groups("http-load");
    let shown: Vec<(&metrics::Group, Vec<usize>)> = groups
        .iter()
        .filter_map(|g| {
            let idx: Vec<usize> = g.names.iter().filter_map(|nm| data.series.iter().position(|s| s.0 == *nm)).collect();
            (!idx.is_empty()).then_some((g, idx))
        })
        .collect();
    let cols = (w / CHART_MIN_W).clamp(1, 3);
    let cw = ((w - 12 * (cols - 1)) / cols).max(160);
    for (r, line) in shown.chunks(cols as usize).enumerate() {
        ui.box_begin(0, BOX_H, 12);
        for (j, (g, idx)) in line.iter().enumerate() {
            let names: Vec<String> = idx.iter().map(|&i| metrics::series_label(data.series[i].0)).collect();
            let series: Vec<LcSeries> = idx.iter().zip(names.iter()).map(|(&i, l)| LcSeries::new(l, &data.series[i].1).unit(g.show.unit())).collect();
            let title = tc(g.title);
            let chart = LineChart::new(&series).x_axis(LX_NANOS).name(title).y_range(0.0, f64::NAN).follow((SPAN * SEC) as f64);
            ui.next_size(cw, 0);
            ui.box_begin(0, BOX_V, 4);
            ui.text(TS_BODY_STRONG, title);
            ui.next_size(-1, CHART_H);
            let _ = ui.line_chart(a2 + 0x220 + (r * cols as usize + j) as u32, &chart);
            ui.box_end();
        }
        ui.box_end();
    }
    ui.box_end();
    ui.scroll_end();
}
