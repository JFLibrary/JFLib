// imapd -- the IMAP server (a subset of RFC 9051 and RFC 3501): "imap-server" on TCP port 143 lets
// its users keep their mail on the server, in the same Maildir the SMTP server delivers to and POP3
// reads (mailbox.rs), with folders and flags
//
//   config (JSON, every member may be left out)
//     {"port": 143, "users": [{"name": "bob", "password": "geheim"}], "mailbox": "/var/mail",
//      "fail_delay_ms": 1000, "max_fails": 3, "idle_ms": 1800000, "banner": "JFNetworker IMAP ready"}
//   ImapServer::from_config(&config)
//
// Not authenticated: CAPABILITY, NOOP, LOGOUT, LOGIN, AUTHENTICATE PLAIN (with or without the
// initial response).  A wrong password is answered after fail_delay_ms with NO
// [AUTHENTICATIONFAILED], after max_fails of them the connection ends.  The first login makes the
// mailbox with its folders Sent and Trash.
//
// Authenticated: SELECT, EXAMINE, CREATE, DELETE, RENAME, SUBSCRIBE and UNSUBSCRIBE (taken, every
// folder counts as subscribed), LIST and LSUB with * and %, STATUS, APPEND.  Selected: CHECK, CLOSE,
// UNSELECT, EXPUNGE, SEARCH, FETCH, STORE, COPY, MOVE, the UID forms of the last four, IDLE (a new
// message shows within 2 s) and NOOP, which tell what changed.
//
// FETCH: FLAGS, UID, RFC822.SIZE, INTERNALDATE, ENVELOPE, BODY[], BODY[HEADER], BODY[TEXT],
// BODY[HEADER.FIELDS (...)], their .PEEK forms, RFC822, RFC822.HEADER, RFC822.TEXT, ALL, FAST, FULL;
// no BODYSTRUCTURE and no partial fetches.  Bodies go after the time of a read on the device's disk;
// BODY[...] without .PEEK sets \Seen.  STORE takes \Seen, \Answered, \Flagged, \Deleted and \Draft;
// keywords are not kept.  SEARCH: netpkt::imap::SearchKey.
//
// The messages are the files of the Maildir as they lie (mailbox.rs): flags in the names, UIDs in a
// file .uidlist of the folder, which a SELECT brings up to date.  A message counts as \Recent while
// it lies in new/ (the SMTP server puts it there, the first flag moves it to cur/): the mailbox stays
// as the server found it until somebody changes something.  A change of files is one step of the
// project.  Several sessions may hold one mailbox, and POP3 as well; what another one removes comes
// up as EXPUNGE at the next NOOP.  The texts after OK, NO and BAD are our own words, and the
// greeting names this simulator's server.
//
// Metrics: sessions, active, logins, failed_logins, fetched, stored, expunged, appended, bytes_sent.
// Notes (mailtrace.rs) and log lines (tag "imapd").

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use jfn_model::files::FsErr;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::imap::{self, Command, FetchItem, Item, Line, LineReader, Read, SearchKey, Section, SeqSet};
use netpkt::imf::{self, Message};
use netpkt::IpAddr;

use crate::http::Wheel;
use crate::kernel::{Ns, MS, SEC};
use crate::mailbox::{self, FolderMsg, INBOX, SENT, TRASH};
use crate::mailtrace::MNote;
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::smtpd::{users_of, MailUser};
use crate::tcp::SockOpts;
use crate::trace::Note;

const K_WHEEL: u32 = 0;
const SEV_WARN: u8 = 4;
const SEV_INFO: u8 = 6;
/// how often a session in IDLE looks at its folder
const IDLE_LOOK: Ns = 2 * SEC;
const CAPS: &str = "IMAP4rev1 AUTH=PLAIN SASL-IR IDLE MOVE UNSELECT LITERAL+";

/// the folder a session has selected
#[derive(Clone, Debug)]
struct Sel {
    folder: String,
    dir: String,
    readonly: bool,
    msgs: Vec<FolderMsg>,
}

/// one connection of a client
#[derive(Clone, Debug)]
struct Session {
    peer: IpAddr,
    reader: LineReader,
    user: Option<String>,
    fails: u8,
    sel: Option<Sel>,
    /// the tag of a running IDLE
    idling: Option<String>,
    /// the tag of an AUTHENTICATE that waits for its answer
    auth: Option<String>,
    /// waiting (a wrong password, a read): lines wait meanwhile
    busy: bool,
    queue: VecDeque<Line>,
    out: Vec<u8>,
    idle: Option<u64>,
    closing: bool,
}

/// a deadline
#[derive(Clone, Debug)]
enum Due {
    /// the answer to a wrong password: (session, tag)
    Refuse(u32, String),
    /// a FETCH read its messages from the disk
    Fetch { sock: u32, tag: String, uid: bool, set: String, items: Vec<FetchItem> },
    /// a session in IDLE looks again
    Look(u32),
    Idle(u32),
}

/// the IMAP server
#[derive(Clone, Debug)]
pub struct ImapServer {
    pub port: u16,
    pub users: Vec<MailUser>,
    pub mailbox: String,
    pub fail_delay: Ns,
    pub max_fails: u8,
    pub idle: Ns,
    pub banner: String,
    listener: Option<u32>,
    sessions: BTreeMap<u32, Session>,
    wheel: Wheel<Due>,
    seq: u32,
    pub sessions_n: u64,
    pub logins: u64,
    pub failed_logins: u64,
    pub fetched: u64,
    pub stored: u64,
    pub expunged: u64,
    pub appended: u64,
    pub bytes_sent: u64,
}

impl Default for ImapServer {
    fn default() -> ImapServer {
        ImapServer::new()
    }
}

/// what SEARCH looks at
struct Facts<'a> {
    seq: u32,
    max_seq: u32,
    max_uid: u32,
    m: &'a FolderMsg,
    msg: Option<Message>,
    text: String,
}

fn matches_key(k: &SearchKey, f: &Facts) -> bool {
    let has = |c: char| f.m.letters.contains(c);
    let header = |name: &str, want: &str| f.msg.as_ref().and_then(|m| m.get(name)).is_some_and(|v| imf::decode_words(v).to_ascii_lowercase().contains(&want.to_ascii_lowercase()));
    match k {
        SearchKey::All => true,
        SearchKey::Seen => has('S'),
        SearchKey::Unseen => !has('S'),
        SearchKey::Answered => has('R'),
        SearchKey::Unanswered => !has('R'),
        SearchKey::Flagged => has('F'),
        SearchKey::Unflagged => !has('F'),
        SearchKey::Deleted => has('T'),
        SearchKey::Undeleted => !has('T'),
        SearchKey::Recent => f.m.recent,
        SearchKey::From(v) => header("From", v),
        SearchKey::To(v) => header("To", v),
        SearchKey::Subject(v) => header("Subject", v),
        SearchKey::Body(v) | SearchKey::Text(v) => f.text.to_ascii_lowercase().contains(&v.to_ascii_lowercase()),
        SearchKey::Larger(n) => f.m.size > *n,
        SearchKey::Smaller(n) => f.m.size < *n,
        SearchKey::Uid(s) => s.contains(f.m.uid, f.max_uid),
        SearchKey::Seq(s) => s.contains(f.seq, f.max_seq),
        SearchKey::Not(x) => !matches_key(x, f),
        SearchKey::Or(a, b) => matches_key(a, f) || matches_key(b, f),
        SearchKey::And(v) => v.iter().all(|x| matches_key(x, f)),
    }
}

