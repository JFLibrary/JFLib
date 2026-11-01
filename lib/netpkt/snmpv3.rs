// snmpv3 -- SNMPv3 messages with the User-based Security Model (RFC 3412, RFC 3414), AES-128 for
// privacy (RFC 3826), HMAC-SHA-2 for authentication (RFC 7860) and engine IDs (RFC 3411 5); the PDUs
// are those of netpkt::snmp
//
//   F_AUTH, F_PRIV, F_REPORTABLE     the bits of msgFlags; MODEL_USM (3)
//   Level                            NoAuthNoPriv | AuthNoPriv | AuthPriv: parse, word(), flags(),
//                                    of_flags(flags) (None: privacy without authentication)
//   AuthProto                        Md5 | Sha1 | Sha256 | Sha384 | Sha512: parse ("MD5", "SHA",
//                                    "SHA-256" ...), name(), tag_len() (12, 12, 24, 32, 48)
//   PrivProto                        Aes128: parse ("AES", "AES128"), name()
//   password_to_key(auth, password)  RFC 3414 A.2: a megabyte of the repeated password hashed
//   localize(auth, ku, engine_id)    the key of one engine (RFC 3414 A.2.2), hash length
//   priv_key(kul)                    the first 16 bytes (RFC 3826 3.1.2.1)
//   tag(auth, key, msg)              the truncated HMAC
//   Usm { engine_id, boots, time, user, auth, privacy }   msgSecurityParameters
//   ScopedPdu { context_engine, context_name, pdu }
//   Scoped                           Plain(ScopedPdu) | Encrypted(bytes)
//   V3 { id, max_size, flags, model, usm, scoped }: parse(b), to_bytes(), level(), reportable()
//   sign(bytes, auth, key)           puts the tag into a message written with zeros in msgAuthParameters;
//   check(bytes, auth, key)          compares it (false also when it is no message or has
//                                    another length)
//   encrypt(key, boots, time, salt, plain) / decrypt(...) AES-128-CFB with the IV of RFC 3826 3.1.2.1
//   open(enc) -> ScopedPdu           the plain bytes as a scoped PDU
//   engine_id_mac(mac)               format 3 (MAC) under the enterprise number 32473 for
//                                    documentation (RFC 5612), parse_engine_id(hex) (5 to 32 bytes),
//                                    engine_id_text(bytes) (hex)
//   USM_STATS_* , UNKNOWN_CONTEXTS   the counters of report PDUs; report_name(oid)
//
// DES (the other privacy protocol of RFC 3414) has no place in crypto and is not here.

use alloc::string::String;
use alloc::vec::Vec;

use crypto::aes::Aes;
use crypto::{Hash, Hmac, Md5, Sha1, Sha256, Sha384, Sha512};

use crate::ber::{self, Reader, Value, Writer, T_SEQUENCE};
use crate::snmp::{read_varbinds, write_varbinds, Bad, Kind, Pdu, VarBind, Version};
use crate::Mac;

pub const F_AUTH: u8 = 1;
pub const F_PRIV: u8 = 2;
pub const F_REPORTABLE: u8 = 4;
/// the User-based Security Model
pub const MODEL_USM: i64 = 3;
/// the message size we say we take
pub const MAX_SIZE: i64 = 65507;

/// a security level
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    NoAuthNoPriv,
    AuthNoPriv,
    AuthPriv,
}

impl Level {
    /// "noAuthNoPriv", "authNoPriv", "authPriv" (also shorter: as net-snmp's tools take them)
    pub fn parse(s: &str) -> Option<Level> {
        match s.to_ascii_lowercase().as_str() {
            "noauthnopriv" | "noauth" | "nanp" => Some(Level::NoAuthNoPriv),
            "authnopriv" | "auth" | "anp" => Some(Level::AuthNoPriv),
            "authpriv" | "priv" | "ap" => Some(Level::AuthPriv),
            _ => None,
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            Level::NoAuthNoPriv => "noAuthNoPriv",
            Level::AuthNoPriv => "authNoPriv",
            Level::AuthPriv => "authPriv",
        }
    }
    /// its bits of msgFlags
    pub fn flags(self) -> u8 {
        match self {
            Level::NoAuthNoPriv => 0,
            Level::AuthNoPriv => F_AUTH,
            Level::AuthPriv => F_AUTH | F_PRIV,
        }
    }
    /// the level of msgFlags (None: privacy without authentication, which RFC 3412 7.2 refuses)
    pub fn of_flags(f: u8) -> Option<Level> {
        match f & 3 {
            0 => Some(Level::NoAuthNoPriv),
            1 => Some(Level::AuthNoPriv),
            3 => Some(Level::AuthPriv),
            _ => None,
        }
    }
}

