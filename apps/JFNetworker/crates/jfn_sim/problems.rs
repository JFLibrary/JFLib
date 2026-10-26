// problems -- what is wrong with the set-up (the problems tab): the engine looks at the devices,
// their interfaces and the cables between them
//
//   Engine::problems() -> Vec<Problem>    errors first, then by device and port
//
//   DuplicateIp         two interfaces of one network segment have the same address
//   NoGateway           a host without a default route where a router shares its network
//   GatewayOffNet       the default gateway lies outside every network of the host
//   GatewayUnknown      the gateway lies in the host's network, but no device there has it
//   MaskMismatch        overlapping networks with different masks in one segment
//   NativeVlanMismatch  trunk ends with different native VLANs (or access ports in different
//                       VLANs): untagged frames change their VLAN on the cable
//   NoCommonVlan        trunk ends that allow no VLAN in common
//   AccessTrunk         an access port against a trunk (or a router port with subinterfaces)
//   WrongCable          the wrong copper cable while the project wants strict MDI / MDI-X
//   Loop                switches and hubs in a circle, unless every switch in it runs spanning
//                       tree (stp.rs); the text names those that do not when some do
//   ShutWithAddress     an interface with an address that is shut down
//   DuplicateIpv6       duplicate address detection found another node with an IPv6 address
//   WlanNoNetwork       a WLAN station whose SSID no access point offers (error), or none of
//                       them in range on the station's bands (warning)
//   WlanPassphrase      a station's passphrase is not the one of its network, or it has none
//   WlanBadNet          a network an access point cannot offer (jfn_model::wlan::check_net)
//   CableTooLong        a cable longer than its medium allows at its speed (phys.rs, P1)
//   OpticMismatch       the optics of a fibre's ends do not fit each other or the fibre
//
// Problem::text is German; Problem::msg is the same as a template with arguments for the app's
// catalogue (jfn_model::msg).
//
// Segments are the broadcast domains of the topology as cabled: cables, the VLANs inside
// bridges (access ports, trunks with their native VLAN and allowed list), 802.1Q tags, hubs.
// They count cables whether they are up or not -- a fault in the set-up stays one while a
// cable is pulled.  Loops count only cables that are up: shutting a port down is how a loop is
// broken by hand.  Only VLANs something uses matter (access, native, SVI, subinterface).

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::cable;
use jfn_model::catalog::{CliStyle, PortSpec, F_HOST, F_HUB};
use jfn_model::ifname;
use jfn_model::msg;
use jfn_model::msg::{phrase, Msg};
use netpkt::{Ipv4, Prefix4};

use crate::bridge::{BridgePort, PortMode};
use crate::ip::{IfKind, Iface};
use crate::kernel::Engine;

/// how bad a problem is
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// traffic cannot flow as meant
    Error,
    /// probably not what was meant
    Warning,
}

impl Severity {
    /// the German name (the app translates it)
    pub fn title(self) -> &'static str {
        // i18n
        match self {
            Severity::Error => "Fehler",
            Severity::Warning => "Warnung",
        }
    }
}

/// what kind of problem (the order is the order of the checks)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProblemKind {
    DuplicateIp,
    NoGateway,
    GatewayOffNet,
    GatewayUnknown,
    MaskMismatch,
    NativeVlanMismatch,
    NoCommonVlan,
    AccessTrunk,
    WrongCable,
    Loop,
    ShutWithAddress,
    DuplicateIpv6,
    /// a WLAN station whose network no access point offers, or none in range
    WlanNoNetwork,
    /// a WLAN station whose passphrase does not fit its network (or is missing)
    WlanPassphrase,
    /// a network an access point cannot offer (SSID, channel, 6 GHz without WPA3, passphrase)
    WlanBadNet,
    /// a cable longer than its medium allows (phys.rs)
    CableTooLong,
    /// a fibre whose optics do not fit each other or the fibre (phys.rs)
    OpticMismatch,
}

/// a problem of the set-up
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub kind: ProblemKind,
    pub severity: Severity,
    /// the device to show (the problems tab selects it)
    pub dev: u32,
    /// its port (index), when the problem sits at one
    pub port: Option<u16>,
    /// the cable it is about
    pub link: Option<u32>,
    /// the other devices involved: the other holder of an address, the far end, the switches
    /// of a loop
    pub others: Vec<u32>,
    /// what is wrong, in German
    pub text: String,
    /// what is wrong, as a message for the app's catalogue (text is its German text)
    pub msg: Msg,
}

