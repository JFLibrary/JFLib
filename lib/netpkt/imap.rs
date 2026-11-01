// imap -- IMAP without I/O (a subset of RFC 9051 and RFC 3501): command and response lines with
// tags, quoted strings, literals and parenthesized lists, the readers of both directions, sequence
// sets, flags, FETCH items, SEARCH keys and the ENVELOPE of a message
//
//   PORT_IMAP (143), verb_known
//   Item                      Atom | Quoted | Literal | List | Nil: one value of a line; text()
//   Line { text, lits }       a whole logical line: the bytes with its literals apart
//   LineReader                bytes in any pieces: push(b) then next() -> Some(Read::Line(line)) or
//                             Some(Read::NeedLiteral(n)), a client that waits for "+" (a server
//                             answers it)
//   tokens(&Line) -> Vec<Item>   the values of a line
//   Command { tag, verb, args }  Command::parse(&line) (the verb in capitals); line(tag, verb,
//                             args) the bytes of a simple command
//   Status, Kind, Response    Response::parse(&line): a tagged status, an untagged one, untagged
//                             data, a continuation; response code, text, the values
//   ok / no / bad / untagged / cont   server lines ("a1 OK done\r\n")
//   astring(s) / quoted(s)    an atom when it can be, else quoted, a literal for other bytes
//   SeqSet                    "1:3,5,*": parse, contains(n, max), to_text
//   FetchItem, Section        parse_fetch(item): what FETCH asks (ALL, FAST, FULL too)
//   SearchKey                 parse_search(args), matches through a closure of facts
//   envelope(&Message)        the ENVELOPE of RFC 9051 7.5.2 as text
//   internal_date(unix)       "01-Jan-2026 00:00:00 +0000"
//   header_of(data) / text_of(data) / header_fields(data, names, not)   the parts FETCH names
//   SEEN, ANSWERED, FLAGGED, DELETED, DRAFT, RECENT    the flags
//
// A line ends with CRLF.  A literal {n} (or {n+}) at its end means n bytes follow the CRLF and the
// line goes on after them; Line keeps those bytes in `lits` and marks their place in `text` with a
// NUL, which IMAP text never has.  Words are not checked here beyond their shape: what a verb takes
// is the server's business.  Strings a server sends in the body of its messages are literals.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::imf::{self, Mailbox, Message};

pub const PORT_IMAP: u16 = 143;

pub const SEEN: &str = "\\Seen";
pub const ANSWERED: &str = "\\Answered";
pub const FLAGGED: &str = "\\Flagged";
pub const DELETED: &str = "\\Deleted";
pub const DRAFT: &str = "\\Draft";
pub const RECENT: &str = "\\Recent";

/// the commands of RFC 9051 and 3501 and of the extensions a dissector meets
pub fn verb_known(v: &str) -> bool {
    matches!(
        v.to_ascii_uppercase().as_str(),
        "CAPABILITY" | "NOOP" | "LOGOUT" | "STARTTLS" | "AUTHENTICATE" | "LOGIN" | "SELECT" | "EXAMINE" | "CREATE" | "DELETE" | "RENAME" | "SUBSCRIBE" | "UNSUBSCRIBE" | "LIST" | "LSUB" | "NAMESPACE" | "STATUS" | "APPEND" | "IDLE" | "CHECK" | "CLOSE" | "UNSELECT" | "EXPUNGE" | "SEARCH" | "FETCH" | "STORE" | "COPY" | "MOVE" | "UID" | "ENABLE" | "DONE"
    )
}

/// one value of a line
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Atom(String),
    Quoted(String),
    Literal(Vec<u8>),
    List(Vec<Item>),
    Nil,
}

impl Item {
    /// the value as text (an atom, a string, a literal; a list or NIL has none)
    pub fn text(&self) -> Option<String> {
        match self {
            Item::Atom(s) | Item::Quoted(s) => Some(s.clone()),
            Item::Literal(b) => Some(String::from_utf8_lossy(b).into_owned()),
            _ => None,
        }
    }
    pub fn atom(s: &str) -> Item {
        Item::Atom(String::from(s))
    }
}

/// a whole logical line: its bytes (CRLF gone, NUL where a literal stands) and its literals
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub text: Vec<u8>,
    pub lits: Vec<Vec<u8>>,
}

