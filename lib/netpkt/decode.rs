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
// RIP (port 520) shows its header and one field per entry (route, authentication, the request
// for the whole table) with the problems a receiver would refuse (RFC 2453, RFC 1058).
// IPv6 shows its extension headers as fields of its layer (a later fragment carries data only);
// ICMPv6 every field of echo, the errors (with the quoted packet) and Neighbor Discovery (RS, RA,
// NS, NA, redirect and one field per option) with what a receiver refuses: a checksum over the
// pseudo header that does not fit, an ND message whose hop limit is not 255 (RFC 4443, 4861).
// OSPF (IP protocol 89) shows the common header, the fields of the five packet types, every LSA
// header and the links, routers and metrics of the LSAs an update carries, with what a receiver
// refuses: another version, a wrong packet or LSA checksum, bodies that do not fit (RFC 2328).
// EAPOL (ethertype 0x888E) shows its header and an EAPOL-Key body field by field: the Key
// Information bits, replay counter, nonce, MIC and the key data (the RSN element and, when it is
// not encrypted, the KDEs); the summary names the message of the 4-way handshake (eapol.rs).
// BGP (TCP port 179) shows every message a segment holds: the header, the fields of OPEN with each
// capability, UPDATE with each withdrawn route, path attribute (its flags and value) and NLRI,
// NOTIFICATION with the names of its code and subcode, ROUTE-REFRESH; what a receiver would answer
// with a NOTIFICATION is a problem with that code; a segment that does not start with the marker
// is the rest of a message of an earlier one (bgp.rs; AS numbers of 2 or 4 octets as
// bgp::as_path_4 guesses).
// TFTP (port 69, and the DATA, ACK, ERROR and OACK packets of a transfer between two ports
// above 1023 by their shape: RFC 1350 moves a transfer to new ports) shows its opcode, file,
// mode, options, block, data and error (tftp.rs).  FTP (TCP port 21) shows each command line
// with its verb and argument and each reply line with its code; FTP-DATA (port 20, the active
// data connections) its bytes; passive data connections run between two free ports and show
// as TCP data (ftp.rs).
// SMTP (TCP ports 25 and 587) shows each command line with its verb and parameter, each reply
// line with its code and enhanced status code, and the lines of a message after DATA as such
// (the line with the dot ends it); POP3 (port 110) each command with its parameters, the status
// indicator and description of a response, the lines of a multi-line one.  Both share the bit
// P_MAIL (smtp.rs, pop3.rs).
// SNMP (UDP ports 161 and 162) shows version, community, the PDU with request id, error status and
// index (non-repeaters and max-repetitions of a GetBulk; enterprise, agent address, generic and
// specific trap and time stamp of a v1 Trap) and each variable binding with the name of a known
// object and its value as net-snmp prints it; BER that does not hold is a problem at its place
// (snmp.rs, ber.rs, mib.rs).
// RADIUS (UDP ports 1812 and 1813, the old 1645 and 1646) shows its code, identifier, length and
// authenticator and every attribute by name with its value (named values of the integers, a
// Vendor-Specific attribute's vendor, a hidden User-Password as hidden), with what a receiver
// discards: a Length that does not fit, attributes that do not end at it (radius.rs).  Telnet
// (TCP port 23) shows each command and negotiation ("Will Echo", "Interrupt Process") and the
// data as text; a command cut off at the end of a segment is a problem (telnet.rs).

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
use crate::rip::{self, RipView};
use crate::icmpv6::{self, Icmp6View, NdMsg, NdOpt};
use crate::ospf;
use crate::bgp;
use crate::tftp;
use crate::ftp;
use crate::smtp;
use crate::pop3;
use crate::ber;
use crate::mib;
use crate::snmp;
use crate::radius;
use crate::telnet;

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
pub const P_EAPOL: u32 = 1 << 25;
/// BGP (TCP port 179); a bit far from the next free ones, which other protocols take in order
pub const P_BGP: u32 = 1 << 30;
/// FTP's control connection and its active data connections (FTP-DATA)
pub const P_FTP: u32 = 1 << 26;
pub const P_DATA: u32 = 1 << 31;
/// mail: SMTP (TCP ports 25 and 587) and POP3 (port 110)
pub const P_MAIL: u32 = 1 << 28;
/// RADIUS (UDP ports 1812, 1813, 1645, 1646)
pub const P_RADIUS: u32 = 1 << 27;
/// Telnet (TCP port 23)
pub const P_TELNET: u32 = 1 << 29;

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
    Eapol,
    Bgp,
    /// FTP's control connection (port 21)
    Ftp,
    /// an active FTP data connection (port 20)
    FtpData,
    /// SMTP (ports 25 and 587)
    Smtp,
    /// POP3 (port 110)
    Pop3,
    /// RADIUS authentication and accounting
    Radius,
    /// Telnet (port 23)
    Telnet,
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
            Proto::Eapol => "EAPOL",
            Proto::Bgp => "BGP",
            Proto::Ftp => "FTP",
            Proto::FtpData => "FTP-DATA",
            Proto::Smtp => "SMTP",
            Proto::Pop3 => "POP3",
            Proto::Radius => "RADIUS",
            Proto::Telnet => "Telnet",
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
            Proto::Eapol => P_EAPOL,
            Proto::Bgp => P_BGP,
            Proto::Ftp | Proto::FtpData => P_FTP,
            Proto::Smtp | Proto::Pop3 => P_MAIL,
            Proto::Radius => P_RADIUS,
            Proto::Telnet => P_TELNET,
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
            p if radius::is_port(p) => Proto::Radius,
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
            bgp::PORT => Proto::Bgp,
            ftp::PORT_FTP => Proto::Ftp,
            ftp::PORT_FTP_DATA => Proto::FtpData,
            smtp::PORT_SMTP | smtp::PORT_SUBMISSION => Proto::Smtp,
            pop3::PORT_POP3 => Proto::Pop3,
            telnet::PORT => Proto::Telnet,
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
            s.trailer = (rest.len() - v.packet().len()) as u32;
            ipv6_stack(&mut s, &v, off);
        }
        eth::ET_LLDP => s.push(Proto::Lldp, off, rest.len()),
        eth::ET_EAPOL => match crate::eapol::EapolView::parse(rest) {
            Some(v) => {
                s.push(Proto::Eapol, off, v.pdu().len());
                s.trailer = (rest.len() - v.pdu().len()) as u32;
            }
            None => s.push(Proto::Data, off, rest.len()),
        },
        _ => {
            if !rest.is_empty() {
                s.push(Proto::Data, off, rest.len());
            }
        }
    }
    s
}

