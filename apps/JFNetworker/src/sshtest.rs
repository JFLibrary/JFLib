// sshtest -- checks of SSH in the simulator (group "ssh" of --selftest: jfn_sim::sshd, sshtrace,
// cli/ssh.rs): a PC logs into an IOS-like router with a local user and with RADIUS, transport input
// refuses telnet or SSH, show ip ssh, show ssh, show users, wrong passwords and the tries, the time
// for a login, exec mode with an exit status, the IOS-like client, RSA and Ed25519 host keys,
// "debug ip ssh", the running configuration, a key through a reload; on a Linux host: sshd with
// a password and with a key from ssh-keygen and authorized_keys, known_hosts asking, remembering
// and refusing a changed key, another user, who and w, Ctrl+C and "~.", scp both ways; frames on the
// wire (identification, KEXINIT, then encrypted); a replay of the inputs (K1) and determinism
//
// The LAN (device ids in order): R1 (JF-R4, Gi0/0/0 10.0.0.1/24), SW1 (switch), SRV1 (server-1u,
// 10.0.0.10: the RADIUS server and sshd with root's password "geheim" and the user deploy), PC1
// (PC, 10.0.0.20).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::{Action, Cmd, Engine, Inline, Note, Report, SesKind, SshNote, MS, SEC};
use netpkt::decode;

use crate::selftest::T;

const BUDGET: usize = 4_000_000;
const R1: u32 = 1;
const SW1: u32 = 2;
const SRV1: u32 = 3;
const PC1: u32 = 4;

fn s(x: &str) -> String {
    String::from(x)
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: s(kind), name: s(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Value::Null) }
}

const RADIUS_CFG: &str = r#"{"clients": [{"address": "10.0.0.0/24", "secret": "testing123"}],
"users": [{"name": "admin", "password": "adminpw", "privilege": 15},
{"name": "bob", "password": "bobpw", "privilege": 1}]}"#;

const SSHD_CFG: &str = r#"{"password": "geheim", "users": [{"name": "deploy", "password": "dpw"}]}"#;