/// the loops reported at most
const MAX_LOOPS: usize = 16;

/// a place a frame can be: on a cable (engine slot) untagged (tag 0) or with a tag, or in a
/// VLAN inside a bridge (node slot)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Spot {
    Wire(usize, u16),
    Vlan(usize, u16),
}

/// two spots a device joins (node slot, its port, the cable)
#[derive(Clone, Copy, Debug)]
struct Edge {
    a: Spot,
    b: Spot,
    node: usize,
    port: u16,
    link: usize,
}

/// union-find over spots
#[derive(Default)]
struct Uf {
    ids: BTreeMap<Spot, usize>,
    up: Vec<usize>,
}

impl Uf {
    fn id(&mut self, s: Spot) -> usize {
        if let Some(&i) = self.ids.get(&s) {
            return i;
        }
        let i = self.up.len();
        self.up.push(i);
        self.ids.insert(s, i);
        i
    }
    fn root(&mut self, mut i: usize) -> usize {
        while self.up[i] != i {
            self.up[i] = self.up[self.up[i]];
            i = self.up[i];
        }
        i
    }
    /// false when they were joined already
    fn join(&mut self, a: usize, b: usize) -> bool {
        let (ra, rb) = (self.root(a), self.root(b));
        if ra == rb {
            return false;
        }
        self.up[ra.max(rb)] = ra.min(rb);
        true
    }
}

/// an address of an interface and the segment it is in
struct Addr {
    dev: u32,
    ni: usize,
    /// the short interface name ("Gi0/0/0", "eth0")
    name: String,
    port: Option<u16>,
    prefix: Prefix4,
    seg: Option<usize>,
}

/// the edges of the tree path from spot `a` to spot `b`
fn tree_path(adj: &BTreeMap<usize, Vec<(usize, usize)>>, a: usize, b: usize) -> Vec<usize> {
    let mut prev: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    let mut seen: BTreeSet<usize> = BTreeSet::new();
    seen.insert(a);
    let mut q: VecDeque<usize> = VecDeque::new();
    q.push_back(a);
    while let Some(x) = q.pop_front() {
        if x == b {
            break;
        }
        for &(y, e) in adj.get(&x).map(|v| v.as_slice()).unwrap_or(&[]) {
            if seen.insert(y) {
                prev.insert(y, (x, e));
                q.push_back(y);
            }
        }
    }
    let mut path = Vec::new();
    let mut x = b;
    while x != a {
        let Some(&(p, e)) = prev.get(&x) else {
            break;
        };
        path.push(e);
        x = p;
    }
    path
}

/// a port's interface by its short name ("Gi0/0/0", "eth0"), others by their full one ("Vlan10")
fn label(f: &Iface) -> String {
    match f.kind {
        IfKind::Port(_) | IfKind::Sub { .. } => ifname::short(&f.name),
        _ => f.name.clone(),
    }
}

fn problem(kind: ProblemKind, severity: Severity, dev: u32, port: Option<u16>, link: Option<u32>, others: Vec<u32>, msg: Msg) -> Problem {
    Problem { kind, severity, dev, port, link, others, text: msg.text(), msg }
}

impl Engine {
    /// what is wrong with the set-up, errors first
    pub fn problems(&self) -> Vec<Problem> {
        let mut out = Vec::new();
        let tags = self.used_vlans();
        let mut uf = Uf::default();
        for e in self.l2_edges(false, &tags) {
            let (a, b) = (uf.id(e.a), uf.id(e.b));
            uf.join(a, b);
        }
        let addrs = self.addrs(&mut uf);
        self.dup_ips(&addrs, &mut out);
        self.masks(&addrs, &mut out);
        self.gateways(&addrs, &mut out);
        self.shut_addrs(&mut out);
        self.cables(&mut out);
        self.loops(&tags, &mut out);
        self.dup_ip6s(&mut out);
        self.wlan_problems(&mut out);
        self.phys_problems(&mut out);
        out.sort_by(|a, b| (a.severity, a.dev, a.port, a.kind, &a.text).cmp(&(b.severity, b.dev, b.port, b.kind, &b.text)));
        out
    }

