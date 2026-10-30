// netsvc -- DHCP, NTP and the resolver at the command lines: the IOS-like DHCP server and
// relay (the pool mode "(dhcp-config)#", excluded addresses, helper addresses, show ip dhcp
// binding|pool|conflict, clear ip dhcp), "ntp server" and "ntp master", their lines in the
// running configuration, and on the Linux shell dhclient and the files of the resolver; help
// texts and messages are our own words, the tables and configuration lines follow the
// originals
//
//   build_netsvc(tree, modes)            the commands in an IOS command tree (ios.rs)
//   Engine::ios_dhcp(ni, parsed)         ip dhcp pool NAME with network, default-router,
//                                        dns-server, domain-name, lease; ip dhcp
//                                        excluded-address; show ip dhcp ...; clear ip dhcp ...
//   Engine::ios_helper(ni, iface, p)     ip helper-address in interface mode
//   Engine::ios_ntp(ni, parsed)          ntp server A, ntp master [STRATUM]
//   Engine::ios_dhcp_config(ni)          the excluded addresses and the pools of the running
//                                        configuration; ios_helper_config(ni, iface) the helper
//                                        lines of an interface; ios_ntp_config(ni) the NTP lines
//                                        before "end"
//   Engine::sh_dhclient(ni, args)        dhclient [-v] [-r] [IF]
//   Engine::sh_cat(ni, args)             cat /etc/resolv.conf | /etc/hosts | /etc/hostname
//   Engine::cli_log_seen(ni)             a line went into the device's log (os.rs): a waiting
//                                        dhclient looks at it
//
// A pool lives in the device's DHCP server ("dhcp-server", dhcpd.rs) once it has a network;
// until then the command line keeps it (DevCfg::pools), as IOS keeps a pool without "network".
// The pool commands read the pool, change it and hand it back (Action::DhcpPool).  The lease
// dates of "show ip dhcp binding" count from the virtual clock's March 1st of the console
// stamps (fmt::ios_stamp), in the year an IOS clock that was never set shows.
//
// dhclient starts or renews the client of the interface (Action::DhcpClient) and keeps the
// console until the client is bound or gives up (60 s without an offer: "No working leases in
// persistent database - sleeping."); with -v it prints the client's lines from the device log
// (dhcpc.rs writes them as dhclient does) while they come.  "-r" sends the RELEASE and returns
// once the address is gone.  /etc/resolv.conf is written from the resolver's settings the way
// Debian's dhclient-script writes it (domain, search, nameserver).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::cmdtree::{ModeId, Parsed, Tree, F_HIDDEN, F_NO};
use netpkt::{Ipv4, Prefix4};

use super::fmt::mask_str;
use super::ios::K;
use super::print::JobOut;
use super::{CliJob, Mode};
use crate::action::Action;
use crate::dhcpd::{BindState, Binding, Detect, PoolCfg};
use crate::ip::IfKind;
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::ntpd::Ntp;

/// router and server addresses a pool line takes at most
const LIST_MAX: usize = 8;
/// how long dhclient waits at most for its answer
const DHCLIENT_WAIT: Ns = 60 * SEC;
/// dhclient -r returns when the script took the address away (dhcpc.rs: 10 ms after the
/// RELEASE)
const RELEASE_WAIT: Ns = 20 * MS;

