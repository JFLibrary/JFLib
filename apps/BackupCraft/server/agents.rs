// agents -- the server's side of the agents
//
//   enroll     POST /api/agent/enroll (public, TLS): a valid one-time pairing code and the
//              agent's Ed25519 key -> a host record and a certificate from the server's CA
//   heartbeat  POST /api/agent/heartbeat (mutual TLS): last seen, address, shared folders
//   sessions   the server connects to an agent (port 7680) with its client certificate and
//              trusts exactly the agent certificate it issued (pinned); one kept-alive session
//              per agent
//   calls      info, list, stat, read, roots, restore (open / write / commit / mkdir / symlink /
//              abort) -- what the engine and the API need
//   SSH push   install an agent over SSH (ssh.rs) and pair it automatically

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::agent::*;
use bc_proto::api::{code, EnrollRequest, EnrollResponse};
use bc_proto::j::{self, Json, Value};
use bc_proto::model::{Entry, EntryKind, Host, HostKind, HostStatus, Listing};
use netio::client::{json_headers, Session};
use netio::tls::{self, ClientConfig, Trust};
use netio::{Client, Stream, Url};
use plat::net::{IpAddr, TcpStream};
use plat::sync::Mutex;

use crate::api::Call;
use crate::ctx::{Ctx, C_HOSTS, C_PAIRING};
use crate::util::{self, bad, err, field, now, ok};

/// the URL agents and remote consoles reach the server at
pub fn public_url(ctx: &Ctx) -> String {
    // every address: the machine's name; one address (loopback too): that address
    let host = if ctx.listen.ip.is_unspecified() { plat::net::hostname() } else { format!("{}", ctx.listen.ip) };
    format!("{}://{}", if ctx.tls { "https" } else { "http" }, plat::net::host_port(&host, ctx.listen.port))
}

/// SHA-256 of the server CA
pub fn ca_fingerprint(ctx: &Ctx) -> String {
    ctx.pki.ca_fingerprint()
}

// ------------------------------------------------------------------ enrolment and heartbeats

pub fn enroll(ctx: &Ctx, c: &Call) -> Response {
    if !ctx.tls {
        return err(409, code::CONFLICT, "Agenten koppeln sich nur über eine verschlüsselte Verbindung (Server mit --listen auf einer Netzwerkadresse starten)");
    }
    let Some(r) = EnrollRequest::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    let key = crypto::hex(&crypto::blake3::hash(r.code.trim().to_uppercase().as_bytes()));
    let Some(pc) = ctx.db.get(C_PAIRING, &key) else {
        c.audit_as(&format!("agent:{}", r.name), "host.enroll", &r.name, "Unbekannter Pairing-Code");
        return field(code::FORBIDDEN, "Der Pairing-Code ist unbekannt oder schon verwendet", "code");
    };
    let _ = ctx.db.delete(C_PAIRING, &key); // once
    if pc.get("expires").and_then(|v| v.as_i64()).unwrap_or(0) < now() {
        return field(code::FORBIDDEN, "Der Pairing-Code ist abgelaufen", "code");
    }
    let pk = crypto::unhex(&r.public_key).filter(|b| b.len() == 32);
    let Some(pk) = pk else { return field(code::BAD_REQUEST, "Ungültiger Schlüssel", "public_key") };
    let mut pkb = [0u8; 32];
    pkb.copy_from_slice(&pk);
    if !crypto::ed25519::valid_public_key(&pkb) {
        return field(code::BAD_REQUEST, "Ungültiger Schlüssel", "public_key");
    }
    let name = String::from(r.name.trim());
    if name.is_empty() || name.len() > 120 {
        return field(code::BAD_REQUEST, "Der Name fehlt", "name");
    }
    // where the server reaches the agent: its address, else where it came from
    let peer_ip = c.req.peer.map(|p| p.ip);
    let address = if r.address.trim().is_empty() {
        peer_ip.map_or(String::new(), |ip| plat::net::host_port(&format!("{}", ip), bc_proto::AGENT_PORT))
    } else {
        String::from(r.address.trim())
    };
    let host_part = address.rsplit_once(':').map_or(address.as_str(), |(h, _)| h).trim_matches(['[', ']']);
    let mut dns: Vec<String> = Vec::new();
    let mut ips: Vec<IpAddr> = peer_ip.into_iter().collect();
    match IpAddr::parse(host_part) {
        Some(ip) if !ips.contains(&ip) => ips.push(ip),
        Some(_) => {}
        None if !host_part.is_empty() => dns.push(host_part.to_lowercase()),
        None => {}
    }
    let id = util::new_id();
    let cert = ctx.pki.issue_agent(&id, &pkb, &dns, &ips);
    let der = netio::pem::decode(&cert, "CERTIFICATE").unwrap_or_default();
    let host = Host {
        id: id.clone(),
        name: name.clone(),
        address,
        os: r.os,
        platform: r.platform,
        kind: HostKind::Agent,
        status: HostStatus::Online,
        version: r.version,
        last_seen: now(),
        fingerprint: crypto::hex(&crypto::sha256(&der)),
        roots: r.roots,
        restore_roots: Vec::new(),
        message: String::new(),
        created: now(),
    };
    if ctx.db.put(C_HOSTS, &id, host.to_json()).is_err() {
        return util::internal("");
    }
    c.audit_as(&format!("agent:{}", name), "host.enroll", &name, &format!("Zertifikat {}", &host.fingerprint[..16]));
    ok(201, &EnrollResponse { host_id: id, certificate: cert, ca: ctx.pki.ca_pem() })
}

