// ssh -- SSH version 2 without I/O (RFC 4250 to 4254, RFC 8731, RFC 8332, RFC 8709): the codec and
// a core for both sides of a connection: feed it the bytes that came, take the bytes to send and
// the events; no sockets, no clock.  Random bytes come from a seed the caller passes in, so a
// connection is the same byte for byte for the same seed
//
//   PORT                     22
//   MSG_*, msg_name(code)    the message numbers and their names
//   DISC_*, reason_name(c)   the reason codes of SSH_MSG_DISCONNECT
//   W, R                     writing and reading the wire types (byte, boolean, uint32, string,
//                            mpint, name-list)
//   KeyPair                  Ed25519 or RSA: generate from the caller's random bytes, blob(),
//                            key_type() ("ssh-ed25519", "ssh-rsa"), alg() ("ssh-ed25519",
//                            "rsa-sha2-256"), sign(data), bits(), public_line(comment),
//                            private_file(comment) and parse_private(text) (OpenSSH's unencrypted
//                            "OPENSSH PRIVATE KEY" format)
//   verify(alg, blob, data, sig), fingerprint(blob) ("SHA256:..."), blob_type(blob), blob_bits(blob),
//   parse_public(line) -> PublicLine { key_type, blob, comment }  (authorized_keys, known_hosts)
//   KexInit                  parse / to_payload; negotiate(client, server) -> Algos
//   exchange_hash(...), derive_key(k, h, letter, session_id, len)   RFC 4253 7.2, RFC 5656 4
//   Cipher                   one direction's protection: make(name, key, iv, mac), key_len(name),
//                            padding(n), seal(seq, payload, pad), open(seq, rx, head)
//   Config                   ident ("SSH-2.0-..."), ciphers, host key algorithms, the methods a
//                            failed login names
//   Core::client(cfg, seed) / Core::server(cfg, keys, seed)
//     push(bytes)            what the connection brought; take_out() the bytes to send;
//     take_events()          what happened (Event); dead(): nothing more will come or go
//     host_key_verdict(ok)   the client's answer to Event::HostKey (the exchange waits for it)
//     auth_none / auth_password / auth_key(user, ..)   the client's logins (after Event::AuthReady)
//     auth_accept / auth_reject / auth_pk_ok(alg, blob) / banner(text)   the server's answers to
//                            Event::AuthRequest (they may come later: a RADIUS server may be asked)
//     open_session() -> id, accept_channel(id) / refuse_channel(id, reason, text),
//     request(id, Request, want_reply), reply(id, ok), send(id, bytes), send_ext(id, 1, bytes),
//     eof(id), close(id)     channels, with windows both ways (what the window does not take waits)
//     rekey()                a new key exchange, as either side may start one
//     disconnect(reason, text)
//     algos() / session_id() / peer_ident() / authenticated() / encrypted() / footprint()
//   Event                    PeerIdent, Algos, HostKey, Encrypted, AuthReady, Banner, AuthSuccess,
//                            AuthFailure, AuthRequest, ChannelOpen, ChannelOpened, ChannelRefused,
//                            ChannelRequest, ChannelReply, Data, Eof, Closed, Debug, Unimplemented,
//                            Rekeyed, Disconnected, Failed
//   Part, dissect(segment)   what a dissector sees in a TCP segment: the identification line,
//                            packets in the clear with their fields, or bytes that are no packet
//                            in the clear (encrypted, or the rest of one)
//
// Algorithms: curve25519-sha256 (also under the name @libssh.org), host and user keys ssh-ed25519
// and rsa-sha2-256 (host keys; user keys too), ciphers chacha20-poly1305@openssh.com, aes128-ctr
// and aes256-ctr with hmac-sha2-256, compression none.  Not here: encrypted key files, the older
// ssh-rsa with SHA-1, port forwarding (a channel of another type is the embedder's to refuse),
// the extension negotiation of RFC 8308 (SSH_MSG_EXT_INFO is tolerated, none is sent) and
// OpenSSH's strict key exchange (ignored when offered).
//
// The embedder decides what the protocol leaves open: which host key to trust, who logs in, which
// channels and requests to take.  A protocol error from the peer sends SSH_MSG_DISCONNECT and
// ends in Event::Failed.  netio::ssh is the other SSH of the library, a blocking client on real
// sockets.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crypto::aes::Aes;
use crypto::ed25519;
use crypto::rsa;
use crypto::{chacha, poly1305, x25519, HashAlg};

/// the well-known port
pub const PORT: u16 = 22;

pub const MSG_DISCONNECT: u8 = 1;
pub const MSG_IGNORE: u8 = 2;
pub const MSG_UNIMPLEMENTED: u8 = 3;
pub const MSG_DEBUG: u8 = 4;
pub const MSG_SERVICE_REQUEST: u8 = 5;
pub const MSG_SERVICE_ACCEPT: u8 = 6;
pub const MSG_EXT_INFO: u8 = 7;
pub const MSG_KEXINIT: u8 = 20;
pub const MSG_NEWKEYS: u8 = 21;
pub const MSG_KEX_ECDH_INIT: u8 = 30;
pub const MSG_KEX_ECDH_REPLY: u8 = 31;
pub const MSG_USERAUTH_REQUEST: u8 = 50;
pub const MSG_USERAUTH_FAILURE: u8 = 51;
pub const MSG_USERAUTH_SUCCESS: u8 = 52;
pub const MSG_USERAUTH_BANNER: u8 = 53;
pub const MSG_USERAUTH_PK_OK: u8 = 60;
pub const MSG_GLOBAL_REQUEST: u8 = 80;
pub const MSG_REQUEST_SUCCESS: u8 = 81;
pub const MSG_REQUEST_FAILURE: u8 = 82;
pub const MSG_CHANNEL_OPEN: u8 = 90;
pub const MSG_CHANNEL_OPEN_CONFIRMATION: u8 = 91;
pub const MSG_CHANNEL_OPEN_FAILURE: u8 = 92;
pub const MSG_CHANNEL_WINDOW_ADJUST: u8 = 93;
pub const MSG_CHANNEL_DATA: u8 = 94;
pub const MSG_CHANNEL_EXTENDED_DATA: u8 = 95;
pub const MSG_CHANNEL_EOF: u8 = 96;
pub const MSG_CHANNEL_CLOSE: u8 = 97;
pub const MSG_CHANNEL_REQUEST: u8 = 98;
pub const MSG_CHANNEL_SUCCESS: u8 = 99;
pub const MSG_CHANNEL_FAILURE: u8 = 100;

/// the name of a message number (RFC 4250 4.1)
pub fn msg_name(code: u8) -> &'static str {
    match code {
        MSG_DISCONNECT => "SSH_MSG_DISCONNECT",
        MSG_IGNORE => "SSH_MSG_IGNORE",
        MSG_UNIMPLEMENTED => "SSH_MSG_UNIMPLEMENTED",
        MSG_DEBUG => "SSH_MSG_DEBUG",
        MSG_SERVICE_REQUEST => "SSH_MSG_SERVICE_REQUEST",
        MSG_SERVICE_ACCEPT => "SSH_MSG_SERVICE_ACCEPT",
        MSG_EXT_INFO => "SSH_MSG_EXT_INFO",
        MSG_KEXINIT => "SSH_MSG_KEXINIT",
        MSG_NEWKEYS => "SSH_MSG_NEWKEYS",
        MSG_KEX_ECDH_INIT => "SSH_MSG_KEX_ECDH_INIT",
        MSG_KEX_ECDH_REPLY => "SSH_MSG_KEX_ECDH_REPLY",
        MSG_USERAUTH_REQUEST => "SSH_MSG_USERAUTH_REQUEST",
        MSG_USERAUTH_FAILURE => "SSH_MSG_USERAUTH_FAILURE",
        MSG_USERAUTH_SUCCESS => "SSH_MSG_USERAUTH_SUCCESS",
        MSG_USERAUTH_BANNER => "SSH_MSG_USERAUTH_BANNER",
        MSG_USERAUTH_PK_OK => "SSH_MSG_USERAUTH_PK_OK",
        MSG_GLOBAL_REQUEST => "SSH_MSG_GLOBAL_REQUEST",
        MSG_REQUEST_SUCCESS => "SSH_MSG_REQUEST_SUCCESS",
        MSG_REQUEST_FAILURE => "SSH_MSG_REQUEST_FAILURE",
        MSG_CHANNEL_OPEN => "SSH_MSG_CHANNEL_OPEN",
        MSG_CHANNEL_OPEN_CONFIRMATION => "SSH_MSG_CHANNEL_OPEN_CONFIRMATION",
        MSG_CHANNEL_OPEN_FAILURE => "SSH_MSG_CHANNEL_OPEN_FAILURE",
        MSG_CHANNEL_WINDOW_ADJUST => "SSH_MSG_CHANNEL_WINDOW_ADJUST",
        MSG_CHANNEL_DATA => "SSH_MSG_CHANNEL_DATA",
        MSG_CHANNEL_EXTENDED_DATA => "SSH_MSG_CHANNEL_EXTENDED_DATA",
        MSG_CHANNEL_EOF => "SSH_MSG_CHANNEL_EOF",
        MSG_CHANNEL_CLOSE => "SSH_MSG_CHANNEL_CLOSE",
        MSG_CHANNEL_REQUEST => "SSH_MSG_CHANNEL_REQUEST",
        MSG_CHANNEL_SUCCESS => "SSH_MSG_CHANNEL_SUCCESS",
        MSG_CHANNEL_FAILURE => "SSH_MSG_CHANNEL_FAILURE",
        _ => "unknown message",
    }
}

