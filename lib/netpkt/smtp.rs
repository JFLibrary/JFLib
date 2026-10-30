// smtp -- the Simple Mail Transfer Protocol (RFC 5321) with the service extensions of EHLO that
// JFNetworker's servers speak: SIZE (RFC 1870), 8BITMIME (RFC 6152), PIPELINING (RFC 2920),
// ENHANCEDSTATUSCODES (RFC 2034 with the codes of RFC 3463) and AUTH with PLAIN and LOGIN
// (RFC 4954, 4616); no STARTTLS
//
//   Command { verb, arg }     Command::parse(line) (the verb in capitals, the argument as
//                             typed), Command::new(verb, arg), to_line() ("EHLO pc1\r\n")
//   Path { addr, params }     parse_path(arg, "FROM" | "TO"): the address of MAIL FROM:<..> or
//                             RCPT TO:<..> (empty for the null reverse path "<>") and the
//                             parameters after it (SIZE=1234, BODY=8BITMIME); fmt_path
//   Reply { code, lines }     Reply::new(code, text), Reply::enh(code, "2.1.0", text) (the
//                             enhanced code before the text), to_bytes(): "250 text" or the
//                             multi-line form "250-first" ... "250 last" of 4.2.1; text(),
//                             status() (the enhanced code), is_positive / is_intermediate /
//                             is_transient / is_permanent
//   ReplyReader               the replies of a server from its bytes in any pieces
//   Ehlo { domain, keywords } of_reply(&Reply): what a 250 to EHLO offers; has("PIPELINING"),
//                             param("SIZE") -> "10240000", size_limit()
//   Stream                    the server's side of a connection: push(bytes), line() while
//                             commands come, begin_data(max) after 354, data() until the line
//                             with the dot (dot-stuffing undone; beyond max the bytes are
//                             counted but not kept)
//   stuff(message)            the bytes after DATA: CRLF line ends, a dot doubled at the start
//                             of a line (4.5.2), ".\r\n" at the end
//   auth_plain(user, pass) / plain_of(b64)   the initial response of AUTH PLAIN and back
//   LOGIN_USER / LOGIN_PASS   the base64 challenges of AUTH LOGIN ("Username:", "Password:")
//   received(...)             the value of a Received field (4.4)
//   PORT_SMTP (25), PORT_SUBMISSION (587, RFC 6409), verb_known
//
// Lines end in CRLF; a server here also takes a lone LF.  Reply texts are the server's own
// words: clients go by the codes (and the enhanced codes when ENHANCEDSTATUSCODES was offered).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::imf;

pub const PORT_SMTP: u16 = 25;
/// message submission (RFC 6409): clients hand in their mail here, after AUTH
pub const PORT_SUBMISSION: u16 = 587;
/// the base64 challenges of AUTH LOGIN: "Username:" and "Password:"
pub const LOGIN_USER: &str = "VXNlcm5hbWU6";
pub const LOGIN_PASS: &str = "UGFzc3dvcmQ6";

/// the commands of RFC 5321 and the extensions a dissector names
pub fn verb_known(v: &str) -> bool {
    const VERBS: [&str; 16] = ["HELO", "EHLO", "MAIL", "RCPT", "DATA", "RSET", "VRFY", "EXPN", "HELP", "NOOP", "QUIT", "AUTH", "STARTTLS", "BDAT", "ETRN", "TURN"];
    VERBS.iter().any(|x| x.eq_ignore_ascii_case(v))
}

/// a command line of a client
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
    /// in capitals
    pub verb: String,
    /// everything after the first space
    pub arg: String,
}

impl Command {
    pub fn new(verb: &str, arg: &str) -> Command {
        Command { verb: verb.to_ascii_uppercase(), arg: String::from(arg) }
    }
    /// a line without its line end; None for an empty one
    pub fn parse(line: &str) -> Option<Command> {
        let l = line.trim_end_matches(['\r', '\n']).trim_start();
        if l.is_empty() {
            return None;
        }
        let (v, a) = l.split_once(' ').unwrap_or((l, ""));
        Some(Command { verb: v.to_ascii_uppercase(), arg: String::from(a.trim_end()) })
    }
    /// the line on the wire, CRLF included
    pub fn to_line(&self) -> String {
        if self.arg.is_empty() { format!("{}\r\n", self.verb) } else { format!("{} {}\r\n", self.verb, self.arg) }
    }
}

