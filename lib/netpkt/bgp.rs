// bgp -- BGP-4 messages byte for byte (RFC 4271): the 19-byte header with its marker, OPEN with
// its optional parameters and the capabilities of RFC 5492 (multiprotocol IPv4 unicast of RFC
// 4760, route refresh of RFC 2918, 4-octet AS numbers of RFC 6793), UPDATE with withdrawn routes,
// path attributes and NLRI, NOTIFICATION with the codes and subcodes of RFC 4271 and the Cease
// subcodes of RFC 4486, KEEPALIVE and ROUTE-REFRESH; the checks of RFC 4271 6.1 to 6.3 as the
// NOTIFICATION a receiver sends
//
//   frame(stream)            the length of the first message of a TCP byte stream once it is
//                            complete (Ok(None): more bytes needed) after the header checks of
//                            6.1 (marker, length, type and the length of the type)
//   BgpView::parse(b)        one message: kind(), len(), body(), marker_ok(); None when the
//                            bytes are shorter than the header or than its length field
//   Message::read(b, as4)    a whole message as a record (Err: the BgpErr whose NOTIFICATION a
//                            receiver sends); to_bytes(as4) writes it with marker and length.
//                            `as4`: the session negotiated 4-octet AS numbers (AS_PATH and
//                            AGGREGATOR carry them)
//   Open { version, asn, hold, id, params }   `asn` as on the wire (AS_TRANS for a 4-octet AS);
//                            Open::new(asn, hold, id, caps) fills it in; caps(), as4(),
//                            peer_as(), has_cap(code)
//   OptParam::Caps(Vec<Cap>) / Other;  Cap::Mp { afi, safi }, RouteRefresh, As4(asn), Other
//   Update { withdrawn, attrs, nlri }; Attr { flags, val } with Attr::new(val) (the flags the
//                            RFCs give the type) and kind(); AttrVal::Origin, AsPath (segments),
//                            NextHop, Med, LocalPref, AtomicAggregate, Aggregator, Communities,
//                            As4Path, As4Aggregator, Other (unknown types byte for byte);
//                            Update::get(kind), origin(), as_path(), next_hop(), med(),
//                            local_pref(), communities()
//   Segment { set, asns }    an AS_SET or AS_SEQUENCE; path_len(segs) counts a set as one AS
//                            (9.1.2.2), path_text(segs) "65002 65003 {65010,65011}"
//   Notification { code, sub, data };  error_name(code, sub), code_name(code)
//   BgpErr { code, sub, data }   what a receiver refuses and the NOTIFICATION it sends for it;
//                            text() in English
//   prefix_len(p) / write_prefix / read_prefixes   NLRI: the length in bits, then the bytes needed
//   as_path_4(value)         a dissector's guess whether an AS_PATH carries 4-octet numbers
//   origin_code(o) / origin_name(o) / community_text(c) / afi_name / cap_name   for dissectors
//
// Constants: PORT (179), VERSION, HDR_LEN, MAX_LEN, MARKER, the minimum lengths OPEN_MIN,
// UPDATE_MIN, NOTIFICATION_MIN, ROUTE_REFRESH_LEN, the types T_*, OPT_CAPS, the capability codes
// CAP_*, AFI_IPV4 / SAFI_UNICAST, AS_TRANS, the attribute flags F_* and types A_*, ORIGIN_*,
// SEG_*, the error codes E_* with their subcodes (HE_*, OE_*, UE_*, CE_*).
//
// The codec reads and writes; what needs the session (the peer's AS, its BGP identifier against
// the own one, the hold time a speaker accepts, NEXT_HOP semantics, AS path loops, LOCAL_PREF
// from an internal peer) is the speaker's business.  Attributes keep their flags and order, so a
// message read and written again is the same bytes (the unused low four flag bits too); a
// malformed AS4_PATH or AS4_AGGREGATOR is kept as Other (RFC 6793 6: attribute discard).  RFC
// 7606 (treat-as-withdraw) is not here: every error is a NOTIFICATION as RFC 4271 says.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::addr::{Ipv4, Prefix4};
use crate::{be16, be32};

/// the TCP port of BGP
pub const PORT: u16 = 179;
/// BGP version 4
pub const VERSION: u8 = 4;
/// the header (marker, length, type) and the longest message (RFC 4271 4)
pub const HDR_LEN: usize = 19;
pub const MAX_LEN: usize = 4096;
/// sixteen bytes of ones
pub const MARKER: [u8; 16] = [0xFF; 16];
/// the shortest OPEN, UPDATE and NOTIFICATION, the length of ROUTE-REFRESH
pub const OPEN_MIN: usize = 29;
pub const UPDATE_MIN: usize = 23;
pub const NOTIFICATION_MIN: usize = 21;
pub const ROUTE_REFRESH_LEN: usize = 23;

/// message types
pub const T_OPEN: u8 = 1;
pub const T_UPDATE: u8 = 2;
pub const T_NOTIFICATION: u8 = 3;
pub const T_KEEPALIVE: u8 = 4;
pub const T_ROUTE_REFRESH: u8 = 5;

/// the optional parameter that carries capabilities (RFC 5492)
pub const OPT_CAPS: u8 = 2;
/// capability codes: multiprotocol (RFC 4760), route refresh (RFC 2918), 4-octet AS (RFC 6793)
pub const CAP_MP: u8 = 1;
pub const CAP_ROUTE_REFRESH: u8 = 2;
pub const CAP_AS4: u8 = 65;
/// the address family and sub-family of IPv4 unicast
pub const AFI_IPV4: u16 = 1;
pub const SAFI_UNICAST: u8 = 1;
/// the 2-octet AS a 4-octet one shows as to an old speaker (RFC 6793)
pub const AS_TRANS: u16 = 23456;

