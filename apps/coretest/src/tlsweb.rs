// tlsweb -- TLS with the keys and features of the web PKI: private keys of every kind (PKCS #8,
// SEC 1, PKCS #1; certificates signed with them), the key exchange groups (X25519MLKEM768 and the
// other hybrids, P-384), handshakes with Ed25519, ECDSA and RSA certificates both ways (mutual
// TLS with keys of different kinds), the server's choice of certificate by SNI and key type,
// ALPN on both sides
//
// Also the PKI of --tls-pki / --tls-serve / --tls-get with --key ed25519|p256|p384|rsa
// (tools/tls_interop.py, tools/tlsfuzzer_check.py).

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crypto::ecdsa::Curve;
use crypto::ed25519::Keypair;
use netio::der;
use netio::tls::{self, kx, ClientConfig, Conn, Identity, ServerConfig, TlsError, Trust};
use netio::x509::{Cert, CertBuilder, PrivateKey, PubKey};

use crate::T;

const NOW: i64 = 1_791_000_000;

/// the kinds of keys
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ed25519,
    P256,
    P384,
    Rsa,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "ed25519" => Kind::Ed25519,
            "p256" => Kind::P256,
            "p384" => Kind::P384,
            "rsa" => Kind::Rsa,
            _ => return None,
        })
    }
}

/// a xorshift stream of bytes from `seed` (test keys only)
pub fn xorshift(seed: u8) -> impl FnMut(&mut [u8]) {
    let mut st = 0x9e37_79b9_7f4a_7c15u64 ^ ((seed as u64) << 32) | 1;
    move |b: &mut [u8]| {
        for x in b.iter_mut() {
            st ^= st << 13;
            st ^= st >> 7;
            st ^= st << 17;
            *x = st as u8;
        }
    }
}

/// a deterministic key of `kind` from `seed` (RSA: 2048 bits from a xorshift stream)
pub fn key(kind: Kind, seed: u8) -> PrivateKey {
    let mut rand = xorshift(seed);
    match kind {
        Kind::Ed25519 => PrivateKey::from(Keypair::from_seed(&[seed; 32])),
        Kind::P256 => PrivateKey::generate_ecdsa(Curve::P256, &mut rand),
        Kind::P384 => PrivateKey::generate_ecdsa(Curve::P384, &mut rand),
        Kind::Rsa => crypto::rsa::PrivateKey::generate(2048, &mut rand).and_then(PrivateKey::rsa).expect("rsa key"),
    }
}

pub struct Pki {
    pub ca: Cert,
    pub ca_key: PrivateKey,
    pub server: Identity,
    pub client: Identity,
}

/// a certificate for `names` (none: a client's) of `key`, issued by (ca, ca_key) or self-signed
pub fn issue(key: &PrivateKey, cn: &str, names: &[&str], ca: Option<(&Cert, &PrivateKey)>, serial: u8) -> Vec<u8> {
    let b = CertBuilder {
        serial: [serial; 16],
        cn: String::from(cn),
        org: String::from("JFLib"),
        not_before: NOW - 3600,
        not_after: NOW + 365 * 86_400,
        is_ca: ca.is_none(),
        dns: names.iter().map(|n| String::from(*n)).collect(),
        server_auth: ca.is_some() && !names.is_empty(),
        client_auth: ca.is_some() && names.is_empty(),
        spki: key.spki(),
        ..CertBuilder::default()
    };
    match ca {
        Some((c, k)) => b.sign_key(Some(c), k, &[]),
        None => b.sign_key(None, key, &[]),
    }
    .unwrap_or_default()
}

