// tls -- TLS 1.3 servers and clients, and TLS 1.2 towards public servers (RFC 8446, 5246)
//
//   Both sides: TLS 1.3 · ChaCha20-Poly1305, AES-128-GCM, AES-256-GCM · X25519MLKEM768
//   (post-quantum hybrid), X25519, P-256, P-384 · certificates and keys of Ed25519, ECDSA
//   (P-256 / P-384) and RSA (the web PKI or a CA of one's own) · mutual authentication ·
//   HelloRetryRequest · KeyUpdate · resumption with session tickets (psk_dhe_ke) · 0-RTT with
//   replay protection (opt-in).
//   Public servers (client side, Trust::System / Custom): also TLS 1.2 with ECDHE and AEAD suites.
//   Not here: external PSKs, psk_ke, TLS 1.2 resumption, renegotiation, revocation checks.
//
//   connect(stream, ClientConfig) -> TlsStream     accept(stream, ServerConfig) -> TlsStream
//   TlsStream implements Stream (HTTP runs over it unchanged); peer_certs() after the handshake
//   Resumption: ServerConfig::tickets (Tickets: keys, lifetime), ClientConfig::sessions
//   (Sessions: a client's tickets per server); TlsStream::resumed()
//   0-RTT: ServerConfig::early_data (bytes; 0 = none), connect_early(stream, cfg, data) ->
//   (TlsStream, bytes taken), TlsStream::read_early (a server: data that may be a replay)
//   Identity::new(chain, key) / Identity::from_pem(certs, key): a chain and its key (PrivateKey:
//   Ed25519, ECDSA, RSA; PKCS #8, SEC 1, PKCS #1)
//   Trust::Anchors(CAs) / Pinned(SHA-256 of the leaf) / Any (trust on first use: the caller
//   shows the fingerprint and pins it) / System(extra CAs) (the operating system's CAs)
//
// conn.rs is the I/O-free core (feed bytes, take bytes); this file drives it over a Stream.

pub mod conn;
pub mod keysched;
pub mod kx;
pub mod record;
pub mod session;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use plat::net::{NetError, NetErrorKind, NetResult};
use plat::sync::Mutex;

use crate::x509::{Cert, PrivateKey};
use crate::Stream;

pub use conn::Conn;
pub use session::{ReplayCheck, Sessions, Tickets};

/// a certificate chain (DER, leaf first) and the leaf's private key (Ed25519, ECDSA, RSA)
#[derive(Clone)]
pub struct Identity {
    pub chain: Vec<Vec<u8>>,
    pub key: PrivateKey,
}

impl Identity {
    /// a chain (DER, leaf first) and its key (an ed25519::Keypair or an x509::PrivateKey)
    pub fn new(chain: Vec<Vec<u8>>, key: impl Into<PrivateKey>) -> Identity {
        Identity { chain, key: key.into() }
    }

    /// from PEM: the certificates (leaf first, as a CA hands them out) and the key ("PRIVATE
    /// KEY", "EC PRIVATE KEY" or "RSA PRIVATE KEY"; not an encrypted one); the key must be the
    /// leaf's
    pub fn from_pem(certs: &str, key: &str) -> Result<Identity, String> {
        let chain = crate::pem::decode_all(certs, "CERTIFICATE");
        let leaf = chain.first().and_then(|c| Cert::parse(c)).ok_or_else(|| String::from("no certificate"))?;
        let der = ["PRIVATE KEY", "EC PRIVATE KEY", "RSA PRIVATE KEY"].iter().find_map(|l| crate::pem::decode(key, l));
        let Some(mut der) = der else {
            return Err(String::from(if key.contains("ENCRYPTED") { "an encrypted key (decrypt it first)" } else { "no private key" }));
        };
        let k = PrivateKey::from_der(&der);
        crypto::zeroize(&mut der);
        let k = k.ok_or_else(|| String::from("a private key of a kind not taken here"))?;
        if k.public_key() != leaf.key {
            return Err(String::from("the key is not the certificate's"));
        }
        Ok(Identity { chain, key: k })
    }
}

