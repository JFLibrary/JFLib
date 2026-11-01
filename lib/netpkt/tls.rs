// tls -- TLS 1.3 without I/O (RFC 8446): a core for both sides of a connection: feed it the bytes
// that came, take the bytes to send, the plain text and the events; no sockets, no clock.  Random
// bytes come from a seed the caller passes in, so a connection is the same byte for byte for the same
// seed.  The records and the handshake are the real thing, a peer of another make understands them
// (apps/coretest group tlscore talks to netio::tls)
//
//   PORT                     443
//   CT_*, HS_*, AL_*, EXT_*  content types, handshake types, alerts, extensions; ct_name(t),
//                            hs_name(t), alert_name(a), ext_name(e), suite_name(s), group_name(g),
//                            sig_name(s)
//   SUITE, VERSION           the one suite, the version's name
//   Identity { chain, key }  a certificate chain (DER, leaf first) and the leaf's Ed25519 key
//   ClientCfg { name, anchors, now, insecure, accept }   the name asked for (also the SNI, unless an
//                            address), what the client trusts, the time for the check; insecure takes
//                            any certificate, accept these fingerprints (SHA-256 of the leaf) whatever
//                            their faults
//   Core::client(cfg, seed) / Core::server(identity, seed)
//     push(bytes)            what the connection brought (any split)
//     take_out()             the bytes to send
//     take_events()          what happened (Event)
//     read() -> bytes        the plain text that came and was not read yet
//     send(bytes)            plain text, in records of at most MAX_PLAIN bytes (after Established)
//     close()                close_notify; dead(): nothing more will come or go
//     established(), peer_closed(), name() (the SNI a client sent), peer_certs(), faults()
//     (x509 F_* bits of the server's chain), excused() (it has faults and went on all the same),
//     failure(), footprint()
//   Event                    Hello { name }, Certificate { faults }, Established, PeerClosed, Failed(Failure)
//   Failure { alert, ours, why }   the alert that ended it, whether we sent it, a word for the log
//   schedule(ecdhe) -> (handshake, master), derive(secret, label, hash), expand_label, finished,
//   traffic_keys             the key schedule (7.1, 7.3, 4.4.4)
//   Part, dissect(segment)   what a dissector sees in a TCP segment: records with their type,
//                            version and length, or bytes that do not start a record
//
// What it speaks: TLS_CHACHA20_POLY1305_SHA256, X25519 and Ed25519 signatures and certificates
// (x509.rs); ClientHello, ServerHello, EncryptedExtensions, Certificate, CertificateVerify, Finished,
// KeyUpdate received, alerts.  Not here: HelloRetryRequest (a client without an X25519 share gets
// handshake_failure), client certificates, resumption and 0-RTT, the other suites (netio::tls has a
// wider client), the middlebox-compatibility ChangeCipherSpec (one received is ignored).  A client
// checks the chain itself; a fault ends the handshake with the alert for it unless the configuration
// excuses it.  The CertificateVerify signature and both Finished are checked always.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crypto::aead;
use crypto::ed25519::{self, Keypair};
use crypto::hmac::{hkdf_expand, hkdf_extract};
use crypto::{x25519, Sha256};

use crate::x509::{check_chain, Cert, F_EXPIRED, F_NAME, F_NOT_YET, F_SELF_SIGNED, F_UNKNOWN_ISSUER};
use crate::IpAddr;

/// the well-known port of HTTP over TLS
pub const PORT: u16 = 443;
/// the one cipher suite
pub const SUITE: u16 = 0x1303;
/// the version as a dialog names it
pub const VERSION: &str = "TLS 1.3";

pub const CT_CCS: u8 = 20;
pub const CT_ALERT: u8 = 21;
pub const CT_HANDSHAKE: u8 = 22;
pub const CT_DATA: u8 = 23;

pub const HS_CLIENT_HELLO: u8 = 1;
pub const HS_SERVER_HELLO: u8 = 2;
pub const HS_NEW_SESSION_TICKET: u8 = 4;
pub const HS_ENCRYPTED_EXTENSIONS: u8 = 8;
pub const HS_CERTIFICATE: u8 = 11;
pub const HS_CERTIFICATE_VERIFY: u8 = 15;
pub const HS_FINISHED: u8 = 20;
pub const HS_KEY_UPDATE: u8 = 24;

pub const AL_CLOSE_NOTIFY: u8 = 0;
pub const AL_UNEXPECTED_MESSAGE: u8 = 10;
pub const AL_BAD_RECORD_MAC: u8 = 20;
pub const AL_RECORD_OVERFLOW: u8 = 22;
pub const AL_HANDSHAKE_FAILURE: u8 = 40;
pub const AL_BAD_CERTIFICATE: u8 = 42;
pub const AL_CERTIFICATE_EXPIRED: u8 = 45;
pub const AL_ILLEGAL_PARAMETER: u8 = 47;
pub const AL_UNKNOWN_CA: u8 = 48;
pub const AL_DECODE_ERROR: u8 = 50;
pub const AL_DECRYPT_ERROR: u8 = 51;
pub const AL_PROTOCOL_VERSION: u8 = 70;
pub const AL_INTERNAL_ERROR: u8 = 80;
pub const AL_MISSING_EXTENSION: u8 = 109;

pub const EXT_SERVER_NAME: u16 = 0;
pub const EXT_GROUPS: u16 = 10;
pub const EXT_SIG_ALGS: u16 = 13;
pub const EXT_VERSIONS: u16 = 43;
pub const EXT_KEY_SHARE: u16 = 51;

