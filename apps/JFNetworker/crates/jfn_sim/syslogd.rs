// syslogd -- a syslog server (rsyslog with imudp, a log collector): receives messages on UDP
// 514 and keeps the last ones with their sender; the sending side is the log of every device
// (os.rs: "logging host" on IOS, "*.* @host" on Linux)
//
//   SyslogServer::new()          kind "syslog-server"; keeps `max` messages (1000)
//   SyslogMsg { t, from, pri, text }
//   Engine::syslog_messages(dev)

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use netpkt::syslog;
use netpkt::udp::PORT_SYSLOG;
use netpkt::Ipv4;

use crate::kernel::{Engine, Ns};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::trace::Note;

/// a message as received
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyslogMsg {
    pub t: Ns,
    pub from: Ipv4,
    pub pri: u8,
    /// everything after "<PRI>"
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct SyslogServer {
    pub max: usize,
    pub msgs: VecDeque<SyslogMsg>,
    /// messages per severity (0 emerg .. 7 debug)
    pub by_sev: [u64; 8],
    bad: u64,
}

impl Default for SyslogServer {
    fn default() -> SyslogServer {
        SyslogServer::new()
    }
}

impl SyslogServer {
    pub fn new() -> SyslogServer {
        SyslogServer { max: 1000, msgs: VecDeque::new(), by_sev: [0; 8], bad: 0 }
    }
}

impl Service for SyslogServer {
    fn kind(&self) -> &'static str {
        "syslog-server"
    }
    fn start(&mut self, os: &mut OsApi) {
        if os.bind(PORT_SYSLOG, None).is_err() {
            os.note(Note::PortInUse { port: PORT_SYSLOG, svc: String::from("syslog-server") });
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        let SvcEvent::Udp(d) = ev else {
            return;
        };
        let Some(m) = syslog::parse(&d.data) else {
            self.bad += 1;
            return;
        };
        let sev = syslog::severity_of(m.pri);
        self.by_sev[sev as usize] += 1;
        if self.msgs.len() >= self.max.max(1) {
            self.msgs.pop_front();
        }
        self.msgs.push_back(SyslogMsg { t: os.now(), from: d.src, pri: m.pri, text: m.rest });
        os.note(Note::SyslogStored { from: d.src, sev });
    }
    fn metrics(&self) -> Vec<Metric> {
        let total: u64 = self.by_sev.iter().sum();
        alloc::vec![
            Metric::new("messages", total as i64),
            Metric::new("errors", self.by_sev[..4].iter().sum::<u64>() as i64),
            Metric::new("warnings", self.by_sev[4] as i64),
            Metric::new("malformed", self.bad as i64),
        ]
    }
    fn status(&self) -> String {
        format!("{} Meldungen", self.by_sev.iter().sum::<u64>())
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
    /// what a device's syslog server received, oldest first
    pub fn syslog_messages(&self, dev: u32) -> Vec<SyslogMsg> {
        self.service_of::<SyslogServer>(dev).map(|s| s.msgs.iter().cloned().collect()).unwrap_or_default()
    }
}