/// a CA, a server (localhost, 127.0.0.1) and a client, all with keys of `kind`
pub fn pki(seed: u8, kind: Kind) -> Pki {
    let ca_key = key(kind, seed);
    let ca = Cert::parse(&issue(&ca_key, &format!("Test CA {} {:?}", seed, kind), &[], None, seed)).unwrap_or_default();
    let leaf = |s: u8, cn: &str, server: bool| {
        let k = key(kind, s);
        let mut b = CertBuilder {
            serial: [s; 16],
            cn: String::from(cn),
            not_before: NOW - 3600,
            not_after: NOW + 365 * 86_400,
            server_auth: server,
            client_auth: !server,
            spki: k.spki(),
            ..CertBuilder::default()
        };
        if server {
            b.dns = alloc::vec![String::from("localhost")];
            b.ips = alloc::vec![netio::x509::IpAddr::LOCALHOST_V4];
        }
        Identity::new(alloc::vec![b.sign_key(Some(&ca), &ca_key, &[]).unwrap_or_default()], k)
    };
    Pki { server: leaf(seed.wrapping_add(1), "localhost", true), client: leaf(seed.wrapping_add(2), "agent-1", false), ca, ca_key }
}

pub fn run(t: &mut T) {
    keys(t);
    groups(t);
    handshakes(t);
    sni(t);
    alpn(t);
    client_cert_schemes(t);
}

