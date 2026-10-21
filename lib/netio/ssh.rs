// ssh -- SSH-2 for running commands on a machine (RFC 4253 / 4252 / 4254)
//
//   Client::connect(stream, HostKeyCheck) -> Client      version, key exchange, host key check
//   client.host_key()                                    type, blob, "SHA256:..." fingerprint
//   client.auth_password(user, pw) / auth_key(user, &Keypair)
//   client.exec(command, stdin, max_out) -> Exec { status, stdout, stderr }
//   Transport::accept(stream, &host_key)                 the server side of the transport
//                                                        (tests drive it with OpenSSH's client)
//   private_key(text) -> Keypair      an unencrypted Ed25519 key ("OPENSSH PRIVATE KEY")
//   public_key_line(&Keypair, comment) -> "ssh-ed25519 AAAA... comment" (authorized_keys)
//   fingerprint(blob) -> "SHA256:..." the way ssh-keygen -l shows it
//
// Algorithms: curve25519-sha256 (also @libssh.org) with OpenSSH's strict key exchange (against
// the Terrapin prefix truncation), ssh-ed25519 host and user keys, chacha20-poly1305@openssh.com.
// The host key is checked by the caller's rule: HostKeyCheck::Pinned(fingerprint) or Any (only
// to learn the fingerprint).  Not here: RSA / ECDSA keys, compression, port forwarding, PTYs,
// rekeying (a peer that starts a new key exchange ends the session; commands here are short).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crypto::ed25519::{self, Keypair};
use crypto::{chacha, poly1305, x25519};
use plat::net::NetError;

use crate::Stream;

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

const VERSION: &str = "SSH-2.0-CraftLIB_netio";
const MAX_PACKET: usize = 256 * 1024;
const CIPHER: &str = "chacha20-poly1305@openssh.com";
const HOST_KEY_ALG: &str = "ssh-ed25519";
/// our receive window and largest packet
const WINDOW: u32 = 2 << 20;
const CHUNK: u32 = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SshError {
    Net(NetError),
    /// the peer broke the protocol or offers nothing we speak
    Protocol(String),
    /// the host key is not the expected one (the fingerprint it showed)
    HostKey(String),
    /// the user name / password / key was refused
    Auth,
    /// the peer ended the session (its reason)
    Disconnected(String),
}

impl core::fmt::Display for SshError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SshError::Net(e) => write!(f, "{}", e),
            SshError::Protocol(s) => write!(f, "SSH: {}", s),
            SshError::HostKey(fp) => write!(f, "SSH: unexpected host key {}", fp),
            SshError::Auth => f.write_str("SSH: authentication refused"),
            SshError::Disconnected(s) => write!(f, "SSH: disconnected: {}", s),
        }
    }
}

impl From<NetError> for SshError {
    fn from(e: NetError) -> Self {
        SshError::Net(e)
    }
}

fn proto<T>(s: &str) -> Result<T, SshError> {
    Err(SshError::Protocol(String::from(s)))
}

// ------------------------------------------------------------------ wire format

/// an SSH message under construction
#[derive(Default)]
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
    /// an unsigned big-endian integer as mpint
    pub fn mpint(self, v: &[u8]) -> W {
        let mut i = 0;
        while i < v.len() && v[i] == 0 {
            i += 1;
        }
        let v = &v[i..];
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

/// reading an SSH message
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
        let s = self.b.get(self.p..self.p + 4)?;
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
    pub fn skip(&mut self, n: usize) -> Option<()> {
        self.b.get(self.p..self.p + n)?;
        self.p += n;
        Some(())
    }
    pub fn rest(&self) -> &'a [u8] {
        &self.b[self.p.min(self.b.len())..]
    }
}

// ------------------------------------------------------------------ keys

/// "SHA256:<base64 without padding>" of a public key blob
pub fn fingerprint(blob: &[u8]) -> String {
    format!("SHA256:{}", crypto::b64::encode_nopad(&crypto::sha256(blob)))
}

fn ed25519_blob(public: &[u8; 32]) -> Vec<u8> {
    W::default().str(HOST_KEY_ALG).bytes(public).0
}

fn ed25519_sig(sig: &[u8; 64]) -> Vec<u8> {
    W::default().str(HOST_KEY_ALG).bytes(sig).0
}

