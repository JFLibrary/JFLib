// bc-agent -- the BackupCraft agent: shares folders of this machine with one backup server
//
//   bc-agent pair --server URL --code CODE [--fingerprint FP | --yes] [--name NAME]
//                 [--address HOST[:PORT]] [--listen ADDR:PORT] [--root DIR].. [--restore-root DIR]..
//                 [--lock] [--force]
//   bc-agent run [--listen ADDR:PORT]
//   bc-agent status
//   bc-agent roots [--add DIR].. [--remove DIR].. [--restore-add DIR].. [--restore-remove DIR]..
//                  [--lock | --unlock]
//   bc-agent service install [--listen ADDR:PORT] | uninstall | start | stop | status
//                  the system service (Windows: BackupCraftAgent, Linux: systemd unit
//                  backupcraft-agent; as administrator / root)
//   bc-agent --version                                        (every command takes --data DIR)
//
// Data directory (default): Windows %ProgramData%\BackupCraft\agent, Linux
// /var/lib/backupcraft-agent (root) or ~/.local/share/backupcraft/agent (config.rs).  The log
// goes to standard output and <data>/logs/agent.log (10 MB, five older files kept).
//
// The agent listens on port 7680 (TLS 1.3) and accepts only the server's client certificate:
// issued by the CA it paired with, CN "BackupCraft Server".  It reads only inside its backup
// folders, writes only inside its backup and restore folders, follows no link below them and
// runs no commands.  Every 30 s it reports to the server (mutual TLS, the server's name is
// checked against its certificate).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod api;
mod config;
mod pair;

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::agent::{Heartbeat, SERVER_CN};
use bc_proto::j::{self, Json};
use bc_proto::model::Os;
use netio::client::{json_headers, Session};
use netio::server::Wrap;
use netio::tls::{self, Trust};
use netio::{Client, Server, ServerConfig, Stream, Url};
use plat::net::{IpAddr, SocketAddr, TcpStream};
use plat::sync::Mutex;

use crate::api::State;
use crate::config::Config;

/// the name of the system service
const SERVICE: &str = if cfg!(windows) { "BackupCraftAgent" } else { "backupcraft-agent" };

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

static LOG: Mutex<Option<Arc<bc_fs::Log>>> = Mutex::new(None);

fn fail(s: &str) -> i32 {
    // the service has no console: its log says why it ended
    let l = LOG.lock().clone();
    match l {
        Some(l) => l.line(&format!("Fehler: {}", s)),
        None => plat::write_stderr(format!("bc-agent: {}\n", s).as_bytes()),
    }
    1
}

/// a line of the agent's log (standard output and <data>/logs/agent.log)
pub fn log(s: &str) {
    let l = LOG.lock().clone();
    match l {
        Some(l) => l.line(s),
        None => out(&format!("{}\n", s)),
    }
}

pub fn now() -> i64 {
    plat::unix_time_ns().div_euclid(1_000_000_000)
}

pub fn os() -> Os {
    if cfg!(windows) { Os::Windows } else { Os::Linux }
}

pub fn platform() -> String {
    String::from(if cfg!(windows) { "Windows" } else { "Linux" })
}

fn usage() -> i32 {
    out("bc-agent pair --server URL --code CODE [--fingerprint FP | --yes] [--name NAME] [--address HOST[:PORT]]\n\
         \x20                [--listen ADDR:PORT] [--root DIR].. [--restore-root DIR].. [--lock] [--force]\n\
         \x20                [--service] [--report FILE]   (also install the service; the result as JSON)\n\
         bc-agent run [--listen ADDR:PORT]\n\
         bc-agent status\n\
         bc-agent roots [--add DIR].. [--remove DIR].. [--restore-add DIR].. [--restore-remove DIR].. [--lock | --unlock]\n\
         bc-agent service install [--listen ADDR:PORT] | uninstall | start | stop | status\n\
         bc-agent --version\n\
         (alle Befehle: --data DIR)\n");
    2
}

