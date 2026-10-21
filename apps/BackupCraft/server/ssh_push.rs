// ssh_push -- install an agent on a Linux machine over SSH and pair it
//
//   fingerprint  POST /api/hosts/ssh-fingerprint {address, port}: the machine's host key, for
//                the administrator to compare before anything is sent
//   push         POST /api/hosts/ssh-push (operation): connect, check exactly that host key, log
//                in (stored credentials: password or Ed25519 key; or a password given once),
//                upload bc-agent, pair it with a fresh one-time code (the CA fingerprint is
//                passed along, so the agent checks this server), start it: as root a systemd
//                service "backupcraft-agent", else a background process of that user
//
// The agent binary for Linux comes from <bc-server's folder>/agents/linux-x86_64/bc-agent,
// --agents DIR, the server's own folder (a Linux server) or ../linux/bc-agent (a build tree).
// Windows machines pair with a code (bc-agent pair) and run the agent as a service with
// bc-agent service install.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::api::{code, SshFingerprint, SshPushRequest};
use bc_proto::j::Json;
use bc_proto::model::CredKind;
use netio::http::Response;
use netio::ssh::{self, Client, HostKeyCheck, SshError};
use plat::net::TcpStream;

use crate::api::Call;
use crate::ctx::Ctx;
use crate::ops::Ops;
use crate::util::{self, bad, err, field, ok};

const CONNECT_MS: u32 = 8000;

fn connect(address: &str, port: u32, check: HostKeyCheck) -> Result<Client, String> {
    let port = if port == 0 { 22 } else { port.min(65535) as u16 };
    let addrs = netio::dns::resolve(address, port).map_err(|e| format!("{}: {}", address, e))?;
    let tcp = TcpStream::connect_any(&addrs, CONNECT_MS).map_err(|e| format!("{}:{} nicht erreichbar: {}", address, port, e))?;
    tcp.set_read_timeout(Some(120_000));
    tcp.set_write_timeout(Some(120_000));
    Client::connect(Box::new(tcp), check).map_err(|e| match e {
        SshError::HostKey(fp) => format!("Der Host-Schlüssel hat sich geändert ({}) – Abbruch", fp),
        e => format!("{}", e),
    })
}

pub fn fingerprint(_ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = SshFingerprint::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    let address = r.address.trim();
    if address.is_empty() {
        return field(code::BAD_REQUEST, "Bitte eine Adresse angeben", "address");
    }
    match connect(address, r.port, HostKeyCheck::Any) {
        Ok(cl) => {
            let hk = cl.host_key().clone();
            cl.close();
            ok(200, &SshFingerprint { address: String::from(address), port: if r.port == 0 { 22 } else { r.port }, fingerprint: hk.fingerprint, key_type: hk.key_type })
        }
        Err(e) => field(code::CONFLICT, &e, "address"),
    }
}

/// 'text' for the remote shell
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// the bc-agent binary for Linux (x86-64)
fn linux_agent() -> Option<(String, Vec<u8>)> {
    let exe = plat::current_exe()?;
    let dir = String::from(plat::fs::parent(&exe));
    let mut tries: Vec<String> = Vec::new();
    if let Some(d) = plat::arg_value("--agents") {
        tries.push(plat::fs::join(&plat::fs::join(&d, "linux-x86_64"), "bc-agent"));
        tries.push(plat::fs::join(&d, "bc-agent"));
    }
    tries.push(plat::fs::join(&plat::fs::join(&plat::fs::join(&dir, "agents"), "linux-x86_64"), "bc-agent"));
    if cfg!(target_os = "linux") {
        tries.push(plat::fs::join(&dir, "bc-agent"));
    }
    tries.push(plat::fs::join(&plat::fs::join(plat::fs::parent(&dir), "linux"), "bc-agent"));
    tries.into_iter().find_map(|p| {
        let b = plat::read_file(&p)?;
        // an ELF executable for x86-64
        (b.len() > 64 && b.starts_with(b"\x7fELF") && b[4] == 2 && u16::from_le_bytes([b[18], b[19]]) == 62).then_some((p, b))
    })
}

/// how to log in: a stored credential or what the request carries
enum Login {
    Password(String, String),
    Key(String, crypto::ed25519::Keypair),
}

