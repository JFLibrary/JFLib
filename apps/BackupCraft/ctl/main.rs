// bcctl -- BackupCraft on the command line: everything the console does that scripts need
//
//   bcctl [--server URL] [--ca FILE | --fingerprint FP] [--json] COMMAND ...
//     setup --token T --user NAME [--name SERVERNAME]   first administrator (password: stdin
//                                                       or BC_PASSWORD)
//     login --user NAME                                 password: stdin or BC_PASSWORD; an
//                                                       https server's unknown CA is shown and
//                                                       confirmed (or --fingerprint), then kept
//     logout | status | jobs | hosts | repos | operations | audit [--limit N]
//     run JOB [--full] [--wait]                         JOB: id or name
//     points JOB | sessions [--job JOB] [--limit N] | log SESSION
//     verify POINT [--wait] | wait OP | cancel OP
//     restore POINT --target DIR [--item PATH]... [--conflict skip|rename|overwrite] [--wait]
//     update [check | download [--wait]]                the update channel: status, check, fetch
//     docs                                              the API's routes and who may call them
//     release keygen FILE                               a publisher key (private: FILE; prints
//                                                       the public key for update.key)
//     release manifest --key FILE --dir DIR --version V [--notes TEXT] [--platform NAME=P]...
//                                                       manifest.json + .sig of the packages in
//                                                       DIR (*.exe: windows-x86_64, *_amd64.deb:
//                                                       linux-x86_64) -- no server needed
//     tape list FILE...                                 the sets on tape files (ctl/tape.rs)
//     tape extract FILE... --to DIR [--set ID] [--kit FILE]
//                                                       a set's files into DIR -- no server
//                                                       needed; encrypted sets: the recovery kit
//
// The login and the trusted CA fingerprints ([trust] host:port) are kept in
// <config>/BackupCraft/bcctl.ini (owner-only); BC_SERVER, BC_TOKEN, BC_PASSWORD and BC_CA
// (a CA file) stand in for them.  Exit codes: 0 done, 1 the server refused or the operation
// failed, 2 usage, 3 no connection or an untrusted server.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod tape;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::{Access, Method, ROUTES};
use bc_proto::j::{self, Json, Value};
use bc_proto::model::*;
use netio::trust::{self, fingerprint_text, Presented, ServerTrust};
use netio::{Client, HttpError, Url};
use plat::sync::Mutex;

struct Cli {
    url: Url,
    token: String,
    json: bool,
    args: Vec<String>,
    ini: Option<String>,
    /// how an https server is trusted, and the CA it showed when it was not
    trust: ServerTrust,
    seen: alloc::sync::Arc<Mutex<Option<Presented>>>,
}

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

fn eprint(s: &str) {
    plat::write_stderr(s.as_bytes());
}

/// the value after an option
fn opt(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn opts(args: &[String], name: &str) -> Vec<String> {
    args.windows(2).filter(|w| w[0] == name).map(|w| w[1].clone()).collect()
}

fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|a| a == name)
}

/// the positional arguments after the command
fn positional(args: &[String]) -> Vec<String> {
    let takes = ["--server", "--token", "--user", "--name", "--target", "--item", "--conflict", "--job", "--limit", "--ca", "--fingerprint", "--key",
                 "--dir", "--version", "--notes", "--platform", "--to", "--set", "--kit"];
    let mut v = Vec::new();
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if takes.contains(&a.as_str()) {
            skip = true;
            continue;
        }
        if !a.starts_with("--") {
            v.push(a.clone());
        }
    }
    v
}

fn when(t: i64) -> String {
    if t <= 0 {
        return String::from("-");
    }
    let z = plat::fs::local_zone_id().and_then(|id| match id.strip_prefix("win:") {
        Some(k) => datetime::Zone::from_windows(k),
        None => datetime::Zone::by_name(&id),
    }).unwrap_or_else(datetime::Zone::utc);
    datetime::format_date_time(datetime::Timestamp::from_secs(t), &z, false)
}

