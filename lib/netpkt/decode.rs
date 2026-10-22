// decode -- dissects a frame into layers for packet inspectors and event filters
//
//   stack(frame) -> Stack          the protocol layers and their header ranges, no allocation
//   proto_mask(frame) -> u32       P_* bits of every layer present (for event filters)
//   stack_ip / ip_mask(packet)     the same for a bare IP packet (serial lines, tunnels)
//   layers(frame) -> Vec<Layer>    per layer: title, fields with byte ranges (for a hex view),
//                                  problems (bad checksum, truncated header, ...)
//   info(frame) -> (Proto, String) the top protocol and a one-line summary like a capture tool
//                                  ("Echo (ping) request id=0x0001, seq=1, ttl=64")
//
// Application protocols are recognised by their well-known ports.  DHCP (fixed fields and
// every option), DNS (header, questions and records, also over TCP), NTP and syslog have
// dissectors; the payload of the others shows as one field.
// BPDUs of STP and RSTP (LLC SAP 0x42) show every field and flag (stp.rs).
// Field names and values follow the usual capture-tool wording (English protocol terms).
// HTTP (ports 80, 8080, 8000, and any TCP segment that starts like a request or a response)
// shows the start line, the header lines and the body bytes of every message the segment
// holds; a segment in the middle of a message is a "Continuation".

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::addr::{Ipv4, Ipv6, Mac};
use crate::arp::{ArpView, OP_REPLY, OP_REQUEST};
use crate::checksum;
use crate::dhcp::{self, DhcpView};
use crate::dns::{self, Message, RData};
use crate::eth::{self, EthView, Llc, ETH_HDR, VLAN_TAG};
use crate::icmp::{self, IcmpView};
use crate::ipv4::{self, Ipv4View};
use crate::ipv6::{self, Ipv6View};
use crate::ntp::{self, NtpView};
use crate::syslog;
use crate::tcp::{self, TcpView};
use crate::udp::{self, UdpView};
use crate::be16;
use crate::http;

pub const P_ETH: u32 = 1 << 0;
pub const P_VLAN: u32 = 1 << 1;
pub const P_LLC: u32 = 1 << 2;
pub const P_ARP: u32 = 1 << 3;
pub const P_IPV4: u32 = 1 << 4;
pub const P_IPV6: u32 = 1 << 5;
pub const P_ICMP: u32 = 1 << 6;
pub const P_ICMPV6: u32 = 1 << 7;
pub const P_IGMP: u32 = 1 << 8;
pub const P_UDP: u32 = 1 << 9;
pub const P_TCP: u32 = 1 << 10;
pub const P_DHCP: u32 = 1 << 11;
pub const P_DNS: u32 = 1 << 12;
pub const P_HTTP: u32 = 1 << 13;
pub const P_TLS: u32 = 1 << 14;
pub const P_SSH: u32 = 1 << 15;
pub const P_STP: u32 = 1 << 16;
pub const P_RIP: u32 = 1 << 17;
pub const P_OSPF: u32 = 1 << 18;
pub const P_NTP: u32 = 1 << 19;
pub const P_SYSLOG: u32 = 1 << 20;
pub const P_TFTP: u32 = 1 << 21;
pub const P_SNMP: u32 = 1 << 22;
pub const P_LLDP: u32 = 1 << 23;
pub const P_DHCPV6: u32 = 1 << 24;
pub const P_DATA: u32 = 1 << 31;

/// a protocol layer
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Proto {
    Eth,
    Vlan,
    Llc,
    Arp,
    Ipv4,
    Ipv6,
    Icmp,
    Icmpv6,
    Igmp,
    Udp,
    Tcp,
    Dhcp,
    Dhcpv6,
    Dns,
    Http,
    Tls,
    Ssh,
    Stp,
    Rip,
    Ospf,
    Ntp,
    Syslog,
    Tftp,
    Snmp,
    Lldp,
    Data,
}

impl Proto {
    pub fn name(self) -> &'static str {
        match self {
            Proto::Eth => "Ethernet",
            Proto::Vlan => "802.1Q",
            Proto::Llc => "LLC",
            Proto::Arp => "ARP",
            Proto::Ipv4 => "IPv4",
            Proto::Ipv6 => "IPv6",
            Proto::Icmp => "ICMP",
            Proto::Icmpv6 => "ICMPv6",
            Proto::Igmp => "IGMP",
            Proto::Udp => "UDP",
            Proto::Tcp => "TCP",
            Proto::Dhcp => "DHCP",
            Proto::Dhcpv6 => "DHCPv6",
            Proto::Dns => "DNS",
            Proto::Http => "HTTP",
            Proto::Tls => "TLS",
            Proto::Ssh => "SSH",
            Proto::Stp => "STP",
            Proto::Rip => "RIP",
            Proto::Ospf => "OSPF",
            Proto::Ntp => "NTP",
            Proto::Syslog => "Syslog",
            Proto::Tftp => "TFTP",
            Proto::Snmp => "SNMP",
            Proto::Lldp => "LLDP",
            Proto::Data => "Data",
        }
    }
    pub fn bit(self) -> u32 {
        match self {
            Proto::Eth => P_ETH,
            Proto::Vlan => P_VLAN,
            Proto::Llc => P_LLC,
            Proto::Arp => P_ARP,
            Proto::Ipv4 => P_IPV4,
            Proto::Ipv6 => P_IPV6,
            Proto::Icmp => P_ICMP,
            Proto::Icmpv6 => P_ICMPV6,
            Proto::Igmp => P_IGMP,
            Proto::Udp => P_UDP,
            Proto::Tcp => P_TCP,
            Proto::Dhcp => P_DHCP,
            Proto::Dhcpv6 => P_DHCPV6,
            Proto::Dns => P_DNS,
            Proto::Http => P_HTTP,
            Proto::Tls => P_TLS,
            Proto::Ssh => P_SSH,
            Proto::Stp => P_STP,
            Proto::Rip => P_RIP,
            Proto::Ospf => P_OSPF,
            Proto::Ntp => P_NTP,
            Proto::Syslog => P_SYSLOG,
            Proto::Tftp => P_TFTP,
            Proto::Snmp => P_SNMP,
            Proto::Lldp => P_LLDP,
            Proto::Data => P_DATA,
        }
    }
}

/// the layers of a frame: protocol, header offset and header length (the application layer
/// and Data cover their payload)
#[derive(Clone, Copy, Debug)]
pub struct Stack {
    pub n: usize,
    pub l: [(Proto, u32, u32); 10],
    /// bytes after the network packet (Ethernet padding)
    pub trailer: u32,
}

impl Stack {
    fn push(&mut self, p: Proto, off: usize, len: usize) {
        if self.n < self.l.len() {
            self.l[self.n] = (p, off as u32, len as u32);
            self.n += 1;
        }
    }
    pub fn layers(&self) -> &[(Proto, u32, u32)] {
        &self.l[..self.n]
    }
    pub fn top(&self) -> Proto {
        if self.n == 0 { Proto::Data } else { self.l[self.n - 1].0 }
    }
    pub fn mask(&self) -> u32 {
        self.layers().iter().fold(0, |m, l| m | l.0.bit())
    }
    pub fn find(&self, p: Proto) -> Option<(usize, usize)> {
        self.layers().iter().find(|l| l.0 == p).map(|l| (l.1 as usize, l.2 as usize))
    }
}

fn udp_app(sp: u16, dp: u16) -> Option<Proto> {
    let pick = |p: u16| -> Option<Proto> {
        Some(match p {
            udp::PORT_DHCP_SERVER | udp::PORT_DHCP_CLIENT => Proto::Dhcp,
            udp::PORT_DHCPV6_CLIENT | udp::PORT_DHCPV6_SERVER => Proto::Dhcpv6,
            udp::PORT_DNS | 5353 => Proto::Dns,
            udp::PORT_NTP => Proto::Ntp,
            udp::PORT_SYSLOG => Proto::Syslog,
            udp::PORT_RIP => Proto::Rip,
            udp::PORT_TFTP => Proto::Tftp,
            udp::PORT_SNMP | 162 => Proto::Snmp,
            _ => return None,
        })
    };
    pick(dp).or_else(|| pick(sp))
}

fn tcp_app(sp: u16, dp: u16) -> Option<Proto> {
    let pick = |p: u16| -> Option<Proto> {
        Some(match p {
            80 | 8080 | 8000 => Proto::Http,
            443 | 8443 => Proto::Tls,
            22 => Proto::Ssh,
            53 => Proto::Dns,
            _ => return None,
        })
    };
    pick(dp).or_else(|| pick(sp))
}

