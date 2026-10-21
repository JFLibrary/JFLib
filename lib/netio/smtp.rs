// smtp -- sending mail (RFC 5321) for notifications
//
//   send(&SmtpConfig, &Mail) -> Result<String, String>   the server's final answer or the
//                                                        reason it failed (German, for people)
//   Security::StartTls (port 587) / Tls (465) / None (25; refuses to authenticate in clear)
//   AUTH PLAIN or LOGIN (whatever the server offers; PLAIN first)
//   message(&Mail, host, now) -> the RFC 5322 text: UTF-8 subject as an encoded word, the body
//   text/plain (and text/html as multipart/alternative) in base64, Date and Message-ID
//
// TLS checks the server like a browser (the system's certificate authorities, plus extra CAs
// for a company's own); after STARTTLS nothing the server sent before the switch is used
// (no command injection across the switch).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use plat::net::TcpStream;

use crate::tls::{self, ClientConfig, Trust};
use crate::x509::Cert;
use crate::Stream;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Security {
    StartTls,
    Tls,
    None,
}

impl Security {
    pub fn parse(s: &str) -> Security {
        match s {
            "tls" | "ssl" => Security::Tls,
            "none" | "plain" => Security::None,
            _ => Security::StartTls,
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Security::StartTls => 587,
            Security::Tls => 465,
            Security::None => 25,
        }
    }
}

#[derive(Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
    pub password: String,
    /// our name in EHLO
    pub helo: String,
    /// CAs trusted in addition to the system's
    pub extra_cas: Vec<Cert>,
    pub timeout_ms: u32,
}

#[derive(Clone, Default)]
pub struct Mail {
    pub from: String,
    pub to: Vec<String>,
    pub subject: String,
    pub text: String,
    /// an HTML alternative (optional)
    pub html: String,
}

/// one line-oriented SMTP conversation
struct Conv {
    s: Box<dyn Stream>,
    buf: Vec<u8>,
}

impl Conv {
    fn line(&mut self) -> Result<String, String> {
        loop {
            if let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
                let l: Vec<u8> = self.buf.drain(..=i).collect();
                return Ok(String::from_utf8_lossy(&l).trim_end().into());
            }
            if self.buf.len() > 64 * 1024 {
                return Err(String::from("zu lange Antwortzeile des Mailservers"));
            }
            let mut b = [0u8; 4096];
            let n = self.s.read(&mut b).map_err(|e| format!("Verbindung zum Mailserver unterbrochen: {}", e))?;
            if n == 0 {
                return Err(String::from("der Mailserver hat die Verbindung geschlossen"));
            }
            self.buf.extend_from_slice(&b[..n]);
        }
    }

    /// a (multi-line) reply -> (code, lines)
    fn reply(&mut self) -> Result<(u16, Vec<String>), String> {
        let mut lines = Vec::new();
        loop {
            let l = self.line()?;
            if l.len() < 3 || !l.as_bytes()[..3].iter().all(u8::is_ascii_digit) {
                return Err(format!("unverständliche Antwort des Mailservers: {}", l));
            }
            let code: u16 = l[..3].parse().unwrap_or(0);
            let more = l.as_bytes().get(3) == Some(&b'-');
            lines.push(String::from(l.get(4..).unwrap_or("")));
            if !more {
                return Ok((code, lines));
            }
        }
    }

    fn send(&mut self, cmd: &str) -> Result<(), String> {
        self.s.write_all(format!("{}\r\n", cmd).as_bytes()).map_err(|e| format!("Senden an den Mailserver fehlgeschlagen: {}", e))
    }

    /// a command whose reply must have one of the codes
    fn cmd(&mut self, cmd: &str, want: &[u16], shown: &str) -> Result<Vec<String>, String> {
        self.send(cmd)?;
        let (code, lines) = self.reply()?;
        if want.contains(&code) {
            Ok(lines)
        } else {
            Err(format!("{} abgelehnt: {} {}", shown, code, lines.join(" ")))
        }
    }
}

fn b64(s: &[u8]) -> String {
    crypto::b64::encode(s)
}

