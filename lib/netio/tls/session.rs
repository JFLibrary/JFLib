// session -- TLS 1.3 resumption (RFC 8446 2.2, 4.6.1): a server's tickets, a client's sessions,
// and the anti-replay register of 0-RTT (RFC 8446 8)
//
//   Tickets::new()          a server's tickets: stateless, sealed with XChaCha20-Poly1305 under
//                           keys that are made and rotated by themselves (or set_keys, for
//                           servers that share them); lifetime, tickets per handshake, the
//                           replay check of 0-RTT
//   Sessions::new(servers)  a client's tickets, per server: the name, the trust and the client
//                           identity of the connection that got them; each is used once
//
// A ticket holds the resumption PSK and what the server must check before it takes it again:
// the suite, the server name, the ALPN protocol, when the handshake that authenticated the
// client ran, the client's certificates (which a resumption checks again) and the trust that
// checked them.  A session lasts at
// most 7 days from that handshake, however often its tickets are renewed (RFC 8446 4.6.1).

use alloc::collections::{BTreeSet, VecDeque};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use plat::sync::Mutex;

use super::{ClientConfig, Revocation, Trust, Verifier};
use crate::x509::Cert;

/// the longest a ticket is valid and a session lasts from the handshake that authenticated the
/// peer: 7 days (RFC 8446 4.6.1)
pub const MAX_LIFETIME: u32 = 604_800;
/// how far the client's age of a ticket and the server's may differ for 0-RTT (RFC 8446 8.3)
pub const FRESH_WINDOW_MS: i64 = 10_000;
/// the tickets a client keeps per server (the oldest go first)
const PER_SERVER: usize = 8;
/// the ClientHellos of 0-RTT the default replay register keeps (it refuses 0-RTT when full)
const MAX_STRIKES: usize = 100_000;
/// the bytes of client certificates a ticket carries (a longer chain gets no ticket)
const MAX_TICKET_CHAIN: usize = 16 * 1024;

fn clock_ms() -> i64 {
    plat::unix_time_ns().div_euclid(1_000_000)
}

// ------------------------------------------------------------------ client

/// a ticket a client keeps (one connection uses it)
pub(crate) struct Session {
    pub suite: u16,
    pub psk: Vec<u8>,
    pub ticket: Vec<u8>,
    pub age_add: u32,
    pub received_ms: i64,
    pub lifetime_ms: i64,
    /// the 0-RTT bytes the server takes with it (0: none)
    pub max_early: u32,
    /// the protocol of the connection that got it (ALPN), which 0-RTT must keep
    pub alpn: Option<Vec<u8>>,
    /// the server's certificates of the handshake that authenticated it
    pub peer: Vec<Cert>,
    /// when that handshake ran (Unix seconds)
    pub auth_s: i64,
    /// until when the good OCSP response stapled in that handshake counts (Unix seconds): its
    /// Response::expires plus the 5 minutes of clock skew (ocsp::SKEW) a full handshake
    /// allows, but not past a delegated responder's certificate; None: none was checked.  The
    /// session is not offered after it, so a resumption takes the certificate's status no
    /// longer than a full handshake would
    pub status_until: Option<i64>,
    /// keeps the check of Trust::Custom alive while its address is part of the key
    pub _pin: Option<Verifier>,
}

impl Drop for Session {
    fn drop(&mut self) {
        crypto::zeroize(&mut self.psk);
    }
}

/// a client's tickets (ClientConfig::sessions): per server, a connection takes the newest and
/// the server sends new ones.  The key of a server is its name, the trust of the connection and
/// its client identity, so a ticket a connection with Trust::Any got is never offered by one that
/// checks certificates, nor one of a client certificate by a connection without it; and its
/// revocation check, so one of Revocation::Off is never offered by a connection that checks.
/// A ticket whose handshake checked a stapled OCSP response goes when that response does: 5
/// minutes (ocsp::SKEW) after its Response::expires, as a full handshake would still take it,
/// at the end of a delegated responder's certificate at the latest.
/// Shared by threads; in memory only
pub struct Sessions {
    inner: Mutex<Vec<([u8; 32], Vec<Session>)>>,
    servers: usize,
    shift_ms: Mutex<i64>,
}

