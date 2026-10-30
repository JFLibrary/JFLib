// snmp -- SNMP messages of version 1 (RFC 1157) and version 2c (RFC 1901, RFC 3416): the
// community, the PDUs GetRequest, GetNextRequest, GetBulkRequest, SetRequest, Response, the
// Trap of version 1, SNMPv2-Trap, InformRequest and Report with their variable bindings; the
// error statuses and their mapping to version 1 (RFC 3584); the objects every notification names
//
//   Version                 V1 (0) | V2c (1); other numbers (SNMPv3's 3) are no message here
//   Kind                    Get | GetNext | Response | Set | TrapV1 | GetBulk | Inform | TrapV2 |
//                           Report: tag() (0xA0 ..), of_tag(), name(), in_v1()
//   VarBind { oid, value }  VarBind::null(oid): a name to ask for
//   Pdu { kind, request_id, error_status, error_index, varbinds }   a GetBulkRequest keeps
//                           non-repeaters and max-repetitions in the two error fields
//                           (Pdu::bulk, non_repeaters(), max_repetitions())
//   TrapV1 { enterprise, agent_addr, generic, specific, timestamp, varbinds }
//   Body                    Pdu(Pdu) | Trap(TrapV1)
//   Message { version, community, body }: parse(b) / to_bytes() / pdu() / kind()
//   E_* (0 .. 18), error_name(status), v1_error(status)   RFC 3416 3; RFC 3584 4.4
//   G_* (0 .. 6), generic_name(g), trap_oid(generic, enterprise, specific)   RFC 3584 3.1
//   SYS_UPTIME_0, SNMP_TRAP_OID_0, SNMP_TRAP_ENTERPRISE_0, SNMP_TRAPS, COLD_START, WARM_START,
//   LINK_DOWN, LINK_UP, AUTH_FAILURE, IF_INDEX, IF_ADMIN_STATUS, IF_OPER_STATUS (RFC 3418, 2863)
//   PORT (161, the agent), PORT_TRAP (162, the notification receiver)
//
// parse refuses what an SNMP entity counts as an ASN.1 parse error (snmpInASNParseErrs): BER that
// is not well-formed (ber.rs), elements missing or in the wrong order, bytes after the message, a
// PDU tag the message's version does not have (GetBulk, Inform, SNMPv2-Trap and Report in
// version 1, the v1 Trap in 2c), and in version 1 the types it has not (Counter64, the
// exceptions).  A version other than 0 and 1 is Bad::Version (snmpInBadVersions).  Error
// status and index are read as they are; what they mean is the receiver's business.

use alloc::string::String;
use alloc::vec::Vec;

use crate::ber::{self, Oid, Reader, Value, Writer, T_IPADDRESS, T_SEQUENCE, T_TIMETICKS};
use crate::Ipv4;

/// the agent's UDP port
pub const PORT: u16 = 161;
/// the port notifications go to
pub const PORT_TRAP: u16 = 162;

/// the version of a community-based message
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Version {
    V1,
    V2c,
}

impl Version {
    /// the value of the version field (0 and 1)
    pub fn number(self) -> i64 {
        match self {
            Version::V1 => 0,
            Version::V2c => 1,
        }
    }
    pub fn of_number(n: i64) -> Option<Version> {
        match n {
            0 => Some(Version::V1),
            1 => Some(Version::V2c),
            _ => None,
        }
    }
    /// "1", "2c" as the tools take it after -v
    pub fn word(self) -> &'static str {
        match self {
            Version::V1 => "1",
            Version::V2c => "2c",
        }
    }
    pub fn parse(s: &str) -> Option<Version> {
        match s.trim() {
            "1" | "v1" => Some(Version::V1),
            "2c" | "v2c" | "2" => Some(Version::V2c),
            _ => None,
        }
    }
}

/// the kind of a PDU (its context tag)
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Get,
    GetNext,
    Response,
    Set,
    TrapV1,
    GetBulk,
    Inform,
    TrapV2,
    Report,
}

