// sessiontest -- checks of the command-line sessions and the telnet servers (group "sessions"
// of --selftest: jfn_sim::cli::ses, telnetd, the vty parts of cli/aaa.rs and cli/ios.rs): two
// local sessions with their own mode, working directory, variables and jobs, a closed session,
// power off, telnet from PC1 to R1 with a line password, with login local and with AAA and
// RADIUS, the vty line limit, the idle timeout in virtual time, show users and show line, exit,
// clear line, terminal monitor, transport input, a Linux telnetd session with who, w and tty,
// ps's TTY column, the replay of the sessions' inputs (K1) and determinism; the console as before
//
// The LAN (device ids in order, as in aaatest.rs): R1 (JF-R4, Gi0/0/0 10.0.0.1/24), SW1
// (switch), SRV1 (server-1u, 10.0.0.10: the RADIUS server and telnetd with the password
// "geheim"), PC1 (PC, 10.0.0.20).  The RADIUS server knows the client 10.0.0.0/24 with the
// secret "testing123" and the users admin (adminpw, privilege 15) and bob (bobpw, privilege 1).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::{ANote, Action, Cmd, Engine, Inline, Note, Report, SesKind, MS, SEC};

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

/// the LAN; `r1` is R1's startup configuration
fn project(r1: &str) -> Project {
    let mut p = Project::new("sessions", 11);
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
        d.services = alloc::vec![def("radius-server", "radius", RADIUS_CFG), def("telnetd", "telnetd", r#"{"password": "geheim"}"#)];
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

/// what the console of a device printed since the last call
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

/// lines typed at a device's console (each followed by `d` of virtual time; no line: `d` only):
/// what it printed
fn cli(e: &mut Engine, dev: u32, lines: &[&str], d: u64) -> String {
    let _ = e.take_reports();
    let mut o = String::new();
    if lines.is_empty() {
        run_for(e, d);
    }
    for l in lines {
        let _ = e.cli_line(dev, l);
        run_for(e, d);
        o.push_str(&out(e, dev));
    }
    if lines.is_empty() {
        o.push_str(&out(e, dev));
    }
    o
}

/// lines typed in session `ses` of a device: what it printed (Report::SesOut)
fn ses(e: &mut Engine, dev: u32, sid: u32, lines: &[&str], d: u64) -> String {
    let _ = e.take_reports();
    if lines.is_empty() {
        run_for(e, d);
    }
    for l in lines {
        let _ = e.cli_ses_line(dev, sid, l);
        run_for(e, d);
    }
    let mut o = String::new();
    for r in e.take_reports() {
        if let Report::SesOut { dev: x, ses: y, bytes, .. } = r
            && x == dev
            && y == sid
        {
            o.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    o
}

/// the AAA notes a device made
fn anotes(e: &Engine, dev: u32) -> Vec<ANote> {
    e.trace.events().filter(|x| x.dev == dev).filter_map(|x| if let Note::Aaa(n) = &x.note { Some(n.clone()) } else { None }).collect()
}

fn has<F: Fn(&ANote) -> bool>(e: &Engine, dev: u32, f: F) -> bool {
    anotes(e, dev).iter().any(f)
}

/// the vty sessions of a device
fn vtys(e: &Engine, dev: u32) -> usize {
    e.cli_sessions(dev).iter().filter(|x| x.kind == SesKind::Telnet).count()
}

// R1 with an address and the vty lines' settings
fn r1_with(lines: &str) -> String {
    format!("hostname R1\n!\ninterface GigabitEthernet0/0/0\n ip address 10.0.0.1 255.255.255.0\n no shutdown\n!\n{}!\nend\n", lines)
}

const R1_PW: &str = "line vty 0 4\n password cisco\n login\n";

pub(crate) fn run(t: &mut T) {
    local_sessions(t);
    ios_local(t);
    closed(t);
    power_off(t);
    telnet_password(t);
    telnet_local(t);
    telnet_radius(t);
    line_limit(t);
    idle_timeout(t);
    show_users(t);
    monitor(t);
    transport(t);
    linux_telnetd(t);
    ps_tty(t);
    replay(t);
    determinism(t);
}

/// two local sessions of PC1 next to its console: their own directory, variables, history and
/// jobs; the console as before
fn local_sessions(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    let a = e.cli_open(PC1);
    let b = e.cli_open(PC1);
    t.eq("two sessions", (a, b), (Some(1), Some(2)));
    let (a, b) = (a.unwrap_or(1), b.unwrap_or(2));
    t.eq("their lines", e.cli_sessions(PC1).iter().map(|x| x.line.clone()).collect::<Vec<_>>(), alloc::vec![s("tty1"), s("pts/0"), s("pts/1")]);
    t.eq("a prompt of its own", e.cli_ses_prompt(PC1, a), s("root@PC1:~# "));
    let _ = ses(&mut e, PC1, a, &["mkdir -p /tmp/x", "cd /tmp/x", "export WO=hier"], 0);
    let _ = ses(&mut e, PC1, b, &["cd /etc"], 0);
    t.eq("working directories", (e.cli_cwd(PC1), e.cli_ses_cwd(PC1, a), e.cli_ses_cwd(PC1, b)), (s("/root"), s("/tmp/x"), s("/etc")));
    t.eq("the prompt follows", e.cli_ses_prompt(PC1, a), s("root@PC1:/tmp/x# "));
    t.eq("a variable of one session", (ses(&mut e, PC1, a, &["echo $WO"], 0), ses(&mut e, PC1, b, &["echo [$WO]"], 0), cli(&mut e, PC1, &["echo [$WO]"], 0)), (s("hier\n"), s("[]\n"), s("[]\n")));
    t.check("its own history", ses(&mut e, PC1, a, &["history"], 0).contains("cd /tmp/x") && !cli(&mut e, PC1, &["history"], 0).contains("cd /tmp/x"));
    // a job of one session runs while the console and the other session take lines
    let o = ses(&mut e, PC1, a, &["sleep 3 && echo fertig &"], 0);
    t.check("[1] PID in its session", o.starts_with("[1] "));
    t.eq("the other sessions have no jobs", (ses(&mut e, PC1, b, &["jobs"], 0), cli(&mut e, PC1, &["jobs"], 0)), (s(""), s("")));
    t.check("its jobs", ses(&mut e, PC1, a, &["jobs"], 0).contains("Running"));
    let fg = ses(&mut e, PC1, b, &["sleep 1"], 0);
    t.check("a command holds its session only", fg.is_empty() && e.cli_ses_busy(PC1, b) && !e.cli_busy(PC1) && !e.cli_ses_busy(PC1, a));
    let o = ses(&mut e, PC1, a, &[], 4 * SEC);
    t.check("the job's output in its session", o.contains("fertig"));
    t.check("done", !e.cli_ses_busy(PC1, b));
    t.eq("$$ of each shell", (cli(&mut e, PC1, &["echo $$"], 0), ses(&mut e, PC1, a, &["echo $$"], 0) != "100\n"), (s("100\n"), true));
    t.eq("tty", (cli(&mut e, PC1, &["tty"], 0), ses(&mut e, PC1, b, &["tty"], 0)), (s("/dev/tty1\n"), s("/dev/pts/1\n")));
    // the console is the console: its prompt report as before, the sessions' their own
    let _ = e.take_reports();
    let _ = e.cli_ses_line(PC1, a, "cd /");
    let rep = e.take_reports();
    t.check("a session's prompt report", rep.iter().any(|r| matches!(r, Report::SesPrompt { dev: PC1, ses, prompt, .. } if *ses == a && prompt == "root@PC1:/# ")) && !rep.iter().any(|r| matches!(r, Report::CliState { .. } | Report::CliOut { .. })));
}

/// a local session of R1: a mode of its own ("tty 1"), show users
fn ios_local(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    let Some(a) = e.cli_open(R1) else {
        t.check("R1 opens a session", false);
        return;
    };
    let _ = ses(&mut e, R1, a, &["enable", "configure terminal", "interface Gi0/0/0"], 0);
    t.eq("modes apart", (e.cli_prompt(R1), e.cli_ses_prompt(R1, a)), (s("R1>"), s("R1(config-if)#")));
    let h = e.cli_ses_help(R1, a, "shut");
    t.check("help in the session's mode", h.contains("shutdown"));
    t.eq("Tab in the session's mode", e.cli_ses_complete(R1, a, "desc").0, Some(s("description ")));
    let _ = e.take_reports();
    let _ = e.cli_ses_line(R1, a, "end");
    let o = out(&mut e, R1);
    t.check("the console hears where the configuration came from", o.contains("%SYS-5-CONFIG_I: configuration changed at tty 1"));
    let u = ses(&mut e, R1, a, &["show users"], 0);
    t.check("show users: both, the asking one marked", u.contains("    con 0 ") && u.contains("*   tty 1 "));
    let _ = ses(&mut e, R1, a, &["exit"], 0);
    t.check("exit ends it", e.cli_sessions(R1).len() == 1);
}

/// a session that is closed: its job ends, it is gone, lines are refused
fn closed(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    let a = e.cli_open(PC1).unwrap_or(1);
    let _ = ses(&mut e, PC1, a, &["ping 10.0.0.10"], SEC);
    t.check("a ping runs in it", e.cli_ses_busy(PC1, a));
    let _ = e.take_reports();
    e.cli_close(PC1, a);
    let rep = e.take_reports();
    t.check("closed: the report", rep.iter().any(|r| matches!(r, Report::SesClosed { dev: PC1, ses } if *ses == a)));
    t.eq("gone", e.cli_sessions(PC1).len(), 1);
    t.check("lines refused", e.cli_ses_line(PC1, a, "ls").is_err());
    run_for(&mut e, 3 * SEC);
    t.check("its ping printed nothing more", !e.take_reports().iter().any(|r| matches!(r, Report::SesOut { .. })));
    // exit in a shell session ends it at the same moment
    let b = e.cli_open(PC1).unwrap_or(2);
    t.eq("ids are not used again", b, 2);
    let o = ses(&mut e, PC1, b, &["exit"], 0);
    t.check("exit", o == "logout\n" && e.cli_sessions(PC1).len() == 1);
    t.check("exit at the console keeps it", cli(&mut e, PC1, &["exit", "pwd"], 0) == "/root\n");
}

/// power off ends every session but the console, which starts afresh
fn power_off(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    let a = e.cli_open(PC1).unwrap_or(1);
    let _ = ses(&mut e, PC1, a, &["sleep 30 &"], 0);
    let _ = cli(&mut e, PC1, &["cd /etc"], 0);
    let _ = e.apply(Action::Power { dev: PC1, on: false });
    t.check("no session while off", e.cli_open(PC1).is_none() && e.cli_sessions(PC1).len() == 1);
    let _ = e.apply(Action::Power { dev: PC1, on: true });
    run_for(&mut e, SEC);
    t.eq("the console afresh", (e.cli_cwd(PC1), e.cli_sessions(PC1).len()), (s("/root"), 1));
    t.eq("a new session after power on", e.cli_open(PC1), Some(2));
}

/// telnet from PC1 to R1 with the line password: the prompts, hidden input, show users, exit
fn telnet_password(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_PW)));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["telnet 10.0.0.1"], SEC);
    t.check("connected, asks for the password", o.contains("% Connected") && o.ends_with("Password: "));
    t.check("the far end hides the password", e.cli_secret(PC1) && e.cli_reads(PC1));
    t.check("a vty session on R1", vtys(&e, R1) == 1 && has(&e, R1, |n| matches!(n, ANote::VtyOpen { line: 0, .. })));
    let o = cli(&mut e, PC1, &["falsch"], SEC);
    t.check("a wrong password", o.contains("% Access denied") && o.ends_with("Password: "));
    let o = cli(&mut e, PC1, &["cisco"], SEC);
    t.eq("logged in: the user level", o, s("R1>"));
    t.check("echo again", !e.cli_secret(PC1));
    let o = cli(&mut e, PC1, &["show users"], SEC);
    t.check("show users over telnet", o.contains("    con 0 ") && o.contains("*   vty 0 ") && o.contains("10.0.0.20") && o.ends_with("R1>"));
    let u = cli(&mut e, R1, &["show users"], 0);
    t.check("show users at the console", u.contains("*   con 0 ") && u.contains("    vty 0 ") && u.contains("10.0.0.20"));
    let l = cli(&mut e, R1, &["show line"], 0);
    t.check("show line", l.contains("vty 0      in use   all        00:10:00   10.0.0.20") && l.contains("vty 4      free"));
    let o = cli(&mut e, PC1, &["show ip ?"], SEC);
    t.check("? over telnet", o.contains("interface") && o.contains("route") && o.ends_with("R1>"));
    let o = cli(&mut e, PC1, &["exit"], SEC);
    t.check("exit closes the connection", o.contains("% Connection closed by foreign host") && !e.cli_busy(PC1));
    t.check("the session is gone", vtys(&e, R1) == 0 && has(&e, R1, |n| matches!(n, ANote::VtyClosed { line: 0, idle: false })));
    t.eq("the console's prompt again", e.cli_prompt(PC1), s("root@PC1:~# "));
    // a device without a password on its lines refuses at once
    let mut f = engine(&project(&r1_with("")));
    run_for(&mut f, SEC);
    let o = cli(&mut f, PC1, &["telnet 10.0.0.1"], SEC);
    t.check("no password, no telnet", o.contains("% Connection refused by remote host"));
}

/// login local on the vty lines: user names, privilege 15, three wrong tries close it
fn telnet_local(t: &mut T) {
    let mut e = engine(&project(&r1_with("username admin privilege 15 secret adminpw\nusername bob secret bobpw\n!\nline vty 0 4\n login local\n")));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["telnet 10.0.0.1"], SEC);
    t.check("asks for the user", o.ends_with("Username: ") && !e.cli_secret(PC1));
    let o = cli(&mut e, PC1, &["admin", "adminpw"], SEC);
    t.check("admin at privilege 15", o.ends_with("R1#") && has(&e, R1, |n| matches!(n, ANote::LoginOk { at: jfn_sim::aaatrace::Where::Vty(0), level: 15, .. })));
    let o = cli(&mut e, PC1, &["logout"], SEC);
    t.check("logout", o.contains("% Connection closed by foreign host"));
    let _ = cli(&mut e, PC1, &["telnet 10.0.0.1"], SEC);
    let o = cli(&mut e, PC1, &["x", "y", "x", "y", "x", "y"], SEC);
    t.check("three wrong tries close it", o.contains("% Access denied") && o.contains("% Connection closed by foreign host") && vtys(&e, R1) == 0);
    t.check("denied note", has(&e, R1, |n| matches!(n, ANote::LoginDenied { at: jfn_sim::aaatrace::Where::Vty(0) })));
}