/// the layers of a frame (header ranges); stops at the first malformed header
pub fn stack(frame: &[u8]) -> Stack {
    let mut s = Stack { n: 0, l: [(Proto::Data, 0, 0); 10], trailer: 0 };
    let Some(e) = EthView::parse(frame) else {
        if !frame.is_empty() {
            s.push(Proto::Data, 0, frame.len());
        }
        return s;
    };
    s.push(Proto::Eth, 0, ETH_HDR);
    let mut et = e.ethertype();
    let mut off = ETH_HDR;
    if et == eth::ET_VLAN && frame.len() >= ETH_HDR + VLAN_TAG {
        s.push(Proto::Vlan, 12, 6);
        et = be16(frame, 16);
        off += VLAN_TAG;
    }
    let rest = &frame[off..];
    if et <= 1500 {
        if let Some(llc) = Llc::parse(rest) {
            s.push(Proto::Llc, off, Llc::LEN);
            let body = (et as usize).min(rest.len());
            if llc.dsap == eth::SAP_STP && body > Llc::LEN {
                s.push(Proto::Stp, off + Llc::LEN, body - Llc::LEN);
            } else if body > Llc::LEN {
                s.push(Proto::Data, off + Llc::LEN, body - Llc::LEN);
            }
            s.trailer = (rest.len() - body) as u32;
        }
        return s;
    }
    match et {
        eth::ET_ARP => {
            if ArpView::parse(rest).is_some() {
                s.push(Proto::Arp, off, crate::arp::ARP_LEN);
                s.trailer = (rest.len() - crate::arp::ARP_LEN) as u32;
            } else {
                s.push(Proto::Data, off, rest.len());
            }
        }
        eth::ET_IPV4 => {
            let Some(v) = Ipv4View::parse(rest) else {
                s.push(Proto::Data, off, rest.len());
                return s;
            };
            s.push(Proto::Ipv4, off, v.header_len());
            s.trailer = (rest.len() - v.total_len()) as u32;
            let poff = off + v.header_len();
            if v.frag_off() > 0 {
                if !v.payload().is_empty() {
                    s.push(Proto::Data, poff, v.payload().len());
                }
                return s;
            }
            transport(&mut s, v.proto(), v.payload(), poff);
        }
        eth::ET_IPV6 => {
            let Some(v) = Ipv6View::parse(rest) else {
                s.push(Proto::Data, off, rest.len());
                return s;
            };
            s.push(Proto::Ipv6, off, ipv6::IPV6_HDR);
            s.trailer = (rest.len() - v.packet().len()) as u32;
            let Some((nh, ext)) = ipv6::upper_layer(&v) else {
                return s;
            };
            let poff = off + ipv6::IPV6_HDR + ext;
            transport(&mut s, nh, &v.payload()[ext..], poff);
        }
        eth::ET_LLDP => s.push(Proto::Lldp, off, rest.len()),
        _ => {
            if !rest.is_empty() {
                s.push(Proto::Data, off, rest.len());
            }
        }
    }
    s
}

fn transport(s: &mut Stack, proto: u8, p: &[u8], off: usize) {
    match proto {
        ipv4::PROTO_ICMP => {
            if p.len() >= icmp::ICMP_HDR {
                s.push(Proto::Icmp, off, p.len());
            }
        }
        ipv4::PROTO_ICMPV6 => {
            if p.len() >= 4 {
                s.push(Proto::Icmpv6, off, p.len());
            }
        }
        ipv4::PROTO_IGMP => s.push(Proto::Igmp, off, p.len()),
        ipv4::PROTO_OSPF => s.push(Proto::Ospf, off, p.len()),
        ipv4::PROTO_UDP => {
            let Some(u) = UdpView::parse(p) else {
                s.push(Proto::Data, off, p.len());
                return;
            };
            s.push(Proto::Udp, off, udp::UDP_HDR);
            let pl = u.payload().len();
            if pl > 0 {
                let app = udp_app(u.src_port(), u.dst_port()).unwrap_or(Proto::Data);
                s.push(app, off + udp::UDP_HDR, pl);
            }
        }
        ipv4::PROTO_TCP => {
            let Some(t) = TcpView::parse(p) else {
                s.push(Proto::Data, off, p.len());
                return;
            };
            s.push(Proto::Tcp, off, t.header_len());
            let pl = t.payload().len();
            if pl > 0 {
                // HTTP on any port when the segment starts like a message
                let app = tcp_app(t.src_port(), t.dst_port()).or_else(|| http::start_kind(t.payload()).map(|_| Proto::Http)).unwrap_or(Proto::Data);
                s.push(app, off + t.header_len(), pl);
            }
        }
        _ => {
            if !p.is_empty() {
                s.push(Proto::Data, off, p.len());
            }
        }
    }
}

/// the P_* bits of every layer of a frame
pub fn proto_mask(frame: &[u8]) -> u32 {
    stack(frame).mask()
}

/// the layers of a bare IP packet (no link header: serial lines, tunnels); offsets are
/// relative to the packet
pub fn stack_ip(packet: &[u8]) -> Stack {
    let mut s = Stack { n: 0, l: [(Proto::Data, 0, 0); 10], trailer: 0 };
    match packet.first().map(|b| b >> 4) {
        Some(4) => {
            if let Some(v) = Ipv4View::parse(packet) {
                s.push(Proto::Ipv4, 0, v.header_len());
                s.trailer = (packet.len() - v.total_len()) as u32;
                if v.frag_off() > 0 {
                    if !v.payload().is_empty() {
                        s.push(Proto::Data, v.header_len(), v.payload().len());
                    }
                } else {
                    transport(&mut s, v.proto(), v.payload(), v.header_len());
                }
            }
        }
        Some(6) => {
            if let Some(v) = Ipv6View::parse(packet) {
                s.push(Proto::Ipv6, 0, ipv6::IPV6_HDR);
                s.trailer = (packet.len() - v.packet().len()) as u32;
                if let Some((nh, ext)) = ipv6::upper_layer(&v) {
                    transport(&mut s, nh, &v.payload()[ext..], ipv6::IPV6_HDR + ext);
                }
            }
        }
        _ => {
            if !packet.is_empty() {
                s.push(Proto::Data, 0, packet.len());
            }
        }
    }
    s
}

/// the P_* bits of a bare IP packet
pub fn ip_mask(packet: &[u8]) -> u32 {
    stack_ip(packet).mask()
}

// ---------------------------------------------------------------- full dissection

/// one header field: name, shown value, byte range in the frame
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub value: String,
    pub off: u32,
    pub len: u32,
}

/// one layer of a dissected frame
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    pub proto: Proto,
    /// header range in the frame
    pub off: u32,
    pub len: u32,
    /// "Internet Protocol Version 4, Src: 10.0.0.1, Dst: 10.0.0.2"
    pub title: String,
    pub fields: Vec<Field>,
    /// "bad header checksum (should be 0x1c46)", ...
    pub problems: Vec<String>,
}

struct B {
    l: Layer,
}

impl B {
    fn new(proto: Proto, off: usize, len: usize, title: String) -> B {
        B { l: Layer { proto, off: off as u32, len: len as u32, title, fields: Vec::new(), problems: Vec::new() } }
    }
    fn f(&mut self, name: &'static str, off: usize, len: usize, value: String) {
        self.l.fields.push(Field { name, value, off: off as u32, len: len as u32 });
    }
    fn bad(&mut self, s: String) {
        self.l.problems.push(s);
    }
}

#[derive(Clone, Copy)]
enum Net {
    None,
    V4(Ipv4, Ipv4),
    V6(Ipv6, Ipv6),
}

fn ck_str(stored: u16, ok: bool, want: Option<u16>) -> String {
    match (ok, want) {
        (true, _) => format!("0x{:04x} [correct]", stored),
        (false, Some(w)) => format!("0x{:04x} [incorrect, should be 0x{:04x}]", stored, w),
        (false, None) => format!("0x{:04x} [incorrect]", stored),
    }
}

