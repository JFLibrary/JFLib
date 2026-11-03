// fwzones -- the parts Zonen and Sitzungen of the Firewall tab (firewall models): a plain picture of what
// may talk to what, zones with their interfaces, classes, policies and zone pairs as forms, the live
// sessions in a grid.  Like the rest of the tab (firewall.rs) the forms type command lines on the
// device's console as one undo step (console::run_lines_running), what shows is read back from the
// engine, and in a past view or without power everything shows but nothing changes
//
//   ZForm::new()                     nothing typed
//   part_zones(ui, id, f, s, lock, plan)     Zonen
//   part_sessions(ui, id, f, s, lock, rows, now, plan)   Sitzungen
//
// What the forms make (pure, for the tests in firewalltest.rs):
//   word_ok(text)                    a name of the configuration: one word, no "self"
//   zone_lines / zone_del_lines / member_lines      zones and their interfaces
//   class_lines / class_del_lines / match_del_lines   classes and their conditions
//   rule_lines / rule_del_lines / policy_del_lines   policies and what they do with a class
//   pair_lines / pair_del_lines      zone pairs with their policies
//   CLEAR_SESSIONS / CLEAR_COUNTERS  the lines of the two buttons
//   matrix(&Firewall) -> Matrix      who may talk to whom: the picture
//   session_rows(&[FwRow], now)      the rows of the grid
//   told_zones(&FwTest) -> Told      what the Test part says about zones and sessions, from the
//                                    answer Firewall::probe gives the engine
//
// Ids: ID_FW2 + 0x400 k (window k): + 0x00 a new zone's name, + 0x01 Anlegen; + 0x02 a class's name,
// + 0x03 what it matches, + 0x04 the protocol, + 0x05 the list, + 0x06 match-any, + 0x07 Klasse anlegen;
// + 0x08 a policy's name, + 0x09 the class, + 0x0A the action, + 0x0B log, + 0x0C Regel setzen; + 0x0D a
// pair's name, + 0x0E from, + 0x0F to, + 0x10 the policy, + 0x11 Paar anlegen; + 0x12 Sitzungen löschen,
// + 0x13 Zähler löschen; + 0x20 + i what removes a zone, + 0x40 + i the zone of the i-th interface,
// + 0x80 + i a class, + 0xA0 + i a condition, + 0xC0 + i a policy, + 0xE0 + i a policy's rule, + 0x100 + i a
// pair; + 0x200 the sessions' grid (rows + 1 + i, at most ROWS_MAX).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_sim::fw::{proto_word, Firewall, FwAction, FwProbe, FwRow, FwTest, Match, TcpState, CLASS_DEFAULT, FW_PROTOCOLS, SELF_ZONE};
use jfn_sim::Ns;
use ui::*;

use super::firewall::{heading, icon_button, key_of, note, offer, remove_button, Form, Plan, Snap, Told, ROWS_MAX};
use crate::app::constants::*;
use crate::app::tr::{t, tf};

/// the fields of the two parts
pub(crate) struct ZForm {
    pub zone: String,
    pub class: String,
    /// 0 a protocol, 1 an access list
    pub cond: i32,
    pub proto: i32,
    pub list: i32,
    pub any: i32,
    pub policy: String,
    pub rule_class: i32,
    pub action: i32,
    pub log: i32,
    pub pair: String,
    pub from: i32,
    pub to: i32,
    pub pair_policy: i32,
    pub grid: GridState,
}

impl ZForm {
    pub(crate) fn new() -> ZForm {
        ZForm { zone: String::new(), class: String::new(), cond: 0, proto: 0, list: 0, any: 0, policy: String::new(), rule_class: 0, action: 0, log: 0, pair: String::new(), from: 0, to: 0, pair_policy: 0, grid: GridState::default() }
    }
}

pub(crate) const CLEAR_SESSIONS: &str = "do clear zone-pair inspect sessions";
pub(crate) const CLEAR_COUNTERS: &str = "do clear zone-pair counter";

// i18n
const ACTIONS: [&str; 4] = ["inspect (mit Sitzung, Antworten kommen zurück)", "pass (nur dieses Paket)", "drop (still verwerfen)", "reject (verwerfen und Absender informieren)"];

// ---------------------------------------------------------------- what the forms type