pub const DISC_HOST_NOT_ALLOWED: u32 = 1;
pub const DISC_PROTOCOL_ERROR: u32 = 2;
pub const DISC_KEY_EXCHANGE_FAILED: u32 = 3;
pub const DISC_MAC_ERROR: u32 = 5;
pub const DISC_SERVICE_NOT_AVAILABLE: u32 = 7;
pub const DISC_VERSION_NOT_SUPPORTED: u32 = 8;
pub const DISC_HOST_KEY_NOT_VERIFIABLE: u32 = 9;
pub const DISC_CONNECTION_LOST: u32 = 10;
pub const DISC_BY_APPLICATION: u32 = 11;
pub const DISC_TOO_MANY_CONNECTIONS: u32 = 12;
pub const DISC_AUTH_CANCELLED: u32 = 13;
pub const DISC_NO_MORE_AUTH_METHODS: u32 = 14;

/// the name of a disconnect reason (RFC 4250 4.2.2)
pub fn reason_name(c: u32) -> &'static str {
    match c {
        DISC_HOST_NOT_ALLOWED => "host not allowed to connect",
        DISC_PROTOCOL_ERROR => "protocol error",
        DISC_KEY_EXCHANGE_FAILED => "key exchange failed",
        4 => "reserved",
        DISC_MAC_ERROR => "MAC error",
        6 => "compression error",
        DISC_SERVICE_NOT_AVAILABLE => "service not available",
        DISC_VERSION_NOT_SUPPORTED => "protocol version not supported",
        DISC_HOST_KEY_NOT_VERIFIABLE => "host key not verifiable",
        DISC_CONNECTION_LOST => "connection lost",
        DISC_BY_APPLICATION => "by application",
        DISC_TOO_MANY_CONNECTIONS => "too many connections",
        DISC_AUTH_CANCELLED => "authentication cancelled by user",
        DISC_NO_MORE_AUTH_METHODS => "no more authentication methods available",
        15 => "illegal user name",
        _ => "unknown reason",
    }
}

const MAX_PACKET: usize = 256 * 1024;
/// our receive window and the largest channel data packet we send or take
const WINDOW: u32 = 2 << 20;
const CHUNK: u32 = 32 * 1024;

const CHACHA: &str = "chacha20-poly1305@openssh.com";
const AES128: &str = "aes128-ctr";
const AES256: &str = "aes256-ctr";
const HMAC256: &str = "hmac-sha2-256";
const KEX: &str = "curve25519-sha256";
const KEX_OLD: &str = "curve25519-sha256@libssh.org";
const ED25519: &str = "ssh-ed25519";
const RSA_SHA2: &str = "rsa-sha2-256";
const RSA_TYPE: &str = "ssh-rsa";

// ---------------------------------------------------------------- the wire types

/// a message under construction
#[derive(Clone, Debug, Default)]
pub struct W(pub Vec<u8>);

impl W {
    pub fn msg(t: u8) -> W {
        W(alloc::vec![t])
    }
    pub fn u8(mut self, v: u8) -> W {
        self.0.push(v);
        self
    }
    pub fn u32(mut self, v: u32) -> W {
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub fn bool(self, v: bool) -> W {
        self.u8(v as u8)
    }
    pub fn bytes(mut self, v: &[u8]) -> W {
        self.0.extend_from_slice(&(v.len() as u32).to_be_bytes());
        self.0.extend_from_slice(v);
        self
    }
    pub fn str(self, v: &str) -> W {
        self.bytes(v.as_bytes())
    }
    /// an unsigned big-endian number as mpint (no leading zeros but the one a set top bit asks for)
    pub fn mpint(self, v: &[u8]) -> W {
        let at = v.iter().position(|&b| b != 0).unwrap_or(v.len());
        let v = &v[at..];
        if v.first().is_some_and(|b| b & 0x80 != 0) {
            let mut t = alloc::vec![0u8];
            t.extend_from_slice(v);
            self.bytes(&t)
        } else {
            self.bytes(v)
        }
    }
    pub fn raw(mut self, v: &[u8]) -> W {
        self.0.extend_from_slice(v);
        self
    }
}

/// reading a message
#[derive(Clone, Debug)]
pub struct R<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    pub fn new(b: &'a [u8]) -> R<'a> {
        R { b, p: 0 }
    }
    pub fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.p)?;
        self.p += 1;
        Some(v)
    }
    pub fn u32(&mut self) -> Option<u32> {
        let s = self.b.get(self.p..self.p.checked_add(4)?)?;
        self.p += 4;
        Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    pub fn bool(&mut self) -> Option<bool> {
        self.u8().map(|v| v != 0)
    }
    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let n = self.u32()? as usize;
        let s = self.b.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
    pub fn str(&mut self) -> Option<&'a str> {
        core::str::from_utf8(self.bytes()?).ok()
    }
    /// a name-list
    pub fn names(&mut self) -> Option<Vec<String>> {
        Some(self.str()?.split(',').filter(|x| !x.is_empty()).map(String::from).collect())
    }
    pub fn skip(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
    pub fn rest(&self) -> &'a [u8] {
        &self.b[self.p.min(self.b.len())..]
    }
    /// how many bytes were read
    pub fn pos(&self) -> usize {
        self.p
    }
}

// ---------------------------------------------------------------- keys

/// a key pair of a host or a user
#[derive(Clone)]
pub enum KeyPair {
    Ed25519(ed25519::Keypair),
    Rsa(Box<rsa::PrivateKey>),
}

impl fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyPair({})", fingerprint(&self.blob()))
    }
}

impl KeyPair {
    /// a new Ed25519 key from 32 random bytes
    pub fn ed25519(seed: &[u8; 32]) -> KeyPair {
        KeyPair::Ed25519(ed25519::Keypair::from_seed(seed))
    }
    /// a new RSA key of `bits` bits (see rsa::PrivateKey::generate)
    pub fn rsa(bits: usize, rand: &mut dyn FnMut(&mut [u8])) -> Option<KeyPair> {
        rsa::PrivateKey::generate(bits, rand).map(|k| KeyPair::Rsa(Box::new(k)))
    }
    /// the type of the key in blobs and files
    pub fn key_type(&self) -> &'static str {
        match self {
            KeyPair::Ed25519(_) => ED25519,
            KeyPair::Rsa(_) => RSA_TYPE,
        }
    }
    /// the algorithm it signs with
    pub fn alg(&self) -> &'static str {
        match self {
            KeyPair::Ed25519(_) => ED25519,
            KeyPair::Rsa(_) => RSA_SHA2,
        }
    }
    /// the public key as the protocol carries it
    pub fn blob(&self) -> Vec<u8> {
        match self {
            KeyPair::Ed25519(k) => W::default().str(ED25519).bytes(&k.public).0,
            KeyPair::Rsa(k) => W::default().str(RSA_TYPE).mpint(k.e()).mpint(&k.n()).0,
        }
    }
    pub fn bits(&self) -> u32 {
        match self {
            KeyPair::Ed25519(_) => 256,
            KeyPair::Rsa(k) => k.bits() as u32,
        }
    }
    /// a signature as the protocol carries it
    pub fn sign(&self, data: &[u8]) -> Vec<u8> {
        match self {
            KeyPair::Ed25519(k) => W::default().str(ED25519).bytes(&k.sign(data)).0,
            KeyPair::Rsa(k) => {
                let sig = k.sign_pkcs1(HashAlg::Sha256, &crypto::sha256(data)).unwrap_or_default();
                W::default().str(RSA_SHA2).bytes(&sig).0
            }
        }
    }
    /// "ssh-ed25519 AAAA... comment"
    pub fn public_line(&self, comment: &str) -> String {
        public_line(self.key_type(), &self.blob(), comment)
    }
    /// the key file in OpenSSH's format, unencrypted
    pub fn private_file(&self, comment: &str) -> String {
        let blob = self.blob();
        let seed = crypto::sha256(&blob);
        let check = u32::from_be_bytes([seed[0], seed[1], seed[2], seed[3]]);
        let mut p = W::default().u32(check).u32(check).str(self.key_type());
        match self {
            KeyPair::Ed25519(k) => {
                let mut sk = Vec::from(&k.seed()[..]);
                sk.extend_from_slice(&k.public);
                p = p.bytes(&k.public).bytes(&sk);
                crypto::zeroize(&mut sk);
            }
            KeyPair::Rsa(k) => {
                p = p.mpint(&k.n()).mpint(k.e()).mpint(k.d()).mpint(&k.iqmp()).mpint(k.p()).mpint(k.q());
            }
        }
        p = p.str(comment);
        let mut i = 1u8;
        while p.0.len() % 8 != 0 {
            p = p.u8(i);
            i += 1;
        }
        let file = W(Vec::from(&b"openssh-key-v1\0"[..])).str("none").str("none").str("").u32(1).bytes(&blob).bytes(&p.0).0;
        let b64 = crypto::b64::encode(&file);
        let mut s = String::from("-----BEGIN OPENSSH PRIVATE KEY-----\n");
        for line in b64.as_bytes().chunks(70) {
            s.push_str(core::str::from_utf8(line).unwrap_or(""));
            s.push('\n');
        }
        s.push_str("-----END OPENSSH PRIVATE KEY-----\n");
        s
    }
    /// a key from a file in OpenSSH's format, unencrypted -> the key and its comment
    pub fn parse_private(text: &str) -> Option<(KeyPair, String)> {
        let body: String = text.lines().filter(|l| !l.starts_with("-----")).map(|l| l.trim()).collect();
        let b = crypto::b64::decode(&body)?;
        let mut r = R::new(b.strip_prefix(b"openssh-key-v1\0")?);
        if r.str()? != "none" || r.str()? != "none" {
            return None;
        }
        r.bytes()?;
        if r.u32()? != 1 {
            return None;
        }
        let blob = r.bytes()?;
        let mut p = R::new(r.bytes()?);
        if p.u32()? != p.u32()? {
            return None;
        }
        let key = match p.str()? {
            ED25519 => {
                let public = p.bytes()?;
                let secret = p.bytes()?;
                if public.len() != 32 || secret.len() != 64 || &secret[32..] != public {
                    return None;
                }
                let mut seed = [0u8; 32];
                seed.copy_from_slice(&secret[..32]);
                let k = ed25519::Keypair::from_seed(&seed);
                crypto::zeroize(&mut seed);
                if k.public[..] != *public {
                    return None;
                }
                KeyPair::Ed25519(k)
            }
            RSA_TYPE => {
                let n = p.bytes()?;
                let e = p.bytes()?;
                let d = p.bytes()?;
                p.bytes()?;
                let pp = p.bytes()?;
                let q = p.bytes()?;
                KeyPair::Rsa(Box::new(rsa::PrivateKey::from_parts(n, e, d, pp, q)?))
            }
            _ => return None,
        };
        let comment = String::from(p.str()?);
        (key.blob() == blob).then_some((key, comment))
    }
}