fn login_of(ctx: &Ctx, r: &SshPushRequest) -> Result<Login, Response> {
    if !r.credential_id.is_empty() {
        let Some((cr, secret)) = crate::admin::credential_secret(ctx, &r.credential_id) else {
            return Err(field(code::BAD_REQUEST, "Zugangsdaten nicht gefunden", "credential_id"));
        };
        let user = if r.username.trim().is_empty() { cr.username.clone() } else { String::from(r.username.trim()) };
        return match cr.kind {
            CredKind::Password => Ok(Login::Password(user, secret)),
            CredKind::SshKey => match ssh::private_key(&secret) {
                Some(k) => Ok(Login::Key(user, k)),
                None => Err(field(code::BAD_REQUEST, "Der Schlüssel ist kein unverschlüsselter Ed25519-Schlüssel im OpenSSH-Format", "credential_id")),
            },
        };
    }
    if r.username.trim().is_empty() || r.password.is_empty() {
        return Err(field(code::BAD_REQUEST, "Bitte Zugangsdaten wählen oder Benutzer und Passwort angeben", "credential_id"));
    }
    Ok(Login::Password(String::from(r.username.trim()), r.password.clone()))
}

/// one remote command; a failure carries its output
fn run(cl: &mut Client, what: &str, cmd: &str, stdin: &[u8]) -> Result<String, String> {
    let x = cl.exec(cmd, stdin, 1 << 20).map_err(|e| format!("{}: {}", what, e))?;
    let out = String::from_utf8_lossy(&x.stdout).into_owned();
    if x.status != Some(0) {
        let err = String::from_utf8_lossy(&x.stderr);
        let detail = if err.trim().is_empty() { out.trim() } else { err.trim() };
        return Err(format!("{} fehlgeschlagen (Code {}): {}", what, x.status.map_or(String::from("?"), |s| format!("{}", s)), detail.chars().take(400).collect::<String>()));
    }
    Ok(out)
}

