// datagallery -- the data views and app shell parts of lib/ui on one page: navigation pane,
// command bar, breadcrumb, KPI cards with sparklines, bar chart, donut, grid, tree, wizard
// steps, segmented control, time / weekday / date input, badges, meters, toasts, spinner,
// rows that wrap (BOX_WRAP), a details grid box whose note wraps (tests/wrap.uis), all of it
// at a text scale of 150 % (tests/textscale.uis); the page "Simulation" (navigation, Labor):
// a timeline over three tracks of events and a hex view of an Ethernet / IPv4 / ICMP frame
// with its layers marked
//
//   python build_rs.py app datagallery --style backupcraft,winui,kde
//   build/win/datagallery.exe --shot shot.png --size 1440x960 --theme dark
//   build/win/datagallery.exe --script apps/datagallery/tests/smoke.uis
//   build/win/datagallery.exe --lang ar             (the library's texts in Arabic, mirrored)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

const DESC: AppDesc = AppDesc {
    title: "Daten & Shell",
    flags: AF_RESIZABLE,
    w: 1440,
    h: 960,
    min_w: 900,
    min_h: 600,
    ..AppDesc::NONE
};

// a catalogue without rows: the languages of the library's texts (i18n::library_languages)
const LANGUAGES: &str = "de\ten\tfr\tes\tit\tnl\tpl\tar\the\n";

const ID_NAV: u32 = 100;
const ID_NAV_TOGGLE: u32 = 101;
const ID_NAV_ITEM: u32 = 110;
const ID_CMDBAR: u32 = 200;
const ID_CMD_NEW: u32 = 201;
const ID_CMD_RUN: u32 = 202;
const ID_CMD_STOP: u32 = 203;
const ID_CMD_EDIT: u32 = 204;
const ID_CMD_DELETE: u32 = 205;
const ID_CMD_REFRESH: u32 = 206;
const ID_CMD_EXPORT: u32 = 207;
const ID_CRUMB: u32 = 300;
const ID_SPARK: u32 = 400;
// a widget's parts take the ids after its own: bar_chart one per column (ID_BARS + 1 ..= + 14),
// wizard_steps and segmented one per step / item -- so the next id is further away than that
const ID_BARS: u32 = 420;
const ID_DONUT: u32 = 450;
const ID_GRID: u32 = 1000;
const ID_TREE: u32 = 20000;
const ID_WIZ: u32 = 30000;
const ID_WIZ_BACK: u32 = 30010;
const ID_WIZ_NEXT: u32 = 30011;
const ID_WIZ_INFO: u32 = 30012;
const ID_SEG: u32 = 30100;
const ID_TIME: u32 = 30200;
const ID_DAYS: u32 = 30300;
const ID_DATE: u32 = 30400;
const ID_TOAST: u32 = 30500;
const ID_SPLIT: u32 = 30600;
const ID_TABS: u32 = 30700;
const ID_CHECK: u32 = 30800;
const ID_SWITCH: u32 = 30801;
const ID_RADIO: u32 = 30802;
const ID_TEXT: u32 = 30810;
const ID_SEARCH: u32 = 30811;
const ID_COMBO: u32 = 30812;
const ID_BTN: u32 = 30820;
const ID_ACCENT: u32 = 30821;
const ID_DIS: u32 = 30822;
const ID_SLIDER: u32 = 30830;
const ID_METER: u32 = 30840;
const ID_DETAILS: u32 = 30850;
const ID_SIM: u32 = 31000;
const ID_TL: u32 = 31010;
const ID_TL_FOLLOW: u32 = 31020;
const ID_TL_FIT: u32 = 31021;
const ID_HEX: u32 = 31100;
const ID_HEX_PASTE: u32 = 31110;
const PAGE_SIM: u32 = 9;
// scripts: the modifier keys the timeline treats as held (`cmd 9100 2` = Ctrl, 1 = Shift)
const CMD_TL_MODS: u32 = 9100;

// the simulation page: 12 s of events; an echo request as one frame with its three layers
const SPAN: f64 = 12.0;
const MARKS: [HexMark; 3] = [
    HexMark { start: 0, len: 14, color: 0xFFC9_792B, key: 1 },
    HexMark { start: 14, len: 20, color: 0xFF3E_6E9E, key: 2 },
    HexMark { start: 34, len: 64, color: 0xFF3D_8C84, key: 3 },
];

