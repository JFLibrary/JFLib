// conn -- the TLS handshake and the record stream, without I/O
//
//   Conn::client(cfg) / Conn::server(cfg)
//   feed(bytes)          bytes from the peer (any split)
//   take_out() -> bytes  to send to the peer
//   read_plain / write_plain / close
//   established(), peer_cert(), version(), suite()
//
// Our own servers and agents (private PKI): TLS 1.3 with TLS_CHACHA20_POLY1305_SHA256, X25519,
// Ed25519 signatures and certificates, a HelloRetryRequest when the client offered X25519
// without a key share (or a cookie), mutual authentication.  The server side speaks only that.
//
// A client that trusts the system's certificate authorities (Trust::System: mail servers,
// update servers, web hooks) also offers TLS_AES_128_GCM_SHA256, RSA (PKCS #1 v1.5 / PSS) and
// ECDSA (P-256 / P-384) signatures and the P-256 key exchange (after a HelloRetryRequest),
// and falls back to TLS 1.2 with ECDHE (X25519 or P-256) and AEAD suites (AES-128-GCM,
// ChaCha20-Poly1305), the extended master secret and the TLS 1.3 downgrade check.  Private
// connections never fall back.
//
// No PSK, no 0-RTT, no session tickets (a client ignores them), no renegotiation;
// post-handshake KeyUpdate is supported.  Every protocol error ends the connection with the
// matching alert.

use alloc::string::String;
use alloc::vec::Vec;

use crypto::{x25519, Sha256};

use super::keysched::{self, Schedule, Secret};
use super::record::{self, Cipher, ALERT, APPLICATION_DATA, CHANGE_CIPHER_SPEC, HANDSHAKE, MAX_PLAIN, MAX_RECORD};
use super::record::{ECDHE_ECDSA_AES128_GCM, ECDHE_ECDSA_CHACHA20, ECDHE_RSA_AES128_GCM, ECDHE_RSA_CHACHA20, TLS_AES_128_GCM_SHA256, TLS_CHACHA20_POLY1305_SHA256};
use super::{ClientConfig, ServerConfig, TlsError, Trust};
use crate::x509::{verify_chain, Cert, PubKey, SigAlg, Usage};
use crypto::ecdsa::Curve;
use crypto::HashAlg;

const SUITE: u16 = TLS_CHACHA20_POLY1305_SHA256;
const X25519: u16 = 0x001d;
const SECP256R1: u16 = 0x0017;
const ED25519: u16 = 0x0807;
const TLS13: u16 = 0x0304;

const SUITES_13: [u16; 2] = [TLS_CHACHA20_POLY1305_SHA256, TLS_AES_128_GCM_SHA256];
const SUITES_12: [u16; 4] = [ECDHE_ECDSA_CHACHA20, ECDHE_RSA_CHACHA20, ECDHE_ECDSA_AES128_GCM, ECDHE_RSA_AES128_GCM];
/// signature schemes a web client accepts (CertificateVerify, ServerKeyExchange, certificates)
const WEB_SIG_ALGS: [u16; 9] = [0x0403, 0x0503, 0x0804, 0x0805, 0x0806, ED25519, 0x0401, 0x0501, 0x0601];
/// "DOWNGRD" + 1 / 0: the last bytes of a TLS 1.3 server's random when it speaks TLS 1.2 / older
const DOWNGRADE: [u8; 7] = *b"DOWNGRD";

const EXT_SERVER_NAME: u16 = 0;
const EXT_GROUPS: u16 = 10;
const EXT_POINT_FORMATS: u16 = 11;
const EXT_SIG_ALGS: u16 = 13;
const EXT_EMS: u16 = 23;
const EXT_COOKIE: u16 = 44;
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
pub const AL_CERTIFICATE_REQUIRED: u8 = 116;

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

fn cv_content(server: bool, hash: &[u8; 32]) -> Vec<u8> {
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
    transcript: Sha256,
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
    eph: [u8; 32],
    session_id: Vec<u8>,
    cookie: Vec<u8>,
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
    /// TLS 1.2: the ECDHE secret, the master secret, the server's keys until its CCS
    pms: Option<[u8; 32]>,
    master12: [u8; 48],
    rd_next: Option<Cipher>,
    /// the group of our key share (X25519, or P-256 when the server asks for it)
    group: u16,
    eph256: [u8; 32],
}