/// the LAN; `r1` is R1's startup configuration
fn project(r1: &str) -> Project {
    let mut p = Project::new("ssh", 13);
    for (i, m) in ["jf-r4", "jf-s24p", "server-1u", "pc"].iter().enumerate() {
        let _ = p.add_device(m, i as i32 * 120, 0);
    }
    let pr = |dev: u32, port: &str| PortRef { dev, port: s(port) };
    let _ = p.connect(&pr(R1, "gi0/0/0"), &pr(SW1, "gi1/0/1"), None);
    let _ = p.connect(&pr(SRV1, "eth0"), &pr(SW1, "gi1/0/2"), None);
    let _ = p.connect(&pr(PC1, "eth0"), &pr(SW1, "gi1/0/3"), None);
    let cfg = [
        (R1, "R1", s(r1)),
        (SW1, "SW1", s("hostname SW1\n!\nend\n")),
        (SRV1, "SRV1", s("hostname SRV1\nip addr add 10.0.0.10/24 dev eth0\n")),
        (PC1, "PC1", s("hostname PC1\nip addr add 10.0.0.20/24 dev eth0\n")),
    ];
    for (id, name, c) in cfg {
        if let Some(d) = p.device_mut(id) {
            d.name = s(name);
            d.startup_config = c;
        }
    }
    if let Some(d) = p.device_mut(SRV1) {
        d.services = alloc::vec![def("radius-server", "radius", RADIUS_CFG), def("sshd", "sshd", SSHD_CFG)];
    }
    p
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn run_for(e: &mut Engine, d: u64) {
    let t = e.now() + d;
    e.run_until(t, BUDGET);
}

/// lines typed at a device's console, each followed by `d` of virtual time (no line: `d` only):
/// what it printed
fn cli(e: &mut Engine, dev: u32, lines: &[&str], d: u64) -> String {
    let _ = e.take_reports();
    let mut o = String::new();
    let mut collect = |e: &mut Engine| {
        for r in e.take_reports() {
            if let Report::CliOut { dev: x, bytes, .. } = r
                && x == dev
            {
                o.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
    };
    if lines.is_empty() {
        run_for(e, d);
        collect(e);
    }
    for l in lines {
        let _ = e.cli_line(dev, l);
        run_for(e, d);
        collect(e);
    }
    o
}

/// the SSH notes a device made
fn notes(e: &Engine, dev: u32) -> Vec<SshNote> {
    e.trace.events().filter(|x| x.dev == dev).filter_map(|x| if let Note::Ssh(n) = &x.note { Some(n.clone()) } else { None }).collect()
}

fn has<F: Fn(&SshNote) -> bool>(e: &Engine, dev: u32, f: F) -> bool {
    notes(e, dev).iter().any(f)
}

fn net_sessions(e: &Engine, dev: u32) -> usize {
    e.cli_sessions(dev).iter().filter(|x| x.kind == SesKind::Telnet).count()
}

// R1 with an address, a domain and a key
fn r1_with(lines: &str) -> String {
    format!("hostname R1\n!\nip domain-name lab.test\ncrypto key generate ed25519\n!\ninterface GigabitEthernet0/0/0\n ip address 10.0.0.1 255.255.255.0\n no shutdown\n!\n{}!\nend\n", lines)
}

const R1_LOCAL: &str = "username admin privilege 15 secret adminpw\nusername bob password bobpw\nline vty 0 4\n login local\n transport input ssh\n";
const R1_RADIUS: &str = "aaa new-model\nradius server S1\n address ipv4 10.0.0.10\n key testing123\n!\naaa authentication login default group radius local\naaa authorization exec default group radius\nusername local1 secret localpw\nline vty 0 4\n login authentication default\n transport input ssh\n";

// a connection without questions about the host key
const NOASK: &str = "-o StrictHostKeyChecking=no";

pub(crate) fn run(t: &mut T) {
    server_local(t);
    server_radius(t);
    transport(t);
    tries_and_time(t);
    exec_ios(t);
    ios_client(t);
    keys_and_config(t);
    linux_password(t);
    linux_key(t);
    known_hosts(t);
    linux_session(t);
    scp(t);
    wire(t);
    replay(t);
    determinism(t);
}

/// PC1 into R1 with a local user: the question about the key, the password, the session
fn server_local(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_LOCAL)));
    run_for(&mut e, SEC);
    t.check("the key is made at boot, quietly", e.take_reports().iter().all(|r| !matches!(r, Report::Files { dev: R1, .. })));
    let o = cli(&mut e, R1, &["show ip ssh"], 0);
    t.check("show ip ssh: on", o.contains("SSH server: on, version 2.0") && o.contains("Time for a login: 120 s, tries: 3") && o.contains("ssh-ed25519, 256 bits, SHA256:"));
    t.eq("no session yet", cli(&mut e, R1, &["show ssh"], 0), s("No SSH sessions.\n"));
    let o = cli(&mut e, PC1, &["ssh -l admin 10.0.0.1"], SEC);
    t.check("the first connection asks", o.contains("10.0.0.1 is new here") && o.contains("SHA256:") && o.ends_with("Type yes or no: "));
    let o = cli(&mut e, PC1, &["vielleicht"], SEC);
    t.check("it asks until it hears yes or no", o == "Type yes or no: ");
    let o = cli(&mut e, PC1, &["yes"], SEC);
    t.check("then the password", o.contains("Remembered 10.0.0.1") && o.ends_with("Password for admin at 10.0.0.1: "));
    t.check("the password is hidden", e.cli_secret(PC1));
    let kh = cli(&mut e, PC1, &[], 0);
    t.eq("nothing else", kh, s(""));
    let o = cli(&mut e, PC1, &["adminpw"], SEC);
    t.check("the prompt of the router, privileged", o.ends_with("R1#"));
    t.check("no more hiding", !e.cli_secret(PC1));
    t.check("the vty line is taken", net_sessions(&e, R1) == 1);
    let v = e.cli_sessions(R1);
    t.check("its user and where from", v.iter().any(|x| x.line == "vty 0" && x.user == "admin" && x.from == netpkt::IpAddr::parse("10.0.0.20")));
    let o = cli(&mut e, PC1, &["show users"], SEC);
    t.check("show users: ssh", o.contains("vty 0") && o.contains("admin") && o.contains("10.0.0.20 (ssh)") && o.lines().any(|l| l.starts_with('*')));
    let o = cli(&mut e, R1, &["show ssh"], 0);
    t.check("show ssh: the session", o.contains("vty 0") && o.contains("chacha20-poly1305@openssh.com") && o.contains("ssh-ed25519") && o.contains("session") && o.contains("admin"));
    let o = cli(&mut e, PC1, &["show ip interface brief"], SEC);
    t.check("a command with its output, CR LF gone", o.contains("GigabitEthernet0/0/0") && o.contains("10.0.0.1") && !o.contains('\r'));
    let o = cli(&mut e, PC1, &["configure terminal", "interface Loopback1"], SEC);
    t.check("configuration modes over the line", o.contains("R1(config)#") && o.contains("R1(config-if)#"));
    let _ = e.take_reports();
    let _ = e.cli_line(PC1, "end");
    run_for(&mut e, SEC);
    let console = e.take_reports().into_iter().any(|r| matches!(r, Report::CliOut { dev: R1, bytes, .. } if String::from_utf8_lossy(&bytes).contains("from vty 0 (10.0.0.20)")));
    t.check("the log says from where", console);
    let o = cli(&mut e, PC1, &["exit"], SEC);
    t.check("exit ends it", o.contains("Session with 10.0.0.1 ended."));
    t.eq("the prompt back", e.cli_prompt(PC1), s("root@PC1:~# "));
    t.eq("exit status 0", cli(&mut e, PC1, &["echo $?"], 0), s("0\n"));
    t.check("the line is free", net_sessions(&e, R1) == 0);
    t.check("notes of the server", has(&e, R1, |n| matches!(n, SshNote::Accept { line: Some(0), .. })) && has(&e, R1, |n| matches!(n, SshNote::Keyed { .. })) && has(&e, R1, |n| matches!(n, SshNote::LoginOk { .. })) && has(&e, R1, |n| matches!(n, SshNote::Session { .. })) && has(&e, R1, |n| matches!(n, SshNote::Closed { .. })));
    t.check("notes of the client", has(&e, PC1, |n| matches!(n, SshNote::Connected { port: 22, .. })) && has(&e, PC1, |n| matches!(n, SshNote::HostKeyNew { .. })) && has(&e, PC1, |n| matches!(n, SshNote::ClientLogin { .. })) && has(&e, PC1, |n| matches!(n, SshNote::ClientEnd { status: 0, .. })));
    // a user of level 1
    let o = cli(&mut e, PC1, &["ssh bob@10.0.0.1", "bobpw"], SEC);
    t.check("known now: no question, level 1", !o.contains("is new here") && o.ends_with("R1>"));
    let o = cli(&mut e, PC1, &["exit"], SEC);
    t.check("again out", o.contains("ended"));
    t.eq("two logins counted", e.metric(R1, "vty-ssh", "logins"), Some(2));
}