impl Line {
    /// a line without literals (tests, tools)
    pub fn plain(s: &str) -> Line {
        Line { text: Vec::from(s.as_bytes()), lits: Vec::new() }
    }
}

/// what a reader found
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    Line(Line),
    /// a literal of n bytes without "+" in its braces was announced: a server says "+ ..." now
    NeedLiteral(usize),
}

/// splits bytes into logical lines
#[derive(Clone, Debug, Default)]
pub struct LineReader {
    buf: Vec<u8>,
    cur: Line,
    /// the literal being read and how much of it is missing
    want: Option<usize>,
}

impl LineReader {
    pub fn new() -> LineReader {
        LineReader::default()
    }
    pub fn push(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    /// bytes it holds that make up no line yet
    pub fn pending(&self) -> usize {
        self.buf.len() + self.cur.text.len()
    }
    /// the next thing that is complete
    pub fn next(&mut self) -> Option<Read> {
        loop {
            if let Some(n) = self.want {
                if self.buf.len() < n {
                    return None;
                }
                let lit: Vec<u8> = self.buf.drain(..n).collect();
                self.cur.lits.push(lit);
                self.want = None;
            }
            let i = self.buf.iter().position(|&c| c == b'\n')?;
            let mut piece: Vec<u8> = self.buf.drain(..=i).collect();
            piece.pop();
            if piece.last() == Some(&b'\r') {
                piece.pop();
            }
            if let Some((at, n, plus)) = literal_at_end(&piece) {
                self.cur.text.extend_from_slice(&piece[..at]);
                self.cur.text.push(0);
                self.want = Some(n);
                if !plus {
                    return Some(Read::NeedLiteral(n));
                }
                continue;
            }
            self.cur.text.extend_from_slice(&piece);
            return Some(Read::Line(core::mem::take(&mut self.cur)));
        }
    }
}

/// a "{n}" or "{n+}" at the end of a piece: where it starts, n, whether it has the plus
fn literal_at_end(b: &[u8]) -> Option<(usize, usize, bool)> {
    if b.last() != Some(&b'}') {
        return None;
    }
    let at = b.iter().rposition(|&c| c == b'{')?;
    let inner = &b[at + 1..b.len() - 1];
    let (digits, plus) = match inner.last() {
        Some(b'+') => (&inner[..inner.len() - 1], true),
        _ => (inner, false),
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let n: usize = core::str::from_utf8(digits).ok()?.parse().ok()?;
    Some((at, n, plus))
}

/// the values of a line, from `from` on
pub fn tokens_from(l: &Line, from: usize) -> Vec<Item> {
    let mut at = from;
    let mut lit = l.text[..from.min(l.text.len())].iter().filter(|&&c| c == 0).count();
    items(l, &mut at, &mut lit, false)
}

/// the values of a whole line
pub fn tokens(l: &Line) -> Vec<Item> {
    tokens_from(l, 0)
}

fn items(l: &Line, at: &mut usize, lit: &mut usize, nested: bool) -> Vec<Item> {
    let b = &l.text;
    let mut out = Vec::new();
    while *at < b.len() {
        match b[*at] {
            b' ' => *at += 1,
            b')' if nested => {
                *at += 1;
                return out;
            }
            b'(' => {
                *at += 1;
                out.push(Item::List(items(l, at, lit, true)));
            }
            0 => {
                *at += 1;
                out.push(Item::Literal(l.lits.get(*lit).cloned().unwrap_or_default()));
                *lit += 1;
            }
            b'"' => {
                *at += 1;
                let mut s = Vec::new();
                while *at < b.len() && b[*at] != b'"' {
                    if b[*at] == b'\\' && *at + 1 < b.len() {
                        *at += 1;
                    }
                    s.push(b[*at]);
                    *at += 1;
                }
                *at += 1;
                out.push(Item::Quoted(String::from_utf8_lossy(&s).into_owned()));
            }
            _ => {
                // an atom; brackets may hold spaces and parentheses (BODY[HEADER.FIELDS (From)])
                let start = *at;
                let mut depth = 0i32;
                while *at < b.len() {
                    match b[*at] {
                        b'[' => depth += 1,
                        b']' => depth -= 1,
                        b' ' | b')' | b'(' if depth <= 0 => break,
                        _ => {}
                    }
                    *at += 1;
                }
                let w = String::from_utf8_lossy(&b[start..*at]).into_owned();
                out.push(if w.eq_ignore_ascii_case("NIL") { Item::Nil } else { Item::Atom(w) });
                if *at == start {
                    *at += 1;
                }
            }
        }
    }
    out
}

/// a command: its tag, the verb in capitals, its values
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub tag: String,
    pub verb: String,
    pub args: Vec<Item>,
}

impl Command {
    /// a command line; None without a tag and a verb
    pub fn parse(l: &Line) -> Option<Command> {
        let t = &l.text;
        let sp = t.iter().position(|&c| c == b' ')?;
        let tag = core::str::from_utf8(&t[..sp]).ok()?;
        if tag.is_empty() {
            return None;
        }
        let rest = sp + 1;
        let end = t[rest..].iter().position(|&c| c == b' ').map_or(t.len(), |i| rest + i);
        let verb = core::str::from_utf8(&t[rest..end]).ok()?;
        if verb.is_empty() {
            return None;
        }
        Some(Command { tag: String::from(tag), verb: verb.to_ascii_uppercase(), args: tokens_from(l, end) })
    }
    /// the text of argument k
    pub fn arg(&self, k: usize) -> Option<String> {
        self.args.get(k).and_then(Item::text)
    }
}

/// a simple command line of a client
pub fn line(tag: &str, verb: &str, args: &str) -> Vec<u8> {
    if args.is_empty() { format!("{} {}\r\n", tag, verb).into_bytes() } else { format!("{} {} {}\r\n", tag, verb, args).into_bytes() }
}

/// the status of a response
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    No,
    Bad,
    Bye,
    PreAuth,
}