/// attribute flags
pub const F_OPTIONAL: u8 = 0x80;
pub const F_TRANSITIVE: u8 = 0x40;
pub const F_PARTIAL: u8 = 0x20;
pub const F_EXTENDED: u8 = 0x10;

/// attribute types
pub const A_ORIGIN: u8 = 1;
pub const A_AS_PATH: u8 = 2;
pub const A_NEXT_HOP: u8 = 3;
pub const A_MED: u8 = 4;
pub const A_LOCAL_PREF: u8 = 5;
pub const A_ATOMIC_AGGREGATE: u8 = 6;
pub const A_AGGREGATOR: u8 = 7;
pub const A_COMMUNITIES: u8 = 8;
pub const A_AS4_PATH: u8 = 17;
pub const A_AS4_AGGREGATOR: u8 = 18;

/// ORIGIN values
pub const ORIGIN_IGP: u8 = 0;
pub const ORIGIN_EGP: u8 = 1;
pub const ORIGIN_INCOMPLETE: u8 = 2;

/// AS_PATH segment types
pub const SEG_SET: u8 = 1;
pub const SEG_SEQUENCE: u8 = 2;

/// error codes (RFC 4271 4.5)
pub const E_HEADER: u8 = 1;
pub const E_OPEN: u8 = 2;
pub const E_UPDATE: u8 = 3;
pub const E_HOLD: u8 = 4;
pub const E_FSM: u8 = 5;
pub const E_CEASE: u8 = 6;
/// Message Header Error subcodes
pub const HE_SYNC: u8 = 1;
pub const HE_LENGTH: u8 = 2;
pub const HE_TYPE: u8 = 3;
/// OPEN Message Error subcodes (7: RFC 5492)
pub const OE_UNSPECIFIC: u8 = 0;
pub const OE_VERSION: u8 = 1;
pub const OE_PEER_AS: u8 = 2;
pub const OE_ID: u8 = 3;
pub const OE_OPT_PARAM: u8 = 4;
pub const OE_HOLD: u8 = 6;
pub const OE_CAPABILITY: u8 = 7;
/// UPDATE Message Error subcodes
pub const UE_ATTR_LIST: u8 = 1;
pub const UE_UNRECOGNIZED: u8 = 2;
pub const UE_MISSING: u8 = 3;
pub const UE_FLAGS: u8 = 4;
pub const UE_LENGTH: u8 = 5;
pub const UE_ORIGIN: u8 = 6;
pub const UE_NEXT_HOP: u8 = 8;
pub const UE_OPTIONAL: u8 = 9;
pub const UE_NETWORK: u8 = 10;
pub const UE_AS_PATH: u8 = 11;
/// Cease subcodes (RFC 4486)
pub const CE_MAX_PREFIXES: u8 = 1;
pub const CE_SHUTDOWN: u8 = 2;
pub const CE_DECONFIGURED: u8 = 3;
pub const CE_RESET: u8 = 4;
pub const CE_REJECTED: u8 = 5;
pub const CE_CONFIG: u8 = 6;
pub const CE_COLLISION: u8 = 7;
pub const CE_RESOURCES: u8 = 8;

// ---------------------------------------------------------------- errors

/// what a receiver refuses: the code, subcode and data of the NOTIFICATION it sends
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BgpErr {
    pub code: u8,
    pub sub: u8,
    pub data: Vec<u8>,
}

impl BgpErr {
    pub fn new(code: u8, sub: u8, data: Vec<u8>) -> BgpErr {
        BgpErr { code, sub, data }
    }
    /// the NOTIFICATION for it
    pub fn notification(&self) -> Notification {
        Notification { code: self.code, sub: self.sub, data: self.data.clone() }
    }
    /// "UPDATE Message Error / Missing Well-known Attribute"
    pub fn text(&self) -> String {
        format!("{} / {}", code_name(self.code), error_name(self.code, self.sub))
    }
}

/// the name of an error code (RFC 4271 4.5)
pub fn code_name(code: u8) -> &'static str {
    match code {
        E_HEADER => "Message Header Error",
        E_OPEN => "OPEN Message Error",
        E_UPDATE => "UPDATE Message Error",
        E_HOLD => "Hold Timer Expired",
        E_FSM => "Finite State Machine Error",
        E_CEASE => "Cease",
        _ => "Unknown error",
    }
}

/// the name of a subcode ("Bad Peer AS"); the code's name where it has none
pub fn error_name(code: u8, sub: u8) -> &'static str {
    match (code, sub) {
        (E_HEADER, HE_SYNC) => "Connection Not Synchronized",
        (E_HEADER, HE_LENGTH) => "Bad Message Length",
        (E_HEADER, HE_TYPE) => "Bad Message Type",
        (E_OPEN, OE_VERSION) => "Unsupported Version Number",
        (E_OPEN, OE_PEER_AS) => "Bad Peer AS",
        (E_OPEN, OE_ID) => "Bad BGP Identifier",
        (E_OPEN, OE_OPT_PARAM) => "Unsupported Optional Parameter",
        (E_OPEN, OE_HOLD) => "Unacceptable Hold Time",
        (E_OPEN, OE_CAPABILITY) => "Unsupported Capability",
        (E_UPDATE, UE_ATTR_LIST) => "Malformed Attribute List",
        (E_UPDATE, UE_UNRECOGNIZED) => "Unrecognized Well-known Attribute",
        (E_UPDATE, UE_MISSING) => "Missing Well-known Attribute",
        (E_UPDATE, UE_FLAGS) => "Attribute Flags Error",
        (E_UPDATE, UE_LENGTH) => "Attribute Length Error",
        (E_UPDATE, UE_ORIGIN) => "Invalid ORIGIN Attribute",
        (E_UPDATE, UE_NEXT_HOP) => "Invalid NEXT_HOP Attribute",
        (E_UPDATE, UE_OPTIONAL) => "Optional Attribute Error",
        (E_UPDATE, UE_NETWORK) => "Invalid Network Field",
        (E_UPDATE, UE_AS_PATH) => "Malformed AS_PATH",
        (E_CEASE, CE_MAX_PREFIXES) => "Maximum Number of Prefixes Reached",
        (E_CEASE, CE_SHUTDOWN) => "Administrative Shutdown",
        (E_CEASE, CE_DECONFIGURED) => "Peer De-configured",
        (E_CEASE, CE_RESET) => "Administrative Reset",
        (E_CEASE, CE_REJECTED) => "Connection Rejected",
        (E_CEASE, CE_CONFIG) => "Other Configuration Change",
        (E_CEASE, CE_COLLISION) => "Connection Collision Resolution",
        (E_CEASE, CE_RESOURCES) => "Out of Resources",
        (E_OPEN | E_UPDATE | E_CEASE, 0) => "Unspecific",
        (c, _) => code_name(c),
    }
}

