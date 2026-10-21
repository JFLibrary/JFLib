// logview -- the Protokoll panel: the log lines of the devices as the simulator reports them
// (Report::Log): what their services log (syslog), the IOS console messages (%LINK,
// %LINEPROTO, %SYS) and debugging output, filtered by device, severity and text
//
//   Log::new()                           empty (a new project)
//   log.push(dev, at, sev, tag, text)    a line (beyond LOG_MAX the oldest go)
//   panel(n, ui)                         the filters, "Leeren", the table (Zeit, Gerät,
//                                        Schwere, Quelle, Meldung); with "Neueste zeigen" new
//                                        lines scroll into view
//
// Severities are syslog's (0 emergency .. 7 debugging) with German names; the filter shows
// one severity and the more severe ones.  Times are the simulator's (since it started).  A
// double click on a line shows the device on the canvas.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_sim::Ns;
use ui::*;

use crate::app::constants::*;
use crate::app::tr::t;
use crate::app::Net;

/// lines kept (the table's row ids end below U4's range)
pub(crate) const LOG_MAX: usize = 3000;

/// the names of the severities 0 .. 7
// i18n
const SEV_NAMES: [&str; 8] = ["Notfall", "Alarm", "Kritisch", "Fehler", "Warnung", "Hinweis", "Info", "Debug"];
/// the severity filter: label, the least severe severity shown
// i18n
const SEV_FILTER: [(&str, u8); 5] = [
    ("Alle Schweregrade", 7),
    ("Info und wichtiger", 6),
    ("Hinweise und wichtiger", 5),
    ("Warnungen und wichtiger", 4),
    ("Fehler und wichtiger", 3),
];

// the columns in the current language
fn cols() -> [GridCol<'static>; 5] {
    [
        GridCol::new(t("Zeit"), 96).flags(GC_NOSORT),
        GridCol::new(t("Gerät"), 84).flags(GC_NOSORT),
        GridCol::new(t("Schwere"), 92).flags(GC_NOSORT),
        GridCol::new(t("Quelle"), 196).flags(GC_NOSORT),
        GridCol::new(t("Meldung"), 360).flags(GC_NOSORT | GC_FILL),
    ]
}

/// a line of the log
struct Row {
    seq: u64,
    at: Ns,
    dev: u32,
    sev: u8,
    tag: String,
    text: String,
}

pub(crate) struct Log {
    rows: VecDeque<Row>,
    next: u64,
    /// the device filter (None: all)
    dev: Option<u32>,
    /// the severity filter (an index of SEV_FILTER)
    sev_sel: i32,
    find: String,
    /// new lines scroll into view
    follow: i32,
    grid: GridState,
    /// the newest line shown at the last look
    seen: u64,
}

impl Log {
    pub(crate) fn new() -> Log {
        Log { rows: VecDeque::new(), next: 1, dev: None, sev_sel: 0, find: String::new(), follow: 1, grid: GridState::default(), seen: 0 }
    }

    /// a line of a device's log
    pub(crate) fn push(&mut self, dev: u32, at: Ns, sev: u8, tag: String, text: String) {
        if self.rows.len() >= LOG_MAX {
            self.rows.pop_front();
        }
        self.rows.push_back(Row { seq: self.next, at, dev, sev: sev.min(7), tag, text });
        self.next += 1;
    }

    pub(crate) fn clear(&mut self) {
        self.rows.clear();
        self.grid = GridState::default();
    }
}

