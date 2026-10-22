// eth -- Ethernet II frames, IEEE 802.1Q tags and 802.2 LLC
//
//   EthView::parse(frame)       dst, src, ethertype (or 802.3 length), payload
//   EthHdr { dst, src, ethertype }  write / push_onto(&mut Pkt)
//   VlanTag { pcp, dei, vid }   tag_of(frame), tagged(frame, tag), untagged(frame)
//   L2::parse(frame)            dst, src, the 802.1Q tag if any, the inner ethertype and where
//                               the payload starts
//   Llc                         802.2 LLC header of 802.3 frames (STP uses DSAP/SSAP 0x42)
//   pad(frame)                  pads to the 60-byte minimum (without FCS)
//
// Frames here never carry the FCS; checksum::crc32 computes it when someone wants to see it.

use alloc::vec::Vec;

use crate::addr::Mac;
use crate::pkt::Pkt;
use crate::{be16, put16};

pub const ETH_HDR: usize = 14;
pub const VLAN_TAG: usize = 4;
/// the shortest frame without FCS
pub const ETH_MIN: usize = 60;
/// the largest payload of a standard frame
pub const ETH_MTU: usize = 1500;

pub const ET_IPV4: u16 = 0x0800;
pub const ET_ARP: u16 = 0x0806;
pub const ET_VLAN: u16 = 0x8100;
pub const ET_IPV6: u16 = 0x86DD;
pub const ET_QINQ: u16 = 0x88A8;
pub const ET_LLDP: u16 = 0x88CC;
/// IEEE 802.1X (EAPOL: the 4-way handshake of WPA2 and WPA3)
pub const ET_EAPOL: u16 = 0x888E;

/// a name for an ethertype ("IPv4", "ARP", ...), None for unknown types
pub fn ethertype_name(t: u16) -> Option<&'static str> {
    Some(match t {
        ET_IPV4 => "IPv4",
        ET_ARP => "ARP",
        ET_VLAN => "802.1Q",
        ET_IPV6 => "IPv6",
        ET_QINQ => "802.1ad",
        ET_LLDP => "LLDP",
        ET_EAPOL => "802.1X Authentication",
        _ => return None,
    })
}

/// a received Ethernet frame (at least 14 bytes)
#[derive(Clone, Copy, Debug)]
pub struct EthView<'a> {
    b: &'a [u8],
}

impl<'a> EthView<'a> {
    pub fn parse(b: &'a [u8]) -> Option<EthView<'a>> {
        if b.len() < ETH_HDR { None } else { Some(EthView { b }) }
    }
    pub fn dst(&self) -> Mac {
        Mac([self.b[0], self.b[1], self.b[2], self.b[3], self.b[4], self.b[5]])
    }
    pub fn src(&self) -> Mac {
        Mac([self.b[6], self.b[7], self.b[8], self.b[9], self.b[10], self.b[11]])
    }
    /// the type field (an 802.3 length when <= 1500)
    pub fn ethertype(&self) -> u16 {
        be16(self.b, 12)
    }
    /// an IEEE 802.3 frame with a length field and an LLC header
    pub fn is_llc(&self) -> bool {
        self.ethertype() <= 1500
    }
    pub fn payload(&self) -> &'a [u8] {
        &self.b[ETH_HDR..]
    }
    pub fn bytes(&self) -> &'a [u8] {
        self.b
    }
}

/// the Ethernet II header to write
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct EthHdr {
    pub dst: Mac,
    pub src: Mac,
    pub ethertype: u16,
}

impl EthHdr {
    pub const LEN: usize = ETH_HDR;

    pub fn read(v: &EthView) -> EthHdr {
        EthHdr { dst: v.dst(), src: v.src(), ethertype: v.ethertype() }
    }
    pub fn write(&self, out: &mut [u8]) {
        out[0..6].copy_from_slice(&self.dst.0);
        out[6..12].copy_from_slice(&self.src.0);
        put16(out, 12, self.ethertype);
    }
    pub fn push_onto(&self, p: &mut Pkt) {
        self.write(p.push(ETH_HDR));
    }
    /// header + payload as a new frame
    pub fn frame(&self, payload: &[u8]) -> Vec<u8> {
        let mut v = alloc::vec![0u8; ETH_HDR + payload.len()];
        self.write(&mut v);
        v[ETH_HDR..].copy_from_slice(payload);
        v
    }
}

