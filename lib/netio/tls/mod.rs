// tls -- TLS for BackupCraft's own servers and agents and for public servers (RFC 8446, 5246)
//
//   Own deployments (both sides): TLS 1.3, TLS_CHACHA20_POLY1305_SHA256 · X25519 · Ed25519
//   certificates (own CA) · mutual authentication · HelloRetryRequest · KeyUpdate.
//   Public servers (client side, Trust::System): also TLS_AES_128_GCM_SHA256, RSA / ECDSA
//   certificates of the system's CAs, and TLS 1.2 with ECDHE (X25519) and AEAD suites.
//   Not here: PSK / resumption, 0-RTT, renegotiation, revocation checks.
//
//   connect(stream, ClientConfig) -> TlsStream     accept(stream, ServerConfig) -> TlsStream
//   TlsStream implements Stream (HTTP runs over it unchanged); peer_certs() after the handshake
//   Trust::Anchors(CAs) / Pinned(SHA-256 of the leaf) / Any (trust on first use: the caller
//   shows the fingerprint and pins it) / System(extra CAs) (the operating system's CAs)
//
// conn.rs is the I/O-free core (feed bytes, take bytes); this file drives it over a Stream.

pub mod conn;
pub mod keysched;
pub mod record;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crypto::ed25519::Keypair;
use plat::net::{NetError, NetErrorKind, NetResult};
use plat::sync::Mutex;

use crate::x509::Cert;
use crate::Stream;

pub use conn::Conn;

/// a certificate chain (DER, leaf first) and the leaf's private key
#[derive(Clone)]
pub struct Identity {
    pub chain: Vec<Vec<u8>>,
    pub key: Keypair,
}

#[derive(Clone)]
pub enum Trust {
    /// certificates issued by one of these CAs
    Anchors(Vec<Cert>),
    /// exactly these leaf certificates (SHA-256 of the DER)
    Pinned(Vec<[u8; 32]>),
    /// anything (only to read the fingerprint for a trust decision)
    Any,
    /// the operating system's certificate authorities, and these as well (a company CA);
    /// allows TLS 1.2 and the public suites (crate::roots)
    System(Vec<Cert>),
}

#[derive(Clone)]
pub struct ClientConfig {
    pub trust: Trust,
    pub identity: Option<Identity>,
    /// the name the certificate must carry (DNS name or IP address; "" = not checked)
    pub server_name: String,
}

#[derive(Clone)]
pub struct ServerConfig {
    pub identity: Identity,
    /// ask for a client certificate trusted like this
    pub client_auth: Option<Trust>,
    /// a client without a certificate is accepted too (one that sends one must pass the
    /// check); for a port that consoles and agents share
    pub client_auth_optional: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsError {
    /// we ended the connection with this alert
    Alert(u8, &'static str),
    /// the peer ended it with this alert
    PeerAlert(u8),
    Net(NetError),
    Closed,
}

impl core::fmt::Display for TlsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TlsError::Alert(a, why) => write!(f, "{} (alert {})", why, a),
            TlsError::PeerAlert(a) => write!(f, "{} (von der Gegenseite)", alert_name(*a)),
            TlsError::Net(e) => write!(f, "{}", e),
            TlsError::Closed => f.write_str("Verbindung geschlossen"),
        }
    }
}

pub fn alert_name(a: u8) -> String {
    String::from(match a {
        conn::AL_HANDSHAKE_FAILURE => "Handshake abgelehnt",
        conn::AL_BAD_CERTIFICATE => "Zertifikat abgelehnt",
        conn::AL_UNKNOWN_CA => "unbekannte Zertifizierungsstelle",
        conn::AL_CERTIFICATE_EXPIRED => "Zertifikat abgelaufen",
        conn::AL_CERTIFICATE_REQUIRED => "Client-Zertifikat erforderlich",
        conn::AL_DECRYPT_ERROR => "Prüfsumme falsch",
        conn::AL_PROTOCOL_VERSION => "TLS-Version nicht unterstützt",
        _ => return format!("TLS-Alert {}", a),
    })
}