/// the IPv6 layer with its extension headers and what it carries (a later fragment: data only)
fn ipv6_stack(s: &mut Stack, v: &Ipv6View, off: usize) {
    let Some((nh, ext)) = ipv6::upper_layer(v) else {
        s.push(Proto::Ipv6, off, ipv6::IPV6_HDR);
        return;
    };
    s.push(Proto::Ipv6, off, ipv6::IPV6_HDR + ext);
    let poff = off + ipv6::IPV6_HDR + ext;
    if ipv6::frag(v).is_some_and(|(f, _)| f.offset > 0) {
        if ext < v.payload().len() {
            s.push(Proto::Data, poff, v.payload().len() - ext);
        }
        return;
    }
    transport(s, nh, &v.payload()[ext..], poff);
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
                // a running TFTP transfer has left port 69: its packets by their shape
                let (sp, dp) = (u.src_port(), u.dst_port());
                let app = udp_app(sp, dp).or_else(|| (sp > 1023 && dp > 1023 && tftp::looks_like(u.payload())).then_some(Proto::Tftp)).unwrap_or(Proto::Data);
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
                s.trailer = (packet.len() - v.packet().len()) as u32;
                ipv6_stack(&mut s, &v, 0);
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
    // the hop limit of the IPv6 packet and whether it is a fragment (ND, ICMPv6 checksums)
    let mut hop6: Option<u8> = None;
    let mut frag6 = false;
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
                    hop6 = Some(v.hop_limit());
                    frag6 = ipv6::frag(&v).is_some();
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
            Proto::Rip => Some(rip_layer(frame, off, len)),
            Proto::Icmpv6 => Some(icmpv6_layer(frame, off, len, net, hop6, frag6)),
            Proto::Ospf => Some(ospf_layer(frame, off, len)),
            Proto::Eapol => Some(eapol_layer(frame, off, len)),
            Proto::Bgp => Some(bgp_layer(frame, off, len)),
            Proto::Tftp => Some(tftp_layer(frame, off, len)),
            Proto::Ftp => Some(ftp_layer(frame, off, len, ftp_response(&st, frame))),
            Proto::Smtp => Some(smtp_layer(frame, off, len, mail_response(&st, frame))),
            Proto::Pop3 => Some(pop3_layer(frame, off, len, mail_response(&st, frame))),
            Proto::Snmp => Some(snmp_layer(frame, off, len)),
            Proto::Radius => Some(radius_layer(frame, off, len)),
            Proto::Telnet => Some(telnet_layer(frame, off, len)),
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

/// a Next Header value with its name ("ICMPv6 (58)", "Fragment Header (44)")
fn nh_str(nh: u8) -> String {
    match ipv6::ext_name(nh).or_else(|| ipv4::proto_name(nh)) {
        Some(n) => format!("{} ({})", n, nh),
        None if nh == ipv6::NH_NONE => format!("No Next Header ({})", nh),
        None => format!("{}", nh),
    }
}

fn ipv6_layer(v: &Ipv6View, off: usize) -> B {
    let ext = ipv6::ext_headers(v);
    let elen = ext.as_ref().and_then(|e| e.last()).map_or(0, |e| e.off + e.len);
    let mut b = B::new(Proto::Ipv6, off, ipv6::IPV6_HDR + elen, format!("Internet Protocol Version 6, Src: {}, Dst: {}", v.src(), v.dst()));
    b.f("Version", off, 1, String::from("6"));
    b.f("Traffic Class", off, 2, format!("0x{:02x}", v.traffic_class()));
    b.f("Flow Label", off + 1, 3, format!("0x{:05x}", v.flow_label()));
    b.f("Payload Length", off + 4, 2, format!("{}", v.payload_len()));
    b.f("Next Header", off + 6, 1, nh_str(v.next_header()));
    b.f("Hop Limit", off + 7, 1, format!("{}", v.hop_limit()));
    b.f("Source Address", off + 8, 16, format!("{}", v.src()));
    b.f("Destination Address", off + 24, 16, format!("{}", v.dst()));
    for e in ext.iter().flatten() {
        let at = off + ipv6::IPV6_HDR + e.off;
        let value = if e.kind == ipv6::NH_FRAGMENT {
            let f = ipv6::FragHdr::read(&v.payload()[e.off..e.off + e.len]);
            format!("offset {}, {}, identification 0x{:08x}, next header {}", f.offset, if f.more { "more fragments" } else { "last fragment" }, f.id, nh_str(f.next))
        } else {
            format!("next header {}, {} bytes", nh_str(e.next), e.len)
        };
        b.f(ipv6::ext_name(e.kind).unwrap_or("Extension Header"), at, e.len, value);
    }
    if ext.is_none() {
        b.bad(String::from("malformed extension headers"));
    }
    if v.hop_limit() == 0 {
        b.bad(String::from("Hop Limit is 0"));
    }
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
        Proto::FtpData => String::from("FTP Data"),
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
        if let Some((f, _)) = ipv6::frag(&v)
            && f.offset > 0
        {
            return (Proto::Ipv6, format!("IPv6 fragment (off={} more={} ident=0x{:08x} nxt={})", f.offset, if f.more { "y" } else { "n" }, f.id, f.next));
        }
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
        Proto::Icmpv6 => {
            let (o, l) = st.find(Proto::Icmpv6).unwrap_or((0, 0));
            icmpv6_info(&frame[o..o + l], ttl.unwrap_or(0))
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
        Proto::Rip => rip_info(b),
        Proto::Ospf => ospf_info(b),
        Proto::Eapol => Some(eapol_info(b)),
        Proto::Bgp => Some(bgp_info(b)),
        Proto::Tftp => Some(tftp_info(b)),
        Proto::Ftp => Some(ftp_info(b, ftp_response(st, frame))),
        Proto::FtpData => Some(format!("FTP Data: {} bytes", b.len())),
        Proto::Smtp => Some(smtp_info(b, mail_response(st, frame))),
        Proto::Pop3 => Some(pop3_info(b, mail_response(st, frame))),
        Proto::Snmp => Some(snmp_info(b)),
        Proto::Radius => Some(radius_info(b)),
        Proto::Telnet => Some(telnet_info(b)),
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
        let end = (p + i + 1).saturating_add(n).saturating_add(2).min(b.len());
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
// ---------------------------------------------------------------- RIP

/// "RIPv2" (version 0 and unknown ones as their number)
fn rip_version(v: u8) -> String {
    format!("RIPv{}", v)
}

/// the request for the whole table: one entry, address family 0, metric 16
fn rip_whole_request(v: &RipView) -> bool {
    v.command() == rip::CMD_REQUEST && v.routes().count() == 1 && v.routes().next().is_some_and(|e| e.afi == 0 && e.metric == rip::INFINITY)
}

fn rip_metric(m: u32) -> String {
    if m == rip::INFINITY { String::from("16 (unreachable)") } else { format!("{}", m) }
}

fn rip_layer(frame: &[u8], off: usize, len: usize) -> B {
    let b = &frame[off..off + len];
    let Some(v) = RipView::parse(b) else {
        let mut l = payload_layer(Proto::Rip, off, len);
        l.bad(String::from("shorter than a RIP header"));
        return l;
    };
    let (cmd, ver) = (v.command(), v.version());
    let mut l = B::new(Proto::Rip, off, len, format!("Routing Information Protocol ({} {})", rip_version(ver), rip::command_name(cmd)));
    l.f("Command", off, 1, format!("{} ({})", rip::command_name(cmd), cmd));
    l.f("Version", off + 1, 1, format!("{} ({})", rip_version(ver), ver));
    l.f("Must be zero", off + 2, 2, format!("0x{:04x}", v.mbz()));
    let whole = rip_whole_request(&v);
    let mut mbz = v.mbz() != 0;
    for i in 0..v.count() {
        let o = off + rip::RIP_HDR + i * rip::RTE_LEN;
        let Some(e) = v.entry(i) else {
            break;
        };
        if e.afi == rip::AFI_AUTH {
            let what = match v.auth() {
                Some(a) if i == 0 => match a.text() {
                    Some(pw) => format!("Simple Password: {}", pw),
                    None if a.kind == rip::AUTH_MD5 => String::from("Keyed MD5 (3)"),
                    None => format!("type {}", a.kind),
                },
                _ => String::from("not the first entry: ignored"),
            };
            l.f("Authentication", o, rip::RTE_LEN, what);
            if i > 0 {
                l.bad(String::from("an authentication entry after the first one"));
            }
            continue;
        }
        if whole && e.afi == 0 {
            l.f("Request", o, rip::RTE_LEN, String::from("the whole routing table (address family 0, metric 16)"));
            continue;
        }
        if e.afi != rip::AFI_INET {
            l.f("Entry", o, rip::RTE_LEN, format!("address family {}, metric {}", e.afi, e.metric));
            continue;
        }
        let mut s = if ver >= 2 {
            match e.prefix() {
                Some(p) => format!("{}/{}, Metric: {}", e.addr, p.len, rip_metric(e.metric)),
                None if e.mask.is_unspecified() => format!("{}, Metric: {}", e.addr, rip_metric(e.metric)),
                None => {
                    l.bad(format!("netmask {} of {} is not contiguous", e.mask, e.addr));
                    format!("{} mask {}, Metric: {}", e.addr, e.mask, rip_metric(e.metric))
                }
            }
        } else {
            format!("{}, Metric: {}", e.addr, rip_metric(e.metric))
        };
        if ver >= 2 && !e.next_hop.is_unspecified() {
            s.push_str(&format!(", Next Hop: {}", e.next_hop));
        }
        if ver >= 2 && e.tag != 0 {
            s.push_str(&format!(", Route Tag: {}", e.tag));
        }
        l.f("IP Address", o, rip::RTE_LEN, s);
        if e.metric == 0 || e.metric > rip::INFINITY {
            l.bad(format!("metric {} of {} is out of range (1 to 16)", e.metric, e.addr));
        }
        if ver == 1 && !e.mbz_clear() {
            mbz = true;
        }
    }
    if v.trailing() > 0 {
        let n = v.trailing();
        l.f("Trailing bytes", off + len - n, n, format!("{} bytes", n));
        l.bad(format!("{} bytes after the last entry", n));
    }
    if v.count() > rip::MAX_ENTRIES {
        l.bad(format!("{} entries, more than 25", v.count()));
    }
    if ver == 1 && mbz {
        l.bad(String::from("a must-be-zero field is set: RIPv1 receivers ignore the message (RFC 1058)"));
    }
    if ver == 0 {
        l.bad(String::from("version 0: receivers ignore the message"));
    }
    if cmd != rip::CMD_REQUEST && cmd != rip::CMD_RESPONSE {
        l.bad(format!("command {} is not a request or a response", cmd));
    }
    l
}

/// "RIPv2 Response, 3 routes", "RIPv2 Request for the whole table", "RIPv1 Request, 2 entries"
fn rip_info(b: &[u8]) -> Option<String> {
    let v = RipView::parse(b)?;
    let n = v.routes().count();
    let mut s = if rip_whole_request(&v) {
        format!("{} Request for the whole table", rip_version(v.version()))
    } else if v.command() == rip::CMD_REQUEST {
        format!("{} Request, {} entr{}", rip_version(v.version()), n, if n == 1 { "y" } else { "ies" })
    } else {
        format!("{} {}, {} route{}", rip_version(v.version()), rip::command_name(v.command()), n, if n == 1 { "" } else { "s" })
    };
    if v.auth().is_some() {
        s.push_str(", authenticated");
    }
    Some(s)
}

// ---------------------------------------------------------------- ICMPv6 and Neighbor Discovery

/// "Destination Unreachable" without the code (the Type field names the type alone)
fn icmpv6_type_name(t: u8) -> &'static str {
    match t {
        icmpv6::T_DEST_UNREACH => "Destination Unreachable",
        icmpv6::T_PACKET_TOO_BIG => "Packet Too Big",
        icmpv6::T_TIME_EXCEEDED => "Time Exceeded",
        icmpv6::T_PARAM_PROBLEM => "Parameter Problem",
        t => icmpv6::name(t, 0),
    }
}

/// a lifetime of a prefix: "2592000 s", "infinity"
fn lifetime_str(v: u32) -> String {
    if v == icmpv6::INFINITE { String::from("infinity") } else { format!("{} s", v) }
}

/// the flags of a Neighbor Advertisement as capture tools abbreviate them: "rtr, sol, ovr"
fn na_flags(router: bool, solicited: bool, override_: bool) -> String {
    let mut v: Vec<&str> = Vec::new();
    if router {
        v.push("rtr");
    }
    if solicited {
        v.push("sol");
    }
    if override_ {
        v.push("ovr");
    }
    v.join(", ")
}

fn icmpv6_layer(frame: &[u8], off: usize, len: usize, net: Net, hop: Option<u8>, frag: bool) -> B {
    let b = &frame[off..off + len];
    let Some(v) = Icmp6View::parse(b) else {
        return payload_layer(Proto::Icmpv6, off, len);
    };
    let (t, c) = (v.typ(), v.code());
    let mut l = B::new(Proto::Icmpv6, off, len, String::from("Internet Control Message Protocol v6"));
    l.f("Type", off, 1, format!("{} ({})", t, icmpv6_type_name(t)));
    l.f("Code", off + 1, 1, format!("{}", c));
    match net {
        Net::V6(s, d) if !frag => {
            let ok = v.checksum_ok(s, d);
            let want = if ok { None } else { Some(v.checksum_want(s, d)) };
            l.f("Checksum", off + 2, 2, ck_str(v.checksum(), ok, want));
            if !ok {
                l.bad(format!("bad ICMPv6 checksum (should be 0x{:04x})", want.unwrap_or(0)));
            }
        }
        Net::V6(..) => l.f("Checksum", off + 2, 2, format!("0x{:04x} [the message goes on in later fragments]", v.checksum())),
        _ => l.f("Checksum", off + 2, 2, format!("0x{:04x}", v.checksum())),
    }
    let short = |l: &mut B, need: usize| {
        if b.len() < need {
            l.bad(format!("shorter than its fixed fields ({} of {} bytes)", b.len(), need));
            true
        } else {
            false
        }
    };
    match t {
        icmpv6::T_ECHO_REQUEST | icmpv6::T_ECHO_REPLY => {
            if short(&mut l, icmpv6::ICMPV6_HDR) {
                return l;
            }
            l.f("Identifier", off + 4, 2, format!("0x{:04x} ({})", v.id(), v.id()));
            l.f("Sequence Number", off + 6, 2, format!("{}", v.seq()));
            if !v.body().is_empty() {
                l.f("Data", off + 8, v.body().len(), format!("{} bytes", v.body().len()));
            }
            return l;
        }
        icmpv6::T_DEST_UNREACH | icmpv6::T_TIME_EXCEEDED | icmpv6::T_PACKET_TOO_BIG | icmpv6::T_PARAM_PROBLEM => {
            if short(&mut l, icmpv6::ICMPV6_HDR) {
                return l;
            }
            match t {
                icmpv6::T_PACKET_TOO_BIG => l.f("MTU", off + 4, 4, format!("{}", v.value())),
                icmpv6::T_PARAM_PROBLEM => l.f("Pointer", off + 4, 4, format!("{}", v.value())),
                _ => l.f("Reserved", off + 4, 4, format!("0x{:08x}", v.value())),
            }
            if let Some(q) = v.quoted() {
                let nh = ipv6::upper_layer(&q).map_or(q.next_header(), |x| x.0);
                l.f("Quoted packet", off + 8, v.body().len(), format!("{} -> {}, {}", q.src(), q.dst(), ipv4::proto_name(nh).unwrap_or("?")));
            } else if !v.body().is_empty() {
                l.f("Data", off + 8, v.body().len(), format!("{} bytes", v.body().len()));
            }
            return l;
        }
        _ => {}
    }
    let Some(fixed) = icmpv6::fixed_len(t) else {
        if b.len() > 4 {
            l.f("Data", off + 4, b.len() - 4, format!("{} bytes", b.len() - 4));
        }
        return l;
    };
    // Neighbor Discovery: the fixed fields, then one field per option
    if short(&mut l, fixed) {
        return l;
    }
    match t {
        icmpv6::T_ROUTER_ADVERT => {
            l.f("Cur hop limit", off + 4, 1, format!("{}", b[4]));
            let prf = match (b[5] >> 3) & 3 {
                1 => "high",
                3 => "low",
                _ => "medium",
            };
            l.f("Flags", off + 5, 1, format!("0x{:02x} (managed: {}, other: {}, preference: {})", b[5], if b[5] & 0x80 != 0 { "yes" } else { "no" }, if b[5] & 0x40 != 0 { "yes" } else { "no" }, prf));
            l.f("Router lifetime", off + 6, 2, format!("{} s", be16(b, 6)));
            l.f("Reachable time", off + 8, 4, format!("{} ms", crate::be32(b, 8)));
            l.f("Retrans timer", off + 12, 4, format!("{} ms", crate::be32(b, 12)));
        }
        icmpv6::T_NEIGHBOR_ADVERT => {
            let fl = na_flags(b[4] & 0x80 != 0, b[4] & 0x40 != 0, b[4] & 0x20 != 0);
            l.f("Flags", off + 4, 4, format!("0x{:08x}{}", crate::be32(b, 4), if fl.is_empty() { String::new() } else { format!(" ({})", fl) }));
            l.f("Target Address", off + 8, 16, format!("{}", Ipv6(b[8..24].try_into().unwrap_or([0; 16]))));
        }
        _ => {
            l.f("Reserved", off + 4, 4, format!("0x{:08x}", crate::be32(b, 4)));
            if fixed >= 24 {
                l.f("Target Address", off + 8, 16, format!("{}", Ipv6(b[8..24].try_into().unwrap_or([0; 16]))));
            }
            if fixed >= 40 {
                l.f("Destination Address", off + 24, 16, format!("{}", Ipv6(b[24..40].try_into().unwrap_or([0; 16]))));
            }
        }
    }
    match icmpv6::options(&b[fixed..]) {
        Ok(opts) => {
            for (o, at, n) in opts {
                let at = off + fixed + at;
                match o {
                    NdOpt::SrcLla(m) => l.f("Source link-layer address", at, n, format!("{}", m)),
                    NdOpt::TgtLla(m) => l.f("Target link-layer address", at, n, format!("{}", m)),
                    NdOpt::Prefix(p) => {
                        let mut fl: Vec<&str> = Vec::new();
                        if p.on_link {
                            fl.push("on-link");
                        }
                        if p.auto {
                            fl.push("autonomous");
                        }
                        let fl = if fl.is_empty() { String::from("no flags") } else { fl.join(", ") };
                        l.f("Prefix information", at, n, format!("{}/{}, {}, valid lifetime {}, preferred lifetime {}", p.prefix, p.len, fl, lifetime_str(p.valid), lifetime_str(p.preferred)));
                        if p.preferred != icmpv6::INFINITE && (p.valid != icmpv6::INFINITE && p.preferred > p.valid) {
                            l.bad(format!("prefix {}/{}: the preferred lifetime exceeds the valid one", p.prefix, p.len));
                        }
                    }
                    NdOpt::Redirected(d) => l.f("Redirected header", at, n, format!("{} bytes of the redirected packet", d.len())),
                    NdOpt::Mtu(m) => l.f("MTU", at, n, format!("{}", m)),
                    NdOpt::Other(k, d) => l.f("Option", at, n, format!("type {}, {} bytes", k, d.len() + 2)),
                }
            }
        }
        Err(e) => l.bad(format!("options: {}", e.text())),
    }
    match icmpv6::Nd::parse(&v) {
        Ok(nd) => {
            if let (Net::V6(s, d), Some(h)) = (net, hop)
                && let Some(why) = nd.check_ip(s, d, h)
            {
                l.bad(format!("a receiver drops this message: {}", why.text()));
            }
        }
        Err(e) => l.bad(format!("not a valid Neighbor Discovery message: {}", e.text())),
    }
    l
}

// ---------------------------------------------------------------- OSPF

/// "Router-LSA 1.1.1.1 from 1.1.1.1, seq 0x80000002, age 3"
fn ospf_lsa_head(h: &ospf::LsaHeader) -> String {
    format!("{} {} from {}, seq 0x{:08x}, age {}", ospf::lsa_name(h.kind), h.id, h.adv, h.seq, h.age)
}

fn ospf_link_kind(k: u8) -> &'static str {
    match k {
        ospf::LINK_P2P => "point-to-point",
        ospf::LINK_TRANSIT => "transit network",
        ospf::LINK_STUB => "stub network",
        ospf::LINK_VIRTUAL => "virtual link",
        _ => "unknown link type",
    }
}

/// the fields of one LSA of an update at `o` (its length checked); its problems
fn ospf_lsa_fields(l: &mut B, lsa: &[u8], o: usize) {
    let Some(h) = ospf::LsaHeader::of(lsa) else {
        return;
    };
    let ok = ospf::lsa_checksum_ok(lsa);
    let ck = if ok { format!("0x{:04x} [correct]", h.checksum) } else { format!("0x{:04x} [incorrect, should be 0x{:04x}]", h.checksum, ospf::lsa_checksum(lsa)) };
    l.f("LSA", o, ospf::LSA_HDR_LEN, format!("{}, options {}, checksum {}, len {}", ospf_lsa_head(&h), ospf::options_text(h.options), ck, h.len));
    if !ok {
        l.bad(format!("bad LSA checksum of {} {} from {}", ospf::lsa_name(h.kind), h.id, h.adv));
    }
    if h.age > ospf::MAX_AGE {
        l.bad(format!("LS age {} is beyond MaxAge (3600)", h.age));
    }
    let x = match ospf::Lsa::parse(lsa) {
        Ok(x) => x,
        Err(e) => {
            l.bad(format!("{} {}: {}", ospf::lsa_name(h.kind), h.id, e.text()));
            return;
        }
    };
    let b = o + ospf::LSA_HDR_LEN;
    match &x.body {
        ospf::LsaBody::Router(r) => {
            let mut fl: Vec<&str> = Vec::new();
            for (bit, n) in [(ospf::RTR_V, "V (virtual link endpoint)"), (ospf::RTR_E, "E (AS boundary router)"), (ospf::RTR_B, "B (area border router)")] {
                if r.flags & bit != 0 {
                    fl.push(n);
                }
            }
            let flags = if fl.is_empty() { String::from("none") } else { fl.join(", ") };
            l.f("Router flags", b, 1, format!("0x{:02x} ({})", r.flags, flags));
            l.f("Number of links", b + 2, 2, format!("{}", r.links.len()));
            let mut p = b + 4;
            for k in &r.links {
                let n = 12 + 4 * k.tos.len();
                let what = match k.kind {
                    ospf::LINK_STUB => format!("{} {}/{}, metric {}", ospf_link_kind(k.kind), k.id, k.data, k.metric),
                    ospf::LINK_TRANSIT => format!("{} with DR {}, interface {}, metric {}", ospf_link_kind(k.kind), k.id, k.data, k.metric),
                    _ => format!("{} to {}, interface {}, metric {}", ospf_link_kind(k.kind), k.id, k.data, k.metric),
                };
                l.f("Link", p, n, what);
                p += n;
            }
        }
        ospf::LsaBody::Network(n) => {
            l.f("Netmask", b, 4, format!("{}", n.mask));
            for (i, r) in n.routers.iter().enumerate() {
                l.f("Attached Router", b + 4 + 4 * i, 4, format!("{}", r));
            }
        }
        ospf::LsaBody::Summary(s) => {
            l.f("Netmask", b, 4, format!("{}", s.mask));
            let m = if s.metric == ospf::LS_INFINITY { String::from("16777215 (unreachable)") } else { format!("{}", s.metric) };
            l.f("Metric", b + 5, 3, m);
        }
        ospf::LsaBody::External(e) => {
            l.f("Netmask", b, 4, format!("{}", e.mask));
            l.f("External Type", b + 4, 1, String::from(if e.e2 { "Type 2 (metric is larger than any path inside the AS)" } else { "Type 1 (metric counts like an OSPF cost)" }));
            let m = if e.metric == ospf::LS_INFINITY { String::from("16777215 (unreachable)") } else { format!("{}", e.metric) };
            l.f("Metric", b + 5, 3, m);
            l.f("Forwarding Address", b + 8, 4, format!("{}", e.fwd));
            l.f("External Route Tag", b + 12, 4, format!("{}", e.tag));
        }
        ospf::LsaBody::Other(d) => {
            if !d.is_empty() {
                l.f("Data", b, d.len(), format!("{} bytes", d.len()));
            }
        }
    }
}

fn ospf_layer(frame: &[u8], off: usize, len: usize) -> B {
    let raw = &frame[off..off + len];
    let Some(v) = ospf::OspfView::parse(raw) else {
        let mut l = payload_layer(Proto::Ospf, off, len);
        l.bad(String::from("shorter than an OSPF header or than its length field"));
        return l;
    };
    let kind = v.kind();
    let mut l = B::new(Proto::Ospf, off, len, format!("Open Shortest Path First ({})", ospf::type_name(kind)));
    l.f("Version", off, 1, format!("{}", v.version()));
    l.f("Message Type", off + 1, 1, format!("{} ({})", ospf::type_name(kind), kind));
    l.f("Packet Length", off + 2, 2, format!("{}", v.len()));
    l.f("Source OSPF Router", off + 4, 4, format!("{}", v.router()));
    l.f("Area ID", off + 8, 4, ospf::area_text(v.area()));
    let au = v.autype();
    if au == ospf::AU_CRYPTO {
        l.f("Checksum", off + 12, 2, format!("0x{:04x} [none with cryptographic authentication]", v.checksum()));
    } else {
        let ok = v.checksum_ok();
        let want = if ok { None } else { Some(v.computed_checksum()) };
        l.f("Checksum", off + 12, 2, ck_str(v.checksum(), ok, want));
        if !ok {
            l.bad(format!("bad OSPF checksum (should be 0x{:04x})", v.computed_checksum()));
        }
    }
    let a = v.auth();
    let (auname, audata) = match au {
        ospf::AU_NULL => ("Null (0)", if a == [0; 8] { String::from("none") } else { hex_short(&a) }),
        ospf::AU_SIMPLE => ("Simple password (1)", String::from_utf8_lossy(&a[..a.iter().position(|&c| c == 0).unwrap_or(8)]).into_owned()),
        ospf::AU_CRYPTO => ("Cryptographic (2)", format!("key ID {}, digest length {}, sequence {}", a[2], a[3], crate::be32(&a, 4))),
        n => {
            l.bad(format!("unknown authentication type {}", n));
            ("Unknown", hex_short(&a))
        }
    };
    l.f("Auth Type", off + 14, 2, String::from(auname));
    l.f("Auth Data", off + 16, 8, audata);
    if raw.len() > v.len() {
        l.f("Trailing bytes", off + v.len(), raw.len() - v.len(), format!("{} bytes", raw.len() - v.len()));
    }
    if v.version() != ospf::VERSION {
        l.bad(format!("version {}: an OSPFv2 router ignores the packet", v.version()));
        return l;
    }
    let pkt = match ospf::Packet::read(&v) {
        Ok(p) => p,
        Err(e) => {
            l.bad(e.text());
            if v.len() > ospf::HDR_LEN {
                l.f("Data", off + ospf::HDR_LEN, v.len() - ospf::HDR_LEN, format!("{} bytes", v.len() - ospf::HDR_LEN));
            }
            return l;
        }
    };
    let b = off + ospf::HDR_LEN;
    match &pkt.body {
        ospf::Body::Hello(h) => {
            l.f("Network Mask", b, 4, format!("{}", h.mask));
            l.f("Hello Interval [sec]", b + 4, 2, format!("{}", h.interval));
            l.f("Options", b + 6, 1, format!("0x{:02x} ({})", h.options, ospf::options_text(h.options)));
            l.f("Router Priority", b + 7, 1, format!("{}", h.priority));
            l.f("Router Dead Interval [sec]", b + 8, 4, format!("{}", h.dead));
            l.f("Designated Router", b + 12, 4, format!("{}", h.dr));
            l.f("Backup Designated Router", b + 16, 4, format!("{}", h.bdr));
            for (i, n) in h.neighbors.iter().enumerate() {
                l.f("Active Neighbor", b + ospf::HELLO_LEN + 4 * i, 4, format!("{}", n));
            }
        }
        ospf::Body::Dd(d) => {
            l.f("Interface MTU", b, 2, format!("{}", d.mtu));
            l.f("Options", b + 2, 1, format!("0x{:02x} ({})", d.options, ospf::options_text(d.options)));
            l.f("DB Description", b + 3, 1, format!("0x{:02x} ({})", d.flags, ospf::dd_flags_text(d.flags)));
            l.f("DD Sequence", b + 4, 4, format!("0x{:08x} ({})", d.seq, d.seq));
            for (i, h) in d.lsas.iter().enumerate() {
                l.f("LSA Header", b + ospf::DD_LEN + ospf::LSA_HDR_LEN * i, ospf::LSA_HDR_LEN, format!("{}, checksum 0x{:04x}, len {}", ospf_lsa_head(h), h.checksum, h.len));
            }
        }
        ospf::Body::Lsr(keys) => {
            for (i, k) in keys.iter().enumerate() {
                l.f("Link State Request", b + ospf::LSR_ENTRY_LEN * i, ospf::LSR_ENTRY_LEN, format!("{} {} from {}", ospf::lsa_name(k.kind), k.id, k.adv));
            }
        }
        ospf::Body::Lsu(lsas) => {
            l.f("Number of LSAs", b, 4, format!("{}", lsas.len()));
            let spans = v.lsu_spans().unwrap_or_default();
            for (&(o, n), lsa) in spans.iter().zip(lsas) {
                ospf_lsa_fields(&mut l, &lsa[..n], off + o);
            }
            let end = spans.last().map(|&(o, n)| o + n).unwrap_or(ospf::HDR_LEN + 4);
            if end < v.len() {
                l.f("Data", off + end, v.len() - end, format!("{} bytes", v.len() - end));
                l.bad(format!("{} bytes after the last LSA", v.len() - end));
            }
        }
        ospf::Body::Ack(hs) => {
            for (i, h) in hs.iter().enumerate() {
                l.f("LSA Header", b + ospf::LSA_HDR_LEN * i, ospf::LSA_HDR_LEN, format!("{}, checksum 0x{:04x}, len {}", ospf_lsa_head(h), h.checksum, h.len));
            }
        }
    }
    l
}

/// "Echo (ping) request id=0x0001, seq=1, hop limit=64", "Neighbor Solicitation for 2001:db8::1
/// from 02:4a:00:00:01:01", "Neighbor Advertisement 2001:db8::1 (sol, ovr) is at ...", "Router
/// Advertisement from ..."
fn icmpv6_info(b: &[u8], hop: u8) -> String {
    let Some(v) = Icmp6View::parse(b) else {
        return String::from("ICMPv6");
    };
    let (t, c) = (v.typ(), v.code());
    if t == icmpv6::T_ECHO_REQUEST || t == icmpv6::T_ECHO_REPLY {
        return format!("{} id=0x{:04x}, seq={}, hop limit={}", icmpv6::name(t, c), v.id(), v.seq(), hop);
    }
    let Ok(nd) = icmpv6::Nd::parse(&v) else {
        return String::from(icmpv6::name(t, c));
    };
    let from = |m: Option<Mac>| m.map(|m| format!(" from {}", m)).unwrap_or_default();
    match nd.msg {
        NdMsg::RouterSolicit => format!("Router Solicitation{}", from(nd.src_lla())),
        NdMsg::RouterAdvert(_) => format!("Router Advertisement{}", from(nd.src_lla())),
        NdMsg::NeighborSolicit { target } => format!("Neighbor Solicitation for {}{}", target, from(nd.src_lla())),
        NdMsg::NeighborAdvert { router, solicited, override_, target } => {
            let fl = na_flags(router, solicited, override_);
            let mut s = format!("Neighbor Advertisement {}", target);
            if !fl.is_empty() {
                s.push_str(&format!(" ({})", fl));
            }
            if let Some(m) = nd.tgt_lla() {
                s.push_str(&format!(" is at {}", m));
            }
            s
        }
        NdMsg::Redirect { .. } => String::from("Redirect"),
    }
}

/// "Hello Packet, DR 10.0.0.1, BDR 10.0.0.2, 2 neighbors", "DB Description (I, M, MS), seq
/// 0x0000a1b2, 0 LSA headers", "LS Request, 2 LSAs", "LS Update, 3 LSAs", "LS Acknowledge, 1
/// LSA header"
fn ospf_info(b: &[u8]) -> Option<String> {
    let v = ospf::OspfView::parse(b)?;
    if v.version() != ospf::VERSION {
        return Some(format!("OSPF version {}", v.version()));
    }
    let n = |k: usize, one: &str, many: &str| format!("{} {}", k, if k == 1 { one } else { many });
    let Ok(p) = ospf::Packet::read(&v) else {
        return Some(String::from(ospf::type_name(v.kind())));
    };
    Some(match &p.body {
        ospf::Body::Hello(h) => {
            let mut s = String::from("Hello Packet");
            if !h.dr.is_unspecified() {
                s.push_str(&format!(", DR {}", h.dr));
            }
            if !h.bdr.is_unspecified() {
                s.push_str(&format!(", BDR {}", h.bdr));
            }
            s.push_str(&format!(", {}", n(h.neighbors.len(), "neighbor", "neighbors")));
            s
        }
        ospf::Body::Dd(d) => format!("DB Description ({}), seq 0x{:08x}, {}", ospf::dd_flags_text(d.flags), d.seq, n(d.lsas.len(), "LSA header", "LSA headers")),
        ospf::Body::Lsr(k) => format!("LS Request, {}", n(k.len(), "LSA", "LSAs")),
        ospf::Body::Lsu(l) => format!("LS Update, {}", n(l.len(), "LSA", "LSAs")),
        ospf::Body::Ack(h) => format!("LS Acknowledge, {}", n(h.len(), "LSA header", "LSA headers")),
    })
}

// ---------------------------------------------------------------- EAPOL

/// a byte string as hex, the first `max` bytes ("…" after them)
fn hex_head(b: &[u8], max: usize) -> String {
    let mut s = String::new();
    for x in b.iter().take(max) {
        s.push_str(&format!("{:02x}", x));
    }
    if b.len() > max {
        s.push('\u{2026}');
    }
    s
}

/// the names of the Key Information bits that are set ("Pairwise, Install, Ack, MIC")
fn key_info_text(info: u16) -> String {
    use crate::eapol::*;
    let mut v: Vec<&str> = Vec::new();
    for (bit, name) in [(KI_PAIRWISE, "Pairwise"), (KI_INSTALL, "Install"), (KI_ACK, "Ack"), (KI_MIC, "MIC"), (KI_SECURE, "Secure"), (KI_ERROR, "Error"), (KI_REQUEST, "Request"), (KI_ENCRYPTED, "Encrypted Key Data")] {
        if info & bit != 0 {
            v.push(name);
        }
    }
    if info & KI_PAIRWISE == 0 {
        v.insert(0, "Group");
    }
    v.join(", ")
}

/// the name of an AKM suite of 00-0F-AC
fn akm_name(a: u8) -> String {
    match a {
        crate::eapol::AKM_PSK => String::from("PSK (2)"),
        crate::eapol::AKM_PSK_SHA256 => String::from("PSK-SHA256 (6)"),
        crate::eapol::AKM_SAE => String::from("SAE (8)"),
        n => format!("{}", n),
    }
}

fn eapol_layer(frame: &[u8], off: usize, len: usize) -> B {
    use crate::eapol::{self, EapolView, Key};
    let b = &frame[off..off + len];
    let Some(v) = EapolView::parse(b) else {
        return payload_layer(Proto::Eapol, off, len);
    };
    let key = if v.kind() == eapol::T_KEY { Key::parse(v.body()) } else { None };
    let title = match key.as_ref().and_then(|k| k.message()) {
        Some(m) => format!("802.1X Authentication, EAPOL-Key (Message {} of 4)", m),
        None => format!("802.1X Authentication, {}", eapol::kind_name(v.kind())),
    };
    let mut l = B::new(Proto::Eapol, off, len, title);
    let ver = match v.version() {
        1 => String::from("802.1X-2001 (1)"),
        2 => String::from("802.1X-2004 (2)"),
        3 => String::from("802.1X-2010 (3)"),
        n => format!("{}", n),
    };
    l.f("Version", off, 1, ver);
    l.f("Type", off + 1, 1, format!("{} ({})", eapol::kind_name(v.kind()), v.kind()));
    l.f("Length", off + 2, 2, format!("{}", v.body_len()));
    let body = off + 4;
    let Some(k) = key else {
        if v.kind() == eapol::T_KEY {
            l.bad(String::from("not a valid EAPOL-Key body (RSN descriptor, 95 bytes and its key data)"));
        }
        if v.body_len() > 0 {
            l.f("Body", body, v.body_len(), format!("{} bytes", v.body_len()));
        }
        return l;
    };
    l.f("Key Descriptor Type", body, 1, String::from("EAPOL RSN Key (2)"));
    let kdv = match k.version() {
        eapol::KDV_AKM => String::from("AKM defined (0)"),
        1 => String::from("RC4 Cipher, HMAC-MD5 MIC (1)"),
        eapol::KDV_SHA1 => String::from("AES Cipher, HMAC-SHA1 MIC (2)"),
        eapol::KDV_CMAC => String::from("AES Cipher, AES-128-CMAC MIC (3)"),
        n => format!("{}", n),
    };
    l.f("Key Information", body + 1, 2, format!("0x{:04x} ({}; {})", k.info, kdv, key_info_text(k.info)));
    l.f("Key Length", body + 3, 2, format!("{}", k.key_len));
    l.f("Replay Counter", body + 5, 8, format!("{}", k.replay));
    l.f("Key Nonce", body + 13, 32, if k.nonce == [0; 32] { String::from("0 (none)") } else { hex_head(&k.nonce, 32) });
    l.f("Key IV", body + 45, 16, hex_head(&k.iv, 16));
    l.f("Key RSC", body + 61, 8, hex_head(&k.rsc, 8));
    l.f("Key ID", body + 69, 8, hex_head(&b[4 + 69..4 + 77], 8));
    l.f("Key MIC", body + eapol::MIC_OFF, eapol::MIC_LEN, if k.has(eapol::KI_MIC) { hex_head(&k.mic, 16) } else { String::from("0 (none)") });
    l.f("Key Data Length", body + 93, 2, format!("{}", k.data.len()));
    let doff = body + eapol::KEY_FIXED;
    if k.has(eapol::KI_ENCRYPTED) {
        if !k.data.is_empty() {
            l.f("WPA Key Data", doff, k.data.len(), format!("{} bytes, encrypted (AES key wrap)", k.data.len()));
        }
        return l;
    }
    // the elements of plain key data: the RSN element, KDEs
    let mut at = doff;
    for (id, e) in eapol::elements(&k.data) {
        match id {
            eapol::EID_RSN => {
                let what = match eapol::Rsne::parse(&k.data[at - doff..]) {
                    Some(r) => format!("RSN Information: CCMP, AKM {}, capabilities 0x{:04x}", akm_name(r.akm), r.caps),
                    None => String::from("RSN Information (unreadable)"),
                };
                l.f("RSN IE", at, e.len() + 2, what);
            }
            eapol::EID_VENDOR if e.len() >= 4 && e[..3] == eapol::OUI => {
                let what = match e[3] {
                    eapol::KDE_GTK => String::from("GTK"),
                    eapol::KDE_PMKID => format!("PMKID {}", hex_head(&e[4..], 16)),
                    eapol::KDE_IGTK => String::from("IGTK"),
                    t => format!("type {}", t),
                };
                l.f("Key Data Element", at, e.len() + 2, what);
            }
            _ => l.f("Element", at, e.len() + 2, format!("ID {}, {} bytes", id, e.len())),
        }
        at += e.len() + 2;
    }
    l
}

/// "Key (Message 1 of 4)", "EAPOL-Start"
fn eapol_info(b: &[u8]) -> String {
    use crate::eapol::{self, EapolView, Key};
    let Some(v) = EapolView::parse(b) else {
        return String::from("EAPOL");
    };
    if v.kind() != eapol::T_KEY {
        return String::from(eapol::kind_name(v.kind()));
    }
    match Key::parse(v.body()) {
        Some(k) => match k.message() {
            Some(m) => format!("Key (Message {} of 4)", m),
            None if k.has(eapol::KI_REQUEST) => String::from("Key (Request)"),
            None => String::from("Key (Group Message)"),
        },
        None => String::from("Key"),
    }
}

// ---------------------------------------------------------------- BGP

/// the attributes of an UPDATE as they lie: (offset, header length, value length, flags, type);
/// stops where one does not fit
fn bgp_attr_walk(b: &[u8]) -> Vec<(usize, usize, usize, u8, u8)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 <= b.len() {
        let (flags, kind) = (b[i], b[i + 1]);
        let ext = flags & bgp::F_EXTENDED != 0;
        let hl = if ext { 4 } else { 3 };
        if i + hl > b.len() {
            break;
        }
        let n = if ext { be16(b, i + 2) as usize } else { b[i + 2] as usize };
        if i + hl + n > b.len() {
            break;
        }
        out.push((i, hl, n, flags, kind));
        i += hl + n;
    }
    out
}

