// rip -- Routing Information Protocol messages (UDP port 520): version 2 after RFC 2453 and
// version 1 after RFC 1058; requests and responses with their route entries, and the
// authentication entry of RFC 2453 4.1 (recognised; simple passwords read and written)
//
//   RipView::parse(b)          command, version, must-be-zero field, the complete 20-byte
//                              entries (entry(i), entries(), routes() without the
//                              authentication entry), auth(), trailing() bytes after them
//   Rte { afi, tag, addr, mask, next_hop, metric }   one entry; Rte::v2 / Rte::v1 build route
//                              entries, prefix() is the destination with a version 2 mask,
//                              mbz_clear() checks the fields version 1 leaves zero
//   Auth { kind, data }        the first entry with AFI 0xFFFF: AUTH_PASSWORD (16 bytes of
//                              plain text: Auth::password, text()), AUTH_MD5 (RFC 4822: kind and
//                              bytes only, not checked)
//   Rip { command, version, auth, entries }   read / to_bytes / response / request_all /
//                              is_request_all (one entry, address family 0, metric 16)
//   CMD_REQUEST, CMD_RESPONSE, AFI_INET, AFI_AUTH, AUTH_*, INFINITY (16), MAX_ENTRIES (25),
//   GROUP (224.0.0.9, RIPv2's multicast address), command_name
//
// The view reads whatever is there: more than 25 entries, bytes after the last entry, a version
// 1 message with a must-be-zero field set or an unknown command are for the receiver to judge
// (decode.rs reports them as problems).  Version 1 entries carry no mask; the receiver infers it
// from the address class and its own interfaces (RFC 1058 3.7).

use alloc::string::String;
use alloc::vec::Vec;

use crate::addr::{Ipv4, Prefix4};
use crate::{be16, be32, put16, put32};

/// the header: command, version, two bytes that must be zero
pub const RIP_HDR: usize = 4;
/// one entry (route or authentication)
pub const RTE_LEN: usize = 20;
/// entries a message holds at most (RFC 2453 3.6); 24 routes with an authentication entry
pub const MAX_ENTRIES: usize = 25;
pub const CMD_REQUEST: u8 = 1;
pub const CMD_RESPONSE: u8 = 2;
/// the address family of IPv4 routes
pub const AFI_INET: u16 = 2;
/// the address family that marks the authentication entry (RFC 2453 4.1)
pub const AFI_AUTH: u16 = 0xFFFF;
/// a plain-text password of 16 bytes (RFC 2453 4.1)
pub const AUTH_PASSWORD: u16 = 2;
/// keyed MD5 (RFC 4822)
pub const AUTH_MD5: u16 = 3;
/// the metric of an unreachable destination
pub const INFINITY: u32 = 16;
/// where version 2 sends its updates (RFC 2453 4.5)
pub const GROUP: Ipv4 = Ipv4([224, 0, 0, 9]);

/// the name of a command as capture tools show it
pub fn command_name(c: u8) -> &'static str {
    match c {
        CMD_REQUEST => "Request",
        CMD_RESPONSE => "Response",
        3 => "Traceon",
        4 => "Traceoff",
        _ => "Unknown",
    }
}

/// one 20-byte entry; for the authentication entry `tag` holds the type and the 16 bytes from
/// `addr` on hold the authentication data (Auth reads them)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rte {
    pub afi: u16,
    /// the route tag (version 2; zero in version 1)
    pub tag: u16,
    pub addr: Ipv4,
    /// the subnet mask (version 2; zero: not given)
    pub mask: Ipv4,
    /// where to send packets for the destination (version 2; 0.0.0.0: to the sender)
    pub next_hop: Ipv4,
    pub metric: u32,
}

impl Rte {
    /// an entry from its 20 bytes (the caller has checked the length)
    pub fn read(b: &[u8]) -> Rte {
        Rte {
            afi: be16(b, 0),
            tag: be16(b, 2),
            addr: Ipv4([b[4], b[5], b[6], b[7]]),
            mask: Ipv4([b[8], b[9], b[10], b[11]]),
            next_hop: Ipv4([b[12], b[13], b[14], b[15]]),
            metric: be32(b, 16),
        }
    }
    /// writes the entry into 20 bytes
    pub fn write(&self, b: &mut [u8]) {
        put16(b, 0, self.afi);
        put16(b, 2, self.tag);
        b[4..8].copy_from_slice(&self.addr.0);
        b[8..12].copy_from_slice(&self.mask.0);
        b[12..16].copy_from_slice(&self.next_hop.0);
        put32(b, 16, self.metric);
    }
    /// a version 2 route: the destination with its mask, the next hop (0.0.0.0: the sender
    /// itself), the metric and the route tag
    pub fn v2(prefix: Prefix4, next_hop: Ipv4, metric: u32, tag: u16) -> Rte {
        Rte { afi: AFI_INET, tag, addr: prefix.network(), mask: prefix.mask(), next_hop, metric }
    }
    /// a version 1 route: the address alone
    pub fn v1(addr: Ipv4, metric: u32) -> Rte {
        Rte { afi: AFI_INET, metric, addr, ..Rte::default() }
    }
    /// the destination with its version 2 mask (None: no mask, or one with gaps)
    pub fn prefix(&self) -> Option<Prefix4> {
        if self.mask.is_unspecified() {
            return None;
        }
        Prefix4::from_mask(self.addr, self.mask)
    }
    /// the fields version 1 leaves zero are zero (route tag, mask, next hop)
    pub fn mbz_clear(&self) -> bool {
        self.tag == 0 && self.mask.is_unspecified() && self.next_hop.is_unspecified()
    }
}

