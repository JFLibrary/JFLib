// notify -- e-mail and web hook notifications
//
//   session_finished(ctx, &session)   after every session: by notify.mode ("failure": failed
//                                     or with warnings; "all": every run), in the background
//   daily(ctx)                        the scheduler's tick: the daily summary at notify.daily
//                                     ("HH:MM", server time), once a day
//   test(ctx, c) / summary_now / status    POST /api/settings/test-email, POST
//                                     /api/notify/summary, GET /api/notify
//
// Channels: SMTP (netio::smtp: STARTTLS / TLS, the system's CAs plus smtp.ca) and a web hook
// (notify.webhook: POST JSON {"text", "title", "status", ...} -- Slack and Teams take "text").
// The last delivery is kept in meta/notify for the console.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::j::{self, Value};
use bc_proto::model::*;
use netio::http::Response;
use netio::smtp::{self, Mail, Security, SmtpConfig};

use crate::api::Call;
use crate::ctx::{Ctx, C_JOBS, C_META, C_SESSIONS};
use crate::util::{self, all, err, now, ok};

const AAD: &str = "setting:smtp.password";

/// the sealed SMTP password
pub fn seal_password(ctx: &Ctx, pass: &str) -> String {
    ctx.keys.seal(pass.as_bytes(), AAD)
}

fn password(ctx: &Ctx) -> String {
    let sealed = ctx.setting("smtp.password");
    if sealed.is_empty() {
        return String::new();
    }
    ctx.keys.unseal(&sealed, AAD).and_then(|b| String::from_utf8(b).ok()).unwrap_or_default()
}

/// what one delivery needs (owned: it runs on its own thread)
#[derive(Clone)]
struct Plan {
    smtp: Option<(SmtpConfig, Mail)>,
    webhook: Option<(String, Value)>,
}

fn smtp_config(ctx: &Ctx) -> Result<Option<SmtpConfig>, String> {
    let host = ctx.setting("smtp.host");
    if host.trim().is_empty() {
        return Ok(None);
    }
    let security = Security::parse(&ctx.setting("smtp.security"));
    let (h, port) = match host.trim().rsplit_once(':') {
        Some((h, p)) if !h.contains(':') || h.starts_with('[') => (String::from(h.trim_matches(['[', ']'])), p.parse().map_err(|_| format!("ungültiger Port in „{}“", host))?),
        _ => (String::from(host.trim()), security.default_port()),
    };
    let ca_text = ctx.setting("smtp.ca");
    let extra_cas = netio::pem::decode_all(&ca_text, "CERTIFICATE").iter().filter_map(|d| netio::x509::Cert::parse(d)).collect();
    Ok(Some(SmtpConfig {
        host: h,
        port,
        security,
        username: ctx.setting("smtp.user"),
        password: password(ctx),
        helo: plat::net::hostname(),
        extra_cas,
        timeout_ms: 20_000,
    }))
}

fn sender(ctx: &Ctx) -> String {
    let f = ctx.setting("smtp.from");
    if f.trim().is_empty() { format!("backupcraft@{}", plat::net::hostname()) } else { String::from(f.trim()) }
}