fn keys(t: &mut T) {
    for kind in [Kind::Ed25519, Kind::P256, Kind::P384] {
        let k = key(kind, 3);
        let back = k.to_pkcs8().and_then(|d| PrivateKey::from_der(&d));
        t.check(&format!("keys: {:?} through PKCS #8", kind), back.is_some_and(|b| b.public_key() == k.public_key()));
    }
    // SEC 1's ECPrivateKey with its curve, as OpenSSL writes "EC PRIVATE KEY"
    let k = key(Kind::P256, 4);
    let secret = k.to_pkcs8().and_then(|d| {
        // PKCS #8 -> its ECPrivateKey -> the secret
        let mut top = der::Der::new(&d);
        let mut s = top.seq()?;
        s.u64()?;
        s.read(der::SEQUENCE)?;
        let inner = s.read(der::OCTET_STRING)?;
        let mut e = der::Der::new(inner).seq()?;
        e.u64()?;
        Some(Vec::from(e.read(der::OCTET_STRING)?))
    });
    t.check("keys: the secret of a P-256 key", secret.as_ref().is_some_and(|s| s.len() == 32));
    let secret = secret.unwrap_or_default();
    let PubKey::Ec { point, .. } = k.public_key() else { return };
    let p256 = &[1u64, 2, 840, 10045, 3, 1, 7][..];
    let sec1 = |pub_point: &[u8]| der::seq(&[&der::small_int(1), &der::octets(&secret), &der::explicit(0, &der::oid(p256)), &der::explicit(1, &der::bits(pub_point))]);
    t.check("keys: SEC 1 ECPrivateKey", PrivateKey::from_der(&sec1(&point)).is_some_and(|b| b.public_key() == k.public_key()));
    let mut wrong = point.clone();
    wrong[40] ^= 1;
    t.check("keys: SEC 1 with another public key is refused", PrivateKey::from_der(&sec1(&wrong)).is_none());
    t.check("keys: an ECDSA secret of zeros is refused", PrivateKey::ecdsa(Curve::P256, &[0; 32]).is_none());
    t.check("keys: an ECDSA secret of n or more is refused", PrivateKey::ecdsa(Curve::P256, &[0xff; 32]).is_none());
    // PKCS #1's RSAPrivateKey (dp, dq, qinv are read, not used)
    let rsa = crypto::rsa::PrivateKey::generate(2048, &mut xorshift(5));
    if let Some(r) = rsa {
        let zero = der::small_int(0);
        let rpk = der::seq(&[&zero, &der::int_u(&r.n()), &der::int_u(r.e()), &der::int_u(r.d()), &der::int_u(r.p()), &der::int_u(r.q()), &zero, &zero, &der::int_u(&r.iqmp())]);
        let k = PrivateKey::from_der(&rpk);
        t.check("keys: PKCS #1 RSAPrivateKey", k.as_ref().is_some_and(|k| k.rsa_bits() == 2048));
        let p8 = k.as_ref().and_then(|k| k.to_pkcs8());
        t.check("keys: RSA through PKCS #8", p8.and_then(|d| PrivateKey::from_der(&d)).is_some_and(|b| Some(b.public_key()) == k.as_ref().map(|k| k.public_key())));
        let mut bad = rpk.clone();
        let at = bad.len() - 300;
        bad[at] ^= 1; // inside d or p
        t.check("keys: an RSA key whose numbers do not fit is refused", PrivateKey::from_der(&bad).is_none());
    } else {
        t.check("keys: an RSA key", false);
    }
    t.check("keys: anything else is refused", PrivateKey::from_der(b"\x30\x03\x02\x01\x05").is_none());
    // q^-1 mod p of primes of other lengths, the shorter one first (p 448 bits, q 576: q has
    // more limbs than p and is reduced mod p first)
    let h = |x: &str| crypto::unhex(x).unwrap_or_default();
    let n = h("a34b04be12477be2c06ec2fb49aaea3c3b9b32e8b6f643179bd29962a088913b8fec2c4111cab80814742212920e41fb9e07e027716772818cbb4ba6e92e38e5ad13c9faef95d99c9bc7c2e6f1357aafd66735ea0cb4559abe23ad2367b45a64d7cb66bab626f0fdf387e23a61d45bf92ffd6640e0fd0e56a5d8cafe2d7c41c7");
    let d = h("12cb549d67cf307415bfe9d330546aa74a18ffd00716eafbf1a458a12066f7884a755d5693eca827cdeadbed15406cd9c0039a0a10245eea704c3b59366af4399442c977373bc4845eb18b075977cc66903a8c8f4bcc3b0097ea3b8ea7f4a5249730b85d95a5f7f22d1c6378691df07490b3ab2d85d6b044915afc6630928e69");
    let pp = h("c8e6c223c176c6e21d0ff7fd60bc39610e184a6ecc01f3c169d27a4064a9d9e23aa3b80a42cdbf88c748b5fb6be400ca2389ea61f99cb86d");
    let qq = h("d013c92d25849bc6cf2c5eb321b6a4b18992fcba35b68fbf958816415ee90c591575990b9367d58ad00b7828cc05cf2fe9f291ac8dfe60e4b636c7a60bea1cad45cf7ac9b7282a83");
    let small = crypto::rsa::PrivateKey::from_parts(&n, &[1, 0, 1], &d, &pp, &qq);
    t.check("keys: iqmp of a short p and a long q", small.as_ref().is_some_and(|k| k.iqmp() == h("4eddf4739231c867c8ee78d5f2cb6c9d40cc94ca8bde8e8dd4715b06b5bf660d442b5ed284f478988d3ffbe282b481c6c9788c73a1118c1a")));
    // a 1024-bit key holds PSS with SHA-256, not with SHA-512 and a salt as long (130 > 128
    // bytes): TLS picks only a scheme the key makes
    if let Some(k) = small.and_then(PrivateKey::rsa) {
        use netio::x509::SigAlg;
        t.check("keys: RSA-1024 makes PSS with SHA-256", k.can(SigAlg::RsaPss(crypto::HashAlg::Sha256, None)) && k.sign(SigAlg::RsaPss(crypto::HashAlg::Sha256, None), b"m", &[7; 64]).is_some());
        t.check("keys: ... not with SHA-512", !k.can(SigAlg::RsaPss(crypto::HashAlg::Sha512, None)) && k.sign(SigAlg::RsaPss(crypto::HashAlg::Sha512, None), b"m", &[7; 64]).is_none());
        t.check("keys: ... but PKCS #1 v1.5 with SHA-512", k.can(SigAlg::RsaPkcs1(crypto::HashAlg::Sha512)));
    } else {
        t.check("keys: an RSA key of a short p and a long q", false);
    }
    // certificates signed with every kind, below CAs of other kinds
    let kinds = [Kind::Ed25519, Kind::P256, Kind::P384, Kind::Rsa];
    let cas: Vec<(Cert, PrivateKey)> = kinds.iter().map(|&k| {
        let key = key(k, 10);
        (Cert::parse(&issue(&key, &format!("CA {:?}", k), &[], None, 10)).unwrap_or_default(), key)
    }).collect();
    for (i, (ca, ca_key)) in cas.iter().enumerate() {
        t.check(&format!("certs: a {:?} CA signs itself", kinds[i]), ca.self_signed() && ca.signed_by(ca) && ca.key == ca_key.public_key());
        let leaf_kind = kinds[(i + 1) % 4];
        let lk = key(leaf_kind, 11);
        let leaf = Cert::parse(&issue(&lk, "web.lab", &["web.lab"], Some((ca, ca_key)), 11)).unwrap_or_default();
        t.check(&format!("certs: a {:?} leaf below a {:?} CA", leaf_kind, kinds[i]), leaf.signed_by(ca) && leaf.key == lk.public_key() && leaf.names("web.lab"));
        t.check(&format!("certs: the {:?} leaf's chain verifies", leaf_kind), netio::x509::verify_chain(&[leaf], core::slice::from_ref(ca), NOW, Some("web.lab"), netio::x509::Usage::Server).is_ok());
    }
    // an identity from PEM; the key must be the certificate's
    let p = pki(20, Kind::P256);
    let pem = netio::pem::encode("CERTIFICATE", &p.server.chain[0]);
    let kpem = netio::pem::encode("PRIVATE KEY", &p.server.key.to_pkcs8().unwrap_or_default());
    t.check("identity from PEM", Identity::from_pem(&pem, &kpem).is_ok_and(|i| i.key.public_key() == p.server.key.public_key()));
    let other = netio::pem::encode("PRIVATE KEY", &p.client.key.to_pkcs8().unwrap_or_default());
    t.check("identity from PEM: another key is refused", Identity::from_pem(&pem, &other).is_err());
    t.check("identity from PEM: an encrypted key is refused", Identity::from_pem(&pem, "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMAA=\n-----END ENCRYPTED PRIVATE KEY-----\n").is_err());
}

