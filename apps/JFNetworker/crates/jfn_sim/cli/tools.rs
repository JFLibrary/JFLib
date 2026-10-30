// tools -- the network tools of the consoles: on the Linux shell curl (HTTP with the options
// used most, its output and messages as curl 8 prints them on a terminal), nc (TCP and UDP
// client, -l server, -z scan; OpenBSD netcat's messages), ss (TCP and UDP sockets with states,
// timers and TCP internals in iproute2's layout), top (a slim overview of the services: state,
// the CPU time of the web servers and the scripts, load average), printf and pipes into nc
// ("printf 'GET / HTTP/1.0\r\n\r\n' | nc web 80"); on the IOS-like console "show tcp brief
// [all]", "show tcp statistics" and "show control-plane host open-ports"
//
// curl and nc are console programs: services the shell starts (OsApi::print, exit,
// SvcEvent::Console); the console is busy while they run, typed lines go to nc, Ctrl+C ends
// them.  top is a console job that draws itself again every few seconds.  The console is a
// terminal: curl shows no progress meter while the body goes to it (only with -o), no "[n bytes
// data]" lines, and the times are virtual.  Help and version texts are our own words; ss lays
// out its columns as iproute2 does when its output is not a terminal (no padding to a screen
// width); a device counts as four CPUs for top.
//
// Both families: curl and nc take IPv6 addresses ("http://[2001:db8::1]/", "nc 2001:db8::1 80")
// and names (A and AAAA in the order of resolver.rs, the next address when the first does not
// connect), -4 / -6 keep them to one family; nc -l listens on 0.0.0.0, on :: with -6 or on the
// address given, and its UDP socket is connected to the peer (an ICMP port unreachable ends it).
// ss shows IPv6 addresses in brackets and a socket on :: as "*" where the device runs IPv6 (as
// 0.0.0.0 elsewhere), -4 / -6 pick a family; show tcp brief writes IPv6 in capitals.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use netpkt::http::{Head, Headers, Version};
use netpkt::IpAddr;

use super::print::JobOut;
use super::print6::up6;
use crate::http::{Agent, AgentEv, Done, Fail, Req, Url, UrlErr};
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{ConsoleEv, Metric, Owner, OsApi, Service, SvcEvent, ANY, ANY4};
use crate::resolver::Family;
use crate::tcp::{SockOpts, TcpEnd, TcpRow, TcpState, TcpTimer};

/// "addr:port" with an IPv6 address in brackets (curl's "Trying", nc, the TCP client)
fn ep(a: IpAddr, p: u16) -> String {
    if a.is_v4() { format!("{}:{}", a, p) } else { format!("[{}]:{}", a, p) }
}

/// the timer keys of a console program: its own, the HTTP agent's
const K_OWN: u32 = 0;
const K_HTTP: u32 = 1;

// ---------------------------------------------------------------- words, echo, printf, pipes

/// the byte index of a '|' outside quotes
fn pipe_at(line: &str) -> Option<usize> {
    let mut q: Option<char> = None;
    let mut esc = false;
    for (i, c) in line.char_indices() {
        if esc {
            esc = false;
            continue;
        }
        match q {
            Some(x) if c == x => q = None,
            Some('"') if c == '\\' => esc = true,
            Some(_) => {}
            None if c == '\\' => esc = true,
            None if c == '\'' || c == '"' => q = Some(c),
            None if c == '|' => return Some(i),
            None => {}
        }
    }
    None
}

/// "left | right" (one pipe)
pub(crate) fn split_pipe(line: &str) -> Option<(&str, &str)> {
    let i = pipe_at(line)?;
    Some((line[..i].trim(), line[i + 1..].trim()))
}

/// the words of a shell line: '...' as it is, "..." and bare words with \ escapes for " \ $
fn sh_words(line: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut cur = String::new();
    let mut any = false;
    let mut q: Option<char> = None;
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        match q {
            Some('\'') if c == '\'' => q = None,
            Some('\'') => cur.push(c),
            Some(_) if c == '"' => q = None,
            Some(_) if c == '\\' => match it.peek() {
                Some(&n) if n == '"' || n == '\\' || n == '$' => {
                    cur.push(n);
                    it.next();
                }
                _ => cur.push('\\'),
            },
            Some(_) => cur.push(c),
            None if c == '\'' || c == '"' => {
                q = Some(c);
                any = true;
            }
            None if c == '\\' => {
                if let Some(n) = it.next() {
                    cur.push(n);
                }
            }
            None if c.is_whitespace() => {
                if any || !cur.is_empty() {
                    v.push(core::mem::take(&mut cur));
                }
                any = false;
            }
            None => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        v.push(cur);
    }
    v
}

/// \n \r \t \\ \a \0NNN \xHH in a printf format or echo -e; `stop` is set by \c
fn unescape(s: &str, out: &mut Vec<u8>, stop: &mut bool) {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 == b.len() {
            out.push(b[i]);
            i += 1;
            continue;
        }
        i += 1;
        match b[i] {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'a' => out.push(7),
            b'\\' => out.push(b'\\'),
            b'c' => {
                *stop = true;
                return;
            }
            b'x' => {
                let mut v = 0u8;
                let mut n = 0;
                while n < 2 && i + 1 < b.len() && b[i + 1].is_ascii_hexdigit() {
                    i += 1;
                    v = v * 16 + (b[i] as char).to_digit(16).unwrap_or(0) as u8;
                    n += 1;
                }
                if n == 0 {
                    out.extend_from_slice(b"\\x");
                } else {
                    out.push(v);
                }
            }
            b'0' => {
                let mut v = 0u32;
                let mut n = 0;
                while n < 3 && i + 1 < b.len() && (b'0'..=b'7').contains(&b[i + 1]) {
                    i += 1;
                    v = v * 8 + (b[i] - b'0') as u32;
                    n += 1;
                }
                out.push(v as u8);
            }
            c => {
                out.push(b'\\');
                out.push(c);
            }
        }
        i += 1;
    }
}

/// what echo writes: -n (no newline), -e (escapes), -E
fn echo_bytes(args: &[String]) -> Vec<u8> {
    let (mut nl, mut esc) = (true, false);
    let mut k = 0;
    while let Some(a) = args.get(k) {
        if a.len() > 1 && a.starts_with('-') && a[1..].bytes().all(|c| matches!(c, b'n' | b'e' | b'E')) {
            for c in a[1..].bytes() {
                match c {
                    b'n' => nl = false,
                    b'e' => esc = true,
                    _ => esc = false,
                }
            }
            k += 1;
        } else {
            break;
        }
    }
    let text = args[k..].join(" ");
    let mut out = Vec::new();
    let mut stop = false;
    if esc {
        unescape(&text, &mut out, &mut stop);
    } else {
        out.extend_from_slice(text.as_bytes());
    }
    if nl && !stop {
        out.push(b'\n');
    }
    out
}