/// "SHA256:<base64 without padding>" of a public key blob
pub fn fingerprint(blob: &[u8]) -> String {
    format!("SHA256:{}", crypto::b64::encode_nopad(&crypto::sha256(blob)))
}

/// the first string of a public key blob ("ssh-ed25519", "ssh-rsa")
pub fn blob_type(blob: &[u8]) -> &str {
    R::new(blob).str().unwrap_or("")
}

/// the bits of a public key blob
pub fn blob_bits(blob: &[u8]) -> u32 {
    let mut r = R::new(blob);
    match r.str() {
        Some(ED25519) => 256,
        Some(RSA_TYPE) => {
            r.bytes();
            match r.bytes() {
                Some(n) => {
                    let at = n.iter().position(|&b| b != 0).unwrap_or(n.len());
                    let n = &n[at..];
                    n.first().map_or(0, |t| (n.len() as u32 - 1) * 8 + (8 - t.leading_zeros()))
                }
                None => 0,
            }
        }
        _ => 0,
    }
}

/// a public key line: "TYPE BASE64 [COMMENT]"
pub fn public_line(key_type: &str, blob: &[u8], comment: &str) -> String {
    let mut s = format!("{} {}", key_type, crypto::b64::encode(blob));
    if !comment.is_empty() {
        s.push(' ');
        s.push_str(comment);
    }
    s
}

/// a line of a public key file, authorized_keys or (after the host names) known_hosts
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicLine {
    pub key_type: String,
    pub blob: Vec<u8>,
    pub comment: String,
}

/// reads "TYPE BASE64 [COMMENT]"
pub fn parse_public(line: &str) -> Option<PublicLine> {
    let mut w = line.trim().splitn(3, char::is_whitespace);
    let key_type = String::from(w.next()?);
    let blob = crypto::b64::decode(w.next()?)?;
    if blob_type(&blob) != key_type {
        return None;
    }
    Some(PublicLine { key_type, blob, comment: String::from(w.next().unwrap_or("").trim()) })
}

/// a signature blob over `data` is good for the public key blob, by the algorithm `alg`
pub fn verify(alg: &str, blob: &[u8], data: &[u8], sig: &[u8]) -> bool {
    let mut s = R::new(sig);
    if s.str() != Some(alg) {
        return false;
    }
    let Some(sig) = s.bytes() else {
        return false;
    };
    let mut r = R::new(blob);
    match (alg, r.str()) {
        (ED25519, Some(ED25519)) => {
            let (Some(p), true) = (r.bytes(), sig.len() == 64) else {
                return false;
            };
            if p.len() != 32 {
                return false;
            }
            let mut pk = [0u8; 32];
            pk.copy_from_slice(p);
            let mut sg = [0u8; 64];
            sg.copy_from_slice(sig);
            ed25519::verify(&pk, data, &sg)
        }
        (RSA_SHA2, Some(RSA_TYPE)) => {
            let (Some(e), Some(n)) = (r.bytes(), r.bytes()) else {
                return false;
            };
            rsa::PublicKey::new(n, e).is_some_and(|k| k.verify_pkcs1(HashAlg::Sha256, &crypto::sha256(data), sig))
        }
        _ => false,
    }
}

// ---------------------------------------------------------------- key exchange

/// what a side offers in SSH_MSG_KEXINIT
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KexInit {
    pub cookie: [u8; 16],
    pub kex: Vec<String>,
    pub host_key: Vec<String>,
    pub enc_cs: Vec<String>,
    pub enc_sc: Vec<String>,
    pub mac_cs: Vec<String>,
    pub mac_sc: Vec<String>,
    pub comp_cs: Vec<String>,
    pub comp_sc: Vec<String>,
    pub lang_cs: Vec<String>,
    pub lang_sc: Vec<String>,
    pub follows: bool,
}

impl KexInit {
    /// reads the message (with its number)
    pub fn parse(p: &[u8]) -> Option<KexInit> {
        let mut r = R::new(p);
        if r.u8()? != MSG_KEXINIT {
            return None;
        }
        let mut cookie = [0u8; 16];
        cookie.copy_from_slice(r.skip(16)?);
        Some(KexInit {
            cookie,
            kex: r.names()?,
            host_key: r.names()?,
            enc_cs: r.names()?,
            enc_sc: r.names()?,
            mac_cs: r.names()?,
            mac_sc: r.names()?,
            comp_cs: r.names()?,
            comp_sc: r.names()?,
            lang_cs: r.names()?,
            lang_sc: r.names()?,
            follows: r.bool()?,
        })
    }
    /// the message
    pub fn to_payload(&self) -> Vec<u8> {
        let j = |v: &[String]| v.join(",");
        W::msg(MSG_KEXINIT)
            .raw(&self.cookie)
            .str(&j(&self.kex))
            .str(&j(&self.host_key))
            .str(&j(&self.enc_cs))
            .str(&j(&self.enc_sc))
            .str(&j(&self.mac_cs))
            .str(&j(&self.mac_sc))
            .str(&j(&self.comp_cs))
            .str(&j(&self.comp_sc))
            .str(&j(&self.lang_cs))
            .str(&j(&self.lang_sc))
            .bool(self.follows)
            .u32(0)
            .0
    }
}

/// what two KEXINITs agree on
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Algos {
    pub kex: String,
    pub host_key: String,
    pub enc_cs: String,
    pub enc_sc: String,
    /// "" where the cipher authenticates by itself
    pub mac_cs: String,
    pub mac_sc: String,
    pub comp_cs: String,
    pub comp_sc: String,
}

/// the first name of the client's list the server has too (RFC 4253 7.1)
fn first_common(client: &[String], server: &[String]) -> Option<String> {
    client.iter().find(|c| server.contains(c)).cloned()
}

/// RFC 4253 7.1: the client's preference decides -> None when a list has nothing in common
pub fn negotiate(c: &KexInit, s: &KexInit) -> Option<Algos> {
    let mac = |enc: &str, cl: &[String], sv: &[String]| -> Option<String> {
        if enc == CHACHA { Some(String::new()) } else { first_common(cl, sv) }
    };
    let enc_cs = first_common(&c.enc_cs, &s.enc_cs)?;
    let enc_sc = first_common(&c.enc_sc, &s.enc_sc)?;
    Some(Algos {
        kex: first_common(&c.kex, &s.kex)?,
        host_key: first_common(&c.host_key, &s.host_key)?,
        mac_cs: mac(&enc_cs, &c.mac_cs, &s.mac_cs)?,
        mac_sc: mac(&enc_sc, &c.mac_sc, &s.mac_sc)?,
        enc_cs,
        enc_sc,
        comp_cs: first_common(&c.comp_cs, &s.comp_cs)?,
        comp_sc: first_common(&c.comp_sc, &s.comp_sc)?,
    })
}

/// the exchange hash of curve25519-sha256 (RFC 8731, RFC 5656 4): `k` the shared secret as a
/// big-endian number
#[allow(clippy::too_many_arguments)]
pub fn exchange_hash(v_c: &str, v_s: &str, i_c: &[u8], i_s: &[u8], k_s: &[u8], q_c: &[u8], q_s: &[u8], k: &[u8]) -> [u8; 32] {
    crypto::sha256(&W::default().str(v_c).str(v_s).bytes(i_c).bytes(i_s).bytes(k_s).bytes(q_c).bytes(q_s).mpint(k).0)
}

/// HASH(K || H || letter || session_id), extended as RFC 4253 7.2 says to `len` bytes
pub fn derive_key(k: &[u8], h: &[u8], letter: u8, sid: &[u8], len: usize) -> Vec<u8> {
    let kk = W::default().mpint(k).0;
    let mut first = kk.clone();
    first.extend_from_slice(h);
    first.push(letter);
    first.extend_from_slice(sid);
    let mut out = Vec::from(&crypto::sha256(&first)[..]);
    while out.len() < len {
        let mut more = kk.clone();
        more.extend_from_slice(h);
        more.extend_from_slice(&out);
        out.extend_from_slice(&crypto::sha256(&more));
    }
    out.truncate(len);
    out
}

// ---------------------------------------------------------------- the packet ciphers

/// the protection of one direction of a connection: nothing before the first NEWKEYS, then a
/// cipher with its keys
#[derive(Clone)]
pub enum Cipher {
    None,
    /// chacha20-poly1305@openssh.com: the packet's key and the length's key
    ChaCha { main: [u8; 32], header: [u8; 32] },
    /// aes128-ctr or aes256-ctr with hmac-sha2-256
    AesCtr { aes: Box<Aes>, ctr: [u8; 16], mac: [u8; 32] },
}

impl Cipher {
    /// from a cipher's name and the keys derive_key gave (key_len(enc), 16 and 32 bytes)
    pub fn make(enc: &str, key: &[u8], iv: &[u8], mac: &[u8]) -> Cipher {
        match enc {
            CHACHA => {
                let mut main = [0u8; 32];
                let mut header = [0u8; 32];
                main.copy_from_slice(&key[..32]);
                header.copy_from_slice(&key[32..64]);
                Cipher::ChaCha { main, header }
            }
            _ => {
                let mut ctr = [0u8; 16];
                ctr.copy_from_slice(&iv[..16]);
                let mut m = [0u8; 32];
                m.copy_from_slice(&mac[..32]);
                match Aes::new(key) {
                    Some(aes) => Cipher::AesCtr { aes: Box::new(aes), ctr, mac: m },
                    None => Cipher::None,
                }
            }
        }
    }