/// send one message
pub fn send(cfg: &SmtpConfig, mail: &Mail) -> Result<String, String> {
    if mail.to.is_empty() {
        return Err(String::from("kein Empfänger"));
    }
    for a in mail.to.iter().chain(core::iter::once(&mail.from)) {
        if !a.contains('@') || a.contains(['<', '>', '\r', '\n', ' ']) {
            return Err(format!("ungültige Adresse „{}“", a));
        }
    }
    let addrs = crate::dns::resolve(&cfg.host, cfg.port).map_err(|e| format!("{}: {}", cfg.host, e))?;
    let tcp = TcpStream::connect_any(&addrs, cfg.timeout_ms).map_err(|e| format!("{}:{} nicht erreichbar: {}", cfg.host, cfg.port, e))?;
    tcp.set_read_timeout(Some(cfg.timeout_ms));
    tcp.set_write_timeout(Some(cfg.timeout_ms));
    let tls_cfg = || ClientConfig { trust: Trust::System(cfg.extra_cas.clone()), identity: None, server_name: cfg.host.clone() };
    let s: Box<dyn Stream> = if cfg.security == Security::Tls {
        Box::new(tls::connect(Box::new(tcp), tls_cfg()).map_err(|e| format!("TLS mit {}: {}", cfg.host, e))?)
    } else {
        Box::new(tcp)
    };
    let mut c = Conv { s, buf: Vec::new() };
    let (code, lines) = c.reply()?;
    if code != 220 {
        return Err(format!("der Mailserver lehnt ab: {} {}", code, lines.join(" ")));
    }
    let mut caps = c.cmd(&format!("EHLO {}", cfg.helo), &[250], "EHLO")?;
    if cfg.security == Security::StartTls {
        if !caps.iter().any(|l| l.eq_ignore_ascii_case("STARTTLS")) {
            return Err(format!("{} bietet kein STARTTLS an", cfg.host));
        }
        c.cmd("STARTTLS", &[220], "STARTTLS")?;
        if !c.buf.is_empty() {
            return Err(String::from("der Mailserver hat vor der Verschlüsselung zu viel gesendet"));
        }
        let plain = core::mem::replace(&mut c.s, Box::new(Closed));
        c.s = Box::new(tls::connect(plain, tls_cfg()).map_err(|e| format!("TLS mit {}: {}", cfg.host, e))?);
        caps = c.cmd(&format!("EHLO {}", cfg.helo), &[250], "EHLO")?;
    }
    if !cfg.username.is_empty() {
        if cfg.security == Security::None {
            return Err(String::from("Anmeldung nur über eine verschlüsselte Verbindung"));
        }
        let auth = caps.iter().find(|l| l.to_ascii_uppercase().starts_with("AUTH")).map(|l| l.to_ascii_uppercase()).unwrap_or_default();
        if auth.split([' ', '=']).any(|m| m == "PLAIN") {
            let token = b64(format!("\0{}\0{}", cfg.username, cfg.password).as_bytes());
            c.cmd(&format!("AUTH PLAIN {}", token), &[235], "Anmeldung")?;
        } else if auth.split([' ', '=']).any(|m| m == "LOGIN") {
            c.cmd("AUTH LOGIN", &[334], "Anmeldung")?;
            c.cmd(&b64(cfg.username.as_bytes()), &[334], "Anmeldung")?;
            c.cmd(&b64(cfg.password.as_bytes()), &[235], "Anmeldung")?;
        } else {
            return Err(String::from("der Mailserver bietet keine bekannte Anmeldung (PLAIN, LOGIN) an"));
        }
    }
    c.cmd(&format!("MAIL FROM:<{}>", mail.from), &[250], "Absender")?;
    for to in &mail.to {
        c.cmd(&format!("RCPT TO:<{}>", to), &[250, 251], &format!("Empfänger {}", to))?;
    }
    c.cmd("DATA", &[354], "DATA")?;
    let now = plat::unix_time_ns().div_euclid(1_000_000_000);
    let text = message(mail, &cfg.helo, now);
    // dot-stuffing; the text has CRLF line ends
    let mut body = String::with_capacity(text.len() + 64);
    for l in text.split("\r\n") {
        if l.starts_with('.') {
            body.push('.');
        }
        body.push_str(l);
        body.push_str("\r\n");
    }
    body.push_str(".\r\n");
    c.s.write_all(body.as_bytes()).map_err(|e| format!("Senden an den Mailserver fehlgeschlagen: {}", e))?;
    let (code, lines) = c.reply()?;
    if code != 250 {
        return Err(format!("die Nachricht wurde abgelehnt: {} {}", code, lines.join(" ")));
    }
    let _ = c.send("QUIT");
    c.s.shutdown();
    Ok(format!("{} {}", code, lines.join(" ")))
}

