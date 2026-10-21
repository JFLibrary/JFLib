// bridge -- an IEEE 802.1Q learning bridge: MAC table with aging, VLANs on access and trunk
// ports, flooding, filtering, tagging on egress
//
//   VlanSet               a set of VLAN ids (bit set over 1..4094), ranges "1-10,20"
//   PortMode              Access(vlan) | Trunk { native, allowed }
//   Bridge::new(ports)    every Ethernet port an access port in VLAN 1; console ports stay out
//   set_access / set_trunk / add_vlan / remove_vlan
//   switchport(port, SwSet)  the IOS settings of a port one by one: "switchport mode", the
//                         access VLAN, the native and the allowed VLANs of a trunk (the mode
//                         decides which of them count; dynamic ports act as access ports)
//   lookup / learn / entries / flush_port
//   Engine::bridge_input  the forwarding decision for one received frame
//
// Ingress: an access port takes untagged frames (and priority tags with VLAN 0) into its VLAN
// and drops tagged ones; a trunk takes untagged frames into its native VLAN and tagged ones
// when their VLAN is allowed.  Egress: untagged on access ports and for the native VLAN of a
// trunk, tagged otherwise.  Link-local control frames (01:80:c2:00:00:0x) stay in the bridge.
// MAC entries age out after 300 s and are flushed when a port's link goes down.  Frames to the
// device's own MAC go to its interface in that VLAN (SVI) when it has one that is up;
// broadcasts and groups reach it as a copy.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use jfn_model::catalog::{Media, PortSpec};
use netpkt::eth::{self, VlanTag, L2};
use netpkt::Mac;

use crate::ip::IfKind;
use crate::kernel::{Engine, Frame, Ns, SEC};
use crate::trace::{HopId, Note};

/// a set of VLAN ids 1..=4094
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct VlanSet([u64; 64]);

impl Default for VlanSet {
    fn default() -> VlanSet {
        VlanSet::all()
    }
}

impl core::fmt::Debug for VlanSet {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        write!(f, "VlanSet({})", self.to_text())
    }
}

impl VlanSet {
    pub fn none() -> VlanSet {
        VlanSet([0; 64])
    }
    /// 1..=4094
    pub fn all() -> VlanSet {
        let mut s = VlanSet([u64::MAX; 64]);
        s.0[0] &= !1; // VLAN 0 (priority tags)
        s.0[63] &= !(1u64 << 63); // VLAN 4095 (reserved)
        s
    }
    pub fn contains(&self, v: u16) -> bool {
        v > 0 && v < 4095 && self.0[(v / 64) as usize] & (1u64 << (v % 64)) != 0
    }
    pub fn insert(&mut self, v: u16) {
        if v > 0 && v < 4095 {
            self.0[(v / 64) as usize] |= 1u64 << (v % 64);
        }
    }
    pub fn remove(&mut self, v: u16) {
        if v < 4096 {
            self.0[(v / 64) as usize] &= !(1u64 << (v % 64));
        }
    }
    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|&w| w == 0)
    }
    /// "1-10,20,30-40" (IOS syntax; "all" and "none" too)
    pub fn parse(s: &str) -> Option<VlanSet> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("all") {
            return Some(VlanSet::all());
        }
        if s.eq_ignore_ascii_case("none") {
            return Some(VlanSet::none());
        }
        let mut set = VlanSet::none();
        for part in s.split(',') {
            let part = part.trim();
            let (a, b) = match part.split_once('-') {
                Some((a, b)) => (a.trim().parse::<u16>().ok()?, b.trim().parse::<u16>().ok()?),
                None => {
                    let v = part.parse::<u16>().ok()?;
                    (v, v)
                }
            };
            if a == 0 || b > 4094 || a > b {
                return None;
            }
            for v in a..=b {
                set.insert(v);
            }
        }
        Some(set)
    }
    /// the ranges as IOS prints them ("1-4094", "10,20-22", "none")
    pub fn to_text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut v = 1u16;
        while v <= 4094 {
            if self.contains(v) {
                let start = v;
                while v < 4094 && self.contains(v + 1) {
                    v += 1;
                }
                parts.push(if start == v { format!("{}", v) } else { format!("{}-{}", start, v) });
            }
            v += 1;
        }
        if parts.is_empty() { String::from("none") } else { parts.join(",") }
    }
    /// the VLANs in both sets
    pub fn intersect(&self, o: &VlanSet) -> VlanSet {
        let mut s = VlanSet::none();
        for (w, (a, b)) in s.0.iter_mut().zip(self.0.iter().zip(o.0.iter())) {
            *w = a & b;
        }
        s
    }
}

