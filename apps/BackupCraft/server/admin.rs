// admin -- server information, capabilities, credentials (secrets sealed with the master
// key, never returned), settings, the recovery kit, configuration backups
//
// Recovery kit: an X25519 key pair; the server keeps the public key (keys/recovery.pub) and
// seals every repository key to it, the private key is shown exactly once.
// Configuration backup (.bcconfig): the database (without tokens and failed logins) and the
// keys as one JSON document, encrypted with a passphrase:
//   "BCC1" ‖ salt (16) ‖ nonce (24) ‖ XChaCha20-Poly1305(Argon2id(passphrase, salt), JSON)
// Restoring decrypts at once and stages the result (owner-only file, sealed with a key derived
// from the current master key); the next start applies it.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::{code, ConfigBackupRequest, ConfigRestoreRequest, CredentialRequest, RecoveryKit, STAGED};
use bc_proto::j::{self, Json, Value};
use bc_proto::model::{Capability, ConfigBackup, Credential, RecoveryStatus, ServerInfo, Setting};
use crypto::{aead, argon2};
use netio::http::Response;

use crate::api::Call;
use crate::ctx::{Ctx, C_CREDS, DEFAULT_SETTINGS};
use crate::ops::Ops;
use crate::util::{self, all, bad, field, list, new_id, no_content, not_found, now, ok};
use alloc::sync::Arc;

pub fn info(ctx: &Ctx) -> Response {
    ok(200, &ServerInfo { name: ctx.server_name(), version: String::from(bc_proto::VERSION), needs_setup: !ctx.is_set_up(), time: now(), tls: ctx.tls, ca_fingerprint: ctx.pki.ca_fingerprint() })
}

pub fn capabilities() -> Response {
    let caps: Vec<Capability> = STAGED
        .iter()
        .map(|c| {
            let on = bc_proto::api::available(c);
            Capability { name: String::from(*c), available: on, reason: String::from(if on { "" } else { "Für eine spätere Version geplant" }) }
        })
        .collect();
    list(&caps)
}

// ------------------------------------------------------------------ credentials

pub fn credentials(ctx: &Ctx) -> Response {
    let mut v: Vec<Credential> = all(&ctx.db, C_CREDS);
    v.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    list(&v)
}

pub fn credential_save(ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = CredentialRequest::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    if r.label.trim().is_empty() {
        return field(code::BAD_REQUEST, "Bitte eine Bezeichnung angeben", "label");
    }
    if r.secret.is_empty() {
        return field(code::BAD_REQUEST, "Das Passwort oder der Schlüssel fehlt", "secret");
    }
    let cr = Credential { id: new_id(), label: String::from(r.label.trim()), username: r.username, kind: r.kind, created: now() };
    let sealed = ctx.keys.seal(r.secret.as_bytes(), &format!("credential:{}", cr.id));
    if ctx.db.put(C_CREDS, &cr.id, util::with(cr.to_json(), "secret", Value::string(sealed))).is_err() {
        return util::internal("");
    }
    c.audit("credential.create", &cr.label, cr.kind.as_str());
    ok(201, &cr)
}

pub fn credential_delete(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some(cr): Option<Credential> = util::rec(ctx.db.get(C_CREDS, &id)) else { return not_found("Zugangsdaten nicht gefunden") };
    let _ = ctx.db.delete(C_CREDS, &id);
    c.audit("credential.delete", &cr.label, "");
    no_content()
}

/// the secret of a credential (for the SSH push)
pub fn credential_secret(ctx: &Ctx, id: &str) -> Option<(Credential, String)> {
    let v = ctx.db.get(C_CREDS, id)?;
    let cr = Credential::from_json(&v)?;
    let s = ctx.keys.unseal(j::str_of(&v, "secret"), &format!("credential:{}", id))?;
    Some((cr, String::from_utf8(s).ok()?))
}

// ------------------------------------------------------------------ settings

pub fn settings(ctx: &Ctx) -> Response {
    list(&ctx.settings())
}

