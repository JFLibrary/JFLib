// define -- the page Aufbau of the panel Experimente: an experiment without JSON
//
//   page(n, ui)     name and title, the load, the times, the SLO, the axes every series has, the series
//                   with their axes, the prices and what does not count for the cost; the findings of
//                   Experiment::problems, live and with their exact words
//
// An axis has its kind (Hardware, Einstellung, Backends) and its own part: hardware of a device or a group
// with a row per value; a setting by its service and its name (jfn_model::experiment::known_keys) with
// the values in one field; the backends of a load balancer, the first N of a pool or copies of a device.
// A value's other members stay.  Widgets: an axis has a block of ID_EX_AXIS_STEP ids (ID_EX_AXIS + k *
// step); at most AXES_MAX axes, HW_MAX values of hardware and POOL_MAX hosts of a pool show (the others
// stay in the experiment).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::catalog::{self, Prices};
use jfn_model::experiment::{fixed, known_keys, set_path, Axis, Series};
use jfn_model::Project;
use ui::*;

use super::{parse_numbers, parse_values, value_of, value_text, Exp};
use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;

/// what shows at most (their ids)
const AXES_MAX: usize = 16;
const HW_MAX: usize = 8;
const POOL_MAX: usize = 32;
const SERIES_MAX: usize = 16;
/// the width of a label (DIP)
const LABEL_W: i32 = 140;

/// a field's text; on a change the new text (the buffer fills from `now` while it has none)
fn tbox(st: &mut Exp, ui: &mut Ui, id: u32, hint: Option<&str>, now: &str) -> Option<String> {
    let buf = st.bufs.entry(id).or_insert_with(|| String::from(now));
    if ui.textbox(id, hint, buf, 400) & EV_ACT != 0 {
        st.typing = id;
        return Some(buf.clone());
    }
    None
}

/// the label of a row with its control
fn row_begin(ui: &mut Ui, label: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(LABEL_W, 0);
    ui.text(TS_BODY | TX_SECONDARY, label);
}

/// a field in its row, its unit after it
fn text_row(st: &mut Exp, ui: &mut Ui, id: u32, label: &str, unit: &str, hint: Option<&str>, now: &str) -> Option<String> {
    row_begin(ui, label);
    ui.next_size(-1, 0);
    let out = tbox(st, ui, id, hint, now);
    if !unit.is_empty() {
        ui.next_size(60, 0);
        ui.text(TS_BODY | TX_SECONDARY, unit);
    }
    ui.box_end();
    field_err(st, ui, id);
    out
}

// what the field's text cannot be
fn field_err(st: &Exp, ui: &mut Ui, id: u32) {
    if let Some(e) = st.errs.get(&id) {
        crate::workspace::error_text(ui, e, 0);
    }
}

/// a choice among names: the one that stands there comes along when the list does not have it
fn name_combo(st: &mut Exp, ui: &mut Ui, id: u32, names: &[String], now: &str) -> Option<String> {
    let mut items: Vec<&str> = alloc::vec![t("– wählen –")];
    items.extend(names.iter().map(|s| s.as_str()));
    if !now.is_empty() && !names.iter().any(|s| s == now) {
        items.push(now);
    }
    let mut sel = items.iter().skip(1).position(|s| *s == now).map_or(0, |k| k as i32 + 1);
    if ui.combobox(id, &mut sel, &items) & EV_ACT != 0 {
        st.chosen = true;
        return Some(if sel <= 0 { String::new() } else { String::from(items[sel as usize]) });
    }
    None
}

/// the list of numbers of a field: its numbers on a change, what is wrong stays at the field
fn number_row(st: &mut Exp, ui: &mut Ui, id: u32, label: &str, hint: &str, now: &[u64]) -> Option<Vec<u64>> {
    let text = now.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(", ");
    let changed = text_row(st, ui, id, label, "", Some(hint), &text)?;
    match parse_numbers(&changed) {
        Ok(v) => {
            st.errs.remove(&id);
            Some(v)
        }
        Err(e) => {
            st.errs.insert(id, e);
            None
        }
    }
}

