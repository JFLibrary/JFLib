// dhcpd -- the DHCP server and the relay agent (RFC 2131, RFC 1542): IOS keeps "ip dhcp pool"
// and "ip helper-address" in one service; Linux runs ISC dhcpd or dhcrelay
//
//   Dhcpd::server(ios) / Dhcpd::relay(ios)     kinds "dhcp-server" and "dhcp-relay"
//   PoolCfg { network, range, routers, dns, domain, lease }
//   Binding { ip, mac, client_id, host, pool, state, since, until, detect }
//   Engine::dhcp_bindings(dev) / dhcp_pools(dev) / dhcp_helpers(dev)
//
// Server: a request is served from the pool whose network holds giaddr (relayed) or the
// address of the interface it came in on.  A client gets its address again while it has a
// binding (an expired one too, as long as nobody took it), else the address it asks for when
// that is free, else the lowest free address (IOS) or the highest of the range (dhcpd); the
// server's own addresses, excluded addresses and conflicts are skipped.  Before an address is
// offered for the first time the server pings it: IOS twice, 500 ms each ("ip dhcp ping
// packets 2", "ip dhcp ping timeout 500"), dhcpd once for 1 s (ping-check); an answer makes it
// a conflict and the next address is tried.  An offer is held for 2 minutes.  IOS sends T1 and
// T2 (options 58, 59) and answers what it cannot grant with a NAK; dhcpd sends no T1/T2 and
// NAKs only when authoritative.  Leases: IOS 1 day, dhcpd 600 s (Debian's
// default-lease-time).  Replies go as RFC 2131 4.1 says: to the relay (giaddr), to ciaddr, as
// a broadcast when the client asks for one, else to the client's MAC and new address
// without ARP; a NAK without relay is always a broadcast.
//
// Relay (an IOS server without a pool for the subnet, or dhcrelay): a request that arrives
// on an interface with helper addresses gets giaddr (the interface's primary address) and
// hops + 1 and goes to every helper over the routing table (the source is the outgoing
// interface, as IOS does without "ip dhcp relay source-interface"); a reply to one of its
// giaddrs goes back out of that interface, as a broadcast when the flag is set, else to the
// client's MAC.  A request that already passed 4 relays is dropped.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use base::metrics::Counter;
use netpkt::dhcp::{self, Dhcp, MsgType, Opt};
use netpkt::syslog::{SEV_INFO, SEV_WARNING};
use netpkt::udp::{PORT_DHCP_CLIENT, PORT_DHCP_SERVER};
use netpkt::{Ipv4, Mac, Prefix4};

use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{Dgram, Metric, OsApi, Service, SvcEvent};
use crate::trace::{Note, RelayDrop};

const K_PING: u32 = 0;
/// an offer is held this long
pub const OFFER_HOLD: Ns = 120 * SEC;
/// relays a request may have passed
pub const MAX_HOPS: u8 = 4;

/// a pool of addresses with what clients get along with them
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolCfg {
    pub network: Prefix4,
    /// the addresses handed out (None: every host address of the network, as IOS does)
    pub range: Option<(Ipv4, Ipv4)>,
    pub routers: Vec<Ipv4>,
    pub dns: Vec<Ipv4>,
    pub domain: Option<String>,
    /// seconds (None: the server's default; u32::MAX: infinite)
    pub lease: Option<u32>,
}