impl Conn {
    fn new(server: bool) -> Conn {
        Conn {
            server,
            state: if server { State::ServerCh } else { State::ClientSh },
            transcript: Sha256::new(),
            inbuf: Vec::new(),
            hs: Vec::new(),
            out: Vec::new(),
            plain: Vec::new(),
            rd: None,
            wr: None,
            c_hs: [0; 32],
            s_hs: [0; 32],
            c_ap: [0; 32],
            s_ap: [0; 32],
            master: [0; 32],
            eph: plat::random_bytes(),
            session_id: Vec::new(),
            cookie: Vec::new(),
            hrr: false,
            cert_requested: false,
            peer: Vec::new(),
            ccfg: None,
            scfg: None,
            peer_closed: false,
            suite: SUITE,
            v12: false,
            web: false,
            client_random: [0; 32],
            server_random: [0; 32],
            ems: false,
            pms: None,
            master12: [0; 48],
            rd_next: None,
            group: X25519,
            eph256: [0; 32],
        }
    }

    /// our public key share in `group`
    fn key_share(&mut self, group: u16) -> Vec<u8> {
        if group == SECP256R1 {
            if self.eph256 == [0; 32] {
                self.eph256 = loop {
                    if let Some(k) = crypto::p256::secret_from(&plat::random_bytes()) {
                        break k;
                    }
                };
            }
            Vec::from(&crypto::p256::public_key(&self.eph256)[..])
        } else {
            Vec::from(&x25519::public_key(&self.eph)[..])
        }
    }

    /// the shared secret with the peer's share in `group`
    fn ecdhe(&self, group: u16, peer: &[u8]) -> Result<[u8; 32], TlsError> {
        let r = if group == SECP256R1 {
            crypto::p256::shared(&self.eph256, peer)
        } else {
            let p: [u8; 32] = peer.try_into().map_err(|_| fail(AL_ILLEGAL_PARAMETER, "key share length"))?;
            x25519::shared(&self.eph, &p)
        };
        r.ok_or(fail(AL_ILLEGAL_PARAMETER, "key share"))
    }

