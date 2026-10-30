// udp -- the User Datagram Protocol (RFC 768, RFC 8200 8.1)
//
//   UdpView::parse(b)       ports, length, checksum, payload (bounded by the length field)
//   checksum_ok_v4 / _v6    0 means "no checksum" over IPv4; over IPv6 it is mandatory
//   UdpHdr { src_port, dst_port }   push_v4 / push_v6 (&mut Pkt) with the checksum
//   port_name(port)         well-known names ("dns", "dhcp-server", ...)

use crate::addr::{Ipv4, Ipv6};
use crate::checksum;
use crate::ipv4::PROTO_UDP;
use crate::pkt::Pkt;
use crate::{be16, put16};

pub const UDP_HDR: usize = 8;

pub const PORT_DNS: u16 = 53;
pub const PORT_DHCP_SERVER: u16 = 67;
pub const PORT_DHCP_CLIENT: u16 = 68;
pub const PORT_TFTP: u16 = 69;
pub const PORT_NTP: u16 = 123;
pub const PORT_SNMP: u16 = 161;
pub const PORT_SYSLOG: u16 = 514;
pub const PORT_RIP: u16 = 520;
pub const PORT_DHCPV6_CLIENT: u16 = 546;
pub const PORT_DHCPV6_SERVER: u16 = 547;

/// a received UDP datagram
#[derive(Clone, Copy, Debug)]
pub struct UdpView<'a> {
    b: &'a [u8],
}

impl<'a> UdpView<'a> {
    pub fn parse(b: &'a [u8]) -> Option<UdpView<'a>> {
        if b.len() < UDP_HDR {
            return None;
        }
        let l = be16(b, 4) as usize;
        if l < UDP_HDR || l > b.len() {
            return None;
        }
        Some(UdpView { b: &b[..l] })
    }
    pub fn src_port(&self) -> u16 {
        be16(self.b, 0)
    }
    pub fn dst_port(&self) -> u16 {
        be16(self.b, 2)
    }
    pub fn len(&self) -> usize {
        self.b.len()
    }
    pub fn checksum(&self) -> u16 {
        be16(self.b, 6)
    }
    pub fn payload(&self) -> &'a [u8] {
        &self.b[UDP_HDR..]
    }
    pub fn bytes(&self) -> &'a [u8] {
        self.b
    }
    /// over IPv4 a zero checksum means "not computed" and is accepted
    pub fn checksum_ok_v4(&self, src: Ipv4, dst: Ipv4) -> bool {
        self.checksum() == 0 || checksum::fold(checksum::sum(checksum::pseudo_v4(src, dst, PROTO_UDP, self.b.len() as u16), self.b)) == 0xFFFF
    }
    pub fn checksum_ok_v6(&self, src: Ipv6, dst: Ipv6) -> bool {
        self.checksum() != 0 && checksum::fold(checksum::sum(checksum::pseudo_v6(src, dst, PROTO_UDP, self.b.len() as u32), self.b)) == 0xFFFF
    }
}

/// a UDP header to write
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct UdpHdr {
    pub src_port: u16,
    pub dst_port: u16,
}

impl UdpHdr {
    pub const LEN: usize = UDP_HDR;

    pub fn new(src_port: u16, dst_port: u16) -> UdpHdr {
        UdpHdr { src_port, dst_port }
    }
    fn push(&self, p: &mut Pkt) {
        let len = (p.len() + UDP_HDR) as u16;
        let h = p.push(UDP_HDR);
        put16(h, 0, self.src_port);
        put16(h, 2, self.dst_port);
        put16(h, 4, len);
    }
    /// prepends the header with the checksum over the IPv4 pseudo header
    pub fn push_v4(&self, p: &mut Pkt, src: Ipv4, dst: Ipv4) {
        self.push(p);
        let ck = checksum::finish(checksum::sum(checksum::pseudo_v4(src, dst, PROTO_UDP, p.len() as u16), p.bytes()));
        put16(p.bytes_mut(), 6, if ck == 0 { 0xFFFF } else { ck });
    }
    /// prepends the header with the checksum over the IPv6 pseudo header
    pub fn push_v6(&self, p: &mut Pkt, src: Ipv6, dst: Ipv6) {
        self.push(p);
        let ck = checksum::finish(checksum::sum(checksum::pseudo_v6(src, dst, PROTO_UDP, p.len() as u32), p.bytes()));
        put16(p.bytes_mut(), 6, if ck == 0 { 0xFFFF } else { ck });
    }
}

/// a short name of a well-known UDP port
pub fn port_name(port: u16) -> Option<&'static str> {
    Some(match port {
        PORT_DNS => "dns",
        PORT_DHCP_SERVER => "dhcp-server",
        PORT_DHCP_CLIENT => "dhcp-client",
        PORT_TFTP => "tftp",
        PORT_NTP => "ntp",
        PORT_SNMP => "snmp",
        PORT_SYSLOG => "syslog",
        PORT_RIP => "rip",
        PORT_DHCPV6_CLIENT => "dhcpv6-client",
        PORT_DHCPV6_SERVER => "dhcpv6-server",
        crate::radius::PORT_AUTH => "radius",
        crate::radius::PORT_ACCT => "radius-acct",
        _ => return None,
    })
}
