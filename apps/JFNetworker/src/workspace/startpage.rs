// startpage -- the page the window opens on, as an office program's: new projects and templates,
// the recent projects, the labs and assessments to learn with; and the project's info
//
//   Page::new()                  nothing read yet (the page reads its lists when it first shows)
//   at_start(opened, headless, opt_in, setting)   the page shows when the app starts
//   startup(n, ui, args)         Net::init: the page after everything else; a headless run asks
//                                for it with --start-page and takes its recent files from
//                                --recent=A|B (the format of the settings file)
//   show(n)                      the page again (Datei › Startseite, once the project has gone)
//   frame(n, ui)                 the page in the place of toolbar, dock and status bar, once per
//                                frame; it hosts the recovery bar and the files' info bar
//   states(n, ui) -> it shows    enabled per command while it shows: what makes sense without a
//                                project; leaving it restores the table's flags
//   command(n, ui, id, arg)      CMD_START_PAGE .. CMD_SP_FORGET
//   info_dialog(n, ui)           Datei › Projektinfo …: description and author, one undo step
//   TEMPLATES, TEMPLATE_DIR      the templates of the app in the order of the page; their German
//                                texts are the files' (startpagetest.rs compares them)
//   open_template(n, ui, k)      open_lab of its file, title and description in the window's language
//   rows_of(recent)              name, folder, state and time of the recent files
//   when_text(mtime_ns, now_ns, zone)   "Heute 14:05", "Gestern 09:10", else the date
//
// The page is a function of the app's data: the rows of Zuletzt are read when the list changes or
// the page comes back, never per frame, and only as far as the app may look: a file outside
// PERMISSIONS answers neither way, so it has no time and opens the file dialog as in the menu
// (file.rs, After::OpenPath); a file that is gone is marked and can leave the list.  Below the page
// the simulator runs an empty project, which arms no timer (Sim::arm), and autosave's timer waits
// (doc/autosave.rs), so an idle page wakes nothing.  Every way off the page ends in Net::after_load,
// which takes it down.
//
// learn() lists what there is to learn from: the courses of the Akademie (workspace/academy, AK1) as a
// group of their own, then the labs and assessments; there is no group without anything in it.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::Cell;

use datetime::{Timestamp, Zone};
use jfn_model::catalog;
use ui::*;

use crate::app::commands::CMDS;
use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;
use crate::doc::edits;
use crate::doc::file::{self, After};
use crate::workspace::icons::*;
use crate::workspace::task::{self, LabInfo};

/// the folder of the templates in the app's assets (apps/JFNetworker/assets/templates)
pub(crate) const TEMPLATE_DIR: &str = "app://templates";

/// a template: its file, its name and what it is (the file's title and description; English in
/// ui.tsv)
pub(crate) struct Template {
    pub file: &'static str,
    pub name: &'static str,
    pub text: &'static str,
}

// i18n: name text
pub(crate) const TEMPLATES: [Template; 6] = [
    Template { file: "heimnetz.jfnet", name: "Heimnetz", text: "Heimrouter mit WLAN und DHCP, dazu PC, Drucker, Laptop und Smartphone." },
    Template { file: "buero-vlans.jfnet", name: "Büro mit VLANs", text: "VLANs für Büro, Technik und Server, ein Router am Trunk, DHCP und DNS." },
    Template { file: "campus.jfnet", name: "Campus", text: "Zwei Gebäude mit L3-Switches, OSPF zum Router, DHCP über Relays, DNS." },
    Template { file: "rechenzentrum.jfnet", name: "Rechenzentrum (Leaf-Spine)", text: "Zwei Spines, vier Leafs, acht Server; geroutet mit OSPF über gleich gute Wege." },
    Template { file: "webdienst.jfnet", name: "Webdienst hinter Loadbalancer", text: "Ein Loadbalancer verteilt die Anfragen auf drei Webserver und prüft sie." },
    Template { file: "groesser-oder-mehr.jfnet", name: "Größer oder mehr?", text: "Ein Webdienst mit Datenbank: ein immer größerer Server gegen immer mehr kleine, als Experiment im Bereich Experimente." },
];

/// the page's content at most (DIP)
const PAGE_W: i32 = 1180;
/// from this content width Zuletzt and Lernen stand side by side (DIP)
const TWO_W: i32 = 860;
/// between cards, between the columns, between the sections (DIP)
const CARD_GAP: i32 = 12;
const COL_GAP: i32 = 28;
const SECTION_GAP: i32 = 24;
/// a section's heading (Zuletzt has its button there: the columns start level)
const HEAD_H: i32 = 32;
/// a card of Neu at least, a card of Lernen at least, the picture of Neu's cards (DIP)
const NEW_MIN_W: i32 = 150;
const LEARN_MIN_W: i32 = 230;
const PICTURE_H: i32 = 84;

