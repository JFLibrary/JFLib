// log -- what the IOS-like console prints by itself: interface changes (%LINK-3-UPDOWN,
// %LINK-5-CHANGED, %LINEPROTO-5-UPDOWN), "%SYS-5-CONFIG_I", and debugging output made from
// the device's decisions (debug ip icmp, debug arp, debug spanning-tree events, debug ip rip:
// cli/rip.rs), each line with a time stamp; the texts after the message codes are our own words
//
//   Engine::if_state(ni, iface, port)   (not shut down, link, line protocol) of an interface
//   Engine::ios_snap(ni)                compares every interface with the last look and logs
//                                       what changed (after link notes and every command)
//   Engine::ios_debug(ni, hop, note)    one decision as debug lines; the packet comes
//                                       from the recorded frame (Trace::hop)
//
// Every message and debugging line also goes to the user interface's log as Report::Log
// (the severity of the code, 7 for debugging); the device's own log keeps only what its
// services log (os.rs), as "debug stays local" in tests/svctest wants it.
//
// The state of a port is its link (a shut down far end takes it down too); a subinterface
// needs its port, a VLAN interface a live port in its VLAN, a loopback only "no shutdown".

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::catalog::CliStyle;
use netpkt::arp::{Arp, ArpView, OP_REPLY, OP_REQUEST};
use netpkt::eth::{ET_ARP, ET_IPV4, L2};
use netpkt::icmp::{self, IcmpView};
use netpkt::ipv4::Ipv4View;
use netpkt::{Ipv4, Mac};

use super::fmt::ios_stamp;
use super::{IfSnap, D_ARP, D_ICMP, D_RIP, D_STP};
use crate::driver::Report;
use crate::ip::IfKind;
use crate::kernel::Engine;
use crate::l3::HDLC_IPV4;
use crate::trace::{HopId, Note};

/// the IPv4 packet of a recorded frame (Ethernet, tagged or not, or Cisco HDLC)
fn ip_of(frame: &[u8]) -> Option<&[u8]> {
    if frame.len() > 4 && frame[..4] == HDLC_IPV4 {
        return Some(&frame[4..]);
    }
    let l2 = L2::parse(frame)?;
    if l2.ethertype == ET_IPV4 { Some(l2.payload(frame)) } else { None }
}

fn arp_of(frame: &[u8]) -> Option<Arp> {
    let l2 = L2::parse(frame)?;
    if l2.ethertype != ET_ARP {
        return None;
    }
    ArpView::parse(l2.payload(frame)).map(|a| Arp::read(&a))
}

fn unreach_name(code: u8) -> &'static str {
    match code {
        icmp::UNREACH_NET => "net",
        icmp::UNREACH_HOST => "host",
        icmp::UNREACH_PROTO => "protocol",
        icmp::UNREACH_PORT => "port",
        icmp::UNREACH_FRAG_NEEDED => "fragmentation needed",
        _ => "filtered",
    }
}