/// an authentication protocol
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthProto {
    Md5,
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl AuthProto {
    pub fn parse(s: &str) -> Option<AuthProto> {
        match s.to_ascii_uppercase().as_str() {
            "MD5" => Some(AuthProto::Md5),
            "SHA" | "SHA1" | "SHA-1" => Some(AuthProto::Sha1),
            "SHA256" | "SHA-256" => Some(AuthProto::Sha256),
            "SHA384" | "SHA-384" => Some(AuthProto::Sha384),
            "SHA512" | "SHA-512" => Some(AuthProto::Sha512),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            AuthProto::Md5 => "MD5",
            AuthProto::Sha1 => "SHA",
            AuthProto::Sha256 => "SHA-256",
            AuthProto::Sha384 => "SHA-384",
            AuthProto::Sha512 => "SHA-512",
        }
    }
    /// the bytes of msgAuthenticationParameters
    pub fn tag_len(self) -> usize {
        match self {
            AuthProto::Md5 | AuthProto::Sha1 => 12,
            AuthProto::Sha256 => 24,
            AuthProto::Sha384 => 32,
            AuthProto::Sha512 => 48,
        }
    }
}

/// a privacy protocol
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrivProto {
    Aes128,
}

impl PrivProto {
    pub fn parse(s: &str) -> Option<PrivProto> {
        match s.to_ascii_uppercase().as_str() {
            "AES" | "AES128" | "AES-128" => Some(PrivProto::Aes128),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            PrivProto::Aes128 => "AES",
        }
    }
}

fn digest<H: Hash>(parts: &[&[u8]]) -> Vec<u8> {
    let mut h = H::new();
    for p in parts {
        h.update(p);
    }
    let mut out = alloc::vec![0u8; H::OUT];
    h.finish_into(&mut out);
    out
}

/// the hash of an authentication protocol
fn hash(auth: AuthProto, parts: &[&[u8]]) -> Vec<u8> {
    match auth {
        AuthProto::Md5 => digest::<Md5>(parts),
        AuthProto::Sha1 => digest::<Sha1>(parts),
        AuthProto::Sha256 => digest::<Sha256>(parts),
        AuthProto::Sha384 => digest::<Sha384>(parts),
        AuthProto::Sha512 => digest::<Sha512>(parts),
    }
}

fn expand<H: Hash>(password: &[u8]) -> Vec<u8> {
    let mut h = H::new();
    let mut buf = [0u8; 64];
    let mut at = 0usize;
    for _ in 0..(1_048_576 / 64) {
        for b in buf.iter_mut() {
            *b = password[at % password.len()];
            at += 1;
        }
        h.update(&buf);
    }
    let mut out = alloc::vec![0u8; H::OUT];
    h.finish_into(&mut out);
    out
}

/// RFC 3414 A.2: a password as a key for any engine (an empty password gives no key)
pub fn password_to_key(auth: AuthProto, password: &[u8]) -> Vec<u8> {
    if password.is_empty() {
        return Vec::new();
    }
    match auth {
        AuthProto::Md5 => expand::<Md5>(password),
        AuthProto::Sha1 => expand::<Sha1>(password),
        AuthProto::Sha256 => expand::<Sha256>(password),
        AuthProto::Sha384 => expand::<Sha384>(password),
        AuthProto::Sha512 => expand::<Sha512>(password),
    }
}

/// RFC 3414 A.2.2: the key of one engine
pub fn localize(auth: AuthProto, ku: &[u8], engine_id: &[u8]) -> Vec<u8> {
    hash(auth, &[ku, engine_id, ku])
}

/// the privacy key of RFC 3826 3.1.2.1
pub fn priv_key(kul: &[u8]) -> Vec<u8> {
    Vec::from(&kul[..kul.len().min(16)])
}

fn hmac<H: Hash>(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut m = Hmac::<H>::new(key);
    m.update(msg);
    let mut out = alloc::vec![0u8; H::OUT];
    m.finish_into(&mut out);
    out
}

/// the truncated HMAC of a whole message
pub fn tag(auth: AuthProto, key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut t = match auth {
        AuthProto::Md5 => hmac::<Md5>(key, msg),
        AuthProto::Sha1 => hmac::<Sha1>(key, msg),
        AuthProto::Sha256 => hmac::<Sha256>(key, msg),
        AuthProto::Sha384 => hmac::<Sha384>(key, msg),
        AuthProto::Sha512 => hmac::<Sha512>(key, msg),
    };
    t.truncate(auth.tag_len());
    t
}

/// msgSecurityParameters of the USM
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Usm {
    pub engine_id: Vec<u8>,
    pub boots: i32,
    pub time: i32,
    pub user: Vec<u8>,
    /// msgAuthenticationParameters
    pub auth: Vec<u8>,
    /// msgPrivacyParameters: the salt
    pub privacy: Vec<u8>,
}

/// a PDU with its context
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopedPdu {
    pub context_engine: Vec<u8>,
    pub context_name: Vec<u8>,
    pub pdu: Pdu,
}

/// the scoped PDU of a message
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scoped {
    Plain(ScopedPdu),
    Encrypted(Vec<u8>),
}

/// an SNMPv3 message
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct V3 {
    pub id: i32,
    pub max_size: i32,
    pub flags: u8,
    pub model: i32,
    pub usm: Usm,
    pub scoped: Scoped,
}

fn read_scoped(m: &mut Reader) -> Result<ScopedPdu, Bad> {
    let mut sp = m.seq(T_SEQUENCE)?;
    let context_engine = Vec::from(sp.octets()?);
    let context_name = Vec::from(sp.octets()?);
    let t = sp.peek()?;
    let kind = Kind::of_tag(t.tag).filter(|k| k.in_v2c()).ok_or(Bad::Pdu(t.tag))?;
    let mut p = sp.seq(t.tag)?;
    let request_id = p.int32()?;
    let error_status = p.int32()?;
    let error_index = p.int32()?;
    let varbinds = read_varbinds(&mut p, Version::V2c)?;
    p.done()?;
    sp.done()?;
    Ok(ScopedPdu { context_engine, context_name, pdu: Pdu { kind, request_id, error_status, error_index, varbinds } })
}

fn write_scoped(w: &mut Writer, s: &ScopedPdu) {
    w.begin(T_SEQUENCE);
    w.octets(&s.context_engine);
    w.octets(&s.context_name);
    w.begin(s.pdu.kind.tag());
    w.int(s.pdu.request_id as i64);
    w.int(s.pdu.error_status as i64);
    w.int(s.pdu.error_index as i64);
    write_varbinds(w, &s.pdu.varbinds);
    w.end();
    w.end();
}

impl ScopedPdu {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        write_scoped(&mut w, self);
        w.into_vec()
    }
    /// plain bytes as a scoped PDU
    pub fn parse(b: &[u8]) -> Result<ScopedPdu, Bad> {
        let mut r = Reader::new(b);
        let s = read_scoped(&mut r)?;
        r.done()?;
        Ok(s)
    }
}

