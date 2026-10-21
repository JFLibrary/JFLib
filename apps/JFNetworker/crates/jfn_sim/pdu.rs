// pdu -- PDUs the user puts together (the PDU tool of the simulation mode): UDP datagrams and
// TCP connections with chosen ports, size and TTL, once or again and again in virtual time
//
//   PduSender::new(PduSpec)     a service: Engine::add_service(dev, name, Box::new(..), true)
//   PduSpec                     protocol (Udp, Tcp), destination (an IPv4 address, or a name
//                               the device's resolver looks up), source port (0: an ephemeral
//                               one), destination port, data bytes, TTL (UDP; None: the
//                               device's), how many (0: until it stops) and how often
//   sender.sent / replies / failed / error, done(), status()
//                               what came of it (the app's PDU list)
//   OsApi::send_ttl(..)         a UDP datagram with a TTL of its own
//   payload(n)                  the data bytes of a PDU ("abcdefghijklmnopqrstuvw" repeated)
//
// UDP: the datagrams leave from a bound port, so answers (an echo service, DNS) count as
// replies; a port nobody listens on answers with ICMP, which the event list shows.  TCP: every
// round opens a connection, writes `size` bytes, sends its FIN and reads until the peer closes;
// an orderly end counts as a reply, a refused, reset or unreachable one as failed.  ICMP echoes
// (Engine::ping / ping_host) and DNS questions (Engine::lookup) need no service: the app
// follows their jobs and lookups.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use netpkt::ipv4::{Ipv4Hdr, PROTO_UDP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::udp::UdpHdr;
use netpkt::Ipv4;

use crate::ip::Origin;
use crate::kernel::{Ns, MS};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::tcp::{SockOpts, TcpEnd};
use crate::trace::Note;

/// the timer of the next round
const K_NEXT: u32 = 0;
/// the most handed to tcp_send at once
const WRITE_MAX: u32 = 65536;