/// one whole number of a field
fn int_row(st: &mut Exp, ui: &mut Ui, id: u32, label: &str, unit: &str, now: u64) -> Option<u64> {
    let changed = text_row(st, ui, id, label, unit, None, &now.to_string())?;
    match changed.trim().parse::<u64>() {
        Ok(v) => {
            st.errs.remove(&id);
            Some(v)
        }
        Err(_) if changed.trim().is_empty() => {
            st.errs.remove(&id);
            Some(0)
        }
        Err(_) => {
            st.errs.insert(id, tf("„{}“ ist keine ganze Zahl.", &[changed.trim()]));
            None
        }
    }
}

/// a limit of the SLO as a field shows it
fn limit_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        x => value_text(x),
    }
}

/// a limit from a field: empty is none, what is no number stays a text so that the finding names it
fn limit_of(text: &str) -> Value {
    if text.trim().is_empty() { Value::Null } else { value_of(text) }
}

/// the devices that run a service of this kind
fn devices_with(p: &Project, kind: &str) -> Vec<String> {
    p.devices.iter().filter(|d| d.services.iter().any(|s| s.kind == kind)).map(|d| d.name.clone()).collect()
}

/// the names of a device's services of a kind (any: all of them)
fn services_on(p: &Project, dev: &str, kind: Option<&str>) -> Vec<String> {
    p.find(dev).map_or(Vec::new(), |d| d.services.iter().filter(|s| kind.is_none_or(|k| s.kind == k)).map(|s| s.name.clone()).collect())
}

