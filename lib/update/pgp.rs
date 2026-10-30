// pgp -- just enough OpenPGP to check the signature appimagetool --sign embeds in an AppImage:
// an ASCII-armored detached signature over a short text and the signer's armored public key
// (RFC 4880 / RFC 9580, version 4 keys and signatures)
//
//   keys(armored) -> Result<Vec<Key>, String>    the primary key and subkeys of a key block
//                                                (RSA, ECDSA on P-256 / P-384, EdDSA "legacy"
//                                                on Ed25519, Ed25519)
//   verify(keys, armored_sig, data) -> Result<Key, String>   the key whose signature over data
//                                                checks (signature types binary and text,
//                                                SHA-256 / -384 / -512)
//
// Binding signatures of subkeys are not checked: the keys are trusted as a whole (the key
// block of the AppImage that is running), not through a web of trust.  Arithmetic is lib/crypto's
// (rsa::PublicKey::verify_pkcs1, ecdsa::verify, ed25519::verify).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crypto::HashAlg;

/// a public key of a key block
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    /// the v4 fingerprint (SHA-1 over the key packet), 20 bytes
    pub fingerprint: [u8; 20],
    material: Material,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Material {
    Rsa { n: Vec<u8>, e: Vec<u8> },
    /// the curve and the uncompressed point (04 || x || y)
    Ecdsa(crypto::ecdsa::Curve, Vec<u8>),
    Ed25519([u8; 32]),
}

// public-key algorithms
const ALG_RSA: u8 = 1;
const ALG_RSA_SIGN: u8 = 3;
const ALG_ECDSA: u8 = 19;
const ALG_EDDSA_LEGACY: u8 = 22;
const ALG_ED25519: u8 = 27;
// packet tags
const TAG_SIGNATURE: u8 = 2;
const TAG_PUBLIC_KEY: u8 = 6;
const TAG_PUBLIC_SUBKEY: u8 = 14;
// signature subpackets
const SUB_ISSUER: u8 = 16;
const SUB_ISSUER_FPR: u8 = 33;
// the OID of Ed25519 in EdDSA legacy keys (1.3.6.1.4.1.11591.15.1)
const OID_ED25519: [u8; 9] = [0x2B, 0x06, 0x01, 0x04, 0x01, 0xDA, 0x47, 0x0F, 0x01];
// NIST P-256 (1.2.840.10045.3.1.7) and P-384 (1.3.132.0.34)
const OID_P256: [u8; 8] = [0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07];
const OID_P384: [u8; 5] = [0x2B, 0x81, 0x04, 0x00, 0x22];

// ---------------------------------------------------------------- armor

// CRC-24 of the armor checksum (RFC 4880 6.1)
fn crc24(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xB7_04CE;
    for &b in data {
        crc ^= (b as u32) << 16;
        for _ in 0..8 {
            crc <<= 1;
            if crc & 0x100_0000 != 0 {
                crc ^= 0x186_4CFB;
            }
        }
    }
    crc & 0xFF_FFFF
}

/// the bytes of the first armored block of `kind` ("SIGNATURE", "PUBLIC KEY BLOCK")
fn dearmor(text: &str, kind: &str) -> Result<Vec<u8>, String> {
    let begin = format!("-----BEGIN PGP {}-----", kind);
    let end = format!("-----END PGP {}-----", kind);
    let mut lines = text.lines().map(str::trim).skip_while(|l| *l != begin);
    if lines.next().is_none() {
        return Err(format!("kein „{}“", begin));
    }
    let rest: Vec<&str> = lines.take_while(|l| *l != end).collect();
    // armor headers ("Comment: ...") end with a blank line; without headers there may be none
    let body = match rest.iter().position(|l| l.is_empty()) {
        Some(i) if rest[..i].iter().all(|l| l.contains(": ")) => &rest[i + 1..],
        _ => &rest[..],
    };
    let mut b64 = String::new();
    let mut sum = None;
    for l in body {
        if let Some(c) = l.strip_prefix('=') {
            sum = Some(c);
            break;
        }
        b64.push_str(l);
    }
    let data = crypto::b64::decode(&b64).ok_or("ungültiges Base64 in der Armor")?;
    if let Some(c) = sum {
        let want = crypto::b64::decode(c).filter(|v| v.len() == 3).ok_or("ungültige Armor-Prüfsumme")?;
        if crc24(&data) != (want[0] as u32) << 16 | (want[1] as u32) << 8 | want[2] as u32 {
            return Err(String::from("die Armor-Prüfsumme stimmt nicht"));
        }
    }
    Ok(data)
}