fn size(b: u64) -> String {
    bc_proto::size_text(b)
}

/// a table with aligned columns
fn table(head: &[&str], rows: &[Vec<String>]) {
    let mut w: Vec<usize> = head.iter().map(|h| h.chars().count()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            w[i] = w[i].max(c.chars().count());
        }
    }
    let line = |cells: &[String]| {
        let mut s = String::new();
        for (i, c) in cells.iter().enumerate() {
            s.push_str(c);
            if i + 1 < cells.len() {
                for _ in c.chars().count()..w[i] + 2 {
                    s.push(' ');
                }
            }
        }
        s.push('\n');
        out(&s);
    };
    line(&head.iter().map(|h| String::from(*h)).collect::<Vec<_>>());
    for r in rows {
        line(r);
    }
}

fn read_password() -> String {
    if let Some(p) = plat::env("BC_PASSWORD") {
        return p;
    }
    eprint("Passwort: ");
    read_line()
}

/// one line of standard input
fn read_line() -> String {
    let mut buf = [0u8; 1024];
    let mut s = Vec::new();
    while let Ok(n) = plat::read_stdin(&mut buf) {
        if n == 0 {
            break;
        }
        s.extend_from_slice(&buf[..n]);
        if s.contains(&b'\n') {
            break;
        }
    }
    String::from(String::from_utf8_lossy(&s).trim_end_matches(['\r', '\n']))
}

impl Cli {
    /// one API call -> (status, body); status 0: no connection
    fn call(&self, method: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut c = Client::new().with_connector(trust::connector(self.trust.clone(), None, self.seen.clone()));
        c.user_agent = format!("bcctl/{}", bc_proto::VERSION);
        c.timeout_ms = 120_000;
        let text = body.as_ref().map(j::text).unwrap_or_default();
        match c.request(method.as_str(), &self.url.with_target(path), &netio::client::json_headers(&self.token), text.as_bytes()) {
            Ok(r) => (r.status, j::parse(r.body_bytes()).unwrap_or(Value::Null)),
            Err(HttpError::Tls(m)) if m == trust::UNTRUSTED => (0, j::obj([("code", Value::string("untrusted")), ("message", Value::string("unbekannte Zertifizierungsstelle"))])),
            Err(e) => (0, j::obj([("message", Value::string(format!("{}", e)))])),
        }
    }

    /// an https server whose CA is not trusted yet: show it, take --fingerprint or ask, keep it
    fn ensure_trust(&mut self) -> Result<(), i32> {
        if !self.url.is_https() || matches!(self.trust, ServerTrust::Ca(_)) {
            return Ok(());
        }
        let (st, v) = self.call(Method::Get, "/health", None);
        if st != 0 || j::str_of(&v, "code") != "untrusted" {
            // a fingerprint from the command line that the server matched is kept
            if let (ServerTrust::Pin(p), true) = (&self.trust, st != 0 && opt(&self.args, "--fingerprint").is_some()) {
                self.keep_pin(p);
            }
            return Ok(());
        }
        let Some(p) = self.seen.lock().take() else { return Err(3) };
        let fp = fingerprint_text(&p.fingerprint);
        if let Some(want) = opt(&self.args, "--fingerprint").and_then(|f| trust::parse_fingerprint(&f)) {
            if want != p.fingerprint {
                eprint(&format!("bcctl: der Server zeigt eine andere Zertifizierungsstelle ({})\n", fp));
                return Err(3);
            }
        } else {
            let warn = if p.changed { "ACHTUNG: die Zertifizierungsstelle hat sich geändert!\n" } else { "" };
            eprint(&format!("{}Zertifizierungsstelle von {}: {}\nSHA-256 {}\n", warn, self.url.origin(), p.name, fp));
            eprint("Vertrauen? Mit dem Fingerabdruck auf dem Server vergleichen (bc-server ca) [j/N] ");
            let answer = read_line();
            if !matches!(answer.trim(), "j" | "J" | "ja" | "y" | "Y" | "yes") {
                return Err(3);
            }
        }
        self.keep_pin(&p.fingerprint);
        self.trust = ServerTrust::Pin(p.fingerprint);
        Ok(())
    }

