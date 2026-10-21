// print -- ping and traceroute as console commands: they start a job of the engine (jobs.rs)
// and print its results while they come in: IOS-like (the first lines in our own words) and
// the way iputils / traceroute print them
//
//   IOS ping         "Ctrl+Shift+6 stops the ping." ... "!", ".", "U", "M", "&" (70 a line),
//                    "Success rate is 80 percent (4/5), round-trip min/avg/max = 1/2/4 ms";
//                    5 echoes of 100 bytes, timeout 2 s, the next when the previous is settled;
//                    without a route every probe waits its timeout (".....")
//   IOS traceroute   "Route to ..., up to 30 hops:" with "  1 10.0.1.1 0 msec 0 msec 0 msec",
//                    timeouts " * ", another answering address on a new line
//   Linux ping       "PING ... 56(84) bytes of data.", a line per reply or error, statistics
//                    with loss, time, rtt min/avg/max/mdev (iputils' arithmetic in µs); runs
//                    until Ctrl+C unless -c; without a route "ping: connect: Network is
//                    unreachable"
//   Linux traceroute "traceroute to ... 30 hops max, 60 byte packets", a line per hop
//   Proc, Top        console programs (curl, nc) and top keep the console busy the same way
//                    (cli/tools.rs): a program ends through OsApi::exit, top draws a screen
//                    whenever the console wakes
//
// Times are the engine's virtual round trips: IOS truncates them to milliseconds, iputils
// prints microseconds.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netpkt::Ipv4;

use super::fmt::{iputils_time, isqrt, ms3, pct_g};
use super::CliJob;
use crate::jobs::{IcmpKind, JobKind, PingEv, PingOpts, TraceOpts};
use crate::kernel::{Engine, Ns, MS, SEC};

/// how a console job prints and how far it got
#[derive(Clone, Debug)]
pub enum JobOut {
    IosPing {
        /// characters on the current line of "!" and "."
        col: usize,
        seen: usize,
        /// probes still to time out of a ping without a route
        fake: u32,
        count: u32,
        timeout: Ns,
    },
    LinuxPing {
        seen: usize,
        sent: u32,
        last_send: Ns,
    },
    IosTrace {
        /// hops and probes printed
        hop: usize,
        probe: usize,
        last: Option<Ipv4>,
        /// something is on the current hop line
        wrote: bool,
    },
    LinuxTrace {
        hop: usize,
        numeric: bool,
    },
    /// a console program (curl, nc: cli/tools.rs), the service it runs as; `stdin`: typed
    /// lines go to it
    Proc {
        svc: u32,
        stdin: bool,
    },
    /// top: screens still to show (0: until Ctrl+C), the time between two, batch mode
    Top {
        left: u32,
        delay: Ns,
        batch: bool,
    },
}

/// the mark of an IOS ping probe
fn ios_mark(e: &PingEv) -> char {
    match e {
        PingEv::Reply { .. } => '!',
        PingEv::Timeout { .. } => '.',
        PingEv::Error { kind, .. } => match kind {
            IcmpKind::NetUnreachable | IcmpKind::HostUnreachable | IcmpKind::ProtoUnreachable | IcmpKind::PortUnreachable | IcmpKind::Prohibited => 'U',
            IcmpKind::FragNeeded(_) => 'M',
            IcmpKind::TtlExceeded | IcmpKind::ReasmExceeded => '&',
            IcmpKind::NoRoute => '.',
            IcmpKind::Other(..) => '?',
        },
    }
}

/// the annotation of a traceroute probe that was answered with an error (Linux and IOS)
fn trace_flag(k: Option<IcmpKind>) -> &'static str {
    match k {
        Some(IcmpKind::HostUnreachable) => " !H",
        Some(IcmpKind::NetUnreachable) => " !N",
        Some(IcmpKind::ProtoUnreachable) => " !P",
        Some(IcmpKind::Prohibited) => " !X",
        Some(IcmpKind::FragNeeded(_)) => " !F",
        _ => "",
    }
}

/// "Success rate is ..." of IOS
fn ios_summary(events: &[PingEv], sent: u32) -> String {
    let rtts: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            PingEv::Reply { rtt, .. } => Some(*rtt / MS),
            _ => None,
        })
        .collect();
    let rx = rtts.len() as u32;
    let pct = if sent == 0 { 0 } else { rx * 100 / sent };
    let mut s = format!("Success rate is {} percent ({}/{})", pct, rx, sent);
    if rx > 0 {
        let min = rtts.iter().min().copied().unwrap_or(0);
        let max = rtts.iter().max().copied().unwrap_or(0);
        let avg = rtts.iter().sum::<u64>() / rx as u64;
        s.push_str(&format!(", round-trip min/avg/max = {}/{}/{} ms", min, avg, max));
    }
    s.push('\n');
    s
}