fn run() -> i32 {
    let args = plat::args();
    let rest: Vec<&str> = args.iter().skip(1).map(|s| s.as_str()).collect();
    if rest.iter().any(|a| *a == "--version" || *a == "-V") {
        out(&bc_proto::version_line("bc-agent"));
        return 0;
    }
    if rest.iter().any(|a| *a == "--help" || *a == "-h") {
        return usage();
    }
    let cmd = rest.first().filter(|a| !a.starts_with("--")).copied().unwrap_or("");
    let dir = config::data_dir();
    if cmd == "run" && rest.contains(&"--service") {
        return plat::service::run_service(SERVICE, || serve(&config::data_dir()));
    }
    match cmd {
        "pair" => {
            let mut r = pair::pair(&dir);
            if rest.contains(&"--service") {
                r = r.and_then(|m| install_service(&dir).map(|s| format!("{}\n{}", m, s)));
            }
            if let Some(f) = plat::arg_value("--report") {
                let msg = match &r {
                    Ok(m) | Err(m) => m.clone(),
                };
                let v = bc_proto::j::obj([("ok", bc_proto::j::Value::Bool(r.is_ok())), ("message", bc_proto::j::Value::string(msg))]);
                let _ = plat::write_file(&f, bc_proto::j::text(&v).as_bytes());
            }
            match r {
                Ok(m) => {
                    out(&format!("{}\n", m));
                    0
                }
                Err(e) => fail(&e),
            }
        }
        "run" => serve(&dir),
        "status" => status(&dir),
        "roots" => roots(&dir),
        "service" => service_cmd(&dir, rest.get(1).copied().unwrap_or("status")),
        _ => usage(),
    }
}