/// how a bridge port treats VLANs
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortMode {
    Access(u16),
    Trunk { native: u16, allowed: VlanSet },
}

/// what "switchport mode" says
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwMode {
    /// "dynamic auto", the default: without DTP partners it is an access port
    Dynamic,
    Access,
    Trunk,
}

/// one IOS switchport setting
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SwSet {
    Mode(SwMode),
    /// "switchport access vlan N"
    Access(u16),
    /// "switchport trunk native vlan N"
    Native(u16),
    /// "switchport trunk allowed vlan ..."
    Allowed(VlanSet),
}

/// a port of the bridge
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgePort {
    /// how it treats VLANs now (follows from the settings below)
    pub mode: PortMode,
    /// takes part in bridging (Ethernet ports do, console ports do not)
    pub member: bool,
    pub admin: SwMode,
    pub access_vlan: u16,
    pub native_vlan: u16,
    pub allowed: VlanSet,
}

impl BridgePort {
    fn new(member: bool) -> BridgePort {
        BridgePort { mode: PortMode::Access(1), member, admin: SwMode::Dynamic, access_vlan: 1, native_vlan: 1, allowed: VlanSet::all() }
    }
    /// the mode that follows from the settings
    fn settle(&mut self) {
        self.mode = match self.admin {
            SwMode::Trunk => PortMode::Trunk { native: self.native_vlan, allowed: self.allowed.clone() },
            _ => PortMode::Access(self.access_vlan),
        };
    }
}

/// a MAC table entry
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FdbEntry {
    pub port: u16,
    pub last: Ns,
    /// configured by hand (never ages)
    pub fixed: bool,
}

/// the switching part of a device
#[derive(Clone, Debug)]
pub struct Bridge {
    pub ports: Vec<BridgePort>,
    pub fdb: BTreeMap<(u16, Mac), FdbEntry>,
    pub aging: Ns,
    pub max_entries: usize,
    /// configured VLANs and their names (VLAN 1 "default" always)
    pub vlans: BTreeMap<u16, String>,
}

