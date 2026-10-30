// mib -- the MIB-Browser of a host's Desktop: the objects of the built-in MIB as a tree, an
// agent with its community and version, Get, GetNext, Walk and Set
//
//   MibState::new()                the browser with nothing chosen
//   browser(n, ui, k, dev)         the program in device window k (ids ID_MIB0 + 0x400 k ..)
//
// The browser has no SNMP of its own: its buttons type net-snmp's commands (snmpget,
// snmpgetnext, snmpwalk, snmpset) on the device's console as if by hand (doc/console.rs typed),
// so the requests are the device's and the answers stand in the console below the tree; a K1
// recording replays them as console lines.  The tree shows mib-2 (system, interfaces, ip, snmp,
// ifMIB) from netpkt::mib; a scalar's instance is 0, a column's the index of a row (ifIndex
// 1 at first).  Set takes the MIB's type ("=") or one the list names.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use netpkt::mib::{self, Access, ObjKind, OBJECTS};
use ui::*;

use crate::app::constants::ID_MIB0;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::console;

/// mib-2: the root of the tree
const ROOT: [u32; 6] = [1, 3, 6, 1, 2, 1];
/// the types snmpset takes (the first: the MIB's own) and their names
const TYPES: [&str; 7] = ["=", "s", "i", "u", "a", "o", "x"];
// i18n
const TYPE_NAMES: [&str; 7] = ["wie die MIB", "Text (s)", "Zahl (i)", "Zahl ohne Vorzeichen (u)", "IPv4-Adresse (a)", "Objektkennung (o)", "Bytes hexadezimal (x)"];

/// the browser's fields and tree
pub(crate) struct MibState {
    pub agent: String,
    pub community: String,
    /// 0: version 1, 1: 2c
    pub version: i32,
    pub tree: TreeState,
    pub inst: String,
    /// the object the instance belongs to
    inst_of: Option<usize>,
    pub kind: i32,
    pub value: String,
}

impl MibState {
    pub(crate) fn new() -> MibState {
        let mut tree = TreeState::default();
        // system open at the start
        if let Some(i) = OBJECTS.iter().position(|o| o.name == "system") {
            tree.expanded.insert(i as u64);
        }
        MibState { agent: String::new(), community: String::from("public"), version: 1, tree, inst: String::new(), inst_of: None, kind: 0, value: String::new() }
    }
}

/// the objects of the tree with their depth below mib-2, in OID order
fn objects() -> Vec<(usize, u16)> {
    let mut v = Vec::new();
    for (i, o) in OBJECTS.iter().enumerate() {
        if o.oid.len() <= ROOT.len() || !o.oid.starts_with(&ROOT) {
            continue;
        }
        let depth = OBJECTS.iter().filter(|p| p.oid.len() > ROOT.len() && p.oid.len() < o.oid.len() && o.oid.starts_with(p.oid)).count();
        v.push((i, depth as u16));
    }
    v
}

