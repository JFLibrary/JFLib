// view -- what the canvas and the inspector show: devices, ports, links and their LEDs, and
// the frames that are on a cable right now
//
//   Engine::view() -> View          a snapshot (the UI keeps it until the next tick)
//   Engine::port_led(dev, port)     the LED of one port end
//   Engine::in_flight(limit)        frames with t_tx <= now < t_rx (newest first, at most
//                                   `limit` recent frames are looked at)
//   Engine::ifaces(dev) / routes(dev) / arp_table(dev)
//                                   IPv4 interfaces with their state (and their IPv6
//                                   addresses: IfView::addrs6), the routing table as it is
//                                   used now, the ARP cache without aged entries
//   Engine::describe(&DevEvent)     the German sentence of a decision, with the device's
//                                   port and interface names; Engine::message(&DevEvent): the
//                                   same as a Msg for the app's catalogue (EventRow::msg too)
//
// Simulation mode (below): frames_at(t) for the animation (which frame is how far along which
// cable), drops_between (the red crosses), event_row(s) for the event list, step_of /
// hop_steps for the packet tab (the PDU a device got, its decisions, the PDUs it sent),
// hop_layers / hop_info (the dissection, HDLC frames of serial lines included).
//
// LEDs: no power (grey), down (red: shut down, err-disabled, unplugged, far end off, wrong cable
// with strict MDI-X), up (green); with spanning tree a bridge port that does not forward is
// blocking (amber: Blocking, Listening, Discarding) or learning (amber: Learning).  The radio of
// an access point without a cable is up while it sends a network (wlan.rs); WLAN associations
// are links of the view too (ids from wlan::ASSOC_ID0), the views of wlan.rs tell more.

use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::cable::CableKind;
use jfn_model::msg::Msg;
use netpkt::{Ipv4, Mac, Prefix4};

use crate::ip::{IfKind, Route};
use crate::kernel::{Engine, Ns};
use crate::node::PortCounters;
use crate::trace::{DevEvent, Hop, HopId};

