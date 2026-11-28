// systrust -- the system's trust (Trust::System, netio::roots, plat::certs) on every machine:
// the system's checks against a CA of one's own (verify_system_roots: Windows CryptoAPI with an
// exclusive root, Apple's Security framework with only these anchors, Linux verify_chain) and
// the reason of each refusal; Trust::System's extra CAs (a company CA) through the system and in
// a handshake; a server's client_auth with System; on Linux the bundle of SSL_CERT_FILE and
// SSL_CERT_DIR; on Android the system's CA folder (only the system's CAs, docs/ANDROID.md AN2).
// Public servers: tools/tls_web_check.py --internet (badssl.com), limbo through the system:
// tools/limbo_check.py --system.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netio::roots::{self, Reason};
use netio::tls::{ClientConfig, Conn, Identity, ServerConfig, Trust};
use netio::x509::{Cert, CertBuilder, IpAddr, PrivateKey};

use crate::tlsweb::{key, pump, Kind};
use crate::T;

const NOW: i64 = 1_791_000_000;

/// a CA and a server certificate below it (P-256: every system takes it)
struct Pki {
    ca: Cert,
    ca_key: PrivateKey,
}

impl Pki {
    /// a CA valid from two years before `at` to three years after (the leaves below it: from
    /// 400 days before to 300 days after)
    fn new(seed: u8, at: i64) -> Pki {
        let ca_key = key(Kind::P256, seed);
        let b = CertBuilder {
            serial: [seed; 16],
            cn: format!("JFLib system trust test CA {}", seed),
            org: String::from("JFLib"),
            not_before: at - 2 * 365 * 86_400,
            not_after: at + 3 * 365 * 86_400,
            is_ca: true,
            spki: ca_key.spki(),
            ..CertBuilder::default()
        };
        let ca = Cert::parse(&b.sign_key(None, &ca_key, &[]).unwrap_or_default()).unwrap_or_default();
        Pki { ca, ca_key }
    }

    /// a server's certificate for web.lab and 127.0.0.1, valid in [from, until]
    fn leaf(&self, seed: u8, from: i64, until: i64, server_auth: bool) -> Identity {
        let k = key(Kind::P256, seed);
        let b = CertBuilder {
            serial: [seed; 16],
            cn: String::from("web.lab"),
            not_before: from,
            not_after: until,
            server_auth,
            client_auth: !server_auth,
            dns: alloc::vec![String::from("web.lab")],
            ips: alloc::vec![IpAddr::LOCALHOST_V4],
            spki: k.spki(),
            ..CertBuilder::default()
        };
        Identity::new(alloc::vec![b.sign_key(Some(&self.ca), &self.ca_key, &[]).unwrap_or_default()], k)
    }

    /// a server's certificate valid from an hour before `at` to 200 days after
    fn good(&self, at: i64) -> Identity {
        self.leaf(60, at - 3600, at + 200 * 86_400, true)
    }
}

/// the clock's time (Unix seconds): a handshake checks against it
fn clock() -> i64 {
    plat::unix_time_ns() / 1_000_000_000
}

fn chain(id: &Identity) -> Vec<Cert> {
    id.chain.iter().filter_map(|d| Cert::parse(d)).collect()
}

/// the handshake in memory; Err: the client's error as text
fn handshake(ccfg: ClientConfig, scfg: ServerConfig) -> Result<(), String> {
    let mut c = Conn::client(ccfg);
    let mut s = Conn::server(scfg);
    pump(&mut c, &mut s).map_err(|e| format!("{}", e))
}

pub fn run(t: &mut T) {
    roots_check(t);
    extra_cas(t);
    #[cfg(target_os = "linux")]
    bundle_env(t);
    #[cfg(target_os = "android")]
    android_bundle(t);
}