/// the vty login through AAA with the RADIUS server
fn telnet_radius(t: &mut T) {
    let r1 = r1_with("aaa new-model\n!\nradius server S1\n address ipv4 10.0.0.10 auth-port 1812 acct-port 1813\n key testing123\n timeout 1\n retransmit 1\n!\naaa authentication login default group radius local\naaa authorization exec default group radius local\n!\nline vty 0 4\n login authentication default\n");
    let mut e = engine(&project(&r1));
    run_for(&mut e, 2 * SEC);
    let _ = cli(&mut e, PC1, &["telnet 10.0.0.1"], SEC);
    let o = cli(&mut e, PC1, &["admin", "adminpw"], 2 * SEC);
    t.check("admin via RADIUS", o.ends_with("R1#") && has(&e, R1, |n| matches!(n, ANote::RadiusAccept { level: Some(15), .. })));
    t.check("the login was the vty line's", has(&e, R1, |n| matches!(n, ANote::LoginOk { at: jfn_sim::aaatrace::Where::Vty(0), how, .. } if how == "RADIUS")));
    t.eq("the server counted it", e.metric(SRV1, "radius", "accepts"), Some(1));
    // the console's own login is not touched
    t.eq("the console as before", e.cli_prompt(R1), s("R1>"));
}

/// five vty lines: a sixth connection is refused; PC1's console and five local sessions telnet
fn line_limit(t: &mut T) {
    let mut e = engine(&project(&r1_with("line vty 0 4\n no login\n")));
    run_for(&mut e, SEC);
    let first = cli(&mut e, PC1, &["telnet 10.0.0.1"], SEC);
    let mut ids = Vec::new();
    for _ in 0..5 {
        ids.push(e.cli_open(PC1).unwrap_or(0));
    }
    let mut outs = Vec::new();
    for &sid in &ids {
        outs.push(ses(&mut e, PC1, sid, &["telnet 10.0.0.1"], SEC));
    }
    t.check("five got in without a login", first.ends_with("R1>") && outs[..4].iter().all(|o| o.ends_with("R1>")));
    t.eq("five vty sessions", vtys(&e, R1), 5);
    t.check("the sixth is refused", outs[4].contains("% No free vty line, try again later") && outs[4].contains("% Connection closed by foreign host"));
    t.check("refused note", has(&e, R1, |n| matches!(n, ANote::VtyRefused { .. })));
    let u = cli(&mut e, R1, &["show users"], 0);
    t.check("show users lists vty 0 to 4", (0..5).all(|n| u.contains(&format!("    vty {} ", n))));
    // clear line frees one
    let o = cli(&mut e, R1, &["enable", "clear line vty 2"], SEC);
    t.check("clear line", o.contains("[OK]") && vtys(&e, R1) == 4);
    let o = ses(&mut e, PC1, ids[4], &["telnet 10.0.0.1"], SEC);
    t.check("a line is free again", o.ends_with("R1>") && vtys(&e, R1) == 5);
}