pub fn settings_save(ctx: &Ctx, c: &Call) -> Response {
    let items: Vec<Setting> = j::from_list(&c.body);
    for s in &items {
        if !DEFAULT_SETTINGS.iter().any(|(k, _)| *k == s.key) {
            return field(code::BAD_REQUEST, &format!("Unbekannte Einstellung {}", s.key), "key");
        }
        let bad_num = |k: &str, lo: u32, hi: u32| s.key == k && s.value.trim().parse::<u32>().map_or(true, |v| v < lo || v > hi);
        if bad_num("session.timeout_hours", 1, 720) || bad_num("login.max_attempts", 3, 50) || bad_num("traffic.max_mbps", 0, 1_000_000) {
            return field(code::BAD_REQUEST, &format!("Ungültiger Wert für {}", s.key), &s.key);
        }
        if s.key == "notify.mode" && !matches!(s.value.as_str(), "off" | "failure" | "all") {
            return field(code::BAD_REQUEST, "Ungültige Benachrichtigung", "notify.mode");
        }
        if s.key == "smtp.security" && !matches!(s.value.as_str(), "starttls" | "tls" | "none") {
            return field(code::BAD_REQUEST, "Ungültige Verschlüsselung", "smtp.security");
        }
        if s.key == "notify.daily" && !s.value.trim().is_empty() && crate::notify::parse_hhmm(&s.value).is_none() {
            return field(code::BAD_REQUEST, "Uhrzeit als HH:MM angeben", "notify.daily");
        }
        if (s.key == "smtp.from" || s.key == "smtp.to") && s.value.split([',', ';', '\n', ' ']).map(str::trim).filter(|a| !a.is_empty()).any(|a| !a.contains('@') || a.contains(['<', '>'])) {
            return field(code::BAD_REQUEST, "Ungültige E-Mail-Adresse", &s.key);
        }
        if s.key == "notify.webhook" && !s.value.trim().is_empty() && netio::Url::parse(s.value.trim()).is_none() {
            return field(code::BAD_REQUEST, "Ungültige Adresse des Web-Hooks", "notify.webhook");
        }
        if s.key == "update.url" && !s.value.trim().is_empty() && netio::Url::parse(s.value.trim()).is_none() {
            return field(code::BAD_REQUEST, "Ungültige Adresse des Update-Kanals", "update.url");
        }
        if s.key == "update.key"
            && s.value.split([',', ' ', '\n']).map(str::trim).filter(|k| !k.is_empty()).any(|k| crypto::unhex(k).is_none_or(|b| b.len() != 32))
        {
            return field(code::BAD_REQUEST, "Der Schlüssel des Herausgebers ist 64 Hex-Zeichen lang", "update.key");
        }
        if s.key == "tape.library" && !s.value.trim().is_empty() && !bc_fs::absolute(s.value.trim(), cfg!(windows)) {
            return field(code::BAD_REQUEST, "Bitte einen absoluten Ordner angeben", "tape.library");
        }
        if s.key == "update.auto" && !matches!(s.value.as_str(), "true" | "false") {
            return field(code::BAD_REQUEST, "true oder false", "update.auto");
        }
        if (s.key == "smtp.ca" || s.key == "update.ca") && !s.value.trim().is_empty() {
            let certs = netio::pem::decode_all(&s.value, "CERTIFICATE");
            if certs.is_empty() || certs.iter().any(|d| netio::x509::Cert::parse(d).is_none()) {
                return field(code::BAD_REQUEST, "Kein lesbares Zertifikat (PEM)", &s.key);
            }
        }
        if s.key == "restore.allowed_roots" {
            for root in s.value.lines().map(str::trim).filter(|l| !l.is_empty()) {
                if !is_absolute(root) {
                    return field(code::BAD_REQUEST, &format!("„{}“ ist kein absoluter Pfad", root), "restore.allowed_roots");
                }
            }
        }
    }
    // secrets: sealed; the placeholder keeps the stored one
    let items: Vec<Setting> = items
        .into_iter()
        .filter(|s| !(crate::ctx::SECRET_SETTINGS.contains(&s.key.as_str()) && s.value == crate::ctx::SECRET_SET))
        .map(|s| {
            if s.key == "smtp.password" && !s.value.is_empty() {
                Setting { value: crate::notify::seal_password(ctx, &s.value), ..s }
            } else {
                s
            }
        })
        .collect();
    let r = ctx.db.write(|_, tx| {
        for s in &items {
            tx.put(crate::ctx::C_SETTINGS, &s.key, s.to_json());
        }
        Ok::<(), datastore::StoreError>(())
    });
    if r.is_err() {
        return util::internal("");
    }
    let keys: Vec<&str> = items.iter().map(|s| s.key.as_str()).collect();
    c.audit("settings.update", "", &keys.join(", "));
    list(&ctx.settings())
}

