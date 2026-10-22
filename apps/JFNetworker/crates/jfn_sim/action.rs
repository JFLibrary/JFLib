// action -- changes from outside the simulation: the editor, scenarios, fault injection
//
//   Engine::apply(Action) -> Result<(), ActErr>
//
//   AddDevice / RemoveDevice / Connect / Disconnect   follow the project while it runs
//   Power { on }                                       switch a device on or off
//   PortAdmin { up }                                   "shutdown" / "no shutdown"
//   Unplug { down } / Impair { .. } / SetCable          a cable's plug, quality and kind
//   Inject { frame }                                   a raw frame leaves a port (tests, PDUs)
//   Access / Trunk / Vlan                              VLAN set-up of a bridge port
//   IfAddr / IfUp / Mtu / ProxyArp                     IPv4 interfaces: addresses, state, MTU
//   Svi / SubIf / Loopback / RoutedPort                interfaces that come and go
//   Route / Forwarding / StaticArp / ClearArp          routing table, "ip routing", ARP
//   Service / RemoveService / ServiceRun ...           services and the virtual OS: DHCP
//                                                      client, server, relay; DNS zones,
//                                                      resolver, hosts; syslog; NTP, clock
//                                                      (svcact.rs)
//   Switchport                                         one IOS switchport setting of a port
//   Hostname / Startup                                 a device's name, its startup configuration
//   HttpLoadSet / HttpServerSet                        the load of a running load generator, the
//                                                      error rate, time and workers of a running
//                                                      web server (httpsvc.rs)
//   Fault / Heal / HealAll                             faults of the lab with marks on the
//                                                      timeline: cables, power, services, disks,
//                                                      partitions (fault.rs)
//   LbBackend / LbAlgorithm / KvSet                    the backends and algorithm of a running
//                                                      load balancer (lb.rs), the consistency
//                                                      and quorums of a KV node (kv.rs)
//   CacheClear / WorkerSet                             empties a cache (cache.rs); the slots,
//                                                      time and error rate of a worker (mq.rs)
//   ScriptSource / RenameService                       a script's new definition that keeps its
//                                                      disk, numbers and errors (scriptsvc.rs);
//                                                      a service's new name (os.rs)
//   Stp                                                a setting of a bridge's spanning tree
//                                                      (stp.rs: mode, priority, timers, the
//                                                      settings of a port, mcheck)
//   Rip                                                RIP's configuration: "router rip", its
//                                                      commands, "ip rip ... version", "clear ip
//                                                      route *" (rip.rs)
//
// After a change of interfaces, addresses or routes the device's RIP looks again at once
// (rip::touches, Engine::rip_kick).
//
// A device that is switched off forgets what it learned (MAC table, ARP cache, fragments) and
// its jobs end; its configuration stays.  Removing a device removes its cables; frames on them
// are lost.
//
// ActErr::msg() is the refusal as a message for the app's catalogue (Display: its German
// text); Refused carries its reason as one (jfn_model::msg).

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;

use base::rng::Pcg32;
use jfn_model::cable::{self, CableKind, Why};
use jfn_model::catalog::{self, CliStyle, F_ROUTER};
use jfn_model::msg;
use jfn_model::msg::Msg;
use jfn_model::project::{Device, Link, ServiceDef};
use netpkt::dns::Rr;
use netpkt::eth::ET_IPV4;
use netpkt::{Ipv4, Mac, Prefix4};

use crate::bridge::{SwSet, VlanSet};
use crate::dhcpd::PoolCfg;
use crate::ip::{ArpEntry, IfKind, Iface, IpStack, Route, RouteSrc};
use crate::kernel::Engine;
use crate::link::{Dir, LinkRt};
use crate::node::Node;