/// exec-timeout: a forgotten session ends after its idle time in virtual time
fn idle_timeout(t: &mut T) {
    let mut e = engine(&project(&r1_with("line vty 0 4\n password cisco\n login\n exec-timeout 0 30\n")));
    run_for(&mut e, SEC);
    let _ = cli(&mut e, PC1, &["telnet 10.0.0.1", "cisco"], SEC);
    t.eq("in", vtys(&e, R1), 1);
    let o = cli(&mut e, PC1, &[], 20 * SEC);
    t.check("still there after 20 s", o.is_empty() && vtys(&e, R1) == 1);
    let _ = cli(&mut e, PC1, &["show users"], SEC);
    let o = cli(&mut e, PC1, &[], 25 * SEC);
    t.check("a line typed keeps it", vtys(&e, R1) == 1 && o.is_empty());
    let o = cli(&mut e, PC1, &[], 10 * SEC);
    t.check("idle 30 s: closed", o.contains("% No input for 00:00:30: the session ends") && o.contains("% Connection closed by foreign host") && vtys(&e, R1) == 0);
    t.check("idle note", has(&e, R1, |n| matches!(n, ANote::VtyClosed { idle: true, .. })));
    // the default is 10 minutes
    let mut f = engine(&project(&r1_with(R1_PW)));
    run_for(&mut f, SEC);
    let _ = cli(&mut f, PC1, &["telnet 10.0.0.1", "cisco"], SEC);
    run_for(&mut f, 9 * 60 * SEC);
    t.eq("after 9 minutes", vtys(&f, R1), 1);
    run_for(&mut f, 2 * 60 * SEC);
    t.eq("after 11 minutes", vtys(&f, R1), 0);
}