/// an absolute path of this platform ("/srv", "C:\\Daten", "\\\\server\\share")
pub fn is_absolute(p: &str) -> bool {
    if cfg!(windows) {
        let b = p.as_bytes();
        (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')) || p.starts_with("\\\\")
    } else {
        p.starts_with('/')
    }
}

// ------------------------------------------------------------------ recovery kit

fn fingerprint(pk: &[u8; 32]) -> String {
    let d = crypto::blake3::hash(pk);
    let h = crypto::hex(&d[..8]);
    format!("{} {} {} {}", &h[0..4], &h[4..8], &h[8..12], &h[12..16])
}

pub fn recovery(ctx: &Ctx) -> Response {
    let created = ctx.db.get(crate::ctx::C_META, "recovery").and_then(|v| v.get("created").and_then(|x| x.as_i64())).unwrap_or(0);
    let fp = ctx.keys.recovery_public().map(|pk| fingerprint(&pk)).unwrap_or_default();
    ok(200, &RecoveryStatus { created: if fp.is_empty() { 0 } else { created }, fingerprint: fp })
}

pub fn recovery_kit(ctx: &Ctx, c: &Call) -> Response {
    let secret: [u8; 32] = plat::random_bytes();
    let public = crypto::x25519::public_key(&secret);
    if !ctx.keys.set_recovery_public(&public) {
        return util::internal("Der öffentliche Schlüssel konnte nicht gespeichert werden");
    }
    let t = now();
    let _ = ctx.db.put(crate::ctx::C_META, "recovery", j::obj([("created", Value::i64(t))]));
    let fp = fingerprint(&public);
    // every repository gets a copy of its key for the new kit
    let mut sealed = 0;
    for r in crate::util::all::<bc_proto::model::Repository>(&ctx.db, crate::ctx::C_REPOS) {
        if crate::engine::seal_for_recovery(ctx, &r, &public).is_ok() {
            sealed += 1;
        }
    }
    c.audit("recovery.kit", "", &format!("Neues Recovery-Kit {} ({} Repositories)", fp, sealed));
    ok(201, &RecoveryKit { private_key: format!("BCRK1-{}", crypto::b64::encode_url(&secret)), fingerprint: fp, created: t })
}

// ------------------------------------------------------------------ configuration backups

const CONFIG_MAGIC: &[u8; 4] = b"BCC1";

fn backups_dir(ctx: &Ctx) -> String {
    ctx.path("config_backups")
}

pub fn config_backups(ctx: &Ctx) -> Response {
    let dir = backups_dir(ctx);
    let mut v: Vec<ConfigBackup> = plat::fs::read_dir(&dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|e| e.name.ends_with(".bcconfig"))
        .filter_map(|e| {
            let m = plat::fs::stat(&plat::fs::join(&dir, &e.name))?;
            Some(ConfigBackup { auto: e.name.starts_with("auto-"), created: m.mtime_ns / 1_000_000_000, size: m.size, name: e.name })
        })
        .collect();
    v.sort_by(|a, b| b.created.cmp(&a.created));
    list(&v)
}

/// the configuration as one JSON document (database without sessions' secrets, keys)
fn bundle(ctx: &Ctx) -> Value {
    let mut db = Vec::new();
    for coll in [crate::ctx::C_META, crate::ctx::C_USERS, crate::ctx::C_SETTINGS, crate::ctx::C_JOBS, crate::ctx::C_HOSTS, crate::ctx::C_REPOS, C_CREDS, crate::ctx::C_POINTS] {
        let recs: Vec<(String, Value)> = ctx.db.list(coll);
        db.push((String::from(coll), Value::Object(recs)));
    }
    let key = |name: &str| plat::read_file(&plat::fs::join(ctx.keys.dir(), name)).map(|b| Value::string(crypto::b64::encode_url(&b))).unwrap_or(Value::Null);
    j::obj([
        ("format", Value::string("bcconfig-1")),
        ("created", Value::i64(now())),
        ("server", Value::string(ctx.server_name())),
        ("db", Value::Object(db)),
        ("keys", j::obj([("master", key("master.key")), ("signing", key("signing.key")), ("recovery", key("recovery.pub"))])),
    ])
}

fn config_key(pass: &str, salt: &[u8]) -> Option<[u8; 32]> {
    let mut k = [0u8; 32];
    argon2::argon2id(pass.as_bytes(), salt, &argon2::Params::KEY, &mut k).then_some(k)
}

/// write an encrypted configuration backup -> its file name
pub fn write_config_backup(ctx: &Ctx, pass: &str, auto: bool) -> Result<String, String> {
    let text = j::text(&bundle(ctx));
    if text.is_empty() {
        return Err(String::from("Die Konfiguration ist zu groß"));
    }
    let salt: [u8; 16] = plat::random_bytes();
    let nonce: [u8; 24] = plat::random_bytes();
    let key = config_key(pass, &salt).ok_or("Schlüsselableitung fehlgeschlagen")?;
    let mut out = Vec::from(&CONFIG_MAGIC[..]);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&aead::x_seal(&key, &nonce, CONFIG_MAGIC, text.as_bytes()));
    let dir = backups_dir(ctx);
    if !plat::fs::mkdir_private(&dir) {
        return Err(format!("{} kann nicht angelegt werden", dir));
    }
    let c = datetime::Zone::utc().to_local(datetime::Timestamp::from_secs(now()));
    let name = format!("{}-{:04}{:02}{:02}-{:02}{:02}{:02}.bcconfig", if auto { "auto" } else { "manual" }, c.year, c.month, c.day, c.hour, c.minute, c.second);
    if !plat::fs::write_file_private(&plat::fs::join(&dir, &name), &out) {
        return Err(String::from("Die Sicherung konnte nicht geschrieben werden"));
    }
    Ok(name)
}