/// what the app knows about a recent file
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// it can be read: its time is known
    Here,
    /// it is gone
    Gone,
    /// the app may not look (outside PERMISSIONS) or the system said something else
    Far,
}

/// a row of Zuletzt
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RecentRow {
    pub path: String,
    pub name: String,
    pub dir: String,
    pub reach: Reach,
    /// when it was changed (ns since 1970)
    pub mtime_ns: Option<i64>,
}

/// the devices and cables of a template, drawn small (render time)
struct Thumb {
    syms: Rc<Symbols>,
    /// (symbol, tint, centre x, centre y) in world units, the bounds of the centres
    nodes: Vec<(usize, usize, i32, i32)>,
    lines: Vec<(usize, usize)>,
    bounds: [i32; 4],
    /// the colour of the cables (set in the frame: the theme's)
    ink: Cell<u32>,
}

impl Draw for Thumb {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        if self.nodes.is_empty() {
            return;
        }
        let [x0, y0, x1, y1] = self.bounds;
        // a symbol stands as wide as it may, the rest of the area is the map of their centres
        let sw = (r[3] / 3).clamp(10, 40);
        let sh = sw * 3 / 4;
        let (mw, mh) = (r[2] - sw - sw / 2, r[3] - sh - sh / 2);
        let (span_x, span_y) = ((x1 - x0).max(1) as f32, (y1 - y0).max(1) as f32);
        let scale = (mw as f32 / span_x).min(mh as f32 / span_y).max(0.01);
        let (ox, oy) = (r[0] + (r[2] as f32 - span_x * scale) as i32 / 2, r[1] + (r[3] as f32 - span_y * scale) as i32 / 2);
        let at = |x: i32, y: i32| (ox + ((x - x0) as f32 * scale) as i32, oy + ((y - y0) as f32 * scale) as i32);
        {
            let mut sh2 = g.shapes();
            sh2.dc.set_color(self.ink.get());
            for &(a, b) in &self.lines {
                let (ax, ay) = at(self.nodes[a].2, self.nodes[a].3);
                let (bx, by) = at(self.nodes[b].2, self.nodes[b].3);
                sh2.line_fx(ax << 8, ay << 8, bx << 8, by << 8, 256);
            }
        }
        for &(sym, tint, x, y) in &self.nodes {
            let (cx, cy) = at(x, y);
            let h = self.syms.height(sym, sw);
            self.syms.draw(g, sym, cx - sw / 2, cy - h / 2, sw, tint, false);
        }
    }
}

/// what the page keeps
pub(crate) struct Page {
    /// the rows of Zuletzt and the list they were read for; read them again
    rows: Vec<RecentRow>,
    rows_for: Vec<String>,
    stale: bool,
    now_ns: i64,
    zone: Option<Zone>,
    /// the drawings of the templates, the labs and assessments (read when the page first shows)
    thumbs: Vec<Option<Rc<Thumb>>>,
    labs: Vec<LabInfo>,
    assess: Vec<LabInfo>,
    read: bool,
    /// the first card takes the keyboard when the page next shows
    focus_first: bool,
    /// the command table's flags were overridden (states)
    dimmed: Cell<bool>,
    /// Datei › Projektinfo …: open, the description takes the keyboard next, what is typed
    pub info_open: Shared<i32>,
    info_focus: bool,
    info_desc: String,
    info_author: String,
}

impl Page {
    pub(crate) fn new() -> Page {
        Page {
            rows: Vec::new(),
            rows_for: Vec::new(),
            stale: true,
            now_ns: 0,
            zone: None,
            thumbs: Vec::new(),
            labs: Vec::new(),
            assess: Vec::new(),
            read: false,
            focus_first: true,
            dimmed: Cell::new(false),
            info_open: Shared::new(0),
            info_focus: false,
            info_desc: String::new(),
            info_author: String::new(),
        }
    }
}

/// the page shows when the app starts: nothing was opened, the setting is on; a headless run (the
/// window scripts) only when it asks with --start-page
pub(crate) fn at_start(opened: bool, headless: bool, opt_in: bool, setting: bool) -> bool {
    !opened && setting && (!headless || opt_in)
}

