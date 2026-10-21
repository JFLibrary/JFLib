// syslog -- syslog messages over UDP as BSD and Linux systems (RFC 3164) and Cisco IOS send
// them; RFC 5424 messages are recognised
//
//   pri(facility, severity) / facility_of(pri) / severity_of(pri)
//   SEV_* / FAC_*; severity_name ("notice"), facility_name ("local7")
//   bsd(pri, ts, host, tag, text)      "<30>Oct 19 12:00:05 pc1 dhclient: bound to 10.0.1.10"
//   ios(pri, seq, ts, text)            "<189>12: *Oct 19 12:00:05.123: %LINK-3-UPDOWN: ..."
//   bsd_time(month, day, h, m, s)      "Oct  9 08:00:05" (day padded with a space)
//   parse(b) -> Option<Msg>            priority and the rest; for RFC 3164 also the time,
//                                      host and tag when they are there

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const SEV_EMERG: u8 = 0;
pub const SEV_ALERT: u8 = 1;
pub const SEV_CRIT: u8 = 2;
pub const SEV_ERR: u8 = 3;
pub const SEV_WARNING: u8 = 4;
pub const SEV_NOTICE: u8 = 5;
pub const SEV_INFO: u8 = 6;
pub const SEV_DEBUG: u8 = 7;

pub const FAC_KERN: u8 = 0;
pub const FAC_USER: u8 = 1;
pub const FAC_DAEMON: u8 = 3;
pub const FAC_AUTH: u8 = 4;
pub const FAC_SYSLOG: u8 = 5;
pub const FAC_LOCAL0: u8 = 16;
pub const FAC_LOCAL7: u8 = 23;

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

pub fn pri(facility: u8, severity: u8) -> u8 {
    (facility & 31) << 3 | (severity & 7)
}
pub fn facility_of(pri: u8) -> u8 {
    pri >> 3
}
pub fn severity_of(pri: u8) -> u8 {
    pri & 7
}

pub fn severity_name(s: u8) -> &'static str {
    ["emerg", "alert", "crit", "err", "warning", "notice", "info", "debug"][(s & 7) as usize]
}

pub fn facility_name(f: u8) -> &'static str {
    match f {
        0 => "kern",
        1 => "user",
        2 => "mail",
        3 => "daemon",
        4 => "auth",
        5 => "syslog",
        6 => "lpr",
        7 => "news",
        8 => "uucp",
        9 => "cron",
        10 => "authpriv",
        11 => "ftp",
        16 => "local0",
        17 => "local1",
        18 => "local2",
        19 => "local3",
        20 => "local4",
        21 => "local5",
        22 => "local6",
        23 => "local7",
        _ => "?",
    }
}

/// "Oct  9 08:00:05" (RFC 3164 4.1.2)
pub fn bsd_time(month: u8, day: u8, h: u8, m: u8, s: u8) -> String {
    format!("{} {:>2} {:02}:{:02}:{:02}", MONTHS[(month.clamp(1, 12) - 1) as usize], day, h, m, s)
}

/// a message as Linux (rsyslog's traditional forward format) sends it
pub fn bsd(pri: u8, ts: &str, host: &str, tag: &str, text: &str) -> Vec<u8> {
    format!("<{}>{} {} {}: {}", pri, ts, host, tag, text).into_bytes()
}

/// a message as IOS sends it to a "logging host": counter, time ("*" in front while the clock
/// is not synchronised), text ("%FAC-SEV-MNEMONIC: ...")
pub fn ios(pri: u8, seq: u32, ts: &str, text: &str) -> Vec<u8> {
    format!("<{}>{}: {}: {}", pri, seq, ts, text).into_bytes()
}

/// a received message
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Msg {
    pub pri: u8,
    /// everything after "<PRI>"
    pub rest: String,
    /// RFC 3164: "Oct 19 12:00:05"
    pub time: Option<String>,
    pub host: Option<String>,
    pub tag: Option<String>,
    /// the text after the tag (or the rest)
    pub text: String,
}

fn bsd_header(r: &str) -> Option<(String, String, String, String)> {
    // "Mmm dd hh:mm:ss host tag: text"
    let b = r.as_bytes();
    if b.len() < 16 || !MONTHS.iter().any(|m| r.starts_with(m)) || b[3] != b' ' || b[6] != b' ' || b[9] != b':' || b[12] != b':' || b[15] != b' ' {
        return None;
    }
    let time = String::from(&r[..15]);
    let after = &r[16..];
    let (host, rest) = after.split_once(' ')?;
    let (tag, text) = rest.split_once(": ")?;
    if tag.is_empty() || tag.contains(' ') {
        return None;
    }
    Some((time, String::from(host), String::from(tag), String::from(text)))
}

pub fn parse(b: &[u8]) -> Option<Msg> {
    let s = core::str::from_utf8(b).ok()?;
    let s = s.trim_end_matches(['\n', '\r', '\0']);
    let r = s.strip_prefix('<')?;
    let (p, rest) = r.split_once('>')?;
    if p.is_empty() || p.len() > 3 || !p.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let pri: u32 = p.parse().ok()?;
    if pri > 191 {
        return None;
    }
    let rest = String::from(rest);
    let (time, host, tag, text) = match bsd_header(&rest) {
        Some((t, h, g, x)) => (Some(t), Some(h), Some(g), x),
        None => (None, None, None, rest.clone()),
    };
    Some(Msg { pri: pri as u8, rest, time, host, tag, text })
}