/// every layer of a frame with its fields
pub fn layers(frame: &[u8]) -> Vec<Layer> {
    let st = stack(frame);
    let mut out: Vec<Layer> = Vec::new();
    let mut net = Net::None;
    // the end of the network packet: TCP has no length of its own
    let ip_end = frame.len() - st.trailer as usize;
    for &(p, off, len) in st.layers() {
        let (off, len) = (off as usize, len as usize);
        let b = match p {
            Proto::Eth => Some(eth_layer(frame, &st)),
            Proto::Vlan => Some(vlan_layer(frame)),
            Proto::Llc => Some(llc_layer(frame, off)),
            Proto::Arp => Some(arp_layer(frame, off)),
            Proto::Ipv4 => {
                let v = Ipv4View::parse(&frame[off..]);
                if let Some(v) = v {
                    net = Net::V4(v.src(), v.dst());
                }
                v.map(|v| ipv4_layer(&v, off))
            }
            Proto::Ipv6 => {
                let v = Ipv6View::parse(&frame[off..]);
                if let Some(v) = v {
                    net = Net::V6(v.src(), v.dst());
                }
                v.map(|v| ipv6_layer(&v, off))
            }
            Proto::Icmp => IcmpView::parse(&frame[off..off + len]).map(|v| icmp_layer(&v, off)),
            Proto::Udp => UdpView::parse(&frame[off..]).map(|v| udp_layer(&v, off, net)),
            Proto::Tcp => TcpView::parse(&frame[off..ip_end]).map(|v| tcp_layer(&v, off, net)),
            Proto::Dhcp => Some(dhcp_layer(frame, off, len)),
            Proto::Dns => Some(dns_layer(frame, off, len, st.find(Proto::Tcp).is_some())),
            Proto::Ntp => Some(ntp_layer(frame, off, len)),
            Proto::Syslog => Some(syslog_layer(frame, off, len)),
            Proto::Http => Some(http_layer(frame, off, len)),
            Proto::Stp => Some(stp_layer(frame, off, len)),
            _ => Some(payload_layer(p, off, len)),
        };
        if let Some(b) = b {
            out.push(b.l);
        }
    }
    out
}

fn eth_layer(frame: &[u8], st: &Stack) -> B {
    let Some(e) = EthView::parse(frame) else {
        return payload_layer(Proto::Data, 0, frame.len());
    };
    let mut b = B::new(Proto::Eth, 0, ETH_HDR, format!("Ethernet II, Src: {}, Dst: {}", e.src(), e.dst()));
    b.f("Destination", 0, 6, mac_str(e.dst()));
    b.f("Source", 6, 6, mac_str(e.src()));
    let t = e.ethertype();
    if t <= 1500 {
        b.l.title = format!("IEEE 802.3 Ethernet, Src: {}, Dst: {}", e.src(), e.dst());
        b.f("Length", 12, 2, format!("{}", t));
    } else {
        b.f("Type", 12, 2, type_str(t));
    }
    if st.trailer > 0 {
        let n = st.trailer as usize;
        b.f("Padding", frame.len() - n, n, format!("{} bytes", n));
    }
    b
}

fn mac_str(m: Mac) -> String {
    let mut s = format!("{}", m);
    if m.is_broadcast() {
        s.push_str(" (broadcast)");
    } else if m.is_multicast() {
        s.push_str(" (multicast)");
    }
    s
}

fn type_str(t: u16) -> String {
    match eth::ethertype_name(t) {
        Some(n) => format!("{} (0x{:04x})", n, t),
        None => format!("0x{:04x}", t),
    }
}

fn vlan_layer(frame: &[u8]) -> B {
    let tag = eth::tag_of(frame).unwrap_or_default();
    let mut b = B::new(Proto::Vlan, 12, 6, format!("802.1Q Virtual LAN, PRI: {}, DEI: {}, ID: {}", tag.pcp, tag.dei as u8, tag.vid));
    b.f("TPID", 12, 2, String::from("0x8100"));
    b.f("Priority", 14, 1, format!("{}", tag.pcp));
    b.f("DEI", 14, 1, format!("{}", tag.dei as u8));
    b.f("ID", 14, 2, format!("{}", tag.vid));
    b.f("Type", 16, 2, type_str(be16(frame, 16)));
    b
}

fn llc_layer(frame: &[u8], off: usize) -> B {
    let l = Llc::parse(&frame[off..]).unwrap_or(Llc { dsap: 0, ssap: 0, control: 0 });
    let mut b = B::new(Proto::Llc, off, Llc::LEN, String::from("Logical-Link Control"));
    let sap = |s: u8| if s == eth::SAP_STP { String::from("Spanning Tree BPDU (0x42)") } else { format!("0x{:02x}", s) };
    b.f("DSAP", off, 1, sap(l.dsap));
    b.f("SSAP", off + 1, 1, sap(l.ssap));
    b.f("Control field", off + 2, 1, format!("0x{:02x}", l.control));
    b
}

fn arp_layer(frame: &[u8], off: usize) -> B {
    let v = ArpView::parse(&frame[off..]);
    let Some(v) = v else {
        return payload_layer(Proto::Arp, off, frame.len() - off);
    };
    let what = match v.op() {
        OP_REQUEST if v.spa().is_unspecified() => "request, probe",
        OP_REQUEST if v.spa() == v.tpa() => "request, gratuitous",
        OP_REQUEST => "request",
        OP_REPLY => "reply",
        _ => "unknown opcode",
    };
    let mut b = B::new(Proto::Arp, off, crate::arp::ARP_LEN, format!("Address Resolution Protocol ({})", what));
    b.f("Hardware type", off, 2, String::from("Ethernet (1)"));
    b.f("Protocol type", off + 2, 2, String::from("IPv4 (0x0800)"));
    b.f("Hardware size", off + 4, 1, String::from("6"));
    b.f("Protocol size", off + 5, 1, String::from("4"));
    let op = match v.op() {
        OP_REQUEST => String::from("request (1)"),
        OP_REPLY => String::from("reply (2)"),
        o => format!("{}", o),
    };
    b.f("Opcode", off + 6, 2, op);
    b.f("Sender MAC address", off + 8, 6, format!("{}", v.sha()));
    b.f("Sender IP address", off + 14, 4, format!("{}", v.spa()));
    b.f("Target MAC address", off + 18, 6, format!("{}", v.tha()));
    b.f("Target IP address", off + 24, 4, format!("{}", v.tpa()));
    b
}

fn ipv4_layer(v: &Ipv4View, off: usize) -> B {
    let hl = v.header_len();
    let mut b = B::new(Proto::Ipv4, off, hl, format!("Internet Protocol Version 4, Src: {}, Dst: {}", v.src(), v.dst()));
    b.f("Version", off, 1, String::from("4"));
    b.f("Header Length", off, 1, format!("{} bytes ({})", hl, hl / 4));
    b.f("Differentiated Services", off + 1, 1, format!("DSCP {}, ECN {}", v.dscp(), v.ecn()));
    b.f("Total Length", off + 2, 2, format!("{}", v.total_len()));
    b.f("Identification", off + 4, 2, format!("0x{:04x} ({})", v.id(), v.id()));
    let mut fl = String::new();
    if v.df() {
        fl.push_str("Don't fragment");
    }
    if v.mf() {
        if !fl.is_empty() {
            fl.push_str(", ");
        }
        fl.push_str("More fragments");
    }
    if fl.is_empty() {
        fl.push_str("none");
    }
    b.f("Flags", off + 6, 1, fl);
    b.f("Fragment Offset", off + 6, 2, format!("{}", v.frag_off()));
    b.f("Time to Live", off + 8, 1, format!("{}", v.ttl()));
    let pn = match ipv4::proto_name(v.proto()) {
        Some(n) => format!("{} ({})", n, v.proto()),
        None => format!("{}", v.proto()),
    };
    b.f("Protocol", off + 9, 1, pn);
    let ok = v.checksum_ok();
    let want = if ok {
        None
    } else {
        let mut h = v.header().to_vec();
        h[10] = 0;
        h[11] = 0;
        Some(checksum::checksum(&h))
    };
    b.f("Header Checksum", off + 10, 2, ck_str(v.checksum(), ok, want));
    if !ok {
        b.bad(format!("bad header checksum (should be 0x{:04x})", want.unwrap_or(0)));
    }
    b.f("Source Address", off + 12, 4, format!("{}", v.src()));
    b.f("Destination Address", off + 16, 4, format!("{}", v.dst()));
    if hl > ipv4::IPV4_MIN {
        b.f("Options", off + ipv4::IPV4_MIN, hl - ipv4::IPV4_MIN, format!("{} bytes", hl - ipv4::IPV4_MIN));
    }
    if v.ttl() == 0 {
        b.bad(String::from("TTL is 0"));
    }
    b
}

