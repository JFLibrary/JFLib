// mailtest -- checks of mail (group mail of --selftest: jfn_sim::smtpd, pop3d, mailc, mailbox,
// mailtrace, cli/mail.rs, the resolver's /etc/resolv.conf): a local delivery and a relay between
// two domains by their MX records on the nanosecond, the Received fields, a bounce for an
// unknown user and for domains without a mail server, retries while the next server is off
// and giving up, the E-Mail program's fetch with UIDL and DELE (and with the messages left on
// the server), curl pop3:// with LIST, RETR, UIDL and TOP, a wrong password at POP3 and at AUTH,
// the submission port with AUTH, the size limit (announced and sent), relaying refused and
// the open relay, the problems list, the project round trip with the mailboxes and determinism
//
// The network (device ids in order): R1 (JF-R4, Gi0/0/0 10.0.1.1/24 a.lab, Gi0/0/1 10.0.2.1/24
// b.lab), SW1 and SW2, MAILA (10.0.1.25: smtp-server of a.lab with anna, pop3-server), DNS1
// (10.0.1.53: the zones a.lab and b.lab with their MX records), PC1 (10.0.1.10, the E-Mail
// program's account of anna), MAILB (10.0.2.25: smtp-server of b.lab with bob, pop3-server),
// PC2 (10.0.2.10).  Every host asks DNS1 (echo "nameserver 10.0.1.53" > /etc/resolv.conf).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::files::File;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::mailc::{self, Account, MailDraft};
use jfn_sim::{Action, Engine, MNote, Note, ProblemKind, Report, SEC};
use netpkt::imf::Message;

use crate::selftest::T;

const BUDGET: usize = 8_000_000;
const R1: u32 = 1;
const MAILA: u32 = 4;
const DNS1: u32 = 5;
const PC1: u32 = 6;
const MAILB: u32 = 7;
const PC2: u32 = 8;

fn s(x: &str) -> String {
    String::from(x)
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: s(kind), name: s(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Value::Null) }
}

const ACCOUNT_ANNA: &str = "name = Anna Muster\naddress = anna@a.lab\nsmtp = mail.a.lab\nsmtp_port = 25\npop3 = mail.a.lab\npop3_port = 110\nuser = anna\npassword = geheim\nkeep = no\n";
const ACCOUNT_BOB: &str = "name = Bob\naddress = bob@b.lab\nsmtp = mail.b.lab\nsmtp_port = 587\npop3 = mail.b.lab\npop3_port = 110\nuser = bob\npassword = geheim\nkeep = no\n";
const SMTP_A: &str = r#"{"hostname": "mail.a.lab", "domains": ["a.lab"], "users": [{"name": "anna", "password": "geheim"}], "retry_ms": 10000, "retry_max_ms": 40000, "give_up_ms": 60000, "connect_timeout_ms": 3000}"#;
const SMTP_B: &str = r#"{"hostname": "mail.b.lab", "domains": ["b.lab"], "users": [{"name": "bob", "password": "geheim"}]}"#;
const POP_A: &str = r#"{"users": [{"name": "anna", "password": "geheim"}]}"#;
const POP_B: &str = r#"{"users": [{"name": "bob", "password": "geheim"}]}"#;
const ZONES: &str = r#"{"zones": [{"origin": "a.lab", "records": ["@ MX 10 mail", "mail A 10.0.1.25", "pc1 A 10.0.1.10"]}, {"origin": "b.lab", "records": ["@ MX 10 mail", "mail A 10.0.2.25", "pc2 A 10.0.2.10"]}, {"origin": "c.lab", "records": ["@ MX 10 mx"]}, {"origin": "lab", "records": []}]}"#;

fn host(name: &str, ip: &str, gw: &str) -> String {
    format!("hostname {}\nip addr add {}/24 dev eth0\nip route add default via {}\necho \"nameserver 10.0.1.53\" > /etc/resolv.conf\n", name, ip, gw)
}

fn account_files(text: &str) -> Vec<File> {
    alloc::vec![File { path: s("/root/Mail"), data: Vec::new(), mtime: 1_767_225_600, dir: true }, File { path: s(mailc::ACCOUNT), data: Vec::from(text.as_bytes()), mtime: 1_767_225_600, dir: false }]
}

