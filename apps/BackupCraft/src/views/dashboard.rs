// dashboard -- the overview: KPI tiles with trends, sessions of the last 14 days, storage,
// what needs attention, running operations, the next runs

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::model::*;
use ui::*;

use crate::app::{Console, Page, ID_DASH};
use crate::icons::*;
use crate::tr::{t, tf};
use crate::{fmt, views, widgets, wizards};

const ID_BARS: u32 = ID_DASH + 100;
const ID_DONUT: u32 = ID_DASH + 200;
const ID_ALERT: u32 = ID_DASH + 300;
const ID_NEXT: u32 = ID_DASH + 400;
const ID_OPS: u32 = ID_DASH + 500;

pub fn frame(app: &mut Console, ui: &mut Ui) {
    let now = ui.now();
    for (p, per) in [("/api/summary", 5000), ("/api/repositories", 15_000), ("/api/operations", 3000), ("/api/jobs", 10_000)] {
        app.net.want(p, per, now);
    }
    let epoch = app.epoch(ui);
    views::scroll_body_begin(ui, ID_DASH + 1);
    let s = app.net.data.summary.clone();
    let server = s.as_ref().map_or(String::new(), |s| s.server_name.clone());
    widgets::page_head(ui, t("Übersicht"), &tf("{} · {} Jobs · {} von {} Hosts online", &[
        &server,
        &format!("{}", s.as_ref().map_or(0, |s| s.jobs)),
        &format!("{}", s.as_ref().map_or(0, |s| s.hosts_online)),
        &format!("{}", s.as_ref().map_or(0, |s| s.hosts)),
    ]));
    if app.may(Role::Operator) {
        ui.next_icon(IC_ADD);
        if ui.accent_button(ID_DASH + 2, t("Neuer Job")) & EV_ACT != 0 {
            wizards::open_named(app, ui, "job");
        }
    }
    widgets::page_head_end(ui);
    let Some(s) = s else {
        widgets::loading(ui, ID_DASH + 3);
        views::scroll_body_end(ui);
        return;
    };
    // KPI tiles
    ui.box_begin(0, BOX_H, 16);
    let prot: Vec<f32> = s.protected_history.iter().map(|b| *b as f32).collect();
    let ok: Vec<f32> = s.history.iter().map(|d| d.ok as f32).collect();
    let fail: Vec<f32> = s.history.iter().map(|d| (d.fail + d.warn) as f32).collect();
    let fill: Vec<f32> = (0..14).map(|i| s.storage_used as f32 * (0.86 + i as f32 * 0.01)).collect();
    ui.next_size(-1, 0);
    if widgets::kpi(ui, ID_DASH + 10, BC_SHIELD_CHECK, t("Geschützte Objekte"), &fmt::count(s.protected_items as u64),
                    &tf("{} Restore-Points", &[&format!("{}", s.points)]), &prot, 0) & EV_ACT != 0 {
        app.go(ui, Page::Jobs);
    }
    ui.next_size(-1, 0);
    let total24 = s.ok_24h + s.warn_24h + s.fail_24h;
    if widgets::kpi(ui, ID_DASH + 20, IC_CHECK_CIRCLE_FILL, t("Erfolgreich (24 h)"), &format!("{}", s.ok_24h),
                    &tf("von {} Sitzungen", &[&format!("{}", total24)]), &ok, ui.color(T_OK)) & EV_ACT != 0 {
        app.go(ui, Page::Monitoring);
    }
    ui.next_size(-1, 0);
    if widgets::kpi(ui, ID_DASH + 30, IC_WARNING, t("Fehler und Warnungen (24 h)"), &format!("{}", s.fail_24h + s.warn_24h),
                    &tf("{} fehlgeschlagen · {} mit Warnung", &[&format!("{}", s.fail_24h), &format!("{}", s.warn_24h)]), &fail,
                    ui.color(if s.fail_24h > 0 { T_CRIT } else { T_WARN })) & EV_ACT != 0 {
        app.go(ui, Page::Monitoring);
    }
    ui.next_size(-1, 0);
    // the disks under the repositories (other data on them takes room too), from the
    // repositories' figures: an older server's summary has no storage_free
    let (cap, free) = bc_proto::model::disks(&app.net.data.repos);
    let filled = cap.saturating_sub(free);
    if widgets::kpi(ui, ID_DASH + 40, BC_REPOSITORY, t("Speicherbelegung"), &fmt::percent(filled, cap),
                    &tf("{} frei von {} · Ersparnis {}", &[&fmt::size(free), &fmt::size(cap),
                        &fmt::ratio((s.storage_logical as u128 * 100 / s.storage_used.max(1) as u128) as u32)]), &fill,
                    ui.color(T_WARN)) & EV_ACT != 0 {
        app.go(ui, Page::Repositories);
    }
    ui.box_end();
    // charts
    ui.box_begin(0, BOX_H, 16);
    ui.next_size(-2, 0);
    widgets::card_begin(ui, t("Sitzungen der letzten 14 Tage"), t("Sicherungen, Kopien und Prüfungen pro Tag"));
    let labels: Vec<String> = s.history.iter().map(|d| {
        let c = app.zone.to_local(datetime::Timestamp::from_secs(d.day - 1));
        format!("{}.{}", c.day, c.month)
    }).collect();
    let lref: Vec<&str> = labels.iter().map(|x| x.as_str()).collect();
    let warn: Vec<f32> = s.history.iter().map(|d| d.warn as f32).collect();
    let failv: Vec<f32> = s.history.iter().map(|d| d.fail as f32).collect();
    let (cok, cwarn, cfail) = (ui.color(T_OK), ui.color(T_WARN), ui.color(T_CRIT));
    ui.next_size(-1, 200);
    ui.bar_chart(ID_BARS, &lref, &[
        BarSeries { label: t("Erfolgreich"), values: &ok, color: cok },
        BarSeries { label: t("Warnung"), values: &warn, color: cwarn },
        BarSeries { label: t("Fehler"), values: &failv, color: cfail },
    ], true);
    ui.legend(&[(t("Erfolgreich"), cok), (t("Warnung"), cwarn), (t("Fehler"), cfail)]);
    widgets::card_end(ui);
    ui.next_size(-1, 0);
    widgets::card_begin(ui, t("Speicher"), &tf("{} Repositories", &[&format!("{}", app.net.data.repos.len())]));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
    let other = filled.saturating_sub(s.storage_used);
    let mut slices: Vec<DonutSlice> = app.net.data.repos.iter().map(|r| DonutSlice { label: &r.name, value: r.used as f32, color: 0 }).collect();
    if other > 0 {
        slices.push(DonutSlice { label: t("Andere Daten"), value: other as f32, color: (ui.color(T_TEXT3) & 0xFFFFFF) | 0x90000000 });
    }
    slices.push(DonutSlice { label: t("Frei"), value: free as f32, color: (ui.color(T_TEXT3) & 0xFFFFFF) | 0x40000000 });
    ui.next_size(150, 150);
    ui.donut(ID_DONUT, &slices, &fmt::percent(filled, cap), t("belegt"));
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, 8);
    let repos = app.net.data.repos.clone();
    for (i, r) in repos.iter().enumerate() {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_ELLIPSIS, &r.name);
        ui.text(TS_CAPTION | TX_SECONDARY, &tf("{} frei", &[&fmt::size(r.free)]));
        ui.box_end();
        ui.next_size(-1, 6);
        ui.meter(ID_DONUT + 10 + i as u32, r.disk_used() as i64, r.capacity as i64, fmt::fill_kind(r.disk_used(), r.capacity));
    }
    ui.box_end();
    ui.box_end();
    widgets::card_end(ui);
    ui.box_end();
    // lists
    ui.box_begin(0, BOX_H, 16);
    ui.next_size(-1, 0);
    let all = widgets::card_begin_action(ui, t("Aufmerksamkeit erforderlich"), ID_ALERT, t("Monitoring"));
    if all {
        app.go(ui, Page::Monitoring);
    }
    if s.alerts.is_empty() {
        ui.box_begin(0, BOX_H | BOX_XCENTER | pad(8), 10);
        ui.icon(BC_SHIELD_CHECK, 20);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Alles in Ordnung. Keine Fehler, keine Warnungen."));
        ui.box_end();
    }
    for (i, a) in s.alerts.iter().take(5).enumerate() {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
        ui.status_dot(fmt::severity(a.severity));
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &a.title);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &format!("{} · {}", a.detail, fmt::when_short(a.at, epoch, &app.zone)));
        ui.box_end();
        if ui.link(ID_ALERT + 10 + i as u32, t("Öffnen"), TS_BODY) & EV_ACT != 0 {
            let page = match a.target_kind.as_str() {
                "repository" => Page::Repositories,
                "host" => Page::Hosts,
                "fleet" => Page::Monitoring,
                "update" => {
                    app.settings.cat = 6;
                    Page::Settings
                }
                _ => Page::Jobs,
            };
            let id = a.target_id.clone();
            app.go(ui, page);
            views::select(app, page, &id);
        }
        ui.box_end();
    }
    widgets::card_end(ui);
    ui.next_size(-1, 0);
    widgets::card_begin(ui, t("Laufende Vorgänge"), "");
    let running: Vec<Operation> = app.net.data.running_ops().cloned().collect();
    if running.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("Gerade läuft nichts."));
    }
    for (i, o) in running.iter().take(4).enumerate() {
        ui.box_begin(0, BOX_V, 4);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &o.title);
        ui.text(TS_CAPTION | TX_SECONDARY, &fmt::percent(o.done, o.total));
        ui.box_end();
        ui.next_size(-1, 0);
        ui.progress(ID_OPS + i as u32, (o.done.min(o.total) * 1000 / o.total.max(1)) as i32, 1000);
        ui.box_end();
    }
    widgets::card_end(ui);
    ui.next_size(-1, 0);
    widgets::card_begin(ui, t("Nächste Läufe"), "");
    if s.next_runs.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("Keine geplanten Läufe."));
    }
    for (i, n) in s.next_runs.iter().enumerate() {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
        ui.icon(BC_CLOCK, 16);
        if ui.link(ID_NEXT + i as u32, &n.job_name, TS_BODY) & EV_ACT != 0 {
            let id = n.job_id.clone();
            app.go(ui, Page::Jobs);
            views::select(app, Page::Jobs, &id);
        }
        ui.spacer(-1);
        ui.text(TS_CAPTION | TX_SECONDARY, &fmt::when_short(n.at, epoch, &app.zone));
        ui.box_end();
    }
    widgets::card_end(ui);
    ui.box_end();
    views::scroll_body_end(ui);
}