/// what printf writes: the format with escapes and %s %d %c %x %%, reused while arguments are
/// left (as the shell's printf)
fn printf_bytes(args: &[String]) -> Result<Vec<u8>, String> {
    let Some(fmt) = args.first() else {
        return Err(String::from("printf: usage: printf FORMAT [ARGUMENT]...\n"));
    };
    let rest = &args[1..];
    let mut out = Vec::new();
    let mut used = 0;
    loop {
        let b = fmt.as_bytes();
        let mut i = 0;
        let mut lit = String::new();
        let mut stop = false;
        let mut took = false;
        while i < b.len() {
            if b[i] == b'%' && i + 1 < b.len() {
                unescape(&core::mem::take(&mut lit), &mut out, &mut stop);
                if stop {
                    return Ok(out);
                }
                i += 1;
                let arg = rest.get(used).cloned().unwrap_or_default();
                match b[i] {
                    b'%' => out.push(b'%'),
                    b's' | b'b' => {
                        took = true;
                        used += 1;
                        if b[i] == b'b' {
                            unescape(&arg, &mut out, &mut stop);
                        } else {
                            out.extend_from_slice(arg.as_bytes());
                        }
                    }
                    b'd' | b'i' | b'u' => {
                        took = true;
                        used += 1;
                        let n: i64 = if arg.is_empty() { 0 } else { arg.trim().parse().map_err(|_| format!("printf: {}: invalid number\n", arg))? };
                        out.extend_from_slice(format!("{}", n).as_bytes());
                    }
                    b'x' => {
                        took = true;
                        used += 1;
                        let n: u64 = if arg.is_empty() { 0 } else { arg.trim().parse().map_err(|_| format!("printf: {}: invalid number\n", arg))? };
                        out.extend_from_slice(format!("{:x}", n).as_bytes());
                    }
                    b'c' => {
                        took = true;
                        used += 1;
                        if let Some(c) = arg.bytes().next() {
                            out.push(c);
                        }
                    }
                    c => {
                        out.push(b'%');
                        out.push(c);
                    }
                }
                i += 1;
                continue;
            }
            lit.push(b[i] as char);
            i += 1;
        }
        unescape(&lit, &mut out, &mut stop);
        if stop || !took || used >= rest.len() {
            break;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- service names

/// the name /etc/services gives a port (ss without -n, nc -v)
fn service_name(port: u16, udp: bool) -> Option<&'static str> {
    Some(match port {
        7 => "echo",
        9 => "discard",
        13 => "daytime",
        21 => "ftp",
        22 => "ssh",
        23 => "telnet",
        25 => "smtp",
        53 => "domain",
        67 => "bootps",
        68 => "bootpc",
        69 if udp => "tftp",
        80 => "http",
        110 => "pop3",
        123 => "ntp",
        143 => "imap2",
        161 if udp => "snmp",
        443 => "https",
        514 if udp => "syslog",
        514 => "shell",
        3306 => "mysql",
        5432 => "postgresql",
        8080 => "http-alt",
        _ => return None,
    })
}

/// a port number from text: a number or a name of /etc/services
fn port_of(s: &str) -> Option<u16> {
    if let Ok(n) = s.parse::<u16>() {
        return Some(n);
    }
    (1..=9000u16).find(|&p| service_name(p, false) == Some(s) || service_name(p, true) == Some(s))
}

// ---------------------------------------------------------------- curl

/// the options of a curl command line
#[derive(Clone, Debug, Default)]
struct CurlOpts {
    urls: Vec<String>,
    verbose: bool,
    silent: bool,
    show_error: bool,
    include: bool,
    head: bool,
    method: Option<String>,
    data: Option<Vec<u8>>,
    headers: Vec<(String, String)>,
    agent: Option<String>,
    output: Option<String>,
    write_out: Option<String>,
    max_time: Ns,
    connect_timeout: Ns,
    location: bool,
    max_redirs: u32,
    fail: bool,
    http10: bool,
    dump_header: Option<String>,
    no_meter: bool,
    /// -4 / -6: the address family to look names up for
    family: Option<Family>,
}

const CURL_HELP: &str = concat!(
    "Usage: curl [options] URL...\n",
    " -d, --data DATA           send DATA in a POST request\n",
    " -f, --fail                treat answers from 400 on as errors (exit 22)\n",
    " -H, --header 'NAME: VALUE'  add or replace a request header\n",
    " -i, --include             write the response head before the body\n",
    " -I, --head                send HEAD and write only the response head\n",
    " -L, --location            follow redirects (--max-redirs N)\n",
    " -m, --max-time SECONDS    give up after this long\n",
    "     --connect-timeout S   give up connecting after this long\n",
    " -o, --output FILE         write the body there instead (shows the progress meter)\n",
    " -s, --silent              no progress meter, no error messages (-S shows errors)\n",
    " -v, --verbose             show the connection, the request and the response head\n",
    " -w, --write-out FORMAT    write values such as %{http_code} after the transfer\n",
    " -X, --request METHOD      use another method\n",
    " -A, --user-agent NAME     the User-Agent header\n",
    " -D, --dump-header -       write the response head to the console\n",
    " -0, --http1.0             speak HTTP/1.0\n",
    " -4, --ipv4                look names up for IPv4 addresses only\n",
    " -6, --ipv6                look names up for IPv6 addresses only\n",
    " -V, --version             say which curl this is\n",
    "Only http:// URLs work here (an IPv6 address in brackets: http://[2001:db8::1]/); the\n",
    "transfer runs in the simulator's virtual time.\n",
);

const CURL_VERSION: &str = "curl 8.5.0 (JFNetworker): HTTP/1.1 over the simulator's TCP\nProtocols: http\nFeatures: (none)\n";

/// the value of a time option ("1.5" seconds)
fn secs_of(s: &str) -> Option<Ns> {
    let (i, f) = s.split_once('.').unwrap_or((s, ""));
    if (i.is_empty() && f.is_empty()) || !i.bytes().all(|b| b.is_ascii_digit()) || !f.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut ns = if i.is_empty() { 0 } else { i.parse::<u64>().ok()?.checked_mul(SEC)? };
    let f = &f[..f.len().min(9)];
    if !f.is_empty() {
        ns += f.parse::<u64>().ok()? * 10u64.pow(9 - f.len() as u32);
    }
    Some(ns)
}

/// parses a curl command line; Err: what to print and the exit status
fn parse_curl(args: &[String]) -> Result<CurlOpts, (String, i32)> {
    let hint = "curl: try 'curl --help' for more information\n";
    let mut o = CurlOpts { max_redirs: 50, ..CurlOpts::default() };
    let mut k = 0;
    // options that take a value
    let takes = |c: char| "XdHAowmD".contains(c);
    let set = |o: &mut CurlOpts, name: &str, val: Option<&str>| -> Result<(), (String, i32)> {
        let v = || val.unwrap_or("");
        match name {
            "v" | "verbose" => o.verbose = true,
            "s" | "silent" => o.silent = true,
            "S" | "show-error" => o.show_error = true,
            "i" | "include" => o.include = true,
            "I" | "head" => o.head = true,
            "L" | "location" => o.location = true,
            "f" | "fail" => o.fail = true,
            "0" | "http1.0" => o.http10 = true,
            "http1.1" => o.http10 = false,
            "4" | "ipv4" => o.family = Some(Family::V4),
            "6" | "ipv6" => o.family = Some(Family::V6),
            "k" | "insecure" | "compressed" | "no-buffer" | "N" => {}
            "no-progress-meter" => o.no_meter = true,
            "X" | "request" => o.method = Some(String::from(v())),
            "d" | "data" | "data-raw" | "data-binary" | "data-ascii" => {
                let d = o.data.get_or_insert_with(Vec::new);
                if !d.is_empty() {
                    d.push(b'&');
                }
                d.extend_from_slice(v().as_bytes());
            }
            "H" | "header" => {
                let h = v();
                match h.split_once(':') {
                    Some((n, val)) => o.headers.push((String::from(n.trim()), String::from(val.trim()))),
                    None => match h.strip_suffix(';') {
                        Some(n) => o.headers.push((String::from(n.trim()), String::new())),
                        None => {}
                    },
                }
            }
            "A" | "user-agent" => o.agent = Some(String::from(v())),
            "o" | "output" => o.output = Some(String::from(v())),
            "w" | "write-out" => o.write_out = Some(String::from(v())),
            "D" | "dump-header" => o.dump_header = Some(String::from(v())),
            "m" | "max-time" | "connect-timeout" | "max-redirs" => {
                let bad = || (format!("curl: option {}{}: expected a proper numerical parameter\n{}", if name.len() == 1 { "-" } else { "--" }, name, hint), 2);
                if name == "max-redirs" {
                    o.max_redirs = v().parse().map_err(|_| bad())?;
                } else {
                    let t = secs_of(v()).ok_or_else(bad)?;
                    if name == "connect-timeout" {
                        o.connect_timeout = t;
                    } else {
                        o.max_time = t;
                    }
                }
            }
            "h" | "help" => return Err((String::from(CURL_HELP), 0)),
            "V" | "version" => return Err((String::from(CURL_VERSION), 0)),
            _ => return Err((format!("curl: option {}{}: is unknown\n{}", if name.len() == 1 { "-" } else { "--" }, name, hint), 2)),
        }
        Ok(())
    };
    let long_takes = |n: &str| matches!(n, "request" | "data" | "data-raw" | "data-binary" | "data-ascii" | "header" | "user-agent" | "output" | "write-out" | "max-time" | "connect-timeout" | "max-redirs" | "dump-header");
    while k < args.len() {
        let a = &args[k];
        if let Some(name) = a.strip_prefix("--") {
            if long_takes(name) {
                k += 1;
                let Some(v) = args.get(k) else {
                    return Err((format!("curl: option --{}: requires parameter\n{}", name, hint), 2));
                };
                set(&mut o, name, Some(v))?;
            } else {
                set(&mut o, name, None)?;
            }
        } else if a.len() > 1 && a.starts_with('-') {
            let cs: Vec<char> = a[1..].chars().collect();
            let mut j = 0;
            while j < cs.len() {
                let c = cs[j];
                let name = String::from(c);
                if takes(c) {
                    let rest: String = cs[j + 1..].iter().collect();
                    let val = if !rest.is_empty() {
                        rest
                    } else {
                        k += 1;
                        match args.get(k) {
                            Some(v) => v.clone(),
                            None => return Err((format!("curl: option -{}: requires parameter\n{}", c, hint), 2)),
                        }
                    };
                    set(&mut o, &name, Some(&val))?;
                    break;
                }
                set(&mut o, &name, None)?;
                j += 1;
            }
        } else {
            o.urls.push(a.clone());
        }
        k += 1;
    }
    if o.urls.is_empty() {
        return Err((String::from(hint), 2));
    }
    Ok(o)
}

/// curl's five-character amounts ("  195k", " 153M")
fn max5(n: u64) -> String {
    const K: u64 = 1024;
    const M: u64 = 1024 * 1024;
    if n < 100_000 {
        format!("{:5}", n)
    } else if n < 10_000 * K {
        format!("{:4}k", n / K)
    } else if n < 100 * M {
        format!("{:2}.{}M", n / M, (n % M) / (M / 10))
    } else if n < 10_000 * M {
        format!("{:4}M", n / M)
    } else {
        format!("{:4}G", n / (1024 * M))
    }
}

/// "--:--:--" or " 0:00:01"
fn time2str(secs: u64) -> String {
    if secs == 0 {
        return String::from("--:--:--");
    }
    let h = secs / 3600;
    if h <= 99 {
        format!("{:2}:{:02}:{:02}", h, (secs % 3600) / 60, secs % 60)
    } else {
        format!("{:3}d {:02}h", h / 24, h % 24)
    }
}

/// bytes per second over `us` microseconds, as curl reckons it
fn trspeed(bytes: u64, us: u64) -> u64 {
    if us == 0 { bytes.saturating_mul(1_000_000) } else { ((bytes as u128 * 1_000_000) / us as u128) as u64 }
}

/// a progress meter line (without the leading CR)
fn meter_line(expect: Option<u64>, dl: u64, ul: u64, ul_total: u64, us: u64, done: bool) -> String {
    let dlspeed = trspeed(dl, us);
    let ulspeed = trspeed(ul, us);
    let cur = dl.saturating_mul(1000) / (us / 1000).max(1);
    let (total, tpct, dpct) = match expect {
        Some(t) => {
            let tot = t + ul_total;
            let pct = if tot > 0 { (dl + ul) * 100 / tot } else { 0 };
            (tot, pct, if t > 0 { dl * 100 / t } else { 0 })
        }
        None if done => (dl + ul_total, 100, 0),
        None => (ul_total, 0, 0),
    };
    let upct = if ul_total > 0 { ul * 100 / ul_total } else { 0 };
    let spent = us / 1_000_000;
    let total_secs = match expect {
        Some(t) if dlspeed > 0 => t / dlspeed,
        _ => 0,
    };
    let left = total_secs.saturating_sub(spent);
    format!("{:3} {}  {:3} {}  {:3} {}  {}  {} {} {} {} {}", tpct, max5(total), dpct, max5(dl), upct, max5(ul), max5(dlspeed), max5(ulspeed), time2str(total_secs), time2str(spent), time2str(left), max5(cur))
}

const METER_HEAD: &str = "  % Total    % Received % Xferd  Average Speed   Time    Time     Time  Current\n                                 Dload  Upload   Total   Spent    Left  Speed\n";

/// a transfer in progress
#[derive(Clone, Debug)]
struct Transfer {
    id: u64,
    url: Url,
    /// the URL as typed (%{url})
    given: String,
    start: Ns,
    meter: bool,
    body: u64,
    expect: Option<u64>,
    head: Option<Head>,
    /// -f and an error status: the body is not written
    skip_body: bool,
    /// what the agent said about the connection after the answer
    after: Option<String>,
    redirects: u32,
}

/// curl on the console
#[derive(Clone, Debug)]
pub struct Curl {
    o: CurlOpts,
    agent: Agent,
    queue: VecDeque<String>,
    cur: Option<Transfer>,
    exit: i32,
    /// connections opened by this curl (%{num_connects} of the transfer)
    conns_before: u64,
}

impl Curl {
    fn new(o: CurlOpts) -> Curl {
        let mut agent = Agent::new(K_HTTP);
        agent.trace = true;
        agent.family = o.family.unwrap_or(Family::Any);
        let queue = o.urls.iter().cloned().collect();
        Curl { o, agent, queue, cur: None, exit: 0, conns_before: 0 }
    }
    fn errors_shown(&self) -> bool {
        !self.o.silent || self.o.show_error
    }
    fn meter_shown(&self) -> bool {
        self.o.output.as_deref().is_some_and(|f| f != "-") && !self.o.silent && !self.o.no_meter
    }
    /// the next URL, or the end
    fn next(&mut self, os: &mut OsApi) {
        while let Some(u) = self.queue.pop_front() {
            match Url::parse(&u) {
                Ok(url) => {
                    self.begin(os, url, u, 0);
                    return;
                }
                Err(e) => {
                    let (code, msg) = match e {
                        UrlErr::Scheme(s) => (1, format!("Protocol \"{}\" not supported or disabled in libcurl", s)),
                        UrlErr::NoHost => (3, String::from("URL rejected: No host part in the URL")),
                        UrlErr::Port => (3, String::from("URL rejected: Port number was not a decimal number between 0 and 65535")),
                        UrlErr::BadHost => (3, String::from("URL rejected: Bad hostname")),
                    };
                    self.exit = code;
                    if self.errors_shown() {
                        os.print(&format!("curl: ({}) {}\n", code, msg));
                    }
                }
            }
        }
        os.exit(self.exit);
    }
    fn begin(&mut self, os: &mut OsApi, url: Url, given: String, redirects: u32) {
        let o = &self.o;
        let method = if o.head {
            String::from("HEAD")
        } else if let Some(m) = &o.method {
            m.clone()
        } else if o.data.is_some() {
            String::from("POST")
        } else {
            String::from("GET")
        };
        let mut h = Headers::new();
        let ua = o.agent.clone().unwrap_or_else(|| String::from("curl/8.5.0"));
        for (k, v) in [("Host", url.host_header()), ("User-Agent", ua), ("Accept", String::from("*/*"))] {
            match o.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)) {
                Some((n, val)) if !val.is_empty() => h.add(n, val),
                Some(_) => {}
                None => h.add(k, &v),
            }
        }
        let body = o.data.clone().unwrap_or_default();
        for (n, v) in &o.headers {
            if v.is_empty() || ["host", "user-agent", "accept", "content-type"].iter().any(|x| n.eq_ignore_ascii_case(x)) {
                continue;
            }
            h.add(n, v);
        }
        if o.data.is_some() {
            h.add("Content-Length", &format!("{}", body.len()));
            match o.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case("content-type")) {
                Some((n, v)) if !v.is_empty() => h.add(n, v),
                Some(_) => {}
                None => h.add("Content-Type", "application/x-www-form-urlencoded"),
            }
        } else if let Some((n, v)) = o.headers.iter().find(|(n, v)| n.eq_ignore_ascii_case("content-type") && !v.is_empty()) {
            h.add(n, v);
        }
        let req = Req {
            method,
            url: url.clone(),
            version: if o.http10 { Version::Http10 } else { Version::Http11 },
            headers: h,
            body,
            timeout: o.max_time,
            connect_timeout: o.connect_timeout,
            reuse: true,
            keep_body: false,
        };
        let meter = self.meter_shown();
        let now = os.now();
        if meter {
            os.print(METER_HEAD);
            os.print(&format!("\r{}", meter_line(None, 0, 0, 0, 0, false)));
            os.timer(K_OWN, now + SEC);
        }
        self.conns_before = self.agent.stats.connects;
        let id = self.agent.submit(os, req);
        self.cur = Some(Transfer { id, url, given, start: now, meter, body: 0, expect: None, head: None, skip_body: false, after: None, redirects });
        self.show(os);
    }
    fn v(&self, os: &mut OsApi, s: &str) {
        if self.o.verbose {
            os.print(s);
        }
    }
    /// what the agent did since the last call
    fn show(&mut self, os: &mut OsApi) {
        for e in self.agent.take_events() {
            let host = self.cur.as_ref().map(|t| t.url.host.clone()).unwrap_or_default();
            match e {
                AgentEv::Trying { addr, port, .. } => self.v(os, &format!("*   Trying {}...\n", ep(addr, port))),
                AgentEv::Connected { remote, .. } => self.v(os, &format!("* Connected to {} ({}) port {}\n", host, remote.0, remote.1)),
                AgentEv::Reusing { .. } => self.v(os, &format!("* Re-using existing connection with host {}\n", host)),
                AgentEv::ConnectFailed { addr, port, local, end, .. } => {
                    if let (Some(l), Some(e)) = (local, end.and_then(|e| e.errno())) {
                        self.v(os, &format!("* connect to {} port {} from {} port {} failed: {}\n", addr, port, l.0, l.1, e));
                    }
                }
                AgentEv::Sent { head, .. } => {
                    if self.o.verbose {
                        let text = String::from_utf8_lossy(&head).into_owned();
                        let mut s = String::new();
                        for l in text.split("\r\n").take(text.split("\r\n").count().saturating_sub(1)) {
                            s.push_str(&format!("> {}\r\n", l));
                        }
                        os.print(&s);
                    }
                }
                AgentEv::Interim { head, .. } => {
                    if self.o.verbose {
                        let mut s = format!("< {}\r\n", head.start_line());
                        for (k, v) in &head.headers.0 {
                            s.push_str(&format!("< {}: {}\r\n", k, v));
                        }
                        s.push_str("< \r\n");
                        os.print(&s);
                    }
                }
                AgentEv::Head { head, .. } => self.on_head(os, head),
                AgentEv::Data { data, .. } => {
                    let to_console = self.o.output.as_deref().is_none_or(|f| f == "-");
                    if let Some(t) = self.cur.as_mut() {
                        t.body += data.len() as u64;
                        if to_console && !t.skip_body && !self.o.head {
                            os.print(&String::from_utf8_lossy(&data));
                        }
                    }
                }
                AgentEv::Kept { conn, .. } => {
                    if let Some(t) = self.cur.as_mut() {
                        t.after = Some(format!("* Connection #{} to host {} left intact\n", conn, host));
                    }
                }
                AgentEv::Closing { .. } => {
                    if let Some(t) = self.cur.as_mut() {
                        t.after = Some(String::from("* Closing connection\n"));
                    }
                }
                AgentEv::Done(d) => {
                    self.done(os, d);
                    if self.cur.is_none() {
                        return;
                    }
                }
            }
        }
    }
    fn on_head(&mut self, os: &mut OsApi, head: Head) {
        let mut s = String::new();
        if head.version == Version::Http10 && self.o.verbose {
            s.push_str("* HTTP 1.0, assume close after body\n");
        }
        let mut text = format!("{}\r\n", head.start_line());
        for (k, v) in &head.headers.0 {
            text.push_str(&format!("{}: {}\r\n", k, v));
        }
        text.push_str("\r\n");
        if self.o.verbose {
            for l in text.split("\r\n").take(text.split("\r\n").count().saturating_sub(1)) {
                s.push_str(&format!("< {}\r\n", l));
            }
        }
        if self.o.include || self.o.head || self.o.dump_header.as_deref() == Some("-") {
            s.push_str(&text);
        }
        os.print(&s);
        if let Some(t) = self.cur.as_mut() {
            t.expect = head.headers.content_length().ok().flatten();
            if self.o.fail && head.status >= 400 {
                t.skip_body = true;
            }
            t.head = Some(head);
        }
    }
    /// the end of a transfer: meter, messages, write-out, redirect, next URL
    fn done(&mut self, os: &mut OsApi, d: Done) {
        let Some(t) = self.cur.take() else {
            return;
        };
        if t.id != d.id {
            // not the transfer of now
            self.cur = Some(t);
            return;
        }
        os.cancel(K_OWN);
        let ms = (d.end - d.start) / MS;
        let host = t.url.host.clone();
        let port = t.url.port;
        let ul_total = d.req_len.saturating_sub(d.req_head);
        let us = (d.end - d.start) / 1000;
        let meter_end = |os: &mut OsApi, ok: bool| {
            if t.meter {
                let ul = if ok || d.sent.is_some() { ul_total } else { 0 };
                os.print(&format!("\r{}\n", meter_line(t.expect, t.body, ul, ul_total, us, true)));
            }
        };
        let (code, msg) = match &d.result {
            Ok(r) => {
                let st = r.head.status;
                if self.o.fail && st >= 400 {
                    meter_end(os, true);
                    self.v(os, &format!("* The requested URL returned error: {}\n* Closing connection\n", st));
                    (22, format!("The requested URL returned error: {}", st))
                } else {
                    meter_end(os, true);
                    if let Some(a) = &t.after {
                        self.v(os, a);
                    }
                    (0, String::new())
                }
            }
            Err(f) => {
                let phase = if d.resolved.is_none() { Phase::Resolving } else if d.connected.is_none() { Phase::Connecting } else { Phase::Transfer };
                let (code, msg) = curl_error(f, phase, &host, port, ms);
                self.v(os, &format!("* {}\n", msg));
                meter_end(os, false);
                self.v(os, "* Closing connection\n");
                (code, msg)
            }
        };
        if let Some(w) = self.o.write_out.clone() {
            let text = self.write_out(os, &w, &t, &d, code, &msg);
            os.print(&text);
        }
        if code != 0 && self.errors_shown() {
            os.print(&format!("curl: ({}) {}\n", code, msg));
        }
        self.exit = code;
        // a redirect to follow
        if code == 0
            && self.o.location
            && let Ok(r) = &d.result
            && matches!(r.head.status, 301 | 302 | 303 | 307 | 308)
            && let Some(loc) = r.head.header("location")
            && let Some(next) = t.url.join(loc)
        {
            if t.redirects >= self.o.max_redirs {
                self.exit = 47;
                if self.errors_shown() {
                    os.print(&format!("curl: (47) Maximum ({}) redirects followed\n", self.o.max_redirs));
                }
                os.exit(47);
                return;
            }
            self.v(os, &format!("* Issue another request to this URL: '{}'\n", next.text()));
            let given = next.text();
            self.begin(os, next, given, t.redirects + 1);
            return;
        }
        self.next(os);
    }
    /// -w: %{variables} and \n escapes
    fn write_out(&self, os: &OsApi, w: &str, t: &Transfer, d: &Done, code: i32, msg: &str) -> String {
        let secs = |x: Option<Ns>| match x {
            Some(v) => {
                let us = v.saturating_sub(d.start) / 1000;
                format!("{}.{:06}", us / 1_000_000, us % 1_000_000)
            }
            None => String::from("0.000000"),
        };
        let head = d.result.as_ref().ok().map(|r| &r.head);
        let status = head.map(|h| h.status).unwrap_or(0);
        let total_us = (d.end - d.start) / 1000;
        let mut out = String::new();
        let mut it = w.chars().peekable();
        while let Some(c) = it.next() {
            match c {
                '\\' => match it.next() {
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some('\\') => out.push('\\'),
                    Some(x) => {
                        out.push('\\');
                        out.push(x);
                    }
                    None => out.push('\\'),
                },
                '%' if it.peek() == Some(&'{') => {
                    it.next();
                    let name: String = it.by_ref().take_while(|&x| x != '}').collect();
                    let v = match name.as_str() {
                        "http_code" | "response_code" => format!("{:03}", status),
                        "size_download" => format!("{}", d.body_len),
                        "size_header" => format!("{}", d.head_len),
                        "size_request" => format!("{}", d.req_len),
                        "size_upload" => format!("{}", d.req_len.saturating_sub(d.req_head)),
                        "num_connects" => format!("{}", if d.reused || d.connected.is_none() { 0 } else { 1 }),
                        "num_redirects" => format!("{}", t.redirects),
                        "remote_ip" => d.remote.filter(|_| d.connected.is_some()).map(|r| format!("{}", r.0)).unwrap_or_default(),
                        "remote_port" => d.remote.filter(|_| d.connected.is_some()).map(|r| format!("{}", r.1)).unwrap_or_default(),
                        "local_ip" => d.local.filter(|_| d.connected.is_some()).map(|r| format!("{}", r.0)).unwrap_or_default(),
                        "local_port" => d.local.filter(|_| d.connected.is_some()).map(|r| format!("{}", r.1)).unwrap_or_default(),
                        "http_version" => match head.map(|h| h.version) {
                            Some(Version::Http10) => String::from("1.0"),
                            Some(Version::Http11) => String::from("1.1"),
                            None => String::from("0"),
                        },
                        "content_type" => head.and_then(|h| h.header("content-type")).map(String::from).unwrap_or_default(),
                        "exitcode" => format!("{}", code),
                        "errormsg" => String::from(msg),
                        "url_effective" => t.url.text(),
                        "url" => t.given.clone(),
                        "redirect_url" => head.filter(|h| (300..400).contains(&h.status)).and_then(|h| h.header("location")).and_then(|l| t.url.join(l)).map(|u| u.text()).unwrap_or_default(),
                        "method" => d.method.clone(),
                        "scheme" => String::from("HTTP"),
                        "time_namelookup" => secs(d.resolved),
                        "time_connect" => secs(d.connected),
                        "time_appconnect" | "time_redirect" => String::from("0.000000"),
                        "time_pretransfer" => secs(d.sent),
                        "time_starttransfer" => secs(d.first),
                        "time_total" => secs(Some(d.end)),
                        "speed_download" => format!("{}", trspeed(d.body_len, total_us)),
                        "speed_upload" => format!("{}", trspeed(d.req_len.saturating_sub(d.req_head), total_us)),
                        _ => {
                            let _ = os;
                            out.push_str(&format!("curl: unknown --write-out variable: '{}'\n", name));
                            continue;
                        }
                    };
                    out.push_str(&v);
                }
                '%' if it.peek() == Some(&'%') => {
                    it.next();
                    out.push('%');
                }
                c => out.push(c),
            }
        }
        out
    }
}