fn err(code: u8, sub: u8) -> BgpErr {
    BgpErr::new(code, sub, Vec::new())
}

// ---------------------------------------------------------------- the stream and the header

/// the length of the first message in a TCP byte stream once all of it is there (None: more
/// bytes needed); Err with the NOTIFICATION of 6.1 for a bad header
pub fn frame(b: &[u8]) -> Result<Option<usize>, BgpErr> {
    if b.len() < HDR_LEN {
        return Ok(None);
    }
    if b[..16] != MARKER {
        return Err(err(E_HEADER, HE_SYNC));
    }
    let len = be16(b, 16) as usize;
    let bad_len = || BgpErr::new(E_HEADER, HE_LENGTH, b[16..18].to_vec());
    if !(HDR_LEN..=MAX_LEN).contains(&len) {
        return Err(bad_len());
    }
    let fits = match b[18] {
        T_OPEN => len >= OPEN_MIN,
        T_UPDATE => len >= UPDATE_MIN,
        T_NOTIFICATION => len >= NOTIFICATION_MIN,
        T_KEEPALIVE => len == HDR_LEN,
        T_ROUTE_REFRESH => len == ROUTE_REFRESH_LEN,
        t => return Err(BgpErr::new(E_HEADER, HE_TYPE, alloc::vec![t])),
    };
    if !fits {
        return Err(bad_len());
    }
    Ok(if b.len() >= len { Some(len) } else { None })
}

/// one message: the header and as many bytes as its length says
#[derive(Clone, Copy, Debug)]
pub struct BgpView<'a> {
    b: &'a [u8],
}

impl<'a> BgpView<'a> {
    /// None when the bytes are shorter than the header or than its length field (bytes after
    /// the length are left out)
    pub fn parse(b: &'a [u8]) -> Option<BgpView<'a>> {
        if b.len() < HDR_LEN {
            return None;
        }
        let len = be16(b, 16) as usize;
        if len < HDR_LEN || len > b.len() {
            return None;
        }
        Some(BgpView { b: &b[..len] })
    }
    pub fn marker_ok(&self) -> bool {
        self.b[..16] == MARKER
    }
    pub fn len(&self) -> usize {
        self.b.len()
    }
    pub fn is_empty(&self) -> bool {
        false
    }
    pub fn kind(&self) -> u8 {
        self.b[18]
    }
    pub fn body(&self) -> &'a [u8] {
        &self.b[HDR_LEN..]
    }
    pub fn bytes(&self) -> &'a [u8] {
        self.b
    }
}

/// the name of a message type ("UPDATE")
pub fn type_name(t: u8) -> &'static str {
    match t {
        T_OPEN => "OPEN",
        T_UPDATE => "UPDATE",
        T_NOTIFICATION => "NOTIFICATION",
        T_KEEPALIVE => "KEEPALIVE",
        T_ROUTE_REFRESH => "ROUTE-REFRESH",
        _ => "Unknown",
    }
}

// ---------------------------------------------------------------- prefixes (NLRI)

/// the bytes a prefix takes in NLRI: its length, then the bytes that hold its bits
pub fn prefix_len(p: &Prefix4) -> usize {
    1 + (p.len as usize).div_ceil(8)
}

/// appends a prefix as NLRI
pub fn write_prefix(out: &mut Vec<u8>, p: &Prefix4) {
    out.push(p.len);
    out.extend_from_slice(&p.trunc().addr.0[..(p.len as usize).div_ceil(8)]);
}

/// the prefixes of a withdrawn-routes or NLRI field; Err(Invalid Network Field) for a length
/// above 32 or bytes that end early.  Bits after the length are dropped (the network form).
pub fn read_prefixes(b: &[u8]) -> Result<Vec<Prefix4>, BgpErr> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let len = b[i];
        let n = (len as usize).div_ceil(8);
        if len > 32 || i + 1 + n > b.len() {
            return Err(err(E_UPDATE, UE_NETWORK));
        }
        let mut a = [0u8; 4];
        a[..n].copy_from_slice(&b[i + 1..i + 1 + n]);
        out.push(Prefix4 { addr: Ipv4(a), len }.trunc());
        i += 1 + n;
    }
    Ok(out)
}

// ---------------------------------------------------------------- OPEN

/// a capability (RFC 5492)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cap {
    /// multiprotocol extensions (RFC 4760)
    Mp { afi: u16, safi: u8 },
    /// route refresh (RFC 2918)
    RouteRefresh,
    /// 4-octet AS numbers with the speaker's AS (RFC 6793)
    As4(u32),
    /// anything else, byte for byte
    Other { code: u8, data: Vec<u8> },
}

