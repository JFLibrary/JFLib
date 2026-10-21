// update -- new versions from an update channel
//
//   status(ctx)            GET /api/update: this version, the channel, the newest version seen,
//                          the package for this platform and whether it is downloaded
//   check(ctx, c)          POST /api/update/check: fetch manifest.json and manifest.json.sig
//                          from update.url, check the Ed25519 signature (the official release
//                          keys or update.key), compare the versions
//   download(ctx, c)       POST /api/update/download (operation): the package of this platform
//                          into <data>/updates/<version>/, size and SHA-256 checked
//   tick(ctx)              the scheduler: with update.auto a check once a day
//
// The channel may be https (the system's CAs plus update.ca) or http: the signature and the
// digest protect the content either way.  Installing stays a step for an administrator (the
// setup or the package manager restarts the service).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::j::{self, Json};
use bc_proto::model::{UpdateManifest, UpdateStatus};
use netio::http::Response;

use crate::api::Call;
use crate::ctx::{Ctx, C_META};
use crate::ops::Ops;
use crate::util::{self, err, now, ok};

const MAX_MANIFEST: usize = 1 << 20;
const MAX_PACKAGE: u64 = 2 << 30;

/// "windows-x86_64" / "linux-x86_64" (the package names of the manifest)
pub fn platform() -> String {
    let os = if cfg!(windows) { "windows" } else { "linux" };
    let arch = if cfg!(target_arch = "x86_64") { "x86_64" } else if cfg!(target_arch = "aarch64") { "aarch64" } else { "other" };
    format!("{}-{}", os, arch)
}