impl Kind {
    pub fn tag(self) -> u8 {
        0xA0 + match self {
            Kind::Get => 0,
            Kind::GetNext => 1,
            Kind::Response => 2,
            Kind::Set => 3,
            Kind::TrapV1 => 4,
            Kind::GetBulk => 5,
            Kind::Inform => 6,
            Kind::TrapV2 => 7,
            Kind::Report => 8,
        }
    }
    pub fn of_tag(t: u8) -> Option<Kind> {
        Some(match t {
            0xA0 => Kind::Get,
            0xA1 => Kind::GetNext,
            0xA2 => Kind::Response,
            0xA3 => Kind::Set,
            0xA4 => Kind::TrapV1,
            0xA5 => Kind::GetBulk,
            0xA6 => Kind::Inform,
            0xA7 => Kind::TrapV2,
            0xA8 => Kind::Report,
            _ => return None,
        })
    }
    /// its name in RFC 3416 (version 1 calls the Response GetResponse)
    pub fn name(self) -> &'static str {
        match self {
            Kind::Get => "GetRequest",
            Kind::GetNext => "GetNextRequest",
            Kind::Response => "Response",
            Kind::Set => "SetRequest",
            Kind::TrapV1 => "Trap",
            Kind::GetBulk => "GetBulkRequest",
            Kind::Inform => "InformRequest",
            Kind::TrapV2 => "SNMPv2-Trap",
            Kind::Report => "Report",
        }
    }
    /// version 1 has it (RFC 1157: get, get-next, get-response, set, trap)
    pub fn in_v1(self) -> bool {
        matches!(self, Kind::Get | Kind::GetNext | Kind::Response | Kind::Set | Kind::TrapV1)
    }
    /// version 2c has it (every one but the v1 Trap)
    pub fn in_v2c(self) -> bool {
        self != Kind::TrapV1
    }
}

/// a variable binding: a name and its value
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarBind {
    pub oid: Oid,
    pub value: Value,
}

impl VarBind {
    pub fn new(oid: Oid, value: Value) -> VarBind {
        VarBind { oid, value }
    }
    /// a name with NULL as its value: what a request asks for
    pub fn null(oid: Oid) -> VarBind {
        VarBind { oid, value: Value::Null }
    }
}

/// a PDU with request id, error status and index (GetBulk: non-repeaters and max-repetitions)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pdu {
    pub kind: Kind,
    pub request_id: i32,
    pub error_status: i32,
    pub error_index: i32,
    pub varbinds: Vec<VarBind>,
}

impl Pdu {
    /// a request (or notification) without an error
    pub fn new(kind: Kind, request_id: i32, varbinds: Vec<VarBind>) -> Pdu {
        Pdu { kind, request_id, error_status: 0, error_index: 0, varbinds }
    }
    /// a GetBulkRequest
    pub fn bulk(request_id: i32, non_repeaters: i32, max_repetitions: i32, varbinds: Vec<VarBind>) -> Pdu {
        Pdu { kind: Kind::GetBulk, request_id, error_status: non_repeaters, error_index: max_repetitions, varbinds }
    }
    /// GetBulk: the first N bindings are asked once (RFC 3416 4.2.3: negative counts as 0)
    pub fn non_repeaters(&self) -> usize {
        self.error_status.max(0) as usize
    }
    /// GetBulk: how often the others repeat
    pub fn max_repetitions(&self) -> usize {
        self.error_index.max(0) as usize
    }
}

/// the Trap of version 1 (RFC 1157 4.1.6)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrapV1 {
    /// the type of object that sends it (sysObjectID)
    pub enterprise: Oid,
    pub agent_addr: Ipv4,
    pub generic: i32,
    pub specific: i32,
    /// sysUpTime when it was sent
    pub timestamp: u32,
    pub varbinds: Vec<VarBind>,
}

/// what a message carries
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Pdu(Pdu),
    Trap(TrapV1),
}

/// a community-based message
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub version: Version,
    pub community: Vec<u8>,
    pub body: Body,
}

/// why bytes are no SNMP message of version 1 or 2c
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bad {
    /// not well-formed BER, or an element missing or out of place
    Ber(ber::Bad),
    /// a version this module does not speak (SNMPv3 is 3)
    Version(i64),
    /// a PDU tag that is none, or one the message's version does not have
    Pdu(u8),
    /// a type version 1 does not have (Counter64, an exception)
    V1Type(u8),
}

impl From<ber::Bad> for Bad {
    fn from(b: ber::Bad) -> Bad {
        Bad::Ber(b)
    }
}