/// "ssh-ed25519 AAAA... comment"
pub fn public_key_line(k: &Keypair, comment: &str) -> String {
    let mut s = format!("{} {}", HOST_KEY_ALG, crypto::b64::encode(&ed25519_blob(&k.public)));
    if !comment.is_empty() {
        s.push(' ');
        s.push_str(comment);
    }
    s
}

/// an unencrypted Ed25519 key in OpenSSH's format ("-----BEGIN OPENSSH PRIVATE KEY-----")
pub fn private_key(text: &str) -> Option<Keypair> {
    let body: String = text.lines().filter(|l| !l.starts_with("-----")).map(|l| l.trim()).collect();
    let b = crypto::b64::decode(&body)?;
    let rest = b.strip_prefix(b"openssh-key-v1\0")?;
    let mut r = R::new(rest);
    if r.str()? != "none" || r.str()? != "none" {
        return None; // encrypted keys are not read here
    }
    r.bytes()?; // kdf options
    if r.u32()? != 1 {
        return None;
    }
    r.bytes()?; // public key
    let mut p = R::new(r.bytes()?);
    if p.u32()? != p.u32()? || p.str()? != HOST_KEY_ALG {
        return None;
    }
    let public = p.bytes()?;
    let secret = p.bytes()?;
    if public.len() != 32 || secret.len() != 64 || &secret[32..] != public {
        return None;
    }
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&secret[..32]);
    let k = Keypair::from_seed(&seed);
    crypto::zeroize(&mut seed);
    (k.public[..] == *public).then_some(k)
}

/// the host key a server showed
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostKey {
    pub key_type: String,
    pub blob: Vec<u8>,
    pub fingerprint: String,
}

#[derive(Clone, Debug)]
pub enum HostKeyCheck {
    /// exactly this key ("SHA256:...")
    Pinned(String),
    /// any key (to read its fingerprint for a decision)
    Any,
}

// ------------------------------------------------------------------ transport

struct ChaPoly {
    main: [u8; 32],
    header: [u8; 32],
}

impl ChaPoly {
    fn new(k: &[u8; 64]) -> ChaPoly {
        let mut main = [0u8; 32];
        let mut header = [0u8; 32];
        main.copy_from_slice(&k[..32]);
        header.copy_from_slice(&k[32..]);
        ChaPoly { main, header }
    }
}

impl Drop for ChaPoly {
    fn drop(&mut self) {
        crypto::zeroize(&mut self.main);
        crypto::zeroize(&mut self.header);
    }
}

/// the 64-bit big-endian sequence number as the cipher's nonce (the original ChaCha20 with a
/// 64-bit counter equals the IETF variant whose first nonce word is zero)
fn nonce(seq: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[8..].copy_from_slice(&seq.to_be_bytes());
    n
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Client,
    Server,
}

/// the encrypted packet stream of one connection
pub struct Transport {
    s: Box<dyn Stream>,
    buf: Vec<u8>,
    seq_out: u32,
    seq_in: u32,
    enc: Option<ChaPoly>,
    dec: Option<ChaPoly>,
    pub session_id: Vec<u8>,
    /// the peer's version line
    pub peer_version: String,
    kex_done: bool,
}

/// the other side's KEXINIT, as far as we need it
struct KexInit {
    kex: Vec<String>,
    host_key: Vec<String>,
    enc_cs: Vec<String>,
    enc_sc: Vec<String>,
    comp_cs: Vec<String>,
    comp_sc: Vec<String>,
    follows: bool,
}

fn names(s: &str) -> Vec<String> {
    s.split(',').filter(|x| !x.is_empty()).map(String::from).collect()
}

fn parse_kexinit(p: &[u8]) -> Option<KexInit> {
    let mut r = R::new(p);
    if r.u8()? != MSG_KEXINIT {
        return None;
    }
    r.skip(16)?;
    let kex = names(r.str()?);
    let host_key = names(r.str()?);
    let enc_cs = names(r.str()?);
    let enc_sc = names(r.str()?);
    r.str()?;
    r.str()?;
    let comp_cs = names(r.str()?);
    let comp_sc = names(r.str()?);
    r.str()?;
    r.str()?;
    let follows = r.bool()?;
    Some(KexInit { kex, host_key, enc_cs, enc_sc, comp_cs, comp_sc, follows })
}

