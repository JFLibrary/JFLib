// sshd -- checks of netio::ssh: a small test server on the library's own transport (password
// and public key logins, exec with stdin and exit codes), the client against it, OpenSSH key
// files; and --ssh-serve for tools/ssh_interop.py, where OpenSSH's own client logs in
//
// Test commands of the server: "echo TEXT" (TEXT and a newline), "sha256" (hex digest and
// length of stdin), "fail" (stderr "boom", exit 3); anything else exits 127.  With --shell
// (Linux, for the SSH push test) every command runs as `sh -c COMMAND` instead.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use crypto::ed25519::Keypair;
use netio::ssh::{self, Client, HostKeyCheck, SshError, Transport, R, W};
use netio::Stream;
use plat::net::{IpAddr, SocketAddr, TcpListener, TcpStream};

use crate::T;

/// a key made by ssh-keygen -t ed25519 (a test vector, used nowhere else)
const KEY: &str = concat!(
    "-----BEGIN OPENSSH PRIVATE KEY-----\n",
    "b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\n",
    "QyNTUxOQAAACD60RnCg8OZDvdZHgvLvR7l2yKg+01chvq+oT2hdvRwLwAAAJDX5tuO1+bb\n",
    "jgAAAAtzc2gtZWQyNTUxOQAAACD60RnCg8OZDvdZHgvLvR7l2yKg+01chvq+oT2hdvRwLw\n",
    "AAAECSjwexJOO9WJ/WNjkFXHTz2Mv6RDbEsp8c43ZxXauvKPrRGcKDw5kO91keC8u9HuXb\n",
    "IqD7TVyG+r6hPaF29HAvAAAACGNvcmV0ZXN0AQIDBAU=\n",
    "-----END OPENSSH PRIVATE KEY-----\n",
);
const KEY_PUB: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPrRGcKDw5kO91keC8u9HuXbIqD7TVyG+r6hPaF29HAv coretest";
const KEY_FP: &str = "SHA256:qH4NJXwAF64ckBxSLD7cVnVAUSfBFgrxuoJZ/uH8z+o";

pub const USER: &str = "bc";
pub const PASSWORD: &str = "geheim-123";

/// who may log in: the password above, or one of these public key blobs
pub struct Logins {
    pub keys: Vec<Vec<u8>>,
}

fn blob_of(line: &str) -> Option<Vec<u8>> {
    let mut f = line.split_whitespace();
    (f.next()? == "ssh-ed25519").then_some(())?;
    crypto::b64::decode(f.next()?)
}

/// --shell: commands run in sh
static SHELL: AtomicBool = AtomicBool::new(false);

/// `sh -c cmd` with stdin -> stdout, stderr, exit code
fn shell(cmd: &str, stdin: &[u8]) -> (Vec<u8>, Vec<u8>, u32) {
    let c = plat::process::Command { program: "sh", args: &["-c", cmd], cwd: None, env: &[] };
    let Ok(mut ch) = c.spawn() else { return (Vec::new(), b"sh: cannot start\n".to_vec(), 127) };
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let (mut sent, mut out_eof, mut err_eof) = (0usize, false, false);
    let mut buf = [0u8; 65536];
    loop {
        let mut busy = false;
        if sent < stdin.len() {
            match ch.write_stdin(&stdin[sent..]) {
                Ok(Some(n)) => {
                    sent += n;
                    busy = true;
                }
                Ok(None) => {}
                Err(_) => sent = stdin.len(),
            }
            if sent == stdin.len() {
                ch.close_stdin();
            }
        } else {
            ch.close_stdin();
        }
        if !out_eof {
            match ch.read_stdout(&mut buf) {
                Ok(Some(0)) | Err(_) => out_eof = true,
                Ok(Some(n)) => {
                    out.extend_from_slice(&buf[..n]);
                    busy = true;
                }
                Ok(None) => {}
            }
        }
        if !err_eof {
            match ch.read_stderr(&mut buf) {
                Ok(Some(0)) | Err(_) => err_eof = true,
                Ok(Some(n)) => {
                    err.extend_from_slice(&buf[..n]);
                    busy = true;
                }
                Ok(None) => {}
            }
        }
        if out_eof && err_eof {
            if let Ok(Some(code)) = ch.try_wait() {
                return (out, err, code as u32);
            }
        }
        if !busy {
            plat::sleep_ms(2);
        }
    }
}