    fn dev_name(&self, dev: u32) -> String {
        match self.node(dev) {
            Some(n) => n.name.clone(),
            None => format!("Gerät {}", dev),
        }
    }

    fn spec(&self, (n, p): (usize, u16)) -> Option<PortSpec> {
        self.nodes.get(n)?.as_ref()?.ports.get(p as usize).map(|x| x.spec.clone())
    }

    /// the VLANs something uses: access ports, native VLANs, bridge interfaces, subinterfaces
    fn used_vlans(&self) -> BTreeSet<u16> {
        let mut s = BTreeSet::new();
        for n in self.nodes.iter().flatten() {
            if let Some(b) = n.bridge.as_ref() {
                for p in b.ports.iter().filter(|p| p.member) {
                    s.insert(match &p.mode {
                        PortMode::Access(v) => *v,
                        PortMode::Trunk { native, .. } => *native,
                    });
                }
            }
            if let Some(st) = n.ip.as_ref() {
                for f in st.ifaces.iter().filter(|f| !f.hidden) {
                    if let IfKind::Svi(v) | IfKind::Sub { vlan: v, .. } = f.kind {
                        s.insert(v);
                    }
                }
            }
        }
        s.remove(&0);
        s
    }

    /// the edges bridges and hubs make between spots (`only_up`: on cables that are up)
    fn l2_edges(&self, only_up: bool, tags: &BTreeSet<u16>) -> Vec<Edge> {
        let live = |li: usize| !only_up || self.link_up(li);
        let mut v = Vec::new();
        for &li in self.link_idx.values() {
            let Some(Some(l)) = self.links.get(li) else {
                continue;
            };
            if !live(li) {
                continue;
            }
            for &(n, p) in &l.ends {
                let Some(Some(node)) = self.nodes.get(n) else {
                    continue;
                };
                // the access point's end of a WLAN association: the station's VLAN (wlan.rs)
                if let Some(vid) = self.wlan_end_vlan(li, n) {
                    v.push(Edge { a: Spot::Wire(li, 0), b: Spot::Vlan(n, vid), node: n, port: p, link: li });
                    continue;
                }
                let Some(BridgePort { mode, member: true, .. }) = node.bridge.as_ref().and_then(|b| b.ports.get(p as usize)) else {
                    continue;
                };
                match mode {
                    PortMode::Access(vid) => v.push(Edge { a: Spot::Wire(li, 0), b: Spot::Vlan(n, *vid), node: n, port: p, link: li }),
                    PortMode::Trunk { native, allowed } => {
                        // the native VLAN leaves untagged; a bridge never sends it tagged
                        v.push(Edge { a: Spot::Wire(li, 0), b: Spot::Vlan(n, *native), node: n, port: p, link: li });
                        for &t in tags.iter().filter(|&&t| t != *native && allowed.contains(t)) {
                            v.push(Edge { a: Spot::Wire(li, t), b: Spot::Vlan(n, t), node: n, port: p, link: li });
                        }
                    }
                }
            }
        }
        // a hub repeats everything between its cables
        for &ni in self.dev_idx.values() {
            let Some(Some(node)) = self.nodes.get(ni) else {
                continue;
            };
            if !node.model.has(F_HUB) {
                continue;
            }
            let wires: Vec<(u16, usize)> = node.ports.iter().enumerate().filter_map(|(p, x)| Some((p as u16, x.link?))).filter(|w| live(w.1)).collect();
            for w in wires.windows(2) {
                for t in core::iter::once(0).chain(tags.iter().copied()) {
                    v.push(Edge { a: Spot::Wire(w[0].1, t), b: Spot::Wire(w[1].1, t), node: ni, port: w[1].0, link: w[1].1 });
                }
            }
        }
        v
    }