/// "Optional, Transitive, Partial"; "Well-known, Transitive"
fn bgp_flags_text(f: u8) -> String {
    let mut v: Vec<&str> = alloc::vec![if f & bgp::F_OPTIONAL != 0 { "Optional" } else { "Well-known" }];
    if f & bgp::F_TRANSITIVE != 0 {
        v.push("Transitive");
    }
    if f & bgp::F_PARTIAL != 0 {
        v.push("Partial");
    }
    if f & bgp::F_EXTENDED != 0 {
        v.push("Extended Length");
    }
    v.join(", ")
}

fn bgp_attr_text(v: &bgp::AttrVal) -> String {
    use crate::bgp::AttrVal;
    match v {
        AttrVal::Origin(o) => String::from(bgp::origin_name(*o)),
        AttrVal::AsPath(s) | AttrVal::As4Path(s) => {
            if s.is_empty() {
                String::from("empty")
            } else {
                bgp::path_text(s)
            }
        }
        AttrVal::NextHop(a) => format!("{}", a),
        AttrVal::Med(m) | AttrVal::LocalPref(m) => format!("{}", m),
        AttrVal::AtomicAggregate => String::from("set"),
        AttrVal::Aggregator { asn, id } | AttrVal::As4Aggregator { asn, id } => format!("AS {}, {}", asn, id),
        AttrVal::Communities(c) => {
            let v: Vec<String> = c.iter().map(|x| bgp::community_text(*x)).collect();
            v.join(" ")
        }
        AttrVal::Other { kind, data } => format!("type {}, {} bytes", kind, data.len()),
    }
}