impl Default for Sessions {
    fn default() -> Sessions {
        Sessions::new(256)
    }
}

impl Sessions {
    /// the tickets of up to `servers` servers (the least recently used go first), 8 each
    pub fn new(servers: usize) -> Sessions {
        Sessions { inner: Mutex::new(Vec::new()), servers: servers.max(1), shift_ms: Mutex::new(0) }
    }

    /// forget every ticket
    pub fn clear(&self) {
        self.inner.lock().clear();
    }

    /// the tickets kept (all servers)
    pub fn len(&self) -> usize {
        self.inner.lock().iter().map(|(_, l)| l.len()).sum()
    }

    /// no ticket kept
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// (tests) move this cache's clock by `ms`
    #[doc(hidden)]
    pub fn test_shift(&self, ms: i64) {
        *self.shift_ms.lock() += ms;
    }

    pub(crate) fn now_ms(&self) -> i64 {
        clock_ms() + *self.shift_ms.lock()
    }

    /// (tests) the 0-RTT limit of every kept ticket, as a client that sends more than its
    /// ticket allows would believe
    #[doc(hidden)]
    pub fn test_max_early(&self, n: u32) {
        self.inner.lock().iter_mut().for_each(|(_, l)| l.iter_mut().for_each(|s| s.max_early = n));
    }

    /// the newest ticket of this server that is still valid (it is gone from the cache then)
    pub(crate) fn take(&self, key: &[u8; 32]) -> Option<Session> {
        let now = self.now_ms();
        let mut g = self.inner.lock();
        let i = g.iter().position(|(k, _)| k == key)?;
        let (k, mut list) = g.remove(i);
        list.retain(|s| valid(s, now));
        let s = list.pop();
        if !list.is_empty() {
            g.push((k, list));
        }
        s
    }

    pub(crate) fn put(&self, key: [u8; 32], s: Session) {
        let mut g = self.inner.lock();
        let mut list = match g.iter().position(|(k, _)| *k == key) {
            Some(i) => g.remove(i).1,
            None => Vec::new(),
        };
        list.push(s);
        if list.len() > PER_SERVER {
            list.remove(0);
        }
        g.push((key, list));
        if g.len() > self.servers {
            g.remove(0);
        }
    }
}

/// a session a client may offer: within the ticket's lifetime, 7 days from the handshake that
/// authenticated the server, the server's certificate not expired, the OCSP response checked in
/// that handshake (if one was) still current
fn valid(s: &Session, now_ms: i64) -> bool {
    let now_s = now_ms.div_euclid(1000);
    now_ms - s.received_ms < s.lifetime_ms
        && now_s - s.auth_s < MAX_LIFETIME as i64
        && s.peer.first().is_none_or(|c| now_s <= c.not_after)
        && s.status_until.is_none_or(|u| now_s <= u)
}

/// what a client's tickets are kept by: the server name, the trust, the revocation check and
/// the client identity
pub(crate) fn session_key(cfg: &ClientConfig) -> [u8; 32] {
    let mut m = Vec::new();
    m.extend_from_slice(cfg.server_name.to_ascii_lowercase().as_bytes());
    m.push(0);
    trust_print(&cfg.trust, &mut m);
    m.push(match cfg.revocation {
        Revocation::Off => b'o',
        Revocation::Stapled => b's',
        Revocation::Required => b'r',
    });
    match cfg.identity.as_ref() {
        Some(id) => {
            m.push(1);
            m.extend_from_slice(&crypto::sha256(id.chain.first().map(|c| c.as_slice()).unwrap_or(&[])));
        }
        None => m.push(0),
    }
    crypto::sha256(&m)
}