fn layer_name(key: u64) -> &'static str {
    match key {
        1 => "Ethernet II",
        2 => "IPv4",
        _ => "ICMP",
    }
}

// a track's events from a fixed seed (xorshift32): bursts around a few times plus a fifth
// spread evenly; per mille `rare` of them in the second colour; sorted by time
fn events(seed: u32, n: usize, bursts: usize, main: u32, second: u32, rare: u32) -> Vec<(f64, u32)> {
    let mut x = seed;
    let mut next = move || {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        x
    };
    let mut unit = move || (next() >> 8) as f64 / 16_777_216.0;
    let centres: Vec<f64> = (0..bursts).map(|_| unit() * SPAN).collect();
    let width = SPAN / bursts as f64 * 0.35;
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        let t = if unit() < 0.2 {
            unit() * SPAN
        } else {
            centres[(unit() * bursts as f64) as usize % bursts] + (unit() + unit() - 1.0) * width
        };
        let c = if unit() * 1000.0 < rare as f64 { second } else { main };
        if (0.0..=SPAN).contains(&t) {
            v.push((t, c));
        }
    }
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v
}

// the internet checksum of b
fn checksum(b: &[u8]) -> u16 {
    let mut s: u32 = 0;
    for p in b.chunks(2) {
        s += (p[0] as u32) << 8 | p.get(1).copied().unwrap_or(0) as u32;
    }
    while s > 0xFFFF {
        s = (s & 0xFFFF) + (s >> 16);
    }
    !(s as u16)
}

// an ICMP echo request from 192.168.1.10 to 192.168.1.1 (98 bytes, checksums filled in)
fn echo_frame() -> Vec<u8> {
    let mut f = Vec::with_capacity(98);
    f.extend_from_slice(&[0x52, 0x54, 0x00, 0x12, 0x34, 0x56, 0x52, 0x54, 0x00, 0xAB, 0xCD, 0xEF, 0x08, 0x00]);
    f.extend_from_slice(&[0x45, 0x00, 0x00, 0x54, 0x1C, 0x46, 0x40, 0x00, 0x40, 0x01, 0x00, 0x00, 192, 168, 1, 10, 192, 168, 1, 1]);
    let c = checksum(&f[14..34]);
    f[24..26].copy_from_slice(&c.to_be_bytes());
    f.extend_from_slice(&[0x08, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x07, 0x5F, 0x2C, 0x9B, 0x66, 0x00, 0x00, 0x00, 0x00]);
    f.extend(0x10u8..0x40);
    let c = checksum(&f[34..]);
    f[36..38].copy_from_slice(&c.to_be_bytes());
    f
}

struct Job {
    name: &'static str,
    kind: &'static str,
    status: u32,
    last: &'static str,
    size: u64,
}

const JOBS: [Job; 9] = [
    Job { name: "Dateiserver – täglich", kind: "Datei", status: BADGE_OK, last: "Heute 02:00", size: 182_000_000_000 },
    Job { name: "SQL-Cluster Logs", kind: "Datei", status: BADGE_WARN, last: "Heute 01:30", size: 21_400_000_000 },
    Job { name: "Mailserver", kind: "Agent", status: BADGE_OK, last: "Gestern 23:00", size: 96_000_000_000 },
    Job { name: "Entwicklung VM-Images", kind: "Image", status: BADGE_CRIT, last: "Gestern 22:15", size: 512_000_000_000 },
    Job { name: "Webserver Konfiguration", kind: "Agent", status: BADGE_OK, last: "Heute 03:10", size: 1_200_000_000 },
    Job { name: "Buchhaltung Archiv", kind: "Datei", status: BADGE_NEUTRAL, last: "Nie", size: 0 },
    Job { name: "Laptop-Flotte Dokumente", kind: "Agent", status: BADGE_INFO, last: "Läuft …", size: 64_000_000_000 },
    Job { name: "Monatsabschluss", kind: "Kopie", status: BADGE_OK, last: "01.10. 04:00", size: 340_000_000_000 },
    Job { name: "Domänencontroller", kind: "Agent", status: BADGE_OK, last: "Heute 02:30", size: 14_000_000_000 },
];