fn groups(t: &mut T) {
    // (group, client share, server share, shared secret)
    let sizes = [
        (kx::X25519, 32, 32, 32),
        (kx::SECP256R1, 65, 65, 32),
        (kx::SECP384R1, 97, 97, 48),
        (kx::X25519_MLKEM768, 1216, 1120, 64),
        (kx::SECP256R1_MLKEM768, 1249, 1153, 64),
        (kx::SECP384R1_MLKEM1024, 1665, 1665, 80),
    ];
    for (g, cl, sl, kl) in sizes {
        let Some(c) = kx::Share::new(g) else {
            t.check(&format!("kx {:#06x}: a share", g), false);
            continue;
        };
        let r = kx::respond(g, &c.public);
        let ok = r.as_ref().is_some_and(|(s, k)| s.len() == sl && k.len() == kl && c.finish(s).as_deref() == Some(&k[..]));
        t.check(&format!("kx {:#06x}: both sides agree, the lengths of the draft", g), c.public.len() == cl && ok);
        t.check(&format!("kx {:#06x}: a share a byte short is refused", g), kx::respond(g, &c.public[1..]).is_none() && r.as_ref().is_some_and(|(s, _)| c.finish(&s[1..]).is_none()));
        if let Some((s, k)) = &r {
            let mut s2 = s.clone();
            let at = if g == kx::X25519_MLKEM768 { 5 } else { s2.len() - 5 };
            s2[at] ^= 1;
            // ML-KEM rejects implicitly (another secret); a changed ECDHE point gives another
            // secret or none
            t.check(&format!("kx {:#06x}: a changed answer gives no or another secret", g), c.finish(&s2).as_deref() != Some(&k[..]));
        }
    }
    t.check("kx: an unknown group", kx::Share::new(0x0019).is_none() && kx::respond(0x0019, &[0; 32]).is_none());
    // an encapsulation key that fails FIPS 203's modulus check (a coefficient of 4095)
    if let Some(c) = kx::Share::new(kx::X25519_MLKEM768) {
        let mut bad = c.public.clone();
        bad[0] = 0xff;
        bad[1] |= 0x0f;
        t.check("kx: an ML-KEM key above the modulus is refused", kx::respond(kx::X25519_MLKEM768, &bad).is_none());
    }
}

