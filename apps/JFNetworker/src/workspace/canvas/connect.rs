// connect -- joining two devices: the Verbinden tool (C), quick connections from the port dots,
// the check of the target while a cable is drawn and the port choice at the pointer
//
//   Drag                            the cable being drawn: where from, the target under the
//                                   pointer, whether a free pair of ports fits
//   drag(n, from, over)             DEv::ConnectDrag
//   finish(n, ui, from, to, quick)  DEv::Connect: Shift at the release, or a drag from a port dot
//                                   to a port dot, connects at once (the best pair); else the
//                                   port choice opens at the pointer
//   overlay(n, ui)                  the target's outline while drawing: accent when a pair
//                                   fits, else red with the tooltip "Kein passender Port"; the
//                                   ring of end A chosen with the keyboard
//   key_pick(n, ui, dev)            Enter on a device with the tool: end A, then end B opens
//                                   the port choice below B (Enter on A again gives it up)
//   popover(n, ui)                  the port choice: device A | device B with every port (kind,
//                                   name, speed, state), the best pair chosen, the cable
//                                   ("Automatisch: Kupfer, gerade" or a kind that fits), Enter
//                                   connects
//   best(p, a, pa, b, pb)           the best free pair: jfn_model::suggest_ports, or with a port
//                                   fixed on a side (a port dot) its ranking for the rest
//
// The cable follows the ports (jfn_model::cable_kind) unless the user chose one; a new link is
// selected and is one undo step.  Occupied ports show their far end and cannot be chosen, ports
// that fit no free port of the other device say what they would need; choosing a port the
// other side's choice does not fit takes the best partner for it there.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::catalog::PortSpec;
use jfn_model::{cable, ifname, CableKind, PortRef, Project};
use ui::*;

use super::paint::{key_port, DEV_H, DEV_W};
use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::edits;
use crate::workspace::icons::port_icon;

/// the row height of the port lists (DIP), the rows they show at most, their width
const ROW_H: u32 = 26;
const ROWS: i32 = 9;
const LIST_W: i32 = 300;

/// a cable being drawn
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Drag {
    pub from: u64,
    /// the target under the pointer (a node or port key, 0: none)
    pub over: u64,
    /// a free pair of ports fits
    pub fits: bool,
}

/// a port of the choice
pub(crate) struct Row {
    pub spec: PortSpec,
    /// the far end of its cable ("PC2 eth0")
    pub peer: Option<String>,
    /// why it fits no free port of the other device
    pub why: Option<String>,
}

impl Row {
    fn usable(&self) -> bool {
        self.peer.is_none() && self.why.is_none()
    }
}

/// the port choice of two devices
pub(crate) struct Choice {
    pub dev: [u32; 2],
    pub rows: [Vec<Row>; 2],
    /// the chosen row of each side (-1: none)
    pub sel: [i32; 2],
    /// 0: Automatisch, k: the kinds that fit the pair, k - 1
    pub cable: i32,
    opened: bool,
    /// opened from the keyboard: below this box (px) instead of at the pointer
    anchor: Option<[i32; 4]>,
}

/// the device of a node or port key
fn dev_of(key: u64) -> u32 {
    key_port(key).map_or(key as u32, |(d, _)| d)
}

/// the port of a port key
fn port_of(key: u64) -> Option<u16> {
    key_port(key).map(|(_, i)| i)
}

// how good a pair is: radio to an access point, data links, other radios, console cables last;
// then the speed, then the lowest port numbers (jfn_model's suggest_ports)
fn rank(a: &PortSpec, b: &PortSpec, ap: bool) -> Option<(u8, u32, i32)> {
    let k = cable::cable_kind(a, b).ok()?;
    let class = match k {
        CableKind::Wireless if ap => 3,
        CableKind::Wireless => 1,
        CableKind::Console => 0,
        _ => 2,
    };
    Some((class, cable::link_kbps(a, b), -(a.index as i32 + b.index as i32)))
}