    pub fn client(cfg: ClientConfig) -> Conn {
        let mut c = Conn::new(false);
        c.session_id = Vec::from(&plat::random_bytes::<32>()[..]);
        c.client_random = plat::random_bytes();
        c.web = matches!(cfg.trust, Trust::System(_));
        c.ccfg = Some(cfg);
        c.send_client_hello();
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

    fn hash(&self) -> [u8; 32] {
        self.transcript.clone().finalize()
    }

    fn add(&mut self, msg: &[u8]) {
        self.transcript.update(msg);
    }

    /// queue a handshake message (protected when write keys exist)
    fn send_hs(&mut self, msg: &[u8]) {
        self.add(msg);
        let rec = match self.wr.as_mut() {
            Some(c) => c.seal(HANDSHAKE, msg),
            None => record::plain(HANDSHAKE, msg),
        };
        self.out.extend_from_slice(&rec);
    }

    /// the TLS 1.3 record keys of a traffic secret for the negotiated suite
    fn cipher13(&self, secret: &Secret) -> Result<Cipher, TlsError> {
        let (kl, _) = record::key_iv_len(self.suite).ok_or(fail(AL_INTERNAL_ERROR, "suite"))?;
        let (k, iv) = keysched::traffic_keys_len(secret, kl);
        Cipher::for_suite(self.suite, &k, &iv, false).ok_or(fail(AL_INTERNAL_ERROR, "suite"))
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
            if self.inbuf.len() < 5 + len {
                return Ok(());
            }
            let hdr = [self.inbuf[0], self.inbuf[1], self.inbuf[2], self.inbuf[3], self.inbuf[4]];
            let body: Vec<u8> = self.inbuf[5..5 + len].to_vec();
            self.inbuf.drain(..5 + len);
            match ty {
                CHANGE_CIPHER_SPEC if self.v12 => {
                    // TLS 1.2: the server's records are protected from here on
                    if body != [1] || self.state != State::Client12Ccs {
                        return Err(fail(AL_UNEXPECTED_MESSAGE, "change_cipher_spec"));
                    }
                    self.rd = self.rd_next.take();
                    self.state = State::Client12Fin;
                }
                CHANGE_CIPHER_SPEC => {
                    // compatibility mode: one byte 1, only during the handshake
                    if body != [1] || self.state == State::Established {
                        return Err(fail(AL_UNEXPECTED_MESSAGE, "change_cipher_spec"));
                    }
                }
                _ if self.v12 && self.rd.is_some() => {
                    let rd = self.rd.as_mut().ok_or(fail(AL_INTERNAL_ERROR, "keys"))?;
                    let (inner, content) = rd.open(&hdr, &body).ok_or(fail(AL_BAD_RECORD_MAC, "record does not authenticate"))?;
                    match inner {
                        HANDSHAKE => self.handshake_bytes(&content)?,
                        ALERT => self.peer_alert(&content)?,
                        APPLICATION_DATA if self.state == State::Established => self.plain.extend_from_slice(&content),
                        _ => return Err(fail(AL_UNEXPECTED_MESSAGE, "record type")),
                    }
                }
                APPLICATION_DATA => {
                    let rd = self.rd.as_mut().ok_or(fail(AL_UNEXPECTED_MESSAGE, "data before keys"))?;
                    let (inner, content) = rd.open(&hdr, &body).ok_or(fail(AL_BAD_RECORD_MAC, "record does not authenticate"))?;
                    match inner {
                        HANDSHAKE => self.handshake_bytes(&content)?,
                        ALERT => self.peer_alert(&content)?,
                        APPLICATION_DATA => {
                            if self.state != State::Established {
                                return Err(fail(AL_UNEXPECTED_MESSAGE, "data during the handshake"));
                            }
                            self.plain.extend_from_slice(&content);
                        }
                        _ => return Err(fail(AL_UNEXPECTED_MESSAGE, "inner type")),
                    }
                }
                HANDSHAKE => {
                    if self.rd.is_some() {
                        return Err(fail(AL_UNEXPECTED_MESSAGE, "plaintext handshake after keys"));
                    }
                    if body.is_empty() {
                        return Err(fail(AL_DECODE_ERROR, "empty handshake record"));
                    }
                    self.handshake_bytes(&body)?;
                }
                // a peer that fails before it has our keys may still send a plaintext alert
                ALERT => self.peer_alert(&body)?,
                _ => return Err(fail(AL_UNEXPECTED_MESSAGE, "record type")),
            }
            if self.state == State::Closed {
                return Ok(());
            }
        }
    }

    fn peer_alert(&mut self, b: &[u8]) -> Result<(), TlsError> {
        if b.len() != 2 {
            return Err(fail(AL_DECODE_ERROR, "alert"));
        }
        if b[1] == AL_CLOSE_NOTIFY {
            self.peer_closed = true;
            return Ok(());
        }
        if b[0] == 1 && self.v12 {
            return Ok(()); // a TLS 1.2 warning (no_renegotiation, ...)
        }
        self.state = State::Closed;
        Err(TlsError::PeerAlert(b[1]))
    }