/// the LED of a port end
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Led {
    NoPower,
    Down,
    Up,
    Blocking,
    Learning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortView {
    pub index: u16,
    pub name: String,
    pub admin_up: bool,
    pub up: bool,
    pub led: Led,
    /// project link id
    pub link: Option<u32>,
    pub mac: Mac,
    pub kbps: u32,
    pub counters: PortCounters,
}

/// an IPv4 interface ("show ip interface brief", "ip addr")
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfView {
    /// the interface number (what Note::describe and ArpRow name)
    pub index: u16,
    pub name: String,
    pub kind: IfKind,
    pub mac: Mac,
    pub addrs: Vec<Prefix4>,
    pub mtu: u16,
    /// not shut down (ports: the port's state)
    pub admin_up: bool,
    /// carries traffic
    pub up: bool,
    pub proxy_arp: bool,
    /// its IPv6 addresses, the link-local one first (empty without IPv6: nd.rs)
    pub addrs6: Vec<crate::nd::Addr6View>,
}

/// an entry of the ARP cache
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArpRow {
    pub iface: u16,
    pub ip: Ipv4,
    /// None: asked, no answer yet ("incomplete")
    pub mac: Option<Mac>,
    /// since it was learned or last asked for
    pub age: Ns,
    pub fixed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevView {
    pub dev: u32,
    pub name: String,
    pub power: bool,
    pub ports: Vec<PortView>,
    /// IPv4 interfaces (empty for devices without IP)
    pub ifaces: Vec<IfView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkView {
    pub id: u32,
    pub up: bool,
    /// (device id, port index) of both ends
    pub ends: [(u32, u16); 2],
    pub led: [Led; 2],
    /// the speed it runs at
    pub kbps: u32,
    pub cable: CableKind,
    pub wrong_cable: bool,
    /// frames waiting to be sent
    pub queued: usize,
}

/// a snapshot of the simulation for the UI
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub now: Ns,
    pub devices: Vec<DevView>,
    pub links: Vec<LinkView>,
    pub pending: usize,
    pub next_hop: HopId,
}

impl Engine {
    fn led_of(&self, ni: usize, pi: u16) -> Led {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return Led::NoPower;
        };
        if !n.power {
            return Led::NoPower;
        }
        let Some(port) = n.ports.get(pi as usize) else {
            return Led::Down;
        };
        match port.link {
            Some(li) if port.admin_up && self.link_up(li) => match n.bridge.as_ref().filter(|b| b.stp.on() && b.ports.get(pi as usize).is_some_and(|bp| bp.member)).map(|b| b.stp.state(pi)) {
                Some(crate::stp::PortState::Learning) => Led::Learning,
                Some(crate::stp::PortState::Blocking | crate::stp::PortState::Listening | crate::stp::PortState::Discarding) => Led::Blocking,
                _ => Led::Up,
            },
            // the radio of an access point that beacons (wlan.rs)
            None if self.wlan_radio_up(ni, pi) => Led::Up,
            _ => Led::Down,
        }
    }
    /// the LED of a port of a device
    pub fn port_led(&self, dev: u32, port: u16) -> Led {
        match self.node_index(dev) {
            Some(ni) => self.led_of(ni, port),
            None => Led::NoPower,
        }
    }
    /// the state of everything
    pub fn view(&self) -> View {
        let mut devices = Vec::new();
        for (&dev, &ni) in &self.dev_idx {
            let Some(Some(n)) = self.nodes.get(ni) else {
                continue;
            };
            let ports = n
                .ports
                .iter()
                .enumerate()
                .map(|(i, p)| PortView {
                    index: i as u16,
                    name: p.spec.name.clone(),
                    admin_up: p.admin_up,
                    up: p.link.is_some_and(|li| self.link_up(li)) || self.wlan_radio_up(ni, i as u16),
                    led: self.led_of(ni, i as u16),
                    link: p.link.and_then(|li| self.links.get(li)?.as_ref().map(|l| l.id)),
                    mac: p.mac,
                    kbps: p.link.map(|li| self.link_kbps(li)).unwrap_or(p.spec.kbps),
                    counters: p.counters,
                })
                .collect();
            devices.push(DevView { dev, name: n.name.clone(), power: n.power, ports, ifaces: self.ifaces_of(ni) });
        }
        let mut links = Vec::new();
        for (&id, &li) in &self.link_idx {
            let Some(Some(l)) = self.links.get(li) else {
                continue;
            };
            let dev = |n: usize| self.nodes.get(n).and_then(|x| x.as_ref()).map(|x| x.dev).unwrap_or(0);
            links.push(LinkView {
                id,
                up: self.link_up(li),
                ends: [(dev(l.ends[0].0), l.ends[0].1), (dev(l.ends[1].0), l.ends[1].1)],
                led: [self.led_of(l.ends[0].0, l.ends[0].1), self.led_of(l.ends[1].0, l.ends[1].1)],
                kbps: self.link_kbps(li),
                cable: l.cable,
                wrong_cable: l.wrong_cable,
                queued: l.queued(),
            });
        }
        View { now: self.now, devices, links, pending: self.pending(), next_hop: self.trace.next_hop_id() }
    }
    /// the frames on a cable at this moment (newest first; looks at the last `limit` frames)
    pub fn in_flight(&self, limit: usize) -> Vec<&Hop> {
        let now = self.now;
        self.trace.hops().rev().take(limit).filter(|h| h.t_tx <= now && now < h.t_rx).collect()
    }

    fn ifaces_of(&self, ni: usize) -> Vec<IfView> {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return Vec::new();
        };
        let Some(st) = n.ip.as_ref() else {
            return Vec::new();
        };
        st.ifaces
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.hidden)
            .map(|(i, f)| IfView {
                index: i as u16,
                name: f.name.clone(),
                kind: f.kind,
                mac: f.mac,
                addrs: f.addrs.clone(),
                mtu: f.mtu,
                admin_up: match f.kind {
                    IfKind::Port(p) => n.ports.get(p as usize).is_some_and(|port| port.admin_up),
                    _ => f.admin_up,
                },
                up: self.iface_up(ni, i as u16),
                proxy_arp: f.proxy_arp,
                addrs6: self.addrs6_of(ni, i as u16),
            })
            .collect()
    }
    /// the IPv4 interfaces of a device
    pub fn ifaces(&self, dev: u32) -> Vec<IfView> {
        self.node_index(dev).map(|ni| self.ifaces_of(ni)).unwrap_or_default()
    }
    /// the routing table of a device as it is used now (routes over interfaces that are down
    /// are left out), ordered by network, prefix length and distance
    pub fn routes(&self, dev: u32) -> Vec<Route> {
        let Some(ni) = self.node_index(dev) else {
            return Vec::new();
        };
        let Some(st) = self.stack(ni) else {
            return Vec::new();
        };
        let mut v = st.rib(&|i| self.iface_up(ni, i));
        v.sort_by_key(|r| (r.prefix.addr.to_u32(), r.prefix.len, r.ad, r.src));
        v
    }
    /// the ARP cache of a device (aged entries left out)
    pub fn arp_table(&self, dev: u32) -> Vec<ArpRow> {
        let Some(st) = self.node_index(dev).and_then(|ni| self.stack(ni)) else {
            return Vec::new();
        };
        let now = self.now;
        st.arp
            .iter()
            .filter(|(_, e)| e.fixed || e.mac.is_none() || now.saturating_sub(e.at) <= st.arp_timeout)
            .map(|(&(iface, ip), e)| ArpRow { iface, ip, mac: e.mac, age: now.saturating_sub(e.at), fixed: e.fixed })
            .collect()
    }
    /// what a device decided, as a German sentence with its port and interface names
    pub fn describe(&self, ev: &DevEvent) -> String {
        self.message(ev).text()
    }
    /// describe() as a message for the app's catalogue
    pub fn message(&self, ev: &DevEvent) -> Msg {
        let n = self.node(ev.dev);
        let port = |p: u16| match n {
            Some(n) => String::from(n.port_name(p)),
            None => alloc::format!("Port {}", p),
        };
        let iface = |i: u16| match n.and_then(|n| n.ip.as_ref()).and_then(|s| s.ifaces.get(i as usize)) {
            Some(f) => f.name.clone(),
            None => alloc::format!("Interface {}", i),
        };
        ev.note.message(&port, &iface)
    }
}