fn ipv6_layer(v: &Ipv6View, off: usize) -> B {
    let mut b = B::new(Proto::Ipv6, off, ipv6::IPV6_HDR, format!("Internet Protocol Version 6, Src: {}, Dst: {}", v.src(), v.dst()));
    b.f("Version", off, 1, String::from("6"));
    b.f("Traffic Class", off, 2, format!("0x{:02x}", v.traffic_class()));
    b.f("Flow Label", off + 1, 3, format!("0x{:05x}", v.flow_label()));
    b.f("Payload Length", off + 4, 2, format!("{}", v.payload_len()));
    let nh = v.next_header();
    let nn = match ipv4::proto_name(nh) {
        Some(n) => format!("{} ({})", n, nh),
        None => format!("{}", nh),
    };
    b.f("Next Header", off + 6, 1, nn);
    b.f("Hop Limit", off + 7, 1, format!("{}", v.hop_limit()));
    b.f("Source Address", off + 8, 16, format!("{}", v.src()));
    b.f("Destination Address", off + 24, 16, format!("{}", v.dst()));
    b
}

fn icmp_layer(v: &IcmpView, off: usize) -> B {
    let (t, c) = (v.typ(), v.code());
    let mut b = B::new(Proto::Icmp, off, v.payload().len() + icmp::ICMP_HDR, String::from("Internet Control Message Protocol"));
    b.f("Type", off, 1, format!("{} ({})", t, icmp::name(t, c)));
    b.f("Code", off + 1, 1, format!("{}", c));
    let ok = v.checksum_ok();
    b.f("Checksum", off + 2, 2, ck_str(v.checksum(), ok, None));
    if !ok {
        b.bad(String::from("bad ICMP checksum"));
    }
    match t {
        icmp::T_ECHO_REQUEST | icmp::T_ECHO_REPLY => {
            b.f("Identifier", off + 4, 2, format!("0x{:04x} ({})", v.id(), v.id()));
            b.f("Sequence Number", off + 6, 2, format!("{}", v.seq()));
            if !v.payload().is_empty() {
                b.f("Data", off + 8, v.payload().len(), format!("{} bytes", v.payload().len()));
            }
        }
        icmp::T_UNREACH if c == icmp::UNREACH_FRAG_NEEDED => {
            b.f("MTU of next hop", off + 6, 2, format!("{}", be16(&v.rest(), 2)));
        }
        icmp::T_REDIRECT => b.f("Gateway Address", off + 4, 4, format!("{}", Ipv4(v.rest()))),
        _ => {}
    }
    if let Some(q) = v.quoted() {
        let n = v.payload().len();
        b.f("Quoted packet", off + 8, n, format!("{} -> {}, {}", q.src(), q.dst(), ipv4::proto_name(q.proto()).unwrap_or("?")));
    }
    b
}

fn udp_layer(v: &UdpView, off: usize, net: Net) -> B {
    let mut b = B::new(Proto::Udp, off, udp::UDP_HDR, format!("User Datagram Protocol, Src Port: {}, Dst Port: {}", v.src_port(), v.dst_port()));
    b.f("Source Port", off, 2, port_str(v.src_port(), udp::port_name(v.src_port())));
    b.f("Destination Port", off + 2, 2, port_str(v.dst_port(), udp::port_name(v.dst_port())));
    b.f("Length", off + 4, 2, format!("{}", v.len()));
    let (ok, absent) = match net {
        Net::V4(s, d) => (v.checksum_ok_v4(s, d), v.checksum() == 0),
        Net::V6(s, d) => (v.checksum_ok_v6(s, d), false),
        Net::None => (true, false),
    };
    if absent {
        b.f("Checksum", off + 6, 2, String::from("0x0000 [none]"));
    } else {
        b.f("Checksum", off + 6, 2, ck_str(v.checksum(), ok, None));
        if !ok {
            b.bad(String::from("bad UDP checksum"));
        }
    }
    b
}

fn tcp_layer(v: &TcpView, off: usize, net: Net) -> B {
    let hl = v.header_len();
    let mut b = B::new(
        Proto::Tcp,
        off,
        hl,
        format!("Transmission Control Protocol, Src Port: {}, Dst Port: {}, Seq: {}, Ack: {}, Len: {}", v.src_port(), v.dst_port(), v.seq(), v.ack(), v.payload().len()),
    );
    b.f("Source Port", off, 2, port_str(v.src_port(), tcp::port_name(v.src_port())));
    b.f("Destination Port", off + 2, 2, port_str(v.dst_port(), tcp::port_name(v.dst_port())));
    b.f("Sequence Number", off + 4, 4, format!("{}", v.seq()));
    b.f("Acknowledgment Number", off + 8, 4, format!("{}", v.ack()));
    b.f("Header Length", off + 12, 1, format!("{} bytes ({})", hl, hl / 4));
    b.f("Flags", off + 13, 1, format!("0x{:03x} ({})", v.flags(), tcp::flags_str(v.flags())));
    b.f("Window", off + 14, 2, format!("{}", v.window()));
    let ok = match net {
        Net::V4(s, d) => v.checksum_ok_v4(s, d),
        Net::V6(s, d) => v.checksum_ok_v6(s, d),
        Net::None => true,
    };
    b.f("Checksum", off + 16, 2, ck_str(v.checksum(), ok, None));
    if !ok {
        b.bad(String::from("bad TCP checksum"));
    }
    b.f("Urgent Pointer", off + 18, 2, format!("{}", v.urgent()));
    if hl > tcp::TCP_MIN {
        b.f("Options", off + tcp::TCP_MIN, hl - tcp::TCP_MIN, opts_str(&v.opts()));
    }
    b
}

fn opts_str(o: &tcp::TcpOpts) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(m) = o.mss {
        parts.push(format!("MSS {}", m));
    }
    if o.sack_ok {
        parts.push(String::from("SACK permitted"));
    }
    if let Some((v, e)) = o.ts {
        parts.push(format!("TS val {} ecr {}", v, e));
    }
    if let Some(w) = o.wscale {
        parts.push(format!("WS {}", w));
    }
    for &(l, r) in &o.sack[..o.sack_n as usize] {
        parts.push(format!("SACK {}-{}", l, r));
    }
    if parts.is_empty() { String::from("none") } else { parts.join(", ") }
}

fn port_str(p: u16, name: Option<&str>) -> String {
    match name {
        Some(n) => format!("{} ({})", p, n),
        None => format!("{}", p),
    }
}

fn payload_layer(p: Proto, off: usize, len: usize) -> B {
    let title = match p {
        Proto::Data => format!("Data ({} bytes)", len),
        Proto::Dhcp => String::from("Dynamic Host Configuration Protocol"),
        Proto::Dhcpv6 => String::from("DHCPv6"),
        Proto::Dns => String::from("Domain Name System"),
        Proto::Http => String::from("Hypertext Transfer Protocol"),
        Proto::Tls => String::from("Transport Layer Security"),
        Proto::Ssh => String::from("SSH Protocol"),
        Proto::Stp => String::from("Spanning Tree Protocol"),
        Proto::Rip => String::from("Routing Information Protocol"),
        Proto::Ospf => String::from("Open Shortest Path First"),
        Proto::Ntp => String::from("Network Time Protocol"),
        Proto::Syslog => String::from("Syslog message"),
        Proto::Tftp => String::from("Trivial File Transfer Protocol"),
        Proto::Snmp => String::from("Simple Network Management Protocol"),
        Proto::Lldp => String::from("Link Layer Discovery Protocol"),
        Proto::Igmp => String::from("Internet Group Management Protocol"),
        Proto::Icmpv6 => String::from("Internet Control Message Protocol v6"),
        other => String::from(other.name()),
    };
    let mut b = B::new(p, off, len, title);
    b.f("Data", off, len, format!("{} bytes", len));
    b
}

// ---------------------------------------------------------------- one-line summaries