impl PoolCfg {
    pub fn new(network: Prefix4) -> PoolCfg {
        PoolCfg { network: network.trunc(), range: None, routers: Vec::new(), dns: Vec::new(), domain: None, lease: None }
    }
    /// the first and last address it may hand out
    pub fn bounds(&self) -> (Ipv4, Ipv4) {
        match self.range {
            Some(r) => r,
            None => (self.network.first_host(), self.network.last_host()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pool {
    pub name: String,
    pub cfg: PoolCfg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindState {
    /// offered, waiting for the REQUEST ("Selecting")
    Offered,
    Bound,
    /// somebody else uses the address
    Conflict,
    /// expired or released, remembered for the same client (dhcpd)
    Free,
}

/// how a conflict was found
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Detect {
    Ping,
    Decline,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub ip: Ipv4,
    pub mac: Mac,
    pub client_id: Option<Vec<u8>>,
    pub host: Option<String>,
    pub pool: String,
    pub state: BindState,
    pub since: Ns,
    /// the end of an offer or lease (Ns::MAX: infinite)
    pub until: Ns,
    pub detect: Option<Detect>,
}

impl Binding {
    fn owned_by(&self, key: &[u8]) -> bool {
        client_key(self.client_id.as_deref(), self.mac) == key
    }
}

/// a client is known by its client identifier, else by its MAC
fn client_key(id: Option<&[u8]>, mac: Mac) -> Vec<u8> {
    match id {
        Some(c) => c.to_vec(),
        None => {
            let mut k = alloc::vec![1u8];
            k.extend_from_slice(&mac.0);
            k
        }
    }
}

/// a ping check in progress
#[derive(Clone, Debug)]
struct Probe {
    ip: Ipv4,
    pool: String,
    req: Dhcp,
    iface: u16,
    sid: Ipv4,
    sent: u8,
    deadline: Ns,
    hop: u64,
}

/// the DHCP server and relay agent of a device
#[derive(Clone, Debug)]
pub struct Dhcpd {
    pub ios: bool,
    /// dhcrelay: no pools, only relaying
    pub relay_only: bool,
    pub pools: Vec<Pool>,
    pub excluded: Vec<(Ipv4, Ipv4)>,
    /// (interface name, server) ("ip helper-address")
    pub helpers: Vec<(String, Ipv4)>,
    pub ping_packets: u8,
    pub ping_timeout: Ns,
    pub authoritative: bool,
    /// seconds
    pub default_lease: u32,
    pub bindings: BTreeMap<Ipv4, Binding>,
    probes: Vec<Probe>,
    ident: u16,
    seq: u16,
    discovers: Counter,
    offers: Counter,
    requests: Counter,
    acks: Counter,
    naks: Counter,
    declines: Counter,
    releases: Counter,
    informs: Counter,
    conflicts: Counter,
    relayed: Counter,
    relayed_back: Counter,
    dropped: Counter,
}

impl Dhcpd {
    /// a DHCP server as IOS or ISC dhcpd behaves
    pub fn server(ios: bool) -> Dhcpd {
        Dhcpd {
            ios,
            relay_only: false,
            pools: Vec::new(),
            excluded: Vec::new(),
            helpers: Vec::new(),
            ping_packets: if ios { 2 } else { 1 },
            ping_timeout: if ios { 500 * MS } else { SEC },
            authoritative: ios,
            default_lease: if ios { 86_400 } else { 600 },
            bindings: BTreeMap::new(),
            probes: Vec::new(),
            ident: 0,
            seq: 0,
            discovers: Counter::new(),
            offers: Counter::new(),
            requests: Counter::new(),
            acks: Counter::new(),
            naks: Counter::new(),
            declines: Counter::new(),
            releases: Counter::new(),
            informs: Counter::new(),
            conflicts: Counter::new(),
            relayed: Counter::new(),
            relayed_back: Counter::new(),
            dropped: Counter::new(),
        }
    }
    /// a relay agent alone (dhcrelay)
    pub fn relay(ios: bool) -> Dhcpd {
        Dhcpd { relay_only: true, ..Dhcpd::server(ios) }
    }
    pub fn pool(&self, name: &str) -> Option<&Pool> {
        self.pools.iter().find(|p| p.name.eq_ignore_ascii_case(name))
    }
    /// adds, replaces (cfg) or removes (None) a pool
    pub fn set_pool(&mut self, name: &str, cfg: Option<PoolCfg>) -> bool {
        let had = self.pools.iter().position(|p| p.name.eq_ignore_ascii_case(name));
        match (had, cfg) {
            (Some(i), Some(c)) => {
                let name = self.pools[i].name.clone();
                if c.network != self.pools[i].cfg.network {
                    self.bindings.retain(|_, b| b.pool != name || c.network.contains(b.ip));
                }
                self.pools[i].cfg = c;
            }
            (None, Some(c)) => self.pools.push(Pool { name: String::from(name), cfg: c }),
            (Some(i), None) => {
                let p = self.pools.remove(i);
                self.bindings.retain(|_, b| b.pool != p.name);
            }
            (None, None) => return false,
        }
        true
    }
    pub fn set_excluded(&mut self, from: Ipv4, to: Ipv4, add: bool) -> bool {
        let r = if from.to_u32() <= to.to_u32() { (from, to) } else { (to, from) };
        let had = self.excluded.contains(&r);
        if add && !had {
            self.excluded.push(r);
        } else if !add {
            self.excluded.retain(|x| *x != r);
        }
        add != had
    }
    pub fn set_helper(&mut self, iface: &str, server: Ipv4, add: bool) -> bool {
        let had = self.helpers.iter().position(|(i, s)| i.eq_ignore_ascii_case(iface) && *s == server);
        match (had, add) {
            (None, true) => self.helpers.push((String::from(iface), server)),
            (Some(i), false) => {
                self.helpers.remove(i);
            }
            _ => return false,
        }
        true
    }
    /// forgets bindings (one address or all) and, with `conflicts`, conflicts
    pub fn clear(&mut self, ip: Option<Ipv4>, conflicts: bool) {
        self.bindings.retain(|a, b| {
            let hit = ip.is_none_or(|x| x == *a);
            let kind = if conflicts { b.state == BindState::Conflict } else { b.state != BindState::Conflict };
            !(hit && kind)
        });
    }

    fn excluded(&self, ip: Ipv4) -> bool {
        self.excluded.iter().any(|(a, b)| (a.to_u32()..=b.to_u32()).contains(&ip.to_u32()))
    }
    fn pool_for(&self, link: Ipv4) -> Option<&Pool> {
        self.pools.iter().filter(|p| p.cfg.network.contains(link)).max_by_key(|p| p.cfg.network.len)
    }
    fn lease_of(&self, p: &Pool) -> u32 {
        p.cfg.lease.unwrap_or(self.default_lease)
    }
    /// offers and leases that ran out
    fn expire(&mut self, now: Ns) {
        let ios = self.ios;
        self.bindings.retain(|_, b| match b.state {
            BindState::Offered | BindState::Bound if b.until <= now => {
                if ios || b.state == BindState::Offered {
                    return false;
                }
                b.state = BindState::Free;
                true
            }
            _ => true,
        });
    }
    /// the address can go to client `key`
    fn usable(&self, os: &OsApi, p: &Pool, ip: Ipv4, key: &[u8]) -> bool {
        let (lo, hi) = p.cfg.bounds();
        if !(lo.to_u32()..=hi.to_u32()).contains(&ip.to_u32()) || p.cfg.network.is_reserved_host(ip) || self.excluded(ip) || os.is_mine(ip) || self.probes.iter().any(|x| x.ip == ip && !client_key(x.req.client_id(), x.req.mac()).eq(key)) {
            return false;
        }
        match self.bindings.get(&ip) {
            None => true,
            Some(b) => b.state == BindState::Free || (b.state != BindState::Conflict && b.owned_by(key)),
        }
    }
    /// a new address for a client: never used ones first, then free ones
    fn allocate(&self, os: &OsApi, p: &Pool, key: &[u8]) -> Option<Ipv4> {
        let (lo, hi) = p.cfg.bounds();
        let (lo, hi) = (lo.to_u32(), hi.to_u32());
        if lo > hi {
            return None;
        }
        for fresh in [true, false] {
            let ok = |a: u32| {
                let ip = Ipv4::from_u32(a);
                self.usable(os, p, ip, key) && (!fresh || !self.bindings.contains_key(&ip))
            };
            let hit = if self.ios { (lo..=hi).find(|&a| ok(a)) } else { (lo..=hi).rev().find(|&a| ok(a)) };
            if let Some(a) = hit {
                return Some(Ipv4::from_u32(a));
            }
        }
        None
    }

    /// "02:4a:.. (PC1) via 10.0.1.1" as dhcpd logs a client
    fn who(&self, os: &OsApi, m: &Dhcp, iface: u16) -> String {
        let via = if m.giaddr.is_unspecified() { os.iface(iface).map(|f| f.name.clone()).unwrap_or_default() } else { format!("{}", m.giaddr) };
        match m.host_name() {
            Some(h) => format!("{} ({}) via {}", m.mac(), h, via),
            None => format!("{} via {}", m.mac(), via),
        }
    }

    /// the reply to `req`
    fn reply(&self, t: MsgType, req: &Dhcp, ip: Ipv4, pool: Option<&Pool>, sid: Ipv4) -> Dhcp {
        let mut r = Dhcp::new(dhcp::OP_REPLY, req.xid, req.mac());
        r.htype = req.htype;
        r.hlen = req.hlen;
        r.chaddr = req.chaddr;
        r.flags = req.flags;
        r.giaddr = req.giaddr;
        r.yiaddr = ip;
        if matches!(t, MsgType::Ack) {
            r.ciaddr = req.ciaddr;
        }
        r.set(Opt::MsgType(t));
        r.set(Opt::ServerId(sid));
        if t == MsgType::Nak {
            if !req.giaddr.is_unspecified() {
                r.flags |= dhcp::FLAG_BROADCAST;
            }
            r.set(Opt::Message(String::from(if self.ios { "requested address not available" } else { "wrong network" })));
            r.yiaddr = Ipv4::UNSPECIFIED;
            return r;
        }
        if let Some(p) = pool {
            let inform = req.msg_type() == Some(MsgType::Inform);
            if !inform {
                let lease = self.lease_of(p);
                r.set(Opt::LeaseTime(lease));
                if self.ios && lease != u32::MAX {
                    r.set(Opt::T1(lease / 2));
                    r.set(Opt::T2((lease as u64 * 7 / 8) as u32));
                }
            }
            r.set(Opt::SubnetMask(p.cfg.network.mask()));
            if !p.cfg.routers.is_empty() {
                r.set(Opt::Routers(p.cfg.routers.clone()));
            }
            if !p.cfg.dns.is_empty() {
                r.set(Opt::DnsServers(p.cfg.dns.clone()));
            }
            if let Some(d) = &p.cfg.domain {
                r.set(Opt::DomainName(d.clone()));
            }
        }
        // RFC 3046: the relay agent information goes back as it came
        if let Some(o) = req.get(dhcp::O_RELAY) {
            r.set(o.clone());
        }
        r
    }

    /// sends a reply the way RFC 2131 4.1 says
    fn send_reply(&self, os: &mut OsApi, req: &Dhcp, r: &Dhcp, iface: u16, sid: Ipv4) {
        let b = r.to_bytes(dhcp::BOOTP_MIN);
        if !req.giaddr.is_unspecified() {
            os.send(PORT_DHCP_SERVER, req.giaddr, PORT_DHCP_SERVER, &b);
        } else if r.msg_type() == Some(MsgType::Nak) || (req.ciaddr.is_unspecified() && req.broadcast()) {
            os.send_raw(iface, Mac::BROADCAST, sid, PORT_DHCP_SERVER, Ipv4::BROADCAST, PORT_DHCP_CLIENT, &b);
        } else if !req.ciaddr.is_unspecified() {
            os.send(PORT_DHCP_SERVER, req.ciaddr, PORT_DHCP_CLIENT, &b);
        } else {
            os.send_raw(iface, req.mac(), sid, PORT_DHCP_SERVER, r.yiaddr, PORT_DHCP_CLIENT, &b);
        }
    }

    fn offer(&mut self, os: &mut OsApi, req: &Dhcp, ip: Ipv4, pool: &str, iface: u16, sid: Ipv4) {
        let now = os.now();
        let Some(p) = self.pool(pool).cloned() else {
            return;
        };
        let key = client_key(req.client_id(), req.mac());
        // an offer to this client elsewhere goes
        self.bindings.retain(|a, b| !(*a != ip && b.state == BindState::Offered && b.owned_by(&key)));
        let state = match self.bindings.get(&ip) {
            Some(b) if b.state == BindState::Bound && b.owned_by(&key) => BindState::Bound,
            _ => BindState::Offered,
        };
        let until = if state == BindState::Bound { self.bindings.get(&ip).map(|b| b.until).unwrap_or(now) } else { now + OFFER_HOLD };
        self.bindings.insert(ip, Binding { ip, mac: req.mac(), client_id: req.client_id().map(|c| c.to_vec()), host: req.host_name().map(String::from), pool: p.name.clone(), state, since: now, until, detect: None });
        let r = self.reply(MsgType::Offer, req, ip, Some(&p), sid);
        os.note(Note::DhcpdOffer { ip, mac: req.mac() });
        if !self.ios {
            let who = self.who(os, req, iface);
            os.log(SEV_INFO, "dhcpd", &format!("DHCPOFFER on {} to {}", ip, who));
        }
        self.offers.inc();
        self.send_reply(os, req, &r, iface, sid);
    }

    fn ack(&mut self, os: &mut OsApi, req: &Dhcp, ip: Ipv4, pool: &Pool, iface: u16, sid: Ipv4) {
        let now = os.now();
        let lease = self.lease_of(pool);
        let until = if lease == u32::MAX { Ns::MAX } else { now + lease as u64 * SEC };
        self.bindings.insert(ip, Binding { ip, mac: req.mac(), client_id: req.client_id().map(|c| c.to_vec()), host: req.host_name().map(String::from), pool: pool.name.clone(), state: BindState::Bound, since: now, until, detect: None });
        let r = self.reply(MsgType::Ack, req, ip, Some(pool), sid);
        os.note(Note::DhcpdAck { ip, mac: req.mac(), lease });
        if !self.ios {
            let who = self.who(os, req, iface);
            os.log(SEV_INFO, "dhcpd", &format!("DHCPACK on {} to {}", ip, who));
        }
        self.acks.inc();
        self.send_reply(os, req, &r, iface, sid);
    }

    fn nak(&mut self, os: &mut OsApi, req: &Dhcp, ip: Ipv4, iface: u16, sid: Ipv4) {
        let r = self.reply(MsgType::Nak, req, Ipv4::UNSPECIFIED, None, sid);
        os.note(Note::DhcpdNak { ip, mac: req.mac() });
        if !self.ios {
            let who = self.who(os, req, iface);
            os.log(SEV_INFO, "dhcpd", &format!("DHCPNAK on {} to {}", ip, who));
        }
        self.naks.inc();
        self.send_reply(os, req, &r, iface, sid);
    }

    /// starts the ping check of `ip` for `req`, or offers at once
    fn check_or_offer(&mut self, os: &mut OsApi, req: Dhcp, ip: Ipv4, pool: &str, iface: u16, sid: Ipv4, fresh: bool) {
        if !fresh || self.ping_packets == 0 {
            self.offer(os, &req, ip, pool, iface, sid);
            return;
        }
        let now = os.now();
        self.seq = self.seq.wrapping_add(1);
        os.note(Note::DhcpdPing { ip });
        os.echo(ip, self.ident, self.seq);
        let hop = os.cause.unwrap_or(0);
        self.probes.push(Probe { ip, pool: String::from(pool), req, iface, sid, sent: 1, deadline: now + self.ping_timeout, hop });
        self.arm(os);
    }

    fn arm(&self, os: &mut OsApi) {
        match self.probes.iter().map(|p| p.deadline).min() {
            Some(t) => os.timer(K_PING, t),
            None => os.cancel(K_PING),
        }
    }

    /// the server identifier for a request that came as `d`: the address it was sent to, or
    /// the address of the interface
    fn sid(os: &OsApi, d: &Dgram) -> Ipv4 {
        if os.is_mine(d.dst) {
            d.dst
        } else {
            os.iface(d.iface).and_then(|f| f.primary()).unwrap_or(Ipv4::UNSPECIFIED)
        }
    }

    fn discover(&mut self, os: &mut OsApi, req: Dhcp, pool: Pool, d: &Dgram) {
        self.discovers.inc();
        if !self.ios {
            let who = self.who(os, &req, d.iface);
            os.log(SEV_INFO, "dhcpd", &format!("DHCPDISCOVER from {}", who));
        }
        let key = client_key(req.client_id(), req.mac());
        // a retransmission while the ping check runs
        if let Some(pr) = self.probes.iter_mut().find(|p| client_key(p.req.client_id(), p.req.mac()) == key) {
            pr.req = req;
            return;
        }
        let sid = Dhcpd::sid(os, d);
        let known = self.bindings.values().find(|b| b.pool == pool.name && b.state != BindState::Conflict && b.owned_by(&key)).map(|b| b.ip);
        if let Some(ip) = known.filter(|&ip| self.usable(os, &pool, ip, &key)) {
            self.check_or_offer(os, req, ip, &pool.name, d.iface, sid, false);
            return;
        }
        let asked = req.requested_ip().filter(|&ip| self.usable(os, &pool, ip, &key));
        match asked.or_else(|| self.allocate(os, &pool, &key)) {
            Some(ip) => self.check_or_offer(os, req, ip, &pool.name, d.iface, sid, true),
            None => {
                os.note(Note::DhcpdExhausted { pool: pool.name.clone() });
                if self.ios {
                    os.log(SEV_WARNING, "%DHCPD-4-NO_FREE_ADDRESS", &format!("Pool {} has no free address", pool.name));
                } else {
                    let who = self.who(os, &req, d.iface);
                    os.log(SEV_INFO, "dhcpd", &format!("DHCPDISCOVER from {}: network {}: no free leases", who, pool.cfg.network));
                }
            }
        }
    }

    fn request(&mut self, os: &mut OsApi, req: Dhcp, pool: Pool, d: &Dgram) {
        self.requests.inc();
        let key = client_key(req.client_id(), req.mac());
        let sid = Dhcpd::sid(os, d);
        let iface = d.iface;
        let mine = |b: Option<&Binding>| b.is_some_and(|b| b.state != BindState::Conflict && b.owned_by(&key));
        let auth = self.authoritative;
        if !self.ios {
            let who = self.who(os, &req, iface);
            let ip = req.requested_ip().unwrap_or(req.ciaddr);
            os.log(SEV_INFO, "dhcpd", &format!("DHCPREQUEST for {} from {}", ip, who));
        }
        if let Some(server) = req.server_id() {
            // SELECTING: the client took an offer
            if !os.is_mine(server) {
                self.bindings.retain(|_, b| !(b.state == BindState::Offered && b.owned_by(&key)));
                os.note(Note::DhcpdOtherServer { mac: req.mac(), server });
                return;
            }
            let ip = req.requested_ip().unwrap_or(req.ciaddr);
            if mine(self.bindings.get(&ip)) && pool.cfg.network.contains(ip) {
                self.ack(os, &req, ip, &pool, iface, server);
            } else {
                self.nak(os, &req, ip, iface, server);
            }
            return;
        }
        let (ip, reboot) = match req.requested_ip() {
            Some(ip) if req.ciaddr.is_unspecified() => (ip, true),
            _ => (req.ciaddr, false),
        };
        if ip.is_unspecified() {
            return;
        }
        // INIT-REBOOT checks the network first; RENEWING and REBINDING the binding
        if reboot && !pool.cfg.network.contains(ip) {
            if auth {
                self.nak(os, &req, ip, iface, sid);
            } else {
                os.note(Note::DhcpdSilent { ip, mac: req.mac() });
            }
            return;
        }
        let b = self.bindings.get(&ip);
        if mine(b) && self.usable(os, &pool, ip, &key) {
            self.ack(os, &req, ip, &pool, iface, sid);
        } else if b.is_some_and(|b| b.state != BindState::Free && !b.owned_by(&key)) || (!reboot && b.is_none()) {
            if auth {
                self.nak(os, &req, ip, iface, sid);
            } else {
                os.note(Note::DhcpdSilent { ip, mac: req.mac() });
            }
        } else {
            // RFC 2131 4.3.2: no record of this client: stay silent
            os.note(Note::DhcpdSilent { ip, mac: req.mac() });
        }
    }

    fn conflict(&mut self, os: &mut OsApi, ip: Ipv4, pool: &str, how: Detect, mac: Mac) {
        let now = os.now();
        self.bindings.insert(ip, Binding { ip, mac, client_id: None, host: None, pool: String::from(pool), state: BindState::Conflict, since: now, until: Ns::MAX, detect: Some(how) });
        self.conflicts.inc();
    }

    fn on_request(&mut self, os: &mut OsApi, d: &Dgram, req: Dhcp) {
        let now = os.now();
        self.expire(now);
        let link = if req.giaddr.is_unspecified() { os.iface(d.iface).and_then(|f| f.primary()).unwrap_or(Ipv4::UNSPECIFIED) } else { req.giaddr };
        let pool = if self.relay_only { None } else { self.pool_for(link).cloned() };
        let Some(pool) = pool else {
            if self.helpers_of(os, d.iface).is_empty() && !self.relay_only {
                os.note(Note::DhcpdNoPool { net: link });
                self.dropped.inc();
            } else {
                self.relay_request(os, d, req);
            }
            return;
        };
        let key = client_key(req.client_id(), req.mac());
        match req.msg_type() {
            Some(MsgType::Discover) => self.discover(os, req, pool, d),
            Some(MsgType::Request) => self.request(os, req, pool, d),
            Some(MsgType::Decline) => {
                let ip = req.requested_ip().unwrap_or(Ipv4::UNSPECIFIED);
                if self.bindings.get(&ip).is_some_and(|b| b.owned_by(&key)) {
                    self.declines.inc();
                    os.note(Note::DhcpdDecline { ip, mac: req.mac() });
                    if self.ios {
                        os.log(SEV_WARNING, "%DHCPD-4-DECLINE_CONFLICT", &format!("DHCP address {} is declined by the client {}.", ip, req.mac().cisco()));
                    } else {
                        let who = self.who(os, &req, d.iface);
                        os.log(SEV_INFO, "dhcpd", &format!("DHCPDECLINE of {} from {}: abandoned", ip, who));
                    }
                    self.conflict(os, ip, &pool.name, Detect::Decline, req.mac());
                }
            }
            Some(MsgType::Release) => {
                let ip = req.ciaddr;
                if self.bindings.get(&ip).is_some_and(|b| b.owned_by(&key) && b.state == BindState::Bound) {
                    self.releases.inc();
                    os.note(Note::DhcpdRelease { ip });
                    if self.ios {
                        self.bindings.remove(&ip);
                    } else {
                        let who = self.who(os, &req, d.iface);
                        os.log(SEV_INFO, "dhcpd", &format!("DHCPRELEASE of {} from {} (found)", ip, who));
                        if let Some(b) = self.bindings.get_mut(&ip) {
                            b.state = BindState::Free;
                            b.until = now;
                        }
                    }
                }
            }
            Some(MsgType::Inform) => {
                self.informs.inc();
                let sid = Dhcpd::sid(os, d);
                let r = self.reply(MsgType::Ack, &req, Ipv4::UNSPECIFIED, Some(&pool), sid);
                os.note(Note::DhcpdInform { to: req.ciaddr });
                self.send_reply(os, &req, &r, d.iface, sid);
            }
            _ => {}
        }
    }

    /// the helper addresses of an interface
    fn helpers_of(&self, os: &OsApi, iface: u16) -> Vec<Ipv4> {
        self.helpers.iter().filter(|(n, _)| os.iface_by_name(n) == Some(iface)).map(|h| h.1).collect()
    }

    fn relay_request(&mut self, os: &mut OsApi, d: &Dgram, req: Dhcp) {
        let helpers = self.helpers_of(os, d.iface);
        if helpers.is_empty() {
            os.note(Note::DhcpRelayDrop { why: RelayDrop::NoHelper });
            self.dropped.inc();
            return;
        }
        if req.hops >= MAX_HOPS {
            os.note(Note::DhcpRelayDrop { why: RelayDrop::Hops });
            self.dropped.inc();
            return;
        }
        let giaddr = if req.giaddr.is_unspecified() {
            match os.iface(d.iface).and_then(|f| f.primary()) {
                Some(a) => a,
                None => {
                    os.note(Note::DhcpRelayDrop { why: RelayDrop::NoAddress });
                    self.dropped.inc();
                    return;
                }
            }
        } else {
            req.giaddr
        };
        // the message as it came, with giaddr and one hop more
        let mut b = d.data.clone();
        b[3] = req.hops + 1;
        b[24..28].copy_from_slice(&giaddr.0);
        for h in helpers {
            if os.send(PORT_DHCP_SERVER, h, PORT_DHCP_SERVER, &b) {
                os.note(Note::DhcpRelayed { to: h, giaddr });
                self.relayed.inc();
            }
        }
    }

    fn relay_reply(&mut self, os: &mut OsApi, d: &Dgram, m: Dhcp) {
        if self.helpers.is_empty() && !self.relay_only {
            return;
        }
        let Some(j) = os.iface_of(m.giaddr) else {
            os.note(Note::DhcpRelayDrop { why: RelayDrop::NotMine });
            self.dropped.inc();
            return;
        };
        let (mac, to) = if m.broadcast() || m.yiaddr.is_unspecified() { (Mac::BROADCAST, Ipv4::BROADCAST) } else { (m.mac(), m.yiaddr) };
        os.send_raw(j, mac, m.giaddr, PORT_DHCP_SERVER, to, PORT_DHCP_CLIENT, &d.data);
        os.note(Note::DhcpRelayedBack { to, iface: j });
        self.relayed_back.inc();
    }

    fn on_timer(&mut self, os: &mut OsApi) {
        let now = os.now();
        let due: Vec<usize> = (0..self.probes.len()).filter(|&i| self.probes[i].deadline <= now).collect();
        let mut done = Vec::new();
        for i in due {
            if self.probes[i].sent < self.ping_packets {
                self.seq = self.seq.wrapping_add(1);
                let ip = self.probes[i].ip;
                self.probes[i].sent += 1;
                self.probes[i].deadline = now + self.ping_timeout;
                os.echo(ip, self.ident, self.seq);
            } else {
                done.push(i);
            }
        }
        // no answer: offer, oldest first
        for i in done.into_iter().rev() {
            let p = self.probes.remove(i);
            os.cause = if p.hop == 0 { None } else { Some(p.hop) };
            self.offer(os, &p.req, p.ip, &p.pool, p.iface, p.sid);
        }
        os.cause = None;
        self.arm(os);
    }

    fn on_echo(&mut self, os: &mut OsApi, from: Ipv4) {
        let Some(i) = self.probes.iter().position(|p| p.ip == from) else {
            return;
        };
        let p = self.probes.remove(i);
        os.note(Note::DhcpdPingConflict { ip: from });
        if self.ios {
            os.log(SEV_WARNING, "%DHCPD-4-PING_CONFLICT", &format!("DHCP address {} is in use, cannot be allocated.", from));
        } else {
            os.log(SEV_INFO, "dhcpd", &format!("Abandoning IP address {}: pinged before offer", from));
        }
        self.conflict(os, from, &p.pool, Detect::Ping, Mac::ZERO);
        // the next address for the same client
        let Some(pool) = self.pool(&p.pool).cloned() else {
            self.arm(os);
            return;
        };
        let key = client_key(p.req.client_id(), p.req.mac());
        match self.allocate(os, &pool, &key) {
            Some(ip) => self.check_or_offer(os, p.req, ip, &p.pool, p.iface, p.sid, true),
            None => {
                os.note(Note::DhcpdExhausted { pool: p.pool.clone() });
                self.arm(os);
            }
        }
    }
}

impl Service for Dhcpd {
    fn kind(&self) -> &'static str {
        if self.relay_only { "dhcp-relay" } else { "dhcp-server" }
    }
    fn start(&mut self, os: &mut OsApi) {
        self.probes.clear();
        self.ident = os.rand() as u16;
        if self.ios {
            // IOS keeps its bindings in RAM: a restart forgets them
            self.bindings.clear();
        }
        if os.bind(PORT_DHCP_SERVER, None).is_err() {
            let name = String::from(self.kind());
            os.note(Note::PortInUse { port: PORT_DHCP_SERVER, svc: name });
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Udp(d) => {
                let Some(m) = Dhcp::parse(&d.data) else {
                    return;
                };
                match m.op {
                    dhcp::OP_REQUEST => self.on_request(os, &d, m),
                    dhcp::OP_REPLY => self.relay_reply(os, &d, m),
                    _ => {}
                }
            }
            SvcEvent::Timer(K_PING) => self.on_timer(os),
            SvcEvent::EchoReply { from, ident, .. } if ident == self.ident => self.on_echo(os, from),
            _ => {}
        }
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.probes.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        let leases = self.bindings.values().filter(|b| b.state == BindState::Bound).count();
        alloc::vec![
            Metric::new("discovers", self.discovers.get() as i64),
            Metric::new("offers", self.offers.get() as i64),
            Metric::new("requests", self.requests.get() as i64),
            Metric::new("acks", self.acks.get() as i64),
            Metric::new("naks", self.naks.get() as i64),
            Metric::new("declines", self.declines.get() as i64),
            Metric::new("releases", self.releases.get() as i64),
            Metric::new("informs", self.informs.get() as i64),
            Metric::new("conflicts", self.conflicts.get() as i64),
            Metric::new("relayed", self.relayed.get() as i64),
            Metric::new("relayed_back", self.relayed_back.get() as i64),
            Metric::new("dropped", self.dropped.get() as i64),
            Metric::new("leases", leases as i64),
        ]
    }
    fn status(&self) -> String {
        if self.relay_only || self.pools.is_empty() {
            return format!("Relay: {} Helper-Adressen", self.helpers.len());
        }
        let leases = self.bindings.values().filter(|b| b.state == BindState::Bound).count();
        format!("{} Pools, {} Leases", self.pools.len(), leases)
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

/// a pool with its use ("show ip dhcp pool")
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolView {
    pub name: String,
    pub cfg: PoolCfg,
    /// addresses bound now
    pub leased: u32,
    /// addresses it may hand out
    pub total: u32,
}

impl Engine {
    /// the bindings of a device's DHCP server ("show ip dhcp binding", "dhcpd.leases"):
    /// offers, leases and conflicts that are still valid, by address
    pub fn dhcp_bindings(&self, dev: u32) -> Vec<Binding> {
        let now = self.now;
        let Some(d) = self.service_of::<Dhcpd>(dev) else {
            return Vec::new();
        };
        d.bindings.values().filter(|b| b.state == BindState::Conflict || (b.state != BindState::Free && b.until > now)).cloned().collect()
    }
    /// the pools of a device's DHCP server
    pub fn dhcp_pools(&self, dev: u32) -> Vec<PoolView> {
        let now = self.now;
        let Some(d) = self.service_of::<Dhcpd>(dev) else {
            return Vec::new();
        };
        d.pools
            .iter()
            .map(|p| {
                let (lo, hi) = p.cfg.bounds();
                let total = if lo.to_u32() <= hi.to_u32() { hi.to_u32() - lo.to_u32() + 1 } else { 0 };
                let leased = d.bindings.values().filter(|b| b.pool == p.name && b.state == BindState::Bound && b.until > now).count() as u32;
                PoolView { name: p.name.clone(), cfg: p.cfg.clone(), leased, total }
            })
            .collect()
    }
    /// the relay targets of a device ("ip helper-address"): (interface, server)
    pub fn dhcp_helpers(&self, dev: u32) -> Vec<(String, Ipv4)> {
        self.service_of::<Dhcpd>(dev).map(|d| d.helpers.clone()).unwrap_or_default()
    }
    /// the excluded ranges of a device's DHCP server
    pub fn dhcp_excluded(&self, dev: u32) -> Vec<(Ipv4, Ipv4)> {
        self.service_of::<Dhcpd>(dev).map(|d| d.excluded.clone()).unwrap_or_default()
    }
}