/// Net::init, after everything else: the page if it is due
pub(crate) fn startup(n: &mut Net, ui: &mut Ui, args: &[String]) {
    if ui.headless() {
        // the recent files of a test: one path alone is the user's (the command line grants it), of
        // several joined with | none is, like a file of an earlier session
        for v in args.iter().filter_map(|a| a.strip_prefix("--recent=")) {
            n.file.recent.extend(v.split('|').filter(|p| !p.is_empty()).map(file::absolute));
        }
        n.file.recent.truncate(RECENT_MAX);
    }
    let opened = n.doc.path.is_some() || plat::arg_value("--gen-topology").is_some();
    let opt_in = args.iter().any(|a| a == "--start-page");
    if at_start(opened, ui.headless(), opt_in, n.prefs.start_page) {
        show(n);
    }
}

/// the page comes (back): its rows are read again, the first card takes the keyboard
pub(crate) fn show(n: &mut Net) {
    n.shell.start = true;
    n.shell.page.stale = true;
    n.shell.page.focus_first = true;
}

/// name, folder and state of the recent files, as far as the app may look
pub(crate) fn rows_of(recent: &[String]) -> Vec<RecentRow> {
    recent
        .iter()
        .map(|p| {
            let (reach, mtime_ns) = match plat::file_exists_checked(p) {
                Some(0) => (Reach::Gone, None),
                Some(_) => (Reach::Here, plat::fs::stat(p).map(|m| m.mtime_ns)),
                None => (Reach::Far, None),
            };
            RecentRow { path: p.clone(), name: String::from(file::base_name(p)), dir: String::from(plat::fs::parent(p)), reach, mtime_ns }
        })
        .collect()
}

/// "Heute 14:05", "Gestern 09:10", else the date, in the window's language
pub(crate) fn when_text(mtime_ns: i64, now_ns: i64, zone: &Zone) -> String {
    let ts = Timestamp::from_nanos(mtime_ns);
    let a = zone.to_local(ts);
    let b = zone.to_local(Timestamp::from_nanos(now_ns));
    let hm = format!("{:02}:{:02}", a.hour, a.minute);
    match datetime::days_from_civil(b.year, b.month, b.day) - datetime::days_from_civil(a.year, a.month, a.day) {
        0 => tf("Heute {}", &[&hm]),
        1 => tf("Gestern {}", &[&hm]),
        _ => datetime::format_date(ts, zone, tr::en()),
    }
}

// the zone of the system (UTC when it does not say)
fn local_zone() -> Zone {
    let id = plat::fs::local_zone_id().unwrap_or_default();
    match id.strip_prefix("win:") {
        Some(key) => Zone::from_windows(key),
        None => Zone::by_name(&id),
    }
    .unwrap_or_else(Zone::utc)
}