/// the network of the header comment; `smtp_a` the settings of MAILA's SMTP server
fn project(smtp_a: &str) -> Project {
    let mut p = Project::new("mail", 23);
    for (i, m) in ["jf-r4", "jf-s24p", "jf-s24p", "server-1u", "server-1u", "pc", "server-1u", "pc"].iter().enumerate() {
        let _ = p.add_device(m, i as i32 * 120, 0);
    }
    let pr = |dev: u32, port: &str| PortRef { dev, port: s(port) };
    let _ = p.connect(&pr(R1, "gi0/0/0"), &pr(2, "gi1/0/1"), None);
    let _ = p.connect(&pr(R1, "gi0/0/1"), &pr(3, "gi1/0/1"), None);
    let _ = p.connect(&pr(MAILA, "eth0"), &pr(2, "gi1/0/2"), None);
    let _ = p.connect(&pr(DNS1, "eth0"), &pr(2, "gi1/0/3"), None);
    let _ = p.connect(&pr(PC1, "eth0"), &pr(2, "gi1/0/4"), None);
    let _ = p.connect(&pr(MAILB, "eth0"), &pr(3, "gi1/0/2"), None);
    let _ = p.connect(&pr(PC2, "eth0"), &pr(3, "gi1/0/3"), None);
    let r1 = "hostname R1\n!\ninterface GigabitEthernet0/0/0\n ip address 10.0.1.1 255.255.255.0\n no shutdown\n!\ninterface GigabitEthernet0/0/1\n ip address 10.0.2.1 255.255.255.0\n no shutdown\n!\nend\n";
    let cfg = [
        (R1, "R1", String::from(r1)),
        (2, "SW1", s("hostname SW1\n!\nend\n")),
        (3, "SW2", s("hostname SW2\n!\nend\n")),
        (MAILA, "MAILA", host("MAILA", "10.0.1.25", "10.0.1.1")),
        (DNS1, "DNS1", s("hostname DNS1\nip addr add 10.0.1.53/24 dev eth0\nip route add default via 10.0.1.1\n")),
        (PC1, "PC1", host("PC1", "10.0.1.10", "10.0.1.1")),
        (MAILB, "MAILB", host("MAILB", "10.0.2.25", "10.0.2.1")),
        (PC2, "PC2", host("PC2", "10.0.2.10", "10.0.2.1")),
    ];
    for (id, name, c) in cfg {
        if let Some(d) = p.device_mut(id) {
            d.name = s(name);
            d.startup_config = c;
        }
    }
    if let Some(d) = p.device_mut(MAILA) {
        d.services = alloc::vec![def("smtp-server", "smtp", smtp_a), def("pop3-server", "pop3", POP_A)];
    }
    if let Some(d) = p.device_mut(MAILB) {
        d.services = alloc::vec![def("smtp-server", "smtp", SMTP_B), def("pop3-server", "pop3", POP_B)];
    }
    if let Some(d) = p.device_mut(DNS1) {
        d.services = alloc::vec![def("dns-server", "dns", ZONES)];
    }
    if let Some(d) = p.device_mut(PC1) {
        d.files = account_files(ACCOUNT_ANNA);
    }
    if let Some(d) = p.device_mut(PC2) {
        d.files = account_files(ACCOUNT_BOB);
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

/// lines typed at a device, then `d` of virtual time: what it printed
fn cli(e: &mut Engine, dev: u32, lines: &[&str], d: u64) -> String {
    let _ = e.take_reports();
    for l in lines {
        let _ = e.cli_line(dev, l);
    }
    run_for(e, d);
    let mut o = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: x, bytes, .. } = r
            && x == dev
        {
            o.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    o
}

/// the mail notes of a device with their times
fn mnotes(e: &Engine, dev: u32) -> Vec<(u64, MNote)> {
    e.trace.events().filter(|x| x.dev == dev).filter_map(|x| if let Note::Mail(n) = &x.note { Some((x.t, n.clone())) } else { None }).collect()
}

/// the paths of a device's files below a directory
fn paths(e: &Engine, dev: u32, dir: &str) -> Vec<String> {
    e.files(dev).into_iter().filter(|f| !f.dir && f.path.starts_with(dir)).map(|f| f.path).collect()
}

/// the text of the first file below a directory
fn first_text(e: &Engine, dev: u32, dir: &str) -> String {
    e.files(dev).into_iter().find(|f| !f.dir && f.path.starts_with(dir)).map(|f| String::from_utf8_lossy(&f.data).into_owned()).unwrap_or_default()
}

/// R1 learns the addresses of DNS1 and MAILB first (the IOS-like router drops a packet that
/// waits for ARP, and a lost DNS question costs the resolver's 5 s): an engine 3 s after the
/// start
fn warm(p: &Project) -> Engine {
    let mut e = engine(p);
    run_for(&mut e, SEC);
    let _ = cli(&mut e, PC2, &["ping -c 1 -W 1 10.0.1.53"], 0);
    let _ = cli(&mut e, MAILA, &["ping -c 1 -W 1 10.0.2.25"], 2 * SEC);
    e
}

fn metric(e: &Engine, dev: u32, name: &str) -> i64 {
    e.metric(dev, "smtp", name).unwrap_or(-1)
}

/// the message file at the start of a time: its header field `f`
fn field_of(text: &str, f: &str) -> String {
    Message::parse(text.as_bytes()).get(f).map(String::from).unwrap_or_default()
}

pub(crate) fn run(t: &mut T) {
    resolv(t);
    local(t);
    relay(t);
    bounce_user(t);
    bounce_dns(t);
    retries(t);
    fetch(t);
    curl_pop3(t);
    logins(t);
    size_limit(t);
    relay_rules(t);
    problems(t);
    client(t);
    save_load(t);
    determinism(t);
}

// ---------------------------------------------------------------- the resolver of the hosts

fn resolv(t: &mut T) {
    let mut e = engine(&project(SMTP_A));
    run_for(&mut e, SEC);
    t.eq("static name server", e.resolv_conf(PC1).map(|c| c.servers.iter().map(|a| format!("{}", a)).collect::<Vec<_>>()), Some(alloc::vec![s("10.0.1.53")]));
    let o = cli(&mut e, PC1, &["cat /etc/resolv.conf"], 0);
    t.eq("resolv.conf", o, s("nameserver 10.0.1.53\n"));
    let o = cli(&mut e, PC1, &["echo \"search a.lab\" >> /etc/resolv.conf", "cat /etc/resolv.conf"], 0);
    t.eq("search added", o, s("search a.lab\nnameserver 10.0.1.53\n"));
    t.check("in the configuration", e.running_config(PC1).contains("echo \"nameserver 10.0.1.53\" > /etc/resolv.conf\necho \"search a.lab\" >> /etc/resolv.conf\n"));
    let o = cli(&mut e, PC1, &["echo nameserver nope > /etc/resolv.conf"], 0);
    t.eq("not an address", o, s("jfn-sh: /etc/resolv.conf: nope is no address\n"));
    t.check("no file in the project", e.files(PC1).iter().all(|f| f.path != "/etc/resolv.conf"));
}

// ---------------------------------------------------------------- local delivery

fn local(t: &mut T) {
    let mut e = engine(&project(SMTP_A));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["echo \"Subject: Notiz\" > n.txt", "echo >> n.txt", "echo \"Hallo Anna\" >> n.txt", "curl -v smtp://mail.a.lab --mail-from anna@a.lab --mail-rcpt anna@a.lab -T n.txt"], 2 * SEC);
    t.check("curl -v: the dialogue", o.contains("> EHLO PC1\n") && o.contains("< 250-mail.a.lab welcomes PC1\n") && o.contains("< 250-SIZE 10485760\n") && o.contains("> MAIL FROM:<anna@a.lab> SIZE=") && o.contains("> RCPT TO:<anna@a.lab>\n") && o.contains("< 354 Go ahead") && o.contains("> QUIT\n") && o.contains("< 221 2.0.0 mail.a.lab closes the connection\n"));
    t.check("curl: queued", o.contains("< 250 2.0.0 Queued as "));
    let files = paths(&e, MAILA, "/var/mail/anna/new/");
    t.eq("one message in anna's Maildir", files.len(), 1);
    let text = first_text(&e, MAILA, "/var/mail/anna/new/");
    t.check("Return-Path on top", text.starts_with("Return-Path: <anna@a.lab>\nReceived: from PC1 ([10.0.1.10]) by mail.a.lab with ESMTP id "));
    t.check("for one recipient", field_of(&text, "Received").contains(" for <anna@a.lab>; "));
    t.check("the text with Unix line ends", text.ends_with("Subject: Notiz\n\nHallo Anna\n"));
    t.eq("numbers", (metric(&e, MAILA, "received"), metric(&e, MAILA, "delivered"), metric(&e, MAILA, "relayed"), metric(&e, MAILA, "queued")), (1, 1, 0, 0));
    let notes = mnotes(&e, MAILA);
    let kinds: Vec<&str> = notes.iter().map(|(_, n)| match n {
        MNote::SmtpSession { .. } => "session",
        MNote::SmtpHelo { .. } => "helo",
        MNote::SmtpMail { .. } => "mail",
        MNote::SmtpRcpt { local: true, .. } => "rcpt-local",
        MNote::Queued { .. } => "queued",
        MNote::Delivered { .. } => "delivered",
        _ => "other",
    }).collect();
    t.eq("the notes in order", kinds, alloc::vec!["session", "helo", "mail", "rcpt-local", "queued", "delivered"]);
    let at: Vec<u64> = notes.iter().map(|x| x.0).collect();
    // the session after the SYN; queued after the disk wrote the message (120 µs for its bytes,
    // 100 µs of latency), delivered after the second write into the Maildir
    t.eq("their times", at, alloc::vec![1_000_013_572, 1_000_017_692, 1_000_023_220, 1_000_026_652, 1_000_154_092, 1_000_274_092]);
    t.check("a step of the project", e.files(MAILA).iter().any(|f| f.dir && f.path == "/var/mail/anna/cur"));
}

// ---------------------------------------------------------------- relaying by MX

fn relay(t: &mut T) {
    let mut e = engine(&project(SMTP_A));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["echo \"Subject: Gruss\" > b.txt", "echo >> b.txt", "echo \"Hallo Bob\" >> b.txt", "curl smtp://mail.a.lab --mail-from anna@a.lab --mail-rcpt bob@b.lab -T b.txt"], 3 * SEC);
    t.eq("curl is quiet", o, String::new());
    t.eq("relayed", (metric(&e, MAILA, "relayed"), metric(&e, MAILB, "received"), metric(&e, MAILB, "delivered")), (1, 1, 1));
    let a = mnotes(&e, MAILA);
    t.check("the MX of b.lab", a.iter().any(|(_, n)| matches!(n, MNote::MxFound { domain, hosts } if domain == "b.lab" && hosts == "10 mail.b.lab")));
    t.check("handed over", a.iter().any(|(_, n)| matches!(n, MNote::Relayed { rcpt, host, reply, .. } if rcpt == "bob@b.lab" && host == "mail.b.lab" && reply.starts_with("250 2.0.0 Queued as "))));
    let text = first_text(&e, MAILB, "/var/mail/bob/new/");
    let recv: Vec<String> = Message::parse(text.as_bytes()).get_all("Received").map(String::from).collect();
    t.eq("two Received fields", recv.len(), 2);
    t.check("the newest first", recv.first().is_some_and(|r| r.starts_with("from mail.a.lab ([10.0.1.25]) by mail.b.lab with ESMTP id ")) && recv.get(1).is_some_and(|r| r.starts_with("from PC1 ([10.0.1.10]) by mail.a.lab with ESMTP id ")));
    t.check("pipelined", e.trace.hops().filter(|h| h.from.0 == MAILA).any(|h| {
        let i = netpkt::decode::info(&h.bytes).1;
        i.starts_with("C: MAIL FROM:<anna@a.lab>")
    }) && e.trace.hops().any(|h| h.from.0 == MAILA && h.bytes.windows(14).any(|w| w == b"\r\nRCPT TO:<bob")));
    let at: Vec<(u64, &str)> = mnotes(&e, MAILB).iter().filter_map(|(t, n)| match n {
        MNote::Queued { .. } => Some((*t, "queued")),
        MNote::Delivered { .. } => Some((*t, "delivered")),
        _ => None,
    }).collect();
    // R1 drops the first SYN to MAILB while it asks for MAILB's MAC: the connection comes 1 s
    // later with the SYN again
    t.eq("at MAILB", at, alloc::vec![(2_040_334_092, "queued"), (2_040_454_092, "delivered")]);
}

