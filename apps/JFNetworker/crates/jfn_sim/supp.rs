// supp -- the 802.1X supplicant of hosts: a wpa_supplicant-like configuration file with network
// blocks, the hidden service "wpa_supplicant" that logs a wired port or a WLAN station in with EAP
// (the crate eapm), and what wpa_cli asks of it
//
//   CONF                          "/etc/wpa_supplicant/wpa_supplicant.conf": the file
//   NetBlock { ssid, key_mgmt, eap, identity, password }   one "network={ ... }"
//   KeyMgmt                       Ieee8021x (a wired port), WpaEap (WPA-Enterprise), Other
//   parse_conf(text) / conf_text(blocks)   the file to blocks and back
//   Supplicant                    the service: Supplicant::new(path)
//   SuppView { port, name, state, user, method, since, authorized_at }   what wpa_cli status says
//   Engine::supp_boot(ni)         a device whose file exists starts it (boot, a new device)
//   Engine::supp_reconfigure(ni)  reads the file again ("wpa_cli reconfigure", Action::Supplicant)
//   Engine::supp_rx(ni, port, frame, hop)   an EAPOL frame for a host port (link.rs)
//   Engine::supp_wlan_rx(...)     the same for a station's EAP on its radio link (wlan.rs)
//   Engine::supp_command(ni, cmd, port)   logon, logoff, reauthenticate
//   Engine::supp_views(dev)       the state per port
//   Engine::wlan_creds(dev, ssid) what a station logs in with: its WLAN settings (identity,
//                                 passphrase as the password, method), else a block of the file
//
// The file is read when the service starts, on "wpa_cli reconfigure" and "wpa_supplicant ... -c
// FILE": a change to it counts then, as with the real program.  A block with key_mgmt=IEEE8021X
// holds for the wired ports of the host (no ssid), one with key_mgmt=WPA-EAP for the network of
// its ssid; other key_mgmt words are kept in the file and do nothing here.
//
// A wired port with a block: when its link is up the supplicant sends EAPOL-Start after 200 ms if
// no request came (the authenticator usually asks first), again every 30 s, three times in all;
// after that the port goes on without 802.1X, and a later request still gets its answer.  Each
// EAP-Request is answered by the method; EAP-Success counts only when the method ran, and it tells
// a DHCP client of the port without a lease to start at once (as a dhclient hook does, since its
// first tries hit a closed port).  EAP-Failure holds the port back for 60 s.
//
// A WLAN station: the access point asks after the association; the MSK of the method goes to the
// association as its PMK (wlan.rs), EAP-MD5 gives none.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use eapm::{Method, Peer, PeerOut};
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::eap;
use netpkt::eapol::{self, EapolView};
use netpkt::eth::L2;
use netpkt::Mac;

use crate::dot1x::eth_frame;
use crate::dot1xtrace::DNote;
use crate::http::Wheel;
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{Ctl, Metric, OsApi, Service, SvcEvent};
use crate::trace::{HopId, Note};

/// the name of the hidden service
pub const SVC: &str = "wpa_supplicant";
/// where the configuration lies
pub const CONF: &str = "/etc/wpa_supplicant/wpa_supplicant.conf";

const K_WHEEL: u32 = 0;
const START_DELAY: Ns = 200 * MS;
const START_PERIOD: Ns = 30 * SEC;
const MAX_STARTS: u8 = 3;
const HELD: Ns = 60 * SEC;

/// key_mgmt of a block
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum KeyMgmt {
    /// IEEE8021X: a wired port
    #[default]
    Ieee8021x,
    /// WPA-EAP: an enterprise network
    WpaEap,
    /// anything else, kept as it was written
    Other(String),
}

/// a network block of the file
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct NetBlock {
    pub ssid: String,
    pub key_mgmt: KeyMgmt,
    /// the EAP method (None: not said; the supplicant then takes EAP-MD5 on a wire, EAP-PWD on air)
    pub eap: Option<Method>,
    pub identity: String,
    pub password: String,
}