/// the top protocol of a frame and a one-line summary
pub fn info(frame: &[u8]) -> (Proto, String) {
    let st = stack(frame);
    let top = st.top();
    let mut ttl = None;
    let mut net = Net::None;
    if let Some((o, _)) = st.find(Proto::Ipv4)
        && let Some(v) = Ipv4View::parse(&frame[o..])
    {
        ttl = Some(v.ttl());
        net = Net::V4(v.src(), v.dst());
        if v.frag_off() > 0 {
            return (Proto::Ipv4, format!("Fragmented IP protocol (proto={}, off={}, ID={:04x})", ipv4::proto_name(v.proto()).unwrap_or("?"), v.frag_off(), v.id()));
        }
    }
    if let Some((o, _)) = st.find(Proto::Ipv6)
        && let Some(v) = Ipv6View::parse(&frame[o..])
    {
        ttl = Some(v.hop_limit());
        net = Net::V6(v.src(), v.dst());
    }
    let s = match top {
        Proto::Arp => {
            let (o, _) = st.find(Proto::Arp).unwrap_or((0, 0));
            match ArpView::parse(&frame[o..]) {
                Some(a) if a.op() == OP_REQUEST && a.spa().is_unspecified() => format!("Who has {}? (ARP Probe)", a.tpa()),
                Some(a) if a.op() == OP_REQUEST && a.spa() == a.tpa() => format!("Gratuitous ARP for {} (Request)", a.spa()),
                Some(a) if a.op() == OP_REQUEST => format!("Who has {}? Tell {}", a.tpa(), a.spa()),
                Some(a) if a.op() == OP_REPLY => format!("{} is at {}", a.spa(), a.sha()),
                _ => String::from("ARP"),
            }
        }
        Proto::Icmp => {
            let (o, l) = st.find(Proto::Icmp).unwrap_or((0, 0));
            match IcmpView::parse(&frame[o..o + l]) {
                Some(v) if v.typ() == icmp::T_ECHO_REQUEST || v.typ() == icmp::T_ECHO_REPLY => {
                    format!("{} id=0x{:04x}, seq={}, ttl={}", icmp::name(v.typ(), v.code()), v.id(), v.seq(), ttl.unwrap_or(0))
                }
                Some(v) => String::from(icmp::name(v.typ(), v.code())),
                None => String::from("ICMP"),
            }
        }
        _ => {
            if let Some(a) = app_info(&st, frame) {
                a
            } else if let Some((o, _)) = st.find(Proto::Tcp) {
                // the segment ends with the network packet (Ethernet padding is no payload)
                tcp_info(&frame[o..(frame.len() - st.trailer as usize).max(o)], net)
            } else if let Some((o, _)) = st.find(Proto::Udp) {
                match UdpView::parse(&frame[o..]) {
                    Some(u) => format!("{} \u{2192} {} Len={}", u.src_port(), u.dst_port(), u.payload().len()),
                    None => String::from("UDP"),
                }
            } else {
                match top {
                    Proto::Stp => String::from("Spanning Tree BPDU"),
                    Proto::Data => format!("{} bytes", frame.len()),
                    p => String::from(p.name()),
                }
            }
        }
    };
    (top, s)
}

fn tcp_info(b: &[u8], _net: Net) -> String {
    let Some(t) = TcpView::parse(b) else {
        return String::from("TCP");
    };
    let mut s = format!("{} \u{2192} {} [{}] Seq={}", t.src_port(), t.dst_port(), tcp::flags_str(t.flags()), t.seq());
    if t.has(tcp::ACK) {
        s.push_str(&format!(" Ack={}", t.ack()));
    }
    s.push_str(&format!(" Win={} Len={}", t.window(), t.payload().len()));
    let o = t.opts();
    if let Some(m) = o.mss {
        s.push_str(&format!(" MSS={}", m));
    }
    if let Some(w) = o.wscale {
        s.push_str(&format!(" WS={}", 1u32 << w));
    }
    if o.sack_ok {
        s.push_str(" SACK_PERM");
    }
    if let Some((v, e)) = o.ts {
        s.push_str(&format!(" TSval={} TSecr={}", v, e));
    }
    s
}

// ---------------------------------------------------------------- application protocols

/// the summary of an application layer that has a dissector (DHCP, DNS, NTP, syslog)
fn app_info(st: &Stack, frame: &[u8]) -> Option<String> {
    let (o, l) = st.find(st.top())?;
    let b = &frame[o..o + l];
    match st.top() {
        Proto::Dhcp => {
            let v = DhcpView::parse(b)?;
            Some(match v.msg_type() {
                Some(t) => format!("DHCP {} - Transaction ID 0x{:x}", t.name(), v.xid()),
                None => format!("BOOTP Boot {} - Transaction ID 0x{:x}", if v.op() == dhcp::OP_REPLY { "Reply" } else { "Request" }, v.xid()),
            })
        }
        Proto::Dns => {
            let b = if st.find(Proto::Tcp).is_some() { b.get(2..)? } else { b };
            let m = Message::parse(b).ok()?;
            Some(dns_info(&m))
        }
        Proto::Ntp => NtpView::parse(b).map(|v| format!("NTP Version {}, {}", v.vn(), ntp::mode_name(v.mode()))),
        Proto::Syslog => syslog::parse(b).map(|m| format!("{}.{}: {}", syslog::facility_name(syslog::facility_of(m.pri)).to_ascii_uppercase(), syslog::severity_name(syslog::severity_of(m.pri)).to_ascii_uppercase(), m.rest)),
        Proto::Http => Some(http_info(b)),
        Proto::Stp => crate::stp::Bpdu::parse(b).ok().map(|x| crate::stp::summary(&x)),
        _ => None,
    }
}

fn rcode_text(r: u8) -> &'static str {
    match r {
        dns::RCODE_NOERROR => "No error",
        dns::RCODE_FORMERR => "Format error",
        dns::RCODE_SERVFAIL => "Server failure",
        dns::RCODE_NXDOMAIN => "No such name",
        dns::RCODE_NOTIMP => "Not implemented",
        dns::RCODE_REFUSED => "Refused",
        _ => "Unknown error",
    }
}

fn dns_info(m: &Message) -> String {
    let h = &m.header;
    let what = if h.opcode == dns::OPCODE_QUERY { String::from("Standard query") } else { format!("Opcode {}", h.opcode) };
    let mut s = if h.qr { format!("{} response 0x{:04x}", what, h.id) } else { format!("{} 0x{:04x}", what, h.id) };
    if h.qr && h.rcode != dns::RCODE_NOERROR {
        s.push(' ');
        s.push_str(rcode_text(h.rcode));
    }
    for q in &m.questions {
        s.push_str(&format!(" {} {}", dns::type_name(q.qtype), dn(&q.name)));
    }
    for r in &m.answers {
        s.push_str(&format!(" {} {}", dns::type_name(r.rtype), r.data.text()));
    }
    if m.answers.is_empty()
        && let Some(r) = m.authority.iter().find(|r| r.rtype == dns::TYPE_SOA)
        && let RData::Soa(soa) = &r.data
    {
        s.push_str(&format!(" SOA {}", dn(&soa.mname)));
    }
    s
}

/// a name as capture tools show it ("<Root>" for the root)
fn dn(n: &str) -> String {
    if n.is_empty() { String::from("<Root>") } else { String::from(n) }
}

fn hex_short(d: &[u8]) -> String {
    let mut s = String::new();
    for (i, c) in d.iter().take(32).enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{:02x}", c));
    }
    if d.len() > 32 {
        s.push_str(" \u{2026}");
    }
    s
}

fn dhcp_opt_text(code: u8, d: &[u8]) -> String {
    let list = |v: &[Ipv4]| v.iter().map(|a| format!("{}", a)).collect::<Vec<_>>().join(", ");
    let v = match dhcp::Opt::decode(code, d) {
        dhcp::Opt::MsgType(t) => format!("{} ({})", t.name(), t as u8),
        dhcp::Opt::SubnetMask(a) | dhcp::Opt::BroadcastAddr(a) | dhcp::Opt::RequestedIp(a) | dhcp::Opt::ServerId(a) => format!("{}", a),
        dhcp::Opt::Routers(v) | dhcp::Opt::DnsServers(v) | dhcp::Opt::NtpServers(v) => list(&v),
        dhcp::Opt::HostName(s) | dhcp::Opt::DomainName(s) | dhcp::Opt::Message(s) => s,
        dhcp::Opt::LeaseTime(n) | dhcp::Opt::T1(n) | dhcp::Opt::T2(n) => {
            if n == u32::MAX {
                String::from("infinity")
            } else {
                format!("{} s", n)
            }
        }
        dhcp::Opt::Params(p) => p.iter().map(|c| format!("{} {}", c, dhcp::option_name(*c))).collect::<Vec<_>>().join(", "),
        dhcp::Opt::MaxSize(n) => format!("{}", n),
        dhcp::Opt::ClientId(c) => {
            if c.len() == 7 && c[0] == 1 {
                format!("Ethernet {}", Mac(c[1..7].try_into().unwrap_or([0; 6])))
            } else {
                format!("type {}, {}", c[0], hex_short(&c[1..]))
            }
        }
        dhcp::Opt::RelayInfo(subs) => subs
            .iter()
            .map(|(c, v)| match *c {
                dhcp::RELAY_CIRCUIT_ID => format!("circuit-id {}", hex_short(v)),
                dhcp::RELAY_REMOTE_ID => format!("remote-id {}", hex_short(v)),
                _ => format!("sub-option {} {}", c, hex_short(v)),
            })
            .collect::<Vec<_>>()
            .join(", "),
        dhcp::Opt::Other(_, d) => {
            if d.is_empty() {
                String::from("empty")
            } else {
                hex_short(&d)
            }
        }
    };
    format!("({}) {}", code, v)
}

