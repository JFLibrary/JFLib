// packet -- the Paket panel: what a device did with a frame (the frame that came in, its
// decisions, the frames it sent), the layers of one of those frames as a tree, and its bytes
//
//   Packet          the tree's and the hex view's state, the step shown and its frame, the
//                   decoded layers (made again only when the selection changes)
//   frame(n, ui)    the panel: left the device's step (inbound and outbound PDUs as links,
//                   the decisions between them), in the middle the tree (L1 ports, Ethernet II,
//                   802.1Q, IPv4, TCP / UDP / ICMP, the application), right the hex view
//
// What it shows follows the selection: an event (a line of the list) shows its device's step
// (Engine::step_of) with the event's frame; a frame clicked on the canvas shows its receiver's
// step when it arrived (Engine::hop_steps), else its sender's.  Tree and hex view are coupled:
// a field or layer chosen in the tree selects its bytes (the view scrolls to them); a byte
// clicked (or reached with the keys) chooses the smallest field holding it.  The fields are
// marked in their layer's category colour, every other one lighter; problems of a layer (a bad
// checksum, a cut header) are lines of their own.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_sim::{DevStep, HopId};
use netpkt::decode::{Layer, Proto};
use ui::*;

use super::{cat_color, cat_of, secs};
use crate::app::constants::*;
use crate::app::Net;

/// the height of a tree row (DIP)
const TREE_ROW: i32 = 22;

/// the key of a tree row: layer l (0: the ports), field f (0: the layer itself)
fn key(l: usize, f: usize) -> u64 {
    ((l as u64 + 1) << 16) | f as u64
}

fn key_layer(k: u64) -> usize {
    ((k >> 16) as usize).saturating_sub(1)
}

/// a row of the tree: label, byte range, depth, whether it is a problem
struct TRow {
    key: u64,
    depth: u16,
    label: String,
    range: Option<(u32, u32)>,
    problem: bool,
    parent: bool,
}

/// the decoded frame shown
struct Shown {
    bytes: Vec<u8>,
    rows: Vec<TRow>,
    marks: Vec<HexMark>,
    /// the fields with bytes: (offset, length, key), smallest first for the lookup
    fields: Vec<(u32, u32, u64)>,
    title: String,
}

pub(crate) struct Packet {
    pub tree: TreeState,
    pub hex: HexState,
    /// what the selection was when it was made: (frame, event)
    made_for: Option<(Option<HopId>, Option<u64>)>,
    /// the device's step and the frames of it (inbound first)
    step: Option<DevStep>,
    pdus: Vec<HopId>,
    /// the frame chosen among them
    cur: Option<HopId>,
    shown: Option<Shown>,
    /// the panes' widths (DIP)
    step_w: i32,
    tree_w: i32,
    dark: bool,
    /// a tree row to scroll to (and passes left to try), the tree's height (DIP)
    reveal: Option<(u64, u8)>,
    tree_h: i32,
}

impl Packet {
    pub(crate) fn new() -> Packet {
        Packet {
            tree: TreeState::default(),
            hex: HexState::default(),
            made_for: None,
            step: None,
            pdus: Vec::new(),
            cur: None,
            shown: None,
            step_w: 280,
            tree_w: 380,
            dark: false,
            reveal: None,
            tree_h: 0,
        }
    }

    /// another engine: nothing shown
    pub(crate) fn reset(&mut self) {
        let (a, b) = (self.step_w, self.tree_w);
        *self = Packet::new();
        self.step_w = a;
        self.tree_w = b;
    }
}

// the selection changed: the device's step and the frame to show
fn follow(n: &mut Net) {
    let want = (n.sim.sel_hop, n.sim.sel_ev);
    if n.sim.pk.made_for == Some(want) {
        return;
    }
    n.sim.pk.made_for = Some(want);
    let eng = n.doc.sim.engine();
    let (step, cur) = match want {
        (Some(hop), _) => {
            let (sent, got) = eng.hop_steps(hop);
            (got.or(sent), Some(hop))
        }
        (None, Some(seq)) => match eng.trace.event(seq) {
            Some(e) => (Some(eng.step_of(e)), e.hop),
            None => (None, None),
        },
        (None, None) => (None, None),
    };
    let mut pdus: Vec<HopId> = Vec::new();
    if let Some(s) = &step {
        pdus.extend(s.inbound);
        pdus.extend(s.outbound.iter().copied());
    }
    let cur = cur.filter(|h| pdus.contains(h) || eng.trace.hop(*h).is_some()).or_else(|| pdus.first().copied());
    let pk = &mut n.sim.pk;
    pk.step = step;
    pk.pdus = pdus;
    pk.cur = None;
    choose(n, cur);
}

