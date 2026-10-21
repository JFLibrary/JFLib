// arp -- the Address Resolution Protocol for IPv4 over Ethernet (RFC 826, 5227)
//
//   ArpView::parse(b)     only htype 1 / ptype 0x0800 / hlen 6 / plen 4 is accepted
//   Arp { op, sha, spa, tha, tpa }   request / reply / probe / announce, write, push_onto
//
// A gratuitous ARP has spa == tpa; an ARP probe (RFC 5227) has spa 0.0.0.0.

use crate::addr::{Ipv4, Mac};
use crate::pkt::Pkt;
use crate::{be16, put16};

pub const ARP_LEN: usize = 28;
pub const OP_REQUEST: u16 = 1;
pub const OP_REPLY: u16 = 2;

/// a received ARP packet for IPv4 over Ethernet
#[derive(Clone, Copy, Debug)]
pub struct ArpView<'a> {
    b: &'a [u8],
}

impl<'a> ArpView<'a> {
    pub fn parse(b: &'a [u8]) -> Option<ArpView<'a>> {
        if b.len() < ARP_LEN || be16(b, 0) != 1 || be16(b, 2) != 0x0800 || b[4] != 6 || b[5] != 4 {
            return None;
        }
        Some(ArpView { b })
    }
    pub fn op(&self) -> u16 {
        be16(self.b, 6)
    }
    pub fn sha(&self) -> Mac {
        Mac(self.b[8..14].try_into().unwrap_or([0; 6]))
    }
    pub fn spa(&self) -> Ipv4 {
        Ipv4(self.b[14..18].try_into().unwrap_or([0; 4]))
    }
    pub fn tha(&self) -> Mac {
        Mac(self.b[18..24].try_into().unwrap_or([0; 6]))
    }
    pub fn tpa(&self) -> Ipv4 {
        Ipv4(self.b[24..28].try_into().unwrap_or([0; 4]))
    }
}

/// an ARP packet to write
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arp {
    pub op: u16,
    pub sha: Mac,
    pub spa: Ipv4,
    pub tha: Mac,
    pub tpa: Ipv4,
}

impl Arp {
    pub const LEN: usize = ARP_LEN;

    pub fn read(v: &ArpView) -> Arp {
        Arp { op: v.op(), sha: v.sha(), spa: v.spa(), tha: v.tha(), tpa: v.tpa() }
    }
    /// "who has tpa? tell spa"
    pub fn request(sha: Mac, spa: Ipv4, tpa: Ipv4) -> Arp {
        Arp { op: OP_REQUEST, sha, spa, tha: Mac::ZERO, tpa }
    }
    /// the answer to a request: `mac` has `ip`
    pub fn reply_to(req: &Arp, mac: Mac, ip: Ipv4) -> Arp {
        Arp { op: OP_REPLY, sha: mac, spa: ip, tha: req.sha, tpa: req.spa }
    }
    /// an RFC 5227 probe: sender address 0.0.0.0
    pub fn probe(sha: Mac, tpa: Ipv4) -> Arp {
        Arp { op: OP_REQUEST, sha, spa: Ipv4::UNSPECIFIED, tha: Mac::ZERO, tpa }
    }
    /// a gratuitous announcement of the own address
    pub fn announce(sha: Mac, ip: Ipv4) -> Arp {
        Arp { op: OP_REQUEST, sha, spa: ip, tha: Mac::ZERO, tpa: ip }
    }
    pub fn is_gratuitous(&self) -> bool {
        self.spa == self.tpa
    }
    pub fn is_probe(&self) -> bool {
        self.spa.is_unspecified()
    }
    pub fn write(&self, out: &mut [u8]) {
        put16(out, 0, 1);
        put16(out, 2, 0x0800);
        out[4] = 6;
        out[5] = 4;
        put16(out, 6, self.op);
        out[8..14].copy_from_slice(&self.sha.0);
        out[14..18].copy_from_slice(&self.spa.0);
        out[18..24].copy_from_slice(&self.tha.0);
        out[24..28].copy_from_slice(&self.tpa.0);
    }
    pub fn push_onto(&self, p: &mut Pkt) {
        self.write(p.push(ARP_LEN));
    }
}
