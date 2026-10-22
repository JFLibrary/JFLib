// author -- making an assessment from the open project (A1): the panel "Aufgabe erstellen"
// (P_AUTHOR, right, a tab beside Eigenschaften; Werkzeuge › Aufgabe erstellen …)
//
//   Author::new()                 nothing begun
//   open(n)                       the panel shows; a project with an assessment fills it from
//                                 that (target, items, texts, feedback, time limit)
//   panel(n, ui)                  the steps, top to bottom:
//                                 1 Zielnetz -- "Aktuelles Netz als Ziel": the network as it runs
//                                   (jfn_sim::assess::capture) becomes the answer
//                                 2 Kriterien -- a tree of the target's settings (devices with
//                                   hostname, interfaces with address, state, VLAN and mode,
//                                   static routes, services, the running configuration's lines;
//                                   the cables): a check box grades one, and below the tree the
//                                   chosen item's points, comparison (gleichwertig, genau, einer
//                                   von mehreren with the other values), texts in both languages
//                                 3 Verbindungsprüfungen -- checks in the .scn syntax ("PC1
//                                   reaches 10.0.2.10") with points, refused with the reason
//                                   when they do not parse
//                                 4 Texte -- title, one line for the list, introduction, each in
//                                   German and English; the topic
//                                 5 Bewertung -- feedback (each item, the score only, nothing until
//                                   it is handed in) and a time limit in minutes
//                                 6 Ausgangsnetz -- "Ausgangsnetz erzeugen": the target without
//                                   what the items check becomes the project (one undo step;
//                                   jfn_sim::assess::start_network), or the network is edited by
//                                   hand; the score of the project as it is now
//                                 7 "Als Aufgabe speichern …": the lab with its assessment into
//                                   the project (one undo step), then the save dialog
//   command(n, ui, id) / states(n, ui)   CMD_AUTHOR, CMD_AUTHOR_TARGET, CMD_AUTHOR_START,
//                                 CMD_AUTHOR_SAVE
//   draft(n) -> Option<Assess>    the assessment as the panel has it (None: no target yet)
//
// An item's text starts as the default of its kind (jfn_sim::assess::item_text, German and
// English through the catalogues, workspace tr::msg_en) and follows the item until the author
// writes one.  The target travels in the saved file: an assessment is for learning, not for
// exams that must be tamper-proof.

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::lab::{canon, Assess, Compare, Feedback, Item, ItemKind, Lab, Target, Text};
use jfn_model::scenario;
use jfn_sim::assess::{candidates, capture, item_key, item_text, start_network, Cand, Grader};
use jfn_sim::Engine;
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{self, t, tc, tf};
use crate::app::Net;

/// points an item may be worth
const RG_POINTS: Range = Range::new(0, 100, 1, 0);
/// the time limit in minutes (0: none)
const RG_MINUTES: Range = Range::new(0, 600, 1, 0);
/// the score of the project shows again after a change at the earliest after this (µs)
const PREVIEW_US: u64 = 500_000;
/// the right side of the dock with the author's panel at least (DIP)
const AUTHOR_W: i32 = 420;

/// an item the author chose, with whether its text is still the default
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Draft {
    pub item: Item,
    pub auto: bool,
}

/// the assessment in the making
pub(crate) struct Author {
    /// the answer (None: not chosen yet)
    pub target: Option<Target>,
    /// the rows of the tree (the target's settings) and its state
    pub cands: Vec<Cand>,
    pub tree: TreeState,
    /// the chosen items: from the tree, then the checks, in the order chosen
    pub items: Vec<Draft>,
    /// the item being edited (an index of items)
    pub cur: Option<usize>,
    /// the other accepted values of the current item (Compare::Any, " | " between them) and its
    /// check's text, for the item they were made for
    pub alts: String,
    pub check: String,
    pub check_err: String,
    buf: Option<usize>,
    /// a new check: its expectation, points, why it was refused
    pub new_check: String,
    pub new_points: i32,
    pub new_err: String,
    /// the texts of the task (German, English), the topic (0 net, 1 ds)
    pub title: [String; 2],
    pub summary: [String; 2],
    pub intro: [String; 2],
    pub topic: i32,
    /// the feedback (Feedback::ALL index) and the time limit in minutes
    pub feedback: i32,
    pub minutes: i32,
    /// the score of the project now: (project revision, draft revision, task clock, points,
    /// total); the draft's revision
    preview: Option<(u32, u32, u64, u32, u32)>,
    rev: u32,
    /// what the last step said (an error, a confirmation)
    pub msg: String,
}