/// a name of the configuration; Err: why not
pub(crate) fn word_ok(text: &str, what: &str) -> Result<String, String> {
    let name = text.trim();
    if name.is_empty() || name.split_whitespace().count() != 1 {
        return Err(tf("{} braucht einen Namen aus einem Wort.", &[what]));
    }
    if name.eq_ignore_ascii_case(SELF_ZONE) {
        return Err(String::from(t("Der Name self gehört dem Gerät selbst.")));
    }
    Ok(String::from(name))
}

/// a new zone
pub(crate) fn zone_lines(text: &str, fw: &Firewall) -> Result<Vec<String>, String> {
    let name = word_ok(text, t("Eine Zone"))?;
    if fw.zones.contains_key(&name) {
        return Err(tf("Die Zone {} gibt es schon.", &[&name]));
    }
    Ok(alloc::vec![format!("zone security {}", name)])
}

/// zone `name` goes; refused while an interface or a pair uses it
pub(crate) fn zone_del_lines(name: &str, fw: &Firewall) -> Result<Vec<String>, String> {
    if fw.members.values().any(|z| z == name) {
        return Err(tf("Zone {} hat noch Schnittstellen: erst sie einer anderen Zone geben.", &[name]));
    }
    if let Some((p, _)) = fw.pairs.iter().find(|(_, p)| p.src == name || p.dst == name) {
        return Err(tf("Zone {} steht noch im Zonenpaar {}: erst das Paar entfernen.", &[name, p]));
    }
    Ok(alloc::vec![format!("no zone security {}", name)])
}

/// the zone of an interface becomes `new`
pub(crate) fn member_lines(iface: &str, new: Option<&str>) -> Vec<String> {
    let mut v = alloc::vec![format!("interface {}", iface)];
    v.push(match new {
        Some(z) => format!("zone-member security {}", z),
        None => String::from("no zone-member security"),
    });
    v
}

/// a class with a condition, or one more condition of a class
pub(crate) fn class_lines(text: &str, any: bool, m: &Match) -> Result<Vec<String>, String> {
    let name = word_ok(text, t("Eine Klasse"))?;
    Ok(alloc::vec![format!("class-map type inspect {}{}", if any { "match-any " } else { "" }, name), m.config()])
}

/// class `name` goes; refused while a policy uses it
pub(crate) fn class_del_lines(name: &str, fw: &Firewall) -> Result<Vec<String>, String> {
    if let Some((p, _)) = fw.policies.iter().find(|(_, p)| p.classes.iter().any(|c| c.class == name)) {
        return Err(tf("Richtlinie {} benutzt die Klasse {}: erst die Regel dort entfernen.", &[p, name]));
    }
    Ok(alloc::vec![format!("no class-map type inspect {}", name)])
}

/// a condition of a class goes
pub(crate) fn match_del_lines(class: &str, any: bool, m: &Match) -> Vec<String> {
    alloc::vec![format!("class-map type inspect {}{}", if any { "match-any " } else { "" }, class), format!("no {}", m.config())]
}

// the line of a class in a policy
fn class_line(class: &str) -> String {
    if class == CLASS_DEFAULT { String::from("class class-default") } else { format!("class type inspect {}", class) }
}

/// what a policy does with a class (a new policy when there is none of the name)
pub(crate) fn rule_lines(policy: &str, class: &str, action: FwAction, log: bool) -> Result<Vec<String>, String> {
    let name = word_ok(policy, t("Eine Richtlinie"))?;
    Ok(alloc::vec![format!("policy-map type inspect {}", name), class_line(class), format!("{}{}", action.word(), if log { " log" } else { "" })])
}

/// a class leaves a policy (class-default goes back to dropping)
pub(crate) fn rule_del_lines(policy: &str, class: &str) -> Vec<String> {
    alloc::vec![format!("policy-map type inspect {}", policy), format!("no {}", class_line(class))]
}

/// policy `name` goes; refused while a pair uses it
pub(crate) fn policy_del_lines(name: &str, fw: &Firewall) -> Result<Vec<String>, String> {
    if let Some((p, _)) = fw.pairs.iter().find(|(_, p)| p.policy.as_deref() == Some(name)) {
        return Err(tf("Das Zonenpaar {} benutzt die Richtlinie {}: erst das Paar ändern.", &[p, name]));
    }
    Ok(alloc::vec![format!("no policy-map type inspect {}", name)])
}

