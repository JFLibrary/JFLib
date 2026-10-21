// fleet -- one server watches others (a central server for several sites), and the tokens
// that let a central server watch this one
//
//   probe(ctx, c)            POST /api/fleet/probe: the address as used and the CA the server
//                            presents (the console shows its fingerprint to confirm)
//   list / save / delete     the watched servers: name, address, token (sealed with the master
//                            key), the CA pinned by its SHA-256
//   check_now(ctx, c)        POST /api/fleet/servers/{id}/check
//   start(ctx)               a thread of main: every watched server once a minute
//   alerts(ctx)              the dashboard's alerts about watched servers
//   report(ctx)              GET /api/fleet/report (Access::Fleet): this server's summary
//   tokens / token_create / token_delete   the fleet tokens (the database keeps the digest)
//   token_ok(ctx, token)     whether a bearer token is a fleet token (notes its use)
//
// A watched server must answer when it is added: that proves the token and pins the CA.  Plain
// http is accepted for loopback addresses only (the token would travel in clear otherwise).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::api::{code, FleetServerRequest, FleetTokenCreated, FleetTokenRequest, ProbeRequest, ProbeResult};
use bc_proto::j::{self, Json, Value};
use bc_proto::model::*;
use netio::http::Response;
use netio::trust::{self, Presented, ServerTrust};
use netio::{Client, HttpError, Url};
use plat::sync::Mutex;

use crate::api::Call;
use crate::ctx::{Ctx, C_FLEET, C_FLEET_TOKENS};
use crate::util::{self, all, field, list, new_id, no_content, not_found, now, ok};

/// seconds between two reports of a server
const PERIOD_S: i64 = 60;

/// one read-modify-write of a watched server at a time (checks and edits)
static LOCK: Mutex<()> = Mutex::new(());

/// what asking a server gave
enum Answer {
    Report(Box<Summary>),
    /// not the pinned CA (or none pinned yet): what it presented
    Untrusted(Option<Presented>),
    /// the token was refused
    Refused,
    /// an older server, or one without fleet
    NotOffered,
    Failed(String),
}

/// the address people typed -> the URL (http only on loopback)
fn address(raw: &str) -> Result<Url, String> {
    let s = bc_proto::server_address(raw).ok_or_else(|| String::from("Bitte eine gültige Adresse angeben (z. B. https://backup-hh01:7642)"))?;
    let u = Url::parse(&format!("{}/", s)).ok_or_else(|| String::from("Bitte eine gültige Adresse angeben"))?;
    if u.scheme == "http" && !bc_proto::is_loopback(&u.host) {
        return Err(String::from("Unverschlüsseltes http nur auf diesem Rechner (localhost); sonst https"));
    }
    Ok(u)
}

/// "https://host:7642"
fn shown(u: &Url) -> String {
    format!("{}://{}", u.scheme, plat::net::host_port(&u.host, u.port))
}

fn message_of(body: &[u8]) -> String {
    j::parse(body).map(|v| String::from(j::str_of(&v, "message"))).unwrap_or_default()
}

/// GET /api/fleet/report of a server
fn ask(url: &Url, token: &str, pin: Option<[u8; 32]>, timeout_ms: u32) -> Answer {
    let seen = Arc::new(Mutex::new(None));
    let mut client = Client::new();
    if url.scheme == "https" {
        let t = match pin {
            Some(p) => ServerTrust::Pin(p),
            None => ServerTrust::Ask,
        };
        client = client.with_connector(trust::connector(t, None, seen.clone()));
    }
    client.user_agent = format!("BackupCraft-Server/{} (fleet)", bc_proto::VERSION);
    client.connect_ms = 5000;
    client.timeout_ms = timeout_ms;
    client.max_body = 8 << 20;
    match client.get(&url.with_target("/api/fleet/report"), &netio::client::json_headers(token)) {
        Err(HttpError::Tls(m)) if m == trust::UNTRUSTED => Answer::Untrusted(seen.lock().take()),
        Err(e) => Answer::Failed(format!("{}", e)),
        Ok(r) => match r.status {
            200 => j::parse(r.body_bytes()).and_then(|v| Summary::from_json(&v)).map_or_else(|| Answer::Failed(String::from("unlesbarer Bericht")), |s| Answer::Report(Box::new(s))),
            401 | 403 => Answer::Refused,
            404 | 501 => Answer::NotOffered,
            s => Answer::Failed(format!("Antwort {}: {}", s, message_of(r.body_bytes()))),
        },
    }
}