fn status_name(s: u32) -> &'static str {
    match s {
        BADGE_OK => "Erfolgreich",
        BADGE_WARN => "Warnung",
        BADGE_CRIT => "Fehlgeschlagen",
        BADGE_INFO => "Läuft",
        _ => "Nie gelaufen",
    }
}

// what the status means for the job (the details card)
fn status_note(s: u32) -> &'static str {
    match s {
        BADGE_OK => "Alle Dateien wurden gesichert und geprüft.",
        BADGE_WARN => "Zwei Protokolldateien waren während der Sicherung gesperrt und wurden übersprungen; der nächste Lauf versucht sie erneut.",
        BADGE_CRIT => "Das Repository war nicht erreichbar. Prüfen Sie die Verbindung und starten Sie den Job erneut.",
        BADGE_INFO => "Die Sicherung läuft gerade.",
        _ => "Der Job ist noch nie gelaufen.",
    }
}

fn size_text(b: u64) -> String {
    if b == 0 {
        return String::from("–");
    }
    let g = b as f64 / 1e9;
    if g >= 1000.0 { format!("{:.1} TB", g / 1000.0) } else if g >= 10.0 { format!("{:.0} GB", g) } else { format!("{:.1} GB", g) }
}

struct Gal {
    nav: NavState,
    page: u32,
    grid: GridState,
    order: Vec<usize>,
    tree: TreeState,
    checks: [u8; 8],
    step: usize,
    seg: i32,
    hour: i32,
    minute: i32,
    days: u32,
    date: (i32, u32, u32),
    split: i32,
    tab: i32,
    check: i32,
    sw: i32,
    radio: i32,
    text: String,
    search: String,
    combo: i32,
    slider: i32,
    tl: TimelineState,
    arp: Vec<(f64, u32)>,
    icmp: Vec<(f64, u32)>,
    tcp: Vec<(f64, u32)>,
    hex: HexState,
    frame: Vec<u8>,
    layer: Option<u64>,
    paste: String,
}

impl Default for Gal {
    fn default() -> Gal {
        let mut grid = GridState::default();
        grid.select_only(Some(1));
        let mut tree = TreeState::default();
        tree.expanded.insert(1);
        tree.expanded.insert(3);
        Gal {
            nav: NavState::default(),
            page: 1,
            grid,
            order: (0..JOBS.len()).collect(),
            tree,
            checks: [2, 1, 2, 1, 0, 1, 0, 0],
            step: 2,
            seg: 2,
            hour: 2,
            minute: 0,
            days: 0b0011111,
            date: (2026, 10, 12),
            split: 820,
            tab: 0,
            check: 1,
            sw: 1,
            radio: 1,
            text: String::from("D:\\Daten\\Projekte"),
            search: String::new(),
            combo: 1,
            slider: 60,
            tl: TimelineState::default(),
            arp: events(0x1234_5678, 400, 6, 0xFFE0_7B00, 0xFFE0_7B00, 0),
            icmp: events(0x9E37_79B9, 1600, 10, 0xFF2B_9348, 0xFFD9_383C, 40),
            tcp: events(0x85EB_CA6B, 3200, 14, 0, 0xFFD9_383C, 8),
            hex: HexState::default(),
            frame: echo_frame(),
            layer: None,
            paste: String::new(),
        }
    }
}

impl Gal {
    fn sort(&mut self) {
        let (col, desc) = self.grid.sort.unwrap_or((0, false));
        // texts in the user's collation, numbers in them by value
        let coll = i18n::collator().numeric(true);
        self.order.sort_by(|&a, &b| {
            let (x, y) = (&JOBS[a], &JOBS[b]);
            let o = match col {
                1 => coll.compare(x.kind, y.kind),
                2 => x.status.cmp(&y.status),
                3 => coll.compare(x.last, y.last),
                4 => x.size.cmp(&y.size),
                _ => coll.compare(x.name, y.name),
            };
            if desc { o.reverse() } else { o }
        });
    }