impl Bad {
    /// what is wrong, for a dissector
    pub fn text(self) -> String {
        match self {
            Bad::Ber(b) => String::from(b.text()),
            Bad::Version(3) => String::from("SNMPv3 (version 3), which this dissector does not take apart"),
            Bad::Version(v) => alloc::format!("an unknown version ({})", v),
            Bad::Pdu(t) => alloc::format!("a PDU type this version does not have (0x{:02x})", t),
            Bad::V1Type(t) => alloc::format!("a type SNMPv1 does not have ({})", ber::type_name(t)),
        }
    }
}

/// the variable bindings of a PDU
fn read_varbinds(r: &mut Reader, version: Version) -> Result<Vec<VarBind>, Bad> {
    let mut list = r.seq(T_SEQUENCE)?;
    let mut v = Vec::new();
    while !list.is_empty() {
        let mut vb = list.seq(T_SEQUENCE)?;
        let oid = vb.oid()?;
        let t = vb.peek()?;
        let value = vb.value()?;
        vb.done()?;
        if version == Version::V1 && (matches!(value, Value::Counter64(_)) || value.is_exception()) {
            return Err(Bad::V1Type(t.tag));
        }
        v.push(VarBind { oid, value });
    }
    Ok(v)
}

fn write_varbinds(w: &mut Writer, vbs: &[VarBind]) {
    w.begin(T_SEQUENCE);
    for vb in vbs {
        w.begin(T_SEQUENCE);
        w.oid(&vb.oid);
        w.value(&vb.value);
        w.end();
    }
    w.end();
}

impl Message {
    /// a message of one PDU
    pub fn new(version: Version, community: &str, pdu: Pdu) -> Message {
        Message { version, community: Vec::from(community.as_bytes()), body: Body::Pdu(pdu) }
    }
    /// bytes as a message
    pub fn parse(b: &[u8]) -> Result<Message, Bad> {
        let mut top = Reader::new(b);
        let mut m = top.seq(T_SEQUENCE)?;
        top.done()?;
        let vn = m.int()?;
        let version = Version::of_number(vn).ok_or(Bad::Version(vn))?;
        let community = Vec::from(m.octets()?);
        let t = m.peek()?;
        let kind = Kind::of_tag(t.tag).ok_or(Bad::Pdu(t.tag))?;
        let ok = match version {
            Version::V1 => kind.in_v1(),
            Version::V2c => kind.in_v2c(),
        };
        if !ok {
            return Err(Bad::Pdu(t.tag));
        }
        let mut p = m.seq(t.tag)?;
        m.done()?;
        let body = if kind == Kind::TrapV1 {
            let enterprise = p.oid()?;
            let at = p.expect(T_IPADDRESS)?;
            let a = at.contents(p.buf());
            if a.len() != 4 {
                return Err(Bad::Ber(ber::Bad::Size));
            }
            let agent_addr = Ipv4([a[0], a[1], a[2], a[3]]);
            let generic = p.int32()?;
            let specific = p.int32()?;
            let timestamp = p.uint(T_TIMETICKS, u32::MAX as u64)? as u32;
            let varbinds = read_varbinds(&mut p, version)?;
            p.done()?;
            Body::Trap(TrapV1 { enterprise, agent_addr, generic, specific, timestamp, varbinds })
        } else {
            let request_id = p.int32()?;
            let error_status = p.int32()?;
            let error_index = p.int32()?;
            let varbinds = read_varbinds(&mut p, version)?;
            p.done()?;
            Body::Pdu(Pdu { kind, request_id, error_status, error_index, varbinds })
        };
        Ok(Message { version, community, body })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.begin(T_SEQUENCE);
        w.int(self.version.number());
        w.octets(&self.community);
        match &self.body {
            Body::Pdu(p) => {
                w.begin(p.kind.tag());
                w.int(p.request_id as i64);
                w.int(p.error_status as i64);
                w.int(p.error_index as i64);
                write_varbinds(&mut w, &p.varbinds);
                w.end();
            }
            Body::Trap(t) => {
                w.begin(Kind::TrapV1.tag());
                w.oid(&t.enterprise);
                w.raw(&ber::tlv(T_IPADDRESS, &t.agent_addr.0));
                w.int(t.generic as i64);
                w.int(t.specific as i64);
                w.uint(T_TIMETICKS, t.timestamp as u64);
                write_varbinds(&mut w, &t.varbinds);
                w.end();
            }
        }
        w.end();
        w.into_vec()
    }
    /// the PDU (None for a v1 Trap)
    pub fn pdu(&self) -> Option<&Pdu> {
        match &self.body {
            Body::Pdu(p) => Some(p),
            Body::Trap(_) => None,
        }
    }
    pub fn kind(&self) -> Kind {
        match &self.body {
            Body::Pdu(p) => p.kind,
            Body::Trap(_) => Kind::TrapV1,
        }
    }
    /// the community as text (bytes that are no UTF-8 replaced)
    pub fn community_text(&self) -> String {
        String::from_utf8_lossy(&self.community).into_owned()
    }
    /// the variable bindings of the PDU or the trap
    pub fn varbinds(&self) -> &[VarBind] {
        match &self.body {
            Body::Pdu(p) => &p.varbinds,
            Body::Trap(t) => &t.varbinds,
        }
    }
}