    /// every address of every interface, with its segment
    fn addrs(&self, uf: &mut Uf) -> Vec<Addr> {
        let mut v = Vec::new();
        for (&dev, &ni) in &self.dev_idx {
            let Some(Some(n)) = self.nodes.get(ni) else {
                continue;
            };
            let Some(st) = n.ip.as_ref() else {
                continue;
            };
            let wire = |p: u16, tag: u16| n.ports.get(p as usize).and_then(|x| x.link).map(|li| Spot::Wire(li, tag));
            for f in st.ifaces.iter().filter(|f| !f.hidden) {
                let (spot, port) = match f.kind {
                    IfKind::Port(p) => (wire(p, 0), Some(p)),
                    IfKind::Sub { port, vlan } => (wire(port, vlan), Some(port)),
                    IfKind::Svi(vl) => (n.bridge.as_ref().map(|_| Spot::Vlan(ni, vl)), None),
                    IfKind::Loopback(_) => continue,
                };
                let seg = spot.map(|s| {
                    let i = uf.id(s);
                    uf.root(i)
                });
                for p in &f.addrs {
                    v.push(Addr { dev, ni, name: label(f), port, prefix: *p, seg });
                }
            }
        }
        v
    }

    fn dup_ips(&self, addrs: &[Addr], out: &mut Vec<Problem>) {
        let mut by: BTreeMap<(usize, Ipv4), Vec<usize>> = BTreeMap::new();
        for (k, a) in addrs.iter().enumerate() {
            if let Some(s) = a.seg {
                by.entry((s, a.prefix.addr)).or_default().push(k);
            }
        }
        for list in by.values().filter(|l| l.len() > 1) {
            for &k in list {
                let a = &addrs[k];
                let rest: Vec<&Addr> = list.iter().filter(|&&j| j != k).map(|&j| &addrs[j]).collect();
                let names: Vec<String> = rest.iter().map(|o| format!("{} ({})", self.dev_name(o.dev), o.name)).collect();
                let mut others: Vec<u32> = rest.iter().map(|o| o.dev).filter(|&d| d != a.dev).collect();
                others.sort_unstable();
                others.dedup();
                let text = msg!("{} an {} ist im selben Netz doppelt vergeben: auch {}.", a.prefix.addr, a.name, names.join(", "));
                out.push(problem(ProblemKind::DuplicateIp, Severity::Error, a.dev, a.port, None, others, text));
            }
        }
    }

    fn masks(&self, addrs: &[Addr], out: &mut Vec<Problem>) {
        for a in addrs {
            let Some(s) = a.seg else {
                continue;
            };
            let peers: Vec<&Addr> = addrs.iter().filter(|b| b.seg == Some(s) && b.dev != a.dev && b.prefix.overlaps(&a.prefix)).collect();
            if peers.iter().all(|b| b.prefix.len == a.prefix.len) {
                continue;
            }
            // the mask most of them use is right, if there is one
            let mut count: BTreeMap<u8, usize> = BTreeMap::new();
            for len in peers.iter().map(|b| b.prefix.len).chain([a.prefix.len]) {
                *count.entry(len).or_default() += 1;
            }
            let most = count.values().copied().max().unwrap_or(0);
            let modes: Vec<u8> = count.iter().filter(|e| *e.1 == most).map(|e| *e.0).collect();
            if modes == [a.prefix.len] {
                continue;
            }
            let odd: Vec<&&Addr> = peers.iter().filter(|b| b.prefix.len != a.prefix.len).collect();
            let mut others: Vec<u32> = odd.iter().map(|b| b.dev).collect();
            others.sort_unstable();
            others.dedup();
            let o = odd[0];
            let text = msg!("Netzmaske passt nicht: {} hat {}, {} ({}) im selben Netz /{}.", a.name, a.prefix, self.dev_name(o.dev), o.name, o.prefix.len);
            out.push(problem(ProblemKind::MaskMismatch, Severity::Warning, a.dev, a.port, None, others, text));
        }
    }