/// a zone pair with its policy; an empty name becomes the two zones' names
pub(crate) fn pair_lines(text: &str, src: &str, dst: &str, policy: Option<&str>, fw: &Firewall) -> Result<Vec<String>, String> {
    if src == dst {
        return Err(String::from(t("Ein Zonenpaar verbindet zwei verschiedene Zonen.")));
    }
    let name = if text.trim().is_empty() { format!("{}-{}", src, dst) } else { word_ok(text, t("Ein Zonenpaar"))? };
    if let Some((other, _)) = fw.pair_of(src, dst).filter(|(n, _)| **n != name) {
        return Err(tf("Von {} nach {} gibt es schon das Zonenpaar {}.", &[src, dst, other]));
    }
    let mut v = alloc::vec![format!("zone-pair security {} source {} destination {}", name, src, dst)];
    match policy {
        Some(p) => v.push(format!("service-policy type inspect {}", p)),
        None => {
            if let Some(old) = fw.pairs.get(&name).and_then(|p| p.policy.clone()) {
                v.push(format!("no service-policy type inspect {}", old));
            }
        }
    }
    Ok(v)
}

/// a zone pair goes
pub(crate) fn pair_del_lines(name: &str) -> Vec<String> {
    alloc::vec![format!("no zone-pair security {}", name)]
}

// ---------------------------------------------------------------- the picture

/// what a direction between two zones allows
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// inside a zone, or the device itself without a policy
    Free,
    /// a policy decides: its name and whether something is let through at all
    Policy { name: String, open: bool },
    /// nothing gets through
    Closed,
}

/// who may talk to whom: the zones with a member (and self), the row of each to all of them
pub(crate) struct Matrix {
    pub zones: Vec<String>,
    pub cells: Vec<Vec<Reach>>,
}

/// the picture of the rules
pub(crate) fn matrix(fw: &Firewall) -> Matrix {
    let mut zones: Vec<String> = fw.zones.keys().cloned().collect();
    zones.push(String::from(SELF_ZONE));
    let cell = |a: &str, b: &str| -> Reach {
        if a == b {
            return Reach::Free;
        }
        let pair = fw.pair_of(a, b).and_then(|(_, p)| p.policy.clone());
        match pair.and_then(|n| fw.policies.get(&n).map(|p| (n, p))) {
            Some((name, p)) => {
                let moves = |x: Option<FwAction>| matches!(x, Some(FwAction::Inspect | FwAction::Pass));
                Reach::Policy { name, open: p.classes.iter().any(|c| moves(c.action)) || moves(p.default.action) }
            }
            None if a == SELF_ZONE || b == SELF_ZONE => Reach::Free,
            None => Reach::Closed,
        }
    };
    let cells = zones.iter().map(|a| zones.iter().map(|b| cell(a, b)).collect()).collect();
    Matrix { zones, cells }
}

// a cell of the picture: a dot and what it means
fn reach_cell(r: &Reach) -> (u32, String) {
    match r {
        Reach::Free => (BADGE_OK, String::from(t("frei"))),
        Reach::Policy { name, open: true } => (BADGE_WARN, name.clone()),
        Reach::Policy { name, open: false } => (BADGE_CRIT, name.clone()),
        Reach::Closed => (BADGE_CRIT, String::from(t("gesperrt"))),
    }
}

// ---------------------------------------------------------------- the Test part