/// a change from outside
#[derive(Clone, Debug)]
pub enum Action {
    AddDevice(Device),
    RemoveDevice(u32),
    Connect(Link),
    Disconnect(u32),
    Power { dev: u32, on: bool },
    PortAdmin { dev: u32, port: String, up: bool },
    Unplug { link: u32, down: bool },
    Impair { link: u32, kbps: Option<u32>, delay_ns: Option<u64>, jitter_ns: Option<u64>, loss_ppm: Option<u32> },
    SetCable { link: u32, cable: CableKind },
    Inject { dev: u32, port: String, frame: Vec<u8> },
    Access { dev: u32, port: String, vlan: u16 },
    Trunk { dev: u32, port: String, native: u16, allowed: VlanSet },
    Vlan { dev: u32, vid: u16, name: Option<String>, remove: bool },
    /// an IPv4 address of an interface ("ip address", "ip addr add"); the first one is the
    /// primary address; `add: false` removes it
    IfAddr { dev: u32, iface: String, prefix: Prefix4, add: bool },
    /// "shutdown" / "no shutdown" of an interface (for a port: the port itself)
    IfUp { dev: u32, iface: String, up: bool },
    /// the IP MTU of an interface (68 to 65535)
    Mtu { dev: u32, iface: String, mtu: u16 },
    /// "ip proxy-arp" of an interface
    ProxyArp { dev: u32, iface: String, on: bool },
    /// the bridge's own interface in a VLAN ("interface Vlan10"; it starts shut down)
    Svi { dev: u32, vlan: u16, add: bool },
    /// an 802.1Q subinterface of a routed port ("Gi0/0/0.10" with "encapsulation dot1Q 10",
    /// "eth0.10"); IOS ones start up, Linux ones down; `add` again changes the VLAN
    SubIf { dev: u32, name: String, vlan: u16, add: bool },
    /// a loopback interface (IOS "Loopback0", Linux "dummy0"; it starts up)
    Loopback { dev: u32, n: u16, add: bool },
    /// a port of an L3 switch is routed ("no switchport") or switched again
    RoutedPort { dev: u32, port: String, routed: bool },
    /// a static route to `prefix` over a next hop and / or an interface
    Route { dev: u32, prefix: Prefix4, via: Option<Ipv4>, iface: Option<String>, ad: u8, add: bool },
    /// routing between interfaces ("ip routing", net.ipv4.ip_forward)
    Forwarding { dev: u32, on: bool },
    /// a static ARP entry; `mac: None` removes it
    StaticArp { dev: u32, ip: Ipv4, mac: Option<Mac> },
    /// forgets the learned ARP entries ("clear arp-cache", "ip neigh flush all")
    ClearArp { dev: u32 },
    /// adds or replaces a service from its definition (svcact.rs: kinds and config)
    Service { dev: u32, def: ServiceDef },
    RemoveService { dev: u32, name: String },
    /// starts or stops a service ("systemctl start|stop", "[no] service dhcp")
    ServiceRun { dev: u32, name: String, on: bool },
    /// the DHCP client of an interface ("ip address dhcp", "dhclient eth0"); off gives the
    /// lease back ("no ip address dhcp", "dhclient -r eth0")
    DhcpClient { dev: u32, iface: String, on: bool },
    /// renews the lease now ("renew dhcp Gi0/0/0", "dhclient eth0" once more)
    DhcpRenew { dev: u32, iface: String },
    /// a pool of the DHCP server ("ip dhcp pool NAME", dhcpd "subnet"); None removes it
    DhcpPool { dev: u32, name: String, pool: Option<PoolCfg> },
    /// addresses the DHCP server never hands out ("ip dhcp excluded-address A B")
    DhcpExclude { dev: u32, from: Ipv4, to: Ipv4, add: bool },
    /// a server the relay agent sends requests of an interface to ("ip helper-address",
    /// dhcrelay)
    DhcpHelper { dev: u32, iface: String, server: Ipv4, add: bool },
    /// forgets bindings ("clear ip dhcp binding *|A") or, with `conflicts`, conflicts
    DhcpClear { dev: u32, ip: Option<Ipv4>, conflicts: bool },
    /// a zone of the DNS server ("zone" in named.conf)
    DnsZone { dev: u32, origin: String, add: bool },
    /// a record of the zone its name lies in ("ip host", a zone file line)
    DnsRecord { dev: u32, rr: Rr, add: bool },
    /// the DNS server passes names outside its zones to the device's resolver
    DnsForward { dev: u32, on: bool },
    /// the static resolver settings ("/etc/resolv.conf", "ip name-server", "ip domain-name");
    /// None keeps that part
    Resolver { dev: u32, servers: Option<Vec<Ipv4>>, search: Option<Vec<String>> },
    /// a static host name ("/etc/hosts", "ip host" for the device's own lookups); None
    /// removes it
    HostEntry { dev: u32, name: String, ip: Option<Ipv4> },
    /// forgets the resolver's cache ("resolvectl flush-caches", "clear host *")
    ClearDnsCache { dev: u32 },
    /// a syslog server for the device's log ("logging host A", "*.* @A")
    LogHost { dev: u32, host: Ipv4, add: bool },
    /// the highest severity that goes to the log hosts ("logging trap 0..7")
    LogLevel { dev: u32, sev: u8 },
    /// a line for the device's log (and its log hosts): "logger", configuration messages
    Log { dev: u32, sev: u8, tag: String, text: String },
    /// an NTP server the device follows ("ntp server A", chrony "server A")
    NtpServer { dev: u32, server: Ipv4, add: bool },
    /// the device serves its own clock at a stratum ("ntp master 3"); None ends that
    NtpMaster { dev: u32, stratum: Option<u8> },
    /// sets the device clock ("clock set", "date -s"): ns since 1970
    ClockSet { dev: u32, unix_ns: i64 },
    /// one IOS switchport setting ("switchport mode trunk", "switchport access vlan 10", ...)
    Switchport { dev: u32, port: String, set: SwSet },
    /// the device's host name (prompts, the names in notes)
    Hostname { dev: u32, name: String },
    /// the configuration the device boots with ("show startup-config"); it is not applied now
    Startup { dev: u32, text: String },
    /// changes a running load generator without losing its numbers: the rate of the open loop
    /// (thousandths of a request per second) and the users of the closed loop (httpsvc.rs)
    HttpLoadSet { dev: u32, svc: String, rate_m: Option<u64>, users: Option<u32> },
    /// changes a running web server: its error rate (ppm), the time of a request (ns) and the
    /// number of workers (httpsvc.rs)
    HttpServerSet { dev: u32, svc: String, error_ppm: Option<u32>, time_ns: Option<u64>, workers: Option<u32> },
    /// a fault of the lab starts now; with `dur` it heals by itself after that long (fault.rs;
    /// Engine::next_fault_id() is the number it gets)
    Fault { fault: crate::fault::Fault, dur: Option<u64> },
    /// heals a fault by its number (Engine::faults)
    Heal { id: u32 },
    /// heals every active fault
    HealAll,
    /// a backend of a running load balancer: add, remove (it drains first), drain, enable,
    /// weight (lb.rs)
    LbBackend { dev: u32, svc: String, op: crate::lb::LbOp },
    /// the algorithm of a running load balancer
    LbAlgorithm { dev: u32, svc: String, algo: crate::lb::Algo },
    /// the consistency and the quorums of "strong" of a running KV node (kv.rs)
    KvSet { dev: u32, svc: String, consistency: Option<crate::kv::Consistency>, w: Option<u8>, r: Option<u8> },
    /// forgets what a cache holds (cache.rs)
    CacheClear { dev: u32, svc: String },
    /// changes a running worker: its slots, its time per message (ns), its error rate (ppm)
    /// (mq.rs)
    WorkerSet { dev: u32, svc: String, concurrency: Option<u32>, time_ns: Option<u64>, error_ppm: Option<u32> },
    /// a script takes a new definition -- the config of its ServiceDef, the source included --
    /// and keeps its disk, numbers, errors and output; one that should run starts again with
    /// it (scriptsvc.rs); a definition that does not compile is refused
    ScriptSource { dev: u32, svc: String, config: agent_core::jmap::Value },
    /// a service goes on under another name, without a restart (os.rs)
    RenameService { dev: u32, name: String, to: String },
    /// a setting of a bridge's spanning tree (stp.rs)
    Stp { dev: u32, set: crate::stp::StpSet },
    /// a change of RIP's configuration, or "clear ip route *" (rip.rs)
    Rip { dev: u32, set: crate::rip::RipSet },
}