// ---------------------------------------------------------------- packets

/// the packets of a message: (tag, body)
fn packets(mut b: &[u8]) -> Result<Vec<(u8, &[u8])>, String> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let h = b[0];
        if h & 0x80 == 0 {
            return Err(String::from("kein OpenPGP-Paket"));
        }
        let (tag, len, skip) = if h & 0x40 != 0 {
            let o1 = *b.get(1).ok_or("Paket abgeschnitten")? as usize;
            match o1 {
                0..=191 => (h & 0x3F, o1, 2),
                192..=223 => (h & 0x3F, ((o1 - 192) << 8) + *b.get(2).ok_or("Paket abgeschnitten")? as usize + 192, 3),
                255 => {
                    let n = b.get(2..6).ok_or("Paket abgeschnitten")?;
                    (h & 0x3F, u32::from_be_bytes([n[0], n[1], n[2], n[3]]) as usize, 6)
                }
                _ => return Err(String::from("Teillängen werden nicht unterstützt")),
            }
        } else {
            let tag = (h >> 2) & 0x0F;
            match h & 3 {
                0 => (tag, *b.get(1).ok_or("Paket abgeschnitten")? as usize, 2),
                1 => {
                    let n = b.get(1..3).ok_or("Paket abgeschnitten")?;
                    (tag, u16::from_be_bytes([n[0], n[1]]) as usize, 3)
                }
                2 => {
                    let n = b.get(1..5).ok_or("Paket abgeschnitten")?;
                    (tag, u32::from_be_bytes([n[0], n[1], n[2], n[3]]) as usize, 5)
                }
                _ => (tag, b.len() - 1, 1),
            }
        };
        let body = b.get(skip..skip + len).ok_or("Paket abgeschnitten")?;
        out.push((tag, body));
        b = &b[skip + len..];
    }
    Ok(out)
}

/// a multiprecision integer at *p (big endian, leading zeros dropped)
fn mpi<'a>(b: &'a [u8], p: &mut usize) -> Result<&'a [u8], String> {
    let bits = b.get(*p..*p + 2).map(|n| u16::from_be_bytes([n[0], n[1]]) as usize).ok_or("MPI abgeschnitten")?;
    let n = bits.div_ceil(8);
    let v = b.get(*p + 2..*p + 2 + n).ok_or("MPI abgeschnitten")?;
    *p += 2 + n;
    Ok(v)
}

// a value of at most 32 bytes as exactly 32 (left-padded)
fn fixed32(v: &[u8]) -> Option<[u8; 32]> {
    let v = &v[v.iter().position(|&x| x != 0).unwrap_or(v.len())..];
    if v.len() > 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out[32 - v.len()..].copy_from_slice(v);
    Some(out)
}

/// a version 4 public key packet -> its key (None: an algorithm this does not check)
fn key_packet(body: &[u8]) -> Result<Option<Key>, String> {
    if body.first() != Some(&4) {
        return Ok(None); // v3 (obsolete), v5, v6
    }
    let alg = *body.get(5).ok_or("Schlüssel abgeschnitten")?;
    let mut p = 6;
    let material = match alg {
        ALG_RSA | ALG_RSA_SIGN => {
            let n = mpi(body, &mut p)?.to_vec();
            let e = mpi(body, &mut p)?.to_vec();
            Material::Rsa { n, e }
        }
        ALG_ECDSA => {
            let olen = *body.get(p).ok_or("Schlüssel abgeschnitten")? as usize;
            let oid = body.get(p + 1..p + 1 + olen).ok_or("Schlüssel abgeschnitten")?;
            p += 1 + olen;
            let curve = if oid == OID_P256 {
                crypto::ecdsa::Curve::P256
            } else if oid == OID_P384 {
                crypto::ecdsa::Curve::P384
            } else {
                return Ok(None);
            };
            Material::Ecdsa(curve, mpi(body, &mut p)?.to_vec())
        }
        ALG_EDDSA_LEGACY => {
            let olen = *body.get(p).ok_or("Schlüssel abgeschnitten")? as usize;
            let oid = body.get(p + 1..p + 1 + olen).ok_or("Schlüssel abgeschnitten")?;
            p += 1 + olen;
            if oid != OID_ED25519 {
                return Ok(None);
            }
            let q = mpi(body, &mut p)?;
            // the native point format: 0x40 and the 32 bytes
            match q.split_first() {
                Some((0x40, k)) if k.len() == 32 => Material::Ed25519(k.try_into().unwrap_or([0; 32])),
                _ => return Err(String::from("ungültiger Ed25519-Schlüssel")),
            }
        }
        ALG_ED25519 => Material::Ed25519(body.get(6..38).and_then(|k| k.try_into().ok()).ok_or("Schlüssel abgeschnitten")?),
        _ => return Ok(None),
    };
    // v4 fingerprint: SHA-1 of 0x99, the two-byte length and the packet body
    let mut h = crypto::Sha1::new();
    h.update(&[0x99, (body.len() >> 8) as u8, body.len() as u8]);
    h.update(body);
    Ok(Some(Key { fingerprint: h.finalize(), material }))
}