/// a trust as bytes: the CAs (SHA-256 of each), the pins, the kind; Trust::Custom by the
/// address of its check
pub(crate) fn trust_print(t: &Trust, m: &mut Vec<u8>) {
    match t {
        Trust::Anchors(cas) => {
            m.push(b'A');
            cas.iter().for_each(|c| m.extend_from_slice(&crypto::sha256(&c.der)));
        }
        Trust::Pinned(pins) => {
            m.push(b'P');
            pins.iter().for_each(|p| m.extend_from_slice(p));
        }
        Trust::Any => m.push(b'Y'),
        Trust::System(extra) => {
            m.push(b'S');
            extra.iter().for_each(|c| m.extend_from_slice(&crypto::sha256(&c.der)));
        }
        Trust::Custom(f) => {
            m.push(b'C');
            m.extend_from_slice(&(Arc::as_ptr(f) as *const () as usize as u64).to_be_bytes());
        }
    }
}

/// SHA-256 of a trust (trust_print)
pub(crate) fn trust_hash(t: &Trust) -> [u8; 32] {
    let mut m = Vec::new();
    trust_print(t, &mut m);
    crypto::sha256(&m)
}

// ------------------------------------------------------------------ server

/// the replay check of 0-RTT (Tickets::replay_check): a value unique to a ClientHello with early
/// data (16 bytes of its PSK binder, which the PSK authenticates) -> true the first time it is
/// seen, false when it was seen in the last 20 seconds (the client's 0-RTT is refused then, not
/// the connection)
pub type ReplayCheck = Arc<dyn Fn(&[u8]) -> bool + Send + Sync>;

struct TicketKey {
    id: [u8; 8],
    key: [u8; 32],
    born_ms: i64,
}

impl Drop for TicketKey {
    fn drop(&mut self) {
        crypto::zeroize(&mut self.key);
    }
}

impl TicketKey {
    fn new(key: [u8; 32], born_ms: i64) -> TicketKey {
        let mut id = [0u8; 8];
        id.copy_from_slice(&crypto::sha256(&[b"jflib tls ticket key ", &key[..]].concat())[..8]);
        TicketKey { id, key, born_ms }
    }
}

struct Keys {
    /// the first seals, all open
    list: Vec<TicketKey>,
    /// made and rotated here (false: Tickets::set_keys)
    auto: bool,
}

/// the ClientHellos of accepted 0-RTT, for twice the freshness window
struct Strikes {
    order: VecDeque<([u8; 16], i64)>,
    seen: BTreeSet<[u8; 16]>,
}

/// a server's session tickets (ServerConfig::tickets).  A ticket is the session's state, sealed
/// with XChaCha20-Poly1305: the server keeps nothing per client.  Its keys are made at random and
/// replaced every lifetime, the previous one still opening the tickets it sealed; servers that
/// share tickets (behind a load balancer) set the same keys with set_keys.  Shared by threads
pub struct Tickets {
    lifetime: u32,
    count: u8,
    keys: Mutex<Keys>,
    replay: Option<ReplayCheck>,
    strikes: Mutex<Strikes>,
    shift_ms: Mutex<i64>,
}

impl Default for Tickets {
    fn default() -> Tickets {
        Tickets::new()
    }
}

impl Tickets {
    /// tickets valid for a day, two per handshake, keys of their own
    pub fn new() -> Tickets {
        Tickets {
            lifetime: 86_400,
            count: 2,
            keys: Mutex::new(Keys { list: Vec::new(), auto: true }),
            replay: None,
            strikes: Mutex::new(Strikes { order: VecDeque::new(), seen: BTreeSet::new() }),
            shift_ms: Mutex::new(0),
        }
    }

    /// how long a ticket is valid (seconds, 1 to 7 days); the keys change at this interval
    pub fn lifetime(mut self, secs: u32) -> Tickets {
        self.lifetime = secs.clamp(1, MAX_LIFETIME);
        self
    }

    /// the tickets a handshake gets (1 to 8): a client uses each once, so one that opens
    /// connections in parallel wants several
    pub fn per_handshake(mut self, n: u8) -> Tickets {
        self.count = n.clamp(1, 8);
        self
    }

