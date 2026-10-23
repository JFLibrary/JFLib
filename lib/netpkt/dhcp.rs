// dhcp -- the Dynamic Host Configuration Protocol for IPv4 (RFC 2131; options RFC 2132, long
// options RFC 3396, relay agent information RFC 3046)
//
//   DhcpView::parse(b)     the fixed BOOTP part (236 bytes) and the magic cookie; fields
//                          without copying; options() walks the options (pad skipped, end
//                          stops; with option 52 the file and sname fields follow); opt(code)
//                          the first instance of an option; msg_type()
//   Dhcp { op, htype, hlen, hops, xid, secs, flags, ciaddr, yiaddr, siaddr, giaddr, chaddr,
//          sname, file, opts: Vec<Opt> }
//                          read(&view) (repeated options are joined, RFC 3396) / to_bytes(min)
//                          (options in their order, long ones split, end, zero padding up to
//                          `min` bytes: BOOTP_MIN for the 300 bytes most clients and servers
//                          send) / new(op, xid, chaddr) / get(code) / set(opt) / mac()
//   Opt                    MsgType, SubnetMask, Routers, DnsServers, HostName, DomainName,
//                          RequestedIp, LeaseTime, ServerId, Params, Message, MaxSize, T1, T2,
//                          ClientId, RelayInfo, BroadcastAddr, NtpServers, Other(code, bytes)
//   MsgType                Discover .. Inform; name()
//   option_name(code)      "Subnet Mask", "Router", ... for dissectors
//
// An option whose length does not fit its type is kept as Other(code, bytes), so reading and
// writing a message gives the same bytes.

use alloc::string::String;
use alloc::vec::Vec;

use crate::addr::{Ipv4, Mac};
use crate::{be16, be32, put16, put32};

/// the fixed BOOTP header
pub const BOOTP_HDR: usize = 236;
/// the fixed header and the magic cookie
pub const DHCP_MIN: usize = 240;
/// the size most clients and servers pad their messages to (RFC 1542)
pub const BOOTP_MIN: usize = 300;
pub const MAGIC: [u8; 4] = [99, 130, 83, 99];

pub const OP_REQUEST: u8 = 1;
pub const OP_REPLY: u8 = 2;
/// the broadcast bit of the flags field
pub const FLAG_BROADCAST: u16 = 0x8000;

pub const O_PAD: u8 = 0;
pub const O_SUBNET_MASK: u8 = 1;
pub const O_ROUTER: u8 = 3;
pub const O_DNS: u8 = 6;
pub const O_HOST_NAME: u8 = 12;
pub const O_DOMAIN_NAME: u8 = 15;
pub const O_BROADCAST: u8 = 28;
pub const O_NTP: u8 = 42;
pub const O_REQUESTED_IP: u8 = 50;
pub const O_LEASE_TIME: u8 = 51;
pub const O_OVERLOAD: u8 = 52;
pub const O_MSG_TYPE: u8 = 53;
pub const O_SERVER_ID: u8 = 54;
pub const O_PARAMS: u8 = 55;
pub const O_MESSAGE: u8 = 56;
pub const O_MAX_SIZE: u8 = 57;
pub const O_T1: u8 = 58;
pub const O_T2: u8 = 59;
pub const O_CLIENT_ID: u8 = 61;
pub const O_RELAY: u8 = 82;
pub const O_END: u8 = 255;

/// relay agent sub-options (RFC 3046)
pub const RELAY_CIRCUIT_ID: u8 = 1;
pub const RELAY_REMOTE_ID: u8 = 2;

/// the DHCP message type (option 53)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MsgType {
    Discover = 1,
    Offer = 2,
    Request = 3,
    Decline = 4,
    Ack = 5,
    Nak = 6,
    Release = 7,
    Inform = 8,
}