fn dhcp_layer(frame: &[u8], off: usize, len: usize) -> B {
    let b = &frame[off..off + len];
    let Some(v) = DhcpView::parse(b) else {
        let mut l = payload_layer(Proto::Dhcp, off, len);
        l.l.title = String::from("Bootstrap Protocol");
        l.bad(String::from("no DHCP magic cookie or too short"));
        return l;
    };
    let title = match v.msg_type() {
        Some(t) => format!("Dynamic Host Configuration Protocol ({})", t.name()),
        None => String::from("Bootstrap Protocol"),
    };
    let mut l = B::new(Proto::Dhcp, off, len, title);
    let op = match v.op() {
        dhcp::OP_REQUEST => String::from("Boot Request (1)"),
        dhcp::OP_REPLY => String::from("Boot Reply (2)"),
        o => format!("{}", o),
    };
    l.f("Message type", off, 1, op);
    l.f("Hardware type", off + 1, 1, if v.htype() == 1 { String::from("Ethernet (0x01)") } else { format!("0x{:02x}", v.htype()) });
    l.f("Hardware address length", off + 2, 1, format!("{}", v.hlen()));
    l.f("Hops", off + 3, 1, format!("{}", v.hops()));
    l.f("Transaction ID", off + 4, 4, format!("0x{:08x}", v.xid()));
    l.f("Seconds elapsed", off + 8, 2, format!("{}", v.secs()));
    l.f("Bootp flags", off + 10, 2, format!("0x{:04x} ({})", v.flags(), if v.broadcast() { "Broadcast" } else { "Unicast" }));
    l.f("Client IP address", off + 12, 4, format!("{}", v.ciaddr()));
    l.f("Your (client) IP address", off + 16, 4, format!("{}", v.yiaddr()));
    l.f("Next server IP address", off + 20, 4, format!("{}", v.siaddr()));
    l.f("Relay agent IP address", off + 24, 4, format!("{}", v.giaddr()));
    match v.mac() {
        Some(m) => {
            l.f("Client MAC address", off + 28, 6, format!("{}", m));
            l.f("Client hardware address padding", off + 34, 10, hex_short(&b[34..44]));
        }
        None => l.f("Client hardware address", off + 28, 16, hex_short(&b[28..44])),
    }
    let name = |s: &[u8]| if s.is_empty() { String::from("not given") } else { String::from_utf8_lossy(s).into_owned() };
    l.f("Server host name", off + 44, 64, name(v.sname()));
    l.f("Boot file name", off + 108, 128, name(v.file()));
    l.f("Magic cookie", off + dhcp::BOOTP_HDR, 4, String::from("DHCP"));
    let mut end = dhcp::DHCP_MIN;
    for (code, d, p) in v.options() {
        l.f(dhcp::option_name(code), off + p, 2 + d.len(), dhcp_opt_text(code, d));
        if p >= dhcp::DHCP_MIN {
            end = end.max(p + 2 + d.len());
        }
    }
    while end < b.len() && b[end] == dhcp::O_PAD {
        end += 1;
    }
    if end < b.len() && b[end] == dhcp::O_END {
        l.f("End", off + end, 1, String::from("(255)"));
        end += 1;
        if end < b.len() {
            l.f("Padding", off + end, b.len() - end, format!("{} bytes", b.len() - end));
        }
    } else {
        l.bad(String::from("options without end"));
    }
    l
}

/// [start, end) of every question and record (None when the message is cut)
fn dns_spans(b: &[u8]) -> Option<Vec<(usize, usize)>> {
    let mut v = Vec::new();
    let mut p = dns::DNS_HDR;
    let qd = be16(b, 4) as usize;
    let rrs = be16(b, 6) as usize + be16(b, 8) as usize + be16(b, 10) as usize;
    for i in 0..qd + rrs {
        let (_, e) = dns::read_name(b, p)?;
        let e = if i < qd {
            e + 4
        } else {
            let rl = be16(b.get(e..e + 10)?, 8) as usize;
            e + 10 + rl
        };
        if e > b.len() {
            return None;
        }
        v.push((p, e));
        p = e;
    }
    Some(v)
}

fn class_str(c: u16) -> String {
    match c {
        dns::CLASS_IN => String::from("IN"),
        255 => String::from("ANY"),
        _ => format!("CLASS{}", c),
    }
}

fn rr_text(r: &dns::Rr) -> String {
    if r.rtype == dns::TYPE_OPT {
        return format!("{}: type OPT, UDP payload size {}", dn(&r.name), r.class);
    }
    let d = match &r.data {
        RData::A(a) => format!("addr {}", a),
        RData::Aaaa(a) => format!("addr {}", a),
        RData::Ns(n) => format!("ns {}", n),
        RData::Cname(n) => format!("cname {}", n),
        RData::Ptr(n) => format!("domain name {}", n),
        RData::Mx { pref, host } => format!("preference {}, mx {}", pref, host),
        RData::Soa(s) => format!("mname {}", s.mname),
        RData::Txt(_) => format!("txt {}", r.data.text()),
        RData::Other(b) => format!("{} bytes", b.len()),
    };
    format!("{}: type {}, class {}, {}, ttl {}", dn(&r.name), dns::type_name(r.rtype), class_str(r.class), d, r.ttl)
}

fn dns_layer(frame: &[u8], off: usize, len: usize, tcp: bool) -> B {
    let mut l = B::new(Proto::Dns, off, len, String::from("Domain Name System"));
    let mut base = off;
    let mut b = &frame[off..off + len];
    if tcp && b.len() >= 2 {
        l.f("Length", off, 2, format!("{}", be16(b, 0)));
        b = &b[2..];
        base += 2;
    }
    let m = match Message::parse(b) {
        Ok(m) => m,
        Err(e) => {
            l.bad(format!("malformed DNS message ({:?})", e));
            l.f("Data", base, b.len(), format!("{} bytes", b.len()));
            return l;
        }
    };
    let h = &m.header;
    l.l.title = format!("Domain Name System ({})", if h.qr { "response" } else { "query" });
    l.f("Transaction ID", base, 2, format!("0x{:04x}", h.id));
    let what = if h.opcode == dns::OPCODE_QUERY { String::from("Standard query") } else { format!("Opcode {}", h.opcode) };
    let mut fl = format!("0x{:04x} {}", h.flags(), what);
    if h.qr {
        fl.push_str(&format!(" response, {}", rcode_text(h.rcode)));
    }
    l.f("Flags", base + 2, 2, fl);
    l.f("Questions", base + 4, 2, format!("{}", m.questions.len()));
    l.f("Answer RRs", base + 6, 2, format!("{}", m.answers.len()));
    l.f("Authority RRs", base + 8, 2, format!("{}", m.authority.len()));
    l.f("Additional RRs", base + 10, 2, format!("{}", m.additional.len()));
    let spans = dns_spans(b).unwrap_or_default();
    let mut i = 0;
    for q in &m.questions {
        let (s, e) = spans.get(i).copied().unwrap_or((0, 0));
        l.f("Query", base + s, e - s, format!("{}: type {}, class {}", dn(&q.name), dns::type_name(q.qtype), class_str(q.qclass)));
        i += 1;
    }
    for (name, sec) in [("Answer", &m.answers), ("Authority", &m.authority), ("Additional", &m.additional)] {
        for r in sec.iter() {
            let (s, e) = spans.get(i).copied().unwrap_or((0, 0));
            l.f(name, base + s, e - s, rr_text(r));
            i += 1;
        }
    }
    if h.tc {
        l.bad(String::from("truncated: the answer did not fit"));
    }
    l
}

