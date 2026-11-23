// conn -- the TLS handshake and the record stream, without I/O
//
//   Conn::client(cfg) / Conn::server(cfg)
//   feed(bytes)          bytes from the peer (any split)
//   take_out() -> bytes  to send to the peer
//   read_plain / write_plain / close
//   established(), peer_cert(), version(), suite()
//
// TLS 1.3 on both sides: the suites ChaCha20-Poly1305, AES-128-GCM (SHA-256) and AES-256-GCM
// (SHA-384); the groups X25519MLKEM768 (post-quantum hybrid), X25519, P-256 and P-384, and on
// the server also SecP256r1MLKEM768 and SecP384r1MLKEM1024 (kx.rs); certificates and handshake
// signatures with Ed25519, ECDSA (P-256 / P-384), RSA-PSS (RSA PKCS #1 v1.5 for certificates);
// mutual authentication with any of these keys.  A client sends key shares for
// X25519MLKEM768 and X25519 and takes a HelloRetryRequest for P-256 or P-384; a server sends one
// for the first group of the client it has when no share fits, and a dummy change_cipher_spec
// for middleboxes (D.4).
//
// A client that trusts the system's certificate authorities (Trust::System: mail servers,
// update servers, web hooks) or a check of its own (Trust::Custom) falls back to TLS 1.2 with
// ECDHE (X25519, P-256, P-384) and AEAD suites (AES-GCM, ChaCha20-Poly1305), the extended
// master secret when the server has it and the TLS 1.3 downgrade check.  Other connections
// never fall back.
//
// No PSK, no 0-RTT, no session tickets (a client ignores them), no renegotiation;
// post-handshake KeyUpdate is supported.  Every protocol error ends the connection with the
// matching alert.

use alloc::string::String;
use alloc::vec::Vec;

use super::keysched::{self, Schedule, Secret, Transcript};
use super::kx::{self, Share, SECP256R1, SECP256R1_MLKEM768, SECP384R1, SECP384R1_MLKEM1024, X25519, X25519_MLKEM768};
use super::record::{self, Cipher, ALERT, APPLICATION_DATA, CHANGE_CIPHER_SPEC, HANDSHAKE, MAX_PLAIN, MAX_RECORD};
use super::record::{ECDHE_ECDSA_AES128_GCM, ECDHE_ECDSA_AES256_GCM, ECDHE_ECDSA_CHACHA20, ECDHE_RSA_AES128_GCM, ECDHE_RSA_AES256_GCM, ECDHE_RSA_CHACHA20};
use super::record::{TLS_AES_128_GCM_SHA256, TLS_AES_256_GCM_SHA384, TLS_CHACHA20_POLY1305_SHA256};
use super::{ClientConfig, Hello, Identity, ServerConfig, TlsError, Trust};
use crate::x509::{verify_chain, Cert, PrivateKey, PubKey, SigAlg, Usage};
use crypto::ecdsa::Curve;
use crypto::HashAlg;

const ED25519: u16 = 0x0807;
const TLS13: u16 = 0x0304;

/// the TLS 1.3 suites, in the order a client offers them and a server prefers them
const SUITES_13: [u16; 3] = [TLS_CHACHA20_POLY1305_SHA256, TLS_AES_128_GCM_SHA256, TLS_AES_256_GCM_SHA384];
const SUITES_12: [u16; 6] = [ECDHE_ECDSA_CHACHA20, ECDHE_RSA_CHACHA20, ECDHE_ECDSA_AES128_GCM, ECDHE_RSA_AES128_GCM, ECDHE_ECDSA_AES256_GCM, ECDHE_RSA_AES256_GCM];
/// the groups a client lists (the first two with a key share in its first ClientHello, the
/// others for a HelloRetryRequest)
const CLIENT_GROUPS: [u16; 6] = [X25519_MLKEM768, X25519, SECP256R1, SECP384R1, SECP256R1_MLKEM768, SECP384R1_MLKEM1024];
/// signature schemes a web client accepts (CertificateVerify, ServerKeyExchange, certificates)
const WEB_SIG_ALGS: [u16; 9] = [0x0403, 0x0503, 0x0804, 0x0805, 0x0806, ED25519, 0x0401, 0x0501, 0x0601];
/// "DOWNGRD" + 1 / 0: the last bytes of a TLS 1.3 server's random when it speaks TLS 1.2 / older
const DOWNGRADE: [u8; 7] = *b"DOWNGRD";

const EXT_SERVER_NAME: u16 = 0;
const EXT_GROUPS: u16 = 10;
const EXT_POINT_FORMATS: u16 = 11;
const EXT_SIG_ALGS: u16 = 13;
const EXT_ALPN: u16 = 16;
/// the most bytes of protocol names a ClientHello offers (ClientConfig::alpn)
const MAX_ALPN: usize = 8 * 1024;
const EXT_EMS: u16 = 23;
const EXT_COOKIE: u16 = 44;
const EXT_EARLY_DATA: u16 = 42;
const EXT_PSK: u16 = 41;
const EXT_PADDING: u16 = 21;
/// early data a server skips (it never accepts 0-RTT, RFC 8446 4.2.10): at most this many bytes
const MAX_EARLY_SKIP: usize = 4 * 16384;
const EXT_VERSIONS: u16 = 43;
const EXT_KEY_SHARE: u16 = 51;
const EXT_RENEGOTIATION_INFO: u16 = 0xff01;

const HS_HELLO_REQUEST: u8 = 0;
const HS_CLIENT_HELLO: u8 = 1;
const HS_SERVER_HELLO: u8 = 2;
const HS_NEW_SESSION_TICKET: u8 = 4;
const HS_ENCRYPTED_EXTENSIONS: u8 = 8;
const HS_CERTIFICATE: u8 = 11;
const HS_SERVER_KEY_EXCHANGE: u8 = 12;
const HS_CERTIFICATE_REQUEST: u8 = 13;
const HS_SERVER_HELLO_DONE: u8 = 14;
const HS_CERTIFICATE_VERIFY: u8 = 15;
const HS_CLIENT_KEY_EXCHANGE: u8 = 16;
const HS_FINISHED: u8 = 20;
const HS_KEY_UPDATE: u8 = 24;
const HS_MESSAGE_HASH: u8 = 254;

pub const AL_CLOSE_NOTIFY: u8 = 0;
pub const AL_UNEXPECTED_MESSAGE: u8 = 10;
pub const AL_BAD_RECORD_MAC: u8 = 20;
pub const AL_RECORD_OVERFLOW: u8 = 22;
pub const AL_HANDSHAKE_FAILURE: u8 = 40;
pub const AL_BAD_CERTIFICATE: u8 = 42;
pub const AL_UNSUPPORTED_CERTIFICATE: u8 = 43;
pub const AL_CERTIFICATE_EXPIRED: u8 = 45;
pub const AL_ILLEGAL_PARAMETER: u8 = 47;
pub const AL_UNKNOWN_CA: u8 = 48;
pub const AL_DECODE_ERROR: u8 = 50;
pub const AL_DECRYPT_ERROR: u8 = 51;
pub const AL_PROTOCOL_VERSION: u8 = 70;
pub const AL_INTERNAL_ERROR: u8 = 80;
pub const AL_NO_RENEGOTIATION: u8 = 100;
pub const AL_MISSING_EXTENSION: u8 = 109;
pub const AL_UNSUPPORTED_EXTENSION: u8 = 110;
pub const AL_CERTIFICATE_REQUIRED: u8 = 116;
pub const AL_NO_APPLICATION_PROTOCOL: u8 = 120;

/// SHA-256("HelloRetryRequest"): the random of a HelloRetryRequest
const HRR_RANDOM: [u8; 32] = [
    0xCF, 0x21, 0xAD, 0x74, 0xE5, 0x9A, 0x61, 0x11, 0xBE, 0x1D, 0x8C, 0x02, 0x1E, 0x65, 0xB8, 0x91, 0xC2, 0xA2, 0x11, 0x16, 0x7A, 0xBB, 0x8C, 0x5E, 0x07, 0x9E, 0x09, 0xE2, 0xC8, 0xA8, 0x33, 0x9C,
];

fn fail(alert: u8, why: &'static str) -> TlsError {
    TlsError::Alert(alert, why)
}

// ------------------------------------------------------------------ parsing helpers

struct R<'a> {
    b: &'a [u8],
}

impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], TlsError> {
        if self.b.len() < n {
            return Err(fail(AL_DECODE_ERROR, "message too short"));
        }
        let (a, rest) = self.b.split_at(n);
        self.b = rest;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, TlsError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, TlsError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u24(&mut self) -> Result<usize, TlsError> {
        let b = self.take(3)?;
        Ok((b[0] as usize) << 16 | (b[1] as usize) << 8 | b[2] as usize)
    }
    fn v8(&mut self) -> Result<&'a [u8], TlsError> {
        let n = self.u8()? as usize;
        self.take(n)
    }
    fn v16(&mut self) -> Result<&'a [u8], TlsError> {
        let n = self.u16()? as usize;
        self.take(n)
    }
    fn v24(&mut self) -> Result<&'a [u8], TlsError> {
        let n = self.u24()?;
        self.take(n)
    }
    fn done(&self) -> Result<(), TlsError> {
        if self.b.is_empty() { Ok(()) } else { Err(fail(AL_DECODE_ERROR, "trailing bytes")) }
    }
}

/// the body of an extension that is one vector with a 16-bit (8-bit) length: nothing may follow
/// it (decode_error)
fn whole16(b: &[u8]) -> Result<&[u8], TlsError> {
    let mut r = R { b };
    let v = r.v16()?;
    r.done()?;
    Ok(v)
}

fn whole8(b: &[u8]) -> Result<&[u8], TlsError> {
    let mut r = R { b };
    let v = r.v8()?;
    r.done()?;
    Ok(v)
}

/// extensions -> (type, data), checked for duplicates
fn extensions(b: &[u8]) -> Result<Vec<(u16, &[u8])>, TlsError> {
    let mut r = R { b };
    let mut out: Vec<(u16, &[u8])> = Vec::new();
    while !r.b.is_empty() {
        let t = r.u16()?;
        let d = r.v16()?;
        if out.iter().any(|(x, _)| *x == t) {
            return Err(fail(AL_ILLEGAL_PARAMETER, "duplicate extension"));
        }
        out.push((t, d));
    }
    Ok(out)
}