/// the page
pub(super) fn page(n: &mut Net, ui: &mut Ui) {
    let Some(mut e) = n.exp.draft.clone() else {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Links ein Experiment wählen oder ein neues anlegen."));
        return;
    };
    let before = e.clone();
    let found: Vec<String> = e.problems(&n.doc.proj).iter().map(tr::msg).collect();
    let runs = e.configs().len() * e.levels().len() * e.seeds_or(n.doc.proj.seed).len();
    let (st, p) = (&mut n.exp, &n.doc.proj);
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_EX_DEF_SCROLL, SC_V | pad(8));
    ui.box_begin(0, BOX_V, 8);
    // what keeps it from running, first: it is what to do next
    findings(ui, &found, runs);
    // name and title
    if let Some(v) = text_row(st, ui, ID_EX_NAME, t("Name"), "", None, &e.name) {
        e.name = String::from(v.trim());
    }
    if let Some(v) = text_row(st, ui, ID_EX_TITLE, t("Titel"), "", Some(t("Wofür es gut ist")), &e.title) {
        e.title = v;
    }
    // the load
    ui.text(TS_BODY_STRONG, t("Last"));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Ein Lastgenerator des Projekts läuft jede Konfiguration durch diese Stufen. Sie müssen streng steigen."));
    row_begin(ui, t("Generator"));
    ui.next_size(-1, 0);
    if let Some(v) = name_combo(st, ui, ID_EX_LOAD_DEV, &devices_with(p, "http-load"), &e.load.dev) {
        e.load.svc = services_on(p, &v, Some("http-load")).into_iter().next().unwrap_or_default();
        e.load.dev = v;
    }
    ui.next_size(-1, 0);
    if let Some(v) = name_combo(st, ui, ID_EX_LOAD_SVC, &services_on(p, &e.load.dev, Some("http-load")), &e.load.svc) {
        e.load.svc = v;
    }
    ui.box_end();
    let users = e.load.rps.is_empty() && !e.load.users.is_empty();
    let mut kind = users as i32;
    row_begin(ui, t("Art der Stufen"));
    ui.next_flags(ST_COMPACT);
    if ui.segmented(ID_EX_LOAD_KIND, &mut kind, &[t("Anfragen/s (offen)"), t("Nutzer (geschlossen)")]) & EV_ACT != 0 {
        st.chosen = true;
        if kind == 1 {
            e.load.users = e.load.rps.iter().map(|&r| r as u32).collect();
            e.load.rps.clear();
        } else {
            e.load.rps = e.load.users.iter().map(|&u| u as u64).collect();
            e.load.users.clear();
        }
        st.bufs.remove(&ID_EX_LEVELS);
    }
    ui.box_end();
    let now: Vec<u64> = if kind == 1 { e.load.users.iter().map(|&u| u as u64).collect() } else { e.load.rps.clone() };
    if let Some(v) = number_row(st, ui, ID_EX_LEVELS, t("Stufen"), "100, 400, 1600", &now) {
        if kind == 1 {
            e.load.users = v.iter().map(|&u| u as u32).collect();
        } else {
            e.load.rps = v;
        }
    }
    // the times
    ui.text(TS_BODY_STRONG, t("Zeit (virtuell)"));
    if let Some(v) = int_row(st, ui, ID_EX_WARMUP, t("Warmlauf"), "s", e.timing.warmup_s) {
        e.timing.warmup_s = v;
    }
    if let Some(v) = int_row(st, ui, ID_EX_SETTLE, t("Übergang je Stufe"), "s", e.timing.settle_s) {
        e.timing.settle_s = v;
    }
    if let Some(v) = int_row(st, ui, ID_EX_MEASURE, t("Messung je Stufe"), "s", e.timing.measure_s) {
        e.timing.measure_s = v;
    }
    if let Some(v) = number_row(st, ui, ID_EX_SEEDS, t("Seeds"), t("leer: der des Projekts"), &e.timing.seeds) {
        e.timing.seeds = v;
    }
    // the SLO
    ui.text(TS_BODY_STRONG, t("Was als gut genug gilt (SLO)"));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Das Knie ist die höchste Stufe, bis zu der alle Stufen diese Grenzen halten. Leer heißt keine Grenze; ohne jede Grenze gibt es kein Knie."));
    for (k, (label, unit)) in [("p50", "ms"), ("p95", "ms"), ("p99", "ms"), ("p99.9", "ms"), (t("Fehler"), "%")].into_iter().enumerate() {
        let v = match k {
            0 => &mut e.slo.p50_ms,
            1 => &mut e.slo.p95_ms,
            2 => &mut e.slo.p99_ms,
            3 => &mut e.slo.p999_ms,
            _ => &mut e.slo.errors_pct,
        };
        if let Some(text) = text_row(st, ui, ID_EX_SLO + k as u32, label, unit, None, &limit_text(v)) {
            *v = limit_of(&text);
        }
    }
    // the axes
    let mut block = 0usize;
    ui.text(TS_BODY_STRONG, t("Achsen aller Reihen"));
    axes(st, ui, p, &mut e.axes, &mut block, ID_EX_ADD_AXIS);
    ui.text(TS_BODY_STRONG, t("Reihen"));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Eine Reihe ist eine Familie von Konfigurationen, etwa „größer“ und „mehr“; jede bekommt das Kreuzprodukt ihrer Achsen."));
    let mut gone = None;
    for (s, series) in e.series.iter_mut().enumerate().take(SERIES_MAX) {
        let sid = ID_EX_SERIES + s as u32 * 8;
        ui.box_begin(0, BOX_V | BOX_CARD | pad(8), 6);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(LABEL_W, 0);
        ui.text(TS_BODY | TX_SECONDARY, t("Name der Reihe"));
        ui.next_size(-1, 0);
        if let Some(v) = tbox(st, ui, sid, None, &series.name) {
            series.name = v;
        }
        ui.next_flags(ST_SUBTLE | ST_COMPACT | ST_ICON_ONLY);
        ui.next_icon(IC_DELETE);
        if ui.button(sid + 1, &tf("Reihe {} entfernen", &[&series.name])) & EV_ACT != 0 {
            gone = Some(s);
        }
        ui.box_end();
        axes(st, ui, p, &mut series.axes, &mut block, sid + 2);
        ui.box_end();
    }
    if let Some(s) = gone {
        e.series.remove(s);
        st.chosen = true;
        st.fresh_fields();
    }
    ui.next_flags(ST_SUBTLE | ST_COMPACT);
    ui.next_icon(IC_ADD);
    if ui.button(ID_EX_ADD_SERIES, t("Reihe hinzufügen")) & EV_ACT != 0 {
        e.series.push(Series::default());
        st.chosen = true;
    }
    // the prices
    let mut open = st.prices;
    let shown = ui.expander_begin(ID_EX_PRICES_X, t("Kosten: Annahmen"), &mut open) != 0;
    st.prices = open;
    if shown {
        ui.box_begin(0, BOX_V, 6);
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Diese Preise sind angenommen, keine Angebote eines Anbieters. Ein leeres Feld nimmt die Annahme in seiner Vorgabe."));
        for (k, key) in Prices::KEYS.iter().enumerate() {
            let now = e.prices.get(*key).map_or(String::new(), value_text);
            let hint = fixed(Prices::ASSUMED.get(key), 1_000_000);
            if let Some(v) = text_row(st, ui, ID_EX_PRICE + k as u32, price_label(key), price_unit(key), Some(&hint), &now) {
                if v.trim().is_empty() {
                    e.prices.remove(*key);
                } else {
                    e.prices.insert(String::from(*key), value_of(&v));
                }
            }
        }
        let now = e.cost_exclude.join(", ");
        if let Some(v) = text_row(st, ui, ID_EX_EXCLUDE, t("Zählt nicht mit"), "", Some(t("Geräte, durch Kommas getrennt")), &now) {
            e.cost_exclude = v.split(',').map(str::trim).filter(|x| !x.is_empty()).map(String::from).collect();
        }
        ui.box_end();
    }
    ui.expander_end();
    ui.box_end();
    ui.scroll_end();
    if e != before {
        n.exp.draft = Some(e);
    }
}