const X25519: u16 = 0x001D;
const ED25519: u16 = 0x0807;
const TLS13: u16 = 0x0304;

/// the largest plain fragment
pub const MAX_PLAIN: usize = 1 << 14;
/// the largest record body on the wire
pub const MAX_RECORD: usize = MAX_PLAIN + 256;

/// the name of a content type
pub fn ct_name(t: u8) -> &'static str {
    match t {
        CT_CCS => "Change Cipher Spec",
        CT_ALERT => "Alert",
        CT_HANDSHAKE => "Handshake",
        CT_DATA => "Application Data",
        _ => "unknown",
    }
}

/// the name of a handshake type
pub fn hs_name(t: u8) -> &'static str {
    match t {
        HS_CLIENT_HELLO => "Client Hello",
        HS_SERVER_HELLO => "Server Hello",
        HS_NEW_SESSION_TICKET => "New Session Ticket",
        HS_ENCRYPTED_EXTENSIONS => "Encrypted Extensions",
        HS_CERTIFICATE => "Certificate",
        HS_CERTIFICATE_VERIFY => "Certificate Verify",
        HS_FINISHED => "Finished",
        HS_KEY_UPDATE => "Key Update",
        _ => "unknown",
    }
}

/// the name of an alert (RFC 8446 6)
pub fn alert_name(a: u8) -> &'static str {
    match a {
        AL_CLOSE_NOTIFY => "close_notify",
        AL_UNEXPECTED_MESSAGE => "unexpected_message",
        AL_BAD_RECORD_MAC => "bad_record_mac",
        AL_RECORD_OVERFLOW => "record_overflow",
        AL_HANDSHAKE_FAILURE => "handshake_failure",
        AL_BAD_CERTIFICATE => "bad_certificate",
        AL_CERTIFICATE_EXPIRED => "certificate_expired",
        AL_ILLEGAL_PARAMETER => "illegal_parameter",
        AL_UNKNOWN_CA => "unknown_ca",
        AL_DECODE_ERROR => "decode_error",
        AL_DECRYPT_ERROR => "decrypt_error",
        AL_PROTOCOL_VERSION => "protocol_version",
        AL_INTERNAL_ERROR => "internal_error",
        AL_MISSING_EXTENSION => "missing_extension",
        _ => "unknown alert",
    }
}

/// the name of an extension
pub fn ext_name(e: u16) -> &'static str {
    match e {
        EXT_SERVER_NAME => "server_name",
        EXT_GROUPS => "supported_groups",
        EXT_SIG_ALGS => "signature_algorithms",
        EXT_VERSIONS => "supported_versions",
        EXT_KEY_SHARE => "key_share",
        _ => "unknown",
    }
}

/// the name of a cipher suite
pub fn suite_name(s: u16) -> &'static str {
    match s {
        0x1301 => "TLS_AES_128_GCM_SHA256",
        0x1302 => "TLS_AES_256_GCM_SHA384",
        SUITE => "TLS_CHACHA20_POLY1305_SHA256",
        _ => "unknown",
    }
}

/// the name of a key exchange group
pub fn group_name(g: u16) -> &'static str {
    match g {
        X25519 => "x25519",
        0x0017 => "secp256r1",
        0x0018 => "secp384r1",
        _ => "unknown",
    }
}

/// the name of a signature scheme
pub fn sig_name(s: u16) -> &'static str {
    match s {
        ED25519 => "ed25519",
        0x0403 => "ecdsa_secp256r1_sha256",
        0x0804 => "rsa_pss_rsae_sha256",
        _ => "unknown",
    }
}

// ---------------------------------------------------------------- the key schedule

fn extract(salt: &[u8], ikm: &[u8]) -> [u8; 32] {
    let mut out = [0u8; 32];
    hkdf_extract::<Sha256>(salt, ikm, &mut out);
    out
}

/// HKDF-Expand-Label(secret, label, context, len) (7.1)
pub fn expand_label(secret: &[u8], label: &str, context: &[u8], out: &mut [u8]) {
    let mut info = Vec::with_capacity(10 + label.len() + context.len());
    info.extend_from_slice(&(out.len() as u16).to_be_bytes());
    info.push(6 + label.len() as u8);
    info.extend_from_slice(b"tls13 ");
    info.extend_from_slice(label.as_bytes());
    info.push(context.len() as u8);
    info.extend_from_slice(context);
    hkdf_expand::<Sha256>(secret, &info, out);
}

/// Derive-Secret(secret, label, transcript hash)
pub fn derive(secret: &[u8; 32], label: &str, hash: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    expand_label(secret, label, hash, &mut out);
    out
}

/// the handshake secret and the master secret from the (EC)DHE secret (no PSK)
pub fn schedule(ecdhe: &[u8; 32]) -> ([u8; 32], [u8; 32]) {
    let empty = crypto::sha256(b"");
    let early = extract(&[0u8; 32], &[0u8; 32]);
    let handshake = extract(&derive(&early, "derived", &empty), ecdhe);
    let master = extract(&derive(&handshake, "derived", &empty), &[0u8; 32]);
    (handshake, master)
}

/// verify_data of a Finished (4.4.4)
pub fn finished(base: &[u8; 32], transcript: &[u8; 32]) -> [u8; 32] {
    let mut key = [0u8; 32];
    expand_label(base, "finished", &[], &mut key);
    crypto::hmac_sha256(&key, transcript)
}

