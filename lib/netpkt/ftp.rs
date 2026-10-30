// ftp -- the control connection of the File Transfer Protocol (TCP port 21, RFC 959): command
// lines, replies with their multi-line form, the host-port of PORT and of the 227 reply to PASV,
// EPRT and the 229 reply to EPSV (RFC 2428), FEAT's list (RFC 2389)
//
//   Command { verb, arg }     Command::parse(line) (the verb in capitals, the argument as
//                             typed), Command::new(verb, arg), to_line() ("USER anna\r\n")
//   Reply { code, lines }     to_bytes(): one line "220 text", or "211-first" ... "211 last"
//                             (the inner lines start with a space when they could look like
//                             the end); is_final / is_ok (1xx preliminary, 2xx done, 3xx more)
//   Lines                     cuts the bytes of a control connection into lines (CRLF, a lone
//                             LF too), in any pieces as they arrive
//   ReplyReader               the replies of a server from its bytes: push(bytes) -> Vec<Reply>
//                             (a multi-line reply ends with a line of the same code and a space)
//   host_port(text) / fmt_host_port(ip, port)   "h1,h2,h3,h4,p1,p2" (PORT's argument, 227's
//                             numbers: host_port finds them anywhere in a reply's text)
//   eprt(arg) / fmt_eprt(addr, port)            "|1|10.0.0.5|40001|", "|2|2001:db8::5|40001|"
//   epsv_port(text) / fmt_epsv(port)            the "(|||port|)" of a 229 reply
//   PORT_FTP (21), PORT_FTP_DATA (20), verb_known
//
// Lines are UTF-8 or ASCII; RFC 959 allows Telnet sequences, which nobody sends here.  The
// reply texts are the server's own words: clients go by the codes and the numbers of 227 and
// 229 only.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::addr::{IpAddr, Ipv4};

pub const PORT_FTP: u16 = 21;
/// the server's port of active data connections
pub const PORT_FTP_DATA: u16 = 20;

/// the commands of RFC 959, 2389, 2428 and 3659 a dissector names
pub fn verb_known(v: &str) -> bool {
    const VERBS: [&str; 44] = [
        "USER", "PASS", "ACCT", "CWD", "CDUP", "SMNT", "QUIT", "REIN", "PORT", "PASV", "TYPE", "STRU", "MODE", "RETR", "STOR", "STOU", "APPE", "ALLO", "REST", "RNFR", "RNTO", "ABOR", "DELE", "RMD", "MKD", "PWD", "LIST", "NLST", "SITE", "SYST", "STAT", "HELP", "NOOP", "FEAT", "OPTS", "EPRT", "EPSV", "SIZE", "MDTM", "MLSD", "MLST", "XPWD", "XCWD", "XMKD",
    ];
    VERBS.iter().any(|x| x.eq_ignore_ascii_case(v)) || v.eq_ignore_ascii_case("XRMD") || v.eq_ignore_ascii_case("XCUP")
}

/// a command line of a client
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
    /// in capitals
    pub verb: String,
    /// everything after the first space (may hold spaces: a file name)
    pub arg: String,
}

impl Command {
    pub fn new(verb: &str, arg: &str) -> Command {
        Command { verb: verb.to_ascii_uppercase(), arg: String::from(arg) }
    }
    /// a line without its line end; None for an empty one
    pub fn parse(line: &str) -> Option<Command> {
        let l = line.trim_end_matches(['\r', '\n']);
        let l = l.trim_start();
        if l.is_empty() {
            return None;
        }
        let (v, a) = l.split_once(' ').unwrap_or((l, ""));
        Some(Command { verb: v.to_ascii_uppercase(), arg: String::from(a) })
    }
    /// the line on the wire, CRLF included
    pub fn to_line(&self) -> String {
        if self.arg.is_empty() { format!("{}\r\n", self.verb) } else { format!("{} {}\r\n", self.verb, self.arg) }
    }
}

/// a reply of a server
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    pub code: u16,
    /// the text of each line (without the code of the first and the last)
    pub lines: Vec<String>,
}

impl Reply {
    pub fn new(code: u16, text: &str) -> Reply {
        Reply { code, lines: text.split('\n').map(String::from).collect() }
    }
    /// the text of the first line
    pub fn text(&self) -> &str {
        self.lines.first().map(String::as_str).unwrap_or("")
    }
    /// 1xx: more replies follow for this command
    pub fn is_preliminary(&self) -> bool {
        (100..200).contains(&self.code)
    }
    /// 2xx: done
    pub fn is_ok(&self) -> bool {
        (200..300).contains(&self.code)
    }
    /// 3xx: the command waits for another (PASS after USER)
    pub fn is_intermediate(&self) -> bool {
        (300..400).contains(&self.code)
    }
    /// the reply as bytes: "code text\r\n", or the multi-line form of RFC 959 4.2
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut s = String::new();
        let n = self.lines.len().max(1);
        if n == 1 {
            s.push_str(&format!("{} {}\r\n", self.code, self.text()));
        } else {
            for (i, l) in self.lines.iter().enumerate() {
                if i == 0 {
                    s.push_str(&format!("{}-{}\r\n", self.code, l));
                } else if i + 1 == n {
                    s.push_str(&format!("{} {}\r\n", self.code, l));
                } else if l.as_bytes().first().is_some_and(|c| c.is_ascii_digit()) {
                    // a line that starts with digits could end the reply: indent it
                    s.push_str(&format!(" {}\r\n", l));
                } else {
                    s.push_str(&format!("{}\r\n", l));
                }
            }
        }
        s.into_bytes()
    }
}