impl Usm {
    /// the contents of msgSecurityParameters
    fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.begin(T_SEQUENCE);
        w.octets(&self.engine_id);
        w.int(self.boots as i64);
        w.int(self.time as i64);
        w.octets(&self.user);
        w.octets(&self.auth);
        w.octets(&self.privacy);
        w.end();
        w.into_vec()
    }
    fn parse(b: &[u8]) -> Result<Usm, Bad> {
        let mut top = Reader::new(b);
        let mut r = top.seq(T_SEQUENCE)?;
        top.done()?;
        let engine_id = Vec::from(r.octets()?);
        let boots = r.int32()?;
        let time = r.int32()?;
        let user = Vec::from(r.octets()?);
        let auth = Vec::from(r.octets()?);
        let privacy = Vec::from(r.octets()?);
        r.done()?;
        if boots < 0 || time < 0 {
            return Err(Bad::Ber(ber::Bad::Size));
        }
        Ok(Usm { engine_id, boots, time, user, auth, privacy })
    }
}

impl V3 {
    /// bytes as a message (Bad::Version: another version)
    pub fn parse(b: &[u8]) -> Result<V3, Bad> {
        let mut top = Reader::new(b);
        let mut m = top.seq(T_SEQUENCE)?;
        top.done()?;
        let vn = m.int()?;
        if vn != 3 {
            return Err(Bad::Version(vn));
        }
        let mut g = m.seq(T_SEQUENCE)?;
        let id = g.int32()?;
        let max_size = g.int32()?;
        let f = g.octets()?;
        let model = g.int32()?;
        g.done()?;
        if f.len() != 1 || id < 0 || max_size < 484 {
            return Err(Bad::Ber(ber::Bad::Size));
        }
        let usm = Usm::parse(m.octets()?)?;
        let scoped = if m.peek()?.tag == T_SEQUENCE { Scoped::Plain(read_scoped(&mut m)?) } else { Scoped::Encrypted(Vec::from(m.octets()?)) };
        m.done()?;
        Ok(V3 { id, max_size, flags: f[0], model, usm, scoped })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.begin(T_SEQUENCE);
        w.int(3);
        w.begin(T_SEQUENCE);
        w.int(self.id as i64);
        w.int(self.max_size as i64);
        w.octets(&[self.flags]);
        w.int(self.model as i64);
        w.end();
        w.octets(&self.usm.to_bytes());
        match &self.scoped {
            Scoped::Plain(s) => write_scoped(&mut w, s),
            Scoped::Encrypted(e) => w.octets(e),
        }
        w.end();
        w.into_vec()
    }
    /// the security level of its flags
    pub fn level(&self) -> Option<Level> {
        Level::of_flags(self.flags)
    }
    pub fn reportable(&self) -> bool {
        self.flags & F_REPORTABLE != 0
    }
    /// the plain scoped PDU, if it is plain
    pub fn plain(&self) -> Option<&ScopedPdu> {
        match &self.scoped {
            Scoped::Plain(s) => Some(s),
            Scoped::Encrypted(_) => None,
        }
    }
}