// ---------------------------------------------------------------- bounces

fn bounce_user(t: &mut T) {
    let mut e = engine(&project(SMTP_A));
    run_for(&mut e, SEC);
    let o = cli(&mut e, PC1, &["echo Hallo | mail -s Test nobody@b.lab"], 5 * SEC);
    t.eq("mail is quiet", o, String::new());
    // MAILB refuses the RCPT and the DATA that came with it (PIPELINING)
    t.eq("bounced", (metric(&e, MAILA, "relayed"), metric(&e, MAILA, "bounced"), metric(&e, MAILA, "delivered"), metric(&e, MAILB, "rejected")), (0, 1, 1, 2));
    let text = first_text(&e, MAILA, "/var/mail/anna/new/");
    let m = Message::parse(text.as_bytes());
    t.eq("from the mail system", m.get("From"), Some("Mail delivery <MAILER-DAEMON@mail.a.lab>"));
    t.eq("its subject", m.subject(), s("Undelivered mail: Test"));
    t.check("the null path", text.starts_with("Return-Path: <>\n"));
    t.check("a report", m.get("Content-Type").is_some_and(|c| c.starts_with("multipart/report; report-type=delivery-status; boundary=")));
    t.check("the text", m.text().contains("<nobody@b.lab>: mail.b.lab said: 550 5.1.1 <nobody@b.lab>: no such user here"));
    t.check("the delivery status", text.contains("\nFinal-Recipient: rfc822; nobody@b.lab\nAction: failed\nStatus: 5.1.1\nRemote-MTA: dns; mail.b.lab\nDiagnostic-Code: smtp; 550 5.1.1 <nobody@b.lab>: no such user here\n"));
    t.check("the original header", text.contains("Content-Type: text/rfc822-headers") && text.contains("Subject: Test\n"));
    let n = mnotes(&e, MAILA);
    t.check("notes", n.iter().any(|(_, x)| matches!(x, MNote::Bounced { rcpt, .. } if rcpt == "nobody@b.lab")) && n.iter().any(|(_, x)| matches!(x, MNote::BounceSent { to, .. } if to == "anna@a.lab")));
}

