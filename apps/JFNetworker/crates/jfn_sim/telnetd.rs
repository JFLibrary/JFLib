// telnetd -- Telnet on a device: the client "telnet HOST [PORT]" of the IOS-like console and
// the Linux shell (a console program that opens a TCP connection, negotiates as a client,
// relays the terminal and the remote's output, and ends with a "%" line on a refusal, a
// timeout or a close), and the helper hooks the login gate (cli/aaa.rs) uses for vty sessions
//
//   TelnetClient              the "telnet" console program (cli/tools.rs-style)
//   Engine::sh_telnet         the Linux shell's "telnet"
//   Engine::ios_telnet        the IOS-like "telnet"
//   VtyServer                 the vty telnet server of IOS-like devices -- OPEN (see below)
//
// A vty telnet server (TCP 23 with a CLI session per vty line, its login rules, a session
// limit and an idle timeout) needs a second, independent CLI session per connection next to the
// device's one console (cli/mod.rs keeps a single CliDev per node); that multi-session work is
// not in this milestone.  The NVT codec (netpkt::telnet), the client and the whole login / AAA
// stack are here, so the server is the next step.  The escape of the client is Ctrl+Shift+6,
// which closes the session and returns to the local command line.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use base::cmdtree::Parsed;
use netpkt::telnet::{self, Item, Options, Reader};
use netpkt::{IpAddr, Ipv4};

use crate::aaa::{AuthResult, Requester};
use crate::aaatrace::ANote;
use crate::cli::ios::K;
use crate::kernel::Engine;
use crate::os::{ConsoleEv, Metric, OsApi, Service, SvcEvent};
use crate::resolver::Family;
use crate::tcp::{SockOpts, TcpEnd};
use crate::trace::Note;

/// the "telnet" console program
#[derive(Clone, Debug)]
pub struct TelnetClient {
    host: String,
    port: u16,
    addr: Option<IpAddr>,
    lookup: Option<u32>,
    sock: Option<u32>,
    connected: bool,
    reader: Reader,
    opts: Options,
    /// bytes typed but not yet sent
    pending: Vec<u8>,
}

impl TelnetClient {
    fn new(host: &str, port: u16) -> TelnetClient {
        TelnetClient { host: String::from(host), port, addr: None, lookup: None, sock: None, connected: false, reader: Reader::new(true), opts: Options::new(&[], &[]), pending: Vec::new() }
    }

    fn connect(&mut self, os: &mut OsApi) {
        let Some(addr) = self.addr else {
            return;
        };
        match os.tcp_connect(addr, self.port, SockOpts::default()) {
            Ok(s) => self.sock = Some(s),
            Err(_) => {
                os.print("% Destination unreachable; gateway or host down\n");
                os.exit(1);
            }
        }
    }

    /// send a line to the remote as network-virtual-terminal text
    fn send_line(&mut self, os: &mut OsApi, line: &str) {
        if let Some(s) = self.sock.filter(|_| self.connected) {
            let mut b = telnet::to_nvt(line.as_bytes());
            b.extend_from_slice(b"\r\n");
            let n = os.tcp_send(s, &b);
            if n < b.len() {
                self.pending.extend_from_slice(&b[n..]);
            }
        }
    }

    /// the remote's bytes: answer the negotiations, print the text
    fn receive(&mut self, os: &mut OsApi, data: &[u8]) {
        let mut text = String::new();
        for item in self.reader.push(data) {
            match item {
                Item::Data(d) => text.push_str(&String::from_utf8_lossy(&d)),
                Item::Opt { verb, opt } => {
                    if let Some(ans) = self.opts.received(verb, opt)
                        && let Some(s) = self.sock
                    {
                        os.tcp_send(s, &ans);
                    }
                }
                _ => {}
            }
        }
        if !text.is_empty() {
            os.print(&text);
        }
    }
}