/// bc-agent service ...
fn service_cmd(dir: &str, sub: &str) -> i32 {
    let r = match sub {
        "install" => install_service(dir),
        "uninstall" => plat::service::uninstall(SERVICE).map(|_| format!("Dienst {} entfernt (Kopplung und Freigaben bleiben)", SERVICE)),
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

/// the agent as a system service (it must be paired)
fn install_service(dir: &str) -> Result<String, String> {
    {
        {
            if config::load_paired(dir).is_err() {
                return Err(format!("zuerst koppeln: bc-agent pair --server URL --code CODE --data {}", dir));
            }
            let Some(exe) = plat::current_exe() else { return Err(String::from("der eigene Programmpfad ist unbekannt")) };
            let mut args = alloc::vec![String::from("run"), String::from("--service"), String::from("--data"), String::from(dir)];
            if let Some(l) = plat::arg_value("--listen") {
                args.push(String::from("--listen"));
                args.push(l);
            }
            let spec = plat::service::ServiceSpec {
                name: SERVICE,
                display: "BackupCraft Agent",
                description: "BackupCraft: Agent für Sicherungen dieses Rechners (Port 7680, nur für den gekoppelten Server)",
                exe: &exe,
                args: &args,
            };
            plat::service::install(&spec).and_then(|_| plat::service::start(SERVICE)).map(|_| format!("Dienst {} eingerichtet und gestartet (Daten: {})", SERVICE, dir))
        }
    }
}

fn status(dir: &str) -> i32 {
    let Some(c) = config::load(dir).filter(|c| !c.host_id.is_empty()) else {
        out(&format!("nicht gekoppelt (Daten: {})\n", dir));
        return 1;
    };
    out(&format!("Host:        {} ({})\nServer:      {}\nCA:          {}\nLauscht:     {}\n", c.name, c.host_id, c.server, netio::trust::fingerprint_text(&crypto::unhex(&c.ca_fingerprint).unwrap_or_default()), listen_of(&c)));
    out(&format!("Sicherung:   {}\nWiederherst.: {}\nFernändern:  {}\nDaten:       {}\n", list_text(&c.roots), list_text(&c.restore_roots), if c.remote_roots { "erlaubt" } else { "gesperrt" }, dir));
    0
}

fn list_text(v: &[String]) -> String {
    if v.is_empty() { String::from("(keine)") } else { v.join(", ") }
}

fn roots(dir: &str) -> i32 {
    let Some(mut c) = config::load(dir).filter(|c| !c.host_id.is_empty()) else { return fail("nicht gekoppelt") };
    let win = cfg!(windows);
    let edit = |list: &mut Vec<String>, add: &str, remove: &str| -> Result<(), String> {
        for a in bc_fs::clean_roots(&pair::values(add), win)? {
            if !list.contains(&a) {
                list.push(a);
            }
        }
        let gone = pair::values(remove);
        list.retain(|r| !gone.iter().any(|g| g.trim() == r));
        Ok(())
    };
    if let Err(e) = edit(&mut c.roots, "--add", "--remove").and_then(|_| edit(&mut c.restore_roots, "--restore-add", "--restore-remove")) {
        return fail(&e);
    }
    let a = plat::args();
    if a.iter().any(|x| x == "--lock") {
        c.remote_roots = false;
    }
    if a.iter().any(|x| x == "--unlock") {
        c.remote_roots = true;
    }
    if let Err(e) = config::save(dir, &c) {
        return fail(&e);
    }
    out(&format!("Sicherung:    {}\nWiederherst.: {}\nFernändern:   {}\n(ein laufender Agent übernimmt das binnen 30 s)\n", list_text(&c.roots), list_text(&c.restore_roots), if c.remote_roots { "erlaubt" } else { "gesperrt" }));
    0
}

fn listen_of(c: &Config) -> SocketAddr {
    plat::arg_value("--listen").or_else(|| Some(c.listen.clone()).filter(|l| !l.is_empty())).and_then(|l| SocketAddr::parse(&l)).unwrap_or(SocketAddr::new(IpAddr::UNSPECIFIED_V4, bc_proto::AGENT_PORT))
}

/// the TLS layer of the agent's port: only the server's client certificate gets through
fn tls_wrap(p: &config::Paired) -> Wrap {
    let cfg = tls::ServerConfig { identity: p.identity.clone(), client_auth: Some(Trust::Anchors(alloc::vec![p.ca.clone()])), client_auth_optional: false };
    Arc::new(move |tcp: TcpStream| {
        let s = tls::accept(Box::new(tcp), cfg.clone()).ok()?;
        let server = s.peer_certs().first().is_some_and(|c| c.subject_cn == SERVER_CN);
        server.then(|| (Box::new(s) as Box<dyn Stream>, Some(String::from("server"))))
    })
}

fn serve(dir: &str) -> i32 {
    *LOG.lock() = Some(Arc::new(bc_fs::Log::new(Some(&plat::fs::join(dir, "logs")), "agent")));
    let p = match config::load_paired(dir) {
        Ok(p) => p,
        Err(e) => return fail(&e),
    };
    let listen = listen_of(&p.cfg);
    let st = Arc::new(State {
        dir: String::from(dir),
        cfg: Mutex::new(p.cfg.clone()),
        changes: Mutex::new(api::ChangeLog::default()),
        changed: plat::sync::Condvar::new(),
        temps: Mutex::new(BTreeMap::new()),
    });
    // CDP: the backup folders are watched; the server asks for the changes
    let s3 = st.clone();
    let _ = plat::thread_spawn(128 * 1024, Box::new(move || watch(&s3)));
    let s2 = st.clone();
    let cfg = ServerConfig { threads: 6, max_body: bc_proto::agent::READ_MAX as usize + (64 << 10), ..ServerConfig::default() };
    let server = match Server::start_with(&listen, cfg, Arc::new(move |req| api::handle(&s2, req)), Some(tls_wrap(&p))) {
        Ok(s) => s,
        Err(e) => return fail(&format!("{} kann nicht geöffnet werden: {}", listen, e)),
    };
    log(&format!("{} bc-agent {} ({}) lauscht auf {}, Server {}", bc_proto::PRODUCT, bc_proto::VERSION, p.cfg.host_id, server.local_addr(), p.cfg.server));
    if p.cfg.roots.is_empty() {
        log("Noch keine Ordner freigegeben: in der Console unter Server & Agenten oder mit bc-agent roots --add DIR");
    }
    heartbeat(&st, &p);
    log("Der Agent hält an");
    server.stop();
    0
}

/// watch the backup folders (again when they change) and note the changes for the server
fn watch(st: &State) {
    let mut roots: Vec<String> = Vec::new();
    let mut w: Option<plat::watch::Watcher> = None;
    while !plat::service::stop_requested() {
        let now_roots = st.cfg.lock().roots.clone();
        if now_roots != roots || (w.is_none() && !now_roots.is_empty()) {
            roots = now_roots;
            w = None;
            if !roots.is_empty() {
                match plat::watch::Watcher::new(&roots) {
                    Ok(x) => w = Some(x),
                    Err(e) => log(&format!("Überwachung der Ordner: {}", e)),
                }
                // everything may have changed meanwhile
                api::note_changes(st, plat::watch::Changes { paths: Vec::new(), overflow: true });
            }
        }
        match &w {
            Some(x) => api::note_changes(st, x.take(1000)),
            None => plat::sleep_ms(1000),
        }
    }
}

/// report to the server every 30 s (and pick up changes of agent.json) until the service
/// manager asks the agent to stop
fn heartbeat(st: &State, p: &config::Paired) {
    let Some(url) = Url::parse(&format!("{}/", p.cfg.server)) else {
        log("ungültige Serveradresse in agent.json");
        while !plat::service::stop_requested() {
            plat::sleep_ms(1000);
        }
        return;
    };
    let mut client = Client::new().with_connector(pair::connector(p.ca.clone(), Some(p.identity.clone())));
    client.user_agent = format!("bc-agent/{}", bc_proto::VERSION);
    let mut session = Session::new(client, url);
    let mut online: Option<bool> = None;
    loop {
        // the file may have changed (bc-agent roots)
        if let Some(c) = config::load(&st.dir) {
            let mut cur = st.cfg.lock();
            if c.host_id == cur.host_id && (c.roots != cur.roots || c.restore_roots != cur.restore_roots || c.remote_roots != cur.remote_roots) {
                *cur = c;
                log("Freigaben aus agent.json übernommen");
            }
        }
        let hb = {
            let c = st.cfg.lock();
            Heartbeat { version: String::from(bc_proto::VERSION), name: c.name.clone(), os: os(), platform: platform(), address: c.address.clone(), roots: c.roots.clone(), restore_roots: c.restore_roots.clone() }
        };
        let r = session.request("POST", "/api/agent/heartbeat", &json_headers(""), j::text(&hb.to_json()).as_bytes(), 1 << 20);
        let mut wait = 30u32;
        let now_online = match &r {
            Ok(x) if x.status == 200 => {
                let v = j::parse(x.body_bytes()).unwrap_or(j::Value::Null);
                wait = v.get("interval").and_then(|i| i.as_u64()).unwrap_or(30).clamp(5, 3600) as u32;
                true
            }
            Ok(x) => {
                if online != Some(false) {
                    let m = j::parse(x.body_bytes()).map(|v| String::from(j::str_of(&v, "message"))).unwrap_or_default();
                    log(&format!("Server antwortet {} {}{}", x.status, m, if x.status == 401 || x.status == 404 { " – wurde dieser Agent auf dem Server entfernt?" } else { "" }));
                }
                false
            }
            Err(e) => {
                if online != Some(false) {
                    log(&format!("Server nicht erreichbar: {}", e));
                }
                false
            }
        };
        if now_online && online != Some(true) {
            log("Verbunden mit dem Server");
        }
        online = Some(now_online);
        for _ in 0..wait {
            if plat::service::stop_requested() {
                return;
            }
            plat::sleep_ms(1000);
        }
    }
}

rt::main!(run());