    fn gateways(&self, addrs: &[Addr], out: &mut Vec<Problem>) {
        let forwards = |ni: usize| self.stack(ni).is_some_and(|s| s.forwarding);
        for (&dev, &ni) in &self.dev_idx {
            let Some(Some(n)) = self.nodes.get(ni) else {
                continue;
            };
            let Some(st) = n.ip.as_ref() else {
                continue;
            };
            if !n.model.has(F_HOST) || st.forwarding {
                continue;
            }
            let mine: Vec<&Addr> = addrs.iter().filter(|a| a.dev == dev).collect();
            if mine.is_empty() {
                continue;
            }
            let defaults: Vec<Ipv4> = st.statics.iter().filter(|r| r.prefix.len == 0).filter_map(|r| r.via).collect();
            if defaults.is_empty() && !st.statics.iter().any(|r| r.prefix.len == 0) {
                // a router in one of its networks could be the gateway
                let router = mine.iter().find_map(|a| addrs.iter().find(|r| r.seg.is_some() && r.seg == a.seg && r.dev != dev && forwards(r.ni) && r.prefix.contains(a.prefix.addr)).map(|r| (*a, r)));
                if let Some((a, r)) = router {
                    let text = msg!("Kein Standard-Gateway: {} erreicht nur das eigene Netz ({} mit {} könnte es sein).", n.name, self.dev_name(r.dev), r.prefix.addr);
                    out.push(problem(ProblemKind::NoGateway, Severity::Warning, dev, a.port, None, alloc::vec![r.dev], text));
                }
                continue;
            }
            for gw in defaults {
                match mine.iter().find(|a| a.prefix.contains(gw)) {
                    None => {
                        let nets: Vec<String> = mine.iter().map(|a| format!("{}", a.prefix.trunc())).collect();
                        let text = msg!("Das Standard-Gateway {} liegt außerhalb der eigenen Netze ({}).", gw, nets.join(", "));
                        out.push(problem(ProblemKind::GatewayOffNet, Severity::Error, dev, mine[0].port, None, Vec::new(), text));
                    }
                    Some(a) => {
                        let holder = addrs.iter().find(|x| x.seg.is_some() && x.seg == a.seg && x.prefix.addr == gw);
                        if holder.is_none() {
                            let text = msg!("Das Standard-Gateway {} ist im Netz von {} nicht zu finden: kein Gerät dort hat diese Adresse.", gw, a.name);
                            out.push(problem(ProblemKind::GatewayUnknown, Severity::Warning, dev, a.port, None, Vec::new(), text));
                        }
                    }
                }
            }
        }
    }

    fn shut_addrs(&self, out: &mut Vec<Problem>) {
        for (&dev, &ni) in &self.dev_idx {
            let Some(Some(n)) = self.nodes.get(ni) else {
                continue;
            };
            let Some(st) = n.ip.as_ref() else {
                continue;
            };
            let port_up = |p: u16| n.ports.get(p as usize).is_some_and(|x| x.admin_up);
            for f in st.ifaces.iter().filter(|f| !f.hidden && !f.addrs.is_empty()) {
                let (up, port) = match f.kind {
                    IfKind::Port(p) => (port_up(p), Some(p)),
                    IfKind::Sub { port, .. } => (f.admin_up && port_up(port), Some(port)),
                    IfKind::Svi(_) | IfKind::Loopback(_) => (f.admin_up, None),
                };
                if up {
                    continue;
                }
                let text = if n.model.cli == CliStyle::Ios {
                    msg!("{} hat die Adresse {}, ist aber abgeschaltet (\u{201e}no shutdown\u{201c} fehlt).", label(f), f.addrs[0])
                } else {
                    msg!("{} hat die Adresse {}, ist aber abgeschaltet.", label(f), f.addrs[0])
                };
                out.push(problem(ProblemKind::ShutWithAddress, Severity::Warning, dev, port, None, Vec::new(), text));
            }
        }
    }