    /// remember the CA fingerprint of this server ([trust] host:port)
    fn keep_pin(&self, fp: &[u8; 32]) {
        if let Some(path) = &self.ini {
            let mut ini = base::ini::Ini::load(path);
            ini.set("trust", &self.url.host_header(), &crypto::hex(fp));
            let _ = plat::fs::mkdir_private(plat::fs::parent(path));
            let _ = plat::fs::write_file_private(path, &ini.to_bytes());
        }
    }

    /// a successful answer, or the error printed and the exit code
    fn ok(&self, method: Method, path: &str, body: Option<Value>) -> Result<Value, i32> {
        let (st, v) = self.call(method, path, body);
        if (200..300).contains(&st) {
            return Ok(v);
        }
        let msg = j::str_of(&v, "message");
        if st == 0 && j::str_of(&v, "code") == "untrusted" {
            let p = self.seen.lock().take();
            let fp = p.as_ref().map_or(String::new(), |p| fingerprint_text(&p.fingerprint));
            let which = if p.as_ref().is_some_and(|p| p.changed) { "GEÄNDERTEN" } else { "unbekannten" };
            eprint(&format!("bcctl: {} weist sich mit einer {} Zertifizierungsstelle aus (SHA-256 {}) – bcctl login vertraut ihr nach Rückfrage, oder --ca DATEI\n", self.url.origin(), which, fp));
            return Err(3);
        }
        if st == 0 {
            eprint(&format!("bcctl: keine Verbindung zu {}: {}\n", self.url.origin(), msg));
            return Err(3);
        }
        if st == 401 && j::str_of(&v, "code") == "unauthorized" && !path.starts_with("/api/auth/login") {
            eprint("bcctl: nicht angemeldet (bcctl login --user NAME)\n");
            return Err(1);
        }
        eprint(&format!("bcctl: {} ({})\n", if msg.is_empty() { "Fehler" } else { msg }, st));
        Err(1)
    }

    fn print(&self, v: &Value) {
        out(&j::text(v));
        out("\n");
    }

    fn save_login(&self, token: &str) {
        if let Some(p) = &self.ini {
            let mut ini = base::ini::Ini::load(p);
            ini.set("bcctl", "server", &self.url.origin());
            ini.set("bcctl", "token", token);
            let _ = plat::fs::mkdir_private(plat::fs::parent(p));
            let _ = plat::fs::write_file_private(p, &ini.to_bytes());
        }
    }

    /// a job by id or name
    fn job(&self, key: &str) -> Result<Job, i32> {
        let jobs: Vec<Job> = j::from_list(&self.ok(Method::Get, "/api/jobs", None)?);
        jobs.into_iter().find(|x| x.id == key || x.name.eq_ignore_ascii_case(key)).ok_or_else(|| {
            eprint(&format!("bcctl: Job „{}“ nicht gefunden\n", key));
            1
        })
    }

    /// follow an operation until it ends -> exit code
    fn wait(&self, id: &str) -> i32 {
        let mut last = String::new();
        loop {
            let v = match self.ok(Method::Get, &format!("/api/operations/{}", id), None) {
                Ok(v) => v,
                Err(c) => return c,
            };
            let Some(o) = Operation::from_json(&v) else { return 1 };
            let line = if o.total > 0 { format!("{}: {} %", o.title, o.done.min(o.total) * 100 / o.total) } else { format!("{}: {}", o.title, o.status.as_str()) };
            if line != last && !self.json {
                eprint(&format!("{}\n", line));
                last = line;
            }
            if o.status.finished() {
                if self.json {
                    self.print(&v);
                } else if o.status == OpStatus::Succeeded {
                    out(&format!("{}\n", o.result));
                } else {
                    eprint(&format!("{}: {}\n", o.status.as_str(), o.error));
                }
                return if o.status == OpStatus::Succeeded { 0 } else { 1 };
            }
            plat::sleep_ms(500);
        }
    }