fn recipients(ctx: &Ctx) -> Vec<String> {
    ctx.setting("smtp.to").split([',', ';', '\n', ' ']).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

fn plan(ctx: &Ctx, subject: &str, text: &str, html: &str, payload: Value) -> Result<Plan, String> {
    let mut p = Plan { smtp: None, webhook: None };
    if let Some(cfg) = smtp_config(ctx)? {
        let to = recipients(ctx);
        if to.is_empty() {
            return Err(String::from("Kein Empfänger eingetragen"));
        }
        p.smtp = Some((cfg, Mail { from: sender(ctx), to, subject: String::from(subject), text: String::from(text), html: String::from(html) }));
    }
    let hook = ctx.setting("notify.webhook");
    if !hook.trim().is_empty() {
        p.webhook = Some((String::from(hook.trim()), payload));
    }
    if p.smtp.is_none() && p.webhook.is_none() {
        return Err(String::from("Weder ein SMTP-Server noch ein Web-Hook ist eingetragen"));
    }
    Ok(p)
}

/// POST the JSON payload to a web hook
fn post_hook(url: &str, payload: &Value) -> Result<String, String> {
    let u = netio::Url::parse(url).ok_or_else(|| format!("ungültige Adresse „{}“", url))?;
    let c = netio::Client::new().with_connector(netio::trust::system_connector(Vec::new()));
    let mut h = netio::http::Headers::new();
    h.set("Content-Type", "application/json");
    let r = c.request("POST", &u, &h, j::text(payload).as_bytes()).map_err(|e| format!("Web-Hook: {}", e))?;
    if (200..300).contains(&r.status) { Ok(format!("Web-Hook: {}", r.status)) } else { Err(format!("Web-Hook antwortet {}", r.status)) }
}

/// deliver on every channel -> one line per channel, Err when any failed
fn deliver(p: &Plan) -> Result<String, String> {
    let mut out = Vec::new();
    let mut failed = false;
    if let Some((cfg, mail)) = &p.smtp {
        match smtp::send(cfg, mail) {
            Ok(_) => out.push(format!("E-Mail an {} gesendet", mail.to.join(", "))),
            Err(e) => {
                failed = true;
                out.push(format!("E-Mail: {}", e));
            }
        }
    }
    if let Some((url, payload)) = &p.webhook {
        match post_hook(url, payload) {
            Ok(m) => out.push(m),
            Err(e) => {
                failed = true;
                out.push(e);
            }
        }
    }
    let text = out.join(" · ");
    if failed { Err(text) } else { Ok(text) }
}

/// change the meta/notify record (created when missing)
fn meta(ctx: &Ctx, f: impl FnOnce(&mut Value)) {
    let _ = ctx.db.write(|data, tx| {
        let mut v = data.get(C_META).and_then(|c| c.get("notify")).cloned().unwrap_or_else(|| j::obj([("last_at", Value::i64(0))]));
        f(&mut v);
        tx.put(C_META, "notify", v);
        Ok::<(), datastore::StoreError>(())
    });
}

fn record(ctx: &Ctx, r: &Result<String, String>, what: &str) {
    let t = now();
    meta(ctx, |v| {
        v.insert("last_at", Value::i64(t));
        v.insert("last_ok", Value::Bool(r.is_ok()));
        v.insert("last_message", Value::string(match r {
            Ok(m) | Err(m) => m.as_str(),
        }));
        v.insert("last_what", Value::string(what));
    });
    if let Err(e) = r {
        crate::log(&format!("Benachrichtigung ({}): {}", what, e));
    }
}

/// the status of the last delivery
pub fn status(ctx: &Ctx) -> Response {
    let v = ctx.db.get(C_META, "notify").unwrap_or_else(|| j::obj([("last_at", Value::i64(0))]));
    ok(200, &NotifyStatus {
        last_at: v.get("last_at").and_then(|x| x.as_i64()).unwrap_or(0),
        last_ok: v.get("last_ok").and_then(|x| x.as_bool()).unwrap_or(false),
        last_message: String::from(j::str_of(&v, "last_message")),
        last_what: String::from(j::str_of(&v, "last_what")),
        daily_sent: v.get("daily_sent").and_then(|x| x.as_i64()).unwrap_or(0),
    })
}

// ------------------------------------------------------------------ texts

fn zone() -> datetime::Zone {
    plat::fs::local_zone_id()
        .and_then(|id| match id.strip_prefix("win:") {
            Some(k) => datetime::Zone::from_windows(k),
            None => datetime::Zone::by_name(&id),
        })
        .unwrap_or_else(datetime::Zone::utc)
}

pub fn when(t: i64) -> String {
    if t <= 0 {
        return String::from("–");
    }
    let c = zone().to_local(datetime::Timestamp::from_secs(t));
    format!("{:02}.{:02}.{} {:02}:{:02}:{:02}", c.day, c.month, c.year, c.hour, c.minute, c.second)
}

fn duration(s: i64) -> String {
    if s < 60 { format!("{} s", s) } else if s < 3600 { format!("{} min {} s", s / 60, s % 60) } else { format!("{} h {} min", s / 3600, s / 60 % 60) }
}

pub fn kind_name(k: SessionKind) -> &'static str {
    match k {
        SessionKind::Backup => "Sicherung",
        SessionKind::Restore => "Wiederherstellung",
        SessionKind::Verify => "Prüfung",
        SessionKind::Retention => "Aufbewahrung",
        SessionKind::Copy => "Kopie",
        SessionKind::Maintenance => "Wartung",
        SessionKind::Replica => "Replikat",
        SessionKind::Failback => "Failback",
        SessionKind::Tape => "Band",
    }
}