/// the system's checks against only our CA: accepted, and each refusal with its reason
fn roots_check(t: &mut T) {
    let p = Pki::new(71, NOW);
    let other = Pki::new(72, NOW);
    let ca = core::slice::from_ref(&p.ca);
    let leaf = chain(&p.good(NOW));
    let r = roots::verify_system_roots(&leaf, "web.lab", ca, NOW);
    t.check(&format!("system roots: a server below the CA ({:?})", r.as_ref().err()), r.is_ok());
    let r = roots::verify_system_roots(&leaf, "WEB.lab.", ca, NOW);
    t.check(&format!("system roots: the name in capitals, with the root's dot ({:?})", r.as_ref().err()), r.is_ok());
    let r = roots::verify_system_roots(&leaf, "127.0.0.1", ca, NOW);
    t.check(&format!("system roots: its IP address ({:?})", r.as_ref().err()), r.is_ok());
    let r = roots::verify_system_roots(&leaf, "other.lab", ca, NOW);
    t.check(&format!("system roots: another name -> Name ({:?})", r), r.as_ref().is_err_and(|e| e.reason == Reason::Name));
    let r = roots::verify_system_roots(&leaf, "127.0.0.2", ca, NOW);
    t.check(&format!("system roots: another IP address -> Name ({:?})", r), r.as_ref().is_err_and(|e| e.reason == Reason::Name));
    let r = roots::verify_system_roots(&leaf, "web.lab", core::slice::from_ref(&other.ca), NOW);
    t.check(&format!("system roots: another CA -> UnknownCa ({:?})", r), r.as_ref().is_err_and(|e| e.reason == Reason::UnknownCa));
    let old = chain(&p.leaf(73, NOW - 400 * 86_400, NOW - 86_400, true));
    let r = roots::verify_system_roots(&old, "web.lab", ca, NOW);
    t.check(&format!("system roots: an expired certificate -> Expired ({:?})", r), r.as_ref().is_err_and(|e| e.reason == Reason::Expired));
    let r = roots::verify_system_roots(&leaf, "web.lab", ca, NOW + 300 * 86_400);
    t.check(&format!("system roots: the time given counts (a year later: Expired) ({:?})", r), r.as_ref().is_err_and(|e| e.reason == Reason::Expired));
    // the extra CAs through the system (Windows: CryptoAPI with them as its roots where its own
    // give no path; Apple: anchors beside the system's; Linux: after the bundle)
    let r = roots::verify_system(&leaf, "web.lab", ca, None, NOW);
    t.check(&format!("system with extra CAs: a server below one ({:?})", r.as_ref().err()), r.is_ok());
    let r = roots::verify_system(&leaf, "other.lab", ca, None, NOW);
    t.check(&format!("system with extra CAs: another name -> Name ({:?})", r), r.as_ref().is_err_and(|e| e.reason == Reason::Name));
    let r = roots::verify_system(&leaf, "web.lab", core::slice::from_ref(&other.ca), None, NOW);
    t.check(&format!("system with extra CAs: not below one -> UnknownCa ({:?})", r), r.as_ref().is_err_and(|e| e.reason == Reason::UnknownCa));
    let client = chain(&p.leaf(74, NOW - 3600, NOW + 200 * 86_400, false));
    let r = roots::verify_system_roots(&client, "web.lab", ca, NOW);
    t.check(&format!("system roots: a client's certificate for a server is refused ({:?})", r), r.is_err());
}

/// Trust::System(extra) in a handshake: the extra CA where the system knows none of the chain
fn extra_cas(t: &mut T) {
    // the handshake checks at the clock's time: the CA and the leaves count from it
    let now = clock();
    let p = Pki::new(81, now);
    let server = p.good(now);
    let sys = || Trust::System(alloc::vec![p.ca.clone()]);
    let r = handshake(ClientConfig::new(sys(), "web.lab"), ServerConfig::new(server.clone()));
    t.check(&format!("System(extra): a server of the extra CA ({:?})", r.as_ref().err()), r.is_ok());
    let r = handshake(ClientConfig::new(Trust::System(Vec::new()), "web.lab"), ServerConfig::new(server.clone()));
    t.check(&format!("System: without the extra CA it is not trusted ({:?})", r), r.as_ref().is_err_and(|e| e.contains("not trusted")));
    let r = handshake(ClientConfig::new(sys(), "other.lab"), ServerConfig::new(server.clone()));
    t.check(&format!("System(extra): another name ({:?})", r), r.as_ref().is_err_and(|e| e.contains("another name")));
    let old = p.leaf(83, now - 400 * 86_400, now - 86_400, true);
    let r = handshake(ClientConfig::new(sys(), "web.lab"), ServerConfig::new(old));
    t.check(&format!("System(extra): an expired certificate ({:?})", r), r.as_ref().is_err_and(|e| e.contains("expired")));
    // a server that takes clients of the extra CA
    let client = p.leaf(84, now - 3600, now + 200 * 86_400, false);
    let scfg = ServerConfig { client_auth: Some(sys()), ..ServerConfig::new(server.clone()) };
    let ccfg = ClientConfig { identity: Some(client), ..ClientConfig::new(Trust::Anchors(alloc::vec![p.ca.clone()]), "web.lab") };
    let r = handshake(ccfg, scfg);
    t.check(&format!("System(extra) as a server's client_auth: a client of the extra CA ({:?})", r.as_ref().err()), r.is_ok());
}