/// why an action was refused (German text through Display)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActErr {
    UnknownDevice(u32),
    UnknownPort { dev: u32, port: String },
    UnknownLink(u32),
    UnknownModel(String),
    DeviceExists(u32),
    LinkExists(u32),
    PortBusy { dev: u32, port: String },
    NoBridge(u32),
    BadVlan(u16),
    Cable(Why),
    /// the device has no IPv4 part (hub, modem, cloud)
    NoIp(u32),
    UnknownIface { dev: u32, iface: String },
    BadMtu(u16),
    /// the device refuses the setting (the reason)
    Refused(Msg),
}

impl ActErr {
    /// the refusal as a message for the app's catalogue (Display: its German text)
    pub fn msg(&self) -> Msg {
        match self {
            ActErr::UnknownDevice(d) => msg!("Das Gerät {} gibt es nicht.", d),
            ActErr::UnknownPort { dev, port } => msg!("Gerät {} hat keinen Port \u{201e}{}\u{201c}.", dev, port),
            ActErr::UnknownLink(l) => msg!("Die Verbindung {} gibt es nicht.", l),
            ActErr::UnknownModel(m) => msg!("Unbekanntes Gerätemodell \u{201e}{}\u{201c}.", m),
            ActErr::DeviceExists(d) => msg!("Das Gerät {} läuft schon.", d),
            ActErr::LinkExists(l) => msg!("Die Verbindung {} besteht schon.", l),
            ActErr::PortBusy { dev, port } => msg!("Der Port {} von Gerät {} ist schon verbunden.", port, dev),
            ActErr::NoBridge(d) => msg!("Gerät {} ist kein Switch.", d),
            ActErr::BadVlan(v) => msg!("VLAN {} gibt es nicht (1 bis 4094).", v),
            ActErr::Cable(w) => w.msg(),
            ActErr::NoIp(d) => msg!("Gerät {} kann kein IP.", d),
            ActErr::UnknownIface { dev, iface } => msg!("Gerät {} hat kein Interface \u{201e}{}\u{201c}.", dev, iface),
            ActErr::BadMtu(m) => msg!("Die MTU {} geht nicht (68 bis 65535).", m),
            ActErr::Refused(why) => why.clone(),
        }
    }
}

impl fmt::Display for ActErr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.msg().text())
    }
}

/// a refusal of IpStack::add_addr as the message it was made from: ip.rs words its three
/// refusals as German text (until add_addr gives a Msg itself); anything else stays as it is
fn addr_refused(s: String) -> ActErr {
    let m = if let Some(p) = s.strip_suffix(" ist keine gültige Interface-Adresse.") {
        msg!("{} ist keine gültige Interface-Adresse.", p)
    } else if let Some((p, other)) = s.strip_suffix('.').and_then(|x| x.split_once(" überschneidet sich mit ")) {
        msg!("{} überschneidet sich mit {}.", p, other)
    } else if s == "Unbekanntes Interface." {
        msg!("Unbekanntes Interface.")
    } else {
        Msg::plain(s)
    };
    ActErr::Refused(m)
}