/// client and server Conns feeding each other until both are done or one fails
pub fn pump(c: &mut Conn, s: &mut Conn) -> Result<(), TlsError> {
    for _ in 0..20 {
        let a = c.take_out();
        if !a.is_empty() {
            s.feed(&a)?;
        }
        let b = s.take_out();
        if !b.is_empty() {
            c.feed(&b)?;
        }
        if c.established() && s.established() && a.is_empty() && b.is_empty() {
            return Ok(());
        }
    }
    Err(TlsError::Closed)
}

/// a handshake in memory, then a message each way
fn talk(ccfg: ClientConfig, scfg: ServerConfig) -> Result<(Conn, Conn), TlsError> {
    let mut c = Conn::client(ccfg);
    let mut s = Conn::server(scfg);
    pump(&mut c, &mut s)?;
    c.write_plain(b"ping")?;
    s.feed(&c.take_out())?;
    s.write_plain(b"pong")?;
    c.feed(&s.take_out())?;
    let mut b = [0u8; 8];
    let (n, m) = (s.read_plain(&mut b), c.read_plain(&mut b));
    if n != 4 || m != 4 {
        return Err(TlsError::Closed);
    }
    Ok((c, s))
}

fn handshakes(t: &mut T) {
    let kinds = [Kind::Ed25519, Kind::P256, Kind::P384, Kind::Rsa];
    let pkis: Vec<Pki> = kinds.iter().enumerate().map(|(i, &k)| pki(30 + 10 * i as u8, k)).collect();
    for (i, p) in pkis.iter().enumerate() {
        let r = talk(ClientConfig::new(Trust::Anchors(alloc::vec![p.ca.clone()]), "localhost"), ServerConfig::new(p.server.clone()));
        let what = format!("{:?} server", kinds[i]);
        match r {
            Ok((c, s)) => {
                t.check(&format!("{}: TLS 1.3, ChaCha20, X25519MLKEM768", what), c.version() == "TLS 1.3" && c.suite() == 0x1303 && c.group() == kx::X25519_MLKEM768 && s.group() == kx::X25519_MLKEM768);
                t.check(&format!("{}: its certificate", what), c.peer_certs().first().is_some_and(|x| x.key == p.server.key.public_key()));
            }
            Err(e) => t.check(&format!("{}: handshake: {}", what, e), false),
        }
        // mutual TLS: a client of the next kind, its CA trusted by the server
        let q = &pkis[(i + 1) % 4];
        let ccfg = ClientConfig { identity: Some(q.client.clone()), ..ClientConfig::new(Trust::Anchors(alloc::vec![p.ca.clone()]), "localhost") };
        let scfg = ServerConfig { client_auth: Some(Trust::Anchors(alloc::vec![q.ca.clone()])), ..ServerConfig::new(p.server.clone()) };
        let r = talk(ccfg, scfg);
        let what = format!("mTLS: {:?} server, {:?} client", kinds[i], kinds[(i + 1) % 4]);
        t.check(&what, r.is_ok_and(|(_, s)| s.peer_certs().first().is_some_and(|x| x.subject_cn == "agent-1")));
        // the client's certificate of a CA the server does not trust
        let ccfg = ClientConfig { identity: Some(q.client.clone()), ..ClientConfig::new(Trust::Anchors(alloc::vec![p.ca.clone()]), "localhost") };
        let scfg = ServerConfig { client_auth: Some(Trust::Anchors(alloc::vec![p.ca.clone()])), ..ServerConfig::new(p.server.clone()) };
        t.check(&format!("{}: a client of another CA is refused", what), talk(ccfg, scfg).is_err());
    }
    // a certificate whose key is not the identity's: the client refuses the CertificateVerify
    let p = &pkis[1];
    let wrong = Identity { key: key(Kind::P256, 99), ..p.server.clone() };
    let r = talk(ClientConfig::new(Trust::Anchors(alloc::vec![p.ca.clone()]), "localhost"), ServerConfig::new(wrong));
    t.check("a server signing with another key is refused (decrypt_error)", matches!(r, Err(TlsError::PeerAlert(51)) | Err(TlsError::Alert(51, _))));
    // a server whose key the client's signature schemes do not cover: none here, every kind is
    // offered; a key of a curve TLS 1.3 ties to another hash still signs with its own
    t.check("P-384 server: ECDSA with SHA-384", pkis[2].server.key.can(netio::x509::SigAlg::Ecdsa(crypto::HashAlg::Sha384)));
    // a certificate of over 16 KiB (900 names): its Certificate message goes in several records
    let p = &pkis[1];
    let names: Vec<String> = (0..900).map(|i| format!("host-{:04}.web.lab", i)).collect();
    let mut all: Vec<&str> = alloc::vec!["localhost"];
    all.extend(names.iter().map(|s| s.as_str()));
    let big = issue(&p.server.key, "localhost", &all, Some((&p.ca, &p.ca_key)), 98);
    t.check("a certificate of over 16 KiB", big.len() > 16 * 1024);
    let r = talk(ClientConfig::new(Trust::Anchors(alloc::vec![p.ca.clone()]), "localhost"), ServerConfig::new(Identity::new(alloc::vec![big], p.server.key.clone())));
    t.check("... is sent in several records and taken", r.is_ok_and(|(c, _)| c.peer_certs().first().is_some_and(|x| x.names("host-0899.web.lab"))));
    // a server name no ClientHello can carry: the connection is closed before it begins
    let c = Conn::client(ClientConfig::new(Trust::Any, &"a".repeat(70_000)));
    t.check("a server name of 70 000 bytes: no ClientHello", c.closed());
}