    /// how many padding bytes a payload takes (RFC 4253 6: at least 4, whole blocks)
    pub fn padding(&self, payload: usize) -> usize {
        let (block, counted) = match self {
            Cipher::None => (8, 5 + payload),
            Cipher::ChaCha { .. } => (8, 1 + payload),
            Cipher::AesCtr { .. } => (16, 5 + payload),
        };
        let pad = block - counted % block;
        if pad < 4 { pad + block } else { pad }
    }

    /// one binary packet: `pad` the padding bytes (self.padding(payload.len()) of them)
    pub fn seal(&mut self, seq: u32, payload: &[u8], pad: &[u8]) -> Vec<u8> {
        let plen = 1 + payload.len() + pad.len();
        let mut pkt = Vec::with_capacity(4 + plen + 32);
        pkt.extend_from_slice(&(plen as u32).to_be_bytes());
        pkt.push(pad.len() as u8);
        pkt.extend_from_slice(payload);
        pkt.extend_from_slice(pad);
        match self {
            Cipher::None => {}
            Cipher::ChaCha { main, header } => {
                let n = cha_nonce(seq);
                chacha::xor(header, 0, &n, &mut pkt[..4]);
                chacha::xor(main, 1, &n, &mut pkt[4..]);
                let b = chacha::block(main, 0, &n);
                let mut pk = [0u8; 32];
                pk.copy_from_slice(&b[..32]);
                let tag = poly1305::poly1305(&pk, &pkt);
                pkt.extend_from_slice(&tag);
            }
            Cipher::AesCtr { aes, ctr, mac } => {
                let mut m = Vec::with_capacity(4 + pkt.len());
                m.extend_from_slice(&seq.to_be_bytes());
                m.extend_from_slice(&pkt);
                let tag = crypto::hmac_sha256(&mac[..], &m);
                ctr_xor(aes, ctr, &mut pkt);
                pkt.extend_from_slice(&tag);
            }
        }
        pkt
    }

    /// the next packet of `rx`: its payload when it is here whole and good (the bytes go), None
    /// when more must come; Err: a reason code and a text.  `head` keeps a first block
    /// decrypted already (AES-CTR) between two calls
    pub fn open(&mut self, seq: u32, rx: &mut Vec<u8>, head: &mut Option<[u8; 16]>) -> Result<Option<Vec<u8>>, (u32, &'static str)> {
        let (len, total) = match self {
            Cipher::None => {
                if rx.len() < 4 {
                    return Ok(None);
                }
                let l = u32::from_be_bytes([rx[0], rx[1], rx[2], rx[3]]) as usize;
                (l, 4 + l)
            }
            Cipher::ChaCha { header, .. } => {
                if rx.len() < 4 {
                    return Ok(None);
                }
                let mut h = [rx[0], rx[1], rx[2], rx[3]];
                chacha::xor(header, 0, &cha_nonce(seq), &mut h);
                let l = u32::from_be_bytes(h) as usize;
                (l, 4 + l + 16)
            }
            Cipher::AesCtr { aes, ctr, .. } => {
                if head.is_none() {
                    if rx.len() < 16 {
                        return Ok(None);
                    }
                    let mut h = [0u8; 16];
                    h.copy_from_slice(&rx[..16]);
                    ctr_xor(aes, ctr, &mut h);
                    *head = Some(h);
                }
                let h = head.unwrap_or([0; 16]);
                let l = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as usize;
                if (4 + l) % 16 != 0 {
                    return Err((DISC_PROTOCOL_ERROR, "bad packet length"));
                }
                (l, 4 + l + 32)
            }
        };
        if !(5..=MAX_PACKET).contains(&len) {
            return Err((DISC_PROTOCOL_ERROR, "bad packet length"));
        }
        if rx.len() < total {
            return Ok(None);
        }
        let mut body: Vec<u8> = rx.drain(..total).collect();
        // the plain packet: length, padding length, payload, padding
        match self {
            Cipher::None => {}
            Cipher::ChaCha { main, .. } => {
                let n = cha_nonce(seq);
                let b = chacha::block(main, 0, &n);
                let mut pk = [0u8; 32];
                pk.copy_from_slice(&b[..32]);
                let tag = poly1305::poly1305(&pk, &body[..4 + len]);
                if !crypto::ct_eq(&tag, &body[4 + len..]) {
                    return Err((DISC_MAC_ERROR, "packet authentication failed"));
                }
                chacha::xor(main, 1, &n, &mut body[4..4 + len]);
            }
            Cipher::AesCtr { aes, ctr, mac } => {
                let h = head.take().unwrap_or([0; 16]);
                body[..16].copy_from_slice(&h);
                ctr_xor(aes, ctr, &mut body[16..4 + len]);
                let mut m = Vec::with_capacity(4 + 4 + len);
                m.extend_from_slice(&seq.to_be_bytes());
                m.extend_from_slice(&body[..4 + len]);
                let tag = crypto::hmac_sha256(&mac[..], &m);
                if !crypto::ct_eq(&tag, &body[4 + len..]) {
                    return Err((DISC_MAC_ERROR, "packet authentication failed"));
                }
            }
        }
        let pad = body[4] as usize;
        if pad < 4 || pad + 1 >= len {
            return Err((DISC_PROTOCOL_ERROR, "bad padding"));
        }
        Ok(Some(body[5..4 + len - pad].to_vec()))
    }
}

/// the length of the key of a cipher
pub fn key_len(enc: &str) -> usize {
    match enc {
        CHACHA => 64,
        AES128 => 16,
        _ => 32,
    }
}

/// the original ChaCha20's nonce, the 64-bit sequence number (it equals the IETF form whose
/// first nonce word is zero, as long as the block counter stays below 2^32)
fn cha_nonce(seq: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[8..].copy_from_slice(&seq.to_be_bytes());
    n
}

/// CTR over whole blocks
fn ctr_xor(aes: &Aes, ctr: &mut [u8; 16], data: &mut [u8]) {
    for block in data.chunks_mut(16) {
        let mut ks = *ctr;
        aes.encrypt(&mut ks);
        for (b, k) in block.iter_mut().zip(ks) {
            *b ^= k;
        }
        for x in ctr.iter_mut().rev() {
            *x = x.wrapping_add(1);
            if *x != 0 {
                break;
            }
        }
    }
}

/// the random numbers of a connection: ChaCha20 from the caller's seed
#[derive(Clone)]
struct Drbg {
    key: [u8; 32],
    ctr: u64,
}

impl Drbg {
    fn fill(&mut self, out: &mut [u8]) {
        let mut n = [0u8; 12];
        for part in out.chunks_mut(64) {
            n[4..].copy_from_slice(&self.ctr.to_be_bytes());
            let b = chacha::block(&self.key, 0, &n);
            part.copy_from_slice(&b[..part.len()]);
            self.ctr += 1;
        }
    }
}

// ---------------------------------------------------------------- what the embedder sees

/// how a login attempt came
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    /// "none": a client asks which methods there are
    None,
    Password(String),
    /// "publickey": `signed` false is a question (auth_pk_ok answers it); else `valid` says the
    /// signature is good for this key
    PublicKey { alg: String, blob: Vec<u8>, signed: bool, valid: bool },
}

/// a channel request
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Pty { term: String, cols: u32, rows: u32 },
    Shell,
    Exec(String),
    Env { name: String, value: String },
    WindowChange { cols: u32, rows: u32 },
    ExitStatus(u32),
    ExitSignal(String),
    /// any other request by its name
    Other(String),
}

/// what happened on a connection
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// the peer's identification line came
    PeerIdent(String),
    /// the algorithms are agreed
    Algos(Algos),
    /// client: the server's key signed the exchange; host_key_verdict goes on (or does not)
    HostKey { alg: String, blob: Vec<u8> },
    /// new keys are in force both ways
    Encrypted,
    /// client: the service ssh-userauth is there, a login may start
    AuthReady,
    /// client: a text from the server to show before the login
    Banner(String),
    /// client: logged in
    AuthSuccess,
    /// client: that failed; the methods that can go on
    AuthFailure { methods: Vec<String>, partial: bool },
    /// server: a login attempt; auth_accept, auth_reject or (for a question) auth_pk_ok answer
    AuthRequest { user: String, method: AuthMethod },
    /// a peer asks for a channel: accept_channel or refuse_channel
    ChannelOpen { channel: u32, kind: String },
    /// the peer took the channel of open_session
    ChannelOpened { channel: u32 },
    ChannelRefused { channel: u32, reason: u32, text: String },
    ChannelRequest { channel: u32, request: Request, want_reply: bool },
    /// the answer to a request with want_reply
    ChannelReply { channel: u32, ok: bool },
    /// data on a channel (ext: Some(1) is stderr)
    Data { channel: u32, ext: Option<u32>, data: Vec<u8> },
    Eof { channel: u32 },
    /// both sides closed the channel
    Closed { channel: u32 },
    /// SSH_MSG_DEBUG with display set
    Debug(String),
    /// the peer does not know our packet of that number
    Unimplemented(u32),
    /// a later key exchange is done (the keys changed, the session goes on)
    Rekeyed,
    /// the peer ended the connection
    Disconnected { reason: u32, text: String },
    /// the peer broke the protocol, the MAC was wrong or no algorithms are common: we told it so
    Failed { reason: u32, text: String },
}

/// what a connection is about
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Client,
    Server,
}

/// the settings of a connection
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    /// the identification line without its line end ("SSH-2.0-...")
    pub ident: String,
    /// in order of preference
    pub ciphers: Vec<String>,
    /// client: the host key algorithms it takes
    pub host_key_algs: Vec<String>,
    /// server: what a failed login names ("publickey,password")
    pub methods: String,
}

impl Default for Config {
    fn default() -> Config {
        let v = |l: &[&str]| l.iter().map(|s| String::from(*s)).collect::<Vec<String>>();
        Config { ident: String::from("SSH-2.0-netpkt"), ciphers: v(&[CHACHA, AES128, AES256]), host_key_algs: v(&[ED25519, RSA_SHA2]), methods: String::from("publickey,password") }
    }
}