// ---------------------------------------------------------------- simulation mode

use alloc::format;

use netpkt::decode::{self, Field, Layer, Proto};

use crate::trace::{Filter, HopDrop, Note};

/// a frame on its cable at a moment (the animation of the simulation mode)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InFlight {
    pub hop: HopId,
    /// project link id
    pub link: u32,
    /// 0: it runs from the first end of the LinkView to the second, 1: back
    pub dir: u8,
    /// (device id, port index) of the sender and of the receiver
    pub from: (u32, u16),
    pub to: (u32, u16),
    pub t_tx: Ns,
    pub t_rx: Ns,
    /// how far it got: 0 at the sender .. 1_000_000 at the receiver
    pub pos: u32,
    /// netpkt::decode P_* bits (its colour)
    pub mask: u32,
    /// it does not arrive (as far as the recording knows)
    pub drop: Option<HopDrop>,
}

impl InFlight {
    /// how far it got, 0.0 .. 1.0 (for ui::dg_point_at)
    pub fn fraction(&self) -> f32 {
        self.pos as f32 / 1_000_000.0
    }
}

/// something was thrown away: a device dropped a frame, or a cable lost one (the red cross of
/// the animation)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropMark {
    pub t: Ns,
    /// the device that dropped it, or the receiver of a frame the cable lost
    pub dev: u32,
    pub hop: Option<HopId>,
    /// the decision (None: lost on the cable)
    pub seq: Option<u64>,
    pub lost: Option<HopDrop>,
}

/// a line of the event list
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventRow {
    pub seq: u64,
    pub t: Ns,
    /// the device the frame came from (None: the device acted on its own)
    pub from: Option<u32>,
    /// the device that decided
    pub dev: u32,
    pub hop: Option<HopId>,
    /// 1 cable, 2 bridge and ARP, 3 IPv4 and ICMP, 4 UDP / TCP
    pub layer: u8,
    /// the frame's top protocol, or the decision's own one ("ARP" for the question a router
    /// asks before it forwards a ping); "" for cables
    pub proto: &'static str,
    /// P_* bits of the frame and of the decision (colours, filter chips)
    pub mask: u32,
    /// the decision as a German sentence (Engine::describe)
    pub text: String,
    pub drop: bool,
    /// the decision as a message for the app's catalogue (text is its German text)
    pub msg: Msg,
}