// the label of a price
fn price_label(key: &str) -> &'static str {
    match key {
        "core" => t("Ein Kern"),
        "big" => t("Faktor über 16 Kerne"),
        "huge" => t("Faktor über 32 Kerne"),
        "ram_gb" => t("1 GiB RAM"),
        "hdd_gb" => t("1 GiB Festplatte"),
        "sata_gb" => t("1 GiB SATA-SSD"),
        "nvme_gb" => t("1 GiB NVMe"),
        "nvme_fast_gb" => t("1 GiB schnelle NVMe"),
        "host" => t("Grundgebühr je Host"),
        "switch" => t("Ein Switch"),
        "router" => t("Router oder Firewall"),
        _ => t("Preis"),
    }
}

// the unit of a price
fn price_unit(key: &str) -> &'static str {
    if matches!(key, "big" | "huge") { "×" } else { "€/h" }
}

// what keeps the experiment from running, or how big it is
fn findings(ui: &mut Ui, found: &[String], runs: usize) {
    if found.is_empty() {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.status_dot(BADGE_OK);
        ui.text(TS_BODY, &tf("Passt zum Projekt: {} Läufe.", &[&runs.to_string()]));
        ui.box_end();
        return;
    }
    for f in found.iter().take(12) {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.status_dot(BADGE_WARN);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, f);
        ui.box_end();
    }
}

// the axes of a list, Achse hinzufügen
fn axes(st: &mut Exp, ui: &mut Ui, p: &Project, list: &mut Vec<Axis>, block: &mut usize, add: u32) {
    let mut gone = None;
    for (k, a) in list.iter_mut().enumerate() {
        if *block >= AXES_MAX {
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Weitere Achsen zeigt die Seite nicht; sie bleiben im Experiment."));
            break;
        }
        if axis(st, ui, p, a, ID_EX_AXIS + *block as u32 * ID_EX_AXIS_STEP) {
            gone = Some(k);
        }
        *block += 1;
    }
    if let Some(k) = gone {
        list.remove(k);
        st.chosen = true;
        st.fresh_fields();
    }
    ui.next_flags(ST_SUBTLE | ST_COMPACT);
    ui.next_icon(IC_ADD);
    if ui.button(add, t("Achse hinzufügen")) & EV_ACT != 0 {
        list.push(Axis { kind: String::from("hw"), ..Axis::default() });
        st.chosen = true;
        // (the axes after it have other ids now)
        st.fresh_fields();
    }
}

// the kinds of an axis in the order of their list
const KINDS: [&str; 3] = ["hw", "config", "backends"];

// one axis -> it goes
fn axis(st: &mut Exp, ui: &mut Ui, p: &Project, a: &mut Axis, id: u32) -> bool {
    let mut gone = false;
    ui.box_begin(0, BOX_V | BOX_CARD | pad(8), 6);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    let names = [t("Hardware"), t("Einstellung"), t("Backends")];
    let mut k = KINDS.iter().position(|x| *x == a.kind).unwrap_or(0) as i32;
    ui.next_size(150, 0);
    if ui.combobox(id, &mut k, &names) & EV_ACT != 0 {
        *a = Axis { kind: String::from(KINDS[k.clamp(0, 2) as usize]), name: a.name.clone(), ..Axis::default() };
        st.chosen = true;
        st.fresh_fields();
    }
    ui.next_size(-1, 0);
    if let Some(v) = tbox(st, ui, id + 1, Some(t("Name der Spalte")), &a.name) {
        a.name = v;
    }
    ui.next_flags(ST_SUBTLE | ST_COMPACT | ST_ICON_ONLY);
    ui.next_icon(IC_DELETE);
    if ui.button(id + 2, t("Achse entfernen")) & EV_ACT != 0 {
        gone = true;
    }
    ui.box_end();
    match a.kind.as_str() {
        "config" => config_axis(st, ui, p, a, id),
        "backends" => backends_axis(st, ui, p, a, id),
        _ => hw_axis(st, ui, p, a, id),
    }
    ui.box_end();
    gone
}