/// show users with the idle times, from the console and from the session
fn show_users(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_PW)));
    run_for(&mut e, SEC);
    let _ = cli(&mut e, PC1, &["telnet 10.0.0.1", "cisco"], SEC);
    run_for(&mut e, 65 * SEC);
    let u = cli(&mut e, R1, &["show users"], 0);
    t.check("idle time of the vty session", u.lines().any(|l| l.starts_with("    vty 0 ") && l.contains("00:01:0") && l.ends_with("10.0.0.20")));
    let v = e.cli_sessions(R1);
    t.check("the sessions' view", v.len() == 2 && v[1].from.is_some() && v[1].idle >= 65 * SEC && v[1].line == "vty 0");
}

/// terminal monitor: console messages reach a vty session only with it; a vty session's
/// configuration is named in the console's message
fn monitor(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_PW)));
    run_for(&mut e, SEC);
    let _ = cli(&mut e, PC1, &["telnet 10.0.0.1", "cisco", "enable"], SEC);
    let _ = cli(&mut e, R1, &["enable", "configure terminal", "end"], 0);
    let o = cli(&mut e, PC1, &[], SEC);
    t.check("no monitor: nothing", !o.contains("%SYS-5-CONFIG_I"));
    let _ = cli(&mut e, PC1, &["terminal monitor"], SEC);
    let _ = cli(&mut e, R1, &["configure terminal", "end"], 0);
    let o = cli(&mut e, PC1, &[], SEC);
    t.check("monitor: the console's message", o.contains("%SYS-5-CONFIG_I: configuration changed at the console"));
    let _ = cli(&mut e, PC1, &["terminal no monitor"], SEC);
    let _ = cli(&mut e, R1, &["configure terminal", "end"], 0);
    t.check("no monitor again", !cli(&mut e, PC1, &[], SEC).contains("%SYS-5-CONFIG_I"));
    let _ = e.take_reports();
    for l in ["configure terminal", "hostname R1x", "end"] {
        let _ = e.cli_line(PC1, l);
        run_for(&mut e, SEC);
    }
    let c = out(&mut e, R1);
    t.check("the console names the vty line", c.contains("%SYS-5-CONFIG_I: configuration changed from vty 0 (10.0.0.20)"));
    t.check("the change took", e.running_config(R1).contains("hostname R1x"));
}