/// the keys (primary and subkeys) of an armored public key block
pub fn keys(armored: &str) -> Result<Vec<Key>, String> {
    let data = dearmor(armored, "PUBLIC KEY BLOCK")?;
    let mut out = Vec::new();
    for (tag, body) in packets(&data)? {
        if tag == TAG_PUBLIC_KEY || tag == TAG_PUBLIC_SUBKEY {
            if let Some(k) = key_packet(body)? {
                out.push(k);
            }
        }
    }
    if out.is_empty() {
        return Err(String::from("kein prüfbarer öffentlicher Schlüssel (RSA, ECDSA P-256 / P-384 oder Ed25519, Version 4)"));
    }
    Ok(out)
}

// ---------------------------------------------------------------- signatures

struct Sig<'a> {
    typ: u8,
    alg: u8,
    hash: HashAlg,
    /// the hashed part: version up to the end of the hashed subpackets
    hashed: &'a [u8],
    left16: [u8; 2],
    issuer_id: Option<[u8; 8]>,
    issuer_fpr: Option<[u8; 20]>,
    /// RSA: s; ECDSA: r and s, each as long as the longer of both; Ed25519: r || s
    value: Vec<u8>,
}

// the issuer subpackets of a subpacket area
fn issuers(mut a: &[u8], id: &mut Option<[u8; 8]>, fpr: &mut Option<[u8; 20]>) -> Result<(), String> {
    while !a.is_empty() {
        let o1 = a[0] as usize;
        let (len, skip) = match o1 {
            0..=191 => (o1, 1),
            192..=254 => (((o1 - 192) << 8) + *a.get(1).ok_or("Unterpaket abgeschnitten")? as usize + 192, 2),
            _ => {
                let n = a.get(1..5).ok_or("Unterpaket abgeschnitten")?;
                (u32::from_be_bytes([n[0], n[1], n[2], n[3]]) as usize, 5)
            }
        };
        let sp = a.get(skip..skip + len).filter(|s| !s.is_empty()).ok_or("Unterpaket abgeschnitten")?;
        match sp[0] & 0x7F {
            SUB_ISSUER if sp.len() == 9 => *id = sp[1..9].try_into().ok(),
            SUB_ISSUER_FPR if sp.len() == 22 && sp[1] == 4 => *fpr = sp[2..22].try_into().ok(),
            _ => {}
        }
        a = &a[skip + len..];
    }
    Ok(())
}

fn signature(body: &[u8]) -> Result<Sig<'_>, String> {
    if body.first() != Some(&4) {
        return Err(format!("Signaturversion {} wird nicht unterstützt (nur 4)", body.first().copied().unwrap_or(0)));
    }
    let (typ, alg, halg) = match body.get(1..4) {
        Some(h) => (h[0], h[1], h[2]),
        None => return Err(String::from("Signatur abgeschnitten")),
    };
    let hash = match halg {
        8 => HashAlg::Sha256,
        9 => HashAlg::Sha384,
        10 => HashAlg::Sha512,
        _ => return Err(format!("Hash-Verfahren {} wird nicht unterstützt (SHA-256, -384, -512)", halg)),
    };
    let hl = body.get(4..6).map(|n| u16::from_be_bytes([n[0], n[1]]) as usize).ok_or("Signatur abgeschnitten")?;
    let hashed = body.get(..6 + hl).ok_or("Signatur abgeschnitten")?;
    let mut p = 6 + hl;
    let ul = body.get(p..p + 2).map(|n| u16::from_be_bytes([n[0], n[1]]) as usize).ok_or("Signatur abgeschnitten")?;
    let unhashed = body.get(p + 2..p + 2 + ul).ok_or("Signatur abgeschnitten")?;
    p += 2 + ul;
    let (mut issuer_id, mut issuer_fpr) = (None, None);
    issuers(&hashed[6..], &mut issuer_id, &mut issuer_fpr)?;
    issuers(unhashed, &mut issuer_id, &mut issuer_fpr)?;
    let left16 = body.get(p..p + 2).map(|n| [n[0], n[1]]).ok_or("Signatur abgeschnitten")?;
    p += 2;
    let value = match alg {
        ALG_RSA | ALG_RSA_SIGN => mpi(body, &mut p)?.to_vec(),
        ALG_ECDSA => {
            let (r, s) = (mpi(body, &mut p)?, mpi(body, &mut p)?);
            let n = r.len().max(s.len());
            let mut v = alloc::vec![0u8; 2 * n];
            v[n - r.len()..n].copy_from_slice(r);
            v[2 * n - s.len()..].copy_from_slice(s);
            v
        }
        ALG_EDDSA_LEGACY => {
            let r = fixed32(mpi(body, &mut p)?).ok_or("ungültige EdDSA-Signatur")?;
            let s = fixed32(mpi(body, &mut p)?).ok_or("ungültige EdDSA-Signatur")?;
            [r, s].concat()
        }
        ALG_ED25519 => body.get(p..p + 64).ok_or("Signatur abgeschnitten")?.to_vec(),
        _ => return Err(format!("Signaturverfahren {} wird nicht unterstützt (RSA, ECDSA, Ed25519)", alg)),
    };
    Ok(Sig { typ, alg, hash, hashed, left16, issuer_id, issuer_fpr, value })
}