impl Author {
    pub(crate) fn new() -> Author {
        Author {
            target: None,
            cands: Vec::new(),
            tree: TreeState::default(),
            items: Vec::new(),
            cur: None,
            alts: String::new(),
            check: String::new(),
            check_err: String::new(),
            buf: None,
            new_check: String::new(),
            new_points: 2,
            new_err: String::new(),
            title: [String::new(), String::new()],
            summary: [String::new(), String::new()],
            intro: [String::new(), String::new()],
            topic: 0,
            feedback: 0,
            minutes: 0,
            preview: None,
            rev: 1,
            msg: String::new(),
        }
    }

    /// the draft changed (the preview grades again)
    fn touch(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }
}

/// an item's default texts in German and English
fn default_text(it: &Item) -> Text {
    let m = item_text(it);
    Text { de: m.text(), en: tr::msg_en(&m) }
}

/// a draft of a new item (its default text)
fn draft_of(it: Item) -> Draft {
    let mut item = it;
    item.text = default_text(&item);
    Draft { item, auto: true }
}

/// the tree's rows for a target: its project and an engine of it
fn tree_of(n: &Net, target: &Target) -> Vec<Cand> {
    let p = target.project(&n.doc.proj);
    match Engine::from_project(&p) {
        Ok(e) => candidates(&p, &e),
        Err(_) => Vec::new(),
    }
}

/// the panel shows (Werkzeuge › Aufgabe erstellen …): filled from the project's assessment the
/// first time, or when the panel holds nothing yet
pub(crate) fn open(n: &mut Net) {
    n.shell.dock.show(P_AUTHOR);
    n.shell.dock.activate(P_AUTHOR);
    if n.shell.dock.size(DOCK_RIGHT) < AUTHOR_W {
        n.shell.dock.set_size(DOCK_RIGHT, AUTHOR_W);
    }
    if n.author.target.is_some() || !n.author.items.is_empty() {
        return;
    }
    let lab = n.doc.proj.lab.clone();
    let a = &mut n.author;
    if let Some(l) = &lab {
        a.title = [l.title.de.clone(), l.title.en.clone()];
        a.summary = [l.summary.de.clone(), l.summary.en.clone()];
        a.intro = [l.intro.iter().map(|x| x.de.clone()).collect::<Vec<_>>().join("\n"), l.intro.iter().map(|x| x.en.clone()).collect::<Vec<_>>().join("\n")];
        a.topic = (l.topic == "ds") as i32;
    } else {
        a.title = [n.doc.proj.title.clone(), String::new()];
    }
    if let Some(asm) = lab.as_ref().and_then(|l| l.assess.clone()) {
        a.feedback = Feedback::ALL.iter().position(|f| *f == asm.feedback).unwrap_or(0) as i32;
        a.minutes = (asm.time_limit_s / 60) as i32;
        a.items = asm.items.iter().map(|it| Draft { item: it.clone(), auto: it.text.is_empty() }).collect();
        for d in a.items.iter_mut().filter(|d| d.auto) {
            d.item.text = default_text(&d.item);
        }
        if !asm.target.devices.is_empty() {
            let cands = tree_of(n, &asm.target);
            let a = &mut n.author;
            a.cands = cands;
            a.target = Some(asm.target);
            expand_devices(a);
        }
    }
    n.author.touch();
}

// the devices of the tree start open
fn expand_devices(a: &mut Author) {
    for c in a.cands.iter().filter(|c| c.depth == 0) {
        a.tree.expanded.insert(c.key);
    }
}