fn kexinit(role: Role) -> Vec<u8> {
    let cookie: [u8; 16] = plat::random_bytes();
    let strict = if role == Role::Client { "kex-strict-c-v00@openssh.com" } else { "kex-strict-s-v00@openssh.com" };
    let kex = format!("curve25519-sha256,curve25519-sha256@libssh.org,{}", strict);
    W::msg(MSG_KEXINIT)
        .raw(&cookie)
        .str(&kex)
        .str(HOST_KEY_ALG)
        .str(CIPHER)
        .str(CIPHER)
        .str("hmac-sha2-256")
        .str("hmac-sha2-256")
        .str("none")
        .str("none")
        .str("")
        .str("")
        .bool(false)
        .u32(0)
        .0
}

/// HASH(K || H || letter || session_id), extended to 64 bytes
fn derive(k: &[u8], h: &[u8], letter: u8, sid: &[u8]) -> [u8; 64] {
    use crypto::Hash;
    let kk = W::default().mpint(k).0;
    let mut a = crypto::Sha256::new();
    a.update(&kk);
    a.update(h);
    a.update(&[letter]);
    a.update(sid);
    let mut out = [0u8; 64];
    a.finish_into(&mut out[..32]);
    let mut b = crypto::Sha256::new();
    b.update(&kk);
    b.update(h);
    b.update(&out[..32]);
    let mut second = [0u8; 32];
    b.finish_into(&mut second);
    out[32..].copy_from_slice(&second);
    out
}

impl Transport {
    fn new(s: Box<dyn Stream>) -> Transport {
        Transport { s, buf: Vec::new(), seq_out: 0, seq_in: 0, enc: None, dec: None, session_id: Vec::new(), peer_version: String::new(), kex_done: false }
    }

    fn fill(&mut self, n: usize) -> Result<(), SshError> {
        let mut tmp = [0u8; 16384];
        while self.buf.len() < n {
            let k = self.s.read(&mut tmp)?;
            if k == 0 {
                return Err(SshError::Disconnected(String::from("connection closed")));
            }
            self.buf.extend_from_slice(&tmp[..k]);
        }
        Ok(())
    }

    fn take(&mut self, n: usize) -> Result<Vec<u8>, SshError> {
        self.fill(n)?;
        Ok(self.buf.drain(..n).collect())
    }

    /// the identification lines: ours out, the peer's in (lines before it are skipped)
    fn versions(&mut self) -> Result<(), SshError> {
        self.s.write_all(format!("{}\r\n", VERSION).as_bytes())?;
        for _ in 0..64 {
            let mut line = Vec::new();
            loop {
                self.fill(1)?;
                let c = self.buf.remove(0);
                if c == b'\n' {
                    break;
                }
                line.push(c);
                if line.len() > 255 {
                    return proto("identification line too long");
                }
            }
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.starts_with(b"SSH-") {
                let v = String::from_utf8_lossy(&line).into_owned();
                if !(v.starts_with("SSH-2.0-") || v.starts_with("SSH-1.99-")) {
                    return proto("not SSH version 2");
                }
                self.peer_version = v;
                return Ok(());
            }
        }
        proto("no SSH identification")
    }

    /// one packet out
    pub fn send(&mut self, payload: &[u8]) -> Result<(), SshError> {
        let aead = self.enc.is_some();
        let counted = 1 + payload.len() + if aead { 0 } else { 4 };
        let mut pad = 8 - counted % 8;
        if pad < 4 {
            pad += 8;
        }
        let plen = 1 + payload.len() + pad;
        let mut pkt = Vec::with_capacity(4 + plen + 16);
        pkt.extend_from_slice(&(plen as u32).to_be_bytes());
        pkt.push(pad as u8);
        pkt.extend_from_slice(payload);
        let mut padding = [0u8; 16];
        plat::fill_random(&mut padding[..pad]);
        pkt.extend_from_slice(&padding[..pad]);
        if let Some(c) = &self.enc {
            let n = nonce(self.seq_out);
            chacha::xor(&c.header, 0, &n, &mut pkt[..4]);
            chacha::xor(&c.main, 1, &n, &mut pkt[4..]);
            let block = chacha::block(&c.main, 0, &n);
            let mut pk = [0u8; 32];
            pk.copy_from_slice(&block[..32]);
            let tag = poly1305::poly1305(&pk, &pkt);
            pkt.extend_from_slice(&tag);
        }
        self.seq_out = self.seq_out.wrapping_add(1);
        self.s.write_all(&pkt)?;
        Ok(())
    }