impl Engine {
    /// (not shut down, link, line protocol) of an interface of node `ni`
    pub(crate) fn if_state(&self, ni: usize, f: Option<u16>, port: Option<u16>) -> (bool, bool, bool) {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return (false, false, false);
        };
        let port_link = |p: u16| n.ports.get(p as usize).is_some_and(|x| x.link.is_some_and(|li| self.link_up(li)));
        let port_admin = |p: u16| n.ports.get(p as usize).is_some_and(|x| x.admin_up);
        let x = f.and_then(|i| n.ip.as_ref().and_then(|s| s.ifaces.get(i as usize)));
        match (x.map(|x| x.kind), port) {
            (Some(IfKind::Sub { port: p, .. }), _) => {
                let admin = x.is_some_and(|x| x.admin_up);
                (admin, admin && port_admin(p) && port_link(p), f.is_some_and(|i| self.iface_up(ni, i)))
            }
            (_, Some(p)) => (port_admin(p), port_link(p), port_link(p) && n.power),
            (Some(IfKind::Svi(v)), None) => {
                let admin = x.is_some_and(|x| x.admin_up);
                let exists = n.bridge.as_ref().is_some_and(|b| b.vlans.contains_key(&v));
                (admin, admin && exists && n.power, f.is_some_and(|i| self.iface_up(ni, i)))
            }
            (Some(_), None) => {
                let admin = x.is_some_and(|x| x.admin_up) && n.power;
                (admin, admin, admin)
            }
            _ => (false, false, false),
        }
    }

    /// a log line of the IOS console now ("%SYS-5-CONFIG_I: ..."); the user interface's log
    /// gets it with the severity of its code
    pub(crate) fn ios_log(&mut self, ni: usize, msg: &str) {
        let line = format!("{}{}\n", ios_stamp(self.now), msg);
        self.out(ni, &line);
        let (tag, text) = msg.split_once(": ").unwrap_or(("", msg));
        let sev = tag.split('-').nth(1).and_then(|s| s.parse::<u8>().ok()).unwrap_or(6).min(7);
        self.log_report(ni, sev, tag, text);
    }

    /// a console line for the user interface's log as well (Report::Log)
    fn log_report(&mut self, ni: usize, sev: u8, tag: &str, text: &str) {
        let dev = self.dev_of(ni);
        let at = self.now;
        self.cli.reports.push(Report::Log { dev, at, sev, tag: String::from(tag), text: String::from(text) });
    }

    /// logs the interface changes of an IOS device since the last look
    pub(crate) fn ios_snap(&mut self, ni: usize) {
        let ok = match self.nodes.get(ni) {
            Some(Some(n)) => n.power && n.cli.style == CliStyle::Ios,
            _ => false,
        };
        if !ok {
            return;
        }
        let now: Vec<IfSnap> = self
            .ios_ifaces(ni)
            .into_iter()
            .map(|(name, f, port)| {
                let (admin, link, proto) = self.if_state(ni, f, port);
                IfSnap { name, admin, link, proto }
            })
            .collect();
        let old = match self.cdev_mut(ni) {
            Some(c) => core::mem::take(&mut c.snap),
            None => return,
        };
        let mut msgs: Vec<String> = Vec::new();
        for s in &now {
            let o = old.iter().find(|o| o.name == s.name).cloned().unwrap_or(IfSnap { name: s.name.clone(), admin: false, link: false, proto: false });
            if o.admin && !s.admin {
                msgs.push(format!("%LINK-5-CHANGED: {} was shut down", s.name));
            } else if !o.link && s.link {
                msgs.push(format!("%LINK-3-UPDOWN: {} is up", s.name));
            } else if o.link && !s.link && s.admin {
                msgs.push(format!("%LINK-3-UPDOWN: {} is down", s.name));
            }
            if o.proto != s.proto {
                msgs.push(format!("%LINEPROTO-5-UPDOWN: line protocol on {} is {}", s.name, if s.proto { "up" } else { "down" }));
            }
        }
        if let Some(c) = self.cdev_mut(ni) {
            c.snap = now;
        }
        for m in msgs {
            self.ios_log(ni, &m);
        }
    }

    fn log_iface_name(&self, ni: usize, i: u16) -> String {
        self.stack(ni).and_then(|s| s.ifaces.get(i as usize)).map(|f| f.name.clone()).unwrap_or_default()
    }
    fn iface_on_link(&self, ni: usize, ip: Ipv4) -> String {
        self.stack(ni).and_then(|s| s.ifaces.iter().find(|f| !f.hidden && !f.is_loopback() && f.on_link(ip))).map(|f| f.name.clone()).unwrap_or_default()
    }

    /// "ARP: got request ..." once per received request
    fn arp_rcvd(&mut self, ni: usize, hop: HopId, a: &Arp, ifn: &str) -> String {
        let seen = self.cdev(ni).is_some_and(|c| c.rcvd_hop == hop);
        if seen {
            return String::new();
        }
        if let Some(c) = self.cdev_mut(ni) {
            c.rcvd_hop = hop;
        }
        let what = if a.op == OP_REPLY { "reply" } else { "request" };
        format!("ARP: got {} from {} ({}) for {} on {}\n", what, a.spa, a.sha.cisco(), a.tpa, ifn)
    }

    /// debug lines for one decision of an IOS device
    pub(crate) fn ios_debug(&mut self, ni: usize, hop: Option<HopId>, note: &Note) {
        let d = self.cdev(ni).map(|c| c.debug).unwrap_or(0);
        let frame = hop.and_then(|h| self.trace.hop(h)).map(|h| h.bytes.clone());
        let stamp = ios_stamp(self.now);
        let mut text = String::new();
        match note {
            Note::EchoAnswered { to } if d & D_ICMP != 0 => {
                let src = frame.as_deref().and_then(ip_of).and_then(Ipv4View::parse).map(|v| v.dst()).unwrap_or(Ipv4::UNSPECIFIED);
                text = format!("ICMP: sent echo reply {} -> {}\n", src, to);
            }
            Note::IpLocal { proto: 1 } if d & D_ICMP != 0 => {
                if let Some(v) = frame.as_deref().and_then(ip_of).and_then(Ipv4View::parse)
                    && let Some(ic) = IcmpView::parse(v.payload())
                {
                    text = match ic.typ() {
                        icmp::T_ECHO_REPLY => format!("ICMP: got echo reply {} -> {}\n", v.src(), v.dst()),
                        icmp::T_UNREACH => {
                            let qd = ic.quoted().map(|q| q.dst()).unwrap_or(Ipv4::UNSPECIFIED);
                            format!("ICMP: got {} unreachable for {} from {}\n", unreach_name(ic.code()), qd, v.src())
                        }
                        icmp::T_TIME_EXCEEDED => format!("ICMP: got time exceeded from {}\n", v.src()),
                        _ => String::new(),
                    };
                }
            }
            Note::IcmpSent { typ, code, to } if d & D_ICMP != 0 => {
                let od = frame.as_deref().and_then(ip_of).and_then(Ipv4View::parse_truncated).map(|v| v.dst()).unwrap_or(Ipv4::UNSPECIFIED);
                text = match *typ {
                    icmp::T_TIME_EXCEEDED => format!("ICMP: sent time exceeded to {} (packet for {})\n", to, od),
                    icmp::T_UNREACH => format!("ICMP: sent {} unreachable to {} (packet for {})\n", unreach_name(*code), to, od),
                    _ => String::new(),
                };
            }
            Note::ArpRequest { iface, ip } if d & D_ARP != 0 => {
                let (mac, src) = self.stack(ni).and_then(|s| s.ifaces.get(*iface as usize)).map(|f| (f.mac, f.primary().unwrap_or(Ipv4::UNSPECIFIED))).unwrap_or((Mac::ZERO, Ipv4::UNSPECIFIED));
                text = format!("ARP: sent request for {} from {} ({}) on {}\n", ip, src, mac.cisco(), self.log_iface_name(ni, *iface));
            }
            Note::ArpReply { iface, ip, mac } if d & D_ARP != 0 => {
                let ifn = self.log_iface_name(ni, *iface);
                if let (Some(h), Some(a)) = (hop, frame.as_deref().and_then(arp_of)) {
                    text = self.arp_rcvd(ni, h, &a, &ifn);
                    text.push_str(&format!("ARP: sent reply {} is at {} to {} ({}) on {}\n", ip, mac.cisco(), a.spa, a.sha.cisco(), ifn));
                }
            }
            Note::ArpProxied { iface, ip } if d & D_ARP != 0 => {
                let ifn = self.log_iface_name(ni, *iface);
                let mac = self.stack(ni).and_then(|s| s.ifaces.get(*iface as usize)).map(|f| f.mac).unwrap_or(Mac::ZERO);
                if let (Some(h), Some(a)) = (hop, frame.as_deref().and_then(arp_of)) {
                    text = self.arp_rcvd(ni, h, &a, &ifn);
                    text.push_str(&format!("ARP: sent proxy reply {} is at {} to {} ({}) on {}\n", ip, mac.cisco(), a.spa, a.sha.cisco(), ifn));
                }
            }
            Note::ArpLearned { ip, mac } if d & D_ARP != 0 => {
                let ifn = self.iface_on_link(ni, *ip);
                if let (Some(h), Some(a)) = (hop, frame.as_deref().and_then(arp_of))
                    && (a.op == OP_REQUEST || a.op == OP_REPLY)
                {
                    text = self.arp_rcvd(ni, h, &a, &ifn);
                }
                text.push_str(&format!("ARP: learned {} is at {}\n", ip, mac.cisco()));
            }
            // "debug spanning-tree events" (cli/stp.rs)
            n if d & D_STP != 0 && n.is_stp_event() => text = self.stp_debug(ni, n),
            n if d & D_RIP != 0 && n.is_rip() => text = self.rip_debug(ni, n),
            _ => {}
        }
        if text.is_empty() {
            return;
        }
        let mut out = String::new();
        for l in text.lines() {
            if l.starts_with(' ') {
                out.push_str(l);
            } else {
                out.push_str(&stamp);
                out.push_str(l);
            }
            out.push('\n');
        }
        self.out(ni, &out);
        // the user interface's log: one debugging line each ("ICMP", "ARP" as their source)
        for l in text.lines().filter(|l| !l.starts_with(' ')) {
            let (tag, rest) = l.split_once(": ").unwrap_or(("debug", l));
            self.log_report(ni, 7, tag, rest);
        }
    }
}
