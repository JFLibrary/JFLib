// eap -- EAP packets (RFC 3748): Request, Response, Success and Failure with the types Identity,
// Notification and Nak, the data of EAP-MD5 (RFC 3748 5.4 with the value of RFC 1994 4.1) and
// the exchanges of EAP-PWD (RFC 5931 3.2).  No cryptography here: EAP-MD5's digest is lent by the
// caller (crypto::md5_parts); the sums of EAP-PWD are crypto::eappwd, and the state machines of
// a peer and a server are the crate eapm.  EAP travels in EAPOL frames (eapol.rs) on a port and
// in the EAP-Message attributes of RADIUS (radius.rs) behind the authenticator
//
//   REQUEST, RESPONSE, SUCCESS, FAILURE              the codes; code_name
//   T_IDENTITY, T_NOTIFICATION, T_NAK, T_MD5, T_PWD  the types; type_name (these and the types a
//                                                    dissector meets: TLS, TTLS, PEAP, MS-CHAPv2)
//   Packet { code, id, typ, data }                   parse(b) / to_bytes; the constructors
//                                                    request / response / success / failure /
//                                                    identity_request / identity_response / nak
//   Packet::identity() / nak_types()                 what such a packet says
//   Md5Data { value, name }                          the Type-Data of EAP-MD5: parse / to_bytes
//   md5_value(id, secret, challenge, md5)            MD5(Identifier || secret || Challenge)
//   Pwd                                              the Type-Data of EAP-PWD: Id, Commit, Confirm;
//                                                    parse / to_bytes; exch_name
//   PwdId { group, rand_func, prf, token, prep, identity }
//   info(b)                                          one line about a packet for the packet tab
//
// A packet is its 4-byte header and, for a Request or a Response, a type byte; bytes after its
// Length field are padding (Ethernet) and go.  EAP-PWD's L and M bits (fragments) are read and
// refused: all three exchanges fit a frame, and a peer that fragments them is no peer of ours.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::{be16, put16};

pub const REQUEST: u8 = 1;
pub const RESPONSE: u8 = 2;
pub const SUCCESS: u8 = 3;
pub const FAILURE: u8 = 4;

pub const T_IDENTITY: u8 = 1;
pub const T_NOTIFICATION: u8 = 2;
pub const T_NAK: u8 = 3;
pub const T_MD5: u8 = 4;
pub const T_TLS: u8 = 13;
pub const T_TTLS: u8 = 21;
pub const T_PEAP: u8 = 25;
pub const T_MSCHAPV2: u8 = 26;
pub const T_PWD: u8 = 52;

/// what Nak says when the peer takes no other method (RFC 3748 5.3.1)
pub const NAK_NONE: u8 = 0;

/// the exchanges of EAP-PWD
pub const PWD_ID: u8 = 1;
pub const PWD_COMMIT: u8 = 2;
pub const PWD_CONFIRM: u8 = 3;

/// EAP-PWD with group 19 (P-256) and SHA-256: an element and a scalar
pub const PWD_ELEMENT: usize = 64;
pub const PWD_SCALAR: usize = 32;

/// MD5 of the concatenation of the parts (what the caller lends: crypto::md5_parts)
pub type Md5Fn = fn(&[&[u8]]) -> [u8; 16];

/// the name of a code
pub fn code_name(c: u8) -> &'static str {
    match c {
        REQUEST => "Request",
        RESPONSE => "Response",
        SUCCESS => "Success",
        FAILURE => "Failure",
        _ => "Unknown",
    }
}

/// the name of a type
pub fn type_name(t: u8) -> String {
    String::from(match t {
        T_IDENTITY => "Identity",
        T_NOTIFICATION => "Notification",
        T_NAK => "Nak",
        T_MD5 => "MD5-Challenge",
        T_TLS => "TLS",
        T_TTLS => "TTLS",
        T_PEAP => "PEAP",
        T_MSCHAPV2 => "MS-CHAPv2",
        T_PWD => "PWD",
        _ => return format!("Type {}", t),
    })
}