/// AAA with RADIUS through the same gate
fn server_radius(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_RADIUS)));
    run_for(&mut e, SEC);
    // (the router's first packet to the server waits for ARP and goes again after its timeout)
    let o = cli(&mut e, PC1, &[&format!("ssh {} admin@10.0.0.1", NOASK), "adminpw"], 8 * SEC);
    t.check("RADIUS lets admin in at level 15", o.ends_with("R1#"));
    t.check("the note of RADIUS", e.trace.events().any(|x| x.dev == R1 && matches!(&x.note, Note::Aaa(jfn_sim::ANote::RadiusAccept { .. }))));
    let _ = cli(&mut e, PC1, &["exit"], SEC);
    let o = cli(&mut e, PC1, &[&format!("ssh {} admin@10.0.0.1", NOASK), "falsch"], 8 * SEC);
    t.check("a reject is a wrong password", o.contains("That password was refused.") && e.cli_secret(PC1));
    t.check("the note", e.trace.events().any(|x| x.dev == R1 && matches!(&x.note, Note::Aaa(jfn_sim::ANote::RadiusReject { .. }))));
    let o = cli(&mut e, PC1, &["adminpw"], 8 * SEC);
    t.check("the next try counts", o.ends_with("R1#"));
    let _ = cli(&mut e, PC1, &["exit"], SEC);
    // the server off: the next method
    let _ = e.apply(Action::Power { dev: SRV1, on: false });
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &[&format!("ssh {} local1@10.0.0.1", NOASK), "localpw"], 30 * SEC);
    t.check("no answer: the local user", o.ends_with("R1>"));
}

/// transport input decides who may come
fn transport(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_LOCAL)));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["telnet 10.0.0.1"], 2 * SEC);
    t.check("transport input ssh refuses telnet", o.contains("Connection refused"));
    let mut e = engine(&project(&r1_with("username admin secret adminpw\nline vty 0 4\n login local\n transport input telnet\n")));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &[&format!("ssh {} admin@10.0.0.1", NOASK)], 2 * SEC);
    t.check("transport input telnet refuses SSH", o.contains("ssh: connect to host 10.0.0.1 port 22: Connection refused"));
    t.eq("with status 255", cli(&mut e, PC1, &["echo $?"], 0), s("255\n"));
    let o = cli(&mut e, PC1, &["telnet 10.0.0.1"], 2 * SEC);
    t.check("and takes telnet", o.contains("Username:"));
    // the default takes both
    let mut e = engine(&project(&r1_with("username admin secret adminpw\nline vty 0 4\n login local\n")));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &[&format!("ssh {} admin@10.0.0.1", NOASK), "adminpw"], SEC);
    t.check("the default takes SSH", o.ends_with("R1>"));
    // no key: nothing listens; a key: it does; the key gone: no more
    let mut e = engine(&project("hostname R1\n!\ninterface GigabitEthernet0/0/0\n ip address 10.0.0.1 255.255.255.0\n no shutdown\n!\nusername admin secret adminpw\nline vty 0 4\n login local\n!\nend\n"));
    run_for(&mut e, SEC);
    let o = cli(&mut e, R1, &["show ip ssh"], 0);
    t.check("without a key it is off", o.contains("SSH server: off") && o.contains("Host keys: none"));
    let o = cli(&mut e, PC1, &[&format!("ssh {} admin@10.0.0.1", NOASK)], 2 * SEC);
    t.check("connection refused", o.contains("Connection refused"));
    let o = cli(&mut e, R1, &["enable", "configure terminal", "crypto key generate rsa"], 0);
    t.check("a key needs a domain", o.contains("% Please define a domain-name first."));
    let o = cli(&mut e, R1, &["ip domain-name lab.test", "crypto key generate rsa modulus 1024"], 0);
    t.check("then it is made", o.contains("Made a 1024-bit RSA key for R1.lab.test.") && o.contains("%SSH-5-ENABLED"));
    t.check("as files of the project", e.take_reports().is_empty() || e.file_store(R1).is_some_and(|f| f.exists("nvram:/ssh_host_rsa_key")));
    let o = cli(&mut e, PC1, &[&format!("ssh {} admin@10.0.0.1", NOASK), "adminpw"], SEC);
    t.check("an RSA host key", o.ends_with("R1>"));
    t.check("rsa-sha2-256 in use", has(&e, R1, |n| matches!(n, SshNote::Keyed { host_key, .. } if host_key == "rsa-sha2-256")));
    let _ = cli(&mut e, PC1, &["exit"], SEC);
    let o = cli(&mut e, R1, &["crypto key zeroize rsa"], 0);
    t.eq("zeroize says nothing", o, s(""));
    let o = cli(&mut e, PC1, &[&format!("ssh {} admin@10.0.0.1", NOASK)], 2 * SEC);
    t.check("the key gone: refused again", o.contains("Connection refused"));
    let o = cli(&mut e, R1, &["crypto key zeroize rsa"], 0);
    t.check("no such key", o.contains("% No such key"));
}

/// wrong passwords and the time for a login
fn tries_and_time(t: &mut T) {
    let mut e = engine(&project(&r1_with(&format!("ip ssh authentication-retries 2\nip ssh time-out 20\n{}", R1_LOCAL))));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &[&format!("ssh {} admin@10.0.0.1", NOASK), "falsch"], SEC);
    t.check("a wrong password, another try", o.contains("That password was refused.") && o.ends_with("Password for admin at 10.0.0.1: "));
    t.check("the server's note", has(&e, R1, |n| matches!(n, SshNote::LoginBad { .. })));
    let o = cli(&mut e, PC1, &["falsch"], SEC);
    t.check("the tries are up", o.contains("closed the connection") && e.cli_prompt(PC1) == "root@PC1:~# ");
    t.check("and its note", has(&e, R1, |n| matches!(n, SshNote::TooManyTries { .. })));
    t.eq("the line is free", net_sessions(&e, R1), 0);
    t.eq("the count", e.metric(R1, "vty-ssh", "bad_logins"), Some(2));
    let o = cli(&mut e, R1, &["show ip ssh"], 0);
    t.check("show ip ssh: the limits", o.contains("Time for a login: 20 s, tries: 2"));
    // nobody types: the time is up
    let o = cli(&mut e, PC1, &[&format!("ssh {} admin@10.0.0.1", NOASK)], SEC);
    t.check("it asks", o.ends_with(": "));
    let o = cli(&mut e, PC1, &[], 25 * SEC);
    t.check("the time for a login is up", o.contains("closed the connection") && has(&e, R1, |n| matches!(n, SshNote::LoginTimeout { .. })));
    t.eq("again free", net_sessions(&e, R1), 0);
    // exec-timeout of the line, as for telnet
    let mut e = engine(&project(&r1_with("username admin secret adminpw\nline vty 0 4\n login local\n exec-timeout 0 30\n transport input ssh\n")));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &[&format!("ssh {} admin@10.0.0.1", NOASK), "adminpw"], SEC);
    t.check("logged in", o.ends_with("R1>"));
    let o = cli(&mut e, PC1, &[], 40 * SEC);
    t.check("the idle timeout ends it", o.contains("% No input for") && e.cli_prompt(PC1) == "root@PC1:~# " && net_sessions(&e, R1) == 0);
    t.check("the note says idle", has(&e, R1, |n| matches!(n, SshNote::Closed { idle: true, .. })));
}