pub fn heartbeat(ctx: &Ctx, c: &Call) -> Response {
    let Some(id) = &c.agent else { return err(401, code::UNAUTHORIZED, "") };
    let Some(hb) = Heartbeat::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    let r = ctx.db.update(C_HOSTS, id, |v| {
        if let Some(mut h) = Host::from_json(v) {
            h.last_seen = now();
            h.status = HostStatus::Online;
            h.message.clear();
            h.version = hb.version.clone();
            h.os = hb.os;
            h.platform = hb.platform.clone();
            if !hb.address.is_empty() {
                h.address = hb.address.clone();
            }
            h.roots = hb.roots.clone();
            h.restore_roots = hb.restore_roots.clone();
            *v = h.to_json();
        }
    });
    match r {
        Ok(Some(_)) => util::json(200, &j::obj([("interval", Value::u64(30))])),
        _ => err(404, code::NOT_FOUND, "Host unbekannt"),
    }
}

use netio::http::Response;

// ------------------------------------------------------------------ talking to an agent

fn connector(ctx: &Ctx, pin: [u8; 32]) -> netio::client::Connector {
    let identity = ctx.pki.client.clone();
    Arc::new(move |_url: &Url, tcp: TcpStream| {
        let cfg = ClientConfig { trust: Trust::Pinned(alloc::vec![pin]), identity: Some(identity.clone()), server_name: String::new() };
        tls::connect(Box::new(tcp), cfg).map(|s| Box::new(s) as Box<dyn Stream>).map_err(|e| netio::HttpError::Tls(format!("{}", e)))
    })
}

/// "host" / "host:port" / "[v6]:port" / bare IPv6 -> with the agent port when none is given
fn with_port(a: &str) -> String {
    let a = a.trim();
    let has_port = if let Some(rest) = a.strip_prefix('[') {
        rest.contains("]:")
    } else {
        a.matches(':').count() == 1
    };
    if has_port { String::from(a) } else { plat::net::host_port(a.trim_matches(['[', ']']), bc_proto::AGENT_PORT) }
}

/// the session to an agent (created on first use, dropped when the host changes)
fn session(ctx: &Ctx, h: &Host) -> Result<Arc<Mutex<Session>>, String> {
    if h.kind != HostKind::Agent {
        return Err(String::from("kein Agent"));
    }
    let key = format!("{}#{}", h.id, h.fingerprint);
    if let Some(s) = ctx.agents.lock().get(&key) {
        return Ok(s.clone());
    }
    let pin = crypto::unhex(&h.fingerprint).filter(|b| b.len() == 32).ok_or("Der Agent hat kein Zertifikat")?;
    let mut p = [0u8; 32];
    p.copy_from_slice(&pin);
    let url = Url::parse(&format!("https://{}/", with_port(&h.address))).ok_or_else(|| format!("ungültige Agentenadresse {}", h.address))?;
    let mut client = Client::new().with_connector(connector(ctx, p));
    client.connect_ms = 5000;
    client.timeout_ms = 60_000;
    client.user_agent = format!("bc-server/{}", bc_proto::VERSION);
    let s = Arc::new(Mutex::new(Session::new(client, url)));
    let mut map = ctx.agents.lock();
    map.retain(|k, _| !k.starts_with(&format!("{}#", h.id)));
    map.insert(key, s.clone());
    Ok(s)
}