/// a LIST pattern: * any characters, % any but the delimiter
fn list_match(pat: &str, name: &str) -> bool {
    let (p, n) = (pat.as_bytes(), name.as_bytes());
    fn go(p: &[u8], n: &[u8]) -> bool {
        match p.first() {
            None => n.is_empty(),
            Some(b'*') => (0..=n.len()).any(|i| go(&p[1..], &n[i..])),
            Some(b'%') => (0..=n.len()).take_while(|&i| i == 0 || n[i - 1] != b'/').any(|i| go(&p[1..], &n[i..])),
            Some(c) => n.first().is_some_and(|x| x.eq_ignore_ascii_case(c)) && go(&p[1..], &n[1..]),
        }
    }
    go(p, n)
}

/// a folder name we keep: no path characters
fn name_ok(n: &str) -> bool {
    !n.is_empty() && !n.contains(['/', '\\', '.']) && !n.chars().any(char::is_control) && !n.eq_ignore_ascii_case(INBOX)
}

/// the flags of a message as IMAP writes them: "(\Seen \Flagged)"
fn flag_list(m: &FolderMsg) -> String {
    format!("({})", mailbox::imap_flags(&m.letters, m.recent).join(" "))
}

/// FLAGS of a STORE: the letters they name
fn letters_of(items: &[Item]) -> String {
    let mut s = String::new();
    let mut add = |w: &str| {
        if let Some(c) = mailbox::letter_of(w) {
            s.push(c);
        }
    };
    for i in items {
        match i {
            Item::Atom(w) => add(w),
            Item::List(l) => {
                for x in l {
                    if let Item::Atom(w) = x {
                        add(w);
                    }
                }
            }
            _ => {}
        }
    }
    s
}

impl ImapServer {
    pub fn new() -> ImapServer {
        ImapServer {
            port: imap::PORT_IMAP,
            users: Vec::new(),
            mailbox: String::from(mailbox::SPOOL),
            fail_delay: SEC,
            max_fails: 3,
            idle: 1800 * SEC,
            banner: String::from("JFNetworker IMAP ready"),
            listener: None,
            sessions: BTreeMap::new(),
            wheel: Wheel::new(K_WHEEL),
            seq: 0,
            sessions_n: 0,
            logins: 0,
            failed_logins: 0,
            fetched: 0,
            stored: 0,
            expunged: 0,
            appended: 0,
            bytes_sent: 0,
        }
    }

    /// the server of a project's definition (svcact.rs)
    pub fn from_config(c: &Value) -> Result<ImapServer, Msg> {
        let mut s = ImapServer::new();
        let num = |k: &str| c.get(k).and_then(|v| v.as_u64());
        let text = |k: &str| c.get(k).and_then(|v| v.as_str()).map(|x| String::from(x.trim()));
        if let Some(p) = num("port") {
            s.port = u16::try_from(p).ok().filter(|&p| p > 0).ok_or_else(|| msg!("Port {} gibt es nicht.", p))?;
        }
        s.users = users_of(c.get("users"))?;
        if let Some(m) = text("mailbox") {
            if !m.starts_with('/') {
                return Err(msg!("\u{201e}mailbox\u{201c} ist kein Verzeichnis."));
            }
            s.mailbox = m;
        }
        if let Some(t) = num("fail_delay_ms") {
            s.fail_delay = t.min(60_000) * MS;
        }
        if let Some(n) = num("max_fails") {
            s.max_fails = n.clamp(1, 100) as u8;
        }
        if let Some(t) = num("idle_ms") {
            s.idle = t.max(1000) * MS;
        }
        if let Some(b) = text("banner").filter(|b| !b.is_empty()) {
            s.banner = b;
        }
        Ok(s)
    }