/// what zones and sessions say about a packet, in words, from the function the engine acts on
pub(crate) fn told_zones(x: &FwTest) -> Told {
    let pass = x.probe.passes();
    let text = match &x.probe {
        FwProbe::NoZones => String::from(t("Keine der beiden Schnittstellen hat eine Zone: das Paket passiert.")),
        FwProbe::SameZone(z) => tf("Es bleibt in Zone {}: das Paket passiert.", &[z]),
        FwProbe::SelfPass { from, to } => tf("Von {} nach {}: das Gerät selbst, keine Richtlinie sagt etwas anderes, das Paket passiert.", &[from, to]),
        FwProbe::Unzoned { zone } => tf("Zwischen Zone {} und einer Schnittstelle ohne Zone: wird still verworfen.", &[zone]),
        FwProbe::NoRule { from, to } => tf("Keine Regel zwischen den Zonen {} und {}: wird still verworfen.", &[from, to]),
        FwProbe::Session { id, back: false, pair } => tf("Bestehende Verbindung: Sitzung {} in {}, das Paket passiert.", &[&id.to_string(), pair]),
        FwProbe::Session { id, back: true, pair } => tf("Bestehende Verbindung: Rückweg von Sitzung {} in {}, das Paket passiert.", &[&id.to_string(), pair]),
        FwProbe::Related { id, pair } => tf("ICMP-Fehler zu Sitzung {} in {}: das Paket passiert.", &[&id.to_string(), pair]),
        FwProbe::NotNew { pair, class } => tf("{}, Klasse {}: inspect, aber das Paket beginnt keine Verbindung und gehört zu keiner Sitzung, wird still verworfen.", &[pair, class]),
        FwProbe::Limit { pair, class, max } => tf("{}, Klasse {}: schon {} halboffene Sitzungen, wird still verworfen.", &[pair, class, &max.to_string()]),
        FwProbe::Policy { pair, policy, class, action, .. } => match action {
            FwAction::Inspect => tf("{}, Richtlinie {}, Klasse {}: inspect, das Paket passiert und beginnt eine Sitzung.", &[pair, policy, class]),
            FwAction::Pass => tf("{}, Richtlinie {}, Klasse {}: pass, das Paket passiert, eine Antwort braucht eine eigene Regel.", &[pair, policy, class]),
            FwAction::Drop => tf("{}, Richtlinie {}, Klasse {}: wird still verworfen.", &[pair, policy, class]),
            FwAction::Reject => tf("{}, Richtlinie {}, Klasse {}: wird verworfen, der Absender bekommt „administratively prohibited“.", &[pair, policy, class]),
        },
    };
    let list = match &x.probe {
        FwProbe::Session { pair, .. } | FwProbe::Related { pair, .. } | FwProbe::NotNew { pair, .. } | FwProbe::Limit { pair, .. } | FwProbe::Policy { pair, .. } => pair.clone(),
        _ => String::new(),
    };
    Told { pass, seq: 0, list, text }
}

// ---------------------------------------------------------------- Zonen

// the zones an interface or pair can name
fn zone_names(s: &Snap) -> Vec<&str> {
    s.fw.zones.keys().map(String::as_str).collect()
}

// the conditions of a class as one line
fn class_summary(c: &jfn_sim::fw::ClassMap) -> String {
    let conds: Vec<String> = c.matches.iter().map(|m| m.config().replace("match ", "")).collect();
    let join = if c.any { t("oder") } else { t("und") };
    if conds.is_empty() { String::from(t("keine Bedingung: trifft nichts")) } else { conds.join(&format!(" {} ", join)) }
}

// what a rule of a policy says
fn rule_text(class: &str, action: Option<FwAction>, log: bool) -> String {
    let a = action.map_or_else(|| String::from(t("drop (keine Aktion genannt)")), |x| String::from(x.word()));
    format!("{} → {}{}", class, a, if log { " log" } else { "" })
}