// show frame `hop` (None: nothing)
fn choose(n: &mut Net, hop: Option<HopId>) {
    if n.sim.pk.cur == hop && n.sim.pk.shown.is_some() == hop.is_some() {
        return;
    }
    n.sim.pk.cur = hop;
    let dark = n.sim.pk.dark;
    let shown = hop.and_then(|h| decode(n, h, dark));
    let pk = &mut n.sim.pk;
    pk.tree = TreeState::default();
    if let Some(s) = &shown {
        // every layer open, the first field chosen
        for r in &s.rows {
            if r.parent {
                pk.tree.expanded.insert(r.key);
            }
        }
    }
    pk.hex = HexState::default();
    pk.shown = shown;
    // another frame starts at its top
    pk.reveal = Some((key(0, 0), 3));
}

fn dev_name(n: &Net, d: u32) -> String {
    n.doc.proj.device(d).map_or_else(|| format!("Gerät {}", d), |x| x.name.clone())
}

// the frame's layers as tree rows and hex marks
fn decode(n: &Net, hop: HopId, dark: bool) -> Option<Shown> {
    let eng = n.doc.sim.engine();
    let h = eng.trace.hop(hop)?;
    let layers: Vec<Layer> = jfn_sim::view::hop_layers(h);
    let (_, info) = jfn_sim::view::hop_info(h);
    let mut rows = Vec::new();
    let mut marks = Vec::new();
    let mut fields = Vec::new();
    // layer 0: the ports and the times
    let ends = format!("{} → {}", eng.end_name(h.from), eng.end_name(h.to));
    rows.push(TRow { key: key(0, 0), depth: 0, label: format!("L1 Ports: {}", ends), range: None, problem: false, parent: true });
    let arrival = match h.drop {
        Some(jfn_sim::HopDrop::Loss) => String::from("auf der Leitung verloren"),
        Some(jfn_sim::HopDrop::LinkDown) => String::from("verloren: die Leitung fiel aus"),
        None if h.t_rx > eng.now() => format!("{} s (noch unterwegs)", secs(h.t_rx)),
        None => format!("{} s", secs(h.t_rx)),
    };
    let link = n.doc.sim.view.links.iter().find(|l| l.id == h.link);
    let mut l1: Vec<String> = alloc::vec![
        format!("Gesendet: {} s", secs(h.t_tx)),
        format!("Angekommen: {}", arrival),
        format!("Länge: {} Bytes ({} Bit)", h.bytes.len(), h.bytes.len() * 8),
    ];
    if h.t_queued < h.t_tx {
        l1.insert(0, format!("Wartete in der Warteschlange: {} s", secs(h.t_tx - h.t_queued)));
    }
    if let Some(l) = link {
        l1.push(format!("Kabel: {}, {}", l.cable.title(), jfn_model::catalog::speed_str(l.kbps)));
    }
    for (j, s) in l1.into_iter().enumerate() {
        rows.push(TRow { key: key(0, j + 1), depth: 1, label: s, range: None, problem: false, parent: false });
    }
    for (i, l) in layers.iter().enumerate() {
        let li = i + 1;
        let c = cat_color(cat_of(l.proto.bit()), dark);
        let c = if matches!(l.proto, Proto::Eth | Proto::Vlan | Proto::Llc) { color_mix(cat_color(super::CAT_OTHER, dark), 0xFF00_0000, 20) } else { c };
        rows.push(TRow { key: key(li, 0), depth: 0, label: l.title.clone(), range: Some((l.off, l.len)), problem: false, parent: !l.fields.is_empty() || !l.problems.is_empty() });
        // the fields in the layer's colour, every other one lighter (a layer without fields: the
        // layer)
        let light = color_mix(c, if dark { 0xFF00_0000 } else { 0xFFFF_FFFF }, 110);
        let mut marked = false;
        for (j, f) in l.fields.iter().enumerate() {
            let k = key(li, j + 1);
            rows.push(TRow { key: k, depth: 1, label: format!("{}: {}", f.name, f.value), range: Some((f.off, f.len)), problem: false, parent: false });
            if f.len > 0 {
                fields.push((f.off, f.len, k));
                marks.push(HexMark { start: f.off, len: f.len, color: if marks.len() % 2 == 0 { c } else { light }, key: k });
                marked = true;
            }
        }
        if !marked && l.len > 0 {
            marks.push(HexMark { start: l.off, len: l.len, color: c, key: key(li, 0) });
        }
        for (j, p) in l.problems.iter().enumerate() {
            rows.push(TRow { key: key(li, l.fields.len() + 1 + j), depth: 1, label: format!("Problem: {}", p), range: None, problem: true, parent: false });
        }
    }
    fields.sort_by_key(|f| (f.1, f.0));
    let title = format!("Rahmen {} · {} · {}", hop, ends, info);
    Some(Shown { bytes: h.bytes.to_vec(), rows, marks, fields, title })
}