/// "Aktuelles Netz als Ziel": the network as it runs is the answer; chosen items keep their
/// points and texts and take the target's values
fn take_target(n: &mut Net) {
    let target = capture(&n.doc.proj, n.doc.sim.engine());
    let cands = candidates(&target.project(&n.doc.proj), n.doc.sim.engine());
    let a = &mut n.author;
    for d in a.items.iter_mut() {
        let k = item_key(&d.item);
        if let Some(c) = cands.iter().find(|c| c.key == k).and_then(|c| c.item.as_ref()) {
            if d.item.cmp != Compare::Any {
                d.item.value = c.value.clone();
            }
            if d.auto {
                d.item.text = default_text(&d.item);
            }
        }
    }
    let devices = target.devices.len();
    let links = target.links.len();
    a.target = Some(target);
    a.cands = cands;
    if a.tree.expanded.is_empty() {
        expand_devices(a);
    }
    a.msg = tf("Ziel übernommen: {} Geräte, {} Kabel.", &[&devices.to_string(), &links.to_string()]);
    a.touch();
}

/// the assessment as the panel has it (None: no target yet)
pub(crate) fn draft(n: &Net) -> Option<Assess> {
    let a = &n.author;
    let target = a.target.clone()?;
    // the tree's items in the order of the tree, then the checks
    let pos = |d: &Draft| a.cands.iter().position(|c| c.key == item_key(&d.item)).unwrap_or(usize::MAX);
    let mut items: Vec<&Draft> = a.items.iter().collect();
    items.sort_by_key(|d| (d.item.kind() == Some(ItemKind::Check), pos(d)));
    let items = items.into_iter().map(|d| d.item.clone()).collect();
    let feedback = Feedback::ALL.get(a.feedback.max(0) as usize).copied().unwrap_or_default();
    Some(Assess { feedback, time_limit_s: a.minutes.max(0) as u32 * 60, items, target })
}

/// the lab with the draft's assessment and texts (goals and hints of the project's lab stay)
fn lab_of(n: &Net, asm: Assess) -> Lab {
    let a = &n.author;
    let mut lab = n.doc.proj.lab.clone().unwrap_or_default();
    lab.title = Text::new(a.title[0].trim(), a.title[1].trim());
    lab.summary = Text::new(a.summary[0].trim(), a.summary[1].trim());
    let de: Vec<&str> = a.intro[0].split('\n').map(str::trim).filter(|s| !s.is_empty()).collect();
    let en: Vec<&str> = a.intro[1].split('\n').map(str::trim).filter(|s| !s.is_empty()).collect();
    lab.intro = de.iter().enumerate().map(|(i, p)| Text::new(p, en.get(i).copied().unwrap_or(""))).collect();
    lab.topic = String::from(if a.topic == 1 { "ds" } else { "net" });
    lab.assess = Some(asm);
    lab
}

/// "Als Aufgabe speichern …": checked, into the project (one undo step), then the save dialog
fn save(n: &mut Net, ui: &mut Ui) {
    let Some(asm) = draft(n) else {
        n.author.msg = String::from(t("Zuerst ein Ziel: „Aktuelles Netz als Ziel“."));
        return;
    };
    if asm.items.is_empty() {
        n.author.msg = String::from(t("Die Aufgabe braucht mindestens ein Kriterium."));
        return;
    }
    if n.author.title[0].trim().is_empty() {
        n.author.msg = String::from(t("Die Aufgabe braucht einen Titel (Deutsch)."));
        return;
    }
    let lab = lab_of(n, asm);
    if let Some(e) = crate::doc::assess::set_lab(&n.doc.proj, Some(lab)) {
        n.doc.apply(e);
    }
    n.author.msg = String::from(t("Die Aufgabe steht im Projekt; jetzt die Datei wählen."));
    crate::doc::file::save_as(n, ui);
}