impl Cap {
    pub fn code(&self) -> u8 {
        match self {
            Cap::Mp { .. } => CAP_MP,
            Cap::RouteRefresh => CAP_ROUTE_REFRESH,
            Cap::As4(_) => CAP_AS4,
            Cap::Other { code, .. } => *code,
        }
    }
    fn value(&self) -> Vec<u8> {
        match self {
            Cap::Mp { afi, safi } => {
                let a = afi.to_be_bytes();
                alloc::vec![a[0], a[1], 0, *safi]
            }
            Cap::RouteRefresh => Vec::new(),
            Cap::As4(n) => n.to_be_bytes().to_vec(),
            Cap::Other { data, .. } => data.clone(),
        }
    }
    fn read(code: u8, v: &[u8]) -> Cap {
        match (code, v.len()) {
            (CAP_MP, 4) => Cap::Mp { afi: be16(v, 0), safi: v[3] },
            (CAP_ROUTE_REFRESH, 0) => Cap::RouteRefresh,
            (CAP_AS4, 4) => Cap::As4(be32(v, 0)),
            _ => Cap::Other { code, data: v.to_vec() },
        }
    }
}

/// the name of a capability code
pub fn cap_name(code: u8) -> &'static str {
    match code {
        CAP_MP => "Multiprotocol extensions",
        CAP_ROUTE_REFRESH => "Route refresh",
        CAP_AS4 => "4-octet AS number",
        64 => "Graceful restart",
        69 => "ADD-PATH",
        70 => "Enhanced route refresh",
        128 => "Route refresh (Cisco)",
        _ => "Unknown capability",
    }
}

/// the name of an address family ("IPv4")
pub fn afi_name(afi: u16, safi: u8) -> String {
    let a = match afi {
        1 => "IPv4",
        2 => "IPv6",
        25 => "L2VPN",
        _ => return format!("AFI {} SAFI {}", afi, safi),
    };
    let s = match safi {
        1 => "unicast",
        2 => "multicast",
        4 => "labeled unicast",
        128 => "VPN",
        _ => return format!("{} SAFI {}", a, safi),
    };
    format!("{} {}", a, s)
}

/// an optional parameter of an OPEN
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptParam {
    /// capabilities (parameter type 2)
    Caps(Vec<Cap>),
    /// another type, byte for byte (a receiver refuses it: Unsupported Optional Parameter)
    Other { kind: u8, data: Vec<u8> },
}

/// an OPEN message (RFC 4271 4.2)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Open {
    pub version: u8,
    /// My Autonomous System as on the wire (AS_TRANS for a 4-octet AS)
    pub asn: u16,
    pub hold: u16,
    pub id: Ipv4,
    pub params: Vec<OptParam>,
}

impl Open {
    /// version 4 from `asn` (AS_TRANS on the wire above 65535) with one parameter of
    /// capabilities (none when `caps` is empty)
    pub fn new(asn: u32, hold: u16, id: Ipv4, caps: Vec<Cap>) -> Open {
        let wire = if asn > 0xFFFF { AS_TRANS } else { asn as u16 };
        let params = if caps.is_empty() { Vec::new() } else { alloc::vec![OptParam::Caps(caps)] };
        Open { version: VERSION, asn: wire, hold, id, params }
    }
    /// every capability, in order
    pub fn caps(&self) -> impl Iterator<Item = &Cap> {
        self.params.iter().flat_map(|p| match p {
            OptParam::Caps(c) => c.as_slice(),
            OptParam::Other { .. } => &[],
        })
    }
    pub fn has_cap(&self, code: u8) -> bool {
        self.caps().any(|c| c.code() == code)
    }
    /// the AS of the 4-octet capability
    pub fn as4(&self) -> Option<u32> {
        self.caps().find_map(|c| match c {
            Cap::As4(n) => Some(*n),
            _ => None,
        })
    }
    /// the speaker's AS: the 4-octet one where it sent the capability, else My Autonomous System
    pub fn peer_as(&self) -> u32 {
        self.as4().unwrap_or(self.asn as u32)
    }
    /// it offers IPv4 unicast (multiprotocol), or names no family at all (then IPv4 unicast
    /// alone, RFC 4760 8)
    pub fn ipv4_unicast(&self) -> bool {
        let mp: Vec<&Cap> = self.caps().filter(|c| c.code() == CAP_MP).collect();
        mp.is_empty() || mp.iter().any(|c| matches!(c, Cap::Mp { afi: AFI_IPV4, safi: SAFI_UNICAST }))
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.push(self.version);
        out.extend_from_slice(&self.asn.to_be_bytes());
        out.extend_from_slice(&self.hold.to_be_bytes());
        out.extend_from_slice(&self.id.0);
        let mut p: Vec<u8> = Vec::new();
        for o in &self.params {
            match o {
                OptParam::Caps(cs) => {
                    let mut v: Vec<u8> = Vec::new();
                    for c in cs {
                        let x = c.value();
                        v.push(c.code());
                        v.push(x.len() as u8);
                        v.extend_from_slice(&x);
                    }
                    p.push(OPT_CAPS);
                    p.push(v.len() as u8);
                    p.extend_from_slice(&v);
                }
                OptParam::Other { kind, data } => {
                    p.push(*kind);
                    p.push(data.len() as u8);
                    p.extend_from_slice(data);
                }
            }
        }
        out.push(p.len() as u8);
        out.extend_from_slice(&p);
    }

