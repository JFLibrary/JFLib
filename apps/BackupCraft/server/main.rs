// bc-server -- the BackupCraft backup server: REST API, scheduler, backup engine
//
//   bc-server [run] [--data DIR] [--listen ADDR:PORT] [--tls] [--name N,..] [--threads N] [--repair]
//   bc-server setup-token [--data DIR]      print the one-time token of the first setup
//   bc-server ca [--data DIR]               print the CA certificate (PEM) and its fingerprint
//   bc-server service install [run options] | uninstall | start | stop | status
//                                           the system service (Windows: BackupCraftServer,
//                                           Linux: systemd unit backupcraft-server; as
//                                           administrator / root)
//   bc-server --version
//
// The log goes to standard output and <data>/logs/server.log (10 MB, five older files kept).
// Data directory (default): Windows %ProgramData%\BackupCraft\server, Linux
// /var/lib/backupcraft (root) or ~/.local/share/backupcraft/server.  The API listens on
// 127.0.0.1:7642.  Loopback speaks plain HTTP unless --tls; every other address speaks TLS 1.3
// with the server's own CA (pki/ca.pem).  Agents prove themselves with a client certificate of
// that CA on the same port; --name adds DNS names / addresses to the server certificate.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod admin;
mod agents;
mod api;
mod auth;
mod cdp;
mod copy;
mod ctx;
mod engine;
mod fleet;
mod hosts;
mod jobs;
mod keys;
mod monitor;
mod notify;
mod ops;
mod pki;
mod replica;
mod repos;
mod update;
mod restore;
mod ssh_push;
mod tape;
mod util;
mod vm;
mod volume;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::j::Json;
use bc_proto::model::Job;
use datastore::{Journal, Options, Store};
use netio::server::Wrap;
use netio::tls::Trust;
use netio::{Server, ServerConfig, Stream};
use plat::net::{IpAddr, SocketAddr, TcpStream};

use crate::ctx::{Ctx, C_JOBS};
use crate::keys::Keys;
use crate::ops::Ops;

/// the name of the system service
const SERVICE: &str = if cfg!(windows) { "BackupCraftServer" } else { "backupcraft-server" };

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

static LOG: plat::sync::Mutex<Option<Arc<bc_fs::Log>>> = plat::sync::Mutex::new(None);

/// a line of the server's log (standard output and <data>/logs/server.log)
pub fn log(s: &str) {
    let l = LOG.lock().clone();
    match l {
        Some(l) => l.line(s),
        None => out(&format!("{}\n", s)),
    }
}

fn fail(s: &str) -> i32 {
    // the service has no console: its log says why it ended
    let l = LOG.lock().clone();
    match l {
        Some(l) => l.line(&format!("Fehler: {}", s)),
        None => plat::write_stderr(format!("bc-server: {}\n", s).as_bytes()),
    }
    1
}

/// remove a directory tree (best effort)
pub fn remove_tree(path: &str) {
    if let Some(list) = plat::fs::read_dir(path) {
        for e in list {
            let p = plat::fs::join(path, &e.name);
            if e.kind == plat::fs::FileKind::Dir {
                remove_tree(&p);
            } else {
                plat::delete(&p);
            }
        }
    }
    plat::delete(path);
}

fn default_data() -> String {
    if cfg!(windows) {
        let base = plat::env("ProgramData").unwrap_or_else(|| String::from("C:\\ProgramData"));
        return plat::fs::join(&plat::fs::join(&base, "BackupCraft"), "server");
    }
    if plat::fs::stat("/var/lib").is_some() && plat::env("USER").as_deref() == Some("root") {
        return String::from("/var/lib/backupcraft");
    }
    let home = plat::env("HOME").unwrap_or_else(|| String::from("."));
    plat::fs::join(&home, ".local/share/backupcraft/server")
}

fn usage() -> i32 {
    out(concat!(
        "bc-server [run] [--data DIR] [--listen ADDR:PORT] [--tls] [--name NAME,..] [--threads N] [--repair]\n",
        "bc-server setup-token [--data DIR]\n",
        "bc-server ca [--data DIR]\n",
        "bc-server service install [--data DIR] [--listen ADDR:PORT] [--tls] [--name NAME,..] [--threads N] [--report FILE]\n",
        "bc-server service uninstall | start | stop | status\n",
        "bc-server --version\n",
    ));
    2
}

fn data_dir() -> String {
    let data = plat::arg_value("--data").unwrap_or_else(default_data);
    if admin::is_absolute(&data) { data } else { plat::fs::join(&plat::cwd(), &data) }
}