/// the Paket panel
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    let dark = ui.theme_dark();
    if n.sim.pk.dark != dark {
        // the marks take the theme's colours
        n.sim.pk.dark = dark;
        n.sim.pk.made_for = None;
        n.sim.pk.cur = None;
    }
    follow(n);
    if n.sim.pk.step.is_none() && n.sim.pk.shown.is_none() {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(12), 0);
        ui.empty_state(JN_PACKET_ICON, "Kein Paket ausgewählt", "Ein Ereignis der Liste oder ein Rahmen auf der Leinwand zeigt hier, was das Gerät damit tat, und die Felder des Pakets Schicht für Schicht samt Bytes.");
        ui.box_end();
        return;
    }
    let w = ui.px2dip(ui.box_rect()[2]);
    let mut a = n.sim.pk.step_w.clamp(180, (w / 3).max(180));
    ui.next_size(-1, -1);
    ui.split_begin(ID_PK_SPLIT, a, false);
    step_pane(n, ui);
    ui.split_next();
    let mut b = n.sim.pk.tree_w.clamp(200, (w - a - 260).max(200));
    ui.next_size(-1, -1);
    ui.split_begin(ID_PK_SPLIT2, b, false);
    tree_pane(n, ui);
    ui.split_next();
    hex_pane(n, ui);
    if ui.split_end(&mut b) {
        n.sim.pk.tree_w = b;
    }
    if ui.split_end(&mut a) {
        n.sim.pk.step_w = a;
    }
}

/// the packet icon of the empty panel
const JN_PACKET_ICON: u32 = crate::workspace::icons::JN_PACKET;

// the device's step: inbound frame, decisions, outbound frames (links choose the frame)
fn step_pane(n: &mut Net, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_PK_STEP, SC_V | pad(8));
    ui.box_begin(0, BOX_V, 6);
    let Some(s) = n.sim.pk.step.clone() else {
        ui.text(TS_BODY | TX_SECONDARY, "Kein Schritt eines Geräts");
        ui.box_end();
        ui.scroll_end();
        return;
    };
    let eng = n.doc.sim.engine();
    ui.text(TS_SUBTITLE, &dev_name(n, s.dev));
    ui.text(TS_CAPTION | TX_SECONDARY, &format!("t = {} s", secs(s.t)));
    let cur = n.sim.pk.cur;
    let mut chosen: Option<HopId> = None;
    let mut k = 0u32;
    let mut pdu_line = |ui: &mut Ui, n: &Net, hop: HopId, inbound: bool| {
        let Some(h) = eng.trace.hop(hop) else { return };
        let proto = jfn_sim::view::proto_name(h.mask);
        let (who, port) = if inbound { (dev_name(n, h.from.0), eng.end_name(h.to)) } else { (dev_name(n, h.to.0), eng.end_name(h.from)) };
        let label = if inbound { format!("#{} {} von {} (an {})", hop, proto, who, port) } else { format!("#{} {} an {} (über {})", hop, proto, who, port) };
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        let d = ui.dp(8);
        let q = ui.place(d, d);
        ui.cmd_rrect(q[0], q[1], d, d, ui.dp(2), 0, cat_color(cat_of(h.mask), ui.theme_dark()), 0, 0);
        let ts = if cur == Some(hop) { TS_BODY_STRONG } else { TS_BODY };
        if ui.link(ID_PK_PDU0 + k, &label, ts) & EV_ACT != 0 {
            chosen = Some(hop);
        }
        ui.box_end();
        k += 1;
    };
    ui.text(TS_BODY_STRONG, "Eingang");
    match s.inbound {
        Some(h) => pdu_line(ui, n, h, true),
        None => {
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, "Kein Rahmen: das Gerät handelte von sich aus (ein Befehl, eine PDU, ein Zeitgeber).");
        }
    }
    ui.text(TS_BODY_STRONG, "Entscheidungen");
    if s.decisions.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, "keine");
    }
    for e in &s.decisions {
        let layer = match e.note.layer() {
            1 => "L1",
            2 => "L2",
            3 => "L3",
            4 => "L4",
            _ => "L7",
        };
        let text = eng.describe(e);
        ui.box_begin(0, BOX_H, 6);
        ui.next_size(24, 0);
        ui.text(TS_CAPTION | TX_SECONDARY, layer);
        ui.next_size(-1, 0);
        let mark = if e.note.is_drop() { "× " } else { "" };
        ui.text(TS_BODY | TX_WRAP, &format!("{}{}", mark, text));
        ui.box_end();
    }
    ui.text(TS_BODY_STRONG, "Ausgang");
    if s.outbound.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, "keine Rahmen");
    }
    for &h in &s.outbound {
        pdu_line(ui, n, h, false);
    }
    ui.box_end();
    ui.scroll_end();
    if let Some(h) = chosen {
        choose(n, Some(h));
        ui.repass();
    }
}