    fn handshake_bytes(&mut self, b: &[u8]) -> Result<(), TlsError> {
        self.hs.extend_from_slice(b);
        loop {
            if self.hs.len() < 4 {
                return Ok(());
            }
            let n = (self.hs[1] as usize) << 16 | (self.hs[2] as usize) << 8 | self.hs[3] as usize;
            if n > 64 * 1024 {
                return Err(fail(AL_DECODE_ERROR, "handshake message too long"));
            }
            if self.hs.len() < 4 + n {
                return Ok(());
            }
            let msg: Vec<u8> = self.hs[..4 + n].to_vec();
            self.hs.drain(..4 + n);
            let keys_before = self.rd.is_some();
            self.message(&msg)?;
            // a key change must fall on a record boundary
            if self.rd.is_some() != keys_before && !self.hs.is_empty() {
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
                extensions(r.v16()?)?;
                r.done()?;
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
                let algs = u16_list(R { b: ext(&exts, EXT_SIG_ALGS).ok_or(fail(AL_MISSING_EXTENSION, "signature_algorithms"))? }.v16()?)?;
                let have_identity = self.ccfg.as_ref().is_some_and(|c| c.identity.is_some());
                if have_identity && !algs.contains(&ED25519) {
                    return Err(fail(AL_HANDSHAKE_FAILURE, "server does not accept Ed25519"));
                }
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
                verify_cv(body, &self.peer[0], server, &hash, self.web && server)?;
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
                let want = keysched::finished(&self.c_hs, &self.hash());
                if !crypto::ct_eq(&want, body) {
                    return Err(fail(AL_DECRYPT_ERROR, "client Finished"));
                }
                self.add(msg);
                self.rd = Some(self.cipher13(&self.c_ap.clone())?);
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
                keysched::prf12(&self.master12, "server finished", &self.hash(), &mut want);
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
                let peer_secret = if self.server { &mut self.c_ap } else { &mut self.s_ap };
                *peer_secret = keysched::update(peer_secret);
                let ps = *peer_secret;
                self.rd = Some(self.cipher13(&ps)?);
                if requested == 1 {
                    let msg = hs_msg(HS_KEY_UPDATE, &[0]);
                    let rec = self.wr.as_mut().ok_or(TlsError::Closed)?.seal(HANDSHAKE, &msg);
                    self.out.extend_from_slice(&rec);
                    let my_secret = if self.server { &mut self.s_ap } else { &mut self.c_ap };
                    *my_secret = keysched::update(my_secret);
                    let ms = *my_secret;
                    self.wr = Some(self.cipher13(&ms)?);
                }
                Ok(())
            }
            _ => Err(fail(AL_UNEXPECTED_MESSAGE, "handshake message out of order")),
        }
    }

    // ------------------------------------------------------------------ client