    /// the cables: the copper kind under strict MDI-X, VLANs at both ends
    fn cables(&self, out: &mut Vec<Problem>) {
        for (&id, &li) in &self.link_idx {
            let Some(Some(l)) = self.links.get(li) else {
                continue;
            };
            let ends = l.ends;
            let dev = |k: usize| self.dev_of(ends[k].0);
            let name = |k: usize| self.end_name((dev(k), ends[k].1));
            let at = |k: usize| (dev(k), Some(ends[k].1), Some(id), alloc::vec![dev(1 - k)]);
            if self.strict_mdix && l.wrong_cable {
                let right = match (self.spec(ends[0]), self.spec(ends[1])) {
                    (Some(a), Some(b)) => cable::cable_kind(&a, &b).ok(),
                    _ => None,
                };
                let text = match right {
                    Some(k) => msg!("Falsches Kabel zwischen {} und {}: hierher gehört {}; ohne Auto-MDI-X bleibt die Leitung aus.", name(0), name(1), phrase(k.title())),
                    None => msg!("Falsches Kabel zwischen {} und {}: hierher gehört ein anderes Kabel; ohne Auto-MDI-X bleibt die Leitung aus.", name(0), name(1)),
                };
                let (d, p, k, o) = at(0);
                out.push(problem(ProblemKind::WrongCable, Severity::Error, d, p, k, o, text));
            }
            let mode = |k: usize| -> Option<PortMode> {
                let node = self.nodes.get(ends[k].0)?.as_ref()?;
                match node.bridge.as_ref()?.ports.get(ends[k].1 as usize)? {
                    BridgePort { mode, member: true, .. } => Some(mode.clone()),
                    _ => None,
                }
            };
            // a routed port with 802.1Q subinterfaces sends tagged frames
            let tagging = |k: usize| self.stack(ends[k].0).is_some_and(|st| st.ifaces.iter().any(|f| !f.hidden && matches!(f.kind, IfKind::Sub { port, .. } if port == ends[k].1)));
            let mut push = |kind: ProblemKind, sev: Severity, k: usize, text: Msg| {
                let (d, p, l, o) = at(k);
                out.push(problem(kind, sev, d, p, l, o, text));
            };
            match (mode(0), mode(1)) {
                (Some(PortMode::Trunk { native: na, allowed: aa }), Some(PortMode::Trunk { native: nb, allowed: ab })) => {
                    if na != nb {
                        let text = msg!("Native VLAN passt nicht: {} hat VLAN {}, {} VLAN {}; ungetaggte Frames wechseln auf dem Kabel das VLAN.", name(0), na, name(1), nb);
                        push(ProblemKind::NativeVlanMismatch, Severity::Warning, 0, text);
                    }
                    if aa.intersect(&ab).is_empty() {
                        let text = msg!("Der Trunk zwischen {} und {} lässt kein VLAN auf beiden Seiten zu.", name(0), name(1));
                        push(ProblemKind::NoCommonVlan, Severity::Error, 0, text);
                    }
                }
                (Some(PortMode::Access(a)), Some(PortMode::Access(b))) if a != b => {
                    let text = msg!("VLAN passt nicht: {} ist Access-Port in VLAN {}, {} in VLAN {}; die Frames wechseln auf dem Kabel das VLAN.", name(0), a, name(1), b);
                    push(ProblemKind::NativeVlanMismatch, Severity::Warning, 0, text);
                }
                (Some(PortMode::Access(v)), Some(PortMode::Trunk { .. })) => {
                    push(ProblemKind::AccessTrunk, Severity::Warning, 0, msg!("{} ist Access-Port (VLAN {}), {} ist Trunk.", name(0), v, name(1)));
                }
                (Some(PortMode::Trunk { .. }), Some(PortMode::Access(v))) => {
                    push(ProblemKind::AccessTrunk, Severity::Warning, 1, msg!("{} ist Access-Port (VLAN {}), {} ist Trunk.", name(1), v, name(0)));
                }
                (Some(PortMode::Access(v)), None) if tagging(1) => {
                    let text = msg!("{} hat 802.1Q-Subinterfaces, {} ist aber Access-Port (VLAN {}): getaggte Frames verwirft der Switch.", name(1), name(0), v);
                    push(ProblemKind::AccessTrunk, Severity::Warning, 0, text);
                }
                (None, Some(PortMode::Access(v))) if tagging(0) => {
                    let text = msg!("{} hat 802.1Q-Subinterfaces, {} ist aber Access-Port (VLAN {}): getaggte Frames verwirft der Switch.", name(0), name(1), v);
                    push(ProblemKind::AccessTrunk, Severity::Warning, 1, text);
                }
                _ => {}
            }
        }
    }

