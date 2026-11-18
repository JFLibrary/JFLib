// browser -- the browser test (a web build without arguments): against the server it came from
// (nettest --serve), through plat's WebSocket and the channel over it.  It runs as the page's
// app without a window: the host's ticks and the WebSocket's wake-ups drive one step after
// another; the lines go to stdout, which the page writes to its console (tests/web.mjs reads
// them).
//
//   open "/ws"; text and binary echo; 200 000 bytes from the server in frames of 1000; 500 round
//   trips of 32 bytes; 16 MiB from the server; 8 MiB echoed; the close; a host the permissions do
//   not name; the channel over "/chan": connect, 300 round trips, Delivered; the end

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netchan::endpoint::{ClientConfig, ClientEvent};
use netchan::{Client, Endpoint};
use plat::net::{NetErrorKind, WebSocket, WsMessage, WsState};

use crate::{median_p99, out, pattern, Rng, PROTOCOL};

const RTTS: usize = 500;
const CHAN_RTTS: usize = 300;
const STEP_US: u64 = 15_000_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Open,
    Text,
    Binary,
    Frag,
    Rtt,
    Blast,
    Bulk,
    Close,
    Denied,
    ChanOpen,
    ChanRtt,
    Done,
}

struct Test {
    step: Step,
    since: u64,
    ws: Option<WebSocket>,
    chan: Option<Client>,
    sent_at: u64,
    i: usize,
    got: usize,
    rtts: Vec<f64>,
    big: Vec<u8>,
    checks: u32,
    failed: u32,
    delivered: usize,
}

fn say(s: &str) {
    out(&format!("nettest browser: {}\n", s));
}

impl Test {
    fn check(&mut self, ok: bool, what: &str) {
        self.checks += 1;
        if !ok {
            self.failed += 1;
            say(&format!("FAIL {}", what));
        }
    }

    fn next(&mut self, step: Step) {
        self.step = step;
        self.since = plat::time_us();
        self.i = 0;
        self.got = 0;
        self.rtts.clear();
    }

    fn late(&mut self) -> bool {
        if plat::time_us() - self.since > STEP_US {
            let s = self.step;
            self.check(false, &format!("{:?}: nothing within 15 s", s));
            return true;
        }
        false
    }

    fn send_bin(&self, b: &[u8]) {
        if let Some(w) = &self.ws {
            let _ = w.send_binary(b);
        }
    }

    fn recv(&self) -> Option<WsMessage> {
        self.ws.as_ref().and_then(|w| w.recv())
    }