/// the statistics iputils prints at the end
fn linux_summary(dst: Ipv4, events: &[PingEv], sent: u32, errors: u32, time: Ns) -> String {
    let us: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            PingEv::Reply { rtt, .. } => Some(*rtt / 1000),
            _ => None,
        })
        .collect();
    let rx = us.len() as u64;
    let mut s = format!("\n--- {} ping statistics ---\n{} packets transmitted, {} received", dst, sent, rx);
    if errors > 0 {
        s.push_str(&format!(", +{} errors", errors));
    }
    if sent > 0 {
        s.push_str(&format!(", {}% packet loss, time {}ms", pct_g(sent as u64 - rx.min(sent as u64), sent as u64), (time + 500_000) / 1_000_000));
    }
    s.push('\n');
    if rx > 0 {
        let sum: u128 = us.iter().map(|&x| x as u128).sum();
        let sum2: u128 = us.iter().map(|&x| (x as u128) * (x as u128)).sum();
        let total = rx as u128;
        let avg = sum / total;
        let var = if sum < i32::MAX as u128 { (sum2 - (sum * sum) / total) / total } else { sum2 / total - avg * avg };
        let mdev = isqrt(var) as u64;
        let min = us.iter().min().copied().unwrap_or(0);
        let max = us.iter().max().copied().unwrap_or(0);
        s.push_str(&format!("rtt min/avg/max/mdev = {}/{}/{}/{} ms\n", ms3(min), ms3(avg as u64), ms3(max), ms3(mdev)));
    }
    s
}

impl Engine {
    /// IOS "ping": 100-byte echoes by default, the next when the previous is settled
    pub(crate) fn ios_ping_start(&mut self, ni: usize, dst: Ipv4, count: u32, bytes: u16, timeout: Ns, src: Option<Ipv4>, df: bool) {
        let dev = self.dev_of(ni);
        let mut head = format!("Ctrl+Shift+6 stops the ping.\nPinging {} with {} echo request{} of {} bytes, {} s timeout each:\n", dst, count, if count == 1 { "" } else { "s" }, bytes, timeout / SEC);
        if let Some(s) = src {
            head.push_str(&format!("Source address: {}\n", s));
        }
        if df {
            head.push_str("Fragmentation forbidden (DF bit set)\n");
        }
        self.out(ni, &head);
        let o = PingOpts { count, size: bytes.saturating_sub(28), ttl: None, df, interval: 0, timeout, src };
        let Ok(id) = self.ping(dev, dst, o) else {
            return;
        };
        // no route at all: the probes never leave, each one waits its timeout
        let no_route = self.job(id).is_some_and(|j| j.done && matches!(&j.kind, JobKind::Ping(p) if p.events.first() == Some(&PingEv::Error { seq: 1, from: Ipv4::UNSPECIFIED, kind: IcmpKind::NoRoute })));
        let out = if no_route { JobOut::IosPing { col: 0, seen: 1, fake: count, count, timeout } } else { JobOut::IosPing { col: 0, seen: 0, fake: 0, count, timeout } };
        self.cli_start_job(ni, id, out);
        if no_route {
            let at = self.now + timeout;
            self.cli_wake_at(ni, at);
        }
    }

    /// IOS "traceroute"
    pub(crate) fn ios_trace_start(&mut self, ni: usize, dst: Ipv4, probes: u8, timeout: Ns, max: u8) {
        let dev = self.dev_of(ni);
        self.out(ni, &format!("Ctrl+Shift+6 stops the traceroute.\nRoute to {}, up to {} hops:\n", dst, max));
        let o = TraceOpts { max_hops: max, probes, timeout, port: 33434, size: 0 };
        let Ok(id) = self.traceroute(dev, dst, o) else {
            return;
        };
        self.cli_start_job(ni, id, JobOut::IosTrace { hop: 0, probe: 0, last: None, wrote: false });
    }

    /// iputils "ping" (`opts.count` 0: until Ctrl+C)
    pub(crate) fn linux_ping_start(&mut self, ni: usize, dst: Ipv4, o: PingOpts) {
        let dev = self.dev_of(ni);
        let size = o.size;
        let Ok(id) = self.ping(dev, dst, o) else {
            return;
        };
        let no_route = self.job(id).is_some_and(|j| j.done && matches!(&j.kind, JobKind::Ping(p) if matches!(p.events.first(), Some(PingEv::Error { kind: IcmpKind::NoRoute, .. }))));
        if no_route {
            self.out(ni, "ping: connect: Network is unreachable\n");
            return;
        }
        self.out(ni, &format!("PING {} ({}) {}({}) bytes of data.\n", dst, dst, size, size as u32 + 28));
        let now = self.now;
        self.cli_start_job(ni, id, JobOut::LinuxPing { seen: 0, sent: 0, last_send: now });
    }