/// a quoted or plain value
fn unquote(v: &str) -> String {
    let v = v.trim();
    String::from(v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(v))
}

/// the network blocks of a file (lines that mean nothing here are skipped)
pub fn parse_conf(text: &str) -> Vec<NetBlock> {
    let mut out = Vec::new();
    let mut cur: Option<NetBlock> = None;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if cur.is_none() {
            if l.replace(' ', "") == "network={" {
                cur = Some(NetBlock::default());
            }
            continue;
        }
        if l == "}" {
            if let Some(b) = cur.take() {
                out.push(b);
            }
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        let Some(b) = cur.as_mut() else { continue };
        match k.trim() {
            "ssid" => b.ssid = unquote(v),
            "identity" => b.identity = unquote(v),
            "password" => b.password = unquote(v),
            "eap" => b.eap = Method::parse(v.split_whitespace().next().unwrap_or("")),
            "key_mgmt" => {
                let w = v.trim();
                b.key_mgmt = match w {
                    "IEEE8021X" => KeyMgmt::Ieee8021x,
                    "WPA-EAP" | "WPA-EAP-SHA256" => KeyMgmt::WpaEap,
                    x => KeyMgmt::Other(String::from(x)),
                };
            }
            _ => {}
        }
    }
    out
}

/// the blocks as a file
pub fn conf_text(blocks: &[NetBlock]) -> String {
    let mut s = String::from("# 802.1X and enterprise WLAN of this host (the desktop's IP-Konfiguration writes it too)\nctrl_interface=/run/wpa_supplicant\n");
    for b in blocks {
        s.push_str("\nnetwork={\n");
        if !b.ssid.is_empty() {
            s.push_str(&format!("    ssid=\"{}\"\n", b.ssid));
        }
        let km = match &b.key_mgmt {
            KeyMgmt::Ieee8021x => "IEEE8021X",
            KeyMgmt::WpaEap => "WPA-EAP",
            KeyMgmt::Other(o) => o.as_str(),
        };
        s.push_str(&format!("    key_mgmt={}\n", km));
        if let Some(m) = b.eap {
            s.push_str(&format!("    eap={}\n", m.word()));
        }
        if !b.identity.is_empty() {
            s.push_str(&format!("    identity=\"{}\"\n", b.identity));
        }
        if !b.password.is_empty() {
            s.push_str(&format!("    password=\"{}\"\n", b.password));
        }
        s.push_str("}\n");
    }
    s
}

/// what a port's supplicant does
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuppState {
    /// the link is down, or logged off
    Off,
    /// EAPOL-Start goes out; no request yet
    Connecting,
    /// nobody answered its Starts
    NoPeer,
    Authenticating,
    Authenticated,
    /// refused: it waits before it answers again
    Held,
    /// it has nothing to log in with
    NoCreds,
}

/// a deadline
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Due {
    Start(u16),
    Held(u16),
}

/// a port of the supplicant
#[derive(Clone, Debug)]
struct SPort {
    state: SuppState,
    peer: Option<Peer>,
    starts: u8,
    timer: Option<u64>,
    user: String,
    method: String,
    since: Ns,
    /// the association whose radio link it runs on
    assoc: Option<u32>,
}

impl SPort {
    fn new(now: Ns) -> SPort {
        SPort { state: SuppState::Off, peer: None, starts: 0, timer: None, user: String::new(), method: String::new(), since: now, assoc: None }
    }
}

/// a port as wpa_cli sees it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuppView {
    pub port: u16,
    pub name: String,
    pub state: SuppState,
    pub user: String,
    pub method: String,
    /// since when it is in this state
    pub since: Ns,
}

/// the hidden service of a supplicant
#[derive(Clone, Debug)]
pub struct Supplicant {
    pub path: String,
    pub conf: Vec<NetBlock>,
    ports: BTreeMap<u16, SPort>,
    wheel: Wheel<Due>,
    pub logins: u64,
    pub refused: u64,
}