/// the address and parameters of MAIL FROM or RCPT TO
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Path {
    /// without the angle brackets; empty: the null reverse path "<>"
    pub addr: String,
    /// "SIZE=1234" -> ("SIZE", "1234"), "BODY=8BITMIME"; keywords in capitals
    pub params: Vec<(String, String)>,
}

impl Path {
    /// a parameter's value
    pub fn param(&self, k: &str) -> Option<&str> {
        self.params.iter().find(|p| p.0.eq_ignore_ascii_case(k)).map(|p| p.1.as_str())
    }
    /// SIZE=n
    pub fn size(&self) -> Option<u64> {
        self.param("SIZE").and_then(|v| v.parse().ok())
    }
}

/// "FROM:<anna@a.lab> SIZE=345" (the argument of MAIL; `key` "FROM", for RCPT "TO"): None when
/// the keyword, the colon or the brackets are missing (a source route "<@a,@b:c@d>" keeps its
/// mailbox, 4.1.2)
pub fn parse_path(arg: &str, key: &str) -> Option<Path> {
    let a = arg.trim_start();
    if a.len() < key.len() + 1 || !a[..key.len()].eq_ignore_ascii_case(key) {
        return None;
    }
    let rest = a[key.len()..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('<')?;
    let close = rest.find('>')?;
    let mut addr = &rest[..close];
    if addr.starts_with('@')
        && let Some((_, m)) = addr.split_once(':')
    {
        addr = m;
    }
    let mut params = Vec::new();
    for p in rest[close + 1..].split_whitespace() {
        let (k, v) = p.split_once('=').unwrap_or((p, ""));
        params.push((k.to_ascii_uppercase(), String::from(v)));
    }
    Some(Path { addr: String::from(addr.trim()), params })
}

/// "FROM:<addr> SIZE=n" with the parameters given
pub fn fmt_path(key: &str, addr: &str, params: &[(&str, &str)]) -> String {
    let mut s = format!("{}:<{}>", key, addr);
    for (k, v) in params {
        if v.is_empty() {
            s.push_str(&format!(" {}", k));
        } else {
            s.push_str(&format!(" {}={}", k, v));
        }
    }
    s
}

/// a reply of a server
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    pub code: u16,
    /// the text of each line (without the code)
    pub lines: Vec<String>,
}

impl Reply {
    pub fn new(code: u16, text: &str) -> Reply {
        Reply { code, lines: text.split('\n').map(String::from).collect() }
    }
    /// a reply whose lines start with an enhanced status code ("2.1.0 Sender fine")
    pub fn enh(code: u16, status: &str, text: &str) -> Reply {
        Reply { code, lines: text.split('\n').map(|l| format!("{} {}", status, l)).collect() }
    }
    /// the text of the first line (with its enhanced code)
    pub fn text(&self) -> &str {
        self.lines.first().map(String::as_str).unwrap_or("")
    }
    /// all lines as one text ("a b c")
    pub fn joined(&self) -> String {
        self.lines.join(" ")
    }
    /// the enhanced status code at the start of the first line ("5.1.1")
    pub fn status(&self) -> Option<&str> {
        let w = self.text().split(' ').next()?;
        let p: Vec<&str> = w.split('.').collect();
        let ok = p.len() == 3 && matches!(p[0], "2" | "4" | "5") && p[1..].iter().all(|x| !x.is_empty() && x.len() <= 3 && x.bytes().all(|c| c.is_ascii_digit()));
        ok.then_some(w)
    }
    /// 2yz: done
    pub fn is_positive(&self) -> bool {
        (200..300).contains(&self.code)
    }
    /// 3yz: go on (354 after DATA, 334 in AUTH)
    pub fn is_intermediate(&self) -> bool {
        (300..400).contains(&self.code)
    }
    /// 4yz: try again later
    pub fn is_transient(&self) -> bool {
        (400..500).contains(&self.code)
    }
    /// 5yz: will not work
    pub fn is_permanent(&self) -> bool {
        (500..600).contains(&self.code)
    }
    /// "code text" lines as they travel (4.2.1)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut s = String::new();
        let n = self.lines.len().max(1);
        for i in 0..n {
            let l = self.lines.get(i).map(String::as_str).unwrap_or("");
            let sep = if i + 1 < n { '-' } else { ' ' };
            s.push_str(&format!("{}{}{}\r\n", self.code, sep, l));
        }
        s.into_bytes()
    }
    /// "550 5.1.1 text" as one line (logs, bounces)
    pub fn line(&self) -> String {
        format!("{} {}", self.code, self.joined())
    }
}