/// the best free pair of ports between devices a and b (port indices); a port given on a side
/// is taken there
pub(crate) fn best(p: &Project, a: u32, pa: Option<u16>, b: u32, pb: Option<u16>) -> Option<(u16, u16)> {
    if a == b {
        return None;
    }
    // nothing fixed: jfn_model's rule for a quick connection
    if pa.is_none() && pb.is_none() {
        let (x, y) = p.suggest_ports(a, b)?;
        let index = |d: u32, name: &str| p.model_of(d)?.port(name).map(|s| s.index);
        return Some((index(a, &x)?, index(b, &y)?));
    }
    let ap = p.model_of(a)?.has(jfn_model::catalog::F_AP) || p.model_of(b)?.has(jfn_model::catalog::F_AP);
    let fa: Vec<PortSpec> = p.free_ports(a).into_iter().filter(|x| pa.is_none_or(|i| x.index == i)).collect();
    let fb: Vec<PortSpec> = p.free_ports(b).into_iter().filter(|x| pb.is_none_or(|i| x.index == i)).collect();
    let mut top: Option<((u8, u32, i32), u16, u16)> = None;
    for x in &fa {
        for y in &fb {
            if let Some(k) = rank(x, y, ap) {
                if top.is_none_or(|t| k > t.0) {
                    top = Some((k, x.index, y.index));
                }
            }
        }
    }
    top.map(|t| (t.1, t.2))
}

/// DEv::ConnectDrag: the target under the pointer and whether it fits
pub(crate) fn drag(n: &mut Net, from: u64, over: u64) {
    let same = n.canvas.conn.is_some_and(|c| c.from == from && c.over == over);
    if same {
        return;
    }
    let fits = over != 0 && best(&n.doc.proj, dev_of(from), port_of(from), dev_of(over), port_of(over)).is_some();
    n.canvas.conn = Some(Drag { from, over, fits });
}

/// DEv::Connect: at once (Shift, or port dot to port dot), else the port choice
pub(crate) fn finish(n: &mut Net, ui: &mut Ui, from: u64, to: u64, quick: bool) {
    n.canvas.conn = None;
    let (a, b) = (dev_of(from), dev_of(to));
    let (pa, pb) = (port_of(from), port_of(to));
    let shift = ui.mods() & MOD_SHIFT != 0;
    // the widget's quick: dragged out of a port dot; a dot to a dot needs no choice either
    let _ = quick;
    if shift || (pa.is_some() && pb.is_some()) {
        match best(&n.doc.proj, a, pa, b, pb) {
            Some((x, y)) => {
                connect(n, ui, a, x, b, y, None);
            }
            None => {
                let name = |d: u32| n.doc.proj.device(d).map_or_else(String::new, |d| d.name.clone());
                n.file.msg = Some((IB_WARNING, String::from(t("Kein passender Port")), tf("{} und {} haben keine freien Ports, die zusammenpassen.", &[&name(a), &name(b)])));
            }
        }
        ui.repass();
        return;
    }
    open(n, ui, a, pa, b, pb);
}

/// the port choice of devices a and b (with a port fixed on a side: the dot it started at)
pub(crate) fn open(n: &mut Net, ui: &mut Ui, a: u32, pa: Option<u16>, b: u32, pb: Option<u16>) {
    let p = &n.doc.proj;
    if p.device(a).is_none() || p.device(b).is_none() || a == b {
        return;
    }
    let rows = [rows(p, a, b), rows(p, b, a)];
    let pair = best(p, a, pa, b, pb).or_else(|| best(p, a, None, b, None));
    let pos = |side: usize, i: Option<u16>| i.and_then(|i| rows[side].iter().position(|r| r.spec.index == i)).map_or(-1, |k| k as i32);
    let sel = [pos(0, pair.map(|x| x.0)), pos(1, pair.map(|x| x.1))];
    n.canvas.choice = Some(Choice { dev: [a, b], rows, sel, cable: 0, opened: false, anchor: None });
    ui.repass();
}

/// Enter on a device with the Verbinden tool: the first one is end A (a ring marks it), the
/// second opens the port choice of the two below it; Enter on A again gives A up
pub(crate) fn key_pick(n: &mut Net, ui: &mut Ui, d: u32) {
    match n.canvas.kb_from {
        None => n.canvas.kb_from = Some(d),
        Some(a) if a == d => n.canvas.kb_from = None,
        Some(a) => {
            n.canvas.kb_from = None;
            open(n, ui, a, None, d, None);
            let anchor = n.canvas.scene.rect(d as u64).map(|r| {
                let vp = &n.canvas.st.vp;
                let q = vp.rect_to_screen(r);
                let c = vp.rect;
                let x0 = (q[0] as i32).clamp(c[0], c[0] + c[2] - 1);
                let x1 = ((q[0] + q[2]) as i32).clamp(x0 + 1, c[0] + c[2]);
                let y0 = (q[1] as i32).clamp(c[1], c[1] + c[3] - 1);
                let y1 = ((q[1] + q[3]) as i32).clamp(y0 + 1, c[1] + c[3]);
                [x0, y0, x1 - x0, y1 - y0]
            });
            if let Some(c) = &mut n.canvas.choice {
                c.anchor = anchor;
            }
        }
    }
    ui.repass();
}