/// the commands of the DHCP server and relay and of NTP in an IOS tree; `m`: user,
/// privileged, global configuration, interface, subinterface and pool mode
pub(crate) fn build_netsvc(t: &mut Tree<K>, m: [ModeId; 6]) {
    let [user, prv, conf, ifm, sub, pool] = m;
    for x in [user, prv] {
        t.add(x, "show ip dhcp", "State of the DHCP server", None, 0);
        t.add(x, "show ip dhcp binding", "Addresses handed out and who has them", Some(K::ShowDhcpBinding), 0);
        t.add(x, "show ip dhcp pool", "The pools and how many addresses are in use", Some(K::ShowDhcpPool), 0);
        t.add(x, "show ip dhcp conflict", "Addresses found in use by somebody else", Some(K::ShowDhcpConflict), 0);
    }
    t.add(prv, "clear ip", "Forget IPv4 state", None, 0);
    t.add(prv, "clear ip dhcp", "Tables of the DHCP server", None, 0);
    for (w, h) in [("binding", "Addresses handed out"), ("conflict", "Addresses found in use")] {
        t.add(prv, &format!("clear ip dhcp {}=what", w), h, None, 0);
        t.add(prv, &format!("clear ip dhcp {}=what *=all", w), "All of them", Some(K::ClearDhcp), 0);
        t.add(prv, &format!("clear ip dhcp {}=what A.B.C.D=ip", w), "Only this address", Some(K::ClearDhcp), 0);
    }

    // global configuration ("ip" itself is there, with its "no")
    t.add(conf, "ip dhcp", "The DHCP server of this device", None, 0);
    t.add(conf, "ip dhcp pool", "Set up a pool of addresses to hand out", None, 0);
    t.add(conf, "ip dhcp pool WORD=pool", "Name of the pool", Some(K::DhcpPool), 0);
    t.add(conf, "ip dhcp excluded-address", "Addresses the server never hands out", None, 0);
    t.add(conf, "ip dhcp excluded-address A.B.C.D=lo", "The first (or only) address", Some(K::DhcpExcluded), 0);
    t.add(conf, "ip dhcp excluded-address A.B.C.D=lo A.B.C.D=hi", "The last address of the range", Some(K::DhcpExcluded), 0);
    t.add(conf, "ntp", "Time from the network", None, F_NO);
    t.add(conf, "ntp server", "Take the time from this server", None, 0);
    t.add(conf, "ntp server A.B.C.D=server", "Address of the time server", Some(K::NtpServer), 0);
    t.add(conf, "ntp master", "Hand out the own clock as the time", Some(K::NtpMaster), 0);
    t.add(conf, "ntp master <1-15>=stratum", "Stratum to claim (8 if not given)", Some(K::NtpMaster), 0);

    // interfaces
    for x in [ifm, sub] {
        t.add(x, "ip helper-address", "Pass DHCP broadcasts on to a server", None, 0);
        t.add(x, "ip helper-address A.B.C.D=helper", "Address of the DHCP server", Some(K::Helper), 0);
    }

    // the pool mode: like every configuration mode, then its own commands
    t.add(pool, "do", "Run an EXEC command from here", None, 0);
    t.add(pool, "do LINE=cmd", "The command", Some(K::Do), 0);
    t.add(pool, "exit", "Back to the device configuration", Some(K::Exit), 0);
    t.add(pool, "end", "Finish configuring", Some(K::End), F_HIDDEN);
    t.add(pool, "network", "The network the pool hands addresses out of", None, F_NO);
    t.add(pool, "network A.B.C.D=net", "Network address", None, 0);
    t.add(pool, "network A.B.C.D=net MASK=mask", "Network mask", Some(K::PoolNetwork), 0);
    t.add(pool, "network A.B.C.D=net WORD=len", "Prefix length such as /24", Some(K::PoolNetwork), F_HIDDEN);
    t.add_no(pool, "network", K::PoolNetwork);
    for (kw, key, k, h1, h2) in [
        ("default-router", "r", K::PoolRouter, "Routers the clients send everything else to", "Address of a router"),
        ("dns-server", "s", K::PoolDns, "Name servers the clients ask", "Address of a name server"),
    ] {
        t.add(pool, kw, h1, None, F_NO);
        let mut path = String::from(kw);
        for i in 1..=LIST_MAX {
            path.push_str(&format!(" A.B.C.D={}{}", key, i));
            t.add(pool, &path, if i == 1 { h2 } else { "One more (up to 8)" }, Some(k), 0);
        }
        t.add_no(pool, kw, k);
    }
    t.add(pool, "domain-name", "The domain clients look short names up in", None, F_NO);
    t.add(pool, "domain-name WORD=domain", "The domain", Some(K::PoolDomain), 0);
    t.add_no(pool, "domain-name", K::PoolDomain);
    t.add(pool, "lease", "How long a client may keep its address", None, F_NO);
    t.add(pool, "lease <0-365>=days", "Days", Some(K::PoolLease), 0);
    t.add(pool, "lease <0-365>=days <0-23>=hours", "Hours", Some(K::PoolLease), 0);
    t.add(pool, "lease <0-365>=days <0-23>=hours <0-59>=mins", "Minutes", Some(K::PoolLease), 0);
    t.add(pool, "lease infinite", "No end", Some(K::PoolLease), 0);
    t.add_no(pool, "lease", K::PoolLease);
}