// ---------------------------------------------------------------- errors

pub const E_NO_ERROR: i32 = 0;
pub const E_TOO_BIG: i32 = 1;
pub const E_NO_SUCH_NAME: i32 = 2;
pub const E_BAD_VALUE: i32 = 3;
pub const E_READ_ONLY: i32 = 4;
pub const E_GEN_ERR: i32 = 5;
pub const E_NO_ACCESS: i32 = 6;
pub const E_WRONG_TYPE: i32 = 7;
pub const E_WRONG_LENGTH: i32 = 8;
pub const E_WRONG_ENCODING: i32 = 9;
pub const E_WRONG_VALUE: i32 = 10;
pub const E_NO_CREATION: i32 = 11;
pub const E_INCONSISTENT_VALUE: i32 = 12;
pub const E_RESOURCE_UNAVAILABLE: i32 = 13;
pub const E_COMMIT_FAILED: i32 = 14;
pub const E_UNDO_FAILED: i32 = 15;
pub const E_AUTHORIZATION_ERROR: i32 = 16;
pub const E_NOT_WRITABLE: i32 = 17;
pub const E_INCONSISTENT_NAME: i32 = 18;

/// the name of an error status (RFC 3416 3)
pub fn error_name(e: i32) -> &'static str {
    match e {
        E_NO_ERROR => "noError",
        E_TOO_BIG => "tooBig",
        E_NO_SUCH_NAME => "noSuchName",
        E_BAD_VALUE => "badValue",
        E_READ_ONLY => "readOnly",
        E_GEN_ERR => "genErr",
        E_NO_ACCESS => "noAccess",
        E_WRONG_TYPE => "wrongType",
        E_WRONG_LENGTH => "wrongLength",
        E_WRONG_ENCODING => "wrongEncoding",
        E_WRONG_VALUE => "wrongValue",
        E_NO_CREATION => "noCreation",
        E_INCONSISTENT_VALUE => "inconsistentValue",
        E_RESOURCE_UNAVAILABLE => "resourceUnavailable",
        E_COMMIT_FAILED => "commitFailed",
        E_UNDO_FAILED => "undoFailed",
        E_AUTHORIZATION_ERROR => "authorizationError",
        E_NOT_WRITABLE => "notWritable",
        E_INCONSISTENT_NAME => "inconsistentName",
        _ => "unknown",
    }
}

/// the status a version-1 response carries for an error of RFC 3416 (RFC 3584 4.4)
pub fn v1_error(e: i32) -> i32 {
    match e {
        E_WRONG_VALUE | E_WRONG_ENCODING | E_WRONG_TYPE | E_WRONG_LENGTH | E_INCONSISTENT_VALUE => E_BAD_VALUE,
        E_NO_ACCESS | E_NOT_WRITABLE | E_NO_CREATION | E_INCONSISTENT_NAME | E_AUTHORIZATION_ERROR => E_NO_SUCH_NAME,
        E_RESOURCE_UNAVAILABLE | E_COMMIT_FAILED | E_UNDO_FAILED => E_GEN_ERR,
        other => other,
    }
}

// ---------------------------------------------------------------- notifications

