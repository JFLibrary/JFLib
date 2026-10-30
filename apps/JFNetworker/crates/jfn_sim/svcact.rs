// svcact -- the actions of the virtual OS and its services, and services from project files
//
//   Engine::os_apply(Action)       Service, RemoveService, ServiceRun, DhcpClient, DhcpRenew,
//                                  DhcpPool, DhcpExclude, DhcpHelper, DhcpClear, DnsZone,
//                                  DnsRecord, DnsForward, Resolver, HostEntry, ClearDnsCache,
//                                  LogHost, LogLevel, Log, NtpServer, NtpMaster, ClockSet;
//                                  NameServers (either family), HostEntry6 (an IPv6 name)
//   Engine::service_def(dev, &ServiceDef)   a service from a project's definition:
//
//     kind            config (JSON)
//     dhcp-client     {"iface": "eth0", "arp_check": true}
//     dhcp-server     {"pools": [{"name": "LAN", "network": "10.0.1.0/24", "range":
//                     ["10.0.1.100", "10.0.1.200"], "routers": ["10.0.1.1"], "dns":
//                     ["10.0.2.10"], "domain": "lab", "lease": 600}], "excluded": [["10.0.1.1",
//                     "10.0.1.9"]], "helpers": [["Gi0/0/0", "10.0.2.10"]], "ping_packets": 1,
//                     "ping_timeout_ms": 1000, "authoritative": false, "lease": 600}
//     dhcp-relay      {"helpers": [["eth0", "10.0.2.10"]]}
//     dns-server      {"zones": [{"origin": "lab", "ttl": 3600, "records": ["www CNAME web",
//                     "web A 10.0.2.20"]}], "forward": false}
//     syslog-server   {"max": 1000}
//     ntp             {"servers": ["10.0.2.10"], "master": 8, "poll": 6}
//     tcp-echo        {"port": 7, "backlog": 0, "rcvbuf": 0, "sndbuf": 0, "nodelay": false}
//     tcp-discard     {"port": 9, "read": true, "read_after_ms": 0, ...}
//     tcp-client      {"server": "10.0.0.2", "port": 7, "bytes": 10000, "chunk": 0,
//                     "interval_ms": 0, "delay_ms": 0, "close": "shutdown", ...} (tcpsvc.rs)
//     http-server     {"port": 80, "workers": 0, "queue": 0, "time_ms": 0, "error_rate": 0,
//                     "routes": [{"path": "/", "body": "...", ...}], ...} (httpsvc.rs)
//     http-client     {"url": "http://10.0.2.10/", "count": 1, "interval_ms": 0, ...}
//     http-load       {"targets": ["http://10.0.2.10/"], "mode": "closed", "users": 10,
//                     "rps": 100, "mix": [...], ...}
//     lb              {"mode": "http", "port": 80, "algorithm": "round-robin", "backends":
//                     [{"name": "web1", "addr": "10.0.2.11", "port": 80}], "health": {...}}
//                     (lb.rs)
//     kv              {"nodes": ["10.0.2.21", "10.0.2.22"], "rf": 2, "consistency": "strong",
//                     ...} (kv.rs)
//     cache           {"backend": "http://10.0.2.11/", "ttl_ms": 30000, "entries": 1000, ...}
//                     (cache.rs)
//     mq              {"port": 9000, "visibility_ms": 5000, "max_deliveries": 5, ...} (mq.rs)
//     worker          {"queue": "http://10.0.3.10:9000", "topic": "jobs", "concurrency": 4,
//                     "time_ms": 50, ...} (mq.rs)
//     script          {"source": "net.udp(7, fn(s, m) { s.send(m.from, m.port, m.data) })",
//                     "ns_per_op": 5, "limits": {"fuel": 10000000, ...}, ...} (scriptsvc.rs)
//     tftp-server     {"root": "/srv/tftp", "writable": false, "overwrite": true, "port": 69,
//                     "timeout_ms": 1000, "retries": 5, "options": true, "quota_kb": 0}
//                     (tftpd.rs)
//     ftp-server      {"root": "/srv/ftp", "anonymous": true, "anon_write": false, "users":
//                     [{"name": "anna", "password": "geheim", "home": "/srv/ftp/anna", "write":
//                     true}], "active": true, "quota_kb": 0, ...} (ftpd.rs)
//     smtp-server     {"domains": ["a.lab"], "users": [{"name": "anna", "password": "geheim"}],
//                     "networks": ["10.0.1.0/24"], "open_relay": false, "submission": true,
//                     "max_size_kb": 10240, "retry_ms": 60000, "give_up_ms": 600000, ...}
//                     (smtpd.rs)
//     pop3-server     {"users": [{"name": "anna", "password": "geheim"}], "mailbox": "/var/mail",
//                     "fail_delay_ms": 1000, ...} (pop3d.rs)
//     snmpd           {"port": 161, "communities": [{"name": "public", "access": "ro", "source":
//                     "10.0.0.0/24"}], "location": "...", "contact": "...", "sinks": [{"host":
//                     "10.0.0.50", "type": "trap", "version": "2c", "community": "public"}],
//                     "notify": ["coldstart", "linkdown", "linkup", "authentication"]} (snmpd.rs)
//     snmptrapd       {"port": 162, "communities": ["public"], "log": "/var/log/snmptrapd.log",
//                     "keep": 100} (snmptrapd.rs)
//     radius-server   {"auth_port": 1812, "acct_port": 1813, "clients": [{"address":
//                     "10.0.0.0/24", "secret": "geheim"}], "users": [{"name": "admin",
//                     "password": "...", "privilege": 15, "reply": "..."}], "acct_log":
//                     "/var/log/radius/accounting"} (radiusd.rs)
//
//   HttpLoadSet / HttpServerSet    change a running load generator or web server in place
//   ScriptSource / RenameService   a script's new definition, keeping its disk and numbers
//                                  (scriptsvc.rs); a service's new name (os.rs)
//
// The actions find the services they change by their type (whatever a project named them)
// and make missing ones on the fly, named after their kind ("dhcp-server", "dns-server",
// "ntp"; the client of an interface is "dhcp-client@eth0"; switching it off releases the
// lease and leaves the client idle).  IOS
// keeps helper addresses in its "dhcp-server" (one DHCP subsystem); a Linux device gets a
// separate "dhcp-relay", and as on a real Linux box a relay and a server cannot share port
// 67 there.  Refusals carry their reason as a message (jfn_model::msg) for the app's catalogue.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::msg;
use jfn_model::msg::Msg;
use jfn_model::project::ServiceDef;
use netpkt::{IpAddr, Ipv4, Prefix4};