/// a client as "show ip dhcp binding" names it: its client identifier, else hardware type 1
/// and the MAC, in dotted hex ("0102.4a00.0004.01")
fn client_text(b: &Binding) -> String {
    let mut bytes = Vec::new();
    match &b.client_id {
        Some(id) => bytes.extend_from_slice(id),
        None => {
            bytes.push(1);
            bytes.extend_from_slice(&b.mac.0);
        }
    }
    let mut s = String::new();
    for (i, x) in bytes.iter().enumerate() {
        if i > 0 && i % 2 == 0 {
            s.push('.');
        }
        s.push_str(&format!("{:02x}", x));
    }
    s
}

/// "Mar 02 1993 12:24 AM": a virtual time on the calendar of the console stamps (an IOS clock
/// that was never set starts on March 1st, 1993)
fn ios_date(t: Ns) -> String {
    // from March on; January and February belong to the next year
    const MONTHS: [(&str, u64); 12] = [("Mar", 31), ("Apr", 30), ("May", 31), ("Jun", 30), ("Jul", 31), ("Aug", 31), ("Sep", 30), ("Oct", 31), ("Nov", 30), ("Dec", 31), ("Jan", 31), ("Feb", 28)];
    let s = t / SEC;
    let (h, m) = ((s / 3600) % 24, (s / 60) % 60);
    let mut day = s / 86_400;
    let mut year = 1993;
    let mut i = 0;
    while day >= MONTHS[i].1 {
        day -= MONTHS[i].1;
        i += 1;
        if i == 10 {
            year += 1;
        } else if i == MONTHS.len() {
            i = 0;
        }
    }
    let month = MONTHS[i].0;
    let (h12, ampm) = match h {
        0 => (12, "AM"),
        1..=11 => (h, "AM"),
        12 => (12, "PM"),
        _ => (h - 12, "PM"),
    };
    format!("{} {:02} {} {:02}:{:02} {}", month, day + 1, year, h12, m, ampm)
}

/// a lease time in words ("1 day", "12 hours", "infinite")
fn lease_words(secs: u32) -> String {
    if secs == u32::MAX {
        return String::from("infinite");
    }
    let (d, h, m) = (secs / 86_400, (secs / 3600) % 24, (secs / 60) % 60);
    let mut parts: Vec<String> = Vec::new();
    for (n, one, many) in [(d, "day", "days"), (h, "hour", "hours"), (m, "minute", "minutes")] {
        if n > 0 {
            parts.push(format!("{} {}", n, if n == 1 { one } else { many }));
        }
    }
    if parts.is_empty() { format!("{} seconds", secs) } else { parts.join(", ") }
}

/// the "lease" line of a pool ("lease 7", "lease 0 12", "lease infinite")
fn lease_line(secs: u32) -> String {
    if secs == u32::MAX {
        return String::from(" lease infinite\n");
    }
    let (d, h, m) = (secs / 86_400, (secs / 3600) % 24, (secs / 60) % 60);
    if m > 0 {
        format!(" lease {} {} {}\n", d, h, m)
    } else if h > 0 {
        format!(" lease {} {}\n", d, h)
    } else {
        format!(" lease {}\n", d)
    }
}

/// a pool block of the running configuration (`net`: the pool has its network)
fn pool_block(name: &str, c: &PoolCfg, net: bool) -> String {
    let mut s = format!("ip dhcp pool {}\n", name);
    if net {
        s.push_str(&format!(" network {} {}\n", c.network.addr, mask_str(c.network.len)));
    }
    if !c.routers.is_empty() {
        let v: Vec<String> = c.routers.iter().map(|a| format!("{}", a)).collect();
        s.push_str(&format!(" default-router {}\n", v.join(" ")));
    }
    if !c.dns.is_empty() {
        let v: Vec<String> = c.dns.iter().map(|a| format!("{}", a)).collect();
        s.push_str(&format!(" dns-server {}\n", v.join(" ")));
    }
    if let Some(d) = &c.domain {
        s.push_str(&format!(" domain-name {}\n", d));
    }
    if let Some(l) = c.lease {
        s.push_str(&lease_line(l));
    }
    s.push_str("!\n");
    s
}