    /// circles of bridges and hubs over cables that are up
    fn loops(&self, tags: &BTreeSet<u16>, out: &mut Vec<Problem>) {
        let edges = self.l2_edges(true, tags);
        let mut uf = Uf::default();
        // the tree edges so far: spot -> (spot, edge)
        let mut adj: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
        // the devices of a circle -> (the edge that closed it first, its VLANs)
        let mut found: BTreeMap<Vec<u32>, (usize, BTreeSet<u16>)> = BTreeMap::new();
        for (k, e) in edges.iter().enumerate() {
            let (a, b) = (uf.id(e.a), uf.id(e.b));
            if uf.join(a, b) {
                adj.entry(a).or_default().push((b, k));
                adj.entry(b).or_default().push((a, k));
                continue;
            }
            if found.len() >= MAX_LOOPS {
                continue;
            }
            let mut circle = tree_path(&adj, b, a);
            circle.push(k);
            let devs: BTreeSet<u32> = circle.iter().map(|&x| self.dev_of(edges[x].node)).collect();
            let mut vlans = BTreeSet::new();
            for s in circle.iter().flat_map(|&x| [edges[x].a, edges[x].b]) {
                match s {
                    Spot::Vlan(_, v) | Spot::Wire(_, v) if v != 0 => {
                        vlans.insert(v);
                    }
                    _ => {}
                }
            }
            found.entry(devs.into_iter().collect()).or_insert((k, BTreeSet::new())).1.extend(vlans);
        }
        for (devs, (k, vlans)) in found {
            // spanning tree breaks the circle when every bridge in it runs it (hubs repeat BPDUs)
            let bridges: Vec<u32> = devs.iter().copied().filter(|&d| self.node(d).is_some_and(|n| n.bridge.is_some())).collect();
            let without: Vec<u32> = bridges.iter().copied().filter(|&d| !self.node_index(d).is_some_and(|ni| self.stp_runs(ni))).collect();
            if !bridges.is_empty() && without.is_empty() {
                continue;
            }
            let e = edges[k];
            let dev = self.dev_of(e.node);
            let names: Vec<String> = devs.iter().map(|&d| self.dev_name(d)).collect();
            let vl: Vec<String> = vlans.iter().map(|v| format!("{}", v)).collect();
            let link = self.links.get(e.link).and_then(|l| l.as_ref()).map(|l| l.id);
            let others: Vec<u32> = devs.iter().copied().filter(|&d| d != dev).collect();
            let off: Vec<String> = without.iter().map(|&d| self.dev_name(d)).collect();
            let text = if without.len() < bridges.len() {
                // some bridges of the circle run spanning tree, these do not
                if vl.is_empty() {
                    msg!("Schleife über {}: {} ohne Spanning Tree, Broadcasts kreisen endlos; „spanning-tree mode rstp“ dort schaltet ihn ein.", names.join(", "), off.join(", "))
                } else {
                    msg!("Schleife über {} (VLAN {}): {} ohne Spanning Tree, Broadcasts kreisen endlos; „spanning-tree mode rstp“ dort schaltet ihn ein.", names.join(", "), vl.join(", "), off.join(", "))
                }
            } else if vl.is_empty() {
                msg!("Schleife ohne Spanning Tree über {}: Broadcasts kreisen endlos.", names.join(", "))
            } else {
                msg!("Schleife ohne Spanning Tree über {} (VLAN {}): Broadcasts kreisen endlos.", names.join(", "), vl.join(", "))
            };
            out.push(problem(ProblemKind::Loop, Severity::Error, dev, Some(e.port), link, others, text));
        }
    }
}

impl Engine {
    /// the IPv6 addresses duplicate address detection found taken (RFC 4862 5.4.5): the address
    /// stays unused until the interface starts again
    fn dup_ip6s(&self, out: &mut Vec<Problem>) {
        for (&dev, &ni) in &self.dev_idx {
            let Some(Some(n)) = self.nodes.get(ni) else {
                continue;
            };
            let Some(st) = n.ip.as_ref() else {
                continue;
            };
            for (i, f) in st.v6.ifs.iter().enumerate() {
                let Some(x) = st.ifaces.get(i) else {
                    continue;
                };
                for a in f.addrs.iter().filter(|a| a.state == crate::ip6::AddrState::Duplicate) {
                    let port = match x.kind {
                        IfKind::Port(p) | IfKind::Sub { port: p, .. } => Some(p),
                        _ => None,
                    };
                    let others: Vec<u32> = a.dup_mac.and_then(|m| self.nodes.iter().flatten().find(|o| o.ports.iter().any(|p| p.mac == m) || o.base_mac == m)).map(|o| alloc::vec![o.dev]).unwrap_or_default();
                    let m = match a.dup_mac {
                        Some(mac) => msg!("{} ist an {} doppelt: die Adressprüfung (DAD) fand {} damit.", a.addr, x.name, mac),
                        None => msg!("{} ist an {} doppelt: ein anderes Gerät prüft oder benutzt dieselbe Adresse.", a.addr, x.name),
                    };
                    out.push(Problem { kind: ProblemKind::DuplicateIpv6, severity: Severity::Error, dev, port, link: None, others, text: m.text(), msg: m });
                }
            }
        }
    }
}