    /// one packet in (any type)
    fn recv_raw(&mut self) -> Result<Vec<u8>, SshError> {
        let head = self.take(4)?;
        let mut len_b = [head[0], head[1], head[2], head[3]];
        let n = nonce(self.seq_in);
        if let Some(c) = &self.dec {
            chacha::xor(&c.header, 0, &n, &mut len_b);
        }
        let len = u32::from_be_bytes(len_b) as usize;
        if !(5..=MAX_PACKET).contains(&len) {
            return proto("bad packet length");
        }
        let tag_len = if self.dec.is_some() { 16 } else { 0 };
        let mut body = self.take(len + tag_len)?;
        if let Some(c) = &self.dec {
            let block = chacha::block(&c.main, 0, &n);
            let mut pk = [0u8; 32];
            pk.copy_from_slice(&block[..32]);
            let mut p = poly1305::Poly1305::new(&pk);
            p.update(&head);
            p.update(&body[..len]);
            if !crypto::ct_eq(&p.finalize(), &body[len..]) {
                return proto("packet authentication failed");
            }
            body.truncate(len);
            chacha::xor(&c.main, 1, &n, &mut body);
        }
        self.seq_in = self.seq_in.wrapping_add(1);
        let pad = body[0] as usize;
        if pad < 4 || pad + 1 >= len {
            return proto("bad padding");
        }
        Ok(body[1..len - pad].to_vec())
    }

    /// the next message that matters: ignore / debug / extension info are skipped, a disconnect
    /// ends the session, a new key exchange is not supported
    pub fn recv(&mut self) -> Result<Vec<u8>, SshError> {
        loop {
            let m = self.recv_raw()?;
            match m.first().copied() {
                None => return proto("empty message"),
                Some(MSG_DISCONNECT) => {
                    let mut r = R::new(&m[1..]);
                    let _code = r.u32();
                    return Err(SshError::Disconnected(String::from(r.str().unwrap_or(""))));
                }
                Some(MSG_IGNORE | MSG_DEBUG | MSG_EXT_INFO) if self.kex_done => continue,
                Some(MSG_UNIMPLEMENTED) => return proto("the peer did not understand a message"),
                Some(MSG_KEXINIT) if self.kex_done => return proto("a new key exchange is not supported"),
                Some(_) => return Ok(m),
            }
        }
    }

    /// the message after this one must be of type `t`
    fn expect(&mut self, t: u8) -> Result<Vec<u8>, SshError> {
        let m = self.recv()?;
        if m[0] != t {
            return Err(SshError::Protocol(format!("expected message {}, got {}", t, m[0])));
        }
        Ok(m)
    }