/// how far a transfer got when it failed
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Resolving,
    Connecting,
    Transfer,
}

/// curl's exit status and message for a failure (-m that runs out before the connection is up
/// names the step it stopped in, as curl's own check of the whole time does)
fn curl_error(f: &Fail, phase: Phase, host: &str, port: u16, ms: u64) -> (i32, String) {
    match f {
        Fail::Resolve(_) => (6, format!("Could not resolve host: {}", host)),
        Fail::NoRoute(_) | Fail::Connect(_) => (7, format!("Failed to connect to {} port {} after {} ms: Couldn't connect to server", host, port, ms)),
        Fail::ConnectTimeout => (28, format!("Failed to connect to {} port {} after {} ms: Timeout was reached", host, port, ms)),
        Fail::Timeout { .. } if phase == Phase::Resolving => (28, format!("Resolving timed out after {} milliseconds", ms)),
        Fail::Timeout { .. } if phase == Phase::Connecting => (28, format!("Connection timed out after {} milliseconds", ms)),
        Fail::Timeout { got, of: Some(n) } => (28, format!("Operation timed out after {} milliseconds with {} out of {} bytes received", ms, got, n)),
        Fail::Timeout { got, of: None } => (28, format!("Operation timed out after {} milliseconds with {} bytes received", ms, got)),
        Fail::Reset(_) | Fail::Aborted => (56, String::from("Recv failure: Connection reset by peer")),
        Fail::Empty => (52, String::from("Empty reply from server")),
        Fail::Truncated { got, of: Some(n) } => (18, format!("transfer closed with {} bytes remaining to read", n.saturating_sub(*got))),
        Fail::Truncated { of: None, .. } => (18, String::from("transfer closed with outstanding read data remaining")),
        Fail::Bad(e) if e.text().contains("HTTP/1") || e.text().contains("status") => (1, String::from("Received HTTP/0.9 when not allowed")),
        Fail::Bad(_) => (8, String::from("Weird server reply")),
    }
}