/// pads a frame with zeros to the 60-byte minimum
pub fn pad(frame: &mut Vec<u8>) {
    if frame.len() < ETH_MIN {
        frame.resize(ETH_MIN, 0);
    }
}

// ---------------------------------------------------------------- 802.1Q

/// an IEEE 802.1Q tag: priority, drop eligible, VLAN id (1..4094)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct VlanTag {
    pub pcp: u8,
    pub dei: bool,
    pub vid: u16,
}

impl VlanTag {
    pub fn new(vid: u16) -> VlanTag {
        VlanTag { pcp: 0, dei: false, vid: vid & 0x0FFF }
    }
    pub fn tci(&self) -> u16 {
        ((self.pcp as u16 & 7) << 13) | ((self.dei as u16) << 12) | (self.vid & 0x0FFF)
    }
    pub fn from_tci(tci: u16) -> VlanTag {
        VlanTag { pcp: (tci >> 13) as u8, dei: tci & 0x1000 != 0, vid: tci & 0x0FFF }
    }
}

/// the outer 802.1Q tag of a frame (TPID 0x8100), None for untagged frames
pub fn tag_of(frame: &[u8]) -> Option<VlanTag> {
    if frame.len() >= ETH_HDR + VLAN_TAG && be16(frame, 12) == ET_VLAN {
        Some(VlanTag::from_tci(be16(frame, 14)))
    } else {
        None
    }
}

/// the frame with a tag inserted after the source MAC (an existing tag stays inside)
pub fn tagged(frame: &[u8], tag: VlanTag) -> Vec<u8> {
    let mut v = Vec::with_capacity(frame.len() + VLAN_TAG);
    v.extend_from_slice(&frame[..12]);
    v.extend_from_slice(&ET_VLAN.to_be_bytes());
    v.extend_from_slice(&tag.tci().to_be_bytes());
    v.extend_from_slice(&frame[12..]);
    v
}

/// the frame without its outer tag (unchanged copy when untagged)
pub fn untagged(frame: &[u8]) -> Vec<u8> {
    if tag_of(frame).is_none() {
        return frame.to_vec();
    }
    let mut v = Vec::with_capacity(frame.len() - VLAN_TAG);
    v.extend_from_slice(&frame[..12]);
    v.extend_from_slice(&frame[16..]);
    v
}

/// the layer-2 view of a frame: addresses, an optional 802.1Q tag, the inner type
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct L2 {
    pub dst: Mac,
    pub src: Mac,
    pub tag: Option<VlanTag>,
    /// the ethertype after the tag (an 802.3 length when <= 1500)
    pub ethertype: u16,
    /// offset of the payload in the frame
    pub off: usize,
}

impl L2 {
    pub fn parse(frame: &[u8]) -> Option<L2> {
        let e = EthView::parse(frame)?;
        let mut et = e.ethertype();
        let mut off = ETH_HDR;
        let mut tag = None;
        if et == ET_VLAN {
            if frame.len() < ETH_HDR + VLAN_TAG {
                return None;
            }
            tag = Some(VlanTag::from_tci(be16(frame, 14)));
            et = be16(frame, 16);
            off += VLAN_TAG;
        }
        Some(L2 { dst: e.dst(), src: e.src(), tag, ethertype: et, off })
    }
    pub fn payload<'a>(&self, frame: &'a [u8]) -> &'a [u8] {
        &frame[self.off..]
    }
}

// ---------------------------------------------------------------- 802.2 LLC

/// the IEEE 802.2 LLC header of an 802.3 frame (unnumbered information frames only)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Llc {
    pub dsap: u8,
    pub ssap: u8,
    pub control: u8,
}

/// the SAP of the spanning tree protocols
pub const SAP_STP: u8 = 0x42;

impl Llc {
    pub const LEN: usize = 3;
    pub const STP: Llc = Llc { dsap: SAP_STP, ssap: SAP_STP, control: 0x03 };

    pub fn parse(b: &[u8]) -> Option<Llc> {
        if b.len() < 3 { None } else { Some(Llc { dsap: b[0], ssap: b[1], control: b[2] }) }
    }
    pub fn write(&self, out: &mut [u8]) {
        out[0] = self.dsap;
        out[1] = self.ssap;
        out[2] = self.control;
    }
}