fn run() -> i32 {
    let args = plat::args();
    let rest: Vec<&str> = args.iter().skip(1).map(|s| s.as_str()).collect();
    if rest.iter().any(|a| *a == "--version" || *a == "-V") {
        out(&bc_proto::version_line("bc-server"));
        return 0;
    }
    if rest.iter().any(|a| *a == "--help" || *a == "-h") {
        return usage();
    }
    if rest.first() == Some(&"service") {
        return service_cmd(rest.get(1).copied().unwrap_or("status"));
    }
    if rest.contains(&"--service") {
        return plat::service::run_service(SERVICE, serve);
    }
    serve()
}

/// bc-server service ...
fn service_cmd(sub: &str) -> i32 {
    let r = match sub {
        "install" => {
            let Some(exe) = plat::current_exe() else { return fail("der eigene Programmpfad ist unbekannt") };
            let mut args = alloc::vec![String::from("run"), String::from("--service"), String::from("--data"), data_dir()];
            for o in ["--listen", "--name", "--threads"] {
                if let Some(v) = plat::arg_value(o) {
                    args.push(String::from(o));
                    args.push(v);
                }
            }
            if plat::args().iter().any(|a| a == "--tls") {
                args.push(String::from("--tls"));
            }
            let spec = plat::service::ServiceSpec {
                name: SERVICE,
                display: "BackupCraft Server",
                description: "BackupCraft: Sicherungsserver (REST-API, Zeitpläne, Repositories)",
                exe: &exe,
                args: &args,
            };
            let r = plat::service::install(&spec).and_then(|_| plat::service::start(SERVICE)).map(|_| {
                format!("Dienst {} eingerichtet und gestartet (Daten: {}; das Setup-Token steht in setup.token)", SERVICE, args[3])
            });
            if let Some(file) = plat::arg_value("--report") {
                report(&file, &args[3], &r);
            }
            r
        }
        "uninstall" => plat::service::uninstall(SERVICE).map(|_| format!("Dienst {} entfernt (die Daten bleiben)", SERVICE)),
        "start" => plat::service::start(SERVICE).map(|_| format!("Dienst {} gestartet", SERVICE)),
        "stop" => plat::service::stop(SERVICE).map(|_| format!("Dienst {} angehalten", SERVICE)),
        "status" => Ok(format!("Dienst {}: {}", SERVICE, plat::service::state(SERVICE).text())),
        _ => return usage(),
    };
    match r {
        Ok(m) => {
            out(&format!("{}\n", m));
            0
        }
        Err(e) => fail(&e),
    }
}

/// --report FILE (the console's setup assistant, which started this install with administrator
/// rights and cannot read the data folder): once the service runs, its setup token (empty when it
/// is set up already) and the SHA-256 of its CA as JSON {ok, message, token, ca_fingerprint}
fn report(file: &str, data: &str, r: &Result<String, String>) {
    let mut v = bc_proto::j::obj([
        ("ok", bc_proto::j::Value::Bool(r.is_ok())),
        ("message", bc_proto::j::Value::string(match r {
            Ok(m) | Err(m) => m.as_str(),
        })),
    ]);
    if r.is_ok() {
        let ca = plat::fs::join(&plat::fs::join(data, "pki"), "ca.pem");
        let token = plat::fs::join(data, "setup.token");
        // the service writes its CA at its start, then (when it is new) the token
        for _ in 0..600 {
            if plat::file_exists(&ca) == 1 {
                break;
            }
            plat::sleep_ms(100);
        }
        for _ in 0..100 {
            if plat::file_exists(&token) == 1 {
                break;
            }
            plat::sleep_ms(100);
        }
        plat::sleep_ms(200);
        let t = plat::read_file(&token).map(|b| String::from(String::from_utf8_lossy(&b).trim())).unwrap_or_default();
        let fp = plat::read_file(&ca)
            .and_then(|b| netio::pem::decode(&String::from_utf8_lossy(&b), "CERTIFICATE"))
            .map(|der| netio::trust::fingerprint_text(&crypto::sha256(&der)))
            .unwrap_or_default();
        v.insert("token", bc_proto::j::Value::string(t));
        v.insert("ca_fingerprint", bc_proto::j::Value::string(fp));
    }
    let _ = plat::write_file(file, bc_proto::j::text(&v).as_bytes());
}