impl Bridge {
    pub fn new(ports: &[PortSpec]) -> Bridge {
        let mut vlans = BTreeMap::new();
        vlans.insert(1, String::from("default"));
        Bridge {
            ports: ports.iter().map(|p| BridgePort::new(p.media.is_ethernet())).collect(),
            fdb: BTreeMap::new(),
            aging: 300 * SEC,
            max_entries: 16_384,
            vlans,
        }
    }
    pub fn set_access(&mut self, port: u16, vlan: u16) -> bool {
        if !(1..=4094).contains(&vlan) {
            return false;
        }
        let Some(p) = self.ports.get_mut(port as usize) else {
            return false;
        };
        p.admin = SwMode::Access;
        p.access_vlan = vlan;
        p.settle();
        self.vlans.entry(vlan).or_insert_with(|| format!("VLAN{:04}", vlan));
        self.flush_port(port);
        true
    }
    pub fn set_trunk(&mut self, port: u16, native: u16, allowed: VlanSet) -> bool {
        if !(1..=4094).contains(&native) {
            return false;
        }
        let Some(p) = self.ports.get_mut(port as usize) else {
            return false;
        };
        p.admin = SwMode::Trunk;
        p.native_vlan = native;
        p.allowed = allowed;
        p.settle();
        self.flush_port(port);
        true
    }
    /// one IOS switchport setting; false for a VLAN out of 1..4094 or an unknown port.  An
    /// access VLAN that does not exist yet is created (as IOS does).
    pub fn switchport(&mut self, port: u16, set: SwSet) -> bool {
        let Some(p) = self.ports.get_mut(port as usize) else {
            return false;
        };
        let before = p.mode.clone();
        match set {
            SwSet::Mode(m) => p.admin = m,
            SwSet::Access(v) | SwSet::Native(v) if !(1..=4094).contains(&v) => return false,
            SwSet::Access(v) => p.access_vlan = v,
            SwSet::Native(v) => p.native_vlan = v,
            SwSet::Allowed(a) => p.allowed = a,
        }
        p.settle();
        let changed = p.mode != before;
        let access = p.access_vlan;
        self.vlans.entry(access).or_insert_with(|| format!("VLAN{:04}", access));
        if changed {
            self.flush_port(port);
        }
        true
    }
    pub fn add_vlan(&mut self, vid: u16, name: Option<&str>) -> bool {
        if !(1..=4094).contains(&vid) {
            return false;
        }
        let n = match name {
            Some(n) => String::from(n),
            None => format!("VLAN{:04}", vid),
        };
        self.vlans.insert(vid, n);
        true
    }
    pub fn remove_vlan(&mut self, vid: u16) -> bool {
        vid != 1 && self.vlans.remove(&vid).is_some()
    }
    /// the VLAN a received frame belongs to, or why it is dropped
    pub fn ingress_vlan(&self, port: u16, tag: Option<VlanTag>) -> Result<u16, Note> {
        let Some(p) = self.ports.get(port as usize) else {
            return Err(Note::DropMalformed { port });
        };
        match (&p.mode, tag) {
            (PortMode::Access(v), None) => Ok(*v),
            (PortMode::Access(v), Some(t)) if t.vid == 0 => Ok(*v),
            (PortMode::Access(_), Some(t)) => Err(Note::DropTagged { port, vid: t.vid }),
            (PortMode::Trunk { native, .. }, None) => Ok(*native),
            (PortMode::Trunk { native, .. }, Some(t)) if t.vid == 0 => Ok(*native),
            (PortMode::Trunk { allowed, .. }, Some(t)) => {
                if allowed.contains(t.vid) {
                    Ok(t.vid)
                } else {
                    Err(Note::DropVlan { port, vid: t.vid })
                }
            }
        }
    }
    /// the port carries VLAN `vlan`
    pub fn carries(&self, port: u16, vlan: u16) -> bool {
        match self.ports.get(port as usize) {
            Some(BridgePort { member: true, mode: PortMode::Access(v), .. }) => *v == vlan,
            Some(BridgePort { member: true, mode: PortMode::Trunk { native, allowed }, .. }) => *native == vlan || allowed.contains(vlan),
            _ => false,
        }
    }
    /// frames of `vlan` leave this port with a tag
    pub fn tags(&self, port: u16, vlan: u16) -> bool {
        matches!(self.ports.get(port as usize), Some(BridgePort { mode: PortMode::Trunk { native, .. }, .. }) if *native != vlan)
    }
    /// the port behind a MAC in a VLAN, unless the entry aged out
    pub fn lookup(&self, vlan: u16, mac: Mac, now: Ns) -> Option<u16> {
        let e = self.fdb.get(&(vlan, mac))?;
        if e.fixed || now.saturating_sub(e.last) <= self.aging { Some(e.port) } else { None }
    }
    /// learns a source address; the note when something changed
    pub fn learn(&mut self, vlan: u16, mac: Mac, port: u16, now: Ns) -> Option<Note> {
        let aging = self.aging;
        if let Some(e) = self.fdb.get_mut(&(vlan, mac)) {
            if e.fixed {
                return None;
            }
            let stale = now.saturating_sub(e.last) > aging;
            let old = e.port;
            e.port = port;
            e.last = now;
            return if old != port && !stale {
                Some(Note::Moved { mac, from: old, to: port, vlan })
            } else if stale || old != port {
                Some(Note::Learned { mac, port, vlan })
            } else {
                None
            };
        }
        if self.fdb.len() >= self.max_entries {
            // make room from aged entries first
            self.fdb.retain(|_, e| e.fixed || now.saturating_sub(e.last) <= aging);
            if self.fdb.len() >= self.max_entries {
                return Some(Note::TableFull { vlan });
            }
        }
        self.fdb.insert((vlan, mac), FdbEntry { port, last: now, fixed: false });
        Some(Note::Learned { mac, port, vlan })
    }
    /// forgets what was learned on a port
    pub fn flush_port(&mut self, port: u16) {
        self.fdb.retain(|_, e| e.fixed || e.port != port);
    }
    /// the live entries: (vlan, mac, port, fixed), sorted by VLAN and MAC
    pub fn entries(&self, now: Ns) -> Vec<(u16, Mac, u16, bool)> {
        self.fdb.iter().filter(|(_, e)| e.fixed || now.saturating_sub(e.last) <= self.aging).map(|(&(v, m), e)| (v, m, e.port, e.fixed)).collect()
    }
}