impl Engine {
    /// applies a change
    pub fn apply(&mut self, a: Action) -> Result<(), ActErr> {
        // a routing process looks at what the change did to its device (rip.rs)
        let touched = crate::rip::touches(&a);
        let r = self.apply_one(a);
        if let Some(dev) = touched {
            self.rip_kick_dev(dev);
        }
        r
    }

    fn apply_one(&mut self, a: Action) -> Result<(), ActErr> {
        self.trace.new_batch();
        match a {
            Action::AddDevice(d) => {
                self.add_node(&d)?;
                // it boots with its configuration (an undone delete keeps its running one)
                self.cli_boot(d.id, &d.startup_config);
                Ok(())
            }
            Action::RemoveDevice(dev) => self.remove_node(dev),
            Action::Connect(l) => self.add_link(&l).map(|_| ()),
            Action::Disconnect(id) => self.remove_link(id),
            Action::Power { dev, on } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                if let Some(Some(n)) = self.nodes.get_mut(ni) {
                    n.power = on;
                    if !on {
                        if let Some(b) = n.bridge.as_mut() {
                            b.fdb.retain(|_, e| e.fixed);
                        }
                        if let Some(st) = n.ip.as_mut() {
                            st.reset();
                        }
                        // timers set before the power went off never fire
                        for g in n.timers.iter_mut() {
                            *g = g.wrapping_add(1);
                        }
                    }
                }
                if !on {
                    self.end_jobs_of(ni);
                    self.stp_power(ni, false);
                }
                self.refresh_node_links(ni);
                self.os_power(ni, on);
                if on {
                    self.stp_power(ni, true);
                }
                Ok(())
            }
            Action::PortAdmin { dev, port, up } => {
                let (ni, pi) = self.resolve(dev, &port)?;
                let li = match self.nodes.get_mut(ni) {
                    Some(Some(n)) => {
                        n.ports[pi as usize].admin_up = up;
                        // "shutdown" ends an err-disabled state (stp.rs)
                        if !up {
                            n.ports[pi as usize].err_disabled = false;
                        }
                        n.ports[pi as usize].link
                    }
                    _ => None,
                };
                if let Some(li) = li {
                    self.refresh_link(li);
                }
                Ok(())
            }
            Action::Unplug { link, down } => {
                let li = *self.link_idx.get(&link).ok_or(ActErr::UnknownLink(link))?;
                if let Some(Some(l)) = self.links.get_mut(li) {
                    l.down = down;
                }
                self.refresh_link(li);
                Ok(())
            }
            Action::Impair { link, kbps, delay_ns, jitter_ns, loss_ppm } => {
                let li = *self.link_idx.get(&link).ok_or(ActErr::UnknownLink(link))?;
                if let Some(Some(l)) = self.links.get_mut(li) {
                    if let Some(k) = kbps {
                        l.kbps = k.max(1);
                    }
                    if let Some(d) = delay_ns {
                        l.delay_ns = d;
                    }
                    if let Some(j) = jitter_ns {
                        l.jitter_ns = j;
                    }
                    if let Some(p) = loss_ppm {
                        l.loss_ppm = p.min(1_000_000);
                    }
                }
                Ok(())
            }
            Action::SetCable { link, cable } => {
                let li = *self.link_idx.get(&link).ok_or(ActErr::UnknownLink(link))?;
                let (pa, pb) = self.link_ports(li).ok_or(ActErr::UnknownLink(link))?;
                cable::cable_fits(cable, &pa, &pb).map_err(ActErr::Cable)?;
                // a changed cable is pulled and plugged in again: what was on it is lost
                let was_down = match self.links.get_mut(li) {
                    Some(Some(l)) => core::mem::replace(&mut l.down, true),
                    _ => true,
                };
                self.refresh_link(li);
                if let Some(Some(l)) = self.links.get_mut(li) {
                    l.cable = cable;
                    l.wrong_cable = !crate::link::right_cable(cable, &pa, &pb);
                    l.down = was_down;
                }
                self.refresh_link(li);
                Ok(())
            }
            Action::Inject { dev, port, frame } => {
                let (ni, pi) = self.resolve(dev, &port)?;
                self.send(ni, pi, Arc::from(frame), None);
                Ok(())
            }
            Action::Access { dev, port, vlan } => {
                let (ni, pi) = self.resolve(dev, &port)?;
                let b = self.bridge_mut(ni, dev)?;
                if b.set_access(pi, vlan) { Ok(()) } else { Err(ActErr::BadVlan(vlan)) }
            }
            Action::Trunk { dev, port, native, allowed } => {
                let (ni, pi) = self.resolve(dev, &port)?;
                let b = self.bridge_mut(ni, dev)?;
                if b.set_trunk(pi, native, allowed) { Ok(()) } else { Err(ActErr::BadVlan(native)) }
            }
            Action::Vlan { dev, vid, name, remove } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                let b = self.bridge_mut(ni, dev)?;
                let ok = if remove { b.remove_vlan(vid) } else { b.add_vlan(vid, name.as_deref()) };
                if ok { Ok(()) } else { Err(ActErr::BadVlan(vid)) }
            }
            Action::IfAddr { dev, iface, prefix, add } => {
                let (ni, i) = self.ip_iface(dev, &iface)?;
                let st = self.ip_mut(ni, dev)?;
                if add {
                    st.add_addr(i, prefix).map_err(addr_refused)
                } else if st.remove_addr(i, prefix) {
                    Ok(())
                } else {
                    Err(ActErr::Refused(msg!("{} ist an {} nicht eingerichtet.", prefix, iface)))
                }
            }
            Action::IfUp { dev, iface, up } => {
                let (ni, i) = self.ip_iface(dev, &iface)?;
                let kind = self.stack(ni).map(|s| s.ifaces[i as usize].kind);
                if let Some(IfKind::Port(p)) = kind {
                    let li = match self.nodes.get_mut(ni) {
                        Some(Some(n)) => {
                            n.ports[p as usize].admin_up = up;
                            if !up {
                                n.ports[p as usize].err_disabled = false;
                            }
                            n.ports[p as usize].link
                        }
                        _ => None,
                    };
                    if let Some(li) = li {
                        self.refresh_link(li);
                    }
                    return Ok(());
                }
                let st = self.ip_mut(ni, dev)?;
                st.ifaces[i as usize].admin_up = up;
                if !up {
                    st.flush_arp(i);
                }
                Ok(())
            }
            Action::Mtu { dev, iface, mtu } => {
                if mtu < 68 {
                    return Err(ActErr::BadMtu(mtu));
                }
                let (ni, i) = self.ip_iface(dev, &iface)?;
                self.ip_mut(ni, dev)?.ifaces[i as usize].mtu = mtu;
                Ok(())
            }
            Action::ProxyArp { dev, iface, on } => {
                let (ni, i) = self.ip_iface(dev, &iface)?;
                self.ip_mut(ni, dev)?.ifaces[i as usize].proxy_arp = on;
                Ok(())
            }
            Action::Svi { dev, vlan, add } => self.svi(dev, vlan, add),
            Action::SubIf { dev, name, vlan, add } => self.sub_if(dev, &name, vlan, add),
            Action::Loopback { dev, n, add } => self.loopback(dev, n, add),
            Action::RoutedPort { dev, port, routed } => self.routed_port(dev, &port, routed),
            Action::Route { dev, prefix, via, iface, ad, add } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                let st = self.ip_mut(ni, dev)?;
                if !add {
                    return if st.remove_route(prefix, via) { Ok(()) } else { Err(ActErr::Refused(msg!("Keine passende Route zu {}.", prefix.trunc()))) };
                }
                let out = match &iface {
                    Some(name) => Some(st.iface_by_name(name).ok_or_else(|| ActErr::UnknownIface { dev, iface: name.clone() })?),
                    None => None,
                };
                if via.is_none() && out.is_none() {
                    return Err(ActErr::Refused(msg!("Eine Route braucht einen nächsten Hop oder ein Interface.")));
                }
                if let Some(gw) = via
                    && (st.is_mine(gw) || gw.is_unspecified() || gw.is_multicast() || gw.is_broadcast() || gw.is_loopback())
                {
                    return Err(ActErr::Refused(msg!("{} taugt nicht als nächster Hop.", gw)));
                }
                st.add_route(Route { prefix, via, iface: out, src: RouteSrc::Static, ad, metric: 0 });
                Ok(())
            }
            Action::Forwarding { dev, on } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                self.ip_mut(ni, dev)?.forwarding = on;
                Ok(())
            }
            Action::StaticArp { dev, ip, mac } => self.static_arp(dev, ip, mac),
            Action::ClearArp { dev } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                self.ip_mut(ni, dev)?.arp.retain(|_, e| e.fixed);
                Ok(())
            }
            Action::Switchport { dev, port, set } => {
                let (ni, pi) = self.resolve(dev, &port)?;
                let bad = match &set {
                    SwSet::Access(v) | SwSet::Native(v) => Some(*v),
                    _ => None,
                };
                let b = self.bridge_mut(ni, dev)?;
                if !b.ports.get(pi as usize).is_some_and(|p| p.member) {
                    return Err(ActErr::Refused(msg!("{} ist kein Switch-Port.", port)));
                }
                if b.switchport(pi, set) { Ok(()) } else { Err(ActErr::BadVlan(bad.unwrap_or(0))) }
            }
            Action::Hostname { dev, name } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                if !jfn_model::project::valid_name(&name) {
                    return Err(ActErr::Refused(msg!("\u{201e}{}\u{201c} ist kein gültiger Gerätename.", name)));
                }
                if let Some(Some(n)) = self.nodes.get_mut(ni) {
                    n.name = name;
                }
                Ok(())
            }
            Action::Startup { dev, text } => {
                let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
                if let Some(Some(n)) = self.nodes.get_mut(ni) {
                    n.cli.cfg.startup = text;
                }
                Ok(())
            }
            Action::Stp { dev, set } => self.stp_apply(dev, set),
            Action::Rip { dev, set } => self.rip_apply(dev, set),
            // services and the virtual OS (svcact.rs)
            other => self.os_apply(other),
        }
    }

    /// the IPv4 part of node `ni`
    fn ip_mut(&mut self, ni: usize, dev: u32) -> Result<&mut IpStack, ActErr> {
        match self.nodes.get_mut(ni) {
            Some(Some(n)) => n.ip.as_mut().ok_or(ActErr::NoIp(dev)),
            _ => Err(ActErr::UnknownDevice(dev)),
        }
    }

    /// (node slot, interface) of a device's IPv4 interface by any spelling
    fn ip_iface(&self, dev: u32, name: &str) -> Result<(usize, u16), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let st = self.stack(ni).ok_or(ActErr::NoIp(dev))?;
        let i = st.iface_by_name(name).ok_or_else(|| ActErr::UnknownIface { dev, iface: String::from(name) })?;
        Ok((ni, i))
    }

    fn svi(&mut self, dev: u32, vlan: u16, add: bool) -> Result<(), ActErr> {
        if !(1..=4094).contains(&vlan) {
            return Err(ActErr::BadVlan(vlan));
        }
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let Some(Some(n)) = self.nodes.get_mut(ni) else {
            return Err(ActErr::UnknownDevice(dev));
        };
        if n.bridge.is_none() {
            return Err(ActErr::NoBridge(dev));
        }
        let (ios, mac) = (n.model.cli == CliStyle::Ios, n.base_mac);
        let st = n.ip.as_mut().ok_or(ActErr::NoIp(dev))?;
        let have = st.iface_of_vlan(vlan);
        if !add {
            return match have {
                Some(i) if !st.ifaces[i as usize].hidden => {
                    st.hide(i);
                    Ok(())
                }
                _ => Err(ActErr::UnknownIface { dev, iface: format!("Vlan{}", vlan) }),
            };
        }
        match have {
            Some(i) => {
                let f = &mut st.ifaces[i as usize];
                if f.hidden {
                    f.hidden = false;
                    f.admin_up = false;
                    f.mtu = 1500;
                }
            }
            None => {
                let name = if ios { format!("Vlan{}", vlan) } else { format!("br0.{}", vlan) };
                st.ifaces.push(Iface { name, kind: IfKind::Svi(vlan), mac, addrs: Vec::new(), mtu: 1500, admin_up: false, hidden: false, proxy_arp: ios });
            }
        }
        Ok(())
    }

    fn loopback(&mut self, dev: u32, n: u16, add: bool) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let Some(Some(node)) = self.nodes.get_mut(ni) else {
            return Err(ActErr::UnknownDevice(dev));
        };
        let name = if node.model.cli == CliStyle::Ios { format!("Loopback{}", n) } else { format!("dummy{}", n) };
        let st = node.ip.as_mut().ok_or(ActErr::NoIp(dev))?;
        let have = st.find_name(&name);
        if !add {
            return match have {
                Some(i) if !st.ifaces[i as usize].hidden => {
                    st.hide(i);
                    Ok(())
                }
                _ => Err(ActErr::UnknownIface { dev, iface: name }),
            };
        }
        match have {
            Some(i) => {
                let f = &mut st.ifaces[i as usize];
                if f.hidden {
                    f.hidden = false;
                    f.admin_up = true;
                    f.mtu = 1500;
                }
            }
            None => st.ifaces.push(Iface { name, kind: IfKind::Loopback(n), mac: Mac::ZERO, addrs: Vec::new(), mtu: 1500, admin_up: true, hidden: false, proxy_arp: false }),
        }
        Ok(())
    }

    fn routed_port(&mut self, dev: u32, port: &str, routed: bool) -> Result<(), ActErr> {
        let (ni, pi) = self.resolve(dev, port)?;
        let Some(Some(n)) = self.nodes.get_mut(ni) else {
            return Err(ActErr::UnknownDevice(dev));
        };
        let (pname, pmac, media) = {
            let p = &n.ports[pi as usize];
            (p.spec.name.clone(), p.mac, p.spec.media)
        };
        let ios = n.model.cli == CliStyle::Ios;
        if !n.model.has(F_ROUTER) {
            return Err(ActErr::Refused(msg!("{} ist ein reiner Layer-2-Switch: {} bleibt ein Switch-Port.", n.name, pname)));
        }
        let b = n.bridge.as_mut().ok_or(ActErr::NoBridge(dev))?;
        if !media.is_ethernet() {
            return Err(ActErr::Refused(msg!("{} ist kein Ethernet-Port.", pname)));
        }
        b.ports[pi as usize].member = !routed;
        b.flush_port(pi);
        let st = n.ip.as_mut().ok_or(ActErr::NoIp(dev))?;
        let have = st.iface_of_port(pi);
        match (routed, have) {
            (true, Some(i)) => {
                let f = &mut st.ifaces[i as usize];
                if f.hidden {
                    f.hidden = false;
                    f.admin_up = true;
                    f.mtu = 1500;
                }
            }
            (true, None) => st.ifaces.push(Iface { name: pname, kind: IfKind::Port(pi), mac: pmac, addrs: Vec::new(), mtu: 1500, admin_up: true, hidden: false, proxy_arp: ios }),
            (false, Some(i)) => {
                st.hide(i);
                let subs: Vec<u16> = (0..st.ifaces.len() as u16).filter(|&s| matches!(st.ifaces[s as usize].kind, IfKind::Sub { port, .. } if port == pi)).collect();
                for s in subs {
                    st.hide(s);
                }
            }
            (false, None) => {}
        }
        // a routed port leaves the spanning tree, a switch port joins it
        self.stp_link(ni, pi);
        Ok(())
    }

    fn sub_if(&mut self, dev: u32, name: &str, vlan: u16, add: bool) -> Result<(), ActErr> {
        let not_sub = || ActErr::Refused(msg!("\u{201e}{}\u{201c} ist kein Subinterface (Port.Nummer).", name));
        let (base, num) = name.trim().rsplit_once('.').ok_or_else(not_sub)?;
        let num: u32 = num.parse().map_err(|_| not_sub())?;
        let (ni, pi) = self.resolve(dev, base)?;
        let Some(Some(n)) = self.nodes.get_mut(ni) else {
            return Err(ActErr::UnknownDevice(dev));
        };
        let ios = n.model.cli == CliStyle::Ios;
        let (pname, pmac, media) = {
            let p = &n.ports[pi as usize];
            (p.spec.name.clone(), p.mac, p.spec.media)
        };
        if !media.is_ethernet() {
            return Err(ActErr::Refused(msg!("{} kann kein 802.1Q.", pname)));
        }
        let full = format!("{}.{}", pname, num);
        let st = n.ip.as_mut().ok_or(ActErr::NoIp(dev))?;
        if !st.iface_of_port(pi).is_some_and(|i| !st.ifaces[i as usize].hidden) {
            return Err(ActErr::Refused(msg!("{} ist ein Switch-Port: erst \u{201e}no switchport\u{201c}.", pname)));
        }
        let have = st.find_name(&full);
        if !add {
            return match have {
                Some(i) if !st.ifaces[i as usize].hidden => {
                    st.hide(i);
                    Ok(())
                }
                _ => Err(ActErr::UnknownIface { dev, iface: full }),
            };
        }
        if !(1..=4094).contains(&vlan) {
            return Err(ActErr::BadVlan(vlan));
        }
        if st.ifaces.iter().any(|f| !f.hidden && f.kind == IfKind::Sub { port: pi, vlan } && !f.name.eq_ignore_ascii_case(&full)) {
            return Err(ActErr::Refused(msg!("VLAN {} hat schon ein anderes Subinterface von {}.", vlan, pname)));
        }
        let kind = IfKind::Sub { port: pi, vlan };
        match have {
            Some(i) => {
                let f = &mut st.ifaces[i as usize];
                let moved = f.kind != kind;
                f.kind = kind;
                if f.hidden {
                    f.hidden = false;
                    f.admin_up = ios;
                    f.mtu = 1500;
                }
                if moved {
                    st.flush_arp(i);
                }
            }
            None => st.ifaces.push(Iface { name: full, kind, mac: pmac, addrs: Vec::new(), mtu: 1500, admin_up: ios, hidden: false, proxy_arp: ios }),
        }
        Ok(())
    }

    fn static_arp(&mut self, dev: u32, ip: Ipv4, mac: Option<Mac>) -> Result<(), ActErr> {
        let now = self.now;
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let st = self.ip_mut(ni, dev)?;
        let Some(i) = st.ifaces.iter().position(|f| !f.hidden && !f.is_loopback() && f.on_link(ip)).map(|i| i as u16) else {
            return Err(ActErr::Refused(msg!("{} liegt in keinem Netz dieses Geräts.", ip)));
        };
        let Some(m) = mac else {
            return match st.arp.get(&(i, ip)) {
                Some(e) if e.fixed => {
                    st.arp.remove(&(i, ip));
                    Ok(())
                }
                _ => Err(ActErr::Refused(msg!("Für {} gibt es keinen statischen ARP-Eintrag.", ip))),
            };
        };
        if !m.is_unicast() || m.is_zero() {
            return Err(ActErr::Refused(msg!("{} taugt nicht als ARP-Eintrag.", m)));
        }
        let old = st.arp.insert((i, ip), ArpEntry { mac: Some(m), at: now, tries: 0, queue: Vec::new(), fixed: true });
        // packets that waited for an answer go now
        for p in old.map(|e| e.queue).unwrap_or_default() {
            self.frame_out(ni, i, m, ET_IPV4, &p.pkt, p.cause);
        }
        Ok(())
    }

    /// (node slot, port index) of a device's port by any spelling
    pub(crate) fn resolve(&self, dev: u32, port: &str) -> Result<(usize, u16), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let pi = self.port_index(dev, port).ok_or_else(|| ActErr::UnknownPort { dev, port: String::from(port) })?;
        Ok((ni, pi))
    }

    fn bridge_mut(&mut self, ni: usize, dev: u32) -> Result<&mut crate::bridge::Bridge, ActErr> {
        match self.nodes.get_mut(ni) {
            Some(Some(n)) => n.bridge.as_mut().ok_or(ActErr::NoBridge(dev)),
            _ => Err(ActErr::UnknownDevice(dev)),
        }
    }

    /// the port specs at both ends of a link
    fn link_ports(&self, li: usize) -> Option<(jfn_model::PortSpec, jfn_model::PortSpec)> {
        let l = self.links.get(li)?.as_ref()?;
        let spec = |(n, p): (usize, u16)| self.nodes.get(n)?.as_ref().map(|node| node.ports[p as usize].spec.clone());
        Some((spec(l.ends[0])?, spec(l.ends[1])?))
    }

    /// a device of the project starts running
    pub(crate) fn add_node(&mut self, d: &Device) -> Result<usize, ActErr> {
        if self.dev_idx.contains_key(&d.id) {
            return Err(ActErr::DeviceExists(d.id));
        }
        let m = catalog::model(&d.model).ok_or_else(|| ActErr::UnknownModel(d.model.clone()))?;
        let node = Node::new(d.id, &d.name, m, !d.off, self.seed);
        self.nodes.push(Some(node));
        let ni = self.nodes.len() - 1;
        self.dev_idx.insert(d.id, ni);
        // the services of the project start when the engine runs (a bad one is left out)
        for s in &d.services {
            let _ = self.service_def(d.id, s);
        }
        Ok(ni)
    }

    pub(crate) fn remove_node(&mut self, dev: u32) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        let links: Vec<u32> = match self.nodes.get(ni) {
            Some(Some(n)) => n.ports.iter().filter_map(|p| p.link).filter_map(|li| self.links.get(li)?.as_ref().map(|l| l.id)).collect(),
            _ => Vec::new(),
        };
        for id in links {
            self.remove_link(id)?;
        }
        self.end_jobs_of(ni);
        self.nodes[ni] = None;
        self.dev_idx.remove(&dev);
        Ok(())
    }

    /// a cable of the project is plugged in
    pub(crate) fn add_link(&mut self, l: &Link) -> Result<usize, ActErr> {
        if self.link_idx.contains_key(&l.id) {
            return Err(ActErr::LinkExists(l.id));
        }
        let (na, pa) = self.resolve(l.a.dev, &l.a.port)?;
        let (nb, pb) = self.resolve(l.b.dev, &l.b.port)?;
        if na == nb {
            return Err(ActErr::Cable(Why::SameDevice));
        }
        let spec = |e: &Engine, n: usize, p: u16| match e.nodes.get(n) {
            Some(Some(node)) => Some((node.ports[p as usize].spec.clone(), node.ports[p as usize].link.is_some())),
            _ => None,
        };
        let (sa, busy_a) = spec(self, na, pa).ok_or(ActErr::UnknownDevice(l.a.dev))?;
        let (sb, busy_b) = spec(self, nb, pb).ok_or(ActErr::UnknownDevice(l.b.dev))?;
        if busy_a {
            return Err(ActErr::PortBusy { dev: l.a.dev, port: sa.name });
        }
        if busy_b {
            return Err(ActErr::PortBusy { dev: l.b.dev, port: sb.name });
        }
        cable::cable_fits(l.cable, &sa, &sb).map_err(ActErr::Cable)?;
        let kbps = if l.kbps == 0 { cable::link_kbps(&sa, &sb) } else { l.kbps };
        let rt = LinkRt {
            id: l.id,
            ends: [(na, pa), (nb, pb)],
            cable: l.cable,
            kbps,
            delay_ns: l.delay_ns,
            jitter_ns: l.jitter_ns,
            loss_ppm: l.loss_ppm.min(1_000_000),
            down: l.down,
            wrong_cable: !crate::link::right_cable(l.cable, &sa, &sb),
            epoch: 0,
            was_up: false,
            dir: [Dir::default(), Dir::default()],
            rng: Pcg32::derive(self.seed ^ 0x4C49_4E4B, l.id as u64),
        };
        self.links.push(Some(rt));
        let li = self.links.len() - 1;
        self.link_idx.insert(l.id, li);
        for (n, p) in [(na, pa), (nb, pb)] {
            if let Some(Some(node)) = self.nodes.get_mut(n) {
                node.ports[p as usize].link = Some(li);
            }
        }
        self.refresh_link(li);
        Ok(li)
    }

    pub(crate) fn remove_link(&mut self, id: u32) -> Result<(), ActErr> {
        let li = *self.link_idx.get(&id).ok_or(ActErr::UnknownLink(id))?;
        if let Some(Some(l)) = self.links.get_mut(li) {
            l.down = true;
        }
        self.refresh_link(li);
        if let Some(Some(l)) = self.links.get(li) {
            for (n, p) in l.ends {
                if let Some(Some(node)) = self.nodes.get_mut(n) {
                    node.ports[p as usize].link = None;
                }
            }
        }
        self.links[li] = None;
        self.link_idx.remove(&id);
        Ok(())
    }
}
