// imaptest -- checks of IMAP (group "imap" of --selftest: jfn_sim::imapd, the folders of mailbox.rs,
// the IMAP part of mailc.rs and cli/mail.rs): a session line by line over nc, LOGIN and AUTHENTICATE
// PLAIN, SELECT and EXAMINE, FETCH of flags, sizes, envelope and bodies, STORE and EXPUNGE, folders
// with CREATE, RENAME and DELETE, COPY and MOVE, APPEND, SEARCH, the UID forms, IDLE, POP3 and IMAP
// on one mailbox, the E-Mail program's IMAP account (folders, read and flagged states on the server,
// delete to Trash, sent mail in Sent), curl imap://, the notes, the service's numbers and determinism
//
// The LAN (device ids in order): MAIL (server-1u, 10.0.0.10: smtp-server for a.lab, pop3-server and
// imap-server; users anna and bob), SW1 (switch), PC1 (10.0.0.21), PC2 (10.0.0.22).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::files::File;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::mailbox;
use jfn_sim::mailc::{self, ImapOp};
use jfn_sim::{Action, Engine, MailDraft, Note, Report, SEC};

use crate::selftest::T;

const BUDGET: usize = 8_000_000;
const MAIL: u32 = 1;
const SW1: u32 = 2;
const PC1: u32 = 3;
const PC2: u32 = 4;

fn s(x: &str) -> String {
    String::from(x)
}

fn def(kind: &str, name: &str, config: &str) -> ServiceDef {
    ServiceDef { kind: s(kind), name: s(name), disabled: false, config: agent_core::json::parse(config.as_bytes()).unwrap_or(Value::Null) }
}

const USERS: &str = r#""users": [{"name": "anna", "password": "geheim"}, {"name": "bob", "password": "bobpw"}]"#;

/// a message in the Maildir as the SMTP server delivers one
fn delivered(user: &str, name: &str, from: &str, subject: &str, text: &str) -> File {
    let data = format!("Return-Path: <{0}>\nFrom: {0}\nTo: {1}@a.lab\nSubject: {2}\nDate: Thu, 01 Jan 2026 00:00:0{3} +0000\nMessage-ID: <{4}@a.lab>\n\n{5}\n", from, user, subject, name.len() % 10, name, text);
    File { path: format!("/var/mail/{}/new/{}", user, name), data: data.into_bytes(), mtime: 1_767_225_600, dir: false }
}

fn dir(path: &str) -> File {
    File { path: s(path), data: Vec::new(), mtime: 1_767_225_600, dir: true }
}

