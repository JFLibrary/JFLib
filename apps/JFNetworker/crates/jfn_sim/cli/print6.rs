// print6 -- ping and traceroute over IPv6 as console commands (jobs6.rs): IOS-like (the first
// lines in our own words, as for IPv4; addresses in capitals) and the way iputils / traceroute
// print IPv6
//
//   IOS ping         "!", ".", "U" (unreachable), "M" (packet too big), "&" (hop limit), "?"
//                    (70 a line) and "Success rate is ..."
//   IOS traceroute   "  1 2001:DB8:1::1 0 msec 0 msec 0 msec", " * " for no answer
//   Linux ping       "PING 2001:db8::2(2001:db8::2) 56 data bytes", "64 bytes from ...:
//                    icmp_seq=1 ttl=64 time=0.012 ms", "From ... icmp_seq=1 Destination
//                    unreachable: Address unreachable", the statistics of IPv4's ping
//   Linux traceroute "traceroute to ... 30 hops max, 80 byte packets" and a line per hop
//
// The jobs print through JobOut::Ping6 / JobOut::Trace6 (print.rs hands them here).  A ping or
// traceroute by name whose name gave an IPv6 address comes here too (print.rs: job_switch):
// "PING www.lab(2001:db8::2) 56 data bytes" names the address it found.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netpkt::Ipv6;

use super::fmt::{iputils_time, ms3};
use super::print::{ios_summary_ms, linux_summary_us, JobOut};
use super::CliJob;
use crate::jobs::JobKind;
use crate::jobs6::{Icmp6Kind, Ping6Ev};
use crate::kernel::{Engine, MS};

/// an IPv6 address as IOS prints it (capitals)
pub(crate) fn up6(a: Ipv6) -> String {
    format!("{}", a).to_uppercase()
}

/// the mark of an IOS ping probe
fn ios_mark(e: &Ping6Ev) -> char {
    match e {
        Ping6Ev::Reply { .. } => '!',
        Ping6Ev::Timeout { .. } => '.',
        Ping6Ev::Error { kind, .. } => match kind {
            Icmp6Kind::NoRoute | Icmp6Kind::Prohibited | Icmp6Kind::BeyondScope | Icmp6Kind::AddrUnreachable | Icmp6Kind::PortUnreachable | Icmp6Kind::Policy | Icmp6Kind::Reject => 'U',
            Icmp6Kind::TooBig(_) => 'M',
            Icmp6Kind::HopLimit | Icmp6Kind::Reassembly => '&',
            Icmp6Kind::LocalNoRoute => '.',
            Icmp6Kind::Param(..) | Icmp6Kind::Other(..) => '?',
        },
    }
}

/// the flag of a traceroute probe answered with an error (traceroute's IPv6 marks)
fn trace_flag(k: Option<Icmp6Kind>, ios: bool) -> String {
    String::from(match k {
        Some(Icmp6Kind::NoRoute) => " !N",
        Some(Icmp6Kind::Prohibited) if ios => " !A",
        Some(Icmp6Kind::Prohibited) => " !X",
        Some(Icmp6Kind::BeyondScope) => " !S",
        Some(Icmp6Kind::AddrUnreachable) => " !H",
        Some(Icmp6Kind::TooBig(_)) => " !F",
        Some(Icmp6Kind::Policy | Icmp6Kind::Reject) => " !X",
        _ => "",
    })
}

fn rtts(events: &[Ping6Ev]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            Ping6Ev::Reply { rtt, .. } => Some(*rtt),
            _ => None,
        })
        .collect()
}