/// a client whose key signs in none of the schemes a CertificateRequest lists sends no
/// certificate (RFC 8446 4.4.2): here the request, rewritten in flight, lists Ed25519 alone
fn client_cert_schemes(t: &mut T) {
    let p = pki(90, Kind::P256);
    let r = pki(91, Kind::Rsa);
    let run = |required: bool| -> Result<(Conn, Conn), TlsError> {
        let ccfg = ClientConfig { identity: Some(r.client.clone()), ..ClientConfig::new(Trust::Anchors(alloc::vec![p.ca.clone()]), "localhost") };
        let scfg = ServerConfig { client_auth: Some(Trust::Anchors(alloc::vec![r.ca.clone()])), client_auth_optional: !required, ..ServerConfig::new(p.server.clone()) };
        let mut c = Conn::client(ccfg);
        let mut s = Conn::server(scfg);
        s.test_cr_sig_algs(alloc::vec![0x0807]);
        pump(&mut c, &mut s)?;
        Ok((c, s))
    };
    t.check("client certificate: no scheme of the request fits: none sent, accepted when optional", run(false).is_ok_and(|(_, s)| s.peer_certs().is_empty()));
    t.check("client certificate: ... refused when required (certificate_required)", matches!(run(true), Err(TlsError::PeerAlert(116)) | Err(TlsError::Alert(116, _))));
}