/// Linux: SSL_CERT_FILE and SSL_CERT_DIR choose the bundle
#[cfg(target_os = "linux")]
fn bundle_env(t: &mut T) {
    let p = Pki::new(91, NOW);
    let leaf = chain(&p.good(NOW));
    let pem = netio::pem::encode("CERTIFICATE", &p.ca.der);
    let dir = format!("{}/coretest-systrust-{:x}", plat::temp_dir().unwrap_or_else(|| String::from("/tmp")), plat::random_u64() & 0xFFFF_FFFF);
    let made = plat::mkdir(&dir) && plat::write_file(&format!("{}/test-ca.pem", dir), pem.as_bytes());
    t.check("bundle: the test folder", made);
    let (file0, dir0) = (plat::env("SSL_CERT_FILE"), plat::env("SSL_CERT_DIR"));

    plat::setenv("SSL_CERT_FILE", Some(&format!("{}/test-ca.pem", dir)));
    plat::setenv("SSL_CERT_DIR", None);
    roots::reload();
    let r = roots::verify_system(&leaf, "web.lab", &[], None, NOW);
    t.check(&format!("bundle: SSL_CERT_FILE names it ({:?})", r.as_ref().err()), r.is_ok() && roots::system_anchors().len() == 1);

    plat::setenv("SSL_CERT_FILE", Some(&format!("{}/missing.pem", dir)));
    roots::reload();
    let r = roots::verify_system(&leaf, "web.lab", &[], None, NOW);
    t.check(&format!("bundle: SSL_CERT_FILE names a missing file: no CA, not the distribution's ({:?})", r), r.is_err() && roots::system_anchors().is_empty());

    plat::setenv("SSL_CERT_DIR", Some(&format!("/nonexistent-coretest:{}/", dir)));
    roots::reload();
    let r = roots::verify_system(&leaf, "web.lab", &[], None, NOW);
    t.check(&format!("bundle: SSL_CERT_DIR's second folder holds it ({:?})", r.as_ref().err()), r.is_ok());

    plat::setenv("SSL_CERT_FILE", file0.as_deref());
    plat::setenv("SSL_CERT_DIR", dir0.as_deref());
    roots::reload();
    let r = roots::verify_system(&leaf, "web.lab", &[], None, NOW);
    t.check(&format!("bundle: the distribution's again, our CA not in it ({:?})", r), r.as_ref().is_err_and(|e| e.reason == Reason::UnknownCa));
    plat::delete(&format!("{}/test-ca.pem", dir));
    plat::delete(&dir);
}

/// Android: the system's CA folder (Conscrypt's APEX on Android 14 and later, else the system
/// image's), only the system's CAs: SSL_CERT_FILE changes nothing
#[cfg(target_os = "android")]
fn android_bundle(t: &mut T) {
    roots::reload();
    let anchors = roots::system_anchors();
    // AOSP carries about 140 CAs; Let's Encrypt's root is among them since Android 7.1.1
    t.check(&format!("Android: the system's CAs ({})", anchors.len()), anchors.len() >= 100);
    t.check("Android: ISRG Root X1 among them", anchors.iter().any(|c| c.subject_cn == "ISRG Root X1" && c.is_ca));
    let dir = plat::certs::android_ca_dir();
    t.check("Android: the folder of the CAs is one of the system's", dir.is_some());
    plat::write_stdout(format!("systrust: the CAs of {}\n", dir.unwrap_or("?")).as_bytes());
    let p = Pki::new(91, NOW);
    let leaf = chain(&p.good(NOW));
    let file0 = plat::env("SSL_CERT_FILE");
    plat::setenv("SSL_CERT_FILE", Some("/nonexistent-coretest.pem"));
    roots::reload();
    t.check("Android: SSL_CERT_FILE does not choose the bundle", roots::system_anchors().len() == anchors.len());
    plat::setenv("SSL_CERT_FILE", file0.as_deref());
    roots::reload();
    let r = roots::verify_system(&leaf, "web.lab", &[], None, NOW);
    t.check(&format!("Android: a CA of one's own is not among them ({:?})", r), r.as_ref().is_err_and(|e| e.reason == Reason::UnknownCa));
}