// ---------------------------------------------------------------- the core

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kex {
    /// no exchange runs
    Idle,
    /// waiting for the peer's KEXINIT
    Init,
    /// waiting for the ECDH message
    Ecdh,
    /// client: waiting for host_key_verdict
    Verdict,
    /// waiting for the peer's NEWKEYS
    NewKeys,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// waiting for the identification line
    Ident,
    /// the first key exchange
    Kex,
    /// client: SSH_MSG_SERVICE_ACCEPT comes; server: SSH_MSG_SERVICE_REQUEST
    Service,
    /// logins
    Auth,
    /// logged in: channels
    Open,
    Dead,
}

#[derive(Clone, Debug)]
struct Chan {
    remote: u32,
    /// the peer took it (or we took it)
    open: bool,
    win_out: u64,
    max_out: u32,
    consumed: u32,
    pending: Vec<u8>,
    eof_want: bool,
    eof_sent: bool,
    close_want: bool,
    close_sent: bool,
    close_got: bool,
}

impl Chan {
    fn new() -> Chan {
        Chan { remote: 0, open: false, win_out: 0, max_out: CHUNK, consumed: 0, pending: Vec::new(), eof_want: false, eof_sent: false, close_want: false, close_sent: false, close_got: false }
    }
}

/// one side of an SSH connection
#[derive(Clone)]
pub struct Core {
    role: Role,
    cfg: Config,
    rng: Drbg,
    keys: Vec<KeyPair>,
    phase: Phase,
    kex: Kex,
    rx: Vec<u8>,
    /// the first block of a packet under AES-CTR, decrypted already
    rx_head: Option<[u8; 16]>,
    tx: Vec<u8>,
    events: VecDeque<Event>,
    seq_in: u32,
    seq_out: u32,
    enc: Cipher,
    dec: Cipher,
    /// the MAC and cipher to use when the peer's NEWKEYS comes
    next_dec: Option<Cipher>,
    peer_ident: String,
    /// the KEXINIT payloads of this exchange
    ours: Vec<u8>,
    theirs: Vec<u8>,
    /// a wrongly guessed first key exchange packet of the peer is to be skipped
    skip_guess: bool,
    secret: [u8; 32],
    algos: Algos,
    session_id: Vec<u8>,
    /// client: the host key it was told to take
    host_blob: Vec<u8>,
    /// client: the exchange waits for the verdict: H, the shared secret
    wait_h: Vec<u8>,
    wait_k: Vec<u8>,
    /// messages held back while a later key exchange runs
    held: Vec<Vec<u8>>,
    /// client: the key of the login that runs, and its user
    auth_key: Option<KeyPair>,
    auth_user: String,
    chans: BTreeMap<u32, Chan>,
    /// the channel id the peer's numbers map to, ours
    next_chan: u32,
    authed: bool,
}

impl fmt::Debug for Core {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Core({:?} {:?} {:?} in {} out {} rx {} tx {} ev {} ch {:?})", self.role, self.phase, self.kex, self.seq_in, self.seq_out, self.rx.len(), self.tx.len(), self.events.len(), self.chans)
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        crypto::zeroize(&mut self.secret);
        crypto::zeroize(&mut self.wait_k);
        crypto::zeroize(&mut self.rng.key);
    }
}

impl Core {
    fn new(role: Role, cfg: Config, keys: Vec<KeyPair>, seed: &[u8; 32]) -> Core {
        let mut c = Core {
            role,
            cfg,
            rng: Drbg { key: *seed, ctr: 0 },
            keys,
            phase: Phase::Ident,
            kex: Kex::Idle,
            rx: Vec::new(),
            rx_head: None,
            tx: Vec::new(),
            events: VecDeque::new(),
            seq_in: 0,
            seq_out: 0,
            enc: Cipher::None,
            dec: Cipher::None,
            next_dec: None,
            peer_ident: String::new(),
            ours: Vec::new(),
            theirs: Vec::new(),
            skip_guess: false,
            secret: [0; 32],
            algos: Algos::default(),
            session_id: Vec::new(),
            host_blob: Vec::new(),
            wait_h: Vec::new(),
            wait_k: Vec::new(),
            held: Vec::new(),
            auth_key: None,
            auth_user: String::new(),
            chans: BTreeMap::new(),
            next_chan: 0,
            authed: false,
        };
        c.tx.extend_from_slice(c.cfg.ident.as_bytes());
        c.tx.extend_from_slice(b"\r\n");
        c
    }
    /// a client: what it sends starts with its identification line
    pub fn client(cfg: Config, seed: &[u8; 32]) -> Core {
        Core::new(Role::Client, cfg, Vec::new(), seed)
    }
    /// a server with its host keys
    pub fn server(cfg: Config, keys: Vec<KeyPair>, seed: &[u8; 32]) -> Core {
        Core::new(Role::Server, cfg, keys, seed)
    }