    /// the key exchange; `host` signs as a server, `check` decides about the key as a client
    fn kex(&mut self, role: Role, host: Option<&Keypair>, check: &HostKeyCheck) -> Result<Option<HostKey>, SshError> {
        self.versions()?;
        let ours = kexinit(role);
        self.send(&ours)?;
        // strict key exchange: the peer's KEXINIT is its first packet
        let theirs = self.recv_raw()?;
        if theirs.first() != Some(&MSG_KEXINIT) || self.seq_in != 1 {
            return proto("key exchange must start with KEXINIT");
        }
        let k = parse_kexinit(&theirs).ok_or_else(|| SshError::Protocol(String::from("bad KEXINIT")))?;
        let has = |l: &[String], x: &str| l.iter().any(|s| s == x);
        let kex_ok = has(&k.kex, "curve25519-sha256") || has(&k.kex, "curve25519-sha256@libssh.org");
        if !kex_ok || !has(&k.host_key, HOST_KEY_ALG) || !has(&k.enc_cs, CIPHER) || !has(&k.enc_sc, CIPHER) || !has(&k.comp_cs, "none") || !has(&k.comp_sc, "none") {
            return proto("no common algorithms (needs curve25519-sha256, ssh-ed25519, chacha20-poly1305@openssh.com)");
        }
        let strict = has(&k.kex, if role == Role::Client { "kex-strict-s-v00@openssh.com" } else { "kex-strict-c-v00@openssh.com" });
        if k.follows && !k.kex.first().is_some_and(|f| f.starts_with("curve25519-sha256")) {
            self.recv_raw()?; // a wrongly guessed first packet
        }
        let mut secret: [u8; 32] = plat::random_bytes();
        let q_ours = x25519::public_key(&secret);
        let (v_c, v_s, i_c, i_s);
        let (k_s, q_c, q_s, shared, sig_blob, hk);
        match role {
            Role::Client => {
                self.send(&W::msg(MSG_KEX_ECDH_INIT).bytes(&q_ours).0)?;
                let m = self.expect_kex(MSG_KEX_ECDH_REPLY, strict)?;
                let mut r = R::new(&m[1..]);
                let blob = r.bytes().ok_or(SshError::Protocol(String::from("bad ECDH reply")))?.to_vec();
                let qs = r.bytes().filter(|q| q.len() == 32).ok_or(SshError::Protocol(String::from("bad ECDH reply")))?;
                let sig = r.bytes().ok_or(SshError::Protocol(String::from("bad ECDH reply")))?.to_vec();
                let fp = fingerprint(&blob);
                let mut kr = R::new(&blob);
                if kr.str() != Some(HOST_KEY_ALG) || kr.bytes().is_none_or(|p| p.len() != 32) {
                    return proto("host key is not ssh-ed25519");
                }
                if let HostKeyCheck::Pinned(want) = check {
                    if *want != fp {
                        return Err(SshError::HostKey(fp));
                    }
                }
                let mut q = [0u8; 32];
                q.copy_from_slice(qs);
                shared = x25519::shared(&secret, &q).ok_or(SshError::Protocol(String::from("bad ECDH value")))?;
                hk = Some(HostKey { key_type: String::from(HOST_KEY_ALG), blob: blob.clone(), fingerprint: fp });
                v_c = String::from(VERSION);
                v_s = self.peer_version.clone();
                i_c = ours;
                i_s = theirs;
                k_s = blob;
                q_c = q_ours.to_vec();
                q_s = qs.to_vec();
                sig_blob = sig;
            }
            Role::Server => {
                let key = host.ok_or(SshError::Protocol(String::from("no host key")))?;
                let m = self.expect_kex(MSG_KEX_ECDH_INIT, strict)?;
                let mut r = R::new(&m[1..]);
                let qc = r.bytes().filter(|q| q.len() == 32).ok_or(SshError::Protocol(String::from("bad ECDH init")))?;
                let mut q = [0u8; 32];
                q.copy_from_slice(qc);
                shared = x25519::shared(&secret, &q).ok_or(SshError::Protocol(String::from("bad ECDH value")))?;
                hk = None;
                v_c = self.peer_version.clone();
                v_s = String::from(VERSION);
                i_c = theirs;
                i_s = ours;
                k_s = ed25519_blob(&key.public);
                q_c = qc.to_vec();
                q_s = q_ours.to_vec();
                sig_blob = Vec::new();
            }
        }
        crypto::zeroize(&mut secret);
        let h = crypto::sha256(&W::default().str(&v_c).str(&v_s).bytes(&i_c).bytes(&i_s).bytes(&k_s).bytes(&q_c).bytes(&q_s).mpint(&shared).0);
        match role {
            Role::Client => {
                let mut r = R::new(&sig_blob);
                let ok = r.str() == Some(HOST_KEY_ALG)
                    && r.bytes().filter(|s| s.len() == 64).is_some_and(|s| {
                        let mut sig = [0u8; 64];
                        sig.copy_from_slice(s);
                        let mut pk = [0u8; 32];
                        pk.copy_from_slice(&k_s[k_s.len() - 32..]);
                        ed25519::verify(&pk, &h, &sig)
                    });
                if !ok {
                    return proto("host key signature invalid");
                }
            }
            Role::Server => {
                let key = host.ok_or(SshError::Protocol(String::from("no host key")))?;
                let sig = ed25519_sig(&key.sign(&h));
                self.send(&W::msg(MSG_KEX_ECDH_REPLY).bytes(&k_s).bytes(&q_s).bytes(&sig).0)?;
            }
        }
        if self.session_id.is_empty() {
            self.session_id = h.to_vec();
        }
        let sid = self.session_id.clone();
        let (out_letter, in_letter) = if role == Role::Client { (b'C', b'D') } else { (b'D', b'C') };
        let mut ko = derive(&shared, &h, out_letter, &sid);
        let mut ki = derive(&shared, &h, in_letter, &sid);
        self.send(&[MSG_NEWKEYS])?;
        self.enc = Some(ChaPoly::new(&ko));
        if strict {
            self.seq_out = 0;
        }
        self.expect_kex(MSG_NEWKEYS, strict)?;
        self.dec = Some(ChaPoly::new(&ki));
        if strict {
            self.seq_in = 0;
        }
        crypto::zeroize(&mut ko);
        crypto::zeroize(&mut ki);
        self.kex_done = true;
        Ok(hk)
    }