    /// the replay check of 0-RTT instead of this process's register: servers that share tickets
    /// must share it too, or a ClientHello replayed to another of them gets its 0-RTT taken
    /// again (RFC 8446 8.2)
    pub fn replay_check(mut self, f: ReplayCheck) -> Tickets {
        self.replay = Some(f);
        self
    }

    /// the keys of servers that share tickets: the first seals, all open (the previous ones,
    /// while tickets they sealed may still come); they no longer change by themselves.  Empty:
    /// keys of their own again
    pub fn set_keys(&self, keys: &[[u8; 32]]) {
        let now = self.now_ms();
        let mut g = self.keys.lock();
        g.list = keys.iter().map(|k| TicketKey::new(*k, now)).collect();
        g.auto = keys.is_empty();
    }

    /// a new key now (keys of their own): the tickets of the previous one are taken for one
    /// more lifetime, older ones no longer
    pub fn rotate(&self) {
        let now = self.now_ms();
        let mut g = self.keys.lock();
        if g.auto {
            g.list.insert(0, TicketKey::new(plat::random_bytes(), now));
            g.list.truncate(2);
        }
    }

    /// (tests) move these tickets' clock by `ms`
    #[doc(hidden)]
    pub fn test_shift(&self, ms: i64) {
        *self.shift_ms.lock() += ms;
    }

    pub(crate) fn now_ms(&self) -> i64 {
        clock_ms() + *self.shift_ms.lock()
    }

    pub(crate) fn lifetime_s(&self) -> u32 {
        self.lifetime
    }

    pub(crate) fn count(&self) -> u8 {
        self.count
    }

    /// keys of their own: a first one, a new one each lifetime, none older than two
    fn due(&self, g: &mut Keys, now: i64) {
        if !g.auto {
            return;
        }
        let life = self.lifetime as i64 * 1000;
        if g.list.first().is_none_or(|k| now - k.born_ms >= life) {
            g.list.insert(0, TicketKey::new(plat::random_bytes(), now));
        }
        g.list.retain(|k| now - k.born_ms < 2 * life);
    }

    pub(crate) fn seal(&self, t: &TicketData) -> Option<Vec<u8>> {
        let now = self.now_ms();
        let mut g = self.keys.lock();
        self.due(&mut g, now);
        let k = g.list.first()?;
        let nonce: [u8; 24] = plat::random_bytes();
        let mut plain = t.encode()?;
        let sealed = crypto::aead::x_seal(&k.key, &nonce, &k.id, &plain);
        crypto::zeroize(&mut plain);
        let mut out = Vec::with_capacity(8 + 24 + sealed.len());
        out.extend_from_slice(&k.id);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&sealed);
        Some(out)
    }

    /// a ticket of ours, opened; anything else (another server's, an old key's, a changed one)
    /// gives None
    pub(crate) fn open(&self, ticket: &[u8]) -> Option<TicketData> {
        if ticket.len() < 8 + 24 + 16 {
            return None;
        }
        let now = self.now_ms();
        let mut plain = {
            let mut g = self.keys.lock();
            self.due(&mut g, now);
            let k = g.list.iter().find(|k| k.id[..] == ticket[..8])?;
            let mut nonce = [0u8; 24];
            nonce.copy_from_slice(&ticket[8..32]);
            crypto::aead::x_open(&k.key, &nonce, &k.id, &ticket[32..])?
        };
        let t = TicketData::decode(&plain);
        crypto::zeroize(&mut plain);
        t
    }

    /// the replay check: the program's, or the register of this process
    pub(crate) fn first_time(&self, id: &[u8]) -> bool {
        if let Some(f) = &self.replay {
            return f(id);
        }
        let mut key = [0u8; 16];
        key.copy_from_slice(&id[..16]);
        let now = self.now_ms();
        let mut g = self.strikes.lock();
        while g.order.front().is_some_and(|(_, at)| now - *at > 2 * FRESH_WINDOW_MS) {
            if let Some((k, _)) = g.order.pop_front() {
                g.seen.remove(&k);
            }
        }
        if g.seen.contains(&key) || g.seen.len() >= MAX_STRIKES {
            return false;
        }
        g.seen.insert(key);
        g.order.push_back((key, now));
        true
    }
}