    /// the bytes to send
    pub fn take_out(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.tx)
    }
    /// what happened since the last call
    pub fn take_events(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }
    /// nothing more will come or go (after the last take_out)
    pub fn dead(&self) -> bool {
        self.phase == Phase::Dead
    }
    /// keys are in force both ways
    pub fn encrypted(&self) -> bool {
        !matches!(self.enc, Cipher::None) && !matches!(self.dec, Cipher::None)
    }
    /// a login succeeded
    pub fn authenticated(&self) -> bool {
        self.authed
    }
    /// the algorithms of the last exchange
    pub fn algos(&self) -> &Algos {
        &self.algos
    }
    /// the session's identifier: H of the first exchange
    pub fn session_id(&self) -> &[u8] {
        &self.session_id
    }
    /// the peer's identification line
    pub fn peer_ident(&self) -> &str {
        &self.peer_ident
    }
    /// the bytes it holds (a simulator's memory budget)
    pub fn footprint(&self) -> usize {
        self.rx.len() + self.tx.len() + self.ours.len() + self.theirs.len() + self.keys.len() * 512 + self.chans.values().map(|c| c.pending.len() + 96).sum::<usize>() + self.held.iter().map(|h| h.len() + 24).sum::<usize>() + 1024
    }

    // ------------------------------------------------------------ packets out

    fn random_cookie(&mut self) -> [u8; 16] {
        let mut c = [0u8; 16];
        self.rng.fill(&mut c);
        c
    }

    /// a message, or held back while a key exchange runs (what belongs to the exchange goes)
    fn send_msg(&mut self, payload: Vec<u8>) {
        if self.phase == Phase::Dead {
            return;
        }
        let t = payload.first().copied().unwrap_or(0);
        let transport = t <= MSG_DEBUG || (MSG_KEXINIT..=49).contains(&t);
        if self.kex != Kex::Idle && !transport {
            self.held.push(payload);
            return;
        }
        self.send_packet(&payload);
    }

    fn send_packet(&mut self, payload: &[u8]) {
        let n = self.enc.padding(payload.len());
        let mut padding = [0u8; 32];
        self.rng.fill(&mut padding[..n]);
        let pkt = self.enc.seal(self.seq_out, payload, &padding[..n]);
        self.seq_out = self.seq_out.wrapping_add(1);
        self.tx.extend_from_slice(&pkt);
    }

    /// ends the connection from our side
    pub fn disconnect(&mut self, reason: u32, text: &str) {
        if self.phase == Phase::Dead {
            return;
        }
        // (a disconnect goes out even during an exchange: it is a transport message)
        self.send_packet(&W::msg(MSG_DISCONNECT).u32(reason).str(text).str("").0);
        self.phase = Phase::Dead;
    }

    fn fail(&mut self, reason: u32, text: &str) {
        if self.phase == Phase::Dead {
            return;
        }
        if self.phase == Phase::Ident {
            // no packets yet: a line says it
            self.tx.extend_from_slice(text.as_bytes());
            self.tx.extend_from_slice(b"\r\n");
            self.phase = Phase::Dead;
        } else {
            self.disconnect(reason, text);
        }
        self.events.push_back(Event::Failed { reason, text: String::from(text) });
    }

    // ------------------------------------------------------------ bytes in

    /// what the connection brought
    pub fn push(&mut self, bytes: &[u8]) {
        if self.phase == Phase::Dead {
            return;
        }
        self.rx.extend_from_slice(bytes);
        self.pump();
    }

    fn pump(&mut self) {
        if self.phase == Phase::Ident && !self.read_ident() {
            return;
        }
        // (a client's exchange waits for the verdict: what came meanwhile stays)
        while self.phase != Phase::Dead && self.kex != Kex::Verdict {
            match self.read_packet() {
                Some(p) => self.on_packet(&p),
                None => break,
            }
        }
    }

    /// the identification line: lines before it are skipped (RFC 4253 4.2) -> it is here
    fn read_ident(&mut self) -> bool {
        loop {
            let Some(at) = self.rx.iter().position(|&b| b == b'\n') else {
                if self.rx.len() > 8192 {
                    self.fail(DISC_PROTOCOL_ERROR, "no identification line");
                }
                return false;
            };
            let mut line: Vec<u8> = self.rx.drain(..=at).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if !line.starts_with(b"SSH-") {
                continue;
            }
            let v = String::from_utf8_lossy(&line).into_owned();
            if !(v.starts_with("SSH-2.0-") || v.starts_with("SSH-1.99-")) {
                self.fail(DISC_VERSION_NOT_SUPPORTED, "Protocol major versions differ.");
                return false;
            }
            self.events.push_back(Event::PeerIdent(v.clone()));
            self.peer_ident = v;
            self.phase = Phase::Kex;
            self.start_kex();
            return true;
        }
    }

    /// the next packet's payload, when it is here whole and good
    fn read_packet(&mut self) -> Option<Vec<u8>> {
        match self.dec.open(self.seq_in, &mut self.rx, &mut self.rx_head) {
            Ok(Some(p)) => {
                self.seq_in = self.seq_in.wrapping_add(1);
                Some(p)
            }
            Ok(None) => None,
            Err((reason, text)) => {
                self.fail(reason, text);
                None
            }
        }
    }

    fn on_packet(&mut self, p: &[u8]) {
        let t = p.first().copied().unwrap_or(0);
        let mut r = R::new(p.get(1..).unwrap_or(&[]));
        match t {
            MSG_DISCONNECT => {
                let reason = r.u32().unwrap_or(0);
                let text = String::from(r.str().unwrap_or(""));
                self.events.push_back(Event::Disconnected { reason, text });
                self.phase = Phase::Dead;
                return;
            }
            MSG_IGNORE | MSG_EXT_INFO => return,
            MSG_DEBUG => {
                if r.bool() == Some(true) {
                    self.events.push_back(Event::Debug(String::from(r.str().unwrap_or(""))));
                }
                return;
            }
            MSG_UNIMPLEMENTED => {
                self.events.push_back(Event::Unimplemented(r.u32().unwrap_or(0)));
                return;
            }
            MSG_KEXINIT => {
                self.on_kexinit(p);
                return;
            }
            MSG_NEWKEYS | MSG_KEX_ECDH_INIT | MSG_KEX_ECDH_REPLY => {
                self.on_kex_msg(t, p);
                return;
            }
            _ => {}
        }
        // (a later exchange: what the peer sent before it saw our KEXINIT still counts)
        if self.kex != Kex::Idle && (self.phase == Phase::Kex || self.kex != Kex::Init) {
            self.fail(DISC_PROTOCOL_ERROR, "a message that has no place in a key exchange");
            return;
        }
        match self.phase {
            Phase::Service => self.on_service(t, p),
            Phase::Auth => self.on_auth(t, p),
            Phase::Open => self.on_conn(t, p),
            _ => self.unimplemented(),
        }
    }

    fn unimplemented(&mut self) {
        let s = self.seq_in.wrapping_sub(1);
        self.send_msg(W::msg(MSG_UNIMPLEMENTED).u32(s).0);
    }

    // ------------------------------------------------------------ the key exchange

    fn our_kexinit(&mut self) -> KexInit {
        let v = |l: &[&str]| l.iter().map(|s| String::from(*s)).collect::<Vec<String>>();
        let host_key = match self.role {
            Role::Client => self.cfg.host_key_algs.clone(),
            Role::Server => self.keys.iter().map(|k| String::from(k.alg())).collect(),
        };
        KexInit {
            cookie: self.random_cookie(),
            kex: v(&[KEX, KEX_OLD]),
            host_key,
            enc_cs: self.cfg.ciphers.clone(),
            enc_sc: self.cfg.ciphers.clone(),
            mac_cs: v(&[HMAC256]),
            mac_sc: v(&[HMAC256]),
            comp_cs: v(&["none"]),
            comp_sc: v(&["none"]),
            lang_cs: Vec::new(),
            lang_sc: Vec::new(),
            follows: false,
        }
    }

    /// our KEXINIT goes out; the peer's is awaited
    fn start_kex(&mut self) {
        let k = self.our_kexinit();
        self.ours = k.to_payload();
        self.kex = Kex::Init;
        let p = self.ours.clone();
        self.send_packet(&p);
    }

    /// starts a new key exchange (either side may)
    pub fn rekey(&mut self) {
        if self.kex == Kex::Idle && !matches!(self.phase, Phase::Ident | Phase::Kex | Phase::Dead) {
            self.start_kex();
        }
    }

    fn on_kexinit(&mut self, p: &[u8]) {
        if !matches!(self.kex, Kex::Idle | Kex::Init) {
            self.fail(DISC_PROTOCOL_ERROR, "a second KEXINIT during a key exchange");
            return;
        }
        if self.kex == Kex::Idle {
            self.start_kex();
        }
        let Some(peer) = KexInit::parse(p) else {
            self.fail(DISC_PROTOCOL_ERROR, "a KEXINIT that cannot be read");
            return;
        };
        let Some(me) = KexInit::parse(&self.ours) else {
            return;
        };
        self.theirs = Vec::from(p);
        let (c, s) = if self.role == Role::Client { (&me, &peer) } else { (&peer, &me) };
        let Some(a) = negotiate(c, s) else {
            self.fail(DISC_KEY_EXCHANGE_FAILED, "no matching algorithms");
            return;
        };
        if a.comp_cs != "none" || a.comp_sc != "none" {
            self.fail(DISC_KEY_EXCHANGE_FAILED, "no matching algorithms");
            return;
        }
        // RFC 4253 7: a guess of the peer that was wrong: its next exchange packet does not count
        self.skip_guess = peer.follows && (peer.kex.first() != Some(&a.kex) || peer.host_key.first() != Some(&a.host_key));
        self.events.push_back(Event::Algos(a.clone()));
        self.algos = a;
        self.rng.fill(&mut self.secret);
        self.kex = Kex::Ecdh;
        if self.role == Role::Client {
            let q = x25519::public_key(&self.secret);
            self.send_packet(&W::msg(MSG_KEX_ECDH_INIT).bytes(&q).0);
        }
    }

    fn on_kex_msg(&mut self, t: u8, p: &[u8]) {
        if self.skip_guess && t != MSG_NEWKEYS {
            self.skip_guess = false;
            return;
        }
        match (t, self.kex, self.role) {
            (MSG_KEX_ECDH_INIT, Kex::Ecdh, Role::Server) => self.on_ecdh_init(p),
            (MSG_KEX_ECDH_REPLY, Kex::Ecdh, Role::Client) => self.on_ecdh_reply(p),
            (MSG_NEWKEYS, Kex::NewKeys, _) => self.on_newkeys(),
            _ => self.fail(DISC_PROTOCOL_ERROR, "a key exchange message out of order"),
        }
    }

    fn on_ecdh_init(&mut self, p: &[u8]) {
        let mut r = R::new(&p[1..]);
        let Some(q_c) = r.bytes().filter(|q| q.len() == 32) else {
            self.fail(DISC_KEY_EXCHANGE_FAILED, "a bad ECDH value");
            return;
        };
        let mut q = [0u8; 32];
        q.copy_from_slice(q_c);
        let Some(shared) = x25519::shared(&self.secret, &q) else {
            self.fail(DISC_KEY_EXCHANGE_FAILED, "a bad ECDH value");
            return;
        };
        let q_s = x25519::public_key(&self.secret);
        crypto::zeroize(&mut self.secret);
        let Some(key) = self.keys.iter().find(|k| k.alg() == self.algos.host_key).cloned() else {
            self.fail(DISC_KEY_EXCHANGE_FAILED, "no host key of that algorithm");
            return;
        };
        let k_s = key.blob();
        let h = exchange_hash(&self.peer_ident, &self.cfg.ident, &self.theirs, &self.ours, &k_s, q_c, &q_s, &shared);
        let sig = key.sign(&h);
        self.send_packet(&W::msg(MSG_KEX_ECDH_REPLY).bytes(&k_s).bytes(&q_s).bytes(&sig).0);
        self.send_newkeys(&shared, &h);
    }

    fn on_ecdh_reply(&mut self, p: &[u8]) {
        let mut r = R::new(&p[1..]);
        let (Some(blob), Some(q_s), Some(sig)) = (r.bytes(), r.bytes().filter(|q| q.len() == 32), r.bytes()) else {
            self.fail(DISC_KEY_EXCHANGE_FAILED, "an ECDH reply that cannot be read");
            return;
        };
        let mut q = [0u8; 32];
        q.copy_from_slice(q_s);
        let Some(shared) = x25519::shared(&self.secret, &q) else {
            self.fail(DISC_KEY_EXCHANGE_FAILED, "a bad ECDH value");
            return;
        };
        let q_c = x25519::public_key(&self.secret);
        crypto::zeroize(&mut self.secret);
        let h = exchange_hash(&self.cfg.ident, &self.peer_ident, &self.ours, &self.theirs, blob, &q_c, q_s, &shared);
        let alg = self.algos.host_key.clone();
        if !verify(&alg, blob, &h, sig) {
            self.fail(DISC_KEY_EXCHANGE_FAILED, "the host key did not sign the exchange");
            return;
        }
        // a later exchange with the key of the first needs no second verdict
        if self.host_blob == blob {
            self.send_newkeys(&shared, &h);
            return;
        }
        self.host_blob = Vec::from(blob);
        self.wait_h = Vec::from(&h[..]);
        self.wait_k = Vec::from(&shared[..]);
        self.kex = Kex::Verdict;
        self.events.push_back(Event::HostKey { alg, blob: Vec::from(blob) });
    }

    /// the client's answer to Event::HostKey: the exchange goes on, or the connection ends
    pub fn host_key_verdict(&mut self, ok: bool) {
        if self.kex != Kex::Verdict {
            return;
        }
        if !ok {
            self.kex = Kex::Idle;
            self.disconnect(DISC_HOST_KEY_NOT_VERIFIABLE, "host key verification failed");
            return;
        }
        let k = core::mem::take(&mut self.wait_k);
        let h = core::mem::take(&mut self.wait_h);
        self.send_newkeys(&k, &h);
        let mut k = k;
        crypto::zeroize(&mut k);
        self.pump();
    }

    /// NEWKEYS goes out and our direction changes keys; the peer's waits for its NEWKEYS
    fn send_newkeys(&mut self, k: &[u8], h: &[u8]) {
        if self.session_id.is_empty() {
            self.session_id = Vec::from(h);
        }
        let sid = self.session_id.clone();
        let a = self.algos.clone();
        let (ci, si) = (derive_key(k, h, b'A', &sid, 16), derive_key(k, h, b'B', &sid, 16));
        let (ce, se) = (derive_key(k, h, b'C', &sid, key_len(&a.enc_cs)), derive_key(k, h, b'D', &sid, key_len(&a.enc_sc)));
        let (cm, sm) = (derive_key(k, h, b'E', &sid, 32), derive_key(k, h, b'F', &sid, 32));
        let cs = Cipher::make(&a.enc_cs, &ce, &ci, &cm);
        let sc = Cipher::make(&a.enc_sc, &se, &si, &sm);
        let (mine, peer) = if self.role == Role::Client { (cs, sc) } else { (sc, cs) };
        self.send_packet(&[MSG_NEWKEYS]);
        self.enc = mine;
        self.next_dec = Some(peer);
        self.kex = Kex::NewKeys;
    }

    fn on_newkeys(&mut self) {
        if let Some(d) = self.next_dec.take() {
            self.dec = d;
        }
        self.kex = Kex::Idle;
        self.ours.clear();
        self.theirs.clear();
        if self.phase == Phase::Kex {
            self.events.push_back(Event::Encrypted);
            self.phase = Phase::Service;
            if self.role == Role::Client {
                self.send_msg(W::msg(MSG_SERVICE_REQUEST).str("ssh-userauth").0);
            }
        } else {
            self.events.push_back(Event::Rekeyed);
        }
        for m in core::mem::take(&mut self.held) {
            self.send_packet(&m);
        }
    }

    // ------------------------------------------------------------ the service and logins

    fn on_service(&mut self, t: u8, p: &[u8]) {
        let mut r = R::new(&p[1..]);
        match (t, self.role) {
            (MSG_SERVICE_REQUEST, Role::Server) => {
                let name = String::from(r.str().unwrap_or(""));
                if name != "ssh-userauth" {
                    self.disconnect(DISC_SERVICE_NOT_AVAILABLE, "no such service");
                    return;
                }
                self.send_msg(W::msg(MSG_SERVICE_ACCEPT).str(&name).0);
                self.phase = Phase::Auth;
            }
            (MSG_SERVICE_ACCEPT, Role::Client) => {
                self.phase = Phase::Auth;
                self.events.push_back(Event::AuthReady);
            }
            _ => self.fail(DISC_PROTOCOL_ERROR, "a message before the service"),
        }
    }

    fn on_auth(&mut self, t: u8, p: &[u8]) {
        let mut r = R::new(&p[1..]);
        match (t, self.role) {
            (MSG_USERAUTH_REQUEST, Role::Server) => self.on_auth_request(p),
            (MSG_GLOBAL_REQUEST, _) => self.on_global(p),
            (MSG_USERAUTH_SUCCESS, Role::Client) => {
                self.authed = true;
                self.auth_key = None;
                self.phase = Phase::Open;
                self.events.push_back(Event::AuthSuccess);
            }
            (MSG_USERAUTH_FAILURE, Role::Client) => {
                self.auth_key = None;
                let methods = r.names().unwrap_or_default();
                let partial = r.bool().unwrap_or(false);
                self.events.push_back(Event::AuthFailure { methods, partial });
            }
            (MSG_USERAUTH_BANNER, Role::Client) => {
                self.events.push_back(Event::Banner(String::from(r.str().unwrap_or(""))));
            }
            (MSG_USERAUTH_PK_OK, Role::Client) => {
                // the server would take the key: now with a signature
                let Some(key) = self.auth_key.clone() else {
                    return;
                };
                let user = self.auth_user.clone();
                self.send_key_auth(&user, &key, true);
            }
            _ => self.unimplemented(),
        }
    }

    fn on_auth_request(&mut self, p: &[u8]) {
        let mut r = R::new(&p[1..]);
        let (Some(user), Some(service), Some(method)) = (r.str(), r.str(), r.str()) else {
            self.fail(DISC_PROTOCOL_ERROR, "a login request that cannot be read");
            return;
        };
        let (user, service, method) = (String::from(user), String::from(service), String::from(method));
        if service != "ssh-connection" {
            self.disconnect(DISC_SERVICE_NOT_AVAILABLE, "no such service");
            return;
        }
        let m = match method.as_str() {
            "none" => AuthMethod::None,
            "password" => {
                // a password change (the flag) is not offered here
                if r.bool() != Some(false) {
                    self.auth_reject();
                    return;
                }
                AuthMethod::Password(String::from(r.str().unwrap_or("")))
            }
            "publickey" => {
                let signed = r.bool().unwrap_or(false);
                let (alg, blob) = (String::from(r.str().unwrap_or("")), Vec::from(r.bytes().unwrap_or(&[])));
                if alg != ED25519 && alg != RSA_SHA2 {
                    self.auth_reject();
                    return;
                }
                let valid = signed
                    && r.bytes().is_some_and(|sig| {
                        let data = W::default().bytes(&self.session_id).u8(MSG_USERAUTH_REQUEST).str(&user).str(&service).str("publickey").bool(true).str(&alg).bytes(&blob).0;
                        verify(&alg, &blob, &data, sig)
                    });
                AuthMethod::PublicKey { alg, blob, signed, valid }
            }
            _ => {
                self.auth_reject();
                return;
            }
        };
        self.events.push_back(Event::AuthRequest { user, method: m });
    }

    /// server: the login is good: channels may open
    pub fn auth_accept(&mut self) {
        if self.role == Role::Server && self.phase == Phase::Auth {
            self.send_msg(alloc::vec![MSG_USERAUTH_SUCCESS]);
            self.authed = true;
            self.phase = Phase::Open;
        }
    }
    /// server: the login failed; the client may try another
    pub fn auth_reject(&mut self) {
        if self.role == Role::Server && self.phase == Phase::Auth {
            let m = self.cfg.methods.clone();
            self.send_msg(W::msg(MSG_USERAUTH_FAILURE).str(&m).bool(false).0);
        }
    }
    /// server: the answer to a question about a key: it would be taken
    pub fn auth_pk_ok(&mut self, alg: &str, blob: &[u8]) {
        if self.role == Role::Server && self.phase == Phase::Auth {
            self.send_msg(W::msg(MSG_USERAUTH_PK_OK).str(alg).bytes(blob).0);
        }
    }
    /// server: a text for the client before a login
    pub fn banner(&mut self, text: &str) {
        if self.role == Role::Server && self.phase == Phase::Auth {
            self.send_msg(W::msg(MSG_USERAUTH_BANNER).str(text).str("").0);
        }
    }
    /// server: the methods a failed login names from now on
    pub fn set_methods(&mut self, methods: &str) {
        self.cfg.methods = String::from(methods);
    }

    /// client: a login without credentials (it asks which methods there are)
    pub fn auth_none(&mut self, user: &str) {
        self.send_msg(W::msg(MSG_USERAUTH_REQUEST).str(user).str("ssh-connection").str("none").0);
    }
    /// client: a login with a password
    pub fn auth_password(&mut self, user: &str, password: &str) {
        self.send_msg(W::msg(MSG_USERAUTH_REQUEST).str(user).str("ssh-connection").str("password").bool(false).str(password).0);
    }
    /// client: a login with a key: first the question, the signature when the server would
    /// take it
    pub fn auth_key(&mut self, user: &str, key: &KeyPair) {
        self.auth_key = Some(key.clone());
        self.auth_user = String::from(user);
        self.send_key_auth(user, key, false);
    }
    fn send_key_auth(&mut self, user: &str, key: &KeyPair, sign: bool) {
        let m = W::msg(MSG_USERAUTH_REQUEST).str(user).str("ssh-connection").str("publickey").bool(sign).str(key.alg()).bytes(&key.blob());
        if sign {
            let data = W::default().bytes(&self.session_id).raw(&m.0).0;
            let sig = key.sign(&data);
            self.send_msg(m.bytes(&sig).0);
        } else {
            self.send_msg(m.0);
        }
    }

    // ------------------------------------------------------------ channels

    fn on_global(&mut self, p: &[u8]) {
        let mut r = R::new(&p[1..]);
        r.str();
        if r.bool() == Some(true) {
            self.send_msg(alloc::vec![MSG_REQUEST_FAILURE]);
        }
    }

    fn on_conn(&mut self, t: u8, p: &[u8]) {
        let mut r = R::new(&p[1..]);
        match t {
            MSG_GLOBAL_REQUEST => self.on_global(p),
            MSG_REQUEST_SUCCESS | MSG_REQUEST_FAILURE => {}
            MSG_USERAUTH_BANNER | MSG_USERAUTH_SUCCESS => {}
            MSG_CHANNEL_OPEN => {
                let (Some(kind), Some(remote), Some(win), Some(max)) = (r.str(), r.u32(), r.u32(), r.u32()) else {
                    self.fail(DISC_PROTOCOL_ERROR, "a channel request that cannot be read");
                    return;
                };
                let kind = String::from(kind);
                let id = self.next_chan;
                self.next_chan += 1;
                let mut c = Chan::new();
                c.remote = remote;
                c.win_out = win as u64;
                c.max_out = max.clamp(1, CHUNK);
                self.chans.insert(id, c);
                self.events.push_back(Event::ChannelOpen { channel: id, kind });
            }
            MSG_CHANNEL_OPEN_CONFIRMATION => {
                let (Some(local), Some(remote), Some(win), Some(max)) = (r.u32(), r.u32(), r.u32(), r.u32()) else {
                    self.fail(DISC_PROTOCOL_ERROR, "a confirmation that cannot be read");
                    return;
                };
                if let Some(c) = self.chans.get_mut(&local) {
                    c.remote = remote;
                    c.win_out = win as u64;
                    c.max_out = max.clamp(1, CHUNK);
                    c.open = true;
                    self.events.push_back(Event::ChannelOpened { channel: local });
                    self.flush_chan(local);
                }
            }
            MSG_CHANNEL_OPEN_FAILURE => {
                let (Some(local), Some(reason)) = (r.u32(), r.u32()) else {
                    return;
                };
                let text = String::from(r.str().unwrap_or(""));
                self.chans.remove(&local);
                self.events.push_back(Event::ChannelRefused { channel: local, reason, text });
            }
            MSG_CHANNEL_WINDOW_ADJUST => {
                let (Some(local), Some(n)) = (r.u32(), r.u32()) else {
                    return;
                };
                if let Some(c) = self.chans.get_mut(&local) {
                    c.win_out += n as u64;
                    self.flush_chan(local);
                }
            }
            MSG_CHANNEL_DATA | MSG_CHANNEL_EXTENDED_DATA => {
                let Some(local) = r.u32() else {
                    return;
                };
                let ext = if t == MSG_CHANNEL_EXTENDED_DATA { r.u32() } else { None };
                let Some(d) = r.bytes() else {
                    return;
                };
                let n = d.len() as u32;
                if let Some(c) = self.chans.get_mut(&local) {
                    c.consumed = c.consumed.saturating_add(n);
                    let (remote, adjust) = (c.remote, c.consumed);
                    if adjust >= WINDOW / 2 {
                        c.consumed = 0;
                        self.send_msg(W::msg(MSG_CHANNEL_WINDOW_ADJUST).u32(remote).u32(adjust).0);
                    }
                    self.events.push_back(Event::Data { channel: local, ext, data: Vec::from(d) });
                }
            }
            MSG_CHANNEL_EOF => {
                if let Some(local) = r.u32() {
                    self.events.push_back(Event::Eof { channel: local });
                }
            }
            MSG_CHANNEL_CLOSE => {
                let Some(local) = r.u32() else {
                    return;
                };
                if let Some(c) = self.chans.get_mut(&local) {
                    c.close_got = true;
                    c.close_want = true;
                    c.pending.clear();
                    self.flush_chan(local);
                }
            }
            MSG_CHANNEL_REQUEST => self.on_chan_request(&mut r),
            MSG_CHANNEL_SUCCESS | MSG_CHANNEL_FAILURE => {
                if let Some(local) = r.u32() {
                    self.events.push_back(Event::ChannelReply { channel: local, ok: t == MSG_CHANNEL_SUCCESS });
                }
            }
            _ => self.unimplemented(),
        }
    }

    fn on_chan_request(&mut self, r: &mut R) {
        let (Some(local), Some(name), Some(want)) = (r.u32(), r.str(), r.bool()) else {
            self.fail(DISC_PROTOCOL_ERROR, "a channel request that cannot be read");
            return;
        };
        let request = match name {
            "pty-req" => {
                let term = String::from(r.str().unwrap_or(""));
                let (cols, rows) = (r.u32().unwrap_or(80), r.u32().unwrap_or(24));
                Request::Pty { term, cols, rows }
            }
            "shell" => Request::Shell,
            "exec" => Request::Exec(String::from(r.str().unwrap_or(""))),
            "env" => Request::Env { name: String::from(r.str().unwrap_or("")), value: String::from(r.str().unwrap_or("")) },
            "window-change" => Request::WindowChange { cols: r.u32().unwrap_or(80), rows: r.u32().unwrap_or(24) },
            "exit-status" => Request::ExitStatus(r.u32().unwrap_or(0)),
            "exit-signal" => Request::ExitSignal(String::from(r.str().unwrap_or(""))),
            other => Request::Other(String::from(other)),
        };
        self.events.push_back(Event::ChannelRequest { channel: local, request, want_reply: want });
    }

    /// what the window allows goes out, then EOF and CLOSE when they are due
    fn flush_chan(&mut self, id: u32) {
        let mut msgs = Vec::new();
        let mut gone = false;
        if let Some(c) = self.chans.get_mut(&id) {
            if !c.open && !c.close_got {
                return;
            }
            while !c.pending.is_empty() && c.win_out > 0 {
                let n = c.pending.len().min(c.win_out as usize).min(c.max_out as usize);
                let part: Vec<u8> = c.pending.drain(..n).collect();
                c.win_out -= n as u64;
                msgs.push(W::msg(MSG_CHANNEL_DATA).u32(c.remote).bytes(&part).0);
            }
            if c.pending.is_empty() {
                if c.eof_want && !c.eof_sent && !c.close_got {
                    c.eof_sent = true;
                    msgs.push(W::msg(MSG_CHANNEL_EOF).u32(c.remote).0);
                }
                if c.close_want && !c.close_sent {
                    c.close_sent = true;
                    msgs.push(W::msg(MSG_CHANNEL_CLOSE).u32(c.remote).0);
                }
            }
            gone = c.close_sent && c.close_got;
        }
        for m in msgs {
            self.send_msg(m);
        }
        if gone {
            self.chans.remove(&id);
            self.events.push_back(Event::Closed { channel: id });
        }
    }

    /// client: asks for a session channel -> its id (Event::ChannelOpened or ChannelRefused)
    pub fn open_session(&mut self) -> u32 {
        let id = self.next_chan;
        self.next_chan += 1;
        self.chans.insert(id, Chan::new());
        self.send_msg(W::msg(MSG_CHANNEL_OPEN).str("session").u32(id).u32(WINDOW).u32(CHUNK).0);
        id
    }
    /// takes a channel the peer asked for
    pub fn accept_channel(&mut self, id: u32) {
        let Some(c) = self.chans.get_mut(&id) else {
            return;
        };
        c.open = true;
        let remote = c.remote;
        self.send_msg(W::msg(MSG_CHANNEL_OPEN_CONFIRMATION).u32(remote).u32(id).u32(WINDOW).u32(CHUNK).0);
    }
    /// refuses a channel the peer asked for (reason 1: administratively prohibited, 3: unknown
    /// channel type)
    pub fn refuse_channel(&mut self, id: u32, reason: u32, text: &str) {
        let Some(c) = self.chans.remove(&id) else {
            return;
        };
        self.send_msg(W::msg(MSG_CHANNEL_OPEN_FAILURE).u32(c.remote).u32(reason).str(text).str("").0);
    }
    /// sends a channel request
    pub fn request(&mut self, id: u32, req: &Request, want_reply: bool) {
        let Some(remote) = self.chans.get(&id).map(|c| c.remote) else {
            return;
        };
        let m = W::msg(MSG_CHANNEL_REQUEST).u32(remote);
        let m = match req {
            Request::Pty { term, cols, rows } => m.str("pty-req").bool(want_reply).str(term).u32(*cols).u32(*rows).u32(0).u32(0).bytes(&[0]),
            Request::Shell => m.str("shell").bool(want_reply),
            Request::Exec(c) => m.str("exec").bool(want_reply).str(c),
            Request::Env { name, value } => m.str("env").bool(want_reply).str(name).str(value),
            Request::WindowChange { cols, rows } => m.str("window-change").bool(false).u32(*cols).u32(*rows).u32(0).u32(0),
            Request::ExitStatus(n) => m.str("exit-status").bool(false).u32(*n),
            Request::ExitSignal(s) => m.str("exit-signal").bool(false).str(s).bool(false).str("").str(""),
            Request::Other(n) => m.str(n).bool(want_reply),
        };
        self.send_msg(m.0);
    }
    /// answers a request with want_reply
    pub fn reply(&mut self, id: u32, ok: bool) {
        let Some(remote) = self.chans.get(&id).map(|c| c.remote) else {
            return;
        };
        self.send_msg(W::msg(if ok { MSG_CHANNEL_SUCCESS } else { MSG_CHANNEL_FAILURE }).u32(remote).0);
    }
    /// data on a channel (what its window does not take waits)
    pub fn send(&mut self, id: u32, data: &[u8]) {
        let Some(c) = self.chans.get_mut(&id) else {
            return;
        };
        if c.eof_want || c.close_want {
            return;
        }
        c.pending.extend_from_slice(data);
        self.flush_chan(id);
    }
    /// stderr data on a channel; it goes when nothing waits
    pub fn send_ext(&mut self, id: u32, code: u32, data: &[u8]) {
        let Some(c) = self.chans.get_mut(&id) else {
            return;
        };
        if c.eof_want || c.close_want || !c.pending.is_empty() || !c.open {
            return;
        }
        let n = data.len().min(c.win_out as usize).min(c.max_out as usize);
        c.win_out -= n as u64;
        let remote = c.remote;
        self.send_msg(W::msg(MSG_CHANNEL_EXTENDED_DATA).u32(remote).u32(code).bytes(&data[..n]).0);
    }
    /// no more data from us on this channel
    pub fn eof(&mut self, id: u32) {
        if let Some(c) = self.chans.get_mut(&id) {
            c.eof_want = true;
            self.flush_chan(id);
        }
    }
    /// closes the channel when what waits went out
    pub fn close(&mut self, id: u32) {
        if let Some(c) = self.chans.get_mut(&id) {
            c.close_want = true;
            self.flush_chan(id);
        }
    }
    /// the channel is still there
    pub fn channel_open(&self, id: u32) -> bool {
        self.chans.contains_key(&id)
    }
    /// bytes that wait for a channel's window
    pub fn channel_pending(&self, id: u32) -> usize {
        self.chans.get(&id).map_or(0, |c| c.pending.len())
    }
}