fn bounce_dns(t: &mut T) {
    let mut e = warm(&project(SMTP_A));
    let _ = cli(&mut e, PC1, &["echo x | mail -s A x@nowhere.lab", "echo y | mail -s B y@c.lab"], 10 * SEC);
    t.eq("two bounces", metric(&e, MAILA, "bounced"), 2);
    let texts: Vec<String> = e.files(MAILA).into_iter().filter(|f| f.path.starts_with("/var/mail/anna/new/")).map(|f| String::from_utf8_lossy(&f.data).into_owned()).collect();
    t.check("a domain DNS does not know", texts.iter().any(|x| x.contains("Status: 5.1.2") && x.contains("<x@nowhere.lab>: the domain nowhere.lab does not exist (DNS)")));
    t.check("an MX without an address", texts.iter().any(|x| x.contains("Status: 5.4.4") && x.contains("<y@c.lab>: the mail servers of c.lab have no address: mx.c.lab")));
}

// ---------------------------------------------------------------- retries

fn retries(t: &mut T) {
    let mut e = warm(&project(SMTP_A));
    let _ = e.apply(Action::Power { dev: MAILB, on: false });
    let _ = cli(&mut e, PC1, &["echo eins | mail -s Spaeter bob@b.lab"], 5 * SEC);
    let deferred = |e: &Engine| -> Vec<u64> { mnotes(e, MAILA).iter().filter(|(_, n)| matches!(n, MNote::Deferred { .. })).map(|x| x.0).collect() };
    // the connection does not come (3 s), the next attempt after 10 s
    t.eq("deferred", deferred(&e).len(), 1);
    t.eq("in the queue", metric(&e, MAILA, "queued"), 1);
    // the next after 20 s (the wait doubles up to retry_max, 40 s)
    run_for(&mut e, 15 * SEC);
    // 3 s after the mail came (the connect timeout), then 10 s and 3 s later
    t.eq("the attempts", deferred(&e), alloc::vec![6_000_169_444, 19_000_169_444]);
    let _ = e.apply(Action::Power { dev: MAILB, on: true });
    run_for(&mut e, 20 * SEC);
    t.eq("delivered after the power came back", (metric(&e, MAILA, "relayed"), metric(&e, MAILA, "queued"), metric(&e, MAILB, "delivered")), (1, 0, 1));
    // giving up: 60 s after the message came
    let mut g = warm(&project(SMTP_A));
    let _ = g.apply(Action::Power { dev: MAILB, on: false });
    let _ = cli(&mut g, PC1, &["echo zwei | mail -s Nie bob@b.lab"], 70 * SEC);
    t.eq("given up", (metric(&g, MAILA, "bounced"), metric(&g, MAILA, "queued")), (1, 0));
    let text = first_text(&g, MAILA, "/var/mail/anna/new/");
    t.check("the bounce names the last reason", text.contains("Status: 4.4.7") && text.contains("<bob@b.lab>: given up after "));
}