pub fn config_backup(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let Some(r) = ConfigBackupRequest::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    if r.passphrase.chars().count() < 12 {
        return field(code::BAD_REQUEST, "Die Passphrase braucht mindestens 12 Zeichen", "passphrase");
    }
    let pass = r.passphrase;
    let op = Ops::start(ctx, "config.backup", "Konfiguration sichern", &c.actor(), "", "", alloc::boxed::Box::new(move |o| {
        write_config_backup(o.ctx(), &pass, false).map(|n| format!("Gesichert als {}", n))
    }));
    c.audit("config.backup", "", "");
    util::accepted(&op)
}

/// decrypt a configuration backup
pub fn open_config_backup(bytes: &[u8], pass: &str) -> Option<Value> {
    if bytes.len() < 4 + 16 + 24 + 16 || &bytes[..4] != CONFIG_MAGIC {
        return None;
    }
    let key = config_key(pass, &bytes[4..20])?;
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&bytes[20..44]);
    let plain = aead::x_open(&key, &nonce, CONFIG_MAGIC, &bytes[44..])?;
    let v = agent_core::json::parse_with_limits(&plain, datastore::record_limits()).ok()?;
    (j::str_of(&v, "format") == "bcconfig-1").then_some(v)
}

pub fn config_restore(ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = ConfigRestoreRequest::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    if r.name.contains(['/', '\\']) || !r.name.ends_with(".bcconfig") {
        return field(code::BAD_REQUEST, "Ungültiger Name", "name");
    }
    let Some(bytes) = plat::read_file(&plat::fs::join(&backups_dir(ctx), &r.name)) else { return not_found("Sicherung nicht gefunden") };
    let Some(v) = open_config_backup(&bytes, &r.passphrase) else {
        return field(code::FORBIDDEN, "Die Passphrase stimmt nicht oder die Datei ist beschädigt", "passphrase");
    };
    // staged for the next start, sealed with a key the current server can derive
    let k = ctx.keys.derive("BackupCraft 2026 staged configuration");
    let nonce: [u8; 24] = plat::random_bytes();
    let mut out = Vec::from(&nonce[..]);
    out.extend_from_slice(&aead::x_seal(&k, &nonce, b"staged", j::text(&v).as_bytes()));
    if !plat::fs::write_file_private(&ctx.path("config.restore"), &out) {
        return util::internal("Die Wiederherstellung konnte nicht vorgemerkt werden");
    }
    c.audit("config.restore", &r.name, "Vorgemerkt für den nächsten Start");
    util::json(200, &j::obj([("message", Value::string("Wird beim nächsten Start angewendet"))]))
}

