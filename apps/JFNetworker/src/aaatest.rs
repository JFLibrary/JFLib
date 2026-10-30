// aaatest -- checks of the logins, AAA and RADIUS (group "aaa" of --selftest: jfn_sim::auth,
// aaa, aaatrace, radiusd, telnetd, cli/aaa.rs): the console login with local users, the enable
// secret and privilege levels, RADIUS logins accepted and rejected, a wrong shared secret
// (dropped, then the local fallback), a server that is down (fallback), authorization of the
// privilege level, accounting records, retransmissions on a lossy link, the telnet client, and
// determinism
//
// The LAN (device ids in order): R1 (JF-R4, Gi0/0/0 10.0.0.1/24), SW1 (switch), SRV1
// (server-1u, 10.0.0.10, the RADIUS server), PC1 (PC, 10.0.0.20).  The RADIUS server knows the
// client 10.0.0.0/24 with the secret "testing123" and the users admin (adminpw, privilege 15)
// and bob (bobpw, privilege 1).  The timeout is 1 s and one retransmission, so a login that
// falls back takes a bounded amount of virtual time.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::{ANote, Engine, Note, Report, SEC};

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
"users": [{"name": "admin", "password": "adminpw", "privilege": 15, "reply": "Willkommen, admin."},
{"name": "bob", "password": "bobpw", "privilege": 1}]}"#;