/// the fields of one message `m` (complete, at frame offset `o`); its kind's summary
fn bgp_message(l: &mut B, m: &[u8], o: usize) {
    let kind = m[18];
    let marker = if m[..16] == bgp::MARKER { String::from("ffffffffffffffffffffffffffffffff") } else { String::from("not all ones [incorrect]") };
    l.f("Marker", o, 16, marker);
    l.f("Length", o + 16, 2, format!("{}", m.len()));
    l.f("Type", o + 18, 1, format!("{} Message ({})", bgp::type_name(kind), kind));
    let body = &m[bgp::HDR_LEN..];
    let bo = o + bgp::HDR_LEN;
    let as4 = match kind {
        bgp::T_UPDATE => {
            let wl = if body.len() >= 2 { be16(body, 0) as usize } else { 0 };
            let ab = body.get(4 + wl..).unwrap_or(&[]);
            let al = body.get(2 + wl..4 + wl).map(|x| be16(x, 0) as usize).unwrap_or(0);
            let ab = ab.get(..al.min(ab.len())).unwrap_or(&[]);
            bgp_attr_walk(ab).iter().find(|a| a.4 == bgp::A_AS_PATH).map(|a| bgp::as_path_4(&ab[a.0 + a.1..a.0 + a.1 + a.2])).unwrap_or(true)
        }
        _ => true,
    };
    if let Err(e) = bgp::Message::read(m, as4) {
        l.bad(format!("{}: {} (the receiver answers with NOTIFICATION {}/{})", bgp::type_name(kind), e.text(), e.code, e.sub));
    }
    match kind {
        bgp::T_OPEN if body.len() >= 10 => {
            l.f("Version", bo, 1, format!("{}", body[0]));
            let asn = be16(body, 1);
            l.f("My AS", bo + 1, 2, if asn == bgp::AS_TRANS { format!("{} (AS_TRANS: the AS is in a capability)", asn) } else { format!("{}", asn) });
            l.f("Hold Time", bo + 3, 2, format!("{} s", be16(body, 3)));
            l.f("BGP Identifier", bo + 5, 4, format!("{}", Ipv4([body[5], body[6], body[7], body[8]])));
            l.f("Optional Parameters Length", bo + 9, 1, format!("{}", body[9]));
            let mut i = 10;
            while i + 2 <= body.len() {
                let (t, n) = (body[i], body[i + 1] as usize);
                let Some(v) = body.get(i + 2..i + 2 + n) else {
                    break;
                };
                if t != bgp::OPT_CAPS {
                    l.f("Optional Parameter", bo + i, 2 + n, format!("type {}, {} bytes", t, n));
                    i += 2 + n;
                    continue;
                }
                let mut j = 0;
                while j + 2 <= v.len() {
                    let (c, k) = (v[j], v[j + 1] as usize);
                    let Some(x) = v.get(j + 2..j + 2 + k) else {
                        break;
                    };
                    let what = match (c, k) {
                        (bgp::CAP_MP, 4) => format!("{}: {}", bgp::cap_name(c), bgp::afi_name(be16(x, 0), x[3])),
                        (bgp::CAP_AS4, 4) => format!("{}: {}", bgp::cap_name(c), crate::be32(x, 0)),
                        (_, 0) => format!("{} ({})", bgp::cap_name(c), c),
                        _ => format!("{} ({}), {} bytes", bgp::cap_name(c), c, k),
                    };
                    l.f("Capability", bo + i + 2 + j, 2 + k, what);
                    j += 2 + k;
                }
                i += 2 + n;
            }
        }
        bgp::T_UPDATE if body.len() >= 4 => {
            let wl = be16(body, 0) as usize;
            l.f("Withdrawn Routes Length", bo, 2, format!("{}", wl));
            let w = body.get(2..2 + wl).unwrap_or(&[]);
            let mut i = 0;
            while i < w.len() {
                let n = 1 + (w[i] as usize).div_ceil(8);
                match bgp::read_prefixes(&w[i..(i + n).min(w.len())]) {
                    Ok(p) if !p.is_empty() => l.f("Withdrawn Route", bo + 2 + i, n.min(w.len() - i), format!("{}", p[0])),
                    _ => {
                        l.f("Withdrawn Route", bo + 2 + i, w.len() - i, String::from("invalid"));
                        break;
                    }
                }
                i += n;
            }
            let Some(al) = body.get(2 + wl..4 + wl).map(|x| be16(x, 0) as usize) else {
                return;
            };
            l.f("Total Path Attribute Length", bo + 2 + wl, 2, format!("{}", al));
            let ao = 4 + wl;
            let ab = body.get(ao..(ao + al).min(body.len())).unwrap_or(&[]);
            let parsed = bgp::Update::read(body, as4).ok();
            for (k, &(i, hl, n, flags, t)) in bgp_attr_walk(ab).iter().enumerate() {
                let val = match parsed.as_ref().and_then(|u| u.attrs.get(k)) {
                    Some(a) => bgp_attr_text(&a.val),
                    None => format!("{} bytes", n),
                };
                let name = match t {
                    bgp::A_ORIGIN..=bgp::A_COMMUNITIES | bgp::A_AS4_PATH | bgp::A_AS4_AGGREGATOR => bgp::attr_name(t),
                    _ => "Path Attribute",
                };
                l.f(name, bo + ao + i, hl + n, format!("{} [{}]", val, bgp_flags_text(flags)));
            }
            let nl = body.get(ao + al..).unwrap_or(&[]);
            let mut i = 0;
            while i < nl.len() {
                let n = 1 + (nl[i] as usize).div_ceil(8);
                match bgp::read_prefixes(&nl[i..(i + n).min(nl.len())]) {
                    Ok(p) if !p.is_empty() => l.f("NLRI", bo + ao + al + i, n.min(nl.len() - i), format!("{}", p[0])),
                    _ => {
                        l.f("NLRI", bo + ao + al + i, nl.len() - i, String::from("invalid"));
                        break;
                    }
                }
                i += n;
            }
        }
        bgp::T_NOTIFICATION if body.len() >= 2 => {
            l.f("Error Code", bo, 1, format!("{} ({})", bgp::code_name(body[0]), body[0]));
            l.f("Error Subcode", bo + 1, 1, format!("{} ({})", bgp::error_name(body[0], body[1]), body[1]));
            if body.len() > 2 {
                l.f("Data", bo + 2, body.len() - 2, hex_short(&body[2..]));
            }
        }
        bgp::T_ROUTE_REFRESH if body.len() >= 4 => l.f("Address Family", bo, 4, bgp::afi_name(be16(body, 0), body[3])),
        _ => {}
    }
}

