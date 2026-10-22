// stp -- Bridge Protocol Data Units of the spanning tree protocols: Configuration and Topology
// Change Notification BPDUs (IEEE 802.1D-1998 clause 9) and RST BPDUs (IEEE 802.1D-2004 9.3.3,
// RSTP of 802.1w) with their flags, bridge and port identifiers, the recommended path costs,
// and the 802.3 frames with LLC that carry them to 01:80:c2:00:00:00
//
//   BridgeId(u64)            priority (16 bits: 4 settable, 12 system ID extension) and MAC:
//                            new(priority, mac), priority(), ext(), mac(); Display
//                            "4096/0/02:4a:00:00:01:00" as capture tools write it
//   PortId(u16)              priority (4 bits, in steps of 16) and port number (12 bits):
//                            new(priority, number), priority(), number(); Display "128.1"
//   path_cost(kbps)          the recommended Port Path Cost of 802.1D-2004 Table 17-3:
//                            20 000 000 000 / speed in kbit/s, 1 .. 200 000 000 (100M 200 000,
//                            1G 20 000, 2.5G 8 000, 10G 2 000, 25G 800, 100G 200)
//   Role                     the port role of an RST BPDU (Unknown, AltBackup, Root, Designated)
//   F_TC, F_PROPOSAL, F_ROLE, F_LEARNING, F_FORWARDING, F_AGREEMENT, F_TCA   the flag bits
//   Bpdu { kind, version, flags, root, cost, bridge, port, msg_age, max_age, hello, fwd_delay }
//                            times in 1/256 s (TIME_UNIT per second); config / tcn / rst build
//                            one from whole seconds (Times), with_role sets the role bits,
//                            times() reads them back in seconds
//   Bpdu::parse(b)           the BPDU after the LLC header, validated as 802.1D-2004 9.3.4 says
//                            (protocol identifier 0, type and length); Err with the reason
//   Bpdu::to_bytes()         35 (Configuration), 4 (TCN) or 36 bytes (RST)
//   Bpdu::frame(src)         the whole frame: 01:80:c2:00:00:00, the 802.3 length, LLC
//                            42 42 03, the BPDU, padded to 60 bytes
//   of_frame(frame)          the BPDU of a received frame (untagged or tagged), None for any
//                            other frame or a BPDU that does not validate
//   flags_text(kind, flags)  "Forwarding, Learning, Port Role: Designated" (dissectors)
//   summary(&Bpdu)           a one-line summary like a capture tool's ("RST. Root =
//                            4096/0/02:4a:00:00:01:00  Cost = 0  Port = 0x8001")
//
// Multi-byte fields are big endian.  The receiver checks what needs its own state (a Message Age
// that is not below Max Age, its own BPDU looped back); the codec only reads and writes.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::addr::Mac;
use crate::eth::{self, Llc};
use crate::{be16, be32, put16, put32};

/// BPDU timer values count in 1/256 s
pub const TIME_UNIT: u16 = 256;
/// the lengths of the three BPDUs
pub const CONFIG_LEN: usize = 35;
pub const TCN_LEN: usize = 4;
pub const RST_LEN: usize = 36;

pub const TYPE_CONFIG: u8 = 0x00;
pub const TYPE_RST: u8 = 0x02;
pub const TYPE_TCN: u8 = 0x80;
/// Protocol Version Identifiers: 802.1D STP, RSTP (802.1w), MSTP (802.1s)
pub const VERSION_STP: u8 = 0;
pub const VERSION_RSTP: u8 = 2;
pub const VERSION_MSTP: u8 = 3;

/// flag bits: Topology Change (both BPDUs), the RST BPDU's own ones, and the Topology Change
/// Acknowledgment of Configuration BPDUs
pub const F_TC: u8 = 0x01;
pub const F_PROPOSAL: u8 = 0x02;
pub const F_ROLE: u8 = 0x0C;
pub const F_LEARNING: u8 = 0x10;
pub const F_FORWARDING: u8 = 0x20;
pub const F_AGREEMENT: u8 = 0x40;
pub const F_TCA: u8 = 0x80;

/// a bridge identifier: the 16-bit priority (4 settable bits and the 12-bit system ID
/// extension, 0 for one spanning tree) and the bridge's MAC address; lower is better
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct BridgeId(pub u64);