// ---------------------------------------------------------------- the E-Mail program fetches

fn fetch(t: &mut T) {
    let mut e = warm(&project(SMTP_A));
    let _ = cli(&mut e, PC1, &["echo eins | mail -s Erste bob@b.lab", "echo zwei | mail -s Zweite bob@b.lab"], 5 * SEC);
    t.eq("two for bob", paths(&e, MAILB, "/var/mail/bob/new/").len(), 2);
    let r = e.apply(Action::MailFetch { dev: PC2 });
    t.check("fetch taken", r.is_ok());
    run_for(&mut e, 3 * SEC);
    let v = e.mail_client(PC2);
    t.check("fetched", v.as_ref().is_some_and(|v| v.busy.is_none() && v.last_fetch.as_ref().is_some_and(|o| o.ok && o.count == 2)));
    t.eq("the status", v.and_then(|v| v.last_fetch).map(|o| o.msg.text()), Some(s("2 neue Nachrichten von mail.b.lab abgerufen.")));
    let inbox = e.file_store(PC2).map(|s| mailc::folder(s, mailc::INBOX)).unwrap_or_default();
    t.eq("the inbox", inbox.iter().map(|m| m.subject.clone()).collect::<Vec<_>>(), alloc::vec![s("Erste"), s("Zweite")]);
    t.eq("deleted on the server (keep = no)", paths(&e, MAILB, "/var/mail/bob/").len(), 0);
    t.check("the UIDLs kept", e.file_store(PC2).and_then(|s| s.read(mailc::UIDLS).ok().map(|b| String::from_utf8_lossy(b).lines().count())) == Some(2));
    let n = mnotes(&e, MAILB);
    t.check("POP3 notes", n.iter().any(|(_, x)| matches!(x, MNote::Pop3Login { user, msgs: 2, .. } if user == "bob")) && n.iter().any(|(_, x)| matches!(x, MNote::Pop3Deleted { n: 2, .. })));
    // again: nothing new
    let _ = e.apply(Action::MailFetch { dev: PC2 });
    run_for(&mut e, 3 * SEC);
    t.eq("nothing new", e.mail_client(PC2).and_then(|v| v.last_fetch).map(|o| (o.ok, o.count, o.msg.text())), Some((true, 0, s("Keine neuen Nachrichten auf mail.b.lab."))));
    // keep: the message stays and is not fetched twice
    let mut k = warm(&project(SMTP_A));
    let _ = k.apply(Action::Files { dev: PC2, files: account_files(&ACCOUNT_BOB.replace("keep = no", "keep = yes")) });
    let _ = cli(&mut k, PC1, &["echo drei | mail -s Dritte bob@b.lab"], 5 * SEC);
    let _ = k.apply(Action::MailFetch { dev: PC2 });
    run_for(&mut k, 3 * SEC);
    let _ = k.apply(Action::MailFetch { dev: PC2 });
    run_for(&mut k, 3 * SEC);
    t.eq("kept on the server", paths(&k, MAILB, "/var/mail/bob/new/").len(), 1);
    t.eq("fetched once", k.file_store(PC2).map(|s| mailc::folder(s, mailc::INBOX).len()), Some(1));
    // sending from the program: port 587 with AUTH, a copy in SENT
    let draft = MailDraft { to: s("Anna <anna@a.lab>"), subject: s("Antwort"), text: s("Danke, Anna!\nBob\n"), ..MailDraft::default() };
    let _ = e.apply(Action::MailSend { dev: PC2, draft });
    run_for(&mut e, 5 * SEC);
    t.eq("sent", e.mail_client(PC2).and_then(|v| v.last_send).map(|o| (o.ok, o.msg.text())), Some((true, s("Gesendet an anna@a.lab über mail.b.lab."))));
    t.eq("logged in at MAILB", metric(&e, MAILB, "logins"), 1);
    t.eq("relayed by MAILB", metric(&e, MAILB, "relayed"), 1);
    let sent = e.file_store(PC2).map(|s| mailc::folder(s, mailc::SENT)).unwrap_or_default();
    t.eq("the copy", sent.iter().map(|m| (m.to.clone(), m.subject.clone())).collect::<Vec<_>>(), alloc::vec![(s("Anna"), s("Antwort"))]);
    let got = first_text(&e, MAILA, "/var/mail/anna/new/");
    t.check("authenticated relay", field_of(&got, "Received").is_empty() || got.contains("with ESMTPA id "));
    t.check("the text", Message::parse(got.as_bytes()).text() == "Danke, Anna!\nBob\n");
}

