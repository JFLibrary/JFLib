// seal -- the keys of the UDP handshake and the sealing of its packets (docs/TRANSPORT.md
// "Verschlüsselung")
//
//   derive(&Handshake, &dh1, dh2) -> Derived { c2s, s2c, confirm }   the keys of a connection
//   confirm_tag(&confirm, accept_bytes) -> [u8; 16]                  ACCEPT's proof of the keys
//   Keys::new(send, recv)        one side's keys (Conn::with_keys)
//   full_pn(largest, seq) -> u64 the packet number a 16-bit sequence number stands for
//   nonce(pn) -> [u8; 12]        ChaCha20-Poly1305's nonce of a packet number
//   Generator::new(seed) / next_secret() -> [u8; 32]   the server's ephemeral secrets (forward secure)
//
// Both sides have an ephemeral X25519 key per connection; the client may also know the server's
// static key (ClientConfig::server_key), as in Noise's NK pattern.  The transcript hash h is
// SHA-256 over a label, the protocol id, the nonce, the flags and the public keys (the client's,
// the server's ephemeral one, the static one if used); the keys are HKDF-SHA256 with salt h over
// dh1 (ephemeral with ephemeral) and dh2 (the client's ephemeral with the static key): 32 bytes
// client to server, 32 bytes server to client, 32 bytes for `confirm`.  Without a static key the
// channel is safe from those who only listen and from blind senders, not from someone on the path
// who answers the handshake in the server's place; with it, only the holder of the static secret
// can make an ACCEPT whose confirm checks.
//
// Every sealed packet takes the next packet number of its direction (u64, from 0), and its 16-bit
// sequence number is that number's low bits.  The receiver takes the number closest to the one
// after the largest it authenticated (as QUIC does with its truncated numbers, RFC 9000 A.3), so
// the nonce never repeats under one key and a replay of an old packet fails its tag or is found
// by the receive window of conn.rs.

use crypto::hmac::{hkdf_sha256, hmac_sha256};
use crypto::Sha256;

/// one side's keys of a connection; zeroed when dropped
pub struct Keys {
    pub(crate) send: [u8; 32],
    pub(crate) recv: [u8; 32],
}

impl Keys {
    /// the key this side seals with and the one it opens with
    pub fn new(send: [u8; 32], recv: [u8; 32]) -> Keys {
        Keys { send, recv }
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        crypto::zeroize(&mut self.send);
        crypto::zeroize(&mut self.recv);
    }
}

/// what the transcript hash covers: the handshake as both sides saw it
pub struct Handshake<'a> {
    pub protocol: u32,
    pub nonce: u64,
    /// the client's ephemeral public key (RESPONSE)
    pub client: &'a [u8; 32],
    /// the server's ephemeral public key (ACCEPT)
    pub server: &'a [u8; 32],
    /// the server's static public key, when the client uses it (FLAG_STATIC_KEY)
    pub stat: Option<&'a [u8; 32]>,
}

/// the keys of one connection; zeroed when dropped
pub struct Derived {
    /// client to server
    pub c2s: [u8; 32],
    /// server to client
    pub s2c: [u8; 32],
    /// the key of ACCEPT's confirm
    pub confirm: [u8; 32],
}

impl Drop for Derived {
    fn drop(&mut self) {
        crypto::zeroize(&mut self.c2s);
        crypto::zeroize(&mut self.s2c);
        crypto::zeroize(&mut self.confirm);
    }
}

/// the keys of a connection: `dh1` = X25519(ephemeral, ephemeral), `dh2` = X25519(the client's
/// ephemeral, the static key) when hs.stat is set (both must be given together)
pub fn derive(hs: &Handshake<'_>, dh1: &[u8; 32], dh2: Option<&[u8; 32]>) -> Derived {
    let mut h = Sha256::new();
    h.update(b"netchan v2 handshake");
    h.update(&hs.protocol.to_le_bytes());
    h.update(&hs.nonce.to_le_bytes());
    h.update(&[if hs.stat.is_some() { crate::packet::FLAG_STATIC_KEY } else { 0 }]);
    h.update(hs.client);
    h.update(hs.server);
    h.update(hs.stat.unwrap_or(&[0; 32]));
    let salt = h.finalize();
    let mut ikm = [0u8; 64];
    ikm[..32].copy_from_slice(dh1);
    let n = match dh2 {
        Some(d) => {
            ikm[32..].copy_from_slice(d);
            64
        }
        None => 32,
    };
    let mut okm = [0u8; 96];
    hkdf_sha256(&salt, &ikm[..n], b"netchan v2 keys", &mut okm);
    let mut d = Derived { c2s: [0; 32], s2c: [0; 32], confirm: [0; 32] };
    d.c2s.copy_from_slice(&okm[..32]);
    d.s2c.copy_from_slice(&okm[32..64]);
    d.confirm.copy_from_slice(&okm[64..]);
    crypto::zeroize(&mut ikm);
    crypto::zeroize(&mut okm);
    d
}

/// ACCEPT's confirm: the first 16 bytes of HMAC-SHA256 under the confirm key over the ACCEPT
/// before it (kind, nonce, token, max payload, the server's key)
pub fn confirm_tag(key: &[u8; 32], accept: &[u8]) -> [u8; 16] {
    let m = hmac_sha256(key, accept);
    let mut t = [0u8; 16];
    t.copy_from_slice(&m[..16]);
    t
}

/// the nonce of packet number `pn` (the keys differ per direction, so both may count from 0)
pub fn nonce(pn: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&pn.to_le_bytes());
    n
}

/// the packet number that sequence number `seq` stands for: the one closest to the number after
/// `largest`, the largest authenticated so far (None: nothing yet, so the next is 0)
pub fn full_pn(largest: Option<u64>, seq: u16) -> u64 {
    const WIN: u64 = 1 << 16;
    const HALF: u64 = WIN / 2;
    let expected = largest.map_or(0, |l| l.saturating_add(1));
    let cand = (expected & !(WIN - 1)) | seq as u64;
    if cand.saturating_add(HALF) <= expected && cand < u64::MAX - WIN {
        cand + WIN
    } else if cand > expected.saturating_add(HALF) && cand >= WIN {
        cand - WIN
    } else {
        cand
    }
}

/// a forward-secure source of the server's ephemeral secrets: each one comes from the state,
/// which is replaced by a hash of itself, so what leaks later does not give the earlier ones.
/// Seeded once with 32 random bytes (fixed in tests: the same keys on every run)
pub struct Generator {
    state: [u8; 32],
}

impl Generator {
    /// a generator from 32 secret bytes
    pub fn new(seed: &[u8; 32]) -> Generator {
        Generator { state: hmac_sha256(seed, b"netchan v2 generator") }
    }

    /// the next 32 secret bytes
    pub fn next_secret(&mut self) -> [u8; 32] {
        let out = hmac_sha256(&self.state, b"out");
        let mut next = hmac_sha256(&self.state, b"next");
        self.state.copy_from_slice(&next);
        crypto::zeroize(&mut next);
        out
    }
}

impl Drop for Generator {
    fn drop(&mut self) {
        crypto::zeroize(&mut self.state);
    }
}