/// an EAP packet
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub code: u8,
    pub id: u8,
    /// the type of a Request or a Response (0: Success and Failure have none)
    pub typ: u8,
    /// the Type-Data
    pub data: Vec<u8>,
}

impl Packet {
    /// the packet at the start of `b`; None when its Length does not fit (shorter than its header
    /// or longer than `b`, a Request or a Response without a type)
    pub fn parse(b: &[u8]) -> Option<Packet> {
        if b.len() < 4 {
            return None;
        }
        let len = be16(b, 2) as usize;
        if len < 4 || len > b.len() {
            return None;
        }
        let (code, id) = (b[0], b[1]);
        match code {
            REQUEST | RESPONSE if len >= 5 => Some(Packet { code, id, typ: b[4], data: b[5..len].to_vec() }),
            SUCCESS | FAILURE => Some(Packet { code, id, typ: 0, data: Vec::new() }),
            _ => None,
        }
    }
    /// the bytes
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = alloc::vec![self.code, self.id, 0, 0];
        if matches!(self.code, REQUEST | RESPONSE) {
            v.push(self.typ);
            v.extend_from_slice(&self.data);
        }
        let n = v.len() as u16;
        put16(&mut v, 2, n);
        v
    }
    pub fn request(id: u8, typ: u8, data: &[u8]) -> Packet {
        Packet { code: REQUEST, id, typ, data: data.to_vec() }
    }
    pub fn response(id: u8, typ: u8, data: &[u8]) -> Packet {
        Packet { code: RESPONSE, id, typ, data: data.to_vec() }
    }
    pub fn success(id: u8) -> Packet {
        Packet { code: SUCCESS, id, typ: 0, data: Vec::new() }
    }
    pub fn failure(id: u8) -> Packet {
        Packet { code: FAILURE, id, typ: 0, data: Vec::new() }
    }
    /// Request/Identity, an optional text for a person in the data
    pub fn identity_request(id: u8, prompt: &str) -> Packet {
        Packet::request(id, T_IDENTITY, prompt.as_bytes())
    }
    /// Response/Identity
    pub fn identity_response(id: u8, name: &str) -> Packet {
        Packet::response(id, T_IDENTITY, name.as_bytes())
    }
    /// Response/Nak: the methods the peer takes instead (none: NAK_NONE)
    pub fn nak(id: u8, types: &[u8]) -> Packet {
        if types.is_empty() { Packet::response(id, T_NAK, &[NAK_NONE]) } else { Packet::response(id, T_NAK, types) }
    }
    /// a Request or a Response of a type
    pub fn is(&self, code: u8, typ: u8) -> bool {
        self.code == code && self.typ == typ
    }
    /// the text of an Identity (bytes that are no UTF-8 are replaced)
    pub fn identity(&self) -> String {
        String::from_utf8_lossy(&self.data).into_owned()
    }
    /// the methods a Nak wants (NAK_NONE left out)
    pub fn nak_types(&self) -> Vec<u8> {
        self.data.iter().copied().filter(|&t| t != NAK_NONE).collect()
    }
}

/// the Type-Data of EAP-MD5: Value-Size, the value (a challenge or a digest), a name
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Md5Data {
    pub value: Vec<u8>,
    pub name: Vec<u8>,
}

impl Md5Data {
    pub fn parse(d: &[u8]) -> Option<Md5Data> {
        let n = *d.first()? as usize;
        if d.len() < 1 + n {
            return None;
        }
        Some(Md5Data { value: d[1..1 + n].to_vec(), name: d[1 + n..].to_vec() })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = alloc::vec![self.value.len() as u8];
        v.extend_from_slice(&self.value);
        v.extend_from_slice(&self.name);
        v
    }
}

/// the value of a CHAP-style answer (RFC 1994 4.1): MD5(Identifier || secret || Challenge)
pub fn md5_value(id: u8, secret: &[u8], challenge: &[u8], md5: Md5Fn) -> [u8; 16] {
    md5(&[&[id], secret, challenge])
}