/// Zonen: the picture, zones and interfaces, classes, policies, pairs
pub(super) fn part_zones(ui: &mut Ui, id: u32, k: usize, f: &mut Form, s: &Snap, lock: u32, plan: &mut Option<Result<Plan, String>>) {
    let z = ID_FW2 + 0x400 * k as u32;
    let _ = id;
    let label = tf("Zonen auf {} ändern", &[&s.name]);
    let run = |lines: Vec<String>| Plan { label: label.clone(), lines, pick: None, list: None, close: false };
    let fw = &s.fw;
    // the picture
    heading(ui, "Wer darf mit wem sprechen?");
    if !fw.active() {
        note(ui, t("Noch hat keine Schnittstelle eine Zone: das Gerät arbeitet als Router, nur Zugriffslisten filtern. Sobald eine Schnittstelle in einer Zone ist, gilt: zwischen zwei Zonen wird verworfen, was keine Richtlinie erlaubt."));
    }
    let m = matrix(fw);
    if m.zones.len() > 1 {
        // (a narrow window makes the cells narrower: the texts end in dots, the tooltips say it all)
        let room = ui.px2dip(ui.box_rect()[2]) - 16;
        let label = (room / (m.zones.len() as i32 + 1)).clamp(56, 100);
        let cell = ((room - label) / m.zones.len() as i32 - 6).clamp(48, 104);
        ui.box_begin(0, BOX_V | BOX_CARD | pad(8), 4);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.next_size(label, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, t("von \\ nach"));
        for b in &m.zones {
            ui.next_size(cell, 0);
            ui.text(TS_CAPTION | TX_ELLIPSIS, b);
        }
        ui.box_end();
        for (a, row) in m.zones.iter().zip(&m.cells) {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
            ui.next_size(label, 0);
            ui.text(TS_CAPTION | TX_ELLIPSIS, a);
            for (b, r) in m.zones.iter().zip(row) {
                let (dot, text) = reach_cell(r);
                ui.next_size(cell, 0);
                ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
                ui.status_dot(dot);
                ui.next_size(-1, 0);
                ui.text(TS_CAPTION | TX_ELLIPSIS, &text);
                let say = tf("Von {} nach {}: {}", &[a, b, &text]);
                ui.tooltip(&say);
                ui.sem(K_TEXT, 0, Some(&say), 0);
                ui.box_end();
            }
            ui.box_end();
        }
        ui.box_end();
        note(ui, t("Grün: frei. Gelb: eine Richtlinie entscheidet. Rot: nichts kommt durch. Antworten auf das, was „inspect“ durchließ, kommen immer zurück. self ist das Gerät selbst."));
    }
    // zones and interfaces
    heading(ui, "Zonen und Schnittstellen");
    for (i, name) in fw.zones.keys().enumerate().take(32) {
        ui.box_begin(0, BOX_H | BOX_XCENTER | lock, 8);
        let members: Vec<&str> = fw.members.iter().filter(|(_, x)| *x == name).filter_map(|(ix, _)| s.ifs.iter().find(|y| y.index == *ix).map(|y| y.short.as_str())).collect();
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &format!("{}: {}", name, if members.is_empty() { String::from(t("keine Schnittstelle")) } else { members.join(", ") }));
        if remove_button(ui, z + 0x20 + i as u32, &tf("Zone {} entfernen", &[name])) {
            *plan = Some(offer(s, zone_del_lines(name, fw).map(&run)));
        }
        ui.box_end();
    }
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP | lock, 8);
    ui.next_size(180, 0);
    ui.textbox(z, Some(t("z. B. INNEN")), &mut f.z.zone, 32);
    if ui.button(z + 1, t("Zone anlegen")) & EV_ACT != 0 {
        *plan = Some(offer(s, zone_lines(&f.z.zone, fw).map(&run)));
    }
    ui.box_end();
    let names = zone_names(s);
    let mut items: Vec<&str> = alloc::vec![t("keine Zone")];
    items.extend(names.iter().copied());
    ui.box_begin(0, BOX_V | lock, 4);
    for (i, x) in s.ifs.iter().filter(|x| !x.addr.is_empty() || fw.members.contains_key(&x.index)).enumerate().take(64) {
        ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
        ui.next_size(170, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &if x.addr.is_empty() { x.short.clone() } else { format!("{}  {}", x.short, x.addr) });
        let cur = fw.members.get(&x.index).map(String::as_str);
        let mut sel = cur.and_then(|c| names.iter().position(|n| *n == c)).map_or(0, |i| i as i32 + 1);
        ui.next_size(150, 0);
        if ui.combobox(z + 0x40 + i as u32, &mut sel, &items) & EV_ACT != 0 {
            let new = if sel == 0 { None } else { names.get(sel as usize - 1).copied() };
            *plan = Some(offer(s, Ok(run(member_lines(&x.name, new)))));
        }
        ui.box_end();
    }
    ui.box_end();
    // classes
    heading(ui, "Klassen: welcher Verkehr?");
    let classes: Vec<&String> = fw.classes.keys().collect();
    let mut row = 0u32;
    for (i, name) in classes.iter().enumerate().take(32) {
        let Some(c) = fw.classes.get(*name) else { continue };
        ui.box_begin(0, BOX_H | BOX_XCENTER | lock, 8);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &format!("{}: {}", name, class_summary(c)));
        if remove_button(ui, z + 0x80 + i as u32, &tf("Klasse {} entfernen", &[name])) {
            *plan = Some(offer(s, class_del_lines(name, fw).map(&run)));
        }
        ui.box_end();
        // (a class with several conditions can lose one)
        if c.matches.len() > 1 {
            for m in &c.matches {
                ui.box_begin(0, BOX_H | BOX_XCENTER | lock, 8);
                ui.next_size(8, 0);
                ui.text(TS_CAPTION, "");
                ui.next_size(-1, 0);
                ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &m.config());
                if row < 32 && remove_button(ui, z + 0xA0 + row, &tf("Bedingung {} aus Klasse {} entfernen", &[&m.config(), name])) {
                    *plan = Some(offer(s, Ok(run(match_del_lines(name, c.any, m)))));
                }
                row += 1;
                ui.box_end();
            }
        }
    }
    let conds = [t("Protokoll"), t("Zugriffsliste")];
    let protos: Vec<&str> = FW_PROTOCOLS.iter().map(|x| x.0).collect();
    let lists = s.acls.names();
    let refs: Vec<&str> = lists.iter().map(String::as_str).collect();
    f.z.list = f.z.list.clamp(0, (refs.len() as i32 - 1).max(0));
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP | lock, 8);
    ui.next_size(130, 0);
    ui.textbox(z + 2, Some(t("Name der Klasse")), &mut f.z.class, 32);
    ui.next_size(120, 0);
    if ui.combobox(z + 3, &mut f.z.cond, &conds) & EV_ACT != 0 {
        ui.repass();
    }
    if f.z.cond == 0 {
        ui.next_size(100, 0);
        ui.combobox(z + 4, &mut f.z.proto, &protos);
    } else if refs.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("(noch keine)"));
    } else {
        ui.next_size(100, 0);
        ui.combobox(z + 5, &mut f.z.list, &refs);
    }
    ui.checkbox(z + 6, t("eine Bedingung genügt (match-any)"), &mut f.z.any);
    if ui.button(z + 7, t("Klasse anlegen oder ergänzen")) & EV_ACT != 0 {
        let cond = if f.z.cond == 0 { protos.get(f.z.proto as usize).map(|p| Match::Protocol(String::from(*p))) } else { refs.get(f.z.list as usize).map(|l| Match::Access(String::from(*l))) };
        let made = match cond {
            Some(m) => class_lines(&f.z.class, f.z.any != 0, &m).map(&run),
            None => Err(String::from(t("Es gibt noch keine Zugriffsliste: erst eine anlegen (Regeln)."))),
        };
        *plan = Some(offer(s, made));
    }
    ui.box_end();
    // policies
    heading(ui, "Richtlinien: was geschieht damit?");
    let mut rule_row = 0u32;
    for (i, (name, p)) in fw.policies.iter().enumerate().take(32) {
        ui.box_begin(0, BOX_V | BOX_CARD | pad(8) | lock, 2);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(-1, 0);
        ui.text(TS_BODY_STRONG | TX_ELLIPSIS, name);
        if remove_button(ui, z + 0xC0 + i as u32, &tf("Richtlinie {} entfernen", &[name])) {
            *plan = Some(offer(s, policy_del_lines(name, fw).map(&run)));
        }
        ui.box_end();
        for c in &p.classes {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &format!("{} ({} {})", rule_text(&c.class, c.action, c.log), c.hits, t("Pakete")));
            if rule_row < 32 && remove_button(ui, z + 0xE0 + rule_row, &tf("Klasse {} aus Richtlinie {} entfernen", &[&c.class, name])) {
                *plan = Some(offer(s, Ok(run(rule_del_lines(name, &c.class)))));
            }
            rule_row += 1;
            ui.box_end();
        }
        ui.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, &format!("{} ({} {})", rule_text(CLASS_DEFAULT, p.default.action, p.default.log), p.default.hits, t("Pakete")));
        ui.box_end();
    }
    let mut cnames: Vec<&str> = classes.iter().map(|x| x.as_str()).collect();
    cnames.push(CLASS_DEFAULT);
    f.z.rule_class = f.z.rule_class.clamp(0, cnames.len() as i32 - 1);
    let actions: Vec<&str> = ACTIONS.iter().map(|x| t(x)).collect();
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP | lock, 8);
    ui.next_size(160, 0);
    ui.textbox(z + 8, Some(t("Name der Richtlinie")), &mut f.z.policy, 32);
    ui.next_size(120, 0);
    ui.combobox(z + 9, &mut f.z.rule_class, &cnames);
    ui.next_size(340, 0);
    ui.combobox(z + 0x0A, &mut f.z.action, &actions);
    ui.checkbox(z + 0x0B, t("protokollieren (log)"), &mut f.z.log);
    if ui.button(z + 0x0C, t("Regel setzen")) & EV_ACT != 0 {
        let class = cnames.get(f.z.rule_class as usize).copied().unwrap_or(CLASS_DEFAULT);
        let action = FwAction::ALL.get(f.z.action as usize).copied().unwrap_or(FwAction::Inspect);
        *plan = Some(offer(s, rule_lines(&f.z.policy, class, action, f.z.log != 0).map(&run)));
    }
    ui.box_end();
    // pairs
    heading(ui, "Zonenpaare: in welche Richtung?");
    for (i, (name, p)) in fw.pairs.iter().enumerate().take(32) {
        ui.box_begin(0, BOX_H | BOX_XCENTER | lock, 8);
        ui.next_size(-1, 0);
        let pol = p.policy.clone().unwrap_or_else(|| String::from(t("keine Richtlinie")));
        ui.text(TS_BODY | TX_ELLIPSIS, &format!("{}: {} → {}, {}", name, p.src, p.dst, pol));
        if remove_button(ui, z + 0x100 + i as u32, &tf("Zonenpaar {} entfernen", &[name])) {
            *plan = Some(offer(s, Ok(run(pair_del_lines(name)))));
        }
        ui.box_end();
    }
    let mut ends: Vec<&str> = names.clone();
    ends.push(SELF_ZONE);
    f.z.from = f.z.from.clamp(0, ends.len() as i32 - 1);
    f.z.to = f.z.to.clamp(0, ends.len() as i32 - 1);
    let mut pols: Vec<&str> = alloc::vec![t("keine Richtlinie")];
    pols.extend(fw.policies.keys().map(String::as_str));
    f.z.pair_policy = f.z.pair_policy.clamp(0, pols.len() as i32 - 1);
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP | lock, 8);
    ui.next_size(130, 0);
    ui.textbox(z + 0x0D, Some(t("Name, frei lassen")), &mut f.z.pair, 32);
    ui.text(TS_BODY | TX_SECONDARY, t("von"));
    ui.next_size(110, 0);
    ui.combobox(z + 0x0E, &mut f.z.from, &ends);
    ui.text(TS_BODY | TX_SECONDARY, t("nach"));
    ui.next_size(110, 0);
    ui.combobox(z + 0x0F, &mut f.z.to, &ends);
    ui.next_size(130, 0);
    ui.combobox(z + 0x10, &mut f.z.pair_policy, &pols);
    if ui.button(z + 0x11, t("Paar anlegen")) & EV_ACT != 0 {
        let a = ends.get(f.z.from as usize).copied().unwrap_or(SELF_ZONE);
        let b = ends.get(f.z.to as usize).copied().unwrap_or(SELF_ZONE);
        let pol = if f.z.pair_policy == 0 { None } else { pols.get(f.z.pair_policy as usize).copied() };
        *plan = Some(offer(s, pair_lines(&f.z.pair, a, b, pol, fw).map(&run)));
    }
    ui.box_end();
    note(ui, t("Jeder Schritt läuft in der Konsole und wird zusammen mit der laufenden Konfiguration zur Startkonfiguration. Eine Klasse nennt Protokolle nur beim Namen: „http“ heißt TCP-Port 80, in die Daten schaut die Firewall nicht."));
}