/// cuts a byte stream into lines
#[derive(Clone, Debug, Default)]
pub struct Lines {
    buf: Vec<u8>,
}

impl Lines {
    pub fn new() -> Lines {
        Lines::default()
    }
    /// the complete lines in `b` and what came before it, without their line ends
    pub fn push(&mut self, b: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(b);
        let mut out = Vec::new();
        while let Some(i) = self.buf.iter().position(|&c| c == b'\n') {
            let mut l: Vec<u8> = self.buf.drain(..=i).collect();
            l.pop();
            if l.last() == Some(&b'\r') {
                l.pop();
            }
            out.push(String::from_utf8_lossy(&l).into_owned());
        }
        out
    }
    /// bytes waiting for their line end
    pub fn pending(&self) -> usize {
        self.buf.len()
    }
}

/// the code of a reply line: three digits and a space or a dash
fn code_of(l: &str) -> Option<(u16, bool)> {
    let b = l.as_bytes();
    if b.len() < 3 || !b[..3].iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let code = (b[0] - b'0') as u16 * 100 + (b[1] - b'0') as u16 * 10 + (b[2] - b'0') as u16;
    match b.get(3) {
        None | Some(b' ') => Some((code, false)),
        Some(b'-') => Some((code, true)),
        _ => None,
    }
}

/// the replies of a server, as its bytes arrive
#[derive(Clone, Debug, Default)]
pub struct ReplyReader {
    lines: Lines,
    /// a multi-line reply that has not ended yet
    open: Option<Reply>,
}

impl ReplyReader {
    pub fn new() -> ReplyReader {
        ReplyReader::default()
    }
    /// the replies that are complete now; a line that is no reply is skipped
    pub fn push(&mut self, b: &[u8]) -> Vec<Reply> {
        let mut out = Vec::new();
        for l in self.lines.push(b) {
            let c = code_of(&l);
            match self.open.as_mut() {
                Some(r) => match c {
                    Some((code, false)) if code == r.code => {
                        r.lines.push(String::from(l.get(4..).unwrap_or("")));
                        if let Some(r) = self.open.take() {
                            out.push(r);
                        }
                    }
                    _ => r.lines.push(String::from(l.strip_prefix(' ').unwrap_or(&l))),
                },
                None => match c {
                    Some((code, true)) => self.open = Some(Reply { code, lines: alloc::vec![String::from(l.get(4..).unwrap_or(""))] }),
                    Some((code, false)) => out.push(Reply { code, lines: alloc::vec![String::from(l.get(4..).unwrap_or(""))] }),
                    None => {}
                },
            }
        }
        out
    }
}

/// "h1,h2,h3,h4,p1,p2" anywhere in a text (PORT's argument; a 227 reply, where clients look
/// for the six numbers whatever the words around them)
pub fn host_port(text: &str) -> Option<(Ipv4, u16)> {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !b[i - 1].is_ascii_digit()) {
            let mut nums: [u32; 6] = [0; 6];
            let mut k = 0;
            let mut j = i;
            loop {
                let s = j;
                while j < b.len() && b[j].is_ascii_digit() && j - s < 4 {
                    j += 1;
                }
                if j == s || (j < b.len() && b[j].is_ascii_digit()) {
                    break;
                }
                nums[k] = text[s..j].parse().unwrap_or(1000);
                k += 1;
                if k == 6 || j >= b.len() || b[j] != b',' {
                    break;
                }
                j += 1;
            }
            if k == 6 && nums.iter().all(|&n| n < 256) {
                return Some((Ipv4([nums[0] as u8, nums[1] as u8, nums[2] as u8, nums[3] as u8]), (nums[4] * 256 + nums[5]) as u16));
            }
        }
        i += 1;
    }
    None
}

/// the six numbers of an IPv4 address and a port
pub fn fmt_host_port(ip: Ipv4, port: u16) -> String {
    let a = ip.0;
    format!("{},{},{},{},{},{}", a[0], a[1], a[2], a[3], port >> 8, port & 0xFF)
}

/// EPRT's argument: a delimiter, the family (1 IPv4, 2 IPv6), the address and the port
pub fn eprt(arg: &str) -> Option<(IpAddr, u16)> {
    let d = arg.chars().next()?;
    if !(33..=126).contains(&(d as u32)) {
        return None;
    }
    let parts: Vec<&str> = arg.split(d).collect();
    if parts.len() != 5 || !parts[0].is_empty() || !parts[4].is_empty() {
        return None;
    }
    let addr = IpAddr::parse(parts[2])?;
    match (parts[1], addr) {
        ("1", IpAddr::V4(_)) | ("2", IpAddr::V6(_)) => {}
        _ => return None,
    }
    let port: u16 = parts[3].parse().ok()?;
    Some((addr, port))
}

/// EPRT's argument for an address and a port
pub fn fmt_eprt(addr: IpAddr, port: u16) -> String {
    format!("|{}|{}|{}|", if addr.is_v4() { 1 } else { 2 }, addr, port)
}

/// the port of a 229 reply: "(|||port|)" in its text (any delimiter of RFC 2428)
pub fn epsv_port(text: &str) -> Option<u16> {
    let open = text.find('(')?;
    let rest = &text[open + 1..];
    let close = rest.find(')')?;
    let inner = &rest[..close];
    let d = inner.chars().next()?;
    let parts: Vec<&str> = inner.split(d).collect();
    if parts.len() != 5 || !parts[1].is_empty() || !parts[2].is_empty() || !parts[4].is_empty() {
        return None;
    }
    parts[3].parse().ok()
}

/// "(|||port|)" for a 229 reply
pub fn fmt_epsv(port: u16) -> String {
    format!("(|||{}|)", port)
}