    fn nav(&mut self, ui: &mut Ui) {
        ui.next_size(if self.nav.compact { 56 } else { 248 }, -1);
        ui.nav_begin(ID_NAV, &mut self.nav);
        ui.box_begin(0, BOX_H | BOX_XCENTER | pad(4), 8);
        ui.nav_toggle(ID_NAV_TOGGLE, "Navigation umschalten", &mut self.nav);
        if !self.nav.compact {
            ui.text(TS_BODY_STRONG, "BackupCraft");
        }
        ui.box_end();
        let items: [(u32, &str, u32, Option<&str>); 10] = [
            (0, "Übersicht", IC_HOME, None),
            (1, "Sicherungsjobs", IC_HISTORY, Some("2")),
            (2, "Wiederherstellung", IC_REFRESH, None),
            (3, "Server & Agenten", IC_GRID, None),
            (4, "Repositories", IC_FOLDER, None),
            (5, "Zugangsdaten", IC_LOCK, None),
            (6, "Replikation", IC_SHUFFLE, Some("bald")),
            (7, "Monitoring", IC_SLIDERS, Some("1")),
            (PAGE_SIM, "Simulation", IC_DEV_TOOLS, None),
            (8, "Einstellungen", IC_SETTINGS, None),
        ];
        for (k, (i, label, icon, badge)) in items.iter().enumerate() {
            match k {
                0 => ui.nav_section("Start"),
                1 => ui.nav_section("Schutz"),
                3 => ui.nav_section("Infrastruktur"),
                6 => ui.nav_section("Betrieb"),
                8 => ui.nav_section("Labor"),
                _ => {}
            }
            if *i == 8 {
                ui.nav_footer();
            }
            if ui.nav_item(ID_NAV_ITEM + i, *icon, label, self.page == *i, *badge) & EV_ACT != 0 {
                self.page = *i;
            }
        }
        ui.nav_end();
    }