fn ext<'a>(list: &[(u16, &'a [u8])], t: u16) -> Option<&'a [u8]> {
    list.iter().find(|(x, _)| *x == t).map(|(_, d)| *d)
}

fn u16_list(b: &[u8]) -> Result<Vec<u16>, TlsError> {
    if b.len() % 2 != 0 {
        return Err(fail(AL_DECODE_ERROR, "odd list"));
    }
    Ok(b.chunks(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect())
}

// ------------------------------------------------------------------ building helpers

fn put16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_v8(out: &mut Vec<u8>, b: &[u8]) {
    out.push(b.len() as u8);
    out.extend_from_slice(b);
}

fn put_v16(out: &mut Vec<u8>, b: &[u8]) {
    put16(out, b.len() as u16);
    out.extend_from_slice(b);
}

fn put_v24(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&[(b.len() >> 16) as u8, (b.len() >> 8) as u8, b.len() as u8]);
    out.extend_from_slice(b);
}

fn put_ext(out: &mut Vec<u8>, t: u16, d: &[u8]) {
    put16(out, t);
    put_v16(out, d);
}

fn u16s(list: &[u16]) -> Vec<u8> {
    let mut b = Vec::new();
    for v in list {
        put16(&mut b, *v);
    }
    b
}

fn hs_msg(ty: u8, body: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(4 + body.len());
    m.push(ty);
    put_v24(&mut m, body);
    m
}

fn cv_content(server: bool, hash: &[u8]) -> Vec<u8> {
    let mut c = Vec::from(&[0x20u8; 64][..]);
    c.extend_from_slice(if server { b"TLS 1.3, server CertificateVerify" as &[u8] } else { b"TLS 1.3, client CertificateVerify" });
    c.push(0);
    c.extend_from_slice(hash);
    c
}

/// a signature scheme of the handshake -> the algorithm, when the key can make it (TLS 1.3:
/// ECDSA names its curve, PKCS #1 v1.5 is not allowed)
fn scheme(s: u16, key: &PubKey, v12: bool) -> Option<SigAlg> {
    let alg = match s {
        ED25519 => SigAlg::Ed25519,
        0x0403 => SigAlg::Ecdsa(HashAlg::Sha256),
        0x0503 => SigAlg::Ecdsa(HashAlg::Sha384),
        0x0603 => SigAlg::Ecdsa(HashAlg::Sha512),
        0x0804 => SigAlg::RsaPss(HashAlg::Sha256, Some(32)),
        0x0805 => SigAlg::RsaPss(HashAlg::Sha384, Some(48)),
        0x0806 => SigAlg::RsaPss(HashAlg::Sha512, Some(64)),
        0x0401 if v12 => SigAlg::RsaPkcs1(HashAlg::Sha256),
        0x0501 if v12 => SigAlg::RsaPkcs1(HashAlg::Sha384),
        0x0601 if v12 => SigAlg::RsaPkcs1(HashAlg::Sha512),
        _ => return None,
    };
    let fits = match (alg, key) {
        (SigAlg::Ed25519, PubKey::Ed25519(_)) => true,
        (SigAlg::Ecdsa(HashAlg::Sha256), PubKey::Ec { curve, .. }) => v12 || *curve == Curve::P256,
        (SigAlg::Ecdsa(HashAlg::Sha384), PubKey::Ec { curve, .. }) => v12 || *curve == Curve::P384,
        (SigAlg::Ecdsa(_), PubKey::Ec { .. }) => v12,
        (SigAlg::RsaPss(..) | SigAlg::RsaPkcs1(_), PubKey::Rsa { .. }) => true,
        _ => false,
    };
    fits.then_some(alg)
}

/// the signature scheme our key signs the handshake with: the first of the peer's list it makes
/// (TLS 1.3: ECDSA with the hash of its curve, RSA with PSS)
fn our_scheme(key: &PrivateKey, offered: &[u16]) -> Option<(u16, SigAlg)> {
    let pk = key.public_key();
    offered.iter().find_map(|&s| {
        let alg = scheme(s, &pk, false)?;
        (WEB_SIG_ALGS.contains(&s) && key.can(alg)).then_some((s, alg))
    })
}

/// our CertificateVerify over the transcript hash, in a scheme of the peer's list
fn certificate_verify(key: &PrivateKey, offered: &[u16], server: bool, hash: &[u8]) -> Result<Vec<u8>, TlsError> {
    let (sch, alg) = our_scheme(key, offered).ok_or(fail(AL_HANDSHAKE_FAILURE, "no signature scheme in common"))?;
    let random: [u8; 64] = plat::random_bytes();
    let sig = key.sign(alg, &cv_content(server, hash), &random).ok_or(fail(AL_INTERNAL_ERROR, "the key does not sign"))?;
    let mut b = Vec::new();
    put16(&mut b, sch);
    put_v16(&mut b, &sig);
    Ok(hs_msg(HS_CERTIFICATE_VERIFY, &b))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    ClientSh,
    ClientEe,
    ClientCertOrCr,
    ClientCert,
    ClientCv,
    ClientFinished,
    // TLS 1.2 (client only)
    Client12Cert,
    Client12Ske,
    Client12Done,
    Client12Ccs,
    Client12Fin,
    ServerCh,
    ServerClientCert,
    ServerClientCv,
    ServerFinished,
    Established,
    Closed,
}

pub struct Conn {
    server: bool,
    state: State,
    transcript: Transcript,
    /// the hash of the suite (SHA-256 until a suite is chosen)
    hash_alg: HashAlg,
    inbuf: Vec<u8>,
    hs: Vec<u8>,
    out: Vec<u8>,
    plain: Vec<u8>,
    rd: Option<Cipher>,
    wr: Option<Cipher>,
    c_hs: Secret,
    s_hs: Secret,
    c_ap: Secret,
    s_ap: Secret,
    master: Secret,
    /// a client's key shares (two in its first ClientHello, one after a HelloRetryRequest; the
    /// TLS 1.2 client's ECDHE key)
    shares: Vec<Share>,
    session_id: Vec<u8>,
    cookie: Vec<u8>,
    /// client: the extensions of its last ClientHello, the only ones the server may answer
    sent_exts: Vec<u16>,
    hrr: bool,
    cert_requested: bool,
    peer: Vec<Cert>,
    ccfg: Option<ClientConfig>,
    scfg: Option<ServerConfig>,
    /// the peer sent close_notify
    peer_closed: bool,
    /// the negotiated suite and version
    suite: u16,
    v12: bool,
    /// a client trusting the system CAs: web suites, TLS 1.2 fallback
    web: bool,
    client_random: [u8; 32],
    server_random: [u8; 32],
    ems: bool,
    /// KeyUpdate messages received (a flood is answered with an alert)
    key_updates: u32,
    /// a client: the signature schemes of the server's CertificateRequest; a server: the ones
    /// it lists there (empty: every scheme it takes; other lists only in tests)
    cr_algs: Vec<u16>,
    /// TLS 1.2: the ECDHE secret, the master secret, the server's keys until its CCS
    pms: Option<Vec<u8>>,
    master12: [u8; 48],
    rd_next: Option<Cipher>,
    /// counts the changes of the read key (set_rd): handshake data must not cross one
    rd_epoch: u32,
    /// the peer's dummy change_cipher_spec of middlebox compatibility came (one is allowed)
    ccs_seen: bool,
    /// a server whose client offered early data: the bytes of 0-RTT records it may still skip
    early_skip: usize,
    /// a server after its HelloRetryRequest: the first ClientHello without what the second may
    /// change (RFC 8446 4.1.2)
    ch1: Option<Vec<u8>>,
    /// the negotiated group (a client: the one a HelloRetryRequest asked for)
    group: u16,
    /// the application protocol agreed on (ALPN)
    alpn: Option<Vec<u8>>,
    /// a server: the name the client asked for (SNI), and the identity it shows
    sni: String,
    identity: Option<Identity>,
}

// the secrets of the key schedule are wiped when the connection goes
impl Drop for Conn {
    fn drop(&mut self) {
        for s in [&mut self.c_hs, &mut self.s_hs, &mut self.c_ap, &mut self.s_ap, &mut self.master] {
            crypto::zeroize(s);
        }
        crypto::zeroize(&mut self.master12);
        if let Some(p) = self.pms.as_mut() {
            crypto::zeroize(p);
        }
    }
}

impl Conn {
    fn new(server: bool) -> Conn {
        Conn {
            server,
            state: if server { State::ServerCh } else { State::ClientSh },
            transcript: Transcript::new(),
            hash_alg: HashAlg::Sha256,
            inbuf: Vec::new(),
            hs: Vec::new(),
            out: Vec::new(),
            plain: Vec::new(),
            rd: None,
            wr: None,
            c_hs: Vec::new(),
            s_hs: Vec::new(),
            c_ap: Vec::new(),
            s_ap: Vec::new(),
            master: Vec::new(),
            shares: Vec::new(),
            session_id: Vec::new(),
            cookie: Vec::new(),
            sent_exts: Vec::new(),
            hrr: false,
            cert_requested: false,
            peer: Vec::new(),
            ccfg: None,
            scfg: None,
            peer_closed: false,
            suite: TLS_CHACHA20_POLY1305_SHA256,
            v12: false,
            web: false,
            client_random: [0; 32],
            server_random: [0; 32],
            ems: false,
            key_updates: 0,
            cr_algs: Vec::new(),
            pms: None,
            master12: [0; 48],
            rd_next: None,
            rd_epoch: 0,
            ccs_seen: false,
            early_skip: 0,
            ch1: None,
            group: 0,
            alpn: None,
            sni: String::new(),
            identity: None,
        }
    }

    /// the key exchange group
    pub fn group(&self) -> u16 {
        self.group
    }

    /// the application protocol agreed on (ALPN; one that is not UTF-8 gives None)
    pub fn alpn(&self) -> Option<&str> {
        self.alpn.as_deref().and_then(|a| core::str::from_utf8(a).ok())
    }

    /// a server: the name of the client's server_name (lower case)
    pub fn server_name(&self) -> &str {
        &self.sni
    }

    /// the suite is chosen: its hash from here on
    fn set_suite(&mut self, suite: u16) {
        self.suite = suite;
        self.hash_alg = record::suite_hash(suite);
        self.transcript.keep(self.hash_alg);
    }