/// a line's code: three digits and a space, a dash or nothing
fn code_of(l: &str) -> Option<(u16, bool, &str)> {
    let b = l.as_bytes();
    if b.len() < 3 || !b[..3].iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let code = (b[0] - b'0') as u16 * 100 + (b[1] - b'0') as u16 * 10 + (b[2] - b'0') as u16;
    match b.get(3) {
        None => Some((code, false, "")),
        Some(b' ') => Some((code, false, &l[4..])),
        Some(b'-') => Some((code, true, &l[4..])),
        _ => None,
    }
}

/// splits bytes into lines (CRLF or LF) as they arrive
#[derive(Clone, Debug, Default)]
struct LineBuf {
    buf: Vec<u8>,
}

impl LineBuf {
    fn push(&mut self, b: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(b);
        let mut out = Vec::new();
        let mut start = 0;
        while let Some(i) = self.buf[start..].iter().position(|&c| c == b'\n') {
            let mut e = start + i;
            if e > start && self.buf[e - 1] == b'\r' {
                e -= 1;
            }
            out.push(String::from_utf8_lossy(&self.buf[start..e]).into_owned());
            start += i + 1;
        }
        self.buf.drain(..start);
        out
    }
}

/// the replies of a server, as its bytes arrive
#[derive(Clone, Debug, Default)]
pub struct ReplyReader {
    lines: LineBuf,
    open: Option<Reply>,
}

impl ReplyReader {
    pub fn new() -> ReplyReader {
        ReplyReader::default()
    }
    /// the replies that are complete now; a line that is no reply line is skipped
    pub fn push(&mut self, b: &[u8]) -> Vec<Reply> {
        let mut out = Vec::new();
        for l in self.lines.push(b) {
            let Some((code, more, text)) = code_of(&l) else { continue };
            let r = self.open.get_or_insert_with(|| Reply { code, lines: Vec::new() });
            r.lines.push(String::from(text));
            if !more {
                if let Some(r) = self.open.take() {
                    out.push(r);
                }
            }
        }
        out
    }
}

/// what a server offers in its answer to EHLO (4.1.1.1)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ehlo {
    /// the name it gives for itself
    pub domain: String,
    /// "SIZE 10240000", "8BITMIME", "AUTH PLAIN LOGIN" ...
    pub keywords: Vec<String>,
}

impl Ehlo {
    /// the 250 reply to EHLO: its first line names the server, every further line is a
    /// keyword with its parameters
    pub fn of_reply(r: &Reply) -> Ehlo {
        let domain = r.lines.first().and_then(|l| l.split_whitespace().next()).map(String::from).unwrap_or_default();
        Ehlo { domain, keywords: r.lines.iter().skip(1).map(|l| String::from(l.trim())).collect() }
    }
    /// the server offers the keyword
    pub fn has(&self, kw: &str) -> bool {
        self.keywords.iter().any(|k| k.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case(kw)))
    }
    /// the parameters after a keyword ("SIZE" -> "10240000", "AUTH" -> "PLAIN LOGIN")
    pub fn param(&self, kw: &str) -> Option<&str> {
        self.keywords.iter().find(|k| k.split_whitespace().next().is_some_and(|w| w.eq_ignore_ascii_case(kw))).map(|k| k.split_once(' ').map_or("", |x| x.1.trim()))
    }
    /// the size limit SIZE names (0: none given)
    pub fn size_limit(&self) -> Option<u64> {
        if !self.has("SIZE") {
            return None;
        }
        Some(self.param("SIZE").and_then(|v| v.parse().ok()).unwrap_or(0))
    }
    /// AUTH offers this mechanism
    pub fn auth(&self, mech: &str) -> bool {
        self.param("AUTH").is_some_and(|p| p.split_whitespace().any(|m| m.eq_ignore_ascii_case(mech)))
    }
    /// the reply a server sends: its name and greeting, then the keywords
    pub fn to_reply(&self, greeting: &str) -> Reply {
        let mut lines = alloc::vec![format!("{} {}", self.domain, greeting)];
        lines.extend(self.keywords.iter().cloned());
        Reply { code: 250, lines }
    }
}