/// the server itself (a console program or the system service)
fn serve() -> i32 {
    let args = plat::args();
    let rest: Vec<&str> = args.iter().skip(1).map(|s| s.as_str()).collect();
    let cmd = rest.first().filter(|a| !a.starts_with("--")).copied().unwrap_or("run");
    let data = data_dir();
    if !plat::fs::mkdir_private(&data) {
        return fail(&format!("das Datenverzeichnis {} kann nicht angelegt werden", data));
    }
    if cmd == "run" {
        *LOG.lock() = Some(Arc::new(bc_fs::Log::new(Some(&plat::fs::join(&data, "logs")), "server")));
    }
    let keydir = plat::fs::join(&data, "keys");
    // a staged configuration restore replaces database and keys before anything opens them
    if plat::file_exists(&plat::fs::join(&data, "config.restore")) == 1 {
        match Keys::load(&keydir, false) {
            Ok(k) => match admin::apply_staged(&data, &k) {
                Some(Ok(m)) => out(&format!("{}\n", m)),
                Some(Err(e)) => return fail(&format!("Konfiguration konnte nicht wiederhergestellt werden: {}", e)),
                None => {}
            },
            Err(e) => return fail(&e),
        }
    }
    let repair = rest.contains(&"--repair");
    let db = match Store::open_with(&plat::fs::join(&data, "db"), Options { repair, ..Options::default() }) {
        Ok(s) => s,
        Err(e) => return fail(&format!("{} (Datenbank {}; mit --repair wird das Protokoll an der Schadstelle abgeschnitten)", e, plat::fs::join(&data, "db"))),
    };
    if let Some(r) = &db.recovery().repaired {
        out(&format!("Datenbank repariert: {}\n", r));
    }
    let set_up = db.get(ctx::C_META, "server").and_then(|v| v.get("setup").and_then(|x| x.as_bool())).unwrap_or(false);
    let keys = match Keys::load(&keydir, !set_up) {
        Ok(k) => k,
        Err(e) => return fail(&format!("{} -- ein eingerichteter Server startet nicht ohne seine Schlüssel", e)),
    };
    let journal = match Journal::open(&plat::fs::join(&data, "journal"), true) {
        Ok(j) => j,
        Err(e) => return fail(&format!("{}", e)),
    };
    let listen = match plat::arg_value("--listen") {
        Some(a) => match SocketAddr::parse(&a) {
            Some(x) => x,
            None => return fail("--listen erwartet ADRESSE:PORT, z. B. 127.0.0.1:7642"),
        },
        None => SocketAddr::new(IpAddr::LOCALHOST_V4, bc_proto::DEFAULT_PORT),
    };
    let tls = !listen.ip.is_loopback() || rest.contains(&"--tls");
    let extra: Vec<String> = plat::arg_value("--name").map(|n| n.split(',').map(|x| String::from(x.trim())).filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    let (dns, ips) = pki::local_names(&listen, &extra);
    let pki = match pki::Pki::load(&data, &keys, &dns, &ips) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    if cmd == "ca" {
        out(&pki.ca_pem());
        out(&format!("SHA-256: {}\n", pki.ca_fingerprint()));
        return 0;
    }
    Ops::interrupt_stale(&db);
    interrupt_sessions(&db);
    let ctx = Arc::new(Ctx {
        data: data.clone(),
        db,
        journal,
        keys,
        ops: Ops::new(),
        listen,
        tls,
        repos: plat::sync::Mutex::new(alloc::collections::BTreeMap::new()),
        pki,
        agents: plat::sync::Mutex::new(alloc::collections::BTreeMap::new()),
    });
    ctx::set_current(&ctx);
    if cmd == "setup-token" {
        return match auth::ensure_setup_token(&ctx) {
            Some(t) => {
                out(&format!("{}\n", t));
                0
            }
            None => fail("der Server ist bereits eingerichtet"),
        };
    }
    if cmd != "run" {
        return usage();
    }
    if let Some(t) = auth::ensure_setup_token(&ctx) {
        log(&format!("Noch nicht eingerichtet. Setup-Token: {}", t));
    }
    let threads = plat::arg_value("--threads").and_then(|t| t.parse().ok()).unwrap_or(16usize).clamp(2, 128);
    let cfg = ServerConfig { threads, max_body: 32 << 20, ..ServerConfig::default() };
    let c2 = ctx.clone();
    let wrap = if tls { Some(tls_wrap(&ctx)) } else { None };
    let server = match Server::start_with(&listen, cfg, Arc::new(move |req| api::handle(&c2, req)), wrap) {
        Ok(s) => s,
        Err(e) => {
            log(&format!("{} kann nicht geöffnet werden: {}", listen, e));
            return fail(&format!("{} kann nicht geöffnet werden: {}", listen, e));
        }
    };
    let scheme = if tls { "https" } else { "http" };
    log(&format!("{} {} lauscht auf {}://{} (Daten: {})", bc_proto::PRODUCT, bc_proto::VERSION, scheme, server.local_addr(), data));
    if tls {
        log(&format!("CA-Fingerprint (SHA-256): {}", ctx.pki.ca_fingerprint()));
    }
    ctx.audit("system", "server.start", &ctx.server_name(), bc_proto::VERSION, true);
    cdp::start(&ctx);
    fleet::start(&ctx);
    vm::start(&ctx);
    scheduler(&ctx);
    log("Der Dienst hält an");
    server.stop();
    ctx.audit("system", "server.stop", &ctx.server_name(), "", true);
    0
}

/// the TLS layer of the API: consoles come without a certificate, agents with the one the CA
/// issued at pairing (CN = host id); it counts only while the host exists, the fingerprint is
/// the one on record and it is not revoked
fn tls_wrap(ctx: &Arc<Ctx>) -> Wrap {
    let cfg = netio::tls::ServerConfig { identity: ctx.pki.server.clone(), client_auth: Some(Trust::Anchors(alloc::vec![ctx.pki.ca.clone()])), client_auth_optional: true };
    let ctx = ctx.clone();
    Arc::new(move |tcp: TcpStream| {
        let s = netio::tls::accept(Box::new(tcp), cfg.clone()).ok()?;
        let id = s.peer_certs().first().and_then(|c| agent_of(&ctx, c));
        Some((Box::new(s) as Box<dyn Stream>, id))
    })
}

fn agent_of(ctx: &Ctx, cert: &netio::x509::Cert) -> Option<String> {
    let fp = cert.fingerprint();
    let revoked: Vec<String> = ctx.db.get(ctx::C_META, "revoked").map(|v| bc_proto::j::Field::take(v.get("fingerprints"))).unwrap_or_default();
    if revoked.contains(&fp) {
        return None;
    }
    let h = ctx.get(ctx::C_HOSTS, &cert.subject_cn)?;
    (bc_proto::j::str_of(&h, "fingerprint") == fp).then(|| cert.subject_cn.clone())
}

/// sessions (and their jobs) that were running when the server stopped
fn interrupt_sessions(db: &Store) {
    use bc_proto::model::{Session, SessionStatus};
    let t = util::now();
    let mut jobs = alloc::collections::BTreeSet::new();
    let _ = db.write(|data, tx| {
        for (id, v) in data.get(ctx::C_SESSIONS).into_iter().flatten() {
            if let Some(mut s) = Session::from_json(v) {
                if s.status == SessionStatus::Running {
                    s.status = SessionStatus::Failed;
                    s.finished = t;
                    s.message = String::from("Unterbrochen: der Server wurde beendet");
                    jobs.insert(s.job_id.clone());
                    tx.put(ctx::C_SESSIONS, id, s.to_json());
                }
            }
        }
        for (id, v) in data.get(C_JOBS).into_iter().flatten() {
            if jobs.contains(id) || bc_proto::j::str_of(v, "last_status") == "running" {
                let mut v = v.clone();
                v.insert("last_status", bc_proto::j::Value::string("failed"));
                tx.put(C_JOBS, id, v);
            }
        }
        Ok::<(), datastore::StoreError>(())
    });
}

/// the scheduler: due jobs start, expired tokens and old operations go (every 15 s)
fn scheduler(ctx: &Arc<Ctx>) {
    let mut round = 0u64;
    loop {
        let t = util::now();
        if ctx.is_set_up() {
            for (id, v) in ctx.db.list(C_JOBS) {
                let Some(job) = Job::from_json(&v) else { continue };
                if job.enabled && job.next_run > 0 && job.next_run <= t {
                    log(&format!("Zeitplan: Job „{}“ startet", job.name));
                    jobs::scheduled(ctx, &job);
                    let next = jobs::next_run(&job.schedule, true, t);
                    let _ = ctx.db.update(C_JOBS, &id, |v| {
                        v.insert("next_run", bc_proto::j::Value::i64(next));
                    });
                }
            }
            if round % 4 == 0 {
                notify::daily(ctx);
                update::tick(ctx);
            }
            if round % 240 == 0 {
                auth::prune(ctx);
                Ops::prune(&ctx.db, 30);
            }
        }
        round += 1;
        // 15 s, ended early by a stop request of the service manager
        for _ in 0..15 {
            if plat::service::stop_requested() {
                return;
            }
            plat::sleep_ms(1000);
        }
    }
}

rt::main!(run());