// a template's devices and cables for its card
fn thumb_of(n: &Net, k: usize) -> Option<Rc<Thumb>> {
    let p = file::read(&format!("{}/{}", TEMPLATE_DIR, TEMPLATES.get(k)?.file)).ok()?;
    let mut ids = Vec::new();
    let mut nodes = Vec::new();
    for d in &p.devices {
        let m = catalog::model(&d.model)?;
        ids.push(d.id);
        nodes.push((n.syms.index(m.symbol)?, tint_of(m.category), d.x + 32, d.y + 24));
    }
    let lines = p.links.iter().filter_map(|l| Some((ids.iter().position(|&i| i == l.a.dev)?, ids.iter().position(|&i| i == l.b.dev)?))).collect();
    let mut b = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
    for &(_, _, x, y) in &nodes {
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    Some(Rc::new(Thumb { syms: n.syms.clone(), nodes, lines, bounds: b, ink: Cell::new(0) }))
}

// what the page shows: its lists once, the rows when they are stale
fn prepare(n: &mut Net) {
    if !n.shell.page.read {
        n.shell.page.read = true;
        n.shell.page.thumbs = (0..TEMPLATES.len()).map(|k| thumb_of(n, k)).collect();
        n.shell.page.labs = task::labs();
        n.shell.page.assess = task::assessments();
        n.shell.page.zone = Some(local_zone());
        n.academy.ensure();
    }
    let page = &mut n.shell.page;
    if page.stale || page.rows_for != n.file.recent {
        page.stale = false;
        page.rows_for = n.file.recent.clone();
        page.rows = rows_of(&n.file.recent);
        page.now_ns = plat::unix_time_ns();
    }
}

/// open template k: a project without a file, called as the template in the window's language
pub(crate) fn open_template(n: &mut Net, ui: &mut Ui, k: usize) {
    let Some(x) = TEMPLATES.get(k) else { return };
    if file::open_lab(n, ui, &format!("{}/{}", TEMPLATE_DIR, x.file)) {
        n.doc.proj.title = String::from(t(x.name));
        n.doc.proj.description = String::from(t(x.text));
    }
}

/// enabled per command while the page shows (every frame) -> it shows (commands.rs sets the others)
pub(crate) fn states(n: &Net, ui: &mut Ui) -> bool {
    let dimmed = &n.shell.page.dimmed;
    if !n.shell.start && !n.academy.on {
        // back on the project: the table's flags, then the states of the frame
        if dimmed.replace(false) {
            for c in CMDS.iter() {
                ui.cmd_state(c.id, c.flags as u32);
            }
        }
        return false;
    }
    dimmed.set(true);
    let course = n.academy.on;
    for c in CMDS.iter() {
        let free = matches!(c.id, CMD_NEW | CMD_OPEN | CMD_LAB_OPEN | CMD_QUIT | CMD_SETTINGS | CMD_SHORTCUTS | CMD_ABOUT | CMD_DESK_FULL | CMD_ACADEMY) || (course && matches!(c.id, CMD_START_PAGE | CMD_AK_BACK));
        ui.cmd_state(c.id, if free { 0 } else { CF_DISABLED });
    }
    true
}

/// the page's commands and the project's info
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        CMD_START_PAGE => file::request(n, ui, After::Start),
        CMD_PROJECT_INFO => {
            let page = &mut n.shell.page;
            page.info_desc = n.doc.proj.description.clone();
            page.info_author = n.doc.proj.author.clone();
            page.info_open.set(1);
            page.info_focus = true;
        }
        CMD_SP_TEMPLATE => file::request(n, ui, After::Template(arg as usize)),
        CMD_SP_LEARN => {
            let page = &n.shell.page;
            let Some(l) = page.labs.iter().chain(page.assess.iter()).nth(arg as usize) else { return true };
            let path = l.path.clone();
            file::request(n, ui, After::Lab(path));
        }
        CMD_SP_FORGET => {
            if (arg as usize) < n.file.recent.len() {
                n.file.recent.remove(arg as usize);
            }
        }
        _ => return false,
    }
    ui.repass();
    true
}

/// the page, in the place of toolbar, dock and status bar
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    prepare(n);
    if core::mem::take(&mut n.shell.page.focus_first) {
        // a recovery offer comes first, it must not get lost; else the first card
        ui.focus(if n.autosave.offer.is_some() { ID_RECOVER_OK } else { ID_SP_NEW });
    }
    ui.focus_pane_begin(ID_SP_PANE);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    crate::doc::autosave::bar(n, ui);
    crate::workspace::canvas::infobar(n, ui);
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_SP_SCROLL, SC_V | pad(24));
    let w = ui.px2dip(ui.box_rect()[2]).clamp(1, PAGE_W);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_CENTER, 0);
    ui.next_size(w, 0);
    ui.box_begin(0, BOX_V, SECTION_GAP);
    ui.box_begin(0, BOX_V, 2);
    ui.text(TS_TITLE, "JFNetworker");
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Netzwerke entwerfen, simulieren und verstehen."));
    ui.box_end();
    new_section(n, ui, w);
    if w >= TWO_W {
        let left = (w - COL_GAP) / 2;
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H, COL_GAP);
        ui.next_size(left, 0);
        ui.box_begin(0, BOX_V, 0);
        recent_section(n, ui);
        ui.box_end();
        ui.next_size(w - COL_GAP - left, 0);
        ui.box_begin(0, BOX_V, 0);
        learn(n, ui, w - COL_GAP - left);
        ui.box_end();
        ui.box_end();
    } else {
        recent_section(n, ui);
        learn(n, ui, w);
    }
    ui.box_end();
    ui.box_end();
    ui.scroll_end();
    ui.box_end();
    ui.focus_pane_end();
}

// a section's heading: its row stays open for a button at its end, the caller's box_end closes it
fn heading(ui: &mut Ui, s: &str) {
    ui.next_size(-1, HEAD_H);
    ui.box_begin(0, BOX_H | BOX_XCENTER, CARD_GAP);
    ui.next_size(-1, 0);
    ui.text(TS_SUBTITLE | TX_ELLIPSIS, s);
}