impl Supplicant {
    pub fn new(path: &str) -> Supplicant {
        Supplicant { path: String::from(path), conf: Vec::new(), ports: BTreeMap::new(), wheel: Wheel::new(K_WHEEL), logins: 0, refused: 0 }
    }

    fn read_conf(&mut self, os: &mut OsApi) {
        self.conf = os.file_read(&self.path).map(|b| parse_conf(&String::from_utf8_lossy(&b))).unwrap_or_default();
    }

    /// the block of the wired ports
    fn wired(&self) -> Option<&NetBlock> {
        self.conf.iter().find(|b| b.key_mgmt == KeyMgmt::Ieee8021x)
    }

    fn name_of(os: &OsApi, port: u16) -> String {
        os.eng.nodes.get(os.ni).and_then(|n| n.as_ref()).map(|n| String::from(n.port_name(port))).unwrap_or_default()
    }

    fn cancel(&mut self, port: u16) {
        if let Some(t) = self.ports.get_mut(&port).and_then(|p| p.timer.take()) {
            self.wheel.cancel(t);
        }
    }

    /// the wired ports with a link start or stop
    pub fn sync(&mut self, os: &mut OsApi) {
        let ni = os.ni;
        let Some(Some(n)) = os.eng.nodes.get(ni) else { return };
        let wired: Vec<u16> = n
            .ports
            .iter()
            .enumerate()
            .filter(|(i, p)| p.spec.media.is_ethernet() && !p.spec.media.is_radio() && !n.bridge.as_ref().is_some_and(|b| b.ports.get(*i).is_some_and(|x| x.member)))
            .map(|(i, _)| i as u16)
            .collect();
        let have = self.wired().is_some();
        for p in wired {
            let up = os.eng.nodes.get(ni).and_then(|n| n.as_ref()).and_then(|n| n.ports[p as usize].link).is_some_and(|li| os.eng.link_up(li));
            let now = os.now();
            let st = self.ports.get(&p).map(|x| x.state);
            if up && have {
                if matches!(st, None | Some(SuppState::Off)) {
                    self.cancel(p);
                    let x = self.ports.entry(p).or_insert_with(|| SPort::new(now));
                    x.state = SuppState::Connecting;
                    x.starts = 0;
                    x.since = now;
                    let id = self.wheel.add(now + START_DELAY, Due::Start(p));
                    if let Some(x) = self.ports.get_mut(&p) {
                        x.timer = Some(id);
                    }
                }
            } else if st.is_some() {
                self.cancel(p);
                let x = self.ports.entry(p).or_insert_with(|| SPort::new(now));
                x.state = SuppState::Off;
                x.peer = None;
                x.since = now;
            }
        }
        self.wheel.arm(os);
    }

    /// reads the file again: every port starts over
    pub fn reconfigure(&mut self, os: &mut OsApi) {
        self.read_conf(os);
        let ports: Vec<u16> = self.ports.keys().copied().collect();
        for p in ports {
            self.cancel(p);
        }
        self.ports.retain(|_, p| p.assoc.is_some());
        self.sync(os);
    }

    /// sends an EAPOL PDU from a port
    fn send(&mut self, os: &mut OsApi, port: u16, pdu: &[u8]) {
        let ni = os.ni;
        let cause = os.cause;
        let Some(mac) = os.eng.nodes.get(ni).and_then(|n| n.as_ref()).and_then(|n| n.ports.get(port as usize)).map(|p| p.mac) else { return };
        let x = self.ports.get(&port);
        match x.and_then(|x| x.assoc) {
            Some(a) => {
                let Some(y) = os.eng.wlan.assocs.get(&a) else { return };
                let (li, bssid) = (y.li, y.bssid);
                os.eng.send_on(li, 0, eth_frame(bssid, mac, pdu), cause);
            }
            None => {
                // the group address: a bridge keeps it, as 802.1X-2004 7.8 asks on a wire
                os.eng.send(ni, port, eth_frame(Mac(eapol::PAE_GROUP), mac, pdu), cause);
            }
        }
    }