pub const G_COLD_START: i32 = 0;
pub const G_WARM_START: i32 = 1;
pub const G_LINK_DOWN: i32 = 2;
pub const G_LINK_UP: i32 = 3;
pub const G_AUTH_FAILURE: i32 = 4;
pub const G_EGP_NEIGHBOR_LOSS: i32 = 5;
pub const G_ENTERPRISE: i32 = 6;

/// the name of a generic trap of version 1
pub fn generic_name(g: i32) -> &'static str {
    match g {
        G_COLD_START => "coldStart",
        G_WARM_START => "warmStart",
        G_LINK_DOWN => "linkDown",
        G_LINK_UP => "linkUp",
        G_AUTH_FAILURE => "authenticationFailure",
        G_EGP_NEIGHBOR_LOSS => "egpNeighborLoss",
        G_ENTERPRISE => "enterpriseSpecific",
        _ => "unknown",
    }
}

/// sysUpTime.0, the first binding of every SNMPv2 notification
pub const SYS_UPTIME_0: [u32; 9] = [1, 3, 6, 1, 2, 1, 1, 3, 0];
/// snmpTrapOID.0, the second one: which notification it is
pub const SNMP_TRAP_OID_0: [u32; 11] = [1, 3, 6, 1, 6, 3, 1, 1, 4, 1, 0];
/// snmpTrapEnterprise.0 (RFC 3584 3.1)
pub const SNMP_TRAP_ENTERPRISE_0: [u32; 11] = [1, 3, 6, 1, 6, 3, 1, 1, 4, 3, 0];
/// snmpTraps: the generic notifications are its children (coldStart .1 .. egpNeighborLoss .6)
pub const SNMP_TRAPS: [u32; 9] = [1, 3, 6, 1, 6, 3, 1, 1, 5];
pub const COLD_START: [u32; 10] = [1, 3, 6, 1, 6, 3, 1, 1, 5, 1];
pub const WARM_START: [u32; 10] = [1, 3, 6, 1, 6, 3, 1, 1, 5, 2];
pub const LINK_DOWN: [u32; 10] = [1, 3, 6, 1, 6, 3, 1, 1, 5, 3];
pub const LINK_UP: [u32; 10] = [1, 3, 6, 1, 6, 3, 1, 1, 5, 4];
pub const AUTH_FAILURE: [u32; 10] = [1, 3, 6, 1, 6, 3, 1, 1, 5, 5];
/// the columns of ifTable a linkDown or linkUp names (IF-MIB, RFC 2863)
pub const IF_INDEX: [u32; 10] = [1, 3, 6, 1, 2, 1, 2, 2, 1, 1];
pub const IF_ADMIN_STATUS: [u32; 10] = [1, 3, 6, 1, 2, 1, 2, 2, 1, 7];
pub const IF_OPER_STATUS: [u32; 10] = [1, 3, 6, 1, 2, 1, 2, 2, 1, 8];

/// the SNMPv2 notification of a version-1 trap (RFC 3584 3.1): snmpTraps.(generic + 1) for the
/// generic ones, else enterprise.0.specific
pub fn trap_oid(generic: i32, enterprise: &Oid, specific: i32) -> Oid {
    if (0..G_ENTERPRISE).contains(&generic) {
        let mut v = Vec::from(&SNMP_TRAPS[..]);
        v.push(generic as u32 + 1);
        return Oid(v);
    }
    enterprise.child(&[0, specific.max(0) as u32])
}

/// the generic trap of an SNMPv2 notification (RFC 3584 3.2): Some(0 .. 5) for the generic
/// ones, None for an enterprise-specific one
pub fn generic_of(trap: &Oid) -> Option<i32> {
    let a = trap.arcs();
    if a.len() == 10 && a.starts_with(&SNMP_TRAPS) && (1..=6).contains(&a[9]) {
        return Some(a[9] as i32 - 1);
    }
    None
}

/// the value of the binding named `oid` (the first one)
pub fn find<'a>(vbs: &'a [VarBind], oid: &[u32]) -> Option<&'a Value> {
    vbs.iter().find(|v| v.oid.arcs() == oid).map(|v| &v.value)
}

/// the notification an SNMPv2-Trap or InformRequest carries (snmpTrapOID.0, the second binding)
pub fn notification(vbs: &[VarBind]) -> Option<&Oid> {
    match find(vbs, &SNMP_TRAP_OID_0) {
        Some(Value::Oid(o)) => Some(o),
        _ => None,
    }
}