impl Status {
    fn of(w: &str) -> Option<Status> {
        match w.to_ascii_uppercase().as_str() {
            "OK" => Some(Status::Ok),
            "NO" => Some(Status::No),
            "BAD" => Some(Status::Bad),
            "BYE" => Some(Status::Bye),
            "PREAUTH" => Some(Status::PreAuth),
            _ => None,
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            Status::Ok => "OK",
            Status::No => "NO",
            Status::Bad => "BAD",
            Status::Bye => "BYE",
            Status::PreAuth => "PREAUTH",
        }
    }
}

/// what sort of line a response is
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// "a1 OK ...": the end of a command
    Tagged(String),
    /// "* ..."
    Untagged,
    /// "+ ...": the server waits for more
    Cont,
}

/// a response line
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub kind: Kind,
    pub status: Option<Status>,
    /// the response code of a status line, without its brackets ("UIDNEXT 7")
    pub code: Option<String>,
    /// the text of a status line or a continuation
    pub text: String,
    /// the values of untagged data ("1", "FETCH", (...))
    pub items: Vec<Item>,
}

impl Response {
    pub fn parse(l: &Line) -> Option<Response> {
        let t = String::from_utf8_lossy(&l.text).into_owned();
        let sp = t.find(' ').unwrap_or(t.len());
        let first = &t[..sp];
        let rest_at = (sp + 1).min(t.len());
        let rest = &t[rest_at..];
        if first == "+" {
            return Some(Response { kind: Kind::Cont, status: None, code: None, text: String::from(rest), items: Vec::new() });
        }
        let kind = if first == "*" { Kind::Untagged } else { Kind::Tagged(String::from(first)) };
        let w2 = rest.split(' ').next().unwrap_or("");
        if let Some(st) = Status::of(w2) {
            let mut text = rest[w2.len()..].trim_start();
            let mut code = None;
            if let Some(c) = text.strip_prefix('[')
                && let Some(e) = c.find(']')
            {
                code = Some(String::from(&c[..e]));
                text = c[e + 1..].trim_start();
            }
            return Some(Response { kind, status: Some(st), code, text: String::from(text), items: Vec::new() });
        }
        if !matches!(kind, Kind::Untagged) {
            return None;
        }
        Some(Response { kind, status: None, code: None, text: String::new(), items: tokens_from(l, rest_at) })
    }
    /// the data's name: the first atom after an optional number ("FETCH", "LIST", "EXISTS")
    pub fn name(&self) -> Option<&str> {
        let mut it = self.items.iter();
        match it.next()? {
            Item::Atom(a) if a.bytes().all(|c| c.is_ascii_digit()) => match it.next()? {
                Item::Atom(b) => Some(b),
                _ => None,
            },
            Item::Atom(a) => Some(a),
            _ => None,
        }
    }
    /// the number before the name
    pub fn number(&self) -> Option<u32> {
        match self.items.first()? {
            Item::Atom(a) => a.parse().ok(),
            _ => None,
        }
    }
}