fn to_net(e: TlsError) -> NetError {
    match e {
        TlsError::Net(n) => n,
        _ => NetError::new(NetErrorKind::Reset, 0),
    }
}

/// TLS over a stream
pub struct TlsStream {
    inner: Box<dyn Stream>,
    conn: Mutex<Conn>,
    /// keeps whole records together when two threads write
    wlock: Mutex<()>,
    last_error: Mutex<Option<TlsError>>,
}

impl TlsStream {
    fn flush(&self, out: Vec<u8>) -> Result<(), TlsError> {
        if out.is_empty() {
            return Ok(());
        }
        let _g = self.wlock.lock();
        self.inner.write_all(&out).map_err(TlsError::Net)
    }

    /// run until the handshake is done
    fn handshake(&self) -> Result<(), TlsError> {
        let mut buf = alloc::vec![0u8; 16 * 1024 + 512];
        loop {
            let out = self.conn.lock().take_out();
            self.flush(out)?;
            if self.conn.lock().established() {
                return Ok(());
            }
            let n = self.inner.read(&mut buf).map_err(TlsError::Net)?;
            if n == 0 {
                return Err(TlsError::Closed);
            }
            let r = self.conn.lock().feed(&buf[..n]);
            if let Err(e) = r {
                let out = self.conn.lock().take_out();
                let _ = self.flush(out);
                return Err(e);
            }
        }
    }

    /// the peer's certificate chain (leaf first)
    pub fn peer_certs(&self) -> Vec<Cert> {
        Vec::from(self.conn.lock().peer_certs())
    }

    pub fn last_error(&self) -> Option<TlsError> {
        *self.last_error.lock()
    }

    /// "TLS 1.3" / "TLS 1.2" and the cipher suite number
    pub fn version(&self) -> (&'static str, u16) {
        let c = self.conn.lock();
        (c.version(), c.suite())
    }
}

/// client side: handshake over `inner`
pub fn connect(inner: Box<dyn Stream>, cfg: ClientConfig) -> Result<TlsStream, TlsError> {
    let s = TlsStream { inner, conn: Mutex::new(Conn::client(cfg)), wlock: Mutex::new(()), last_error: Mutex::new(None) };
    s.handshake()?;
    Ok(s)
}

/// server side: handshake over `inner`
pub fn accept(inner: Box<dyn Stream>, cfg: ServerConfig) -> Result<TlsStream, TlsError> {
    let s = TlsStream { inner, conn: Mutex::new(Conn::server(cfg)), wlock: Mutex::new(()), last_error: Mutex::new(None) };
    s.handshake()?;
    Ok(s)
}

impl Stream for TlsStream {
    fn read(&self, buf: &mut [u8]) -> NetResult<usize> {
        let mut raw = alloc::vec![0u8; 16 * 1024 + 512];
        loop {
            {
                let mut c = self.conn.lock();
                if c.has_plain() {
                    return Ok(c.read_plain(buf));
                }
                if c.closed() {
                    return Ok(0);
                }
            }
            let n = self.inner.read(&mut raw)?;
            if n == 0 {
                return Ok(0);
            }
            let r = self.conn.lock().feed(&raw[..n]);
            let out = self.conn.lock().take_out(); // KeyUpdate answers, alerts
            let _ = self.flush(out);
            if let Err(e) = r {
                *self.last_error.lock() = Some(e);
                return Err(to_net(e));
            }
        }
    }

    fn write_all(&self, data: &[u8]) -> NetResult<()> {
        let out = {
            let mut c = self.conn.lock();
            c.write_plain(data).map_err(to_net)?;
            c.take_out()
        };
        self.flush(out).map_err(to_net)
    }

    fn set_timeouts(&self, read_ms: Option<u32>, write_ms: Option<u32>) {
        self.inner.set_timeouts(read_ms, write_ms);
    }

    fn shutdown(&self) {
        let out = {
            let mut c = self.conn.lock();
            c.close();
            c.take_out()
        };
        let _ = self.flush(out);
        self.inner.shutdown();
    }

    fn wait_readable(&self, ms: u32) -> bool {
        if self.conn.lock().has_plain() {
            return true;
        }
        self.inner.wait_readable(ms)
    }
}