// the rows of device `d`'s ports, checked against the free ports of device `other`
fn rows(p: &Project, d: u32, other: u32) -> Vec<Row> {
    let free: Vec<PortSpec> = p.free_ports(other);
    let other_name = p.device(other).map_or_else(String::new, |x| x.name.clone());
    let mut medias: Vec<&'static str> = Vec::new();
    for f in &free {
        if !medias.contains(&f.media.name()) {
            medias.push(f.media.name());
        }
    }
    p.ports(d)
        .into_iter()
        .map(|(spec, link)| {
            let peer = link.and_then(|l| p.link(l)).map(|l| {
                let (od, op) = if l.a.dev == d && ifname::same(&l.a.port, &spec.name) { (l.b.dev, &l.b.port) } else { (l.a.dev, &l.a.port) };
                format!("{} {}", p.device(od).map_or("?", |x| x.name.as_str()), ifname::short(op))
            });
            let why = if peer.is_some() || free.iter().any(|f| cable::cable_kind(&spec, f).is_ok()) {
                None
            } else if free.is_empty() {
                Some(tf("{} hat keinen freien Port", &[&other_name]))
            } else {
                Some(tf("passt nicht zu {}", &[&medias.iter().map(|&m| t(m)).collect::<Vec<_>>().join(", ")]))
            };
            Row { spec, peer, why }
        })
        .collect()
}

impl Choice {
    /// the chosen ports, when both are chosen and usable
    fn pair(&self) -> Option<(&PortSpec, &PortSpec)> {
        let a = self.rows[0].get(self.sel[0] as usize)?;
        let b = self.rows[1].get(self.sel[1] as usize)?;
        if !a.usable() || !b.usable() {
            return None;
        }
        Some((&a.spec, &b.spec))
    }

    /// the cable kinds that fit the chosen pair (the right one first)
    fn kinds(&self) -> Vec<CableKind> {
        let Some((a, b)) = self.pair() else { return Vec::new() };
        let Ok(right) = cable::cable_kind(a, b) else { return Vec::new() };
        let mut v = alloc::vec![right];
        for &k in CableKind::ALL {
            if k != right && cable::cable_fits(k, a, b).is_ok() {
                v.push(k);
            }
        }
        v
    }

    /// side `s` chose row `i`: when the other side's port does not fit it, the best partner
    fn choose(&mut self, p: &Project, s: usize, i: i32) {
        self.sel[s] = i;
        let o = 1 - s;
        let fits = match (self.rows[s].get(i as usize), self.rows[o].get(self.sel[o] as usize)) {
            (Some(x), Some(y)) => y.usable() && cable::cable_kind(&x.spec, &y.spec).is_ok(),
            _ => false,
        };
        if fits {
            return;
        }
        let Some(x) = self.rows[s].get(i as usize).map(|r| r.spec.index) else { return };
        let pair = if s == 0 { best(p, self.dev[0], Some(x), self.dev[1], None) } else { best(p, self.dev[0], None, self.dev[1], Some(x)).map(|(a, b)| (b, a)) };
        if let Some((_, y)) = pair {
            self.sel[o] = self.rows[o].iter().position(|r| r.spec.index == y).map_or(-1, |k| k as i32);
        }
    }
}

/// connect port x of device a with port y of device b (cable None: the right one) -> the link
pub(crate) fn connect(n: &mut Net, ui: &mut Ui, a: u32, x: u16, b: u32, y: u16, cable: Option<CableKind>) -> Option<u32> {
    let p = &n.doc.proj;
    let name = |d: u32, i: u16| p.model_of(d).and_then(|m| m.ports().into_iter().find(|s| s.index == i)).map(|s| s.name);
    let (Some(na), Some(nb)) = (name(a, x), name(b, y)) else { return None };
    let (ra, rb) = (PortRef { dev: a, port: na }, PortRef { dev: b, port: nb });
    match edits::connect(&mut n.doc.proj, &ra, &rb, cable) {
        Ok((id, e)) => {
            n.doc.record(e);
            n.canvas.select_only(id);
            ui.repass();
            Some(id)
        }
        Err(e) => {
            n.file.msg = Some((IB_ERROR, String::from(t("Die Geräte wurden nicht verbunden")), crate::app::tr::msg(&e.msg())));
            ui.repass();
            None
        }
    }
}