    fn send(&mut self, os: &mut OsApi, sock: u32, b: &[u8]) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if !s.out.is_empty() {
            s.out.extend_from_slice(b);
            return;
        }
        let n = os.tcp_send(sock, b);
        s.out.extend_from_slice(&b[n..]);
    }

    fn ok(&mut self, os: &mut OsApi, sock: u32, tag: &str, text: &str) {
        self.send(os, sock, &imap::ok(tag, None, text));
    }

    fn no(&mut self, os: &mut OsApi, sock: u32, tag: &str, code: Option<&str>, verb: &str, text: &str) {
        os.note(Note::Mail(MNote::ImapRefused { cmd: String::from(verb) }));
        self.send(os, sock, &imap::no(tag, code, text));
    }

    fn bad(&mut self, os: &mut OsApi, sock: u32, tag: &str, verb: &str, text: &str) {
        os.note(Note::Mail(MNote::ImapRefused { cmd: String::from(verb) }));
        self.send(os, sock, &imap::bad(tag, text));
    }

    fn touch(&mut self, os: &mut OsApi, sock: u32) {
        let at = os.now() + self.idle;
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if let Some(id) = s.idle.take() {
            self.wheel.cancel(id);
        }
        let id = self.wheel.add(at, Due::Idle(sock));
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.idle = Some(id);
        }
    }

    fn end(&mut self, os: &mut OsApi, sock: u32) {
        if let Some(s) = self.sessions.remove(&sock)
            && let Some(id) = s.idle
        {
            self.wheel.cancel(id);
        }
        os.tcp_close(sock);
    }

    fn closing(&mut self, os: &mut OsApi, sock: u32) {
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.closing = true;
        }
        os.tcp_shutdown(sock);
    }

    /// bytes of a client
    fn bytes(&mut self, os: &mut OsApi, sock: u32, data: &[u8]) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        if s.closing {
            return;
        }
        s.reader.push(data);
        loop {
            let Some(s) = self.sessions.get_mut(&sock) else { return };
            match s.reader.next() {
                Some(Read::NeedLiteral(_)) => self.send(os, sock, &imap::cont("Go on")),
                Some(Read::Line(l)) => s.queue.push_back(l),
                None => break,
            }
        }
        self.drain(os, sock);
    }

    fn drain(&mut self, os: &mut OsApi, sock: u32) {
        while let Some(l) = self.sessions.get_mut(&sock).and_then(|s| if s.busy || s.closing { None } else { s.queue.pop_front() }) {
            self.line(os, sock, &l);
        }
    }

    fn line(&mut self, os: &mut OsApi, sock: u32, l: &Line) {
        self.touch(os, sock);
        let Some(s) = self.sessions.get(&sock) else { return };
        // the end of IDLE
        if let Some(tag) = s.idling.clone() {
            if l.text.eq_ignore_ascii_case(b"DONE") {
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.idling = None;
                }
                self.ok(os, sock, &tag, "IDLE is over");
            }
            return;
        }
        // the answer of AUTHENTICATE
        if let Some(tag) = s.auth.clone() {
            if let Some(s) = self.sessions.get_mut(&sock) {
                s.auth = None;
            }
            let text = String::from_utf8_lossy(&l.text).into_owned();
            return self.auth_plain(os, sock, &tag, &text);
        }
        let Some(c) = Command::parse(l) else {
            let text = String::from_utf8_lossy(&l.text).into_owned();
            let tag = text.split(' ').next().unwrap_or("*");
            let tag = if tag.is_empty() { "*" } else { tag };
            return self.bad(os, sock, tag, "?", "A command needs a tag and a word");
        };
        self.command(os, sock, &c);
    }

    fn command(&mut self, os: &mut OsApi, sock: u32, c: &Command) {
        let tag = c.tag.as_str();
        let v = c.verb.as_str();
        let (logged, selected) = match self.sessions.get(&sock) {
            Some(s) => (s.user.is_some(), s.sel.is_some()),
            None => return,
        };
        match v {
            "CAPABILITY" => {
                self.send(os, sock, &imap::untagged(&format!("CAPABILITY {}", CAPS)));
                self.ok(os, sock, tag, "That is what this server can do");
            }
            "NOOP" => {
                self.sync(os, sock);
                self.ok(os, sock, tag, "Still here");
            }
            "LOGOUT" => {
                self.send(os, sock, &imap::untagged("BYE Goodbye"));
                self.ok(os, sock, tag, "Logged out");
                self.closing(os, sock);
            }
            "LOGIN" if !logged => {
                let (Some(u), Some(p)) = (c.arg(0), c.arg(1)) else {
                    return self.bad(os, sock, tag, v, "LOGIN needs a user name and a password");
                };
                self.login(os, sock, tag, &u, &p);
            }
            "AUTHENTICATE" if !logged => {
                if !c.arg(0).is_some_and(|m| m.eq_ignore_ascii_case("PLAIN")) {
                    return self.no(os, sock, tag, None, v, "Only PLAIN is on offer");
                }
                match c.arg(1) {
                    Some(ir) => self.auth_plain(os, sock, tag, &ir),
                    None => {
                        if let Some(s) = self.sessions.get_mut(&sock) {
                            s.auth = Some(String::from(tag));
                        }
                        self.send(os, sock, &imap::cont(""));
                    }
                }
            }
            "LOGIN" | "AUTHENTICATE" => self.bad(os, sock, tag, v, "Logged in already"),
            _ if !logged => {
                if imap::verb_known(v) {
                    self.bad(os, sock, tag, v, "Log in first")
                } else {
                    self.bad(os, sock, tag, v, "Unknown command")
                }
            }
            "SELECT" | "EXAMINE" => self.select(os, sock, tag, c, v == "EXAMINE"),
            "CREATE" => self.create(os, sock, tag, c),
            "DELETE" => self.delete(os, sock, tag, c),
            "RENAME" => self.rename(os, sock, tag, c),
            "SUBSCRIBE" | "UNSUBSCRIBE" => self.ok(os, sock, tag, "Every folder counts as subscribed"),
            "LIST" | "LSUB" => self.list(os, sock, tag, c),
            "STATUS" => self.status_cmd(os, sock, tag, c),
            "APPEND" => self.append(os, sock, tag, c),
            "IDLE" => {
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.idling = Some(String::from(tag));
                }
                self.send(os, sock, &imap::cont("Waiting for news"));
                let at = os.now() + IDLE_LOOK;
                self.wheel.add(at, Due::Look(sock));
                self.wheel.arm(os);
            }
            _ if !selected => {
                if imap::verb_known(v) {
                    self.bad(os, sock, tag, v, "Select a folder first")
                } else {
                    self.bad(os, sock, tag, v, "Unknown command")
                }
            }
            "CHECK" => {
                self.sync(os, sock);
                self.ok(os, sock, tag, "Checked");
            }
            "CLOSE" => {
                self.expunge(os, sock, false);
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.sel = None;
                }
                self.ok(os, sock, tag, "Closed");
            }
            "UNSELECT" => {
                if let Some(s) = self.sessions.get_mut(&sock) {
                    s.sel = None;
                }
                self.ok(os, sock, tag, "No folder is selected now");
            }
            "EXPUNGE" => {
                if self.readonly(sock) {
                    return self.no(os, sock, tag, None, v, "This folder was opened to read only");
                }
                self.expunge(os, sock, true);
                self.ok(os, sock, tag, "Expunged");
            }
            "SEARCH" => self.search(os, sock, tag, &c.args, false),
            "FETCH" => self.fetch(os, sock, tag, &c.args, false),
            "STORE" => self.store(os, sock, tag, &c.args, false),
            "COPY" => self.copy(os, sock, tag, &c.args, false, false),
            "MOVE" => self.copy(os, sock, tag, &c.args, false, true),
            "UID" => {
                let sub = c.arg(0).unwrap_or_default().to_ascii_uppercase();
                let rest = &c.args[c.args.len().min(1)..];
                match sub.as_str() {
                    "SEARCH" => self.search(os, sock, tag, rest, true),
                    "FETCH" => self.fetch(os, sock, tag, rest, true),
                    "STORE" => self.store(os, sock, tag, rest, true),
                    "COPY" => self.copy(os, sock, tag, rest, true, false),
                    "MOVE" => self.copy(os, sock, tag, rest, true, true),
                    _ => self.bad(os, sock, tag, v, "UID goes with SEARCH, FETCH, STORE, COPY or MOVE"),
                }
            }
            x if imap::verb_known(x) => self.bad(os, sock, tag, x, "Not available here"),
            x => self.bad(os, sock, tag, x, "Unknown command"),
        }
    }

    fn readonly(&self, sock: u32) -> bool {
        self.sessions.get(&sock).and_then(|s| s.sel.as_ref()).is_some_and(|x| x.readonly)
    }

    fn user_of(&self, sock: u32) -> String {
        self.sessions.get(&sock).and_then(|s| s.user.clone()).unwrap_or_default()
    }

    /// LOGIN and AUTHENTICATE PLAIN
    fn login(&mut self, os: &mut OsApi, sock: u32, tag: &str, name: &str, pw: &str) {
        let peer = self.sessions.get(&sock).map(|s| s.peer).unwrap_or(crate::os::ANY4);
        let user = self.users.iter().find(|u| u.name.eq_ignore_ascii_case(name) && !u.password.is_empty() && u.password == pw).map(|u| u.name.clone());
        let Some(user) = user else {
            self.failed_logins += 1;
            os.note(Note::Mail(MNote::ImapLoginFailed { user: String::from(name) }));
            os.log(SEV_WARN, "imapd", &format!("login of {} from {} refused", name, peer));
            let at = os.now() + self.fail_delay;
            if let Some(s) = self.sessions.get_mut(&sock) {
                s.busy = true;
                s.fails += 1;
            }
            self.wheel.add(at, Due::Refuse(sock, String::from(tag)));
            self.wheel.arm(os);
            return;
        };
        self.logins += 1;
        // the mailbox with its folders comes with the first login
        let mut made = false;
        for f in [INBOX, SENT, TRASH] {
            let dir = mailbox::folder_dir(&self.mailbox, &user, f);
            let cur = format!("{}/cur", dir);
            if !os.files().is_some_and(|st| st.is_dir(&cur)) {
                let _ = os.store_mkdir(&cur);
                made = true;
            }
        }
        if made {
            os.store_report(&mailbox::maildir(&self.mailbox, &user));
        }
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.user = Some(user.clone());
        }
        os.note(Note::Mail(MNote::ImapLogin { user: user.clone() }));
        os.log(SEV_INFO, "imapd", &format!("{} logged in from {}", user, peer));
        self.send(os, sock, &imap::ok(tag, Some(&format!("CAPABILITY {}", CAPS)), "Logged in"));
    }

    /// the base64 of AUTHENTICATE PLAIN
    fn auth_plain(&mut self, os: &mut OsApi, sock: u32, tag: &str, b64: &str) {
        if b64.trim() == "*" {
            return self.bad(os, sock, tag, "AUTHENTICATE", "Called off");
        }
        let Some(raw) = imf::b64_decode(b64.trim().as_bytes()) else {
            return self.bad(os, sock, tag, "AUTHENTICATE", "That is no base64");
        };
        let parts: Vec<&[u8]> = raw.split(|&c| c == 0).collect();
        if parts.len() != 3 {
            return self.bad(os, sock, tag, "AUTHENTICATE", "PLAIN has three parts");
        }
        let (u, p) = (String::from_utf8_lossy(parts[1]).into_owned(), String::from_utf8_lossy(parts[2]).into_owned());
        self.login(os, sock, tag, &u, &p);
    }

    fn refuse_now(&mut self, os: &mut OsApi, sock: u32, tag: &str) {
        let Some(s) = self.sessions.get_mut(&sock) else { return };
        s.busy = false;
        let last = s.fails >= self.max_fails;
        self.send(os, sock, &imap::no(tag, Some("AUTHENTICATIONFAILED"), "Wrong user name or password"));
        if last {
            self.send(os, sock, &imap::untagged("BYE Too many failed logins, goodbye"));
            return self.closing(os, sock);
        }
        self.drain(os, sock);
    }

    /// a folder's messages now; a changed .uidlist is written (one step of the project)
    fn read_folder(&mut self, os: &mut OsApi, dir: &str) -> (u32, u32, Vec<FolderMsg>) {
        let fresh = (os.rand() | 1) & 0x7FFF_FFFF;
        let (msgs, write) = match os.files() {
            Some(st) => mailbox::folder_msgs(st, dir, fresh),
            None => (Vec::new(), None),
        };
        if let Some((_, text)) = write {
            let p = mailbox::uidlist_file(dir);
            let _ = os.store_put(&p, &text);
            os.store_report(&p);
        }
        let (val, next, _) = os.files().map(|st| mailbox::uidlist(st, dir)).unwrap_or((0, 1, BTreeMap::new()));
        (val, next, msgs)
    }

    fn resolve_folder(&self, os: &OsApi, sock: u32, name: &str) -> Option<(String, String)> {
        let user = self.user_of(sock);
        let canon = if name.eq_ignore_ascii_case(INBOX) { String::from(INBOX) } else { String::from(name) };
        let dir = mailbox::folder_dir(&self.mailbox, &user, &canon);
        if canon == INBOX || os.files().is_some_and(|st| st.is_dir(&dir)) { Some((canon, dir)) } else { None }
    }

    fn select(&mut self, os: &mut OsApi, sock: u32, tag: &str, c: &Command, readonly: bool) {
        let Some(name) = c.arg(0) else {
            return self.bad(os, sock, tag, &c.verb, "Which folder?");
        };
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.sel = None;
        }
        let Some((folder, dir)) = self.resolve_folder(os, sock, &name) else {
            return self.no(os, sock, tag, Some("NONEXISTENT"), &c.verb, "There is no such folder");
        };
        let (val, next, msgs) = self.read_folder(os, &dir);
        let n = msgs.len() as u32;
        let recent = msgs.iter().filter(|m| m.recent).count();
        let unseen = msgs.iter().position(|m| !m.letters.contains('S'));
        let mut b = Vec::new();
        b.extend_from_slice(&imap::untagged(&format!("{} EXISTS", n)));
        b.extend_from_slice(&imap::untagged(&format!("{} RECENT", recent)));
        b.extend_from_slice(&imap::untagged("FLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)"));
        b.extend_from_slice(&imap::untagged("OK [PERMANENTFLAGS (\\Answered \\Flagged \\Deleted \\Seen \\Draft)] These flags stay"));
        if let Some(u) = unseen {
            b.extend_from_slice(&imap::untagged(&format!("OK [UNSEEN {}] The first one not read", u + 1)));
        }
        b.extend_from_slice(&imap::untagged(&format!("OK [UIDVALIDITY {}] UIDs hold", val)));
        b.extend_from_slice(&imap::untagged(&format!("OK [UIDNEXT {}] The next UID", next)));
        b.extend_from_slice(&imap::ok(tag, Some(if readonly { "READ-ONLY" } else { "READ-WRITE" }), if readonly { "Opened to read" } else { "Opened" }));
        self.send(os, sock, &b);
        let user = self.user_of(sock);
        os.note(Note::Mail(MNote::ImapSelect { user, folder: folder.clone(), msgs: n }));
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.sel = Some(Sel { folder, dir, readonly, msgs });
        }
    }

    /// what changed in the selected folder: EXPUNGE, EXISTS, RECENT, FETCH (FLAGS)
    fn sync(&mut self, os: &mut OsApi, sock: u32) {
        let Some(sel) = self.sessions.get(&sock).and_then(|s| s.sel.clone()) else { return };
        let (_, _, now) = self.read_folder(os, &sel.dir);
        let mut b = Vec::new();
        let mut old = sel.msgs.clone();
        let mut i = 0;
        while i < old.len() {
            if now.iter().any(|m| m.uid == old[i].uid) {
                i += 1;
            } else {
                b.extend_from_slice(&imap::untagged(&format!("{} EXPUNGE", i + 1)));
                old.remove(i);
            }
        }
        for (k, m) in old.iter().enumerate() {
            if let Some(x) = now.iter().find(|y| y.uid == m.uid)
                && (x.letters != m.letters || x.recent != m.recent)
            {
                b.extend_from_slice(&imap::untagged(&format!("{} FETCH (FLAGS {})", k + 1, flag_list(x))));
            }
        }
        if now.len() > old.len() {
            b.extend_from_slice(&imap::untagged(&format!("{} EXISTS", now.len())));
            b.extend_from_slice(&imap::untagged(&format!("{} RECENT", now.iter().filter(|m| m.recent).count())));
        }
        if !b.is_empty() {
            self.send(os, sock, &b);
        }
        if let Some(s) = self.sessions.get_mut(&sock).and_then(|s| s.sel.as_mut()) {
            s.msgs = now;
        }
    }

    fn create(&mut self, os: &mut OsApi, sock: u32, tag: &str, c: &Command) {
        let Some(name) = c.arg(0).filter(|n| name_ok(n)) else {
            return self.no(os, sock, tag, Some("CANNOT"), "CREATE", "A folder name has no / . or control characters");
        };
        let user = self.user_of(sock);
        let dir = mailbox::folder_dir(&self.mailbox, &user, &name);
        if os.files().is_some_and(|st| st.is_dir(&dir)) {
            return self.no(os, sock, tag, Some("ALREADYEXISTS"), "CREATE", "That folder exists already");
        }
        let _ = os.store_mkdir(&format!("{}/cur", dir));
        os.store_report(&dir);
        os.note(Note::Mail(MNote::ImapFolder { what: String::from("CREATE"), folder: name }));
        self.ok(os, sock, tag, "Created");
    }

    fn delete(&mut self, os: &mut OsApi, sock: u32, tag: &str, c: &Command) {
        let Some(name) = c.arg(0) else {
            return self.bad(os, sock, tag, "DELETE", "Which folder?");
        };
        if name.eq_ignore_ascii_case(INBOX) {
            return self.no(os, sock, tag, Some("CANNOT"), "DELETE", "INBOX stays");
        }
        let Some((_, dir)) = self.resolve_folder(os, sock, &name) else {
            return self.no(os, sock, tag, Some("NONEXISTENT"), "DELETE", "There is no such folder");
        };
        if self.sessions.get(&sock).and_then(|s| s.sel.as_ref()).is_some_and(|x| x.dir == dir)
            && let Some(s) = self.sessions.get_mut(&sock)
        {
            s.sel = None;
        }
        if let Some(st) = os.eng.os_mut(os.ni) {
            let _ = st.files.remove_tree(&dir);
        }
        os.store_report(&dir);
        os.note(Note::Mail(MNote::ImapFolder { what: String::from("DELETE"), folder: name }));
        self.ok(os, sock, tag, "Deleted");
    }

    fn rename(&mut self, os: &mut OsApi, sock: u32, tag: &str, c: &Command) {
        let (Some(a), Some(b)) = (c.arg(0), c.arg(1)) else {
            return self.bad(os, sock, tag, "RENAME", "From which name to which?");
        };
        if a.eq_ignore_ascii_case(INBOX) || !name_ok(&b) {
            return self.no(os, sock, tag, Some("CANNOT"), "RENAME", "INBOX keeps its name; a folder name has no / . or control characters");
        }
        let Some((_, from)) = self.resolve_folder(os, sock, &a) else {
            return self.no(os, sock, tag, Some("NONEXISTENT"), "RENAME", "There is no such folder");
        };
        let user = self.user_of(sock);
        let to = mailbox::folder_dir(&self.mailbox, &user, &b);
        if os.files().is_some_and(|st| st.is_dir(&to)) {
            return self.no(os, sock, tag, Some("ALREADYEXISTS"), "RENAME", "That folder exists already");
        }
        let r: Result<(), FsErr> = match os.eng.os_mut(os.ni) {
            Some(st) => st.files.rename(&from, &to),
            None => Err(FsErr::NotFound),
        };
        if r.is_err() {
            return self.no(os, sock, tag, Some("CANNOT"), "RENAME", "The folder cannot be renamed");
        }
        os.store_report(&to);
        os.note(Note::Mail(MNote::ImapFolder { what: String::from("RENAME"), folder: b }));
        self.ok(os, sock, tag, "Renamed");
    }

    fn list(&mut self, os: &mut OsApi, sock: u32, tag: &str, c: &Command) {
        let (reference, pattern) = (c.arg(0).unwrap_or_default(), c.arg(1).unwrap_or_default());
        let mut b = Vec::new();
        if pattern.is_empty() {
            b.extend_from_slice(&imap::untagged(&format!("{} (\\Noselect) \"/\" \"\"", c.verb)));
        } else {
            let user = self.user_of(sock);
            let names = os.files().map(|st| mailbox::folders(st, &self.mailbox, &user)).unwrap_or_default();
            let full = format!("{}{}", reference, pattern);
            for n in names.iter().filter(|n| list_match(&full, n)) {
                let attr = if n == SENT {
                    "\\HasNoChildren \\Sent"
                } else if n == TRASH {
                    "\\HasNoChildren \\Trash"
                } else {
                    "\\HasNoChildren"
                };
                b.extend_from_slice(&imap::untagged(&format!("{} ({}) \"/\" {}", c.verb, attr, imap::astring(n))));
            }
        }
        b.extend_from_slice(&imap::ok(tag, None, "That is all"));
        self.send(os, sock, &b);
    }

    fn status_cmd(&mut self, os: &mut OsApi, sock: u32, tag: &str, c: &Command) {
        let Some(name) = c.arg(0) else {
            return self.bad(os, sock, tag, "STATUS", "Which folder?");
        };
        let Some((folder, dir)) = self.resolve_folder(os, sock, &name) else {
            return self.no(os, sock, tag, Some("NONEXISTENT"), "STATUS", "There is no such folder");
        };
        let (val, next, msgs) = self.read_folder(os, &dir);
        let mut parts = Vec::new();
        if let Some(Item::List(items)) = c.args.get(1) {
            for w in items.iter().filter_map(Item::text) {
                let v = match w.to_ascii_uppercase().as_str() {
                    "MESSAGES" => msgs.len() as u64,
                    "RECENT" => msgs.iter().filter(|m| m.recent).count() as u64,
                    "UNSEEN" => msgs.iter().filter(|m| !m.letters.contains('S')).count() as u64,
                    "UIDNEXT" => next as u64,
                    "UIDVALIDITY" => val as u64,
                    "SIZE" => msgs.iter().map(|m| m.size).sum(),
                    "DELETED" => msgs.iter().filter(|m| m.letters.contains('T')).count() as u64,
                    _ => return self.bad(os, sock, tag, "STATUS", "This server does not count that"),
                };
                parts.push(format!("{} {}", w.to_ascii_uppercase(), v));
            }
        }
        let mut b = imap::untagged(&format!("STATUS {} ({})", imap::astring(&folder), parts.join(" ")));
        b.extend_from_slice(&imap::ok(tag, None, "Counted"));
        self.send(os, sock, &b);
    }

    /// a name for a new message in a folder
    fn new_name(&mut self, os: &mut OsApi) -> String {
        self.seq += 1;
        let clock = os.clock();
        let host = os.hostname();
        mailbox::unique(clock.div_euclid(1_000_000_000), (clock.rem_euclid(1_000_000_000) / 1000) as u32, self.seq, &host)
    }

    fn append(&mut self, os: &mut OsApi, sock: u32, tag: &str, c: &Command) {
        let Some(name) = c.arg(0) else {
            return self.bad(os, sock, tag, "APPEND", "Which folder?");
        };
        let Some(Item::Literal(data)) = c.args.last() else {
            return self.bad(os, sock, tag, "APPEND", "The message comes as a literal");
        };
        let Some((folder, dir)) = self.resolve_folder(os, sock, &name) else {
            return self.no(os, sock, tag, Some("TRYCREATE"), "APPEND", "There is no such folder");
        };
        let letters = letters_of(&c.args[1..c.args.len() - 1]);
        let unique = self.new_name(os);
        let path = if letters.is_empty() { format!("{}/new/{}", dir, unique) } else { format!("{}/cur/{}", dir, mailbox::set_flags(&unique, &letters)) };
        let bytes = data.len() as u64;
        if os.store_put(&path, &imf::lf(data)).is_err() {
            return self.no(os, sock, tag, Some("LIMIT"), "APPEND", "The disk is full");
        }
        os.store_report(&path);
        self.appended += 1;
        os.note(Note::Mail(MNote::ImapAppend { folder, bytes }));
        self.sync(os, sock);
        self.ok(os, sock, tag, "Appended");
    }

    /// the messages of a set in the selected folder: (sequence number from 1, message)
    fn in_set(&self, sock: u32, set: &SeqSet, uid: bool) -> Vec<(u32, FolderMsg)> {
        let Some(sel) = self.sessions.get(&sock).and_then(|s| s.sel.as_ref()) else { return Vec::new() };
        let (max_seq, max_uid) = (sel.msgs.len() as u32, sel.msgs.last().map_or(0, |m| m.uid));
        sel.msgs.iter().enumerate().map(|(i, m)| (i as u32 + 1, m.clone())).filter(|(k, m)| if uid { set.contains(m.uid, max_uid) } else { set.contains(*k, max_seq) }).collect()
    }

    fn search(&mut self, os: &mut OsApi, sock: u32, tag: &str, args: &[Item], uid: bool) {
        // CHARSET is taken and left out
        let args = if args.first().and_then(Item::text).is_some_and(|w| w.eq_ignore_ascii_case("CHARSET")) { &args[args.len().min(2)..] } else { args };
        let Some(keys) = imap::parse_search(args) else {
            return self.bad(os, sock, tag, "SEARCH", "This server does not know that key");
        };
        let Some(sel) = self.sessions.get(&sock).and_then(|s| s.sel.clone()) else { return };
        let (max_seq, max_uid) = (sel.msgs.len() as u32, sel.msgs.last().map_or(0, |m| m.uid));
        let mut hits = Vec::new();
        for (i, m) in sel.msgs.iter().enumerate() {
            let data = os.file_read(&m.path).unwrap_or_default();
            let parsed = Message::parse(&data);
            let text = parsed.text();
            let f = Facts { seq: i as u32 + 1, max_seq, max_uid, m, msg: Some(parsed), text };
            if keys.iter().all(|k| matches_key(k, &f)) {
                hits.push(format!("{}", if uid { m.uid } else { i as u32 + 1 }));
            }
        }
        let mut b = if hits.is_empty() { imap::untagged("SEARCH") } else { imap::untagged(&format!("SEARCH {}", hits.join(" "))) };
        b.extend_from_slice(&imap::ok(tag, None, "Searched"));
        self.send(os, sock, &b);
    }

    fn fetch(&mut self, os: &mut OsApi, sock: u32, tag: &str, args: &[Item], uid: bool) {
        let (Some(set), Some(arg)) = (args.first().and_then(Item::text), args.get(1)) else {
            return self.bad(os, sock, tag, "FETCH", "FETCH takes a set and what to fetch");
        };
        let (Some(parsed), Some(items)) = (SeqSet::parse(&set), imap::parse_fetch(arg)) else {
            return self.bad(os, sock, tag, "FETCH", "This server does not know what that asks for");
        };
        // what holds a body comes after the disk read
        let wants_body = items.iter().any(|i| matches!(i, FetchItem::Body { .. } | FetchItem::Rfc822 | FetchItem::Rfc822Header | FetchItem::Rfc822Text | FetchItem::Envelope));
        if !wants_body {
            return self.fetch_now(os, sock, tag, uid, &parsed, &items);
        }
        let bytes: u64 = self.in_set(sock, &parsed, uid).iter().map(|x| x.1.size).sum();
        let at = os.disk_read(bytes);
        if let Some(s) = self.sessions.get_mut(&sock) {
            s.busy = true;
        }
        self.wheel.add(at, Due::Fetch { sock, tag: String::from(tag), uid, set, items });
        self.wheel.arm(os);
    }

    fn fetch_now(&mut self, os: &mut OsApi, sock: u32, tag: &str, uid: bool, set: &SeqSet, items: &[FetchItem]) {
        let rows = self.in_set(sock, set, uid);
        let readonly = self.readonly(sock);
        let mut total = 0u64;
        let mut count = 0u32;
        let mut changed = false;
        let mut out: Vec<u8> = Vec::new();
        for (seq, m) in rows {
            let data = os.file_read(&m.path).map(|d| imf::crlf(&d)).unwrap_or_default();
            let mut parts: Vec<Vec<u8>> = Vec::new();
            let mut uid_shown = false;
            let mut seen_now = false;
            for it in items {
                match it {
                    FetchItem::Uid => {
                        uid_shown = true;
                        parts.push(format!("UID {}", m.uid).into_bytes());
                    }
                    FetchItem::Flags => parts.push(format!("FLAGS {}", flag_list(&m)).into_bytes()),
                    FetchItem::Rfc822Size => parts.push(format!("RFC822.SIZE {}", m.size).into_bytes()),
                    FetchItem::InternalDate => {
                        let t = os.files().and_then(|st| st.stat(&m.path)).map_or(0, |s| s.mtime);
                        parts.push(format!("INTERNALDATE {}", imap::quoted(&imap::internal_date(t))).into_bytes());
                    }
                    FetchItem::Envelope => parts.push(format!("ENVELOPE {}", imap::envelope(&Message::parse(&data))).into_bytes()),
                    FetchItem::Rfc822 | FetchItem::Rfc822Header | FetchItem::Rfc822Text | FetchItem::Body { .. } => {
                        let (name, body): (String, Vec<u8>) = match it {
                            FetchItem::Rfc822 => (String::from("RFC822"), data.clone()),
                            FetchItem::Rfc822Header => (String::from("RFC822.HEADER"), imap::header_of(&data).to_vec()),
                            FetchItem::Rfc822Text => (String::from("RFC822.TEXT"), imap::text_of(&data).to_vec()),
                            FetchItem::Body { section, name, .. } => (
                                name.clone(),
                                match section {
                                    Section::Full => data.clone(),
                                    Section::Header => imap::header_of(&data).to_vec(),
                                    Section::Text => imap::text_of(&data).to_vec(),
                                    Section::Fields(names, not) => imap::header_fields(&data, names, *not),
                                },
                            ),
                            _ => (String::new(), Vec::new()),
                        };
                        let peek = matches!(it, FetchItem::Body { peek: true, .. } | FetchItem::Rfc822Header);
                        if !peek {
                            seen_now = true;
                        }
                        total += body.len() as u64;
                        let mut p = format!("{} {{{}}}\r\n", name, body.len()).into_bytes();
                        p.extend_from_slice(&body);
                        parts.push(p);
                    }
                }
            }
            if uid && !uid_shown {
                parts.insert(0, format!("UID {}", m.uid).into_bytes());
            }
            // reading without PEEK sets \Seen, and says so
            if seen_now && !readonly && !m.letters.contains('S') {
                let letters = format!("{}S", m.letters);
                if let Some(x) = self.set_letters(os, sock, &m, &letters) {
                    parts.push(format!("FLAGS {}", flag_list(&x)).into_bytes());
                    changed = true;
                }
            }
            out.extend_from_slice(format!("* {} FETCH (", seq).as_bytes());
            for (i, p) in parts.iter().enumerate() {
                if i > 0 {
                    out.push(b' ');
                }
                out.extend_from_slice(p);
            }
            out.extend_from_slice(b")\r\n");
            count += 1;
        }
        out.extend_from_slice(&imap::ok(tag, None, "Fetched"));
        self.bytes_sent += total;
        self.send(os, sock, &out);
        if total > 0 {
            self.fetched += count as u64;
            let folder = self.sessions.get(&sock).and_then(|s| s.sel.as_ref()).map(|x| x.folder.clone()).unwrap_or_default();
            os.note(Note::Mail(MNote::ImapFetch { folder, n: count, bytes: total }));
        }
        if changed {
            os.store_report(&format!("{}/cur", self.sessions.get(&sock).and_then(|s| s.sel.as_ref()).map(|x| x.dir.clone()).unwrap_or_default()));
        }
    }

    /// gives a message these letters (it moves to cur/ with them); the message as it is now, kept
    /// in the session's list
    fn set_letters(&mut self, os: &mut OsApi, sock: u32, m: &FolderMsg, letters: &str) -> Option<FolderMsg> {
        let dir = self.sessions.get(&sock).and_then(|s| s.sel.as_ref()).map(|x| x.dir.clone())?;
        let to = format!("{}/cur/{}", dir, mailbox::set_flags(&m.name, letters));
        if to != m.path {
            let data = os.file_read(&m.path).ok()?;
            os.store_put(&to, &data).ok()?;
            os.store_remove(&m.path).ok()?;
        }
        let mut x = m.clone();
        x.path = to;
        x.letters = mailbox::flags_of(&mailbox::set_flags(&m.name, letters));
        x.recent = false;
        if let Some(sel) = self.sessions.get_mut(&sock).and_then(|s| s.sel.as_mut())
            && let Some(slot) = sel.msgs.iter_mut().find(|y| y.uid == m.uid)
        {
            *slot = x.clone();
        }
        Some(x)
    }

    fn store(&mut self, os: &mut OsApi, sock: u32, tag: &str, args: &[Item], uid: bool) {
        let (Some(set), Some(what)) = (args.first().and_then(Item::text), args.get(1).and_then(Item::text)) else {
            return self.bad(os, sock, tag, "STORE", "STORE takes a set, how and the flags");
        };
        let Some(parsed) = SeqSet::parse(&set) else {
            return self.bad(os, sock, tag, "STORE", "That is no set of messages");
        };
        if self.readonly(sock) {
            return self.no(os, sock, tag, None, "STORE", "This folder was opened to read only");
        }
        let w = what.to_ascii_uppercase();
        let silent = w.ends_with(".SILENT");
        let base = w.trim_end_matches(".SILENT");
        let (add, remove) = match base {
            "FLAGS" => (false, false),
            "+FLAGS" => (true, false),
            "-FLAGS" => (false, true),
            _ => return self.bad(os, sock, tag, "STORE", "FLAGS, +FLAGS or -FLAGS, please"),
        };
        let letters = letters_of(&args[2..]);
        let rows = self.in_set(sock, &parsed, uid);
        let mut out = Vec::new();
        let mut n = 0;
        for (seq, m) in rows {
            let new: String = if add {
                format!("{}{}", m.letters, letters)
            } else if remove {
                m.letters.chars().filter(|c| !letters.contains(*c)).collect()
            } else {
                letters.clone()
            };
            if let Some(x) = self.set_letters(os, sock, &m, &new) {
                n += 1;
                if !silent {
                    let u = if uid { format!("UID {} ", x.uid) } else { String::new() };
                    out.extend_from_slice(&imap::untagged(&format!("{} FETCH ({}FLAGS {})", seq, u, flag_list(&x))));
                }
            }
        }
        out.extend_from_slice(&imap::ok(tag, None, "Stored"));
        self.send(os, sock, &out);
        if n > 0 {
            self.stored += n as u64;
            let (folder, dir) = self.sessions.get(&sock).and_then(|s| s.sel.as_ref()).map(|x| (x.folder.clone(), x.dir.clone())).unwrap_or_default();
            os.store_report(&format!("{}/cur", dir));
            os.note(Note::Mail(MNote::ImapStore { folder, n }));
        }
    }

    /// EXPUNGE (`tell`: with its untagged lines) and the silent one of CLOSE
    fn expunge(&mut self, os: &mut OsApi, sock: u32, tell: bool) {
        let Some(sel) = self.sessions.get(&sock).and_then(|s| s.sel.clone()) else { return };
        if sel.readonly {
            return;
        }
        let mut b = Vec::new();
        let mut left = Vec::new();
        let mut n = 0u32;
        let mut seq = 1;
        for m in &sel.msgs {
            if m.letters.contains('T') && os.store_remove(&m.path).is_ok() {
                n += 1;
                b.extend_from_slice(&imap::untagged(&format!("{} EXPUNGE", seq)));
            } else {
                left.push(m.clone());
                seq += 1;
            }
        }
        if let Some(s) = self.sessions.get_mut(&sock).and_then(|s| s.sel.as_mut()) {
            s.msgs = left;
        }
        if n > 0 {
            self.expunged += n as u64;
            os.store_report(&sel.dir);
            os.note(Note::Mail(MNote::ImapExpunge { folder: sel.folder.clone(), n }));
            os.log(SEV_INFO, "imapd", &format!("{} removed {} messages from {}", self.user_of(sock), n, sel.folder));
        }
        if tell && !b.is_empty() {
            self.send(os, sock, &b);
        }
    }

    fn copy(&mut self, os: &mut OsApi, sock: u32, tag: &str, args: &[Item], uid: bool, moved: bool) {
        let verb = if moved { "MOVE" } else { "COPY" };
        let (Some(set), Some(name)) = (args.first().and_then(Item::text), args.get(1).and_then(Item::text)) else {
            return self.bad(os, sock, tag, verb, "It takes a set and a folder");
        };
        let Some(parsed) = SeqSet::parse(&set) else {
            return self.bad(os, sock, tag, verb, "That is no set of messages");
        };
        let Some((to_folder, to_dir)) = self.resolve_folder(os, sock, &name) else {
            return self.no(os, sock, tag, Some("TRYCREATE"), verb, "There is no such folder");
        };
        if moved && self.readonly(sock) {
            return self.no(os, sock, tag, None, verb, "This folder was opened to read only");
        }
        let rows = self.in_set(sock, &parsed, uid);
        let mut out = Vec::new();
        let mut n = 0;
        let mut gone = 0u32;
        for (seq, m) in rows {
            let Ok(data) = os.file_read(&m.path) else { continue };
            let unique = self.new_name(os);
            let target = format!("{}/cur/{}", to_dir, mailbox::set_flags(&unique, &m.letters));
            if os.store_put(&target, &data).is_err() {
                continue;
            }
            n += 1;
            if moved && os.store_remove(&m.path).is_ok() {
                out.extend_from_slice(&imap::untagged(&format!("{} EXPUNGE", seq - gone)));
                gone += 1;
                if let Some(sel) = self.sessions.get_mut(&sock).and_then(|s| s.sel.as_mut()) {
                    sel.msgs.retain(|x| x.uid != m.uid);
                }
            }
        }
        out.extend_from_slice(&imap::ok(tag, None, if moved { "Moved" } else { "Copied" }));
        self.send(os, sock, &out);
        if n > 0 {
            os.store_report(&to_dir);
            os.note(Note::Mail(MNote::ImapCopy { to: to_folder, n, moved }));
        }
    }

    fn deadlines(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (id, d) in self.wheel.due(now) {
            match d {
                Due::Refuse(s, tag) => self.refuse_now(os, s, &tag),
                Due::Fetch { sock, tag, uid, set, items } => {
                    if let Some(s) = self.sessions.get_mut(&sock) {
                        s.busy = false;
                    }
                    if let Some(parsed) = SeqSet::parse(&set) {
                        self.fetch_now(os, sock, &tag, uid, &parsed, &items);
                    }
                    self.drain(os, sock);
                }
                Due::Look(s) => {
                    if self.sessions.get(&s).is_some_and(|x| x.idling.is_some()) {
                        self.sync(os, s);
                        self.wheel.add(now + IDLE_LOOK, Due::Look(s));
                    }
                }
                Due::Idle(s) => {
                    if self.sessions.get(&s).is_some_and(|x| x.idle == Some(id)) {
                        self.send(os, s, &imap::untagged("BYE Nothing happened for too long, goodbye"));
                        self.end(os, s);
                    }
                }
            }
        }
        self.wheel.arm(os);
    }
}