/// "0.10.2" > "0.9.7" (numbers compared part by part; text after '-' sorts before the release)
pub fn newer(a: &str, b: &str) -> bool {
    let key = |v: &str| -> (Vec<u64>, bool) {
        let (core, pre) = match v.split_once('-') {
            Some((c, _)) => (c, true),
            None => (v, false),
        };
        (core.split('.').map(|p| p.parse().unwrap_or(0)).collect(), !pre)
    };
    let (ka, kb) = (key(a.trim().trim_start_matches('v')), key(b.trim().trim_start_matches('v')));
    let n = ka.0.len().max(kb.0.len());
    for i in 0..n {
        let (x, y) = (ka.0.get(i).copied().unwrap_or(0), kb.0.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    ka.1 && !kb.1
}

fn stored(ctx: &Ctx) -> UpdateStatus {
    let mut s = ctx.db.get(C_META, "update").and_then(|v| UpdateStatus::from_json(&v)).unwrap_or_default();
    s.current = String::from(bc_proto::VERSION);
    s.channel = ctx.setting("update.url");
    s.platform = platform();
    s.available = !s.latest.is_empty() && newer(&s.latest, &s.current);
    if !s.downloaded.is_empty() && plat::fs::stat(&s.downloaded).is_none() {
        s.downloaded.clear();
    }
    s
}

fn store(ctx: &Ctx, s: &UpdateStatus) {
    let _ = ctx.db.put(C_META, "update", s.to_json());
}

pub fn status(ctx: &Ctx) -> Response {
    ok(200, &stored(ctx))
}

/// the trusted publisher keys: the official ones and update.key
fn keys(ctx: &Ctx) -> Vec<[u8; 32]> {
    let extra = ctx.setting("update.key");
    bc_proto::RELEASE_KEYS
        .iter()
        .copied()
        .chain(extra.split([',', ' ', '\n']).map(str::trim).filter(|k| !k.is_empty()))
        .filter_map(|h| crypto::unhex(h).and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()))
        .collect()
}

fn fetch(ctx: &Ctx, url: &str, max: usize) -> Result<Vec<u8>, String> {
    let u = netio::Url::parse(url).ok_or_else(|| format!("ungültige Adresse „{}“", url))?;
    let extra = netio::pem::decode_all(&ctx.setting("update.ca"), "CERTIFICATE").iter().filter_map(|d| netio::x509::Cert::parse(d)).collect();
    let mut c = netio::Client::new().with_connector(netio::trust::system_connector(extra));
    c.max_body = max;
    c.timeout_ms = 120_000;
    let r = c.get(&u, &netio::http::Headers::new()).map_err(|e| format!("{}: {}", url, e))?;
    if r.status != 200 {
        return Err(format!("{} antwortet {}", url, r.status));
    }
    Ok(Vec::from(r.body_bytes()))
}

/// the address of a file next to the manifest
fn sibling(manifest_url: &str, name: &str) -> String {
    match manifest_url.rsplit_once('/') {
        Some((base, _)) => format!("{}/{}", base, name),
        None => String::from(name),
    }
}

/// fetch and check the manifest -> the new status
fn check_now(ctx: &Ctx) -> Result<UpdateStatus, String> {
    let url = ctx.setting("update.url");
    if url.trim().is_empty() {
        return Err(String::from("Kein Update-Kanal eingetragen"));
    }
    let keys = keys(ctx);
    if keys.is_empty() {
        return Err(String::from("Kein Schlüssel des Herausgebers eingetragen (update.key)"));
    }
    let body = fetch(ctx, url.trim(), MAX_MANIFEST)?;
    let sig_text = fetch(ctx, &format!("{}.sig", url.trim()), 4096)?;
    let sig = crypto::b64::decode(String::from_utf8_lossy(&sig_text).trim()).and_then(|b| <[u8; 64]>::try_from(b.as_slice()).ok()).ok_or("Die Signatur des Manifests ist unlesbar")?;
    if !keys.iter().any(|k| crypto::ed25519::verify(k, &body, &sig)) {
        return Err(String::from("Die Signatur des Manifests passt zu keinem vertrauenswürdigen Schlüssel"));
    }
    let v = j::parse(&body).ok_or("Das Manifest ist kein gültiges JSON")?;
    let m = UpdateManifest::from_json(&v).ok_or("Das Manifest ist unvollständig")?;
    if m.product != bc_proto::PRODUCT || m.version.is_empty() {
        return Err(String::from("Das Manifest gehört nicht zu BackupCraft"));
    }
    let mut s = stored(ctx);
    let file = m.files.iter().find(|f| f.platform == platform());
    if s.latest != m.version {
        s.downloaded.clear();
    }
    s.latest = m.version.clone();
    s.notes = m.notes.clone();
    s.date = m.date;
    s.file = file.map_or(String::new(), |f| f.name.clone());
    s.size = file.map_or(0, |f| f.size);
    s.sha256 = file.map_or(String::new(), |f| f.sha256.clone());
    s.checked = now();
    s.error.clear();
    s.available = newer(&s.latest, &s.current);
    store(ctx, &s);
    Ok(s)
}

pub fn check(ctx: &Ctx, c: &Call) -> Response {
    let r = check_now(ctx);
    c.audit("update.check", "", match &r {
        Ok(s) => s.latest.as_str(),
        Err(e) => e.as_str(),
    });
    match r {
        Ok(s) => ok(200, &s),
        Err(e) => {
            let mut s = stored(ctx);
            s.error = e.clone();
            s.checked = now();
            store(ctx, &s);
            err(502, bc_proto::api::code::UPDATE, &e)
        }
    }
}

pub fn download(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let s = stored(ctx);
    if !s.available || s.file.is_empty() {
        return err(409, bc_proto::api::code::CONFLICT, "Kein Update für diese Plattform bekannt (zuerst nach Updates suchen)");
    }
    if s.size > MAX_PACKAGE {
        return err(409, bc_proto::api::code::CONFLICT, "Das Paket ist zu groß");
    }
    let url = sibling(ctx.setting("update.url").trim(), &s.file);
    let title = format!("Update {} herunterladen", s.latest);
    let version = s.latest.clone();
    let op = Ops::start(ctx, "update.download", &title, &c.actor(), "", "", Box::new(move |o| {
        let ctx = o.ctx().clone();
        o.progress(0, s.size.max(1));
        let body = fetch(&ctx, &url, s.size as usize + 1)?;
        if body.len() as u64 != s.size {
            return Err(format!("Das Paket hat {} statt {} Bytes", body.len(), s.size));
        }
        if !crypto::ct_eq(crypto::hex(&crypto::sha256(&body)).as_bytes(), s.sha256.to_ascii_lowercase().as_bytes()) {
            return Err(String::from("Die Prüfsumme des Pakets stimmt nicht (SHA-256)"));
        }
        let name = s.file.replace(['/', '\\'], "_");
        let dir = plat::fs::join(&ctx.path("updates"), &s.latest.replace(['/', '\\', '.'], "_"));
        if !plat::fs::mkdir_private(&ctx.path("updates")) || !plat::fs::mkdir_private(&dir) {
            return Err(format!("{} kann nicht angelegt werden", dir));
        }
        let path = plat::fs::join(&dir, &name);
        if !plat::fs::write_file_private(&path, &body) {
            return Err(format!("{} kann nicht geschrieben werden", path));
        }
        let mut st = stored(&ctx);
        st.downloaded = path.clone();
        store(&ctx, &st);
        o.progress(s.size, s.size.max(1));
        Ok(format!("{} geprüft und gespeichert: {}", s.file, path))
    }));
    c.audit("update.download", &version, "");
    util::accepted(&op)
}

/// once a day with update.auto
pub fn tick(ctx: &Ctx) {
    if ctx.setting("update.auto") != "true" || ctx.setting("update.url").trim().is_empty() {
        return;
    }
    let s = stored(ctx);
    if now() - s.checked < 86_400 {
        return;
    }
    if let Err(e) = check_now(ctx) {
        let mut s = stored(ctx);
        s.error = e;
        s.checked = now();
        store(ctx, &s);
    }
}

/// the dashboard hint: a newer version is available
pub fn alert(ctx: &Ctx) -> Option<bc_proto::model::Alert> {
    let s = stored(ctx);
    s.available.then(|| bc_proto::model::Alert {
        severity: bc_proto::model::Severity::Info,
        title: format!("BackupCraft {} ist verfügbar", s.latest),
        detail: if s.downloaded.is_empty() { String::from("Einstellungen › Über BackupCraft") } else { format!("Heruntergeladen: {}", s.downloaded) },
        target_kind: String::from("update"),
        target_id: String::new(),
        at: s.checked,
    })
}