fn bgp_layer(frame: &[u8], off: usize, len: usize) -> B {
    let b = &frame[off..off + len];
    let mut l = B::new(Proto::Bgp, off, len, String::new());
    let mut kinds: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let rest = &b[i..];
        if (rest.len() >= 16 && rest[..16] != bgp::MARKER) || (rest.len() < 16 && !bgp::MARKER.starts_with(rest)) {
            // the rest of a message an earlier segment began
            l.f("Continuation", off + i, rest.len(), format!("{} bytes of a message", rest.len()));
            kinds.push("Continuation");
            break;
        }
        match bgp::frame(rest) {
            Ok(Some(n)) => {
                bgp_message(&mut l, &rest[..n], off + i);
                kinds.push(bgp::type_name(rest[18]));
                i += n;
            }
            Ok(None) => {
                l.f("Segment of a message", off + i, rest.len(), format!("{} bytes, the rest follows", rest.len()));
                kinds.push("Segment");
                break;
            }
            Err(e) => {
                l.f("Data", off + i, rest.len(), format!("{} bytes", rest.len()));
                l.bad(format!("{} (the receiver answers with NOTIFICATION {}/{})", e.text(), e.code, e.sub));
                break;
            }
        }
    }
    let msgs: Vec<String> = kinds.iter().map(|k| format!("{} Message", k)).collect();
    l.l.title = format!("Border Gateway Protocol - {}", msgs.join(", "));
    l
}