/// a command in exec mode against the router
fn exec_ios(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_LOCAL)));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &[&format!("ssh {} admin@10.0.0.1 show ip interface brief", NOASK), "adminpw"], SEC);
    t.check("the command's output, then done", o.contains("GigabitEthernet0/0/0") && o.contains("10.0.0.1") && !o.contains("R1#") && e.cli_prompt(PC1) == "root@PC1:~# ");
    t.eq("status 0", cli(&mut e, PC1, &["echo $?"], 0), s("0\n"));
    t.eq("the line is free", net_sessions(&e, R1), 0);
    t.check("the note names the command", has(&e, R1, |n| matches!(n, SshNote::Session { what, .. } if what == "show ip interface brief")));
}

/// the client of the IOS-like console, into a Linux host
fn ios_client(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    // (R1's first packet to SRV1 waits for ARP: TCP sends it again)
    let o = cli(&mut e, R1, &["enable", "ssh -l root 10.0.0.10"], 3 * SEC);
    t.check("no question about the key, the password", o.ends_with("Password: ") && e.cli_secret(R1));
    let o = cli(&mut e, R1, &["falsch"], SEC);
    t.check("refused", o.contains("% Access denied") && o.ends_with("Password: "));
    let o = cli(&mut e, R1, &["geheim"], SEC);
    t.check("a shell", o.ends_with("root@SRV1:~# "));
    let o = cli(&mut e, R1, &["hostname"], SEC);
    t.check("it runs there", o.contains("SRV1\n"));
    let o = cli(&mut e, R1, &["exit 4"], SEC);
    t.check("back", o.contains("% Session with 10.0.0.10 ended") && e.cli_prompt(R1) == "R1#");
    // options, a refused cipher
    let o = cli(&mut e, R1, &["ssh -c aes256-ctr -l root 10.0.0.10", "geheim"], SEC);
    t.check("-c aes256-ctr", o.ends_with("root@SRV1:~# ") && has(&e, SRV1, |n| matches!(n, SshNote::Keyed { cipher, .. } if cipher == "aes256-ctr")));
    let _ = cli(&mut e, R1, &["exit"], SEC);
    let o = cli(&mut e, R1, &["ssh -v 1 -l root 10.0.0.10"], 0);
    t.check("only version 2", o.contains("% Invalid input") || o.contains("^"));
    let o = cli(&mut e, R1, &["ssh -l root -p 2222 10.0.0.10"], 2 * SEC);
    t.check("a port without a server", o.contains("% Connection refused"));
    let o = cli(&mut e, R1, &["ssh -l root nirgends"], 3 * SEC);
    t.check("a name nobody knows", o.contains("% Bad IP address or host name: nirgends"));
    let o = cli(&mut e, R1, &["debug ip ssh"], 0);
    t.check("debug ip ssh", o.contains("Debug output for SSH: on"));
    t.check("show debugging", cli(&mut e, R1, &["show debugging"], 0).contains("Debug output for SSH: on"));
    let o = cli(&mut e, R1, &["ssh -l root 10.0.0.10", "geheim"], SEC);
    t.check("the debug lines of the client", o.contains("SSH: connected to 10.0.0.10 port 22") && o.contains("SSH: logged in as root at 10.0.0.10 (password)"));
    let _ = cli(&mut e, R1, &["exit"], SEC);
    t.check("undebug", cli(&mut e, R1, &["undebug ip ssh"], 0).contains("off"));
}

/// the running configuration, show crypto key, a key through a reload
fn keys_and_config(t: &mut T) {
    let mut e = engine(&project(&r1_with("ip ssh version 2\nip ssh time-out 60\n")));
    run_for(&mut e, SEC);
    let run = e.running_config(R1);
    t.check("the lines of the configuration", run.contains("ip domain-name lab.test\n") && run.contains("ip ssh version 2\n") && run.contains("ip ssh time-out 60\n") && !run.contains("crypto key"));
    let o = cli(&mut e, R1, &["show crypto key mypubkey"], 0);
    t.check("show crypto key", o.contains("Key for R1.lab.test") && o.contains("Kind: ssh-ed25519") && o.contains("Length: 256 bits") && o.contains("Public: ssh-ed25519 AAAA"));
    t.check("no RSA key", cli(&mut e, R1, &["show crypto key mypubkey rsa"], 0).contains("% No such key"));
    let before = cli(&mut e, R1, &["show ip ssh"], 0);
    let dir = cli(&mut e, R1, &["enable", "dir nvram:"], 0);
    t.check("the key is a file of nvram:", dir.contains("ssh_host_ed25519_key"));
    let _ = e.apply(Action::Power { dev: R1, on: false });
    run_for(&mut e, SEC);
    let _ = e.apply(Action::Power { dev: R1, on: true });
    run_for(&mut e, 5 * SEC);
    t.eq("the key stays through a reload", cli(&mut e, R1, &["show ip ssh"], 0), before);
    let o = cli(&mut e, R1, &["enable", "configure terminal", "ip ssh version 1"], 0);
    t.check("version 1 is refused", o.contains("% This device speaks SSH version 2 only"));
    let o = cli(&mut e, R1, &["no ip domain-name", "no ip ssh version", "no ip ssh time-out", "end"], 0);
    let run = e.running_config(R1);
    t.check("the no-forms", !o.contains("% ") && !run.contains("ip domain-name") && !run.contains("ip ssh"));
    // the factory's name
    let mut e = engine(&project("hostname R1\nip domain-name x.y\nend\n"));
    run_for(&mut e, SEC);
    let _ = cli(&mut e, R1, &["enable", "configure terminal", "no hostname"], 0);
    let o = cli(&mut e, R1, &["crypto key generate ed25519"], 0);
    t.check("a key wants a host name of its own", o.contains("% Please define a hostname other than Router."));
}

