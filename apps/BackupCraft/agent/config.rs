// config -- the agent's data directory: its settings, key, certificate and the server's CA
//
//   agent.json   host id, name, server URL, CA fingerprint, address, shared folders, whether
//                the server may change them
//   agent.key    the Ed25519 seed (hex; only the owner can read the file)
//   agent.pem    the certificate the server's CA issued at pairing
//   ca.pem       the server's CA

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::j::{self, Json};
use crypto::ed25519::Keypair;
use netio::tls::Identity;
use netio::x509::Cert;

bc_proto::record! {
    pub struct Config {
        pub host_id: String,
        pub name: String,
        /// the server's URL (https://host:port)
        pub server: String,
        /// SHA-256 of the server's CA (hex)
        pub ca_fingerprint: String,
        /// where the server reaches this agent ("" = the address it sees)
        pub address: String,
        /// the address the agent listens on
        pub listen: String,
        /// folders the server may back up (read)
        pub roots: Vec<String>,
        /// folders the server may restore into (write; the backup folders too)
        pub restore_roots: Vec<String>,
        /// the server may change the folders (bc-agent roots --lock forbids it)
        pub remote_roots: bool,
    }
}

pub fn default_dir() -> String {
    if cfg!(windows) {
        let base = plat::env("ProgramData").unwrap_or_else(|| String::from("C:\\ProgramData"));
        return plat::fs::join(&plat::fs::join(&base, "BackupCraft"), "agent");
    }
    if plat::env("USER").as_deref() == Some("root") {
        return String::from("/var/lib/backupcraft-agent");
    }
    let home = plat::env("HOME").unwrap_or_else(|| String::from("."));
    plat::fs::join(&home, ".local/share/backupcraft/agent")
}

/// the data directory of this run (--data, absolute)
pub fn data_dir() -> String {
    let d = plat::arg_value("--data").unwrap_or_else(default_dir);
    if bc_fs::absolute(&d, cfg!(windows)) { d } else { plat::fs::join(&plat::cwd(), &d) }
}

fn path(dir: &str, name: &str) -> String {
    plat::fs::join(dir, name)
}

pub fn load(dir: &str) -> Option<Config> {
    let b = plat::read_file(&path(dir, "agent.json"))?;
    Config::from_json(&j::parse(&b)?)
}

pub fn save(dir: &str, c: &Config) -> Result<(), String> {
    let p = path(dir, "agent.json");
    let tmp = format!("{}.tmp", p);
    if plat::fs::write_file_private(&tmp, j::text(&c.to_json()).as_bytes()) && plat::rename(&tmp, &p) {
        Ok(())
    } else {
        Err(format!("{} kann nicht geschrieben werden", p))
    }
}

/// everything a paired agent needs to talk TLS
pub struct Paired {
    pub cfg: Config,
    pub identity: Identity,
    pub ca: Cert,
}

fn read_cert(p: &str) -> Option<Cert> {
    let b = plat::read_file(p)?;
    Cert::parse(&netio::pem::decode(&String::from_utf8_lossy(&b), "CERTIFICATE")?)
}

pub fn load_paired(dir: &str) -> Result<Paired, String> {
    let cfg = load(dir).filter(|c| !c.host_id.is_empty()).ok_or_else(|| format!("nicht gekoppelt ({} fehlt) – zuerst: bc-agent pair --server URL --code CODE", path(dir, "agent.json")))?;
    let seed = plat::read_file(&path(dir, "agent.key")).and_then(|b| crypto::unhex(String::from_utf8_lossy(&b).trim())).filter(|s| s.len() == 32).ok_or("agent.key fehlt oder ist beschädigt")?;
    let mut s = [0u8; 32];
    s.copy_from_slice(&seed);
    let key = Keypair::from_seed(&s);
    crypto::zeroize(&mut s);
    let cert = read_cert(&path(dir, "agent.pem")).ok_or("agent.pem fehlt oder ist beschädigt")?;
    let ca = read_cert(&path(dir, "ca.pem")).ok_or("ca.pem fehlt oder ist beschädigt")?;
    if cert.public_key != key.public {
        return Err(String::from("agent.pem passt nicht zu agent.key"));
    }
    if crypto::hex(&crypto::sha256(&ca.der)) != cfg.ca_fingerprint {
        return Err(String::from("ca.pem ist nicht die Zertifizierungsstelle der Kopplung"));
    }
    Ok(Paired { cfg, identity: Identity { chain: alloc::vec![cert.der.clone(), ca.der.clone()], key }, ca })
}

/// store the result of a pairing
pub fn save_paired(dir: &str, cfg: &Config, seed: &[u8; 32], cert_pem: &str, ca_pem: &str) -> Result<(), String> {
    if !plat::fs::mkdir_private(dir) {
        return Err(format!("{} kann nicht angelegt werden", dir));
    }
    let ok = plat::fs::write_file_private(&path(dir, "agent.key"), crypto::hex(seed).as_bytes())
        && plat::fs::write_file_private(&path(dir, "agent.pem"), cert_pem.as_bytes())
        && plat::fs::write_file_private(&path(dir, "ca.pem"), ca_pem.as_bytes());
    if !ok {
        return Err(format!("{} kann nicht beschrieben werden", dir));
    }
    save(dir, cfg)
}