/// the AEAD key and IV of a traffic secret (7.3)
pub fn traffic_keys(secret: &[u8; 32]) -> ([u8; 32], [u8; 12]) {
    let (mut key, mut iv) = ([0u8; 32], [0u8; 12]);
    expand_label(secret, "key", &[], &mut key);
    expand_label(secret, "iv", &[], &mut iv);
    (key, iv)
}

fn cv_content(hash: &[u8; 32]) -> Vec<u8> {
    let mut c = alloc::vec![0x20u8; 64];
    c.extend_from_slice(b"TLS 1.3, server CertificateVerify");
    c.push(0);
    c.extend_from_slice(hash);
    c
}

// ---------------------------------------------------------------- records

// one direction's keys
#[derive(Clone)]
struct Cipher {
    key: [u8; 32],
    iv: [u8; 12],
    seq: u64,
}

impl Cipher {
    fn of(secret: &[u8; 32]) -> Cipher {
        let (key, iv) = traffic_keys(secret);
        Cipher { key, iv, seq: 0 }
    }

    fn nonce(&self) -> [u8; 12] {
        let mut n = self.iv;
        for (i, b) in self.seq.to_be_bytes().iter().enumerate() {
            n[4 + i] ^= b;
        }
        n
    }

    // a protected record of `content` of type `ty` (5.2)
    fn seal(&mut self, ty: u8, content: &[u8]) -> Vec<u8> {
        let len = content.len() + 1 + 16;
        let mut out = Vec::with_capacity(5 + len);
        out.extend_from_slice(&[CT_DATA, 3, 3, (len >> 8) as u8, len as u8]);
        let mut body = Vec::with_capacity(content.len() + 1);
        body.extend_from_slice(content);
        body.push(ty);
        let hdr = [out[0], out[1], out[2], out[3], out[4]];
        let tag = aead::seal_in_place(&self.key, &self.nonce(), &hdr, &mut body);
        self.seq += 1;
        out.extend_from_slice(&body);
        out.extend_from_slice(&tag);
        out
    }

    // a protected record's body (header given) -> type, content
    fn open(&mut self, hdr: &[u8; 5], body: &[u8]) -> Option<(u8, Vec<u8>)> {
        if body.len() < 17 || body.len() > MAX_RECORD {
            return None;
        }
        let n = body.len() - 16;
        let mut tag = [0u8; 16];
        tag.copy_from_slice(&body[n..]);
        let mut plain = Vec::from(&body[..n]);
        if !aead::open_in_place(&self.key, &self.nonce(), hdr, &mut plain, &tag) {
            return None;
        }
        self.seq += 1;
        while plain.last() == Some(&0) {
            plain.pop();
        }
        let ty = plain.pop()?;
        if plain.len() > MAX_PLAIN {
            return None;
        }
        Some((ty, plain))
    }
}

fn plain_record(version: u8, ty: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + content.len());
    out.extend_from_slice(&[ty, 3, version, (content.len() >> 8) as u8, content.len() as u8]);
    out.extend_from_slice(content);
    out
}

// ---------------------------------------------------------------- reading and writing fields

struct R<'a> {
    b: &'a [u8],
}

type Res<T> = Result<T, (u8, &'static str)>;

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        if self.b.len() < n {
            return Err((AL_DECODE_ERROR, "message too short"));
        }
        let (a, rest) = self.b.split_at(n);
        self.b = rest;
        Ok(a)
    }
    fn u8(&mut self) -> Res<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Res<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn v8(&mut self) -> Res<&'a [u8]> {
        let n = self.u8()? as usize;
        self.take(n)
    }
    fn v16(&mut self) -> Res<&'a [u8]> {
        let n = self.u16()? as usize;
        self.take(n)
    }
    fn v24(&mut self) -> Res<&'a [u8]> {
        let b = self.take(3)?;
        let n = (b[0] as usize) << 16 | (b[1] as usize) << 8 | b[2] as usize;
        self.take(n)
    }
    fn done(&self) -> Res<()> {
        if self.b.is_empty() { Ok(()) } else { Err((AL_DECODE_ERROR, "trailing bytes")) }
    }
}

fn put_v8(out: &mut Vec<u8>, b: &[u8]) {
    out.push(b.len() as u8);
    out.extend_from_slice(b);
}

fn put_v16(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u16).to_be_bytes());
    out.extend_from_slice(b);
}

fn put_v24(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&[(b.len() >> 16) as u8, (b.len() >> 8) as u8, b.len() as u8]);
    out.extend_from_slice(b);
}

fn put_ext(out: &mut Vec<u8>, t: u16, d: &[u8]) {
    out.extend_from_slice(&t.to_be_bytes());
    put_v16(out, d);
}

fn hs_msg(ty: u8, body: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(4 + body.len());
    m.push(ty);
    put_v24(&mut m, body);
    m
}

// the extensions of a hello, none twice
fn extensions(b: &[u8]) -> Res<Vec<(u16, &[u8])>> {
    let mut r = R { b };
    let mut out: Vec<(u16, &[u8])> = Vec::new();
    while !r.b.is_empty() {
        let t = r.u16()?;
        let d = r.v16()?;
        if out.iter().any(|x| x.0 == t) {
            return Err((AL_ILLEGAL_PARAMETER, "an extension twice"));
        }
        out.push((t, d));
    }
    Ok(out)
}

fn ext<'a>(list: &[(u16, &'a [u8])], t: u16) -> Option<&'a [u8]> {
    list.iter().find(|x| x.0 == t).map(|x| x.1)
}

fn u16s(b: &[u8]) -> Res<Vec<u16>> {
    if b.len() % 2 != 0 {
        return Err((AL_DECODE_ERROR, "a list of an odd length"));
    }
    Ok(b.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect())
}