/// what a ticket holds
pub(crate) struct TicketData {
    pub suite: u16,
    pub psk: Vec<u8>,
    pub issued_ms: i64,
    /// when the handshake that authenticated the client ran (Unix seconds)
    pub auth_s: i64,
    pub age_add: u32,
    pub lifetime_s: u32,
    pub max_early: u32,
    pub sni: String,
    pub alpn: Option<Vec<u8>>,
    /// SHA-256 of the trust that checked the client's certificates (trust_hash), zeros without
    pub client_trust: [u8; 32],
    /// the client's certificates (DER, leaf first; the leaf only when they are long)
    pub chain: Vec<Vec<u8>>,
}

impl Drop for TicketData {
    fn drop(&mut self) {
        crypto::zeroize(&mut self.psk);
    }
}

const TICKET_FORMAT: u8 = 1;

impl TicketData {
    /// None: the client's certificates are too long for a ticket (it holds all or none, since
    /// a resumption checks them again)
    fn encode(&self) -> Option<Vec<u8>> {
        let total: usize = self.chain.iter().map(|c| c.len()).sum();
        if total > MAX_TICKET_CHAIN || self.chain.len() > 255 {
            return None;
        }
        let mut b = Vec::new();
        b.push(TICKET_FORMAT);
        b.extend_from_slice(&self.suite.to_be_bytes());
        b.push(self.psk.len() as u8);
        b.extend_from_slice(&self.psk);
        b.extend_from_slice(&self.issued_ms.to_be_bytes());
        b.extend_from_slice(&self.auth_s.to_be_bytes());
        b.extend_from_slice(&self.age_add.to_be_bytes());
        b.extend_from_slice(&self.lifetime_s.to_be_bytes());
        b.extend_from_slice(&self.max_early.to_be_bytes());
        b.push(self.sni.len().min(255) as u8);
        b.extend_from_slice(&self.sni.as_bytes()[..self.sni.len().min(255)]);
        match &self.alpn {
            Some(a) => {
                b.push(1);
                b.push(a.len() as u8);
                b.extend_from_slice(a);
            }
            None => b.push(0),
        }
        b.extend_from_slice(&self.client_trust);
        b.push(self.chain.len() as u8);
        for c in &self.chain {
            b.extend_from_slice(&(c.len() as u32).to_be_bytes());
            b.extend_from_slice(c);
        }
        Some(b)
    }

    fn decode(b: &[u8]) -> Option<TicketData> {
        let mut r = Rd { b };
        if r.u8()? != TICKET_FORMAT {
            return None;
        }
        let suite = u16::from_be_bytes(r.take(2)?.try_into().ok()?);
        let n = r.u8()? as usize;
        let psk = Vec::from(r.take(n)?);
        let issued_ms = i64::from_be_bytes(r.take(8)?.try_into().ok()?);
        let auth_s = i64::from_be_bytes(r.take(8)?.try_into().ok()?);
        let age_add = r.u32()?;
        let lifetime_s = r.u32()?;
        let max_early = r.u32()?;
        let n = r.u8()? as usize;
        let sni = String::from(core::str::from_utf8(r.take(n)?).ok()?);
        let alpn = match r.u8()? {
            0 => None,
            _ => {
                let n = r.u8()? as usize;
                Some(Vec::from(r.take(n)?))
            }
        };
        let client_trust: [u8; 32] = r.take(32)?.try_into().ok()?;
        let mut chain = Vec::new();
        for _ in 0..r.u8()? {
            let n = r.u32()? as usize;
            chain.push(Vec::from(r.take(n)?));
        }
        if !r.b.is_empty() {
            return None;
        }
        Some(TicketData { suite, psk, issued_ms, auth_s, age_add, lifetime_s, max_early, sni, alpn, client_trust, chain })
    }
}

struct Rd<'a> {
    b: &'a [u8],
}

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.b.len() < n {
            return None;
        }
        let (a, rest) = self.b.split_at(n);
        self.b = rest;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }
}
