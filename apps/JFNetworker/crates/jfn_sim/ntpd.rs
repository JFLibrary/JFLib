// ntpd -- NTP client and server on the device clock (the on-wire protocol of RFC 5905 with a
// simple clock discipline): chrony or ntpd on Linux, "ntp server" and "ntp master" on IOS
//
//   Ntp::new(ios)            kind "ntp": `servers` to follow, `master` to serve the own clock
//                            at a stratum, `poll` as a power of two (6: 64 s)
//   NtpPeer                  per server: reach register, stratum, last offset and delay
//   Engine::ntp_peers(dev) / clock_state(dev) (os.rs)
//
// Client: every server gets a request right at the start and then every 2^poll seconds.  An
// answer (mode 4 with our transmit time as its origin, stratum 1 to 15, not unsynchronised)
// is a sample: offset ((T2 - T1) + (T3 - T4)) / 2 and delay (T4 - T1) - (T3 - T2).  Only the
// best server (lowest stratum, then lowest delay) sets the clock: the first sample, or one
// off by more than 128 ms, steps it; smaller offsets are slewed away over the next poll
// interval (at most 500 ppm), and from the second sample on the rate error is corrected
// (frequency += offset / interval).  The device is then synchronised at the server's stratum
// + 1.  The virtual network has no asymmetry unless a cable is impaired, so a sample is exact
// to the nanosecond; the drift of the device's oscillator (+-30 ppm) is what remains.
//
// Server: answers client requests (mode 3) while the clock is synchronised or the device is
// a master (IOS "ntp master", stratum 8 by default; reference "LOCL"); an unsynchronised
// device stays silent as chrony does without a reference.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use base::metrics::Counter;
use netpkt::ntp::{self, Ntp as Packet, NtpView, Ts};
use netpkt::syslog::SEV_NOTICE;
use netpkt::udp::PORT_NTP;
use netpkt::Ipv4;

use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{Ctl, Metric, OsApi, Service, SvcEvent};
use crate::trace::Note;

const K_POLL: u32 = 0;
/// above this offset the clock is stepped (ntpd's step threshold)
pub const STEP_LIMIT: i64 = 128 * MS as i64;

/// a server the client follows
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NtpPeer {
    pub addr: Ipv4,
    /// the last eight polls, newest in bit 0 (as "show ntp associations" shows reach in octal)
    pub reach: u8,
    pub stratum: u8,
    /// of the last sample (ns)
    pub offset: i64,
    pub delay: i64,
    /// when the last answer came (virtual time)
    pub last: Option<Ns>,
    /// our transmit time of the request on its way
    sent: Option<Ts>,
    sent_at: i64,
}

/// NTP of a device: client, server or both
#[derive(Clone, Debug)]
pub struct Ntp {
    pub ios: bool,
    pub peers: Vec<NtpPeer>,
    /// serve the own clock at this stratum (IOS "ntp master")
    pub master: Option<u8>,
    /// the poll interval as a power of two seconds
    pub poll: u8,
    /// the system peer (the server the clock follows)
    pub selected: Option<Ipv4>,
    samples: Counter,
    requests: Counter,
    served: Counter,
    steps: Counter,
    last_update: Option<Ns>,
}

impl Ntp {
    pub fn new(ios: bool) -> Ntp {
        Ntp { ios, peers: Vec::new(), master: None, poll: 6, selected: None, samples: Counter::new(), requests: Counter::new(), served: Counter::new(), steps: Counter::new(), last_update: None }
    }
    /// adds or removes a server
    pub fn set_server(&mut self, addr: Ipv4, add: bool) -> bool {
        let had = self.peers.iter().position(|p| p.addr == addr);
        match (had, add) {
            (None, true) => self.peers.push(NtpPeer { addr, reach: 0, stratum: 16, offset: 0, delay: 0, last: None, sent: None, sent_at: 0 }),
            (Some(i), false) => {
                self.peers.remove(i);
                if self.selected == Some(addr) {
                    self.selected = None;
                }
            }
            _ => return false,
        }
        true
    }
    fn interval(&self) -> Ns {
        (1u64 << self.poll.clamp(4, 17)) * SEC
    }

    fn poll_all(&mut self, os: &mut OsApi) {
        let now = os.now();
        for p in self.peers.iter_mut() {
            // a request without an answer counts as a miss
            p.reach <<= 1;
            let t1 = os.clock();
            let ts = Ts::from_unix_ns(t1);
            p.sent = Some(ts);
            p.sent_at = t1;
            let pk = Packet { poll: self.poll as i8, ..Packet::client(4, ts) };
            os.note(Note::NtpRequest { server: p.addr });
            os.send(PORT_NTP, p.addr, PORT_NTP, &pk.to_bytes());
            self.requests.inc();
        }
        if !self.peers.is_empty() {
            os.timer(K_POLL, now + self.interval());
        }
    }

