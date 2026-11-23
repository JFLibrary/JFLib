// kx -- the key exchange groups of TLS: X25519, P-256 and P-384 (ECDHE: RFC 8446 4.2.8.2, RFC
// 8422 5.4) and the hybrids of an ECDHE share and ML-KEM (draft-ietf-tls-ecdhe-mlkem): each
// side's share is the two halves concatenated, the shared secret both secrets
//
//   group                 client share                server share                secret
//   X25519MLKEM768        ML-KEM-768 ek || X25519     ciphertext || X25519        ML-KEM || X25519
//   SecP256r1MLKEM768     P-256 point || ek           P-256 point || ciphertext   ECDH || ML-KEM
//   SecP384r1MLKEM1024    P-384 point || ek (1024)    P-384 point || ciphertext   ECDH || ML-KEM
//
//   Share::new(group) -> our key share (client, TLS 1.2 client: ECDHE only), its secrets
//   share.finish(peer) -> the shared secret from the peer's share
//   respond(group, client_share) -> (our share, the shared secret) for a TLS 1.3 server
//
// Every length is exact; an ECDHE point off the curve, an X25519 result of zeros or an
// encapsulation key that fails FIPS 203's modulus check gives None (illegal_parameter).

use alloc::vec::Vec;

use crypto::mlkem::{self, Params};
use crypto::{p256, p384, x25519};

pub const SECP256R1: u16 = 0x0017;
pub const SECP384R1: u16 = 0x0018;
pub const X25519: u16 = 0x001d;
pub const SECP256R1_MLKEM768: u16 = 0x11eb;
pub const X25519_MLKEM768: u16 = 0x11ec;
pub const SECP384R1_MLKEM1024: u16 = 0x11ed;

/// every group here, in the order a server prefers them when it may choose
pub const GROUPS: [u16; 6] = [X25519_MLKEM768, X25519, SECP256R1, SECP384R1, SECP256R1_MLKEM768, SECP384R1_MLKEM1024];

/// the ECDHE part and the ML-KEM part of a group
fn parts(group: u16) -> Option<(u16, Option<Params>)> {
    Some(match group {
        X25519 | SECP256R1 | SECP384R1 => (group, None),
        X25519_MLKEM768 => (X25519, Some(Params::MlKem768)),
        SECP256R1_MLKEM768 => (SECP256R1, Some(Params::MlKem768)),
        SECP384R1_MLKEM1024 => (SECP384R1, Some(Params::MlKem1024)),
        _ => return None,
    })
}

/// whether this is a group here
pub fn known(group: u16) -> bool {
    parts(group).is_some()
}

/// X25519MLKEM768 puts ML-KEM first (draft-ietf-tls-ecdhe-mlkem 4.1), the others ECDHE
fn kem_first(group: u16) -> bool {
    group == X25519_MLKEM768
}

/// an ECDHE key pair: (secret, public)
fn ec_keypair(curve: u16) -> (Vec<u8>, Vec<u8>) {
    match curve {
        SECP256R1 => loop {
            if let Some(s) = p256::secret_from(&plat::random_bytes()) {
                break (Vec::from(&s[..]), Vec::from(&p256::public_key(&s)[..]));
            }
        },
        SECP384R1 => loop {
            if let Some(s) = p384::secret_from(&plat::random_bytes()) {
                break (Vec::from(&s[..]), Vec::from(&p384::public_key(&s)[..]));
            }
        },
        _ => {
            let s: [u8; 32] = plat::random_bytes();
            (Vec::from(&s[..]), Vec::from(&x25519::public_key(&s)[..]))
        }
    }
}

/// the public share's length of an ECDHE curve
fn ec_len(curve: u16) -> usize {
    match curve {
        SECP256R1 => 65,
        SECP384R1 => 97,
        _ => 32,
    }
}

fn ec_shared(curve: u16, secret: &[u8], peer: &[u8]) -> Option<Vec<u8>> {
    match curve {
        SECP256R1 => p256::shared(secret.try_into().ok()?, peer).map(|s| Vec::from(&s[..])),
        SECP384R1 => p384::shared(secret.try_into().ok()?, peer).map(|s| Vec::from(&s[..])),
        _ => x25519::shared(secret.try_into().ok()?, peer.try_into().ok()?).map(|s| Vec::from(&s[..])),
    }
}

/// the two halves of a hybrid share: (ECDHE, ML-KEM), each of its exact length
fn split(group: u16, share: &[u8], kem_len: usize) -> Option<(&[u8], &[u8])> {
    let (curve, _) = parts(group)?;
    let el = ec_len(curve);
    if share.len() != el + kem_len {
        return None;
    }
    Some(if kem_first(group) { (&share[kem_len..], &share[..kem_len]) } else { share.split_at(el) })
}

fn join(group: u16, ec: &[u8], kem: &[u8]) -> Vec<u8> {
    if kem_first(group) { [kem, ec].concat() } else { [ec, kem].concat() }
}

/// a key share we made, with its secrets (wiped when dropped)
pub struct Share {
    pub group: u16,
    pub public: Vec<u8>,
    ec_secret: Vec<u8>,
    dk: Vec<u8>,
}

impl Drop for Share {
    fn drop(&mut self) {
        crypto::zeroize(&mut self.ec_secret);
        crypto::zeroize(&mut self.dk);
    }
}

impl Share {
    /// a new share of `group` (None: not a group here)
    pub fn new(group: u16) -> Option<Share> {
        let (curve, kem) = parts(group)?;
        let (ec_secret, ec_public) = ec_keypair(curve);
        let Some(params) = kem else {
            return Some(Share { group, public: ec_public, ec_secret, dk: Vec::new() });
        };
        let mut seed: [u8; 64] = plat::random_bytes();
        let (ek, dk) = mlkem::keygen(params, &seed);
        crypto::zeroize(&mut seed);
        Some(Share { group, public: join(group, &ec_public, &ek), ec_secret, dk })
    }

    /// the shared secret from the peer's share (None: of the wrong length or not valid)
    pub fn finish(&self, peer: &[u8]) -> Option<Vec<u8>> {
        let (curve, kem) = parts(self.group)?;
        let Some(params) = kem else {
            return ec_shared(curve, &self.ec_secret, peer);
        };
        let (ec, ct) = split(self.group, peer, params.ct_len())?;
        let mut e = ec_shared(curve, &self.ec_secret, ec)?;
        let mut k = mlkem::decaps(params, &self.dk, ct)?;
        let out = join(self.group, &e, &k);
        crypto::zeroize(&mut e);
        crypto::zeroize(&mut k);
        Some(out)
    }
}

/// a TLS 1.3 server's answer to the client's share of `group`: (our share, the shared secret)
pub fn respond(group: u16, client: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let (curve, kem) = parts(group)?;
    let Some(params) = kem else {
        let ours = Share::new(group)?;
        let shared = ours.finish(client)?;
        return Some((ours.public.clone(), shared));
    };
    let (ec, ek) = split(group, client, params.ek_len())?;
    let (mut ec_secret, ec_public) = ec_keypair(curve);
    let e = ec_shared(curve, &ec_secret, ec);
    crypto::zeroize(&mut ec_secret);
    let mut e = e?;
    let mut m: [u8; 32] = plat::random_bytes();
    let enc = mlkem::encaps(params, ek, &m);
    crypto::zeroize(&mut m);
    let (ct, mut k) = enc?;
    let shared = join(group, &e, &k);
    crypto::zeroize(&mut e);
    crypto::zeroize(&mut k);
    Some((join(group, &ec_public, &ct), shared))
}