/// "Ausgangsnetz erzeugen": the target without what the items check becomes the project
fn make_start(n: &mut Net) {
    let Some(target) = n.author.target.clone() else { return };
    let items: Vec<Item> = n.author.items.iter().map(|d| d.item.clone()).collect();
    let tp = target.project(&n.doc.proj);
    let Ok(e) = Engine::from_project(&tp) else { return };
    let start = start_network(&tp, &e, &items);
    if let Some(edit) = crate::doc::assess::network(&n.doc.proj, start.devices, start.links) {
        n.doc.apply(edit);
    }
    // the default texts name the devices as the start network does now
    for d in n.author.items.iter_mut().filter(|d| d.auto) {
        d.item.text = default_text(&d.item);
    }
    n.author.msg = String::from(t("Ausgangsnetz erzeugt (rückgängig mit Strg+Z)."));
    n.author.touch();
}

/// the commands of the author's panel
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32) -> bool {
    match id {
        CMD_AUTHOR => {
            open(n);
            ui.focus(ID_AU_TARGET);
        }
        CMD_AUTHOR_TARGET => take_target(n),
        CMD_AUTHOR_START => make_start(n),
        CMD_AUTHOR_SAVE => save(n, ui),
        _ => return false,
    }
    ui.repass();
    true
}

/// enabled of the commands (every frame)
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    let a = &n.author;
    ui.cmd_state(CMD_AUTHOR_START, if a.target.is_some() && !a.items.is_empty() { 0 } else { CF_DISABLED });
    ui.cmd_state(CMD_AUTHOR_SAVE, if a.target.is_some() && !a.items.is_empty() { 0 } else { CF_DISABLED });
}

// ---------------------------------------------------------------- the panel

// a step's card with its heading
fn step(ui: &mut Ui, title: &str) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 6);
    ui.heading(3, TS_BODY_STRONG, title);
}

// a labelled field: the label column, then the control (close with box_end)
fn field(ui: &mut Ui, label: &'static str) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(110, 0);
    ui.text(TS_BODY | TX_SECONDARY, tc(label));
}

// a text in German and English: the label, then a box for each (their placeholders name the
// language) -> one of them changed
fn two_texts(ui: &mut Ui, label: &'static str, ids: [u32; 2], bufs: &mut [String; 2], cap: u32, multi: bool) -> bool {
    let mut changed = false;
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H, 8);
    ui.next_size(110, 0);
    ui.text(TS_BODY | TX_SECONDARY, tc(label));
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, 4);
    for (k, lang) in [t("Deutsch"), t("English")].iter().enumerate() {
        ui.next_size(-1, if multi { 72 } else { 0 });
        let flags = if multi { TB_MULTI | TB_WRAP } else { 0 };
        if ui.textbox(ids[k], Some(lang), &mut bufs[k], cap | flags) & EV_ACT != 0 {
            changed = true;
        }
    }
    ui.box_end();
    ui.box_end();
    changed
}

// the visible rows of the tree: the indexes of the candidates whose groups are open
fn visible(a: &Author) -> Vec<usize> {
    let mut v = Vec::new();
    let mut hide: Option<u16> = None;
    for (i, c) in a.cands.iter().enumerate() {
        if let Some(d) = hide {
            if c.depth > d {
                continue;
            }
            hide = None;
        }
        v.push(i);
        let kids = a.cands.get(i + 1).is_some_and(|x| x.depth > c.depth);
        if kids && !a.tree.expanded.contains(&c.key) {
            hide = Some(c.depth);
        }
        if v.len() == AUTHOR_ROWS {
            break;
        }
    }
    v
}