/// transport input none refuses telnet; access-class lets only some in
fn transport(t: &mut T) {
    let mut e = engine(&project(&r1_with(R1_PW)));
    run_for(&mut e, SEC);
    let _ = cli(&mut e, R1, &["enable", "configure terminal", "line vty 0 4", "transport input none", "end"], 0);
    t.check("written", e.running_config(R1).contains("line vty 0 4\n password cisco\n login\n transport input none\n"));
    let o = cli(&mut e, PC1, &["telnet 10.0.0.1"], SEC);
    t.check("transport input none: refused", o.contains("% Connection refused by remote host"));
    let _ = cli(&mut e, R1, &["configure terminal", "line vty 0 4", "transport input ssh telnet", "end"], 0);
    let o = cli(&mut e, PC1, &["telnet 10.0.0.1"], SEC);
    t.check("telnet again", o.ends_with("Password: "));
    let _ = e.cli_interrupt(PC1);
    run_for(&mut e, SEC);
    let _ = cli(&mut e, R1, &["configure terminal", "access-list 5 permit host 10.0.0.99", "line vty 0 4", "access-class 5 in", "end"], 0);
    let o = cli(&mut e, PC1, &["telnet 10.0.0.1"], SEC);
    t.check("access-class: not from PC1", o.contains("% Connection closed by foreign host") && has(&e, R1, |n| matches!(n, ANote::VtyRefused { why, .. } if why == "access-class 5")));
}