impl Service for ImapServer {
    fn kind(&self) -> &'static str {
        "imap-server"
    }
    fn start(&mut self, os: &mut OsApi) {
        match os.tcp_listen(self.port, 0, SockOpts::default()) {
            Ok(l) => self.listener = Some(l),
            Err(_) => os.note(Note::PortInUse { port: self.port, svc: String::from("imap-server") }),
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::TcpAccept { listener, .. } if Some(listener) == self.listener => {
                while let Some(c) = os.tcp_accept(listener) {
                    let peer = os.tcp_info(c).map(|r| r.remote.0).unwrap_or(crate::os::ANY4);
                    self.sessions_n += 1;
                    self.sessions.insert(c, Session { peer, reader: LineReader::new(), user: None, fails: 0, sel: None, idling: None, auth: None, busy: false, queue: VecDeque::new(), out: Vec::new(), idle: None, closing: false });
                    os.note(Note::Mail(MNote::ImapSession { from: peer }));
                    let b = self.banner.clone();
                    self.send(os, c, &imap::untagged(&format!("OK [CAPABILITY {}] {}", CAPS, b)));
                    self.touch(os, c);
                }
            }
            SvcEvent::TcpData { sock, .. } => {
                let b = os.tcp_recv(sock, usize::MAX);
                if self.sessions.contains_key(&sock) {
                    self.bytes(os, sock, &b);
                }
            }
            SvcEvent::TcpWritable { sock } => {
                if let Some(s) = self.sessions.get_mut(&sock) {
                    let n = os.tcp_send(sock, &s.out);
                    s.out.drain(..n);
                }
            }
            SvcEvent::TcpEof { sock, .. } | SvcEvent::TcpClosed { sock, .. } => {
                if self.sessions.contains_key(&sock) {
                    self.end(os, sock);
                } else {
                    os.tcp_close(sock);
                }
            }
            SvcEvent::Timer(K_WHEEL) => self.deadlines(os),
            _ => {}
        }
        self.wheel.arm(os);
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.listener = None;
        self.sessions.clear();
        self.wheel.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("sessions", self.sessions_n as i64),
            Metric::new("active", self.sessions.len() as i64),
            Metric::new("logins", self.logins as i64),
            Metric::new("failed_logins", self.failed_logins as i64),
            Metric::new("fetched", self.fetched as i64),
            Metric::new("stored", self.stored as i64),
            Metric::new("expunged", self.expunged as i64),
            Metric::new("appended", self.appended as i64),
            Metric::new("bytes_sent", self.bytes_sent as i64),
        ]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        msg!("Port {}, {} Benutzer: {} Sitzungen, {} Anmeldungen, {} abgelehnt, {} Nachrichten geholt, {} entfernt", self.port, self.users.len(), self.sessions_n, self.logins, self.failed_logins, self.fetched, self.expunged)
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn footprint(&self) -> usize {
        1024 + self.users.len() * 64 + self.sessions.values().map(|s| 512 + s.out.len() + s.reader.pending() + s.sel.as_ref().map_or(0, |x| x.msgs.len() * 160)).sum::<usize>()
    }
}