/// "a1 OK text" with an optional code
fn status_line(tag: &str, word: &str, code: Option<&str>, text: &str) -> Vec<u8> {
    match code {
        Some(c) => format!("{} {} [{}] {}\r\n", tag, word, c, text).into_bytes(),
        None => format!("{} {} {}\r\n", tag, word, text).into_bytes(),
    }
}

/// a status line OK / NO / BAD of a command
pub fn ok(tag: &str, code: Option<&str>, text: &str) -> Vec<u8> {
    status_line(tag, "OK", code, text)
}
pub fn no(tag: &str, code: Option<&str>, text: &str) -> Vec<u8> {
    status_line(tag, "NO", code, text)
}
pub fn bad(tag: &str, text: &str) -> Vec<u8> {
    status_line(tag, "BAD", None, text)
}
/// "* text"
pub fn untagged(text: &str) -> Vec<u8> {
    format!("* {}\r\n", text).into_bytes()
}
/// "+ text"
pub fn cont(text: &str) -> Vec<u8> {
    format!("+ {}\r\n", text).into_bytes()
}

/// a string in quotes
pub fn quoted(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            o.push('\\');
        }
        o.push(c);
    }
    o.push('"');
    o
}

/// an atom when it can be one, else a quoted string, else a literal (for other bytes)
pub fn astring(s: &str) -> String {
    if !s.is_empty() && s.bytes().all(|c| c.is_ascii_graphic() && !matches!(c, b'(' | b')' | b'{' | b'"' | b'\\' | b'%' | b'*' | b']')) {
        return String::from(s);
    }
    if s.bytes().all(|c| (0x20..0x7F).contains(&c)) {
        return quoted(s);
    }
    format!("{{{}}}\r\n{}", s.len(), s)
}

/// a string for an ENVELOPE: NIL when empty
fn nstring(s: &str) -> String {
    if s.is_empty() { String::from("NIL") } else { astring_q(s) }
}

/// quoted, or a literal for other bytes
fn astring_q(s: &str) -> String {
    if s.bytes().all(|c| (0x20..0x7F).contains(&c)) { quoted(s) } else { format!("{{{}}}\r\n{}", s.len(), s) }
}

/// a set of message numbers or UIDs: "1:3,5,*"
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeqSet {
    /// (from, to) with 0 for "*"
    ranges: Vec<(u32, u32)>,
}

impl SeqSet {
    pub fn parse(s: &str) -> Option<SeqSet> {
        let mut ranges = Vec::new();
        let num = |x: &str| -> Option<u32> {
            if x == "*" {
                return Some(0);
            }
            x.parse::<u32>().ok().filter(|&n| n > 0)
        };
        for part in s.split(',') {
            match part.split_once(':') {
                Some((a, b)) => ranges.push((num(a)?, num(b)?)),
                None => {
                    let n = num(part)?;
                    ranges.push((n, n));
                }
            }
        }
        if ranges.is_empty() { None } else { Some(SeqSet { ranges }) }
    }
    /// n is in the set; `max` is what "*" stands for
    pub fn contains(&self, n: u32, max: u32) -> bool {
        self.ranges.iter().any(|&(a, b)| {
            let (a, b) = (if a == 0 { max } else { a }, if b == 0 { max } else { b });
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            lo <= n && n <= hi
        })
    }
    pub fn to_text(&self) -> String {
        let one = |n: u32| if n == 0 { String::from("*") } else { format!("{}", n) };
        let v: Vec<String> = self.ranges.iter().map(|&(a, b)| if a == b { one(a) } else { format!("{}:{}", one(a), one(b)) }).collect();
        v.join(",")
    }
}

/// the part of a message BODY[...] names
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Section {
    Full,
    Header,
    /// HEADER.FIELDS (or .NOT, `not`)
    Fields(Vec<String>, bool),
    Text,
}

/// what FETCH asks for
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchItem {
    Flags,
    Uid,
    Rfc822Size,
    InternalDate,
    Envelope,
    /// BODY[...] or BODY.PEEK[...] (peek: \Seen stays as it is); the text shows it in the answer
    Body { peek: bool, section: Section, name: String },
    Rfc822,
    Rfc822Header,
    Rfc822Text,
}