    pub fn client(cfg: ClientConfig) -> Conn {
        let mut c = Conn::new(false);
        c.session_id = Vec::from(&plat::random_bytes::<32>()[..]);
        c.client_random = plat::random_bytes();
        c.web = matches!(cfg.trust, Trust::System(_) | Trust::Custom(_));
        c.ccfg = Some(cfg);
        // a ClientHello too long to send (a server name of tens of KiB): the connection is
        // closed before it begins
        if c.send_client_hello().is_err() {
            c.state = State::Closed;
        }
        c
    }

    pub fn server(cfg: ServerConfig) -> Conn {
        let mut c = Conn::new(true);
        c.scfg = Some(cfg);
        c
    }

    pub fn established(&self) -> bool {
        self.state == State::Established
    }

    pub fn closed(&self) -> bool {
        self.state == State::Closed || self.peer_closed
    }

    /// the peer sent close_notify (an orderly end, not a cut connection)
    pub fn peer_closed(&self) -> bool {
        self.peer_closed
    }

    /// the peer's certificate chain (leaf first)
    pub fn peer_certs(&self) -> &[Cert] {
        &self.peer
    }

    /// "TLS 1.3" / "TLS 1.2"
    pub fn version(&self) -> &'static str {
        if self.v12 { "TLS 1.2" } else { "TLS 1.3" }
    }

    /// the negotiated cipher suite
    pub fn suite(&self) -> u16 {
        self.suite
    }

    pub fn take_out(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.out)
    }

    pub fn has_plain(&self) -> bool {
        !self.plain.is_empty()
    }

    pub fn read_plain(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.plain.len());
        buf[..n].copy_from_slice(&self.plain[..n]);
        self.plain.drain(..n);
        n
    }

    pub fn write_plain(&mut self, data: &[u8]) -> Result<(), TlsError> {
        if self.state != State::Established {
            return Err(TlsError::Closed);
        }
        for chunk in data.chunks(MAX_PLAIN) {
            let rec = self.wr.as_mut().ok_or(TlsError::Closed)?.seal(APPLICATION_DATA, chunk);
            self.out.extend_from_slice(&rec);
        }
        Ok(())
    }

    /// (tests) the signature schemes a server's CertificateRequest lists (empty: all it takes)
    #[doc(hidden)]
    pub fn test_cr_sig_algs(&mut self, algs: Vec<u16>) {
        self.cr_algs = algs;
    }

    /// (tests) an application data record without data
    pub fn write_empty(&mut self) -> Result<(), TlsError> {
        if self.state != State::Established {
            return Err(TlsError::Closed);
        }
        let rec = self.wr.as_mut().ok_or(TlsError::Closed)?.seal(APPLICATION_DATA, &[]);
        self.out.extend_from_slice(&rec);
        Ok(())
    }

    /// send close_notify
    pub fn close(&mut self) {
        if self.state != State::Closed {
            self.alert(AL_CLOSE_NOTIFY, false);
            self.state = State::Closed;
        }
    }

    fn alert(&mut self, desc: u8, fatal: bool) {
        let body = [if fatal { 2 } else { 1 }, desc];
        let rec = match self.wr.as_mut() {
            Some(c) => c.seal(ALERT, &body),
            None => record::plain(ALERT, &body),
        };
        self.out.extend_from_slice(&rec);
    }

    fn hash(&self) -> Vec<u8> {
        self.transcript.hash(self.hash_alg)
    }

    fn add(&mut self, msg: &[u8]) {
        self.transcript.update(msg);
    }

    /// queue a handshake message (protected when write keys exist)
    fn send_hs(&mut self, msg: &[u8]) {
        self.add(msg);
        // a message longer than a record (a long certificate chain, a large ClientHello) goes in
        // several (RFC 8446 5.1); the transcript has it once, whole
        for part in msg.chunks(MAX_PLAIN) {
            let rec = match self.wr.as_mut() {
                Some(c) => c.seal(HANDSHAKE, part),
                None => record::plain(HANDSHAKE, part),
            };
            self.out.extend_from_slice(&rec);
        }
    }

    /// the TLS 1.3 record keys of a traffic secret for the negotiated suite
    fn cipher13(&self, secret: &Secret) -> Result<Cipher, TlsError> {
        let (kl, _) = record::key_iv_len(self.suite).ok_or(fail(AL_INTERNAL_ERROR, "suite"))?;
        let (k, iv) = keysched::traffic_keys_len(self.hash_alg, secret, kl);
        Cipher::for_suite(self.suite, &k, &iv, false).ok_or(fail(AL_INTERNAL_ERROR, "suite"))
    }

    /// the new read key (every change counts, see handshake_bytes)
    fn set_rd(&mut self, c: Option<Cipher>) {
        self.rd = c;
        self.rd_epoch = self.rd_epoch.wrapping_add(1);
    }

    /// bytes from the peer
    pub fn feed(&mut self, data: &[u8]) -> Result<(), TlsError> {
        if self.state == State::Closed {
            return Err(TlsError::Closed);
        }
        self.inbuf.extend_from_slice(data);
        match self.records() {
            Ok(()) => Ok(()),
            Err(e) => {
                if let TlsError::Alert(a, _) = e {
                    self.alert(a, true);
                }
                self.state = State::Closed;
                Err(e)
            }
        }
    }

    fn records(&mut self) -> Result<(), TlsError> {
        loop {
            if self.peer_closed {
                self.inbuf.clear();
                return Ok(());
            }
            if self.inbuf.len() < 5 {
                return Ok(());
            }
            let ty = self.inbuf[0];
            let len = (self.inbuf[3] as usize) << 8 | self.inbuf[4] as usize;
            if len > MAX_RECORD {
                return Err(fail(AL_RECORD_OVERFLOW, "record too long"));
            }
            if self.inbuf[1] != 3 {
                return Err(fail(AL_PROTOCOL_VERSION, "not a TLS record"));
            }
            // a record that is not protected holds at most 2^14 bytes (RFC 8446 5.1)
            let protected = self.rd.is_some() && (ty == APPLICATION_DATA || self.v12) && ty != CHANGE_CIPHER_SPEC;
            if !protected && len > MAX_PLAIN {
                return Err(fail(AL_RECORD_OVERFLOW, "plaintext record too long"));
            }
            if self.inbuf.len() < 5 + len {
                return Ok(());
            }
            let hdr = [self.inbuf[0], self.inbuf[1], self.inbuf[2], self.inbuf[3], self.inbuf[4]];
            let body: Vec<u8> = self.inbuf[5..5 + len].to_vec();
            self.inbuf.drain(..5 + len);
            match ty {
                CHANGE_CIPHER_SPEC if self.v12 => {
                    // TLS 1.2: the server's records are protected from here on
                    self.whole_messages()?;
                    if body != [1] || self.state != State::Client12Ccs {
                        return Err(fail(AL_UNEXPECTED_MESSAGE, "change_cipher_spec"));
                    }
                    let next = self.rd_next.take();
                    self.set_rd(next);
                    self.state = State::Client12Fin;
                }
                CHANGE_CIPHER_SPEC => {
                    // compatibility mode (RFC 8446 D.4): one byte 1, during the handshake; a server
                    // takes one from its client, not before the first ClientHello (a client stays
                    // lenient with servers)
                    self.whole_messages()?;
                    let again = self.server && (self.ccs_seen || (self.state == State::ServerCh && !self.hrr));
                    if body != [1] || self.state == State::Established || again {
                        return Err(fail(AL_UNEXPECTED_MESSAGE, "change_cipher_spec"));
                    }
                    self.ccs_seen = true;
                }
                _ if self.v12 && self.rd.is_some() => {
                    let rd = self.rd.as_mut().ok_or(fail(AL_INTERNAL_ERROR, "keys"))?;
                    let (inner, content) = rd.open(&hdr, &body).map_err(open_failure)?;
                    match inner {
                        HANDSHAKE => self.handshake_bytes(&content)?,
                        ALERT => self.peer_alert(&content)?,
                        APPLICATION_DATA if self.state == State::Established => {
                            self.whole_messages()?;
                            self.plain.extend_from_slice(&content)
                        }
                        _ => return Err(fail(AL_UNEXPECTED_MESSAGE, "record type")),
                    }
                }
                // 0-RTT data the server does not take (RFC 8446 4.2.10): before its keys (after a
                // HelloRetryRequest) every application_data record, after them those that do not
                // open with the client's handshake key, up to MAX_EARLY_SKIP bytes
                APPLICATION_DATA if self.skips_early(body.len()) && self.rd.is_none() => {
                    self.early_skip -= body.len();
                }
                APPLICATION_DATA => {
                    let rd = self.rd.as_mut().ok_or(fail(AL_UNEXPECTED_MESSAGE, "data before keys"))?;
                    let opened = rd.open(&hdr, &body);
                    if self.skips_early(body.len()) && opened == Err(record::OpenError::Mac) {
                        self.early_skip -= body.len();
                        continue;
                    }
                    let (inner, content) = opened.map_err(open_failure)?;
                    // the first record that opens is the client's second flight: no more skipping
                    self.early_skip = 0;
                    match inner {
                        HANDSHAKE => self.handshake_bytes(&content)?,
                        ALERT => self.peer_alert(&content)?,
                        APPLICATION_DATA => {
                            if self.state != State::Established {
                                return Err(fail(AL_UNEXPECTED_MESSAGE, "data during the handshake"));
                            }
                            self.whole_messages()?;
                            self.plain.extend_from_slice(&content);
                        }
                        _ => return Err(fail(AL_UNEXPECTED_MESSAGE, "inner type")),
                    }
                }
                HANDSHAKE => {
                    if self.rd.is_some() {
                        return Err(fail(AL_UNEXPECTED_MESSAGE, "plaintext handshake after keys"));
                    }
                    self.handshake_bytes(&body)?;
                }
                // a peer that fails during the handshake may send its alert in plaintext (it may
                // not have the keys, or not trust them); it ends the connection either way, as a
                // TCP reset of an attacker could
                ALERT if self.rd.is_none() || self.state != State::Established => self.peer_alert(&body)?,
                _ => return Err(fail(AL_UNEXPECTED_MESSAGE, "record type")),
            }
            if self.state == State::Closed {
                return Ok(());
            }
        }
    }

    /// a record of len bytes is 0-RTT data to skip: the client offered early data, a ClientHello
    /// came whole (early data follows it), and len fits what is left to skip
    fn skips_early(&self, len: usize) -> bool {
        self.server && self.early_skip > 0 && len <= self.early_skip && self.hs.is_empty()
    }

    /// a handshake message split over records has no other records between its parts (RFC 8446
    /// 5.1): application data or a change_cipher_spec while one is incomplete (a KeyUpdate whose
    /// first bytes came) is refused; an alert ends the connection anyway, with the peer's reason
    fn whole_messages(&self) -> Result<(), TlsError> {
        if self.hs.is_empty() {
            Ok(())
        } else {
            Err(fail(AL_UNEXPECTED_MESSAGE, "a record inside a fragmented handshake message"))
        }
    }

    fn peer_alert(&mut self, b: &[u8]) -> Result<(), TlsError> {
        if b.is_empty() {
            return Err(fail(AL_UNEXPECTED_MESSAGE, "empty alert record")); // RFC 8446 5.4
        }
        if b.len() != 2 {
            return Err(fail(AL_DECODE_ERROR, "alert"));
        }
        if b[1] == AL_CLOSE_NOTIFY {
            self.peer_closed = true;
            // TLS 1.2: the other side answers with its own and closes (RFC 5246 7.2.1); TLS 1.3
            // leaves the write side open until the program closes it (RFC 8446 6.1)
            if self.v12 {
                self.close();
            }
            return Ok(());
        }
        if b[0] == 1 && self.v12 {
            return Ok(()); // a TLS 1.2 warning (no_renegotiation, ...)
        }
        self.state = State::Closed;
        Err(TlsError::PeerAlert(b[1]))
    }

    fn handshake_bytes(&mut self, b: &[u8]) -> Result<(), TlsError> {
        // a handshake record holds part of a message (RFC 8446 5.1, RFC 5246 6.2.1), protected or not
        if b.is_empty() {
            return Err(fail(AL_UNEXPECTED_MESSAGE, "empty handshake record"));
        }
        self.hs.extend_from_slice(b);
        loop {
            if self.hs.len() < 4 {
                return Ok(());
            }
            let n = (self.hs[1] as usize) << 16 | (self.hs[2] as usize) << 8 | self.hs[3] as usize;
            // 128 KiB: a ClientHello may list 32 767 signature schemes (RFC 8446 4.2.3; OpenSSL
            // takes 131 396 bytes)
            if n > MAX_HANDSHAKE {
                return Err(fail(AL_DECODE_ERROR, "handshake message too long"));
            }
            if self.hs.len() < 4 + n {
                return Ok(());
            }
            let msg: Vec<u8> = self.hs[..4 + n].to_vec();
            self.hs.drain(..4 + n);
            let epoch = self.rd_epoch;
            self.message(&msg)?;
            // a key change must fall on a record boundary (RFC 8446 5.1): the first keys, the
            // application keys after Finished and every KeyUpdate
            if self.rd_epoch != epoch && !self.hs.is_empty() {
                return Err(fail(AL_UNEXPECTED_MESSAGE, "handshake data across a key change"));
            }
        }
    }

    fn message(&mut self, msg: &[u8]) -> Result<(), TlsError> {
        let ty = msg[0];
        let body = &msg[4..];
        match (self.state, ty) {
            (State::ServerCh, HS_CLIENT_HELLO) => self.server_client_hello(msg, body),
            (State::ClientSh, HS_SERVER_HELLO) => self.client_server_hello(msg, body),
            (State::ClientEe, HS_ENCRYPTED_EXTENSIONS) => {
                let mut r = R { b: body };
                let exts = extensions(r.v16()?)?;
                r.done()?;
                // server_name (empty: the server used the name), the server's supported_groups, ALPN
                self.server_exts(&exts, &[EXT_SERVER_NAME, EXT_GROUPS, EXT_ALPN])?;
                self.server_alpn(&exts)?;
                self.add(msg);
                self.state = State::ClientCertOrCr;
                Ok(())
            }
            (State::ClientCertOrCr, HS_CERTIFICATE_REQUEST) => {
                let mut r = R { b: body };
                if !r.v8()?.is_empty() {
                    return Err(fail(AL_ILLEGAL_PARAMETER, "certificate request context"));
                }
                let exts = extensions(r.v16()?)?;
                r.done()?;
                let algs = u16_list(whole16(ext(&exts, EXT_SIG_ALGS).ok_or(fail(AL_MISSING_EXTENSION, "signature_algorithms"))?)?)?;
                if algs.is_empty() {
                    return Err(fail(AL_DECODE_ERROR, "an empty signature_algorithms list"));
                }
                // an identity whose key signs in none of these schemes is not shown: the Certificate
                // is empty (RFC 8446 4.4.2), and a server that requires one refuses that
                self.cr_algs = algs;
                self.cert_requested = true;
                self.add(msg);
                self.state = State::ClientCert;
                Ok(())
            }
            (State::ClientCertOrCr | State::ClientCert, HS_CERTIFICATE) => {
                self.peer = parse_certificates(body)?;
                if self.peer.is_empty() {
                    return Err(fail(AL_DECODE_ERROR, "the server sent no certificate"));
                }
                self.check_server_chain()?;
                self.add(msg);
                self.state = State::ClientCv;
                Ok(())
            }
            (State::ClientCv, HS_CERTIFICATE_VERIFY) | (State::ServerClientCv, HS_CERTIFICATE_VERIFY) => {
                let server = self.state == State::ClientCv;
                let hash = self.hash();
                verify_cv(body, &self.peer[0], server, &hash)?;
                self.add(msg);
                self.state = if server { State::ClientFinished } else { State::ServerFinished };
                Ok(())
            }
            (State::ClientFinished, HS_FINISHED) => self.client_server_finished(msg, body),
            (State::ServerClientCert, HS_CERTIFICATE) => {
                let certs = parse_certificates(body)?;
                self.add(msg);
                if certs.is_empty() {
                    if self.scfg.as_ref().is_some_and(|c| c.client_auth_optional) {
                        // no certificate, no CertificateVerify: Finished comes next
                        self.state = State::ServerFinished;
                        return Ok(());
                    }
                    return Err(fail(AL_CERTIFICATE_REQUIRED, "a client certificate is required"));
                }
                self.peer = certs;
                self.check_client_chain()?;
                self.state = State::ServerClientCv;
                Ok(())
            }
            (State::ServerFinished, HS_FINISHED) => {
                let want = keysched::finished(self.hash_alg, &self.c_hs, &self.hash());
                finished_len(body, want.len())?;
                if !crypto::ct_eq(&want, body) {
                    return Err(fail(AL_DECRYPT_ERROR, "client Finished"));
                }
                self.add(msg);
                let c = self.cipher13(&self.c_ap)?;
                self.set_rd(Some(c));
                self.state = State::Established;
                Ok(())
            }
            // ------------------------------------------------ TLS 1.2 (client)
            (State::Client12Cert, HS_CERTIFICATE) => {
                self.peer = parse_certificates12(body)?;
                if self.peer.is_empty() {
                    return Err(fail(AL_DECODE_ERROR, "the server sent no certificate"));
                }
                self.check_server_chain()?;
                self.add(msg);
                self.state = State::Client12Ske;
                Ok(())
            }
            (State::Client12Ske, HS_SERVER_KEY_EXCHANGE) => {
                self.server_key_exchange(body)?;
                self.add(msg);
                self.state = State::Client12Done;
                Ok(())
            }
            (State::Client12Done, HS_CERTIFICATE_REQUEST) => {
                // certificate types, signature algorithms, CA names: we answer with no certificate
                let mut r = R { b: body };
                r.v8()?;
                r.v16()?;
                r.v16()?;
                r.done()?;
                self.cert_requested = true;
                self.add(msg);
                Ok(())
            }
            (State::Client12Done, HS_SERVER_HELLO_DONE) => {
                if !body.is_empty() {
                    return Err(fail(AL_DECODE_ERROR, "server_hello_done"));
                }
                self.add(msg);
                self.client_finish12()
            }
            (State::Client12Fin, HS_FINISHED) => {
                let mut want = [0u8; 12];
                keysched::prf12(self.hash_alg, &self.master12, "server finished", &self.hash(), &mut want);
                finished_len(body, want.len())?;
                if !crypto::ct_eq(&want, body) {
                    return Err(fail(AL_DECRYPT_ERROR, "server Finished"));
                }
                self.add(msg);
                self.state = State::Established;
                Ok(())
            }
            (State::Established, HS_HELLO_REQUEST) if self.v12 && body.is_empty() => {
                self.alert(AL_NO_RENEGOTIATION, false);
                Ok(())
            }
            (State::Established, HS_NEW_SESSION_TICKET) if !self.server => Ok(()), // no resumption
            (State::Established, HS_KEY_UPDATE) if !self.v12 => {
                let mut r = R { b: body };
                let requested = r.u8()?;
                r.done()?;
                if requested > 1 {
                    return Err(fail(AL_ILLEGAL_PARAMETER, "key_update"));
                }
                self.key_updates += 1;
                if self.key_updates > 1024 {
                    return Err(fail(AL_UNEXPECTED_MESSAGE, "too many key updates"));
                }
                // the next secret replaces the old one, which is wiped
                let h = self.hash_alg;
                let next = keysched::update(h, if self.server { &self.c_ap } else { &self.s_ap });
                let c = self.cipher13(&next)?;
                let peer_secret = if self.server { &mut self.c_ap } else { &mut self.s_ap };
                crypto::zeroize(peer_secret);
                *peer_secret = next;
                self.set_rd(Some(c));
                if requested == 1 {
                    // our answer still goes under the old write key
                    let msg = hs_msg(HS_KEY_UPDATE, &[0]);
                    let rec = self.wr.as_mut().ok_or(TlsError::Closed)?.seal(HANDSHAKE, &msg);
                    self.out.extend_from_slice(&rec);
                    let next = keysched::update(h, if self.server { &self.s_ap } else { &self.c_ap });
                    let c = self.cipher13(&next)?;
                    let my_secret = if self.server { &mut self.s_ap } else { &mut self.c_ap };
                    crypto::zeroize(my_secret);
                    *my_secret = next;
                    self.wr = Some(c);
                }
                Ok(())
            }
            _ => Err(fail(AL_UNEXPECTED_MESSAGE, "handshake message out of order")),
        }
    }

    // ------------------------------------------------------------------ client

    fn send_client_hello(&mut self) -> Result<(), TlsError> {
        let cfg = self.ccfg.as_ref().map(|c| c.server_name.clone()).unwrap_or_default();
        let mut b = Vec::new();
        put16(&mut b, 0x0303);
        b.extend_from_slice(&self.client_random);
        put_v8(&mut b, &self.session_id);
        if self.web {
            put_v16(&mut b, &u16s(&[&SUITES_13[..], &SUITES_12[..]].concat()));
        } else {
            put_v16(&mut b, &u16s(&SUITES_13));
        }
        put_v8(&mut b, &[0]);
        let mut e = Vec::new();
        if !cfg.is_empty() && plat::net::IpAddr::parse(&cfg).is_none() {
            let mut sni = Vec::new();
            sni.push(0); // host_name
            put_v16(&mut sni, cfg.as_bytes());
            let mut list = Vec::new();
            put_v16(&mut list, &sni);
            put_ext(&mut e, EXT_SERVER_NAME, &list);
        }
        // ALPN: the names of 1 to 255 bytes, as many as fit in MAX_ALPN (ClientConfig::alpn)
        let alpn = self.ccfg.as_ref().map(|c| c.alpn.clone()).unwrap_or_default();
        let mut list = Vec::new();
        for p in alpn.iter().filter(|p| !p.is_empty() && p.len() < 256) {
            if list.len() + 1 + p.len() > MAX_ALPN {
                break;
            }
            put_v8(&mut list, p.as_bytes());
        }
        if !list.is_empty() {
            let mut v = Vec::new();
            put_v16(&mut v, &list);
            put_ext(&mut e, EXT_ALPN, &v);
        }
        if self.web {
            put_ext(&mut e, EXT_VERSIONS, &[4, 0x03, 0x04, 0x03, 0x03]);
            put_ext(&mut e, EXT_POINT_FORMATS, &[1, 0]);
            put_ext(&mut e, EXT_EMS, &[]);
            put_ext(&mut e, EXT_RENEGOTIATION_INFO, &[0]);
        } else {
            put_ext(&mut e, EXT_VERSIONS, &[2, 0x03, 0x04]);
        }
        // P-384 listed: TLS 1.2 servers with a P-384 certificate want to see the curve
        let mut groups = Vec::new();
        put_v16(&mut groups, &u16s(&CLIENT_GROUPS));
        put_ext(&mut e, EXT_GROUPS, &groups);
        let mut algs = Vec::new();
        put_v16(&mut algs, &u16s(&WEB_SIG_ALGS));
        put_ext(&mut e, EXT_SIG_ALGS, &algs);
        // shares for X25519MLKEM768 and X25519; after a HelloRetryRequest the one it asked for
        if self.shares.is_empty() {
            self.shares = [X25519_MLKEM768, X25519].iter().filter_map(|g| Share::new(*g)).collect();
        }
        let mut entries = Vec::new();
        for sh in &self.shares {
            put16(&mut entries, sh.group);
            put_v16(&mut entries, &sh.public);
        }
        let mut ks = Vec::new();
        put_v16(&mut ks, &entries);
        put_ext(&mut e, EXT_KEY_SHARE, &ks);
        if !self.cookie.is_empty() {
            let mut c = Vec::new();
            put_v16(&mut c, &self.cookie);
            put_ext(&mut e, EXT_COOKIE, &c);
        }
        // a length field that overflows makes the extensions pass 2^16 - 1 too (the ALPN list
        // is bounded, a share is short; a server name or cookie of tens of KiB is not)
        if e.len() > 0xFFFF {
            return Err(fail(AL_INTERNAL_ERROR, "the ClientHello's extensions are too long"));
        }
        self.sent_exts.clear();
        let mut i = 0;
        while i + 4 <= e.len() {
            self.sent_exts.push(u16::from_be_bytes([e[i], e[i + 1]]));
            i += 4 + (u16::from_be_bytes([e[i + 2], e[i + 3]]) as usize);
        }
        put_v16(&mut b, &e);
        let msg = hs_msg(HS_CLIENT_HELLO, &b);
        self.send_hs(&msg);
        Ok(())
    }

    /// what a server's message may hold (RFC 8446 4.2, RFC 5246 7.4.1.4): an extension the
    /// ClientHello did not offer is unsupported_extension (a HelloRetryRequest's cookie is the
    /// server's own), one it offered outside the messages that answer it illegal_parameter;
    /// unknown types and GREASE values (RFC 8701 4) are never offered
    fn server_exts(&self, exts: &[(u16, &[u8])], allowed: &[u16]) -> Result<(), TlsError> {
        for (t, _) in exts {
            if !self.sent_exts.contains(t) && *t != EXT_COOKIE {
                return Err(fail(AL_UNSUPPORTED_EXTENSION, "an extension the client did not offer"));
            }
            if !allowed.contains(t) {
                return Err(fail(AL_ILLEGAL_PARAMETER, "an extension in a message it does not belong in"));
            }
        }
        Ok(())
    }

    /// the server's ALPN answer (EncryptedExtensions, a TLS 1.2 ServerHello): exactly one
    /// protocol, one we offered (RFC 7301 3.1)
    fn server_alpn(&mut self, exts: &[(u16, &[u8])]) -> Result<(), TlsError> {
        let Some(a) = ext(exts, EXT_ALPN) else { return Ok(()) };
        let mut r = R { b: a };
        let mut list = R { b: r.v16()? };
        r.done()?;
        let p = list.v8()?;
        list.done()?;
        let offered = self.ccfg.as_ref().is_some_and(|c| c.alpn.iter().any(|o| o.as_bytes() == p));
        if p.is_empty() || !offered {
            return Err(fail(AL_ILLEGAL_PARAMETER, "an application protocol the client did not offer"));
        }
        self.alpn = Some(Vec::from(p));
        Ok(())
    }

    fn client_server_hello(&mut self, msg: &[u8], body: &[u8]) -> Result<(), TlsError> {
        let mut r = R { b: body };
        let legacy = r.u16()?;
        let random = r.take(32)?;
        let sid = r.v8()?;
        let suite = r.u16()?;
        if r.u8()? != 0 {
            return Err(fail(AL_ILLEGAL_PARAMETER, "compression"));
        }
        let exts = extensions(if r.b.is_empty() { &[] } else { r.v16()? })?;
        r.done()?;
        // with supported_versions the client ignores legacy_version (RFC 8446 4.2.1); without
        // it the ServerHello is TLS 1.2's, the oldest version offered
        let Some(v) = ext(&exts, EXT_VERSIONS) else {
            if legacy != 0x0303 {
                return Err(fail(AL_PROTOCOL_VERSION, "a version older than TLS 1.2"));
            }
            if self.web && !self.hrr && random != HRR_RANDOM {
                return self.server_hello12(msg, random, suite, &exts);
            }
            return Err(fail(AL_PROTOCOL_VERSION, "the server does not speak TLS 1.3"));
        };
        if sid != self.session_id.as_slice() {
            return Err(fail(AL_ILLEGAL_PARAMETER, "session id not echoed"));
        }
        if random == HRR_RANDOM {
            self.server_exts(&exts, &[EXT_VERSIONS, EXT_KEY_SHARE, EXT_COOKIE])?;
        } else {
            self.server_exts(&exts, &[EXT_VERSIONS, EXT_KEY_SHARE])?;
        }
        if !SUITES_13.contains(&suite) || (self.hrr && suite != self.suite) {
            return Err(fail(AL_ILLEGAL_PARAMETER, "cipher suite"));
        }
        self.set_suite(suite);
        if v != [0x03, 0x04] {
            return Err(fail(AL_ILLEGAL_PARAMETER, "supported_versions"));
        }
        if random == HRR_RANDOM {
            if self.hrr {
                return Err(fail(AL_UNEXPECTED_MESSAGE, "second HelloRetryRequest"));
            }
            let cookie = ext(&exts, EXT_COOKIE);
            // the group asked for must be one the client listed and sent no share for (4.1.4)
            match ext(&exts, EXT_KEY_SHARE) {
                Some(g) => {
                    let g = R { b: g }.u16().and_then(|v| if g.len() == 2 { Ok(v) } else { Err(fail(AL_DECODE_ERROR, "key_share")) })?;
                    if !CLIENT_GROUPS.contains(&g) || self.shares.iter().any(|s| s.group == g) {
                        return Err(fail(AL_ILLEGAL_PARAMETER, "HelloRetryRequest for a group not listed, or one with a share"));
                    }
                    self.group = g;
                    self.shares = Share::new(g).into_iter().collect();
                }
                None if cookie.is_none() => return Err(fail(AL_ILLEGAL_PARAMETER, "HelloRetryRequest without a change")),
                None => {}
            }
            if let Some(c) = cookie {
                self.cookie = Vec::from(whole16(c)?);
            }
            self.hrr = true;
            // transcript: message_hash(ClientHello1), HRR
            let h1 = self.hash();
            self.transcript = Transcript::new();
            self.transcript.keep(self.hash_alg);
            let mh = hs_msg(HS_MESSAGE_HASH, &h1);
            self.add(&mh);
            self.add(msg);
            return self.send_client_hello();
        }
        let ks = ext(&exts, EXT_KEY_SHARE).ok_or(fail(AL_MISSING_EXTENSION, "key_share"))?;
        let mut k = R { b: ks };
        let g = k.u16()?;
        let pk = k.v16()?;
        k.done()?;
        let share = self.shares.iter().find(|s| s.group == g).ok_or(fail(AL_ILLEGAL_PARAMETER, "key share group"))?;
        let mut shared = share.finish(pk).ok_or(fail(AL_ILLEGAL_PARAMETER, "key share"))?;
        self.group = g;
        self.shares.clear();
        self.add(msg);
        self.handshake_keys(&shared);
        crypto::zeroize(&mut shared);
        let c = self.cipher13(&self.s_hs)?;
        self.set_rd(Some(c));
        self.state = State::ClientEe;
        Ok(())
    }

    fn handshake_keys(&mut self, shared: &[u8]) {
        let a = self.hash_alg;
        let mut s = Schedule::new(a, None, shared);
        let h = self.hash();
        self.c_hs = keysched::derive(a, &s.handshake, "c hs traffic", &h);
        self.s_hs = keysched::derive(a, &s.handshake, "s hs traffic", &h);
        crypto::zeroize(&mut self.master);
        self.master = core::mem::take(&mut s.master);
    }

    fn app_secrets(&mut self) {
        let a = self.hash_alg;
        let h = self.hash();
        self.c_ap = keysched::derive(a, &self.master, "c ap traffic", &h);
        self.s_ap = keysched::derive(a, &self.master, "s ap traffic", &h);
    }

    fn check_server_chain(&mut self) -> Result<(), TlsError> {
        let cfg = self.ccfg.as_ref().ok_or(fail(AL_INTERNAL_ERROR, "config"))?;
        check_trust(&cfg.trust, &self.peer, Some(&cfg.server_name), Usage::Server)
    }

    fn check_client_chain(&mut self) -> Result<(), TlsError> {
        let trust = self.scfg.as_ref().and_then(|c| c.client_auth.as_ref()).ok_or(fail(AL_UNEXPECTED_MESSAGE, "certificate not requested"))?;
        check_trust(trust, &self.peer, None, Usage::Client)
    }

    fn client_server_finished(&mut self, msg: &[u8], body: &[u8]) -> Result<(), TlsError> {
        let want = keysched::finished(self.hash_alg, &self.s_hs, &self.hash());
        finished_len(body, want.len())?;
        if !crypto::ct_eq(&want, body) {
            return Err(fail(AL_DECRYPT_ERROR, "server Finished"));
        }
        self.add(msg);
        self.app_secrets();
        // our handshake messages go under the client handshake key
        self.wr = Some(self.cipher13(&self.c_hs)?);
        if self.cert_requested {
            let id = self.ccfg.as_ref().and_then(|c| c.identity.clone()).filter(|i| our_scheme(&i.key, &self.cr_algs).is_some());
            let chain: Vec<Vec<u8>> = id.as_ref().map(|i| i.chain.clone()).unwrap_or_default();
            let cm = hs_msg(HS_CERTIFICATE, &certificate_body(&chain));
            self.send_hs(&cm);
            if let Some(i) = id {
                if !chain.is_empty() {
                    let m = certificate_verify(&i.key, &self.cr_algs, false, &self.hash())?;
                    self.send_hs(&m);
                }
            }
        }
        let fin = keysched::finished(self.hash_alg, &self.c_hs, &self.hash());
        let m = hs_msg(HS_FINISHED, &fin);
        self.send_hs(&m);
        self.wr = Some(self.cipher13(&self.c_ap)?);
        let c = self.cipher13(&self.s_ap)?;
        self.set_rd(Some(c));
        self.state = State::Established;
        Ok(())
    }

    // ------------------------------------------------------------------ client, TLS 1.2

    fn server_hello12(&mut self, msg: &[u8], random: &[u8], suite: u16, exts: &[(u16, &[u8])]) -> Result<(), TlsError> {
        if random[24..31] == DOWNGRADE && (random[31] == 0 || random[31] == 1) {
            return Err(fail(AL_ILLEGAL_PARAMETER, "downgrade from TLS 1.3"));
        }
        if !SUITES_12.contains(&suite) {
            return Err(fail(AL_ILLEGAL_PARAMETER, "cipher suite"));
        }
        self.server_exts(exts, &[EXT_SERVER_NAME, EXT_POINT_FORMATS, EXT_EMS, EXT_RENEGOTIATION_INFO, EXT_ALPN])?;
        self.server_alpn(exts)?;
        if ext(exts, EXT_RENEGOTIATION_INFO).is_some_and(|r| r != [0]) {
            return Err(fail(AL_HANDSHAKE_FAILURE, "renegotiation_info"));
        }
        self.v12 = true;
        self.set_suite(suite);
        // the extended master secret when the server has it (RFC 7627); a server without it is
        // taken: the attack it closes (Triple Handshake) needs resumption or renegotiation,
        // which this client does neither of.  Resumption, once it comes, must require it
        self.ems = ext(exts, EXT_EMS).is_some();
        self.server_random.copy_from_slice(random);
        self.add(msg);
        self.state = State::Client12Cert;
        Ok(())
    }

    fn server_key_exchange(&mut self, body: &[u8]) -> Result<(), TlsError> {
        let mut r = R { b: body };
        let params_start = r.b;
        if r.u8()? != 3 {
            return Err(fail(AL_HANDSHAKE_FAILURE, "not a named curve"));
        }
        let group = r.u16()?;
        if ![X25519, SECP256R1, SECP384R1].contains(&group) {
            return Err(fail(AL_HANDSHAKE_FAILURE, "the server chose a curve other than X25519 / P-256 / P-384"));
        }
        let point = r.v8()?;
        let params = &params_start[..params_start.len() - r.b.len()];
        let sch = r.u16()?;
        let sig = r.v16()?;
        r.done()?;
        let leaf = &self.peer[0];
        if !WEB_SIG_ALGS.contains(&sch) {
            return Err(fail(AL_ILLEGAL_PARAMETER, "signature scheme"));
        }
        let alg = scheme(sch, &leaf.key, true).ok_or(fail(AL_ILLEGAL_PARAMETER, "signature scheme"))?;
        // ECDHE_ECDSA suites need an EC (or Ed25519, RFC 8422) key, ECDHE_RSA an RSA key
        let ecdsa_suite = matches!(self.suite, ECDHE_ECDSA_AES128_GCM | ECDHE_ECDSA_AES256_GCM | ECDHE_ECDSA_CHACHA20);
        if ecdsa_suite != matches!(leaf.key, PubKey::Ec { .. } | PubKey::Ed25519(_)) {
            return Err(fail(AL_UNSUPPORTED_CERTIFICATE, "key type does not fit the suite"));
        }
        let mut signed = Vec::with_capacity(64 + params.len());
        signed.extend_from_slice(&self.client_random);
        signed.extend_from_slice(&self.server_random);
        signed.extend_from_slice(params);
        if !leaf.verify(alg, &signed, sig) {
            return Err(fail(AL_DECRYPT_ERROR, "ServerKeyExchange signature"));
        }
        let share = Share::new(group).ok_or(fail(AL_INTERNAL_ERROR, "group"))?;
        self.pms = Some(share.finish(point).ok_or(fail(AL_ILLEGAL_PARAMETER, "key share"))?);
        self.shares = alloc::vec![share];
        self.group = group;
        Ok(())
    }

    fn client_finish12(&mut self) -> Result<(), TlsError> {
        if self.cert_requested {
            let cm = hs_msg(HS_CERTIFICATE, &[0, 0, 0]);
            self.send_hs(&cm);
        }
        let mut b = Vec::new();
        let share = self.shares.first().map(|s| s.public.clone()).ok_or(fail(AL_UNEXPECTED_MESSAGE, "no key exchange"))?;
        put_v8(&mut b, &share);
        let cke = hs_msg(HS_CLIENT_KEY_EXCHANGE, &b);
        self.send_hs(&cke);
        self.shares.clear();
        let mut pms = self.pms.take().ok_or(fail(AL_UNEXPECTED_MESSAGE, "no key exchange"))?;
        let h = self.hash_alg;
        if self.ems {
            keysched::prf12(h, &pms, "extended master secret", &self.hash(), &mut self.master12);
        } else {
            let seed = [self.client_random, self.server_random].concat();
            keysched::prf12(h, &pms, "master secret", &seed, &mut self.master12);
        }
        crypto::zeroize(&mut pms);
        let (kl, il) = record::key_iv_len(self.suite).ok_or(fail(AL_INTERNAL_ERROR, "suite"))?;
        let mut kb = alloc::vec![0u8; 2 * (kl + il)];
        let seed = [self.server_random, self.client_random].concat();
        keysched::prf12(h, &self.master12, "key expansion", &seed, &mut kb);
        let (ck, rest) = kb.split_at(kl);
        let (sk, rest) = rest.split_at(kl);
        let (civ, siv) = rest.split_at(il);
        let wr = Cipher::for_suite(self.suite, ck, civ, true).ok_or(fail(AL_INTERNAL_ERROR, "suite"))?;
        self.rd_next = Some(Cipher::for_suite(self.suite, sk, siv, true).ok_or(fail(AL_INTERNAL_ERROR, "suite"))?);
        crypto::zeroize(&mut kb);
        self.out.extend_from_slice(&record::plain(CHANGE_CIPHER_SPEC, &[1]));
        self.wr = Some(wr);
        let mut fin = [0u8; 12];
        keysched::prf12(h, &self.master12, "client finished", &self.hash(), &mut fin);
        let m = hs_msg(HS_FINISHED, &fin);
        self.send_hs(&m);
        self.state = State::Client12Ccs;
        Ok(())
    }

    // ------------------------------------------------------------------ server

    /// middlebox compatibility (RFC 8446 D.4): to a client that sent a legacy session id, a dummy
    /// change_cipher_spec right after the first handshake message (ServerHello or
    /// HelloRetryRequest), so that middleboxes see what looks like a resumed TLS 1.2 session
    fn compat_ccs(&mut self, sid: &[u8]) {
        if !sid.is_empty() {
            self.out.extend_from_slice(&record::plain(CHANGE_CIPHER_SPEC, &[1]));
        }
    }

    fn server_client_hello(&mut self, msg: &[u8], body: &[u8]) -> Result<(), TlsError> {
        let mut r = R { b: body };
        // legacy_version MUST be 0x0303 in a TLS 1.3 ClientHello (RFC 8446 4.1.2); an older one
        // (SSL 3.0, garbage) is refused
        if r.u16()? < 0x0303 {
            return Err(fail(AL_PROTOCOL_VERSION, "legacy_version below TLS 1.2"));
        }
        r.take(32)?;
        let sid = Vec::from(r.v8()?);
        if sid.len() > 32 {
            return Err(fail(AL_ILLEGAL_PARAMETER, "session id"));
        }
        let suites = u16_list(r.v16()?)?;
        let comp = r.v8()?;
        let exts = extensions(r.v16()?)?;
        r.done()?;
        if comp != [0] {
            return Err(fail(AL_ILLEGAL_PARAMETER, "compression"));
        }
        // pre_shared_key comes last (RFC 8446 4.2.11; the server checks it though it takes no PSK)
        if exts.iter().rev().skip(1).any(|(t, _)| *t == EXT_PSK) {
            return Err(fail(AL_ILLEGAL_PARAMETER, "pre_shared_key not the last extension"));
        }
        let versions = ext(&exts, EXT_VERSIONS).ok_or(fail(AL_PROTOCOL_VERSION, "TLS 1.3 required"))?;
        if !u16_list(whole8(versions)?)?.contains(&TLS13) {
            return Err(fail(AL_PROTOCOL_VERSION, "TLS 1.3 required"));
        }
        // the first of ours the client offers: ChaCha20-Poly1305, AES-128-GCM (which RFC 8446
        // 9.1 makes mandatory), AES-256-GCM with SHA-384
        let Some(suite) = SUITES_13.iter().copied().find(|s| suites.contains(s)) else {
            return Err(fail(AL_HANDSHAKE_FAILURE, "no common cipher suite (ChaCha20-Poly1305, AES-128-GCM, AES-256-GCM)"));
        };
        if self.hrr && suite != self.suite {
            return Err(fail(AL_ILLEGAL_PARAMETER, "another cipher suite after HelloRetryRequest"));
        }
        if !self.hrr {
            self.set_suite(suite);
        }
        let algs = u16_list(whole16(ext(&exts, EXT_SIG_ALGS).ok_or(fail(AL_MISSING_EXTENSION, "signature_algorithms"))?)?)?;
        // both lists hold at least one entry (RFC 8446 4.2.3, 4.2.7)
        if algs.is_empty() {
            return Err(fail(AL_DECODE_ERROR, "an empty signature_algorithms list"));
        }
        let sni = match ext(&exts, EXT_SERVER_NAME) {
            Some(b) => client_server_name(b)?,
            None => String::new(),
        };
        let offered_alpn = match ext(&exts, EXT_ALPN) {
            Some(b) => alpn_list(b)?,
            None => Vec::new(),
        };
        let cfg = self.scfg.as_ref().ok_or(fail(AL_INTERNAL_ERROR, "config"))?;
        // ALPN: the server's first protocol that the client offers (RFC 7301 3.2)
        let alpn = if cfg.alpn.is_empty() || offered_alpn.is_empty() {
            None
        } else {
            let p = cfg.alpn.iter().find(|p| offered_alpn.iter().any(|o| o.as_slice() == p.as_bytes()));
            Some(Vec::from(p.ok_or(fail(AL_NO_APPLICATION_PROTOCOL, "no application protocol in common"))?.as_bytes()))
        };
        let hello = Hello { server_name: sni.clone(), alpn: offered_alpn.clone(), sig_algs: algs.clone() };
        let identity = choose_identity(cfg, &hello);
        if our_scheme(&identity.key, &algs).is_none() {
            return Err(fail(AL_HANDSHAKE_FAILURE, "the client takes no signature our key makes"));
        }
        let groups = u16_list(whole16(ext(&exts, EXT_GROUPS).ok_or(fail(AL_MISSING_EXTENSION, "supported_groups"))?)?)?;
        if groups.is_empty() {
            return Err(fail(AL_DECODE_ERROR, "an empty supported_groups list"));
        }
        // early data is never taken (no tickets, no PSK): its records are skipped; a second
        // ClientHello must not offer it, and after it the client sends none (RFC 8446 4.2.10):
        // a record that does not open is an error again
        if self.hrr {
            self.early_skip = 0;
        }
        if ext(&exts, EXT_EARLY_DATA).is_some() {
            if self.hrr {
                return Err(fail(AL_ILLEGAL_PARAMETER, "early_data in the second ClientHello"));
            }
            self.early_skip = MAX_EARLY_SKIP;
        }
        // supported_groups and key_share come together (RFC 8446 9.2)
        if ext(&exts, EXT_KEY_SHARE).is_none() {
            return Err(fail(AL_MISSING_EXTENSION, "key_share"));
        }
        // the second ClientHello after a HelloRetryRequest is the first one, except for what
        // RFC 8446 4.1.2 lets it change (key_share, cookie, early_data, padding, and the binders
        // and ticket ages of pre_shared_key)
        let fixed = ch_fixed(&sid, &suites, comp, &exts);
        if self.hrr && self.ch1.as_ref() != Some(&fixed) {
            return Err(fail(AL_ILLEGAL_PARAMETER, "the second ClientHello changed"));
        }
        // the key shares: each for a group the client lists, at most one per group (RFC 8446
        // 4.2.8); the groups of kx (P-256, which 9.1 makes mandatory, among them)
        let mut shares: Vec<(u16, &[u8])> = Vec::new();
        if let Some(ks) = ext(&exts, EXT_KEY_SHARE) {
            let mut k = R { b: whole16(ks)? };
            while !k.b.is_empty() {
                let g = k.u16()?;
                let key = k.v16()?;
                if !groups.contains(&g) || shares.iter().any(|s| s.0 == g) {
                    return Err(fail(AL_ILLEGAL_PARAMETER, "key share for a group not listed, or twice"));
                }
                shares.push((g, key));
            }
        }
        if self.hrr && ext(&exts, EXT_COOKIE).is_some_and(|c| whole16(c).map(|v| v != self.cookie.as_slice()).unwrap_or(true)) {
            return Err(fail(AL_ILLEGAL_PARAMETER, "cookie"));
        }
        // the client's order decides: the first group it lists that we have, with its share when
        // it sent one, else a HelloRetryRequest for that group
        let ours = |g: &u16| kx::known(*g);
        let pick = groups.iter().filter(|g| ours(g)).find_map(|g| shares.iter().find(|s| s.0 == *g)).copied();
        let Some((group, pk)) = pick else {
            let Some(&want) = groups.iter().find(|g| ours(g)) else {
                return Err(fail(AL_HANDSHAKE_FAILURE, "no common group"));
            };
            if self.hrr {
                return Err(fail(AL_ILLEGAL_PARAMETER, "no share for the asked group after HelloRetryRequest"));
            }
            // HelloRetryRequest: ask for a share of that group
            self.hrr = true;
            self.ch1 = Some(fixed);
            self.group = want;
            self.add(msg);
            let h1 = self.hash();
            self.transcript = Transcript::new();
            self.transcript.keep(self.hash_alg);
            let mh = hs_msg(HS_MESSAGE_HASH, &h1);
            self.add(&mh);
            let mut b = Vec::new();
            put16(&mut b, 0x0303);
            b.extend_from_slice(&HRR_RANDOM);
            put_v8(&mut b, &sid);
            put16(&mut b, suite);
            b.push(0);
            let mut e = Vec::new();
            put_ext(&mut e, EXT_VERSIONS, &[0x03, 0x04]);
            put_ext(&mut e, EXT_KEY_SHARE, &want.to_be_bytes());
            put_v16(&mut b, &e);
            let m = hs_msg(HS_SERVER_HELLO, &b);
            self.send_hs(&m);
            self.compat_ccs(&sid);
            return Ok(());
        };
        if self.hrr && (group != self.group || shares.len() != 1) {
            return Err(fail(AL_ILLEGAL_PARAMETER, "the share after HelloRetryRequest is not the one asked for"));
        }
        self.group = group;
        let named = !sni.is_empty() && Cert::parse(identity.chain.first().map(|c| c.as_slice()).unwrap_or(&[])).is_some_and(|c| c.names(&sni));
        self.sni = sni;
        self.alpn = alpn;
        self.identity = Some(identity);
        let (public, mut shared) = kx::respond(group, pk).ok_or(fail(AL_ILLEGAL_PARAMETER, "key share"))?;
        self.add(msg);
        // ServerHello
        let mut b = Vec::new();
        put16(&mut b, 0x0303);
        b.extend_from_slice(&plat::random_bytes::<32>());
        put_v8(&mut b, &sid);
        put16(&mut b, suite);
        b.push(0);
        let mut e = Vec::new();
        put_ext(&mut e, EXT_VERSIONS, &[0x03, 0x04]);
        let mut ks = Vec::new();
        put16(&mut ks, group);
        put_v16(&mut ks, &public);
        put_ext(&mut e, EXT_KEY_SHARE, &ks);
        put_v16(&mut b, &e);
        let sh = hs_msg(HS_SERVER_HELLO, &b);
        self.send_hs(&sh);
        if !self.hrr {
            self.compat_ccs(&sid);
        }
        self.handshake_keys(&shared);
        crypto::zeroize(&mut shared);
        self.wr = Some(self.cipher13(&self.s_hs)?);
        let c = self.cipher13(&self.c_hs)?;
        self.set_rd(Some(c));
        // EncryptedExtensions: server_name (empty) when the certificate is for the name asked
        // for (RFC 6066 3), the protocol chosen (ALPN)
        let mut e = Vec::new();
        if named {
            put_ext(&mut e, EXT_SERVER_NAME, &[]);
        }
        if let Some(p) = &self.alpn {
            let mut list = Vec::new();
            put_v8(&mut list, p);
            let mut v = Vec::new();
            put_v16(&mut v, &list);
            put_ext(&mut e, EXT_ALPN, &v);
        }
        let mut b = Vec::new();
        put_v16(&mut b, &e);
        let ee = hs_msg(HS_ENCRYPTED_EXTENSIONS, &b);
        self.send_hs(&ee);
        let cfg = self.scfg.as_ref().ok_or(fail(AL_INTERNAL_ERROR, "config"))?;
        let want_client = cfg.client_auth.is_some();
        let id = self.identity.clone().ok_or(fail(AL_INTERNAL_ERROR, "identity"))?;
        let (chain, key) = (id.chain, id.key);
        if want_client {
            let mut b = Vec::new();
            b.push(0); // empty context
            let mut e = Vec::new();
            let mut list = Vec::new();
            put_v16(&mut list, &u16s(if self.cr_algs.is_empty() { &WEB_SIG_ALGS[..] } else { &self.cr_algs }));
            put_ext(&mut e, EXT_SIG_ALGS, &list);
            put_v16(&mut b, &e);
            let m = hs_msg(HS_CERTIFICATE_REQUEST, &b);
            self.send_hs(&m);
        }
        let cm = hs_msg(HS_CERTIFICATE, &certificate_body(&chain));
        self.send_hs(&cm);
        let m = certificate_verify(&key, &algs, true, &self.hash())?;
        self.send_hs(&m);
        let fin = keysched::finished(self.hash_alg, &self.s_hs, &self.hash());
        let m = hs_msg(HS_FINISHED, &fin);
        self.send_hs(&m);
        self.app_secrets();
        self.wr = Some(self.cipher13(&self.s_ap)?);
        self.state = if want_client { State::ServerClientCert } else { State::ServerFinished };
        Ok(())
    }
}