pub fn status_name(s: SessionStatus) -> &'static str {
    match s {
        SessionStatus::Success => "Erfolgreich",
        SessionStatus::Warning => "Mit Warnungen",
        SessionStatus::Failed => "Fehlgeschlagen",
        SessionStatus::Cancelled => "Abgebrochen",
        SessionStatus::Running => "Läuft",
        SessionStatus::Never => "–",
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// an HTML mail: a coloured title and a table of facts
fn html(title: &str, color: &str, rows: &[(String, String)], tail: &str) -> String {
    let mut h = format!(
        "<html><body style=\"font-family:Segoe UI,Arial,sans-serif;font-size:14px;color:#18202B\"><div style=\"border-left:6px solid {};padding:4px 12px;margin-bottom:12px\"><b style=\"font-size:18px\">{}</b></div><table cellpadding=\"4\">",
        color,
        esc(title)
    );
    for (k, v) in rows {
        h.push_str(&format!("<tr><td style=\"color:#5b6675\">{}</td><td>{}</td></tr>", esc(k), esc(v)));
    }
    h.push_str("</table>");
    if !tail.is_empty() {
        h.push_str(&format!("<pre style=\"background:#F3F5F8;padding:8px;font-size:12px\">{}</pre>", esc(tail)));
    }
    h.push_str("<p style=\"color:#5b6675;font-size:12px\">BackupCraft</p></body></html>");
    h
}

fn text(title: &str, rows: &[(String, String)], tail: &str, server: &str) -> String {
    let w = rows.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);
    let mut s = format!("{}\n\n", title);
    for (k, v) in rows {
        s.push_str(&format!("{}:{} {}\n", k, " ".repeat(w - k.chars().count()), v));
    }
    if !tail.is_empty() {
        s.push_str(&format!("\n{}\n", tail));
    }
    s.push_str(&format!("\n-- \nBackupCraft · Server {}\n", server));
    s
}

/// subject, text, HTML and the web hook payload of a finished session
fn session_message(ctx: &Ctx, s: &Session) -> (String, String, String, Value) {
    let server = ctx.server_name();
    let what = format!("{} „{}“", kind_name(s.kind), s.job_name);
    let st = status_name(s.status);
    let subject = format!("[BackupCraft] {}: {}", st, what);
    let title = format!("{} auf {}: {}", what, server, st);
    let mut rows = alloc::vec![
        (String::from("Gestartet"), when(s.started)),
        (String::from("Beendet"), format!("{} ({})", when(s.finished), duration((s.finished - s.started).max(0)))),
    ];
    if s.files_total > 0 {
        rows.push((String::from("Dateien"), format!("{}", s.files_total)));
    }
    if s.bytes_read > 0 || s.bytes_new > 0 {
        rows.push((String::from("Daten"), format!("{} gelesen, {} neu gespeichert", crate::engine::size(s.bytes_read), crate::engine::size(s.bytes_new))));
    }
    if s.warnings > 0 || s.errors > 0 {
        rows.push((String::from("Warnungen / Fehler"), format!("{} / {}", s.warnings, s.errors)));
    }
    rows.push((String::from("Meldung"), s.message.clone()));
    // the last warnings and errors of the log
    let log = ctx.session_log(&s.id);
    let lines: Vec<String> = log
        .iter()
        .filter(|l| matches!(l.level, LogLevel::Warning | LogLevel::Error))
        .rev()
        .take(15)
        .map(|l| format!("{} {}", when(l.at), l.msg))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let tail = if lines.is_empty() { String::new() } else { format!("Warnungen und Fehler:\n{}", lines.join("\n")) };
    let color = match s.status {
        SessionStatus::Success => "#2F9E44",
        SessionStatus::Warning | SessionStatus::Cancelled => "#E67700",
        _ => "#E03131",
    };
    let payload = j::obj([
        ("text", Value::string(format!("{}\n{}", title, s.message).as_str())),
        ("title", Value::string(title.as_str())),
        ("event", Value::string("session")),
        ("kind", bc_proto::j::Field::put(&s.kind)),
        ("status", bc_proto::j::Field::put(&s.status)),
        ("job", Value::string(s.job_name.as_str())),
        ("server", Value::string(server.as_str())),
        ("started", Value::i64(s.started)),
        ("finished", Value::i64(s.finished)),
        ("message", Value::string(s.message.as_str())),
        ("session_id", Value::string(s.id.as_str())),
    ]);
    let txt = text(&title, &rows, &tail, &server);
    let h = html(&title, color, &rows, &tail);
    (subject, txt, h, payload)
}