fn project() -> Project {
    let mut p = Project::new("imap", 11);
    for (i, m) in ["server-1u", "jf-s24p", "pc", "pc"].iter().enumerate() {
        let _ = p.add_device(m, i as i32 * 120, 0);
    }
    let pr = |dev: u32, port: &str| PortRef { dev, port: s(port) };
    let _ = p.connect(&pr(MAIL, "eth0"), &pr(SW1, "gi1/0/1"), None);
    let _ = p.connect(&pr(PC1, "eth0"), &pr(SW1, "gi1/0/2"), None);
    let _ = p.connect(&pr(PC2, "eth0"), &pr(SW1, "gi1/0/3"), None);
    let cfg = [
        (MAIL, "MAIL", s("hostname MAIL\nip addr add 10.0.0.10/24 dev eth0\n")),
        (SW1, "SW1", s("hostname SW1\n!\nend\n")),
        (PC1, "PC1", s("hostname PC1\nip addr add 10.0.0.21/24 dev eth0\n")),
        (PC2, "PC2", s("hostname PC2\nip addr add 10.0.0.22/24 dev eth0\n")),
    ];
    for (id, name, c) in cfg {
        if let Some(d) = p.device_mut(id) {
            d.name = s(name);
            d.startup_config = c;
        }
    }
    if let Some(d) = p.device_mut(MAIL) {
        d.services = alloc::vec![
            def("smtp-server", "smtp", &format!(r#"{{"hostname": "mail.a.lab", "domains": ["a.lab"], {}, "networks": ["10.0.0.0/24"]}}"#, USERS)),
            def("pop3-server", "pop3", &format!(r#"{{{}}}"#, USERS)),
            def("imap-server", "imap", &format!(r#"{{"fail_delay_ms": 100, {}}}"#, USERS)),
        ];
        d.files.push(dir("/var/mail"));
        d.files.push(dir("/var/mail/anna"));
        d.files.push(dir("/var/mail/anna/new"));
        d.files.push(delivered("anna", "1767225601.M1Q1.mail", "bob@a.lab", "Hallo Anna", "Der erste Text"));
        d.files.push(delivered("anna", "1767225602.M2Q2.mail", "carl@b.lab", "Rechnung", "Zweiter Text"));
        d.files.push(delivered("anna", "1767225603.M3Q3.mail", "bob@a.lab", "Wieder Bob", "Dritter Text"));
    }
    p
}

fn engine() -> Engine {
    let p = project();
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    run_for(&mut e, 200 * jfn_sim::MS);
    e
}

fn run_for(e: &mut Engine, d: u64) {
    let t = e.now() + d;
    e.run_until(t, BUDGET);
}

/// a client of the server: nc from PC1 with lines from the console
struct Client {
    dev: u32,
}

impl Client {
    fn open(e: &mut Engine, dev: u32) -> (Client, String) {
        let _ = e.take_reports();
        let _ = e.cli_line(dev, "nc 10.0.0.10 143");
        run_for(e, SEC);
        (Client { dev }, out(e, dev))
    }
    /// a line to the server, its answer
    fn line(&self, e: &mut Engine, l: &str) -> String {
        let _ = e.cli_line(self.dev, l);
        run_for(e, SEC);
        out(e, self.dev)
    }
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

fn cli(e: &mut Engine, dev: u32, line: &str, d: u64) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    run_for(e, d);
    out(e, dev)
}

fn metric(e: &Engine, name: &str) -> i64 {
    e.metric(MAIL, "imap", name).unwrap_or(-1)
}

fn mail_notes(e: &Engine, dev: u32) -> Vec<jfn_sim::mailtrace::MNote> {
    e.trace.events().filter(|x| x.dev == dev).filter_map(|x| if let Note::Mail(n) = &x.note { Some(n.clone()) } else { None }).collect()
}

fn files_of(e: &Engine, dev: u32, dir: &str) -> Vec<String> {
    e.file_store(dev).map(|st| st.list(dir).unwrap_or_default().into_iter().filter(|x| !x.1.dir).map(|x| x.0).collect()).unwrap_or_default()
}

pub(crate) fn run(t: &mut T) {
    login(t);
    select_fetch(t);
    store_expunge(t);
    folders(t);
    copy_move_append(t);
    search(t);
    uid_forms(t);
    idle(t);
    with_pop3(t);
    errors(t);
    client(t);
    curl(t);
    determinism(t);
}

fn login(t: &mut T) {
    let mut e = engine();
    let (c, hello) = Client::open(&mut e, PC1);
    t.check("greeting names our server", hello.contains("* OK") && hello.contains("JFNetworker IMAP ready"));
    t.check("capabilities in the greeting", hello.contains("IMAP4rev1"));
    let o = c.line(&mut e, "a1 CAPABILITY");
    t.check("capability", o.contains("* CAPABILITY IMAP4rev1 AUTH=PLAIN") && o.contains("a1 OK"));
    let o = c.line(&mut e, "a2 SELECT INBOX");
    t.check("not before the login", o.contains("a2 BAD"));
    let o = c.line(&mut e, "a3 LOGIN anna falsch");
    t.check("a wrong password", o.contains("a3 NO [AUTHENTICATIONFAILED]"));
    t.eq("counted", metric(&e, "failed_logins"), 1);
    let o = c.line(&mut e, "a4 LOGIN anna geheim");
    t.check("logged in", o.contains("a4 OK"));
    t.eq("login counted", metric(&e, "logins"), 1);
    t.check("the note", mail_notes(&e, MAIL).iter().any(|n| matches!(n, jfn_sim::mailtrace::MNote::ImapLogin { user } if user == "anna")));
    let o = c.line(&mut e, "a5 LOGIN anna geheim");
    t.check("once is enough", o.contains("a5 BAD"));
    // the mailbox has its folders now
    let st = e.file_store(MAIL).cloned();
    t.check("Sent", st.as_ref().is_some_and(|x| x.is_dir("/var/mail/anna/.Sent/cur")));
    t.check("Trash", st.as_ref().is_some_and(|x| x.is_dir("/var/mail/anna/.Trash/cur")));
    let o = c.line(&mut e, "a6 NOOP");
    t.check("noop", o.contains("a6 OK"));
    let o = c.line(&mut e, "a7 LOGOUT");
    t.check("logout", o.contains("* BYE") && o.contains("a7 OK"));
    // AUTHENTICATE PLAIN with the initial response: \0bob\0bobpw
    let (c, _) = Client::open(&mut e, PC2);
    let b64 = imf_b64(b"\0bob\0bobpw");
    let o = c.line(&mut e, &format!("b1 AUTHENTICATE PLAIN {}", b64));
    t.check("AUTHENTICATE with its response", o.contains("b1 OK"));
    // and without: the server asks with +
    let mut e = engine();
    let (c, _) = Client::open(&mut e, PC2);
    let o = c.line(&mut e, "b1 AUTHENTICATE PLAIN");
    t.check("asks", o.starts_with("+"));
    let o = c.line(&mut e, &b64);
    t.check("AUTHENTICATE in two steps", o.contains("b1 OK"));
    let (c2, _) = Client::open(&mut e, PC1);
    let o = c2.line(&mut e, "c1 AUTHENTICATE GSSAPI");
    t.check("only PLAIN", o.contains("c1 NO"));
    // too many wrong passwords end the session
    let mut e = engine();
    let (c, _) = Client::open(&mut e, PC1);
    let _ = c.line(&mut e, "a1 LOGIN anna x");
    let _ = c.line(&mut e, "a2 LOGIN anna x");
    let o = c.line(&mut e, "a3 LOGIN anna x");
    t.check("three strikes", o.contains("* BYE"));
}

fn imf_b64(b: &[u8]) -> String {
    netpkt::imf::b64_encode(b)
}

fn select_fetch(t: &mut T) {
    let mut e = engine();
    let (c, _) = Client::open(&mut e, PC1);
    let _ = c.line(&mut e, "a1 LOGIN anna geheim");
    let o = c.line(&mut e, "a2 SELECT INBOX");
    t.check("EXISTS", o.contains("* 3 EXISTS"));
    t.check("RECENT", o.contains("* 3 RECENT"));
    t.check("FLAGS", o.contains("* FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)"));
    t.check("UNSEEN", o.contains("[UNSEEN 1]"));
    t.check("UIDVALIDITY", o.contains("[UIDVALIDITY "));
    t.check("UIDNEXT", o.contains("[UIDNEXT 4]"));
    t.check("read-write", o.contains("a2 OK [READ-WRITE]"));
    t.check("the UIDs are on the disk", e.file_store(MAIL).is_some_and(|st| st.exists("/var/mail/anna/.uidlist")));
    let o = c.line(&mut e, "a3 FETCH 1 (FLAGS UID RFC822.SIZE)");
    t.check("flags and size", o.contains("* 1 FETCH (FLAGS (\\Recent) UID 1 RFC822.SIZE "));
    let o = c.line(&mut e, "a4 FETCH 2 ENVELOPE");
    t.check("envelope", o.contains("ENVELOPE (") && o.contains("\"Rechnung\"") && o.contains("\"carl\" \"b.lab\""));
    // BODY.PEEK leaves the flags alone
    let o = c.line(&mut e, "a5 FETCH 1 BODY.PEEK[HEADER]");
    t.check("the header", o.contains("Subject: Hallo Anna") && !o.contains("Der erste Text"));
    let o = c.line(&mut e, "a6 FETCH 1 FLAGS");
    t.check("peek keeps it unread", !o.contains("\\Seen"));
    let o = c.line(&mut e, "a7 FETCH 1 BODY.PEEK[HEADER.FIELDS (SUBJECT)]");
    t.check("one field", o.contains("Subject: Hallo Anna") && !o.contains("From:"));
    let o = c.line(&mut e, "a8 FETCH 1 BODY[TEXT]");
    t.check("the text", o.contains("Der erste Text") && !o.contains("Subject:"));
    t.check("BODY without PEEK sets \\Seen", o.contains("FLAGS (\\Seen)"));
    let st = e.file_store(MAIL).cloned();
    t.check("it moved to cur/ with its flag", st.as_ref().is_some_and(|x| x.exists("/var/mail/anna/cur/1767225601.M1Q1.mail:2,S")));
    t.check("gone from new/", st.as_ref().is_some_and(|x| !x.exists("/var/mail/anna/new/1767225601.M1Q1.mail")));
    let o = c.line(&mut e, "a9 FETCH 2 BODY[]");
    t.check("the message", o.contains("Subject: Rechnung") && o.contains("Zweiter Text"));
    t.check("its literal", o.contains("BODY[] {"));
    let o = c.line(&mut e, "a10 FETCH 1:* FAST");
    t.check("FAST and a range", o.contains("* 1 FETCH") && o.contains("* 3 FETCH") && o.contains("INTERNALDATE"));
    let o = c.line(&mut e, "a11 FETCH 9 FLAGS");
    t.check("nothing to send", o.contains("a11 OK") && !o.contains("* 9 FETCH"));
    let o = c.line(&mut e, "a12 FETCH 1 NONSENSE");
    t.check("what it does not know", o.contains("a12 BAD"));
    t.check("fetched counted", metric(&e, "fetched") >= 3);
    t.check("bytes counted", metric(&e, "bytes_sent") > 0);
    // EXAMINE reads only
    let o = c.line(&mut e, "b1 EXAMINE INBOX");
    t.check("read-only", o.contains("b1 OK [READ-ONLY]"));
    let o = c.line(&mut e, "b2 STORE 2 +FLAGS (\\Seen)");
    t.check("no STORE", o.contains("b2 NO"));
    let o = c.line(&mut e, "b3 FETCH 3 BODY[]");
    t.check("no \\Seen in read-only", !o.contains("\\Seen"));
    let o = c.line(&mut e, "b4 SELECT Nirgends");
    t.check("no such folder", o.contains("b4 NO [NONEXISTENT]"));
    let o = c.line(&mut e, "b5 FETCH 1 FLAGS");
    t.check("nothing selected after a failed SELECT", o.contains("b5 BAD"));
    let o = c.line(&mut e, "b6 STATUS INBOX (MESSAGES UNSEEN RECENT UIDNEXT)");
    t.check("status", o.contains("* STATUS INBOX (MESSAGES 3 UNSEEN 1 RECENT 1 UIDNEXT 4)"));
}

fn store_expunge(t: &mut T) {
    let mut e = engine();
    let (c, _) = Client::open(&mut e, PC1);
    let _ = c.line(&mut e, "a1 LOGIN anna geheim");
    let _ = c.line(&mut e, "a2 SELECT INBOX");
    let o = c.line(&mut e, "a3 STORE 1 +FLAGS (\\Flagged \\Seen)");
    t.check("the answer", o.contains("* 1 FETCH (FLAGS (\\Flagged \\Seen))"));
    let st = e.file_store(MAIL).cloned();
    t.check("in the name, in Maildir's order", st.as_ref().is_some_and(|x| x.exists("/var/mail/anna/cur/1767225601.M1Q1.mail:2,FS")));
    let o = c.line(&mut e, "a4 STORE 1 -FLAGS.SILENT (\\Flagged)");
    t.check("silent", o.contains("a4 OK") && !o.contains("FETCH"));
    let o = c.line(&mut e, "a5 STORE 2 FLAGS (\\Answered)");
    t.check("set", o.contains("* 2 FETCH (FLAGS (\\Answered))"));
    t.eq("counted", metric(&e, "stored"), 3);
    let _ = c.line(&mut e, "a6 STORE 1:2 +FLAGS (\\Deleted)");
    let o = c.line(&mut e, "a7 EXPUNGE");
    t.check("the numbers shift down", o.contains("* 1 EXPUNGE") && o.matches("* 1 EXPUNGE").count() == 2);
    t.check("OK", o.contains("a7 OK"));
    t.eq("expunged", metric(&e, "expunged"), 2);
    t.eq("files left", files_of(&e, MAIL, "/var/mail/anna/new").len() + files_of(&e, MAIL, "/var/mail/anna/cur").len(), 1);
    let o = c.line(&mut e, "a8 FETCH 1 (UID)");
    t.check("the last one keeps its UID", o.contains("* 1 FETCH (UID 3)"));
    // CLOSE expunges without saying so
    let _ = c.line(&mut e, "a9 STORE 1 +FLAGS.SILENT (\\Deleted)");
    let o = c.line(&mut e, "a10 CLOSE");
    t.check("silent", !o.contains("EXPUNGE") && o.contains("a10 OK"));
    t.eq("all gone", files_of(&e, MAIL, "/var/mail/anna/cur").len() + files_of(&e, MAIL, "/var/mail/anna/new").len(), 0);
    let o = c.line(&mut e, "a11 FETCH 1 FLAGS");
    t.check("CLOSE leaves no folder", o.contains("a11 BAD"));
    // a second session sees what the first did at its next NOOP
    let mut e = engine();
    let (c1, _) = Client::open(&mut e, PC1);
    let (c2, _) = Client::open(&mut e, PC2);
    let _ = c1.line(&mut e, "a1 LOGIN anna geheim");
    let _ = c1.line(&mut e, "a2 SELECT INBOX");
    let _ = c2.line(&mut e, "b1 LOGIN anna geheim");
    let _ = c2.line(&mut e, "b2 SELECT INBOX");
    let _ = c1.line(&mut e, "a3 STORE 1 +FLAGS.SILENT (\\Seen)");
    let o = c2.line(&mut e, "b3 NOOP");
    t.check("a change of flags shows", o.contains("* 1 FETCH (FLAGS (\\Seen))"));
    let _ = c1.line(&mut e, "a4 STORE 2 +FLAGS.SILENT (\\Deleted)");
    let _ = c1.line(&mut e, "a5 EXPUNGE");
    let o = c2.line(&mut e, "b4 NOOP");
    t.check("an expunge shows", o.contains("* 2 EXPUNGE"));
}

fn folders(t: &mut T) {
    let mut e = engine();
    let (c, _) = Client::open(&mut e, PC1);
    let _ = c.line(&mut e, "a1 LOGIN anna geheim");
    let o = c.line(&mut e, "a2 LIST \"\" \"*\"");
    t.check("INBOX", o.contains("* LIST (\\HasNoChildren) \"/\" INBOX"));
    t.check("Sent", o.contains("* LIST (\\HasNoChildren \\Sent) \"/\" Sent"));
    t.check("Trash", o.contains("* LIST (\\HasNoChildren \\Trash) \"/\" Trash"));
    t.check("the delimiter", c.line(&mut e, "a3 LIST \"\" \"\"").contains("* LIST (\\Noselect) \"/\" \"\""));
    t.check("a pattern", c.line(&mut e, "a4 LIST \"\" \"T%\"").contains("Trash"));
    t.check("a pattern out", !c.line(&mut e, "a5 LIST \"\" \"T%\"").contains("INBOX"));
    t.check("LSUB", c.line(&mut e, "a6 LSUB \"\" \"*\"").contains("* LSUB (\\HasNoChildren) \"/\" INBOX"));
    t.check("CREATE", c.line(&mut e, "b1 CREATE Projekte").contains("b1 OK"));
    t.check("it is on the disk", e.file_store(MAIL).is_some_and(|st| st.is_dir("/var/mail/anna/.Projekte/cur")));
    t.check("it lists", c.line(&mut e, "b2 LIST \"\" Projekte").contains("Projekte"));
    t.check("no twins", c.line(&mut e, "b3 CREATE Projekte").contains("b3 NO [ALREADYEXISTS]"));
    t.check("no INBOX", c.line(&mut e, "b4 CREATE inbox").contains("b4 NO"));
    t.check("no paths", c.line(&mut e, "b5 CREATE a/b").contains("b5 NO"));
    t.check("RENAME", c.line(&mut e, "c1 RENAME Projekte Archiv").contains("c1 OK"));
    t.check("renamed", e.file_store(MAIL).is_some_and(|st| st.is_dir("/var/mail/anna/.Archiv") && !st.is_dir("/var/mail/anna/.Projekte")));
    t.check("INBOX stays", c.line(&mut e, "c2 RENAME INBOX X").contains("c2 NO"));
    t.check("select it", c.line(&mut e, "c3 SELECT Archiv").contains("* 0 EXISTS"));
    t.check("DELETE", c.line(&mut e, "c4 DELETE Archiv").contains("c4 OK"));
    t.check("gone", !e.file_store(MAIL).is_some_and(|st| st.exists("/var/mail/anna/.Archiv")));
    t.check("INBOX cannot go", c.line(&mut e, "c5 DELETE INBOX").contains("c5 NO"));
    t.check("no such", c.line(&mut e, "c6 DELETE Nirgends").contains("c6 NO [NONEXISTENT]"));
    t.check("subscribe is taken", c.line(&mut e, "c7 SUBSCRIBE INBOX").contains("c7 OK"));
    // the folder of a message with a name that needs quotes
    t.check("quoted name", c.line(&mut e, "d1 CREATE \"Meine Post\"").contains("d1 OK"));
    t.check("listed in quotes", c.line(&mut e, "d2 LIST \"\" \"M*\"").contains("\"Meine Post\""));
}

fn copy_move_append(t: &mut T) {
    let mut e = engine();
    let (c, _) = Client::open(&mut e, PC1);
    let _ = c.line(&mut e, "a1 LOGIN anna geheim");
    let _ = c.line(&mut e, "a2 SELECT INBOX");
    let _ = c.line(&mut e, "a3 STORE 1 +FLAGS.SILENT (\\Flagged)");
    let o = c.line(&mut e, "b1 COPY 1 Trash");
    t.check("COPY", o.contains("b1 OK"));
    t.eq("copied with its flags", files_of(&e, MAIL, "/var/mail/anna/.Trash/cur").iter().filter(|n| n.ends_with(":2,F")).count(), 1);
    t.check("still here", c.line(&mut e, "b2 NOOP").contains("b2 OK"));
    let o = c.line(&mut e, "b3 MOVE 2 Sent");
    t.check("MOVE", o.contains("* 2 EXPUNGE") && o.contains("b3 OK"));
    t.eq("moved", files_of(&e, MAIL, "/var/mail/anna/.Sent/cur").len(), 1);
    t.eq("two left", files_of(&e, MAIL, "/var/mail/anna/cur").len() + files_of(&e, MAIL, "/var/mail/anna/new").len(), 2);
    let o = c.line(&mut e, "b4 COPY 1 Nirgends");
    t.check("TRYCREATE", o.contains("b4 NO [TRYCREATE]"));
    // the moved one in its folder
    let o = c.line(&mut e, "b5 SELECT Sent");
    t.check("one there", o.contains("* 1 EXISTS"));
    let o = c.line(&mut e, "b6 FETCH 1 BODY.PEEK[]");
    t.check("the message is there", o.contains("Rechnung"));
    // APPEND: nc sends the literal as the next lines; the codec counts the bytes
    let _ = e.take_reports();
    // (nc ends its lines with LF)
    let msg = "From: a@a.lab\nSubject: Entwurf\n\nText\n";
    let o = c.line(&mut e, &format!("c1 APPEND Sent (\\Seen) {{{}+}}", msg.len()));
    t.check("no question with the plus", !o.starts_with("+"));
    for l in ["From: a@a.lab", "Subject: Entwurf", "", "Text", ""] {
        let _ = e.cli_line(PC1, l);
    }
    run_for(&mut e, SEC);
    let o = out(&mut e, PC1);
    t.check("appended", o.contains("c1 OK"));
    t.eq("counted", metric(&e, "appended"), 1);
    t.eq("seen in the name", files_of(&e, MAIL, "/var/mail/anna/.Sent/cur").iter().filter(|n| n.ends_with(":2,S")).count(), 1);
    t.check("EXISTS comes", o.contains("* 2 EXISTS"));
}

fn search(t: &mut T) {
    let mut e = engine();
    let (c, _) = Client::open(&mut e, PC1);
    let _ = c.line(&mut e, "a1 LOGIN anna geheim");
    let _ = c.line(&mut e, "a2 SELECT INBOX");
    let _ = c.line(&mut e, "a3 STORE 2 +FLAGS.SILENT (\\Seen \\Flagged)");
    let found = |c: &Client, e: &mut Engine, q: &str| c.line(e, &format!("s {}", q)).lines().find(|l| l.starts_with("* SEARCH")).map(String::from).unwrap_or_default();
    t.eq("ALL", found(&c, &mut e, "SEARCH ALL"), s("* SEARCH 1 2 3"));
    t.eq("UNSEEN", found(&c, &mut e, "SEARCH UNSEEN"), s("* SEARCH 1 3"));
    t.eq("SEEN", found(&c, &mut e, "SEARCH SEEN"), s("* SEARCH 2"));
    t.eq("FLAGGED", found(&c, &mut e, "SEARCH FLAGGED"), s("* SEARCH 2"));
    t.eq("FROM", found(&c, &mut e, "SEARCH FROM bob"), s("* SEARCH 1 3"));
    t.eq("SUBJECT", found(&c, &mut e, "SEARCH SUBJECT rechnung"), s("* SEARCH 2"));
    t.eq("several", found(&c, &mut e, "SEARCH FROM bob UNSEEN SUBJECT wieder"), s("* SEARCH 3"));
    t.eq("NOT", found(&c, &mut e, "SEARCH NOT FROM bob"), s("* SEARCH 2"));
    t.eq("OR", found(&c, &mut e, "SEARCH OR SUBJECT Hallo SUBJECT Rechnung"), s("* SEARCH 1 2"));
    t.eq("a range", found(&c, &mut e, "SEARCH 2:3"), s("* SEARCH 2 3"));
    t.eq("TEXT", found(&c, &mut e, "SEARCH TEXT zweiter"), s("* SEARCH 2"));
    t.eq("nothing", found(&c, &mut e, "SEARCH SUBJECT nichts"), s("* SEARCH"));
    t.eq("CHARSET", found(&c, &mut e, "SEARCH CHARSET UTF-8 FROM carl"), s("* SEARCH 2"));
    t.check("what it does not know", c.line(&mut e, "s SEARCH WHATEVER").contains("s BAD"));
}

fn uid_forms(t: &mut T) {
    let mut e = engine();
    let (c, _) = Client::open(&mut e, PC1);
    let _ = c.line(&mut e, "a1 LOGIN anna geheim");
    let _ = c.line(&mut e, "a2 SELECT INBOX");
    let _ = c.line(&mut e, "a3 STORE 1 +FLAGS.SILENT (\\Deleted)");
    let _ = c.line(&mut e, "a4 EXPUNGE");
    // the numbers moved, the UIDs did not
    let o = c.line(&mut e, "b1 UID FETCH 3 FLAGS");
    t.check("UID FETCH shows the UID", o.contains("* 2 FETCH (UID 3 FLAGS"));
    let o = c.line(&mut e, "b2 UID SEARCH ALL");
    t.check("UID SEARCH", o.contains("* SEARCH 2 3"));
    let o = c.line(&mut e, "b3 UID STORE 2 +FLAGS (\\Seen)");
    t.check("UID STORE", o.contains("* 1 FETCH (UID 2 FLAGS (\\Seen))"));
    let o = c.line(&mut e, "b4 UID COPY 3 Trash");
    t.check("UID COPY", o.contains("b4 OK"));
    let o = c.line(&mut e, "b5 UID MOVE 2 Sent");
    t.check("UID MOVE", o.contains("* 1 EXPUNGE"));
    let o = c.line(&mut e, "b6 UID FETCH 1:* (UID)");
    t.check("what is left", o.contains("UID 3") && !o.contains("UID 2"));
    let o = c.line(&mut e, "b7 UID BOGUS 1");
    t.check("UID with something else", o.contains("b7 BAD"));
    // a new login keeps the UIDs and gives the next one to a new message
    let (c2, _) = Client::open(&mut e, PC2);
    let _ = c2.line(&mut e, "c1 LOGIN anna geheim");
    let o = c2.line(&mut e, "c2 SELECT INBOX");
    t.check("UIDNEXT stays", o.contains("[UIDNEXT 4]"));
}

fn idle(t: &mut T) {
    let mut e = engine();
    let (c, _) = Client::open(&mut e, PC1);
    let _ = c.line(&mut e, "a1 LOGIN anna geheim");
    let _ = c.line(&mut e, "a2 SELECT INBOX");
    let o = c.line(&mut e, "a3 IDLE");
    t.check("waits", o.starts_with("+"));
    // a new message arrives over SMTP while it waits
    let _ = e.cli_line(PC2, "echo hallo > /tmp/m");
    run_for(&mut e, 100 * jfn_sim::MS);
    let _ = e.cli_line(PC2, "curl smtp://10.0.0.10 --mail-from bob@a.lab --mail-rcpt anna@a.lab -T /tmp/m");
    run_for(&mut e, 5 * SEC);
    let o = out(&mut e, PC1);
    t.check("EXISTS comes without asking", o.contains("* 4 EXISTS"));
    let o = c.line(&mut e, "DONE");
    t.check("the end of IDLE", o.contains("a3 OK"));
    let o = c.line(&mut e, "a4 FETCH 4 (UID)");
    t.check("it has the next UID", o.contains("UID 4"));
}

fn with_pop3(t: &mut T) {
    let mut e = engine();
    let (c, _) = Client::open(&mut e, PC1);
    let _ = c.line(&mut e, "a1 LOGIN anna geheim");
    let _ = c.line(&mut e, "a2 SELECT INBOX");
    let _ = c.line(&mut e, "a3 STORE 1 +FLAGS (\\Seen)");
    // POP3 finds all three, the one in cur/ with its flags under the same unique name
    let o = cli(&mut e, PC2, "curl pop3://10.0.0.10 -u anna:geheim -X UIDL", 2 * SEC);
    t.check("POP3 sees the moved message", o.contains("1 1767225601.M1Q1.mail") && o.contains("3 1767225603.M3Q3.mail"));
    let o = cli(&mut e, PC2, "curl pop3://10.0.0.10/1 -u anna:geheim", 2 * SEC);
    t.check("and reads it", o.contains("Der erste Text"));
    // POP3 removes one: IMAP hears of it at its next NOOP
    let _ = cli(&mut e, PC2, "curl pop3://10.0.0.10/2 -u anna:geheim -X DELE", 2 * SEC);
    t.eq("removed", e.metric(MAIL, "pop3", "deleted"), Some(1));
    let o = c.line(&mut e, "a4 NOOP");
    t.check("EXPUNGE after POP3", o.contains("* 2 EXPUNGE"));
    // a delivery lands in both
    let _ = cli(&mut e, PC2, "echo hallo > /tmp/m", 100 * jfn_sim::MS);
    let _ = cli(&mut e, PC2, "curl smtp://10.0.0.10 --mail-from bob@a.lab --mail-rcpt anna@a.lab -T /tmp/m", 2 * SEC);
    let o = c.line(&mut e, "a5 NOOP");
    t.check("IMAP sees the delivery", o.contains("* 3 EXISTS"));
    let o = cli(&mut e, PC2, "curl pop3://10.0.0.10 -u anna:geheim -X UIDL", 2 * SEC);
    t.check("POP3 too", o.lines().count() == 3);
    // what POP3 leaves out: the folders
    t.check("POP3 does not read Trash", !o.contains(".Trash"));
    // the .uidlist is no message for POP3
    t.check("nor the UID list", !o.contains("uidlist"));
}

fn errors(t: &mut T) {
    let mut e = engine();
    let (c, _) = Client::open(&mut e, PC1);
    t.check("no tag", c.line(&mut e, "HALLO").contains("BAD"));
    t.check("unknown", c.line(&mut e, "a1 FROBNICATE").contains("a1 BAD"));
    let _ = c.line(&mut e, "a2 LOGIN anna geheim");
    t.check("select first", c.line(&mut e, "a3 EXPUNGE").contains("a3 BAD"));
    t.check("missing args", c.line(&mut e, "a4 SELECT").contains("a4 BAD"));
    let _ = c.line(&mut e, "a5 SELECT INBOX");
    t.check("store nonsense", c.line(&mut e, "a6 STORE 1 ALLES (\\Seen)").contains("a6 BAD"));
    t.check("a set that is none", c.line(&mut e, "a7 STORE x +FLAGS (\\Seen)").contains("a7 BAD"));
    t.check("the note", mail_notes(&e, MAIL).iter().any(|n| matches!(n, jfn_sim::mailtrace::MNote::ImapRefused { .. })));
    // the service on the device and its settings
    let bad = jfn_sim::ImapServer::from_config(&agent_core::json::parse(b"{\"port\": 0}").unwrap_or(Value::Null));
    t.check("port 0 is refused", bad.is_err());
    let good = jfn_sim::ImapServer::from_config(&agent_core::json::parse(b"{\"port\": 1143, \"banner\": \"Hi\"}").unwrap_or(Value::Null));
    t.check("own port", good.is_ok_and(|g| g.port == 1143 && g.banner == "Hi"));
    t.check("the service's status", e.services(MAIL).iter().any(|x| x.kind == "imap-server" && x.status.contains("Port 143")));
    t.eq("sessions", metric(&e, "sessions"), 1);
    t.eq("active", metric(&e, "active"), 1);
    // the dissector knows the lines of a real session
    let h: Vec<Vec<u8>> = e.trace.hops().filter(|h| h.mask & netpkt::decode::P_MAIL != 0).map(|h| h.bytes.to_vec()).collect();
    t.check("frames with IMAP", h.iter().any(|f| netpkt::decode::info(f).1.contains("LOGIN anna")));
    t.check("the category", h.iter().all(|f| netpkt::decode::proto_mask(f) & netpkt::decode::P_TCP != 0));
}

/// the account of the E-Mail program on PC1
fn account(e: &mut Engine, dev: u32, user: &str) {
    let a = mailc::Account { name: s("Anna"), address: format!("{}@a.lab", user), smtp: s("10.0.0.10"), smtp_port: 25, pop3: String::new(), pop3_port: 110, imap: s("10.0.0.10"), imap_port: 143, user: s(user), password: s(if user == "anna" { "geheim" } else { "bobpw" }), keep: false };
    let _ = cli(e, dev, "mkdir -p /root/Mail", 10 * jfn_sim::MS);
    let store = e.file_store(dev).cloned();
    let Some(mut st) = store else { return };
    let _ = st.write(mailc::ACCOUNT, a.to_text().as_bytes(), 1_767_225_600);
    let _ = e.apply(Action::Files { dev, files: st.records() });
}

fn imap_op(e: &mut Engine, dev: u32, op: ImapOp) {
    let _ = e.apply(Action::MailImap { dev, op });
    run_for(e, 3 * SEC);
}

fn client(t: &mut T) {
    let mut e = engine();
    account(&mut e, PC1, "anna");
    // the account's file keeps IMAP, a POP3 account's has no new lines
    let parsed_acct = e.file_store(PC1).and_then(mailc::Account::of);
    t.check("IMAP account", parsed_acct.as_ref().is_some_and(|a| a.is_imap() && a.imap_port == 143));
    let pop = mailc::Account { name: s("x"), address: s("a@b"), smtp: s("h"), smtp_port: 25, pop3: s("h"), pop3_port: 110, ..mailc::Account::default() };
    t.check("a POP3 account's text stays", !pop.to_text().contains("imap"));
    // the first session: folders and the inbox, nothing marked as read
    let _ = e.apply(Action::MailFetch { dev: PC1 });
    run_for(&mut e, 3 * SEC);
    let st = e.file_store(PC1).cloned();
    let folders = st.as_ref().map(mailc::imap_folders).unwrap_or_default();
    t.eq("the server's folders", folders, alloc::vec![s("INBOX"), s("Sent"), s("Trash")]);
    let inbox = st.as_ref().map(|x| mailc::folder(x, &mailc::imap_dir("INBOX"))).unwrap_or_default();
    t.eq("three messages", inbox.len(), 3);
    t.check("with their subjects", inbox.iter().any(|m| m.subject == "Rechnung"));
    t.eq("uids in the names", mailc::uid_of_path("/root/Mail/imap/INBOX/2.eml"), Some(2));
    let marks = st.as_ref().map(|x| mailc::imap_marks(x, "INBOX")).unwrap_or_default();
    t.eq("all unread", marks.values().filter(|m| m.0).count(), 0);
    t.eq("still unread on the server", files_of(&e, MAIL, "/var/mail/anna/new").len(), 3);
    t.check("the status line", e.mail_client(PC1).and_then(|v| v.last_fetch).is_some_and(|o| o.ok && o.count == 3 && o.msg.text().contains("INBOX auf 10.0.0.10 abgeglichen, 3 neue Nachrichten")));
    t.check("the note", mail_notes(&e, PC1).iter().any(|n| matches!(n, jfn_sim::mailtrace::MNote::ClientSynced { n: 3, .. })));
    // again: nothing new, no bodies
    let before = metric(&e, "fetched");
    let _ = e.apply(Action::MailFetch { dev: PC1 });
    run_for(&mut e, 3 * SEC);
    t.eq("nothing fetched again", metric(&e, "fetched"), before);
    t.check("nothing new", e.mail_client(PC1).and_then(|v| v.last_fetch).is_some_and(|o| o.count == 0));
    // read and flagged: on the server, and back in the copy
    imap_op(&mut e, PC1, ImapOp::Flag { folder: s("INBOX"), uid: 1, flag: s("\\Seen"), on: true });
    imap_op(&mut e, PC1, ImapOp::Flag { folder: s("INBOX"), uid: 2, flag: s("\\Flagged"), on: true });
    let st = e.file_store(PC1).cloned();
    let marks = st.as_ref().map(|x| mailc::imap_marks(x, "INBOX")).unwrap_or_default();
    t.eq("read in the copy", marks.get(&1).copied(), Some((true, false)));
    t.eq("flagged in the copy", marks.get(&2).copied(), Some((false, true)));
    t.check("read on the server", e.file_store(MAIL).is_some_and(|x| x.exists("/var/mail/anna/cur/1767225601.M1Q1.mail:2,S")));
    t.check("flagged on the server", e.file_store(MAIL).is_some_and(|x| x.exists("/var/mail/anna/cur/1767225602.M2Q2.mail:2,F")));
    imap_op(&mut e, PC1, ImapOp::Flag { folder: s("INBOX"), uid: 2, flag: s("\\Flagged"), on: false });
    let marks = e.file_store(PC1).map(|x| mailc::imap_marks(x, "INBOX")).unwrap_or_default();
    t.eq("unflagged", marks.get(&2).copied(), Some((false, false)));
    // delete: to Trash, there for good
    imap_op(&mut e, PC1, ImapOp::Delete { folder: s("INBOX"), uid: 3 });
    let n = |e: &Engine, f: &str| e.file_store(PC1).map_or(0, |x| mailc::folder(x, &mailc::imap_dir(f)).len());
    t.eq("two in the inbox", n(&e, "INBOX"), 2);
    t.eq("one on the server in Trash", files_of(&e, MAIL, "/var/mail/anna/.Trash/cur").len(), 1);
    imap_op(&mut e, PC1, ImapOp::Sync { folder: s("Trash") });
    t.eq("the copy of Trash", n(&e, "Trash"), 1);
    let trash = e.file_store(PC1).map(|x| mailc::folder(x, &mailc::imap_dir("Trash"))).unwrap_or_default();
    let uid = trash.first().and_then(|m| mailc::uid_of_path(&m.path)).unwrap_or(0);
    imap_op(&mut e, PC1, ImapOp::Delete { folder: s("Trash"), uid });
    t.eq("gone from Trash", files_of(&e, MAIL, "/var/mail/anna/.Trash/cur").len(), 0);
    t.eq("the copy too", n(&e, "Trash"), 0);
    // a message sent lands in Sent on the server (and in its copy), not in the local folder
    let draft = MailDraft { to: s("bob@a.lab"), subject: s("Hallo Bob"), text: s("Viele Grüße"), ..MailDraft::default() };
    let _ = e.apply(Action::MailSend { dev: PC1, draft });
    run_for(&mut e, 5 * SEC);
    t.eq("in Sent on the server", files_of(&e, MAIL, "/var/mail/anna/.Sent/cur").len(), 1);
    t.eq("seen there", files_of(&e, MAIL, "/var/mail/anna/.Sent/cur").iter().filter(|n| n.ends_with(":2,S")).count(), 1);
    t.eq("in the copy", n(&e, "Sent"), 1);
    t.check("not in the POP3 folder", e.file_store(PC1).is_some_and(|x| mailc::folder(x, mailc::SENT).is_empty()));
    t.check("delivered", !files_of(&e, MAIL, "/var/mail/bob/new").is_empty());
    // folders
    imap_op(&mut e, PC1, ImapOp::CreateFolder { name: s("Projekte") });
    t.check("a new folder", e.file_store(PC1).map(mailc::imap_folders).unwrap_or_default().contains(&s("Projekte")));
    t.check("on the server", e.file_store(MAIL).is_some_and(|x| x.is_dir("/var/mail/anna/.Projekte")));
    imap_op(&mut e, PC1, ImapOp::DeleteFolder { name: s("Projekte") });
    t.check("deleted", !e.file_store(PC1).map(mailc::imap_folders).unwrap_or_default().contains(&s("Projekte")));
    // mail -H shows the inbox's copy of an IMAP account
    let o = cli(&mut e, PC1, "mail -H", 10 * jfn_sim::MS);
    t.check("mail -H", o.contains("Hallo Anna"));
    // a wrong password: the program says so and nothing changes
    let mut e2 = engine();
    account(&mut e2, PC2, "bob");
    let st = e2.file_store(PC2).cloned();
    if let Some(mut x) = st {
        let text = String::from_utf8_lossy(x.read(mailc::ACCOUNT).unwrap_or(&[])).replace("bobpw", "falsch");
        let _ = x.write(mailc::ACCOUNT, text.as_bytes(), 1_767_225_600);
        let _ = e2.apply(Action::Files { dev: PC2, files: x.records() });
    }
    let _ = e2.apply(Action::MailFetch { dev: PC2 });
    run_for(&mut e2, 3 * SEC);
    t.check("refused", e2.mail_client(PC2).and_then(|v| v.last_fetch).is_some_and(|o| !o.ok && o.msg.text().contains("lehnt die Anmeldung ab")));
    // no server there
    let mut e3 = engine();
    account(&mut e3, PC2, "bob");
    let _ = e3.apply(Action::Power { dev: MAIL, on: false });
    let _ = e3.apply(Action::MailFetch { dev: PC2 });
    run_for(&mut e3, 40 * SEC);
    t.check("no connection", e3.mail_client(PC2).and_then(|v| v.last_fetch).is_some_and(|o| !o.ok));
    // the mailbox helpers
    t.eq("folder dir", mailbox::folder_dir("/var/mail", "a", "INBOX"), s("/var/mail/a"));
    t.eq("folder dir 2", mailbox::folder_dir("/var/mail", "a", "Sent"), s("/var/mail/a/.Sent"));
    t.eq("set flags", mailbox::set_flags("x", "TSF"), s("x:2,FST"));
    t.eq("flags of", mailbox::flags_of("x:2,FS"), s("FS"));
    t.eq("words", mailbox::imap_flags("FS", true), alloc::vec!["\\Flagged", "\\Seen", "\\Recent"]);
    t.eq("letter", mailbox::letter_of("\\SEEN"), Some('S'));
    t.eq("no letter", mailbox::letter_of("$Junk"), None);
}

fn curl(t: &mut T) {
    let mut e = engine();
    let o = cli(&mut e, PC1, "curl imap://10.0.0.10/ -u anna:geheim", 2 * SEC);
    t.check("folders", o.contains("* LIST") && o.contains("INBOX") && o.contains("Trash"));
    let o = cli(&mut e, PC1, "curl \"imap://10.0.0.10/INBOX;UID=2\" -u anna:geheim", 2 * SEC);
    t.check("a message", o.contains("Subject: Rechnung") && o.contains("Zweiter Text"));
    let o = cli(&mut e, PC1, "curl \"imap://10.0.0.10/INBOX?UNSEEN\" -u anna:geheim", 2 * SEC);
    t.check("a search", o.contains("* SEARCH 1 3"));
    let o = cli(&mut e, PC1, "curl imap://10.0.0.10/INBOX -u anna:geheim -X \"STORE 3 +FLAGS (\\Flagged)\"", 2 * SEC);
    t.check("-X", o.contains("* 3 FETCH (FLAGS (\\Flagged))"));
    let o = cli(&mut e, PC1, "curl imap://anna:falsch@10.0.0.10/", 3 * SEC);
    t.check("refused", o.contains("curl: (67) Login denied"));
    let o = cli(&mut e, PC1, "curl imap://10.0.0.10/Nirgends -u anna:geheim", 2 * SEC);
    t.check("no such folder", o.contains("curl: (78) Select failed"));
    let o = cli(&mut e, PC1, "curl -v imap://10.0.0.10/ -u anna:geheim", 2 * SEC);
    t.check("-v", o.contains("> A001 CAPABILITY") && o.contains("< A001 OK"));
    let o = cli(&mut e, PC1, "curl -o /tmp/m.txt \"imap://10.0.0.10/INBOX;UID=1\" -u anna:geheim", 2 * SEC);
    t.check("-o: nothing on the console", !o.contains("Der erste Text"));
    t.check("-o: in the file", e.file_store(PC1).is_some_and(|x| x.read("/tmp/m.txt").is_ok_and(|b| String::from_utf8_lossy(b).contains("Der erste Text"))));
    // nobody at home
    let _ = e.apply(Action::Power { dev: MAIL, on: false });
    let o = cli(&mut e, PC1, "curl imap://10.0.0.10/ -u anna:geheim", 35 * SEC);
    t.check("no connection", o.contains("curl: ("));
}

fn determinism(t: &mut T) {
    let run = || {
        let mut e = engine();
        account(&mut e, PC1, "anna");
        let _ = e.apply(Action::MailFetch { dev: PC1 });
        run_for(&mut e, 3 * SEC);
        imap_op(&mut e, PC1, ImapOp::Flag { folder: s("INBOX"), uid: 1, flag: s("\\Seen"), on: true });
        let frames: Vec<(u64, Vec<u8>)> = e.trace.hops().map(|h| (h.t_rx, h.bytes.to_vec())).collect();
        (frames, e.files(MAIL), jfn_sim::keyframe::footprint(&e) > 0)
    };
    let (a, b) = (run(), run());
    t.check("frames", !a.0.is_empty());
    t.eq("twice the same bytes", a.0 == b.0, true);
    t.eq("the same files", a.1 == b.1, true);
    t.check("a keyframe counts it", a.2);
}