/// one call to an agent -> (status, body bytes)
fn call(ctx: &Ctx, h: &Host, method: &str, target: &str, body: &[u8], max: usize) -> Result<(u16, Vec<u8>), String> {
    let s = session(ctx, h)?;
    let mut s = s.lock();
    let hdr = json_headers("");
    match s.request(method, target, &hdr, body, max) {
        Ok(r) => Ok((r.status, Vec::from(r.body_bytes()))),
        Err(e) => {
            let _ = ctx.db.update(C_HOSTS, &h.id, |v| {
                v.insert("message", Value::string(format!("{}", e)));
            });
            Err(format!("{} nicht erreichbar: {}", h.name, e))
        }
    }
}

fn call_json<T: Json>(ctx: &Ctx, h: &Host, method: &str, target: &str, body: Option<&Value>) -> Result<T, String> {
    let text = body.map(j::text).unwrap_or_default();
    let (st, b) = call(ctx, h, method, target, text.as_bytes(), 32 << 20)?;
    let v = j::parse(&b).unwrap_or(Value::Null);
    if !(200..300).contains(&st) {
        let m = j::str_of(&v, "message");
        return Err(if m.is_empty() { format!("{}: HTTP {}", h.name, st) } else { format!("{}: {}", h.name, m) });
    }
    T::from_json(&v).ok_or_else(|| format!("{}: unerwartete Antwort", h.name))
}

/// a call whose answer carries nothing but success
fn call_ok(ctx: &Ctx, h: &Host, target: &str, body: &Value) -> Result<(), String> {
    let (st, b) = call(ctx, h, "POST", target, j::text(body).as_bytes(), 1 << 20)?;
    if !(200..300).contains(&st) {
        let m = String::from(j::str_of(&j::parse(&b).unwrap_or(Value::Null), "message"));
        return Err(if m.is_empty() { format!("{}: HTTP {}", h.name, st) } else { m });
    }
    Ok(())
}

fn q(s: &str) -> String {
    netio::url::percent_encode(s)
}

pub fn info(ctx: &Ctx, h: &Host) -> Result<AgentInfo, String> {
    call_json(ctx, h, "GET", "/v1/info", None)
}

pub fn list(ctx: &Ctx, h: &Host, path: &str, offset: usize) -> Result<AgentListing, String> {
    call_json(ctx, h, "GET", &format!("/v1/list?path={}&offset={}", q(path), offset), None)
}

pub fn stat(ctx: &Ctx, h: &Host, path: &str) -> Result<AgentEntry, String> {
    call_json(ctx, h, "GET", &format!("/v1/stat?path={}", q(path)), None)
}

/// up to `len` bytes of a file at `offset`
pub fn read(ctx: &Ctx, h: &Host, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
    let (st, b) = call(ctx, h, "GET", &format!("/v1/read?path={}&offset={}&len={}", q(path), offset, len.min(READ_MAX)), &[], READ_MAX as usize + 1024)?;
    if st != 200 {
        let v = j::parse(&b).unwrap_or(Value::Null);
        return Err(format!("{}: {}", path, j::str_of(&v, "message")));
    }
    Ok(b)
}

pub fn set_roots(ctx: &Ctx, h: &Host, roots: &[String], restore_roots: &[String]) -> Result<AgentRoots, String> {
    let v = AgentRoots { roots: Vec::from(roots), restore_roots: Vec::from(restore_roots) }.to_json();
    call_json(ctx, h, "PUT", "/v1/roots", Some(&v))
}

pub fn restore_open(ctx: &Ctx, h: &Host, path: &str, conflict: &str) -> Result<RestoreOpened, String> {
    let v = RestoreOpen { path: String::from(path), conflict: String::from(conflict) }.to_json();
    call_json(ctx, h, "POST", "/v1/restore/open", Some(&v))
}