// ------------------------------------------------------------------ triggers

/// after a session: notify by the mode, on a thread of its own
pub fn session_finished(ctx: &Ctx, s: &Session) {
    let mode = ctx.setting("notify.mode");
    let cdp = s.kind == SessionKind::Backup && crate::util::rec::<Job>(ctx.get(crate::ctx::C_JOBS, &s.job_id)).is_some_and(|j| j.cdp);
    let wanted = match mode.as_str() {
        "all" if cdp => matches!(s.status, SessionStatus::Failed | SessionStatus::Warning),
        "all" => s.status != SessionStatus::Running,
        "failure" => matches!(s.status, SessionStatus::Failed | SessionStatus::Warning),
        _ => false,
    };
    if !wanted {
        return;
    }
    let (subject, text, html, payload) = session_message(ctx, s);
    let p = match plan(ctx, &subject, &text, &html, payload) {
        Ok(p) => p,
        Err(e) => {
            record(ctx, &Err(e), "Sitzung");
            return;
        }
    };
    // the result is recorded by the server's context, which outlives the thread
    let Some(arc) = crate::ctx::current() else { return };
    let _ = plat::thread_spawn(256 * 1024, Box::new(move || {
        let r = deliver(&p);
        record(&arc, &r, "Sitzung");
    }));
}

/// the summary of the last 24 hours
fn summary_message(ctx: &Ctx) -> (String, String, String, Value) {
    let t = now();
    let server = ctx.server_name();
    let sessions: Vec<Session> = all::<Session>(&ctx.db, C_SESSIONS).into_iter().filter(|s| s.started >= t - 86_400).collect();
    let count = |st: SessionStatus| sessions.iter().filter(|s| s.status == st).count();
    let (okn, warn, fail) = (count(SessionStatus::Success), count(SessionStatus::Warning), count(SessionStatus::Failed));
    let title = format!("Tageszusammenfassung von {}", server);
    let subject = format!("[BackupCraft] Zusammenfassung: {} erfolgreich, {} Warnungen, {} Fehler", okn, warn, fail);
    let mut rows = alloc::vec![
        (String::from("Zeitraum"), format!("{} – {}", when(t - 86_400), when(t))),
        (String::from("Sitzungen"), format!("{} erfolgreich, {} mit Warnungen, {} fehlgeschlagen", okn, warn, fail)),
    ];
    let jobs: Vec<Job> = all(&ctx.db, C_JOBS);
    rows.push((String::from("Jobs"), format!("{} ({} aktiv)", jobs.len(), jobs.iter().filter(|j| j.enabled).count())));
    for r in crate::repos::all_live(ctx) {
        let fill = if r.capacity > 0 { format!(" · Platte {} % belegt, {} frei", r.disk_used().saturating_mul(100) / r.capacity, crate::engine::size(r.free)) } else { String::new() };
        rows.push((format!("Repository {}", r.name), format!("{} gespeichert{}", crate::engine::size(r.used), fill)));
    }
    let bad: Vec<String> = sessions
        .iter()
        .filter(|s| matches!(s.status, SessionStatus::Failed | SessionStatus::Warning))
        .map(|s| format!("{}  {} „{}“: {} – {}", when(s.started), kind_name(s.kind), s.job_name, status_name(s.status), s.message))
        .collect();
    let tail = if bad.is_empty() { String::from("Keine Fehler und Warnungen.") } else { format!("Fehler und Warnungen:\n{}", bad.join("\n")) };
    let color = if fail > 0 { "#E03131" } else if warn > 0 { "#E67700" } else { "#2F9E44" };
    let payload = j::obj([
        ("text", Value::string(format!("{}: {} erfolgreich, {} Warnungen, {} Fehler", title, okn, warn, fail).as_str())),
        ("title", Value::string(title.as_str())),
        ("event", Value::string("summary")),
        ("server", Value::string(server.as_str())),
        ("ok", Value::u64(okn as u64)),
        ("warnings", Value::u64(warn as u64)),
        ("failed", Value::u64(fail as u64)),
    ]);
    (subject, text(&title, &rows, &tail, &server), html(&title, color, &rows, &tail), payload)
}