/// IEEE 802.1D link-local group addresses (STP, LACP, LLDP, 802.1X): never forwarded
fn link_local(m: Mac) -> bool {
    m.0[..5] == [0x01, 0x80, 0xC2, 0x00, 0x00] && m.0[5] <= 0x0F
}

impl Engine {
    /// the forwarding decision of a bridge for one frame received on port `pi`
    pub(crate) fn bridge_input(&mut self, ni: usize, pi: u16, frame: Frame, hop: HopId) {
        let now = self.now;
        // ports whose link carries traffic (a bridge floods only to those)
        let live: Vec<bool> = match self.nodes.get(ni) {
            Some(Some(n)) => n.ports.iter().map(|p| p.link.is_some_and(|li| self.link_up(li))).collect(),
            _ => return,
        };
        // the bridge's own interfaces that are up: (vlan, interface)
        let svis: Vec<(u16, u16)> = match self.stack(ni) {
            Some(st) => st
                .ifaces
                .iter()
                .enumerate()
                .filter_map(|(i, f)| match f.kind {
                    IfKind::Svi(v) if self.iface_up(ni, i as u16) => Some((v, i as u16)),
                    _ => None,
                })
                .collect(),
            None => Vec::new(),
        };
        let Some(Some(node)) = self.nodes.get_mut(ni) else {
            return;
        };
        let dev = node.dev;
        let Some(l2) = L2::parse(&frame) else {
            self.note(dev, Some(hop), Note::DropMalformed { port: pi });
            return;
        };
        let Some(br) = node.bridge.as_mut() else {
            return;
        };
        if !br.ports.get(pi as usize).is_some_and(|p| p.member) {
            return;
        }
        let vlan = match br.ingress_vlan(pi, l2.tag) {
            Ok(v) => v,
            Err(n) => {
                node.ports[pi as usize].counters.drops += 1;
                self.note(dev, Some(hop), n);
                return;
            }
        };
        if link_local(l2.dst) {
            self.note(dev, Some(hop), Note::Consumed { port: pi });
            return;
        }
        let learned = if l2.src.is_unicast() { br.learn(vlan, l2.src, pi, now) } else { None };
        let svi = svis.iter().find(|s| s.0 == vlan).map(|s| s.1);
        if l2.dst == node.base_mac {
            // for the bridge itself: its interface in this VLAN takes it
            if let Some(n) = learned {
                self.note(dev, Some(hop), n);
            }
            match svi {
                Some(i) => {
                    let plain: Frame = if l2.tag.is_some() { Arc::from(eth::untagged(&frame)) } else { frame };
                    self.l3_from_svi(ni, i, &plain, hop);
                }
                None => self.note(dev, Some(hop), Note::NotForMe { port: pi }),
            }
            return;
        }
        let target = if l2.dst.is_unicast() { br.lookup(vlan, l2.dst, now) } else { None };
        // the ports that carry the VLAN and have a cable
        let out: Vec<u16> = match target {
            Some(p) if p == pi => Vec::new(),
            Some(p) => alloc::vec![p],
            None => (0..br.ports.len() as u16).filter(|&p| p != pi && br.carries(p, vlan) && live[p as usize]).collect(),
        };
        let tagged_on: Vec<bool> = out.iter().map(|&p| br.tags(p, vlan)).collect();
        if let Some(n) = learned {
            self.note(dev, Some(hop), n);
        }
        match target {
            Some(p) if p == pi => {
                self.note(dev, Some(hop), Note::Filtered { vlan });
                return;
            }
            Some(p) => self.note(dev, Some(hop), Note::Forwarded { vlan, port: p }),
            None => self.note(dev, Some(hop), Note::Flooded { vlan, ports: out.len() as u16 }),
        }
        // one untagged and (if needed) one tagged copy, shared by every egress port
        let plain: Frame = if l2.tag.is_some() { Arc::from(eth::untagged(&frame)) } else { frame };
        let mut tagged: Option<Frame> = None;
        for (i, p) in out.into_iter().enumerate() {
            let f = if tagged_on[i] {
                tagged.get_or_insert_with(|| Arc::from(eth::tagged(&plain, VlanTag::new(vlan)))).clone()
            } else {
                plain.clone()
            };
            self.send(ni, p, f, Some(hop));
        }
        // broadcasts and groups also reach the bridge's own interface in the VLAN
        if l2.dst.is_multicast()
            && let Some(i) = svi
        {
            self.l3_from_svi(ni, i, &plain, hop);
        }
    }

