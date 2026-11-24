// suitevec -- the operations of `coretest --vectors` behind tools/wycheproof_check.py
// (C2SP/wycheproof) and tools/limbo_check.py (C2SP/x509-limbo); each one goes through the API a
// program of JFLib calls (signatures: netio::x509::Cert::verify, as TLS and the certificate
// checks do; chains: netio::x509::verify_chain, as tls/conn.rs and roots.rs do)
//
//   certverify kind alg hash salt keyA keyB msg sig   -> 01 accepted / 00 refused
//       kind 1 Ed25519 (keyA: 32 bytes), 2 RSA (keyA n, keyB e), 3 P-256, 4 P-384 (keyA 04||x||y)
//       alg  1 Ed25519, 2 RSA PKCS #1 v1.5, 3 RSA-PSS (salt: 2 bytes big-endian, empty: any),
//            4 ECDSA (sig: the DER Ecdsa-Sig-Value)
//       hash 1 SHA-256, 2 SHA-384, 3 SHA-512, 4 SHA-224, 5..8 SHA3-224 / 256 / 384 / 512 (the
//            message is hashed here; crate::crypto::hash_of)
//   rsasign hash n e d p q digest              -> RSASSA-PKCS1-v1_5 signature, - (key refused)
//   x25519shared k u                           -> x25519::shared, - (None: all-zero secret)
//   aeadopen / xaead / xaeadopen k nonce aad x -> (X)ChaCha20-Poly1305 open (x = ct||tag) / seal
//   hkdf hash salt ikm info len(2 bytes)       -> okm; - when len > 255 HashLen (hkdf_expand
//                                                 asserts that: the caller's precondition)
//   hmac hash key msg                          -> the full HMAC; hash 0 = SHA-1, else as above
//   x509chain usage now name n leaf inter.. anchors..
//       usage 0 server / 1 client, now: 8 bytes big-endian (seconds), name: 00 = none or 01 + UTF-8,
//       n: the number of intermediates (2 bytes); then the DER certificates
//       -> 01 / 00 + verify_chain's reason (UTF-8) / 02 + 0 (an unreadable leaf: tls/conn.rs
//          refuses the Certificate message; an unreadable intermediate is left out, as there)

use alloc::vec::Vec;

use crypto::HashAlg;
use netio::x509::{verify_chain, Cert, PubKey, SigAlg, Usage};

/// hash ids: crate::crypto::hash_of
fn hash(id: Option<&u8>) -> Option<HashAlg> {
    crate::crypto::hash_of(id)
}

fn hkdf<H: crypto::Hash>(salt: &[u8], ikm: &[u8], info: &[u8], len: usize) -> Option<Vec<u8>> {
    if len > 255 * H::OUT {
        return None;
    }
    let mut prk = [0u8; 64];
    crypto::hmac::hkdf_extract::<H>(salt, ikm, &mut prk[..H::OUT]);
    let mut out = alloc::vec![0u8; len];
    crypto::hmac::hkdf_expand::<H>(&prk[..H::OUT], info, &mut out);
    Some(out)
}

fn hmac<H: crypto::Hash>(key: &[u8], msg: &[u8]) -> Vec<u8> {
    let mut m = crypto::Hmac::<H>::new(key);
    m.update(msg);
    let mut out = alloc::vec![0u8; H::OUT];
    m.finish_into(&mut out);
    out
}

fn n24(a: &[u8]) -> Option<[u8; 24]> {
    a.try_into().ok()
}