/// the items of FETCH's argument: one atom or a list; None for one this does not know
pub fn parse_fetch(arg: &Item) -> Option<Vec<FetchItem>> {
    let one = |w: &str| -> Option<Vec<FetchItem>> {
        let u = w.to_ascii_uppercase();
        Some(match u.as_str() {
            "ALL" => alloc::vec![FetchItem::Flags, FetchItem::InternalDate, FetchItem::Rfc822Size, FetchItem::Envelope],
            "FAST" => alloc::vec![FetchItem::Flags, FetchItem::InternalDate, FetchItem::Rfc822Size],
            "FULL" => alloc::vec![FetchItem::Flags, FetchItem::InternalDate, FetchItem::Rfc822Size, FetchItem::Envelope],
            "FLAGS" => alloc::vec![FetchItem::Flags],
            "UID" => alloc::vec![FetchItem::Uid],
            "RFC822.SIZE" => alloc::vec![FetchItem::Rfc822Size],
            "INTERNALDATE" => alloc::vec![FetchItem::InternalDate],
            "ENVELOPE" => alloc::vec![FetchItem::Envelope],
            "RFC822" => alloc::vec![FetchItem::Rfc822],
            "RFC822.HEADER" => alloc::vec![FetchItem::Rfc822Header],
            "RFC822.TEXT" => alloc::vec![FetchItem::Rfc822Text],
            _ => {
                let (peek, rest) = if let Some(r) = u.strip_prefix("BODY.PEEK[") {
                    (true, r)
                } else if let Some(r) = u.strip_prefix("BODY[") {
                    (false, r)
                } else {
                    return None;
                };
                let inner = rest.strip_suffix(']')?;
                let section = match inner {
                    "" => Section::Full,
                    "HEADER" => Section::Header,
                    "TEXT" => Section::Text,
                    x if x.starts_with("HEADER.FIELDS") => {
                        let not = x.starts_with("HEADER.FIELDS.NOT");
                        let open = x.find('(')?;
                        let close = x.rfind(')')?;
                        Section::Fields(x[open + 1..close].split_whitespace().map(String::from).collect(), not)
                    }
                    _ => return None,
                };
                alloc::vec![FetchItem::Body { peek, section, name: format!("BODY[{}]", inner) }]
            }
        })
    };
    match arg {
        Item::Atom(w) => one(w),
        Item::List(l) => {
            let mut v = Vec::new();
            for x in l {
                let Item::Atom(w) = x else { return None };
                v.extend(one(w)?);
            }
            Some(v)
        }
        _ => None,
    }
}

/// a SEARCH key
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchKey {
    All,
    Seen,
    Unseen,
    Answered,
    Unanswered,
    Flagged,
    Unflagged,
    Deleted,
    Undeleted,
    Recent,
    From(String),
    To(String),
    Subject(String),
    Body(String),
    Text(String),
    Larger(u64),
    Smaller(u64),
    Uid(SeqSet),
    Seq(SeqSet),
    Not(alloc::boxed::Box<SearchKey>),
    Or(alloc::boxed::Box<SearchKey>, alloc::boxed::Box<SearchKey>),
    And(Vec<SearchKey>),
}

/// the keys of SEARCH (all must hold); None for what this does not know
pub fn parse_search(args: &[Item]) -> Option<Vec<SearchKey>> {
    let mut at = 0;
    let mut out = Vec::new();
    while at < args.len() {
        out.push(search_key(args, &mut at)?);
    }
    Some(out)
}