/// "00:01:02.345" of a virtual time
fn clock(t: Ns) -> String {
    let ms = t / 1_000_000;
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

fn badge_of(sev: u8) -> u32 {
    match sev {
        0..=3 => BADGE_CRIT,
        4 => BADGE_WARN,
        5 => BADGE_INFO,
        _ => BADGE_NEUTRAL,
    }
}

/// the Protokoll panel
pub(crate) fn panel(n: &mut Net, ui: &mut Ui) {
    // the filters: device, severity, text; "Neueste zeigen", "Leeren"
    let mut devs: Vec<(u32, String)> = n.doc.proj.devices.iter().map(|d| (d.id, d.name.clone())).collect();
    devs.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    if n.log.dev.is_some_and(|d| !devs.iter().any(|x| x.0 == d)) {
        n.log.dev = None;
    }
    let mut names: Vec<&str> = alloc::vec![t("Alle Geräte")];
    names.extend(devs.iter().map(|d| d.1.as_str()));
    let mut dsel = n.log.dev.and_then(|d| devs.iter().position(|x| x.0 == d)).map_or(0, |i| i as i32 + 1);
    ui.box_begin(0, BOX_H | BOX_XCENTER | pad(6), 8);
    ui.next_size(150, 0);
    if ui.combobox(ID_LOG_DEV, &mut dsel, &names) & EV_ACT != 0 {
        n.log.dev = if dsel <= 0 { None } else { devs.get(dsel as usize - 1).map(|d| d.0) };
        ui.repass();
    }
    ui.tooltip(t("Nur die Meldungen dieses Geräts"));
    let labels: Vec<&str> = SEV_FILTER.iter().map(|f| t(f.0)).collect();
    ui.next_size(190, 0);
    if ui.combobox(ID_LOG_SEV, &mut n.log.sev_sel, &labels) & EV_ACT != 0 {
        ui.repass();
    }
    ui.tooltip(t("Schwere: 0 Notfall bis 7 Debug"));
    ui.next_size(200, 0);
    if ui.textbox(ID_LOG_FIND, Some(t("Meldungen durchsuchen")), &mut n.log.find, 80) & EV_ACT != 0 {
        ui.repass();
    }
    ui.spacer(-1);
    ui.next_flags(ST_COMPACT);
    ui.checkbox(ID_LOG_FOLLOW, t("Neueste zeigen"), &mut n.log.follow);
    ui.cmd_button(CMD_LOG_CLEAR, ST_ICON_ONLY | ST_SUBTLE);
    ui.box_end();
    // the lines that pass the filters
    let Log { rows, dev, sev_sel, find, follow, grid, seen, .. } = &mut n.log;
    let max = SEV_FILTER.get(*sev_sel as usize).map_or(7, |f| f.1);
    let find = find.trim().to_lowercase();
    let name_of = |d: u32| devs.iter().find(|x| x.0 == d).map_or("?", |x| x.1.as_str());
    let shown: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| r.sev <= max && dev.is_none_or(|d| d == r.dev))
        .filter(|(_, r)| find.is_empty() || r.text.to_lowercase().contains(&find) || r.tag.to_lowercase().contains(&find) || name_of(r.dev).to_lowercase().contains(&find))
        .map(|(i, _)| i)
        .collect();
    if shown.is_empty() {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(12), 0);
        if rows.is_empty() {
            ui.empty_state(IC_HISTORY, t("Noch keine Meldungen"), t("Die Geräte protokollieren hier, was sie tun: Zustände der Ports (%LINK), Konfigurationen, DHCP, Zeit, und mit „debug“ jede Entscheidung."));
        } else {
            ui.empty_state(IC_FILTER, t("Keine passenden Meldungen"), t("Die Filter verbergen alle Meldungen; „Alle Geräte“ und „Alle Schweregrade“ zeigen sie wieder."));
        }
        ui.box_end();
        return;
    }
    let keys: Vec<u64> = shown.iter().map(|&i| rows[i].seq).collect();
    let newest = keys.last().copied().unwrap_or(0);
    grid.retain(&keys);
    ui.next_size(-1, -1);
    let cols = cols();
    let (first, end) = ui.grid_begin(ID_LOG_GRID, grid, &cols, &keys);
    for i in first..end {
        let r = &rows[shown[i as usize]];
        ui.grid_row(grid, i);
        ui.grid_text_ts(grid, 0, TS_CAPTION, &clock(r.at));
        ui.grid_text(grid, 1, name_of(r.dev));
        ui.grid_cell(grid, 2);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.status_dot(badge_of(r.sev));
        ui.text(TS_BODY | TX_ELLIPSIS, &format!("{} {}", r.sev, t(SEV_NAMES[r.sev as usize])));
        ui.box_end();
        ui.grid_text(grid, 3, &r.tag);
        ui.grid_text(grid, 4, &r.text);
    }
    let ev = ui.grid_end(grid);
    // new lines: to the end of the table
    if *follow != 0 && newest != *seen {
        let (_, rh, _) = ui.style.data().grid_metrics(ui);
        ui.scroll_to(ID_LOG_GRID, -1, (keys.len() as i32 + 1) * rh);
    }
    *seen = newest;
    // a double click: the device on the canvas
    let picked = if ev.ev & EV_DOUBLE != 0 { shown.get(ev.row.max(0) as usize).map(|&i| rows[i].dev) } else { None };
    if let Some(dev) = picked {
        n.canvas.select_only(dev);
        if let Some(d) = n.doc.proj.device(dev) {
            n.canvas.st.vp.cx = d.x as f64 + 32.0;
            n.canvas.st.vp.cy = d.y as f64 + 24.0;
        }
        ui.repass();
    }
}