#[derive(Clone)]
#[non_exhaustive]
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
    /// a check the program brings (a PKI of its own, keys pinned by their SPKI, Certificate
    /// Transparency, the platform's verifier, a test peer); a client offers with it what it
    /// offers with System (TLS 1.2, the public suites)
    Custom(Verifier),
}

/// the check of Trust::Custom: the certificates the peer sent that could be read (leaf first),
/// the name the client asked for ("" for a client's certificate) and the time (Unix seconds) ->
/// Ok, or why they are refused (the handshake ends with bad_certificate)
pub type Verifier = Arc<dyn Fn(&[Cert], &str, i64) -> Result<(), String> + Send + Sync>;

/// a client's side: `ClientConfig { identity: Some(id), ..ClientConfig::new(trust, "host") }`
#[derive(Clone)]
pub struct ClientConfig {
    pub trust: Trust,
    pub identity: Option<Identity>,
    /// the name the certificate must carry (DNS name or IP address); may be empty only with
    /// Trust::Pinned / Trust::Any, which do not check it
    pub server_name: String,
    /// the application protocols to offer (ALPN, RFC 7301: "h2", "http/1.1", "acme-tls/1"),
    /// in order of preference; the server's choice is TlsStream::alpn.  An empty name or one
    /// over 255 bytes is left out, and the names after the first 8 KiB of them; none left: no
    /// ALPN offered
    pub alpn: Vec<String>,
    /// session tickets to resume with (TLS 1.3): the connection offers one of this server and
    /// keeps the ones it gets.  Several configs may share a cache: it keeps the tickets by
    /// server name, trust and client identity.  None: no resumption
    pub sessions: Option<Arc<Sessions>>,
}

impl ClientConfig {
    /// this trust and server name; no client certificate, no ALPN, no resumption
    pub fn new(trust: Trust, server_name: &str) -> ClientConfig {
        ClientConfig { trust, identity: None, server_name: String::from(server_name), alpn: Vec::new(), sessions: None }
    }
}

/// what a ClientHello asks for: the program's ServerConfig::select picks the identity by it
#[derive(Clone, Debug)]
pub struct Hello {
    /// the name of server_name (SNI), lower case; empty: none
    pub server_name: String,
    /// the application protocols offered (ALPN), as the client sent them: byte strings
    /// (RFC 7301 3.1), b"h2", b"http/1.1"
    pub alpn: Vec<Vec<u8>>,
    /// the signature schemes the client takes (TLS code points)
    pub sig_algs: Vec<u16>,
}

/// ServerConfig::select: the identity for a ClientHello, or None for the configured ones
pub type Select = Arc<dyn Fn(&Hello) -> Option<Identity> + Send + Sync>;

/// a server's side: `ServerConfig { client_auth: Some(trust), ..ServerConfig::new(identity) }`
#[derive(Clone)]
pub struct ServerConfig {
    pub identity: Identity,
    /// more certificates (SNI: other names; or other key types for the same name, as an ECDSA
    /// and an RSA certificate): the server shows the first of these and then `identity` that
    /// names the server_name the client asked for and whose key makes a signature the client
    /// takes; without such one `identity`, or the first whose key fits when its key does not
    pub identities: Vec<Identity>,
    /// a choice of the program's own, asked before the lists (certificates renewed while the
    /// server runs, ACME's tls-alpn-01 challenge); None from it: the lists decide
    pub select: Option<Select>,
    /// ask for a client certificate trusted like this
    pub client_auth: Option<Trust>,
    /// a client without a certificate is accepted too (one that sends one must pass the
    /// check); for a port that consoles and agents share
    pub client_auth_optional: bool,
    /// the application protocols the server speaks (ALPN), in its order of preference; a
    /// client that offers protocols, none of them these, is refused (no_application_protocol,
    /// RFC 7301 3.2).  Empty: ALPN is not answered
    pub alpn: Vec<String>,
    /// session tickets (TLS 1.3 resumption): sent after each handshake and taken back for a
    /// handshake without certificates.  A resumed connection keeps the client certificates of
    /// its first handshake (peer_certs) for up to 7 days, while the same client_auth trust asks
    /// for them.  None: no tickets
    pub tickets: Option<Arc<Tickets>>,
    /// 0-RTT (needs tickets): the most bytes of early data a client may send with a ticket; 0
    /// (the default): none.  Early data can be replayed by an attacker (RFC 8446 8, E.5): the
    /// server takes it only from a ClientHello within 10 seconds of the ticket's age and the
    /// first time it comes (Tickets::replay_check), keeps it apart (TlsStream::read_early) and
    /// hands it to read() only once the handshake is done.  With 0-RTT taken, accept returns
    /// after the server's flight, before the client's Finished
    pub early_data: u32,
}