// ---------------------------------------------------------------- Sitzungen

/// a row of the grid: Nr., Protokoll, Von, Nach, Zustand, Paar, Klasse, Pakete, Endet in
pub(crate) fn session_rows(rows: &[FwRow], now: Ns) -> Vec<[String; 9]> {
    rows.iter()
        .map(|r| {
            let x = &r.session;
            let ep = |a: netpkt::IpAddr, p: u16| -> String {
                let tp = x.proto == netpkt::ipv4::PROTO_TCP || x.proto == netpkt::ipv4::PROTO_UDP;
                match a {
                    netpkt::IpAddr::V4(y) if tp => format!("{}:{}", y, p),
                    netpkt::IpAddr::V6(y) if tp => format!("[{}]:{}", y, p),
                    netpkt::IpAddr::V4(y) => y.to_string(),
                    netpkt::IpAddr::V6(y) => y.to_string(),
                }
            };
            [
                x.id.to_string(),
                proto_word(x.proto),
                ep(x.src, x.sport),
                ep(x.dst, x.dport),
                String::from(x.tcp.map_or("–", |s| s.word())),
                x.pair.clone(),
                x.class.clone(),
                format!("{} / {}", x.packets[0], x.packets[1]),
                format!("{} s", r.ends.saturating_sub(now) / jfn_sim::SEC),
            ]
        })
        .collect()
}