    /// Linux "traceroute"
    pub(crate) fn linux_trace_start(&mut self, ni: usize, dst: Ipv4, o: TraceOpts, numeric: bool) {
        let dev = self.dev_of(ni);
        self.out(ni, &format!("traceroute to {} ({}), {} hops max, {} byte packets\n", dst, dst, o.max_hops, o.size as u32 + 28));
        let Ok(id) = self.traceroute(dev, dst, o) else {
            return;
        };
        self.cli_start_job(ni, id, JobOut::LinuxTrace { hop: 0, numeric });
    }

    /// prints what the job of node `ni` has to say now
    pub(crate) fn job_progress(&mut self, ni: usize) {
        let Some(cj) = self.cdev(ni).and_then(|c| c.job.clone()) else {
            return;
        };
        let Some(job) = self.job(cj.id).cloned() else {
            return;
        };
        let now = self.now;
        let mut text = String::new();
        let mut out = cj.out.clone();
        let mut done = job.done;
        match (&mut out, &job.kind) {
            (JobOut::IosPing { col, seen, fake, .. }, JobKind::Ping(p)) => {
                if *fake > 0 {
                    return;
                }
                for e in &p.events[(*seen).min(p.events.len())..] {
                    if *col == 70 {
                        text.push('\n');
                        *col = 0;
                    }
                    text.push(ios_mark(e));
                    *col += 1;
                }
                *seen = p.events.len();
                if done {
                    text.push('\n');
                    text.push_str(&ios_summary(&p.events, p.sent));
                }
            }
            (JobOut::LinuxPing { seen, sent, last_send }, JobKind::Ping(p)) => {
                if p.sent > *sent {
                    *sent = p.sent;
                    *last_send = now;
                }
                for e in &p.events[(*seen).min(p.events.len())..] {
                    text.push_str(&self.linux_ping_line(ni, e));
                }
                *seen = p.events.len();
                if done {
                    text.push_str(&linux_summary(p.dst, &p.events, p.sent, p.errors, *last_send - cj.start));
                }
            }
            (JobOut::IosTrace { hop, probe, last, wrote }, JobKind::Trace(t)) => {
                // every probe not printed yet, in order; hops only grow at the end
                while let Some(h) = t.hops.get(*hop) {
                    if *probe >= h.probes.len() {
                        if *hop + 1 < t.hops.len() {
                            *hop += 1;
                            *probe = 0;
                            continue;
                        }
                        break;
                    }
                    if *probe == 0 {
                        if *hop > 0 {
                            text.push('\n');
                        }
                        text.push_str(&format!("{:>3} ", h.ttl));
                        *last = None;
                        *wrote = false;
                    }
                    let pr = h.probes[*probe];
                    match pr.from {
                        Some(a) if pr.kind != Some(IcmpKind::NoRoute) => {
                            if *last != Some(a) {
                                if *wrote {
                                    text.push_str("\n    ");
                                }
                                text.push_str(&format!("{}", a));
                                *last = Some(a);
                            }
                            text.push_str(&format!(" {} msec{}", pr.rtt / MS, trace_flag(pr.kind)));
                        }
                        _ => text.push_str(" * "),
                    }
                    *wrote = true;
                    *probe += 1;
                }
                if done {
                    text.push('\n');
                }
            }
            (JobOut::LinuxTrace { hop, numeric }, JobKind::Trace(t)) => {
                while *hop < t.hops.len() {
                    let h = &t.hops[*hop];
                    let complete = h.probes.len() >= t.opts.probes as usize || done;
                    if !complete {
                        break;
                    }
                    let mut l = format!("{:>2} ", h.ttl);
                    let mut last: Option<Ipv4> = None;
                    for pr in &h.probes {
                        match pr.from {
                            Some(a) if pr.kind != Some(IcmpKind::NoRoute) => {
                                if last != Some(a) {
                                    if *numeric {
                                        l.push_str(&format!(" {}", a));
                                    } else {
                                        l.push_str(&format!(" {} ({})", a, a));
                                    }
                                    last = Some(a);
                                }
                                l.push_str(&format!("  {} ms{}", ms3((pr.rtt + 500) / 1000), trace_flag(pr.kind)));
                            }
                            _ => l.push_str(" *"),
                        }
                    }
                    l.push('\n');
                    text.push_str(&l);
                    *hop += 1;
                }
            }
            _ => done = true,
        }
        if let Some(c) = self.cdev_mut(ni)
            && let Some(j) = c.job.as_mut()
        {
            j.out = out;
        }
        self.out(ni, &text);
        if done {
            self.cli_job_done(ni);
        }
    }