/// one operation (None: not one of these, or refused / "-")
pub fn op(alg: &str, args: &[Vec<u8>]) -> Option<Vec<u8>> {
    let arg = |i: usize| args.get(i).map(|v| v.as_slice()).unwrap_or(&[]);
    let k32 = |i: usize| -> Option<[u8; 32]> { arg(i).try_into().ok() };
    match alg {
        "certverify" => {
            let h = hash(arg(2).first());
            let key = match arg(0).first()? {
                1 => PubKey::Ed25519(k32(4)?),
                2 => PubKey::Rsa { n: arg(4).to_vec(), e: arg(5).to_vec() },
                3 => PubKey::Ec { curve: crypto::ecdsa::Curve::P256, point: arg(4).to_vec() },
                4 => PubKey::Ec { curve: crypto::ecdsa::Curve::P384, point: arg(4).to_vec() },
                _ => return None,
            };
            let salt = match arg(3) {
                [a, b] => Some(u16::from_be_bytes([*a, *b]) as usize),
                _ => None,
            };
            let sig_alg = match arg(1).first()? {
                1 => SigAlg::Ed25519,
                2 => SigAlg::RsaPkcs1(h?),
                3 => SigAlg::RsaPss(h?, salt),
                4 => SigAlg::Ecdsa(h?),
                _ => return None,
            };
            let cert = Cert { key, ..Cert::default() };
            Some(alloc::vec![cert.verify(sig_alg, arg(6), arg(7)) as u8])
        }
        "rsasign" => {
            let k = crypto::rsa::PrivateKey::from_parts(arg(1), arg(2), arg(3), arg(4), arg(5))?;
            k.sign_pkcs1(hash(arg(0).first())?, arg(6))
        }
        "x25519shared" => crypto::x25519::shared(&k32(0)?, &k32(1)?).map(|s| s.to_vec()),
        "aeadopen" => crypto::aead::open(&k32(0)?, &arg(1).try_into().ok()?, arg(2), arg(3)),
        "xaead" => Some(crypto::aead::x_seal(&k32(0)?, &n24(arg(1))?, arg(2), arg(3))),
        "xaeadopen" => crypto::aead::x_open(&k32(0)?, &n24(arg(1))?, arg(2), arg(3)),
        "hkdf" => {
            let len = u16::from_be_bytes(arg(4).try_into().ok()?) as usize;
            let (s, i, f) = (arg(1), arg(2), arg(3));
            match arg(0).first()? {
                0 => hkdf::<crypto::Sha1>(s, i, f, len),
                1 => hkdf::<crypto::Sha256>(s, i, f, len),
                2 => hkdf::<crypto::Sha384>(s, i, f, len),
                3 => hkdf::<crypto::Sha512>(s, i, f, len),
                _ => None,
            }
        }
        "hmac" => match arg(0).first()? {
            0 => Some(hmac::<crypto::Sha1>(arg(1), arg(2))),
            1 => Some(hmac::<crypto::Sha256>(arg(1), arg(2))),
            2 => Some(hmac::<crypto::Sha384>(arg(1), arg(2))),
            3 => Some(hmac::<crypto::Sha512>(arg(1), arg(2))),
            4 => Some(hmac::<crypto::Sha224>(arg(1), arg(2))),
            5 => Some(hmac::<crypto::Sha3_224>(arg(1), arg(2))),
            6 => Some(hmac::<crypto::Sha3_256>(arg(1), arg(2))),
            7 => Some(hmac::<crypto::Sha3_384>(arg(1), arg(2))),
            8 => Some(hmac::<crypto::Sha3_512>(arg(1), arg(2))),
            _ => None,
        },
        "x509chain" => {
            let usage = if arg(0).first() == Some(&1) { Usage::Client } else { Usage::Server };
            let now = i64::from_be_bytes(arg(1).try_into().ok()?);
            let name = match arg(2).split_first() {
                Some((1, n)) => Some(core::str::from_utf8(n).ok()?),
                _ => None,
            };
            let n = u16::from_be_bytes(arg(3).try_into().ok()?) as usize;
            let mut chain = Vec::new();
            for i in 0..=n {
                match Cert::parse(arg(4 + i)) {
                    Some(c) => chain.push(c),
                    None if i == 0 => return Some(alloc::vec![2, 0]),
                    None => {}
                }
            }
            // anchors that do not parse are left out, as roots::system_anchors does
            let anchors: Vec<Cert> = (5 + n..args.len()).filter_map(|i| Cert::parse(arg(i))).collect();
            Some(match verify_chain(&chain, &anchors, now, name, usage) {
                Ok(()) => alloc::vec![1],
                Err(e) => {
                    let mut v = alloc::vec![0];
                    v.extend_from_slice(e.as_bytes());
                    v
                }
            })
        }
        _ => None,
    }
}