impl MsgType {
    pub fn from_u8(v: u8) -> Option<MsgType> {
        Some(match v {
            1 => MsgType::Discover,
            2 => MsgType::Offer,
            3 => MsgType::Request,
            4 => MsgType::Decline,
            5 => MsgType::Ack,
            6 => MsgType::Nak,
            7 => MsgType::Release,
            8 => MsgType::Inform,
            _ => return None,
        })
    }
    /// "Discover", "Offer", ... (capture tools write "DHCP Discover")
    pub fn name(self) -> &'static str {
        match self {
            MsgType::Discover => "Discover",
            MsgType::Offer => "Offer",
            MsgType::Request => "Request",
            MsgType::Decline => "Decline",
            MsgType::Ack => "ACK",
            MsgType::Nak => "NAK",
            MsgType::Release => "Release",
            MsgType::Inform => "Inform",
        }
    }
}

/// the name of an option for dissectors
pub fn option_name(code: u8) -> &'static str {
    match code {
        O_PAD => "Pad",
        O_SUBNET_MASK => "Subnet Mask",
        2 => "Time Offset",
        O_ROUTER => "Router",
        O_DNS => "Domain Name Server",
        O_HOST_NAME => "Host Name",
        O_DOMAIN_NAME => "Domain Name",
        26 => "Interface MTU",
        O_BROADCAST => "Broadcast Address",
        O_NTP => "Network Time Protocol Servers",
        44 => "NetBIOS over TCP/IP Name Server",
        47 => "NetBIOS over TCP/IP Scope",
        O_REQUESTED_IP => "Requested IP Address",
        O_LEASE_TIME => "IP Address Lease Time",
        O_OVERLOAD => "Option Overload",
        O_MSG_TYPE => "DHCP Message Type",
        O_SERVER_ID => "DHCP Server Identifier",
        O_PARAMS => "Parameter Request List",
        O_MESSAGE => "Message",
        O_MAX_SIZE => "Maximum DHCP Message Size",
        O_T1 => "Renewal Time Value",
        O_T2 => "Rebinding Time Value",
        60 => "Vendor class identifier",
        O_CLIENT_ID => "Client identifier",
        81 => "Client Fully Qualified Domain Name",
        O_RELAY => "Agent Information Option",
        119 => "Domain Search",
        121 => "Classless Static Route",
        O_END => "End",
        _ => "Unknown",
    }
}

/// a received DHCP message
#[derive(Clone, Copy, Debug)]
pub struct DhcpView<'a> {
    b: &'a [u8],
}

fn ip_at(b: &[u8], off: usize) -> Ipv4 {
    Ipv4([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// the bytes of a zero-terminated field
fn cstr(b: &[u8]) -> &[u8] {
    &b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]
}

impl<'a> DhcpView<'a> {
    /// a BOOTP message with the DHCP magic cookie
    pub fn parse(b: &'a [u8]) -> Option<DhcpView<'a>> {
        if b.len() < DHCP_MIN || b[BOOTP_HDR..DHCP_MIN] != MAGIC {
            return None;
        }
        Some(DhcpView { b })
    }
    pub fn bytes(&self) -> &'a [u8] {
        self.b
    }
    pub fn op(&self) -> u8 {
        self.b[0]
    }
    pub fn htype(&self) -> u8 {
        self.b[1]
    }
    pub fn hlen(&self) -> u8 {
        self.b[2]
    }
    pub fn hops(&self) -> u8 {
        self.b[3]
    }
    pub fn xid(&self) -> u32 {
        be32(self.b, 4)
    }
    pub fn secs(&self) -> u16 {
        be16(self.b, 8)
    }
    pub fn flags(&self) -> u16 {
        be16(self.b, 10)
    }
    pub fn broadcast(&self) -> bool {
        self.flags() & FLAG_BROADCAST != 0
    }
    pub fn ciaddr(&self) -> Ipv4 {
        ip_at(self.b, 12)
    }
    pub fn yiaddr(&self) -> Ipv4 {
        ip_at(self.b, 16)
    }
    pub fn siaddr(&self) -> Ipv4 {
        ip_at(self.b, 20)
    }
    pub fn giaddr(&self) -> Ipv4 {
        ip_at(self.b, 24)
    }
    /// the client hardware address field (16 bytes)
    pub fn chaddr(&self) -> [u8; 16] {
        self.b[28..44].try_into().unwrap_or([0; 16])
    }
    /// the client's MAC (Ethernet: htype 1, hlen 6)
    pub fn mac(&self) -> Option<Mac> {
        if self.htype() == 1 && self.hlen() == 6 { Some(Mac(self.b[28..34].try_into().unwrap_or([0; 6]))) } else { None }
    }
    /// the server host name field up to its zero
    pub fn sname(&self) -> &'a [u8] {
        cstr(&self.b[44..108])
    }
    /// the boot file name field up to its zero
    pub fn file(&self) -> &'a [u8] {
        cstr(&self.b[108..236])
    }
    /// every option in order: (code, data, offset of the code byte); pad and end are left out
    pub fn options(&self) -> Vec<(u8, &'a [u8], usize)> {
        let mut out = Vec::new();
        let overload = walk(self.b, DHCP_MIN, self.b.len(), &mut out);
        // RFC 2132 9.3: the file field (1), the sname field (2) or both (3) carry options too
        if overload & 1 != 0 {
            walk(self.b, 108, 236, &mut out);
        }
        if overload & 2 != 0 {
            walk(self.b, 44, 108, &mut out);
        }
        out
    }
    /// the data of the first instance of an option
    pub fn opt(&self, code: u8) -> Option<&'a [u8]> {
        self.options().into_iter().find(|o| o.0 == code).map(|o| o.1)
    }
    pub fn msg_type(&self) -> Option<MsgType> {
        self.opt(O_MSG_TYPE).filter(|d| d.len() == 1).and_then(|d| MsgType::from_u8(d[0]))
    }
}