use crate::action::{ActErr, Action};
use crate::dhcpc::DhcpClient;
use crate::dhcpd::{Dhcpd, PoolCfg};
use crate::dnsd::{parse_record, DnsServer, Zone};
use crate::kernel::{Engine, MS};
use crate::ntpd::Ntp;
use crate::os::{Ctl, Service};
use crate::syslogd::SyslogServer;

fn text_of(v: Option<&Value>) -> Option<String> {
    v.and_then(|x| x.as_str()).map(String::from)
}

fn ip_of(v: &Value) -> Option<Ipv4> {
    Ipv4::parse(v.as_str()?.trim())
}

fn ips_of(v: Option<&Value>) -> Vec<Ipv4> {
    v.and_then(|x| x.as_array()).map(|a| a.iter().filter_map(ip_of).collect()).unwrap_or_default()
}

fn bad(kind: &str, what: Msg) -> ActErr {
    ActErr::Refused(msg!("Dienst {}: {}", kind, what))
}

/// (interface, address) pairs: [["eth0", "10.0.2.10"], ...]
fn pairs_of(v: Option<&Value>) -> Vec<(String, Ipv4)> {
    v.and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let p = p.as_array()?;
                    Some((String::from(p.first()?.as_str()?), ip_of(p.get(1)?)?))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn pool_of(v: &Value) -> Option<(String, PoolCfg)> {
    let name = text_of(v.get("name"))?;
    let net = Prefix4::parse(v.get("network")?.as_str()?)?;
    let mut c = PoolCfg::new(net);
    if let Some(r) = v.get("range").and_then(|r| r.as_array()) {
        c.range = Some((ip_of(r.first()?)?, ip_of(r.get(1)?)?));
    }
    c.routers = ips_of(v.get("routers"));
    c.dns = ips_of(v.get("dns"));
    c.domain = text_of(v.get("domain"));
    c.lease = v.get("lease").and_then(|l| l.as_u64()).map(|l| l.min(u32::MAX as u64) as u32);
    Some((name, c))
}

impl Engine {
    /// a service from a project's definition (see the table above); it starts when the engine
    /// runs, unless disabled
    pub fn service_def(&mut self, dev: u32, def: &ServiceDef) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let ios = self.os(ni).ok_or(ActErr::NoIp(dev))?.ios;
        let c = &def.config;
        let kind = def.kind.as_str();
        let svc: Box<dyn Service> = match kind {
            "dhcp-client" => {
                let iface = text_of(c.get("iface")).ok_or_else(|| bad(kind, msg!("\u{201e}iface\u{201c} fehlt.")))?;
                let mut d = DhcpClient::new(&iface, ios);
                if let Some(b) = c.get("arp_check").and_then(|b| b.as_bool()) {
                    d.arp_check = b;
                }
                Box::new(d)
            }
            "dhcp-server" | "dhcp-relay" => {
                let mut d = if kind == "dhcp-relay" { Dhcpd::relay(ios) } else { Dhcpd::server(ios) };
                for p in c.get("pools").and_then(|p| p.as_array()).unwrap_or(&[]) {
                    let (name, cfg) = pool_of(p).ok_or_else(|| bad(kind, msg!("ein Pool braucht \u{201e}name\u{201c} und \u{201e}network\u{201c}.")))?;
                    d.set_pool(&name, Some(cfg));
                }
                for (a, b) in pairs_of(c.get("excluded")).into_iter().map(|(a, b)| (Ipv4::parse(&a), b)) {
                    if let Some(a) = a {
                        d.set_excluded(a, b, true);
                    }
                }
                d.helpers = pairs_of(c.get("helpers"));
                if let Some(n) = c.get("ping_packets").and_then(|n| n.as_u64()) {
                    d.ping_packets = n.min(10) as u8;
                }
                if let Some(n) = c.get("ping_timeout_ms").and_then(|n| n.as_u64()) {
                    d.ping_timeout = n * MS;
                }
                if let Some(b) = c.get("authoritative").and_then(|b| b.as_bool()) {
                    d.authoritative = b;
                }
                if let Some(n) = c.get("lease").and_then(|n| n.as_u64()) {
                    d.default_lease = n.min(u32::MAX as u64) as u32;
                }
                Box::new(d)
            }
            "dns-server" => {
                let mut s = DnsServer::new(ios);
                for z in c.get("zones").and_then(|z| z.as_array()).unwrap_or(&[]) {
                    let origin = text_of(z.get("origin")).unwrap_or_default();
                    let mut zone = Zone::new(&origin);
                    if let Some(t) = z.get("ttl").and_then(|t| t.as_u64()) {
                        zone.ttl = t.min(u32::MAX as u64) as u32;
                    }
                    for line in z.get("records").and_then(|r| r.as_array()).unwrap_or(&[]) {
                        let l = line.as_str().unwrap_or("");
                        let rr = parse_record(l, &zone.origin, zone.ttl).ok_or_else(|| bad(kind, msg!("\u{201e}{}\u{201c} ist kein Eintrag.", l)))?;
                        zone.add(rr);
                    }
                    s.zones.push(zone);
                }
                if let Some(b) = c.get("forward").and_then(|b| b.as_bool()) {
                    s.forward = b;
                }
                Box::new(s)
            }
            "syslog-server" => {
                let mut s = SyslogServer::new();
                if let Some(n) = c.get("max").and_then(|n| n.as_u64()) {
                    s.max = n.clamp(1, 100_000) as usize;
                }
                Box::new(s)
            }
            "ntp" => {
                let mut n = Ntp::new(ios);
                for a in ips_of(c.get("servers")) {
                    n.set_server(a, true);
                }
                n.master = c.get("master").and_then(|m| m.as_u64()).map(|m| m.clamp(1, 15) as u8);
                if let Some(p) = c.get("poll").and_then(|p| p.as_u64()) {
                    n.poll = p.clamp(4, 17) as u8;
                }
                Box::new(n)
            }
            "tcp-echo" | "tcp-discard" | "tcp-client" => crate::tcpsvc::from_def_msg(kind, c).map_err(|w| bad(kind, w))?,
            "http-server" | "http-client" | "http-load" => crate::httpsvc::from_def_msg(kind, c).map_err(|w| bad(kind, w))?,
            "lb" => crate::lb::from_def_msg(c).map_err(|w| bad(kind, w))?,
            "kv" => crate::kv::from_def_msg(c).map_err(|w| bad(kind, w))?,
            "cache" => crate::cache::from_def_msg(c).map_err(|w| bad(kind, w))?,
            "mq" | "worker" => crate::mq::from_def_msg(kind, c).map_err(|w| bad(kind, w))?,
            "script" => crate::scriptsvc::from_def_msg(c).map_err(|w| bad(kind, w))?,
            "tftp-server" => Box::new(crate::tftpd::TftpServer::from_config(c, ios).map_err(|w| bad(kind, w))?),
            "ftp-server" => Box::new(crate::ftpd::FtpServer::from_config(c).map_err(|w| bad(kind, w))?),
            "smtp-server" => Box::new(crate::smtpd::SmtpServer::from_config(c).map_err(|w| bad(kind, w))?),
            "pop3-server" => Box::new(crate::pop3d::Pop3Server::from_config(c).map_err(|w| bad(kind, w))?),
            "snmpd" => Box::new(crate::snmpd::SnmpAgent::from_config(c, ios).map_err(|w| bad(kind, w))?),
            "snmptrapd" => Box::new(crate::snmptrapd::TrapReceiver::from_config(c).map_err(|w| bad(kind, w))?),
            "radius-server" => Box::new(crate::radiusd::RadiusServerSvc::from_config(c).map_err(|w| bad(kind, w))?),
            k => return Err(ActErr::Refused(msg!("Unbekannte Dienstart \u{201e}{}\u{201c}.", k))),
        };
        let name = if def.name.is_empty() { def.kind.clone() } else { def.name.clone() };
        self.add_service(dev, &name, svc, !def.disabled)
    }

    /// a running service of type T by name: (node slot, service id)
    fn http_svc<T: Service + 'static>(&self, dev: u32, name: &str) -> Result<(usize, u32), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let os = self.os(ni).ok_or(ActErr::NoIp(dev))?;
        let s = os.svc(name).ok_or_else(|| ActErr::Refused(msg!("Den Dienst \u{201e}{}\u{201c} gibt es nicht.", name)))?;
        if !s.svc.as_ref().is_some_and(|x| x.as_any().downcast_ref::<T>().is_some()) {
            return Err(ActErr::Refused(msg!("Der Dienst \u{201e}{}\u{201c} ist von anderer Art.", name)));
        }
        if !s.running {
            return Err(ActErr::Refused(msg!("Der Dienst \u{201e}{}\u{201c} läuft nicht.", name)));
        }
        Ok((ni, s.id))
    }

    /// the id of the first service of type T that `pick` accepts
    fn find<T: Service + 'static>(&self, ni: usize, pick: &dyn Fn(&T) -> bool) -> Option<u32> {
        self.os(ni)?.svcs.iter().find(|s| s.svc.as_ref().and_then(|x| x.as_any().downcast_ref::<T>()).is_some_and(pick)).map(|s| s.id)
    }

    /// the first service of type T that `pick` accepts, whatever its name; made as `name` when
    /// there is none
    fn ensure<T: Service + 'static>(&mut self, dev: u32, name: &str, pick: &dyn Fn(&T) -> bool, make: impl FnOnce(bool) -> T) -> Result<&mut T, ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let ios = self.os(ni).ok_or(ActErr::NoIp(dev))?.ios;
        let id = match self.find::<T>(ni, pick) {
            Some(id) => id,
            None => {
                self.add_service(dev, name, Box::new(make(ios)), true)?;
                self.os(ni).and_then(|o| o.svc_id(name)).ok_or(ActErr::NoIp(dev))?
            }
        };
        let os = self.os_mut(ni).ok_or(ActErr::NoIp(dev))?;
        os.svcs.iter_mut().find(|s| s.id == id).and_then(|s| s.svc.as_mut()).and_then(|x| x.as_any_mut().downcast_mut::<T>()).ok_or_else(|| ActErr::Refused(msg!("Der Dienst \u{201e}{}\u{201c} ist von anderer Art.", name)))
    }

    /// the DHCP server of a device (IOS: also its relay agent)
    fn dhcp_server(&mut self, dev: u32) -> Result<&mut Dhcpd, ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let ios = self.os(ni).ok_or(ActErr::NoIp(dev))?.ios;
        if !ios && self.find::<Dhcpd>(ni, &|d| d.relay_only).is_some() {
            return Err(ActErr::Refused(msg!("Auf diesem Gerät läuft schon ein DHCP-Relay: Port 67 ist belegt.")));
        }
        self.ensure(dev, "dhcp-server", &|d: &Dhcpd| !d.relay_only, Dhcpd::server)
    }

    /// an interface's canonical name
    fn iface_name(&self, dev: u32, iface: &str) -> Result<(usize, String), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let st = self.stack(ni).ok_or(ActErr::NoIp(dev))?;
        let i = st.iface_by_name(iface).ok_or_else(|| ActErr::UnknownIface { dev, iface: String::from(iface) })?;
        Ok((ni, st.ifaces[i as usize].name.clone()))
    }

    fn dhcp_client(&mut self, dev: u32, iface: &str, on: bool) -> Result<(), ActErr> {
        let (ni, canon) = self.iface_name(dev, iface)?;
        let name = format!("dhcp-client@{}", canon);
        let os = self.os(ni).ok_or(ActErr::NoIp(dev))?;
        let ios = os.ios;
        let have = os.svc(&name).map(|s| (s.id, s.running));
        if on {
            match have {
                Some((id, true)) => self.svc_ctl(ni, id, Ctl::Renew),
                Some(_) => self.set_service_running(dev, &name, true)?,
                None => self.add_service(dev, &name, Box::new(DhcpClient::new(&canon, ios)), true)?,
            }
        } else if let Some((id, _)) = have {
            // the lease goes back to the server; the client stays, idle
            self.svc_ctl(ni, id, Ctl::Release);
        }
        Ok(())
    }

    pub(crate) fn os_apply(&mut self, a: Action) -> Result<(), ActErr> {
        match a {
            Action::Service { dev, def } => self.service_def(dev, &def),
            Action::RemoveService { dev, name } => self.remove_service(dev, &name),
            Action::ServiceRun { dev, name, on } => self.set_service_running(dev, &name, on),
            Action::DhcpClient { dev, iface, on } => self.dhcp_client(dev, &iface, on),
            Action::DhcpRenew { dev, iface } => {
                let (ni, canon) = self.iface_name(dev, &iface)?;
                let name = format!("dhcp-client@{}", canon);
                let id = self.os(ni).and_then(|o| o.svc_id(&name)).ok_or_else(|| ActErr::Refused(msg!("{} holt seine Adresse nicht per DHCP.", canon)))?;
                self.svc_ctl(ni, id, Ctl::Renew);
                Ok(())
            }
            Action::DhcpPool { dev, name, pool } => {
                if let Some(p) = &pool
                    && p.network.len > 30
                {
                    return Err(ActErr::Refused(msg!("{} ist zu klein für einen Pool.", p.network)));
                }
                let removing = pool.is_none();
                if self.dhcp_server(dev)?.set_pool(&name, pool) || !removing {
                    Ok(())
                } else {
                    Err(ActErr::Refused(msg!("Den Pool \u{201e}{}\u{201c} gibt es nicht.", name)))
                }
            }
            Action::DhcpExclude { dev, from, to, add } => {
                self.dhcp_server(dev)?.set_excluded(from, to, add);
                Ok(())
            }
            Action::DhcpHelper { dev, iface, server, add } => {
                let (ni, canon) = self.iface_name(dev, &iface)?;
                let ios = self.os(ni).ok_or(ActErr::NoIp(dev))?.ios;
                let d = if ios {
                    self.ensure(dev, "dhcp-server", &|d: &Dhcpd| !d.relay_only, Dhcpd::server)?
                } else {
                    if self.find::<Dhcpd>(ni, &|d| !d.relay_only).is_some() {
                        return Err(ActErr::Refused(msg!("Auf diesem Gerät läuft schon ein DHCP-Server: Port 67 ist belegt.")));
                    }
                    self.ensure(dev, "dhcp-relay", &|d: &Dhcpd| d.relay_only, Dhcpd::relay)?
                };
                if d.set_helper(&canon, server, add) || add {
                    Ok(())
                } else {
                    Err(ActErr::Refused(msg!("{} ist an {} nicht eingetragen.", server, canon)))
                }
            }
            Action::DhcpClear { dev, ip, conflicts } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                if self.find::<Dhcpd>(ni, &|d| !d.relay_only).is_none() {
                    return Err(ActErr::Refused(msg!("Hier läuft kein DHCP-Server.")));
                }
                self.dhcp_server(dev)?.clear(ip, conflicts);
                Ok(())
            }
            Action::DnsZone { dev, origin, add } => {
                let s = self.ensure(dev, "dns-server", &|_: &DnsServer| true, DnsServer::new)?;
                let have = s.zone(&origin).is_some();
                match (add, have) {
                    (true, false) => s.zones.push(Zone::new(&origin)),
                    (false, true) => s.zones.retain(|z| !netpkt::dns::name_eq(&z.origin, &origin)),
                    (false, false) => return Err(ActErr::Refused(msg!("Die Zone \u{201e}{}\u{201c} gibt es nicht.", origin))),
                    (true, true) => {}
                }
                Ok(())
            }
            Action::DnsRecord { dev, rr, add } => {
                let s = self.ensure(dev, "dns-server", &|_: &DnsServer| true, DnsServer::new)?;
                let ok = if !add {
                    s.remove_record(&rr)
                } else if s.zone_for(&rr.name).is_some() {
                    s.add_record(rr.clone());
                    true
                } else {
                    false
                };
                if ok { Ok(()) } else { Err(ActErr::Refused(msg!("Für {} gibt es keine passende Zone oder keinen solchen Eintrag.", rr.name))) }
            }
            Action::DnsForward { dev, on } => {
                self.ensure(dev, "dns-server", &|_: &DnsServer| true, DnsServer::new)?.forward = on;
                Ok(())
            }
            Action::Resolver { dev, servers, search } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                let os = self.os_mut(ni).ok_or(ActErr::NoIp(dev))?;
                if let Some(s) = servers {
                    os.resolver.statics.servers = s.into_iter().map(IpAddr::V4).collect();
                }
                if let Some(d) = search {
                    os.resolver.statics.search = d.into_iter().map(|x| String::from(x.trim_end_matches('.'))).collect();
                }
                Ok(())
            }
            Action::NameServers { dev, servers } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                self.os_mut(ni).ok_or(ActErr::NoIp(dev))?.resolver.statics.servers = servers;
                Ok(())
            }
            Action::HostEntry { dev, name, ip } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                let os = self.os_mut(ni).ok_or(ActErr::NoIp(dev))?;
                os.hosts.retain(|(h, a)| !(h.eq_ignore_ascii_case(&name) && a.is_v4()));
                if let Some(a) = ip {
                    os.hosts.push((name, IpAddr::V4(a)));
                }
                Ok(())
            }
            Action::HostEntry6 { dev, name, ip } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                let os = self.os_mut(ni).ok_or(ActErr::NoIp(dev))?;
                os.hosts.retain(|(h, a)| !(h.eq_ignore_ascii_case(&name) && !a.is_v4()));
                if let Some(a) = ip {
                    os.hosts.push((name, IpAddr::V6(a)));
                }
                Ok(())
            }
            Action::ClearDnsCache { dev } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                self.os_mut(ni).ok_or(ActErr::NoIp(dev))?.resolver.flush();
                Ok(())
            }
            Action::LogHost { dev, host, add } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                let os = self.os_mut(ni).ok_or(ActErr::NoIp(dev))?;
                os.log.hosts.retain(|h| *h != host);
                if add {
                    os.log.hosts.push(host);
                }
                Ok(())
            }
            Action::LogLevel { dev, sev } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                self.os_mut(ni).ok_or(ActErr::NoIp(dev))?.log.trap = sev.min(7);
                Ok(())
            }
            Action::Log { dev, sev, tag, text } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                if self.os(ni).is_none() {
                    return Err(ActErr::NoIp(dev));
                }
                self.os_log(ni, sev.min(7), &tag, &text, None);
                Ok(())
            }
            Action::NtpServer { dev, server, add } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                let n = self.ensure(dev, "ntp", &|_: &Ntp| true, Ntp::new)?;
                let changed = n.set_server(server, add);
                if changed && add
                    && let Some(id) = self.find::<Ntp>(ni, &|_| true)
                    && self.os(ni).is_some_and(|o| o.running(id))
                {
                    self.svc_ctl(ni, id, Ctl::Renew);
                }
                Ok(())
            }
            Action::NtpMaster { dev, stratum } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                let m = stratum.map(|s| s.clamp(1, 15));
                self.ensure(dev, "ntp", &|_: &Ntp| true, Ntp::new)?.master = m;
                let running = self.find::<Ntp>(ni, &|_| true).is_some_and(|id| self.os(ni).is_some_and(|o| o.running(id)));
                if let Some(os) = self.os_mut(ni)
                    && running
                    && os.clock.source.is_none()
                {
                    os.clock.synced = m.is_some();
                    os.clock.stratum = m.unwrap_or(16);
                }
                Ok(())
            }
            Action::ClockSet { dev, unix_ns } => {
                let now = self.now;
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                self.os_mut(ni).ok_or(ActErr::NoIp(dev))?.clock.set(now, unix_ns);
                Ok(())
            }
            Action::HttpLoadSet { dev, svc, rate_m, users } => {
                let (ni, id) = self.http_svc::<crate::httpsvc::HttpLoad>(dev, &svc)?;
                self.svc_call(ni, id, None, |s, api| {
                    if let Some(l) = s.as_any_mut().downcast_mut::<crate::httpsvc::HttpLoad>() {
                        l.set_load(api, rate_m, users);
                    }
                });
                Ok(())
            }
            Action::HttpServerSet { dev, svc, error_ppm, time_ns, workers } => {
                let (ni, id) = self.http_svc::<crate::httpsvc::HttpServer>(dev, &svc)?;
                self.svc_call(ni, id, None, |s, api| {
                    if let Some(h) = s.as_any_mut().downcast_mut::<crate::httpsvc::HttpServer>() {
                        h.set(api, error_ppm, time_ns, workers);
                    }
                });
                Ok(())
            }
            // a script's new definition, a service's new name
            Action::ScriptSource { dev, svc, config } => self.script_source(dev, &svc, &config),
            Action::RenameService { dev, name, to } => self.rename_service(dev, &name, &to),
            // the lab: faults and its services (lab.rs)
            other => self.lab_apply(other),
        }
    }
}