/// one message in a few words ("UPDATE Message, 2 routes, 1 withdrawn")
fn bgp_msg_info(m: &[u8]) -> String {
    let as4 = true;
    let kind = m[18];
    let name = bgp::type_name(kind);
    match bgp::Message::read(m, as4).or_else(|_| bgp::Message::read(m, false)) {
        Ok(bgp::Message::Open(o)) => format!("OPEN Message, AS {}, hold time {}, ID {}", o.peer_as(), o.hold, o.id),
        Ok(bgp::Message::Update(u)) => {
            let mut s = String::from("UPDATE Message");
            if !u.nlri.is_empty() {
                s.push_str(&format!(", {} route{}", u.nlri.len(), if u.nlri.len() == 1 { "" } else { "s" }));
            }
            if !u.withdrawn.is_empty() {
                s.push_str(&format!(", {} withdrawn", u.withdrawn.len()));
            }
            if u.nlri.is_empty() && u.withdrawn.is_empty() {
                s.push_str(", nothing (End-of-RIB)");
            }
            s
        }
        Ok(bgp::Message::Notification(n)) => format!("NOTIFICATION Message, {}/{} {}", n.code, n.sub, bgp::error_name(n.code, n.sub)),
        Ok(bgp::Message::Keepalive) => String::from("KEEPALIVE Message"),
        Ok(bgp::Message::RouteRefresh { afi, safi }) => format!("ROUTE-REFRESH Message, {}", bgp::afi_name(afi, safi)),
        Err(e) => format!("{} Message ({})", name, bgp::error_name(e.code, e.sub)),
    }
}

/// the messages of a segment, one after the other
fn bgp_info(b: &[u8]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let rest = &b[i..];
        if rest.len() < 16 || rest[..16] != bgp::MARKER {
            parts.push(String::from("Continuation"));
            break;
        }
        match bgp::frame(rest) {
            Ok(Some(n)) => {
                parts.push(bgp_msg_info(&rest[..n]));
                i += n;
            }
            Ok(None) => {
                parts.push(String::from("Segment of a message"));
                break;
            }
            Err(e) => {
                parts.push(format!("Malformed ({})", bgp::error_name(e.code, e.sub)));
                break;
            }
        }
    }
    parts.join(", ")
}

// ---------------------------------------------------------------- TFTP and FTP

/// what a TFTP field holds, for the dissector's problems
fn tftp_bad(e: tftp::Bad) -> &'static str {
    match e {
        tftp::Bad::Short => "shorter than its TFTP header",
        tftp::Bad::Opcode(_) => "an opcode TFTP does not define",
        tftp::Bad::Unterminated => "a string field without its terminating zero",
        tftp::Bad::OddOptions => "an option without a value",
    }
}

fn tftp_layer(frame: &[u8], off: usize, len: usize) -> B {
    let b = &frame[off..off + len];
    let v = match tftp::TftpView::parse(b) {
        Ok(v) => v,
        Err(e) => {
            let mut l = payload_layer(Proto::Tftp, off, len);
            if b.len() >= 2 {
                l.f("Opcode", off, 2, format!("{} ({})", tftp::op_name(be16(b, 0)), be16(b, 0)));
            }
            l.bad(String::from(tftp_bad(e)));
            return l;
        }
    };
    let op = v.opcode();
    let mut l = B::new(Proto::Tftp, off, len, format!("Trivial File Transfer Protocol, {}", tftp::op_name(op)));
    l.f("Opcode", off, 2, format!("{} ({})", tftp::op_name(op), op));
    let opts = |l: &mut B, f: &[tftp::Span]| {
        for c in f.chunks(2) {
            if let [k, val] = c {
                l.f("Option", off + k.0, val.0 + val.1 + 1 - k.0, format!("{} = {}", v.text(*k), v.text(*val)));
            }
        }
    };
    match op {
        tftp::OP_RRQ | tftp::OP_WRQ => {
            let f = v.fields();
            if let [file, mode, ..] = f[..] {
                l.f(if op == tftp::OP_RRQ { "Source File" } else { "Destination File" }, off + file.0, file.1 + 1, v.text(file));
                let m = v.text(mode);
                l.f("Type", off + mode.0, mode.1 + 1, m.clone());
                if !tftp::mode_known(&m) {
                    l.bad(format!("unknown transfer mode \"{}\"", m));
                }
                opts(&mut l, &f[2..]);
            }
        }
        tftp::OP_DATA => {
            l.f("Block", off + 2, 2, format!("{}", v.block()));
            let n = v.data().len();
            l.f("Data", off + tftp::TFTP_HDR, n, format!("{} bytes", n));
        }
        tftp::OP_ACK => l.f("Block", off + 2, 2, format!("{}", v.block())),
        tftp::OP_ERROR => {
            let c = v.error_code();
            l.f("Error Code", off + 2, 2, format!("{} ({})", tftp::error_name(c), c));
            l.f("Error Message", off + tftp::TFTP_HDR, len - tftp::TFTP_HDR, v.error_msg());
            if c > tftp::ERR_OPTION {
                l.bad(format!("an error code TFTP does not define ({})", c));
            }
        }
        _ => opts(&mut l, &v.fields()),
    }
    l
}

/// "Read Request, File: r1-config, Transfer type: octet", "Data Packet, Block: 1 (last)",
/// "Error Code, Code: File not found, Message: ..."
fn tftp_info(b: &[u8]) -> String {
    let Ok(p) = tftp::Packet::parse(b) else {
        return String::from("TFTP");
    };
    let opts = |o: &[(String, String)]| -> String { o.iter().map(|(k, v)| format!(", {}={}", k, v)).collect() };
    match &p {
        tftp::Packet::Rrq { file, mode, opts: o } | tftp::Packet::Wrq { file, mode, opts: o } => format!("{}, File: {}, Transfer type: {}{}", tftp::op_name(p.opcode()), file, mode, opts(o)),
        tftp::Packet::Data { block, data } => format!("Data Packet, Block: {}{}", block, if data.len() < tftp::BLOCK { " (last)" } else { "" }),
        tftp::Packet::Ack { block } => format!("Acknowledgement, Block: {}", block),
        tftp::Packet::Error { code, msg } => format!("Error Code, Code: {}, Message: {}", tftp::error_name(*code), msg),
        tftp::Packet::Oack { opts: o } => format!("Option Acknowledgement{}", opts(o)),
    }
}

/// the segment of an FTP control connection comes from the server
fn ftp_response(st: &Stack, frame: &[u8]) -> bool {
    match st.find(Proto::Tcp).and_then(|(o, _)| TcpView::parse(&frame[o..])) {
        Some(t) => t.src_port() == ftp::PORT_FTP && t.dst_port() != ftp::PORT_FTP,
        None => false,
    }
}