// ---------------------------------------------------------------- for dissectors

/// what is in a TCP segment of an SSH connection
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// an identification line (without its line end)
    Ident { at: usize, len: usize, line: String },
    /// a packet in the clear; `len` its bytes here (fewer than 4 + packet_length when it goes on
    /// in the next segment)
    Packet { at: usize, len: usize, packet_length: u32, padding: u8, code: u8 },
    /// bytes that are no packet in the clear: encrypted, or the rest of a packet
    Opaque { at: usize, len: usize },
}

/// a segment's parts.  A packet in the clear has a length that fits, a padding of at least 4
/// and a known message number; what does not look like one is opaque (an encrypted length
/// looks like one only by rare chance)
pub fn dissect(b: &[u8]) -> Vec<Part> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < b.len() {
        let rest = &b[at..];
        if rest.starts_with(b"SSH-") {
            let end = rest.iter().position(|&c| c == b'\n').map_or(rest.len(), |e| e + 1);
            let line = String::from_utf8_lossy(&rest[..end]).trim_end().into();
            out.push(Part::Ident { at, len: end, line });
            at += end;
            continue;
        }
        if rest.len() >= 6 {
            let plen = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
            let (pad, code) = (rest[4], rest[5]);
            let known = msg_name(code) != "unknown message";
            if (2..=MAX_PACKET as u32).contains(&plen) && pad >= 4 && (pad as u32 + 1) < plen && (plen + 4) % 8 == 0 && known {
                let n = (plen as usize + 4).min(rest.len());
                out.push(Part::Packet { at, len: n, packet_length: plen, padding: pad, code });
                at += n;
                continue;
            }
        }
        out.push(Part::Opaque { at, len: rest.len() });
        break;
    }
    out
}
