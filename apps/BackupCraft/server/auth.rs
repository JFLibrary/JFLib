// auth -- the first administrator (setup token), passwords (Argon2id, at least 12 characters),
// tokens (32 random bytes; the database keeps only their BLAKE3 digest), failed logins per
// user name (a lock-out after login.max_attempts failures within a minute, kept across
// restarts), users and roles

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::{code, LoginRequest, LoginResponse, PasswordChange, SetupRequest, UserRequest};
use bc_proto::j::{self, Json, Value};
use bc_proto::model::{Host, HostKind, HostStatus, Os, Role, User};
use crypto::argon2;
use netio::http::Response;

use crate::api::Call;
use crate::ctx::{Ctx, C_HOSTS, C_LOGIN, C_META, C_TOKENS, C_USERS};
use crate::util::{self, all, bad, conflict, err, field, list, new_id, no_content, not_found, now, ok, rec};

pub const MIN_PASSWORD: usize = 12;
const DUMMY_PHC: &str = "$argon2id$v=19$m=19456,t=2,p=1$YmFja3VwY3JhZnQtZHVtbXk$2b8hgg2Wv7r9pWm1T4cR2xq0u0W6m3QpGQ6r1uD3w9s";

fn hash_password(pw: &str) -> Option<String> {
    let salt: [u8; 16] = plat::random_bytes();
    argon2::hash(pw.as_bytes(), &salt, &argon2::Params::INTERACTIVE)
}

/// the digest a token is stored under
fn token_key(token: &str) -> String {
    crypto::hex(&crypto::blake3::hash(token.as_bytes()))
}

/// the user of a bearer token (expired tokens are removed)
pub fn user_of_token(ctx: &Ctx, token: &str) -> Option<User> {
    if token.is_empty() || token.len() > 128 {
        return None;
    }
    let k = token_key(token);
    let t = ctx.db.get(C_TOKENS, &k)?;
    if t.get("expires").and_then(|v| v.as_i64()).unwrap_or(0) < now() {
        let _ = ctx.db.delete(C_TOKENS, &k);
        return None;
    }
    let u: User = rec(ctx.db.get(C_USERS, j::str_of(&t, "user_id")))?;
    (!u.disabled).then_some(u)
}

/// the setup token (created at the first start of a server that is not set up)
pub fn ensure_setup_token(ctx: &Ctx) -> Option<String> {
    if ctx.is_set_up() {
        return None;
    }
    let p = ctx.path("setup.token");
    if let Some(b) = plat::read_file(&p) {
        let s = String::from(String::from_utf8_lossy(&b).trim());
        if s.len() >= 16 {
            return Some(s);
        }
    }
    let t = crypto::b64::encode_url(&plat::random_bytes::<18>());
    plat::fs::write_file_private(&p, t.as_bytes()).then_some(t)
}

/// the server itself as a host: roots for backups (every fixed drive / the root directory)
pub fn server_host(ctx: &Ctx) -> Host {
    let os = if cfg!(windows) { Os::Windows } else { Os::Linux };
    let roots: Vec<String> = if cfg!(windows) {
        (b'C'..=b'Z').map(|d| format!("{}:\\", d as char)).filter(|p| plat::fs::stat(p).is_some_and(|m| m.is_dir())).collect()
    } else {
        alloc::vec![String::from("/")]
    };
    Host {
        id: new_id(),
        name: plat::net::hostname(),
        address: format!("{}", ctx.listen),
        os,
        platform: String::from(if cfg!(windows) { "Windows" } else { "Linux" }),
        kind: HostKind::Server,
        status: HostStatus::Online,
        version: String::from(bc_proto::VERSION),
        last_seen: now(),
        roots,
        restore_roots: alloc::vec![ctx.path("restore")],
        created: now(),
        ..Host::default()
    }
}