    fn started(&self, v: &Value) -> i32 {
        let id = String::from(j::str_of(v, "operation_id"));
        if flag(&self.args, "--wait") {
            return self.wait(&id);
        }
        out(&format!("{}\n", id));
        0
    }

    fn run_cmd(&mut self, cmd: &str) -> Result<i32, i32> {
        let a = self.args.clone();
        let pos = positional(&a);
        let arg = |i: usize| -> Result<String, i32> {
            pos.get(i).cloned().ok_or_else(|| {
                eprint(&format!("bcctl {}: Argument fehlt (bcctl --help)\n", cmd));
                2
            })
        };
        match cmd {
            "setup" => {
                let token = opt(&a, "--token").ok_or(2)?;
                let user = opt(&a, "--user").ok_or(2)?;
                let pw = read_password();
                let body = j::obj([("token", Value::string(token)), ("username", Value::string(user.as_str())), ("password", Value::string(pw.as_str())),
                                   ("server_name", Value::string(opt(&a, "--name").unwrap_or_default()))]);
                self.ok(Method::Post, "/api/setup", Some(body))?;
                out(&format!("{} ist Administrator.\n", user));
                let r = self.ok(Method::Post, "/api/auth/login", Some(j::obj([("username", Value::string(user.as_str())), ("password", Value::string(pw))])))?;
                self.save_login(j::str_of(&r, "token"));
            }
            "login" => {
                let user = opt(&a, "--user").ok_or(2)?;
                self.ensure_trust()?;
                let pw = read_password();
                let r = self.ok(Method::Post, "/api/auth/login", Some(j::obj([("username", Value::string(user.as_str())), ("password", Value::string(pw))])))?;
                self.save_login(j::str_of(&r, "token"));
                out(&format!("Angemeldet als {} bei {} (bis {})\n", user, self.url.origin(), when(r.get("expires").and_then(|x| x.as_i64()).unwrap_or(0))));
            }
            "logout" => {
                let _ = self.ok(Method::Post, "/api/auth/logout", None);
                self.save_login("");
                out("Abgemeldet\n");
            }
            "status" => {
                let info = self.ok(Method::Get, "/api", None)?;
                let s = self.ok(Method::Get, "/api/summary", None)?;
                if self.json {
                    self.print(&s);
                    return Ok(0);
                }
                let s = Summary::from_json(&s).unwrap_or_default();
                out(&format!("{} {} · {}\n", j::str_of(&info, "name"), j::str_of(&info, "version"), self.url.origin()));
                out(&format!("Jobs {} ({} aktiv) · Hosts {}/{} online · {} Restore-Points\n", s.jobs, s.jobs_enabled, s.hosts_online, s.hosts, s.points));
                out(&format!("24 h: {} erfolgreich, {} Warnungen, {} fehlgeschlagen · {} laufende Vorgänge\n", s.ok_24h, s.warn_24h, s.fail_24h, s.running));
                out(&format!("Speicher: {} in Repositories · {} frei von {}\n", size(s.storage_used), size(s.storage_free), size(s.storage_capacity)));
                for al in &s.alerts {
                    out(&format!("! {} – {}\n", al.title, al.detail));
                }
            }
            "jobs" => {
                let v = self.ok(Method::Get, "/api/jobs", None)?;
                if self.json {
                    self.print(&v);
                    return Ok(0);
                }
                let rows: Vec<Vec<String>> = j::from_list::<Job>(&v).iter().map(|x| alloc::vec![x.name.clone(), String::from(x.last_status.as_str()), when(x.last_run), when(x.next_run), format!("{}", x.points), x.id.clone()]).collect();
                table(&["Name", "Status", "Letzter Lauf", "Nächster Lauf", "Points", "Id"], &rows);
            }
            "hosts" => {
                let v = self.ok(Method::Get, "/api/hosts", None)?;
                if self.json {
                    self.print(&v);
                    return Ok(0);
                }
                let rows: Vec<Vec<String>> = j::from_list::<Host>(&v).iter().map(|h| alloc::vec![h.name.clone(), String::from(h.kind.as_str()), String::from(h.status.as_str()), h.platform.clone(), h.roots.join(", ")]).collect();
                table(&["Name", "Art", "Status", "System", "Freigaben"], &rows);
            }
            "repos" => {
                let v = self.ok(Method::Get, "/api/repositories", None)?;
                if self.json {
                    self.print(&v);
                    return Ok(0);
                }
                let rows: Vec<Vec<String>> = j::from_list::<Repository>(&v).iter().map(|r| alloc::vec![r.name.clone(), String::from(r.status.as_str()), size(r.used), size(r.free), format!("{}", r.points), r.path.clone()]).collect();
                table(&["Name", "Status", "Belegt", "Frei", "Points", "Pfad"], &rows);
            }
            "run" => {
                let job = self.job(&arg(0)?)?;
                let v = self.ok(Method::Post, &format!("/api/jobs/{}/run", job.id), Some(j::obj([("full", Value::Bool(flag(&a, "--full")))])))?;
                return Ok(self.started(&v));
            }
            "points" => {
                let job = self.job(&arg(0)?)?;
                let v = self.ok(Method::Get, &format!("/api/jobs/{}/points", job.id), None)?;
                if self.json {
                    self.print(&v);
                    return Ok(0);
                }
                let rows: Vec<Vec<String>> = j::from_list::<RestorePoint>(&v).iter().map(|p| alloc::vec![when(p.created), String::from(if p.full { "voll" } else { "inkrementell" }), format!("{}", p.files), size(p.size), size(p.stored_new), p.id.clone()]).collect();
                table(&["Zeitpunkt", "Art", "Dateien", "Größe", "Neu", "Id"], &rows);
            }
            "sessions" => {
                let mut path = format!("/api/sessions?limit={}", opt(&a, "--limit").unwrap_or_else(|| String::from("20")));
                if let Some(jn) = opt(&a, "--job") {
                    path.push_str(&format!("&job={}", self.job(&jn)?.id));
                }
                let v = self.ok(Method::Get, &path, None)?;
                if self.json {
                    self.print(&v);
                    return Ok(0);
                }
                let rows: Vec<Vec<String>> = j::from_list::<Session>(&v).iter().map(|s| alloc::vec![when(s.started), s.job_name.clone(), String::from(s.kind.as_str()), String::from(s.status.as_str()), size(s.bytes_read), s.id.clone()]).collect();
                table(&["Start", "Job", "Art", "Status", "Gelesen", "Id"], &rows);
            }
            "log" => {
                let v = self.ok(Method::Get, &format!("/api/sessions/{}/log", arg(0)?), None)?;
                if self.json {
                    self.print(&v);
                    return Ok(0);
                }
                for l in j::from_list::<LogLine>(&v) {
                    out(&format!("{}  {:<7}  {}\n", when(l.at), l.level.as_str(), l.msg));
                }
            }
            "operations" => {
                let v = self.ok(Method::Get, "/api/operations", None)?;
                if self.json {
                    self.print(&v);
                    return Ok(0);
                }
                let rows: Vec<Vec<String>> = j::from_list::<Operation>(&v).iter().take(30).map(|o| alloc::vec![when(o.created), o.title.clone(), String::from(o.status.as_str()), if o.result.is_empty() { o.error.clone() } else { o.result.clone() }, o.id.clone()]).collect();
                table(&["Erstellt", "Vorgang", "Status", "Ergebnis", "Id"], &rows);
            }
            "audit" => {
                let v = self.ok(Method::Get, &format!("/api/audit?limit={}", opt(&a, "--limit").unwrap_or_else(|| String::from("50"))), None)?;
                if self.json {
                    self.print(&v);
                    return Ok(0);
                }
                let rows: Vec<Vec<String>> = j::from_list::<AuditEntry>(&v).iter().map(|e| alloc::vec![when(e.at), e.actor.clone(), e.action.clone(), e.target.clone(), String::from(if e.ok { "ok" } else { "abgelehnt" }), e.detail.clone()]).collect();
                table(&["Zeit", "Wer", "Aktion", "Ziel", "", "Details"], &rows);
            }
            "wait" => return Ok(self.wait(&arg(0)?)),
            "cancel" => {
                self.ok(Method::Post, &format!("/api/operations/{}/cancel", arg(0)?), None)?;
                out("Abbruch angefordert\n");
            }
            "verify" => {
                let v = self.ok(Method::Post, &format!("/api/points/{}/verify", arg(0)?), Some(Value::Object(Vec::new())))?;
                return Ok(self.started(&v));
            }
            "restore" => {
                let target = opt(&a, "--target").unwrap_or_default();
                let items: Vec<Value> = opts(&a, "--item").into_iter().map(Value::string).collect();
                let body = j::obj([
                    ("items", Value::Array(items)),
                    ("original", Value::Bool(flag(&a, "--original"))),
                    ("host_id", Value::string("")),
                    ("target", Value::string(target)),
                    ("conflict", Value::string(opt(&a, "--conflict").unwrap_or_else(|| String::from("skip")))),
                ]);
                let v = self.ok(Method::Post, &format!("/api/points/{}/restore", arg(0)?), Some(body))?;
                return Ok(self.started(&v));
            }
            "update" => {
                let v = match pos.first().map(String::as_str) {
                    Some("check") => self.ok(Method::Post, "/api/update/check", Some(j::obj([])))?,
                    Some("download") => {
                        let v = self.ok(Method::Post, "/api/update/download", Some(j::obj([])))?;
                        return Ok(self.started(&v));
                    }
                    Some(x) => {
                        eprint(&format!("bcctl update: unbekannt „{}“ (check, download)\n", x));
                        return Err(2);
                    }
                    None => self.ok(Method::Get, "/api/update", None)?,
                };
                if self.json {
                    out(&format!("{}\n", j::text(&v)));
                } else if let Some(s) = UpdateStatus::from_json(&v) {
                    out(&format!("Diese Version:  {}\nKanal:          {}\n", s.current, if s.channel.is_empty() { "-" } else { &s.channel }));
                    if !s.latest.is_empty() {
                        out(&format!("Neueste:        {}{}\n", s.latest, if s.available { "  (verfügbar)" } else { "" }));
                        out(&format!("Paket:          {} ({} Bytes)\n", if s.file.is_empty() { "– (keins für diese Plattform)" } else { &s.file }, s.size));
                    }
                    if !s.downloaded.is_empty() {
                        out(&format!("Heruntergeladen: {}\n", s.downloaded));
                    }
                    if !s.error.is_empty() {
                        out(&format!("Fehler:         {}\n", s.error));
                    }
                    if !s.notes.is_empty() {
                        out(&format!("\n{}\n", s.notes));
                    }
                }
            }
            "release" => return Ok(release(&a, &pos)),
            "tape" => return Ok(tape::cmd(&a, &pos)),
            "docs" => {
                for r in ROUTES {
                    let who = match r.access {
                        Access::Public => String::from("öffentlich"),
                        Access::Agent => String::from("Agent"),
                        Access::Fleet => String::from("Überwachungs-Token"),
                        Access::Role(x) => String::from(x.as_str()),
                    };
                    out(&format!("{:<7} {:<36} {:<10} {}{}{}\n", r.method.as_str(), r.path, who, r.summary, if r.long { " (Vorgang)" } else { "" },
                                 if r.capability.is_empty() { String::new() } else { format!(" [{}]", r.capability) }));
                }
            }
            _ => {
                eprint(&format!("bcctl: unbekannter Befehl „{}“ (bcctl --help)\n", cmd));
                return Err(2);
            }
        }
        Ok(0)
    }
}