// a device or a group of the project: "Gruppe: Web" stands for the group
fn target_row(st: &mut Exp, ui: &mut Ui, p: &Project, a: &mut Axis, id: u32) {
    let devices = p.devices.iter().map(|d| d.name.clone());
    let groups = p.groups.iter().map(|g| tf("Gruppe: {}", &[&g.name]));
    let names: Vec<String> = devices.chain(groups).collect();
    let now = if a.group.is_empty() { a.dev.clone() } else { tf("Gruppe: {}", &[&a.group]) };
    row_begin(ui, t("Gilt für"));
    ui.next_size(-1, 0);
    if let Some(v) = name_combo(st, ui, id, &names, &now) {
        let prefix = tf("Gruppe: {}", &[""]);
        match v.strip_prefix(prefix.as_str()) {
            Some(g) => {
                a.group = String::from(g);
                a.dev.clear();
            }
            None => {
                a.dev = v;
                a.group.clear();
            }
        }
    }
    ui.box_end();
}

// hardware: a row per value
fn hw_axis(st: &mut Exp, ui: &mut Ui, p: &Project, a: &mut Axis, id: u32) {
    target_row(st, ui, p, a, id + 3);
    let presets: Vec<String> = catalog::hw_presets().map(|m| String::from(m.id)).collect();
    let mut gone = None;
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Je Zeile eine Hardware: eine Vorlage, darauf Kerne, Tempo (1 ist der Referenzkern) und RAM in GiB. Was leer bleibt, kommt von der Vorlage oder vom Gerät."));
    for (r, v) in a.values.iter_mut().enumerate().take(HW_MAX) {
        let rid = id + 16 + r as u32 * 6;
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.next_size(150, 0);
        let preset = v.get("preset").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if let Some(x) = name_combo(st, ui, rid, &presets, &preset) {
            member(v, "preset", &x, false);
        }
        for (k, (key, hint)) in [("cores", t("Kerne")), ("speed", t("Tempo")), ("ram_gb", t("RAM"))].into_iter().enumerate() {
            let now = v.get(key).map_or(String::new(), value_text);
            ui.next_size(-1, 0);
            if let Some(x) = tbox(st, ui, rid + 1 + k as u32, Some(hint), &now) {
                member(v, key, &x, true);
            }
        }
        ui.next_flags(ST_SUBTLE | ST_COMPACT | ST_ICON_ONLY);
        ui.next_icon(IC_DELETE);
        if ui.button(rid + 4, t("Wert entfernen")) & EV_ACT != 0 {
            gone = Some(r);
        }
        ui.box_end();
    }
    if let Some(r) = gone {
        a.values.remove(r);
        st.chosen = true;
        st.fresh_fields();
    }
    ui.next_flags(ST_SUBTLE | ST_COMPACT);
    ui.next_icon(IC_ADD);
    if a.values.len() < HW_MAX && ui.button(id + 14, t("Wert hinzufügen")) & EV_ACT != 0 {
        a.values.push(agent_core::jmap::empty_object());
        st.chosen = true;
    }
}

// a member of a value by its text: empty takes it out, what is no number stays a text for the finding
fn member(v: &mut Value, key: &str, text: &str, number: bool) {
    let text = text.trim();
    if text.is_empty() {
        if let Value::Object(o) = v {
            o.retain(|(k, _)| k != key);
        }
        return;
    }
    let _ = set_path(v, key, if number { value_of(text) } else { Value::string(text) });
}