fn sni(t: &mut T) {
    let p = pki(70, Kind::P256);
    let r = pki(71, Kind::Rsa);
    let id = |k: &PrivateKey, ca: &Pki, name: &str, serial: u8| Identity::new(alloc::vec![issue(k, name, &[name], Some((&ca.ca, &ca.ca_key)), serial)], k.clone());
    let a = id(&key(Kind::P256, 72), &p, "a.lab", 72);
    let b = id(&key(Kind::Rsa, 71), &r, "b.lab", 73);
    let scfg = ServerConfig { identities: alloc::vec![a, b], ..ServerConfig::new(p.server.clone()) };
    let both = Trust::Anchors(alloc::vec![p.ca.clone(), r.ca.clone()]);
    for (name, want) in [("a.lab", "a.lab"), ("b.lab", "b.lab"), ("localhost", "localhost")] {
        let x = talk(ClientConfig::new(both.clone(), name), scfg.clone());
        t.check(&format!("sni: {} gets its certificate", name), x.is_ok_and(|(c, s)| c.peer_certs().first().is_some_and(|l| l.subject_cn == want) && s.server_name() == name));
    }
    // a name no certificate has: the default one (which the client then refuses for the name)
    let x = talk(ClientConfig::new(Trust::Any, "c.lab"), scfg.clone());
    t.check("sni: another name gets the default certificate", x.is_ok_and(|(c, _)| c.peer_certs().first().is_some_and(|l| l.subject_cn == "localhost")));
    t.check("sni: ... which a checking client refuses", talk(ClientConfig::new(both.clone(), "c.lab"), scfg.clone()).is_err());
    // the program's own choice comes first
    let chosen = id(&key(Kind::P384, 74), &p, "picked.lab", 74);
    let sel: tls::Select = Arc::new(move |h: &tls::Hello| (h.server_name == "a.lab").then(|| chosen.clone()));
    let scfg2 = ServerConfig { select: Some(sel), ..scfg.clone() };
    let x = talk(ClientConfig::new(Trust::Any, "a.lab"), scfg2.clone());
    t.check("sni: ServerConfig::select comes first", x.is_ok_and(|(c, _)| c.peer_certs().first().is_some_and(|l| l.subject_cn == "picked.lab")));
    let x = talk(ClientConfig::new(Trust::Any, "b.lab"), scfg2);
    t.check("sni: ... and None from it leaves the choice to the lists", x.is_ok_and(|(c, _)| c.peer_certs().first().is_some_and(|l| l.subject_cn == "b.lab")));
}