pub fn setup(ctx: &Ctx, c: &Call) -> Response {
    if ctx.is_set_up() {
        return conflict("Der Server ist bereits eingerichtet");
    }
    let Some(r) = SetupRequest::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    let want = ensure_setup_token(ctx).unwrap_or_default();
    if want.is_empty() || !crypto::ct_eq(want.as_bytes(), r.token.trim().as_bytes()) {
        return field(code::UNAUTHORIZED, "Das Setup-Token stimmt nicht", "token");
    }
    let name = r.username.trim();
    if name.is_empty() || name.len() > 64 {
        return field(code::BAD_REQUEST, "Bitte einen Benutzernamen angeben", "username");
    }
    if r.password.chars().count() < MIN_PASSWORD {
        return field(code::BAD_REQUEST, "Das Passwort braucht mindestens 12 Zeichen", "password");
    }
    let Some(phc) = hash_password(&r.password) else { return util::internal("Passwort konnte nicht gespeichert werden") };
    let t = now();
    let u = User { id: new_id(), username: String::from(name), role: Role::Admin, created: t, last_login: 0, disabled: false };
    let host = server_host(ctx);
    let res = ctx.db.write(|_, tx| {
        tx.put(C_USERS, &u.id, util::with(u.to_json(), "phc", Value::string(phc.as_str())));
        tx.put(C_HOSTS, &host.id, host.to_json());
        tx.put(C_META, "server", j::obj([("setup", Value::Bool(true)), ("created", Value::i64(t)), ("server_host", Value::string(host.id.as_str()))]));
        Ok::<(), datastore::StoreError>(())
    });
    if res.is_err() {
        return util::internal("Die Einrichtung konnte nicht gespeichert werden");
    }
    if !r.server_name.trim().is_empty() {
        let _ = ctx.set_setting("server.name", r.server_name.trim());
    }
    plat::delete(&ctx.path("setup.token"));
    c.audit_as(name, "setup", name, "Erster Administrator angelegt");
    ok(201, &u)
}

/// the failure record of a user name: (failures in the window, window start)
fn failures(ctx: &Ctx, name: &str) -> (u32, i64) {
    let v = ctx.db.get(C_LOGIN, name);
    let n = v.as_ref().and_then(|v| v.get("n").and_then(|x| x.as_u64())).unwrap_or(0) as u32;
    let since = v.as_ref().and_then(|v| v.get("since").and_then(|x| x.as_i64())).unwrap_or(0);
    (n, since)
}

pub fn login(ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = LoginRequest::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    let name = r.username.trim();
    let key = name.to_lowercase();
    let t = now();
    let max = ctx.setting_u32("login.max_attempts", 5).clamp(3, 50);
    let (n, since) = failures(ctx, &key);
    if n >= max && t - since < 60 {
        c.audit_as(name, "login", name, "Gesperrt: zu viele Fehlversuche");
        return err(429, code::RATE_LIMITED, "Zu viele Fehlversuche. Bitte eine Minute warten.");
    }
    let found = ctx.db.filter(C_USERS, |_, v| j::str_of(v, "username").eq_ignore_ascii_case(name)).into_iter().next();
    let phc = found.as_ref().map_or(String::from(DUMMY_PHC), |(_, v)| String::from(j::str_of(v, "phc")));
    // the hash runs for unknown users too: the answer time does not tell whether a name exists
    let good = argon2::verify(&phc, r.password.as_bytes()) && found.is_some();
    let user = found.as_ref().and_then(|(_, v)| User::from_json(v));
    match user {
        Some(u) if good && !u.disabled => {
            let _ = ctx.db.delete(C_LOGIN, &key);
            let token = crypto::b64::encode_url(&plat::random_bytes::<32>());
            let hours = ctx.setting_u32("session.timeout_hours", 24).clamp(1, 24 * 30) as i64;
            let expires = t + hours * 3600;
            let rec = j::obj([("user_id", Value::string(u.id.as_str())), ("created", Value::i64(t)), ("expires", Value::i64(expires))]);
            let mut u2 = u.clone();
            u2.last_login = t;
            let raw = found.map(|(_, v)| v).unwrap_or(Value::Null);
            let res = ctx.db.write(|_, tx| {
                tx.put(C_TOKENS, &token_key(&token), rec);
                tx.put(C_USERS, &u.id, util::with(u2.to_json(), "phc", raw.get("phc").cloned().unwrap_or(Value::Null)));
                Ok::<(), datastore::StoreError>(())
            });
            if res.is_err() {
                return util::internal("Anmeldung konnte nicht gespeichert werden");
            }
            c.audit_as(&u.username, "login", &u.username, "");
            ok(200, &LoginResponse { token, expires, user: u2 })
        }
        _ => {
            let (n, since) = if t - since >= 60 { (0, t) } else { (n, since) };
            let _ = ctx.db.put(C_LOGIN, &key, j::obj([("n", Value::u64(n as u64 + 1)), ("since", Value::i64(since))]));
            c.audit_as(name, "login", name, "Falscher Benutzername oder falsches Passwort");
            err(401, code::UNAUTHORIZED, "Benutzername oder Passwort falsch")
        }
    }
}