/// bcctl release keygen FILE | manifest --key FILE --dir DIR --version V
fn release(a: &[String], pos: &[String]) -> i32 {
    match pos.first().map(String::as_str) {
        Some("keygen") => {
            let Some(file) = pos.get(1) else { return 2 };
            if plat::fs::stat(file).is_some() {
                eprint(&format!("bcctl: {} gibt es schon\n", file));
                return 1;
            }
            let seed: [u8; 32] = plat::random_bytes();
            let kp = crypto::ed25519::Keypair::from_seed(&seed);
            if !plat::fs::write_file_private(file, format!("BCRELKEY1 {}\n", crypto::hex(&seed)).as_bytes()) {
                eprint(&format!("bcctl: {} kann nicht geschrieben werden\n", file));
                return 1;
            }
            out(&format!("{}\n", crypto::hex(&kp.public)));
            0
        }
        Some("manifest") => {
            let (Some(key), Some(dir), Some(version)) = (opt(a, "--key"), opt(a, "--dir"), opt(a, "--version")) else {
                eprint("bcctl release manifest --key FILE --dir DIR --version V\n");
                return 2;
            };
            let seed = plat::read_file(&key).and_then(|b| {
                let t = String::from_utf8_lossy(&b).into_owned();
                t.trim().strip_prefix("BCRELKEY1 ").and_then(crypto::unhex).and_then(|s| <[u8; 32]>::try_from(s.as_slice()).ok())
            });
            let Some(seed) = seed else {
                eprint(&format!("bcctl: {} ist kein Schlüssel von bcctl release keygen\n", key));
                return 1;
            };
            let kp = crypto::ed25519::Keypair::from_seed(&seed);
            let overrides: Vec<(String, String)> = opts(a, "--platform").iter().filter_map(|x| x.split_once('=').map(|(n, p)| (String::from(n), String::from(p)))).collect();
            let mut files = Vec::new();
            for e in plat::fs::read_dir(&dir).unwrap_or_default() {
                let name = e.name.clone();
                if name.starts_with("manifest.json") {
                    continue;
                }
                let platform = overrides.iter().find(|(n, _)| *n == name).map(|(_, p)| p.clone()).or_else(|| {
                    if name.ends_with(".exe") {
                        Some(String::from("windows-x86_64"))
                    } else if name.ends_with("_amd64.deb") {
                        Some(String::from("linux-x86_64"))
                    } else {
                        None
                    }
                });
                let Some(platform) = platform else { continue };
                let Some(bytes) = plat::read_file(&plat::fs::join(&dir, &name)) else { continue };
                out(&format!("{:<16} {}\n", platform, name));
                files.push(UpdateFile { platform, name, size: bytes.len() as u64, sha256: crypto::hex(&crypto::sha256(&bytes)) });
            }
            if files.is_empty() {
                eprint(&format!("bcctl: keine Pakete in {}\n", dir));
                return 1;
            }
            let m = UpdateManifest {
                product: String::from(bc_proto::PRODUCT),
                version,
                date: plat::unix_time_ns().div_euclid(1_000_000_000),
                notes: opt(a, "--notes").unwrap_or_default(),
                files,
            };
            let text = j::text(&m.to_json());
            let sig = kp.sign(text.as_bytes());
            let mp = plat::fs::join(&dir, "manifest.json");
            if !plat::write_file(&mp, text.as_bytes()) || !plat::write_file(&format!("{}.sig", mp), format!("{}\n", crypto::b64::encode(&sig)).as_bytes()) {
                eprint(&format!("bcctl: {} kann nicht geschrieben werden\n", mp));
                return 1;
            }
            out(&format!("{} und {}.sig geschrieben (Schlüssel {})\n", mp, mp, crypto::hex(&kp.public)));
            0
        }
        _ => {
            eprint("bcctl release keygen FILE | manifest --key FILE --dir DIR --version V\n");
            2
        }
    }
}