fn short_secs(v: u32) -> String {
    let us = (v as u64 * 1_000_000) >> 16;
    format!("{}.{:06} s", us / 1_000_000, us % 1_000_000)
}

fn ts_str(t: ntp::Ts) -> String {
    if t.0 == 0 {
        return String::from("(0)");
    }
    let ns = t.to_unix_ns();
    let (y, mo, d, h, mi, s) = ntp::civil(ns.div_euclid(1_000_000_000));
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:09} UTC", y, mo, d, h, mi, s, ns.rem_euclid(1_000_000_000))
}

fn ntp_layer(frame: &[u8], off: usize, len: usize) -> B {
    let Some(v) = NtpView::parse(&frame[off..off + len]) else {
        let mut l = payload_layer(Proto::Ntp, off, len);
        l.bad(String::from("shorter than an NTP header"));
        return l;
    };
    let mut l = B::new(Proto::Ntp, off, ntp::NTP_LEN, format!("Network Time Protocol (NTP Version {}, {})", v.vn(), ntp::mode_name(v.mode())));
    let li = match v.li() {
        0 => "no warning",
        1 => "last minute of the day has 61 seconds",
        2 => "last minute of the day has 59 seconds",
        _ => "unknown (clock unsynchronized)",
    };
    l.f("Flags", off, 1, format!("0x{:02x}, Leap Indicator: {}, Version {}, Mode: {}", frame[off], li, v.vn(), ntp::mode_name(v.mode())));
    l.f("Peer Clock Stratum", off + 1, 1, format!("{}", v.stratum()));
    l.f("Peer Polling Interval", off + 2, 1, format!("{}", v.poll()));
    l.f("Peer Clock Precision", off + 3, 1, format!("{}", v.precision()));
    l.f("Root Delay", off + 4, 4, short_secs(v.root_delay()));
    l.f("Root Dispersion", off + 8, 4, short_secs(v.root_disp()));
    let r = v.refid();
    let refid = if v.stratum() <= 1 && r.iter().all(|c| *c == 0 || c.is_ascii_uppercase()) {
        String::from_utf8_lossy(&r[..r.iter().position(|c| *c == 0).unwrap_or(4)]).into_owned()
    } else {
        format!("{}", Ipv4(r))
    };
    l.f("Reference ID", off + 12, 4, refid);
    l.f("Reference Timestamp", off + 16, 8, ts_str(v.reference()));
    l.f("Origin Timestamp", off + 24, 8, ts_str(v.origin()));
    l.f("Receive Timestamp", off + 32, 8, ts_str(v.receive()));
    l.f("Transmit Timestamp", off + 40, 8, ts_str(v.transmit()));
    l
}

fn syslog_layer(frame: &[u8], off: usize, len: usize) -> B {
    let b = &frame[off..off + len];
    let Some(m) = syslog::parse(b) else {
        let mut l = payload_layer(Proto::Syslog, off, len);
        l.bad(String::from("no syslog priority"));
        return l;
    };
    let (fac, sev) = (syslog::facility_of(m.pri), syslog::severity_of(m.pri));
    let mut l = B::new(Proto::Syslog, off, len, format!("Syslog message: {}.{}: {}", syslog::facility_name(fac).to_ascii_uppercase(), syslog::severity_name(sev).to_ascii_uppercase(), m.rest));
    let pl = b.iter().position(|c| *c == b'>').map(|p| p + 1).unwrap_or(0);
    l.f("Facility", off, pl, format!("{} ({})", syslog::facility_name(fac), fac));
    l.f("Level", off, pl, format!("{} ({})", syslog::severity_name(sev), sev));
    if let Some(t) = &m.time {
        l.f("Timestamp", off + pl, t.len(), t.clone());
    }
    if let Some(h) = &m.host {
        l.f("Hostname", off + pl + 16, h.len(), h.clone());
    }
    l.f("Message", off + pl, len - pl, m.rest.clone());
    l
}

// ---------------------------------------------------------------- HTTP

/// the field names of header lines the inspector knows by name (others: "Header")
const HTTP_NAMES: [&str; 36] = [
    "Host", "User-Agent", "Accept", "Accept-Encoding", "Accept-Language", "Connection", "Keep-Alive", "Content-Type",
    "Content-Length", "Transfer-Encoding", "Content-Encoding", "Server", "Date", "Location", "Cache-Control", "Expect",
    "Upgrade", "Authorization", "Cookie", "Set-Cookie", "Last-Modified", "ETag", "If-None-Match", "If-Modified-Since",
    "Referer", "Retry-After", "Allow", "Range", "Content-Range", "Vary", "Pragma", "Origin", "Via", "X-Forwarded-For",
    "Trailer", "TE",
];

/// one message (or the start of one) in a segment
struct HttpMsg {
    /// where it starts in the payload
    at: usize,
    request: bool,
    /// the head; Ok(None) while it goes on in the next segment
    head: Result<Option<http::Head>, http::HttpErr>,
    /// the body bytes in this segment: (start, length)
    body: (usize, usize),
    framing: http::Framing,
    /// the message ends in this segment
    complete: bool,
}

/// the messages a TCP payload holds, one after the other; the walk stops at a message that
/// goes on in the next segment; Err(()) when the payload does not start like a message
fn http_walk(b: &[u8]) -> Result<Vec<HttpMsg>, ()> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < b.len() {
        let rest = &b[p..];
        let Some(request) = http::start_kind(rest) else {
            if out.is_empty() {
                return Err(());
            }
            break;
        };
        let head = http::parse_head(rest, request);
        let mut m = HttpMsg { at: p, request, head, body: (p, 0), framing: http::Framing::None, complete: false };
        let Ok(Some(h)) = &m.head else {
            out.push(m);
            break;
        };
        let body_at = p + h.len;
        let avail = b.len() - body_at;
        let framing = if request { http::request_framing(&h.headers) } else { http::response_framing(h.status, &h.headers, "GET") };
        m.framing = framing.unwrap_or(http::Framing::UntilClose);
        let (n, complete) = match m.framing {
            http::Framing::None => (0, true),
            http::Framing::Length(len) => (avail.min(len as usize), avail as u64 >= len),
            http::Framing::Chunked => chunked_end(&b[body_at..]).map_or((avail, false), |e| (e, true)),
            http::Framing::UntilClose => (avail, false),
        };
        m.body = (body_at, n);
        m.complete = complete;
        p = body_at + n;
        out.push(m);
        if !complete {
            break;
        }
    }
    Ok(out)
}

/// the length of a complete chunked body at the start of `b` (None: it goes on)
fn chunked_end(b: &[u8]) -> Option<usize> {
    let mut p = 0;
    loop {
        let i = b.get(p..)?.iter().position(|&c| c == b'\n')?;
        let size = b[p..p + i].split(|&c| c == b';').next().unwrap_or(&[]).trim_ascii();
        let n = usize::from_str_radix(core::str::from_utf8(size).ok()?, 16).ok()?;
        p += i + 1;
        if n == 0 {
            // trailers up to the empty line
            loop {
                let j = b.get(p..)?.iter().position(|&c| c == b'\n')?;
                let empty = b[p..p + j].iter().all(|&c| c == b'\r');
                p += j + 1;
                if empty {
                    return Some(p);
                }
            }
        }
        p = p.checked_add(n)?;
        match b.get(p..)? {
            [b'\r', b'\n', ..] => p += 2,
            [b'\n', ..] => p += 1,
            _ => return None,
        }
    }
}

/// the lines of a head without the empty lines before it: (start, end of the text, end with
/// the line end); the empty line that ends the head is the last one
fn http_lines(b: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut v: Vec<(usize, usize, usize)> = Vec::new();
    let mut p = 0;
    while let Some(i) = b[p..].iter().position(|&c| c == b'\n') {
        let end = p + i + 1;
        let e = if i > 0 && b[p + i - 1] == b'\r' { p + i - 1 } else { p + i };
        if e == p && v.is_empty() {
            p = end;
            continue;
        }
        v.push((p, e, end));
        p = end;
        if e == v[v.len() - 1].0 {
            break;
        }
    }
    v
}