/// sshd with a password; its key comes at its start
fn linux_password(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    let store = e.file_store(SRV1);
    t.check("sshd made its key", store.is_some_and(|f| f.exists("/etc/ssh/ssh_host_ed25519_key") && f.exists("/etc/ssh/ssh_host_ed25519_key.pub")));
    t.check("the note", has(&e, SRV1, |n| matches!(n, SshNote::KeyMade { .. })));
    let o = cli(&mut e, SRV1, &["cat /etc/ssh/ssh_host_ed25519_key.pub"], 0);
    t.check("a public line", o.starts_with("ssh-ed25519 AAAA") && o.contains(" root@SRV1"));
    let o = cli(&mut e, SRV1, &["ss -tln"], 0);
    t.check("it listens on 22", o.contains(":22 "));
    let o = cli(&mut e, PC1, &[&format!("ssh {} 10.0.0.10", NOASK), "geheim"], SEC);
    t.check("root by default, a shell", o.ends_with("root@SRV1:~# "));
    let o = cli(&mut e, PC1, &["who"], SEC);
    t.check("who: the pts and from where", o.contains("root     pts/0") && o.contains("(10.0.0.20)"));
    let w = cli(&mut e, PC1, &["w"], SEC);
    t.check("w too", w.contains("2 users") && w.contains("pts/0    10.0.0.20"));
    t.check("tty", cli(&mut e, PC1, &["tty"], SEC).starts_with("/dev/pts/0"));
    let o = cli(&mut e, PC1, &["exit 3"], SEC);
    t.check("exit", o.contains("logout") && o.contains("Session with 10.0.0.10 ended."));
    t.eq("exit status from far away", cli(&mut e, PC1, &["echo $?"], 0), s("3\n"));
    t.eq("the session went", e.cli_sessions(SRV1).len(), 1);
    // exec: the status of the command
    let o = cli(&mut e, PC1, &[&format!("ssh {} root@10.0.0.10 'false'", NOASK), "geheim"], SEC);
    t.check("exec: no prompt", !o.contains("root@SRV1"));
    t.eq("exit status 1", cli(&mut e, PC1, &["echo $?"], 0), s("1\n"));
    let o = cli(&mut e, PC1, &[&format!("ssh {} root@10.0.0.10 'echo eins; echo zwei'", NOASK), "geheim"], SEC);
    t.check("a line of the shell over there", o.contains("eins\nzwei\n"));
    t.eq("exit status 0", cli(&mut e, PC1, &["echo $?"], 0), s("0\n"));
    let o = cli(&mut e, PC1, &[&format!("ssh {} root@10.0.0.10 hostname | grep SRV", NOASK), "geheim"], SEC);
    t.check("through a pipe", o.contains("SRV1"));
    // a user who is none, a user of the service, no password at all
    let o = cli(&mut e, PC1, &[&format!("ssh {} niemand@10.0.0.10", NOASK), "geheim", "geheim", "geheim"], SEC);
    t.check("an unknown user: refused three times", o.contains("the login as niemand at 10.0.0.10 was refused"));
    t.check("and its note", has(&e, PC1, |n| matches!(n, SshNote::ClientDenied { .. })));
    let o = cli(&mut e, PC1, &[&format!("ssh {} deploy@10.0.0.10", NOASK), "dpw"], SEC);
    t.check("another user, in its own home", o.ends_with("deploy@SRV1:~$ "));
    let o = cli(&mut e, PC1, &["pwd", "echo $USER", "who"], SEC);
    t.check("what it sees", o.contains("/home/deploy\n") && o.contains("deploy\n") && o.contains("deploy   pts/0"));
    t.check("the app's view", e.cli_sessions(SRV1).iter().any(|v| v.user == "deploy"));
    let _ = cli(&mut e, PC1, &["exit"], SEC);
    // password_auth off
    let mut p = project(&r1_with(""));
    if let Some(d) = p.device_mut(SRV1) {
        d.services = alloc::vec![def("sshd", "sshd", r#"{"password": "geheim", "password_auth": false}"#)];
    }
    let mut e = engine(&p);
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &[&format!("ssh {} root@10.0.0.10", NOASK)], SEC);
    t.check("no passwords here, no key at hand", o.contains("was refused") && !o.contains("Password for"));
    // a port of its own, what the service refuses on IOS-like devices
    let mut p = project(&r1_with(""));
    if let Some(d) = p.device_mut(SRV1) {
        d.services = alloc::vec![def("sshd", "sshd", r#"{"port": 2222, "password": "geheim"}"#)];
    }
    let mut e = engine(&p);
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["ssh -o StrictHostKeyChecking=accept-new -p 2222 root@10.0.0.10", "geheim"], SEC);
    t.check("a port of its own", o.ends_with("root@SRV1:~# "));
    let _ = cli(&mut e, PC1, &["exit"], SEC);
    t.check("known_hosts with the port", cli(&mut e, PC1, &["cat ~/.ssh/known_hosts"], 0).starts_with("[10.0.0.10]:2222 ssh-ed25519 AAAA"));
    let mut p = project(&r1_with(""));
    if let Some(d) = p.device_mut(R1) {
        d.services = alloc::vec![def("sshd", "x", "{}")];
    }
    t.check("no sshd on an IOS-like device", engine(&p).services(R1).is_empty());
}

/// ssh-keygen, authorized_keys, a login by key
fn linux_key(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["ssh-keygen -t ed25519 -N \"\""], 0);
    t.check("ssh-keygen says what it made", o.contains("Made a ed25519 key pair.") && o.contains("/root/.ssh/id_ed25519.pub") && o.contains("root@PC1"));
    let pubkey = cli(&mut e, PC1, &["cat ~/.ssh/id_ed25519.pub"], 0);
    t.check("the public file", pubkey.starts_with("ssh-ed25519 AAAA") && pubkey.ends_with("root@PC1\n"));
    t.check("the secret file's format", cli(&mut e, PC1, &["head -n 1 ~/.ssh/id_ed25519"], 0).starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
    t.check("-l", cli(&mut e, PC1, &["ssh-keygen -l -f ~/.ssh/id_ed25519.pub"], 0).starts_with("256 SHA256:"));
    t.eq("-y", cli(&mut e, PC1, &["ssh-keygen -y -f ~/.ssh/id_ed25519"], 0), pubkey.clone());
    t.check("it does not overwrite", cli(&mut e, PC1, &["ssh-keygen"], 0).contains("is there already"));
    // not on the server yet: the password is asked
    let o = cli(&mut e, PC1, &[&format!("ssh {} root@10.0.0.10", NOASK)], SEC);
    t.check("an unknown key: a password", o.ends_with("Password for root at 10.0.0.10: "));
    let o = cli(&mut e, PC1, &["geheim"], SEC);
    t.check("a shell by password", o.ends_with("root@SRV1:~# "));
    let _ = cli(&mut e, PC1, &["exit"], SEC);
    // the key on the server
    let _ = cli(&mut e, SRV1, &["mkdir -p /root/.ssh", &format!("echo '{}' >> /root/.ssh/authorized_keys", pubkey.trim())], 0);
    let o = cli(&mut e, PC1, &[&format!("ssh {} root@10.0.0.10", NOASK)], SEC);
    t.check("by key: no password", o.ends_with("root@SRV1:~# "));
    t.check("the notes", has(&e, SRV1, |n| matches!(n, SshNote::LoginOk { how, .. } if how == "Schlüssel")) && has(&e, PC1, |n| matches!(n, SshNote::ClientLogin { how, .. } if how == "publickey")));
    let _ = cli(&mut e, PC1, &["exit"], SEC);
    let o = cli(&mut e, PC1, &[&format!("ssh -o BatchMode=yes {} root@10.0.0.10 hostname", NOASK)], SEC);
    t.check("BatchMode: the command", o.contains("SRV1"));
    // RSA user key, -i
    let o = cli(&mut e, PC1, &["ssh-keygen -t rsa -b 1024 -f /tmp/rsa -q", "ssh-keygen -l -f /tmp/rsa"], 0);
    t.check("an RSA key", o.starts_with("1024 SHA256:"));
    let line = cli(&mut e, PC1, &["cat /tmp/rsa.pub"], 0);
    let _ = cli(&mut e, SRV1, &[&format!("echo '{}' >> /root/.ssh/authorized_keys", line.trim())], 0);
    let o = cli(&mut e, PC1, &[&format!("ssh -i /tmp/rsa {} root@10.0.0.10 hostname", NOASK)], SEC);
    t.check("by RSA key", o.contains("SRV1"));
    // a key the file does not have: BatchMode has no way
    let o = cli(&mut e, PC1, &["ssh-keygen -f /tmp/other -q", &format!("ssh -i /tmp/other -o BatchMode=yes {} root@10.0.0.10 hostname", NOASK)], SEC);
    t.check("another key is refused", o.contains("was refused"));
    t.eq("with status 255", cli(&mut e, PC1, &["echo $?"], 0), s("255\n"));
    let o = cli(&mut e, PC1, &["ssh-keygen -N geheim -f /tmp/x"], 0);
    t.check("no passphrases", o.contains("not made here"));
}

/// known_hosts: new, remembered, changed, forgotten
fn known_hosts(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["ssh root@10.0.0.10 hostname", "no"], SEC);
    t.check("no: refused", o.contains("Refused: the host key was not taken."));
    t.eq("with status 255", cli(&mut e, PC1, &["echo $?"], 0), s("255\n"));
    t.check("the note", has(&e, PC1, |n| matches!(n, SshNote::HostKeyRefused { .. })));
    let o = cli(&mut e, PC1, &["ssh -o StrictHostKeyChecking=yes root@10.0.0.10 hostname"], SEC);
    t.check("yes: no question, refused", o.contains("Refused: no key of 10.0.0.10 is remembered"));
    let o = cli(&mut e, PC1, &["ssh -o BatchMode=yes root@10.0.0.10 hostname"], SEC);
    t.check("BatchMode: no question either", o.contains("Refused: no key of 10.0.0.10 is remembered"));
    let o = cli(&mut e, PC1, &["ssh -o StrictHostKeyChecking=accept-new root@10.0.0.10 hostname", "geheim"], SEC);
    t.check("accept-new", o.contains("Remembered 10.0.0.10") && o.contains("SRV1"));
    let kh = cli(&mut e, PC1, &["cat ~/.ssh/known_hosts"], 0);
    let pubk = cli(&mut e, SRV1, &["cat /etc/ssh/ssh_host_ed25519_key.pub"], 0);
    let key: Vec<&str> = pubk.split_whitespace().take(2).collect();
    t.eq("known_hosts holds the key", kh, format!("10.0.0.10 {} {}\n", key[0], key[1]));
    let o = cli(&mut e, PC1, &["ssh -o StrictHostKeyChecking=yes root@10.0.0.10 hostname", "geheim"], SEC);
    t.check("now known", o.contains("SRV1") && !o.contains("Remembered"));
    // the host gets a new key
    let _ = cli(&mut e, SRV1, &["rm /etc/ssh/ssh_host_ed25519_key"], 0);
    let _ = e.apply(Action::ServiceRun { dev: SRV1, name: s("sshd"), on: false });
    run_for(&mut e, SEC);
    let _ = e.apply(Action::ServiceRun { dev: SRV1, name: s("sshd"), on: true });
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["ssh root@10.0.0.10 hostname"], SEC);
    t.check("a changed key refuses, no question", o.contains("is not the one remembered") && o.contains("(line 1)") && o.contains("Refused: the host key changed.") && !o.contains("Type yes"));
    t.eq("with status 255", cli(&mut e, PC1, &["echo $?"], 0), s("255\n"));
    t.check("the note", has(&e, PC1, |n| matches!(n, SshNote::HostKeyChanged { .. })));
    t.check("the server hears why", has(&e, SRV1, |n| matches!(n, SshNote::Broken { .. })));
    let o = cli(&mut e, PC1, &["ssh-keygen -R 10.0.0.10"], 0);
    t.check("ssh-keygen -R", o.contains("Forgot 10.0.0.10"));
    let o = cli(&mut e, PC1, &["ssh root@10.0.0.10 hostname"], SEC);
    t.check("then it asks again", o.contains("10.0.0.10 is new here"));
    let _ = cli(&mut e, PC1, &["no"], SEC);
    t.check("-R of nothing", cli(&mut e, PC1, &["ssh-keygen -R nirgends"], 0).contains("is not in"));
}