impl BridgeId {
    pub fn new(priority: u16, mac: Mac) -> BridgeId {
        BridgeId(((priority as u64) << 48) | mac.to_u64())
    }
    /// the whole priority field (settable part plus system ID extension)
    pub fn priority(&self) -> u16 {
        (self.0 >> 48) as u16
    }
    /// the system ID extension (the low 12 bits of the priority field)
    pub fn ext(&self) -> u16 {
        self.priority() & 0x0FFF
    }
    pub fn mac(&self) -> Mac {
        Mac::from_u64(self.0 & 0xFFFF_FFFF_FFFF)
    }
    pub fn bytes(&self) -> [u8; 8] {
        self.0.to_be_bytes()
    }
    pub fn from_bytes(b: &[u8]) -> BridgeId {
        let mut a = [0u8; 8];
        a.copy_from_slice(&b[..8]);
        BridgeId(u64::from_be_bytes(a))
    }
}

impl fmt::Display for BridgeId {
    /// "32768/0/02:4a:00:00:01:00": settable priority / system ID extension / MAC
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}/{}", self.priority() & 0xF000, self.ext(), self.mac())
    }
}

/// a port identifier: 4 bits of priority (0 .. 240 in steps of 16) and a 12-bit port number
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct PortId(pub u16);

impl PortId {
    pub fn new(priority: u8, number: u16) -> PortId {
        PortId(((priority as u16 & 0xF0) << 8) | (number & 0x0FFF))
    }
    pub fn priority(&self) -> u8 {
        (self.0 >> 8) as u8 & 0xF0
    }
    pub fn number(&self) -> u16 {
        self.0 & 0x0FFF
    }
}

impl fmt::Display for PortId {
    /// "128.1"
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{}", self.priority(), self.number())
    }
}

/// the recommended Port Path Cost of a link speed (802.1D-2004 Table 17-3: 20 000 000 000
/// divided by the speed in kbit/s, within 1 .. 200 000 000)
pub fn path_cost(kbps: u32) -> u32 {
    (20_000_000_000u64 / kbps.max(1) as u64).clamp(1, 200_000_000) as u32
}

/// the port role an RST BPDU conveys (flag bits 3 and 4)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Unknown,
    AltBackup,
    Root,
    Designated,
}

impl Role {
    pub fn bits(self) -> u8 {
        (match self {
            Role::Unknown => 0,
            Role::AltBackup => 1,
            Role::Root => 2,
            Role::Designated => 3,
        }) << 2
    }
    pub fn of_flags(flags: u8) -> Role {
        match (flags & F_ROLE) >> 2 {
            1 => Role::AltBackup,
            2 => Role::Root,
            3 => Role::Designated,
            _ => Role::Unknown,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Role::Unknown => "Unknown",
            Role::AltBackup => "Alternate or Backup",
            Role::Root => "Root",
            Role::Designated => "Designated",
        }
    }
}

/// which BPDU
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Config,
    Tcn,
    Rst,
}

/// the four timer values of a BPDU in whole seconds
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Times {
    pub msg_age: u16,
    pub max_age: u16,
    pub hello: u16,
    pub fwd_delay: u16,
}

/// a BPDU; the times count in 1/256 s
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bpdu {
    pub kind: Kind,
    /// the Protocol Version Identifier as sent (0 for Configuration and TCN BPDUs, 2 for RSTP)
    pub version: u8,
    pub flags: u8,
    pub root: BridgeId,
    pub cost: u32,
    pub bridge: BridgeId,
    pub port: PortId,
    pub msg_age: u16,
    pub max_age: u16,
    pub hello: u16,
    pub fwd_delay: u16,
}

/// why bytes are no BPDU
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BpduErr {
    /// shorter than its type needs
    Short,
    /// the Protocol Identifier is not 0
    Protocol,
    /// a BPDU type (or type and version) this codec does not know
    Type,
}

impl BpduErr {
    pub fn text(self) -> &'static str {
        match self {
            BpduErr::Short => "shorter than its BPDU type needs",
            BpduErr::Protocol => "Protocol Identifier is not 0x0000",
            BpduErr::Type => "unknown BPDU type",
        }
    }
}