impl Service for Curl {
    fn kind(&self) -> &'static str {
        "curl"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.next(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if let SvcEvent::Timer(K_OWN) = ev {
            // the progress meter once a second
            if let Some(t) = &self.cur
                && t.meter
            {
                let us = (os.now() - t.start) / 1000;
                os.print(&format!("\r{}", meter_line(t.expect, t.body, 0, 0, us, false)));
                let at = os.now() + SEC;
                os.timer(K_OWN, at);
            }
            return;
        }
        if self.agent.on(&ev, os) {
            self.show(os);
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        Vec::new()
    }
    fn status(&self) -> String {
        String::from("curl")
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
}

// ---------------------------------------------------------------- nc

/// the options of an nc command line
#[derive(Clone, Debug, Default)]
struct NcOpts {
    listen: bool,
    udp: bool,
    verbose: bool,
    zero: bool,
    keep: bool,
    crlf: bool,
    /// -N: shut the connection down after the end of the input
    shutdown: bool,
    /// -q: quit this long after the end of the input
    quit: Option<Ns>,
    /// -w: give up connecting after this long
    wait: Option<Ns>,
    /// -d: read nothing from the console
    detach: bool,
    sport: u16,
    host: String,
    ports: Vec<u16>,
    /// -4 / -6 (a listener: IPv4 by default, :: for both with -6)
    family: Option<Family>,
}

const NC_HELP: &str = "nc: a network Swiss army knife of the simulator\n\
usage: nc [-46CdhklNnuvz] [-p source_port] [-q seconds] [-w timeout] [destination] [port]\n\
\t-4\t\tIPv4 only\n\
\t-6\t\tIPv6 only (with -l: listen on ::, which takes IPv4 too)\n\
\t-C\t\tsend CRLF as the line end\n\
\t-d\t\tdo not read from the console\n\
\t-h\t\tthis help\n\
\t-k\t\tlisten again after a connection ends (with -l)\n\
\t-l\t\tlisten for a connection instead of making one\n\
\t-N\t\tshut the connection down when the input ends\n\
\t-n\t\tno name lookups\n\
\t-p port\t\tconnect from this local port\n\
\t-q secs\t\tquit this long after the input ends\n\
\t-u\t\tUDP instead of TCP\n\
\t-v\t\tsay what happens\n\
\t-w secs\t\tgive up connecting after this long\n\
\t-z\t\tonly see whether ports accept a connection (ports may be lo-hi)\n\
Typed lines go to the other side; Ctrl+C ends nc.\n";

const NC_USAGE: &str = "usage: nc [-46CdhklNnuvz] [-p source_port] [-q seconds] [-w timeout] [destination] [port]\n";

fn parse_nc(args: &[String]) -> Result<NcOpts, (String, i32)> {
    let mut o = NcOpts::default();
    let mut pos: Vec<String> = Vec::new();
    let mut k = 0;
    while k < args.len() {
        let a = &args[k];
        if a.len() > 1 && a.starts_with('-') {
            let cs: Vec<char> = a[1..].chars().collect();
            let mut j = 0;
            while j < cs.len() {
                let c = cs[j];
                if "pqwsIOiTMmWVPXx".contains(c) {
                    let rest: String = cs[j + 1..].iter().collect();
                    let v = if !rest.is_empty() {
                        rest
                    } else {
                        k += 1;
                        args.get(k).cloned().ok_or_else(|| (format!("nc: option requires an argument -- '{}'\n{}", c, NC_USAGE), 1))?
                    };
                    match c {
                        'p' => o.sport = v.parse().map_err(|_| (format!("nc: port number invalid: {}\n", v), 1))?,
                        'q' => o.quit = Some(secs_of(&v).ok_or_else(|| (format!("nc: quit timer {}: invalid\n", v), 1))?),
                        'w' => o.wait = Some(secs_of(&v).ok_or_else(|| (format!("nc: timeout {}: invalid\n", v), 1))?),
                        _ => {}
                    }
                    break;
                }
                match c {
                    'l' => o.listen = true,
                    'u' => o.udp = true,
                    'v' => o.verbose = true,
                    'z' => o.zero = true,
                    'k' => o.keep = true,
                    'C' => o.crlf = true,
                    'N' => o.shutdown = true,
                    'd' => o.detach = true,
                    '4' => o.family = Some(Family::V4),
                    '6' => o.family = Some(Family::V6),
                    'n' | 'D' | 'r' | 'S' | 't' => {}
                    'h' => return Err((String::from(NC_HELP), 1)),
                    _ => return Err((format!("nc: invalid option -- '{}'\n{}", c, NC_USAGE), 1)),
                }
                j += 1;
            }
        } else {
            pos.push(a.clone());
        }
        k += 1;
    }
    let range = |s: &str| -> Result<Vec<u16>, (String, i32)> {
        let bad = || (format!("nc: port range not valid: {}\n", s), 1);
        match s.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (port_of(a).ok_or_else(bad)?, port_of(b).ok_or_else(bad)?);
                if a > b || a == 0 {
                    return Err(bad());
                }
                Ok((a..=b).collect())
            }
            None => {
                let p = port_of(s).ok_or_else(|| (format!("nc: port number invalid: {}\n", s), 1))?;
                Ok(alloc::vec![p])
            }
        }
    };
    if o.listen {
        // nc -l [addr] port, or nc -l -p port
        match pos.len() {
            0 if o.sport > 0 => o.ports = alloc::vec![o.sport],
            1 => o.ports = range(&pos[0])?,
            2 => {
                o.host = pos[0].clone();
                o.ports = range(&pos[1])?;
            }
            _ => return Err((String::from(NC_USAGE), 1)),
        }
        o.ports.truncate(1);
        return Ok(o);
    }
    if pos.len() < 2 {
        return Err((String::from(NC_USAGE), 1));
    }
    o.host = pos[0].clone();
    for p in &pos[1..] {
        o.ports.extend(range(p)?);
    }
    if !o.zero {
        o.ports.truncate(1);
    }
    Ok(o)
}

/// nc on the console
#[derive(Clone, Debug)]
pub struct Nc {
    o: NcOpts,
    addr: Option<IpAddr>,
    /// the name's other addresses, tried when the connection to one fails (an answer with
    /// both families), and all of them for the next port of a scan
    alts: Vec<IpAddr>,
    all: Vec<IpAddr>,
    lookup: Option<u32>,
    /// the port being tried (index into o.ports)
    at: usize,
    sock: Option<u32>,
    connected: bool,
    listener: Option<u32>,
    udp_port: Option<u16>,
    peer: Option<(IpAddr, u16)>,
    /// typed or piped bytes not sent yet, and whether the input ended
    pending: Vec<u8>,
    eof: bool,
    any_ok: bool,
}

impl Nc {
    fn new(o: NcOpts, input: Option<Vec<u8>>) -> Nc {
        let eof = input.is_some() || o.detach;
        Nc { o, addr: None, alts: Vec::new(), all: Vec::new(), lookup: None, at: 0, sock: None, connected: false, listener: None, udp_port: None, peer: None, pending: input.unwrap_or_default(), eof, any_ok: false }
    }
    /// the addresses to connect to: the first now, the others when it fails (only an answer
    /// with both families gives others)
    fn set_addrs(&mut self, all: Vec<IpAddr>) {
        let both = all.iter().any(|a| a.is_v4()) && all.iter().any(|a| !a.is_v4());
        self.all = if both { all } else { all.into_iter().take(1).collect() };
        self.addr = self.all.first().copied();
        self.alts = self.all.iter().skip(1).copied().collect();
    }
    /// the connection to the current address failed: the next one of the name, if any
    fn next_addr(&mut self, os: &mut OsApi) -> bool {
        if self.alts.is_empty() {
            return false;
        }
        self.addr = Some(self.alts.remove(0));
        self.sock = None;
        self.connect(os);
        true
    }
    fn port(&self) -> u16 {
        self.o.ports.get(self.at).copied().unwrap_or(0)
    }
    fn tag(&self) -> &'static str {
        if self.o.udp { "udp" } else { "tcp" }
    }
    fn exit(&mut self, os: &mut OsApi, code: i32) {
        if let Some(s) = self.sock.take() {
            os.tcp_close(s);
        }
        if let Some(l) = self.listener.take() {
            os.tcp_close(l);
        }
        if let Some(p) = self.udp_port.take() {
            os.close(p);
        }
        os.exit(code);
    }
    /// connects to the current port
    fn connect(&mut self, os: &mut OsApi) {
        let Some(addr) = self.addr else {
            return;
        };
        let port = self.port();
        if self.o.udp {
            // a socket of the peer's family, connected to it: ICMP errors reach nc
            let bind = if addr.is_v4() { ANY4 } else { ANY };
            match os.bind_on(bind, self.o.sport, None) {
                Ok(p) => {
                    self.udp_port = Some(p);
                    self.connected = true;
                    self.peer = Some((addr, port));
                    os.udp_connect(p, Some((addr, port)));
                    self.flush(os);
                }
                Err(_) => {
                    os.print("nc: bind failed: Address already in use\n");
                    self.exit(os, 1);
                }
            }
            return;
        }
        let opts = SockOpts::default();
        match os.tcp_connect_from(self.o.sport, addr, port, opts) {
            Ok(s) => {
                self.sock = Some(s);
                if let Some(w) = self.o.wait {
                    let at = os.now() + w;
                    os.timer(K_OWN, at);
                }
            }
            Err(_) => {
                if self.o.verbose {
                    os.print(&format!("nc: connect to {} port {} ({}) failed: Network is unreachable\n", self.o.host, port, self.tag()));
                }
                if !self.next_addr(os) {
                    self.next_port(os);
                }
            }
        }
    }
    /// a scan goes on with the next port; else nc is done
    fn next_port(&mut self, os: &mut OsApi) {
        self.sock = None;
        self.connected = false;
        self.at += 1;
        if self.at < self.o.ports.len() {
            let all = core::mem::take(&mut self.all);
            if !all.is_empty() {
                self.set_addrs(all);
            }
            self.connect(os);
        } else {
            let code = if self.any_ok { 0 } else { 1 };
            self.exit(os, code);
        }
    }
    /// sends what waits; shuts down or quits after the end of the input
    fn flush(&mut self, os: &mut OsApi) {
        if !self.connected || self.o.zero {
            return;
        }
        if !self.pending.is_empty() {
            if self.o.udp {
                if let (Some(p), Some((a, port))) = (self.udp_port, self.peer) {
                    let d = core::mem::take(&mut self.pending);
                    os.send(p, a, port, &d);
                }
            } else if let Some(s) = self.sock {
                let n = os.tcp_send(s, &self.pending);
                self.pending.drain(..n);
            }
        }
        if self.eof && self.pending.is_empty() {
            if self.o.shutdown
                && let Some(s) = self.sock
            {
                os.tcp_shutdown(s);
            }
            if let Some(q) = self.o.quit {
                let at = os.now() + q;
                os.timer(2, at);
            }
        }
    }
    fn line(&mut self, os: &mut OsApi, l: &str) {
        self.pending.extend_from_slice(l.as_bytes());
        self.pending.extend_from_slice(if self.o.crlf { b"\r\n" } else { b"\n" });
        self.flush(os);
    }
}