    /// the body of an OPEN (6.2): the version, the optional parameters as their lengths say
    /// (else OPEN Message Error / Unspecific), known parameter types only, a hold time of 0 or
    /// at least 3 s, a BGP identifier that is a unicast host address
    pub fn read(b: &[u8]) -> Result<Open, BgpErr> {
        if b.len() < OPEN_MIN - HDR_LEN {
            return Err(BgpErr::new(E_HEADER, HE_LENGTH, ((b.len() + HDR_LEN) as u16).to_be_bytes().to_vec()));
        }
        let version = b[0];
        if version != VERSION {
            return Err(BgpErr::new(E_OPEN, OE_VERSION, (VERSION as u16).to_be_bytes().to_vec()));
        }
        let asn = be16(b, 1);
        let hold = be16(b, 3);
        let id = Ipv4([b[5], b[6], b[7], b[8]]);
        let plen = b[9] as usize;
        if 10 + plen != b.len() {
            return Err(err(E_OPEN, OE_UNSPECIFIC));
        }
        let mut params = Vec::new();
        let mut i = 10;
        while i < b.len() {
            if i + 2 > b.len() {
                return Err(err(E_OPEN, OE_UNSPECIFIC));
            }
            let (kind, n) = (b[i], b[i + 1] as usize);
            let v = b.get(i + 2..i + 2 + n).ok_or_else(|| err(E_OPEN, OE_UNSPECIFIC))?;
            if kind != OPT_CAPS {
                return Err(err(E_OPEN, OE_OPT_PARAM));
            }
            let mut caps = Vec::new();
            let mut j = 0;
            while j < v.len() {
                if j + 2 > v.len() {
                    return Err(err(E_OPEN, OE_UNSPECIFIC));
                }
                let (code, m) = (v[j], v[j + 1] as usize);
                let x = v.get(j + 2..j + 2 + m).ok_or_else(|| err(E_OPEN, OE_UNSPECIFIC))?;
                caps.push(Cap::read(code, x));
                j += 2 + m;
            }
            params.push(OptParam::Caps(caps));
            i += 2 + n;
        }
        if hold == 1 || hold == 2 {
            return Err(err(E_OPEN, OE_HOLD));
        }
        if id.is_unspecified() || id.is_broadcast() || id.0[0] >= 224 {
            return Err(err(E_OPEN, OE_ID));
        }
        Ok(Open { version, asn, hold, id, params })
    }
}

// ---------------------------------------------------------------- path attributes

/// an AS_SET or an AS_SEQUENCE
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub set: bool,
    pub asns: Vec<u32>,
}

impl Segment {
    pub fn seq(asns: Vec<u32>) -> Segment {
        Segment { set: false, asns }
    }
}

/// the length of an AS path for the decision process: an AS_SET counts as one AS (9.1.2.2)
pub fn path_len(segs: &[Segment]) -> usize {
    segs.iter().map(|s| if s.set { 1 } else { s.asns.len() }).sum()
}

/// "65002 65003 {65010,65011}"; "" for an empty path
pub fn path_text(segs: &[Segment]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for s in segs {
        if s.set {
            let v: Vec<String> = s.asns.iter().map(|a| format!("{}", a)).collect();
            parts.push(format!("{{{}}}", v.join(",")));
        } else {
            for a in &s.asns {
                parts.push(format!("{}", a));
            }
        }
    }
    parts.join(" ")
}

/// "i", "e", "?"
pub fn origin_code(o: u8) -> &'static str {
    match o {
        ORIGIN_IGP => "i",
        ORIGIN_EGP => "e",
        _ => "?",
    }
}

/// "IGP", "EGP", "INCOMPLETE"
pub fn origin_name(o: u8) -> &'static str {
    match o {
        ORIGIN_IGP => "IGP",
        ORIGIN_EGP => "EGP",
        ORIGIN_INCOMPLETE => "INCOMPLETE",
        _ => "invalid",
    }
}

/// a community as people write it: "65001:100", or the well-known names of RFC 1997
pub fn community_text(c: u32) -> String {
    match c {
        0xFFFF_FF01 => String::from("no-export"),
        0xFFFF_FF02 => String::from("no-advertise"),
        0xFFFF_FF03 => String::from("local-AS"),
        _ => format!("{}:{}", c >> 16, c & 0xFFFF),
    }
}

/// the name of an attribute type ("AS_PATH")
pub fn attr_name(t: u8) -> &'static str {
    match t {
        A_ORIGIN => "ORIGIN",
        A_AS_PATH => "AS_PATH",
        A_NEXT_HOP => "NEXT_HOP",
        A_MED => "MULTI_EXIT_DISC",
        A_LOCAL_PREF => "LOCAL_PREF",
        A_ATOMIC_AGGREGATE => "ATOMIC_AGGREGATE",
        A_AGGREGATOR => "AGGREGATOR",
        A_COMMUNITIES => "COMMUNITIES",
        A_AS4_PATH => "AS4_PATH",
        A_AS4_AGGREGATOR => "AS4_AGGREGATOR",
        _ => "Unknown",
    }
}

/// the value of a path attribute
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttrVal {
    Origin(u8),
    AsPath(Vec<Segment>),
    NextHop(Ipv4),
    Med(u32),
    LocalPref(u32),
    AtomicAggregate,
    /// the AS and BGP identifier of the speaker that aggregated
    Aggregator { asn: u32, id: Ipv4 },
    Communities(Vec<u32>),
    /// the 4-octet path next to an AS_PATH of 2-octet numbers (RFC 6793)
    As4Path(Vec<Segment>),
    As4Aggregator { asn: u32, id: Ipv4 },
    /// a type this codec does not know (or a malformed AS4 attribute), byte for byte
    Other { kind: u8, data: Vec<u8> },
}