    fn on_reply(&mut self, os: &mut OsApi, from: Ipv4, v: &NtpView) {
        let t4 = os.clock();
        let now = os.now();
        let Some(p) = self.peers.iter_mut().find(|p| p.addr == from) else {
            return;
        };
        // the answer must belong to our request (RFC 5905 8: the origin is our transmit time)
        if p.sent != Some(v.origin()) {
            return;
        }
        p.sent = None;
        p.reach |= 1;
        p.stratum = v.stratum();
        if v.stratum() == 0 || v.stratum() >= 16 || v.li() == ntp::LI_ALARM {
            return;
        }
        let (t1, t2, t3) = (p.sent_at, v.receive().to_unix_ns(), v.transmit().to_unix_ns());
        let (offset, delay) = ntp::offset_delay(t1, t2, t3, t4);
        p.offset = offset;
        p.delay = delay;
        p.last = Some(now);
        self.samples.inc();
        os.note(Note::NtpSample { server: from, offset, delay });
        // the best server sets the clock
        let best = self.peers.iter().filter(|q| q.last.is_some() && q.stratum < 16).min_by_key(|q| (q.stratum, q.delay)).map(|q| q.addr);
        if best != Some(from) {
            return;
        }
        let first = self.selected != Some(from) || self.last_update.is_none();
        let root = v.root_delay() as u64 * 1_000_000_000 >> 16;
        let interval = self.interval();
        let last = self.last_update;
        let ios = self.ios;
        let Some(c) = os.clock_mut() else {
            return;
        };
        let stepped = !c.synced || first || offset.abs() > STEP_LIMIT;
        if stepped {
            c.step(now, offset);
        } else {
            if let Some(prev) = last {
                let dt = now.saturating_sub(prev).max(1);
                let f = c.freq_ppb + ((offset as i128 * 1_000_000_000) / dt as i128) as i64;
                c.set_freq(now, f);
            }
            c.slew(now, offset, interval);
        }
        let was = c.synced;
        c.synced = true;
        c.stratum = v.stratum().saturating_add(1);
        c.source = Some(from);
        c.updated = Some(now);
        c.root_delay = root + delay.max(0) as u64;
        self.last_update = Some(now);
        self.selected = Some(from);
        if stepped {
            self.steps.inc();
            os.note(Note::NtpStep { offset });
        } else {
            os.note(Note::NtpSlew { offset });
        }
        if !was {
            if ios {
                os.log(SEV_NOTICE, "%NTP-5-PEERSYNC", &format!("NTP synced to peer {}", from));
            } else {
                os.log(SEV_NOTICE, "chronyd", &format!("Selected source {}", from));
            }
        }
    }

    fn on_request(&mut self, os: &mut OsApi, from: Ipv4, sport: u16, v: &NtpView) {
        let now = os.now();
        let t = os.clock();
        let Some(c) = os.clock_ref().cloned() else {
            return;
        };
        let (stratum, refid, reference) = match (c.synced && c.source.is_some(), self.master) {
            (true, _) => (c.stratum, c.source.map(|a| a.0).unwrap_or([0; 4]), c.updated.map(|u| t - (now.saturating_sub(u)) as i64).unwrap_or(t)),
            (false, Some(m)) => (m, *b"LOCL", t),
            (false, None) => {
                os.note(Note::NtpNoTime { to: from });
                return;
            }
        };
        let root_delay = ((c.root_delay as u128) << 16) / 1_000_000_000;
        let r = Packet {
            li: ntp::LI_NONE,
            vn: v.vn().clamp(1, 4),
            mode: ntp::MODE_SERVER,
            stratum,
            poll: v.poll(),
            precision: -20,
            root_delay: root_delay.min(u32::MAX as u128) as u32,
            root_disp: 1 << 10,
            refid,
            reference: Ts::from_unix_ns(reference),
            origin: v.transmit(),
            receive: Ts::from_unix_ns(t),
            transmit: Ts::from_unix_ns(t),
        };
        os.note(Note::NtpServed { to: from, stratum });
        os.send(PORT_NTP, from, sport, &r.to_bytes());
        self.served.inc();
    }
}

impl Service for Ntp {
    fn kind(&self) -> &'static str {
        "ntp"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.selected = None;
        self.last_update = None;
        for p in self.peers.iter_mut() {
            p.reach = 0;
            p.sent = None;
            p.last = None;
        }
        if os.bind(PORT_NTP, None).is_err() {
            os.note(Note::PortInUse { port: PORT_NTP, svc: String::from("ntp") });
            return;
        }
        if let Some(m) = self.master
            && let Some(c) = os.clock_mut()
            && c.source.is_none()
        {
            // the own clock is the reference
            c.synced = true;
            c.stratum = m;
        }
        self.poll_all(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Udp(d) => {
                let Some(v) = NtpView::parse(&d.data) else {
                    return;
                };
                match v.mode() {
                    ntp::MODE_SERVER => self.on_reply(os, d.src, &v),
                    ntp::MODE_CLIENT => self.on_request(os, d.src, d.sport, &v),
                    _ => {}
                }
            }
            SvcEvent::Timer(K_POLL) => self.poll_all(os),
            SvcEvent::Ctl(Ctl::Renew) => self.poll_all(os),
            _ => {}
        }
    }
    fn stop(&mut self, os: &mut OsApi) {
        if let Some(c) = os.clock_mut() {
            c.synced = false;
            c.stratum = 16;
            c.source = None;
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        let sel = self.peers.iter().find(|p| Some(p.addr) == self.selected);
        alloc::vec![
            Metric::new("requests", self.requests.get() as i64),
            Metric::new("samples", self.samples.get() as i64),
            Metric::new("served", self.served.get() as i64),
            Metric::new("steps", self.steps.get() as i64),
            Metric::new("synced", self.selected.is_some() as i64),
            Metric::new("offset_ns", sel.map(|p| p.offset).unwrap_or(0)),
            Metric::new("delay_ns", sel.map(|p| p.delay).unwrap_or(0)),
        ]
    }
    fn status(&self) -> String {
        match (self.selected, self.master) {
            (Some(s), _) => format!("synchronisiert mit {}", s),
            (None, Some(m)) => format!("eigene Uhr, Stratum {}", m),
            (None, None) => String::from("nicht synchronisiert"),
        }
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
    /// the NTP servers a device follows ("show ntp associations", "chronyc sources")
    pub fn ntp_peers(&self, dev: u32) -> Vec<NtpPeer> {
        self.service_of::<Ntp>(dev).map(|n| n.peers.clone()).unwrap_or_default()
    }
}
