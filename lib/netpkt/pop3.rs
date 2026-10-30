// pop3 -- the Post Office Protocol version 3 (RFC 1939) with its optional commands UIDL, TOP
// and USER / PASS, and CAPA of RFC 2449: command lines, the status line +OK / -ERR, multi-line
// responses with their dot-stuffing
//
//   Command { verb, args }    Command::parse(line) (the verb in capitals), Command::new(verb,
//                             args), to_line() ("RETR 1\r\n")
//   Response { ok, text, body }   Response::ok(text) / err(text), with_body(bytes) (a multi-
//                             line response: the lines of the bytes follow the status line,
//                             each with a leading dot doubled, then the line with the dot);
//                             to_bytes()
//   multi_line(&Command)      a +OK to it carries a body: LIST and UIDL without an argument,
//                             RETR, TOP, CAPA
//   Reader                    the responses of a server from its bytes in any pieces; the
//                             client says for each command it sent whether a body may follow
//                             (expect(multi)); Reader::new() waits for the greeting first
//   PORT_POP3 (110), verb_known
//
// A body travels with CRLF line ends; Response::body holds it with the dots undone and CRLF
// after every line (the message as the server stored it).  The text after +OK and -ERR is the
// server's own words; RFC 2449's codes in brackets ("-ERR [AUTH] ...") stay part of it.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const PORT_POP3: u16 = 110;

/// the commands of RFC 1939 and 2449 (and those of later RFCs a dissector names)
pub fn verb_known(v: &str) -> bool {
    const VERBS: [&str; 15] = ["USER", "PASS", "APOP", "STAT", "LIST", "RETR", "DELE", "NOOP", "RSET", "QUIT", "TOP", "UIDL", "CAPA", "STLS", "AUTH"];
    VERBS.iter().any(|x| x.eq_ignore_ascii_case(v)) || v.eq_ignore_ascii_case("UTF8")
}

/// a command line of a client
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
    /// in capitals
    pub verb: String,
    /// the words after it (PASS keeps the rest of the line as one argument: a password may
    /// hold spaces)
    pub args: Vec<String>,
}

impl Command {
    pub fn new(verb: &str, args: &[&str]) -> Command {
        Command { verb: verb.to_ascii_uppercase(), args: args.iter().map(|a| String::from(*a)).collect() }
    }
    /// a line without its line end; None for an empty one
    pub fn parse(line: &str) -> Option<Command> {
        let l = line.trim_end_matches(['\r', '\n']).trim_start();
        if l.is_empty() {
            return None;
        }
        let (v, rest) = l.split_once(' ').unwrap_or((l, ""));
        let verb = v.to_ascii_uppercase();
        let args = if verb == "PASS" { if rest.is_empty() { Vec::new() } else { alloc::vec![String::from(rest)] } } else { rest.split_whitespace().map(String::from).collect() };
        Some(Command { verb, args })
    }
    /// the line on the wire, CRLF included
    pub fn to_line(&self) -> String {
        if self.args.is_empty() { format!("{}\r\n", self.verb) } else { format!("{} {}\r\n", self.verb, self.args.join(" ")) }
    }
    /// argument k as a number (a message number)
    pub fn num(&self, k: usize) -> Option<u32> {
        self.args.get(k)?.parse().ok()
    }
}

/// a +OK to this command is followed by lines and the line with the dot (RFC 1939 3)
pub fn multi_line(c: &Command) -> bool {
    match c.verb.as_str() {
        "LIST" | "UIDL" => c.args.is_empty(),
        "RETR" | "TOP" | "CAPA" => true,
        _ => false,
    }
}

/// a response of a server
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// +OK (else -ERR)
    pub ok: bool,
    /// the text after the status indicator
    pub text: String,
    /// the lines of a multi-line response, CRLF after each, dots undone
    pub body: Option<Vec<u8>>,
}

impl Response {
    pub fn ok(text: &str) -> Response {
        Response { ok: true, text: String::from(text), body: None }
    }
    pub fn err(text: &str) -> Response {
        Response { ok: false, text: String::from(text), body: None }
    }
    /// the same response with lines after it
    pub fn with_body(mut self, body: &[u8]) -> Response {
        self.body = Some(Vec::from(body));
        self
    }
    /// the status line ("+OK text") without the line end
    pub fn status_line(&self) -> String {
        let s = if self.ok { "+OK" } else { "-ERR" };
        if self.text.is_empty() { String::from(s) } else { format!("{} {}", s, self.text) }
    }
    /// the bytes on the wire: the status line, the body lines with dot-stuffing and ".\r\n"
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = self.status_line().into_bytes();
        v.extend_from_slice(b"\r\n");
        if let Some(b) = &self.body {
            v.extend_from_slice(&stuffed(b));
        }
        v
    }
}

/// a body as it travels: CRLF line ends, a line that starts with a dot gets another one, then
/// ".\r\n"
pub fn stuffed(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 8);
    if !body.is_empty() {
        let b = body.strip_suffix(b"\n").unwrap_or(body);
        let b = b.strip_suffix(b"\r").unwrap_or(b);
        for line in b.split(|&c| c == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.first() == Some(&b'.') {
                out.push(b'.');
            }
            out.extend_from_slice(line);
            out.extend_from_slice(b"\r\n");
        }
    }
    out.extend_from_slice(b".\r\n");
    out
}

/// the responses of a server, as its bytes arrive
#[derive(Clone, Debug, Default)]
pub struct Reader {
    buf: Vec<u8>,
    /// for each command sent: may a body follow its +OK
    expect: VecDeque<bool>,
    /// a multi-line response whose dot did not come yet
    open: Option<Response>,
}

impl Reader {
    /// a reader that waits for the greeting first (a single line)
    pub fn new() -> Reader {
        let mut r = Reader::default();
        r.expect.push_back(false);
        r
    }
    /// the next response (after the greeting, which is single-line) answers a command whose
    /// +OK carries a body when `multi`
    pub fn expect(&mut self, multi: bool) {
        self.expect.push_back(multi);
    }
    /// bytes of the server: the responses complete now
    pub fn push(&mut self, b: &[u8]) -> Vec<Response> {
        self.buf.extend_from_slice(b);
        let mut out = Vec::new();
        let mut start = 0;
        while let Some(i) = self.buf[start..].iter().position(|&c| c == b'\n') {
            let end = start + i;
            let mut e = end;
            if e > start && self.buf[e - 1] == b'\r' {
                e -= 1;
            }
            let line: Vec<u8> = self.buf[start..e].to_vec();
            start = end + 1;
            if let Some(r) = self.open.as_mut() {
                if line == b"." {
                    if let Some(r) = self.open.take() {
                        out.push(r);
                    }
                    continue;
                }
                let l = if line.first() == Some(&b'.') { &line[1..] } else { &line[..] };
                let body = r.body.get_or_insert_with(Vec::new);
                body.extend_from_slice(l);
                body.extend_from_slice(b"\r\n");
                continue;
            }
            let text = String::from_utf8_lossy(&line).into_owned();
            let (ok, rest) = if let Some(r) = text.strip_prefix("+OK") {
                (true, r)
            } else if let Some(r) = text.strip_prefix("-ERR") {
                (false, r)
            } else {
                // no status line: skipped
                continue;
            };
            let multi = self.expect.pop_front().unwrap_or(false);
            let r = Response { ok, text: String::from(rest.strip_prefix(' ').unwrap_or(rest)), body: None };
            if ok && multi {
                self.open = Some(Response { body: Some(Vec::new()), ..r });
            } else {
                out.push(r);
            }
        }
        self.buf.drain(..start);
        out
    }
}