fn session_cols(wide: bool) -> Vec<GridCol<'static>> {
    let mut v = alloc::vec![GridCol::new(t("Nr."), 44).flags(GC_NOSORT | GC_RIGHT), GridCol::new(t("Protokoll"), 78).flags(GC_NOSORT), GridCol::new(t("Von"), 150).flags(GC_NOSORT | GC_FILL), GridCol::new(t("Nach"), 150).flags(GC_NOSORT | GC_FILL)];
    if wide {
        v.push(GridCol::new(t("Zustand"), 92).flags(GC_NOSORT));
        v.push(GridCol::new(t("Paar"), 90).flags(GC_NOSORT));
        v.push(GridCol::new(t("Klasse"), 90).flags(GC_NOSORT));
        v.push(GridCol::new(t("Pakete"), 72).flags(GC_NOSORT | GC_RIGHT));
        v.push(GridCol::new(t("Endet in"), 70).flags(GC_NOSORT | GC_RIGHT));
    }
    v
}

/// Sitzungen: the grid of what runs now, the counters, the buttons
pub(super) fn part_sessions(ui: &mut Ui, id: u32, k: usize, f: &mut Form, s: &Snap, lock: u32, rows: &[FwRow], now: Ns, plan: &mut Option<Result<Plan, String>>) {
    let z = ID_FW2 + 0x400 * k as u32;
    let _ = id;
    let label = tf("Firewall auf {}", &[&s.name]);
    let run = |lines: Vec<String>| Plan { label: label.clone(), lines, pick: None, list: None, close: false };
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 12);
    heading(ui, "Laufende Sitzungen");
    ui.box_begin(0, BOX_H | lock, 8);
    if icon_button(ui, z + 0x12, IC_CLEAR, t("Sitzungen löschen"), true) {
        *plan = Some(offer(s, Ok(run(alloc::vec![String::from(CLEAR_SESSIONS)]))));
    }
    if icon_button(ui, z + 0x13, IC_REFRESH, t("Zähler löschen"), true) {
        *plan = Some(offer(s, Ok(run(alloc::vec![String::from(CLEAR_COUNTERS)]))));
    }
    ui.box_end();
    ui.box_end();
    let st = s.fw.stats;
    note(ui, &tf("Sitzungen jetzt: {}, halboffen: {} (höchstens {}). Begonnen: {}, abgelaufen: {}, durch RST beendet: {}. Verworfen ohne Regel: {}, ohne Verbindungsbeginn: {}, über der Grenze: {}.", &[&rows.len().to_string(), &rows.iter().filter(|r| r.session.tcp.is_some_and(TcpState::half_open)).count().to_string(), &s.fw.max_half_open.to_string(), &st.created.to_string(), &st.expired.to_string(), &st.reset.to_string(), &st.no_rule.to_string(), &st.not_new.to_string(), &st.limited.to_string()]));
    let shown = &rows[..rows.len().min(ROWS_MAX)];
    let texts = session_rows(shown, now);
    if texts.is_empty() {
        note(ui, t("Gerade läuft keine Sitzung. Eine Sitzung beginnt, wenn eine Klasse mit „inspect“ das erste Paket einer Verbindung durchlässt."));
        return;
    }
    let keys: Vec<u64> = shown.iter().map(|r| key_of(&r.session.id.to_string())).collect();
    f.z.grid.retain(&keys);
    let wide = ui.px2dip(ui.box_rect()[2]) >= 760;
    let cols = session_cols(wide);
    ui.next_size(-1, 34 + 28 * texts.len().clamp(3, 12) as i32);
    let (first, end) = ui.grid_begin(z + 0x200, &mut f.z.grid, &cols, &keys);
    for i in first..end {
        ui.grid_row(&mut f.z.grid, i);
        let x = &texts[i as usize];
        for c in 0..cols.len() {
            ui.grid_text(&f.z.grid, c, &x[c]);
        }
        let say = tf("Sitzung {}: {} {} nach {}, {}", &[&x[0], &x[1], &x[2], &x[3], &x[4]]);
        ui.sem(K_TEXT, 0, Some(&say), 0);
    }
    ui.grid_end(&mut f.z.grid);
    if rows.len() > ROWS_MAX {
        note(ui, &tf("Die ersten {} von {}; alle zeigt „show policy-map type inspect zone-pair sessions“.", &[&ROWS_MAX.to_string(), &rows.len().to_string()]));
    }
}