    /// a key exchange message; with the strict exchange nothing else may come in between
    fn expect_kex(&mut self, t: u8, strict: bool) -> Result<Vec<u8>, SshError> {
        loop {
            let m = self.recv()?;
            if m[0] == t {
                return Ok(m);
            }
            if strict || !matches!(m[0], MSG_IGNORE | MSG_DEBUG) {
                return Err(SshError::Protocol(format!("unexpected message {} during the key exchange", m[0])));
            }
        }
    }

    /// the server side: version, key exchange with this host key
    pub fn accept(s: Box<dyn Stream>, host: &Keypair) -> Result<Transport, SshError> {
        let mut t = Transport::new(s);
        t.kex(Role::Server, Some(host), &HostKeyCheck::Any)?;
        Ok(t)
    }

    pub fn disconnect(&mut self, reason: &str) {
        let _ = self.send(&W::msg(MSG_DISCONNECT).u32(11).str(reason).str("").0);
        self.s.shutdown();
    }
}

// ------------------------------------------------------------------ client

/// what a command did
#[derive(Clone, Debug, Default)]
pub struct Exec {
    /// the exit code (None: it ended by a signal or the server did not say)
    pub status: Option<u32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub struct Client {
    t: Transport,
    host_key: HostKey,
    service: bool,
    next_channel: u32,
}

impl Client {
    pub fn connect(s: Box<dyn Stream>, check: HostKeyCheck) -> Result<Client, SshError> {
        let mut t = Transport::new(s);
        let host_key = t.kex(Role::Client, None, &check)?.ok_or(SshError::Protocol(String::from("no host key")))?;
        Ok(Client { t, host_key, service: false, next_channel: 0 })
    }

    pub fn host_key(&self) -> &HostKey {
        &self.host_key
    }

    fn userauth_service(&mut self) -> Result<(), SshError> {
        if !self.service {
            self.t.send(&W::msg(MSG_SERVICE_REQUEST).str("ssh-userauth").0)?;
            self.t.expect(MSG_SERVICE_ACCEPT)?;
            self.service = true;
        }
        Ok(())
    }

    fn auth_answer(&mut self) -> Result<(), SshError> {
        loop {
            let m = self.t.recv()?;
            match m[0] {
                MSG_USERAUTH_SUCCESS => return Ok(()),
                MSG_USERAUTH_FAILURE => return Err(SshError::Auth),
                MSG_USERAUTH_BANNER | MSG_GLOBAL_REQUEST => {}
                t => return Err(SshError::Protocol(format!("unexpected message {} during authentication", t))),
            }
        }
    }

    pub fn auth_password(&mut self, user: &str, password: &str) -> Result<(), SshError> {
        self.userauth_service()?;
        self.t.send(&W::msg(MSG_USERAUTH_REQUEST).str(user).str("ssh-connection").str("password").bool(false).str(password).0)?;
        self.auth_answer()
    }

    pub fn auth_key(&mut self, user: &str, key: &Keypair) -> Result<(), SshError> {
        self.userauth_service()?;
        let blob = ed25519_blob(&key.public);
        let signed = W::default().bytes(&self.t.session_id).u8(MSG_USERAUTH_REQUEST).str(user).str("ssh-connection").str("publickey").bool(true).str(HOST_KEY_ALG).bytes(&blob).0;
        let sig = ed25519_sig(&key.sign(&signed));
        self.t.send(&W::msg(MSG_USERAUTH_REQUEST).str(user).str("ssh-connection").str("publickey").bool(true).str(HOST_KEY_ALG).bytes(&blob).bytes(&sig).0)?;
        self.auth_answer()
    }