fn ftp_layer(frame: &[u8], off: usize, len: usize, response: bool) -> B {
    let b = &frame[off..off + len];
    let mut l = B::new(Proto::Ftp, off, len, String::from("File Transfer Protocol (FTP)"));
    let mut p = 0;
    while p < b.len() {
        let Some(i) = b[p..].iter().position(|&c| c == b'\n') else {
            l.f("Continuation", off + p, b.len() - p, format!("{} bytes", b.len() - p));
            break;
        };
        let end = p + i + 1;
        let e = if i > 0 && b[p + i - 1] == b'\r' { p + i - 1 } else { p + i };
        let line = String::from_utf8_lossy(&b[p..e]).into_owned();
        if response {
            l.f("Response", off + p, end - p, line.clone());
            let d = line.as_bytes();
            if d.len() >= 3 && d[..3].iter().all(|c| c.is_ascii_digit()) {
                l.f("Response code", off + p, 3, String::from(&line[..3]));
                if line.len() > 4 {
                    l.f("Response arg", off + p + 4, e - p - 4, String::from(&line[4..]));
                }
            }
        } else {
            l.f("Request", off + p, end - p, line.clone());
            if let Some(c) = ftp::Command::parse(&line) {
                let vl = line.trim_start().split(' ').next().map_or(0, str::len);
                let lead = line.len() - line.trim_start().len();
                l.f("Request command", off + p + lead, vl, c.verb.clone());
                if !c.arg.is_empty() {
                    l.f("Request arg", off + p + lead + vl + 1, e - (p + lead + vl + 1), c.arg.clone());
                }
                if !ftp::verb_known(&c.verb) {
                    l.bad(format!("a command FTP does not define: {}", c.verb));
                }
            }
        }
        p = end;
    }
    l
}

/// "Request: USER anna", "Response: 230 Logged in." (the first line of the segment)
fn ftp_info(b: &[u8], response: bool) -> String {
    let e = b.iter().position(|&c| c == b'\n').unwrap_or(b.len());
    let line = String::from_utf8_lossy(&b[..e]);
    format!("{}: {}", if response { "Response" } else { "Request" }, line.trim_end())
}

// ---------------------------------------------------------------- mail: SMTP and POP3

/// the segment of an SMTP or POP3 connection comes from the server (its well-known port is the
/// source and not the destination)
fn mail_response(st: &Stack, frame: &[u8]) -> bool {
    let server = |p: u16| p == smtp::PORT_SMTP || p == smtp::PORT_SUBMISSION || p == pop3::PORT_POP3;
    match st.find(Proto::Tcp).and_then(|(o, _)| TcpView::parse(&frame[o..])) {
        Some(t) => server(t.src_port()) && !server(t.dst_port()),
        None => false,
    }
}

/// the lines of an application layer: (start, end without the line end, end with it)
fn lines_of(b: &[u8]) -> Vec<(usize, usize, usize)> {
    let mut v = Vec::new();
    let mut p = 0;
    while p < b.len() {
        match b[p..].iter().position(|&c| c == b'\n') {
            Some(i) => {
                let end = p + i + 1;
                let e = if i > 0 && b[p + i - 1] == b'\r' { p + i - 1 } else { p + i };
                v.push((p, e, end));
                p = end;
            }
            None => {
                v.push((p, b.len(), b.len()));
                break;
            }
        }
    }
    v
}

/// the request of a line: (verb, its length, where the parameter starts) when it starts with a
/// command the protocol knows
fn mail_verb(line: &str, known: fn(&str) -> bool) -> Option<(String, usize, usize)> {
    let lead = line.len() - line.trim_start().len();
    let w = line[lead..].split(' ').next()?;
    if w.is_empty() || !known(w) {
        return None;
    }
    Some((w.to_ascii_uppercase(), w.len(), lead + w.len() + 1))
}

fn smtp_layer(frame: &[u8], off: usize, len: usize, response: bool) -> B {
    let b = &frame[off..off + len];
    let mut l = B::new(Proto::Smtp, off, len, String::from("Simple Mail Transfer Protocol"));
    for (s, e, end) in lines_of(b) {
        if end == b.len() && e == end && !b.ends_with(b"\n") {
            l.f("Continuation", off + s, end - s, format!("{} bytes", end - s));
            break;
        }
        let line = String::from_utf8_lossy(&b[s..e]).into_owned();
        if e - s > 998 {
            l.bad(String::from("a line longer than 1000 bytes (RFC 5321 4.5.3.1.6)"));
        }
        if response {
            l.f("Response", off + s, end - s, line.clone());
            let d = line.as_bytes();
            if d.len() >= 3 && d[..3].iter().all(|c| c.is_ascii_digit()) && (d.len() == 3 || d[3] == b' ' || d[3] == b'-') {
                l.f("Response code", off + s, 3, String::from(&line[..3]));
                let rest = line.get(4..).unwrap_or("");
                let r = smtp::Reply::new(0, rest);
                let mut at = 4;
                if let Some(st) = r.status() {
                    l.f("Enhanced status code", off + s + 4, st.len(), String::from(st));
                    at += st.len() + 1;
                }
                if line.len() > at {
                    l.f("Response parameter", off + s + at, e - s - at, String::from(&line[at..]));
                }
            } else {
                l.bad(String::from("a reply line without its code"));
            }
        } else if line == "." {
            l.f("End of message", off + s, end - s, String::from("."));
        } else if let Some((verb, vl, at)) = mail_verb(&line, smtp::verb_known) {
            l.f("Command Line", off + s, end - s, line.clone());
            let lead = line.len() - line.trim_start().len();
            l.f("Command", off + s + lead, vl, verb);
            if line.len() > at {
                l.f("Request parameter", off + s + at, e - s - at, String::from(&line[at..]));
            }
        } else {
            l.f("Message line", off + s, end - s, line.clone());
        }
    }
    l
}

/// "C: EHLO pc1", "S: 250-mail.a.lab ...", "C: DATA fragment, 345 bytes" (a segment of the
/// message after DATA)
fn smtp_info(b: &[u8], response: bool) -> String {
    let e = b.iter().position(|&c| c == b'\n').unwrap_or(b.len());
    let line = String::from_utf8_lossy(&b[..e]);
    let line = line.trim_end();
    if response {
        return format!("S: {}", line);
    }
    if mail_verb(line, smtp::verb_known).is_some() {
        return format!("C: {}", line);
    }
    format!("C: DATA fragment, {} bytes", b.len())
}

fn pop3_layer(frame: &[u8], off: usize, len: usize, response: bool) -> B {
    let b = &frame[off..off + len];
    let mut l = B::new(Proto::Pop3, off, len, String::from("Post Office Protocol"));
    for (s, e, end) in lines_of(b) {
        if end == b.len() && e == end && !b.ends_with(b"\n") {
            l.f("Continuation", off + s, end - s, format!("{} bytes", end - s));
            break;
        }
        let line = String::from_utf8_lossy(&b[s..e]).into_owned();
        if response {
            let ind = if line.starts_with("+OK") { Some("+OK") } else if line.starts_with("-ERR") { Some("-ERR") } else { None };
            match ind {
                Some(i) => {
                    l.f("Response", off + s, end - s, line.clone());
                    l.f("Response indicator", off + s, i.len(), String::from(i));
                    if line.len() > i.len() + 1 {
                        l.f("Response description", off + s + i.len() + 1, e - s - i.len() - 1, String::from(&line[i.len() + 1..]));
                    }
                }
                None if line == "." => l.f("End of data", off + s, end - s, String::from(".")),
                None => l.f("Data line", off + s, end - s, line.clone()),
            }
        } else {
            l.f("Request", off + s, end - s, line.clone());
            match mail_verb(&line, pop3::verb_known) {
                Some((verb, vl, at)) => {
                    let lead = line.len() - line.trim_start().len();
                    l.f("Request command", off + s + lead, vl, verb);
                    if line.len() > at {
                        l.f("Request parameter", off + s + at, e - s - at, String::from(&line[at..]));
                    }
                }
                None => {
                    let w = line.trim_start().split(' ').next().unwrap_or("");
                    l.bad(format!("a command POP3 does not define: {}", w));
                }
            }
        }
    }
    l
}

/// "C: USER bob", "S: +OK 2 messages", "S: DATA fragment, 1200 bytes" (the rest of a
/// multi-line response)
fn pop3_info(b: &[u8], response: bool) -> String {
    let e = b.iter().position(|&c| c == b'\n').unwrap_or(b.len());
    let line = String::from_utf8_lossy(&b[..e]);
    let line = line.trim_end();
    if !response {
        return format!("C: {}", line);
    }
    if line.starts_with("+OK") || line.starts_with("-ERR") {
        return format!("S: {}", line);
    }
    format!("S: DATA fragment, {} bytes", b.len())
}

// ---------------------------------------------------------------- SNMP

/// "SNMPv2-MIB::sysName.0 (1.3.6.1.2.1.1.5.0)"
fn snmp_name(o: &ber::Oid) -> String {
    let n = mib::name_of(o.arcs(), false);
    if n == format!("{}", o) { n } else { format!("{} ({})", n, o) }
}

fn snmp_layer(frame: &[u8], off: usize, len: usize) -> B {
    let b = &frame[off..off + len];
    let mut l = B::new(Proto::Snmp, off, len, String::from("Simple Network Management Protocol"));
    if let Err(e) = snmp_fields(&mut l, b, off) {
        l.bad(e.text());
    }
    l
}