    fn kpis(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_H, 12);
        let cards: [(&str, &str, &str, u32, [f32; 10]); 4] = [
            ("Geschützte Objekte", "248", "+6 diese Woche", 0, [230.0, 232.0, 233.0, 236.0, 238.0, 238.0, 241.0, 243.0, 246.0, 248.0]),
            ("Erfolgreich (24 h)", "37", "von 40 Sitzungen", 0xFF2B9348, [30.0, 34.0, 31.0, 36.0, 35.0, 38.0, 36.0, 39.0, 37.0, 37.0]),
            ("Fehlgeschlagen", "2", "1 neu seit gestern", 0xFFD9383C, [0.0, 1.0, 0.0, 0.0, 2.0, 1.0, 0.0, 1.0, 1.0, 2.0]),
            ("Speicherbelegung", "71 %", "14,2 von 20 TB", 0xFFE07B00, [52.0, 54.0, 57.0, 59.0, 61.0, 63.0, 65.0, 67.0, 69.0, 71.0]),
        ];
        for (i, (title, value, sub, color, spark)) in cards.iter().enumerate() {
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 4);
            ui.text(TS_CAPTION | TX_SECONDARY, title);
            ui.box_begin(0, BOX_H | BOX_XEND, 8);
            ui.text(TS_TITLE, value);
            ui.spacer(-1);
            ui.next_size(96, 34);
            ui.sparkline(ID_SPARK + i as u32, spark, *color);
            ui.box_end();
            ui.text(TS_CAPTION | TX_TERTIARY, sub);
            ui.box_end();
        }
        ui.box_end();
    }

    fn charts(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_H, 12);
        ui.next_size(-2, 0);
        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 8);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.text(TS_BODY_STRONG, "Sitzungen der letzten 14 Tage");
        ui.spacer(-1);
        ui.legend(&[("Erfolgreich", 0xFF2B9348), ("Warnung", 0xFFE07B00), ("Fehler", 0xFFD9383C)]);
        ui.box_end();
        let labels = ["29.9", "30.9", "1.10", "2.10", "3.10", "4.10", "5.10", "6.10", "7.10", "8.10", "9.10", "10.10", "11.10", "12.10"];
        let ok = [31.0, 33.0, 30.0, 35.0, 34.0, 36.0, 28.0, 30.0, 37.0, 35.0, 38.0, 36.0, 37.0, 33.0];
        let warn = [2.0, 1.0, 3.0, 1.0, 2.0, 0.0, 1.0, 2.0, 1.0, 3.0, 1.0, 2.0, 1.0, 1.0];
        let fail = [0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 2.0, 1.0];
        ui.next_size(-1, 190);
        ui.bar_chart(ID_BARS, &labels, &[
            BarSeries { label: "Erfolgreich", values: &ok, color: 0xFF2B9348 },
            BarSeries { label: "Warnung", values: &warn, color: 0xFFE07B00 },
            BarSeries { label: "Fehler", values: &fail, color: 0xFFD9383C },
        ], true);
        ui.box_end();
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 8);
        ui.text(TS_BODY_STRONG | TX_ELLIPSIS, "Repository-Belegung");
        ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
        ui.next_size(150, 150);
        ui.donut(ID_DONUT, &[
            DonutSlice { label: "Dateien", value: 8.4, color: 0 },
            DonutSlice { label: "Images", value: 4.1, color: 0 },
            DonutSlice { label: "Agenten", value: 1.7, color: 0 },
            DonutSlice { label: "Frei", value: 5.8, color: 0x50808890 },
        ], "71 %", "belegt");
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V, 6);
        // the labels share the room the donut leaves (large text: cut with "…")
        for (name, v, c) in [("Primär (NAS)", 71, BADGE_WARN), ("Offsite", 38, BADGE_OK), ("Archiv", 93, BADGE_CRIT)] {
            ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, name);
            ui.next_size(-1, 8);
            ui.meter(ID_METER + v as u32, v, 100, c);
        }
        ui.box_end();
        ui.box_end();
        ui.box_end();
        ui.box_end();
    }

    fn grid(&mut self, ui: &mut Ui) {
        let cols = [
            GridCol::new("Name", 220).flags(GC_FILL),
            GridCol::new("Typ", 90),
            GridCol::new("Status", 140),
            GridCol::new("Letzter Lauf", 120),
            GridCol::new("Größe", 90).flags(GC_RIGHT),
        ];
        let keys: Vec<u64> = self.order.iter().map(|&i| i as u64).collect();
        let (first, end) = ui.grid_begin(ID_GRID, &mut self.grid, &cols, &keys);
        for i in first..end {
            let j = &JOBS[self.order[i as usize]];
            ui.grid_row(&mut self.grid, i);
            ui.grid_text(&self.grid, 0, j.name);
            ui.grid_text_ts(&self.grid, 1, TS_BODY | TX_SECONDARY, j.kind);
            ui.grid_cell(&self.grid, 2);
            ui.badge(j.status, status_name(j.status));
            ui.grid_text_ts(&self.grid, 3, TS_BODY | TX_SECONDARY, j.last);
            ui.grid_text(&self.grid, 4, &size_text(j.size));
        }
        let ev = ui.grid_end(&mut self.grid);
        if ev.sort_changed {
            self.sort();
        }
        if ev.ev & EV_DOUBLE != 0 && ev.row >= 0 {
            let name = JOBS[self.order[ev.row as usize]].name;
            ui.toast(IB_INFO, "Job öffnen", name, 0);
        }
    }

    fn tree(&mut self, ui: &mut Ui) {
        let all: [(u64, u16, bool, u32, &str, Option<&str>, usize); 8] = [
            (1, 0, true, IC_FOLDER, "D:\\Daten", Some("1,2 TB"), 0),
            (2, 1, true, IC_FOLDER, "Buchhaltung", Some("84 GB"), 1),
            (3, 1, true, IC_FOLDER, "Projekte", Some("640 GB"), 2),
            (4, 2, false, IC_DOCUMENT, "Angebot_2026.docx", Some("2,1 MB"), 3),
            (5, 2, false, IC_FOLDER, "Archiv", Some("312 GB"), 4),
            (6, 2, false, IC_DOCUMENT, "Plan.xlsx", Some("640 KB"), 5),
            (7, 1, false, IC_FOLDER, "Temp", Some("12 GB"), 6),
            (8, 0, true, IC_FOLDER, "E:\\Medien", Some("3,4 TB"), 7),
        ];
        let mut rows = Vec::new();
        let mut hidden_below: Option<u16> = None;
        for (key, depth, kids, icon, label, detail, ci) in all.iter() {
            if let Some(d) = hidden_below {
                if *depth > d {
                    continue;
                }
                hidden_below = None;
            }
            rows.push(TreeRow { key: *key, depth: *depth, has_children: *kids, check: Some(self.checks[*ci]), icon: *icon, label, detail: *detail });
            if *kids && !self.tree.expanded.contains(key) {
                hidden_below = Some(*depth);
            }
        }
        let ev = ui.tree(ID_TREE, &mut self.tree, &rows, 0);
        if let Some((k, v)) = ev.check {
            if let Some(p) = all.iter().position(|r| r.0 == k) {
                self.checks[all[p].6] = v;
            }
        }
    }

    fn controls(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 12);
        ui.text(TS_BODY_STRONG, "Steuerelemente");
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
        ui.button(ID_BTN, "Abbrechen");
        ui.accent_button(ID_ACCENT, "Job starten");
        ui.next_flags(ST_DISABLED);
        ui.button(ID_DIS, "Deaktiviert");
        ui.next_icon(IC_REFRESH);
        ui.next_flags(ST_SUBTLE);
        ui.button(ID_BTN + 3, "Aktualisieren");
        ui.next_icon(IC_DOWNLOAD);
        ui.button(ID_BTN + 4, "Export");
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 16);
        ui.checkbox(ID_CHECK, "Verschlüsseln", &mut self.check);
        ui.switch(ID_SWITCH, "Aktiviert", &mut self.sw);
        ui.radio(ID_RADIO, "Inkrementell", &mut self.radio, 1);
        ui.radio(ID_RADIO + 1, "Vollständig", &mut self.radio, 2);
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(-1, 0);
        ui.textbox(ID_TEXT, Some("Pfad"), &mut self.text, 256);
        ui.next_size(160, 0);
        ui.combobox(ID_COMBO, &mut self.combo, &["Primär (NAS)", "Offsite", "Archiv"]);
        ui.box_end();
        ui.next_size(-1, 0);
        ui.textbox_icon(ID_SEARCH, IC_SEARCH, Some("Jobs, Server, Restore-Points suchen"), &mut self.search, 128);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.next_size(-1, 0);
        ui.slider(ID_SLIDER, &mut self.slider, &Range::new(0, 100, 1, 0));
        ui.text(TS_BODY | TX_SECONDARY, &format!("{} Mbit/s", self.slider * 10));
        ui.box_end();
        ui.next_size(-1, 0);
        ui.progress(ID_SLIDER + 1, 62, 100);
        // the spinner's, the time's and the date's rows: what does not fit in the card starts a
        // new row (the label wraps at large text)
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 12);
        ui.spinner(ID_SLIDER + 2, 20);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, "Verbindung wird hergestellt …");
        ui.next_size(160, 0);
        ui.progress_indeterminate(ID_SLIDER + 3);
        ui.box_end();
        ui.segmented(ID_SEG, &mut self.seg, &["Manuell", "Stündlich", "Täglich", "Wöchentlich", "Monatlich"]);
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 16);
        ui.time_box(ID_TIME, &mut self.hour, &mut self.minute);
        ui.weekday_toggles(ID_DAYS, &mut self.days);
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 12);
        let (mut y, mut m, mut d) = self.date;
        if ui.date_picker(ID_DATE, &mut y, &mut m, &mut d) & EV_ACT != 0 {
            self.date = (y, m, d);
        }
        ui.badge(BADGE_OK, "Erfolgreich");
        ui.badge(BADGE_WARN, "Warnung");
        ui.badge(BADGE_CRIT, "Fehlgeschlagen");
        ui.badge(BADGE_INFO, "Läuft");
        ui.badge(BADGE_NEUTRAL, "Nie");
        ui.box_end();
        ui.box_end();
    }

    // the selected job: labels and values in a grid box; the note wraps and its row grows
    fn details(&mut self, ui: &mut Ui) {
        let Some(&k) = self.grid.sel.iter().next() else { return };
        let j = &JOBS[k as usize];
        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 12);
        ui.text(TS_BODY_STRONG, "Details");
        ui.gridbox_begin(ID_DETAILS, &[92, -1], 0, 6);
        let size = size_text(j.size);
        for (key, value) in [("Name", j.name), ("Typ", j.kind), ("Status", status_name(j.status)), ("Hinweis", status_note(j.status)), ("Größe", size.as_str())] {
            ui.text(TS_BODY | TX_SECONDARY, key);
            ui.text(TS_BODY | TX_WRAP, value);
        }
        ui.gridbox_end();
        ui.box_end();
    }

    fn side(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 8);
        ui.text(TS_BODY_STRONG, "Neuer Sicherungsjob");
        let steps = ["Name", "Quelle", "Ziel", "Aufbewahrung", "Zeitplan", "Zusammenfassung"];
        if let Some(s) = ui.wizard_steps(ID_WIZ, &steps, self.step, &[]) {
            self.step = s;
        }
        ui.box_begin(0, BOX_H, 8);
        ui.next_flags(if self.step == 0 { ST_DISABLED } else { 0 });
        if ui.button(ID_WIZ_BACK, "Zurück") & EV_ACT != 0 && self.step > 0 {
            self.step -= 1;
        }
        ui.spacer(-1);
        if ui.accent_button(ID_WIZ_NEXT, if self.step + 1 == steps.len() { "Fertigstellen" } else { "Weiter" }) & EV_ACT != 0
            && self.step + 1 < steps.len()
        {
            self.step += 1;
        }
        ui.box_end();
        ui.box_end();
        ui.infobar(ID_WIZ_INFO, IB_WARNING, "Repository fast voll", "Archiv: 93 % belegt");
        ui.box_begin(0, BOX_V | BOX_CARD | pad(8), 4);
        ui.tabs(ID_TABS, &mut self.tab, &["Quelle", "Restore-Points", "Protokoll"]);
        ui.next_size(-1, 280);
        self.tree(ui);
        ui.box_end();
    }

    // the simulation page: the timeline ("Zeitleiste") and the hex view ("Hex")
    fn simulation(&mut self, ui: &mut Ui) {
        ui.breadcrumb(ID_CRUMB, &["BackupCraft", "Labor", "Simulation"]);
        ui.page_header("Simulation", "Ereignisse eines Netzwerk-Laufs auf der Zeitleiste, ein Paket als Hex-Ansicht");
        ui.next_size(-1, -1);
        ui.scroll_begin(ID_SIM, SC_V | pad(2));
        ui.box_begin(0, BOX_V, 14);
        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 10);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
        ui.text(TS_BODY_STRONG, "Zeitleiste");
        ui.text(TS_CAPTION | TX_SECONDARY, "TCP, ICMP und ARP über 12 s, rot: verworfen");
        ui.spacer(-1);
        let t = format!("t = {} s", ui.fmt_f64(self.tl.t, 3));
        ui.text(TS_BODY, &t);
        let v = format!("Ansicht {} – {} s", ui.fmt_f64(self.tl.view.0, 3), ui.fmt_f64(self.tl.view.1, 3));
        ui.text(TS_BODY | TX_SECONDARY, &v);
        let r = match self.tl.sel {
            Some((a, b)) => format!("Bereich {} – {} s", ui.fmt_f64(a, 3), ui.fmt_f64(b, 3)),
            None => String::from("Bereich –"),
        };
        ui.text(TS_BODY | TX_SECONDARY, &r);
        let mut follow = self.tl.follow as i32;
        if ui.checkbox(ID_TL_FOLLOW, "Live folgen", &mut follow) & EV_ACT != 0 {
            self.tl.follow = follow != 0;
        }
        if ui.button(ID_TL_FIT, "Ganze Spanne") & EV_ACT != 0 {
            self.tl.fit();
        }
        ui.box_end();
        // TCP in the palette's first colour (0), the others in their own
        let tracks = [
            TlTrack { label: "TCP", ticks: &self.tcp },
            TlTrack { label: "ICMP", ticks: &self.icmp },
            TlTrack { label: "ARP", ticks: &self.arp },
        ];
        ui.next_size(-1, 0);
        ui.timeline(ID_TL, &mut self.tl, (0.0, SPAN), &tracks);
        ui.box_end();

        ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 10);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
        ui.text(TS_BODY_STRONG, "Hex");
        ui.text(TS_CAPTION | TX_SECONDARY, "ICMP-Echo-Anfrage 192.168.1.10 → 192.168.1.1, 98 Bytes");
        ui.spacer(-1);
        ui.legend(&[(layer_name(1), MARKS[0].color), (layer_name(2), MARKS[1].color), (layer_name(3), MARKS[2].color)]);
        ui.box_end();
        ui.next_size(-1, 120);
        let ev = ui.hex_view(ID_HEX, &mut self.hex, &self.frame, &MARKS);
        if let Some(k) = ev.clicked_key {
            self.layer = Some(k);
        }
        ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
        let sel = match self.hex.sel {
            Some((s, 1)) => format!("Auswahl: 0x{:04X} (1 Byte)", s),
            Some((s, l)) => format!("Auswahl: 0x{:04X}–0x{:04X} ({} Bytes)", s, s + l - 1, l),
            None => String::from("Auswahl: –"),
        };
        ui.text(TS_BODY, &sel);
        ui.text(TS_BODY | TX_SECONDARY, &format!("Feld: {}", self.layer.map_or("–", layer_name)));
        let at = ev.hover.map_or(String::from("Zeiger: –"), |b| format!("Zeiger: 0x{:04X}", b));
        ui.text(TS_BODY | TX_SECONDARY, &at);
        ui.spacer(-1);
        ui.next_size(300, 0);
        ui.textbox(ID_HEX_PASTE, Some("Kopierte Bytes hier einfügen"), &mut self.paste, 512);
        ui.box_end();
        ui.box_end();
        ui.box_end();
        ui.scroll_end();
    }

    // the jobs page (every other navigation item)
    fn jobs(&mut self, ui: &mut Ui) {
        // header: breadcrumb, title, command bar
        ui.breadcrumb(ID_CRUMB, &["BackupCraft", "Schutz", "Sicherungsjobs"]);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.page_header("Sicherungsjobs", "9 Jobs · 2 benötigen Aufmerksamkeit · nächster Lauf heute 22:00");
        ui.spacer(-1);
        ui.next_flags(ST_SUBTLE);
        if ui.button(ID_TOAST, "Toast zeigen") & EV_ACT != 0 {
            ui.toast(IB_SUCCESS, "Job gestartet", "„Dateiserver – täglich“ läuft jetzt (Vorgang 7f3a).", 0);
        }
        ui.box_end();
        let sel = !self.grid.sel.is_empty();
        let clicked = ui.command_bar(ID_CMDBAR, &[
            CmdItem::new(ID_CMD_NEW, IC_ADD, "Neuer Job").flags(ST_ACCENT),
            CmdItem::new(ID_CMD_RUN, IC_PLAY, "Jetzt starten").enabled(sel).flags(CBI_SEP),
            CmdItem::new(ID_CMD_STOP, IC_STOP, "Stoppen").enabled(false),
            CmdItem::new(ID_CMD_EDIT, IC_EDIT, "Bearbeiten").enabled(sel),
            CmdItem::new(ID_CMD_DELETE, IC_DELETE, "Löschen").enabled(sel),
            CmdItem::new(ID_CMD_EXPORT, IC_DOWNLOAD, "Exportieren"),
            CmdItem::new(ID_CMD_REFRESH, IC_REFRESH, "Aktualisieren").flags(CBI_END),
        ]);
        if let Some(c) = clicked {
            ui.toast(IB_INFO, "Befehl", &format!("Befehl {} ausgeführt", c), 0);
        }
        self.kpis(ui);
        ui.next_size(-1, -1);
        ui.split_begin(ID_SPLIT, self.split, false);
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V, 14);
        self.charts(ui);
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP, 0);
        self.grid(ui);
        ui.box_end();
        ui.box_end();
        ui.split_next();
        ui.next_size(-1, -1);
        ui.scroll_begin(ID_SPLIT + 10, SC_V | pad(2));
        ui.box_begin(0, BOX_V, 12);
        self.side(ui);
        self.controls(ui);
        self.details(ui);
        ui.box_end();
        ui.scroll_end();
        let mut split = self.split;
        if ui.split_end(&mut split) {
            self.split = split;
        }
    }
}

impl App for Gal {
    fn init(&mut self, _ui: &mut Ui) -> bool {
        // the library's parts (date picker, weekdays, tree, command bar) speak the user's
        // language when the library has it; the gallery's own texts stay German
        i18n::set_catalog(i18n::Catalog::from_tsv(LANGUAGES));
        self.sort();
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_H, 0);
        self.nav(ui);
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | pad(20), 14);
        if self.page == PAGE_SIM {
            self.simulation(ui);
        } else {
            self.jobs(ui);
        }
        ui.box_end();
        ui.box_end();
        ui.toast_host();
    }

    fn command(&mut self, _ui: &mut Ui, id: u32, arg: u32) -> bool {
        if id != CMD_TL_MODS {
            return false;
        }
        self.tl.test_mods = arg;
        true
    }
}

ui::main!(ui::launch(&DESC, Gal::default(), &style::STYLE));