/// telnetd on SRV1: the login as root, a shell on pts/0, who, w, tty, exit
fn linux_telnetd(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["telnet 10.0.0.10"], SEC);
    t.check("telnetd asks for the login", o.ends_with("SRV1 login: "));
    let o = cli(&mut e, PC1, &["root", "falsch"], SEC);
    t.check("a wrong password", o.contains("Login incorrect") && o.ends_with("SRV1 login: "));
    let o = cli(&mut e, PC1, &["root", "geheim"], SEC);
    t.check("a shell", o.ends_with("root@SRV1:~# "));
    let o = cli(&mut e, PC1, &["who"], SEC);
    t.check("who: the console and pts/0", o.contains("root     tty1         2026-01-01 00:00\n") && o.contains("root     pts/0        2026-01-01 00:00 (10.0.0.20)\n"));
    t.check("tty", cli(&mut e, PC1, &["tty"], SEC).starts_with("/dev/pts/0\n"));
    let w = cli(&mut e, PC1, &["w"], SEC);
    t.check("w", w.contains("2 users,  load average") && w.contains("USER     TTY      FROM") && w.contains("pts/0    10.0.0.20") && w.contains(" w\n"));
    let o = cli(&mut e, PC1, &["cd /tmp", "pwd"], SEC);
    t.check("its own working directory", o.contains("/tmp\n") && e.cli_cwd(SRV1) == "/root");
    t.check("SRV1 has the session", e.cli_sessions(SRV1).len() == 2);
    let o = cli(&mut e, PC1, &["exit"], SEC);
    t.check("exit", o.contains("logout") && o.contains("% Connection closed by foreign host") && e.cli_sessions(SRV1).len() == 1);
    t.check("telnetd's numbers", e.metric(SRV1, "telnetd", "accepted") == Some(1));
}