/// the target's outline while a cable is drawn (after ui.diagram, inside its clip)
pub(crate) fn overlay(n: &Net, ui: &mut Ui) {
    // end A chosen with Enter: an accent ring until end B
    if let Some(rect) = n.canvas.kb_from.filter(|_| n.canvas.tool == super::tools::Tool::Connect).and_then(|d| n.canvas.scene.rect(d as u64)) {
        let vp = n.canvas.st.vp;
        let q = vp.rect_to_screen(rect);
        let (pad, t, rad) = (ui.dp(5), ui.dp(2), ui.dp(5));
        let col = ui.color(T_ACCENT);
        let r = vp.rect;
        ui.clip_push(r[0], r[1], r[2], r[3]);
        ui.cmd_rrect(q[0] as i32 - pad, q[1] as i32 - pad, q[2] as i32 + pad * 2, q[3] as i32 + pad * 2, rad, t, 0, col, col);
        ui.clip_pop();
    }
    let Some(c) = n.canvas.conn else { return };
    if c.over == 0 {
        return;
    }
    let Some(rect) = n.canvas.scene.rect(dev_of(c.over) as u64) else { return };
    let vp = n.canvas.st.vp;
    let q = vp.rect_to_screen([rect[0], rect[1], DEV_W.max(rect[2]), DEV_H.max(rect[3])]);
    let (pad, t, rad) = (ui.dp(5), ui.dp(2), ui.dp(5));
    let col = if c.fits { ui.color(T_ACCENT) } else { ui.color(T_CRIT) };
    let (x, y) = (q[0] as i32 - pad, q[1] as i32 - pad);
    let (w, h) = (q[2] as i32 + pad * 2, q[3] as i32 + pad * 2);
    let r = vp.rect;
    ui.clip_push(r[0], r[1], r[2], r[3]);
    ui.cmd_rrect(x, y, w, h, rad, t, 0, col, col);
    ui.clip_pop();
}

/// the tooltip of the target that does not fit (right after ui.diagram: the canvas is the last
/// widget) -> shown
pub(crate) fn tooltip(c: &super::Canvas, ui: &mut Ui) -> bool {
    match c.conn {
        Some(c) if c.over != 0 && !c.fits => {
            ui.tooltip_part(t("Kein passender Port"), c.over);
            true
        }
        _ => false,
    }
}

// one row of a port list
fn row(ui: &mut Ui, r: &Row, i: i32) {
    ui.list_item_begin(i);
    let dim = if r.usable() { 0 } else { TX_DISABLED };
    ui.icon(port_icon(r.spec.media), 16);
    ui.next_size(96, 0);
    ui.text(TS_BODY | TX_ELLIPSIS | dim, &ifname::short(&r.spec.name));
    let speed = match r.spec.media {
        jfn_model::Media::Console | jfn_model::Media::Usb => String::from(t(r.spec.media.name())),
        _ => crate::app::tr::speed(r.spec.kbps),
    };
    ui.next_size(56, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS | dim, &speed);
    let state = match (&r.peer, &r.why) {
        (Some(p), _) => format!("→ {}", p),
        (None, Some(w)) => w.clone(),
        (None, None) => String::from(t("frei")),
    };
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_ELLIPSIS | if r.usable() { TX_SECONDARY } else { TX_DISABLED }, &state);
    ui.list_item_end();
}

// a side's list: the device's name and model, its ports -> the row chosen in this pass
fn side(n: &Net, ui: &mut Ui, s: usize) -> Option<i32> {
    let c = n.canvas.choice.as_ref()?;
    let p = &n.doc.proj;
    let d = p.device(c.dev[s])?;
    let model = p.model_of(d.id).map_or("", |m| m.name);
    ui.box_begin(0, BOX_V, 4);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    ui.text(TS_BODY_STRONG, &d.name);
    ui.text(TS_CAPTION | TX_SECONDARY, model);
    ui.box_end();
    let rows = &c.rows[s];
    let id = if s == 0 { ID_CONN_A } else { ID_CONN_B };
    let mut sel = c.sel[s];
    let before = sel;
    ui.next_size(LIST_W, (rows.len() as i32).clamp(1, ROWS) * ROW_H as i32);
    let (first, end) = ui.list_begin(id, &mut sel, rows.len() as i32, ROW_H);
    for i in first..end {
        row(ui, &rows[i as usize], i);
    }
    ui.list_end(&mut sel);
    ui.box_end();
    if sel == before {
        return None;
    }
    // occupied ports and ports without a partner are not for choosing: the next usable one in
    // that direction (keys), else the choice stays
    let step = if sel > before { 1 } else { -1 };
    let mut k = sel;
    while k >= 0 && (k as usize) < rows.len() {
        if rows[k as usize].usable() {
            return Some(k);
        }
        k += step;
    }
    Some(before)
}