/// where msgAuthenticationParameters' contents are in a message's bytes
fn auth_range(b: &[u8]) -> Option<(usize, usize)> {
    let mut top = Reader::new(b);
    let mut m = top.seq(T_SEQUENCE).ok()?;
    m.int().ok()?;
    m.seq(T_SEQUENCE).ok()?;
    let t = m.expect(ber::T_OCTETS).ok()?;
    let mut s = m.inside(&t).seq(T_SEQUENCE).ok()?;
    s.octets().ok()?;
    s.int().ok()?;
    s.int().ok()?;
    s.octets().ok()?;
    let a = s.expect(ber::T_OCTETS).ok()?;
    Some((a.off + a.hdr, a.len))
}

/// puts the tag into a message written with zeros in msgAuthenticationParameters
pub fn sign(bytes: &mut [u8], auth: AuthProto, key: &[u8]) -> bool {
    let Some((at, n)) = auth_range(bytes) else {
        return false;
    };
    if n != auth.tag_len() {
        return false;
    }
    let t = tag(auth, key, bytes);
    bytes[at..at + n].copy_from_slice(&t);
    true
}

/// the tag of a message is right
pub fn check(bytes: &[u8], auth: AuthProto, key: &[u8]) -> bool {
    let Some((at, n)) = auth_range(bytes) else {
        return false;
    };
    if n != auth.tag_len() {
        return false;
    }
    let mut copy = Vec::from(bytes);
    copy[at..at + n].fill(0);
    crypto::ct_eq(&tag(auth, key, &copy), &bytes[at..at + n])
}

fn cfb(key: &[u8], boots: i32, time: i32, salt: &[u8], data: &[u8], decrypt: bool) -> Option<Vec<u8>> {
    if key.len() != 16 || salt.len() != 8 {
        return None;
    }
    let aes = Aes::new(key)?;
    let mut iv = [0u8; 16];
    iv[..4].copy_from_slice(&boots.to_be_bytes());
    iv[4..8].copy_from_slice(&time.to_be_bytes());
    iv[8..].copy_from_slice(salt);
    let mut out = Vec::with_capacity(data.len());
    for part in data.chunks(16) {
        let mut ks = iv;
        aes.encrypt(&mut ks);
        let c: Vec<u8> = part.iter().zip(ks).map(|(a, b)| a ^ b).collect();
        let cipher: &[u8] = if decrypt { part } else { &c };
        iv[..cipher.len()].copy_from_slice(cipher);
        out.extend_from_slice(&c);
    }
    Some(out)
}

