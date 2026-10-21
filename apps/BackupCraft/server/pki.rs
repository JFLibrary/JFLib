// pki -- the server's own certificate authority (Ed25519, ten years)
//
//   <data>/pki/ca.pem          the CA certificate (agents and consoles trust it)
//   <data>/pki/ca.key          its key, sealed with the master key
//   <data>/pki/server.*        the API's TLS certificate: this machine's names and addresses,
//                              re-issued when they change or within 30 days of expiry
//   <data>/pki/client.*        the server's identity towards agents (CN "BackupCraft Server")
//   agent certificates         CN = host id, client use only, one year.  The server reaches an
//                              agent by pinning exactly this certificate, so an agent's
//                              certificate never passes as a server certificate of the CA
//
// The CA fingerprint (SHA-256 of its certificate) is what an agent shows when it pairs.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crypto::ed25519::Keypair;
use netio::tls::Identity;
use netio::x509::{Cert, CertBuilder};
use plat::net::IpAddr;

use crate::keys::Keys;

const YEAR: i64 = 365 * 86_400;

pub struct Pki {
    pub ca: Cert,
    ca_key: Keypair,
    pub server: Identity,
    pub client: Identity,
}

fn now() -> i64 {
    crate::util::now()
}

fn read_cert(path: &str) -> Option<Cert> {
    let text = plat::read_file(path)?;
    Cert::parse(&netio::pem::decode(&String::from_utf8_lossy(&text), "CERTIFICATE")?)
}

fn read_key(keys: &Keys, path: &str, aad: &str) -> Option<Keypair> {
    let b = plat::read_file(path)?;
    let seed = keys.unseal(&String::from_utf8_lossy(&b), aad)?;
    let mut s = [0u8; 32];
    (seed.len() == 32).then(|| {
        s.copy_from_slice(&seed);
        Keypair::from_seed(&s)
    })
}

fn write_pair(keys: &Keys, dir: &str, name: &str, der: &[u8], key: &Keypair) -> Result<(), String> {
    let cp = plat::fs::join(dir, &format!("{}.pem", name));
    let kp = plat::fs::join(dir, &format!("{}.key", name));
    if !plat::fs::write_file_private(&cp, netio::pem::encode("CERTIFICATE", der).as_bytes())
        || !plat::fs::write_file_private(&kp, keys.seal(key.seed(), &format!("pki:{}", name)).as_bytes())
    {
        return Err(format!("{} kann nicht geschrieben werden", cp));
    }
    Ok(())
}

/// the names the API certificate carries
pub fn local_names(listen: &plat::net::SocketAddr, extra: &[String]) -> (Vec<String>, Vec<IpAddr>) {
    let mut dns = alloc::vec![String::from("localhost"), plat::net::hostname()];
    for e in extra {
        if IpAddr::parse(e).is_none() && !dns.contains(e) {
            dns.push(e.to_lowercase());
        }
    }
    let mut ips = alloc::vec![IpAddr::LOCALHOST_V4, IpAddr::LOCALHOST_V6];
    if !listen.ip.is_unspecified() && !ips.contains(&listen.ip) {
        ips.push(listen.ip);
    }
    for e in extra {
        if let Some(ip) = IpAddr::parse(e) {
            if !ips.contains(&ip) {
                ips.push(ip);
            }
        }
    }
    (dns, ips)
}

impl Pki {
    /// load the PKI of a data directory (created on first use; the API certificate follows
    /// the names)
    pub fn load(data: &str, keys: &Keys, dns: &[String], ips: &[IpAddr]) -> Result<Pki, String> {
        let dir = plat::fs::join(data, "pki");
        if !plat::fs::mkdir_private(&dir) {
            return Err(format!("{} kann nicht angelegt werden", dir));
        }
        let t = now();
        let (ca, ca_key) = match (read_cert(&plat::fs::join(&dir, "ca.pem")), read_key(keys, &plat::fs::join(&dir, "ca.key"), "pki:ca")) {
            (Some(c), Some(k)) if c.public_key == k.public => (c, k),
            (None, None) => {
                let k = Keypair::from_seed(&plat::random_bytes());
                let der = CertBuilder {
                    serial: plat::random_bytes(),
                    cn: format!("BackupCraft CA {}", plat::net::hostname()),
                    org: String::from("BackupCraft"),
                    not_before: t - 3600,
                    not_after: t + 10 * YEAR,
                    public_key: k.public,
                    is_ca: true,
                    ..CertBuilder::default()
                }
                .sign(None, &k);
                write_pair(keys, &dir, "ca", &der, &k)?;
                (Cert::parse(&der).ok_or("CA-Zertifikat unlesbar")?, k)
            }
            _ => return Err(String::from("die Zertifizierungsstelle (pki/ca.*) ist unvollständig oder passt nicht zum Master-Schlüssel")),
        };
        let issue = |name: &str, cn: &str, server: bool, client: bool, dns: &[String], ips: &[IpAddr]| -> Result<Identity, String> {
            let path = plat::fs::join(&dir, &format!("{}.pem", name));
            if let (Some(c), Some(k)) = (read_cert(&path), read_key(keys, &plat::fs::join(&dir, &format!("{}.key", name)), &format!("pki:{}", name))) {
                let names_ok = dns.iter().all(|d| c.dns.contains(d)) && ips.iter().all(|i| c.ips.contains(i));
                if c.public_key == k.public && c.signed_by(&ca) && c.not_after - t > 30 * 86_400 && names_ok {
                    return Ok(Identity { chain: alloc::vec![c.der.clone(), ca.der.clone()], key: k });
                }
            }
            let k = Keypair::from_seed(&plat::random_bytes());
            let der = CertBuilder {
                serial: plat::random_bytes(),
                cn: String::from(cn),
                org: String::from("BackupCraft"),
                not_before: t - 3600,
                not_after: t + 2 * YEAR,
                public_key: k.public,
                dns: Vec::from(dns),
                ips: Vec::from(ips),
                server_auth: server,
                client_auth: client,
                ..CertBuilder::default()
            }
            .sign(Some(&ca), &ca_key);
            write_pair(keys, &dir, name, &der, &k)?;
            Ok(Identity { chain: alloc::vec![der, ca.der.clone()], key: k })
        };
        let server = issue("server", &plat::net::hostname(), true, false, dns, ips)?;
        let client = issue("client", bc_proto::agent::SERVER_CN, false, true, &[], &[])?;
        Ok(Pki { ca, ca_key, server, client })
    }

    /// SHA-256 of the CA certificate as "AB:CD:..."
    pub fn ca_fingerprint(&self) -> String {
        netio::trust::fingerprint_text(&crypto::sha256(&self.ca.der))
    }

    /// an agent's certificate (PEM): CN = host id, its names, client use
    pub fn issue_agent(&self, host_id: &str, public_key: &[u8; 32], dns: &[String], ips: &[IpAddr]) -> String {
        let t = now();
        let der = CertBuilder {
            serial: plat::random_bytes(),
            cn: String::from(host_id),
            org: String::from("BackupCraft Agent"),
            not_before: t - 3600,
            not_after: t + YEAR,
            public_key: *public_key,
            dns: Vec::from(dns),
            ips: Vec::from(ips),
            server_auth: false,
            client_auth: true,
            ..CertBuilder::default()
        }
        .sign(Some(&self.ca), &self.ca_key);
        netio::pem::encode("CERTIFICATE", &der)
    }

    pub fn ca_pem(&self) -> String {
        netio::pem::encode("CERTIFICATE", &self.ca.der)
    }
}