// a setting: its service, its name, the values
fn config_axis(st: &mut Exp, ui: &mut Ui, p: &Project, a: &mut Axis, id: u32) {
    target_row(st, ui, p, a, id + 3);
    let first = a.path.split_once('.').map_or(a.path.as_str(), |x| x.0).to_string();
    let rest = a.path.split_once('.').map_or("", |x| x.1).to_string();
    let who = if a.group.is_empty() { a.dev.clone() } else { p.groups.iter().find(|g| g.name.eq_ignore_ascii_case(&a.group)).and_then(|g| jfn_model::group::Groups::of(p).devices(p, g.id).first().and_then(|id| p.device(*id)).map(|d| d.name.clone())).unwrap_or_default() };
    let services = services_on(p, &who, None);
    row_begin(ui, t("Dienst"));
    ui.next_size(-1, 0);
    if let Some(v) = name_combo(st, ui, id + 5, &services, &first) {
        a.path = alloc::format!("{}.{}", v, rest);
        st.bufs.remove(&(id + 7));
    }
    ui.box_end();
    // the names the service knows, or a path further in ("routes.0.cpu_ms") typed
    let kind = p.find(&who).and_then(|d| d.services.iter().find(|s| s.name.eq_ignore_ascii_case(&first) || s.kind == first)).map(|s| s.kind.clone()).unwrap_or_default();
    let keys: Vec<String> = known_keys(&kind).unwrap_or(&[]).iter().map(|k| String::from(*k)).collect();
    row_begin(ui, t("Einstellung"));
    ui.next_size(-1, 0);
    if let Some(v) = name_combo(st, ui, id + 6, &keys, &rest) {
        a.path = alloc::format!("{}.{}", first, v);
        st.bufs.remove(&(id + 7));
    }
    ui.next_size(-1, 0);
    if let Some(v) = tbox(st, ui, id + 7, Some(t("oder ein Pfad, etwa routes.0.cpu_ms")), &rest) {
        a.path = alloc::format!("{}.{}", first, v.trim());
    }
    ui.box_end();
    let now = a.values.iter().map(value_text).collect::<Vec<_>>().join(", ");
    if let Some(v) = text_row(st, ui, id + 8, t("Werte"), "", Some("1, 4, 16"), &now) {
        a.values = parse_values(&v);
    }
}

// the backends of a load balancer: the first N of a pool, or copies of a device
fn backends_axis(st: &mut Exp, ui: &mut Ui, p: &Project, a: &mut Axis, id: u32) {
    row_begin(ui, t("Load Balancer"));
    ui.next_size(-1, 0);
    if let Some(v) = name_combo(st, ui, id + 3, &devices_with(p, "lb"), &a.dev) {
        a.svc = services_on(p, &v, Some("lb")).into_iter().next().unwrap_or_default();
        a.dev = v;
    }
    ui.next_size(-1, 0);
    if let Some(v) = name_combo(st, ui, id + 5, &services_on(p, &a.dev, Some("lb")), &a.svc) {
        a.svc = v;
    }
    ui.box_end();
    let mut copies = (!a.template.is_empty()) as i32;
    row_begin(ui, t("Woher"));
    ui.next_flags(ST_COMPACT);
    if ui.segmented(id + 9, &mut copies, &[t("Aus einem Pool"), t("Kopien eines Geräts")]) & EV_ACT != 0 {
        st.chosen = true;
        if copies == 1 {
            a.pool.clear();
        } else {
            a.template.clear();
        }
    }
    ui.box_end();
    let hosts: Vec<String> = p.devices.iter().filter(|d| catalog::model(&d.model).is_some_and(|m| m.has(catalog::F_HOST))).map(|d| d.name.clone()).collect();
    if copies == 1 || !a.template.is_empty() {
        row_begin(ui, t("Vorlage"));
        ui.next_size(-1, 0);
        if let Some(v) = name_combo(st, ui, id + 12, &hosts, &a.template) {
            a.template = v;
        }
        ui.box_end();
    } else {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY, t("Pool"));
        ui.box_begin(0, BOX_H | BOX_WRAP, 8);
        for (k, h) in hosts.iter().enumerate().take(POOL_MAX) {
            let mut on = a.pool.iter().any(|x| x == h) as i32;
            if ui.checkbox(id + 64 + k as u32, h, &mut on) & EV_ACT != 0 {
                st.chosen = true;
                if on != 0 {
                    a.pool.push(h.clone());
                    // the order of the project: the first N are always the same
                    let order = |n: &String| hosts.iter().position(|x| x == n).unwrap_or(usize::MAX);
                    a.pool.sort_by_key(order);
                } else {
                    a.pool.retain(|x| x != h);
                }
            }
        }
        ui.box_end();
    }
    let now: Vec<u64> = a.counts.iter().map(|&c| c as u64).collect();
    if let Some(v) = number_row(st, ui, id + 13, t("Anzahlen"), "1, 2, 4", &now) {
        a.counts = v.iter().map(|&c| c as u32).collect();
    }
}