fn search_key(a: &[Item], at: &mut usize) -> Option<SearchKey> {
    let x = a.get(*at)?;
    *at += 1;
    if let Item::List(l) = x {
        return parse_search(l).map(SearchKey::And);
    }
    let w = x.text()?;
    let value = |at: &mut usize| -> Option<String> {
        let v = a.get(*at)?.text()?;
        *at += 1;
        Some(v)
    };
    Some(match w.to_ascii_uppercase().as_str() {
        "ALL" => SearchKey::All,
        "SEEN" => SearchKey::Seen,
        "UNSEEN" => SearchKey::Unseen,
        "ANSWERED" => SearchKey::Answered,
        "UNANSWERED" => SearchKey::Unanswered,
        "FLAGGED" => SearchKey::Flagged,
        "UNFLAGGED" => SearchKey::Unflagged,
        "DELETED" => SearchKey::Deleted,
        "UNDELETED" => SearchKey::Undeleted,
        "RECENT" | "NEW" => SearchKey::Recent,
        "FROM" => SearchKey::From(value(at)?),
        "TO" => SearchKey::To(value(at)?),
        "SUBJECT" => SearchKey::Subject(value(at)?),
        "BODY" => SearchKey::Body(value(at)?),
        "TEXT" => SearchKey::Text(value(at)?),
        "LARGER" => SearchKey::Larger(value(at)?.parse().ok()?),
        "SMALLER" => SearchKey::Smaller(value(at)?.parse().ok()?),
        "UID" => SearchKey::Uid(SeqSet::parse(&value(at)?)?),
        "NOT" => SearchKey::Not(alloc::boxed::Box::new(search_key(a, at)?)),
        "OR" => {
            let k1 = search_key(a, at)?;
            let k2 = search_key(a, at)?;
            SearchKey::Or(alloc::boxed::Box::new(k1), alloc::boxed::Box::new(k2))
        }
        other => SearchKey::Seq(SeqSet::parse(other)?),
    })
}

/// an address list of an ENVELOPE
fn addrs(v: &[Mailbox]) -> String {
    if v.is_empty() {
        return String::from("NIL");
    }
    let mut s = String::from("(");
    for m in v {
        let (user, host) = imf::split_addr(&m.addr).unwrap_or((&m.addr, ""));
        s.push_str(&format!("({} NIL {} {})", nstring(&m.name), nstring(user), nstring(host)));
    }
    s.push(')');
    s
}

/// the ENVELOPE of a message as its text (RFC 9051 7.5.2): date, subject, from, sender, reply-to,
/// to, cc, bcc, in-reply-to, message-id
pub fn envelope(m: &Message) -> String {
    let from = m.from().map(|f| alloc::vec![f]).unwrap_or_default();
    let list = |name: &str| m.get(name).map(imf::parse_list).unwrap_or_default();
    let sender = if m.get("Sender").is_some() { list("Sender") } else { from.clone() };
    let reply = if m.get("Reply-To").is_some() { list("Reply-To") } else { from.clone() };
    format!(
        "({} {} {} {} {} {} {} {} {} {})",
        nstring(m.get("Date").unwrap_or("")),
        nstring(m.get("Subject").unwrap_or("")),
        addrs(&from),
        addrs(&sender),
        addrs(&reply),
        addrs(&m.to()),
        addrs(&m.cc()),
        addrs(&list("Bcc")),
        nstring(m.get("In-Reply-To").unwrap_or("")),
        nstring(m.get("Message-ID").unwrap_or("")),
    )
}

/// "01-Jan-2026 00:00:00 +0000" of seconds since 1970 (UTC)
pub fn internal_date(unix: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // civil from days (Howard Hinnant's algorithm)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{:02}-{}-{:04} {:02}:{:02}:{:02} +0000", d, MONTHS[(m - 1) as usize], y, secs / 3600, (secs / 60) % 60, secs % 60)
}

/// where the header of a message ends: after its empty line
fn split_at(data: &[u8]) -> usize {
    let mut i = 0;
    while i < data.len() {
        let e = data[i..].iter().position(|&c| c == b'\n').map_or(data.len(), |p| i + p + 1);
        let l = &data[i..e];
        if l == b"\r\n" || l == b"\n" {
            return e;
        }
        i = e;
    }
    data.len()
}

/// the header of a message with its empty line
pub fn header_of(data: &[u8]) -> &[u8] {
    &data[..split_at(data)]
}

/// the text after the header
pub fn text_of(data: &[u8]) -> &[u8] {
    &data[split_at(data)..]
}

/// the fields of the header with these names (not: all others), each with its continuation lines,
/// and the empty line
pub fn header_fields(data: &[u8], names: &[String], not: bool) -> Vec<u8> {
    let head = header_of(data);
    let mut out = Vec::new();
    let mut keep = false;
    for l in head.split_inclusive(|&c| c == b'\n') {
        if l == b"\r\n" || l == b"\n" {
            break;
        }
        if !matches!(l.first(), Some(b' ' | b'\t')) {
            let name = l.split(|&c| c == b':').next().unwrap_or(&[]);
            let hit = names.iter().any(|n| n.as_bytes().eq_ignore_ascii_case(name));
            keep = hit != not;
        }
        if keep {
            out.extend_from_slice(l);
        }
    }
    out.extend_from_slice(b"\r\n");
    out
}