/// a session: Ctrl+C travels, "~." leaves, the far end goes away
fn linux_session(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    let _ = cli(&mut e, PC1, &[&format!("ssh {} root@10.0.0.10", NOASK), "geheim"], SEC);
    let o = cli(&mut e, PC1, &["ping 10.0.0.1"], 3 * SEC);
    t.check("a command that runs over there", o.contains("bytes from 10.0.0.1"));
    t.check("the local program keeps reading", e.cli_reads(PC1));
    let _ = e.cli_interrupt(PC1);
    let o = cli(&mut e, PC1, &[], SEC);
    t.check("Ctrl+C stops it there, the session stays", o.contains("ping statistics") && o.ends_with("root@SRV1:~# "));
    t.check("still connected", e.cli_reads(PC1) && e.cli_sessions(SRV1).len() == 2);
    let _ = e.cli_interrupt(PC1);
    let o = cli(&mut e, PC1, &[], SEC);
    t.check("Ctrl+C at the far prompt: the prompt again", o.ends_with("root@SRV1:~# "));
    let o = cli(&mut e, PC1, &["~."], SEC);
    t.check("~. leaves", o.contains("Closed from this side.") && e.cli_prompt(PC1) == "root@PC1:~# ");
    t.eq("with status 255", cli(&mut e, PC1, &["echo $?"], 0), s("255\n"));
    t.eq("the far session went", e.cli_sessions(SRV1).len(), 1);
    // the far end goes away
    let _ = cli(&mut e, PC1, &[&format!("ssh {} root@10.0.0.10", NOASK), "geheim"], SEC);
    let _ = e.apply(Action::ServiceRun { dev: SRV1, name: s("sshd"), on: false });
    let o = cli(&mut e, PC1, &[], SEC);
    t.check("a stopped server ends the session", o.contains("ssh: connection to 10.0.0.10 closed") && e.cli_prompt(PC1) == "root@PC1:~# ");
    t.eq("no session left", e.cli_sessions(SRV1).len(), 1);
}