/// the data bytes of a PDU: the letters a to w over and over (readable in a hex view)
pub fn payload(n: u32) -> Vec<u8> {
    const ABC: &[u8; 23] = b"abcdefghijklmnopqrstuvw";
    (0..n as usize).map(|i| ABC[i % ABC.len()]).collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PduProto {
    Udp,
    Tcp,
}

/// what to send
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PduSpec {
    pub proto: PduProto,
    /// an IPv4 address or a name (the device's resolver looks it up)
    pub dst: String,
    /// 0: an ephemeral port
    pub sport: u16,
    pub dport: u16,
    /// data bytes per datagram or connection
    pub size: u32,
    /// UDP: the IPv4 TTL (None: the device's own)
    pub ttl: Option<u8>,
    /// datagrams or connections (0: until the service stops)
    pub count: u32,
    /// between two of them (at least 1 ms)
    pub interval: Ns,
}

/// a TCP connection of a round
#[derive(Clone, Debug)]
struct Conn {
    sock: u32,
    /// bytes still to write
    left: u32,
    /// the FIN went out
    shut: bool,
}

/// the service that sends a PDU (kind "pdu")
#[derive(Clone, Debug)]
pub struct PduSender {
    pub spec: PduSpec,
    /// the address it sends to (a name's after the lookup)
    pub dst: Option<Ipv4>,
    lookup: Option<u32>,
    /// UDP: the port it sends from
    port: Option<u16>,
    conns: Vec<Conn>,
    /// datagrams sent, connections opened
    pub sent: u32,
    /// datagrams back to its port, connections that ended in order
    pub replies: u32,
    /// datagrams without a route, connections that failed
    pub failed: u32,
    /// the last thing that went wrong (German)
    pub error: Option<String>,
    /// stopped by the user, the power or a removal
    pub stopped: bool,
    rounds: u32,
}

impl PduSender {
    pub fn new(spec: PduSpec) -> PduSender {
        PduSender { spec, dst: None, lookup: None, port: None, conns: Vec::new(), sent: 0, replies: 0, failed: 0, error: None, stopped: false, rounds: 0 }
    }

    /// nothing more will happen: every round ran and every connection ended, it was stopped,
    /// or it could not start (a name nobody knows, a port in use)
    pub fn done(&self) -> bool {
        let all = self.spec.count > 0 && self.rounds >= self.spec.count && self.conns.is_empty();
        let stuck = self.rounds == 0 && self.error.is_some();
        self.stopped || all || stuck
    }

    // the address is known: bind the UDP port, the first round
    fn begin(&mut self, os: &mut OsApi) {
        if self.spec.proto == PduProto::Udp {
            match os.bind(self.spec.sport, None) {
                Ok(p) => self.port = Some(p),
                Err(e) => {
                    self.error = Some(e);
                    return;
                }
            }
        }
        self.round(os);
    }

    // one datagram or connection; the timer for the next
    fn round(&mut self, os: &mut OsApi) {
        let Some(dst) = self.dst else { return };
        if self.stopped || (self.spec.count > 0 && self.rounds >= self.spec.count) {
            return;
        }
        self.rounds += 1;
        match self.spec.proto {
            PduProto::Udp => {
                let data = payload(self.spec.size);
                let port = self.port.unwrap_or(0);
                if os.send_ttl(port, dst, self.spec.dport, &data, self.spec.ttl) {
                    self.sent += 1;
                } else {
                    self.failed += 1;
                    self.error = Some(String::from("keine Route zum Ziel"));
                }
            }
            PduProto::Tcp => match os.tcp_connect_from(self.spec.sport, dst, self.spec.dport, SockOpts::default()) {
                Ok(sock) => {
                    self.conns.push(Conn { sock, left: self.spec.size, shut: false });
                    self.sent += 1;
                }
                Err(e) => {
                    self.failed += 1;
                    self.error = Some(e);
                }
            },
        }
        if self.spec.count == 0 || self.rounds < self.spec.count {
            let at = os.now() + self.spec.interval.max(MS);
            os.timer(K_NEXT, at);
        }
    }

    // TCP: write what is left of a connection's bytes, then its FIN
    fn pump(&mut self, os: &mut OsApi, sock: u32) {
        let Some(c) = self.conns.iter_mut().find(|c| c.sock == sock) else { return };
        while c.left > 0 {
            let want = c.left.min(WRITE_MAX);
            let done = self.spec.size - c.left;
            let data: Vec<u8> = payload(done + want).split_off(done as usize);
            let n = os.tcp_send(sock, &data) as u32;
            c.left -= n.min(c.left);
            if n < want {
                return; // TcpWritable follows
            }
        }
        if !c.shut {
            c.shut = true;
            os.tcp_shutdown(sock);
        }
    }

    // TCP: a connection is over
    fn ended(&mut self, os: &mut OsApi, sock: u32, end: TcpEnd) {
        let Some(i) = self.conns.iter().position(|c| c.sock == sock) else { return };
        let _ = os.tcp_recv(sock, usize::MAX);
        os.tcp_close(sock);
        let c = self.conns.remove(i);
        if end == TcpEnd::Closed && c.left == 0 {
            self.replies += 1;
        } else {
            self.failed += 1;
            self.error = Some(String::from(end.text()));
        }
    }
}

impl Service for PduSender {
    fn kind(&self) -> &'static str {
        "pdu"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.conns.clear();
        self.port = None;
        self.lookup = None;
        self.stopped = false;
        let dst = String::from(self.spec.dst.trim());
        match Ipv4::parse(&dst) {
            Some(ip) => {
                self.dst = Some(ip);
                self.begin(os);
            }
            None => self.lookup = Some(os.resolve(&dst, netpkt::dns::TYPE_A)),
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Resolved(id) if Some(id) == self.lookup => {
                self.lookup = None;
                let res = os.lookup(id).and_then(|l| l.result.clone());
                match res {
                    Some(Ok(a)) => match a.addrs.first() {
                        Some(&ip) => {
                            self.dst = Some(ip);
                            self.begin(os);
                        }
                        None => self.error = Some(format!("„{}“ hat keine IPv4-Adresse", self.spec.dst.trim())),
                    },
                    Some(Err(e)) => self.error = Some(format!("„{}“: {}", self.spec.dst.trim(), e.text())),
                    None => self.error = Some(format!("„{}“: keine Antwort", self.spec.dst.trim())),
                }
            }
            SvcEvent::Timer(K_NEXT) => self.round(os),
            SvcEvent::Udp(d) if Some(d.dport) == self.port => self.replies += 1,
            SvcEvent::TcpConnected { sock, .. } | SvcEvent::TcpWritable { sock } => self.pump(os, sock),
            SvcEvent::TcpData { sock, .. } => {
                let _ = os.tcp_recv(sock, usize::MAX);
            }
            SvcEvent::TcpEof { sock, .. } => self.ended(os, sock, TcpEnd::Closed),
            SvcEvent::TcpClosed { sock, end } => self.ended(os, sock, end),
            _ => {}
        }
    }
    fn stop(&mut self, _os: &mut OsApi) {
        // its sockets and timers go with it
        self.stopped = true;
        self.conns.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("sent", self.sent as i64),
            Metric::new("replies", self.replies as i64),
            Metric::new("failed", self.failed as i64),
            Metric::new("done", self.done() as i64),
        ]
    }
    fn status(&self) -> String {
        let what = match self.spec.proto {
            PduProto::Udp => "UDP",
            PduProto::Tcp => "TCP",
        };
        let to = format!("{} an {}:{}", what, self.spec.dst.trim(), self.spec.dport);
        match (&self.error, self.dst) {
            (Some(e), _) if self.sent == 0 => format!("{}: {}", to, e),
            (_, None) => format!("{}: fragt nach dem Namen …", to),
            _ => format!("{}: {} gesendet, {} Antworten, {} gescheitert", to, self.sent, self.replies, self.failed),
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

impl OsApi<'_> {
    /// sends a UDP datagram over the routing table with a TTL of its own (None: the device's);
    /// false without a route
    pub fn send_ttl(&mut self, sport: u16, dst: Ipv4, dport: u16, data: &[u8], ttl: Option<u8>) -> bool {
        let (ni, cause) = (self.ni, self.cause);
        let eng = &mut *self.eng;
        let dev = eng.dev_of(ni);
        let Some(src) = eng.source_for(ni, dst) else {
            eng.note(dev, cause, Note::IpNoRoute { dst });
            return false;
        };
        let Some(st) = eng.stack_mut(ni) else { return false };
        let (ttl, id) = (ttl.unwrap_or(st.ttl).max(1), st.next_id());
        let mut p = Pkt::new(HEADROOM, data);
        UdpHdr::new(sport, dport).push_v4(&mut p, src, dst);
        let mut h = Ipv4Hdr::new(src, dst, PROTO_UDP, ttl);
        h.id = id;
        h.push_onto(&mut p);
        eng.ip_output(ni, p.into_vec(), Origin::Local { job: None }, cause);
        true
    }
}