/// a placeholder while the plain stream moves into TLS
struct Closed;

impl Stream for Closed {
    fn read(&self, _buf: &mut [u8]) -> plat::net::NetResult<usize> {
        Ok(0)
    }
    fn write_all(&self, _data: &[u8]) -> plat::net::NetResult<()> {
        Err(plat::net::NetError::new(plat::net::NetErrorKind::Reset, 0))
    }
    fn set_timeouts(&self, _r: Option<u32>, _w: Option<u32>) {}
    fn shutdown(&self) {}
    fn wait_readable(&self, _ms: u32) -> bool {
        false
    }
}

/// "=?UTF-8?B?...?=" when the text is not plain ASCII
fn encoded_word(s: &str) -> String {
    if s.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        return String::from(s);
    }
    // whole characters per word, at most 45 bytes (60 base64 characters)
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if cur.len() + ch.len_utf8() > 45 {
            out.push(format!("=?UTF-8?B?{}?=", b64(cur.as_bytes())));
            cur.clear();
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(format!("=?UTF-8?B?{}?=", b64(cur.as_bytes())));
    }
    out.join("\r\n ")
}

fn wrap76(b64text: &str) -> String {
    let mut s = String::new();
    for c in b64text.as_bytes().chunks(76) {
        s.push_str(core::str::from_utf8(c).unwrap_or(""));
        s.push_str("\r\n");
    }
    s
}

/// "Tue, 13 Oct 2026 15:04:05 +0000"
pub fn rfc5322_date(t: i64) -> String {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let wd = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"][days.rem_euclid(7) as usize];
    let mon = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][(m - 1) as usize];
    format!("{}, {} {} {} {:02}:{:02}:{:02} +0000", wd, d, mon, y, secs / 3600, secs / 60 % 60, secs % 60)
}

/// the message text (CRLF line ends)
pub fn message(mail: &Mail, host: &str, now: i64) -> String {
    let id = crypto::hex(&plat::random_bytes::<12>());
    let mut m = String::new();
    m.push_str(&format!("From: {}\r\n", mail.from));
    m.push_str(&format!("To: {}\r\n", mail.to.join(", ")));
    m.push_str(&format!("Subject: {}\r\n", encoded_word(&mail.subject)));
    m.push_str(&format!("Date: {}\r\n", rfc5322_date(now)));
    m.push_str(&format!("Message-ID: <{}@{}>\r\n", id, if host.is_empty() { "backupcraft" } else { host }));
    m.push_str("MIME-Version: 1.0\r\n");
    m.push_str("Auto-Submitted: auto-generated\r\n");
    let text = mail.text.replace("\r\n", "\n").replace('\n', "\r\n");
    if mail.html.is_empty() {
        m.push_str("Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n");
        m.push_str(&wrap76(&b64(text.as_bytes())));
    } else {
        let boundary = format!("=_bc_{}", id);
        m.push_str(&format!("Content-Type: multipart/alternative; boundary=\"{}\"\r\n\r\n", boundary));
        m.push_str(&format!("--{}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n", boundary));
        m.push_str(&wrap76(&b64(text.as_bytes())));
        m.push_str(&format!("--{}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n", boundary));
        m.push_str(&wrap76(&b64(mail.html.as_bytes())));
        m.push_str(&format!("--{}--\r\n", boundary));
    }
    m
}