/// the test commands
fn run_command(cmd: &str, stdin: &[u8]) -> (Vec<u8>, Vec<u8>, u32) {
    if SHELL.load(Ordering::Relaxed) {
        return shell(cmd, stdin);
    }
    if let Some(t) = cmd.strip_prefix("echo ") {
        return (format!("{}\n", t).into_bytes(), Vec::new(), 0);
    }
    match cmd {
        "sha256" => (format!("{} {}\n", crypto::hex(&crypto::sha256(stdin)), stdin.len()).into_bytes(), Vec::new(), 0),
        "fail" => (Vec::new(), b"boom\n".to_vec(), 3),
        _ => (Vec::new(), format!("{}: not found\n", cmd).into_bytes(), 127),
    }
}

/// one connection of the test server
pub fn serve_conn(s: Box<dyn Stream>, host: &Keypair, logins: &Logins) -> Result<(), SshError> {
    let mut t = Transport::accept(s, host)?;
    let mut authed = false;
    // channel state: (remote id, stdin so far, command)
    let mut chan: Option<(u32, Vec<u8>, String)> = None;
    let mut consumed = 0u32;
    loop {
        let m = match t.recv() {
            Ok(m) => m,
            Err(SshError::Disconnected(_)) => return Ok(()),
            Err(e) => return Err(e),
        };
        let mut r = R::new(&m[1..]);
        match m[0] {
            ssh::MSG_SERVICE_REQUEST => {
                let name = r.str().unwrap_or("");
                t.send(&W::msg(ssh::MSG_SERVICE_ACCEPT).str(name).0)?;
            }
            ssh::MSG_USERAUTH_REQUEST => {
                let user = r.str().unwrap_or("");
                let _service = r.str();
                let method = r.str().unwrap_or("");
                let ok = match method {
                    "password" => {
                        r.bool();
                        user == USER && r.str() == Some(PASSWORD)
                    }
                    "publickey" => {
                        let signed = r.bool().unwrap_or(false);
                        let alg = r.str().unwrap_or("");
                        let blob = r.bytes().unwrap_or(&[]).to_vec();
                        let known = user == USER && alg == "ssh-ed25519" && logins.keys.iter().any(|k| *k == blob);
                        if !signed {
                            // a query: may this key log in?
                            if known {
                                t.send(&W::msg(ssh::MSG_USERAUTH_PK_OK).str(alg).bytes(&blob).0)?;
                                continue;
                            }
                            false
                        } else {
                            let sig = r.bytes().unwrap_or(&[]);
                            let data = W::default().bytes(&t.session_id).u8(ssh::MSG_USERAUTH_REQUEST).str(user).str("ssh-connection").str("publickey").bool(true).str(alg).bytes(&blob).0;
                            let mut sr = R::new(sig);
                            known
                                && sr.str() == Some("ssh-ed25519")
                                && sr.bytes().filter(|s| s.len() == 64).is_some_and(|s| {
                                    let mut sg = [0u8; 64];
                                    sg.copy_from_slice(s);
                                    let mut pk = [0u8; 32];
                                    pk.copy_from_slice(&blob[blob.len() - 32..]);
                                    crypto::ed25519::verify(&pk, &data, &sg)
                                })
                        }
                    }
                    _ => false,
                };
                if ok {
                    authed = true;
                    t.send(&[ssh::MSG_USERAUTH_SUCCESS])?;
                } else {
                    t.send(&W::msg(ssh::MSG_USERAUTH_FAILURE).str("publickey,password").bool(false).0)?;
                }
            }
            _ if !authed => return Err(SshError::Protocol(String::from("not logged in"))),
            ssh::MSG_CHANNEL_OPEN => {
                let _kind = r.str();
                let remote = r.u32().unwrap_or(0);
                chan = Some((remote, Vec::new(), String::new()));
                t.send(&W::msg(ssh::MSG_CHANNEL_OPEN_CONFIRMATION).u32(remote).u32(0).u32(2 << 20).u32(32768).0)?;
            }
            ssh::MSG_CHANNEL_REQUEST => {
                r.u32();
                let name = r.str().unwrap_or("");
                let want = r.bool().unwrap_or(false);
                let Some(c) = chan.as_mut() else { continue };
                if name == "exec" {
                    c.2 = String::from(r.str().unwrap_or(""));
                    if want {
                        t.send(&W::msg(ssh::MSG_CHANNEL_SUCCESS).u32(c.0).0)?;
                    }
                } else if want {
                    t.send(&W::msg(ssh::MSG_CHANNEL_FAILURE).u32(c.0).0)?;
                }
            }
            ssh::MSG_CHANNEL_DATA => {
                r.u32();
                let d = r.bytes().unwrap_or(&[]);
                if let Some(c) = chan.as_mut() {
                    c.1.extend_from_slice(d);
                    consumed += d.len() as u32;
                    if consumed >= 1 << 20 {
                        t.send(&W::msg(ssh::MSG_CHANNEL_WINDOW_ADJUST).u32(c.0).u32(consumed).0)?;
                        consumed = 0;
                    }
                }
            }
            ssh::MSG_CHANNEL_EOF => {
                let Some((remote, stdin, cmd)) = chan.take() else { continue };
                let (out, err, code) = run_command(&cmd, &stdin);
                for part in out.chunks(32768) {
                    t.send(&W::msg(ssh::MSG_CHANNEL_DATA).u32(remote).bytes(part).0)?;
                }
                for part in err.chunks(32768) {
                    t.send(&W::msg(ssh::MSG_CHANNEL_EXTENDED_DATA).u32(remote).u32(1).bytes(part).0)?;
                }
                t.send(&W::msg(ssh::MSG_CHANNEL_REQUEST).u32(remote).str("exit-status").bool(false).u32(code).0)?;
                t.send(&W::msg(ssh::MSG_CHANNEL_EOF).u32(remote).0)?;
                t.send(&W::msg(ssh::MSG_CHANNEL_CLOSE).u32(remote).0)?;
                consumed = 0;
            }
            ssh::MSG_GLOBAL_REQUEST => {
                r.str();
                if r.bool() == Some(true) {
                    t.send(&[ssh::MSG_REQUEST_FAILURE])?;
                }
            }
            _ => {}
        }
    }
}