/// a path attribute with its flags (the extended length bit is set when the value needs it)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr {
    pub flags: u8,
    pub val: AttrVal,
}

/// the flags the RFCs give a known type (well-known: transitive; optional: transitive or not)
fn std_flags(kind: u8) -> Option<u8> {
    Some(match kind {
        A_ORIGIN | A_AS_PATH | A_NEXT_HOP | A_LOCAL_PREF | A_ATOMIC_AGGREGATE => F_TRANSITIVE,
        A_MED => F_OPTIONAL,
        A_AGGREGATOR | A_COMMUNITIES | A_AS4_PATH | A_AS4_AGGREGATOR => F_OPTIONAL | F_TRANSITIVE,
        _ => return None,
    })
}

fn write_segments(out: &mut Vec<u8>, segs: &[Segment], four: bool) {
    for s in segs {
        out.push(if s.set { SEG_SET } else { SEG_SEQUENCE });
        out.push(s.asns.len() as u8);
        for &a in &s.asns {
            if four {
                out.extend_from_slice(&a.to_be_bytes());
            } else {
                let w = if a > 0xFFFF { AS_TRANS } else { a as u16 };
                out.extend_from_slice(&w.to_be_bytes());
            }
        }
    }
}

/// the segments of an AS_PATH or AS4_PATH value; None when malformed (a type other than set or
/// sequence, an empty segment, numbers that do not fill the value exactly)
fn read_segments(v: &[u8], four: bool) -> Option<Vec<Segment>> {
    let w = if four { 4 } else { 2 };
    let mut out = Vec::new();
    let mut i = 0;
    while i < v.len() {
        if i + 2 > v.len() {
            return None;
        }
        let (t, n) = (v[i], v[i + 1] as usize);
        if (t != SEG_SET && t != SEG_SEQUENCE) || n == 0 || i + 2 + n * w > v.len() {
            return None;
        }
        let asns = (0..n).map(|k| if four { be32(v, i + 2 + k * 4) } else { be16(v, i + 2 + k * 2) as u32 }).collect();
        out.push(Segment { set: t == SEG_SET, asns });
        i += 2 + n * w;
    }
    Some(out)
}

/// a dissector's guess: the AS_PATH value reads as 4-octet numbers (a session between speakers
/// with the capability) and not as 2-octet ones, or as both and the 2-octet reading gives AS 0
/// (the high halves of 4-octet numbers)
pub fn as_path_4(v: &[u8]) -> bool {
    match (read_segments(v, true).is_some(), read_segments(v, false).is_some()) {
        (true, false) => true,
        (false, _) => false,
        (true, true) => read_segments(v, false).is_some_and(|s| s.iter().flat_map(|x| x.asns.iter()).any(|&a| a == 0)),
    }
}

impl Attr {
    /// a known attribute with the flags its type has
    pub fn new(val: AttrVal) -> Attr {
        let kind = Attr { flags: 0, val: val.clone() }.kind();
        let flags = std_flags(kind).unwrap_or(F_OPTIONAL | F_TRANSITIVE);
        Attr { flags, val }
    }
    pub fn kind(&self) -> u8 {
        match &self.val {
            AttrVal::Origin(_) => A_ORIGIN,
            AttrVal::AsPath(_) => A_AS_PATH,
            AttrVal::NextHop(_) => A_NEXT_HOP,
            AttrVal::Med(_) => A_MED,
            AttrVal::LocalPref(_) => A_LOCAL_PREF,
            AttrVal::AtomicAggregate => A_ATOMIC_AGGREGATE,
            AttrVal::Aggregator { .. } => A_AGGREGATOR,
            AttrVal::Communities(_) => A_COMMUNITIES,
            AttrVal::As4Path(_) => A_AS4_PATH,
            AttrVal::As4Aggregator { .. } => A_AS4_AGGREGATOR,
            AttrVal::Other { kind, .. } => *kind,
        }
    }
    /// the value bytes (`as4`: AS_PATH and AGGREGATOR with 4-octet numbers)
    pub fn value(&self, as4: bool) -> Vec<u8> {
        let mut v = Vec::new();
        match &self.val {
            AttrVal::Origin(o) => v.push(*o),
            AttrVal::AsPath(s) => write_segments(&mut v, s, as4),
            AttrVal::NextHop(a) => v.extend_from_slice(&a.0),
            AttrVal::Med(m) | AttrVal::LocalPref(m) => v.extend_from_slice(&m.to_be_bytes()),
            AttrVal::AtomicAggregate => {}
            AttrVal::Aggregator { asn, id } => {
                if as4 {
                    v.extend_from_slice(&asn.to_be_bytes());
                } else {
                    let w = if *asn > 0xFFFF { AS_TRANS } else { *asn as u16 };
                    v.extend_from_slice(&w.to_be_bytes());
                }
                v.extend_from_slice(&id.0);
            }
            AttrVal::Communities(cs) => {
                for c in cs {
                    v.extend_from_slice(&c.to_be_bytes());
                }
            }
            AttrVal::As4Path(s) => write_segments(&mut v, s, true),
            AttrVal::As4Aggregator { asn, id } => {
                v.extend_from_slice(&asn.to_be_bytes());
                v.extend_from_slice(&id.0);
            }
            AttrVal::Other { data, .. } => v.extend_from_slice(data),
        }
        v
    }
    /// appends flags, type, length (two bytes with the extended length bit) and value
    pub fn write(&self, out: &mut Vec<u8>, as4: bool) {
        let v = self.value(as4);
        let ext = self.flags & F_EXTENDED != 0 || v.len() > 255;
        out.push(if ext { self.flags | F_EXTENDED } else { self.flags });
        out.push(self.kind());
        if ext {
            out.extend_from_slice(&(v.len() as u16).to_be_bytes());
        } else {
            out.push(v.len() as u8);
        }
        out.extend_from_slice(&v);
    }
}