pub fn logout(ctx: &Ctx, c: &Call) -> Response {
    if let Some(t) = c.token.as_deref() {
        let _ = ctx.db.delete(C_TOKENS, &token_key(t));
    }
    c.audit("logout", "", "");
    no_content()
}

pub fn me(_ctx: &Ctx, c: &Call) -> Response {
    match &c.user {
        Some(u) => ok(200, u),
        None => err(401, code::UNAUTHORIZED, ""),
    }
}

pub fn change_password(ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = PasswordChange::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    let Some(u) = &c.user else { return err(401, code::UNAUTHORIZED, "") };
    let Some(raw) = ctx.db.get(C_USERS, &u.id) else { return not_found("Benutzer nicht gefunden") };
    if !argon2::verify(j::str_of(&raw, "phc"), r.old.as_bytes()) {
        return field(code::FORBIDDEN, "Das bisherige Passwort stimmt nicht", "old");
    }
    if r.new.chars().count() < MIN_PASSWORD {
        return field(code::BAD_REQUEST, "Das Passwort braucht mindestens 12 Zeichen", "new");
    }
    let Some(phc) = hash_password(&r.new) else { return util::internal("") };
    // the other sessions of this user end
    let uid = u.id.clone();
    let tokens: Vec<String> = ctx.db.filter(C_TOKENS, |_, v| j::str_of(v, "user_id") == uid).into_iter().map(|(k, _)| k).collect();
    let mine = c.token.as_deref().map(token_key).unwrap_or_default();
    let _ = ctx.db.write(|_, tx| {
        tx.put(C_USERS, &uid, util::with(raw.clone(), "phc", Value::string(phc.as_str())));
        for k in &tokens {
            if *k != mine {
                tx.delete(C_TOKENS, k);
            }
        }
        Ok::<(), datastore::StoreError>(())
    });
    c.audit("user.password", &u.username, "Eigenes Passwort geändert");
    no_content()
}

pub fn users(ctx: &Ctx, _c: &Call) -> Response {
    let mut v: Vec<User> = all(&ctx.db, C_USERS);
    v.sort_by(|a, b| a.username.to_lowercase().cmp(&b.username.to_lowercase()));
    list(&v)
}

fn admins(ctx: &Ctx) -> usize {
    all::<User>(&ctx.db, C_USERS).iter().filter(|u| u.role == Role::Admin && !u.disabled).count()
}

