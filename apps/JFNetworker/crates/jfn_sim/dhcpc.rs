// dhcpc -- the DHCP client of an interface (RFC 2131): ISC dhclient on Linux, the client of
// "ip address dhcp" on IOS
//
//   DhcpClient::new(iface, ios)      a service of kind "dhcp-client" (one per interface)
//   ClientState                      Init, Selecting, Requesting, Checking (ARP probes after
//                                    the ACK), Bound, Renewing, Rebinding, Rebooting
//                                    (INIT-REBOOT), Declined (10 s after a conflict), Idle (no
//                                    offers: later again), Stopped
//   Lease                            address, mask, routers, name servers, domain, server, T1,
//                                    T2 and expiry in virtual time
//   Engine::dhcp_leases(dev)         the client of every interface: state, lease, log lines
//
// Linux (dhclient 4.4 with a Fedora-style dhclient-script): DISCOVER at once, then after 3 s;
// each further interval grows by a random 0 to 2 times itself in whole seconds, above 15 s
// (backoff-cutoff) it becomes 7 + rand mod 15, plus a random fraction of a second; no offer
// within 60 s (timeout): sleep 300 s (retry).  The first offer is taken (select-timeout 0),
// the REQUEST retransmits the same way (starting from 3 s, so 3 + rand mod 6).  After the ACK
// the script probes the address twice a second apart (arping -D) and binds 2 s after the ACK;
// an answer from another device declines it, 10 s later the client starts again.  The
// renewal time is randomised as dhclient does it: (3·T1 + 3)/4 + (rand mod T1 + 3)/4 seconds,
// T1 = option 58 or half the lease; T2 = option 59 or 7/8 of the lease.  RENEWING asks the
// server (unicast), REBINDING (after T2) everybody (broadcast); at the expiry the address
// goes.  The lease is kept like dhclient.leases: after a restart the client asks for the same
// address (INIT-REBOOT; after 10 s without an answer it starts over).  A release sends the
// RELEASE and the script removes the address 10 ms later.  No broadcast flag:
// dhclient reads unicast answers through its packet socket; it sends its host name and
// dhclient's parameter request list.
//
// IOS: the broadcast flag, the client identifier "cisco-<mac>-<interface>", RFC 2131
// retransmission (4, 8, 16, 32, 64 s, each +-1 s; while renewing half the time to T2, at least
// 60 s), no ARP check, the default route with distance 254, a new start when the interface
// comes up, and nothing kept over a restart.
//
// WLAN (wlan.rs): when a station's connection is up, the client hears of it (Ctl::Carrier, as a
// supplicant's hook starts dhclient): without a lease it starts its DISCOVER (or INIT-REBOOT)
// at once instead of waiting for its next retransmission.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use base::metrics::Counter;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::dhcp::{self, Dhcp, MsgType, Opt};
use netpkt::syslog::SEV_INFO;
use netpkt::udp::{PORT_DHCP_CLIENT, PORT_DHCP_SERVER};
use netpkt::{Ipv4, Mac, Prefix4};

use crate::ip::{Route, RouteSrc};
use crate::kernel::{Engine, Ns, SEC};
use crate::os::{Ctl, Metric, OsApi, Service, SvcEvent};
use crate::trace::{DhcpReq, Note};

const K_RETX: u32 = 0;
const K_T1: u32 = 1;
const K_T2: u32 = 2;
const K_EXP: u32 = 3;

/// dhclient's parameter request list (Debian's dhclient.conf)
pub const PRL_LINUX: [u8; 13] = [1, 28, 2, 3, 15, 6, 119, 12, 44, 47, 26, 121, 42];
/// the parameter request list of the IOS client
pub const PRL_IOS: [u8; 8] = [1, 6, 15, 44, 3, 33, 150, 43];

/// dhclient: initial-interval, backoff-cutoff, timeout, retry, reboot (s)
const INITIAL: u64 = 3;
const CUTOFF: u64 = 15;
const TIMEOUT: u64 = 60;
const RETRY: u64 = 300;
const REBOOT: u64 = 10;
/// waiting after a DECLINE before starting again (RFC 2131 3.1.5)
const DECLINE_WAIT: u64 = 10;
/// dhclient-script removes the address this long after the RELEASE was sent
const SCRIPT_DELAY: Ns = 10 * crate::kernel::MS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClientState {
    Stopped,
    Init,
    Selecting,
    Requesting,
    /// probing the address before it is used
    Checking,
    Bound,
    Renewing,
    Rebinding,
    Rebooting,
    Declined,
    Idle,
}