impl Service for Nc {
    fn kind(&self) -> &'static str {
        "nc"
    }
    fn start(&mut self, os: &mut OsApi) {
        if self.o.listen {
            let port = self.port();
            // the address to listen on: one given, :: for both families with -6, else every
            // IPv4 address (as OpenBSD netcat binds)
            let given = IpAddr::parse(self.o.host.trim_start_matches('[').trim_end_matches(']'));
            let bind = match (given, self.o.family) {
                (Some(a), _) => a,
                (None, Some(Family::V6)) => ANY,
                _ => ANY4,
            };
            let shown = if bind == ANY { String::from("::") } else { format!("{}", bind) };
            if self.o.udp {
                match os.bind_on(bind, port, None) {
                    Ok(p) => {
                        self.udp_port = Some(p);
                        if self.o.verbose {
                            os.print(&format!("Bound on {} {}\n", shown, port));
                        }
                    }
                    Err(_) => {
                        os.print("nc: Address already in use\n");
                        self.exit(os, 1);
                    }
                }
                return;
            }
            match os.tcp_listen_on(bind, port, 1, SockOpts::default()) {
                Ok(l) => {
                    self.listener = Some(l);
                    if self.o.verbose {
                        os.print(&format!("Listening on {} {}\n", shown, port));
                    }
                }
                Err(_) => {
                    os.print("nc: Address already in use\n");
                    self.exit(os, 1);
                }
            }
            return;
        }
        let fam = self.o.family.unwrap_or(Family::Any);
        let literal = IpAddr::parse(self.o.host.trim_start_matches('[').trim_end_matches(']'));
        match literal {
            Some(a) if fam == Family::Any || (fam == Family::V4) == a.is_v4() => {
                self.set_addrs(alloc::vec![a]);
                self.connect(os);
            }
            Some(_) => {
                os.print(&format!("nc: getaddrinfo for host \"{}\" port {}: Address family for hostname not supported\n", self.o.host, self.port()));
                self.exit(os, 1);
            }
            None => self.lookup = Some(os.resolve_host(&self.o.host, fam)),
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Resolved(l) if Some(l) == self.lookup => {
                let all = os.lookup(l).and_then(|x| x.result.clone()).and_then(|r| r.ok()).map(|r| r.sorted).unwrap_or_default();
                if all.is_empty() {
                    os.print(&format!("nc: getaddrinfo for host \"{}\" port {}: Name or service not known\n", self.o.host, self.port()));
                    self.exit(os, 1);
                } else {
                    self.set_addrs(all);
                    self.connect(os);
                }
            }
            SvcEvent::TcpConnected { sock, .. } if Some(sock) == self.sock => {
                os.cancel(K_OWN);
                self.connected = true;
                self.any_ok = true;
                if self.o.verbose {
                    let name = service_name(self.port(), false).unwrap_or("*");
                    os.print(&format!("Connection to {} {} port [tcp/{}] succeeded!\n", self.o.host, self.port(), name));
                }
                if self.o.zero {
                    os.tcp_close(sock);
                    self.next_port(os);
                    return;
                }
                self.flush(os);
            }
            SvcEvent::TcpAccept { listener, .. } if Some(listener) == self.listener => {
                while let Some(s) = os.tcp_accept(listener) {
                    if self.sock.is_some() {
                        os.tcp_abort(s);
                        continue;
                    }
                    self.sock = Some(s);
                    self.connected = true;
                    if self.o.verbose
                        && let Some(r) = os.tcp_info(s)
                    {
                        os.print(&format!("Connection received on {} {}\n", r.remote.0, r.remote.1));
                    }
                    self.flush(os);
                }
            }
            SvcEvent::TcpData { sock, .. } if Some(sock) == self.sock => {
                let d = os.tcp_recv(sock, usize::MAX);
                os.print(&String::from_utf8_lossy(&d));
            }
            SvcEvent::TcpWritable { sock } if Some(sock) == self.sock => self.flush(os),
            SvcEvent::TcpEof { sock, .. } | SvcEvent::TcpClosed { sock, end: TcpEnd::Closed } if Some(sock) == self.sock => {
                let d = os.tcp_recv(sock, usize::MAX);
                os.print(&String::from_utf8_lossy(&d));
                os.tcp_close(sock);
                self.sock = None;
                self.connected = false;
                if self.o.listen && self.o.keep {
                    return;
                }
                self.exit(os, 0);
            }
            SvcEvent::TcpClosed { sock, end } if Some(sock) == self.sock => {
                let d = os.tcp_recv(sock, usize::MAX);
                os.print(&String::from_utf8_lossy(&d));
                os.tcp_close(sock);
                self.sock = None;
                if !self.connected {
                    // the connection was not set up
                    if self.o.verbose {
                        os.print(&format!("nc: connect to {} port {} ({}) failed: {}\n", self.o.host, self.port(), self.tag(), end.errno().unwrap_or("Connection refused")));
                    }
                    if !self.next_addr(os) {
                        self.next_port(os);
                    }
                    return;
                }
                self.connected = false;
                if self.o.listen && self.o.keep {
                    return;
                }
                self.exit(os, 1);
            }
            SvcEvent::Udp(d) if Some(d.dport) == self.udp_port => {
                // a client hears only its peer (a connected UDP socket)
                if !self.o.listen && self.peer != Some((d.src, d.sport)) {
                    return;
                }
                if self.o.listen && self.peer.is_none() && self.o.verbose {
                    os.print(&format!("Connection received on {} {}\n", d.src, d.sport));
                }
                if self.o.listen {
                    self.peer = Some((d.src, d.sport));
                    self.connected = true;
                }
                os.print(&String::from_utf8_lossy(&d.data));
                self.flush(os);
            }
            // the peer of a connected UDP socket refused (port unreachable): OpenBSD netcat's
            // poll sees the error on the socket and ends without a word
            SvcEvent::UdpError { port, .. } if Some(port) == self.udp_port && !self.o.listen => self.exit(os, 0),
            SvcEvent::Console(ConsoleEv::Line(l)) => {
                if !self.o.detach {
                    self.line(os, &l);
                }
            }
            SvcEvent::Console(ConsoleEv::Eof) => {
                self.eof = true;
                self.flush(os);
            }
            SvcEvent::Timer(K_OWN) => {
                // -w: the connection is not up in time
                if !self.connected
                    && let Some(s) = self.sock.take()
                {
                    os.tcp_close(s);
                    if self.o.verbose {
                        os.print(&format!("nc: connect to {} port {} ({}) timed out: Operation now in progress\n", self.o.host, self.port(), self.tag()));
                    }
                    self.next_port(os);
                }
            }
            SvcEvent::Timer(2) => self.exit(os, 0),
            _ => {}
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        Vec::new()
    }
    fn status(&self) -> String {
        String::from("nc")
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
}

// ---------------------------------------------------------------- ss

const SS_HELP: &str = concat!(
    "Usage: ss [ OPTIONS ]\n       ss [ OPTIONS ] [ FILTER ]\n",
    "   -h, --help          this text\n",
    "   -V, --version       which ss this is\n",
    "   -n, --numeric       ports as numbers, not service names\n",
    "   -a, --all           every socket (listening, unconnected, closing ones too)\n",
    "   -l, --listening     only listening sockets\n",
    "   -o, --options       the timer of each connection\n",
    "   -p, --processes     the program that holds the socket\n",
    "   -i, --info          what TCP knows about each connection (windows, RTT, counters)\n",
    "   -s, --summary       counts by protocol and state\n",
    "   -H, --no-header     no title line\n",
    "   -4, --ipv4          IPv4 sockets only\n",
    "   -6, --ipv6          IPv6 sockets only (one on :: that takes both families is one of them)\n",
    "   -t, --tcp           TCP sockets\n",
    "   -u, --udp           UDP sockets\n",
    "   FILTER := [ state STATE ]... [ EXPRESSION ]\n",
    "   STATE := established | syn-sent | syn-recv | fin-wait-1 | fin-wait-2 | time-wait |\n",
    "            closed | close-wait | last-ack | listening | closing | all | connected |\n",
    "            synchronized | bucket | big\n",
    "   EXPRESSION: sport|dport OP :PORT, src|dst ADDRESS[:PORT] (IPv6: [ADDRESS]:PORT), joined\n",
    "               by and, or, not, ( );\n",
    "   OP := = | != | < | > | <= | >= (or eq, ne, lt, gt, le, ge)\n",
);

/// a socket state bit of ss's filters (TCP states, and CLOSE for unconnected UDP)
fn st_bit(s: TcpState) -> u32 {
    1 << (match s {
        TcpState::Established => 1,
        TcpState::SynSent => 2,
        TcpState::SynReceived => 3,
        TcpState::FinWait1 => 4,
        TcpState::FinWait2 => 5,
        TcpState::TimeWait => 6,
        TcpState::Closed => 7,
        TcpState::CloseWait => 8,
        TcpState::LastAck => 9,
        TcpState::Listen => 10,
        TcpState::Closing => 11,
    })
}

const SS_ALL: u32 = 0xFFE;
const SS_CONN: u32 = SS_ALL & !((1 << 10) | (1 << 7) | (1 << 6) | (1 << 3));

fn state_bits(s: &str) -> Option<u32> {
    Some(match s {
        "all" => SS_ALL,
        "connected" => SS_ALL & !((1 << 10) | (1 << 7)),
        "synchronized" => SS_ALL & !((1 << 10) | (1 << 7) | (1 << 2)),
        "bucket" => (1 << 3) | (1 << 6),
        "big" => SS_ALL & !((1 << 3) | (1 << 6)),
        "established" => 1 << 1,
        "syn-sent" => 1 << 2,
        "syn-recv" | "syn-received" => 1 << 3,
        "fin-wait-1" => 1 << 4,
        "fin-wait-2" => 1 << 5,
        "time-wait" => 1 << 6,
        "closed" | "unconnected" => 1 << 7,
        "close-wait" => 1 << 8,
        "last-ack" => 1 << 9,
        "listening" | "listen" => 1 << 10,
        "closing" => 1 << 11,
        _ => return None,
    })
}

/// one socket for ss
#[derive(Clone, Debug)]
struct SsRow {
    udp: bool,
    bit: u32,
    state: &'static str,
    recv_q: u32,
    send_q: u32,
    local: (IpAddr, u16),
    /// "%eth0" of a socket bound to an interface
    dev: String,
    remote: (IpAddr, u16),
    /// an IPv6 socket of the kernel's (on a device with IPv6, a socket on :: is one)
    v6: bool,
    listening: bool,
    owner: Option<(String, u32, u32)>,
    timer: Option<(TcpTimer, Ns, u8)>,
    info: Option<String>,
}

/// a filter expression of ss
#[derive(Clone, Debug)]
enum SsExpr {
    Port { local: bool, op: &'static str, port: u16 },
    Addr { local: bool, addr: IpAddr, port: Option<u16> },
    Not(Box<SsExpr>),
    And(Box<SsExpr>, Box<SsExpr>),
    Or(Box<SsExpr>, Box<SsExpr>),
}

impl SsExpr {
    fn holds(&self, r: &SsRow) -> bool {
        match self {
            SsExpr::Port { local, op, port } => {
                let p = if *local { r.local.1 } else { r.remote.1 };
                match *op {
                    "=" => p == *port,
                    "!=" => p != *port,
                    "<" => p < *port,
                    ">" => p > *port,
                    "<=" => p <= *port,
                    _ => p >= *port,
                }
            }
            SsExpr::Addr { local, addr, port } => {
                let (a, p) = if *local { r.local } else { r.remote };
                a == *addr && port.is_none_or(|x| x == p)
            }
            SsExpr::Not(e) => !e.holds(r),
            SsExpr::And(a, b) => a.holds(r) && b.holds(r),
            SsExpr::Or(a, b) => a.holds(r) || b.holds(r),
        }
    }
}

struct SsParser<'a> {
    w: &'a [String],
    k: usize,
}

impl SsParser<'_> {
    fn peek(&self) -> Option<&str> {
        self.w.get(self.k).map(String::as_str)
    }
    fn take(&mut self) -> Option<String> {
        let x = self.w.get(self.k).cloned();
        self.k += 1;
        x
    }
    fn expr(&mut self) -> Result<SsExpr, String> {
        let mut a = self.term()?;
        while matches!(self.peek(), Some("or") | Some("||") | Some(",")) {
            self.k += 1;
            let b = self.term()?;
            a = SsExpr::Or(Box::new(a), Box::new(b));
        }
        Ok(a)
    }
    fn term(&mut self) -> Result<SsExpr, String> {
        let mut a = self.factor()?;
        loop {
            match self.peek() {
                Some("and") | Some("&&") => {
                    self.k += 1;
                }
                Some("or") | Some("||") | Some(",") | Some(")") | None => return Ok(a),
                _ => {}
            }
            let b = self.factor()?;
            a = SsExpr::And(Box::new(a), Box::new(b));
        }
    }
    fn factor(&mut self) -> Result<SsExpr, String> {
        let Some(t) = self.take() else {
            return Err(String::from("ss: the filter ends too early\n"));
        };
        match t.as_str() {
            "not" | "!" => Ok(SsExpr::Not(Box::new(self.factor()?))),
            "(" => {
                let e = self.expr()?;
                if self.take().as_deref() != Some(")") {
                    return Err(String::from("ss: a parenthesis of the filter is not closed\n"));
                }
                Ok(e)
            }
            "sport" | "dport" => {
                let local = t == "sport";
                let mut op = "=";
                if let Some(o) = self.peek() {
                    let m = match o {
                        "=" | "==" | "eq" => Some("="),
                        "!=" | "ne" | "neq" => Some("!="),
                        "<" | "lt" => Some("<"),
                        ">" | "gt" => Some(">"),
                        "<=" | "le" | "leq" => Some("<="),
                        ">=" | "ge" | "geq" => Some(">="),
                        _ => None,
                    };
                    if let Some(m) = m {
                        op = m;
                        self.k += 1;
                    }
                }
                let v = self.take().unwrap_or_default();
                let p = port_of(v.trim_start_matches(':')).ok_or_else(|| format!("ss: \"{}\" is no port\n", v))?;
                Ok(SsExpr::Port { local, op, port: p })
            }
            "src" | "dst" => {
                let v = self.take().unwrap_or_default();
                let port = |p: &str| port_of(p).ok_or_else(|| format!("ss: \"{}\" is no port\n", p));
                // "[2001:db8::1]:80", "[2001:db8::1]", "2001:db8::1", "10.0.0.1:80", "10.0.0.1"
                let (a, p) = if let Some(r) = v.strip_prefix('[') {
                    match r.split_once(']') {
                        Some((a, rest)) => (a, match rest.strip_prefix(':') {
                            Some(x) => Some(port(x)?),
                            None => None,
                        }),
                        None => (r, None),
                    }
                } else if v.matches(':').count() > 1 {
                    (v.as_str(), None)
                } else {
                    match v.rsplit_once(':') {
                        Some((a, x)) => (a, Some(port(x)?)),
                        None => (v.as_str(), None),
                    }
                };
                let addr = IpAddr::parse(a).ok_or_else(|| format!("ss: \"{}\" is no IP address\n", a))?;
                Ok(SsExpr::Addr { local: t == "src", addr, port: p })
            }
            other => Err(format!("ss: the filter does not know \"{}\"\n", other)),
        }
    }
}