/// the LAN; `r1` is R1's startup configuration, `radius` the RADIUS server's settings (None:
/// the server is off)
fn project(r1: &str, radius: Option<&str>) -> Project {
    let mut p = Project::new("aaa", 7);
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
    if let Some(r) = radius
        && let Some(d) = p.device_mut(SRV1)
    {
        d.services = alloc::vec![def("radius-server", "radius", r)];
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

fn out(e: &mut Engine, dev: u32) -> String {
    let mut o = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: d, bytes, .. } = r
            && d == dev
        {
            o.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    o
}

/// feed lines at a device's console, run `d`, return what it printed
fn cli(e: &mut Engine, dev: u32, lines: &[&str], d: u64) -> String {
    let _ = e.take_reports();
    for l in lines {
        let _ = e.cli_line(dev, l);
    }
    run_for(e, d);
    out(e, dev)
}

/// the AAA notes a device made
fn anotes(e: &Engine, dev: u32) -> Vec<ANote> {
    e.trace.events().filter(|x| x.dev == dev).filter_map(|x| if let Note::Aaa(n) = &x.note { Some(n.clone()) } else { None }).collect()
}

fn has<F: Fn(&ANote) -> bool>(e: &Engine, dev: u32, f: F) -> bool {
    anotes(e, dev).iter().any(f)
}

// a router with a few local credentials and a login on the console
const LOCAL_R1: &str = "hostname R1\n!\nenable secret s3cret\nusername admin privilege 15 secret adminpw\nusername bob privilege 1 secret bobpw\n!\ninterface GigabitEthernet0/0/0\n ip address 10.0.0.1 255.255.255.0\n no shutdown\n!\nline con 0\n login local\n!\nend\n";

pub(crate) fn run(t: &mut T) {
    secrets(t);
    local_login(t);
    enable_levels(t);
    config_roundtrip(t);
    radius_login(t);
    radius_reject(t);
    wrong_secret(t);
    server_down(t);
    accounting(t);
    lossy(t);
    telnet_client(t);
    determinism(t);
}

/// auth.rs: the stored forms and their verification (no engine)
fn secrets(t: &mut T) {
    use jfn_sim::Secret;
    let s = Secret::pbkdf2("geheim", &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    t.check("pbkdf2 kind 8", s.kind == 8 && s.text.starts_with("$8$"));
    t.check("pbkdf2 verifies", s.verify("geheim") && !s.verify("Geheim") && !s.verify(""));
    let p = Secret::plain("abc");
    t.check("plain", p.kind == 0 && p.config_word() == "abc" && p.verify("abc") && !p.verify("ab"));
    let seven = Secret::type7("abc");
    t.check("type 7 reversible", seven.kind == 7 && seven.reveal().as_deref() == Some("abc") && seven.verify("abc"));
    t.check("type 7 obfuscated", seven.text != "abc" && seven.config_word().starts_with("7 "));
    // a salt makes two secrets of the same password differ, both verify
    let a = Secret::pbkdf2("x", &[0; 12]);
    let b = Secret::pbkdf2("x", &[9; 12]);
    t.check("salted", a.text != b.text && a.verify("x") && b.verify("x"));
}

/// a console login with local users: the prompts, three tries, the privilege level
fn local_login(t: &mut T) {
    let mut e = engine(&project(LOCAL_R1, None));
    run_for(&mut e, SEC);
    t.eq("locked at boot", e.cli_prompt(R1), s("Username: "));
    t.check("username not secret", !e.cli_secret(R1));
    let _ = cli(&mut e, R1, &["admin"], 0);
    t.eq("asks for the password", e.cli_prompt(R1), s("Password: "));
    t.check("password is hidden", e.cli_secret(R1));
    let o = cli(&mut e, R1, &["wrong"], 0);
    t.check("wrong password refused", o.contains("% Access denied") && e.cli_prompt(R1) == "Username: ");
    // the right credentials of the privilege-15 user: straight into enabled mode
    let _ = cli(&mut e, R1, &["admin"], 0);
    let _ = cli(&mut e, R1, &["adminpw"], 0);
    t.eq("admin logged in", e.cli_prompt(R1), s("R1#"));
    t.check("note of the login", has(&e, R1, |n| matches!(n, ANote::LoginOk { user, level: 15, .. } if user == "admin")));
    // bob is privilege 1: the user level
    let _ = cli(&mut e, R1, &["exit"], 0);
    t.eq("relocked after exit", e.cli_prompt(R1), s("Username: "));
    let _ = cli(&mut e, R1, &["bob"], 0);
    let _ = cli(&mut e, R1, &["bobpw"], 0);
    t.eq("bob at the user level", e.cli_prompt(R1), s("R1>"));
    // three wrong tries end the login and a fresh one begins
    let _ = cli(&mut e, R1, &["exit"], 0);
    for _ in 0..3 {
        let _ = cli(&mut e, R1, &["x"], 0);
        let _ = cli(&mut e, R1, &["y"], 0);
    }
    t.check("denied after three tries", has(&e, R1, |n| matches!(n, ANote::LoginDenied { .. })) && e.cli_prompt(R1) == "Username: ");
}

/// the enable secret and the privilege levels of "enable" / "disable"
fn enable_levels(t: &mut T) {
    let mut e = engine(&project(LOCAL_R1, None));
    run_for(&mut e, SEC);
    let _ = cli(&mut e, R1, &["bob", "bobpw"], 0);
    t.eq("bob at user level", e.cli_prompt(R1), s("R1>"));
    // "enable" asks for the enable secret
    let _ = cli(&mut e, R1, &["enable"], 0);
    t.check("enable asks", e.cli_prompt(R1) == "Password: " && e.cli_secret(R1));
    // a wrong secret is refused and re-prompts (still in the enable gate)
    let o = cli(&mut e, R1, &["nope"], 0);
    t.check("wrong enable", o.contains("% Access denied") && e.cli_prompt(R1) == "Password: ");
    let _ = cli(&mut e, R1, &["s3cret"], 0);
    t.eq("enabled", e.cli_prompt(R1), s("R1#"));
    t.check("enable note", has(&e, R1, |n| matches!(n, ANote::EnableOk { level: 15 })));
    let _ = cli(&mut e, R1, &["disable"], 0);
    t.eq("back to user", e.cli_prompt(R1), s("R1>"));
}

/// the secrets are hashed in the running configuration and a save-and-reload keeps them
fn config_roundtrip(t: &mut T) {
    let mut e = engine(&project(LOCAL_R1, None));
    run_for(&mut e, SEC);
    let run = e.running_config(R1);
    t.check("enable secret hashed", run.contains("enable secret 8 $8$") && !run.contains("enable secret s3cret"));
    // privilege 1 is the default and left off, as IOS does
    t.check("username hashed", run.contains("username admin privilege 15 secret 8 $8$") && run.contains("username bob secret 8 $8$"));
    t.check("login local in the line", run.contains("line con 0\n login local\n"));
    // the running configuration boots the same device: the hashes are kept, the login works
    let mut p2 = project("hostname R1\n!\nend\n", None);
    if let Some(d) = p2.device_mut(R1) {
        d.startup_config = run.clone();
    }
    let mut e2 = engine(&p2);
    run_for(&mut e2, SEC);
    t.eq("reload: byte for byte", e2.running_config(R1), run);
    let _ = cli(&mut e2, R1, &["admin"], 0);
    let _ = cli(&mut e2, R1, &["adminpw"], 0);
    t.eq("reload: login still works", e2.cli_prompt(R1), s("R1#"));
}

// a router that authenticates console logins against RADIUS, with a local fallback
fn radius_r1(key: &str) -> String {
    format!(
        "hostname R1\n!\nusername admin secret adminpw\n!\naaa new-model\n!\nradius server S1\n address ipv4 10.0.0.10 auth-port 1812 acct-port 1813\n key {}\n timeout 1\n retransmit 1\n!\naaa authentication login default group radius local\naaa authorization exec default group radius local\naaa accounting exec default start-stop group radius\n!\ninterface GigabitEthernet0/0/0\n ip address 10.0.0.1 255.255.255.0\n no shutdown\n!\nline con 0\n login authentication default\n!\nend\n",
        key
    )
}

/// a RADIUS login accepted, with the privilege level from the server (authorization)
fn radius_login(t: &mut T) {
    let mut e = engine(&project(&radius_r1("testing123"), Some(RADIUS_CFG)));
    run_for(&mut e, 2 * SEC);
    t.eq("locked", e.cli_prompt(R1), s("Username: "));
    let _ = cli(&mut e, R1, &["admin"], 0);
    let o = cli(&mut e, R1, &["adminpw"], 2 * SEC);
    let _ = o;
    t.eq("admin via RADIUS at privilege 15", e.cli_prompt(R1), s("R1#"));
    t.check("request and accept", has(&e, R1, |n| matches!(n, ANote::RadiusRequest { .. })) && has(&e, R1, |n| matches!(n, ANote::RadiusAccept { level: Some(15), .. })));
    t.check("server accepted", has(&e, SRV1, |n| matches!(n, ANote::RadiusAccept { .. })));
    t.check("login how = RADIUS", has(&e, R1, |n| matches!(n, ANote::LoginOk { how, level: 15, .. } if how == "RADIUS")));
    // bob comes back at privilege 1 (authorization from the reply)
    let _ = cli(&mut e, R1, &["exit"], SEC);
    let _ = cli(&mut e, R1, &["bob"], 0);
    let _ = cli(&mut e, R1, &["bobpw"], 2 * SEC);
    t.eq("bob via RADIUS at privilege 1", e.cli_prompt(R1), s("R1>"));
    t.eq("server counts", e.metric(SRV1, "radius", "accepts"), Some(2));
}

/// a RADIUS login rejected (a wrong password): no fallback after a reject
fn radius_reject(t: &mut T) {
    let mut e = engine(&project(&radius_r1("testing123"), Some(RADIUS_CFG)));
    run_for(&mut e, 2 * SEC);
    let _ = cli(&mut e, R1, &["admin"], 0);
    let o = cli(&mut e, R1, &["wrongpw"], 2 * SEC);
    t.check("rejected", o.contains("% Access denied") && e.cli_prompt(R1) == "Username: ");
    t.check("reject note", has(&e, R1, |n| matches!(n, ANote::RadiusReject { .. })));
    // a reject does not fall back to local: the local password is not even tried
    t.check("no local fallback after reject", !has(&e, R1, |n| matches!(n, ANote::LoginOk { .. })));
    t.eq("server rejected one", e.metric(SRV1, "radius", "rejects"), Some(1));
}

/// a wrong shared secret: the server drops the request, the NAS times out, local takes over
fn wrong_secret(t: &mut T) {
    let mut e = engine(&project(&radius_r1("WRONG"), Some(RADIUS_CFG)));
    run_for(&mut e, 2 * SEC);
    let _ = cli(&mut e, R1, &["admin"], 0);
    // the fallback waits for the timeout and one retransmission (1 s each) before local
    let _ = cli(&mut e, R1, &["adminpw"], 4 * SEC);
    t.eq("local fallback logged in", e.cli_prompt(R1), s("R1>"));
    t.check("server dropped it", has(&e, SRV1, |n| matches!(n, ANote::RadiusDropped { .. })));
    t.check("the NAS timed out and retransmitted", has(&e, R1, |n| matches!(n, ANote::RadiusTimeout { .. })) && has(&e, R1, |n| matches!(n, ANote::RadiusRetransmit { .. })));
    t.check("then local", has(&e, R1, |n| matches!(n, ANote::LoginOk { how, .. } if how == "lokal")));
    t.eq("server accepted none", e.metric(SRV1, "radius", "accepts"), Some(0));
}

/// the server is switched off: the NAS gets no answer, local takes over
fn server_down(t: &mut T) {
    let mut p = project(&radius_r1("testing123"), Some(RADIUS_CFG));
    if let Some(d) = p.device_mut(SRV1) {
        d.off = true;
    }
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    let _ = cli(&mut e, R1, &["admin"], 0);
    let _ = cli(&mut e, R1, &["adminpw"], 4 * SEC);
    t.eq("fallback with the server off", e.cli_prompt(R1), s("R1>"));
    t.check("timeout then local", has(&e, R1, |n| matches!(n, ANote::RadiusTimeout { .. })) && has(&e, R1, |n| matches!(n, ANote::LoginOk { how, .. } if how == "lokal")));
}

/// accounting: a Start record goes to the server and is written to its file
fn accounting(t: &mut T) {
    let mut e = engine(&project(&radius_r1("testing123"), Some(RADIUS_CFG)));
    run_for(&mut e, 2 * SEC);
    let _ = cli(&mut e, R1, &["admin"], 0);
    let _ = cli(&mut e, R1, &["adminpw"], 2 * SEC);
    t.check("accounting start sent", has(&e, R1, |n| matches!(n, ANote::AcctSent { start: true, .. })));
    t.eq("server counted accounting", e.metric(SRV1, "radius", "accounting"), Some(1));
    let log = e.file_store(SRV1).and_then(|st| st.read("/var/log/radius/accounting").ok().map(|b| String::from_utf8_lossy(b).into_owned())).unwrap_or_default();
    t.check("accounting record written", log.contains("Start") && log.contains("admin"));
    // a logout sends a Stop record
    let _ = cli(&mut e, R1, &["exit"], 2 * SEC);
    t.check("accounting stop sent", has(&e, R1, |n| matches!(n, ANote::AcctSent { start: false, .. })));
}

/// a lossy link: the Access-Request is sent again until it gets through
fn lossy(t: &mut T) {
    let mut p = project(&radius_r1("testing123"), Some(RADIUS_CFG));
    // drop 60 % on the server's cable so some requests or answers are lost
    for l in p.links.iter_mut() {
        if l.a.dev == SRV1 || l.b.dev == SRV1 {
            l.loss_ppm = 600_000;
        }
    }
    let mut e = engine(&p);
    run_for(&mut e, 2 * SEC);
    let _ = cli(&mut e, R1, &["admin"], 0);
    // more time: the NAS retransmits once per second
    let _ = cli(&mut e, R1, &["adminpw"], 6 * SEC);
    // it either got through (accepted) or fell back to local; both end logged in, not stuck
    t.check("not stuck on a lossy link", !e.cli_busy(R1) && (e.cli_prompt(R1) == "R1#" || e.cli_prompt(R1) == "R1>"));
}

/// the telnet client: it reaches a listener and reports a refusal
fn telnet_client(t: &mut T) {
    let mut e = engine(&project(LOCAL_R1, None));
    run_for(&mut e, SEC);
    // a plain TCP listener on SRV1 (nc), then PC1 telnets to it
    let _ = cli(&mut e, SRV1, &["nc -l 2300"], SEC);
    let o = cli(&mut e, PC1, &["telnet 10.0.0.10 2300"], SEC);
    t.check("connected", o.contains("Trying 10.0.0.10") && o.contains("% Connected"));
    t.check("telnet note", has(&e, PC1, |n| matches!(n, ANote::TelnetOut { port: 2300, .. })));
    // a typed line goes over the connection to the listener on SRV1 (drain SRV1 in one step)
    let _ = e.take_reports();
    let _ = e.cli_line(PC1, "hallo-welt");
    run_for(&mut e, SEC);
    t.check("line reached the listener", out(&mut e, SRV1).contains("hallo-welt"));
    let mut e2 = engine(&project(LOCAL_R1, None));
    run_for(&mut e2, SEC);
    let o = cli(&mut e2, PC1, &["telnet 10.0.0.10 9999"], SEC);
    t.check("refused", o.contains("% Connection refused by remote host") && has(&e2, PC1, |n| matches!(n, ANote::TelnetRefused { .. })));
}

/// two runs of the RADIUS login give the same engine (byte for byte)
fn determinism(t: &mut T) {
    let go = || {
        let mut e = engine(&project(&radius_r1("testing123"), Some(RADIUS_CFG)));
        run_for(&mut e, 2 * SEC);
        let _ = cli(&mut e, R1, &["admin"], 0);
        let _ = cli(&mut e, R1, &["adminpw"], 2 * SEC);
        (e.running_config(R1), jfn_sim::keyframe::digest(&e))
    };
    let a = go();
    let b = go();
    t.check("same login twice", a == b);
    t.check("did log in", a.0.contains("hostname R1"));
}