/// the state a report shows
fn status_of(s: &Summary) -> (&'static str, String) {
    let first = |sev: Severity| s.alerts.iter().find(|a| a.severity == sev).map(|a| a.title.clone());
    if let Some(t) = first(Severity::Critical) {
        return ("critical", t);
    }
    if s.fail_24h > 0 {
        return ("critical", format!("{} fehlgeschlagene Sicherungen in 24 h", s.fail_24h));
    }
    if let Some(t) = first(Severity::Warning) {
        return ("warning", t);
    }
    if s.warn_24h > 0 {
        return ("warning", format!("{} Sicherungen mit Warnungen in 24 h", s.warn_24h));
    }
    ("ok", format!("{} Jobs, {} Restore-Points", s.jobs, s.points))
}

fn apply(rec: &mut FleetServer, a: Answer, t: i64) {
    rec.checked = t;
    match a {
        Answer::Report(s) => {
            let (st, msg) = status_of(&s);
            rec.status = String::from(st);
            rec.message = msg;
            rec.seen = t;
            rec.report = *s;
        }
        Answer::Untrusted(p) => {
            rec.status = String::from("untrusted");
            rec.message = match p {
                Some(p) => format!("Andere Zertifizierungsstelle als bestätigt: {} ({}) – unter „Bearbeiten“ neu bestätigen", p.name, trust::fingerprint_text(&p.fingerprint)),
                None => String::from("Die Zertifizierungsstelle passt nicht"),
            };
        }
        Answer::Refused => {
            rec.status = String::from("unauthorized");
            rec.message = String::from("Der Server lehnt das Überwachungs-Token ab (widerrufen?)");
        }
        Answer::NotOffered => {
            rec.status = String::from("unreachable");
            rec.message = String::from("Der Server bietet keine Überwachung an (ältere Version?)");
        }
        Answer::Failed(e) => {
            rec.status = String::from("unreachable");
            rec.message = e;
        }
    }
}

/// a watched server and its token
fn load(ctx: &Ctx, id: &str) -> Option<(FleetServer, String)> {
    let v = ctx.db.get(C_FLEET, id)?;
    let s = FleetServer::from_json(&v)?;
    let tok = ctx.keys.unseal(j::str_of(&v, "token"), &format!("fleet:{}", id)).and_then(|b| String::from_utf8(b).ok()).unwrap_or_default();
    Some((s, tok))
}

fn store(ctx: &Ctx, s: &FleetServer, token: &str) -> bool {
    let sealed = ctx.keys.seal(token.as_bytes(), &format!("fleet:{}", s.id));
    ctx.db.put(C_FLEET, &s.id, util::with(s.to_json(), "token", Value::string(sealed))).is_ok()
}

fn pin_of(s: &FleetServer) -> Option<[u8; 32]> {
    trust::parse_fingerprint(&s.fingerprint)
}

// ------------------------------------------------------------------ the central side

pub fn list_servers(ctx: &Ctx) -> Response {
    let mut v: Vec<FleetServer> = all(&ctx.db, C_FLEET);
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list(&v)
}

/// the CA a server presents
pub fn probe(_ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = ProbeRequest::from_json(&c.body) else { return util::bad("Ungültige Anfrage") };
    let url = match address(&r.url) {
        Ok(u) => u,
        Err(e) => return field(code::BAD_REQUEST, &e, "url"),
    };
    let base = shown(&url);
    match ask(&url, "", None, 10_000) {
        Answer::Untrusted(Some(p)) => ok(200, &ProbeResult {
            url: base,
            tls: true,
            fingerprint: crypto::hex(&p.fingerprint),
            fingerprint_text: trust::fingerprint_text(&p.fingerprint),
            ca_name: p.name,
        }),
        // plain http on loopback: it answered
        Answer::Refused | Answer::Report(_) if url.scheme == "http" => ok(200, &ProbeResult { url: base, tls: false, ..ProbeResult::default() }),
        Answer::NotOffered => field(code::BAD_REQUEST, &format!("{} bietet keine Überwachung an (ältere Version?)", base), "url"),
        Answer::Failed(e) => field(code::BAD_REQUEST, &format!("{} ist nicht erreichbar: {}", base, e), "url"),
        _ => field(code::BAD_REQUEST, &format!("{} antwortet unerwartet", base), "url"),
    }
}