/// the program in device window k
pub(crate) fn browser(n: &mut Net, ui: &mut Ui, k: usize, dev: u32) {
    let id = ID_MIB0 + 0x400 * k as u32;
    let Some(w) = n.devwin.get(k) else { return };
    let st = &mut w.desk.mib;
    let mut cmd: Option<String> = None;
    // the agent
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
    ui.text(TS_BODY | TX_SECONDARY, t("Agent"));
    ui.next_size(160, 0);
    ui.textbox(id + 1, Some(t("z. B. 10.0.0.1")), &mut st.agent, 64);
    ui.text(TS_BODY | TX_SECONDARY, t("Community"));
    ui.next_size(120, 0);
    ui.textbox(id + 2, Some("public"), &mut st.community, 32);
    ui.text(TS_BODY | TX_SECONDARY, t("Version"));
    ui.next_size(80, 0);
    ui.combobox(id + 3, &mut st.version, &["1", "2c"]);
    ui.box_end();
    // the tree and what is chosen
    let all = objects();
    let mut rows_data: Vec<(u64, u16, bool, &'static str)> = Vec::new();
    let mut hidden_below: Option<&'static [u32]> = None;
    for &(i, depth) in &all {
        let o = &OBJECTS[i];
        if let Some(h) = hidden_below {
            if o.oid.starts_with(h) {
                continue;
            }
            hidden_below = None;
        }
        let kids = all.iter().any(|&(j, _)| OBJECTS[j].oid.len() > o.oid.len() && OBJECTS[j].oid.starts_with(o.oid));
        rows_data.push((i as u64, depth, kids, o.name));
        if kids && !st.tree.expanded.contains(&(i as u64)) {
            hidden_below = Some(o.oid);
        }
    }
    let rows: Vec<TreeRow> = rows_data.iter().map(|r| TreeRow { key: r.0, depth: r.1, has_children: r.2, check: None, icon: 0, label: r.3, detail: None }).collect();
    ui.next_size(-1, 280);
    ui.box_begin(0, BOX_H, 12);
    ui.next_size(260, -1);
    let ev = ui.tree(id + 0x100, &mut st.tree, &rows, 0);
    if ev.sel_changed || ev.expanded.is_some() || ev.collapsed.is_some() {
        ui.repass();
    }
    let chosen = st.tree.cur.map(|c| c as usize).filter(|&c| c < OBJECTS.len());
    if chosen != st.inst_of {
        st.inst_of = chosen;
        st.inst = match chosen.map(|c| OBJECTS[c].kind) {
            Some(ObjKind::Scalar) => String::from("0"),
            Some(ObjKind::Column) => String::from("1"),
            _ => String::new(),
        };
        st.value.clear();
    }
    ui.next_size(-1, -1);
    ui.box_begin(id + 4, BOX_V | BOX_CARD | pad(10), 6);
    match chosen {
        None => {
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Links ein Objekt wählen; Get, GetNext, Walk und Set laufen als Befehle in der Eingabeaufforderung des Geräts."));
        }
        Some(c) => {
            let o = &OBJECTS[c];
            let ready = !st.agent.trim().is_empty() && !st.community.trim().is_empty() && !st.agent.contains(char::is_whitespace) && !st.community.contains(char::is_whitespace);
            ui.text(TS_BODY_STRONG, &format!("{}::{}", o.module, o.name));
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_SECONDARY, &mib::name_of(o.oid, true));
            let what = match o.kind {
                ObjKind::Scalar | ObjKind::Column => {
                    let acc = match o.access {
                        Access::ReadWrite => t("lesen und schreiben"),
                        Access::ReadOnly => t("nur lesen"),
                        _ => t("nicht lesbar"),
                    };
                    let kind = if o.kind == ObjKind::Scalar { t("Einzelwert") } else { t("Spalte einer Tabelle") };
                    format!("{} · {} · {}", kind, o.syntax.name(), acc)
                }
                ObjKind::Table => String::from(t("Tabelle")),
                ObjKind::Entry => String::from(t("Zeile einer Tabelle")),
                _ => String::from(t("Knoten")),
            };
            ui.text(TS_CAPTION | TX_SECONDARY, &what);
            let leaf = matches!(o.kind, ObjKind::Scalar | ObjKind::Column);
            if leaf {
                ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
                ui.text(TS_BODY | TX_SECONDARY, t("Instanz"));
                ui.next_size(140, 0);
                ui.textbox(id + 5, Some(if o.kind == ObjKind::Scalar { "0" } else { "1" }), &mut st.inst, 64);
                ui.box_end();
            }
            let name = format!("{}::{}", o.module, o.name);
            let full = if leaf && !st.inst.trim().is_empty() { format!("{}.{}", name, st.inst.trim().trim_start_matches('.')) } else { name.clone() };
            let head = format!("-v{} -c {} {}", if st.version == 0 { "1" } else { "2c" }, st.community.trim(), st.agent.trim());
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.next_flags(if ready && leaf { 0 } else { ST_DISABLED });
            if ui.button(id + 6, t("Get")) & EV_ACT != 0 {
                cmd = Some(format!("snmpget {} {}", head, full));
            }
            ui.next_flags(if ready { 0 } else { ST_DISABLED });
            if ui.button(id + 7, t("GetNext")) & EV_ACT != 0 {
                cmd = Some(format!("snmpgetnext {} {}", head, full));
            }
            ui.next_flags(if ready { 0 } else { ST_DISABLED });
            if ui.button(id + 8, t("Walk")) & EV_ACT != 0 {
                cmd = Some(format!("snmpwalk {} {}", head, name));
            }
            ui.box_end();
            if leaf && o.access == Access::ReadWrite {
                ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
                let labels: Vec<&str> = TYPE_NAMES.iter().map(|x| t(x)).collect();
                ui.next_size(200, 0);
                ui.combobox(id + 9, &mut st.kind, &labels);
                ui.next_size(180, 0);
                ui.textbox(id + 10, Some(t("der neue Wert")), &mut st.value, 255);
                ui.next_flags(if ready && !st.value.contains('"') { ST_ACCENT } else { ST_DISABLED });
                if ui.button(id + 11, t("Set")) & EV_ACT != 0 {
                    let ty = TYPES[st.kind.clamp(0, TYPES.len() as i32 - 1) as usize];
                    cmd = Some(format!("snmpset {} {} {} \"{}\"", head, full, ty, st.value));
                }
                ui.box_end();
            }
            if !ready {
                ui.next_size(-1, 0);
                ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Agent und Community ohne Leerzeichen angeben."));
            }
        }
    }
    ui.box_end();
    ui.box_end();
    // the answers: the device's console
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY, &tf("Die Antworten in der Eingabeaufforderung von {}", &[&n.doc.proj.device(dev).map(|d| d.name.clone()).unwrap_or_else(|| dev.to_string())]));
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    console::view(n, ui, dev);
    ui.box_end();
    if let Some(c) = cmd {
        n.cons.focus = Some(dev);
        console::typed(n, dev, &c);
        ui.repass();
    }
}