/// the authentication entry of a version 2 message
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Auth {
    pub kind: u16,
    pub data: [u8; 16],
}

impl Auth {
    /// a plain-text password: left-justified, padded with zero bytes, cut at 16 bytes
    pub fn password(pw: &[u8]) -> Auth {
        let mut data = [0u8; 16];
        let n = pw.len().min(16);
        data[..n].copy_from_slice(&pw[..n]);
        Auth { kind: AUTH_PASSWORD, data }
    }
    /// the password of a plain-text entry (up to the first zero byte)
    pub fn text(&self) -> Option<String> {
        if self.kind != AUTH_PASSWORD {
            return None;
        }
        let n = self.data.iter().position(|&c| c == 0).unwrap_or(16);
        Some(String::from_utf8_lossy(&self.data[..n]).into_owned())
    }
    fn read(b: &[u8]) -> Auth {
        let mut data = [0u8; 16];
        data.copy_from_slice(&b[4..20]);
        Auth { kind: be16(b, 2), data }
    }
    fn write(&self, b: &mut [u8]) {
        put16(b, 0, AFI_AUTH);
        put16(b, 2, self.kind);
        b[4..20].copy_from_slice(&self.data);
    }
}

/// a received RIP message
#[derive(Clone, Copy, Debug)]
pub struct RipView<'a> {
    b: &'a [u8],
}

impl<'a> RipView<'a> {
    /// at least the header
    pub fn parse(b: &'a [u8]) -> Option<RipView<'a>> {
        if b.len() < RIP_HDR {
            return None;
        }
        Some(RipView { b })
    }
    pub fn command(&self) -> u8 {
        self.b[0]
    }
    pub fn version(&self) -> u8 {
        self.b[1]
    }
    /// the two header bytes that must be zero
    pub fn mbz(&self) -> u16 {
        be16(self.b, 2)
    }
    pub fn bytes(&self) -> &'a [u8] {
        self.b
    }
    /// complete entries
    pub fn count(&self) -> usize {
        (self.b.len() - RIP_HDR) / RTE_LEN
    }
    /// bytes after the last complete entry
    pub fn trailing(&self) -> usize {
        (self.b.len() - RIP_HDR) % RTE_LEN
    }
    /// entry `i` (authentication entries included)
    pub fn entry(&self, i: usize) -> Option<Rte> {
        let o = RIP_HDR + i * RTE_LEN;
        self.b.get(o..o + RTE_LEN).map(Rte::read)
    }
    pub fn entries(&self) -> impl Iterator<Item = Rte> + '_ {
        (0..self.count()).filter_map(move |i| self.entry(i))
    }
    /// the authentication entry: only the first entry may be one (RFC 2453 4.1)
    pub fn auth(&self) -> Option<Auth> {
        let b = self.b.get(RIP_HDR..RIP_HDR + RTE_LEN)?;
        if be16(b, 0) == AFI_AUTH { Some(Auth::read(b)) } else { None }
    }
    /// the entries after a leading authentication entry
    pub fn routes(&self) -> impl Iterator<Item = Rte> + '_ {
        let skip = if self.auth().is_some() { 1 } else { 0 };
        self.entries().skip(skip)
    }
}

/// a RIP message to write (or one read)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rip {
    pub command: u8,
    pub version: u8,
    /// written as the first entry
    pub auth: Option<Auth>,
    pub entries: Vec<Rte>,
}

impl Rip {
    pub fn response(version: u8, entries: Vec<Rte>) -> Rip {
        Rip { command: CMD_RESPONSE, version, auth: None, entries }
    }
    /// the request for the whole routing table: a single entry with address family 0 and
    /// metric 16 (RFC 2453 3.9.1)
    pub fn request_all(version: u8) -> Rip {
        Rip { command: CMD_REQUEST, version, auth: None, entries: alloc::vec![Rte { metric: INFINITY, ..Rte::default() }] }
    }
    /// this is the request for the whole table
    pub fn is_request_all(&self) -> bool {
        self.command == CMD_REQUEST && self.entries.len() == 1 && self.entries[0].afi == 0 && self.entries[0].metric == INFINITY
    }
    pub fn read(v: &RipView) -> Rip {
        Rip { command: v.command(), version: v.version(), auth: v.auth(), entries: v.routes().collect() }
    }
    /// the message as bytes (no limit on the number of entries: the sender splits at 25)
    pub fn to_bytes(&self) -> Vec<u8> {
        let n = self.entries.len() + self.auth.is_some() as usize;
        let mut b = alloc::vec![0u8; RIP_HDR + n * RTE_LEN];
        b[0] = self.command;
        b[1] = self.version;
        let mut o = RIP_HDR;
        if let Some(a) = &self.auth {
            a.write(&mut b[o..o + RTE_LEN]);
            o += RTE_LEN;
        }
        for e in &self.entries {
            e.write(&mut b[o..o + RTE_LEN]);
            o += RTE_LEN;
        }
        b
    }
}