impl Bpdu {
    /// a Configuration BPDU (only the TC and TC acknowledgment flags count)
    pub fn config(flags: u8, root: BridgeId, cost: u32, bridge: BridgeId, port: PortId, t: Times) -> Bpdu {
        let s = |v: u16| v.saturating_mul(TIME_UNIT);
        Bpdu { kind: Kind::Config, version: VERSION_STP, flags: flags & (F_TC | F_TCA), root, cost, bridge, port, msg_age: s(t.msg_age), max_age: s(t.max_age), hello: s(t.hello), fwd_delay: s(t.fwd_delay) }
    }
    /// a Topology Change Notification BPDU
    pub fn tcn() -> Bpdu {
        Bpdu { kind: Kind::Tcn, version: VERSION_STP, flags: 0, root: BridgeId(0), cost: 0, bridge: BridgeId(0), port: PortId(0), msg_age: 0, max_age: 0, hello: 0, fwd_delay: 0 }
    }
    /// an RST BPDU (the port role is part of the flags: Role::bits, with_role)
    pub fn rst(flags: u8, root: BridgeId, cost: u32, bridge: BridgeId, port: PortId, t: Times) -> Bpdu {
        let mut b = Bpdu::config(0, root, cost, bridge, port, t);
        b.kind = Kind::Rst;
        b.version = VERSION_RSTP;
        b.flags = flags;
        b
    }
    /// the same BPDU conveying another port role
    pub fn with_role(mut self, r: Role) -> Bpdu {
        self.flags = (self.flags & !F_ROLE) | r.bits();
        self
    }
    pub fn role(&self) -> Role {
        if self.kind == Kind::Rst { Role::of_flags(self.flags) } else { Role::Unknown }
    }
    pub fn tc(&self) -> bool {
        self.flags & F_TC != 0
    }
    pub fn tca(&self) -> bool {
        self.kind == Kind::Config && self.flags & F_TCA != 0
    }
    pub fn proposal(&self) -> bool {
        self.kind == Kind::Rst && self.flags & F_PROPOSAL != 0
    }
    pub fn agreement(&self) -> bool {
        self.kind == Kind::Rst && self.flags & F_AGREEMENT != 0
    }
    pub fn learning(&self) -> bool {
        self.kind == Kind::Rst && self.flags & F_LEARNING != 0
    }
    pub fn forwarding(&self) -> bool {
        self.kind == Kind::Rst && self.flags & F_FORWARDING != 0
    }
    /// the timer values in whole seconds (rounded down)
    pub fn times(&self) -> Times {
        let s = |v: u16| v / TIME_UNIT;
        Times { msg_age: s(self.msg_age), max_age: s(self.max_age), hello: s(self.hello), fwd_delay: s(self.fwd_delay) }
    }

    /// the BPDU in `b` (the bytes after the LLC header), as 802.1D-2004 9.3.4 validates it
    pub fn parse(b: &[u8]) -> Result<Bpdu, BpduErr> {
        if b.len() < TCN_LEN {
            return Err(BpduErr::Short);
        }
        if be16(b, 0) != 0 {
            return Err(BpduErr::Protocol);
        }
        let (version, typ) = (b[2], b[3]);
        let kind = match typ {
            TYPE_CONFIG => Kind::Config,
            TYPE_TCN => Kind::Tcn,
            TYPE_RST if version >= VERSION_RSTP => Kind::Rst,
            _ => return Err(BpduErr::Type),
        };
        if kind == Kind::Tcn {
            return Ok(Bpdu { version, ..Bpdu::tcn() });
        }
        let need = if kind == Kind::Rst { RST_LEN } else { CONFIG_LEN };
        if b.len() < need {
            return Err(BpduErr::Short);
        }
        Ok(Bpdu {
            kind,
            version,
            flags: b[4],
            root: BridgeId::from_bytes(&b[5..13]),
            cost: be32(b, 13),
            bridge: BridgeId::from_bytes(&b[17..25]),
            port: PortId(be16(b, 25)),
            msg_age: be16(b, 27),
            max_age: be16(b, 29),
            hello: be16(b, 31),
            fwd_delay: be16(b, 33),
        })
    }