    fn send_start(&mut self, os: &mut OsApi, port: u16) {
        let name = Self::name_of(os, port);
        os.note(Note::Dot1x(DNote::SuppStart { port: name }));
        self.send(os, port, &eapol::start(eapol::VERSION_2004));
    }

    /// what this port logs in with
    fn creds_of(&self, os: &OsApi, port: u16) -> Option<(String, String, Method)> {
        match self.ports.get(&port).and_then(|x| x.assoc) {
            Some(_) => {
                let dev = os.dev();
                let ssid = os.eng.wlan.stas.get(&dev).and_then(|s| s.join.as_ref()).map(|j| j.ssid.clone())?;
                os.eng.wlan_creds(dev, &ssid)
            }
            None => {
                let b = self.wired()?;
                if b.identity.is_empty() {
                    return None;
                }
                Some((b.identity.clone(), b.password.clone(), b.eap.unwrap_or(Method::Md5)))
            }
        }
    }

    /// an EAPOL PDU from the authenticator (`assoc`: a station's radio link)
    pub fn rx(&mut self, os: &mut OsApi, port: u16, pdu: &[u8], assoc: Option<u32>) {
        let Some(v) = EapolView::parse(pdu) else { return };
        if v.kind() != eapol::T_EAP {
            return;
        }
        let b = v.body().to_vec();
        let now = os.now();
        let x = self.ports.entry(port).or_insert_with(|| SPort::new(now));
        x.assoc = assoc;
        let name = Self::name_of(os, port);
        // a held port answers nothing until its time is over (a new association starts afresh)
        if self.ports.get(&port).is_some_and(|x| x.state == SuppState::Held) && assoc.is_none() {
            return;
        }
        if self.ports.get(&port).is_some_and(|x| x.peer.is_none()) {
            match self.creds_of(os, port) {
                Some((user, pass, method)) => {
                    if let Some(x) = self.ports.get_mut(&port) {
                        x.peer = Some(Peer::new(&user, &pass, method));
                        x.user = user;
                    }
                }
                None => {
                    if let Some(x) = self.ports.get_mut(&port) {
                        x.state = SuppState::NoCreds;
                        x.since = now;
                    }
                    os.note(Note::Dot1x(DNote::SuppNoCreds { port: name }));
                    return;
                }
            }
        }
        self.cancel(port);
        let Some(mut peer) = self.ports.get_mut(&port).and_then(|x| x.peer.take()) else { return };
        let out = {
            let mut rnd = |o: &mut [u8]| {
                for c in o.chunks_mut(4) {
                    let r = os.rand().to_be_bytes();
                    let n = c.len();
                    c.copy_from_slice(&r[..n]);
                }
            };
            peer.on_packet(&b, &mut rnd)
        };
        let (msk, used) = (peer.msk().copied(), peer.method_used());
        let user = peer.identity.clone();
        if let Some(x) = self.ports.get_mut(&port) {
            x.peer = Some(peer);
        }
        os.note(Note::Dot1x(DNote::SuppEap { port: name.clone(), what: eap::info(&b) }));
        match out {
            PeerOut::Send(r) => {
                if let Some(x) = self.ports.get_mut(&port) {
                    if x.state != SuppState::Authenticating {
                        x.since = now;
                    }
                    x.state = SuppState::Authenticating;
                }
                self.send(os, port, &eapol::eap_pdu(eapol::VERSION_2004, &r));
            }
            PeerOut::Success => {
                self.logins += 1;
                let word = used.map(|m| m.word()).unwrap_or("?");
                let method = format!("EAP-{}", word);
                if let Some(x) = self.ports.get_mut(&port) {
                    x.state = SuppState::Authenticated;
                    x.method = method.clone();
                    x.since = now;
                }
                os.note(Note::Dot1x(DNote::SuppOk { port: name.clone(), user: user.clone(), method: method.clone() }));
                os.log(6, "wpa_supplicant", &format!("{}: {} is logged in with {}", name, user, method));
                match assoc {
                    Some(a) => os.eng.wlan_sta_keys(a, msk),
                    None => {
                        // what comes after a closed port: a DHCP client without a lease starts now
                        let svc = format!("dhcp-client@{}", name);
                        let ni = os.ni;
                        if let Some(id) = os.eng.os(ni).and_then(|o| o.svc_id(&svc)) {
                            os.eng.svc_ctl(ni, id, Ctl::Carrier);
                        }
                    }
                }
            }
            PeerOut::Failure => {
                self.refused += 1;
                if let Some(x) = self.ports.get_mut(&port) {
                    x.state = SuppState::Held;
                    x.since = now;
                    x.peer = None;
                }
                os.note(Note::Dot1x(DNote::SuppFail { port: name.clone(), user: user.clone() }));
                os.log(4, "wpa_supplicant", &format!("{}: the login of {} was refused", name, user));
                if assoc.is_none() {
                    let id = self.wheel.add(now + HELD, Due::Held(port));
                    if let Some(x) = self.ports.get_mut(&port) {
                        x.timer = Some(id);
                    }
                }
            }
            PeerOut::Silent => {}
        }
        self.wheel.arm(os);
    }