// the signed hash: the data (text signatures: line ends as CRLF), the hashed part and the trailer
fn digest(s: &Sig, data: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(data.len() + s.hashed.len() + 6);
    if s.typ == 0x01 {
        let mut prev = 0u8;
        for &c in data {
            if c == b'\n' && prev != b'\r' {
                msg.push(b'\r');
            }
            msg.push(c);
            prev = c;
        }
    } else {
        msg.extend_from_slice(data);
    }
    msg.extend_from_slice(s.hashed);
    msg.extend_from_slice(&[0x04, 0xFF]);
    msg.extend_from_slice(&(s.hashed.len() as u32).to_be_bytes());
    s.hash.digest(&msg)
}

fn check(k: &Key, s: &Sig, d: &[u8]) -> bool {
    match (&k.material, s.alg) {
        (Material::Rsa { n, e }, ALG_RSA | ALG_RSA_SIGN) => {
            let Some(pk) = crypto::rsa::PublicKey::new(n, e) else { return false };
            // the signature value as long as the modulus
            let k_len = n.iter().skip_while(|&&b| b == 0).count();
            if s.value.len() > k_len {
                return false;
            }
            let mut sig = alloc::vec![0u8; k_len - s.value.len()];
            sig.extend_from_slice(&s.value);
            pk.verify_pkcs1(s.hash, d, &sig)
        }
        (Material::Ecdsa(curve, q), ALG_ECDSA) => {
            let (r, sv) = s.value.split_at(s.value.len() / 2);
            crypto::ecdsa::verify(*curve, q, d, r, sv)
        }
        (Material::Ed25519(pk), ALG_EDDSA_LEGACY | ALG_ED25519) => {
            let Ok(sig) = <[u8; 64]>::try_from(s.value.as_slice()) else { return false };
            crypto::ed25519::verify(pk, d, &sig)
        }
        _ => false,
    }
}

/// the key of `keys` whose detached signature (armored) over data checks -> it, or why not
pub fn verify(keys: &[Key], armored_sig: &str, data: &[u8]) -> Result<Key, String> {
    let raw = dearmor(armored_sig, "SIGNATURE")?;
    let sigs: Vec<&[u8]> = packets(&raw)?.into_iter().filter(|(t, _)| *t == TAG_SIGNATURE).map(|(_, b)| b).collect();
    if sigs.is_empty() {
        return Err(String::from("keine Signatur"));
    }
    let mut why = String::from("die Signatur passt zu keinem Schlüssel");
    for body in sigs {
        let s = match signature(body) {
            Ok(s) => s,
            Err(e) => {
                why = e;
                continue;
            }
        };
        if s.typ > 0x01 {
            why = format!("Signaturtyp 0x{:02X} ist keine Dokumentsignatur", s.typ);
            continue;
        }
        let d = digest(&s, data);
        if d[..2] != s.left16 {
            why = String::from("die Signatur gehört nicht zu diesen Daten");
            continue;
        }
        for k in keys {
            let named = s.issuer_fpr.map(|f| f == k.fingerprint).or(s.issuer_id.map(|i| i == k.fingerprint[12..])).unwrap_or(true);
            if named && check(k, &s, &d) {
                return Ok(k.clone());
            }
        }
    }
    Err(why)
}