impl ClientState {
    /// the RFC 2131 name
    pub fn name(self) -> &'static str {
        match self {
            ClientState::Stopped => "STOPPED",
            ClientState::Init => "INIT",
            ClientState::Selecting => "SELECTING",
            ClientState::Requesting => "REQUESTING",
            ClientState::Checking => "CHECKING",
            ClientState::Bound => "BOUND",
            ClientState::Renewing => "RENEWING",
            ClientState::Rebinding => "REBINDING",
            ClientState::Rebooting => "REBOOTING",
            ClientState::Declined => "DECLINED",
            ClientState::Idle => "IDLE",
        }
    }
}

/// what a server granted
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    pub addr: Prefix4,
    pub routers: Vec<Ipv4>,
    pub dns: Vec<Ipv4>,
    pub domain: Option<String>,
    pub server: Ipv4,
    /// seconds
    pub lease: u32,
    pub obtained: Ns,
    pub t1: Ns,
    pub t2: Ns,
    pub expiry: Ns,
}

/// what the client did, for dhclient's messages and IOS debug output
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientEv {
    Discover { xid: u32, interval: u32 },
    /// `from`: the sender of the packet (a relay agent or the server)
    Offer { ip: Ipv4, from: Ipv4 },
    Request { ip: Ipv4, to: Ipv4, xid: u32 },
    Ack { ip: Ipv4, from: Ipv4 },
    Nak { from: Ipv4 },
    Decline { ip: Ipv4 },
    Bound { ip: Ipv4, renew: u32 },
    Release { ip: Ipv4, to: Ipv4 },
    Expired { ip: Ipv4 },
    NoOffers,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientLog {
    pub t: Ns,
    pub ev: ClientEv,
}

/// the DHCP client of one interface
#[derive(Clone, Debug)]
pub struct DhcpClient {
    pub iface_name: String,
    pub ios: bool,
    /// probe the address before using it (Linux)
    pub arp_check: bool,
    pub state: ClientState,
    iface: Option<u16>,
    mac: Mac,
    xid: u32,
    /// the first packet of this exchange (the secs field counts from here)
    first: Ns,
    /// dhclient's interval in seconds
    interval: u64,
    tries: u32,
    offer: Option<(Ipv4, Ipv4)>,
    pending: Option<Lease>,
    probes: u8,
    acquiring: Option<Ns>,
    pub lease: Option<Lease>,
    /// the lease file: the last lease, for INIT-REBOOT
    pub saved: Option<Lease>,
    pub log: Vec<ClientLog>,
    discovers: Counter,
    requests: Counter,
    acks: Counter,
    naks: Counter,
    declines: Counter,
    /// how long the last acquisition took (ns)
    last_bind: Ns,
}

/// the classful mask when the server sends none
fn classful(ip: Ipv4) -> u8 {
    match ip.0[0] {
        0..=127 => 8,
        128..=191 => 16,
        _ => 24,
    }
}

impl DhcpClient {
    pub fn new(iface: &str, ios: bool) -> DhcpClient {
        DhcpClient {
            iface_name: String::from(iface),
            ios,
            arp_check: !ios,
            state: ClientState::Init,
            iface: None,
            mac: Mac::ZERO,
            xid: 0,
            first: 0,
            interval: 0,
            tries: 0,
            offer: None,
            pending: None,
            probes: 0,
            acquiring: None,
            lease: None,
            saved: None,
            log: Vec::new(),
            discovers: Counter::new(),
            requests: Counter::new(),
            acks: Counter::new(),
            naks: Counter::new(),
            declines: Counter::new(),
            last_bind: 0,
        }
    }

    fn push_log(&mut self, t: Ns, ev: ClientEv) {
        if self.log.len() >= 200 {
            self.log.remove(0);
        }
        self.log.push(ClientLog { t, ev });
    }

    fn ifname(&self, os: &OsApi) -> String {
        self.iface.and_then(|i| os.iface(i)).map(|f| f.name.clone()).unwrap_or_else(|| self.iface_name.clone())
    }

    /// a message of this client
    fn message(&self, os: &mut OsApi, t: MsgType) -> Dhcp {
        let mut m = Dhcp::new(dhcp::OP_REQUEST, self.xid, self.mac);
        m.secs = ((os.now().saturating_sub(self.first)) / SEC).min(65535) as u16;
        if self.ios {
            m.flags = dhcp::FLAG_BROADCAST;
        }
        m.set(Opt::MsgType(t));
        m
    }

    /// host name, client id and parameter list after the message's own options
    fn finish(&self, os: &OsApi, m: &mut Dhcp) {
        if self.ios {
            let ifn = jfn_model::ifname::short(&self.ifname(os));
            let mut id = alloc::vec![0u8];
            id.extend_from_slice(format!("cisco-{}-{}", self.mac.cisco(), ifn).as_bytes());
            m.set(Opt::ClientId(id));
        }
        m.set(Opt::HostName(os.hostname()));
        m.set(Opt::Params(if self.ios { PRL_IOS.to_vec() } else { PRL_LINUX.to_vec() }));
    }

    /// the next retransmission interval
    fn next_interval(&mut self, os: &mut OsApi, cap: Option<Ns>) -> Ns {
        let now = os.now();
        if self.ios {
            let base = match self.state {
                ClientState::Renewing => self.lease.as_ref().map(|l| (l.t2.saturating_sub(now) / 2).max(60 * SEC)).unwrap_or(60 * SEC),
                ClientState::Rebinding => self.lease.as_ref().map(|l| (l.expiry.saturating_sub(now) / 2).max(60 * SEC)).unwrap_or(60 * SEC),
                _ => (4u64 << self.tries.min(4)) * SEC,
            };
            self.tries += 1;
            let jitter = os.rand() % 2001;
            let t = (base + jitter as u64 * 1_000_000).saturating_sub(SEC);
            return match cap {
                Some(c) => t.min(c.saturating_sub(now) + SEC),
                None => t,
            };
        }
        if self.interval == 0 {
            self.interval = INITIAL;
        } else {
            self.interval += os.rand() as u64 % (2 * self.interval);
        }
        if self.interval > CUTOFF {
            self.interval = CUTOFF / 2 + os.rand() as u64 % CUTOFF;
        }
        if let Some(c) = cap {
            let left = c.saturating_sub(now) / SEC;
            if self.interval > left {
                self.interval = left + 1;
            }
        }
        self.interval * SEC + (os.rand() % 1_000_000) as u64 * 1000
    }

    fn discover(&mut self, os: &mut OsApi) {
        self.state = ClientState::Selecting;
        self.xid = os.rand();
        self.first = os.now();
        self.interval = 0;
        self.tries = 0;
        self.offer = None;
        if self.acquiring.is_none() {
            self.acquiring = Some(os.now());
        }
        self.send_discover(os);
    }

    fn send_discover(&mut self, os: &mut OsApi) {
        let Some(iface) = self.iface else {
            return;
        };
        let now = os.now();
        // dhclient: the backoff never passes the timeout
        let cap = if self.ios { None } else { Some(self.first + TIMEOUT * SEC) };
        let wait = self.next_interval(os, cap);
        let mut m = self.message(os, MsgType::Discover);
        self.finish(os, &mut m);
        let b = m.to_bytes(dhcp::BOOTP_MIN);
        os.note(Note::DhcpDiscover { iface, xid: self.xid, secs: m.secs });
        let ifn = self.ifname(os);
        let secs = (wait / SEC) as u32;
        self.push_log(now, ClientEv::Discover { xid: self.xid, interval: secs });
        if !self.ios {
            os.log(SEV_INFO, "dhclient", &format!("DHCPDISCOVER on {} to 255.255.255.255 port 67 interval {} (xid=0x{:x})", ifn, secs, self.xid));
        }
        os.send_raw(iface, Mac::BROADCAST, Ipv4::UNSPECIFIED, PORT_DHCP_CLIENT, Ipv4::BROADCAST, PORT_DHCP_SERVER, &b);
        self.discovers.inc();
        os.timer(K_RETX, now + wait);
    }

    fn send_request(&mut self, os: &mut OsApi, kind: DhcpReq) {
        let Some(iface) = self.iface else {
            return;
        };
        let now = os.now();
        let cap = match kind {
            DhcpReq::Renew | DhcpReq::Rebind => self.lease.as_ref().map(|l| l.expiry),
            _ => None,
        };
        let wait = self.next_interval(os, cap);
        let mut m = self.message(os, MsgType::Request);
        let (ip, server) = match kind {
            DhcpReq::Select => self.offer.unwrap_or((Ipv4::UNSPECIFIED, Ipv4::UNSPECIFIED)),
            DhcpReq::Reboot => (self.saved.as_ref().map(|l| l.addr.addr).unwrap_or(Ipv4::UNSPECIFIED), Ipv4::UNSPECIFIED),
            DhcpReq::Renew | DhcpReq::Rebind => self.lease.as_ref().map(|l| (l.addr.addr, l.server)).unwrap_or((Ipv4::UNSPECIFIED, Ipv4::UNSPECIFIED)),
        };
        match kind {
            DhcpReq::Select => {
                m.set(Opt::RequestedIp(ip));
                m.set(Opt::ServerId(server));
            }
            DhcpReq::Reboot => m.set(Opt::RequestedIp(ip)),
            DhcpReq::Renew | DhcpReq::Rebind => m.ciaddr = ip,
        }
        self.finish(os, &mut m);
        let b = m.to_bytes(dhcp::BOOTP_MIN);
        os.note(Note::DhcpRequest { iface, ip, kind });
        let to = if kind == DhcpReq::Renew { server } else { Ipv4::BROADCAST };
        self.push_log(now, ClientEv::Request { ip, to, xid: self.xid });
        if !self.ios {
            let ifn = self.ifname(os);
            os.log(SEV_INFO, "dhclient", &format!("DHCPREQUEST for {} on {} to {} port 67 (xid=0x{:x})", ip, ifn, to, self.xid));
        }
        match kind {
            DhcpReq::Renew => {
                os.send(PORT_DHCP_CLIENT, server, PORT_DHCP_SERVER, &b);
            }
            DhcpReq::Rebind => {
                os.send_raw(iface, Mac::BROADCAST, ip, PORT_DHCP_CLIENT, Ipv4::BROADCAST, PORT_DHCP_SERVER, &b);
            }
            _ => {
                os.send_raw(iface, Mac::BROADCAST, Ipv4::UNSPECIFIED, PORT_DHCP_CLIENT, Ipv4::BROADCAST, PORT_DHCP_SERVER, &b);
            }
        }
        self.requests.inc();
        os.timer(K_RETX, now + wait);
    }

    /// starts a new exchange of REQUESTs (selecting an offer, renewing, rebinding, rebooting)
    fn begin_request(&mut self, os: &mut OsApi, state: ClientState, kind: DhcpReq) {
        self.state = state;
        if state != ClientState::Rebinding {
            self.xid = if state == ClientState::Requesting { self.xid } else { os.rand() };
            self.first = os.now();
            self.interval = if state == ClientState::Rebooting { 0 } else { INITIAL };
            self.tries = 0;
        }
        self.send_request(os, kind);
    }

    /// the lease an ACK grants
    fn lease_of(&self, os: &mut OsApi, m: &Dhcp, from: Ipv4) -> Lease {
        let now = os.now();
        let ip = m.yiaddr;
        let len = m.subnet_mask().and_then(|k| k.mask_len()).unwrap_or_else(|| classful(ip));
        let secs = m.lease_time().unwrap_or(86_400);
        let (t1, t2, expiry) = if secs == u32::MAX {
            (Ns::MAX, Ns::MAX, Ns::MAX)
        } else {
            let lease = secs as u64;
            let mut r = m.t1().map(|v| v as u64).unwrap_or(lease / 2).max(1);
            if !self.ios {
                // dhclient spreads renewals over 3/4 to 4/4 of T1
                r = (3 * r + 3) / 4 + (os.rand() as u64 % r + 3) / 4;
            }
            let rb = m.t2().map(|v| v as u64).unwrap_or(lease * 7 / 8);
            (now + r * SEC, now + rb * SEC, now + lease * SEC)
        };
        Lease {
            addr: Prefix4 { addr: ip, len },
            routers: m.routers(),
            dns: m.dns_servers(),
            domain: m.domain_name().map(String::from),
            server: m.server_id().unwrap_or(from),
            lease: secs,
            obtained: now,
            t1,
            t2,
            expiry,
        }
    }

    /// takes the address into use
    fn bind(&mut self, os: &mut OsApi, l: Lease, announce: bool) {
        let Some(iface) = self.iface else {
            return;
        };
        let now = os.now();
        if self.lease.as_ref().is_some_and(|old| old.addr != l.addr || old.routers.first() != l.routers.first()) {
            self.unconfigure(os);
        }
        if self.lease.is_none() {
            let _ = os.add_addr(iface, l.addr);
            if let Some(&gw) = l.routers.first() {
                os.add_route(Route { prefix: Prefix4 { addr: Ipv4::UNSPECIFIED, len: 0 }, via: Some(gw), iface: Some(iface), src: RouteSrc::Static, ad: if self.ios { 254 } else { 1 }, metric: 0 });
            }
        }
        os.set_dhcp_dns(Some((l.dns.clone(), l.domain.iter().cloned().collect())));
        if announce {
            os.arp_announce(iface, l.addr.addr);
        }
        let renew = (l.t1.saturating_sub(now) / SEC).min(u32::MAX as u64) as u32;
        os.note(Note::DhcpBound { iface, addr: l.addr, router: l.routers.first().copied(), renew });
        self.push_log(now, ClientEv::Bound { ip: l.addr.addr, renew });
        let ifn = self.ifname(os);
        if self.ios {
            os.log(SEV_INFO, "%DHCP-6-ADDRESS_ASSIGN", &format!("Interface {} assigned DHCP address {}, mask {}, hostname {}", ifn, l.addr.addr, l.addr.mask(), os.hostname()));
        } else {
            os.log(SEV_INFO, "dhclient", &format!("bound to {} -- renewal in {} seconds.", l.addr.addr, renew));
        }
        os.cancel(K_RETX);
        if l.t1 != Ns::MAX {
            os.timer(K_T1, l.t1);
            os.timer(K_T2, l.t2);
            os.timer(K_EXP, l.expiry);
        }
        if let Some(t0) = self.acquiring.take() {
            self.last_bind = now - t0;
        }
        self.state = ClientState::Bound;
        self.saved = if self.ios { None } else { Some(l.clone()) };
        self.lease = Some(l);
    }

    /// the address, its default route and the name servers go
    fn unconfigure(&mut self, os: &mut OsApi) {
        let Some(l) = self.lease.take() else {
            return;
        };
        if let Some(iface) = self.iface {
            os.remove_addr(iface, l.addr);
        }
        if let Some(&gw) = l.routers.first() {
            os.remove_route(Prefix4 { addr: Ipv4::UNSPECIFIED, len: 0 }, Some(gw));
        }
        os.set_dhcp_dns(None);
    }

    fn release(&mut self, os: &mut OsApi) {
        let now = os.now();
        let Some(l) = self.lease.clone() else {
            // nothing to give back: the client just stops asking
            if let (Some(iface), Some(p)) = (self.iface, self.pending.take()) {
                os.arp_watch(iface, p.addr.addr, false);
            }
            self.state = ClientState::Stopped;
            for k in [K_RETX, K_T1, K_T2, K_EXP] {
                os.cancel(k);
            }
            return;
        };
        self.xid = os.rand();
        let mut m = self.message(os, MsgType::Release);
        m.secs = 0;
        m.flags = 0;
        m.ciaddr = l.addr.addr;
        m.set(Opt::ServerId(l.server));
        let b = m.to_bytes(dhcp::BOOTP_MIN);
        os.note(Note::DhcpReleased { ip: l.addr.addr, server: l.server });
        self.push_log(now, ClientEv::Release { ip: l.addr.addr, to: l.server });
        if !self.ios {
            let ifn = self.ifname(os);
            os.log(SEV_INFO, "dhclient", &format!("DHCPRELEASE of {} on {} to {} port 67 (xid=0x{:x})", l.addr.addr, ifn, l.server, self.xid));
        }
        os.send(PORT_DHCP_CLIENT, l.server, PORT_DHCP_SERVER, &b);
        self.saved = None;
        self.state = ClientState::Stopped;
        for k in [K_RETX, K_T1, K_T2] {
            os.cancel(k);
        }
        // the script takes the address away a little later, so the RELEASE can still leave
        // (it may wait for ARP)
        os.timer(K_EXP, now + SCRIPT_DELAY);
    }

    fn decline(&mut self, os: &mut OsApi, mac: Mac) {
        let Some(l) = self.pending.take() else {
            return;
        };
        let Some(iface) = self.iface else {
            return;
        };
        let now = os.now();
        os.note(Note::DhcpConflict { ip: l.addr.addr, mac });
        os.arp_watch(iface, l.addr.addr, false);
        let mut m = self.message(os, MsgType::Decline);
        m.secs = 0;
        m.flags = 0;
        m.set(Opt::RequestedIp(l.addr.addr));
        m.set(Opt::ServerId(l.server));
        let b = m.to_bytes(dhcp::BOOTP_MIN);
        self.push_log(now, ClientEv::Decline { ip: l.addr.addr });
        if !self.ios {
            let ifn = self.ifname(os);
            os.log(SEV_INFO, "dhclient", &format!("DHCPDECLINE of {} on {} to 255.255.255.255 port 67 (xid=0x{:x})", l.addr.addr, ifn, self.xid));
        }
        os.send_raw(iface, Mac::BROADCAST, Ipv4::UNSPECIFIED, PORT_DHCP_CLIENT, Ipv4::BROADCAST, PORT_DHCP_SERVER, &b);
        self.declines.inc();
        self.saved = None;
        self.state = ClientState::Declined;
        os.timer(K_RETX, now + DECLINE_WAIT * SEC);
    }

    fn on_reply(&mut self, os: &mut OsApi, from: Ipv4, data: &[u8]) {
        let Some(m) = Dhcp::parse(data) else {
            return;
        };
        if m.op != dhcp::OP_REPLY || m.xid != self.xid || m.mac() != self.mac {
            return;
        }
        let now = os.now();
        match m.msg_type() {
            Some(MsgType::Offer) if self.state == ClientState::Selecting => {
                let server = m.server_id().unwrap_or(from);
                os.note(Note::DhcpOffered { ip: m.yiaddr, server });
                self.push_log(now, ClientEv::Offer { ip: m.yiaddr, from });
                if !self.ios {
                    os.log(SEV_INFO, "dhclient", &format!("DHCPOFFER of {} from {}", m.yiaddr, from));
                }
                self.offer = Some((m.yiaddr, server));
                self.begin_request(os, ClientState::Requesting, DhcpReq::Select);
            }
            Some(MsgType::Ack) if matches!(self.state, ClientState::Requesting | ClientState::Rebooting | ClientState::Renewing | ClientState::Rebinding) => {
                let l = self.lease_of(os, &m, from);
                os.note(Note::DhcpAcked { ip: m.yiaddr, server: l.server, lease: l.lease });
                self.push_log(now, ClientEv::Ack { ip: m.yiaddr, from });
                if !self.ios {
                    os.log(SEV_INFO, "dhclient", &format!("DHCPACK of {} from {} (xid=0x{:x})", m.yiaddr, from, self.xid));
                }
                self.acks.inc();
                let fresh = matches!(self.state, ClientState::Requesting | ClientState::Rebooting);
                match self.iface {
                    Some(iface) if fresh && self.arp_check => {
                        // dhclient-script: arping -D -c 2: two probes a second apart
                        os.note(Note::DhcpProbe { ip: l.addr.addr });
                        os.arp_watch(iface, l.addr.addr, true);
                        os.arp_probe(iface, l.addr.addr);
                        self.pending = Some(l);
                        self.probes = 1;
                        self.state = ClientState::Checking;
                        os.timer(K_RETX, now + SEC);
                    }
                    _ => self.bind(os, l, false),
                }
            }
            Some(MsgType::Nak) if matches!(self.state, ClientState::Requesting | ClientState::Rebooting | ClientState::Renewing | ClientState::Rebinding) => {
                os.note(Note::DhcpNaked { server: m.server_id().unwrap_or(from) });
                self.push_log(now, ClientEv::Nak { from });
                if !self.ios {
                    os.log(SEV_INFO, "dhclient", &format!("DHCPNAK from {} (xid=0x{:x})", from, self.xid));
                }
                self.naks.inc();
                self.unconfigure(os);
                self.saved = None;
                for k in [K_T1, K_T2, K_EXP] {
                    os.cancel(k);
                }
                self.discover(os);
            }
            _ => {}
        }
    }

    fn on_timer(&mut self, os: &mut OsApi, key: u32) {
        let now = os.now();
        match (key, self.state) {
            (K_RETX, ClientState::Selecting) => {
                if now >= self.first + TIMEOUT * SEC && !self.ios {
                    let iface = self.iface.unwrap_or(0);
                    os.note(Note::DhcpNoOffers { iface, retry: RETRY as u32 });
                    self.push_log(now, ClientEv::NoOffers);
                    os.log(SEV_INFO, "dhclient", "No DHCPOFFERS received.");
                    os.log(SEV_INFO, "dhclient", "No working leases in persistent database - sleeping.");
                    self.state = ClientState::Idle;
                    os.timer(K_RETX, now + RETRY * SEC);
                } else {
                    self.send_discover(os);
                }
            }
            (K_RETX, ClientState::Requesting) => {
                if now >= self.first + TIMEOUT * SEC {
                    self.discover(os);
                } else {
                    self.send_request(os, DhcpReq::Select);
                }
            }
            (K_RETX, ClientState::Rebooting) => {
                if now >= self.first + REBOOT * SEC {
                    self.saved = None;
                    self.discover(os);
                } else {
                    self.send_request(os, DhcpReq::Reboot);
                }
            }
            (K_RETX, ClientState::Renewing) => self.send_request(os, DhcpReq::Renew),
            (K_RETX, ClientState::Rebinding) => self.send_request(os, DhcpReq::Rebind),
            (K_RETX, ClientState::Checking) => {
                if self.probes < 2 {
                    if let (Some(iface), Some(l)) = (self.iface, self.pending.as_ref()) {
                        let ip = l.addr.addr;
                        os.arp_probe(iface, ip);
                    }
                    self.probes += 1;
                    os.timer(K_RETX, now + SEC);
                } else if let Some(l) = self.pending.take() {
                    if let Some(iface) = self.iface {
                        os.arp_watch(iface, l.addr.addr, false);
                    }
                    self.bind(os, l, true);
                }
            }
            (K_RETX, ClientState::Declined | ClientState::Idle | ClientState::Init) => self.discover(os),
            (K_T1, ClientState::Bound) => self.begin_request(os, ClientState::Renewing, DhcpReq::Renew),
            (K_T2, ClientState::Bound | ClientState::Renewing) => {
                self.state = ClientState::Rebinding;
                self.tries = 0;
                self.send_request(os, DhcpReq::Rebind);
            }
            (K_EXP, ClientState::Stopped) => self.unconfigure(os),
            (K_EXP, ClientState::Bound | ClientState::Renewing | ClientState::Rebinding) => {
                let ip = self.lease.as_ref().map(|l| l.addr.addr).unwrap_or(Ipv4::UNSPECIFIED);
                os.note(Note::DhcpExpired { ip });
                self.push_log(now, ClientEv::Expired { ip });
                if !self.ios {
                    os.log(SEV_INFO, "dhclient", &format!("DHCPEXPIRE of {}", ip));
                }
                self.unconfigure(os);
                self.saved = None;
                self.discover(os);
            }
            _ => {}
        }
    }

    fn reboot_or_discover(&mut self, os: &mut OsApi) {
        let now = os.now();
        match self.saved.clone() {
            Some(l) if l.expiry > now && !self.ios => {
                self.acquiring = Some(now);
                self.begin_request(os, ClientState::Rebooting, DhcpReq::Reboot);
            }
            _ => {
                self.saved = None;
                self.discover(os);
            }
        }
    }
}

impl Service for DhcpClient {
    fn kind(&self) -> &'static str {
        "dhcp-client"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.lease = None;
        self.pending = None;
        let Some(iface) = os.iface_by_name(&self.iface_name) else {
            self.state = ClientState::Stopped;
            return;
        };
        self.iface = Some(iface);
        self.mac = os.iface(iface).map(|f| f.mac).unwrap_or(Mac::ZERO);
        if let Err(_e) = os.bind_raw(PORT_DHCP_CLIENT, iface) {
            os.note(Note::PortInUse { port: PORT_DHCP_CLIENT, svc: format!("DHCP-Client {}", self.iface_name) });
            self.state = ClientState::Stopped;
            return;
        }
        self.acquiring = None;
        // IOS waits for the interface; dhclient sends anyway
        if self.ios && !os.iface_up(iface) {
            self.state = ClientState::Init;
            return;
        }
        self.reboot_or_discover(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Udp(d) => self.on_reply(os, d.src4(), &d.data),
            SvcEvent::Timer(k) => self.on_timer(os, k),
            SvcEvent::ArpConflict { ip, mac, .. } => {
                if self.state == ClientState::Checking && self.pending.as_ref().is_some_and(|l| l.addr.addr == ip) {
                    self.decline(os, mac);
                }
            }
            SvcEvent::Link { iface, up } => {
                if self.ios && up && Some(iface) == self.iface && matches!(self.state, ClientState::Init | ClientState::Idle | ClientState::Stopped) {
                    self.discover(os);
                }
            }
            SvcEvent::Ctl(Ctl::Release) => self.release(os),
            SvcEvent::Ctl(Ctl::Renew) => match self.state {
                ClientState::Bound => self.begin_request(os, ClientState::Renewing, DhcpReq::Renew),
                ClientState::Stopped | ClientState::Idle | ClientState::Declined | ClientState::Init => self.discover(os),
                _ => {}
            },
            // a WLAN connection came up (wlan.rs): without a lease, at once
            SvcEvent::Ctl(Ctl::Carrier) => match self.state {
                ClientState::Init | ClientState::Selecting | ClientState::Idle => self.discover(os),
                ClientState::Rebooting => self.reboot_or_discover(os),
                _ => {}
            },
            _ => {}
        }
    }
    fn stop(&mut self, os: &mut OsApi) {
        // the address does not survive the stop; the lease file does (Linux)
        if let (Some(iface), Some(l)) = (self.iface, self.pending.take()) {
            os.arp_watch(iface, l.addr.addr, false);
        }
        self.unconfigure(os);
        self.state = ClientState::Stopped;
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("discovers", self.discovers.get() as i64),
            Metric::new("requests", self.requests.get() as i64),
            Metric::new("acks", self.acks.get() as i64),
            Metric::new("naks", self.naks.get() as i64),
            Metric::new("declines", self.declines.get() as i64),
            Metric::new("bound", matches!(self.state, ClientState::Bound | ClientState::Renewing | ClientState::Rebinding) as i64),
            Metric::new("bind_ms", (self.last_bind / 1_000_000) as i64),
        ]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        match (&self.state, &self.lease) {
            (ClientState::Bound | ClientState::Renewing | ClientState::Rebinding, Some(l)) => msg!("{}: {} von {} ({})", self.iface_name, l.addr, l.server, self.state.name()),
            _ => msg!("{}: {}", self.iface_name, self.state.name()),
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

/// the client side of an interface ("dhclient -v", "show dhcp lease")
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseView {
    pub service: String,
    pub iface: String,
    pub state: ClientState,
    pub xid: u32,
    pub lease: Option<Lease>,
    pub log: Vec<ClientLog>,
}

impl Engine {
    /// the DHCP clients of a device
    pub fn dhcp_leases(&self, dev: u32) -> Vec<LeaseView> {
        let Some(os) = self.node_index(dev).and_then(|ni| self.os(ni)) else {
            return Vec::new();
        };
        os.svcs
            .iter()
            .filter_map(|s| {
                let c = s.svc.as_ref()?.as_any().downcast_ref::<DhcpClient>()?;
                Some(LeaseView { service: s.name.clone(), iface: c.iface_name.clone(), state: c.state, xid: c.xid, lease: c.lease.clone(), log: c.log.clone() })
            })
            .collect()
    }
}