// ---------------------------------------------------------------- curl pop3://

fn curl_pop3(t: &mut T) {
    let mut e = warm(&project(SMTP_A));
    let _ = cli(&mut e, PC1, &["echo \"Zeile eins\" | mail -s Hallo bob@b.lab"], 5 * SEC);
    let o = cli(&mut e, PC2, &["curl -u bob:geheim pop3://mail.b.lab/"], 2 * SEC);
    t.check("LIST", o.starts_with("1 ") && o.lines().count() == 1);
    let o = cli(&mut e, PC2, &["curl -u bob:geheim pop3://mail.b.lab/1"], 2 * SEC);
    t.check("RETR", o.contains("Subject: Hallo\n") && o.contains("\nZeile eins\n") && o.starts_with("Return-Path: <anna@a.lab>\n"));
    let o = cli(&mut e, PC2, &["curl -u bob:geheim -X UIDL pop3://mail.b.lab/"], 2 * SEC);
    let uid = paths(&e, MAILB, "/var/mail/bob/new/").first().map(|p| String::from(p.rsplit('/').next().unwrap_or(""))).unwrap_or_default();
    t.eq("UIDL", o, format!("1 {}\n", uid));
    let o = cli(&mut e, PC2, &["curl -v -u bob:geheim -X \"TOP 1 0\" pop3://mail.b.lab/"], 2 * SEC);
    t.check("TOP: the header only", o.contains("< +OK The top of the message follows\n") && o.contains("Subject: Hallo\n") && !o.contains("Zeile eins"));
    t.check("-v: CAPA, USER and PASS", o.contains("> CAPA\n") && o.contains("> USER bob\n") && o.contains("> PASS geheim\n") && o.contains("> QUIT\n"));
    let o = cli(&mut e, PC2, &["curl -u bob:geheim pop3://mail.b.lab/7"], 2 * SEC);
    t.eq("no such message", o, s("curl: (8) Weird server reply\n"));
    let o = cli(&mut e, PC2, &["curl -u bob:geheim -X \"DELE 1\" pop3://mail.b.lab/", "curl -u bob:geheim pop3://mail.b.lab/"], 3 * SEC);
    // the second line waited for the first (its echo), the list is empty
    t.eq("DELE, then an empty list", o, s("root@PC2:~# curl -u bob:geheim pop3://mail.b.lab/\n"));
    t.eq("the mailbox empty", paths(&e, MAILB, "/var/mail/bob/new/").len(), 0);
}

// ---------------------------------------------------------------- logins

fn logins(t: &mut T) {
    let mut e = warm(&project(SMTP_A));
    let o = cli(&mut e, PC2, &["curl -v -u bob:falsch pop3://mail.b.lab/"], 3 * SEC);
    t.check("POP3: a wrong password", o.contains("< -ERR [AUTH] Wrong user name or password\n") && o.ends_with("curl: (67) Access denied. -\n"));
    let refused: Vec<u64> = mnotes(&e, MAILB).iter().filter(|(_, n)| matches!(n, MNote::Pop3LoginFailed { .. })).map(|x| x.0).collect();
    // the note at PASS, the answer a second later (fail_delay_ms)
    t.eq("refused when", refused, alloc::vec![3_000_026_252]);
    t.eq("pop3 numbers", (e.metric(MAILB, "pop3", "failed_logins"), e.metric(MAILB, "pop3", "logins")), (Some(1), Some(0)));
    // SMTP AUTH on 587
    let o = cli(&mut e, PC2, &["echo x > m.txt", "curl -u bob:falsch --mail-from bob@b.lab --mail-rcpt anna@a.lab -T m.txt smtp://mail.b.lab:587"], 3 * SEC);
    t.eq("AUTH refused", o, s("curl: (67) Authentication failed: 535\n"));
    let o = cli(&mut e, PC2, &["curl --mail-from bob@b.lab --mail-rcpt anna@a.lab -T m.txt smtp://mail.b.lab:587"], 3 * SEC);
    t.eq("587 without a login", o, s("curl: (55) MAIL failed: 530\n"));
    let o = cli(&mut e, PC2, &["curl -v -u bob:geheim --mail-from bob@b.lab --mail-rcpt anna@a.lab -T m.txt smtp://mail.b.lab:587"], 3 * SEC);
    t.check("AUTH PLAIN on 587", o.contains("< 250 AUTH PLAIN LOGIN\n") && o.contains("> AUTH PLAIN AGJvYgBnZWhlaW0=\n") && o.contains("< 235 2.7.0 Logged in\n"));
    t.eq("smtp logins", (metric(&e, MAILB, "logins"), metric(&e, MAILB, "failed_logins")), (1, 1));
}