// ---------------------------------------------------------------- the core

/// a certificate chain (DER, leaf first) and the leaf's key
#[derive(Clone)]
pub struct Identity {
    pub chain: Vec<Vec<u8>>,
    pub key: Keypair,
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Identity({} certificates)", self.chain.len())
    }
}

/// what a client trusts (see the module header)
#[derive(Clone, Debug, Default)]
pub struct ClientCfg {
    /// the name asked for: what the certificate must name, the SNI unless it is an address
    pub name: String,
    pub anchors: Vec<Cert>,
    /// seconds since 1970, for the certificates' times
    pub now: i64,
    /// any certificate goes
    pub insecure: bool,
    /// fingerprints (SHA-256 of the leaf) that go whatever their faults
    pub accept: Vec<[u8; 32]>,
}

/// what ended a connection
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failure {
    pub alert: u8,
    /// we sent the alert (false: the peer did)
    pub ours: bool,
    pub why: &'static str,
}

/// what happened
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// a server: a ClientHello came, with this SNI ("" for none)
    Hello { name: String },
    /// a client: the server's chain came and was checked (x509 F_* bits; 0: nothing wrong)
    Certificate { faults: u32 },
    Established,
    /// close_notify came
    PeerClosed,
    Failed(Failure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    ClientSh,
    ClientEe,
    ClientCert,
    ClientCv,
    ClientFin,
    ServerCh,
    ServerFin,
    Established,
    Closed,
}

/// a connection's TLS, one side (see the module header)
#[derive(Clone)]
pub struct Core {
    server: bool,
    state: State,
    transcript: Sha256,
    inbuf: Vec<u8>,
    hs: Vec<u8>,
    out: Vec<u8>,
    plain: Vec<u8>,
    events: Vec<Event>,
    rd: Option<Cipher>,
    wr: Option<Cipher>,
    c_hs: [u8; 32],
    s_hs: [u8; 32],
    c_ap: [u8; 32],
    s_ap: [u8; 32],
    master: [u8; 32],
    eph: [u8; 32],
    session_id: Vec<u8>,
    seed: [u8; 32],
    cfg: Option<ClientCfg>,
    id: Option<Identity>,
    name: String,
    peer: Vec<Cert>,
    faults: u32,
    failure: Option<Failure>,
    peer_closed: bool,
    key_updates: u32,
}

impl fmt::Debug for Core {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tls::Core({:?}, {})", self.state, if self.server { "server" } else { "client" })
    }
}

// random bytes of a connection: the seed and what they are for
fn rand32(seed: &[u8; 32], what: &[u8]) -> [u8; 32] {
    crypto::hmac_sha256(seed, what)
}

impl Core {
    fn new(server: bool, seed: &[u8; 32]) -> Core {
        Core {
            server,
            state: if server { State::ServerCh } else { State::ClientSh },
            transcript: Sha256::new(),
            inbuf: Vec::new(),
            hs: Vec::new(),
            out: Vec::new(),
            plain: Vec::new(),
            events: Vec::new(),
            rd: None,
            wr: None,
            c_hs: [0; 32],
            s_hs: [0; 32],
            c_ap: [0; 32],
            s_ap: [0; 32],
            master: [0; 32],
            eph: rand32(seed, b"tls ephemeral"),
            session_id: Vec::new(),
            seed: *seed,
            cfg: None,
            id: None,
            name: String::new(),
            peer: Vec::new(),
            faults: 0,
            failure: None,
            peer_closed: false,
            key_updates: 0,
        }
    }

    /// a client: its ClientHello waits in take_out at once
    pub fn client(cfg: ClientCfg, seed: &[u8; 32]) -> Core {
        let mut c = Core::new(false, seed);
        c.name = cfg.name.clone();
        c.session_id = Vec::from(&rand32(seed, b"tls session id")[..]);
        c.cfg = Some(cfg);
        c.client_hello();
        c
    }

    /// a server
    pub fn server(id: Identity, seed: &[u8; 32]) -> Core {
        let mut c = Core::new(true, seed);
        c.id = Some(id);
        c
    }

    pub fn established(&self) -> bool {
        self.state == State::Established
    }

    /// close_notify came
    pub fn peer_closed(&self) -> bool {
        self.peer_closed
    }

    /// nothing more will come or go: a failure, or both sides said close_notify
    pub fn dead(&self) -> bool {
        self.state == State::Closed
    }

    /// the name: the SNI a client sent (a server), the one asked for (a client)
    pub fn name(&self) -> &str {
        &self.name
    }

    /// the server's chain, leaf first (a client, once it came)
    pub fn peer_certs(&self) -> &[Cert] {
        &self.peer
    }

    /// what is wrong with the server's chain (x509 F_* bits)
    pub fn faults(&self) -> u32 {
        self.faults
    }

    /// the server's chain has faults and the handshake went on all the same (ClientCfg)
    pub fn excused(&self) -> bool {
        self.faults != 0 && self.failure.is_none()
    }

    /// what ended the handshake or the connection
    pub fn failure(&self) -> Option<Failure> {
        self.failure
    }

    /// the bytes it holds (an estimate for a budget)
    pub fn footprint(&self) -> usize {
        1024 + self.inbuf.capacity() + self.hs.capacity() + self.out.capacity() + self.plain.capacity() + self.peer.iter().map(Cert::bytes).sum::<usize>() + self.cfg.as_ref().map_or(0, |c| c.anchors.iter().map(Cert::bytes).sum::<usize>())
    }