pub fn user_save(ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = UserRequest::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    let id = c.param("id");
    if id.is_empty() {
        let name = r.username.trim();
        if name.is_empty() || name.len() > 64 || name.contains(['/', '\\', ':']) {
            return field(code::BAD_REQUEST, "Bitte einen gültigen Benutzernamen angeben", "username");
        }
        if r.password.chars().count() < MIN_PASSWORD {
            return field(code::BAD_REQUEST, "Das Passwort braucht mindestens 12 Zeichen", "password");
        }
        if !ctx.db.filter(C_USERS, |_, v| j::str_of(v, "username").eq_ignore_ascii_case(name)).is_empty() {
            return ok(409, &bc_proto::api::ApiError { code: String::from(code::CONFLICT), message: String::from("Den Benutzer gibt es schon"), field: String::from("username") });
        }
        let Some(phc) = hash_password(&r.password) else { return util::internal("") };
        let u = User { id: new_id(), username: String::from(name), role: r.role, created: now(), last_login: 0, disabled: false };
        if ctx.db.put(C_USERS, &u.id, util::with(u.to_json(), "phc", Value::string(phc.as_str()))).is_err() {
            return util::internal("");
        }
        c.audit("user.create", &u.username, r.role.as_str());
        return ok(201, &u);
    }
    let Some(raw) = ctx.db.get(C_USERS, &id) else { return not_found("Benutzer nicht gefunden") };
    let Some(mut u) = User::from_json(&raw) else { return not_found("") };
    let demote = u.role == Role::Admin && (r.role != Role::Admin || r.disabled) && !u.disabled;
    if demote && admins(ctx) <= 1 {
        return conflict("Der letzte Administrator muss Administrator bleiben");
    }
    if c.user.as_ref().is_some_and(|me| me.id == u.id) && r.disabled {
        return conflict("Das eigene Konto kann nicht gesperrt werden");
    }
    let mut detail = Vec::new();
    if u.role != r.role {
        detail.push(format!("Rolle {} -> {}", u.role.as_str(), r.role.as_str()));
    }
    if u.disabled != r.disabled {
        detail.push(String::from(if r.disabled { "gesperrt" } else { "entsperrt" }));
    }
    u.role = r.role;
    u.disabled = r.disabled;
    let mut phc = Value::string(j::str_of(&raw, "phc"));
    if !r.password.is_empty() {
        if r.password.chars().count() < MIN_PASSWORD {
            return field(code::BAD_REQUEST, "Das Passwort braucht mindestens 12 Zeichen", "password");
        }
        let Some(h) = hash_password(&r.password) else { return util::internal("") };
        phc = Value::string(h);
        detail.push(String::from("Passwort neu gesetzt"));
    }
    let uid = u.id.clone();
    let end_sessions = r.disabled || !r.password.is_empty();
    let tokens: Vec<String> = if end_sessions { ctx.db.filter(C_TOKENS, |_, v| j::str_of(v, "user_id") == uid).into_iter().map(|(k, _)| k).collect() } else { Vec::new() };
    let _ = ctx.db.write(|_, tx| {
        tx.put(C_USERS, &uid, util::with(u.to_json(), "phc", phc));
        for k in &tokens {
            tx.delete(C_TOKENS, k);
        }
        Ok::<(), datastore::StoreError>(())
    });
    c.audit("user.update", &u.username, &detail.join(", "));
    ok(200, &u)
}

pub fn user_delete(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some(u): Option<User> = rec(ctx.db.get(C_USERS, &id)) else { return not_found("Benutzer nicht gefunden") };
    if u.role == Role::Admin && !u.disabled && admins(ctx) <= 1 {
        return conflict("Der letzte Administrator kann nicht gelöscht werden");
    }
    if c.user.as_ref().is_some_and(|me| me.id == u.id) {
        return conflict("Das eigene Konto kann nicht gelöscht werden");
    }
    let tokens: Vec<String> = ctx.db.filter(C_TOKENS, |_, v| j::str_of(v, "user_id") == id).into_iter().map(|(k, _)| k).collect();
    let _ = ctx.db.write(|_, tx| {
        tx.delete(C_USERS, &id);
        for k in &tokens {
            tx.delete(C_TOKENS, k);
        }
        Ok::<(), datastore::StoreError>(())
    });
    c.audit("user.delete", &u.username, "");
    no_content()
}

/// drop expired tokens and stale failure records
pub fn prune(ctx: &Ctx) {
    let t = now();
    let old: Vec<String> = ctx.db.filter(C_TOKENS, |_, v| v.get("expires").and_then(|x| x.as_i64()).unwrap_or(0) < t).into_iter().map(|(k, _)| k).collect();
    let stale: Vec<String> = ctx.db.filter(C_LOGIN, |_, v| t - v.get("since").and_then(|x| x.as_i64()).unwrap_or(0) > 3600).into_iter().map(|(k, _)| k).collect();
    if old.is_empty() && stale.is_empty() {
        return;
    }
    let _ = ctx.db.write(|_, tx| {
        for k in &old {
            tx.delete(C_TOKENS, k);
        }
        for k in &stale {
            tx.delete(C_LOGIN, k);
        }
        Ok::<(), datastore::StoreError>(())
    });
}