/// the scheduler's tick: send the daily summary once its time has come
pub fn daily(ctx: &Ctx) {
    let at = ctx.setting("notify.daily");
    let Some((h, m)) = parse_hhmm(&at) else { return };
    let c = zone().to_local(datetime::Timestamp::from_secs(now()));
    if (c.hour as u32, c.minute as u32) < (h, m) {
        return;
    }
    let today = c.year as i64 * 10_000 + c.month as i64 * 100 + c.day as i64;
    let sent = ctx.db.get(C_META, "notify").and_then(|v| v.get("daily_sent").and_then(|x| x.as_i64())).unwrap_or(0);
    if sent >= today {
        return;
    }
    meta(ctx, |v| {
        v.insert("daily_sent", Value::i64(today));
    });
    let (subject, text, html, payload) = summary_message(ctx);
    match plan(ctx, &subject, &text, &html, payload) {
        Ok(p) => {
            let r = deliver(&p);
            record(ctx, &r, "Zusammenfassung");
        }
        Err(e) => record(ctx, &Err(e), "Zusammenfassung"),
    }
}

/// "07:30" -> (7, 30)
pub fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some((h, m))
}

/// POST /api/settings/test-email: a test message on every configured channel, now
pub fn test(ctx: &Ctx, c: &Call) -> Response {
    let server = ctx.server_name();
    let title = format!("Testnachricht von {}", server);
    let rows = alloc::vec![(String::from("Gesendet"), when(now())), (String::from("Von"), c.actor())];
    let payload = j::obj([("text", Value::string(title.as_str())), ("title", Value::string(title.as_str())), ("event", Value::string("test")), ("server", Value::string(server.as_str()))]);
    let p = match plan(ctx, &format!("[BackupCraft] {}", title), &text(&title, &rows, "Die Benachrichtigungen von BackupCraft kommen an.", &server), &html(&title, "#0B7285", &rows, ""), payload) {
        Ok(p) => p,
        Err(e) => return util::field(bc_proto::api::code::BAD_REQUEST, &e, "smtp.host"),
    };
    let r = deliver(&p);
    record(ctx, &r, "Test");
    c.audit("notify.test", "", match &r {
        Ok(m) | Err(m) => m,
    });
    match r {
        Ok(m) => crate::util::json(200, &j::obj([("message", Value::string(m.as_str()))])),
        Err(e) => err(502, bc_proto::api::code::NOTIFY, &e),
    }
}

/// POST /api/notify/summary: the daily summary now
pub fn summary_now(ctx: &Ctx, c: &Call) -> Response {
    let (subject, text, html, payload) = summary_message(ctx);
    let p = match plan(ctx, &subject, &text, &html, payload) {
        Ok(p) => p,
        Err(e) => return util::field(bc_proto::api::code::BAD_REQUEST, &e, "smtp.host"),
    };
    let r = deliver(&p);
    record(ctx, &r, "Zusammenfassung");
    c.audit("notify.summary", "", "");
    match r {
        Ok(m) => crate::util::json(200, &j::obj([("message", Value::string(m.as_str()))])),
        Err(e) => err(502, bc_proto::api::code::NOTIFY, &e),
    }
}