/// what one device did in one batch: the frame it handled, its decisions, the frames it sent
/// (the packet tab with the inbound and outbound PDU)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevStep {
    pub dev: u32,
    pub batch: u64,
    pub t: Ns,
    /// the frame that arrived (None: the device acted on its own: a command, a job, a timer)
    pub inbound: Option<HopId>,
    /// what it decided, in order
    pub decisions: Vec<DevEvent>,
    /// the frames it sent, in order (later than t when they waited in a queue)
    pub outbound: Vec<HopId>,
}

/// protocols from the top of the stack down: a P_* mask is named after its highest one
const TOP_DOWN: [Proto; 27] = [
    Proto::Bgp,
    Proto::Dhcpv6,
    Proto::Lldp,
    Proto::Snmp,
    Proto::Tftp,
    Proto::Syslog,
    Proto::Ntp,
    Proto::Ospf,
    Proto::Rip,
    Proto::Stp,
    Proto::Ssh,
    Proto::Tls,
    Proto::Http,
    Proto::Dns,
    Proto::Dhcp,
    Proto::Tcp,
    Proto::Udp,
    Proto::Igmp,
    Proto::Icmpv6,
    Proto::Icmp,
    Proto::Ipv6,
    Proto::Ipv4,
    Proto::Eapol,
    Proto::Arp,
    Proto::Llc,
    Proto::Vlan,
    Proto::Eth,
];

/// the name of the top protocol among P_* bits ("ICMP", "ARP"; "" for none)
pub fn proto_name(mask: u32) -> &'static str {
    TOP_DOWN.iter().find(|p| mask & p.bit() != 0).map_or("", |p| p.name())
}

/// netpkt dissects Ethernet: the packet of a Cisco HDLC frame goes behind a stand-in header
/// (12 bytes of addresses, then the HDLC protocol field as the ethertype), so it sits
/// HDLC_SHIFT bytes later than in the frame
fn hdlc_standin(b: &[u8]) -> Vec<u8> {
    let mut f = alloc::vec![0u8; 12];
    f.extend_from_slice(&b[2..]);
    f
}

const HDLC_SHIFT: u32 = 10;

/// the layers of a recorded frame for the inspector (netpkt::decode::layers); the Cisco HDLC
/// frame of a serial line starts with a layer of its own (address, control, protocol)
pub fn hop_layers(h: &Hop) -> Vec<Layer> {
    let b = &h.bytes;
    if !h.hdlc || b.len() < 4 {
        return decode::layers(b);
    }
    let et = u16::from_be_bytes([b[2], b[3]]);
    let proto = match netpkt::eth::ethertype_name(et) {
        Some(n) => format!("{} (0x{:04x})", n, et),
        None => format!("0x{:04x}", et),
    };
    let cast = if b[0] & 0x80 != 0 { "broadcast" } else { "unicast" };
    let fields = alloc::vec![
        Field { name: "Address", value: format!("0x{:02x} ({})", b[0], cast), off: 0, len: 1 },
        Field { name: "Control", value: format!("0x{:02x}", b[1]), off: 1, len: 1 },
        Field { name: "Protocol", value: proto, off: 2, len: 2 },
    ];
    // the link layer of a serial line (netpkt has no HDLC protocol of its own)
    let mut out = alloc::vec![Layer { proto: Proto::Eth, off: 0, len: 4, title: String::from("Cisco HDLC"), fields, problems: Vec::new() }];
    for mut l in decode::layers(&hdlc_standin(b)).into_iter().skip(1) {
        l.off -= HDLC_SHIFT;
        for f in l.fields.iter_mut() {
            f.off -= HDLC_SHIFT;
        }
        out.push(l);
    }
    out
}

/// the top protocol of a recorded frame and a one-line summary (netpkt::decode::info)
pub fn hop_info(h: &Hop) -> (Proto, String) {
    if h.hdlc && h.bytes.len() >= 4 { decode::info(&hdlc_standin(&h.bytes)) } else { decode::info(&h.bytes) }
}

impl Engine {
    /// the frames on a cable at `t` that the filter passes, in send order (it looks at most at
    /// the `limit` latest frames sent up to `t`)
    pub fn frames_at(&self, t: Ns, f: &Filter, limit: usize) -> Vec<InFlight> {
        self.trace
            .on_wire(t, limit)
            .into_iter()
            .filter(|h| f.hop(h))
            .map(|h| {
                let dir = match self.link_rt(h.link) {
                    Some(l) if (self.dev_of(l.ends[0].0), l.ends[0].1) != h.from => 1,
                    _ => 0,
                };
                let pos = ((t - h.t_tx) as u128 * 1_000_000 / (h.t_rx - h.t_tx).max(1) as u128) as u32;
                InFlight { hop: h.id, link: h.link, dir, from: h.from, to: h.to, t_tx: h.t_tx, t_rx: h.t_rx, pos, mask: h.mask, drop: h.drop }
            })
            .collect()
    }