/// the fields of one message
fn http_fields(l: &mut B, b: &[u8], off: usize, m: &HttpMsg) {
    let msg = &b[m.at..];
    let lines = http_lines(msg);
    if let Some(&(s, e, _)) = lines.first() {
        let line = &msg[s..e];
        let names = if m.request { ["Request Method", "Request URI", "Request Version"] } else { ["Response Version", "Status Code", "Response Phrase"] };
        let mut start = 0;
        for (k, name) in names.iter().enumerate() {
            let end = if k == 2 { line.len() } else { line[start..].iter().position(|&c| c == b' ').map_or(line.len(), |x| start + x) };
            if end > start {
                l.f(name, off + m.at + s + start, end - start, String::from_utf8_lossy(&line[start..end]).into_owned());
            }
            start = (end + 1).min(line.len());
        }
    }
    for &(s, e, end) in lines.iter().skip(1) {
        if s == e {
            break;
        }
        let t = String::from_utf8_lossy(&msg[s..e]).into_owned();
        match t.split_once(':').and_then(|(k, v)| HTTP_NAMES.iter().find(|n| n.eq_ignore_ascii_case(k.trim())).map(|n| (*n, v.trim()))) {
            Some((n, v)) => l.f(n, off + m.at + s, end - s, String::from(v)),
            None => l.f("Header", off + m.at + s, end - s, t.clone()),
        }
    }
    match &m.head {
        Err(e) => l.bad(format!("malformed HTTP {}: {}", if m.request { "request" } else { "response" }, e.text())),
        Ok(None) => {}
        Ok(Some(_)) => {
            let (at, n) = m.body;
            if m.framing == http::Framing::Chunked {
                http_chunks(l, &b[at..at + n], off + at, m.complete);
            } else if n > 0 {
                l.f("File Data", off + at, n, format!("{} bytes", n));
            }
        }
    }
}

/// the chunks of a chunked body as fields
fn http_chunks(l: &mut B, b: &[u8], off: usize, complete: bool) {
    let mut p = 0;
    while p < b.len() {
        let Some(i) = b[p..].iter().position(|&c| c == b'\n') else {
            break;
        };
        let size = b[p..p + i].split(|&c| c == b';').next().unwrap_or(&[]).trim_ascii();
        let Some(n) = core::str::from_utf8(size).ok().and_then(|s| usize::from_str_radix(s, 16).ok()) else {
            l.bad(String::from("bad chunk size"));
            return;
        };
        if n == 0 {
            l.f("End of chunked encoding", off + p, b.len() - p, String::from("0"));
            return;
        }
        let end = (p + i + 1 + n + 2).min(b.len());
        l.f("Data chunk", off + p, end - p, format!("{} octets", n));
        p = end;
    }
    if !complete && p < b.len() {
        l.f("Data", off + p, b.len() - p, format!("{} bytes", b.len() - p));
    }
}

fn http_layer(frame: &[u8], off: usize, len: usize) -> B {
    let b = &frame[off..off + len];
    let mut l = B::new(Proto::Http, off, len, String::from("Hypertext Transfer Protocol"));
    let Ok(msgs) = http_walk(b) else {
        l.f("Continuation", off, len, format!("{} bytes", len));
        return l;
    };
    let mut end = 0;
    for m in &msgs {
        http_fields(&mut l, b, off, m);
        end = if matches!(m.head, Ok(Some(_))) { m.body.0 + m.body.1 } else { b.len() };
    }
    if end < b.len() {
        l.f("Data", off + end, b.len() - end, format!("{} bytes", b.len() - end));
    }
    l
}

/// "GET /index.html HTTP/1.1", "HTTP/1.1 200 OK  (text/html)", "Continuation"; pipelined
/// messages one after the other
fn http_info(b: &[u8]) -> String {
    let Ok(msgs) = http_walk(b) else {
        return String::from("Continuation");
    };
    let mut parts: Vec<String> = Vec::new();
    for m in &msgs {
        match &m.head {
            Ok(Some(h)) if h.is_request() => parts.push(h.start_line()),
            Ok(Some(h)) => match h.header("content-type") {
                Some(ct) => parts.push(format!("{}  ({})", h.start_line(), ct.split(';').next().unwrap_or("").trim())),
                None => parts.push(h.start_line()),
            },
            _ => {
                // a head that goes on, or a malformed one: its first line as it is
                let rest = &b[m.at..];
                let e = rest.iter().position(|&c| c == b'\n').unwrap_or(rest.len()).min(120);
                parts.push(String::from(String::from_utf8_lossy(&rest[..e]).trim()));
            }
        }
    }
    parts.join(" , ")
}

// ---------------------------------------------------------------- STP and RSTP

/// a BPDU timer value (1/256 s) in seconds: "20", "1.50"
fn bpdu_time(v: u16) -> String {
    let (s, r) = (v / crate::stp::TIME_UNIT, v % crate::stp::TIME_UNIT);
    if r == 0 { format!("{}", s) } else { format!("{}.{:02}", s, r as u32 * 100 / crate::stp::TIME_UNIT as u32) }
}

/// "32768 / 0 / 02:4a:00:00:01:00"
fn bridge_id_str(id: crate::stp::BridgeId) -> String {
    format!("{} / {} / {}", id.priority() & 0xF000, id.ext(), id.mac())
}

fn stp_layer(frame: &[u8], off: usize, len: usize) -> B {
    use crate::stp::{self, Bpdu, Kind};
    let b = &frame[off..off + len];
    let parsed = Bpdu::parse(b);
    let title = match &parsed {
        Ok(x) if x.kind == Kind::Config => "Spanning Tree Protocol (Configuration BPDU)",
        Ok(x) if x.kind == Kind::Tcn => "Spanning Tree Protocol (Topology Change Notification)",
        Ok(x) if x.version >= stp::VERSION_MSTP => "Spanning Tree Protocol (MST BPDU)",
        Ok(_) => "Spanning Tree Protocol (RST BPDU)",
        Err(_) => "Spanning Tree Protocol",
    };
    let mut l = B::new(Proto::Stp, off, len, String::from(title));
    if b.len() >= 2 {
        let pid = be16(b, 0);
        l.f("Protocol Identifier", off, 2, if pid == 0 { String::from("Spanning Tree Protocol (0x0000)") } else { format!("0x{:04x}", pid) });
    }
    if b.len() >= 3 {
        let v = match b[2] {
            stp::VERSION_STP => String::from("Spanning Tree (0)"),
            stp::VERSION_RSTP => String::from("Rapid Spanning Tree (2)"),
            stp::VERSION_MSTP => String::from("Multiple Spanning Tree (3)"),
            n => format!("{}", n),
        };
        l.f("Protocol Version Identifier", off + 2, 1, v);
    }
    if b.len() >= 4 {
        let t = match b[3] {
            stp::TYPE_CONFIG => String::from("Configuration (0x00)"),
            stp::TYPE_TCN => String::from("Topology Change Notification (0x80)"),
            stp::TYPE_RST => String::from("Rapid/Multiple Spanning Tree (0x02)"),
            n => format!("0x{:02x}", n),
        };
        l.f("BPDU Type", off + 3, 1, t);
    }
    let x = match parsed {
        Ok(x) => x,
        Err(e) => {
            l.bad(format!("not a valid BPDU: {}", e.text()));
            if b.len() > 4 {
                l.f("Data", off + 4, b.len() - 4, format!("{} bytes", b.len() - 4));
            }
            return l;
        }
    };
    if x.kind == Kind::Tcn {
        return l;
    }
    l.f("BPDU flags", off + 4, 1, format!("0x{:02x} ({})", x.flags, stp::flags_text(x.kind, x.flags)));
    l.f("Root Identifier", off + 5, 8, bridge_id_str(x.root));
    l.f("Root Path Cost", off + 13, 4, format!("{}", x.cost));
    l.f("Bridge Identifier", off + 17, 8, bridge_id_str(x.bridge));
    l.f("Port identifier", off + 25, 2, format!("0x{:04x} (priority {}, port {})", x.port.0, x.port.priority(), x.port.number()));
    l.f("Message Age", off + 27, 2, bpdu_time(x.msg_age));
    l.f("Max Age", off + 29, 2, bpdu_time(x.max_age));
    l.f("Hello Time", off + 31, 2, bpdu_time(x.hello));
    l.f("Forward Delay", off + 33, 2, bpdu_time(x.fwd_delay));
    if x.kind == Kind::Rst && b.len() > stp::CONFIG_LEN {
        l.f("Version 1 Length", off + stp::CONFIG_LEN, 1, format!("{}", b[stp::CONFIG_LEN]));
    }
    if x.msg_age >= x.max_age {
        l.bad(String::from("Message Age is not below Max Age: a bridge drops this BPDU"));
    }
    l
}