/// the author's panel
pub(crate) fn panel(n: &mut Net, ui: &mut Ui) {
    let en = tr::en();
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_AU, SC_V | pad(12));
    ui.box_begin(0, BOX_V, 10);
    ui.next_size(-1, 0);
    ui.text(TS_SUBTITLE | TX_WRAP, t("Aufgabe erstellen"));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Aus dem Projekt wird eine Aufgabe mit Punkten: das fertige Netz als Ziel, die Kriterien daraus, dann das Netz, mit dem Lernende beginnen. Das Ziel steht in der Datei: zum Üben, nicht für Prüfungen, die niemand einsehen darf."));
    // ---- 1 the target
    step(ui, t("1  Zielnetz"));
    let status = match &n.author.target {
        Some(tg) => tf("Ziel: {} Geräte, {} Kabel.", &[&tg.devices.len().to_string(), &tg.links.len().to_string()]),
        None => String::from(t("Noch kein Ziel. Das Netz fertig einrichten, dann übernehmen.")),
    };
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_WRAP, &status);
    ui.next_icon(IC_FLAG);
    if ui.button(ID_AU_TARGET, t("Aktuelles Netz als Ziel")) & EV_ACT != 0 {
        ui.cmd_run(CMD_AUTHOR_TARGET, 0);
    }
    ui.tooltip(t("Das Netz, wie es jetzt läuft (mit den Running-Configs), wird die Lösung"));
    ui.box_end();
    // ---- 2 the items from the target
    step(ui, t("2  Kriterien aus dem Zielnetz"));
    if n.author.target.is_none() {
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Erst mit einem Ziel."));
    } else {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Ein Häkchen bewertet die Einstellung; ausgewählt lassen sich Punkte und Texte ändern."));
        tree(n, ui);
        editor(n, ui, en, false);
    }
    ui.box_end();
    // ---- 3 the checks
    step(ui, t("3  Verbindungsprüfungen"));
    checks(n, ui);
    ui.box_end();
    // ---- 4 the texts
    step(ui, t("4  Texte"));
    let a = &mut n.author;
    let mut changed = false;
    for (label, ids, bufs, cap, multi) in [
        ("Titel", [ID_AU_TITLE_DE, ID_AU_TITLE_EN], &mut a.title, 200, false),
        ("Kurztext", [ID_AU_SUMMARY_DE, ID_AU_SUMMARY_EN], &mut a.summary, 300, false),
        ("Einleitung", [ID_AU_INTRO_DE, ID_AU_INTRO_EN], &mut a.intro, 4000, true),
    ] {
        if two_texts(ui, label, ids, bufs, cap, multi) {
            changed = true;
        }
    }
    field(ui, "Thema");
    for (k, label) in [t("Netzwerk"), t("Verteilte Systeme")].iter().enumerate() {
        ui.radio(ID_AU_TOPIC + k as u32, label, &mut a.topic, k as i32);
    }
    ui.box_end();
    if changed {
        a.touch();
    }
    ui.box_end();
    // ---- 5 the feedback and the time limit
    step(ui, t("5  Bewertung"));
    for (k, label) in [t("Jedes Kriterium richtig oder falsch zeigen"), t("Nur die Punktzahl zeigen"), t("Nichts zeigen, bis abgegeben ist")].iter().enumerate() {
        ui.radio(ID_AU_FEEDBACK + k as u32, label, &mut a.feedback, k as i32);
    }
    field(ui, "Zeit (Minuten)");
    ui.next_size(120, 0);
    ui.number_box(ID_AU_MINUTES, &mut a.minutes, &RG_MINUTES);
    ui.text(TS_CAPTION | TX_TERTIARY, t("0: ohne Grenze"));
    ui.box_end();
    ui.box_end();
    // ---- 6 the start network
    step(ui, t("6  Ausgangsnetz"));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Lernende beginnen mit dem Netz des Projekts. „Ausgangsnetz erzeugen“ nimmt dem Ziel, was die Kriterien prüfen; oder das Netz von Hand zurückbauen."));
    let ready = n.author.target.is_some() && !n.author.items.is_empty();
    ui.next_icon(IC_REFRESH);
    if !ready {
        ui.next_flags(ST_DISABLED);
    }
    if ui.button(ID_AU_START, t("Ausgangsnetz erzeugen")) & EV_ACT != 0 {
        ui.cmd_run(CMD_AUTHOR_START, 0);
    }
    preview(n, ui);
    ui.box_end();
    // ---- 7 save
    step(ui, t("7  Speichern"));
    let total: u32 = n.author.items.iter().map(|d| d.item.points).sum();
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_WRAP, &tf("{} Kriterien, {}.", &[&n.author.items.len().to_string(), &crate::workspace::assess::points_text(total)]));
    ui.next_icon(IC_SAVE_AS);
    if !ready {
        ui.next_flags(ST_DISABLED);
    }
    if ui.accent_button(ID_AU_SAVE, t("Als Aufgabe speichern …")) & EV_ACT != 0 {
        ui.cmd_run(CMD_AUTHOR_SAVE, 0);
    }
    if !n.author.msg.is_empty() {
        let m = n.author.msg.clone();
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &m);
    }
    ui.box_end();
    ui.box_end();
    ui.scroll_end();
}