    /// what was thrown away after `t0` up to and including `t1`, in time order: decisions that
    /// dropped something (Note::is_drop) and frames a cable lost (at their receiver)
    pub fn drops_between(&self, t0: Ns, t1: Ns, f: &Filter) -> Vec<DropMark> {
        let mut v: Vec<DropMark> = self
            .trace
            .events_between(t0, t1)
            .filter(|e| e.note.is_drop() && !matches!(e.note, Note::LostInFlight { .. }) && f.event(&self.trace, e))
            .map(|e| DropMark { t: e.t, dev: e.dev, hop: e.hop, seq: Some(e.seq), lost: None })
            .collect();
        let lost = self.trace.arrived_between(t0, t1).filter(|h| h.drop.is_some() && f.hop(h));
        v.extend(lost.map(|h| DropMark { t: h.t_rx, dev: h.to.0, hop: Some(h.id), seq: None, lost: h.drop }));
        v.sort_by_key(|d| (d.t, d.seq.unwrap_or(u64::MAX), d.hop));
        v
    }

    /// the line of the event list for a decision
    pub fn event_row(&self, e: &DevEvent) -> EventRow {
        let hop = e.hop.and_then(|h| self.trace.hop(h));
        let hm = hop.map_or(0, |h| h.mask);
        let nm = e.mask(hm);
        let own = nm & !hm;
        let msg = self.message(e);
        EventRow {
            seq: e.seq,
            t: e.t,
            from: hop.map(|h| h.from.0),
            dev: e.dev,
            hop: e.hop,
            layer: e.note.layer(),
            proto: proto_name(if own != 0 { own } else { hm | nm }),
            mask: hm | nm,
            text: msg.text(),
            drop: e.note.is_drop(),
            msg,
        }
    }

    /// the event list after decision number `after` (0: from the start): the decisions the
    /// filter passes, at most `limit` of them
    pub fn event_rows(&self, f: &Filter, after: u64, limit: usize) -> Vec<EventRow> {
        self.trace.events_after(after).filter(|e| f.event(&self.trace, e)).take(limit).map(|e| self.event_row(e)).collect()
    }

    /// what the device of a decision did in that batch (the packet tab of an event)
    pub fn step_of(&self, e: &DevEvent) -> DevStep {
        self.dev_step(e.dev, e.batch, e.t)
    }

    /// what the sender did when it sent a frame, and what the receiver did with it (None when
    /// it is still under way, was lost, or the receiver left nothing in the recording)
    pub fn hop_steps(&self, hop: HopId) -> (Option<DevStep>, Option<DevStep>) {
        let Some(h) = self.trace.hop(hop) else {
            return (None, None);
        };
        let sent = self.dev_step(h.from.0, h.batch, h.t_queued);
        let got = self.trace.events_between(h.t_rx.saturating_sub(1), h.t_rx).find(|e| e.hop == Some(hop) && e.dev == h.to.0).map(|e| self.dev_step(e.dev, e.batch, e.t));
        (Some(sent), got)
    }

    /// what device `dev` did in batch `batch` (at `t`)
    fn dev_step(&self, dev: u32, batch: u64, t: Ns) -> DevStep {
        let decisions: Vec<DevEvent> = self.trace.batch_events(batch, dev).cloned().collect();
        let inbound = decisions.iter().filter_map(|e| e.hop).find(|&id| self.trace.hop(id).is_some_and(|h| h.to.0 == dev && h.t_rx == t));
        let mut outbound: Vec<HopId> = self.trace.sent_at(t).filter(|h| h.batch == batch && h.from.0 == dev).map(|h| h.id).collect();
        let late = self.trace.late().filter(|l| l.1 == batch && self.trace.hop(l.0).is_some_and(|h| h.from.0 == dev));
        outbound.extend(late.map(|l| l.0));
        outbound.sort_unstable();
        DevStep { dev, batch, t, inbound, decisions, outbound }
    }
}