    /// a frame from the bridge's own interface in `vlan`: to the port behind the destination,
    /// or flooded to every live port of the VLAN (tagged on trunks)
    pub(crate) fn bridge_output(&mut self, ni: usize, vlan: u16, frame: Frame, cause: Option<HopId>) {
        let now = self.now;
        let Some(Some(node)) = self.nodes.get(ni) else {
            return;
        };
        let Some(br) = node.bridge.as_ref() else {
            return;
        };
        let live = |p: u16| node.ports[p as usize].link.is_some_and(|li| self.link_up(li));
        let dst = netpkt::eth::EthView::parse(&frame).map(|e| e.dst());
        let target = dst.filter(|d| d.is_unicast()).and_then(|d| br.lookup(vlan, d, now)).filter(|&p| live(p) && br.carries(p, vlan));
        let out: Vec<u16> = match target {
            Some(p) => alloc::vec![p],
            None => (0..br.ports.len() as u16).filter(|&p| br.carries(p, vlan) && live(p)).collect(),
        };
        let tags: Vec<bool> = out.iter().map(|&p| br.tags(p, vlan)).collect();
        let mut tagged: Option<Frame> = None;
        for (i, p) in out.into_iter().enumerate() {
            let f = if tags[i] {
                tagged.get_or_insert_with(|| Arc::from(eth::tagged(&frame, VlanTag::new(vlan)))).clone()
            } else {
                frame.clone()
            };
            self.send(ni, p, f, cause);
        }
    }

    /// the bridge of a device
    pub fn bridge(&self, dev: u32) -> Option<&Bridge> {
        self.node(dev)?.bridge.as_ref()
    }
}

/// true for media a bridge forwards between
pub fn bridged(m: Media) -> bool {
    m.is_ethernet()
}