pub fn restore_write(ctx: &Ctx, h: &Host, tmp: &str, offset: u64, data: &[u8]) -> Result<(), String> {
    let (st, b) = call(ctx, h, "POST", &format!("/v1/restore/write?tmp={}&offset={}", q(tmp), offset), data, 4096)?;
    if st != 200 {
        return Err(String::from(j::str_of(&j::parse(&b).unwrap_or(Value::Null), "message")));
    }
    Ok(())
}

pub fn restore_commit(ctx: &Ctx, h: &Host, c: &RestoreCommit) -> Result<(), String> {
    call_ok(ctx, h, "/v1/restore/commit", &c.to_json())
}

/// the changes in an agent's backup folders after `since` (a long poll of up to wait_ms, on its
/// own connection: the shared one stays free for backups)
pub fn changes(ctx: &Ctx, h: &Host, since: u64, wait_ms: u32) -> Result<AgentChanges, String> {
    let pin = crypto::unhex(&h.fingerprint).filter(|b| b.len() == 32).ok_or("Der Agent hat kein Zertifikat")?;
    let mut p = [0u8; 32];
    p.copy_from_slice(&pin);
    let url = Url::parse(&format!("https://{}/v1/changes?since={}&wait={}", with_port(&h.address), since, wait_ms)).ok_or_else(|| format!("ungültige Agentenadresse {}", h.address))?;
    let mut client = Client::new().with_connector(connector(ctx, p));
    client.connect_ms = 5000;
    client.timeout_ms = wait_ms + 15_000;
    client.max_body = 32 << 20;
    let r = client.get(&url, &json_headers("")).map_err(|e| format!("{} nicht erreichbar: {}", h.name, e))?;
    if r.status != 200 {
        return Err(format!("{} antwortet {}", h.name, r.status));
    }
    j::parse(r.body_bytes()).and_then(|v| AgentChanges::from_json(&v)).ok_or_else(|| String::from("unlesbare Antwort des Agenten"))
}

/// remove an entry inside the agent's restore folders (replica mirrors)
pub fn restore_remove(ctx: &Ctx, h: &Host, path: &str) -> Result<(), String> {
    call_ok(ctx, h, "/v1/restore/remove", &bc_proto::agent::RemovePath { path: String::from(path) }.to_json())
}

pub fn restore_mkdir(ctx: &Ctx, h: &Host, d: &RestoreDir) -> Result<(), String> {
    call_ok(ctx, h, "/v1/restore/mkdir", &d.to_json())
}

pub fn restore_symlink(ctx: &Ctx, h: &Host, l: &RestoreLink) -> Result<(), String> {
    call_ok(ctx, h, "/v1/restore/symlink", &l.to_json())
}

pub fn restore_abort(ctx: &Ctx, h: &Host, tmp: &str) {
    let _ = call(ctx, h, "POST", &format!("/v1/restore/abort?tmp={}", q(tmp)), &[], 4096);
}

// ------------------------------------------------------------------ API helpers

pub fn browse(ctx: &Ctx, h: &Host, path: &str, offset: usize) -> Result<Listing, String> {
    let l = list(ctx, h, path, offset)?;
    let entries = l
        .entries
        .into_iter()
        .map(|e| Entry {
            kind: match e.kind.as_str() {
                "dir" => EntryKind::Dir,
                "link" => EntryKind::Link,
                "other" => EntryKind::Other,
                _ => EntryKind::File,
            },
            name: e.name,
            size: e.size,
            mtime: e.mtime_ns.div_euclid(1_000_000_000),
        })
        .collect();
    Ok(Listing { path: l.path, entries, more: l.more, offset: offset as u32 })
}

pub fn check(ctx: &Arc<Ctx>, h: &Host) -> Result<String, String> {
    let i = info(ctx, h)?;
    if i.host_id != h.id {
        return Err(String::from("Der Agent meldet eine andere Kennung"));
    }
    Ok(format!("{} {} erreichbar, Zertifikat gültig, {} Freigaben", i.name, i.version, i.roots.len()))
}

pub fn ssh_fingerprint(ctx: &Ctx, c: &Call) -> Response {
    crate::ssh_push::fingerprint(ctx, c)
}

pub fn ssh_push(ctx: &Arc<Ctx>, c: &Call) -> Response {
    crate::ssh_push::push(ctx, c)
}