impl ServerConfig {
    /// this identity; no client certificates, no ALPN, no tickets
    pub fn new(identity: Identity) -> ServerConfig {
        ServerConfig {
            identity,
            identities: Vec::new(),
            select: None,
            client_auth: None,
            client_auth_optional: false,
            alpn: Vec::new(),
            tickets: None,
            early_data: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
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
        conn::AL_NO_APPLICATION_PROTOCOL => "kein gemeinsames Anwendungsprotokoll (ALPN)",
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
    /// the read timeout of set_timeouts (u32::MAX: none): records that carry no data (empty
    /// ones, KeyUpdates) end a read with TimedOut after it, as silence would
    read_ms: core::sync::atomic::AtomicU32,
}

impl TlsStream {
    fn new(inner: Box<dyn Stream>, conn: Conn) -> TlsStream {
        TlsStream { inner, conn: Mutex::new(conn), wlock: Mutex::new(()), last_error: Mutex::new(None), read_ms: core::sync::atomic::AtomicU32::new(u32::MAX) }
    }

    /// (tests) send an application data record without data
    #[doc(hidden)]
    pub fn test_send_empty_record(&self) -> NetResult<()> {
        self.conn.lock().write_empty().map_err(to_net)?;
        self.flush().map_err(to_net)
    }

    /// sends what the connection has sealed; taking it and writing it happen under wlock, so
    /// records leave in the order of their sequence numbers
    fn flush(&self) -> Result<(), TlsError> {
        let _g = self.wlock.lock();
        let out = self.conn.lock().take_out();
        if out.is_empty() {
            return Ok(());
        }
        self.inner.write_all(&out).map_err(TlsError::Net)
    }

    /// run until the handshake is done
    fn handshake(&self) -> Result<(), TlsError> {
        let mut buf = alloc::vec![0u8; 16 * 1024 + 512];
        // the stream's timeout counts each read: a peer sending a byte at a time must not hold
        // the connection (a server worker) forever
        let deadline = plat::time_us() + 60_000_000;
        loop {
            self.flush()?;
            {
                let c = self.conn.lock();
                if c.ready() {
                    return Ok(());
                }
                if c.closed() {
                    return Err(TlsError::Closed);
                }
            }
            if plat::time_us() > deadline {
                return Err(TlsError::Net(NetError::new(NetErrorKind::TimedOut, 0)));
            }
            let n = self.inner.read(&mut buf).map_err(TlsError::Net)?;
            if n == 0 {
                return Err(TlsError::Closed);
            }
            let r = self.conn.lock().feed(&buf[..n]);
            if let Err(e) = r {
                let _ = self.flush();
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

    /// the key exchange group (kx: X25519_MLKEM768, X25519, SECP256R1 ...)
    pub fn group(&self) -> u16 {
        self.conn.lock().group()
    }

    /// the application protocol both sides agreed on (ALPN), if any
    pub fn alpn(&self) -> Option<String> {
        self.conn.lock().alpn().map(String::from)
    }

    /// a server: the name the client asked for (SNI, lower case; empty: none)
    pub fn server_name(&self) -> String {
        String::from(self.conn.lock().server_name())
    }

    /// the handshake resumed a session (a ticket instead of certificates)
    pub fn resumed(&self) -> bool {
        self.conn.lock().resumed()
    }

    /// a server that took 0-RTT: the client's early data as it comes, before the handshake is
    /// done; Ok(0) at its end (or when none was taken).  An attacker may have replayed it
    /// (RFC 8446 8): act on it only where twice does no harm (RFC 8470).  What is not read here
    /// comes first from read() once the handshake is done
    pub fn read_early(&self, buf: &mut [u8]) -> NetResult<usize> {
        let mut raw = alloc::vec![0u8; 16 * 1024 + 512];
        loop {
            {
                let mut c = self.conn.lock();
                if c.has_early() {
                    return Ok(c.read_early(buf));
                }
                if !c.in_early() || c.closed() {
                    return Ok(0);
                }
            }
            let n = self.inner.read(&mut raw)?;
            if n == 0 {
                return Ok(0);
            }
            let r = self.conn.lock().feed(&raw[..n]);
            let _ = self.flush();
            if let Err(e) = r {
                *self.last_error.lock() = Some(e);
                return Err(to_net(e));
            }
        }
    }
}

/// client side: handshake over `inner`
pub fn connect(inner: Box<dyn Stream>, cfg: ClientConfig) -> Result<TlsStream, TlsError> {
    let s = TlsStream::new(inner, Conn::client(cfg));
    s.handshake()?;
    Ok(s)
}

/// client side with 0-RTT: `early` goes right after the ClientHello when a ticket of
/// cfg.sessions allows early data (as much as the ticket takes).  -> the stream and the bytes of
/// `early` the server took: 0 when none went (no such ticket) or the server refused them; the
/// caller sends the rest with write_all, if it still applies (TlsStream::alpn may differ: RFC
/// 8446 4.2.10).  Early data can be replayed by an attacker: only for requests that do no harm
/// twice
pub fn connect_early(inner: Box<dyn Stream>, cfg: ClientConfig, early: &[u8]) -> Result<(TlsStream, usize), TlsError> {
    let s = TlsStream::new(inner, Conn::client_early(cfg, early));
    s.handshake()?;
    let n = s.conn.lock().early_taken();
    Ok((s, n))
}

/// server side: handshake over `inner`
pub fn accept(inner: Box<dyn Stream>, cfg: ServerConfig) -> Result<TlsStream, TlsError> {
    let s = TlsStream::new(inner, Conn::server(cfg));
    s.handshake()?;
    Ok(s)
}

impl Stream for TlsStream {
    fn read(&self, buf: &mut [u8]) -> NetResult<usize> {
        let mut raw = alloc::vec![0u8; 16 * 1024 + 512];
        let t0 = plat::time_us();
        let read_ms = self.read_ms.load(core::sync::atomic::Ordering::Relaxed);
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
            // records without data do not keep a read going past the timeout (the caller's
            // deadline is checked between reads)
            if read_ms != u32::MAX && plat::time_us().saturating_sub(t0) >= read_ms as u64 * 1000 {
                return Err(NetError::new(NetErrorKind::TimedOut, 0));
            }
            let n = self.inner.read(&mut raw)?;
            if n == 0 {
                return Ok(0);
            }
            let r = self.conn.lock().feed(&raw[..n]);
            let _ = self.flush(); // KeyUpdate answers, alerts
            if let Err(e) = r {
                *self.last_error.lock() = Some(e);
                return Err(to_net(e));
            }
        }
    }

    fn write_all(&self, data: &[u8]) -> NetResult<()> {
        self.conn.lock().write_plain(data).map_err(to_net)?;
        self.flush().map_err(to_net)
    }

    fn set_timeouts(&self, read_ms: Option<u32>, write_ms: Option<u32>) {
        self.read_ms.store(read_ms.unwrap_or(u32::MAX), core::sync::atomic::Ordering::Relaxed);
        self.inner.set_timeouts(read_ms, write_ms);
    }

    fn shutdown(&self) {
        self.conn.lock().close();
        let _ = self.flush();
        self.inner.shutdown();
    }

    fn end_is_clean(&self) -> bool {
        self.conn.lock().peer_closed()
    }

    fn wait_readable(&self, ms: u32) -> bool {
        if self.conn.lock().has_plain() {
            return true;
        }
        self.inner.wait_readable(ms)
    }
}