// the tree of the target's settings: a check box chooses an item, the current row is edited
fn tree(n: &mut Net, ui: &mut Ui) {
    let a = &mut n.author;
    let rows_ix = visible(a);
    let chosen: BTreeSet<u64> = a.items.iter().map(|d| item_key(&d.item)).collect();
    let labels: Vec<String> = rows_ix.iter().map(|&i| tr::msg(&a.cands[i].label)).collect();
    let details: Vec<String> = rows_ix
        .iter()
        .map(|&i| {
            let k = a.cands[i].key;
            a.items.iter().find(|d| item_key(&d.item) == k).map_or_else(String::new, |d| crate::workspace::assess::points_text(d.item.points))
        })
        .collect();
    let rows: Vec<TreeRow> = rows_ix
        .iter()
        .enumerate()
        .map(|(r, &i)| {
            let c = &a.cands[i];
            TreeRow {
                key: c.key,
                depth: c.depth,
                has_children: a.cands.get(i + 1).is_some_and(|x| x.depth > c.depth),
                check: c.item.as_ref().map(|_| chosen.contains(&c.key) as u8),
                icon: 0,
                label: &labels[r],
                detail: if details[r].is_empty() { None } else { Some(details[r].as_str()) },
            }
        })
        .collect();
    ui.next_size(-1, 260);
    let ev = ui.tree(ID_AU_TREE, &mut a.tree, &rows, 0);
    if let Some((key, on)) = ev.check {
        if on == 1 {
            if let Some(it) = a.cands.iter().find(|c| c.key == key).and_then(|c| c.item.clone())
                && !chosen.contains(&key)
            {
                a.items.push(draft_of(it));
                a.cur = Some(a.items.len() - 1);
            }
        } else {
            a.items.retain(|d| item_key(&d.item) != key);
            a.cur = None;
        }
        a.tree.cur = Some(key);
        a.touch();
        ui.repass();
    }
    if ev.sel_changed {
        let k = a.tree.cur;
        a.cur = a.items.iter().position(|d| Some(item_key(&d.item)) == k);
        ui.repass();
    }
}