/// the path attributes of an UPDATE (6.3): each one whole and once, the flags of its type, the
/// length of its type, its value checked
fn read_attrs(b: &[u8], as4: bool) -> Result<Vec<Attr>, BgpErr> {
    let mut out: Vec<Attr> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if i + 2 > b.len() {
            return Err(err(E_UPDATE, UE_ATTR_LIST));
        }
        let (flags, kind) = (b[i], b[i + 1]);
        let ext = flags & F_EXTENDED != 0;
        let hl = if ext { 4 } else { 3 };
        if i + hl > b.len() {
            return Err(err(E_UPDATE, UE_ATTR_LIST));
        }
        let n = if ext { be16(b, i + 2) as usize } else { b[i + 2] as usize };
        if i + hl + n > b.len() {
            return Err(err(E_UPDATE, UE_ATTR_LIST));
        }
        let whole = b[i..i + hl + n].to_vec();
        let v = &b[i + hl..i + hl + n];
        i += hl + n;
        if out.iter().any(|a| a.kind() == kind) {
            return Err(err(E_UPDATE, UE_ATTR_LIST));
        }
        let Some(want) = std_flags(kind) else {
            if flags & F_OPTIONAL == 0 {
                return Err(BgpErr::new(E_UPDATE, UE_UNRECOGNIZED, whole));
            }
            out.push(Attr { flags, val: AttrVal::Other { kind, data: v.to_vec() } });
            continue;
        };
        // optional and transitive must be as the type says; partial only on optional
        // transitive ones (the low four bits are ignored)
        let f = flags & (F_OPTIONAL | F_TRANSITIVE | F_PARTIAL);
        let ok = if want == F_OPTIONAL | F_TRANSITIVE { f & !F_PARTIAL == want } else { f == want };
        if !ok {
            return Err(BgpErr::new(E_UPDATE, UE_FLAGS, whole));
        }
        let len_err = || BgpErr::new(E_UPDATE, UE_LENGTH, whole.clone());
        let val = match kind {
            A_ORIGIN => {
                if n != 1 {
                    return Err(len_err());
                }
                if v[0] > ORIGIN_INCOMPLETE {
                    return Err(BgpErr::new(E_UPDATE, UE_ORIGIN, whole));
                }
                AttrVal::Origin(v[0])
            }
            A_AS_PATH => AttrVal::AsPath(read_segments(v, as4).ok_or_else(|| err(E_UPDATE, UE_AS_PATH))?),
            A_NEXT_HOP => {
                if n != 4 {
                    return Err(len_err());
                }
                AttrVal::NextHop(Ipv4([v[0], v[1], v[2], v[3]]))
            }
            A_MED | A_LOCAL_PREF => {
                if n != 4 {
                    return Err(len_err());
                }
                if kind == A_MED { AttrVal::Med(be32(v, 0)) } else { AttrVal::LocalPref(be32(v, 0)) }
            }
            A_ATOMIC_AGGREGATE => {
                if n != 0 {
                    return Err(len_err());
                }
                AttrVal::AtomicAggregate
            }
            A_AGGREGATOR => {
                let w = if as4 { 8 } else { 6 };
                if n != w {
                    return Err(len_err());
                }
                let asn = if as4 { be32(v, 0) } else { be16(v, 0) as u32 };
                AttrVal::Aggregator { asn, id: Ipv4([v[w - 4], v[w - 3], v[w - 2], v[w - 1]]) }
            }
            A_COMMUNITIES => {
                if n % 4 != 0 {
                    return Err(BgpErr::new(E_UPDATE, UE_OPTIONAL, whole));
                }
                AttrVal::Communities((0..n / 4).map(|k| be32(v, k * 4)).collect())
            }
            A_AS4_PATH => match read_segments(v, true) {
                Some(s) => AttrVal::As4Path(s),
                None => AttrVal::Other { kind, data: v.to_vec() },
            },
            A_AS4_AGGREGATOR if n == 8 => AttrVal::As4Aggregator { asn: be32(v, 0), id: Ipv4([v[4], v[5], v[6], v[7]]) },
            _ => AttrVal::Other { kind, data: v.to_vec() },
        };
        out.push(Attr { flags, val });
    }
    Ok(out)
}

// ---------------------------------------------------------------- UPDATE

/// an UPDATE message (RFC 4271 4.3)
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Update {
    pub withdrawn: Vec<Prefix4>,
    pub attrs: Vec<Attr>,
    pub nlri: Vec<Prefix4>,
}

impl Update {
    /// the attribute of a type
    pub fn get(&self, kind: u8) -> Option<&AttrVal> {
        self.attrs.iter().find(|a| a.kind() == kind).map(|a| &a.val)
    }
    pub fn origin(&self) -> Option<u8> {
        match self.get(A_ORIGIN) {
            Some(AttrVal::Origin(o)) => Some(*o),
            _ => None,
        }
    }
    pub fn as_path(&self) -> Option<&[Segment]> {
        match self.get(A_AS_PATH) {
            Some(AttrVal::AsPath(s)) => Some(s),
            _ => None,
        }
    }
    pub fn as4_path(&self) -> Option<&[Segment]> {
        match self.get(A_AS4_PATH) {
            Some(AttrVal::As4Path(s)) => Some(s),
            _ => None,
        }
    }
    pub fn next_hop(&self) -> Option<Ipv4> {
        match self.get(A_NEXT_HOP) {
            Some(AttrVal::NextHop(a)) => Some(*a),
            _ => None,
        }
    }
    pub fn med(&self) -> Option<u32> {
        match self.get(A_MED) {
            Some(AttrVal::Med(m)) => Some(*m),
            _ => None,
        }
    }
    pub fn local_pref(&self) -> Option<u32> {
        match self.get(A_LOCAL_PREF) {
            Some(AttrVal::LocalPref(m)) => Some(*m),
            _ => None,
        }
    }
    pub fn communities(&self) -> Option<&[u32]> {
        match self.get(A_COMMUNITIES) {
            Some(AttrVal::Communities(c)) => Some(c),
            _ => None,
        }
    }