/// an address as ss writes it: IPv4 as it is, IPv6 in brackets, the :: of a socket that takes
/// both families "*" (as iproute2 shows a socket without IPV6_V6ONLY) when it is an IPv6 socket
/// (`v6`: the device runs IPv6), else 0.0.0.0
fn ss_addr(a: IpAddr, v6: bool) -> String {
    match a {
        IpAddr::V4(x) => format!("{}", x),
        IpAddr::V6(x) if x.is_unspecified() => String::from(if v6 { "*" } else { "0.0.0.0" }),
        IpAddr::V6(x) => format!("[{}]", x),
    }
}

/// ss's milliseconds of a timer ("200ms", "1.020ms", "59sec", "119min")
fn ss_ms(ms: u64) -> String {
    let mut secs = ms / 1000;
    let minutes = secs / 60;
    secs %= 60;
    let mut msecs = ms % 1000;
    let mut s = String::new();
    if minutes > 0 {
        msecs = 0;
        s.push_str(&format!("{}min", minutes));
        if minutes > 9 {
            secs = 0;
        }
    }
    if secs > 0 {
        if secs > 9 {
            msecs = 0;
        }
        s.push_str(&format!("{}{}", secs, if msecs > 0 { "." } else { "sec" }));
    }
    if msecs > 0 {
        s.push_str(&format!("{:03}ms", msecs));
    }
    s
}

/// "0.075" (ms with three decimals)
fn ms_3(ns: Ns) -> String {
    let us = ns / 1000;
    format!("{}.{:03}", us / 1000, us % 1000)
}

/// the -i line of a connection
fn ss_info(r: &TcpRow) -> String {
    let mut s = String::from(" reno");
    if r.state == TcpState::Listen {
        return s;
    }
    s.push_str(&format!(" rto:{}", r.rto / MS));
    if let Some(rtt) = r.srtt {
        s.push_str(&format!(" rtt:{}/{}", ms_3(rtt), ms_3(r.rttvar)));
    }
    let mss = r.mss.max(1) as u32;
    // the headers the MSS leaves out: 40 bytes over IPv4, 60 over IPv6
    let hdr = if r.local.0.is_v4() { 40 } else { 60 };
    s.push_str(&format!(" mss:{} pmtu:{} advmss:{} cwnd:{}", r.mss, r.mss as u32 + hdr, r.mss, r.cwnd / mss));
    if r.ssthresh < crate::tcp::SSTHRESH_INF {
        s.push_str(&format!(" ssthresh:{}", r.ssthresh / mss));
    }
    let c = &r.counters;
    if c.bytes_sent > 0 {
        s.push_str(&format!(" bytes_sent:{}", c.bytes_sent));
    }
    if c.retrans > 0 {
        s.push_str(&format!(" bytes_retrans:{}", c.retrans * r.mss as u64));
    }
    if c.bytes_acked > 0 {
        s.push_str(&format!(" bytes_acked:{}", c.bytes_acked));
    }
    if c.bytes_received > 0 {
        s.push_str(&format!(" bytes_received:{}", c.bytes_received));
    }
    s.push_str(&format!(" segs_out:{} segs_in:{}", c.segs_out, c.segs_in));
    if let Some(rtt) = r.srtt.filter(|&x| x > 0) {
        let bps = (r.cwnd as u128 * 8 * 1_000_000_000) / rtt as u128;
        s.push_str(&format!(" send {}bps", bps));
    }
    if c.retrans > 0 || r.retries > 0 {
        s.push_str(&format!(" retrans:{}/{}", r.retries, c.retrans));
    }
    if c.dupacks_in > 0 {
        s.push_str(&format!(" dsack_dups:{}", c.dup_in));
    }
    // Linux starts rcv_space at the window, at most ten segments
    s.push_str(&format!(" rcv_space:{} snd_wnd:{}", r.rcv_wnd.min(10 * mss), r.snd_wnd));
    s
}