/// AES-128-CFB of RFC 3826 (None: a key or salt of another length)
pub fn encrypt(key: &[u8], boots: i32, time: i32, salt: &[u8], plain: &[u8]) -> Option<Vec<u8>> {
    cfb(key, boots, time, salt, plain, false)
}

/// the other way
pub fn decrypt(key: &[u8], boots: i32, time: i32, salt: &[u8], cipher: &[u8]) -> Option<Vec<u8>> {
    cfb(key, boots, time, salt, cipher, true)
}

// ---------------------------------------------------------------- engine IDs

/// format 3 (a MAC address) under the enterprise number for documentation (RFC 5612), so no
/// real vendor is named
pub fn engine_id_mac(mac: Mac) -> Vec<u8> {
    let mut v = alloc::vec![0x80, 0x00, 0x7E, 0xD9, 0x03];
    v.extend_from_slice(&mac.0);
    v
}

/// hexadecimal text, 5 to 32 bytes
pub fn parse_engine_id(s: &str) -> Option<Vec<u8>> {
    let d: String = s.chars().filter(|c| !matches!(c, ' ' | ':' | '-')).collect();
    let d = d.strip_prefix("0x").unwrap_or(&d);
    let b = crypto::unhex(d)?;
    (5..=32).contains(&b.len()).then_some(b)
}

pub fn engine_id_text(b: &[u8]) -> String {
    crypto::hex(b)
}

// ---------------------------------------------------------------- report PDUs

pub const USM_STATS_UNSUPPORTED_SEC_LEVELS: [u32; 11] = [1, 3, 6, 1, 6, 3, 15, 1, 1, 1, 0];
pub const USM_STATS_NOT_IN_TIME_WINDOWS: [u32; 11] = [1, 3, 6, 1, 6, 3, 15, 1, 1, 2, 0];
pub const USM_STATS_UNKNOWN_USER_NAMES: [u32; 11] = [1, 3, 6, 1, 6, 3, 15, 1, 1, 3, 0];
pub const USM_STATS_UNKNOWN_ENGINE_IDS: [u32; 11] = [1, 3, 6, 1, 6, 3, 15, 1, 1, 4, 0];
pub const USM_STATS_WRONG_DIGESTS: [u32; 11] = [1, 3, 6, 1, 6, 3, 15, 1, 1, 5, 0];
pub const USM_STATS_DECRYPTION_ERRORS: [u32; 11] = [1, 3, 6, 1, 6, 3, 15, 1, 1, 6, 0];
/// snmpUnknownContexts of RFC 3413 (a context name the engine does not have)
pub const UNKNOWN_CONTEXTS: [u32; 10] = [1, 3, 6, 1, 6, 3, 12, 1, 5, 0];

/// the name of a report's counter
pub fn report_name(oid: &[u32]) -> &'static str {
    if oid == USM_STATS_UNSUPPORTED_SEC_LEVELS {
        "usmStatsUnsupportedSecLevels"
    } else if oid == USM_STATS_NOT_IN_TIME_WINDOWS {
        "usmStatsNotInTimeWindows"
    } else if oid == USM_STATS_UNKNOWN_USER_NAMES {
        "usmStatsUnknownUserNames"
    } else if oid == USM_STATS_UNKNOWN_ENGINE_IDS {
        "usmStatsUnknownEngineIDs"
    } else if oid == USM_STATS_WRONG_DIGESTS {
        "usmStatsWrongDigests"
    } else if oid == USM_STATS_DECRYPTION_ERRORS {
        "usmStatsDecryptionErrors"
    } else if oid == UNKNOWN_CONTEXTS {
        "snmpUnknownContexts"
    } else {
        "unknown counter"
    }
}

/// a Report PDU with one counter
pub fn report_pdu(request_id: i32, counter: &[u32], value: u32) -> Pdu {
    Pdu { kind: Kind::Report, request_id, error_status: 0, error_index: 0, varbinds: alloc::vec![VarBind::new(ber::Oid::new(counter), Value::Counter32(value))] }
}
