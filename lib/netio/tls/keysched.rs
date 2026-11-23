// keysched -- the TLS 1.3 key schedule (RFC 8446 7.1) with the hash of the cipher suite (SHA-256
// or SHA-384): HKDF-Expand-Label, Derive-Secret, traffic keys and finished keys; the TLS 1.2 PRF
// (RFC 5246 5) with SHA-256 or SHA-384; the running transcript hash of a handshake

use alloc::vec::Vec;

use crypto::hmac::{hkdf_expand, hkdf_extract};
use crypto::{HashAlg, Sha256, Sha384};

/// a secret of the schedule (as long as the hash: 32 or 48 bytes)
pub type Secret = Vec<u8>;

fn expand(h: HashAlg, prk: &[u8], info: &[u8], out: &mut [u8]) {
    match h {
        HashAlg::Sha384 => hkdf_expand::<Sha384>(prk, info, out),
        _ => hkdf_expand::<Sha256>(prk, info, out),
    }
}

/// HKDF-Expand-Label(secret, label, context, len)
pub fn expand_label(h: HashAlg, secret: &[u8], label: &str, context: &[u8], out: &mut [u8]) {
    let full = [b"tls13 ", label.as_bytes()].concat();
    let mut info = Vec::with_capacity(4 + full.len() + context.len());
    info.extend_from_slice(&(out.len() as u16).to_be_bytes());
    info.push(full.len() as u8);
    info.extend_from_slice(&full);
    info.push(context.len() as u8);
    info.extend_from_slice(context);
    expand(h, secret, &info, out);
}

/// Derive-Secret(secret, label, transcript hash)
pub fn derive(h: HashAlg, secret: &[u8], label: &str, hash: &[u8]) -> Secret {
    let mut out = alloc::vec![0u8; h.len()];
    expand_label(h, secret, label, hash, &mut out);
    out
}

pub fn extract(h: HashAlg, salt: &[u8], ikm: &[u8]) -> Secret {
    let mut out = alloc::vec![0u8; h.len()];
    match h {
        HashAlg::Sha384 => hkdf_extract::<Sha384>(salt, ikm, &mut out),
        _ => hkdf_extract::<Sha256>(salt, ikm, &mut out),
    }
    out
}

/// the hash of no messages
pub fn empty_hash(h: HashAlg) -> Vec<u8> {
    h.digest(b"")
}

/// the secrets of one handshake
pub struct Schedule {
    pub hash: HashAlg,
    /// the early secret (of the PSK, or of zeros)
    pub early: Secret,
    pub handshake: Secret,
    pub master: Secret,
}

impl Schedule {
    /// from the pre-shared key (None: zeros) and the (EC)DHE / hybrid shared secret
    pub fn new(h: HashAlg, psk: Option<&[u8]>, shared: &[u8]) -> Schedule {
        let zeros = alloc::vec![0u8; h.len()];
        let early = extract(h, &zeros, psk.unwrap_or(&zeros));
        let derived = derive(h, &early, "derived", &empty_hash(h));
        let handshake = extract(h, &derived, shared);
        let derived2 = derive(h, &handshake, "derived", &empty_hash(h));
        let master = extract(h, &derived2, &zeros);
        Schedule { hash: h, early, handshake, master }
    }

    /// the binder key of a resumption PSK ("res binder") or an external one ("ext binder")
    pub fn binder_key(&self, external: bool) -> Secret {
        derive(self.hash, &self.early, if external { "ext binder" } else { "res binder" }, &empty_hash(self.hash))
    }
}

impl Drop for Schedule {
    fn drop(&mut self) {
        crypto::zeroize(&mut self.early);
        crypto::zeroize(&mut self.handshake);
        crypto::zeroize(&mut self.master);
    }
}

/// the AEAD key (of key_len bytes) and IV of a traffic secret
pub fn traffic_keys_len(h: HashAlg, secret: &[u8], key_len: usize) -> (Vec<u8>, [u8; 12]) {
    let mut key = alloc::vec![0u8; key_len];
    let mut iv = [0u8; 12];
    expand_label(h, secret, "key", &[], &mut key);
    expand_label(h, secret, "iv", &[], &mut iv);
    (key, iv)
}

/// TLS 1.2: PRF(secret, label, seed) with P_SHA256, or P_SHA384 for the SHA-384 suites
pub fn prf12(h: HashAlg, secret: &[u8], label: &str, seed: &[u8], out: &mut [u8]) {
    let mut ls = Vec::from(label.as_bytes());
    ls.extend_from_slice(seed);
    let mut a = h.hmac(secret, &ls);
    let mut off = 0;
    while off < out.len() {
        let mut m = a.clone();
        m.extend_from_slice(&ls);
        let block = h.hmac(secret, &m);
        let n = (out.len() - off).min(block.len());
        out[off..off + n].copy_from_slice(&block[..n]);
        off += n;
        a = h.hmac(secret, &a);
    }
}

/// verify_data of a Finished message (and a PSK binder: the binder key as `base`)
pub fn finished(h: HashAlg, base: &[u8], transcript: &[u8]) -> Vec<u8> {
    let mut fk = alloc::vec![0u8; h.len()];
    expand_label(h, base, "finished", &[], &mut fk);
    let out = h.hmac(&fk, transcript);
    crypto::zeroize(&mut fk);
    out
}

/// the next traffic secret (KeyUpdate)
pub fn update(h: HashAlg, secret: &[u8]) -> Secret {
    let mut out = alloc::vec![0u8; h.len()];
    expand_label(h, secret, "traffic upd", &[], &mut out);
    out
}

/// the transcript hash of a handshake: both hashes until the cipher suite picks one
#[derive(Clone)]
pub struct Transcript {
    s256: Option<Sha256>,
    s384: Option<Sha384>,
}

impl Default for Transcript {
    fn default() -> Transcript {
        Transcript::new()
    }
}

impl Transcript {
    pub fn new() -> Transcript {
        Transcript { s256: Some(Sha256::new()), s384: Some(Sha384::new()) }
    }

    pub fn update(&mut self, m: &[u8]) {
        if let Some(h) = self.s256.as_mut() {
            h.update(m);
        }
        if let Some(h) = self.s384.as_mut() {
            h.update(m);
        }
    }

    /// the hash so far with `h` (SHA-384 or SHA-256)
    pub fn hash(&self, h: HashAlg) -> Vec<u8> {
        match h {
            HashAlg::Sha384 => self.s384.clone().map(|s| s.finalize().to_vec()).unwrap_or_default(),
            _ => self.s256.clone().map(|s| s.finalize().to_vec()).unwrap_or_default(),
        }
    }

    /// keep only `h` from here on (the suite is chosen)
    pub fn keep(&mut self, h: HashAlg) {
        if h == HashAlg::Sha384 {
            self.s256 = None;
        } else {
            self.s384 = None;
        }
    }
}