/// the fields of a message in order; the first element that does not hold ends them
fn snmp_fields(l: &mut B, b: &[u8], off: usize) -> Result<(), snmp::Bad> {
    let mut top = ber::Reader::new(b);
    let mut m = top.seq(ber::T_SEQUENCE)?;
    let vt = m.peek()?;
    let vn = m.int()?;
    let version = snmp::Version::of_number(vn);
    let vname = match version {
        Some(v) => format!("v{} ({})", v.word(), vn),
        None if vn == 3 => format!("v3 ({})", vn),
        None => format!("{}", vn),
    };
    l.f("Version", off + vt.off, vt.hdr + vt.len, vname.clone());
    let Some(version) = version else {
        return Err(snmp::Bad::Version(vn));
    };
    let ct = m.peek()?;
    let community = m.octets()?;
    l.f("Community", off + ct.off, ct.hdr + ct.len, String::from_utf8_lossy(community).into_owned());
    let pt = m.peek()?;
    let Some(kind) = snmp::Kind::of_tag(pt.tag) else {
        l.f("PDU", off + pt.off, pt.hdr, format!("unknown (0x{:02x})", pt.tag));
        return Err(snmp::Bad::Pdu(pt.tag));
    };
    l.l.title = format!("Simple Network Management Protocol, {}, {}", vname.split(' ').next().unwrap_or(""), kind.name());
    l.f("PDU", off + pt.off, pt.hdr, format!("{} (0x{:02x})", kind.name(), pt.tag));
    let fits = if version == snmp::Version::V1 { kind.in_v1() } else { kind.in_v2c() };
    if !fits {
        return Err(snmp::Bad::Pdu(pt.tag));
    }
    let mut p = m.seq(pt.tag)?;
    if kind == snmp::Kind::TrapV1 {
        let t = p.peek()?;
        let ent = p.oid()?;
        l.f("Enterprise", off + t.off, t.hdr + t.len, snmp_name(&ent));
        let t = p.expect(ber::T_IPADDRESS)?;
        let a = t.contents(b);
        if a.len() != 4 {
            return Err(snmp::Bad::Ber(ber::Bad::Size));
        }
        l.f("Agent address", off + t.off, t.hdr + t.len, format!("{}", Ipv4([a[0], a[1], a[2], a[3]])));
        let t = p.peek()?;
        let g = p.int32()?;
        l.f("Generic trap", off + t.off, t.hdr + t.len, format!("{} ({})", snmp::generic_name(g), g));
        let t = p.peek()?;
        let sp = p.int32()?;
        l.f("Specific trap", off + t.off, t.hdr + t.len, format!("{}", sp));
        let t = p.peek()?;
        let ts = p.uint(ber::T_TIMETICKS, u32::MAX as u64)? as u32;
        l.f("Time stamp", off + t.off, t.hdr + t.len, mib::ticks_text(ts));
    } else {
        let t = p.peek()?;
        let id = p.int32()?;
        l.f("Request ID", off + t.off, t.hdr + t.len, format!("{}", id));
        let t = p.peek()?;
        let es = p.int32()?;
        let t2 = p.peek()?;
        let ei = p.int32()?;
        if kind == snmp::Kind::GetBulk {
            l.f("Non-repeaters", off + t.off, t.hdr + t.len, format!("{}", es));
            l.f("Max-repetitions", off + t2.off, t2.hdr + t2.len, format!("{}", ei));
        } else {
            l.f("Error status", off + t.off, t.hdr + t.len, format!("{} ({})", snmp::error_name(es), es));
            l.f("Error index", off + t2.off, t2.hdr + t2.len, format!("{}", ei));
            if !(0..=snmp::E_INCONSISTENT_NAME).contains(&es) {
                l.bad(format!("an error status SNMP does not define ({})", es));
            }
        }
    }
    let lt = p.peek()?;
    let mut list = p.seq(ber::T_SEQUENCE)?;
    let at = l.l.fields.len();
    let mut n = 0;
    while !list.is_empty() {
        let mut vb = list.seq(ber::T_SEQUENCE)?;
        let ot = vb.peek()?;
        let oid = vb.oid()?;
        l.f("Object Name", off + ot.off, ot.hdr + ot.len, snmp_name(&oid));
        let vt = vb.peek()?;
        let value = vb.value()?;
        l.f("Value", off + vt.off, vt.hdr + vt.len, mib::show(oid.arcs(), &value, false));
        if version == snmp::Version::V1 && (matches!(value, ber::Value::Counter64(_)) || value.is_exception()) {
            return Err(snmp::Bad::V1Type(vt.tag));
        }
        vb.done()?;
        n += 1;
    }
    l.l.fields.insert(at, Field { name: "Variable bindings", value: format!("{} binding{}", n, if n == 1 { "" } else { "s" }), off: (off + lt.off) as u32, len: (lt.hdr + lt.len) as u32 });
    p.done()?;
    m.done()?;
    top.done()?;
    Ok(())
}

/// "GetRequest 1234: SNMPv2-MIB::sysName.0", "Response 1234: SNMPv2-MIB::sysName.0 = STRING: R1",
/// "Response 1234: noSuchName (index 1)", "SNMPv2-Trap 7: IF-MIB::linkDown", "Trap (v1): linkDown"
fn snmp_info(b: &[u8]) -> String {
    let m = match snmp::Message::parse(b) {
        Ok(m) => m,
        Err(e) => return format!("SNMP (malformed: {})", e.text()),
    };
    let first = |vbs: &[snmp::VarBind], value: bool| -> String {
        let Some(v) = vbs.first() else {
            return String::new();
        };
        let mut s = mib::name_of(v.oid.arcs(), false);
        if value {
            s.push_str(" = ");
            s.push_str(&mib::show(v.oid.arcs(), &v.value, false));
        }
        if vbs.len() > 1 {
            s.push_str(&format!(" (+{})", vbs.len() - 1));
        }
        s
    };
    match &m.body {
        snmp::Body::Trap(t) => format!("Trap (v1): {}, agent {}", snmp::generic_name(t.generic), t.agent_addr),
        snmp::Body::Pdu(p) => match p.kind {
            snmp::Kind::TrapV2 | snmp::Kind::Inform => {
                let what = snmp::notification(&p.varbinds).map(|o| mib::name_of(o.arcs(), false)).unwrap_or_else(|| String::from("?"));
                format!("{} {}: {}", p.kind.name(), p.request_id, what)
            }
            snmp::Kind::Response if p.error_status != 0 => format!("Response {}: {} (index {})", p.request_id, snmp::error_name(p.error_status), p.error_index),
            snmp::Kind::Response | snmp::Kind::Set => format!("{} {}: {}", p.kind.name(), p.request_id, first(&p.varbinds, true)),
            snmp::Kind::GetBulk => format!("GetBulkRequest {}: N={} M={} {}", p.request_id, p.non_repeaters(), p.max_repetitions(), first(&p.varbinds, false)),
            k => format!("{} {}: {}", k.name(), p.request_id, first(&p.varbinds, false)),
        },
    }
}

// ---------------------------------------------------------------- RADIUS and Telnet

/// why a receiver discards bytes as RADIUS
fn radius_bad(e: radius::Bad) -> String {
    match e {
        radius::Bad::Short => String::from("shorter than its RADIUS header or its Length field"),
        radius::Bad::Length(l) => format!("a Length of {} (20 to 4096 allowed)", l),
        radius::Bad::Attr(at) => format!("an attribute at byte {} does not end inside the packet", at),
    }
}

fn radius_layer(frame: &[u8], off: usize, len: usize) -> B {
    let b = &frame[off..off + len];
    let v = match radius::RadiusView::parse(b) {
        Ok(v) => v,
        Err(e) => {
            let mut l = payload_layer(Proto::Radius, off, len);
            l.l.title = String::from("RADIUS Protocol");
            if let Some(&c) = b.first() {
                l.f("Code", off, 1, format!("{} ({})", radius::code_name(c), c));
            }
            l.bad(radius_bad(e));
            return l;
        }
    };
    let mut l = B::new(Proto::Radius, off, len, format!("RADIUS Protocol, {} (id {})", radius::code_name(v.code()), v.id()));
    l.f("Code", off, 1, format!("{} ({})", radius::code_name(v.code()), v.code()));
    l.f("Packet identifier", off + 1, 1, format!("0x{:02x} ({})", v.id(), v.id()));
    l.f("Length", off + 2, 2, format!("{}", v.length()));
    let what = if matches!(v.code(), radius::ACCESS_REQUEST | radius::ACCOUNTING_REQUEST | radius::STATUS_SERVER) { "Request Authenticator" } else { "Response Authenticator" };
    let mut a = String::new();
    for x in v.authenticator() {
        a.push_str(&format!("{:02x}", x));
    }
    l.f(what, off + 4, 16, a);
    for (at, t, val) in v.attrs() {
        l.f("Attribute", off + at, val.len() + 2, format!("{} ({}): {}", radius::attr_name(t), t, radius::value_text(t, val)));
        if t == radius::A_MESSAGE_AUTHENTICATOR && val.len() != 16 {
            l.bad(format!("a Message-Authenticator of {} bytes (16 are right)", val.len()));
        }
    }
    if v.length() as usize > len {
        l.bad(String::from("shorter than its Length field"));
    } else if (v.length() as usize) < len {
        let pad = len - v.length() as usize;
        l.f("Padding", off + v.length() as usize, pad, format!("{} bytes", pad));
    }
    l
}

/// "Access-Request id=7 (admin)", "Accounting-Request id=9 (Start, admin)", "Access-Accept id=7"
fn radius_info(b: &[u8]) -> String {
    let Ok(p) = radius::Packet::parse(b) else {
        return String::from("RADIUS (malformed)");
    };
    let mut s = format!("{} id={}", radius::code_name(p.code), p.id);
    let user = p.text(radius::A_USER_NAME);
    let status = p.int(radius::A_ACCT_STATUS_TYPE).and_then(|v| radius::value_name(radius::A_ACCT_STATUS_TYPE, v));
    match (status, user) {
        (Some(st), Some(u)) => s.push_str(&format!(" ({}, {})", st, u)),
        (Some(st), None) => s.push_str(&format!(" ({})", st)),
        (None, Some(u)) => s.push_str(&format!(" ({})", u)),
        (None, None) => {}
    }
    s
}

/// Telnet data as a dissector shows it: CR, LF and other control bytes escaped
fn telnet_text(d: &[u8], max: usize) -> String {
    let mut s = String::new();
    for (i, &c) in d.iter().enumerate() {
        if i == max {
            s.push_str("...");
            break;
        }
        match c {
            b'\r' => s.push_str("\\r"),
            b'\n' => s.push_str("\\n"),
            b'\t' => s.push_str("\\t"),
            0x20..=0x7E => s.push(c as char),
            _ => s.push_str(&format!("\\x{:02x}", c)),
        }
    }
    s
}

fn telnet_layer(frame: &[u8], off: usize, len: usize) -> B {
    let b = &frame[off..off + len];
    let mut l = B::new(Proto::Telnet, off, len, String::from("Telnet"));
    for (at, n, item) in telnet::parse(b) {
        match &item {
            telnet::Item::Data(d) => l.f("Data", off + at, n, telnet_text(d, 200)),
            telnet::Item::Opt { .. } | telnet::Item::Cmd(_) => l.f("Command", off + at, n, item.describe()),
            telnet::Item::Sub { .. } => l.f("Suboption", off + at, n, item.describe()),
            telnet::Item::Partial(_) => {
                l.f("Continuation", off + at, n, format!("{} bytes", n));
                l.bad(String::from("a command cut off at the end of the segment"));
            }
        }
    }
    l
}

/// "Will Echo, Do Suppress Go Ahead", "\"R1>\"", "Interrupt Process"
fn telnet_info(b: &[u8]) -> String {
    let parts: Vec<String> = telnet::parse(b)
        .into_iter()
        .map(|(_, _, item)| match &item {
            telnet::Item::Data(d) => format!("\"{}\"", telnet_text(d, 40)),
            other => other.describe(),
        })
        .collect();
    if parts.is_empty() { String::from("Telnet") } else { parts.join(", ") }
}