/// EAP-pwd-ID: what the server offers and the peer takes (RFC 5931 3.2.1)
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PwdId {
    /// the group's number in IANA's registry (19: P-256)
    pub group: u16,
    /// the random function (1: HMAC-SHA-256)
    pub rand_func: u8,
    /// the PRF (1: HMAC-SHA-256)
    pub prf: u8,
    pub token: [u8; 4],
    /// how the password is prepared (0: none)
    pub prep: u8,
    pub identity: Vec<u8>,
}

/// the Type-Data of EAP-PWD
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pwd {
    Id(PwdId),
    Commit { element: [u8; PWD_ELEMENT], scalar: [u8; PWD_SCALAR] },
    Confirm([u8; 32]),
}

impl Pwd {
    /// the data of a packet; None for other sizes (only group 19 is read), a fragment or an
    /// unknown exchange
    pub fn parse(d: &[u8]) -> Option<Pwd> {
        let h = *d.first()?;
        if h & 0xC0 != 0 {
            return None;
        }
        let b = &d[1..];
        match h & 0x3F {
            PWD_ID if b.len() >= 9 => {
                let mut token = [0u8; 4];
                token.copy_from_slice(&b[4..8]);
                Some(Pwd::Id(PwdId { group: be16(b, 0), rand_func: b[2], prf: b[3], token, prep: b[8], identity: b[9..].to_vec() }))
            }
            PWD_COMMIT if b.len() == PWD_ELEMENT + PWD_SCALAR => {
                let mut element = [0u8; PWD_ELEMENT];
                element.copy_from_slice(&b[..PWD_ELEMENT]);
                let mut scalar = [0u8; PWD_SCALAR];
                scalar.copy_from_slice(&b[PWD_ELEMENT..]);
                Some(Pwd::Commit { element, scalar })
            }
            PWD_CONFIRM if b.len() == 32 => {
                let mut c = [0u8; 32];
                c.copy_from_slice(b);
                Some(Pwd::Confirm(c))
            }
            _ => None,
        }
    }
    /// the data bytes (L and M clear)
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Pwd::Id(i) => {
                let mut v = alloc::vec![PWD_ID, 0, 0, i.rand_func, i.prf];
                put16(&mut v, 1, i.group);
                v.extend_from_slice(&i.token);
                v.push(i.prep);
                v.extend_from_slice(&i.identity);
                v
            }
            Pwd::Commit { element, scalar } => {
                let mut v = alloc::vec![PWD_COMMIT];
                v.extend_from_slice(element);
                v.extend_from_slice(scalar);
                v
            }
            Pwd::Confirm(c) => {
                let mut v = alloc::vec![PWD_CONFIRM];
                v.extend_from_slice(c);
                v
            }
        }
    }
}

/// the name of an EAP-PWD exchange
pub fn exch_name(x: u8) -> &'static str {
    match x {
        PWD_ID => "PWD-ID",
        PWD_COMMIT => "PWD-Commit",
        PWD_CONFIRM => "PWD-Confirm",
        _ => "PWD",
    }
}

/// one line about a packet: "Request, Identity", "Response, Identity (anna)", "Success",
/// "Request, PWD-Commit", "Response, Nak (PWD)"
pub fn info(b: &[u8]) -> String {
    let Some(p) = Packet::parse(b) else {
        return String::from("EAP (malformed)");
    };
    match p.code {
        SUCCESS | FAILURE => String::from(code_name(p.code)),
        c => {
            let what = match p.typ {
                T_IDENTITY if c == RESPONSE => format!("Identity ({})", p.identity()),
                T_NAK => {
                    let names: Vec<String> = p.nak_types().into_iter().map(type_name).collect();
                    format!("Nak ({})", if names.is_empty() { String::from("none") } else { names.join(", ") })
                }
                T_PWD => match p.data.first() {
                    Some(h) => String::from(exch_name(h & 0x3F)),
                    None => String::from("PWD"),
                },
                t => type_name(t),
            };
            format!("{}, {}", code_name(c), what)
        }
    }
}