pub fn save(ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = FleetServerRequest::from_json(&c.body) else { return util::bad("Ungültige Anfrage") };
    let id = c.param("id");
    let old = if id.is_empty() { None } else { load(ctx, &id) };
    if !id.is_empty() && old.is_none() {
        return not_found("Überwachter Server nicht gefunden");
    }
    let name = r.name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return field(code::BAD_REQUEST, "Bitte einen Namen angeben", "name");
    }
    let url = match address(&r.url) {
        Ok(u) => u,
        Err(e) => return field(code::BAD_REQUEST, &e, "url"),
    };
    let base = shown(&url);
    if all::<FleetServer>(&ctx.db, C_FLEET).iter().any(|s| s.id != id && s.url.eq_ignore_ascii_case(&base)) {
        return field(code::CONFLICT, "Dieser Server wird schon überwacht", "url");
    }
    let same_place = old.as_ref().is_some_and(|(o, _)| o.url.eq_ignore_ascii_case(&base));
    let token = match r.token.trim() {
        "" => old.as_ref().map_or(String::new(), |(_, t)| t.clone()),
        t => String::from(t),
    };
    if token.is_empty() {
        return field(code::BAD_REQUEST, "Bitte das Überwachungs-Token des Servers angeben", "token");
    }
    let pin = if url.scheme == "https" {
        match trust::parse_fingerprint(&r.fingerprint) {
            Some(p) => Some(p),
            None if same_place => old.as_ref().and_then(|(o, _)| pin_of(o)),
            None => return field(code::BAD_REQUEST, "Bitte zuerst die Zertifizierungsstelle des Servers bestätigen", "fingerprint"),
        }
    } else {
        None
    };
    let answer = ask(&url, &token, pin, 15_000);
    match &answer {
        Answer::Report(_) => {}
        Answer::Untrusted(_) => return field(code::CONFLICT, "Der Server zeigt eine andere Zertifizierungsstelle als die bestätigte", "fingerprint"),
        Answer::Refused => return field(code::BAD_REQUEST, "Der Server lehnt das Token ab", "token"),
        Answer::NotOffered => return field(code::BAD_REQUEST, &format!("{} bietet keine Überwachung an (ältere Version?)", base), "url"),
        Answer::Failed(e) => return field(code::BAD_REQUEST, &format!("{} ist nicht erreichbar: {}", base, e), "url"),
    }
    let t = now();
    let mut rec = match &old {
        Some((o, _)) => o.clone(),
        None => FleetServer { id: new_id(), created: t, ..FleetServer::default() },
    };
    rec.name = String::from(name);
    rec.url = base.clone();
    rec.fingerprint = pin.map_or(String::new(), |p| crypto::hex(&p));
    apply(&mut rec, answer, t);
    let _g = LOCK.lock();
    if !store(ctx, &rec, &token) {
        return util::internal("Konnte nicht gespeichert werden");
    }
    c.audit(if id.is_empty() { "fleet.add" } else { "fleet.update" }, &rec.name, &base);
    ok(if id.is_empty() { 201 } else { 200 }, &rec)
}

pub fn delete(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some((s, _)) = load(ctx, &id) else { return not_found("Überwachter Server nicht gefunden") };
    let _g = LOCK.lock();
    let _ = ctx.db.delete(C_FLEET, &id);
    c.audit("fleet.remove", &s.name, &s.url);
    no_content()
}

/// ask one server now and keep the answer
fn check(ctx: &Ctx, id: &str) -> Option<FleetServer> {
    let (s, token) = load(ctx, id)?;
    let url = address(&s.url).ok()?;
    let a = ask(&url, &token, pin_of(&s), 15_000);
    let _g = LOCK.lock();
    // edited or removed meanwhile: that wins
    let (mut cur, tok) = load(ctx, id)?;
    if cur.url != s.url || cur.fingerprint != s.fingerprint || tok != token {
        return Some(cur);
    }
    apply(&mut cur, a, now());
    store(ctx, &cur, &tok);
    Some(cur)
}

pub fn check_now(ctx: &Ctx, c: &Call) -> Response {
    match check(ctx, &c.param("id")) {
        Some(s) => {
            c.audit("fleet.check", &s.name, &s.message);
            ok(200, &s)
        }
        None => not_found("Überwachter Server nicht gefunden"),
    }
}

