// pair -- couple this machine with a backup server
//
//   1. connect to the server (https) and read its CA from the TLS handshake; the CA's SHA-256
//      fingerprint must be the one the console shows next to the pairing code (--fingerprint),
//      or the user confirms it here (--yes skips the question)
//   2. a new Ed25519 key; enrol with the one-time code over a connection that trusts exactly
//      that CA and checks the server's name
//   3. keep the certificate the CA issued, the CA and the settings

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::api::{EnrollRequest, EnrollResponse};
use bc_proto::j::{self, Json};
use bc_proto::model::Os;
use crypto::ed25519::Keypair;
use netio::client::json_headers;
use netio::tls::Identity;
use netio::trust::{self, fingerprint_text, Presented, ServerTrust};
use netio::x509::Cert;
use netio::{Client, Url};
use plat::sync::Mutex;

use crate::config::{self, Config};
use crate::{out, platform};

/// a connector that trusts the server's own CA and checks its name
pub fn connector(ca: Cert, identity: Option<Identity>) -> netio::client::Connector {
    trust::connector(ServerTrust::Ca(ca), identity, Arc::new(Mutex::new(None)))
}

/// the CA the server presents (its chain must end in a CA that signed the server certificate,
/// valid for the server's name)
fn probe_ca(url: &Url) -> Result<Cert, String> {
    let seen: Arc<Mutex<Option<Presented>>> = Arc::new(Mutex::new(None));
    let c = Client::new().with_connector(trust::connector(ServerTrust::Ask, None, seen.clone()));
    let health = Url::parse(&format!("https://{}/health", url.host_header())).ok_or("ungültige Adresse")?;
    let r = c.get(&health, &json_headers(""));
    match seen.lock().take() {
        Some(p) => netio::pem::decode(&p.pem, "CERTIFICATE").and_then(|d| Cert::parse(&d)).ok_or_else(|| String::from("unlesbare Zertifizierungsstelle")),
        None => Err(match r {
            Err(e) => format!("{}", e),
            Ok(_) => String::from("keine TLS-Verbindung"),
        }),
    }
}

fn confirm(question: &str) -> bool {
    out(question);
    let mut buf = [0u8; 64];
    match plat::read_stdin(&mut buf) {
        Ok(n) => matches!(buf[..n].trim_ascii(), b"j" | b"J" | b"ja" | b"y" | b"Y" | b"yes"),
        Err(_) => false,
    }
}

/// the values of a repeated option ("--root A --root B")
pub fn values(name: &str) -> Vec<String> {
    let a = plat::args();
    let mut v = Vec::new();
    let mut i = 1;
    while i + 1 < a.len() {
        if a[i] == name {
            v.push(a[i + 1].clone());
            i += 1;
        }
        i += 1;
    }
    v
}

pub fn pair(dir: &str) -> Result<String, String> {
    let server = plat::arg_value("--server").ok_or("--server URL fehlt (die Adresse aus der Console)")?;
    let code = plat::arg_value("--code").ok_or("--code fehlt (der Pairing-Code aus der Console)")?;
    let server = if server.contains("://") { server } else { format!("https://{}", server) };
    let mut url = Url::parse(&server).ok_or_else(|| format!("ungültige Serveradresse {}", server))?;
    if !url.is_https() {
        return Err(String::from("Agenten koppeln sich nur über https"));
    }
    if !server.trim_start_matches("https://").contains(':') && url.port == 443 {
        url.port = bc_proto::DEFAULT_PORT;
    }
    let base = format!("https://{}", url.host_header());
    if config::load(dir).is_some_and(|c| !c.host_id.is_empty()) && !plat::args().iter().any(|a| a == "--force") {
        return Err(format!("dieser Rechner ist bereits gekoppelt ({}); mit --force neu koppeln", plat::fs::join(dir, "agent.json")));
    }
    let win = cfg!(windows);
    let roots = bc_fs::clean_roots(&values("--root"), win)?;
    let restore_roots = bc_fs::clean_roots(&values("--restore-root"), win)?;

    // 1. the CA
    let ca = probe_ca(&url).map_err(|e| format!("{}: {}", base, e))?;
    let fp = crypto::sha256(&ca.der);
    let fp_hex = crypto::hex(&fp);
    match plat::arg_value("--fingerprint") {
        Some(want) if trust::parse_fingerprint(&want) == Some(fp) => {}
        Some(_) => return Err(format!("der Server zeigt eine andere Zertifizierungsstelle:\n  {}\nAbbruch – das ist nicht der erwartete Server (oder ein Angriff auf die Verbindung).", fingerprint_text(&fp))),
        None => {
            out(&format!("Zertifizierungsstelle von {}:\n  {}\n  SHA-256 {}\n", base, ca.subject_cn, fingerprint_text(&fp)));
            let yes = plat::args().iter().any(|a| a == "--yes");
            if !yes && !confirm("Stimmt der Fingerprint mit der Anzeige in der Console überein? [j/N] ") {
                return Err(String::from("abgebrochen"));
            }
        }
    }

    // 2. enrol
    let seed: [u8; 32] = plat::random_bytes();
    let key = Keypair::from_seed(&seed);
    let name = plat::arg_value("--name").unwrap_or_else(plat::net::hostname);
    let req = EnrollRequest {
        code: String::from(code.trim()),
        name: name.clone(),
        os: if win { Os::Windows } else { Os::Linux },
        platform: platform(),
        version: String::from(bc_proto::VERSION),
        public_key: crypto::hex(&key.public),
        address: plat::arg_value("--address").unwrap_or_default(),
        roots: roots.clone(),
    };
    let client = Client::new().with_connector(connector(ca.clone(), None));
    let target = Url::parse(&format!("{}/api/agent/enroll", base)).ok_or("ungültige Adresse")?;
    let r = client.request("POST", &target, &json_headers(""), j::text(&req.to_json()).as_bytes()).map_err(|e| format!("{}: {}", base, e))?;
    let v = j::parse(r.body_bytes()).unwrap_or(j::Value::Null);
    if r.status != 201 && r.status != 200 {
        let m = j::str_of(&v, "message");
        return Err(if m.is_empty() { format!("der Server lehnt ab (HTTP {})", r.status) } else { String::from(m) });
    }
    let e = EnrollResponse::from_json(&v).ok_or("unerwartete Antwort des Servers")?;
    let cert = netio::pem::decode(&e.certificate, "CERTIFICATE").and_then(|d| Cert::parse(&d)).ok_or("der Server liefert kein gültiges Zertifikat")?;
    if cert.public_key != key.public || !cert.signed_by(&ca) || cert.subject_cn != e.host_id {
        return Err(String::from("das ausgestellte Zertifikat passt nicht"));
    }

    // 3. keep it
    let cfg = Config {
        host_id: e.host_id.clone(),
        name,
        server: base.clone(),
        ca_fingerprint: fp_hex,
        address: req.address,
        listen: plat::arg_value("--listen").unwrap_or_default(),
        roots,
        restore_roots,
        remote_roots: !plat::args().iter().any(|a| a == "--lock"),
    };
    config::save_paired(dir, &cfg, &seed, &e.certificate, &netio::pem::encode("CERTIFICATE", &ca.der))?;
    Ok(format!("Gekoppelt mit {} als {} ({}).\nDaten: {}\nJetzt den Agenten starten: bc-agent run", base, cfg.name, cfg.host_id, dir))
}