// the layer tree
fn tree_pane(n: &mut Net, ui: &mut Ui) {
    let pk = &mut n.sim.pk;
    let Some(s) = pk.shown.as_ref() else {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(8), 0);
        ui.text(TS_BODY | TX_SECONDARY, "Kein Rahmen gewählt");
        ui.box_end();
        return;
    };
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | pad(4), 4);
    ui.next_size(-1, 0);
    ui.text(TS_BODY_STRONG | TX_ELLIPSIS, &s.title);
    // the visible rows: a closed layer hides its fields
    let mut vis: Vec<TreeRow> = Vec::new();
    let mut open = true;
    for r in &s.rows {
        if r.depth == 0 {
            open = pk.tree.expanded.contains(&r.key);
        } else if !open {
            continue;
        }
        vis.push(TreeRow { key: r.key, depth: r.depth, has_children: r.parent, check: None, icon: if r.problem { IC_WARNING } else { 0 }, label: &r.label, detail: None });
    }
    // a field the hex view chose: its row into view (rows of TREE_ROW DIP); the scroll view
    // keeps to the last pass' height, so it may take another pass
    if let Some((k, tries)) = pk.reveal.take() {
        if let Some(i) = vis.iter().position(|r| r.key == k) {
            let (_, y) = ui.scroll_pos(ID_PK_TREE);
            let top = i as i32 * TREE_ROW;
            let vh = pk.tree_h.max(TREE_ROW);
            let want = if top < y { Some(top) } else if top + TREE_ROW > y + vh { Some(top + TREE_ROW - vh) } else { None };
            if let Some(w) = want {
                ui.scroll_to(ID_PK_TREE, -1, w);
                if tries > 0 {
                    pk.reveal = Some((k, tries - 1));
                }
            }
        }
    }
    ui.next_size(-1, -1);
    let ev = ui.tree(ID_PK_TREE, &mut pk.tree, &vis, TREE_ROW);
    pk.tree_h = ui.px2dip(ui.last_rect()[3]);
    ui.box_end();
    if ev.sel_changed {
        let range = pk.tree.cur.and_then(|k| s.rows.iter().find(|r| r.key == k)).and_then(|r| r.range).filter(|r| r.1 > 0);
        pk.hex.sel = range;
        ui.repass();
    }
}

// the bytes with the layers' colours
fn hex_pane(n: &mut Net, ui: &mut Ui) {
    let pk = &mut n.sim.pk;
    let Some(s) = &pk.shown else {
        return;
    };
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | pad(4), 4);
    // the bytes chosen and their field
    let field = pk.tree.cur.and_then(|k| s.rows.iter().find(|r| r.key == k)).map(|r| r.label.as_str());
    let text = match (pk.hex.sel, field) {
        (Some((o, 1)), Some(f)) => format!("Byte {} · {}", o, f),
        (Some((o, l)), Some(f)) => format!("Bytes {}–{} · {}", o, o + l - 1, f),
        (Some((o, 1)), None) => format!("Byte {}", o),
        (Some((o, l)), None) => format!("Bytes {}–{}", o, o + l - 1),
        (None, _) => format!("{} Bytes · ein Klick auf ein Byte wählt sein Feld", s.bytes.len()),
    };
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &text);
    ui.next_size(-1, -1);
    let ev = ui.hex_view(ID_PK_HEX, &mut pk.hex, &s.bytes, &s.marks);
    ui.box_end();
    if ev.clicked_key.is_some() {
        // the smallest field holding the byte
        let off = pk.hex.caret();
        let hit = s.fields.iter().find(|f| f.0 <= off && off < f.0 + f.1).map(|f| f.2).or(ev.clicked_key);
        if let Some(k) = hit {
            pk.tree.expanded.insert(key(key_layer(k), 0));
            pk.tree.cur = Some(k);
            pk.reveal = Some((k, 3));
            if let Some(r) = s.rows.iter().find(|r| r.key == k).and_then(|r| r.range).filter(|r| r.1 > 0) {
                if ev.ev & EV_DOUBLE == 0 {
                    pk.hex.sel = Some(r);
                }
            }
            ui.repass();
        }
    }
}