impl Service for TelnetClient {
    fn kind(&self) -> &'static str {
        "telnet"
    }
    fn start(&mut self, os: &mut OsApi) {
        match IpAddr::parse(self.host.trim_start_matches('[').trim_end_matches(']')) {
            Some(a) => {
                self.addr = Some(a);
                os.print(&format!("Trying {} ...\n", self.host));
                self.connect(os);
            }
            None => self.lookup = Some(os.resolve_host(&self.host, Family::Any)),
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Resolved(l) if Some(l) == self.lookup => {
                let all = os.lookup(l).and_then(|x| x.result.clone()).and_then(|r| r.ok()).map(|r| r.sorted).unwrap_or_default();
                match all.first() {
                    Some(&a) => {
                        self.addr = Some(a);
                        os.print(&format!("Trying {} ...\n", a));
                        self.connect(os);
                    }
                    None => {
                        os.print(&format!("% Bad IP address or host name: {}\n", self.host));
                        os.exit(1);
                    }
                }
            }
            SvcEvent::TcpConnected { sock, .. } if Some(sock) == self.sock => {
                self.connected = true;
                os.print("% Connected. The escape is Ctrl+Shift+6.\n");
                os.note(Note::Aaa(ANote::TelnetOut { to: self.addr.and_then(|a| a.v4()).unwrap_or(Ipv4::UNSPECIFIED), port: self.port }));
                if !self.pending.is_empty() {
                    let d = core::mem::take(&mut self.pending);
                    os.tcp_send(sock, &d);
                }
            }
            SvcEvent::TcpData { sock, .. } if Some(sock) == self.sock => {
                let d = os.tcp_recv(sock, usize::MAX);
                self.receive(os, &d);
            }
            SvcEvent::TcpWritable { sock } if Some(sock) == self.sock => {
                if !self.pending.is_empty() {
                    let d = core::mem::take(&mut self.pending);
                    let n = os.tcp_send(sock, &d);
                    self.pending.extend_from_slice(&d[n..]);
                }
            }
            SvcEvent::TcpEof { sock, .. } | SvcEvent::TcpClosed { sock, end: TcpEnd::Closed } if Some(sock) == self.sock => {
                let d = os.tcp_recv(sock, usize::MAX);
                self.receive(os, &d);
                os.print("\n% Connection closed by foreign host\n");
                os.tcp_close(sock);
                self.sock = None;
                os.exit(0);
            }
            SvcEvent::TcpClosed { sock, end } if Some(sock) == self.sock => {
                self.sock = None;
                let to = self.addr.and_then(|a| a.v4()).unwrap_or(Ipv4::UNSPECIFIED);
                if !self.connected {
                    let why = match end {
                        TcpEnd::Refused => "Connection refused by remote host",
                        TcpEnd::Unreachable => "Destination unreachable; gateway or host down",
                        _ => "Connection timed out; remote host not responding",
                    };
                    os.print(&format!("% {}\n", why));
                    os.note(Note::Aaa(ANote::TelnetRefused { to, port: self.port, why: String::from(why) }));
                } else {
                    os.print("\n% Connection closed by foreign host\n");
                }
                os.exit(1);
            }
            SvcEvent::Console(ConsoleEv::Line(l)) => self.send_line(os, &l),
            _ => {}
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        Vec::new()
    }
    fn status(&self) -> String {
        format!("telnet {} {}", self.host, self.port)
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

impl Engine {
    /// "telnet HOST [PORT]" in the Linux shell
    pub(crate) fn sh_telnet(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let pos: Vec<&str> = args.iter().copied().filter(|a| !a.starts_with('-')).collect();
        let Some(host) = pos.first() else {
            self.out(ni, "usage: telnet HOST [PORT]\n");
            return Ok(());
        };
        let port = pos.get(1).and_then(|p| p.parse::<u16>().ok()).unwrap_or(telnet::PORT);
        self.run_program(ni, "telnet", Box::new(TelnetClient::new(host, port)), true)
    }

    /// "telnet HOST [PORT]" in the IOS-like EXEC
    pub(crate) fn ios_telnet(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let host = String::from(p.text("host").unwrap_or(""));
        let port = p.num("port").map(|n| n as u16).unwrap_or(telnet::PORT);
        if host.is_empty() {
            return self.ios_err(ni, "% A host is needed\n");
        }
        self.run_program(ni, "telnet", Box::new(TelnetClient::new(&host, port)), true)
    }

    // ---------------------------------------------------------------- vty hooks (server: open)

    /// the config-line name of a vty requester (no vty sessions yet: empty)
    pub(crate) fn vty_line_name(&self, _ni: usize, _via: Requester) -> String {
        String::new()
    }
    /// a vty session waits for the NAS (no vty sessions yet)
    pub(crate) fn vty_set_waiting(&mut self, _ni: usize, _line: u16, _on: bool) {}
    /// a RADIUS outcome for a vty session (no vty sessions yet)
    pub(crate) fn vty_apply(&mut self, _ni: usize, _line: u16, _outcome: AuthResult) {}
    /// the "show users" lines of the vty sessions (none yet)
    pub(crate) fn vty_users_lines(&self, _ni: usize) -> String {
        String::new()
    }
}