    fn send_client_hello(&mut self) {
        let cfg = self.ccfg.as_ref().map(|c| c.server_name.clone()).unwrap_or_default();
        let mut b = Vec::new();
        put16(&mut b, 0x0303);
        b.extend_from_slice(&self.client_random);
        put_v8(&mut b, &self.session_id);
        if self.web {
            put_v16(&mut b, &u16s(&[&SUITES_13[..], &SUITES_12[..]].concat()));
        } else {
            put_v16(&mut b, &SUITE.to_be_bytes());
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
        if self.web {
            put_ext(&mut e, EXT_VERSIONS, &[4, 0x03, 0x04, 0x03, 0x03]);
            put_ext(&mut e, EXT_POINT_FORMATS, &[1, 0]);
            put_ext(&mut e, EXT_EMS, &[]);
            put_ext(&mut e, EXT_RENEGOTIATION_INFO, &[0]);
        } else {
            put_ext(&mut e, EXT_VERSIONS, &[2, 0x03, 0x04]);
        }
        if self.web {
            // P-384 last: TLS 1.2 servers with a P-384 certificate want to see the curve listed
            put_ext(&mut e, EXT_GROUPS, &[0, 6, 0x00, 0x1d, 0x00, 0x17, 0x00, 0x18]);
        } else {
            put_ext(&mut e, EXT_GROUPS, &[0, 2, 0x00, 0x1d]);
        }
        if self.web {
            let mut algs = Vec::new();
            put_v16(&mut algs, &u16s(&WEB_SIG_ALGS));
            put_ext(&mut e, EXT_SIG_ALGS, &algs);
        } else {
            put_ext(&mut e, EXT_SIG_ALGS, &[0, 2, 0x08, 0x07]);
        }
        let mut ks = Vec::new();
        let mut entry = Vec::new();
        let group = self.group;
        put16(&mut entry, group);
        put_v16(&mut entry, &self.key_share(group));
        put_v16(&mut ks, &entry);
        put_ext(&mut e, EXT_KEY_SHARE, &ks);
        if !self.cookie.is_empty() {
            let mut c = Vec::new();
            put_v16(&mut c, &self.cookie);
            put_ext(&mut e, EXT_COOKIE, &c);
        }
        put_v16(&mut b, &e);
        let msg = hs_msg(HS_CLIENT_HELLO, &b);
        self.send_hs(&msg);
    }

    fn client_server_hello(&mut self, msg: &[u8], body: &[u8]) -> Result<(), TlsError> {
        let mut r = R { b: body };
        if r.u16()? != 0x0303 {
            return Err(fail(AL_PROTOCOL_VERSION, "legacy version"));
        }
        let random = r.take(32)?;
        let sid = r.v8()?;
        let suite = r.u16()?;
        if r.u8()? != 0 {
            return Err(fail(AL_ILLEGAL_PARAMETER, "compression"));
        }
        let exts = extensions(if r.b.is_empty() { &[] } else { r.v16()? })?;
        r.done()?;
        let Some(v) = ext(&exts, EXT_VERSIONS) else {
            if self.web && random != HRR_RANDOM {
                return self.server_hello12(msg, random, suite, &exts);
            }
            return Err(fail(AL_PROTOCOL_VERSION, "the server does not speak TLS 1.3"));
        };
        if sid != self.session_id.as_slice() {
            return Err(fail(AL_ILLEGAL_PARAMETER, "session id not echoed"));
        }
        let offered: &[u16] = if self.web { &SUITES_13 } else { &[SUITE] };
        if !offered.contains(&suite) || (self.hrr && suite != self.suite) {
            return Err(fail(AL_ILLEGAL_PARAMETER, "cipher suite"));
        }
        self.suite = suite;
        if v != [0x03, 0x04] {
            return Err(fail(AL_ILLEGAL_PARAMETER, "supported_versions"));
        }
        if random == HRR_RANDOM {
            if self.hrr {
                return Err(fail(AL_UNEXPECTED_MESSAGE, "second HelloRetryRequest"));
            }
            let cookie = ext(&exts, EXT_COOKIE);
            match ext(&exts, EXT_KEY_SHARE) {
                Some(g) if g == self.group.to_be_bytes() => return Err(fail(AL_ILLEGAL_PARAMETER, "HelloRetryRequest for the offered group")),
                Some(g) if self.web && g == SECP256R1.to_be_bytes() => self.group = SECP256R1,
                Some(_) => return Err(fail(AL_HANDSHAKE_FAILURE, "the server wants a key exchange other than X25519 / P-256")),
                None if cookie.is_none() => return Err(fail(AL_ILLEGAL_PARAMETER, "HelloRetryRequest without a change")),
                None => {}
            }
            if let Some(c) = cookie {
                self.cookie = Vec::from(R { b: c }.v16()?);
            }
            self.hrr = true;
            // transcript: message_hash(ClientHello1), HRR
            let h1 = self.hash();
            self.transcript = Sha256::new();
            let mh = hs_msg(HS_MESSAGE_HASH, &h1);
            self.add(&mh);
            self.add(msg);
            self.send_client_hello();
            return Ok(());
        }
        let ks = ext(&exts, EXT_KEY_SHARE).ok_or(fail(AL_MISSING_EXTENSION, "key_share"))?;
        let mut k = R { b: ks };
        if k.u16()? != self.group {
            return Err(fail(AL_ILLEGAL_PARAMETER, "key share group"));
        }
        let pk = k.v16()?;
        k.done()?;
        let shared = self.ecdhe(self.group, pk)?;
        self.add(msg);
        self.handshake_keys(&shared);
        self.rd = Some(self.cipher13(&self.s_hs.clone())?);
        self.state = State::ClientEe;
        Ok(())
    }

    fn handshake_keys(&mut self, shared: &[u8; 32]) {
        let s = Schedule::new(shared);
        let h = self.hash();
        self.c_hs = keysched::derive(&s.handshake, "c hs traffic", &h);
        self.s_hs = keysched::derive(&s.handshake, "s hs traffic", &h);
        self.master = s.master;
    }

    fn app_secrets(&mut self) {
        let h = self.hash();
        self.c_ap = keysched::derive(&self.master, "c ap traffic", &h);
        self.s_ap = keysched::derive(&self.master, "s ap traffic", &h);
    }

    fn check_server_chain(&mut self) -> Result<(), TlsError> {
        let cfg = self.ccfg.as_ref().ok_or(fail(AL_INTERNAL_ERROR, "config"))?;
        check_trust(&cfg.trust, &self.peer, Some(&cfg.server_name), Usage::Server)
    }

    fn check_client_chain(&mut self) -> Result<(), TlsError> {
        let trust = self.scfg.as_ref().and_then(|c| c.client_auth.as_ref()).ok_or(fail(AL_UNEXPECTED_MESSAGE, "certificate not requested"))?;
        if self.peer[0].public_key.len() != 32 {
            return Err(fail(AL_UNSUPPORTED_CERTIFICATE, "not an Ed25519 certificate"));
        }
        check_trust(trust, &self.peer, None, Usage::Client)
    }

    fn client_server_finished(&mut self, msg: &[u8], body: &[u8]) -> Result<(), TlsError> {
        let want = keysched::finished(&self.s_hs, &self.hash());
        if !crypto::ct_eq(&want, body) {
            return Err(fail(AL_DECRYPT_ERROR, "server Finished"));
        }
        self.add(msg);
        self.app_secrets();
        // our handshake messages go under the client handshake key
        self.wr = Some(self.cipher13(&self.c_hs.clone())?);
        if self.cert_requested {
            let id = self.ccfg.as_ref().and_then(|c| c.identity.clone());
            let chain: Vec<Vec<u8>> = id.as_ref().map(|i| i.chain.clone()).unwrap_or_default();
            let cm = hs_msg(HS_CERTIFICATE, &certificate_body(&chain));
            self.send_hs(&cm);
            if let Some(i) = id {
                if !chain.is_empty() {
                    let sig = i.key.sign(&cv_content(false, &self.hash()));
                    let mut b = Vec::new();
                    put16(&mut b, ED25519);
                    put_v16(&mut b, &sig);
                    let m = hs_msg(HS_CERTIFICATE_VERIFY, &b);
                    self.send_hs(&m);
                }
            }
        }
        let fin = keysched::finished(&self.c_hs, &self.hash());
        let m = hs_msg(HS_FINISHED, &fin);
        self.send_hs(&m);
        self.wr = Some(self.cipher13(&self.c_ap.clone())?);
        self.rd = Some(self.cipher13(&self.s_ap.clone())?);
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
        if ext(exts, EXT_RENEGOTIATION_INFO).is_some_and(|r| r != [0]) {
            return Err(fail(AL_HANDSHAKE_FAILURE, "renegotiation_info"));
        }
        self.v12 = true;
        self.suite = suite;
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
        if group != X25519 && group != SECP256R1 {
            return Err(fail(AL_HANDSHAKE_FAILURE, "the server chose a curve other than X25519 / P-256"));
        }
        let point = r.v8()?;
        let params = &params_start[..params_start.len() - r.b.len()];
        let sch = r.u16()?;
        let sig = r.v16()?;
        r.done()?;
        let leaf = &self.peer[0];
        let alg = scheme(sch, &leaf.key, true).ok_or(fail(AL_ILLEGAL_PARAMETER, "signature scheme"))?;
        // ECDHE_ECDSA suites need an EC (or Ed25519, RFC 8422) key, ECDHE_RSA an RSA key
        let ecdsa_suite = matches!(self.suite, ECDHE_ECDSA_AES128_GCM | ECDHE_ECDSA_CHACHA20);
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
        if group == SECP256R1 {
            self.key_share(SECP256R1);
        }
        self.pms = Some(self.ecdhe(group, point)?);
        self.group = group;
        Ok(())
    }

    fn client_finish12(&mut self) -> Result<(), TlsError> {
        if self.cert_requested {
            let cm = hs_msg(HS_CERTIFICATE, &[0, 0, 0]);
            self.send_hs(&cm);
        }
        let mut b = Vec::new();
        let share = self.key_share(self.group);
        put_v8(&mut b, &share);
        let cke = hs_msg(HS_CLIENT_KEY_EXCHANGE, &b);
        self.send_hs(&cke);
        let pms = self.pms.take().ok_or(fail(AL_UNEXPECTED_MESSAGE, "no key exchange"))?;
        if self.ems {
            keysched::prf12(&pms, "extended master secret", &self.hash(), &mut self.master12);
        } else {
            let seed = [self.client_random, self.server_random].concat();
            keysched::prf12(&pms, "master secret", &seed, &mut self.master12);
        }
        let (kl, il) = record::key_iv_len(self.suite).ok_or(fail(AL_INTERNAL_ERROR, "suite"))?;
        let mut kb = alloc::vec![0u8; 2 * (kl + il)];
        let seed = [self.server_random, self.client_random].concat();
        keysched::prf12(&self.master12, "key expansion", &seed, &mut kb);
        let (ck, rest) = kb.split_at(kl);
        let (sk, rest) = rest.split_at(kl);
        let (civ, siv) = rest.split_at(il);
        let wr = Cipher::for_suite(self.suite, ck, civ, true).ok_or(fail(AL_INTERNAL_ERROR, "suite"))?;
        self.rd_next = Some(Cipher::for_suite(self.suite, sk, siv, true).ok_or(fail(AL_INTERNAL_ERROR, "suite"))?);
        crypto::zeroize(&mut kb);
        self.out.extend_from_slice(&record::plain(CHANGE_CIPHER_SPEC, &[1]));
        self.wr = Some(wr);
        let mut fin = [0u8; 12];
        keysched::prf12(&self.master12, "client finished", &self.hash(), &mut fin);
        let m = hs_msg(HS_FINISHED, &fin);
        self.send_hs(&m);
        self.state = State::Client12Ccs;
        Ok(())
    }

    // ------------------------------------------------------------------ server

    fn server_client_hello(&mut self, msg: &[u8], body: &[u8]) -> Result<(), TlsError> {
        let mut r = R { b: body };
        let _legacy = r.u16()?;
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
        let versions = ext(&exts, EXT_VERSIONS).ok_or(fail(AL_PROTOCOL_VERSION, "TLS 1.3 required"))?;
        if !u16_list(R { b: versions }.v8()?)?.contains(&TLS13) {
            return Err(fail(AL_PROTOCOL_VERSION, "TLS 1.3 required"));
        }
        if !suites.contains(&SUITE) {
            return Err(fail(AL_HANDSHAKE_FAILURE, "TLS_CHACHA20_POLY1305_SHA256 required"));
        }
        let algs = u16_list(R { b: ext(&exts, EXT_SIG_ALGS).ok_or(fail(AL_MISSING_EXTENSION, "signature_algorithms"))? }.v16()?)?;
        if !algs.contains(&ED25519) {
            return Err(fail(AL_HANDSHAKE_FAILURE, "Ed25519 required"));
        }
        let groups = u16_list(R { b: ext(&exts, EXT_GROUPS).ok_or(fail(AL_MISSING_EXTENSION, "supported_groups"))? }.v16()?)?;
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
        if self.hrr && ext(&exts, EXT_COOKIE).is_some_and(|c| R { b: c }.v16().map(|v| v != self.cookie.as_slice()).unwrap_or(true)) {
            return Err(fail(AL_ILLEGAL_PARAMETER, "cookie"));
        }
        let Some(pk) = share else {
            if !groups.contains(&X25519) {
                return Err(fail(AL_HANDSHAKE_FAILURE, "X25519 required"));
            }
            if self.hrr {
                return Err(fail(AL_ILLEGAL_PARAMETER, "no X25519 share after HelloRetryRequest"));
            }
            // HelloRetryRequest: ask for an X25519 share
            self.hrr = true;
            self.add(msg);
            let h1 = self.hash();
            self.transcript = Sha256::new();
            let mh = hs_msg(HS_MESSAGE_HASH, &h1);
            self.add(&mh);
            let mut b = Vec::new();
            put16(&mut b, 0x0303);
            b.extend_from_slice(&HRR_RANDOM);
            put_v8(&mut b, &sid);
            put16(&mut b, SUITE);
            b.push(0);
            let mut e = Vec::new();
            put_ext(&mut e, EXT_VERSIONS, &[0x03, 0x04]);
            put_ext(&mut e, EXT_KEY_SHARE, &X25519.to_be_bytes());
            put_v16(&mut b, &e);
            let m = hs_msg(HS_SERVER_HELLO, &b);
            self.send_hs(&m);
            return Ok(());
        };
        if pk.len() != 32 {
            return Err(fail(AL_ILLEGAL_PARAMETER, "key share length"));
        }
        let mut peer = [0u8; 32];
        peer.copy_from_slice(pk);
        let shared = x25519::shared(&self.eph, &peer).ok_or(fail(AL_ILLEGAL_PARAMETER, "key share"))?;
        self.add(msg);
        // ServerHello
        let mut b = Vec::new();
        put16(&mut b, 0x0303);
        b.extend_from_slice(&plat::random_bytes::<32>());
        put_v8(&mut b, &sid);
        put16(&mut b, SUITE);
        b.push(0);
        let mut e = Vec::new();
        put_ext(&mut e, EXT_VERSIONS, &[0x03, 0x04]);
        let mut ks = Vec::new();
        put16(&mut ks, X25519);
        put_v16(&mut ks, &x25519::public_key(&self.eph));
        put_ext(&mut e, EXT_KEY_SHARE, &ks);
        put_v16(&mut b, &e);
        let sh = hs_msg(HS_SERVER_HELLO, &b);
        self.send_hs(&sh);
        self.handshake_keys(&shared);
        self.wr = Some(self.cipher13(&self.s_hs.clone())?);
        self.rd = Some(self.cipher13(&self.c_hs.clone())?);
        // EncryptedExtensions
        let ee = hs_msg(HS_ENCRYPTED_EXTENSIONS, &[0, 0]);
        self.send_hs(&ee);
        let cfg = self.scfg.as_ref().ok_or(fail(AL_INTERNAL_ERROR, "config"))?;
        let want_client = cfg.client_auth.is_some();
        let chain = cfg.identity.chain.clone();
        let key = cfg.identity.key.clone();
        if want_client {
            let mut b = Vec::new();
            b.push(0); // empty context
            let mut e = Vec::new();
            put_ext(&mut e, EXT_SIG_ALGS, &[0, 2, 0x08, 0x07]);
            put_v16(&mut b, &e);
            let m = hs_msg(HS_CERTIFICATE_REQUEST, &b);
            self.send_hs(&m);
        }
        let cm = hs_msg(HS_CERTIFICATE, &certificate_body(&chain));
        self.send_hs(&cm);
        let sig = key.sign(&cv_content(true, &self.hash()));
        let mut b = Vec::new();
        put16(&mut b, ED25519);
        put_v16(&mut b, &sig);
        let m = hs_msg(HS_CERTIFICATE_VERIFY, &b);
        self.send_hs(&m);
        let fin = keysched::finished(&self.s_hs, &self.hash());
        let m = hs_msg(HS_FINISHED, &fin);
        self.send_hs(&m);
        self.app_secrets();
        self.wr = Some(self.cipher13(&self.s_ap.clone())?);
        self.state = if want_client { State::ServerClientCert } else { State::ServerFinished };
        Ok(())
    }
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
    while !list.b.is_empty() {
        let der = list.v24()?;
        list.v16()?;
        if out.len() >= 10 {
            return Err(fail(AL_BAD_CERTIFICATE, "chain too long"));
        }
        let c = Cert::parse(der).ok_or(fail(AL_BAD_CERTIFICATE, "certificate unreadable"))?;
        out.push(c);
    }
    Ok(out)
}

/// TLS 1.2: a list of certificates without context or extensions
fn parse_certificates12(body: &[u8]) -> Result<Vec<Cert>, TlsError> {
    let mut r = R { b: body };
    let mut list = R { b: r.v24()? };
    r.done()?;
    let mut out = Vec::new();
    while !list.b.is_empty() {
        let der = list.v24()?;
        if out.len() >= 10 {
            return Err(fail(AL_BAD_CERTIFICATE, "chain too long"));
        }
        out.push(Cert::parse(der).ok_or(fail(AL_BAD_CERTIFICATE, "certificate unreadable"))?);
    }
    Ok(out)
}

fn verify_cv(body: &[u8], leaf: &Cert, server: bool, hash: &[u8; 32], web: bool) -> Result<(), TlsError> {
    let mut r = R { b: body };
    let sch = r.u16()?;
    let sig = r.v16()?;
    r.done()?;
    let alg = if web {
        scheme(sch, &leaf.key, false).ok_or(fail(AL_ILLEGAL_PARAMETER, "signature scheme"))?
    } else {
        if sch != ED25519 {
            return Err(fail(AL_ILLEGAL_PARAMETER, "signature scheme"));
        }
        if leaf.public_key.len() != 32 || sig.len() != 64 {
            return Err(fail(AL_UNSUPPORTED_CERTIFICATE, "not an Ed25519 certificate"));
        }
        SigAlg::Ed25519
    };
    if !leaf.verify(alg, &cv_content(server, hash), sig) {
        return Err(fail(AL_DECRYPT_ERROR, "CertificateVerify"));
    }
    Ok(())
}

fn check_trust(trust: &Trust, chain: &[Cert], name: Option<&String>, usage: Usage) -> Result<(), TlsError> {
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
            if leaf.public_key.len() != 32 {
                return Err(fail(AL_UNSUPPORTED_CERTIFICATE, "not an Ed25519 certificate"));
            }
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
    }
}