// how many cards stand in a row of width w: the largest of `counts` that leaves each its minimum
fn columns(w: i32, counts: &[i32], min_w: i32) -> i32 {
    counts.iter().copied().find(|&c| (w - CARD_GAP * (c - 1)) / c >= min_w).unwrap_or(1)
}

// the width of card k of `cols` in a row: together with their gaps exactly w
fn card_w(w: i32, cols: i32, k: i32) -> i32 {
    let room = w - CARD_GAP * (cols - 1);
    (room * (k + 1) / cols - room * k / cols).max(1)
}

// Neu: the empty project, then the templates
fn new_section(n: &mut Net, ui: &mut Ui, w: i32) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, CARD_GAP);
    heading(ui, t("Neu"));
    ui.box_end();
    let cols = columns(w, &[6, 3, 2], NEW_MIN_W);
    let ink = ui.color(T_TEXT3);
    let key = n.syms.key();
    let all: Vec<usize> = (0..=TEMPLATES.len()).collect();
    for row in all.chunks(cols as usize) {
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H, CARD_GAP);
        for (c, &k) in row.iter().enumerate() {
            ui.next_size(card_w(w, cols, c as i32), 0);
            let ev = if k == 0 {
                ui.action_card(ID_SP_NEW, t("Leeres Projekt"), &CardDesc { title: t("Leeres Projekt"), text: t("Eine leere Zeichenfläche: Geräte aus der Palette ziehen und verbinden."), icon: IC_ADD, picture_h: PICTURE_H, ..CardDesc::default() })
            } else {
                let x = &TEMPLATES[k - 1];
                let thumb = n.shell.page.thumbs.get(k - 1).cloned().flatten();
                if let Some(d) = &thumb {
                    d.ink.set(ink);
                }
                let draw = thumb.map(|d| -> DrawRef { d });
                let ver = key ^ (k as u32).wrapping_mul(0x9E37_79B9) ^ ink;
                ui.action_card(ID_SP_NEW + k as u32, &tf("Vorlage: {}", &[t(x.name)]), &CardDesc { title: t(x.name), text: t(x.text), icon: IC_DOCUMENT, picture_h: PICTURE_H, draw, draw_ver: ver, ..CardDesc::default() })
            };
            if ev & EV_ACT != 0 {
                if k == 0 {
                    ui.cmd_run(CMD_NEW, 0);
                } else {
                    ui.cmd_run(CMD_SP_TEMPLATE, k as u32 - 1);
                }
            }
        }
        ui.box_end();
    }
    ui.box_end();
}

// Zuletzt: Öffnen …, the recent files with folder and time; a missing one is marked and can go
fn recent_section(n: &mut Net, ui: &mut Ui) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, CARD_GAP);
    heading(ui, t("Zuletzt"));
    ui.next_icon(IC_OPEN_FILE);
    if ui.button(ID_SP_OPEN, t("Öffnen …")) & EV_ACT != 0 {
        ui.cmd_run(CMD_OPEN, 0);
    }
    ui.box_end();
    let rows = n.shell.page.rows.clone();
    if rows.is_empty() {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Hier erscheinen die Projekte, die Sie öffnen und speichern."));
    } else {
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | BOX_CARD | pad(4), 2);
        for (i, r) in rows.iter().enumerate().take(RECENT_MAX) {
            let i = i as u32;
            let when = match (r.mtime_ns, n.shell.page.zone.as_ref()) {
                (Some(m), Some(z)) => when_text(m, n.shell.page.now_ns, z),
                _ => String::new(),
            };
            let gone = r.reach == Reach::Gone;
            let name = if gone { tf("{}, nicht gefunden", &[&r.name]) } else { r.name.clone() };
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
            ui.next_size(-1, 0);
            ui.next_flags(ST_SUBTLE);
            let d = CardDesc { title: &r.name, icon: IC_DOCUMENT, text: &r.dir, one_line: true, aside: &when, badge: if gone { Some((BADGE_WARN, t("Nicht gefunden"))) } else { None }, ..CardDesc::default() };
            if ui.action_card(ID_SP_RECENT + i, &name, &d) & EV_ACT != 0 {
                ui.cmd_run(CMD_RECENT0 + i, 0);
            }
            ui.tooltip(&r.path);
            if gone {
                ui.next_flags(ST_ICON_ONLY | ST_SUBTLE);
                if ui.icon_button(ID_SP_FORGET + i, IC_CANCEL, t("Aus der Liste entfernen")) & EV_ACT != 0 {
                    ui.cmd_run(CMD_SP_FORGET, i);
                }
            }
            ui.box_end();
        }
        ui.box_end();
    }
    ui.box_end();
}