/// walks the options in b[from..to]; returns the value of option 52 (0 when absent)
fn walk<'a>(b: &'a [u8], from: usize, to: usize, out: &mut Vec<(u8, &'a [u8], usize)>) -> u8 {
    let mut overload = 0;
    let mut p = from;
    while p < to {
        let code = b[p];
        if code == O_PAD {
            p += 1;
            continue;
        }
        if code == O_END || p + 1 >= to {
            break;
        }
        let l = b[p + 1] as usize;
        if p + 2 + l > to {
            break;
        }
        let d = &b[p + 2..p + 2 + l];
        if code == O_OVERLOAD && l == 1 {
            overload = d[0];
        }
        out.push((code, d, p));
        p += 2 + l;
    }
    overload
}

/// an option with its value
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opt {
    MsgType(MsgType),
    SubnetMask(Ipv4),
    Routers(Vec<Ipv4>),
    DnsServers(Vec<Ipv4>),
    HostName(String),
    DomainName(String),
    BroadcastAddr(Ipv4),
    NtpServers(Vec<Ipv4>),
    RequestedIp(Ipv4),
    /// seconds (0xFFFF_FFFF: infinite)
    LeaseTime(u32),
    ServerId(Ipv4),
    /// option codes the client asks for
    Params(Vec<u8>),
    /// a text a server adds to a NAK, or a client to a DECLINE
    Message(String),
    MaxSize(u16),
    /// renewal time in seconds
    T1(u32),
    /// rebinding time in seconds
    T2(u32),
    /// the type byte (1: Ethernet) and the identifier
    ClientId(Vec<u8>),
    /// relay agent sub-options (code, data)
    RelayInfo(Vec<(u8, Vec<u8>)>),
    Other(u8, Vec<u8>),
}

fn ips(d: &[u8]) -> Option<Vec<Ipv4>> {
    if d.is_empty() || d.len() % 4 != 0 {
        return None;
    }
    Some(d.chunks(4).map(|c| ip_at(c, 0)).collect())
}

fn one_ip(d: &[u8]) -> Option<Ipv4> {
    if d.len() == 4 { Some(ip_at(d, 0)) } else { None }
}

fn u32_of(d: &[u8]) -> Option<u32> {
    if d.len() == 4 { Some(be32(d, 0)) } else { None }
}

fn text(d: &[u8]) -> Option<String> {
    if d.is_empty() {
        return None;
    }
    core::str::from_utf8(d).ok().map(String::from)
}