// ---------------------------------------------------------------- the size limit

fn size_limit(t: &mut T) {
    let small = r#"{"hostname": "mail.a.lab", "domains": ["a.lab"], "users": ["anna"], "max_size_kb": 1}"#;
    let mut e = engine(&project(small));
    run_for(&mut e, SEC);
    let big = "x".repeat(700);
    let o = cli(&mut e, PC1, &[&format!("echo {} > big.txt", big), &format!("echo {} >> big.txt", big), "curl -v --mail-from anna@a.lab --mail-rcpt anna@a.lab -T big.txt smtp://mail.a.lab"], 2 * SEC);
    t.check("SIZE refused at MAIL", o.contains("< 552 5.3.4 Message too big, at most 1024 bytes\n") && o.ends_with("curl: (55) MAIL failed: 552\n"));
    // a client that does not announce it: after the dot
    let o = cli(&mut e, PC1, &[&format!("printf 'HELO pc1\\r\\nMAIL FROM:<anna@a.lab>\\r\\nRCPT TO:<anna@a.lab>\\r\\nDATA\\r\\n{}\\r\\n{}\\r\\n.\\r\\nQUIT\\r\\n' | nc mail.a.lab 25", big, big)], 2 * SEC);
    t.check("too big after the dot", o.contains("552 Message too big, at most 1024 bytes"));
    t.eq("counted", (metric(&e, MAILA, "too_big"), metric(&e, MAILA, "received")), (2, 0));
}

// ---------------------------------------------------------------- relaying refused, the open relay

fn relay_rules(t: &mut T) {
    let mut e = warm(&project(SMTP_A));
    // PC2 is not in MAILA's networks
    let o = cli(&mut e, PC2, &["echo x > m.txt", "curl --mail-from bob@b.lab --mail-rcpt carl@b.lab -T m.txt smtp://mail.a.lab"], 3 * SEC);
    t.eq("no relay for a stranger", o, s("curl: (55) RCPT failed: 554\n"));
    t.check("noted", mnotes(&e, MAILA).iter().any(|(_, n)| matches!(n, MNote::RelayDenied { rcpt, .. } if rcpt == "carl@b.lab")));
    let o = cli(&mut e, PC2, &["curl --mail-from bob@b.lab --mail-rcpt anna@a.lab -T m.txt smtp://mail.a.lab"], 3 * SEC);
    t.eq("mail for a.lab from anywhere", o, String::new());
    t.eq("denied once", metric(&e, MAILA, "relay_denied"), 1);
    let open = r#"{"hostname": "mail.a.lab", "domains": ["a.lab"], "users": ["anna"], "open_relay": true}"#;
    let mut o2 = warm(&project(open));
    let out = cli(&mut o2, PC2, &["echo x > m.txt", "curl --mail-from bob@b.lab --mail-rcpt bob@b.lab -T m.txt smtp://mail.a.lab"], 3 * SEC);
    t.eq("an open relay takes it", (out, metric(&o2, MAILA, "relayed")), (String::new(), 1));
    t.check("the problem", o2.problems().iter().any(|p| p.kind == ProblemKind::MailOpenRelay && p.dev == MAILA && p.text == "Dienst smtp auf MAILA ist ein offener Relay: er leitet Mail jedes Absenders weiter."));
}

// ---------------------------------------------------------------- problems