/// ps: every session's shell and jobs with their terminal
fn ps_tty(t: &mut T) {
    let mut e = engine(&project(&r1_with("")));
    run_for(&mut e, SEC);
    let a = e.cli_open(PC1).unwrap_or(1);
    let _ = ses(&mut e, PC1, a, &["sleep 30 &"], 0);
    let ps = cli(&mut e, PC1, &["ps"], 0);
    let lines: Vec<&str> = ps.lines().collect();
    t.eq("the head", lines.first().copied(), Some("  PID USER     TTY      TIME COMMAND"));
    t.check("init and the console's shell", lines.get(1) == Some(&"    1 root     ?        0:00 init") && lines.get(2) == Some(&"  100 root     tty1     0:00 -sh"));
    t.check("the session's shell and job", lines.iter().any(|l| l.ends_with(" root     pts/0    0:00 -sh")) && lines.iter().any(|l| l.ends_with(" root     pts/0    0:00 sleep 30")));
    t.check("ps itself on tty1", lines.last().is_some_and(|l| l.ends_with(" root     tty1     0:00 ps")));
    let ps2 = ses(&mut e, PC1, a, &["ps"], 0);
    t.check("from the session: ps on pts/0", ps2.lines().last().is_some_and(|l| l.ends_with(" root     pts/0    0:00 ps")));
}

/// the sessions' inputs as commands of the driver replay with the keyframes (K1)
fn replay(t: &mut T) {
    let mut d = Inline::new(engine(&project(&r1_with(R1_PW))));
    d.rewind_on(jfn_sim::Budget::default());
    let _ = d.send(Cmd::RunFor(SEC), 0);
    let a = d.open_session(PC1, 0).unwrap_or(0);
    let _ = d.send(Cmd::SesLine { dev: PC1, ses: a, line: s("cd /tmp") }, 0);
    let _ = d.send(Cmd::SesLine { dev: PC1, ses: a, line: s("telnet 10.0.0.1") }, 0);
    let _ = d.send(Cmd::RunFor(500 * MS), 0);
    let _ = d.send(Cmd::SesLine { dev: PC1, ses: a, line: s("cisco") }, 0);
    let _ = d.send(Cmd::RunFor(SEC), 0);
    let at = d.pos();
    let want = jfn_sim::keyframe::digest(&d.eng);
    let _ = d.send(Cmd::RunFor(SEC), 0);
    let got = d.restore_pos(at, None).and_then(|mut r| {
        let rw = d.rewind()?;
        let done = (0..1000).any(|_| r.run(rw, 1_000_000));
        if done { Some(r.finish()) } else { None }
    });
    t.check("a replay brings the sessions back", got.as_ref().is_some_and(|p| jfn_sim::keyframe::digest(&p.eng) == want && p.eng.cli_ses_cwd(PC1, a) == "/tmp" && vtys(&p.eng, R1) == 1));
    let _ = d.send(Cmd::SesClose { dev: PC1, ses: a }, 0);
    t.eq("closed by command", d.eng.cli_sessions(PC1).len(), 1);
}

/// two runs of a telnet session give the same engine, byte for byte
fn determinism(t: &mut T) {
    let go = || {
        let mut e = engine(&project(&r1_with(R1_PW)));
        run_for(&mut e, SEC);
        let a = e.cli_open(PC1).unwrap_or(1);
        let _ = ses(&mut e, PC1, a, &["telnet 10.0.0.1", "cisco", "show users"], SEC);
        let o = cli(&mut e, PC1, &["telnet 10.0.0.10", "root", "geheim", "who"], SEC);
        (o, jfn_sim::keyframe::digest(&e))
    };
    let a = go();
    let b = go();
    t.check("same sessions twice", a == b);
    t.check("they did run", a.0.contains("pts/0"));
}