    /// a line of iputils ping for one probe
    fn linux_ping_line(&self, ni: usize, e: &PingEv) -> String {
        match *e {
            PingEv::Reply { seq, from, bytes, ttl, rtt } => format!("{} bytes from {}: icmp_seq={} ttl={}{}\n", bytes.saturating_sub(20), from, seq, ttl, iputils_time(rtt / 1000)),
            PingEv::Timeout { .. } => String::new(),
            PingEv::Error { seq, from, kind } => {
                let mine = self.stack(ni).is_some_and(|s| s.is_mine(from));
                match kind {
                    IcmpKind::FragNeeded(mtu) if mine => format!("ping: local error: message too long, mtu={}\n", mtu),
                    IcmpKind::FragNeeded(mtu) => format!("From {} icmp_seq={} Frag needed and DF set (mtu = {})\n", from, seq, mtu),
                    IcmpKind::NoRoute => String::from("ping: sendmsg: Network is unreachable\n"),
                    IcmpKind::HostUnreachable => format!("From {} icmp_seq={} Destination Host Unreachable\n", from, seq),
                    IcmpKind::NetUnreachable => format!("From {} icmp_seq={} Destination Net Unreachable\n", from, seq),
                    IcmpKind::PortUnreachable => format!("From {} icmp_seq={} Destination Port Unreachable\n", from, seq),
                    IcmpKind::ProtoUnreachable => format!("From {} icmp_seq={} Destination Protocol Unreachable\n", from, seq),
                    IcmpKind::Prohibited => format!("From {} icmp_seq={} Packet filtered\n", from, seq),
                    IcmpKind::TtlExceeded => format!("From {} icmp_seq={} Time to live exceeded\n", from, seq),
                    IcmpKind::ReasmExceeded => format!("From {} icmp_seq={} Frag reassembly time exceeded\n", from, seq),
                    IcmpKind::Other(t, c) => format!("From {} icmp_seq={} Unknown ICMP type {} code {}\n", from, seq, t, c),
                }
            }
        }
    }

    /// the timer of a console job (an IOS ping without a route: one "." per timeout)
    pub(crate) fn job_timer(&mut self, ni: usize) {
        let Some(mut cj) = self.cdev(ni).and_then(|c| c.job.clone()) else {
            return;
        };
        // a console program that may have ended, top's next screen (cli/tools.rs)
        match cj.out {
            JobOut::Proc { svc, .. } => {
                self.proc_wake(ni, svc);
                return;
            }
            JobOut::Top { .. } => {
                self.top_tick(ni);
                return;
            }
            _ => {}
        }
        let JobOut::IosPing { col, fake, count, timeout, .. } = &mut cj.out else {
            return;
        };
        if *fake == 0 {
            return;
        }
        let mut text = String::new();
        if *col == 70 {
            text.push('\n');
            *col = 0;
        }
        text.push('.');
        *col += 1;
        *fake -= 1;
        let (left, count, timeout) = (*fake, *count, *timeout);
        if left == 0 {
            text.push('\n');
            text.push_str(&ios_summary(&[], count));
        }
        if let Some(c) = self.cdev_mut(ni) {
            c.job = Some(cj);
        }
        self.out(ni, &text);
        if left == 0 {
            self.cli_job_done(ni);
        } else {
            let at = self.now + timeout;
            self.cli_wake_at(ni, at);
        }
    }

    /// Ctrl+C / Ctrl+Shift+6 stopped a console job (the engine's job is cancelled already)
    pub(crate) fn job_interrupted(&mut self, ni: usize, cj: CliJob) {
        let job = self.job(cj.id).cloned();
        let text = match (&cj.out, job.as_ref().map(|j| &j.kind)) {
            (JobOut::IosPing { fake, count, .. }, Some(JobKind::Ping(p))) => {
                if *fake > 0 {
                    // a ping without a route: the dots printed so far
                    format!("\n{}", ios_summary(&[], count - fake))
                } else {
                    format!("\n{}", ios_summary(&p.events, p.events.len() as u32))
                }
            }
            (JobOut::LinuxPing { last_send, sent, .. }, Some(JobKind::Ping(p))) => {
                let sent = (*sent).max(p.sent);
                linux_summary(p.dst, &p.events, sent, p.errors, *last_send - cj.start)
            }
            (JobOut::Proc { svc, .. }, _) => {
                // the program ends as a process does on SIGINT: its sockets close
                self.console_kill(ni, *svc);
                String::from("\n")
            }
            _ => String::from("\n"),
        };
        self.out(ni, &text);
        self.cli_state(ni);
    }
}