// the editor of the current item: points, comparison, other values, texts; a check's
// expectation (`checks`: the editor below the checks)
fn editor(n: &mut Net, ui: &mut Ui, en: bool, checks: bool) {
    let a = &mut n.author;
    let Some(i) = a.cur.filter(|&i| i < a.items.len()) else {
        if !checks {
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Keine Einstellung gewählt."));
        }
        return;
    };
    let kind = a.items[i].item.kind();
    if (kind == Some(ItemKind::Check)) != checks {
        return;
    }
    // the buffers belong to the current item
    if a.buf != Some(i) {
        a.buf = Some(i);
        let it = &a.items[i].item;
        a.alts = if it.cmp == Compare::Any { it.values().iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(" | ") } else { String::new() };
        a.check = it.checks.join("; ");
        a.check_err.clear();
    }
    let mut changed = false;
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V | pad(6), 6);
    ui.next_size(-1, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, a.items[i].item.text.get(en));
    field(ui, "Punkte");
    ui.next_size(120, 0);
    let mut p = a.items[i].item.points as i32;
    if ui.number_box(ID_AU_POINTS, &mut p, &RG_POINTS) & (EV_ACT | EV_COMMIT) != 0 && p.max(0) as u32 != a.items[i].item.points {
        a.items[i].item.points = p.max(0) as u32;
        changed = true;
    }
    ui.box_end();
    if let Some(k) = kind.filter(|&k| k != ItemKind::Check) {
        field(ui, "Vergleich");
        let was = a.items[i].item.cmp;
        let mut cmp = Compare::ALL.iter().position(|c| *c == was).unwrap_or(0) as i32;
        ui.box_begin(0, BOX_V, 2);
        ui.radio(ID_AU_CMP, t("gleichwertig"), &mut cmp, 0);
        ui.radio(ID_AU_CMP + 1, t("genau"), &mut cmp, 1);
        if k.is_text() {
            ui.radio(ID_AU_CMP + 2, t("einer von mehreren Werten"), &mut cmp, 2);
        }
        ui.box_end();
        ui.box_end();
        let now = Compare::ALL.get(cmp as usize).copied().unwrap_or_default();
        if now != was {
            let it = &mut a.items[i].item;
            let first = it.values().first().map(|v| (*v).clone()).unwrap_or(Value::Null);
            if now == Compare::Any {
                // the value becomes the first of a list
                a.alts = first.as_str().map(String::from).unwrap_or_default();
                it.value = Value::Array(alloc::vec![first]);
            } else if was == Compare::Any {
                it.value = first;
            }
            it.cmp = now;
            changed = true;
        }
        if now == Compare::Any {
            field(ui, "Werte");
            ui.next_size(-1, 0);
            if ui.textbox(ID_AU_ALTS, Some(t("Werte, durch | getrennt")), &mut a.alts, 1000) & EV_ACT != 0 {
                let vals: Vec<Value> = a.alts.split('|').map(str::trim).filter(|s| !s.is_empty()).map(Value::string).collect();
                a.items[i].item.value = Value::Array(vals);
                changed = true;
            }
            ui.box_end();
            let bad: Option<String> = a.alts.split('|').map(str::trim).find(|s| !s.is_empty() && canon(k, s).is_none()).map(String::from);
            if let Some(b) = bad {
                crate::workspace::error_text(ui, &tf("„{}“ ist kein gültiger Wert.", &[&b]), 0);
            }
        }
    } else {
        field(ui, "Prüfung");
        ui.next_size(-1, 0);
        if ui.textbox(ID_AU_CHECK, Some(t("z. B. PC1 reaches 10.0.2.10")), &mut a.check, 1000) & EV_ACT != 0 {
            let list: Vec<String> = a.check.split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
            match list.iter().map(|c| scenario::parse_expectation(c)).find_map(Result::err) {
                Some(e) => a.check_err = e,
                None => {
                    a.check_err.clear();
                    a.items[i].item.checks = list;
                    changed = true;
                }
            }
        }
        ui.box_end();
        if !a.check_err.is_empty() {
            let e = a.check_err.clone();
            crate::workspace::error_text(ui, &e, 0);
        }
    }
    let text = &mut a.items[i].item.text;
    let mut bufs = [core::mem::take(&mut text.de), core::mem::take(&mut text.en)];
    let typed = two_texts(ui, "Text", [ID_AU_TEXT_DE, ID_AU_TEXT_EN], &mut bufs, 300, false);
    let [de, en] = bufs;
    (text.de, text.en) = (de, en);
    ui.next_flags(ST_SUBTLE | ST_COMPACT);
    ui.next_icon(IC_DELETE);
    let remove = ui.button(ID_AU_REMOVE, t("Nicht bewerten")) & EV_ACT != 0;
    ui.box_end();
    if typed {
        // the author's own words: the text no longer follows the item
        a.items[i].auto = false;
        changed = true;
    } else if changed && a.items[i].auto {
        a.items[i].item.text = default_text(&a.items[i].item);
    }
    if remove {
        a.items.remove(i);
        a.cur = None;
        a.buf = None;
        changed = true;
        ui.repass();
    }
    if changed {
        a.touch();
    }
}