impl Engine {
    // ---------------------------------------------------------------- IOS

    /// the pool of this name: its settings and whether the DHCP server has it (else the
    /// command line keeps it, without a network yet)
    fn pool_of(&self, ni: usize, name: &str) -> Option<(String, PoolCfg, bool)> {
        let dev = self.dev_of(ni);
        if let Some(p) = self.dhcp_pools(dev).into_iter().find(|p| p.name.eq_ignore_ascii_case(name)) {
            return Some((p.name, p.cfg, true));
        }
        self.cdev(ni)?.cfg.pools.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(n, c)| (n.clone(), c.clone(), false))
    }

    /// keeps a pool without a network on the command line (replacing one of the same name)
    fn pool_pending(&mut self, ni: usize, name: &str, cfg: Option<PoolCfg>) {
        if let Some(c) = self.cdev_mut(ni) {
            c.cfg.pools.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
            if let Some(x) = cfg {
                c.cfg.pools.push((String::from(name), x));
            }
        }
    }

    /// "ip dhcp ..." in the global configuration, the commands of the pool mode, show and
    /// clear ip dhcp
    pub(crate) fn ios_dhcp(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        match p.cmd {
            K::DhcpPool => {
                let name = String::from(p.text("pool").unwrap_or(""));
                let had = self.pool_of(ni, &name);
                if p.no {
                    let Some((canon, _, live)) = had else {
                        return self.ios_err(ni, &format!("% There is no pool {}\n", name));
                    };
                    if live {
                        let _ = self.apply(Action::DhcpPool { dev, name: canon.clone(), pool: None });
                    }
                    self.pool_pending(ni, &canon, None);
                    return Ok(());
                }
                let canon = match had {
                    Some((n, _, _)) => n,
                    None => {
                        // the server takes it with its network; until then the command line
                        self.pool_pending(ni, &name, Some(PoolCfg::new(Prefix4 { addr: Ipv4::UNSPECIFIED, len: 0 })));
                        name
                    }
                };
                if let Some(c) = self.cdev_mut(ni) {
                    c.mode = Mode::DhcpPool;
                    c.ifs.clear();
                    c.vlans.clear();
                    c.pool = canon;
                }
                Ok(())
            }
            K::PoolNetwork | K::PoolRouter | K::PoolDns | K::PoolDomain | K::PoolLease => self.ios_pool_cmd(ni, p),
            K::DhcpExcluded => {
                let lo = Ipv4(p.ip("lo").unwrap_or([0; 4]));
                let hi = p.ip("hi").map(Ipv4).unwrap_or(lo);
                if hi.to_u32() < lo.to_u32() {
                    return self.ios_err(ni, "% The range ends before it starts\n");
                }
                match self.apply(Action::DhcpExclude { dev, from: lo, to: hi, add: !p.no }) {
                    Ok(()) => Ok(()),
                    Err(e) => self.ios_err(ni, &format!("% {}\n", e)),
                }
            }
            K::ShowDhcpBinding => {
                let text = self.show_dhcp_binding(ni);
                self.out(ni, &text);
                Ok(())
            }
            K::ShowDhcpPool => {
                let text = self.show_dhcp_pool(ni);
                self.out(ni, &text);
                Ok(())
            }
            K::ShowDhcpConflict => {
                let text = self.show_dhcp_conflict(ni);
                self.out(ni, &text);
                Ok(())
            }
            K::ClearDhcp => {
                let conflicts = p.text("what") == Some("conflict");
                // nothing to forget is fine, as on IOS
                let _ = self.apply(Action::DhcpClear { dev, ip: p.ip("ip").map(Ipv4), conflicts });
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// a command of the pool mode: read the pool, change it, hand it back
    fn ios_pool_cmd(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let name = self.cdev(ni).map(|c| c.pool.clone()).unwrap_or_default();
        let Some((name, mut cfg, live)) = self.pool_of(ni, &name) else {
            return self.ios_err(ni, "% No pool is being configured\n");
        };
        let mut has_net = live;
        match p.cmd {
            K::PoolNetwork => {
                if p.no {
                    has_net = false;
                } else {
                    let addr = Ipv4(p.ip("net").unwrap_or([0; 4]));
                    let len = match (p.mask("mask"), p.text("len")) {
                        (Some(m), _) => Some(m),
                        (None, Some(w)) => w.strip_prefix('/').and_then(|x| x.parse::<u8>().ok()).filter(|&l| l <= 32),
                        _ => None,
                    };
                    let Some(len) = len else {
                        return self.ios_err(ni, "% Give the mask as A.B.C.D or as /nn\n");
                    };
                    let net = Prefix4 { addr, len };
                    if len == 0 || len > 30 {
                        return self.ios_err(ni, &format!("% {} leaves no room for clients\n", net));
                    }
                    if net.trunc().addr != addr {
                        return self.ios_err(ni, &format!("% {} is not a network address ({} is)\n", addr, net.trunc().addr));
                    }
                    cfg.network = net;
                    has_net = true;
                }
            }
            K::PoolRouter | K::PoolDns => {
                let key = if p.cmd == K::PoolRouter { "r" } else { "s" };
                let list: Vec<Ipv4> = if p.no { Vec::new() } else { (1..=LIST_MAX).filter_map(|i| p.ip(&format!("{}{}", key, i)).map(Ipv4)).collect() };
                if p.cmd == K::PoolRouter {
                    cfg.routers = list;
                } else {
                    cfg.dns = list;
                }
            }
            K::PoolDomain => cfg.domain = if p.no { None } else { p.text("domain").map(|d| String::from(d.trim_end_matches('.'))) },
            K::PoolLease => {
                cfg.lease = if p.no {
                    None
                } else if p.has("infinite") {
                    Some(u32::MAX)
                } else {
                    let s = p.num("days").unwrap_or(0) * 86_400 + p.num("hours").unwrap_or(0) * 3600 + p.num("mins").unwrap_or(0) * 60;
                    if s == 0 {
                        return self.ios_err(ni, "% A lease needs at least a minute\n");
                    }
                    Some(s as u32)
                };
            }
            _ => {}
        }
        if has_net {
            match self.apply(Action::DhcpPool { dev, name: name.clone(), pool: Some(cfg) }) {
                Ok(()) => {
                    self.pool_pending(ni, &name, None);
                    Ok(())
                }
                Err(e) => self.ios_err(ni, &format!("% {}\n", e)),
            }
        } else {
            if live {
                let _ = self.apply(Action::DhcpPool { dev, name: name.clone(), pool: None });
            }
            self.pool_pending(ni, &name, Some(cfg));
            Ok(())
        }
    }

    /// "ip helper-address A" (and its no-form) on one interface
    pub(crate) fn ios_helper(&mut self, ni: usize, name: &str, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        if self.stack(ni).and_then(|s| s.iface_by_name(name)).is_none() {
            return self.ios_err(ni, "% IP addresses may not be configured on L2 links.\n");
        }
        let server = Ipv4(p.ip("helper").unwrap_or([0; 4]));
        if !p.no && (server.is_unspecified() || server.is_broadcast() || server.is_multicast() || server.is_loopback()) {
            return self.ios_err(ni, &format!("% {} cannot be a DHCP server\n", server));
        }
        // a helper that is not there goes silently, as on IOS
        let _ = self.apply(Action::DhcpHelper { dev, iface: String::from(name), server, add: !p.no });
        Ok(())
    }

    /// "ntp server A", "ntp master [N]" and their no-forms
    pub(crate) fn ios_ntp(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let r = if p.cmd == K::NtpServer {
            let server = Ipv4(p.ip("server").unwrap_or([0; 4]));
            if !p.no && (server.is_unspecified() || server.is_broadcast() || server.is_multicast() || server.is_loopback()) {
                return self.ios_err(ni, &format!("% {} cannot be a time server\n", server));
            }
            if p.no && !self.ntp_peers(dev).iter().any(|x| x.addr == server) {
                // nothing to remove: silent, as on IOS
                return Ok(());
            }
            self.apply(Action::NtpServer { dev, server, add: !p.no })
        } else {
            let stratum = if p.no { None } else { Some(p.num("stratum").unwrap_or(8) as u8) };
            self.apply(Action::NtpMaster { dev, stratum })
        };
        match r {
            Ok(()) => Ok(()),
            Err(e) => self.ios_err(ni, &format!("% {}\n", e)),
        }
    }

    /// the NTP lines of the running configuration ("!" first; nothing without NTP)
    pub(crate) fn ios_ntp_config(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let Some(n) = self.service_of::<Ntp>(dev) else {
            return String::new();
        };
        let mut s = String::new();
        match n.master {
            Some(8) => s.push_str("ntp master\n"),
            Some(m) => s.push_str(&format!("ntp master {}\n", m)),
            None => {}
        }
        for p in &n.peers {
            s.push_str(&format!("ntp server {}\n", p.addr));
        }
        if s.is_empty() { s } else { format!("!\n{}", s) }
    }

    /// the excluded addresses and the pools in the running configuration
    pub(crate) fn ios_dhcp_config(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let mut s = String::new();
        let ex = self.dhcp_excluded(dev);
        for (a, b) in &ex {
            if a == b {
                s.push_str(&format!("ip dhcp excluded-address {}\n", a));
            } else {
                s.push_str(&format!("ip dhcp excluded-address {} {}\n", a, b));
            }
        }
        if !ex.is_empty() {
            s.push_str("!\n");
        }
        for p in self.dhcp_pools(dev) {
            s.push_str(&pool_block(&p.name, &p.cfg, true));
        }
        if let Some(c) = self.cdev(ni) {
            for (n, cfg) in &c.cfg.pools {
                s.push_str(&pool_block(n, cfg, false));
            }
        }
        s
    }

    /// the "ip helper-address" lines of an interface
    pub(crate) fn ios_helper_config(&self, ni: usize, iface: &str) -> String {
        let dev = self.dev_of(ni);
        let mut s = String::new();
        for (f, a) in self.dhcp_helpers(dev) {
            if f.eq_ignore_ascii_case(iface) {
                s.push_str(&format!(" ip helper-address {}\n", a));
            }
        }
        s
    }

    fn show_dhcp_binding(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let mut s = String::from("Addresses this server has handed out:\nIP address          Client-ID/              Lease expiration        Type\n                    Hardware address/\n                    User name\n");
        for b in self.dhcp_bindings(dev).iter().filter(|b| b.state == BindState::Bound) {
            let exp = if b.until == Ns::MAX { String::from("Infinite") } else { ios_date(b.until) };
            s.push_str(&format!("{:<20}{:<24}{:<24}Automatic\n", format!("{}", b.ip), client_text(b), exp));
        }
        s
    }

    fn show_dhcp_conflict(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let mut s = String::from("IP address        Detection method   Detection time          VRF\n");
        for b in self.dhcp_bindings(dev).iter().filter(|b| b.state == BindState::Conflict) {
            let how = match b.detect {
                Some(Detect::Decline) => "Gratuitous ARP",
                _ => "Ping",
            };
            s.push_str(&format!("{:<18}{:<19}{:<24}\n", format!("{}", b.ip), how, ios_date(b.since)).trim_end());
            s.push('\n');
        }
        s
    }

    fn show_dhcp_pool(&self, ni: usize) -> String {
        let dev = self.dev_of(ni);
        let ex = self.dhcp_excluded(dev);
        let default_lease = 86_400;
        let mut s = String::new();
        let row = |s: &mut String, k: &str, v: &str| s.push_str(&format!(" {:<31}: {}\n", k, v));
        for p in self.dhcp_pools(dev) {
            let (lo, hi) = p.cfg.bounds();
            let excluded = (lo.to_u32()..=hi.to_u32()).filter(|a| ex.iter().any(|(x, y)| (x.to_u32()..=y.to_u32()).contains(a))).count();
            let list = |v: &[Ipv4]| if v.is_empty() { String::from("-") } else { v.iter().map(|a| format!("{}", a)).collect::<Vec<_>>().join(", ") };
            s.push_str(&format!("\nPool {} :\n", p.name));
            row(&mut s, "Network", &format!("{}", p.cfg.network));
            row(&mut s, "Total addresses", &format!("{}", p.total));
            row(&mut s, "Excluded addresses", &format!("{}", excluded));
            row(&mut s, "Leased addresses", &format!("{}", p.leased));
            row(&mut s, "Default router", &list(&p.cfg.routers));
            row(&mut s, "DNS server", &list(&p.cfg.dns));
            row(&mut s, "Domain name", p.cfg.domain.as_deref().unwrap_or("-"));
            row(&mut s, "Lease", &lease_words(p.cfg.lease.unwrap_or(default_lease)));
        }
        if let Some(c) = self.cdev(ni) {
            for (n, _) in &c.cfg.pools {
                s.push_str(&format!("\nPool {} :\n", n));
                row(&mut s, "Network", "- (not set yet: the pool hands nothing out)");
            }
        }
        s
    }

    // ---------------------------------------------------------------- Linux

    /// an interface of the shell by name, or the first Ethernet port
    fn sh_dhcp_iface(&self, ni: usize, name: Option<&str>) -> Option<String> {
        let st = self.stack(ni)?;
        let f = match name {
            Some(n) => st.ifaces.iter().find(|f| !f.hidden && f.name == n),
            None => st.ifaces.iter().find(|f| !f.hidden && matches!(f.kind, IfKind::Port(_))),
        }?;
        Some(f.name.clone())
    }

    /// the newest line of the device's log
    fn log_mark(&self, ni: usize) -> u32 {
        self.os(ni).and_then(|o| o.log.lines.back().map(|l| l.seq)).unwrap_or(0)
    }

    /// dhclient [-v] [-r] [IF]
    pub(crate) fn sh_dhclient(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let (mut verbose, mut release) = (false, false);
        let mut name: Option<&str> = None;
        for a in args {
            match *a {
                "-v" => verbose = true,
                "-r" => release = true,
                // IPv4 (the only one), once, in the foreground: as it does anyway
                "-4" | "-1" | "-d" => {}
                o if o.starts_with('-') => {
                    self.out(ni, &format!("dhclient: unknown option {}\nUsage: dhclient [-v] [-r] [INTERFACE]\n  -v  show the DHCP messages while they happen\n  -r  give the address back (DHCPRELEASE)\n", o));
                    return Err(format!("dhclient: unknown option {}", o));
                }
                x => name = Some(x),
            }
        }
        let Some(iface) = self.sh_dhcp_iface(ni, name) else {
            let text = format!("dhclient: there is no interface {}\n", name.unwrap_or("eth0"));
            self.out(ni, &text);
            return Err(String::from(text.trim_end()));
        };
        if self.cdev(ni).is_some_and(|c| c.booting) {
            // a startup configuration only switches the client on
            let _ = self.apply(Action::DhcpClient { dev, iface, on: !release });
            return Ok(());
        }
        let has_lease = self.dhcp_leases(dev).iter().any(|l| l.iface == iface && l.lease.is_some());
        if release && !has_lease {
            // nothing to give back
            return Ok(());
        }
        let mark = self.log_mark(ni);
        if let Err(e) = self.apply(Action::DhcpClient { dev, iface: iface.clone(), on: !release }) {
            let text = format!("dhclient: {}\n", e);
            self.out(ni, &text);
            return Err(String::from(text.trim_end()));
        }
        let now = self.now;
        let until = now + if release { RELEASE_WAIT } else { DHCLIENT_WAIT };
        self.next_job += 1;
        let id = self.next_job;
        if let Some(c) = self.cdev_mut(ni) {
            c.job = Some(CliJob { id, out: JobOut::Dhclient { mark, verbose, release, until }, start: now });
        }
        self.cli_wake_at(ni, until);
        self.cli_state(ni);
        self.dhclient_progress(ni);
        Ok(())
    }

    /// a line went into the device's log: a waiting dhclient looks at it
    pub(crate) fn cli_log_seen(&mut self, ni: usize) {
        if self.cdev(ni).is_some_and(|c| matches!(c.job, Some(CliJob { out: JobOut::Dhclient { .. }, .. }))) {
            self.dhclient_progress(ni);
        }
    }

    /// dhclient's lines since the last look; the end when the client is bound or gave up
    pub(crate) fn dhclient_progress(&mut self, ni: usize) {
        let Some(CliJob { out: JobOut::Dhclient { mark, verbose, release, until }, .. }) = self.cdev(ni).and_then(|c| c.job.clone()) else {
            return;
        };
        let lines: Vec<(u32, String)> = self.os(ni).map(|o| o.log.lines.iter().filter(|l| l.seq > mark && l.tag == "dhclient").map(|l| (l.seq, l.text.clone())).collect()).unwrap_or_default();
        let mut text = String::new();
        let mut last = mark;
        let mut done = false;
        for (seq, l) in lines {
            last = seq;
            if verbose {
                text.push_str(&l);
                text.push('\n');
            }
            if !release && (l.starts_with("bound to ") || l.starts_with("No working leases")) {
                done = true;
                break;
            }
        }
        if let Some(CliJob { out: JobOut::Dhclient { mark, .. }, .. }) = self.cdev_mut(ni).and_then(|c| c.job.as_mut()) {
            *mark = last;
        }
        self.out(ni, &text);
        // bound, gave up, or waited long enough (a renewal nobody answers)
        if done || self.now >= until {
            if let Some(c) = self.cdev_mut(ni) {
                // the wake-up at `until` is not needed any more
                c.epoch = c.epoch.wrapping_add(1);
            }
            self.cli_job_done(ni);
        }
    }

    /// the console's timer of a dhclient (at `until`): the release is through, or it waited
    /// long enough
    pub(crate) fn dhclient_timer(&mut self, ni: usize) {
        self.dhclient_progress(ni);
    }

    /// cat FILE ...: the files the resolver and the host name make
    pub(crate) fn sh_cat(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut text = String::new();
        let mut err: Option<String> = None;
        for f in args.iter().filter(|a| !a.starts_with('-')) {
            match *f {
                "/etc/resolv.conf" => text.push_str(&self.resolv_text(ni)),
                "/etc/hosts" => text.push_str(&self.hosts_text(ni)),
                "/etc/hostname" => {
                    let h = self.host(ni);
                    text.push_str(&h);
                    text.push('\n');
                }
                // the device's files (files.rs)
                other => match self.cat_file(ni, other) {
                    Ok(t) => text.push_str(&t),
                    Err(e) => {
                        text.push_str(&e);
                        err.get_or_insert(String::from(e.trim_end()));
                    }
                },
            }
        }
        self.out(ni, &text);
        match err {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// /etc/resolv.conf as dhclient-script (from DHCP) or by hand it would be
    pub(super) fn resolv_text(&self, ni: usize) -> String {
        let Some(os) = self.os(ni) else {
            return String::new();
        };
        let c = os.resolver.conf();
        let from_dhcp = os.resolver.statics.servers.is_empty() && os.resolver.dhcp.is_some();
        let mut s = String::new();
        if from_dhcp && let Some(d) = c.search.first() {
            s.push_str(&format!("domain {}\n", d));
        }
        if !c.search.is_empty() {
            s.push_str(&format!("search {}\n", c.search.join(" ")));
        }
        for a in &c.servers {
            s.push_str(&format!("nameserver {}\n", a));
        }
        let mut opts: Vec<String> = Vec::new();
        if c.ndots != 1 {
            opts.push(format!("ndots:{}", c.ndots));
        }
        if c.timeout != 5 * SEC {
            opts.push(format!("timeout:{}", c.timeout / SEC));
        }
        if c.attempts != 2 {
            opts.push(format!("attempts:{}", c.attempts));
        }
        if !opts.is_empty() {
            s.push_str(&format!("options {}\n", opts.join(" ")));
        }
        s
    }

    /// /etc/hosts: localhost, the device's own name, the static entries
    pub(super) fn hosts_text(&self, ni: usize) -> String {
        let mut s = format!("127.0.0.1\tlocalhost\n127.0.1.1\t{}\n", self.host(ni));
        if let Some(os) = self.os(ni) {
            for (h, a) in &os.hosts {
                s.push_str(&format!("{}\t{}\n", a, h));
            }
        }
        s
    }
}