impl Engine {
    /// prints what an IPv6 job of node `ni` has to say now (print.rs: job_progress)
    pub(crate) fn job6_progress(&mut self, ni: usize) {
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
            (JobOut::Ping6 { ios: true, col, seen, .. }, JobKind::Ping6(p)) => {
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
                    let ms: Vec<u64> = rtts(&p.events).iter().map(|r| r / MS).collect();
                    text.push_str(&ios_summary_ms(&ms, p.sent));
                }
            }
            (JobOut::Ping6 { ios: false, seen, sent, last_send, head, t0, shown, from, .. }, JobKind::Ping6(p)) => {
                if !*head {
                    let no_route = matches!(p.events.first(), Some(Ping6Ev::Error { kind: Icmp6Kind::LocalNoRoute, .. }));
                    if no_route && done {
                        text.push_str("ping: connect: Network is unreachable\n");
                    } else {
                        let via = match from {
                            Some(f) => format!(" from {} {}:", p.src, f),
                            None => String::new(),
                        };
                        // a name: its address in the parentheses (no reverse lookup gives one
                        // back); an address as typed, with its zone
                        let typed = Ipv6::parse(shown.split('%').next().unwrap_or("")).is_some();
                        let addr = if typed { shown.clone() } else { format!("{}", p.dst) };
                        text.push_str(&format!("PING {}({}){} {} data bytes\n", shown, addr, via, p.opts.size));
                        *head = true;
                        *t0 = now;
                        *last_send = now;
                    }
                }
                if *head {
                    if p.sent > *sent {
                        *sent = p.sent;
                        *last_send = now;
                    }
                    for e in &p.events[(*seen).min(p.events.len())..] {
                        text.push_str(&self.linux_ping6_line(ni, e));
                    }
                    *seen = p.events.len();
                    if done {
                        let us: Vec<u64> = rtts(&p.events).iter().map(|r| r / 1000).collect();
                        text.push_str(&linux_summary_us(shown, &us, p.sent, p.errors, *last_send - *t0));
                    }
                }
            }
            (JobOut::Trace6 { ios: true, hop, probe, last, wrote, .. }, JobKind::Trace6(t)) => {
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
                        Some(a) if pr.kind != Some(Icmp6Kind::LocalNoRoute) => {
                            if *last != Some(a) {
                                if *wrote {
                                    text.push_str("\n    ");
                                }
                                text.push_str(&up6(a));
                                *last = Some(a);
                            }
                            text.push_str(&format!(" {} msec{}", pr.rtt / MS, trace_flag(pr.kind, true)));
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
            (JobOut::Trace6 { ios: false, hop, numeric, head, shown, .. }, JobKind::Trace6(t)) => {
                if !*head {
                    text.push_str(&format!("traceroute to {} ({}), {} hops max, {} byte packets\n", shown, t.dst, t.opts.max_hops, t.opts.size as u32 + 48));
                    *head = true;
                }
                while *hop < t.hops.len() {
                    let h = &t.hops[*hop];
                    let complete = h.probes.len() >= t.opts.probes as usize || done;
                    if !complete {
                        break;
                    }
                    let mut l = format!("{:>2} ", h.ttl);
                    let mut last: Option<Ipv6> = None;
                    for pr in &h.probes {
                        match pr.from {
                            Some(a) if pr.kind != Some(Icmp6Kind::LocalNoRoute) => {
                                if last != Some(a) {
                                    if *numeric {
                                        l.push_str(&format!(" {}", a));
                                    } else {
                                        l.push_str(&format!(" {} ({})", a, a));
                                    }
                                    last = Some(a);
                                }
                                l.push_str(&format!("  {} ms{}", ms3((pr.rtt + 500) / 1000), trace_flag(pr.kind, false)));
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

    /// a line of iputils ping for one probe over IPv6
    fn linux_ping6_line(&self, ni: usize, e: &Ping6Ev) -> String {
        match *e {
            Ping6Ev::Reply { seq, from, bytes, hops, rtt } => format!("{} bytes from {}: icmp_seq={} ttl={}{}\n", bytes, from, seq, hops, iputils_time(rtt / 1000)),
            Ping6Ev::Timeout { .. } => String::new(),
            Ping6Ev::Error { seq, from, kind } => {
                let mine = from.is_unspecified() || self.v6(ni).is_some_and(|v| v.find(from).is_some());
                let what = match kind {
                    Icmp6Kind::TooBig(mtu) if mine => return format!("ping: local error: message too long, mtu={}\n", mtu),
                    Icmp6Kind::LocalNoRoute => return String::from("ping: sendmsg: Network is unreachable\n"),
                    Icmp6Kind::NoRoute => String::from("Destination unreachable: No route"),
                    Icmp6Kind::Prohibited => String::from("Destination unreachable: Administratively prohibited"),
                    Icmp6Kind::BeyondScope => String::from("Destination unreachable: Beyond scope of source address"),
                    Icmp6Kind::AddrUnreachable => String::from("Destination unreachable: Address unreachable"),
                    Icmp6Kind::PortUnreachable => String::from("Destination unreachable: Port unreachable"),
                    Icmp6Kind::Policy => String::from("Destination unreachable: Unknown code 5"),
                    Icmp6Kind::Reject => String::from("Destination unreachable: Unknown code 6"),
                    Icmp6Kind::TooBig(mtu) => format!("Packet too big: mtu={}", mtu),
                    Icmp6Kind::HopLimit => String::from("Time exceeded: Hop limit"),
                    Icmp6Kind::Reassembly => String::from("Time exceeded: Defragmentation failure"),
                    Icmp6Kind::Param(c, at) => format!(
                        "Parameter problem: {}at {}",
                        match c {
                            0 => String::from("Wrong header field "),
                            1 => String::from("Unknown header "),
                            2 => String::from("Unknown option "),
                            x => format!("code {} ", x),
                        },
                        at
                    ),
                    Icmp6Kind::Other(t, c) => format!("Unknown ICMP type {} code {}", t, c),
                };
                format!("From {} icmp_seq={} {}\n", from, seq, what)
            }
        }
    }

    /// what Ctrl+C prints of an IPv6 job (print.rs: job_interrupted)
    pub(crate) fn job6_interrupted(&self, cj: &CliJob) -> String {
        let job = self.job(cj.id);
        match (&cj.out, job.map(|j| &j.kind)) {
            (JobOut::Ping6 { ios: true, .. }, Some(JobKind::Ping6(p))) => {
                let ms: Vec<u64> = rtts(&p.events).iter().map(|r| r / MS).collect();
                format!("\n{}", ios_summary_ms(&ms, p.events.len() as u32))
            }
            (JobOut::Ping6 { ios: false, last_send, sent, head: true, t0, shown, .. }, Some(JobKind::Ping6(p))) => {
                let us: Vec<u64> = rtts(&p.events).iter().map(|r| r / 1000).collect();
                linux_summary_us(shown, &us, (*sent).max(p.sent), p.errors, *last_send - *t0)
            }
            _ => String::from("\n"),
        }
    }
}