/// the host name of a client's server_name (RFC 6066 3): one host_name entry (lower case);
/// other name types are skipped, a second host_name is illegal_parameter, a name that is not one
/// (an IP address, a trailing dot, bytes outside letters, digits, '-', '_', '.') is taken as none
fn client_server_name(b: &[u8]) -> Result<String, TlsError> {
    let mut r = R { b };
    let mut list = R { b: r.v16()? };
    r.done()?;
    if list.b.is_empty() {
        return Err(fail(AL_DECODE_ERROR, "empty server_name list"));
    }
    let mut host: Option<&[u8]> = None;
    while !list.b.is_empty() {
        let ty = list.u8()?;
        let name = list.v16()?;
        if ty == 0 {
            if host.is_some() {
                return Err(fail(AL_ILLEGAL_PARAMETER, "two host names in server_name"));
            }
            host = Some(name);
        }
    }
    let Some(h) = host else { return Ok(String::new()) };
    let ok = !h.is_empty() && h.len() <= 253 && h.last() != Some(&b'.') && h.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'));
    let text = String::from_utf8_lossy(h).to_ascii_lowercase();
    Ok(if ok && plat::net::IpAddr::parse(&text).is_none() { text } else { String::new() })
}

/// a ProtocolNameList (RFC 7301 3.1): at least one name, none empty
fn alpn_list(b: &[u8]) -> Result<Vec<Vec<u8>>, TlsError> {
    let mut r = R { b };
    let mut list = R { b: r.v16()? };
    r.done()?;
    let mut out = Vec::new();
    while !list.b.is_empty() {
        let p = list.v8()?;
        if p.is_empty() {
            return Err(fail(AL_DECODE_ERROR, "an empty protocol name"));
        }
        out.push(Vec::from(p));
    }
    if out.is_empty() {
        return Err(fail(AL_DECODE_ERROR, "an empty protocol list"));
    }
    Ok(out)
}