    /// a station's association ended: its radio link is gone
    pub fn assoc_gone(&mut self, port: u16) {
        self.cancel(port);
        self.ports.remove(&port);
    }

    fn due(&mut self, os: &mut OsApi, d: Due) {
        match d {
            Due::Start(p) => {
                let Some(x) = self.ports.get_mut(&p) else { return };
                x.timer = None;
                if x.state != SuppState::Connecting {
                    return;
                }
                if x.starts >= MAX_STARTS {
                    x.state = SuppState::NoPeer;
                    x.since = os.eng.now;
                    let name = Self::name_of(os, p);
                    os.note(Note::Dot1x(DNote::SuppNoPeer { port: name }));
                    return;
                }
                x.starts += 1;
                self.send_start(os, p);
                let id = self.wheel.add(os.now() + START_PERIOD, Due::Start(p));
                if let Some(x) = self.ports.get_mut(&p) {
                    x.timer = Some(id);
                }
            }
            Due::Held(p) => {
                let now = os.now();
                let Some(x) = self.ports.get_mut(&p) else { return };
                x.timer = None;
                if x.state == SuppState::Held {
                    x.state = SuppState::Connecting;
                    x.starts = 0;
                    x.since = now;
                    let id = self.wheel.add(now, Due::Start(p));
                    if let Some(x) = self.ports.get_mut(&p) {
                        x.timer = Some(id);
                    }
                }
            }
        }
    }

    /// "wpa_cli logoff": EAPOL-Logoff, the port stays off until "logon"
    pub fn logoff(&mut self, os: &mut OsApi) {
        let ports: Vec<u16> = self.ports.keys().copied().collect();
        let now = os.now();
        for p in ports {
            if self.ports.get(&p).is_some_and(|x| x.state == SuppState::Off) {
                continue;
            }
            self.send(os, p, &eapol::logoff(eapol::VERSION_2004));
            self.cancel(p);
            if let Some(x) = self.ports.get_mut(&p) {
                x.state = SuppState::Off;
                x.peer = None;
                x.since = now;
            }
            let name = Self::name_of(os, p);
            os.note(Note::Dot1x(DNote::SuppLogoff { port: name }));
        }
        self.wheel.arm(os);
    }