    fn write(&self, out: &mut Vec<u8>, as4: bool) {
        let mut w = Vec::new();
        for p in &self.withdrawn {
            write_prefix(&mut w, p);
        }
        out.extend_from_slice(&(w.len() as u16).to_be_bytes());
        out.extend_from_slice(&w);
        let mut a = Vec::new();
        for x in &self.attrs {
            x.write(&mut a, as4);
        }
        out.extend_from_slice(&(a.len() as u16).to_be_bytes());
        out.extend_from_slice(&a);
        for p in &self.nlri {
            write_prefix(out, p);
        }
    }

    /// the bytes of the message this update makes (to keep it within MAX_LEN)
    pub fn wire_len(&self, as4: bool) -> usize {
        let mut v = Vec::new();
        self.write(&mut v, as4);
        HDR_LEN + v.len()
    }

    /// the body of an UPDATE (6.3): the lengths fit the message (else Malformed Attribute
    /// List), the prefixes are valid (Invalid Network Field), the attributes as read_attrs says,
    /// and NLRI come with ORIGIN, AS_PATH and NEXT_HOP (Missing Well-known Attribute, its type
    /// as data)
    pub fn read(b: &[u8], as4: bool) -> Result<Update, BgpErr> {
        if b.len() < 4 {
            return Err(err(E_UPDATE, UE_ATTR_LIST));
        }
        let wl = be16(b, 0) as usize;
        if 2 + wl + 2 > b.len() {
            return Err(err(E_UPDATE, UE_ATTR_LIST));
        }
        let al = be16(b, 2 + wl) as usize;
        if 4 + wl + al > b.len() {
            return Err(err(E_UPDATE, UE_ATTR_LIST));
        }
        let withdrawn = read_prefixes(&b[2..2 + wl])?;
        let attrs = read_attrs(&b[4 + wl..4 + wl + al], as4)?;
        let nlri = read_prefixes(&b[4 + wl + al..])?;
        let u = Update { withdrawn, attrs, nlri };
        if !u.nlri.is_empty() {
            for k in [A_ORIGIN, A_AS_PATH, A_NEXT_HOP] {
                if u.get(k).is_none() {
                    return Err(BgpErr::new(E_UPDATE, UE_MISSING, alloc::vec![k]));
                }
            }
        }
        Ok(u)
    }
}

// ---------------------------------------------------------------- NOTIFICATION and the message

/// a NOTIFICATION message (RFC 4271 4.5)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub code: u8,
    pub sub: u8,
    pub data: Vec<u8>,
}

impl Notification {
    pub fn new(code: u8, sub: u8) -> Notification {
        Notification { code, sub, data: Vec::new() }
    }
    /// "Cease / Administrative Reset"
    pub fn text(&self) -> String {
        format!("{} / {}", code_name(self.code), error_name(self.code, self.sub))
    }
}

/// a BGP message
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Open(Open),
    Update(Update),
    Notification(Notification),
    Keepalive,
    /// a request to send the routes of a family again (RFC 2918)
    RouteRefresh { afi: u16, safi: u8 },
}

impl Message {
    pub fn kind(&self) -> u8 {
        match self {
            Message::Open(_) => T_OPEN,
            Message::Update(_) => T_UPDATE,
            Message::Notification(_) => T_NOTIFICATION,
            Message::Keepalive => T_KEEPALIVE,
            Message::RouteRefresh { .. } => T_ROUTE_REFRESH,
        }
    }

    /// one whole message (the first of `b`; frame() says how long it is) with the checks of 6.1
    /// to 6.3
    pub fn read(b: &[u8], as4: bool) -> Result<Message, BgpErr> {
        let len = frame(b)?.ok_or_else(|| BgpErr::new(E_HEADER, HE_LENGTH, b.get(16..18).unwrap_or(&[0, 0]).to_vec()))?;
        let body = &b[HDR_LEN..len];
        Ok(match b[18] {
            T_OPEN => Message::Open(Open::read(body)?),
            T_UPDATE => Message::Update(Update::read(body, as4)?),
            T_NOTIFICATION => Message::Notification(Notification { code: body[0], sub: body[1], data: body[2..].to_vec() }),
            T_KEEPALIVE => Message::Keepalive,
            _ => Message::RouteRefresh { afi: be16(body, 0), safi: body[3] },
        })
    }

    /// the message on the wire: marker, length, type, body
    pub fn to_bytes(&self, as4: bool) -> Vec<u8> {
        let mut out: Vec<u8> = Vec::with_capacity(64);
        out.extend_from_slice(&MARKER);
        out.extend_from_slice(&[0, 0, self.kind()]);
        match self {
            Message::Open(o) => o.write(&mut out),
            Message::Update(u) => u.write(&mut out, as4),
            Message::Notification(n) => {
                out.push(n.code);
                out.push(n.sub);
                out.extend_from_slice(&n.data);
            }
            Message::Keepalive => {}
            Message::RouteRefresh { afi, safi } => {
                out.extend_from_slice(&afi.to_be_bytes());
                out.push(0);
                out.push(*safi);
            }
        }
        let n = out.len() as u16;
        out[16..18].copy_from_slice(&n.to_be_bytes());
        out
    }
}