// Lernen: the courses of the Akademie, the labs and assessments of the app.
fn learn(n: &mut Net, ui: &mut Ui, w: i32) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, CARD_GAP);
    heading(ui, t("Lernen"));
    ui.box_end();
    course_group(n, ui, w);
    let (labs, assess) = (n.shell.page.labs.clone(), n.shell.page.assess.clone());
    learn_group(ui, t("Labore: ein Netz, Ziele, die der Simulator prüft, und Hinweise."), &labs, 0, IC_FLAG, w);
    learn_group(ui, t("Aufgaben mit Bewertung: das Netz aufbauen und Punkte bekommen."), &assess, labs.len() as u32, IC_ACCEPT, w);
    ui.box_end();
}

// the courses of the Akademie: their line, their cards with what the learner has done
fn course_group(n: &mut Net, ui: &mut Ui, w: i32) {
    let cards = n.academy.cards();
    if cards.is_empty() {
        return;
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Akademie: Kurse mit Lektionen zum Lesen und Laboren im Simulator."));
    let cols = columns(w, &[2], LEARN_MIN_W);
    for (r, row) in cards.chunks(cols as usize).enumerate() {
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H, CARD_GAP);
        for (c, x) in row.iter().enumerate() {
            let i = (r * cols as usize + c) as u32;
            ui.next_size(card_w(w, cols, c as i32), 0);
            let badge = x.badge.as_ref().map(|(k, s)| (*k, s.as_str()));
            let d = CardDesc { title: &x.title, icon: IC_OFFICE_READ_MODE, text: &x.text, badge, ..CardDesc::default() };
            if ui.action_card(ID_SP_COURSE + i, &tf("Kurs: {}", &[&x.title]), &d) & EV_ACT != 0 {
                ui.cmd_run(CMD_SP_COURSE, i);
            }
        }
        ui.box_end();
    }
}

// a group of Lernen: its line, its cards (index `first` and on in the page's list)
fn learn_group(ui: &mut Ui, what: &str, items: &[LabInfo], first: u32, icon: u32, w: i32) {
    if items.is_empty() {
        return;
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, what);
    let cols = columns(w, &[2], LEARN_MIN_W);
    let en = tr::en();
    for (r, row) in items.chunks(cols as usize).enumerate() {
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H, CARD_GAP);
        for (c, l) in row.iter().enumerate() {
            let i = first + (r * cols as usize + c) as u32;
            let aside = task::size_text(l.goals, l.points);
            let text = format!("{} · {}", aside, l.summary.get(en));
            ui.next_size(card_w(w, cols, c as i32), 0);
            let d = CardDesc { title: l.title.get(en), icon, text: &text, ..CardDesc::default() };
            if ui.action_card(ID_SP_LEARN + i, l.title.get(en), &d) & EV_ACT != 0 {
                ui.cmd_run(CMD_SP_LEARN, i);
            }
            ui.tooltip(task::topic_name(&l.topic));
        }
        ui.box_end();
    }
}

/// Datei › Projektinfo …: description and author, one undo step when they changed
pub(crate) fn info_dialog(n: &mut Net, ui: &mut Ui) {
    let open = n.shell.page.info_open.clone();
    if ui.dialog_begin(ID_SP_INFO, t("Projektinfo"), &open) {
        let page = &mut n.shell.page;
        if core::mem::take(&mut page.info_focus) {
            ui.focus(ID_SP_INFO_DESC);
        }
        ui.next_size(440, 0);
        ui.box_begin(0, BOX_V, 6);
        ui.text(TS_BODY_STRONG, t("Beschreibung"));
        ui.next_size(-1, 110);
        ui.textbox(ID_SP_INFO_DESC, Some(t("Worum es in diesem Projekt geht")), &mut page.info_desc, 2000 | TB_MULTI | TB_WRAP);
        ui.spacer(4);
        ui.text(TS_BODY_STRONG, t("Autor"));
        ui.next_size(-1, 0);
        ui.textbox(ID_SP_INFO_AUTHOR, Some(t("Ihr Name")), &mut page.info_author, 120);
        ui.box_end();
        if ui.dialog_buttons(Some(t("Übernehmen")), None, Some(t("Abbrechen"))) == DLG_PRIMARY {
            if let Some(e) = edits::info(&n.doc.proj, &n.shell.page.info_desc, &n.shell.page.info_author) {
                n.doc.apply(e);
                ui.repass();
            }
        }
    }
    ui.dialog_end();
}