fn alpn(t: &mut T) {
    let p = pki(80, Kind::P256);
    let trust = Trust::Anchors(alloc::vec![p.ca.clone()]);
    let v = |l: &[&str]| l.iter().map(|s| String::from(*s)).collect::<Vec<String>>();
    let go = |client: &[&str], server: &[&str]| {
        let ccfg = ClientConfig { alpn: v(client), ..ClientConfig::new(trust.clone(), "localhost") };
        let scfg = ServerConfig { alpn: v(server), ..ServerConfig::new(p.server.clone()) };
        talk(ccfg, scfg).map(|(c, s)| (c.alpn().map(String::from), s.alpn().map(String::from)))
    };
    let both = |x: &str| Ok((Some(String::from(x)), Some(String::from(x))));
    t.check("alpn: http/1.1", go(&["http/1.1"], &["h2", "http/1.1"]) == both("http/1.1"));
    t.check("alpn: the server's preference", go(&["h2", "x"], &["x", "h2"]) == both("x"));
    t.check("alpn: a server without ALPN answers none", go(&["h2"], &[]) == Ok((None, None)));
    t.check("alpn: a client without ALPN gets none", go(&[], &["h2"]) == Ok((None, None)));
    let r = go(&["spdy/1"], &["h2"]);
    t.check("alpn: no protocol in common is no_application_protocol", matches!(r, Err(TlsError::PeerAlert(120)) | Err(TlsError::Alert(120, _))));
    // names a ClientHello cannot carry are left out, and without names no ALPN goes out
    let long = "x".repeat(256);
    t.check("alpn: an empty name and one of 256 bytes are not offered", go(&["", &long], &["h2"]) == Ok((None, None)));
    t.check("alpn: ... the others are", go(&["", &long, "h2"], &["h2"]) == both("h2"));
    let many: Vec<String> = (0..100).map(|i| format!("{:0200}", i)).collect();
    let mut names: Vec<&str> = alloc::vec!["h2"];
    names.extend(many.iter().map(|s| s.as_str()));
    t.check("alpn: 20 KB of names: the first 8 KiB offered", go(&names, &["h2"]) == both("h2"));
    t.check("alpn: ... the rest not", matches!(go(&names, &[many[99].as_str()]), Err(TlsError::PeerAlert(120)) | Err(TlsError::Alert(120, _))));
    // a server that chooses a name the client left out (past the 8 KiB) is refused like one that
    // chooses a name never configured: a TLS 1.2 ServerHello carries the answer in the clear
    let answer = |name: &str| {
        let accept_all: tls::Verifier = Arc::new(|_, _, _| Ok(()));
        let ccfg = ClientConfig { alpn: names.iter().map(|s| String::from(*s)).collect(), ..ClientConfig::new(Trust::Custom(accept_all), "localhost") };
        let mut c = Conn::client(ccfg);
        let ch = c.take_out();
        let sid = ch.get(44..44 + *ch.get(43).unwrap_or(&0) as usize).unwrap_or_default().to_vec();
        let mut exts = Vec::new();
        for (ty, data) in [(23u16, Vec::new()), (0xff01, alloc::vec![0]), (16, [&((name.len() + 1) as u16).to_be_bytes()[..], &[name.len() as u8], name.as_bytes()].concat())] {
            exts.extend_from_slice(&ty.to_be_bytes());
            exts.extend_from_slice(&(data.len() as u16).to_be_bytes());
            exts.extend_from_slice(&data);
        }
        let mut body = alloc::vec![3u8, 3];
        body.extend_from_slice(&[0x5a; 32]);
        body.push(sid.len() as u8);
        body.extend_from_slice(&sid);
        body.extend_from_slice(&[0xc0, 0x2f, 0]);
        body.extend_from_slice(&(exts.len() as u16).to_be_bytes());
        body.extend_from_slice(&exts);
        let mut hs = alloc::vec![2u8, 0, (body.len() >> 8) as u8, body.len() as u8];
        hs.extend_from_slice(&body);
        let mut rec = alloc::vec![22u8, 3, 3, (hs.len() >> 8) as u8, hs.len() as u8];
        rec.extend_from_slice(&hs);
        c.feed(&rec)
    };
    t.check("alpn: a TLS 1.2 server choosing an offered name is taken", answer("h2").is_ok());
    t.check("alpn: ... one the client left out is illegal_parameter", matches!(answer(&many[99]), Err(TlsError::Alert(47, _))));
    t.check("alpn: ... one never configured too", matches!(answer("spdy/3"), Err(TlsError::Alert(47, _))));
    // the client's view of the names: the bytes it sent
    let seen = Arc::new(plat::sync::Mutex::new(Vec::new()));
    let s2 = seen.clone();
    let sel: tls::Select = Arc::new(move |h: &tls::Hello| {
        *s2.lock() = h.alpn.clone();
        None
    });
    let ccfg = ClientConfig { alpn: v(&["h2", "http/1.1"]), ..ClientConfig::new(trust.clone(), "localhost") };
    let scfg = ServerConfig { select: Some(sel), ..ServerConfig::new(p.server.clone()) };
    let ok = talk(ccfg, scfg).is_ok();
    t.check("alpn: Hello::alpn has the names as bytes", ok && *seen.lock() == alloc::vec![b"h2".to_vec(), b"http/1.1".to_vec()]);
}