/// iproute2's column layout without a terminal: every column as wide as its widest entry (the
/// very last entry, the extras of the last line, does not count)
fn ss_render(rows: &[Vec<String>], show: &[bool; 9]) -> String {
    // align (true: right), left delimiter
    const COLS: [(bool, &str); 9] = [(false, ""), (false, " "), (false, " "), (false, " "), (true, " "), (false, ""), (true, " "), (false, ""), (false, "")];
    let mut max = [0usize; 9];
    let last = rows.len().saturating_sub(1);
    for (i, r) in rows.iter().enumerate() {
        for c in 0..9 {
            if !show[c] || (i == last && c == 8) {
                continue;
            }
            max[c] = max[c].max(r[c].chars().count());
        }
    }
    let mut width = [0usize; 9];
    let mut first = true;
    for c in 0..9 {
        if !show[c] {
            continue;
        }
        width[c] = if !first && max[c] > 0 { max[c] + COLS[c].1.len() } else { max[c] };
        if width[c] > 0 {
            first = false;
        }
    }
    let mut out = String::new();
    for r in rows {
        let mut started = false;
        for c in 0..9 {
            if !show[c] {
                continue;
            }
            let tok = &r[c];
            let len = tok.chars().count();
            let mut printed = 0;
            if started {
                out.push_str(COLS[c].1);
                printed = COLS[c].1.len();
            }
            started = true;
            if width[c] > 0 && COLS[c].0 {
                let s = width[c] as i64 - len as i64 - printed as i64;
                if s > 0 {
                    out.extend(core::iter::repeat_n(' ', s as usize));
                    printed += s as usize;
                }
            }
            out.push_str(tok);
            printed += len;
            if width[c] > 0 && !COLS[c].0 && width[c] > printed {
                out.extend(core::iter::repeat_n(' ', width[c] - printed));
            }
        }
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------- top

/// "M:SS.hh" of top's TIME+ column
fn time_plus(ns: Ns) -> String {
    let cs = ns / (10 * MS);
    format!("{}:{:02}.{:02}", cs / 6000, (cs / 100) % 60, cs % 100)
}

/// "%.2f" of thousandths
fn f2(m: u64) -> String {
    let c = (m + 5) / 10;
    format!("{}.{:02}", c / 100, c % 100)
}

/// "%.1f" of thousandths
fn f1(m: u64) -> String {
    let d = (m + 50) / 100;
    format!("{}.{}", d / 10, d % 10)
}

const TOP_HELP: &str = "Usage: top [-b] [-n ITERATIONS] [-d SECONDS]\n  -b  batch mode: the screens one after the other, no clearing\n  -n  stop after this many screens\n  -d  seconds between two screens (3 if not given)\nThe overview shows the services of the device; Ctrl+C ends it.\n";

/// CPUs a device counts for top
const CPUS: u64 = 4;

// ---------------------------------------------------------------- the commands

const SS_USAGE: &str = "Usage: ss [ OPTIONS ]\n       ss [ OPTIONS ] [ FILTER ]\n   -h, --help          the options\n";

impl Engine {
    fn tool_err(&mut self, ni: usize, text: &str) -> Result<(), String> {
        self.out(ni, text);
        Err(String::from(text.lines().next().unwrap_or("")))
    }

    /// starts a console program; the console is busy until it ends
    pub(crate) fn run_program(&mut self, ni: usize, name: &str, svc: Box<dyn Service>, stdin: bool) -> Result<(), String> {
        let Some(id) = self.console_program(ni, name, svc) else {
            return self.tool_err(ni, &format!("{}: this device has no network stack\n", name));
        };
        self.next_job += 1;
        let jid = self.next_job;
        self.cli_start_job(ni, jid, JobOut::Proc { svc: id, stdin });
        Ok(())
    }

    /// the console woke for a program: when it ended, it goes and the console takes lines
    /// again
    pub(crate) fn proc_wake(&mut self, ni: usize, svc: u32) {
        if self.console_exit(ni, svc).is_none() {
            return;
        }
        self.console_kill(ni, svc);
        self.cli_job_done(ni);
    }

    /// a line typed while a program runs that reads the console: true when it took it
    pub(crate) fn proc_line(&mut self, ni: usize, line: &str) -> bool {
        let svc = self.cdev(ni).and_then(|c| c.job.as_ref()).and_then(|j| match j.out {
            JobOut::Proc { svc, stdin: true } => Some(svc),
            _ => None,
        });
        match svc {
            Some(s) => {
                self.console_input(ni, s, ConsoleEv::Line(String::from(line)));
                true
            }
            None => false,
        }
    }

    pub(crate) fn sh_curl(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        // smtp:// and pop3:// URLs (cli/mail.rs)
        if args.iter().any(|a| a.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("smtp://") || p.eq_ignore_ascii_case("pop3://"))) {
            return self.sh_curl_mail(ni, args);
        }
        // ftp:// URLs (ftpc.rs)
        if args.iter().any(|a| a.len() > 6 && a[..6].eq_ignore_ascii_case("ftp://")) {
            return self.sh_curl_ftp(ni, args);
        }
        let a: Vec<String> = args.iter().map(|s| String::from(*s)).collect();
        match parse_curl(&a) {
            Ok(o) => self.run_program(ni, "curl", Box::new(Curl::new(o)), false),
            Err((text, 0)) => {
                self.out(ni, &text);
                Ok(())
            }
            Err((text, _)) => self.tool_err(ni, &text),
        }
    }

    /// nc; `input` is what a pipe gives it (then the console is not read)
    pub(crate) fn sh_nc(&mut self, ni: usize, args: &[&str], input: Option<Vec<u8>>) -> Result<(), String> {
        let a: Vec<String> = args.iter().map(|s| String::from(*s)).collect();
        match parse_nc(&a) {
            Ok(o) => {
                let stdin = input.is_none() && !o.detach;
                self.run_program(ni, "nc", Box::new(Nc::new(o, input)), stdin)
            }
            Err((text, _)) => self.tool_err(ni, &text),
        }
    }

    pub(crate) fn sh_printf(&mut self, ni: usize, line: &str) -> Result<(), String> {
        let w = sh_words(line);
        match printf_bytes(&w[1..]) {
            Ok(d) => {
                self.out(ni, &String::from_utf8_lossy(&d));
                Ok(())
            }
            Err(e) => self.tool_err(ni, &e),
        }
    }

    /// "LEFT | RIGHT": echo or printf into nc (or cat)
    pub(crate) fn sh_pipe(&mut self, ni: usize, left: &str, right: &str) -> Result<(), String> {
        let lw = sh_words(left);
        let rw = sh_words(right);
        let data = match lw.first().map(String::as_str) {
            Some("echo") => echo_bytes(&lw[1..]),
            Some("printf") => match printf_bytes(&lw[1..]) {
                Ok(d) => d,
                Err(e) => return self.tool_err(ni, &e),
            },
            _ => return self.tool_err(ni, "jfn-sh: a pipe here takes echo or printf on the left and nc, cat or mail on the right\n"),
        };
        match rw.first().map(String::as_str) {
            Some("nc") | Some("netcat") => {
                let args: Vec<&str> = rw[1..].iter().map(String::as_str).collect();
                self.sh_nc(ni, &args, Some(data))
            }
            Some("cat") => {
                self.out(ni, &String::from_utf8_lossy(&data));
                Ok(())
            }
            // the text of a message (cli/mail.rs)
            Some("mail") => {
                let args: Vec<&str> = rw[1..].iter().map(String::as_str).collect();
                self.sh_mail(ni, &args, Some(data))
            }
            _ => self.tool_err(ni, "jfn-sh: a pipe here takes echo or printf on the left and nc, cat or mail on the right\n"),
        }
    }

    // ------------------------------------------------------------ ss

    /// the sockets of a device as ss lists them
    fn ss_rows(&self, ni: usize, numeric_owner: bool) -> Vec<SsRow> {
        let dev = self.dev_of(ni);
        let ids = self.service_ids(dev);
        let id_of = |name: &str| ids.iter().find(|s| s.1 == name).map(|s| s.0);
        let mut fds: alloc::collections::BTreeMap<u32, u32> = alloc::collections::BTreeMap::new();
        let mut next_fd = |id: u32| -> u32 {
            let e = fds.entry(id).or_insert(3);
            let fd = *e;
            *e += 1;
            fd
        };
        let _ = numeric_owner;
        let mut rows = Vec::new();
        let st = self.stack(ni);
        // a socket on :: is an IPv6 socket of the kernel when the device runs IPv6 (else the
        // device has IPv4 sockets only)
        let six = self.v6_used(ni);
        let is6 = |a: IpAddr| !a.is_v4() && (six || a != ANY);
        for s in self.sockets(dev) {
            let owner = match s.owner {
                Owner::Svc(id) => ids.iter().find(|x| x.0 == id).map(|x| (x.1.clone(), 100 + id, next_fd(id))),
                _ => None,
            };
            let devname = s.iface.and_then(|i| st.and_then(|x| x.ifaces.get(i as usize))).map(|f| format!("%{}", f.name)).unwrap_or_default();
            let none = if s.addr.is_v4() { ANY4 } else { ANY };
            rows.push(SsRow { udp: true, bit: 1 << 7, state: "UNCONN", recv_q: 0, send_q: 0, local: (s.addr, s.port), dev: devname, remote: (none, 0), v6: is6(s.addr), listening: true, owner, timer: None, info: None });
        }
        for r in self.tcp_conns(dev) {
            let owner = r.owner.as_ref().and_then(|n| id_of(n).map(|id| (n.clone(), 100 + id, next_fd(id))));
            let timer = r.timer.filter(|t| t.0 != TcpTimer::DelayedAck).map(|t| (t.0, t.1, r.retries));
            rows.push(SsRow {
                udp: false,
                bit: st_bit(r.state),
                state: r.state.ss_name(),
                recv_q: r.recv_q,
                send_q: r.send_q,
                local: r.local,
                dev: String::new(),
                remote: r.remote,
                v6: is6(r.local.0),
                listening: r.state == TcpState::Listen,
                owner,
                timer,
                info: Some(ss_info(&r)),
            });
        }
        rows
    }

    pub(crate) fn sh_ss(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let (mut tcp, mut udp, mut all, mut listen, mut numeric, mut procs, mut opts, mut info, mut summary, mut header) = (false, false, false, false, false, false, false, false, false, true);
        // -4 / -6: one family (0: both)
        let mut fam = 0u8;
        let mut k = 0;
        while k < args.len() {
            let a = args[k];
            if let Some(l) = a.strip_prefix("--") {
                match l {
                    "tcp" => tcp = true,
                    "udp" => udp = true,
                    "all" => all = true,
                    "listening" => listen = true,
                    "numeric" => numeric = true,
                    "processes" => procs = true,
                    "options" => opts = true,
                    "info" => info = true,
                    "summary" => summary = true,
                    "no-header" => header = false,
                    "ipv4" => fam = 4,
                    "ipv6" => fam = 6,
                    "resolve" => {}
                    "help" => {
                        self.out(ni, SS_HELP);
                        return Ok(());
                    }
                    "version" => {
                        self.out(ni, "ss of the JFNetworker shell (sockets of the simulated device)\n");
                        return Ok(());
                    }
                    _ => return self.tool_err(ni, &format!("ss: unrecognized option '--{}'\n{}", l, SS_USAGE)),
                }
            } else if a.len() > 1 && a.starts_with('-') {
                for c in a[1..].chars() {
                    match c {
                        't' => tcp = true,
                        'u' => udp = true,
                        'a' => all = true,
                        'l' => listen = true,
                        'n' => numeric = true,
                        'p' => procs = true,
                        'o' => opts = true,
                        'i' => info = true,
                        's' => summary = true,
                        'H' => header = false,
                        '4' => fam = 4,
                        '6' => fam = 6,
                        'r' => {}
                        'h' => {
                            self.out(ni, SS_HELP);
                            return Ok(());
                        }
                        'V' => {
                            self.out(ni, "ss of the JFNetworker shell (sockets of the simulated device)\n");
                            return Ok(());
                        }
                        _ => return self.tool_err(ni, &format!("ss: invalid option -- '{}'\n{}", c, SS_USAGE)),
                    }
                }
            } else {
                break;
            }
            k += 1;
        }
        let rest: Vec<String> = args[k..].iter().map(|s| String::from(*s)).collect();
        let mut states = 0u32;
        let mut words: Vec<String> = Vec::new();
        let mut j = 0;
        while j < rest.len() {
            if rest[j] == "state" {
                j += 1;
                match rest.get(j).and_then(|s| state_bits(s)) {
                    Some(b) => states |= b,
                    None => return self.tool_err(ni, &format!("ss: wrong state name: {}\n", rest.get(j).map(String::as_str).unwrap_or(""))),
                }
            } else {
                // a parenthesis may stick to its word
                let w = rest[j].as_str();
                let mut w = w;
                while let Some(x) = w.strip_prefix('(') {
                    words.push(String::from("("));
                    w = x;
                }
                let mut close = 0;
                while let Some(x) = w.strip_suffix(')') {
                    close += 1;
                    w = x;
                }
                if !w.is_empty() {
                    words.push(String::from(w));
                }
                for _ in 0..close {
                    words.push(String::from(")"));
                }
            }
            j += 1;
        }
        let expr = if words.is_empty() {
            None
        } else {
            let mut p = SsParser { w: &words, k: 0 };
            match p.expr() {
                Ok(e) if p.k >= words.len() => Some(e),
                Ok(_) => return self.tool_err(ni, "ss: the filter does not end where it should\n"),
                Err(e) => return self.tool_err(ni, &e),
            }
        };
        if states == 0 {
            states = if all {
                SS_ALL
            } else if listen {
                (1 << 10) | (1 << 7)
            } else {
                SS_CONN
            };
        }
        if !tcp && !udp {
            tcp = true;
            udp = true;
        }
        let rows = self.ss_rows(ni, numeric);
        if summary {
            let t: Vec<&SsRow> = rows.iter().filter(|r| !r.udp).collect();
            let u = rows.iter().filter(|r| r.udp).count();
            let u6 = rows.iter().filter(|r| r.udp && r.v6).count();
            let tw = t.iter().filter(|r| r.state == "TIME-WAIT").count();
            let closed = tw + t.iter().filter(|r| r.state == "UNCONN").count();
            let estab = t.iter().filter(|r| r.state == "ESTAB").count();
            let orphan = t.iter().filter(|r| r.owner.is_none() && !r.listening && r.state != "TIME-WAIT").count();
            let hashed = t.len() - closed;
            let hashed6 = t.iter().filter(|r| r.v6 && r.state != "TIME-WAIT" && r.state != "UNCONN").count();
            let line = |name: &str, n: usize, n6: usize| format!("{}\t  {:<9} {:<9} {:<9}\n", name, n, n - n6, n6);
            let mut s = format!("Total: {}\nTCP:   {} (estab {}, closed {}, orphaned {}, timewait {})\n\nTransport Total     IP        IPv6\n", t.len() + u, t.len(), estab, closed, orphan, tw);
            s.push_str(&line("RAW", 0, 0));
            s.push_str(&line("UDP", u, u6));
            s.push_str(&line("TCP", hashed, hashed6));
            s.push_str(&line("INET", u + hashed, u6 + hashed6));
            s.push_str(&line("FRAG", 0, 0));
            s.push('\n');
            self.out(ni, &s);
            return Ok(());
        }
        let shown: Vec<&SsRow> = rows.iter().filter(|r| (if r.udp { udp } else { tcp }) && (fam == 0 || r.v6 == (fam == 6)) && r.bit & states != 0 && expr.as_ref().is_none_or(|e| e.holds(r))).collect();
        // UDP before TCP, as iproute2 lists them
        let mut table: Vec<Vec<String>> = Vec::new();
        if header {
            table.push(["Netid", "State", "Recv-Q", "Send-Q", "Local Address:", "Port", "Peer Address:", "Port", "Process"].iter().map(|x| String::from(*x)).collect());
        }
        let port = |p: u16, u: bool| -> String {
            if p == 0 {
                String::from("*")
            } else if !numeric && let Some(n) = service_name(p, u) {
                String::from(n)
            } else {
                format!("{}", p)
            }
        };
        for r in shown.iter().filter(|r| r.udp).chain(shown.iter().filter(|r| !r.udp)) {
            let mut proc = String::new();
            if procs && let Some((n, pid, fd)) = &r.owner {
                proc.push_str(&format!(" users:((\"{}\",pid={},fd={}))", n, pid, fd));
            }
            if opts && let Some((t, left, n)) = r.timer {
                let name = match t {
                    TcpTimer::Retransmit => "on",
                    TcpTimer::Persist => "persist",
                    TcpTimer::Keepalive => "keepalive",
                    TcpTimer::TimeWait => "timewait",
                    TcpTimer::DelayedAck => "unknown",
                };
                proc.push_str(&format!(" timer:({},{},{})", name, ss_ms(left / MS), n));
            }
            if info && let Some(i) = &r.info {
                proc.push_str("\n\t");
                proc.push_str(i);
            }
            table.push(alloc::vec![
                String::from(if r.udp { "udp" } else { "tcp" }),
                String::from(r.state),
                format!("{}", r.recv_q),
                format!("{}", r.send_q),
                format!("{}{}:", ss_addr(r.local.0, r.v6), r.dev),
                port(r.local.1, r.udp),
                format!("{}:", ss_addr(r.remote.0, r.v6)),
                port(r.remote.1, r.udp),
                proc,
            ]);
        }
        let one_state = states & (states - 1) == 0;
        let show = [tcp && udp, !one_state, true, true, true, true, true, true, true];
        let text = ss_render(&table, &show);
        self.out(ni, &text);
        Ok(())
    }

    // ------------------------------------------------------------ top

    pub(crate) fn sh_top(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let (mut batch, mut n, mut delay) = (false, 0u32, 3 * SEC);
        let mut k = 0;
        while k < args.len() {
            let a = args[k];
            let val = |k: &mut usize| -> Option<String> {
                *k += 1;
                args.get(*k).map(|s| String::from(*s))
            };
            match a {
                "-b" => batch = true,
                "-h" | "--help" => {
                    self.out(ni, TOP_HELP);
                    return Ok(());
                }
                "-n" => match val(&mut k).and_then(|v| v.parse::<u32>().ok()) {
                    Some(x) if x > 0 => n = x,
                    _ => return self.tool_err(ni, "top: -n wants a number of screens\n"),
                },
                "-d" => match val(&mut k).and_then(|v| secs_of(&v)) {
                    Some(d) if d >= 100 * MS => delay = d,
                    _ => return self.tool_err(ni, "top: -d wants seconds (0.1 at least)\n"),
                },
                x if x.starts_with("-n") => match x[2..].parse::<u32>() {
                    Ok(v) if v > 0 => n = v,
                    _ => return self.tool_err(ni, "top: -n wants a number of screens\n"),
                },
                x if x.starts_with("-b") => {
                    batch = true;
                    if let Some(r) = x[2..].strip_prefix('n') {
                        match r.parse::<u32>() {
                            Ok(v) if v > 0 => n = v,
                            _ => return self.tool_err(ni, "top: -n wants a number of screens\n"),
                        }
                    }
                }
                _ => return self.tool_err(ni, &format!("top: unknown option {}\n{}", a, TOP_HELP)),
            }
            k += 1;
        }
        let text = self.top_screen(ni, delay, batch);
        self.out(ni, &text);
        if n == 1 {
            return Ok(());
        }
        self.next_job += 1;
        let jid = self.next_job;
        let start = self.now;
        if let Some(c) = self.cdev_mut(ni) {
            c.job = Some(super::CliJob { id: jid, out: JobOut::Top { left: if n == 0 { 0 } else { n - 1 }, delay, batch }, start });
        }
        self.cli.jobs.insert(jid, ni);
        let at = self.now + delay;
        self.cli_wake_at(ni, at);
        self.cli_state(ni);
        Ok(())
    }

    /// the next screen of top (the console woke)
    pub(crate) fn top_tick(&mut self, ni: usize) {
        let Some(mut cj) = self.cdev(ni).and_then(|c| c.job.clone()) else {
            return;
        };
        let JobOut::Top { left, delay, batch } = cj.out else {
            return;
        };
        let text = self.top_screen(ni, delay, batch);
        let mut s = String::from(if batch { "\n" } else { "\x1b[H\x1b[2J" });
        s.push_str(&text);
        self.out(ni, &s);
        if left == 1 {
            self.cli_job_done(ni);
            return;
        }
        cj.out = JobOut::Top { left: left.saturating_sub(1), delay, batch };
        if let Some(c) = self.cdev_mut(ni) {
            c.job = Some(cj);
        }
        let at = self.now + delay;
        self.cli_wake_at(ni, at);
    }

    /// one screen of top: the time, the load, the services
    fn top_screen(&self, ni: usize, delay: Ns, _batch: bool) -> String {
        let dev = self.dev_of(ni);
        let now = self.now;
        let clock = self.clock(dev).unwrap_or(crate::os::EPOCH_UNIX_NS + now as i64);
        let secs = clock.div_euclid(1_000_000_000).rem_euclid(86_400);
        let up = now / SEC;
        let uptime = if up < 3600 {
            format!("{} min", up / 60)
        } else if up < 86_400 {
            format!("{:2}:{:02}", up / 3600, (up / 60) % 60)
        } else {
            let d = up / 86_400;
            format!("{} day{}, {:2}:{:02}", d, if d == 1 { "" } else { "s" }, (up / 3600) % 24, (up / 60) % 60)
        };
        // the web servers' work: busy and waiting time per window
        let ids = self.service_ids(dev);
        let servers: Vec<(u32, &crate::httpsvc::HttpServer)> = ids.iter().filter_map(|s| self.service::<crate::httpsvc::HttpServer>(dev, &s.1).map(|h| (s.0, h))).collect();
        // the scripts' CPU time (scriptsvc.rs)
        let scripts: Vec<(u32, &crate::scriptsvc::ScriptService)> = ids.iter().filter_map(|s| self.service::<crate::scriptsvc::ScriptService>(dev, &s.1).map(|x| (s.0, x))).collect();
        let load = |w: u64| -> u64 {
            let sum: u64 = servers.iter().map(|(_, h)| {
                let (b, q) = h.load(now, w);
                b + q
            }).sum::<u64>() + scripts.iter().map(|(_, x)| x.load(now, w)).sum::<u64>();
            (sum as u128 * 1000 / (w as u128 * SEC as u128)) as u64
        };
        let dsecs = (delay / SEC).max(1);
        let mut rows: Vec<(u32, char, u64, Ns, String)> = alloc::vec![(1, 'S', 0, 0, String::from("init"))];
        let mut busy_total = 0u64;
        let mut running = 1;
        for (id, name, _, run) in &ids {
            if !run {
                continue;
            }
            let (cpu_m, time, st) = match servers.iter().find(|s| s.0 == *id) {
                Some((_, h)) => {
                    let (b, _) = h.load(now, dsecs);
                    busy_total += b;
                    ((b as u128 * 100_000 / (dsecs as u128 * SEC as u128)) as u64, h.busy_ns, if h.busy() > 0 { 'R' } else { 'S' })
                }
                None => match scripts.iter().find(|s| s.0 == *id) {
                    Some((_, x)) => {
                        let b = x.load(now, dsecs);
                        busy_total += b;
                        ((b as u128 * 100_000 / (dsecs as u128 * SEC as u128)) as u64, x.busy_ns(), if x.busy(now) { 'R' } else { 'S' })
                    }
                    None => (0, 0, 'S'),
                },
            };
            if st == 'R' {
                running += 1;
            }
            rows.push((100 + id, st, cpu_m, time, name.clone()));
        }
        // top itself
        let me = 100 + ids.iter().map(|s| s.0).max().unwrap_or(0) + 1;
        rows.push((me, 'R', 0, 0, String::from("top")));
        rows.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
        let us = (busy_total as u128 * 100_000 / (dsecs as u128 * SEC as u128 * CPUS as u128)) as u64;
        let mut s = format!("top - {:02}:{:02}:{:02} up {},  1 user,  load average: {}, {}, {}\n", secs / 3600, (secs / 60) % 60, secs % 60, uptime, f2(load(60)), f2(load(300)), f2(load(900)));
        s.push_str(&format!("Tasks: {:3} total, {:3} running, {:3} sleeping,   0 stopped,   0 zombie\n", rows.len(), running, rows.len() - running));
        s.push_str(&format!("%Cpu(s):{:>5} us,  0.0 sy,  0.0 ni,{:>5} id,  0.0 wa,  0.0 hi,  0.0 si,  0.0 st \n\n", f1(us), f1(100_000u64.saturating_sub(us))));
        s.push_str("    PID USER      PR  NI S  %CPU     TIME+ COMMAND\n");
        for (pid, st, cpu, time, name) in rows {
            s.push_str(&format!("{:7} {:<8} {:>3} {:>3} {} {:>5} {:>9} {}\n", pid, "root", 20, 0, st, f1(cpu), time_plus(time), name));
        }
        s
    }

    // ------------------------------------------------------------ IOS

    /// "show tcp brief [all]"
    pub(crate) fn ios_tcp_brief(&self, ni: usize, all: bool) -> String {
        let dev = self.dev_of(ni);
        let mut s = String::from("TCB       Local Address               Foreign Address             (state)\n");
        // IPv6 in capitals, the port after a dot as for IPv4
        let at = |a: IpAddr, p: u16| match a {
            _ if a.is_unspecified() => format!("*.{}", p),
            IpAddr::V4(x) => format!("{}.{}", x, p),
            IpAddr::V6(x) => format!("{}.{}", up6(x), p),
        };
        for r in self.tcp_conns(dev) {
            if r.state == TcpState::Listen && !all {
                continue;
            }
            let remote = if r.state == TcpState::Listen { String::from("*.*") } else { at(r.remote.0, r.remote.1) };
            s.push_str(&format!("{:08X}  {:<27} {:<27} {}\n", 0x6A3C_2000u32.wrapping_add(r.id.wrapping_mul(0x2A8)), at(r.local.0, r.local.1), remote, r.state.ios_name()));
        }
        s
    }

    /// "show tcp statistics": the counters of the device's TCP
    pub(crate) fn ios_tcp_stats(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let st = self.tcp_stats(dev).unwrap_or_default();
        format!(
            "Rcvd: {} Total, {} checksum error\nSent: {} Total, {} retransmitted, {} resets\n{} Connections initiated, {} connections accepted, {} connections established\n{} Connection attempts failed, {} established connections reset\n{} Total rxmt timeout, {} SYNs dropped (queue full), {} delayed ACKs\n",
            st.in_segs, st.in_errs, st.out_segs, st.retrans_segs, st.out_rsts, st.active_opens, st.passive_opens, st.curr_estab, st.attempt_fails, st.estab_resets, st.timeouts, st.listen_drops, st.delayed_acks
        )
    }

    /// "show control-plane host open-ports": the device's own listening and connected sockets
    pub(crate) fn ios_open_ports(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let ids = self.service_ids(dev);
        let mut s = String::from("Active internet connections (servers and established)\nProt               Local Address             Foreign Address                  Service    State\n");
        let addr = |a: IpAddr, p: u16| match a {
            _ if a.is_unspecified() => format!("*:{}", p),
            IpAddr::V4(x) => format!("{}:{}", x, p),
            IpAddr::V6(x) => format!("[{}]:{}", up6(x), p),
        };
        for r in self.tcp_conns(dev) {
            if !matches!(r.state, TcpState::Listen | TcpState::Established) {
                continue;
            }
            let svc = r.owner.clone().unwrap_or_default();
            let remote = if r.state == TcpState::Listen { String::from("*:0") } else { addr(r.remote.0, r.remote.1) };
            s.push_str(&format!("{:>4}{:>28}{:>28}{:>25}{:>9}\n", "tcp", addr(r.local.0, r.local.1), remote, svc, if r.state == TcpState::Listen { "LISTEN" } else { "ESTABLIS" }));
        }
        for u in self.sockets(dev) {
            let svc = match u.owner {
                Owner::Svc(id) => ids.iter().find(|x| x.0 == id).map(|x| x.1.clone()).unwrap_or_default(),
                Owner::Resolver(_) => String::from("Resolver"),
                Owner::Log => String::from("Syslog"),
            };
            s.push_str(&format!("{:>4}{:>28}{:>28}{:>25}{:>9}\n", "udp", addr(u.addr, u.port), "*:0", svc, "LISTEN"));
        }
        s
    }
}