/// scp both ways, in a session of its own
fn scp(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    let _ = cli(&mut e, PC1, &["echo hallo welt > /tmp/a.txt"], 0);
    let o = cli(&mut e, PC1, &[&format!("scp {} /tmp/a.txt root@10.0.0.10:/tmp/b.txt", NOASK), "geheim"], SEC);
    t.check("scp up", o.contains("a.txt: 11 bytes copied"));
    t.eq("it is there", cli(&mut e, SRV1, &["cat /tmp/b.txt"], 0), s("hallo welt\n"));
    t.eq("status 0", cli(&mut e, PC1, &["echo $?"], 0), s("0\n"));
    let o = cli(&mut e, PC1, &[&format!("scp -q {} root@10.0.0.10:/tmp/b.txt /tmp/c.txt", NOASK), "geheim"], SEC);
    t.eq("scp down, quiet", o.contains("copied"), false);
    t.eq("back here", cli(&mut e, PC1, &["cat /tmp/c.txt"], 0), s("hallo welt\n"));
    let o = cli(&mut e, PC1, &[&format!("scp {} root@10.0.0.10:/tmp/nichts /tmp/d.txt", NOASK), "geheim"], SEC);
    t.check("a file that is not there", o.contains("No such file"));
    t.eq("status 1", cli(&mut e, PC1, &["echo $?"], 0), s("1\n"));
    let o = cli(&mut e, PC1, &["scp /tmp/nichts root@10.0.0.10:/tmp/d.txt"], SEC);
    t.check("a local file that is not there", o.contains("No such file"));
    // into a directory, by the user's key
    let _ = cli(&mut e, PC1, &["ssh-keygen -q"], 0);
    let pubkey = cli(&mut e, PC1, &["cat ~/.ssh/id_ed25519.pub"], 0);
    let _ = cli(&mut e, SRV1, &["mkdir -p /root/.ssh", &format!("echo '{}' > /root/.ssh/authorized_keys", pubkey.trim())], 0);
    let o = cli(&mut e, PC1, &[&format!("scp {} /tmp/a.txt 10.0.0.10:/srv", NOASK)], SEC);
    t.check("into a directory", o.contains("copied"));
    t.eq("its name stays", cli(&mut e, SRV1, &["cat /srv/a.txt"], 0), s("hallo welt\n"));
    let o = cli(&mut e, PC1, &[&format!("scp {} /tmp/a.txt 10.0.0.10:rel.txt", NOASK)], SEC);
    t.check("a relative path: the home", o.contains("copied"));
    t.eq("in root's home", cli(&mut e, SRV1, &["cat /root/rel.txt"], 0), s("hallo welt\n"));
    t.check("the note", has(&e, PC1, |n| matches!(n, SshNote::Scp { upload: true, .. })));
    t.check("usage", cli(&mut e, PC1, &["scp /a /b"], 0).contains("usage: scp"));
}