/// apply a staged configuration (at start, before the database opens) -> what happened
pub fn apply_staged(data: &str, keys: &crate::keys::Keys) -> Option<Result<String, String>> {
    let path = plat::fs::join(data, "config.restore");
    let bytes = plat::read_file(&path)?;
    let k = keys.derive("BackupCraft 2026 staged configuration");
    let r = (|| {
        if bytes.len() < 24 + 16 {
            return Err(String::from("vorgemerkte Datei ist beschädigt"));
        }
        let mut nonce = [0u8; 24];
        nonce.copy_from_slice(&bytes[..24]);
        let plain = aead::x_open(&k, &nonce, b"staged", &bytes[24..]).ok_or("vorgemerkte Datei ist nicht lesbar")?;
        let v = agent_core::json::parse_with_limits(&plain, datastore::record_limits()).map_err(|_| String::from("vorgemerkte Datei ist kein JSON"))?;
        // keys first: the database refers to them
        let kd = keys.dir();
        for (name, field) in [("master.key", "master"), ("signing.key", "signing"), ("recovery.pub", "recovery")] {
            if let Some(b) = v.get("keys").and_then(|x| x.get(field)).and_then(|x| x.as_str()).and_then(crypto::b64::decode_url) {
                if !plat::fs::write_file_private(&plat::fs::join(kd, name), &b) {
                    return Err(format!("{} kann nicht geschrieben werden", name));
                }
            }
        }
        // the database: a fresh store replaces the old one (kept as db.old)
        let db = plat::fs::join(data, "db");
        let old = plat::fs::join(data, "db.old");
        crate::remove_tree(&old);
        if plat::file_exists(&db) == 2 && !plat::rename(&db, &old) {
            return Err(String::from("die alte Datenbank kann nicht beiseitegelegt werden"));
        }
        let store = datastore::Store::open(&db).map_err(|e| format!("{}", e))?;
        let res = store.write(|_, tx| {
            for (coll, recs) in v.get("db").and_then(|x| x.as_object()).unwrap_or(&[]) {
                for (id, rec) in recs.as_object().unwrap_or(&[]) {
                    tx.put(coll, id, rec.clone());
                }
            }
            Ok::<(), datastore::StoreError>(())
        });
        res.map_err(|e| format!("{}", e))?;
        Ok(format!("Konfiguration vom {} wiederhergestellt", j::str_of(&v, "server")))
    })();
    plat::delete(&path);
    Some(r)
}