    /// the bytes of the BPDU
    pub fn to_bytes(&self) -> Vec<u8> {
        let typ = match self.kind {
            Kind::Config => TYPE_CONFIG,
            Kind::Tcn => TYPE_TCN,
            Kind::Rst => TYPE_RST,
        };
        if self.kind == Kind::Tcn {
            return alloc::vec![0, 0, self.version, typ];
        }
        let mut v = alloc::vec![0u8; if self.kind == Kind::Rst { RST_LEN } else { CONFIG_LEN }];
        v[2] = self.version;
        v[3] = typ;
        v[4] = self.flags;
        v[5..13].copy_from_slice(&self.root.bytes());
        put32(&mut v, 13, self.cost);
        v[17..25].copy_from_slice(&self.bridge.bytes());
        put16(&mut v, 25, self.port.0);
        put16(&mut v, 27, self.msg_age);
        put16(&mut v, 29, self.max_age);
        put16(&mut v, 31, self.hello);
        put16(&mut v, 33, self.fwd_delay);
        // an RST BPDU ends with Version 1 Length 0
        v
    }

    /// the frame that carries the BPDU from a port with MAC `src`: IEEE 802.3 to the bridge
    /// group address, LLC with SAP 0x42, padded to the 60-byte minimum
    pub fn frame(&self, src: Mac) -> Vec<u8> {
        let body = self.to_bytes();
        let mut f = Vec::with_capacity(eth::ETH_MIN);
        f.extend_from_slice(&Mac::STP.0);
        f.extend_from_slice(&src.0);
        f.extend_from_slice(&((Llc::LEN + body.len()) as u16).to_be_bytes());
        let mut llc = [0u8; Llc::LEN];
        Llc::STP.write(&mut llc);
        f.extend_from_slice(&llc);
        f.extend_from_slice(&body);
        eth::pad(&mut f);
        f
    }
}

/// the BPDU a received frame carries: to 01:80:c2:00:00:00 (tagged or not), an 802.3 length,
/// LLC with SAP 0x42; None for any other frame and for one that does not validate
pub fn of_frame(frame: &[u8]) -> Option<Bpdu> {
    let l2 = eth::L2::parse(frame)?;
    if l2.dst != Mac::STP || l2.ethertype > 1500 {
        return None;
    }
    let rest = l2.payload(frame);
    let llc = Llc::parse(rest)?;
    if llc.dsap != eth::SAP_STP || llc.ssap != eth::SAP_STP {
        return None;
    }
    let body = &rest[Llc::LEN..(l2.ethertype as usize).clamp(Llc::LEN, rest.len())];
    Bpdu::parse(body).ok()
}

/// the flags in words, as capture tools list them ("Agreement, Forwarding, Learning, Port Role:
/// Root"; "Topology Change" first, the acknowledgment last; "none")
pub fn flags_text(kind: Kind, flags: u8) -> String {
    let mut v: Vec<String> = Vec::new();
    if flags & F_TC != 0 {
        v.push(String::from("Topology Change"));
    }
    if kind == Kind::Rst {
        if flags & F_AGREEMENT != 0 {
            v.push(String::from("Agreement"));
        }
        if flags & F_FORWARDING != 0 {
            v.push(String::from("Forwarding"));
        }
        if flags & F_LEARNING != 0 {
            v.push(String::from("Learning"));
        }
        v.push(format!("Port Role: {}", Role::of_flags(flags).name()));
        if flags & F_PROPOSAL != 0 {
            v.push(String::from("Proposal"));
        }
    }
    if flags & F_TCA != 0 {
        v.push(String::from("Topology Change Acknowledgment"));
    }
    if v.is_empty() { String::from("none") } else { v.join(", ") }
}

/// one line about a BPDU, as capture tools sum one up ("Conf. TC + Root = 32768/0/02:4a:00:00:01:00
/// Cost = 20000  Port = 0x8002", "RST. Root = ...", "Topology Change Notification")
pub fn summary(b: &Bpdu) -> String {
    let what = match b.kind {
        Kind::Tcn => return String::from("Topology Change Notification"),
        Kind::Config => "Conf.",
        Kind::Rst if b.version >= VERSION_MSTP => "MST.",
        Kind::Rst => "RST.",
    };
    let mut s = String::from(what);
    if b.tc() {
        s.push_str(" TC +");
    }
    if b.tca() {
        s.push_str(" TCA +");
    }
    s.push_str(&format!(" Root = {}  Cost = {}  Port = 0x{:04x}", b.root, b.cost, b.port.0));
    s
}