fn host_key() -> Keypair {
    Keypair::from_seed(&crypto::sha256(b"coretest ssh host key"))
}

/// a test server on a free loopback port for `n` connections -> its port
fn spawn(n: usize, logins: Arc<Logins>) -> u16 {
    let Ok(l) = TcpListener::bind(&SocketAddr::new(IpAddr::LOCALHOST_V4, 0)) else { return 0 };
    let port = l.local_addr().map_or(0, |a| a.port);
    let _ = plat::thread_spawn(256 * 1024, Box::new(move || {
        for _ in 0..n {
            let Ok(Some((s, _))) = l.accept_timeout(10_000) else { return };
            s.set_read_timeout(Some(10_000));
            let _ = serve_conn(Box::new(s), &host_key(), &logins);
        }
    }));
    port
}

fn connect(port: u16, check: HostKeyCheck) -> Result<Client, SshError> {
    let s = TcpStream::connect(&SocketAddr::new(IpAddr::LOCALHOST_V4, port), 3000)?;
    s.set_read_timeout(Some(10_000));
    Client::connect(Box::new(s), check)
}

pub fn run(t: &mut T) {
    // key files
    let k = ssh::private_key(KEY);
    t.check("OpenSSH private key read", k.is_some());
    let Some(k) = k else { return };
    t.eq("public key line", ssh::public_key_line(&k, "coretest"), String::from(KEY_PUB));
    t.eq("fingerprint like ssh-keygen -l", ssh::fingerprint(&blob_of(KEY_PUB).unwrap_or_default()), String::from(KEY_FP));
    t.check("broken key refused", ssh::private_key(&KEY.replace("IqD7TVyG", "IqD7TVyH")).is_none());
    let enc = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABA=\n-----END OPENSSH PRIVATE KEY-----\n";
    t.check("encrypted key not read", ssh::private_key(enc).is_none());

    let logins = Arc::new(Logins { keys: alloc::vec![blob_of(KEY_PUB).unwrap_or_default()] });
    let port = spawn(6, logins);
    t.check("test server", port != 0);
    let host_fp = ssh::fingerprint(&W::default().str("ssh-ed25519").bytes(&host_key().public).0);

    // 1: learn the key, wrong password, password, commands
    match connect(port, HostKeyCheck::Any) {
        Ok(mut c) => {
            t.eq("host key fingerprint", c.host_key().fingerprint.clone(), host_fp.clone());
            t.eq("wrong password", c.auth_password(USER, "falsch").err(), Some(SshError::Auth));
            t.check("password login", c.auth_password(USER, PASSWORD).is_ok());
            match c.exec("echo hallo welt", b"", 1 << 20) {
                Ok(x) => {
                    t.eq("echo stdout", x.stdout, b"hallo welt\n".to_vec());
                    t.eq("echo status", x.status, Some(0));
                }
                Err(e) => t.check(&format!("echo: {}", e), false),
            }
            let big: Vec<u8> = (0..(3u32 << 20) + 17).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
            match c.exec("sha256", &big, 1 << 20) {
                Ok(x) => t.eq("3 MiB through stdin (windows, chunks)", String::from_utf8_lossy(&x.stdout).into_owned(), format!("{} {}\n", crypto::hex(&crypto::sha256(&big)), big.len())),
                Err(e) => t.check(&format!("sha256: {}", e), false),
            }
            match c.exec("fail", b"", 1 << 20) {
                Ok(x) => {
                    t.eq("exit code", x.status, Some(3));
                    t.eq("stderr", x.stderr, b"boom\n".to_vec());
                }
                Err(e) => t.check(&format!("fail: {}", e), false),
            }
            c.close();
        }
        Err(e) => t.check(&format!("connect: {}", e), false),
    }
    // 2: a pinned key that does not match
    match connect(port, HostKeyCheck::Pinned(String::from("SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"))) {
        Err(SshError::HostKey(fp)) => t.eq("wrong pin refused, key shown", fp, host_fp.clone()),
        Err(e) => t.check(&format!("wrong pin: {}", e), false),
        Ok(_) => t.check("wrong pin refused", false),
    }
    // 3: the pinned key and a public key login
    match connect(port, HostKeyCheck::Pinned(host_fp.clone())) {
        Ok(mut c) => {
            t.check("key login", c.auth_key(USER, &k).is_ok());
            t.eq("after key login", c.exec("echo ok", b"", 64).map(|x| x.stdout).unwrap_or_default(), b"ok\n".to_vec());
            c.close();
        }
        Err(e) => t.check(&format!("pinned: {}", e), false),
    }
    // 4: an unknown key
    match connect(port, HostKeyCheck::Pinned(host_fp)) {
        Ok(mut c) => {
            let other = Keypair::from_seed(&[7u8; 32]);
            t.eq("unknown key refused", c.auth_key(USER, &other).err(), Some(SshError::Auth));
            c.close();
        }
        Err(e) => t.check(&format!("unknown key: {}", e), false),
    }
}