/// the identity a server shows (ServerConfig::identities): the program's choice; else the first
/// of the lists that names the host asked for with a key the client takes; else `identity` when
/// its key fits, else the first that does (the handshake fails when none does)
fn choose_identity(cfg: &ServerConfig, hello: &Hello) -> Identity {
    if let Some(id) = cfg.select.as_ref().and_then(|f| f(hello)) {
        return id;
    }
    let all: Vec<&Identity> = core::iter::once(&cfg.identity).chain(cfg.identities.iter()).collect();
    let fits = |i: &&Identity| our_scheme(&i.key, &hello.sig_algs).is_some();
    let names = |i: &&Identity| !hello.server_name.is_empty() && i.chain.first().and_then(|c| Cert::parse(c)).is_some_and(|c| c.names(&hello.server_name));
    // a name: the more specific lists first, then the default
    let named = cfg.identities.iter().chain(core::iter::once(&cfg.identity)).find(|i| fits(i) && names(i));
    let pick = named.or_else(|| all.iter().copied().find(|i| fits(i)));
    pick.unwrap_or(&cfg.identity).clone()
}

/// what a second ClientHello must repeat of the first: the session id, the cipher suites, the
/// compression methods and every extension but key_share (51), cookie (44), early_data (42) and
/// padding (21), in their order (RFC 8446 4.1.2)
fn ch_fixed(sid: &[u8], suites: &[u16], comp: &[u8], exts: &[(u16, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    put_v8(&mut out, sid);
    for s in suites {
        put16(&mut out, *s);
    }
    put_v8(&mut out, comp);
    for (t, v) in exts {
        if ![EXT_KEY_SHARE, EXT_COOKIE, EXT_EARLY_DATA, EXT_PADDING, EXT_PSK].contains(t) {
            put16(&mut out, *t);
            out.extend_from_slice(&(v.len() as u32).to_be_bytes());
            out.extend_from_slice(v);
        }
    }
    out
}

/// the alert for a protected record that does not open
fn open_failure(e: record::OpenError) -> TlsError {
    match e {
        record::OpenError::Mac => fail(AL_BAD_RECORD_MAC, "record does not authenticate"),
        record::OpenError::Overflow => fail(AL_RECORD_OVERFLOW, "record too long"),
        record::OpenError::NoType => fail(AL_UNEXPECTED_MESSAGE, "record without a content type"),
    }
}

/// a Finished of another length than the hash (or TLS 1.2's 12 bytes) is malformed, not wrong:
/// decode_error (RFC 8446 4.4.4, 6.2), while a wrong value of the right length is decrypt_error
fn finished_len(body: &[u8], want: usize) -> Result<(), TlsError> {
    if body.len() == want { Ok(()) } else { Err(fail(AL_DECODE_ERROR, "Finished of the wrong length")) }
}

fn certificate_body(chain: &[Vec<u8>]) -> Vec<u8> {
    let mut list = Vec::new();
    for c in chain {
        put_v24(&mut list, c);
        put16(&mut list, 0); // no entry extensions
    }
    let mut b = Vec::new();
    b.push(0); // empty context
    put_v24(&mut b, &list);
    b
}

fn parse_certificates(body: &[u8]) -> Result<Vec<Cert>, TlsError> {
    let mut r = R { b: body };
    if !r.v8()?.is_empty() {
        return Err(fail(AL_ILLEGAL_PARAMETER, "certificate context"));
    }
    let mut list = R { b: r.v24()? };
    r.done()?;
    let mut out = Vec::new();
    let mut n = 0;
    while !list.b.is_empty() {
        let der = list.v24()?;
        list.v16()?;
        push_cert(&mut out, der, n)?;
        n += 1;
    }
    Ok(out)
}

/// the longest handshake message taken
const MAX_HANDSHAKE: usize = 128 * 1024;

/// the most certificates a Certificate message may hold (the path search has its own limits:
/// nine certificates deep, 64 signature checks)
const MAX_CERTS: usize = 20;

/// a certificate of a Certificate message: the first (the peer's own) must be readable, the
/// others only help to build a path, and one that cannot be read is left out, as browsers do
fn push_cert(out: &mut Vec<Cert>, der: &[u8], index: usize) -> Result<(), TlsError> {
    if index >= MAX_CERTS {
        return Err(fail(AL_BAD_CERTIFICATE, "chain too long"));
    }
    match Cert::parse(der) {
        Some(c) => out.push(c),
        None if index == 0 => return Err(fail(AL_BAD_CERTIFICATE, "certificate unreadable")),
        None => {}
    }
    Ok(())
}

/// TLS 1.2: a list of certificates without context or extensions
fn parse_certificates12(body: &[u8]) -> Result<Vec<Cert>, TlsError> {
    let mut r = R { b: body };
    let mut list = R { b: r.v24()? };
    r.done()?;
    let mut out = Vec::new();
    let mut n = 0;
    while !list.b.is_empty() {
        let der = list.v24()?;
        push_cert(&mut out, der, n)?;
        n += 1;
    }
    Ok(out)
}

/// the peer's CertificateVerify: a scheme we offered that fits its certificate's key (TLS 1.3:
/// no PKCS #1 v1.5, ECDSA with its curve's hash)
fn verify_cv(body: &[u8], leaf: &Cert, server: bool, hash: &[u8]) -> Result<(), TlsError> {
    let mut r = R { b: body };
    let sch = r.u16()?;
    let sig = r.v16()?;
    r.done()?;
    if !WEB_SIG_ALGS.contains(&sch) {
        return Err(fail(AL_ILLEGAL_PARAMETER, "signature scheme"));
    }
    if matches!(leaf.key, PubKey::Other) {
        return Err(fail(AL_UNSUPPORTED_CERTIFICATE, "a key type this side does not take"));
    }
    let alg = scheme(sch, &leaf.key, false).ok_or(fail(AL_ILLEGAL_PARAMETER, "signature scheme"))?;
    if !leaf.verify(alg, &cv_content(server, hash), sig) {
        return Err(fail(AL_DECRYPT_ERROR, "CertificateVerify"));
    }
    Ok(())
}

fn check_trust(trust: &Trust, chain: &[Cert], name: Option<&String>, usage: Usage) -> Result<(), TlsError> {
    if matches!(trust, Trust::Anchors(_) | Trust::System(_)) && name.is_some_and(|n| n.is_empty()) {
        return Err(fail(AL_BAD_CERTIFICATE, "no server name to check"));
    }
    let leaf = &chain[0];
    let now = plat::unix_time_ns() / 1_000_000_000;
    let expired = now > leaf.not_after || now < leaf.not_before;
    match trust {
        Trust::Any => Ok(()),
        Trust::Pinned(pins) => {
            let fp = crypto::sha256(&leaf.der);
            if pins.iter().any(|p| crypto::ct_eq(p, &fp)) { Ok(()) } else { Err(fail(AL_BAD_CERTIFICATE, "certificate not pinned")) }
        }
        Trust::Anchors(anchors) => {
            let name = name.filter(|n| !n.is_empty()).map(|s| s.as_str());
            verify_chain(chain, anchors, now, name, usage)
                .map_err(|_| if expired { fail(AL_CERTIFICATE_EXPIRED, "certificate expired") } else { fail(AL_UNKNOWN_CA, "certificate not trusted") })
        }
        Trust::System(extra) => {
            let name = name.map(|s| s.as_str()).unwrap_or("");
            if expired {
                return Err(fail(AL_CERTIFICATE_EXPIRED, "certificate expired"));
            }
            if crate::roots::verify_system(chain, name, now).is_ok() {
                return Ok(());
            }
            if !extra.is_empty() && verify_chain(chain, extra, now, Some(name).filter(|n| !n.is_empty()), usage).is_ok() {
                return Ok(());
            }
            Err(if !name.is_empty() && !leaf.names(name) { fail(AL_BAD_CERTIFICATE, "certificate for another name") } else { fail(AL_UNKNOWN_CA, "certificate not trusted") })
        }
        Trust::Custom(check) => check(chain, name.map(|s| s.as_str()).unwrap_or(""), now).map_err(|_| fail(AL_BAD_CERTIFICATE, "certificate refused by the program's check")),
    }
}