pub fn push(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let Some(r) = SshPushRequest::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    if !ctx.tls {
        return err(409, code::CONFLICT, "Agenten koppeln sich nur über eine verschlüsselte Verbindung (Server mit --listen auf einer Netzwerkadresse oder --tls starten)");
    }
    let address = String::from(r.address.trim());
    if address.is_empty() {
        return field(code::BAD_REQUEST, "Bitte eine Adresse angeben", "address");
    }
    if !r.host_fingerprint.starts_with("SHA256:") {
        return field(code::BAD_REQUEST, "Bitte zuerst den Host-Schlüssel prüfen", "host_fingerprint");
    }
    let login = match login_of(ctx, &r) {
        Ok(l) => l,
        Err(resp) => return resp,
    };
    let Some((_, agent)) = linux_agent() else {
        return err(409, code::CONFLICT, "bc-agent für Linux liegt nicht beim Server (agents/linux-x86_64/bc-agent oder --agents DIR)");
    };
    let roots = match bc_fs::clean_roots(&r.roots, false) {
        Ok(v) => v,
        Err(e) => return field(code::BAD_REQUEST, &e, "roots"),
    };
    let name = if r.name.trim().is_empty() { address.clone() } else { String::from(r.name.trim()) };
    let actor = c.actor();
    let title = format!("Agent auf {} installieren", address);
    let (port, fp) = (r.port, r.host_fingerprint.clone());
    let target = address.clone();
    let op = Ops::start(ctx, "host.ssh_push", &title, &actor, &target, "", Box::new(move |o| {
        let ctx = o.ctx().clone();
        let total = 5u64;
        o.progress(0, total);
        let mut cl = connect(&address, port, HostKeyCheck::Pinned(fp.clone()))?;
        let auth = match &login {
            Login::Password(u, p) => cl.auth_password(u, p),
            Login::Key(u, k) => cl.auth_key(u, k),
        };
        auth.map_err(|e| if e == SshError::Auth { String::from("Anmeldung abgelehnt (Benutzer, Passwort oder Schlüssel)") } else { format!("{}", e) })?;
        o.progress(1, total);
        let os = run(&mut cl, "Systemabfrage", "uname -sm", b"").unwrap_or_default();
        if !os.starts_with("Linux") {
            return Err(format!("{} ist kein Linux-Rechner ({}) – Windows-Rechner bitte mit einem Pairing-Code koppeln", address, os.trim()));
        }
        if !os.contains("x86_64") {
            return Err(format!("{}: nur x86-64 wird unterstützt ({})", address, os.trim()));
        }
        if o.cancelled() {
            return Err(String::from("Abgebrochen"));
        }
        // the binary: as root to /opt, else below the user's home
        let place = concat!(
            "set -e; umask 022; ",
            "if [ \"$(id -u)\" = 0 ]; then d=/opt/backupcraft-agent; data=/var/lib/backupcraft-agent; ",
            "else d=\"$HOME/.local/share/backupcraft/agent-bin\"; data=\"$HOME/.local/share/backupcraft/agent\"; fi; ",
            "mkdir -p \"$d\"; cat > \"$d/bc-agent.new\"; chmod 755 \"$d/bc-agent.new\"; \"$d/bc-agent.new\" --version >/dev/null; ",
            "mv -f \"$d/bc-agent.new\" \"$d/bc-agent\"; echo \"$d|$data|$(id -u)\"",
        );
        let where_ = run(&mut cl, "Übertragen", place, &agent)?;
        let mut parts = where_.trim().split('|');
        let (bin_dir, data, uid) = (String::from(parts.next().unwrap_or("")), String::from(parts.next().unwrap_or("")), String::from(parts.next().unwrap_or("")));
        if bin_dir.is_empty() || data.is_empty() {
            return Err(String::from("Übertragen: unerwartete Antwort"));
        }
        let bin = format!("{}/bc-agent", bin_dir);
        o.progress(2, total);
        // an agent running already is stopped (the pattern does not match the shell running it)
        let stop = format!("(systemctl stop backupcraft-agent 2>/dev/null; pkill -f {} 2>/dev/null; true)", sq(&format!("{}/[b]c-agent run", bin_dir)));
        // a machine paired before keeps its identity: only the program is new
        let check = format!("{}; if {} status --data {} >/dev/null 2>&1; then echo paired; else echo new; fi", stop, sq(&bin), sq(&data));
        let paired = run(&mut cl, "Prüfen", &check, b"")?.trim() == "paired";
        if !paired {
            let pc = crate::hosts::new_pairing(&ctx, "ssh-push").map_err(|_| String::from("Pairing-Code konnte nicht angelegt werden"))?;
            let mut pair = format!("{} pair --server {} --code {} --fingerprint {} --name {} --data {} --force --yes", sq(&bin), sq(&pc.server_url), sq(&pc.code), sq(&pc.ca_fingerprint), sq(&name), sq(&data));
            for root in &roots {
                pair.push_str(&format!(" --root {}", sq(root)));
            }
            run(&mut cl, "Koppeln", &pair, b"")?;
        }
        o.progress(3, total);
        // start: a systemd service as root, else a background process of the user
        let started = if uid == "0" {
            // the agent sets up its own systemd unit (bc-agent service install)
            let svc = format!(
                "set -e; if [ -d /run/systemd/system ]; then if [ -f /etc/systemd/system/backupcraft-agent.service ]; then systemctl restart backupcraft-agent; else {} service install --data {} >/dev/null; fi; echo systemd; else echo none; fi",
                sq(&bin), sq(&data)
            );
            run(&mut cl, "Dienst einrichten", &svc, b"")?
        } else {
            String::from("none")
        };
        let how = if started.trim() == "systemd" {
            String::from("als systemd-Dienst backupcraft-agent")
        } else {
            // its own session (the SSH session's end does not reach it); still running a moment
            // later, else its log tells why
            let log = sq(&format!("{}/agent.log", data));
            let bg = format!(
                "cd / && (setsid nohup {} run --data {} > {} 2>&1 < /dev/null &) ; sleep 2; if pgrep -f {} >/dev/null; then echo started; else tail -n 5 {} >&2; exit 1; fi",
                sq(&bin), sq(&data), log, sq(&format!("{}/[b]c-agent run", bin_dir)), log
            );
            run(&mut cl, "Starten", &bg, b"")?;
            String::from("als Hintergrundprozess (ohne systemd-Dienst: nach einem Neustart erneut verteilen oder als root installieren)")
        };
        cl.close();
        o.progress(4, total);
        // the host, and the server reaches its agent
        let deadline = util::now() + 30;
        let mut reached = None;
        while util::now() < deadline && reached.is_none() {
            plat::sleep_ms(500);
            let mut hosts: Vec<bc_proto::model::Host> = util::all::<bc_proto::model::Host>(&ctx.db, crate::ctx::C_HOSTS).into_iter().filter(|h| h.name == name && h.kind == bc_proto::model::HostKind::Agent).collect();
            hosts.sort_by_key(|h| -h.created);
            if let Some(h) = hosts.into_iter().next() {
                if crate::agents::info(&ctx, &h).is_ok() {
                    reached = Some(h);
                }
            }
        }
        o.progress(5, total);
        let did = if paired { "aktualisiert und neu gestartet" } else { "installiert, gekoppelt und gestartet" };
        match reached {
            Some(h) => Ok(format!("{}: Agent {} {}; der Server erreicht ihn", h.name, did, how)),
            None => Ok(format!("{}: Agent {} {} – der Server erreicht ihn noch nicht (Firewall, Port 7680?)", name, did, how)),
        }
    }));
    c.audit("host.ssh_push", &r.address, &format!("Host-Schlüssel {}", r.host_fingerprint));
    util::accepted(&op)
}