fn problems(t: &mut T) {
    let e = engine(&project(SMTP_A));
    let ps = e.problems();
    let mail: Vec<(ProblemKind, String)> = ps.iter().filter(|p| matches!(p.kind, ProblemKind::MailMxNowhere | ProblemKind::MailOpenRelay | ProblemKind::MailNoMailbox)).map(|p| (p.kind, p.text.clone())).collect();
    t.eq("an MX that points nowhere", mail, alloc::vec![(ProblemKind::MailMxNowhere, s("Der MX-Eintrag von c.lab (Dienst dns auf DNS1) zeigt auf mx.c.lab, das keine Adresse hat."))]);
    let mut p = project(SMTP_A);
    if let Some(d) = p.device_mut(MAILB) {
        d.services[1] = def("pop3-server", "pop3", r#"{"users": [{"name": "bob", "password": "x"}, {"name": "eve", "password": "y"}]}"#);
    }
    let e2 = engine(&p);
    t.check("a POP3 user without a mailbox", e2.problems().iter().any(|x| x.kind == ProblemKind::MailNoMailbox && x.text == "POP3-Benutzer eve auf MAILB hat kein Postfach, und kein SMTP-Server dort nimmt Mail für ihn an."));
    t.check("bob has one", !e2.problems().iter().any(|x| x.kind == ProblemKind::MailNoMailbox && x.text.contains(" bob ")));
}

// ---------------------------------------------------------------- the client's parts

fn client(t: &mut T) {
    let a = Account::parse(ACCOUNT_ANNA);
    t.eq("account", (a.name.as_str(), a.address.as_str(), a.smtp.as_str(), a.smtp_port, a.pop3_port, a.keep), ("Anna Muster", "anna@a.lab", "mail.a.lab", 25, 110, false));
    t.eq("round trip", Account::parse(&a.to_text()), a.clone());
    t.eq("defaults", (Account::parse("").smtp_port, Account::parse("").pop3_port), (25, 110));
    t.eq("file names of UIDLs", mailc::file_of_uid("1767225612.M3Q1.mailb"), s("1767225612.M3Q1.mailb.eml"));
    t.eq("odd ones", mailc::file_of_uid("a/b:c"), s("a_b_c.eml"));
    let mut e = engine(&project(SMTP_A));
    run_for(&mut e, SEC);
    let bad = MailDraft { to: s("kein-name"), subject: s("x"), ..MailDraft::default() };
    let _ = e.apply(Action::MailSend { dev: PC1, draft: bad });
    run_for(&mut e, SEC);
    t.eq("a bad address", e.mail_client(PC1).and_then(|v| v.last_send).map(|o| (o.ok, o.msg.text())), Some((false, s("Nicht gesendet: kein-name ist keine E-Mail-Adresse."))));
    let _ = e.apply(Action::Files { dev: PC1, files: Vec::new() });
    let _ = e.apply(Action::MailFetch { dev: PC1 });
    run_for(&mut e, SEC);
    t.eq("no account", e.mail_client(PC1).and_then(|v| v.last_fetch).map(|o| (o.ok, o.msg.text())), Some((false, s("Nicht abgerufen: das Konto nennt keinen POP3-Server oder keinen Benutzer."))));
    t.check("hidden among the services", e.services(PC1).iter().all(|s| s.kind != "mail-client"));
    let _ = e.apply(Action::Power { dev: PC2, on: false });
    t.check("refused while off", e.apply(Action::MailFetch { dev: PC2 }).is_err());
    // mail -H and its errors
    let mut f = warm(&project(SMTP_A));
    t.eq("mail -H without mail", cli(&mut f, PC2, &["mail -H"], 0), s("No mail for root\n"));
    t.eq("a bad address", cli(&mut f, PC2, &["mail -s x nobody"], 0), s("mail: nobody: not an e-mail address\n"));
    let _ = cli(&mut f, PC1, &["echo eins | mail -s Hallo bob@b.lab"], 5 * SEC);
    let _ = f.apply(Action::MailFetch { dev: PC2 });
    run_for(&mut f, 3 * SEC);
    let o = cli(&mut f, PC2, &["mail -H"], 0);
    t.check("mail -H", o.starts_with("   1 Anna Muster        Thu Jan  1 00:00 ") && o.ends_with(" Hallo\n"));
    let o = cli(&mut f, MAILA, &["echo lokal | mail -s Selbst anna@a.lab"], 3 * SEC);
    t.eq("mail on the server itself (127.0.0.1)", (o, metric(&f, MAILA, "delivered")), (String::new(), 1));
}

// ---------------------------------------------------------------- the project keeps the mailboxes

fn save_load(t: &mut T) {
    let mut p = project(SMTP_A);
    let mut e = engine(&p);
    run_for(&mut e, SEC);
    let _ = cli(&mut e, PC1, &["echo eins | mail -s Bleibt bob@b.lab"], 5 * SEC);
    // the app takes the reports as steps of the project
    for d in [MAILA, MAILB] {
        if let Some(x) = p.device_mut(d) {
            x.files = e.files(d);
        }
    }
    let text = p.to_text();
    let back = Project::from_text(&text).ok();
    t.check("in the project file", text.contains("\"path\": \"/var/mail/bob/new/") && back.is_some());
    let Some(back) = back else { return };
    let mut e2 = warm(&back);
    let o = cli(&mut e2, PC2, &["curl -u bob:geheim pop3://mail.b.lab/1"], 2 * SEC);
    t.check("POP3 after loading", o.contains("Subject: Bleibt\n"));
}

// ---------------------------------------------------------------- determinism

fn determinism(t: &mut T) {
    let go = || {
        let mut e = engine(&project(SMTP_A));
        run_for(&mut e, SEC);
        let _ = cli(&mut e, PC1, &["echo eins | mail -s A bob@b.lab", "echo zwei | mail -s B nobody@b.lab"], 5 * SEC);
        let _ = e.apply(Action::MailFetch { dev: PC2 });
        run_for(&mut e, 3 * SEC);
        let hops: Vec<(u64, u64, Vec<u8>)> = e.trace.hops().map(|h| (h.t_tx, h.t_rx, h.bytes.to_vec())).collect();
        (hops, e.files(MAILA), e.files(PC2), jfn_sim::keyframe::digest(&e))
    };
    let (a, b) = (go(), go());
    t.check("twice the same", a == b && !a.0.is_empty());
}