    /// "wpa_cli logon" and "wpa_cli reauthenticate": EAPOL-Start now
    pub fn logon(&mut self, os: &mut OsApi) {
        let ports: Vec<u16> = self.ports.keys().copied().collect();
        let now = os.now();
        for p in ports {
            let up = os.eng.nodes.get(os.ni).and_then(|n| n.as_ref()).and_then(|n| n.ports[p as usize].link).is_some_and(|li| os.eng.link_up(li));
            if !up {
                continue;
            }
            self.cancel(p);
            if let Some(x) = self.ports.get_mut(&p) {
                x.peer = None;
                x.state = SuppState::Connecting;
                x.starts = 1;
                x.since = now;
            }
            self.send_start(os, p);
            let id = self.wheel.add(now + START_PERIOD, Due::Start(p));
            if let Some(x) = self.ports.get_mut(&p) {
                x.timer = Some(id);
            }
        }
        self.wheel.arm(os);
    }

    /// the state per port
    pub fn views(&self, e: &Engine, ni: usize) -> Vec<SuppView> {
        self.ports
            .iter()
            .map(|(p, x)| SuppView {
                port: *p,
                name: e.nodes.get(ni).and_then(|n| n.as_ref()).map(|n| String::from(n.port_name(*p))).unwrap_or_default(),
                state: x.state,
                user: x.user.clone(),
                method: x.method.clone(),
                since: x.since,
            })
            .collect()
    }
}

impl Service for Supplicant {
    fn kind(&self) -> &'static str {
        SVC
    }
    fn start(&mut self, os: &mut OsApi) {
        self.ports.clear();
        self.wheel.clear();
        self.read_conf(os);
        self.sync(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Link { .. } => self.sync(os),
            SvcEvent::Timer(K_WHEEL) => {
                self.wheel.fired();
                for (_, d) in self.wheel.due(os.now()) {
                    self.due(os, d);
                }
            }
            _ => {}
        }
        self.wheel.arm(os);
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.ports.clear();
        self.wheel.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![Metric::new("logins", self.logins as i64), Metric::new("refused", self.refused as i64)]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        msg!("{}: {} Anmeldungen, {} abgelehnt", self.path, self.logins, self.refused)
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
    fn footprint(&self) -> usize {
        self.path.len() + self.conf.iter().map(|b| b.ssid.len() + b.identity.len() + b.password.len() + 64).sum::<usize>() + self.ports.len() * (core::mem::size_of::<SPort>() + 512) + 256
    }
}