/// what the frames show: real SSH bytes, dissected
fn wire(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_LOCAL)));
    run_for(&mut e, SEC);
    let _ = cli(&mut e, PC1, &[&format!("ssh {} admin@10.0.0.1", NOASK), "adminpw", "show ip interface brief", "exit"], SEC);
    let (mut ident, mut kexinit, mut ecdh, mut newkeys, mut encrypted) = (0, 0, 0, 0, 0);
    let mut idents: Vec<String> = Vec::new();
    for h in e.trace.hops() {
        if decode::proto_mask(&h.bytes) & decode::P_SSH == 0 {
            continue;
        }
        let (_, info) = decode::info(&h.bytes);
        if info.starts_with("SSH-2.0-") {
            ident += 1;
            idents.push(info.clone());
        }
        if info.contains("SSH_MSG_KEXINIT") {
            kexinit += 1;
        }
        if info.contains("SSH_MSG_KEX_ECDH") {
            ecdh += 1;
        }
        if info.contains("SSH_MSG_NEWKEYS") {
            newkeys += 1;
        }
        if info.starts_with("verschlüsselt") {
            encrypted += 1;
        }
    }
    t.check("identification both ways (each counted per hop)", ident >= 2 && idents.iter().all(|i| i == "SSH-2.0-JFNetworker_1"));
    t.check("KEXINIT and the ECDH messages in the clear", kexinit >= 2 && ecdh >= 2 && newkeys >= 2);
    t.check("then encrypted", encrypted >= 6);
    t.check("the password is nowhere on the wire", !e.trace.hops().any(|h| h.bytes.windows(7).any(|w| w == b"adminpw")));
    t.check("the command neither", !e.trace.hops().any(|h| h.bytes.windows(4).any(|w| w == b"show")));
    let layer = e.trace.hops().find(|h| decode::info(&h.bytes).1.contains("SSH_MSG_KEXINIT")).map(|h| decode::layers(&h.bytes));
    t.check("the layer's fields", layer.is_some_and(|ls| ls.iter().any(|l| l.proto == decode::Proto::Ssh && l.fields.iter().any(|f| f.name == "Message Code") && l.fields.iter().any(|f| f.name == "KEX Algorithms"))));
    t.check("the mask of the notes", notes(&e, R1).iter().any(|n| matches!(n, SshNote::Keyed { .. }) && n.mask() & decode::P_SSH != 0));
}

/// the inputs as commands of the driver replay with the keyframes (K1)
fn replay(t: &mut T) {
    let mut d = Inline::new(engine(&project(&r1_with(R1_LOCAL))));
    d.rewind_on(jfn_sim::Budget::default());
    let _ = d.send(Cmd::RunFor(SEC), 0);
    let a = d.open_session(PC1, 0).unwrap_or(0);
    let _ = d.send(Cmd::SesLine { dev: PC1, ses: a, line: s("ssh -o StrictHostKeyChecking=no admin@10.0.0.1") }, 0);
    let _ = d.send(Cmd::RunFor(500 * MS), 0);
    let _ = d.send(Cmd::SesLine { dev: PC1, ses: a, line: s("adminpw") }, 0);
    let _ = d.send(Cmd::RunFor(SEC), 0);
    let at = d.pos();
    let want = jfn_sim::keyframe::digest(&d.eng);
    let _ = d.send(Cmd::RunFor(SEC), 0);
    let got = d.restore_pos(at, None).and_then(|mut r| {
        let rw = d.rewind()?;
        let done = (0..1000).any(|_| r.run(rw, 1_000_000));
        if done { Some(r.finish()) } else { None }
    });
    t.check("a replay brings the SSH session back", got.as_ref().is_some_and(|p| jfn_sim::keyframe::digest(&p.eng) == want && net_sessions(&p.eng, R1) == 1));
    let _ = d.send(Cmd::SesClose { dev: PC1, ses: a }, 0);
    t.eq("closed by command", d.eng.cli_sessions(PC1).len(), 1);
}

/// two runs give the same engine and the same frames
fn determinism(t: &mut T) {
    let go = || {
        let mut e = engine(&project(&r1_with(R1_LOCAL)));
        run_for(&mut e, SEC);
        let o = cli(&mut e, PC1, &["ssh -o StrictHostKeyChecking=accept-new admin@10.0.0.1", "adminpw", "show users"], SEC);
        let o2 = cli(&mut e, PC1, &["exit", "ssh-keygen -q", "cat ~/.ssh/id_ed25519.pub"], SEC);
        let frames: Vec<u8> = e.trace.hops().flat_map(|h| h.bytes.iter().copied().collect::<Vec<u8>>()).collect();
        (o, o2, crypto::sha256(&frames), jfn_sim::keyframe::digest(&e))
    };
    let a = go();
    let b = go();
    t.check("same sessions, keys and frames twice", a == b);
    t.check("they did run", a.0.contains("(ssh)") && a.1.contains("ssh-ed25519"));
    // another seed: other keys
    let mut p = project(&r1_with(R1_LOCAL));
    p.seed = 99;
    let mut e = engine(&p);
    run_for(&mut e, SEC);
    let k = cli(&mut e, PC1, &["ssh-keygen -q", "cat ~/.ssh/id_ed25519.pub"], SEC);
    t.check("another seed, another key", !a.1.contains(&k));
}