    /// run a command, feed it `stdin`, collect at most `max_out` bytes of each output
    pub fn exec(&mut self, command: &str, stdin: &[u8], max_out: usize) -> Result<Exec, SshError> {
        let local = self.next_channel;
        self.next_channel += 1;
        self.t.send(&W::msg(MSG_CHANNEL_OPEN).str("session").u32(local).u32(WINDOW).u32(CHUNK).0)?;
        let (remote, mut rwin, rmax) = loop {
            let m = self.t.recv()?;
            let mut r = R::new(&m[1..]);
            match m[0] {
                MSG_CHANNEL_OPEN_CONFIRMATION => {
                    let bad = || SshError::Protocol(String::from("bad channel confirmation"));
                    if r.u32().ok_or_else(bad)? != local {
                        return proto("confirmation for another channel");
                    }
                    break (r.u32().ok_or_else(bad)?, r.u32().ok_or_else(bad)? as u64, r.u32().ok_or_else(bad)?.max(1));
                }
                MSG_CHANNEL_OPEN_FAILURE => {
                    r.u32();
                    r.u32();
                    return Err(SshError::Protocol(format!("channel refused: {}", r.str().unwrap_or(""))));
                }
                MSG_GLOBAL_REQUEST => self.global_request(&m)?,
                _ => {}
            }
        };
        self.t.send(&W::msg(MSG_CHANNEL_REQUEST).u32(remote).str("exec").bool(true).str(command).0)?;
        let mut out = Exec::default();
        let (mut started, mut sent, mut eof_sent) = (false, 0usize, false);
        let mut consumed = 0u32;
        loop {
            if started && !eof_sent {
                while sent < stdin.len() && rwin > 0 {
                    let n = (stdin.len() - sent).min(rwin as usize).min(rmax as usize).min(CHUNK as usize);
                    self.t.send(&W::msg(MSG_CHANNEL_DATA).u32(remote).bytes(&stdin[sent..sent + n]).0)?;
                    sent += n;
                    rwin -= n as u64;
                }
                if sent == stdin.len() {
                    self.t.send(&W::msg(MSG_CHANNEL_EOF).u32(remote).0)?;
                    eof_sent = true;
                }
            }
            let m = self.t.recv()?;
            let mut r = R::new(&m[1..]);
            let bad = || SshError::Protocol(String::from("bad channel message"));
            match m[0] {
                MSG_CHANNEL_SUCCESS => started = true,
                MSG_CHANNEL_FAILURE if !started => return proto("the server refused to run the command"),
                MSG_CHANNEL_WINDOW_ADJUST => {
                    r.u32();
                    rwin += r.u32().ok_or_else(bad)? as u64;
                }
                MSG_CHANNEL_DATA | MSG_CHANNEL_EXTENDED_DATA => {
                    r.u32();
                    let ext = m[0] == MSG_CHANNEL_EXTENDED_DATA;
                    if ext {
                        r.u32();
                    }
                    let d = r.bytes().ok_or_else(bad)?;
                    let dst = if ext { &mut out.stderr } else { &mut out.stdout };
                    let room = max_out.saturating_sub(dst.len());
                    dst.extend_from_slice(&d[..d.len().min(room)]);
                    consumed += d.len() as u32;
                    if consumed >= WINDOW / 2 {
                        self.t.send(&W::msg(MSG_CHANNEL_WINDOW_ADJUST).u32(remote).u32(consumed).0)?;
                        consumed = 0;
                    }
                }
                MSG_CHANNEL_REQUEST => {
                    r.u32();
                    let name = r.str().unwrap_or("");
                    let want = r.bool().unwrap_or(false);
                    match name {
                        "exit-status" => out.status = r.u32(),
                        "exit-signal" => {
                            out.status = None;
                            let sig = r.str().unwrap_or("");
                            out.stderr.extend_from_slice(format!("\n(signal {})", sig).as_bytes());
                        }
                        _ => {}
                    }
                    if want {
                        self.t.send(&W::msg(MSG_CHANNEL_FAILURE).u32(remote).0)?;
                    }
                }
                MSG_CHANNEL_EOF => {}
                MSG_CHANNEL_CLOSE => {
                    // the server closed the channel: close our side too
                    self.t.send(&W::msg(MSG_CHANNEL_CLOSE).u32(remote).0)?;
                    break;
                }
                MSG_GLOBAL_REQUEST => self.global_request(&m)?,
                _ => {}
            }
        }
        Ok(out)
    }

    fn global_request(&mut self, m: &[u8]) -> Result<(), SshError> {
        let mut r = R::new(&m[1..]);
        r.str();
        if r.bool() == Some(true) {
            self.t.send(&[MSG_REQUEST_FAILURE])?;
        }
        Ok(())
    }

    pub fn close(mut self) {
        self.t.disconnect("bye");
    }
}