impl Engine {
    /// the supplicant's service; made with the file `path` when it is not there and `make`
    fn supp_service(&mut self, ni: usize, path: &str, make: bool) -> Option<u32> {
        if let Some(id) = self.os(ni).and_then(|o| o.svc_id(SVC)) {
            return Some(id);
        }
        if !make {
            return None;
        }
        let dev = self.dev_of(ni);
        self.add_service(dev, SVC, Box::new(Supplicant::new(path)), true).ok()?;
        let id = self.os(ni)?.svc_id(SVC)?;
        if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)) {
            s.console = true;
        }
        Some(id)
    }

    /// a host whose configuration file exists runs a supplicant
    pub(crate) fn supp_boot(&mut self, ni: usize) {
        let linux = self.os(ni).is_some_and(|o| !o.ios);
        if linux && self.store(ni).is_some_and(|s| s.exists(CONF)) {
            let _ = self.supp_service(ni, CONF, true);
            self.wlan_creds_changed(ni);
        }
    }

    /// reads the file (`path`, or the one it runs with) again; the service starts if it does not
    /// run yet
    pub(crate) fn supp_reconfigure(&mut self, ni: usize, path: Option<&str>) {
        let Some(id) = self.supp_service(ni, path.unwrap_or(CONF), true) else { return };
        let running = self.os(ni).is_some_and(|o| o.running(id));
        let path = path.map(String::from);
        self.svc_call(ni, id, None, |svc, os| {
            if let Some(s) = svc.as_any_mut().downcast_mut::<Supplicant>() {
                if let Some(p) = path {
                    s.path = p;
                }
                if running {
                    s.reconfigure(os);
                }
            }
        });
        self.wlan_creds_changed(ni);
    }

    /// an EAPOL frame reached a host's port: true when a supplicant took it
    pub(crate) fn supp_rx(&mut self, ni: usize, port: u16, frame: &[u8], hop: HopId) -> bool {
        let Some(id) = self.os(ni).and_then(|o| o.svc_id(SVC)) else { return false };
        let Some(l2) = L2::parse(frame) else { return false };
        let Some(pdu) = frame.get(l2.off..) else { return false };
        self.svc_call(ni, id, Some(hop), |svc, os| {
            if let Some(s) = svc.as_any_mut().downcast_mut::<Supplicant>() {
                s.rx(os, port, pdu, None);
            }
        });
        true
    }

    /// an EAP frame on a station's radio link
    pub(crate) fn supp_wlan_rx(&mut self, ni: usize, port: u16, assoc: u32, pdu: &[u8], hop: HopId) {
        let Some(id) = self.supp_service(ni, CONF, true) else { return };
        if !self.os(ni).is_some_and(|o| o.running(id)) {
            // made just now for an enterprise network without a file: it runs from here on
            if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)) {
                s.running = true;
            }
            if let Some(o) = self.os_mut(ni) {
                o.inbox.retain(|j| !matches!(j, crate::os::Job::Start(i) if *i == id));
            }
        }
        self.svc_call(ni, id, Some(hop), |svc, os| {
            if let Some(s) = svc.as_any_mut().downcast_mut::<Supplicant>() {
                s.rx(os, port, pdu, Some(assoc));
            }
        });
    }

    /// a station's association went
    pub(crate) fn supp_assoc_gone(&mut self, ni: usize, port: u16) {
        let Some(id) = self.os(ni).and_then(|o| o.svc_id(SVC)) else { return };
        self.svc_call(ni, id, None, |svc, _| {
            if let Some(s) = svc.as_any_mut().downcast_mut::<Supplicant>() {
                s.assoc_gone(port);
            }
        });
    }

    /// wpa_cli's actions on the service; false without one
    pub(crate) fn supp_command(&mut self, ni: usize, cmd: &str) -> bool {
        let Some(id) = self.os(ni).and_then(|o| o.svc_id(SVC)) else { return false };
        let cmd = String::from(cmd);
        self.svc_call(ni, id, None, |svc, os| {
            if let Some(s) = svc.as_any_mut().downcast_mut::<Supplicant>() {
                match cmd.as_str() {
                    "logoff" => s.logoff(os),
                    _ => s.logon(os),
                }
            }
        });
        true
    }

    /// the supplicant's ports
    pub fn supp_views(&self, dev: u32) -> Vec<SuppView> {
        let Some(ni) = self.node_index(dev) else { return Vec::new() };
        match self.service::<Supplicant>(dev, SVC) {
            Some(s) => s.views(self, ni),
            None => Vec::new(),
        }
    }

    /// the network blocks the supplicant runs with, read from its file now
    pub fn supp_conf(&self, dev: u32) -> Vec<NetBlock> {
        let Some(st) = self.node_index(dev).and_then(|ni| self.store(ni)) else { return Vec::new() };
        st.read(CONF).map(|b| parse_conf(&String::from_utf8_lossy(b))).unwrap_or_default()
    }

    /// what a station logs in to `ssid` with: (identity, password, method)
    pub fn wlan_creds(&self, dev: u32, ssid: &str) -> Option<(String, String, Method)> {
        if let Some(j) = self.wlan.stas.get(&dev).and_then(|s| s.join.as_ref()).filter(|j| j.ssid == ssid && !j.identity.is_empty()) {
            let m = Method::parse(&j.eap).unwrap_or(Method::Pwd);
            return Some((j.identity.clone(), j.passphrase.clone(), m));
        }
        let b = self.supp_conf(dev).into_iter().find(|b| b.key_mgmt == KeyMgmt::WpaEap && b.ssid == ssid && !b.identity.is_empty())?;
        Some((b.identity, b.password, b.eap.unwrap_or(Method::Pwd)))
    }
}