/// the thread that asks every server once a minute
pub fn start(ctx: &Arc<Ctx>) {
    let c = ctx.clone();
    let _ = plat::thread_spawn(256 * 1024, Box::new(move || loop {
        if c.is_set_up() {
            let t = now();
            for s in all::<FleetServer>(&c.db, C_FLEET) {
                if plat::service::stop_requested() {
                    return;
                }
                if t - s.checked >= PERIOD_S {
                    check(&c, &s.id);
                }
            }
        }
        for _ in 0..10 {
            if plat::service::stop_requested() {
                return;
            }
            plat::sleep_ms(500);
        }
    }));
}

/// alerts about watched servers (the dashboard of the central server)
pub fn alerts(ctx: &Ctx) -> Vec<Alert> {
    let mut out = Vec::new();
    for s in all::<FleetServer>(&ctx.db, C_FLEET) {
        let (sev, title) = match s.status.as_str() {
            "unreachable" => (Severity::Critical, format!("Server {} nicht erreichbar", s.name)),
            "unauthorized" => (Severity::Critical, format!("Server {} lehnt die Überwachung ab", s.name)),
            "untrusted" => (Severity::Critical, format!("Server {}: andere Zertifizierungsstelle", s.name)),
            "critical" => (Severity::Critical, format!("{}: {}", s.name, s.message)),
            "warning" => (Severity::Warning, format!("{}: {}", s.name, s.message)),
            _ => continue,
        };
        let detail = if matches!(s.status.as_str(), "critical" | "warning") { s.url.clone() } else { s.message.clone() };
        out.push(Alert { severity: sev, title, detail, target_kind: String::from("fleet"), target_id: s.id.clone(), at: s.checked });
    }
    out
}

// ------------------------------------------------------------------ the watched side

fn digest(token: &str) -> String {
    crypto::hex(&crypto::blake3::hash(token.as_bytes()))
}

/// this server's report for a central server
pub fn report(ctx: &Ctx) -> Response {
    ok(200, &crate::monitor::build(ctx))
}

pub fn tokens(ctx: &Ctx) -> Response {
    let mut v: Vec<FleetToken> = all(&ctx.db, C_FLEET_TOKENS);
    v.sort_by_key(|t| t.created);
    list(&v)
}

pub fn token_create(ctx: &Ctx, c: &Call) -> Response {
    let label = FleetTokenRequest::from_json(&c.body).map(|r| String::from(r.label.trim())).unwrap_or_default();
    if label.is_empty() || label.chars().count() > 80 {
        return field(code::BAD_REQUEST, "Bitte eine Bezeichnung angeben (z. B. der Name der Zentrale)", "label");
    }
    let token = format!("bcf_{}", crypto::b64::encode_url(&plat::random_bytes::<32>()));
    let rec = FleetToken { id: new_id(), label: label.clone(), created: now(), created_by: c.actor(), last_used: 0 };
    if ctx.db.put(C_FLEET_TOKENS, &digest(&token), rec.to_json()).is_err() {
        return util::internal("Konnte nicht gespeichert werden");
    }
    c.audit("fleet.token.create", &label, "");
    ok(201, &FleetTokenCreated { id: rec.id, label, token })
}

pub fn token_delete(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let found = ctx.db.filter(C_FLEET_TOKENS, |_, v| j::str_of(v, "id") == id);
    let Some((key, v)) = found.into_iter().next() else { return not_found("Überwachungs-Token nicht gefunden") };
    let _ = ctx.db.delete(C_FLEET_TOKENS, &key);
    c.audit("fleet.token.revoke", j::str_of(&v, "label"), "");
    no_content()
}

/// whether a bearer token is a fleet token (its last use is noted once a minute)
pub fn token_ok(ctx: &Ctx, token: &str) -> bool {
    if !token.starts_with("bcf_") || token.len() > 128 {
        return false;
    }
    let k = digest(token);
    let Some(v) = ctx.db.get(C_FLEET_TOKENS, &k) else { return false };
    let t = now();
    if let Some(mut rec) = FleetToken::from_json(&v) {
        if t - rec.last_used >= 60 {
            rec.last_used = t;
            let _ = ctx.db.put(C_FLEET_TOKENS, &k, rec.to_json());
        }
    }
    true
}