/// a message after DATA
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Data {
    /// the message with CRLF line ends, dot-stuffing undone (empty when it was too big)
    pub body: Vec<u8>,
    /// its size in bytes, also when it was too big
    pub size: u64,
    /// more than the limit came
    pub over: bool,
}

/// the server's side of a connection: command lines, then the message after 354
#[derive(Clone, Debug, Default)]
pub struct Stream {
    buf: Vec<u8>,
    data: Option<Data>,
    max: u64,
}

impl Stream {
    pub fn new() -> Stream {
        Stream::default()
    }
    pub fn push(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    /// bytes not taken yet
    pub fn pending(&self) -> usize {
        self.buf.len()
    }
    /// a message is being read
    pub fn in_data(&self) -> bool {
        self.data.is_some()
    }
    /// the next command line (its line end removed); None until a line is complete or while a
    /// message is read
    pub fn line(&mut self) -> Option<String> {
        if self.data.is_some() {
            return None;
        }
        let i = self.buf.iter().position(|&c| c == b'\n')?;
        let mut l: Vec<u8> = self.buf.drain(..=i).collect();
        l.pop();
        if l.last() == Some(&b'\r') {
            l.pop();
        }
        Some(String::from_utf8_lossy(&l).into_owned())
    }
    /// the message starts after the 354; more than `max` bytes are counted, not kept (0: no
    /// limit)
    pub fn begin_data(&mut self, max: u64) {
        self.data = Some(Data::default());
        self.max = max;
    }
    /// the message once its line with the dot came (the bytes after it stay for the next
    /// commands); None while it goes on
    pub fn data(&mut self) -> Option<Data> {
        let d = self.data.as_mut()?;
        let mut start = 0;
        let mut done = false;
        while let Some(i) = self.buf[start..].iter().position(|&c| c == b'\n') {
            let end = start + i;
            let mut e = end;
            if e > start && self.buf[e - 1] == b'\r' {
                e -= 1;
            }
            let line = &self.buf[start..e];
            start = end + 1;
            if line == b"." {
                done = true;
                break;
            }
            let line = if line.first() == Some(&b'.') { &line[1..] } else { line };
            d.size += line.len() as u64 + 2;
            if self.max > 0 && d.size > self.max {
                d.over = true;
                d.body.clear();
            }
            if !d.over {
                d.body.extend_from_slice(line);
                d.body.extend_from_slice(b"\r\n");
            }
        }
        self.buf.drain(..start);
        if done { self.data.take() } else { None }
    }
}

/// the bytes after DATA: CRLF line ends, a dot doubled at the start of a line, the line with
/// the dot at the end (4.5.2)
pub fn stuff(message: &[u8]) -> Vec<u8> {
    let m = imf::crlf(message);
    let mut out = Vec::with_capacity(m.len() + 8);
    let mut bol = true;
    for &c in &m {
        if bol && c == b'.' {
            out.push(b'.');
        }
        out.push(c);
        bol = c == b'\n';
    }
    if !out.is_empty() && !out.ends_with(b"\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    out
}

/// the initial response of AUTH PLAIN (RFC 4616): base64 of "\0user\0password"
pub fn auth_plain(user: &str, pass: &str) -> String {
    let mut v = alloc::vec![0u8];
    v.extend_from_slice(user.as_bytes());
    v.push(0);
    v.extend_from_slice(pass.as_bytes());
    imf::b64_encode(&v)
}

/// the user and password of an AUTH PLAIN response (the authorization identity ignored)
pub fn plain_of(b64: &str) -> Option<(String, String)> {
    let raw = imf::b64_decode(b64.trim().as_bytes())?;
    let mut it = raw.split(|&c| c == 0);
    let _authz = it.next()?;
    let user = it.next()?;
    let pass = it.next()?;
    Some((String::from_utf8_lossy(user).into_owned(), String::from_utf8_lossy(pass).into_owned()))
}

/// the value of a Received field (4.4): "from HELO (NAME [ADDR]) by HOST with PROTO id ID for
/// <RCPT>; DATE"
pub fn received(helo: &str, peer: &str, by: &str, with: &str, id: &str, rcpt: Option<&str>, date: &str) -> String {
    let mut s = format!("from {} ({}) by {} with {} id {}", helo, peer, by, with, id);
    if let Some(r) = rcpt {
        s.push_str(&format!(" for <{}>", r));
    }
    s.push_str("; ");
    s.push_str(date);
    s
}