impl Opt {
    /// the option of `code` with `data` (Other when the data does not fit the type)
    pub fn decode(code: u8, d: &[u8]) -> Opt {
        let typed = match code {
            O_MSG_TYPE if d.len() == 1 => MsgType::from_u8(d[0]).map(Opt::MsgType),
            O_SUBNET_MASK => one_ip(d).map(Opt::SubnetMask),
            O_ROUTER => ips(d).map(Opt::Routers),
            O_DNS => ips(d).map(Opt::DnsServers),
            O_HOST_NAME => text(d).map(Opt::HostName),
            O_DOMAIN_NAME => text(d).map(Opt::DomainName),
            O_BROADCAST => one_ip(d).map(Opt::BroadcastAddr),
            O_NTP => ips(d).map(Opt::NtpServers),
            O_REQUESTED_IP => one_ip(d).map(Opt::RequestedIp),
            O_LEASE_TIME => u32_of(d).map(Opt::LeaseTime),
            O_SERVER_ID => one_ip(d).map(Opt::ServerId),
            O_PARAMS if !d.is_empty() => Some(Opt::Params(d.to_vec())),
            O_MESSAGE => text(d).map(Opt::Message),
            O_MAX_SIZE if d.len() == 2 => Some(Opt::MaxSize(be16(d, 0))),
            O_T1 => u32_of(d).map(Opt::T1),
            O_T2 => u32_of(d).map(Opt::T2),
            O_CLIENT_ID if d.len() >= 2 => Some(Opt::ClientId(d.to_vec())),
            O_RELAY => relay_subs(d).map(Opt::RelayInfo),
            _ => None,
        };
        typed.unwrap_or_else(|| Opt::Other(code, d.to_vec()))
    }
    pub fn code(&self) -> u8 {
        match self {
            Opt::MsgType(_) => O_MSG_TYPE,
            Opt::SubnetMask(_) => O_SUBNET_MASK,
            Opt::Routers(_) => O_ROUTER,
            Opt::DnsServers(_) => O_DNS,
            Opt::HostName(_) => O_HOST_NAME,
            Opt::DomainName(_) => O_DOMAIN_NAME,
            Opt::BroadcastAddr(_) => O_BROADCAST,
            Opt::NtpServers(_) => O_NTP,
            Opt::RequestedIp(_) => O_REQUESTED_IP,
            Opt::LeaseTime(_) => O_LEASE_TIME,
            Opt::ServerId(_) => O_SERVER_ID,
            Opt::Params(_) => O_PARAMS,
            Opt::Message(_) => O_MESSAGE,
            Opt::MaxSize(_) => O_MAX_SIZE,
            Opt::T1(_) => O_T1,
            Opt::T2(_) => O_T2,
            Opt::ClientId(_) => O_CLIENT_ID,
            Opt::RelayInfo(_) => O_RELAY,
            Opt::Other(c, _) => *c,
        }
    }
    /// the data bytes of the option
    pub fn data(&self) -> Vec<u8> {
        let ipv = |v: &[Ipv4]| v.iter().flat_map(|a| a.0).collect::<Vec<u8>>();
        match self {
            Opt::MsgType(t) => alloc::vec![*t as u8],
            Opt::SubnetMask(a) | Opt::BroadcastAddr(a) | Opt::RequestedIp(a) | Opt::ServerId(a) => a.0.to_vec(),
            Opt::Routers(v) | Opt::DnsServers(v) | Opt::NtpServers(v) => ipv(v),
            Opt::HostName(s) | Opt::DomainName(s) | Opt::Message(s) => s.as_bytes().to_vec(),
            Opt::LeaseTime(n) | Opt::T1(n) | Opt::T2(n) => n.to_be_bytes().to_vec(),
            Opt::Params(p) => p.clone(),
            Opt::MaxSize(n) => n.to_be_bytes().to_vec(),
            Opt::ClientId(c) => c.clone(),
            Opt::RelayInfo(subs) => {
                let mut d = Vec::new();
                for (c, v) in subs {
                    d.push(*c);
                    d.push(v.len().min(255) as u8);
                    d.extend_from_slice(&v[..v.len().min(255)]);
                }
                d
            }
            Opt::Other(_, d) => d.clone(),
        }
    }
}