/// --ssh-serve PORT [--authorized FILE] [--hostkey-out FILE] [--shell]: the test server for
/// OpenSSH's client (and, with --shell, for the SSH push of bc-server)
pub fn serve(port: u16) -> i32 {
    SHELL.store(plat::args().iter().any(|a| a == "--shell") && cfg!(target_os = "linux"), Ordering::Relaxed);
    let mut keys = Vec::new();
    if let Some(f) = plat::arg_value("--authorized") {
        for line in String::from_utf8_lossy(&plat::read_file(&f).unwrap_or_default()).lines() {
            if let Some(b) = blob_of(line) {
                keys.push(b);
            }
        }
    }
    if let Some(f) = plat::arg_value("--hostkey-out") {
        plat::write_file(&f, format!("{}\n", ssh::public_key_line(&host_key(), "coretest-host")).as_bytes());
    }
    let logins = Arc::new(Logins { keys });
    let Ok(l) = TcpListener::bind(&SocketAddr::new(IpAddr::LOCALHOST_V4, port)) else { return 1 };
    plat::write_stdout(b"ready\n");
    loop {
        let Ok(Some((s, _))) = l.accept_timeout(60_000) else { return 0 };
        let lg = logins.clone();
        let _ = plat::thread_spawn(256 * 1024, Box::new(move || {
            s.set_read_timeout(Some(30_000));
            if let Err(e) = serve_conn(Box::new(s), &host_key(), &lg) {
                plat::write_stderr(format!("ssh: {}\n", e).as_bytes());
            }
        }));
    }
}

/// --ssh-key FILE: the public line of an OpenSSH private key file (or exit 1)
pub fn key_line(file: &str) -> i32 {
    match plat::read_file(file).and_then(|b| ssh::private_key(&String::from_utf8_lossy(&b))) {
        Some(k) => {
            plat::write_stdout(format!("{}\n", ssh::public_key_line(&k, "")).as_bytes());
            0
        }
        None => 1,
    }
}