    // one step as far as it goes now -> whether it moved on
    fn work(&mut self) -> bool {
        let before = self.step;
        match self.step {
            Step::Open => {
                if self.ws.is_none() {
                    match WebSocket::connect("/ws") {
                        Ok(w) => self.ws = Some(w),
                        Err(e) => {
                            self.check(false, &format!("connect /ws: {}", e));
                            self.next(Step::Denied);
                            return true;
                        }
                    }
                }
                let st = self.ws.as_ref().map(|w| w.state()).unwrap_or(WsState::Closed);
                if st == WsState::Open {
                    say("open /ws");
                    let _ = self.ws.as_ref().map(|w| w.send_text("hallo Würfel ✓"));
                    self.next(Step::Text);
                } else if st == WsState::Closed || self.late() {
                    let info = self.ws.as_ref().and_then(|w| w.close_info());
                    self.check(false, &format!("open /ws: {:?}", info));
                    self.next(Step::Denied);
                }
            }
            Step::Text => {
                if let Some(m) = self.recv() {
                    self.check(m == WsMessage::Text(String::from("hallo Würfel ✓")), "text echo");
                    self.big = Rng::new(5).bytes(100_000);
                    let b = self.big.clone();
                    self.send_bin(&b);
                    self.next(Step::Binary);
                } else if self.late() {
                    self.next(Step::Close);
                }
            }
            Step::Binary => {
                if let Some(m) = self.recv() {
                    let ok = m == WsMessage::Binary(self.big.clone());
                    self.check(ok, "100 000 bytes binary echo");
                    let _ = self.ws.as_ref().map(|w| w.send_text("frag:200000"));
                    self.next(Step::Frag);
                } else if self.late() {
                    self.next(Step::Close);
                }
            }
            Step::Frag => {
                if let Some(m) = self.recv() {
                    self.check(m == WsMessage::Binary(pattern(200_000)), "200 000 bytes from the server in frames of 1000");
                    self.next(Step::Rtt);
                    self.sent_at = plat::time_us();
                    self.send_bin(&[0u8; 32]);
                } else if self.late() {
                    self.next(Step::Close);
                }
            }
            Step::Rtt => {
                while let Some(m) = self.recv() {
                    if !matches!(m, WsMessage::Binary(ref b) if b.len() == 32) {
                        continue;
                    }
                    let now = plat::time_us();
                    self.rtts.push((now - self.sent_at) as f64 / 1000.0);
                    self.i += 1;
                    if self.i == RTTS {
                        let (m, p) = median_p99(&mut self.rtts);
                        say(&format!("websocket rtt 32 B: median {:.3} ms, p99 {:.3} ms ({} round trips)", m, p, RTTS));
                        let _ = self.ws.as_ref().map(|w| w.send_text("blast:256:65536"));
                        self.next(Step::Blast);
                        return true;
                    }
                    self.sent_at = plat::time_us();
                    self.send_bin(&[0u8; 32]);
                }
                if self.late() {
                    self.next(Step::Close);
                }
            }
            Step::Blast => {
                while let Some(m) = self.recv() {
                    if let WsMessage::Binary(b) = m {
                        self.got += b.len();
                    }
                }
                if self.got >= 256 << 16 {
                    let s = (plat::time_us() - self.since) as f64 / 1e6;
                    say(&format!("websocket server -> browser: {:.1} MB/s ({} MiB in messages of 64 KiB)", self.got as f64 / s / 1e6, self.got >> 20));
                    self.check(self.got == 256 << 16, "16 MiB from the server");
                    let b = alloc::vec![3u8; 65536];
                    for _ in 0..128 {
                        self.send_bin(&b);
                    }
                    self.next(Step::Bulk);
                } else if self.late() {
                    self.next(Step::Close);
                }
            }
            Step::Bulk => {
                while let Some(m) = self.recv() {
                    if let WsMessage::Binary(b) = m {
                        self.got += b.len();
                    }
                }
                if self.got >= 128 << 16 {
                    let s = (plat::time_us() - self.since) as f64 / 1e6;
                    say(&format!("websocket browser -> server -> browser: {:.1} MB/s each way ({} MiB echoed)", self.got as f64 / s / 1e6, self.got >> 20));
                    if let Some(w) = &self.ws {
                        w.close(1000, "fertig");
                    }
                    self.next(Step::Close);
                } else if self.late() {
                    self.next(Step::Close);
                }
            }
            Step::Close => {
                let st = self.ws.as_ref().map(|w| w.state()).unwrap_or(WsState::Closed);
                if st == WsState::Open {
                    if let Some(w) = &self.ws {
                        w.close(1000, "fertig");
                    }
                }
                if st == WsState::Closed {
                    let info = self.ws.as_ref().and_then(|w| w.close_info());
                    self.check(info.as_ref().is_some_and(|i| i.0 == 1000), &format!("close 1000: {:?}", info));
                    self.check(self.ws.as_ref().is_some_and(|w| w.send_text("late").is_err()), "no send after the close");
                    self.ws = None;
                    self.next(Step::Denied);
                } else if self.late() {
                    self.ws = None;
                    self.next(Step::Denied);
                }
            }
            Step::Denied => {
                let r = WebSocket::connect("ws://example.org/");
                self.check(matches!(r, Err(ref e) if e.kind == NetErrorKind::Denied), "a host the permissions do not name is refused");
                let r = WebSocket::connect("http://localhost/");
                self.check(r.is_err(), "http:// is no WebSocket URL");
                say("permissions checked");
                match Client::connect(Endpoint::WebSocket(String::from("/chan")), ClientConfig { protocol: PROTOCOL, ..ClientConfig::default() }) {
                    Ok(c) => self.chan = Some(c),
                    Err(e) => self.check(false, &format!("channel connect: {}", e)),
                }
                let udp = Client::connect(Endpoint::Udp(plat::net::SocketAddr::new(plat::net::IpAddr::LOCALHOST_V4, 1)), ClientConfig::default());
                self.check(matches!(udp, Err(netchan::Error::Unsupported)), "no UDP in the browser");
                self.next(if self.chan.is_some() { Step::ChanOpen } else { Step::Done });
            }
            Step::ChanOpen => {
                let mut up = false;
                let mut end = None;
                if let Some(c) = &mut self.chan {
                    while let Some(e) = c.poll() {
                        match e {
                            ClientEvent::Connected => up = true,
                            ClientEvent::Disconnected(r) => end = Some(r),
                            _ => {}
                        }
                    }
                }
                if up {
                    let rel = self.chan.as_ref().is_some_and(|c| c.reliable());
                    self.check(rel, "the channel over the WebSocket is reliable");
                    say("channel connected over /chan");
                    self.next(Step::ChanRtt);
                    self.sent_at = plat::time_us();
                    if let Some(c) = &mut self.chan {
                        let _ = c.send(b"rtt 0");
                    }
                } else if let Some(r) = end {
                    self.check(false, &format!("channel: {:?}", r));
                    self.next(Step::Done);
                } else if self.late() {
                    self.next(Step::Done);
                }
            }
            Step::ChanRtt => {
                let mut events = Vec::new();
                if let Some(c) = &mut self.chan {
                    while let Some(e) = c.poll() {
                        events.push(e);
                    }
                }
                for e in events {
                    match e {
                        ClientEvent::Message { data, .. } => {
                            if data != format!("rtt {}", self.i).as_bytes() {
                                continue;
                            }
                            let now = plat::time_us();
                            self.rtts.push((now - self.sent_at) as f64 / 1000.0);
                            self.i += 1;
                            if self.i == CHAN_RTTS {
                                continue;
                            }
                            self.sent_at = plat::time_us();
                            let msg = format!("rtt {}", self.i);
                            if let Some(c) = &mut self.chan {
                                let _ = c.send(msg.as_bytes());
                            }
                        }
                        ClientEvent::Delivered(_) => self.delivered += 1,
                        ClientEvent::Disconnected(r) => {
                            self.check(false, &format!("channel ended: {:?}", r));
                            self.next(Step::Done);
                            return true;
                        }
                        _ => {}
                    }
                }
                if self.i >= CHAN_RTTS {
                    let (m, p) = median_p99(&mut self.rtts);
                    say(&format!("channel rtt over websocket: median {:.3} ms, p99 {:.3} ms ({} round trips)", m, p, CHAN_RTTS));
                    self.check(self.delivered >= CHAN_RTTS / 2, &format!("Delivered for the channel's packets: {}", self.delivered));
                    if let Some(c) = &mut self.chan {
                        c.disconnect();
                    }
                    self.next(Step::Done);
                } else if self.late() {
                    self.next(Step::Done);
                }
            }
            Step::Done => {
                if self.since != 0 {
                    say(&format!("done: {} checks, {} failed", self.checks, self.failed));
                    self.since = 0;
                    self.chan = None;
                }
            }
        }
        self.step != before
    }
}