// the checks: their rows (a click edits one, × removes it), a new one
fn checks(n: &mut Net, ui: &mut Ui) {
    let en = tr::en();
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Prüfungen in der Sprache der Szenarien, z. B. „PC1 reaches 10.0.2.10“ (ein Ping kommt an) oder „R1 route 10.0.3.0/24“; mehrere mit „;“ gelten zusammen."));
    let list: Vec<(usize, String, u32)> = n.author.items.iter().enumerate().filter(|(_, d)| d.item.kind() == Some(ItemKind::Check)).map(|(i, d)| (i, String::from(d.item.text.get(en)), d.item.points)).collect();
    let mut select = None;
    let mut remove = None;
    for (k, (i, text, points)) in list.iter().enumerate().take(AUTHOR_CHECKS) {
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.next_size(-1, 0);
        let on = n.author.cur == Some(*i);
        ui.next_flags(ST_SUBTLE | if on { ST_CHECKED } else { 0 });
        if ui.button(ID_AU_CHECK0 + k as u32, &format!("{} · {}", text, crate::workspace::assess::points_text(*points))) & EV_ACT != 0 {
            select = Some(*i);
        }
        if ui.icon_button(ID_AU_CHECK_X0 + k as u32, IC_CANCEL, t("Prüfung entfernen")) & EV_ACT != 0 {
            remove = Some(*i);
        }
        ui.box_end();
    }
    editor(n, ui, en, true);
    let a = &mut n.author;
    field(ui, "Neue Prüfung");
    ui.next_size(-1, 0);
    if ui.textbox(ID_AU_NEW_CHECK, Some(t("z. B. PC1 reaches 10.0.2.10")), &mut a.new_check, 1000) & EV_ACT != 0 {
        a.new_err.clear();
    }
    ui.box_end();
    field(ui, "Punkte");
    ui.next_size(100, 0);
    ui.number_box(ID_AU_NEW_POINTS, &mut a.new_points, &RG_POINTS);
    ui.next_icon(IC_ADD);
    let add = ui.button(ID_AU_ADD_CHECK, t("Hinzufügen")) & EV_ACT != 0;
    ui.box_end();
    if !a.new_err.is_empty() {
        let e = a.new_err.clone();
        crate::workspace::error_text(ui, &e, 0);
    }
    if add {
        let list: Vec<String> = a.new_check.split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
        match list.iter().map(|c| scenario::parse_expectation(c)).find_map(Result::err) {
            _ if list.is_empty() => a.new_err = String::from(t("Die Prüfung ist leer.")),
            Some(e) => a.new_err = e,
            None => {
                let it = Item { what: String::from(ItemKind::Check.as_str()), checks: list, points: a.new_points.max(0) as u32, ..Item::default() };
                a.items.push(draft_of(it));
                a.cur = Some(a.items.len() - 1);
                a.buf = None;
                a.new_check.clear();
                a.new_err.clear();
                a.touch();
            }
        }
        ui.repass();
    }
    if let Some(i) = select {
        a.cur = Some(i);
        a.buf = None;
        ui.repass();
    }
    if let Some(i) = remove {
        a.items.remove(i);
        a.cur = None;
        a.buf = None;
        a.touch();
        ui.repass();
    }
}

// the score of the project now against the draft (graded again after changes, at most every
// PREVIEW_US)
fn preview(n: &mut Net, ui: &mut Ui) {
    let clock = plat::task::now_us();
    let stale = n.author.preview.is_none_or(|(rev, drev, at, _, _)| (rev != n.doc.rev || drev != n.author.rev) && clock >= at + PREVIEW_US);
    if stale && let Some(asm) = draft(n) {
        let g = Grader::new(&asm, &n.doc.proj);
        let cons = &n.cons;
        let out = |dev: u32| cons.by_dev.get(&dev).map(|c| c.term.text()).unwrap_or_default();
        let s = g.grade(&n.doc.proj, n.doc.sim.engine(), &out);
        n.author.preview = Some((n.doc.rev, n.author.rev, clock, s.points, s.total));
    }
    if let (Some((_, _, _, p, total)), true) = (n.author.preview, n.author.target.is_some()) {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, &tf("Das Projekt jetzt: {} von {} Punkten.", &[&p.to_string(), &total.to_string()]));
    }
}