    pub fn take_out(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.out)
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        core::mem::take(&mut self.events)
    }

    /// the plain text that came and was not read yet
    pub fn read(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.plain)
    }

    /// plain text to send (nothing before the handshake's end: false)
    pub fn send(&mut self, data: &[u8]) -> bool {
        if self.state != State::Established {
            return false;
        }
        for chunk in data.chunks(MAX_PLAIN) {
            if let Some(w) = self.wr.as_mut() {
                let rec = w.seal(CT_DATA, chunk);
                self.out.extend_from_slice(&rec);
            }
        }
        true
    }

    /// close_notify; nothing is sent after it
    pub fn close(&mut self) {
        if self.state != State::Closed {
            self.alert(AL_CLOSE_NOTIFY, false);
            self.state = State::Closed;
        }
    }

    fn alert(&mut self, desc: u8, fatal: bool) {
        let body = [if fatal { 2 } else { 1 }, desc];
        let rec = match self.wr.as_mut() {
            Some(c) => c.seal(CT_ALERT, &body),
            None => plain_record(3, CT_ALERT, &body),
        };
        self.out.extend_from_slice(&rec);
    }

    fn hash(&self) -> [u8; 32] {
        self.transcript.clone().finalize()
    }

    fn send_hs(&mut self, msg: &[u8], version: u8) {
        self.transcript.update(msg);
        let rec = match self.wr.as_mut() {
            Some(c) => c.seal(CT_HANDSHAKE, msg),
            None => plain_record(version, CT_HANDSHAKE, msg),
        };
        self.out.extend_from_slice(&rec);
    }

    /// bytes from the peer (any split)
    pub fn push(&mut self, data: &[u8]) {
        if self.state == State::Closed || self.peer_closed {
            return;
        }
        self.inbuf.extend_from_slice(data);
        if let Err((alert, why)) = self.records() {
            self.alert(alert, true);
            self.state = State::Closed;
            self.fail(Failure { alert, ours: true, why });
        }
    }

    fn fail(&mut self, f: Failure) {
        self.failure = Some(f);
        self.events.push(Event::Failed(f));
    }

    fn records(&mut self) -> Res<()> {
        loop {
            if self.peer_closed || self.state == State::Closed || self.inbuf.len() < 5 {
                return Ok(());
            }
            let ty = self.inbuf[0];
            let len = (self.inbuf[3] as usize) << 8 | self.inbuf[4] as usize;
            if !(CT_CCS..=CT_DATA).contains(&ty) || self.inbuf[1] != 3 {
                return Err((AL_PROTOCOL_VERSION, "not a TLS record"));
            }
            if len > MAX_RECORD {
                return Err((AL_RECORD_OVERFLOW, "a record too long"));
            }
            if self.inbuf.len() < 5 + len {
                return Ok(());
            }
            let hdr = [self.inbuf[0], self.inbuf[1], self.inbuf[2], self.inbuf[3], self.inbuf[4]];
            let body: Vec<u8> = self.inbuf[5..5 + len].to_vec();
            self.inbuf.drain(..5 + len);
            match ty {
                // compatibility mode: one byte 1, only during the handshake
                CT_CCS => {
                    if body != [1] || self.state == State::Established {
                        return Err((AL_UNEXPECTED_MESSAGE, "change_cipher_spec"));
                    }
                }
                CT_DATA => {
                    let rd = self.rd.as_mut().ok_or((AL_UNEXPECTED_MESSAGE, "data before the keys"))?;
                    let (inner, content) = rd.open(&hdr, &body).ok_or((AL_BAD_RECORD_MAC, "a record that does not authenticate"))?;
                    match inner {
                        CT_HANDSHAKE => self.handshake_bytes(&content)?,
                        CT_ALERT => self.peer_alert(&content)?,
                        CT_DATA => {
                            if self.state != State::Established {
                                return Err((AL_UNEXPECTED_MESSAGE, "data during the handshake"));
                            }
                            self.plain.extend_from_slice(&content);
                        }
                        _ => return Err((AL_UNEXPECTED_MESSAGE, "an inner type")),
                    }
                }
                CT_HANDSHAKE => {
                    if self.rd.is_some() {
                        return Err((AL_UNEXPECTED_MESSAGE, "a handshake in the clear after the keys"));
                    }
                    if body.is_empty() {
                        return Err((AL_DECODE_ERROR, "an empty handshake record"));
                    }
                    self.handshake_bytes(&body)?;
                }
                // a peer that fails before it has our keys sends its alert in the clear
                CT_ALERT if self.rd.is_none() || self.state != State::Established => self.peer_alert(&body)?,
                _ => return Err((AL_UNEXPECTED_MESSAGE, "a record type")),
            }
        }
    }

    fn peer_alert(&mut self, b: &[u8]) -> Res<()> {
        if b.len() != 2 {
            return Err((AL_DECODE_ERROR, "an alert"));
        }
        if b[1] == AL_CLOSE_NOTIFY {
            self.peer_closed = true;
            self.events.push(Event::PeerClosed);
            return Ok(());
        }
        self.state = State::Closed;
        self.fail(Failure { alert: b[1], ours: false, why: alert_name(b[1]) });
        Ok(())
    }

    fn handshake_bytes(&mut self, b: &[u8]) -> Res<()> {
        self.hs.extend_from_slice(b);
        loop {
            if self.hs.len() < 4 {
                return Ok(());
            }
            let n = (self.hs[1] as usize) << 16 | (self.hs[2] as usize) << 8 | self.hs[3] as usize;
            if n > 64 * 1024 {
                return Err((AL_DECODE_ERROR, "a handshake message too long"));
            }
            if self.hs.len() < 4 + n {
                return Ok(());
            }
            let msg: Vec<u8> = self.hs[..4 + n].to_vec();
            self.hs.drain(..4 + n);
            let keys_before = self.rd.is_some();
            self.message(&msg)?;
            // a key change falls on a record's end
            if self.rd.is_some() != keys_before && !self.hs.is_empty() {
                return Err((AL_UNEXPECTED_MESSAGE, "a handshake message across a key change"));
            }
        }
    }

    fn message(&mut self, msg: &[u8]) -> Res<()> {
        let (ty, body) = (msg[0], &msg[4..]);
        match (self.state, ty) {
            (State::ServerCh, HS_CLIENT_HELLO) => self.server_hello(msg, body),
            (State::ClientSh, HS_SERVER_HELLO) => self.client_sh(msg, body),
            (State::ClientEe, HS_ENCRYPTED_EXTENSIONS) => {
                let mut r = R { b: body };
                extensions(r.v16()?)?;
                r.done()?;
                self.transcript.update(msg);
                self.state = State::ClientCert;
                Ok(())
            }
            (State::ClientCert, HS_CERTIFICATE) => {
                self.certificate(body)?;
                self.transcript.update(msg);
                self.state = State::ClientCv;
                Ok(())
            }
            (State::ClientCv, HS_CERTIFICATE_VERIFY) => {
                let mut r = R { b: body };
                let scheme = r.u16()?;
                let sig = r.v16()?;
                r.done()?;
                if scheme != ED25519 {
                    return Err((AL_ILLEGAL_PARAMETER, "a signature scheme not offered"));
                }
                let key = self.peer.first().and_then(|c| c.key).ok_or((AL_BAD_CERTIFICATE, "a certificate without an Ed25519 key"))?;
                let sig: &[u8; 64] = sig.try_into().map_err(|_| (AL_DECODE_ERROR, "a signature's length"))?;
                if !ed25519::verify(&key, &cv_content(&self.hash()), sig) {
                    return Err((AL_DECRYPT_ERROR, "CertificateVerify"));
                }
                self.transcript.update(msg);
                self.state = State::ClientFin;
                Ok(())
            }
            (State::ClientFin, HS_FINISHED) => self.client_finished(msg, body),
            (State::ServerFin, HS_FINISHED) => {
                let want = finished(&self.c_hs, &self.hash());
                if !crypto::ct_eq(&want, body) {
                    return Err((AL_DECRYPT_ERROR, "the client's Finished"));
                }
                self.transcript.update(msg);
                self.rd = Some(Cipher::of(&self.c_ap));
                self.state = State::Established;
                self.events.push(Event::Established);
                Ok(())
            }
            (State::Established, HS_NEW_SESSION_TICKET) if !self.server => Ok(()),
            (State::Established, HS_KEY_UPDATE) => {
                let mut r = R { b: body };
                let requested = r.u8()?;
                r.done()?;
                if requested > 1 {
                    return Err((AL_ILLEGAL_PARAMETER, "key_update"));
                }
                self.key_updates += 1;
                if self.key_updates > 1024 {
                    return Err((AL_UNEXPECTED_MESSAGE, "too many key updates"));
                }
                let peer = if self.server { &mut self.c_ap } else { &mut self.s_ap };
                let mut next = [0u8; 32];
                expand_label(&peer[..], "traffic upd", &[], &mut next);
                *peer = next;
                self.rd = Some(Cipher::of(&next));
                if requested == 1 {
                    let m = hs_msg(HS_KEY_UPDATE, &[0]);
                    if let Some(w) = self.wr.as_mut() {
                        let rec = w.seal(CT_HANDSHAKE, &m);
                        self.out.extend_from_slice(&rec);
                    }
                    let mine = if self.server { &mut self.s_ap } else { &mut self.c_ap };
                    let mut next = [0u8; 32];
                    expand_label(&mine[..], "traffic upd", &[], &mut next);
                    *mine = next;
                    self.wr = Some(Cipher::of(&next));
                }
                Ok(())
            }
            _ => Err((AL_UNEXPECTED_MESSAGE, "a handshake message out of order")),
        }
    }

    // ---------------------------------------------------------------- the client

    fn client_hello(&mut self) {
        let random = rand32(&self.seed, b"tls client random");
        let mut b = Vec::new();
        b.extend_from_slice(&[3, 3]);
        b.extend_from_slice(&random);
        let sid = self.session_id.clone();
        put_v8(&mut b, &sid);
        put_v16(&mut b, &SUITE.to_be_bytes());
        put_v8(&mut b, &[0]);
        let mut e = Vec::new();
        if !self.name.is_empty() && IpAddr::parse(self.name.trim_matches(['[', ']'])).is_none() {
            let mut sni = alloc::vec![0u8];
            put_v16(&mut sni, self.name.as_bytes());
            let mut list = Vec::new();
            put_v16(&mut list, &sni);
            put_ext(&mut e, EXT_SERVER_NAME, &list);
        }
        put_ext(&mut e, EXT_VERSIONS, &[2, 0x03, 0x04]);
        put_ext(&mut e, EXT_GROUPS, &[0, 2, 0x00, 0x1D]);
        put_ext(&mut e, EXT_SIG_ALGS, &[0, 2, 0x08, 0x07]);
        let mut entry = Vec::new();
        entry.extend_from_slice(&X25519.to_be_bytes());
        put_v16(&mut entry, &x25519::public_key(&self.eph));
        let mut ks = Vec::new();
        put_v16(&mut ks, &entry);
        put_ext(&mut e, EXT_KEY_SHARE, &ks);
        put_v16(&mut b, &e);
        // legacy_record_version 0x0301 for the first ClientHello (5.1)
        self.send_hs(&hs_msg(HS_CLIENT_HELLO, &b), 1);
    }

    fn client_sh(&mut self, msg: &[u8], body: &[u8]) -> Res<()> {
        let mut r = R { b: body };
        if r.u16()? != 0x0303 {
            return Err((AL_PROTOCOL_VERSION, "a legacy version"));
        }
        r.take(32)?;
        let sid = r.v8()?;
        let suite = r.u16()?;
        if r.u8()? != 0 {
            return Err((AL_ILLEGAL_PARAMETER, "compression"));
        }
        let exts = extensions(r.v16()?)?;
        r.done()?;
        if ext(&exts, EXT_VERSIONS) != Some(&[0x03u8, 0x04][..]) {
            return Err((AL_PROTOCOL_VERSION, "the server does not speak TLS 1.3"));
        }
        if sid != self.session_id.as_slice() {
            return Err((AL_ILLEGAL_PARAMETER, "a session id not echoed"));
        }
        if suite != SUITE {
            return Err((AL_ILLEGAL_PARAMETER, "a cipher suite not offered"));
        }
        let ks = ext(&exts, EXT_KEY_SHARE).ok_or((AL_MISSING_EXTENSION, "key_share"))?;
        let mut k = R { b: ks };
        if k.u16()? != X25519 {
            return Err((AL_ILLEGAL_PARAMETER, "a key share's group"));
        }
        let pk = k.v16()?;
        k.done()?;
        let pk: &[u8; 32] = pk.try_into().map_err(|_| (AL_ILLEGAL_PARAMETER, "a key share's length"))?;
        let shared = x25519::shared(&self.eph, pk).ok_or((AL_ILLEGAL_PARAMETER, "a key share"))?;
        self.transcript.update(msg);
        self.handshake_keys(&shared);
        self.rd = Some(Cipher::of(&self.s_hs));
        self.state = State::ClientEe;
        Ok(())
    }

    fn handshake_keys(&mut self, shared: &[u8; 32]) {
        let (handshake, master) = schedule(shared);
        let h = self.hash();
        self.c_hs = derive(&handshake, "c hs traffic", &h);
        self.s_hs = derive(&handshake, "s hs traffic", &h);
        self.master = master;
    }

    fn app_secrets(&mut self) {
        let h = self.hash();
        self.c_ap = derive(&self.master, "c ap traffic", &h);
        self.s_ap = derive(&self.master, "s ap traffic", &h);
    }

    fn certificate(&mut self, body: &[u8]) -> Res<()> {
        let mut r = R { b: body };
        if !r.v8()?.is_empty() {
            return Err((AL_ILLEGAL_PARAMETER, "a certificate context"));
        }
        let mut list = R { b: r.v24()? };
        r.done()?;
        let mut chain = Vec::new();
        while !list.b.is_empty() {
            let der = list.v24()?;
            list.v16()?;
            if chain.len() >= 10 {
                return Err((AL_BAD_CERTIFICATE, "a chain too long"));
            }
            chain.push(Cert::parse(der).ok_or((AL_BAD_CERTIFICATE, "a certificate that cannot be read"))?);
        }
        if chain.is_empty() {
            return Err((AL_DECODE_ERROR, "no certificate"));
        }
        let Some(cfg) = self.cfg.as_ref() else { return Err((AL_INTERNAL_ERROR, "no configuration")) };
        let faults = check_chain(&chain, &cfg.anchors, &cfg.name, cfg.now);
        let excused = cfg.insecure || cfg.accept.contains(&chain[0].fingerprint());
        self.peer = chain;
        self.faults = faults;
        self.events.push(Event::Certificate { faults });
        if faults == 0 || excused {
            return Ok(());
        }
        // the alert for the worst of them (RFC 8446 6.2)
        Err(if faults & (F_EXPIRED | F_NOT_YET) != 0 {
            (AL_CERTIFICATE_EXPIRED, "the certificate's time")
        } else if faults & (F_SELF_SIGNED | F_UNKNOWN_ISSUER) != 0 {
            (AL_UNKNOWN_CA, "an issuer nobody trusts")
        } else if faults & F_NAME != 0 {
            (AL_BAD_CERTIFICATE, "a certificate for another name")
        } else {
            (AL_BAD_CERTIFICATE, "a certificate that does not check out")
        })
    }

    fn client_finished(&mut self, msg: &[u8], body: &[u8]) -> Res<()> {
        let want = finished(&self.s_hs, &self.hash());
        if !crypto::ct_eq(&want, body) {
            return Err((AL_DECRYPT_ERROR, "the server's Finished"));
        }
        self.transcript.update(msg);
        self.app_secrets();
        self.wr = Some(Cipher::of(&self.c_hs));
        let fin = finished(&self.c_hs, &self.hash());
        self.send_hs(&hs_msg(HS_FINISHED, &fin), 3);
        self.wr = Some(Cipher::of(&self.c_ap));
        self.rd = Some(Cipher::of(&self.s_ap));
        self.state = State::Established;
        self.events.push(Event::Established);
        Ok(())
    }

    // ---------------------------------------------------------------- the server

    fn server_hello(&mut self, msg: &[u8], body: &[u8]) -> Res<()> {
        let mut r = R { b: body };
        r.u16()?;
        r.take(32)?;
        let sid = Vec::from(r.v8()?);
        if sid.len() > 32 {
            return Err((AL_ILLEGAL_PARAMETER, "a session id"));
        }
        let suites = u16s(r.v16()?)?;
        let comp = r.v8()?;
        let exts = extensions(r.v16()?)?;
        r.done()?;
        if comp != [0] {
            return Err((AL_ILLEGAL_PARAMETER, "compression"));
        }
        let versions = ext(&exts, EXT_VERSIONS).ok_or((AL_PROTOCOL_VERSION, "TLS 1.3 required"))?;
        if !u16s(R { b: versions }.v8()?)?.contains(&TLS13) {
            return Err((AL_PROTOCOL_VERSION, "TLS 1.3 required"));
        }
        if !suites.contains(&SUITE) {
            return Err((AL_HANDSHAKE_FAILURE, "TLS_CHACHA20_POLY1305_SHA256 required"));
        }
        let algs = u16s(R { b: ext(&exts, EXT_SIG_ALGS).ok_or((AL_MISSING_EXTENSION, "signature_algorithms"))? }.v16()?)?;
        if !algs.contains(&ED25519) {
            return Err((AL_HANDSHAKE_FAILURE, "Ed25519 required"));
        }
        let mut share = None;
        if let Some(ks) = ext(&exts, EXT_KEY_SHARE) {
            let mut k = R { b: R { b: ks }.v16()? };
            while !k.b.is_empty() {
                let g = k.u16()?;
                let key = k.v16()?;
                if g == X25519 {
                    share = Some(key);
                }
            }
        }
        // (no HelloRetryRequest here)
        let pk = share.ok_or((AL_HANDSHAKE_FAILURE, "an X25519 key share required"))?;
        let pk: &[u8; 32] = pk.try_into().map_err(|_| (AL_ILLEGAL_PARAMETER, "a key share's length"))?;
        if let Some(n) = ext(&exts, EXT_SERVER_NAME) {
            let mut list = R { b: R { b: n }.v16()? };
            if list.u8()? == 0 {
                self.name = String::from_utf8_lossy(list.v16()?).into_owned();
            }
        }
        self.events.push(Event::Hello { name: self.name.clone() });
        let shared = x25519::shared(&self.eph, pk).ok_or((AL_ILLEGAL_PARAMETER, "a key share"))?;
        self.transcript.update(msg);
        // ServerHello
        let random = rand32(&self.seed, b"tls server random");
        let mut b = Vec::new();
        b.extend_from_slice(&[3, 3]);
        b.extend_from_slice(&random);
        put_v8(&mut b, &sid);
        b.extend_from_slice(&SUITE.to_be_bytes());
        b.push(0);
        let mut e = Vec::new();
        put_ext(&mut e, EXT_VERSIONS, &[0x03, 0x04]);
        let mut ks = Vec::new();
        ks.extend_from_slice(&X25519.to_be_bytes());
        put_v16(&mut ks, &x25519::public_key(&self.eph));
        put_ext(&mut e, EXT_KEY_SHARE, &ks);
        put_v16(&mut b, &e);
        self.send_hs(&hs_msg(HS_SERVER_HELLO, &b), 3);
        self.handshake_keys(&shared);
        self.wr = Some(Cipher::of(&self.s_hs));
        self.rd = Some(Cipher::of(&self.c_hs));
        self.send_hs(&hs_msg(HS_ENCRYPTED_EXTENSIONS, &[0, 0]), 3);
        let id = self.id.clone().ok_or((AL_INTERNAL_ERROR, "no certificate"))?;
        let mut list = Vec::new();
        for c in &id.chain {
            put_v24(&mut list, c);
            list.extend_from_slice(&[0, 0]);
        }
        let mut cb = alloc::vec![0u8];
        put_v24(&mut cb, &list);
        self.send_hs(&hs_msg(HS_CERTIFICATE, &cb), 3);
        let sig = id.key.sign(&cv_content(&self.hash()));
        let mut b = Vec::new();
        b.extend_from_slice(&ED25519.to_be_bytes());
        put_v16(&mut b, &sig);
        self.send_hs(&hs_msg(HS_CERTIFICATE_VERIFY, &b), 3);
        let fin = finished(&self.s_hs, &self.hash());
        self.send_hs(&hs_msg(HS_FINISHED, &fin), 3);
        self.app_secrets();
        self.wr = Some(Cipher::of(&self.s_ap));
        self.state = State::ServerFin;
        Ok(())
    }
}

// ---------------------------------------------------------------- what a dissector sees

/// a part of a TCP segment
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// a record: its place, its bytes here (less than 5 + length when the rest comes later), its type,
    /// its version on the wire, its length
    Record { at: usize, len: usize, ty: u8, version: u16, length: usize },
    /// bytes that do not start a record (the rest of one)
    Rest { at: usize, len: usize },
}

/// the parts of a segment's payload
pub fn dissect(b: &[u8]) -> Vec<Part> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let h = &b[at..];
        if h.len() >= 5 && (CT_CCS..=CT_DATA).contains(&h[0]) && h[1] == 3 && h[2] <= 4 {
            let length = (h[3] as usize) << 8 | h[4] as usize;
            if length <= MAX_RECORD {
                let len = (5 + length).min(h.len());
                out.push(Part::Record { at, len, ty: h[0], version: u16::from_be_bytes([h[1], h[2]]), length });
                at += len;
                continue;
            }
        }
        out.push(Part::Rest { at, len: h.len() });
        break;
    }
    out
}