impl plat::web::WebApplication for Test {
    fn frame_delay(&self) -> u32 {
        if self.step == Step::Done { 250 } else { 0 }
    }

    fn tick(&mut self, _time_ms: u32) {
        for _ in 0..32 {
            if !self.work() {
                break;
            }
        }
    }

    fn control(&mut self, _line: &[u8]) -> Option<String> {
        None
    }
}

/// install the test as the page's app
pub fn start() {
    say("start");
    let mut t = Test { step: Step::Open, since: plat::time_us(), ws: None, chan: None, sent_at: 0, i: 0, got: 0, rtts: Vec::new(), big: Vec::new(), checks: 0, failed: 0, delivered: 0 };
    // the page's own server needs no net.connect rule (the user's decision of 2026-11-18): who counts as
    // it, and the app cannot move it with setenv
    use plat::net::{is_page_server, page_origin};
    let origin = page_origin();
    t.check(origin.is_some(), "the worker reported the page's origin");
    t.check(
        is_page_server(Some("http://localhost:8080"), "localhost", 8080) && is_page_server(Some("https://game.example.org"), "game.example.org", 443),
        "the page's own server: the origin's host and port",
    );
    t.check(
        !is_page_server(Some("http://localhost:8080"), "localhost", 8081)
            && !is_page_server(Some("http://localhost:8080"), "example.org", 8080)
            && !is_page_server(None, "localhost", 8080),
        "another host or port is not the page's server",
    );
    plat::setenv("jfgui_PAGE_ORIGIN", Some("http://evil.example"));
    t.check(page_origin() == origin, "the app's setenv does not move the page's origin");
    plat::setenv("jfgui_PAGE_ORIGIN", origin.as_deref());
    plat::web::install(Box::new(t));
}