fn relay_subs(d: &[u8]) -> Option<Vec<(u8, Vec<u8>)>> {
    let mut v = Vec::new();
    let mut p = 0;
    while p < d.len() {
        if p + 2 > d.len() || p + 2 + d[p + 1] as usize > d.len() {
            return None;
        }
        let l = d[p + 1] as usize;
        v.push((d[p], d[p + 2..p + 2 + l].to_vec()));
        p += 2 + l;
    }
    if v.is_empty() { None } else { Some(v) }
}

/// a DHCP message to read or write
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dhcp {
    pub op: u8,
    pub htype: u8,
    pub hlen: u8,
    pub hops: u8,
    pub xid: u32,
    pub secs: u16,
    pub flags: u16,
    pub ciaddr: Ipv4,
    pub yiaddr: Ipv4,
    pub siaddr: Ipv4,
    pub giaddr: Ipv4,
    pub chaddr: [u8; 16],
    pub sname: Vec<u8>,
    pub file: Vec<u8>,
    /// in the order they are written
    pub opts: Vec<Opt>,
}

impl Dhcp {
    /// an empty message of an Ethernet client
    pub fn new(op: u8, xid: u32, mac: Mac) -> Dhcp {
        let mut chaddr = [0u8; 16];
        chaddr[..6].copy_from_slice(&mac.0);
        Dhcp {
            op,
            htype: 1,
            hlen: 6,
            hops: 0,
            xid,
            secs: 0,
            flags: 0,
            ciaddr: Ipv4::UNSPECIFIED,
            yiaddr: Ipv4::UNSPECIFIED,
            siaddr: Ipv4::UNSPECIFIED,
            giaddr: Ipv4::UNSPECIFIED,
            chaddr,
            sname: Vec::new(),
            file: Vec::new(),
            opts: Vec::new(),
        }
    }
    /// a message as received; an option that occurs more than once is joined (RFC 3396);
    /// with option 52 the sname and file fields only carried options
    pub fn read(v: &DhcpView) -> Dhcp {
        let all = v.options();
        let mut order: Vec<u8> = Vec::new();
        let mut joined: Vec<Vec<u8>> = Vec::new();
        let mut overload = 0;
        for (code, d, _) in &all {
            if *code == O_OVERLOAD && d.len() == 1 {
                overload = d[0];
            }
            match order.iter().position(|c| c == code) {
                Some(i) => joined[i].extend_from_slice(d),
                None => {
                    order.push(*code);
                    joined.push(d.to_vec());
                }
            }
        }
        let opts = order.iter().zip(joined.iter()).map(|(c, d)| Opt::decode(*c, d)).collect();
        Dhcp {
            op: v.op(),
            htype: v.htype(),
            hlen: v.hlen(),
            hops: v.hops(),
            xid: v.xid(),
            secs: v.secs(),
            flags: v.flags(),
            ciaddr: v.ciaddr(),
            yiaddr: v.yiaddr(),
            siaddr: v.siaddr(),
            giaddr: v.giaddr(),
            chaddr: v.chaddr(),
            sname: if overload & 2 != 0 { Vec::new() } else { v.sname().to_vec() },
            file: if overload & 1 != 0 { Vec::new() } else { v.file().to_vec() },
            opts,
        }
    }
    /// parses a message (None: no BOOTP message with the DHCP cookie)
    pub fn parse(b: &[u8]) -> Option<Dhcp> {
        DhcpView::parse(b).map(|v| Dhcp::read(&v))
    }
    /// the client's MAC (Ethernet)
    pub fn mac(&self) -> Mac {
        Mac(self.chaddr[..6].try_into().unwrap_or([0; 6]))
    }
    pub fn broadcast(&self) -> bool {
        self.flags & FLAG_BROADCAST != 0
    }
    pub fn get(&self, code: u8) -> Option<&Opt> {
        self.opts.iter().find(|o| o.code() == code)
    }
    /// adds an option or replaces the one with the same code
    pub fn set(&mut self, o: Opt) {
        match self.opts.iter_mut().find(|x| x.code() == o.code()) {
            Some(x) => *x = o,
            None => self.opts.push(o),
        }
    }
    pub fn remove(&mut self, code: u8) {
        self.opts.retain(|o| o.code() != code);
    }
    pub fn msg_type(&self) -> Option<MsgType> {
        match self.get(O_MSG_TYPE) {
            Some(Opt::MsgType(t)) => Some(*t),
            _ => None,
        }
    }
    pub fn server_id(&self) -> Option<Ipv4> {
        match self.get(O_SERVER_ID) {
            Some(Opt::ServerId(a)) => Some(*a),
            _ => None,
        }
    }
    pub fn requested_ip(&self) -> Option<Ipv4> {
        match self.get(O_REQUESTED_IP) {
            Some(Opt::RequestedIp(a)) => Some(*a),
            _ => None,
        }
    }
    pub fn lease_time(&self) -> Option<u32> {
        match self.get(O_LEASE_TIME) {
            Some(Opt::LeaseTime(n)) => Some(*n),
            _ => None,
        }
    }
    pub fn subnet_mask(&self) -> Option<Ipv4> {
        match self.get(O_SUBNET_MASK) {
            Some(Opt::SubnetMask(a)) => Some(*a),
            _ => None,
        }
    }
    pub fn routers(&self) -> Vec<Ipv4> {
        match self.get(O_ROUTER) {
            Some(Opt::Routers(v)) => v.clone(),
            _ => Vec::new(),
        }
    }
    pub fn dns_servers(&self) -> Vec<Ipv4> {
        match self.get(O_DNS) {
            Some(Opt::DnsServers(v)) => v.clone(),
            _ => Vec::new(),
        }
    }
    pub fn domain_name(&self) -> Option<&str> {
        match self.get(O_DOMAIN_NAME) {
            Some(Opt::DomainName(s)) => Some(s.as_str()),
            _ => None,
        }
    }
    pub fn host_name(&self) -> Option<&str> {
        match self.get(O_HOST_NAME) {
            Some(Opt::HostName(s)) => Some(s.as_str()),
            _ => None,
        }
    }
    pub fn t1(&self) -> Option<u32> {
        match self.get(O_T1) {
            Some(Opt::T1(n)) => Some(*n),
            _ => None,
        }
    }
    pub fn t2(&self) -> Option<u32> {
        match self.get(O_T2) {
            Some(Opt::T2(n)) => Some(*n),
            _ => None,
        }
    }
    pub fn client_id(&self) -> Option<&[u8]> {
        match self.get(O_CLIENT_ID) {
            Some(Opt::ClientId(c)) => Some(c.as_slice()),
            _ => None,
        }
    }
    /// the message as bytes, padded with zeros to at least `min_len` bytes
    pub fn to_bytes(&self, min_len: usize) -> Vec<u8> {
        let mut b = alloc::vec![0u8; DHCP_MIN];
        b[0] = self.op;
        b[1] = self.htype;
        b[2] = self.hlen;
        b[3] = self.hops;
        put32(&mut b, 4, self.xid);
        put16(&mut b, 8, self.secs);
        put16(&mut b, 10, self.flags);
        b[12..16].copy_from_slice(&self.ciaddr.0);
        b[16..20].copy_from_slice(&self.yiaddr.0);
        b[20..24].copy_from_slice(&self.siaddr.0);
        b[24..28].copy_from_slice(&self.giaddr.0);
        b[28..44].copy_from_slice(&self.chaddr);
        let sn = self.sname.len().min(63);
        b[44..44 + sn].copy_from_slice(&self.sname[..sn]);
        let fl = self.file.len().min(127);
        b[108..108 + fl].copy_from_slice(&self.file[..fl]);
        b[BOOTP_HDR..DHCP_MIN].copy_from_slice(&MAGIC);
        for o in &self.opts {
            if o.code() == O_PAD || o.code() == O_END {
                continue;
            }
            let d = o.data();
            // RFC 3396: longer data goes into several options with the same code
            let mut chunks = d.chunks(255).peekable();
            if chunks.peek().is_none() {
                b.push(o.code());
                b.push(0);
            }
            for c in chunks {
                b.push(o.code());
                b.push(c.len() as u8);
                b.extend_from_slice(c);
            }
        }
        b.push(O_END);
        if b.len() < min_len {
            b.resize(min_len, 0);
        }
        b
    }
}