/// the port choice (a flyout at the pointer where the cable was dropped)
pub(crate) fn popover(n: &mut Net, ui: &mut Ui) {
    if n.canvas.choice.is_none() && ui.popup_is_open(ID_CONNECT) {
        ui.popup_close();
    }
    // a device that went (undo, another project) ends the choice
    let gone = n.canvas.choice.as_ref().is_some_and(|c| c.dev.iter().any(|&d| n.doc.proj.device(d).is_none()));
    let mut close = gone;
    if let Some(c) = &mut n.canvas.choice {
        if !c.opened {
            c.opened = true;
            ui.popup_open(ID_CONNECT);
            // the keyboard to the target's ports (without a chosen one the list keeps none)
            ui.focus(ID_CONN_B);
        } else if !ui.popup_is_open(ID_CONNECT) {
            close = true; // Esc, a click elsewhere
        }
    }
    if close {
        if ui.popup_is_open(ID_CONNECT) {
            ui.popup_close();
        }
        n.canvas.choice = None;
        return;
    }
    let mut act = 0; // 1 connect, 2 cancel
    // opened from the keyboard: below device B, else at the pointer
    let anchor = n.canvas.choice.as_ref().and_then(|c| c.anchor);
    if let Some(a) = anchor {
        ui.last_set(a, 0);
    }
    if ui.flyout_begin(ID_CONNECT, if anchor.is_some() { FL_BELOW } else { FL_POINT }) {
        if n.canvas.choice.is_some() {
            // Enter connects (taken before the lists; not while the cable's list is open)
            let enter = ui.popup_count() == 1 && ui.key(VK_RETURN);
            // a fixed width: the flyout knows its size in the first pass
            ui.next_size(2 * LIST_W + 12 + 16, 0);
            ui.box_begin(0, BOX_V | pad(8), 8);
            ui.text(TS_BODY_STRONG, t("Ports wählen"));
            ui.box_begin(0, BOX_H, 12);
            for s in 0..2 {
                if let Some(i) = side(n, ui, s) {
                    let p = &n.doc.proj;
                    if let Some(c) = &mut n.canvas.choice {
                        c.choose(p, s, i);
                    }
                    ui.repass();
                }
            }
            ui.box_end();
            // the cable: Automatisch (the right one for the pair) or a kind that fits
            let Some(c) = n.canvas.choice.as_mut() else {
                ui.box_end();
                ui.flyout_end();
                return;
            };
            let kinds = c.kinds();
            if c.cable as usize > kinds.len() {
                c.cable = 0;
            }
            let mut items: Vec<String> = Vec::new();
            match kinds.first() {
                Some(k) => items.push(tf("Automatisch: {}", &[t(k.title())])),
                None => items.push(String::from(t("Automatisch"))),
            }
            items.extend(kinds.iter().map(|k| String::from(t(k.title()))));
            let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.text(TS_BODY | TX_SECONDARY, t("Kabel"));
            ui.next_size(260, 0);
            ui.next_flags(if kinds.is_empty() { ST_DISABLED } else { 0 });
            ui.combobox(ID_CONN_CABLE, &mut c.cable, &refs);
            ui.spacer(-1);
            let ok = c.pair().is_some();
            if ui.button(ID_CONN_CANCEL, t("Abbrechen")) & EV_ACT != 0 {
                act = 2;
            }
            ui.next_flags(if ok { 0 } else { ST_DISABLED });
            if ui.accent_button(ID_CONN_OK, t("Verbinden")) & EV_ACT != 0 {
                act = 1;
            }
            ui.box_end();
            if !ok {
                ui.text(TS_CAPTION | TX_SECONDARY, t("Diese beiden Geräte haben keine freien Ports, die zusammenpassen."));
            }
            ui.box_end();
            if act == 0 && ok && enter {
                act = 1;
            }
        }
    }
    ui.flyout_end();
    match act {
        1 => {
            let pick = n.canvas.choice.as_ref().and_then(|c| {
                let (a, b) = c.pair()?;
                let kinds = c.kinds();
                let cable = if c.cable > 0 { kinds.get(c.cable as usize - 1).copied() } else { None };
                Some((c.dev, a.index, b.index, cable))
            });
            ui.popup_close();
            n.canvas.choice = None;
            if let Some((d, x, y, cable)) = pick {
                connect(n, ui, d[0], x, d[1], y, cable);
            }
        }
        2 => {
            ui.popup_close();
            n.canvas.choice = None;
            ui.repass();
        }
        _ => {}
    }
}