fn help() -> i32 {
    out("bcctl [--server URL] [--json] BEFEHL\n\
  setup --token T --user NAME [--name SERVERNAME]\n\
  login --user NAME | logout | status\n\
  jobs | hosts | repos | operations | audit [--limit N]\n\
  run JOB [--full] [--wait] | points JOB | sessions [--job JOB] [--limit N] | log SITZUNG\n\
  verify POINT [--wait] | wait VORGANG | cancel VORGANG\n\
  restore POINT --target ORDNER [--item PFAD]... [--conflict skip|rename|overwrite] [--original] [--wait]\n\
  update [check | download [--wait]]\n\
  release keygen DATEI | release manifest --key DATEI --dir ORDNER --version V [--notes TEXT]\n\
  tape list BAND... | tape extract BAND... --to ORDNER [--set ID] [--kit DATEI]   (ohne Server)\n\
  docs\n\
Passwort: Standardeingabe oder BC_PASSWORD.\n");
    2
}

fn run() -> i32 {
    let args: Vec<String> = plat::args().into_iter().skip(1).collect();
    // --version alone (release manifest takes --version V)
    if args.len() == 1 && (args[0] == "--version" || args[0] == "-V") {
        out(&bc_proto::version_line("bcctl"));
        return 0;
    }
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        return help();
    }
    let ini = plat::config_dir().map(|d| plat::fs::join(&plat::fs::join(&d, "BackupCraft"), "bcctl.ini"));
    let saved = ini.as_ref().map(|p| base::ini::Ini::load(p));
    let server = opt(&args, "--server")
        .or_else(|| plat::env("BC_SERVER"))
        .or_else(|| saved.as_ref().and_then(|i| i.get("bcctl", "server").map(String::from)))
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", bc_proto::DEFAULT_PORT));
    let server = if server.contains("://") { server } else { format!("http://{}", server) };
    let Some(url) = Url::parse(&server) else {
        eprint(&format!("bcctl: ungültige Serveradresse {}\n", server));
        return 2;
    };
    let token = plat::env("BC_TOKEN").or_else(|| saved.as_ref().and_then(|i| i.get("bcctl", "token").map(String::from))).unwrap_or_default();
    // an https server: its CA file, a fingerprint given or kept, or ask (bcctl login)
    let ca = opt(&args, "--ca").or_else(|| plat::env("BC_CA")).map(|f| {
        plat::read_file(&f).and_then(|b| netio::pem::decode(&String::from_utf8_lossy(&b), "CERTIFICATE")).and_then(|d| netio::x509::Cert::parse(&d)).ok_or(f)
    });
    let trust = match ca {
        Some(Ok(c)) => ServerTrust::Ca(c),
        Some(Err(f)) => {
            eprint(&format!("bcctl: {} ist kein lesbares Zertifikat (PEM)\n", f));
            return 2;
        }
        None => match opt(&args, "--fingerprint").and_then(|f| trust::parse_fingerprint(&f))
            .or_else(|| saved.as_ref().and_then(|i| i.get("trust", &url.host_header())).and_then(trust::parse_fingerprint))
        {
            Some(p) => ServerTrust::Pin(p),
            None => ServerTrust::Ask,
        },
    };
    let cmd = positional(&args).first().cloned().unwrap_or_default();
    let rest: Vec<String> = {
        let i = args.iter().position(|a| *a == cmd).map_or(args.len(), |i| i + 1);
        let mut v: Vec<String> = args[i..].to_vec();
        // global options before the command count too
        for k in ["--json", "--wait", "--full"] {
            if args[..i.min(args.len())].iter().any(|a| a == k) {
                v.push(String::from(k));
            }
        }
        v
    };
    let mut cli = Cli { url, token, json: flag(&args, "--json"), args: rest, ini, trust, seen: alloc::sync::Arc::new(Mutex::new(None)) };
    match cli.run_cmd(&cmd) {
        Ok(c) | Err(c) => c,
    }
}

rt::main!(run());
