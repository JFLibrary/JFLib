// dot1x -- port access control (IEEE 802.1X-2004): the authenticator of IOS-like switches and of
// access points with WPA-Enterprise, its configuration, the sessions of the ports and stations,
// and the way to RADIUS.  The supplicant is supp.rs, the EAP methods are the crate eapm, the
// server is radiusd.rs
//
//   Dot1xCfg { system, methods, ports }   "dot1x system-auth-control", "aaa authentication dot1x
//                                         default ...", and per port PortCfg (DevCfg::dot1x)
//   PortCfg                               authentication port-control, dot1x pae authenticator,
//                                         dot1x timeout tx-period | quiet-period | reauth-period |
//                                         supp-timeout, dot1x max-req, authentication periodic
//   Dot1xAuth                             the hidden service "dot1x-auth" (its own socket and
//                                         timers; it owns no fixed node timer slot)
//   Engine::dot1x_kick(ni)                the configuration or a port changed: the service looks
//                                         at its ports again (made the first time it is needed)
//   Engine::dot1x_rx(ni, port, frame, hop)   an EAPOL frame of a bridge port (bridge.rs)
//   Engine::dot1x_link(ni, port)          a port's link changed (link.rs)
//   Engine::dot1x_wlan_begin / _rx / _end the same for a WLAN association (wlan.rs)
//   Engine::dot1x_reauth(ni, port)        "dot1x re-authenticate", "clear authentication sessions"
//   Engine::dot1x_sessions(dev)           every session as SesView (show commands, the window)
//   Engine::dot1x_port_closed(dev, port)  the port lets only EAPOL through
//
// A port with "authentication port-control auto" under "dot1x system-auth-control" is closed
// until the server said yes: BridgePort::closed keeps it out of the bridge in both directions,
// its LED shows Led::Unauth, only EAPOL passes.  force-authorized is open as a port without 802.1X
// is (and answers EAPOL-Start with EAP-Success), force-unauthorized stays closed.
//
// The session of a port: Connecting (EAP-Request/Identity to the PAE group address when the link
// comes up, then every tx-period), Authenticating (the supplicant's EAP goes to RADIUS in
// Access-Requests with State and Message-Authenticator, each Access-Challenge comes back as the
// next EAP-Request, repeated every supp-timeout up to max-req times), Authenticated (the port is
// open, with the VLAN of Tunnel-Private-Group-ID in place of its access VLAN while it lasts, a new
// authentication after reauth-period under "authentication periodic", or after Session-Timeout),
// Held (after Access-Reject or a supplicant gone silent: closed for quiet-period, then Connecting).
// RADIUS follows the servers' settings like the logins of radiusd.rs do: retransmissions, the next
// server, the next method of the list.  "local" as a method: the switch checks EAP-MD5 itself
// against "username NAME password ..." (a hashed secret cannot answer a challenge); that is all
// local fallback means here.
//
// On an access point the session belongs to an association (wlan.rs): the same way to RADIUS with
// NAS-Port-Type Wireless-802.11, the servers and secret of the network; the frames go to the
// station on its radio link, a request is repeated after 3 s.  The MSK of MS-MPPE-Recv-Key and
// -Send-Key (RFC 2548) becomes the PMK and the 4-way handshake follows; a refusal, or success
// without keys, ends the association with reason 23.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;

use eapm::{Method as EapMethod, ServerOut};
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::eap::{self, Packet as EapPacket};
use netpkt::eapol::{self, EapolView};
use netpkt::eth::{EthHdr, ET_EAPOL, L2};
use netpkt::radius::{self, Attr, Packet, RadiusView};
use netpkt::{Ipv4, Mac};

use crate::aaa::{Method, RadiusServer};
use crate::aaatrace::ANote;
use crate::dot1xtrace::DNote;
use crate::http::Wheel;
use crate::kernel::{Engine, Frame, Ns, SEC};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::trace::{HopId, Note};

/// the name of the hidden service
pub const SVC: &str = "dot1x-auth";
const K_WHEEL: u32 = 0;
/// how a station's request repeats on a radio link
const WLAN_SUPP_S: u32 = 3;
/// a WLAN network's RADIUS server: seconds to wait, times to send again
pub(crate) const WLAN_RADIUS_TIMEOUT: u32 = 2;
pub(crate) const WLAN_RADIUS_RETRANSMIT: u32 = 2;

/// md5 and hmac-md5 lent to netpkt::radius
fn md5(parts: &[&[u8]]) -> [u8; 16] {
    crypto::md5_parts(parts)
}
fn hmac(key: &[u8], data: &[u8]) -> [u8; 16] {
    crypto::hmac_md5(key, data)
}

/// "authentication port-control"
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PortCtl {
    /// the default: open, as a port without 802.1X
    #[default]
    ForceAuth,
    Auto,
    ForceUnauth,
}

impl PortCtl {
    pub fn word(self) -> &'static str {
        match self {
            PortCtl::ForceAuth => "force-authorized",
            PortCtl::Auto => "auto",
            PortCtl::ForceUnauth => "force-unauthorized",
        }
    }
}

/// the 802.1X settings of a port
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortCfg {
    pub ctl: PortCtl,
    /// "dot1x pae authenticator" (None: not said, it counts as yes; Some(false): "no dot1x pae")
    pub pae: Option<bool>,
    pub tx_period: u32,
    pub quiet_period: u32,
    pub supp_timeout: u32,
    /// "dot1x timeout reauth-period" (0: not set; then Session-Timeout of the server counts)
    pub reauth_period: u32,
    pub max_req: u8,
    /// "authentication periodic"
    pub periodic: bool,
}

/// the timers as a switch has them when nothing is set
pub const TX_PERIOD: u32 = 30;
pub const QUIET_PERIOD: u32 = 60;
pub const SUPP_TIMEOUT: u32 = 30;
pub const MAX_REQ: u8 = 2;

impl Default for PortCfg {
    fn default() -> PortCfg {
        PortCfg { ctl: PortCtl::ForceAuth, pae: None, tx_period: TX_PERIOD, quiet_period: QUIET_PERIOD, supp_timeout: SUPP_TIMEOUT, reauth_period: 0, max_req: MAX_REQ, periodic: false }
    }
}

/// the 802.1X configuration of a device
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dot1xCfg {
    /// "dot1x system-auth-control"
    pub system: bool,
    /// "aaa authentication dot1x default ..."
    pub methods: Vec<Method>,
    /// by port index; a port's defaults are not stored
    pub ports: BTreeMap<u16, PortCfg>,
}

impl Dot1xCfg {
    pub fn is_empty(&self) -> bool {
        !self.system && self.methods.is_empty() && self.ports.is_empty()
    }
    /// the bytes it holds (keyframe::node_bytes)
    pub fn footprint(&self) -> usize {
        self.methods.len() * 32 + self.ports.len() * (core::mem::size_of::<PortCfg>() + 16) + 32
    }
    /// the port takes part: the system is on, the port is an authenticator's
    pub fn controls(&self, port: u16) -> Option<&PortCfg> {
        if !self.system {
            return None;
        }
        self.ports.get(&port).filter(|c| c.pae != Some(false) && c.ctl != PortCtl::ForceAuth)
    }
}

/// what a session belongs to
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SesKey {
    Port(u16),
    /// a WLAN association (its id)
    Sta(u32),
}

/// the state of a session
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaeState {
    Connecting,
    Authenticating,
    Authenticated,
    Held,
    ForceUnauth,
}

impl PaeState {
    pub fn word(self) -> &'static str {
        match self {
            PaeState::Connecting => "CONNECTING",
            PaeState::Authenticating => "AUTHENTICATING",
            PaeState::Authenticated => "AUTHENTICATED",
            PaeState::Held => "HELD",
            PaeState::ForceUnauth => "FORCE-UNAUTH",
        }
    }
}

/// the exchange with RADIUS of a session
#[derive(Clone, Debug)]
struct Rad {
    methods: Vec<Method>,
    mi: usize,
    servers: Vec<RadiusServer>,
    si: usize,
    attempt: u32,
    id: u8,
    auth: [u8; 16],
    state: Option<Vec<u8>>,
    /// the supplicant's EAP this request carries
    eap: Vec<u8>,
    wid: u64,
}

/// a deadline of a session
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Due {
    /// repeat the last request / the end of the quiet period / the next authentication
    Ses(SesKey),
    /// RADIUS did not answer
    Rad(SesKey),
}

/// a session of a port or a station
#[derive(Clone, Debug)]
struct Ses {
    state: PaeState,
    user: String,
    peer: Option<Mac>,
    /// the identifier of the last request
    id: u8,
    /// the last request, to repeat it
    last: Vec<u8>,
    tries: u8,
    timer: Option<u64>,
    /// what the timer is for: the next authentication of an open port
    reauth_due: bool,
    rad: Option<Rad>,
    /// the switch checks the password itself (method "local")
    local: Option<eapm::Server>,
    /// the EAP type the supplicant last answered with
    method: u8,
    vlan: Option<u16>,
    since: Ns,
    /// a new authentication runs on an open port
    reauthing: bool,
    /// Session-Timeout of the server (seconds, 0: none)
    session_timeout: u32,
}

impl Ses {
    fn new(now: Ns) -> Ses {
        Ses { state: PaeState::Connecting, user: String::new(), peer: None, id: 0, last: Vec::new(), tries: 0, timer: None, reauth_due: false, rad: None, local: None, method: 0, vlan: None, since: now, reauthing: false, session_timeout: 0 }
    }
}

/// a session for the show commands and the window
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SesView {
    pub key: SesKey,
    /// the port or the station, as the device names it
    pub name: String,
    pub state: PaeState,
    pub user: String,
    pub mac: Option<Mac>,
    /// the EAP method by name ("" before one ran)
    pub method: String,
    pub vlan: Option<u16>,
    pub since: Ns,
    /// the port lets traffic through
    pub open: bool,
}

/// the hidden service of an authenticator
#[derive(Clone, Debug)]
pub struct Dot1xAuth {
    sock: Option<u16>,
    ses: BTreeMap<SesKey, Ses>,
    wheel: Wheel<Due>,
    pub starts: u64,
    pub successes: u64,
    pub refusals: u64,
    pub requests: u64,
    pub timeouts: u64,
    pub dropped: u64,
}

impl Dot1xAuth {
    pub fn new() -> Dot1xAuth {
        Dot1xAuth { sock: None, ses: BTreeMap::new(), wheel: Wheel::new(K_WHEEL), starts: 0, successes: 0, refusals: 0, requests: 0, timeouts: 0, dropped: 0 }
    }

    fn ensure_sock(&mut self, os: &mut OsApi) {
        if self.sock.is_none() {
            self.sock = os.bind(0, None).ok();
        }
    }

    /// the name of a session for notes and lines
    fn name_of(os: &OsApi, key: SesKey) -> String {
        match key {
            SesKey::Port(p) => os.eng.nodes.get(os.ni).and_then(|n| n.as_ref()).map(|n| jfn_model::ifname::short(n.port_name(p))).unwrap_or_default(),
            SesKey::Sta(a) => os.eng.wlan.assocs.get(&a).and_then(|x| os.eng.node(x.sta)).map(|n| n.name.clone()).unwrap_or_default(),
        }
    }

    /// the settings of a session: (tx period, quiet period, supp timeout, max requests)
    fn timers(os: &OsApi, key: SesKey) -> (u32, u32, u32, u8) {
        match key {
            SesKey::Port(p) => {
                let d = PortCfg::default();
                let c = os.eng.cdev(os.ni).and_then(|c| c.cfg.dot1x.ports.get(&p)).unwrap_or(&d);
                (c.tx_period, c.quiet_period, c.supp_timeout, c.max_req)
            }
            SesKey::Sta(_) => (WLAN_SUPP_S, QUIET_PERIOD, WLAN_SUPP_S, MAX_REQ),
        }
    }

    /// sends an EAPOL PDU to a session's supplicant
    fn send_pdu(&mut self, os: &mut OsApi, key: SesKey, pdu: &[u8]) {
        let cause = os.cause;
        match key {
            SesKey::Port(p) => {
                let Some(mac) = os.eng.nodes.get(os.ni).and_then(|n| n.as_ref()).and_then(|n| n.ports.get(p as usize)).map(|x| x.mac) else { return };
                let f = eth_frame(Mac(eapol::PAE_GROUP), mac, pdu);
                let ni = os.ni;
                os.eng.send(ni, p, f, cause);
            }
            SesKey::Sta(a) => {
                let Some(x) = os.eng.wlan.assocs.get(&a) else { return };
                let (li, sta, bssid) = (x.li, x.sta_mac, x.bssid);
                os.eng.send_on(li, 1, eth_frame(sta, bssid, pdu), cause);
            }
        }
    }

    /// sends an EAP packet with its note
    fn send_eap(&mut self, os: &mut OsApi, key: SesKey, b: &[u8]) {
        let port = Self::name_of(os, key);
        os.note(Note::Dot1x(DNote::TxEap { port, what: eap::info(b) }));
        self.send_pdu(os, key, &eapol::eap_pdu(eapol::VERSION_2004, b));
    }

    fn cancel_timer(&mut self, key: SesKey) {
        if let Some(t) = self.ses.get_mut(&key).and_then(|s| s.timer.take()) {
            self.wheel.cancel(t);
        }
    }

    fn set_timer(&mut self, os: &mut OsApi, key: SesKey, secs: u32) {
        self.cancel_timer(key);
        let at = os.now() + secs.max(1) as Ns * SEC;
        let id = self.wheel.add(at, Due::Ses(key));
        if let Some(s) = self.ses.get_mut(&key) {
            s.timer = Some(id);
        }
    }

    /// a new EAP-Request/Identity
    fn ask_identity(&mut self, os: &mut OsApi, key: SesKey) {
        let (tx, ..) = Self::timers(os, key);
        let Some(s) = self.ses.get_mut(&key) else { return };
        s.id = s.id.wrapping_add(1);
        s.tries = 0;
        s.reauth_due = false;
        let b = EapPacket::identity_request(s.id, "").to_bytes();
        s.last = b.clone();
        self.send_eap(os, key, &b);
        self.set_timer(os, key, tx);
    }

    /// drops what a session waits for from RADIUS
    fn forget_rad(&mut self, key: SesKey) {
        if let Some(r) = self.ses.get_mut(&key).and_then(|s| s.rad.take()) {
            self.wheel.cancel(r.wid);
        }
        if let Some(s) = self.ses.get_mut(&key) {
            s.local = None;
        }
    }

    /// looks at the ports: makes and ends the sessions the configuration and links ask for, opens
    /// and closes the ports (every bridge member; a station's session lives with its association)
    pub fn sync(&mut self, os: &mut OsApi) {
        self.ensure_sock(os);
        let ni = os.ni;
        let Some(Some(n)) = os.eng.nodes.get(ni) else { return };
        let Some(br) = n.bridge.as_ref() else { return };
        let ports: Vec<u16> = (0..br.ports.len() as u16).filter(|&p| br.ports[p as usize].member).collect();
        for p in ports {
            let cfg = os.eng.cdev(ni).and_then(|c| c.cfg.dot1x.controls(p).cloned());
            let up = os.eng.nodes.get(ni).and_then(|n| n.as_ref()).and_then(|n| n.ports[p as usize].link).is_some_and(|li| os.eng.link_up(li));
            let key = SesKey::Port(p);
            match cfg {
                None => {
                    if self.ses.contains_key(&key) {
                        self.end(os, key);
                    }
                    os.eng.dot1x_set_closed(ni, p, false);
                }
                Some(c) if c.ctl == PortCtl::ForceUnauth => {
                    os.eng.dot1x_set_closed(ni, p, true);
                    self.forget_rad(key);
                    self.cancel_timer(key);
                    let now = os.now();
                    let s = self.ses.entry(key).or_insert_with(|| Ses::new(now));
                    s.state = PaeState::ForceUnauth;
                }
                Some(_) => {
                    if self.ses.get(&key).is_some_and(|s| s.state == PaeState::ForceUnauth) {
                        self.ses.remove(&key);
                    }
                    let had = self.ses.contains_key(&key);
                    if !up {
                        // the port is down: closed, and it starts over when the link comes
                        if had {
                            self.end(os, key);
                            let port = Self::name_of(os, key);
                            os.note(Note::Dot1x(DNote::Closed { port }));
                        }
                        os.eng.dot1x_set_closed(ni, p, true);
                    } else if !had {
                        os.eng.dot1x_set_closed(ni, p, true);
                        let now = os.now();
                        self.ses.insert(key, Ses::new(now));
                        self.ask_identity(os, key);
                    }
                }
            }
        }
        self.wheel.arm(os);
    }

    /// a session ends (its port's VLAN comes back)
    fn end(&mut self, os: &mut OsApi, key: SesKey) {
        self.forget_rad(key);
        self.cancel_timer(key);
        if let (SesKey::Port(p), Some(s)) = (key, self.ses.get(&key)) {
            if s.vlan.is_some() {
                let ni = os.ni;
                os.eng.dot1x_set_vlan(ni, p, None);
            }
        }
        self.ses.remove(&key);
    }

    /// a WLAN association starts its 802.1X
    pub fn begin_sta(&mut self, os: &mut OsApi, assoc: u32) {
        self.ensure_sock(os);
        let key = SesKey::Sta(assoc);
        let now = os.now();
        self.ses.insert(key, Ses::new(now));
        self.ask_identity(os, key);
        self.wheel.arm(os);
    }

    /// a WLAN association went
    pub fn end_sta(&mut self, os: &mut OsApi, assoc: u32) {
        self.end(os, SesKey::Sta(assoc));
        self.wheel.arm(os);
    }

    /// an EAPOL PDU from a supplicant
    pub fn rx(&mut self, os: &mut OsApi, key: SesKey, src: Mac, pdu: &[u8]) {
        let Some(v) = EapolView::parse(pdu) else { return };
        let port = Self::name_of(os, key);
        match v.kind() {
            eapol::T_START => {
                self.starts += 1;
                os.note(Note::Dot1x(DNote::Start { port }));
                self.on_start(os, key);
            }
            eapol::T_LOGOFF => {
                os.note(Note::Dot1x(DNote::Logoff { port }));
                self.on_logoff(os, key);
            }
            eapol::T_EAP => self.on_eap(os, key, src, v.body()),
            _ => {}
        }
        self.wheel.arm(os);
    }

    fn on_start(&mut self, os: &mut OsApi, key: SesKey) {
        let state = self.ses.get(&key).map(|s| s.state);
        match state {
            None => {
                // force-authorized (or no 802.1X on the port): the answer is yes at once
                if let SesKey::Port(p) = key
                    && os.eng.cdev(os.ni).is_some_and(|c| c.cfg.dot1x.system && c.cfg.dot1x.ports.get(&p).is_some_and(|x| x.ctl == PortCtl::ForceAuth))
                {
                    self.send_eap(os, key, &EapPacket::success(1).to_bytes());
                }
            }
            Some(PaeState::ForceUnauth) => self.send_eap(os, key, &EapPacket::failure(1).to_bytes()),
            Some(PaeState::Held) => {}
            Some(PaeState::Authenticated) => {
                if let Some(s) = self.ses.get_mut(&key) {
                    s.reauthing = true;
                    s.state = PaeState::Authenticating;
                }
                self.ask_identity(os, key);
            }
            Some(_) => {
                self.forget_rad(key);
                if let Some(s) = self.ses.get_mut(&key) {
                    s.state = PaeState::Connecting;
                }
                self.ask_identity(os, key);
            }
        }
    }

    fn on_logoff(&mut self, os: &mut OsApi, key: SesKey) {
        if !self.ses.contains_key(&key) {
            return;
        }
        self.forget_rad(key);
        let was_open = self.ses.get(&key).is_some_and(|s| s.state == PaeState::Authenticated || s.reauthing);
        match key {
            SesKey::Port(p) => {
                let ni = os.ni;
                if was_open {
                    let port = Self::name_of(os, key);
                    os.note(Note::Dot1x(DNote::Closed { port }));
                    os.eng.dot1x_set_closed(ni, p, true);
                    os.eng.dot1x_set_vlan(ni, p, None);
                    let line = format!("%DOT1X-5-LOGOFF: {} logged off, the port is closed", Self::name_of(os, key));
                    os.eng.ios_log(ni, &line);
                }
                let (tx, ..) = Self::timers(os, key);
                if let Some(s) = self.ses.get_mut(&key) {
                    s.state = PaeState::Connecting;
                    s.user.clear();
                    s.vlan = None;
                    s.reauthing = false;
                    s.reauth_due = false;
                    s.tries = 0;
                    s.method = 0;
                    s.since = os.eng.now;
                }
                self.set_timer(os, key, tx);
            }
            SesKey::Sta(a) => {
                self.end(os, key);
                os.eng.wlan_eap_done(a, Err(23));
            }
        }
    }

    fn on_eap(&mut self, os: &mut OsApi, key: SesKey, src: Mac, b: &[u8]) {
        let Some(p) = EapPacket::parse(b) else { return };
        if p.code != eap::RESPONSE {
            return;
        }
        let port = Self::name_of(os, key);
        os.note(Note::Dot1x(DNote::RxEap { port: port.clone(), what: eap::info(b) }));
        let Some(s) = self.ses.get_mut(&key) else { return };
        if s.id != p.id || matches!(s.state, PaeState::Held | PaeState::ForceUnauth) {
            return;
        }
        s.peer = Some(src);
        if p.typ != eap::T_IDENTITY {
            s.method = p.typ;
        }
        self.cancel_timer(key);
        if p.is(eap::RESPONSE, eap::T_IDENTITY) {
            // a new authentication: its first Access-Request
            self.forget_rad(key);
            let Some(s) = self.ses.get_mut(&key) else { return };
            s.user = p.identity();
            if s.state == PaeState::Authenticated {
                s.reauthing = true;
            }
            s.state = PaeState::Authenticating;
            let methods = Self::methods_of(os, key);
            let servers = methods.first().map(|m| Self::servers_of(os, key, m)).unwrap_or_default();
            if let Some(s) = self.ses.get_mut(&key) {
                s.rad = Some(Rad { methods, mi: 0, servers, si: 0, attempt: 1, id: 0, auth: [0; 16], state: None, eap: b.to_vec(), wid: 0 });
            }
            self.rad_method(os, key);
            return;
        }
        if self.ses.get(&key).is_some_and(|s| s.local.is_some()) {
            return self.local_step(os, key, b);
        }
        if let Some(r) = self.ses.get_mut(&key).and_then(|s| s.rad.as_mut()) {
            r.eap = b.to_vec();
            r.attempt = 1;
            r.si = r.si.min(r.servers.len().saturating_sub(1));
        }
        self.rad_new_request(os, key);
    }

    /// the method list of a session: the device's for a port, the network's server for a station
    fn methods_of(os: &OsApi, key: SesKey) -> Vec<Method> {
        match key {
            SesKey::Port(_) => os.eng.cdev(os.ni).map(|c| c.cfg.dot1x.methods.clone()).unwrap_or_default(),
            SesKey::Sta(_) => alloc::vec![Method::GroupRadius],
        }
    }

    fn servers_of(os: &OsApi, key: SesKey, m: &Method) -> Vec<RadiusServer> {
        match key {
            SesKey::Port(_) => os.eng.aaa_servers_of(os.ni, m),
            SesKey::Sta(a) => os.eng.wlan_radius_of(a).into_iter().collect(),
        }
    }

    /// the method at `mi`: RADIUS asks its servers, "local" decides here; none left: refused
    fn rad_method(&mut self, os: &mut OsApi, key: SesKey) {
        let m = self.ses.get(&key).and_then(|s| s.rad.as_ref()).and_then(|r| r.methods.get(r.mi).cloned());
        match m {
            Some(Method::Local) => {
                let eap_bytes = self.ses.get(&key).and_then(|s| s.rad.as_ref()).map(|r| r.eap.clone()).unwrap_or_default();
                if let Some(s) = self.ses.get_mut(&key) {
                    s.local = Some(eapm::Server::new("local", &[EapMethod::Md5]));
                    s.rad = None;
                }
                self.local_step(os, key, &eap_bytes);
            }
            Some(Method::GroupRadius | Method::Group(_)) => {
                if self.ses.get(&key).and_then(|s| s.rad.as_ref()).is_some_and(|r| r.servers.is_empty()) {
                    return self.rad_next_method(os, key);
                }
                self.rad_new_request(os, key);
            }
            Some(_) => self.rad_next_method(os, key),
            None => {
                let port = Self::name_of(os, key);
                os.note(Note::Dot1x(DNote::NoServer { port }));
                self.refused(os, key);
            }
        }
    }

    fn rad_next_method(&mut self, os: &mut OsApi, key: SesKey) {
        let m = {
            let Some(r) = self.ses.get_mut(&key).and_then(|s| s.rad.as_mut()) else { return };
            r.mi += 1;
            r.si = 0;
            r.attempt = 1;
            r.state = None;
            r.methods.get(r.mi).cloned()
        };
        let servers = m.as_ref().map(|m| Self::servers_of(os, key, m)).unwrap_or_default();
        if let Some(r) = self.ses.get_mut(&key).and_then(|s| s.rad.as_mut()) {
            r.servers = servers;
        }
        self.rad_method(os, key);
    }

    /// a new Access-Request for what the supplicant sent: a fresh id and authenticator
    fn rad_new_request(&mut self, os: &mut OsApi, key: SesKey) {
        let mut auth = [0u8; 16];
        for c in auth.chunks_mut(4) {
            c.copy_from_slice(&os.rand().to_be_bytes());
        }
        let id = (os.rand() & 0xFF) as u8;
        if let Some(r) = self.ses.get_mut(&key).and_then(|s| s.rad.as_mut()) {
            r.id = id;
            r.auth = auth;
        }
        self.rad_send(os, key);
    }

    /// sends the Access-Request of a session to its current server
    fn rad_send(&mut self, os: &mut OsApi, key: SesKey) {
        let Some(sock) = self.sock else { return };
        let Some(s) = self.ses.get(&key) else { return };
        let Some(r) = s.rad.as_ref() else { return };
        let Some(server) = r.servers.get(r.si).cloned() else {
            return self.rad_next_method(os, key);
        };
        let src = os.source_for(server.addr).unwrap_or(Ipv4::UNSPECIFIED);
        let mut p = Packet::new(radius::ACCESS_REQUEST, r.id, r.auth);
        p.push(Attr::text(radius::A_USER_NAME, &s.user));
        if !src.is_unspecified() {
            p.push(Attr::addr(radius::A_NAS_IP_ADDRESS, src));
        }
        let peer = s.peer;
        let (nas_port, port_type, called) = match key {
            SesKey::Port(pi) => {
                let mac = os.eng.nodes.get(os.ni).and_then(|n| n.as_ref()).and_then(|n| n.ports.get(pi as usize)).map(|x| x.mac).unwrap_or(Mac::ZERO);
                (pi as u32 + 1, radius::PORT_TYPE_ETHERNET, station_id(mac))
            }
            SesKey::Sta(a) => {
                let x = os.eng.wlan.assocs.get(&a);
                (x.map_or(0, |x| x.aid as u32), radius::PORT_TYPE_WIRELESS, x.map_or_else(String::new, |x| format!("{}:{}", station_id(x.bssid), x.ssid)))
            }
        };
        p.push(Attr::int(radius::A_NAS_PORT, nas_port));
        p.push(Attr::int(radius::A_SERVICE_TYPE, radius::SERVICE_FRAMED));
        p.push(Attr::int(radius::A_FRAMED_MTU, 1500));
        p.push(Attr::int(radius::A_NAS_PORT_TYPE, port_type));
        p.push(Attr::text(radius::A_CALLED_STATION_ID, &called));
        if let Some(m) = peer {
            p.push(Attr::text(radius::A_CALLING_STATION_ID, &station_id(m)));
        }
        if let Some(st) = &r.state {
            p.push(Attr::bytes(radius::A_STATE, st));
        }
        p.push_eap(&r.eap);
        p.push(Attr::message_auth());
        let mut b = p.to_bytes();
        radius::set_message_auth(&mut b, None, server.key.as_bytes(), hmac);
        let (user, attempt) = (s.user.clone(), r.attempt);
        self.requests += 1;
        let sent = os.send_from(src, sock, server.addr, server.auth_port, &b);
        if sent {
            os.note(Note::Aaa(ANote::RadiusRequest { to: server.addr, user, attempt }));
        } else {
            os.note(Note::Aaa(ANote::RadiusTimeout { to: server.addr }));
            self.timeouts += 1;
            return self.rad_next_server(os, key);
        }
        let at = os.now() + server.timeout.max(1) as Ns * SEC;
        let wid = self.wheel.add(at, Due::Rad(key));
        if let Some(r) = self.ses.get_mut(&key).and_then(|s| s.rad.as_mut()) {
            r.wid = wid;
        }
    }

    fn rad_next_server(&mut self, os: &mut OsApi, key: SesKey) {
        let more = {
            let Some(r) = self.ses.get_mut(&key).and_then(|s| s.rad.as_mut()) else { return };
            r.si += 1;
            r.attempt = 1;
            r.state = None;
            r.si < r.servers.len()
        };
        if more {
            self.rad_send(os, key);
        } else {
            self.rad_next_method(os, key);
        }
    }

    fn rad_timeout(&mut self, os: &mut OsApi, key: SesKey) {
        let Some(r) = self.ses.get(&key).and_then(|s| s.rad.as_ref()) else { return };
        let Some(server) = r.servers.get(r.si) else { return };
        let (to, attempt, retransmit) = (server.addr, r.attempt, server.retransmit);
        if attempt <= retransmit {
            if let Some(r) = self.ses.get_mut(&key).and_then(|s| s.rad.as_mut()) {
                r.attempt += 1;
            }
            os.note(Note::Aaa(ANote::RadiusRetransmit { to, attempt: attempt + 1 }));
            self.rad_send(os, key);
        } else {
            self.timeouts += 1;
            os.note(Note::Aaa(ANote::RadiusTimeout { to }));
            self.rad_next_server(os, key);
        }
    }

    /// an answer of a RADIUS server
    fn rad_answer(&mut self, os: &mut OsApi, d: crate::os::Dgram) {
        let from = d.src4();
        let Ok(v) = RadiusView::parse(&d.data) else { return };
        let key = self.ses.iter().find(|(_, s)| s.rad.as_ref().is_some_and(|r| r.id == v.id() && r.servers.get(r.si).is_some_and(|x| x.addr == from))).map(|(k, _)| *k);
        let Some(key) = key else { return };
        let (server_key, req_auth, user) = {
            let s = &self.ses[&key];
            let r = s.rad.as_ref().unwrap_or_else(|| unreachable!());
            (r.servers[r.si].key.clone(), r.auth, s.user.clone())
        };
        if !radius::check_response_auth(&d.data, &req_auth, server_key.as_bytes(), md5) {
            self.dropped += 1;
            os.note(Note::Aaa(ANote::RadiusDropped { from, why: String::from("falscher Response-Authenticator") }));
            return;
        }
        let ma = radius::check_message_auth(&d.data, Some(&req_auth), server_key.as_bytes(), hmac);
        let p = Packet::read(&v);
        // RFC 3579 3.2: with EAP-Message the Message-Authenticator must be there and right
        if ma == Some(false) || (ma.is_none() && p.attr(radius::A_EAP_MESSAGE).is_some()) {
            self.dropped += 1;
            os.note(Note::Aaa(ANote::RadiusDropped { from, why: String::from("falscher Message-Authenticator") }));
            return;
        }
        if let Some(r) = self.ses.get(&key).and_then(|s| s.rad.as_ref()) {
            self.wheel.cancel(r.wid);
        }
        match p.code {
            radius::ACCESS_CHALLENGE => {
                os.note(Note::Aaa(ANote::RadiusChallenge { from, user }));
                let state = p.attr(radius::A_STATE).map(|a| a.value.clone());
                if let Some(r) = self.ses.get_mut(&key).and_then(|s| s.rad.as_mut()) {
                    r.state = state;
                }
                match p.eap() {
                    Some(e) => self.relay_request(os, key, &e),
                    None => self.refused(os, key),
                }
            }
            radius::ACCESS_ACCEPT => {
                os.note(Note::Aaa(ANote::RadiusAccept { from, user, level: None }));
                let msk = p.msk(server_key.as_bytes(), &req_auth, md5);
                let vlan = p.vlan();
                let timeout = p.int(radius::A_SESSION_TIMEOUT).unwrap_or(0);
                if let Some(e) = p.eap() {
                    self.send_eap(os, key, &e);
                }
                self.authorized(os, key, vlan, timeout, msk);
            }
            radius::ACCESS_REJECT => {
                os.note(Note::Aaa(ANote::RadiusReject { from, user }));
                if let Some(e) = p.eap() {
                    self.send_eap(os, key, &e);
                }
                self.refused(os, key);
            }
            _ => {}
        }
    }

    /// a Request goes to the supplicant, with its identifier noted and its repetitions armed
    fn relay_request(&mut self, os: &mut OsApi, key: SesKey, b: &[u8]) {
        let Some(p) = EapPacket::parse(b) else { return };
        let (_, _, supp, _) = Self::timers(os, key);
        if let Some(s) = self.ses.get_mut(&key) {
            s.id = p.id;
            s.last = b.to_vec();
            s.tries = 0;
        }
        self.send_eap(os, key, b);
        self.set_timer(os, key, supp);
    }

    /// the switch checks a method itself ("local")
    fn local_step(&mut self, os: &mut OsApi, key: SesKey, b: &[u8]) {
        let ni = os.ni;
        let users: Vec<(String, String)> = os.eng.cdev(ni).map(|c| c.cfg.auth.users.iter().filter_map(|u| u.secret.reveal().map(|p| (u.name.clone(), p))).collect()).unwrap_or_default();
        let lookup = |name: &str| users.iter().find(|(n, _)| n == name).map(|(_, p)| p.clone());
        let Some(mut srv) = self.ses.get_mut(&key).and_then(|s| s.local.take()) else { return };
        let out = {
            let mut rnd = |o: &mut [u8]| {
                for c in o.chunks_mut(4) {
                    let r = os.rand().to_be_bytes();
                    let n = c.len();
                    c.copy_from_slice(&r[..n]);
                }
            };
            srv.on_packet(b, &lookup, &mut rnd)
        };
        if let Some(s) = self.ses.get_mut(&key) {
            s.local = Some(srv);
        }
        match out {
            ServerOut::Send(x) => self.relay_request(os, key, &x),
            ServerOut::Success { eap, .. } => {
                self.send_eap(os, key, &eap);
                self.authorized(os, key, None, 0, None);
            }
            ServerOut::Failure { eap } => {
                self.send_eap(os, key, &eap);
                self.refused(os, key);
            }
            ServerOut::Silent => {}
        }
    }

    /// the server said yes
    fn authorized(&mut self, os: &mut OsApi, key: SesKey, vlan: Option<u16>, timeout: u32, msk: Option<[u8; 64]>) {
        self.forget_rad(key);
        self.cancel_timer(key);
        self.successes += 1;
        let port = Self::name_of(os, key);
        let ni = os.ni;
        let user = self.ses.get(&key).map(|s| s.user.clone()).unwrap_or_default();
        match key {
            SesKey::Port(p) => {
                if let Some(v) = vlan
                    && !os.eng.bridge_has_vlan(ni, v)
                {
                    os.note(Note::Dot1x(DNote::NoVlan { port, vlan: v }));
                    return self.refused(os, key);
                }
                os.eng.dot1x_set_vlan(ni, p, vlan);
                os.eng.dot1x_set_closed(ni, p, false);
                let now = os.now();
                let (periodic, reauth) = os.eng.cdev(ni).and_then(|c| c.cfg.dot1x.ports.get(&p)).map(|c| (c.periodic, c.reauth_period)).unwrap_or((false, 0));
                if let Some(s) = self.ses.get_mut(&key) {
                    s.state = PaeState::Authenticated;
                    s.reauthing = false;
                    s.vlan = vlan;
                    s.session_timeout = timeout;
                    s.since = now;
                    s.tries = 0;
                }
                os.note(Note::Dot1x(DNote::Open { port: port.clone(), user: user.clone(), vlan }));
                os.eng.ios_log(ni, &format!("%DOT1X-5-SUCCESS: {}: {} is authorized", port, user));
                let period = if reauth > 0 { reauth } else { timeout };
                if periodic && period > 0 {
                    self.set_timer(os, key, period);
                    if let Some(s) = self.ses.get_mut(&key) {
                        s.reauth_due = true;
                    }
                }
            }
            SesKey::Sta(a) => {
                let now = os.now();
                if msk.is_none() {
                    os.note(Note::Dot1x(DNote::NoKeys { sta: port }));
                }
                if let Some(s) = self.ses.get_mut(&key) {
                    s.state = PaeState::Authenticated;
                    s.since = now;
                }
                if msk.is_none() {
                    self.end(os, key);
                }
                os.eng.wlan_eap_done(a, msk.ok_or(23u16));
            }
        }
        self.wheel.arm(os);
    }

    /// the server said no (or nobody answered): the port stays closed for the quiet period
    fn refused(&mut self, os: &mut OsApi, key: SesKey) {
        self.forget_rad(key);
        self.cancel_timer(key);
        self.refusals += 1;
        let port = Self::name_of(os, key);
        let ni = os.ni;
        let user = self.ses.get(&key).map(|s| s.user.clone()).unwrap_or_default();
        let (_, quiet, ..) = Self::timers(os, key);
        match key {
            SesKey::Port(p) => {
                os.eng.dot1x_set_closed(ni, p, true);
                os.eng.dot1x_set_vlan(ni, p, None);
                if let Some(s) = self.ses.get_mut(&key) {
                    s.state = PaeState::Held;
                    s.reauthing = false;
                    s.vlan = None;
                    s.tries = 0;
                }
                os.note(Note::Dot1x(DNote::Refused { port: port.clone(), user: user.clone(), quiet }));
                os.eng.ios_log(ni, &format!("%DOT1X-5-FAIL: {}: {} is not authorized", port, user));
                self.set_timer(os, key, quiet);
            }
            SesKey::Sta(a) => {
                os.note(Note::Dot1x(DNote::Refused { port, user, quiet: 0 }));
                self.end(os, key);
                os.eng.wlan_eap_done(a, Err(23));
            }
        }
        self.wheel.arm(os);
    }

    /// the timer of a session
    fn ses_due(&mut self, os: &mut OsApi, key: SesKey) {
        let Some(s) = self.ses.get_mut(&key) else { return };
        s.timer = None;
        let (state, last, tries, reauth_due) = (s.state, s.last.clone(), s.tries, s.reauth_due);
        let port = Self::name_of(os, key);
        let (tx, _, supp, max) = Self::timers(os, key);
        match state {
            PaeState::Connecting => self.ask_identity(os, key),
            PaeState::Held => {
                os.note(Note::Dot1x(DNote::Again { port }));
                if let Some(s) = self.ses.get_mut(&key) {
                    s.state = PaeState::Connecting;
                    s.user.clear();
                    s.method = 0;
                }
                self.ask_identity(os, key);
            }
            PaeState::Authenticated if reauth_due => {
                os.note(Note::Dot1x(DNote::Reauth { port }));
                if let Some(s) = self.ses.get_mut(&key) {
                    s.reauthing = true;
                    s.state = PaeState::Authenticating;
                }
                self.ask_identity(os, key);
            }
            PaeState::Authenticating => {
                if tries >= max {
                    os.note(Note::Dot1x(DNote::NoAnswer { port }));
                    return self.refused(os, key);
                }
                if let Some(s) = self.ses.get_mut(&key) {
                    s.tries = tries + 1;
                }
                os.note(Note::Dot1x(DNote::Retry { port, n: tries + 2 }));
                self.send_eap(os, key, &last);
                self.set_timer(os, key, if last.get(4) == Some(&eap::T_IDENTITY) { tx } else { supp });
            }
            _ => {}
        }
    }

    /// starts an authentication again ("dot1x re-authenticate")
    pub fn reauth(&mut self, os: &mut OsApi, key: SesKey) {
        let Some(s) = self.ses.get(&key) else { return };
        let port = Self::name_of(os, key);
        match s.state {
            PaeState::Authenticated => {
                os.note(Note::Dot1x(DNote::Reauth { port }));
                if let Some(s) = self.ses.get_mut(&key) {
                    s.reauthing = true;
                    s.state = PaeState::Authenticating;
                }
                self.ask_identity(os, key);
            }
            PaeState::Connecting | PaeState::Authenticating | PaeState::Held => {
                self.forget_rad(key);
                if let Some(s) = self.ses.get_mut(&key) {
                    s.state = PaeState::Connecting;
                }
                self.ask_identity(os, key);
            }
            PaeState::ForceUnauth => {}
        }
        self.wheel.arm(os);
    }

    /// the sessions for the show commands
    pub fn views(&self, e: &Engine, ni: usize) -> Vec<SesView> {
        self.ses
            .iter()
            .map(|(k, s)| {
                let name = match k {
                    SesKey::Port(p) => e.nodes.get(ni).and_then(|n| n.as_ref()).map(|n| jfn_model::ifname::short(n.port_name(*p))).unwrap_or_default(),
                    SesKey::Sta(a) => e.wlan.assocs.get(a).and_then(|x| e.node(x.sta)).map(|n| n.name.clone()).unwrap_or_default(),
                };
                let method = if s.method == 0 { String::new() } else { EapMethod::of_type(s.method).map(|m| String::from(m.word())).unwrap_or_else(|| eap::type_name(s.method)) };
                SesView { key: *k, name, state: s.state, user: s.user.clone(), mac: s.peer, method, vlan: s.vlan, since: s.since, open: s.state == PaeState::Authenticated || s.reauthing }
            })
            .collect()
    }
}

impl Default for Dot1xAuth {
    fn default() -> Dot1xAuth {
        Dot1xAuth::new()
    }
}

impl Service for Dot1xAuth {
    fn kind(&self) -> &'static str {
        SVC
    }
    fn start(&mut self, os: &mut OsApi) {
        self.ses.clear();
        self.wheel.clear();
        self.sock = None;
        self.sync(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Udp(d) => self.rad_answer(os, d),
            SvcEvent::Timer(K_WHEEL) => {
                self.wheel.fired();
                for (_, d) in self.wheel.due(os.now()) {
                    match d {
                        Due::Ses(k) => self.ses_due(os, k),
                        Due::Rad(k) => self.rad_timeout(os, k),
                    }
                }
            }
            _ => {}
        }
        self.wheel.arm(os);
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.ses.clear();
        self.wheel.clear();
        self.sock = None;
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("starts", self.starts as i64),
            Metric::new("successes", self.successes as i64),
            Metric::new("refusals", self.refusals as i64),
            Metric::new("requests", self.requests as i64),
            Metric::new("timeouts", self.timeouts as i64),
            Metric::new("dropped", self.dropped as i64),
        ]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        msg!("{} Sitzungen, {} angenommen, {} abgelehnt", self.ses.len(), self.successes, self.refusals)
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
        self.ses.values().map(|s| s.user.len() + s.last.len() + s.rad.as_ref().map_or(0, |r| r.servers.len() * 48 + r.methods.len() * 32 + r.eap.len() + 64) + core::mem::size_of::<Ses>()).sum::<usize>() + 256
    }
}

/// a MAC as Called-Station-Id and Calling-Station-Id write it (RFC 3580 3.20): "02-4A-00-00-01-01"
pub fn station_id(m: Mac) -> String {
    let b = m.0;
    format!("{:02X}-{:02X}-{:02X}-{:02X}-{:02X}-{:02X}", b[0], b[1], b[2], b[3], b[4], b[5])
}

/// an EAPOL PDU as a frame (padded to 60 bytes)
pub(crate) fn eth_frame(dst: Mac, src: Mac, pdu: &[u8]) -> Frame {
    let mut f = EthHdr { dst, src, ethertype: ET_EAPOL }.frame(pdu);
    netpkt::eth::pad(&mut f);
    Arc::from(f)
}

impl Engine {
    /// the service of an authenticator (made the first time, hidden from the inspector)
    fn dot1x_service(&mut self, ni: usize) -> Option<u32> {
        if let Some(id) = self.os(ni).and_then(|o| o.svc_id(SVC)) {
            return Some(id);
        }
        let dev = self.dev_of(ni);
        self.add_service(dev, SVC, Box::new(Dot1xAuth::new()), true).ok()?;
        let id = self.os(ni)?.svc_id(SVC)?;
        if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)) {
            s.console = true;
        }
        Some(id)
    }

    /// the configuration or a port changed: the service looks again; a device without 802.1X
    /// settings and without a service stays as it is
    pub(crate) fn dot1x_kick(&mut self, ni: usize) {
        let wanted = self.cdev(ni).is_some_and(|c| c.cfg.dot1x.system && c.cfg.dot1x.ports.values().any(|p| p.ctl != PortCtl::ForceAuth));
        let id = match self.os(ni).and_then(|o| o.svc_id(SVC)) {
            Some(id) => Some(id),
            None if wanted => self.dot1x_service(ni),
            None => None,
        };
        let Some(id) = id else { return };
        if self.os(ni).is_some_and(|o| o.running(id)) {
            self.svc_call(ni, id, None, |svc, os| {
                if let Some(a) = svc.as_any_mut().downcast_mut::<Dot1xAuth>() {
                    a.sync(os);
                }
            });
        }
    }

    /// the problems of the ports: auto without a server behind it, auto with 802.1X switched off
    pub(crate) fn dot1x_problems(&self, out: &mut Vec<crate::problems::Problem>) {
        use crate::problems::{Problem, ProblemKind, Severity};
        for slot in self.nodes.iter().flatten() {
            let cfg = &slot.cli.cfg.dot1x;
            let auto: Vec<u16> = cfg.ports.iter().filter(|(_, c)| c.ctl == PortCtl::Auto && c.pae != Some(false)).map(|(p, _)| *p).collect();
            for p in auto {
                let port = jfn_model::ifname::short(slot.port_name(p));
                let local = cfg.methods.contains(&Method::Local);
                let servers = cfg.methods.iter().any(|m| !slot.cli.cfg.aaa.servers_of(m).is_empty());
                let (kind, severity, m) = if !cfg.system {
                    (ProblemKind::Dot1xOff, Severity::Warning, msg!("{} {}: 802.1X verlangt, aber \u{201e}dot1x system-auth-control\u{201c} fehlt: der Port bleibt offen.", slot.name, port))
                } else if !local && !servers {
                    (ProblemKind::Dot1xNoServer, Severity::Error, msg!("{} {}: 802.1X verlangt, aber kein RADIUS-Server eingetragen: der Port bleibt geschlossen.", slot.name, port))
                } else {
                    continue;
                };
                out.push(Problem { kind, severity, dev: slot.dev, port: Some(p), link: None, others: Vec::new(), text: m.text(), msg: m });
            }
        }
    }

    /// the power went off: the sessions are gone, the ports stay as the service will find them
    /// when it starts again (it closes what asks for it then)
    pub(crate) fn dot1x_power_off(&mut self, ni: usize) {
        let auto: Vec<u16> = self.cdev(ni).map(|c| c.cfg.dot1x.ports.iter().filter(|(p, _)| c.cfg.dot1x.controls(**p).is_some()).map(|(p, _)| *p).collect()).unwrap_or_default();
        for p in auto {
            self.dot1x_set_closed(ni, p, true);
            self.dot1x_set_vlan(ni, p, None);
        }
    }

    /// a port's link changed (nothing for devices without the service)
    pub(crate) fn dot1x_link(&mut self, ni: usize, _port: u16) {
        let Some(id) = self.os(ni).and_then(|o| o.svc_id(SVC)) else { return };
        if self.os(ni).is_some_and(|o| o.running(id)) {
            self.svc_call(ni, id, None, |svc, os| {
                if let Some(a) = svc.as_any_mut().downcast_mut::<Dot1xAuth>() {
                    a.sync(os);
                }
            });
        }
    }

    /// an EAPOL frame on a bridge port: true when an authenticator took it
    pub(crate) fn dot1x_rx(&mut self, ni: usize, port: u16, frame: &[u8], hop: HopId) -> bool {
        let Some(id) = self.os(ni).and_then(|o| o.svc_id(SVC)) else { return false };
        let Some(l2) = L2::parse(frame) else { return false };
        let Some(pdu) = frame.get(l2.off..) else { return false };
        if !self.os(ni).is_some_and(|o| o.running(id)) {
            return false;
        }
        self.svc_call(ni, id, Some(hop), |svc, os| {
            if let Some(a) = svc.as_any_mut().downcast_mut::<Dot1xAuth>() {
                a.rx(os, SesKey::Port(port), l2.src, pdu);
            }
        });
        true
    }

    /// a WLAN association starts its 802.1X on the access point
    pub(crate) fn dot1x_wlan_begin(&mut self, ni: usize, assoc: u32) {
        let Some(id) = self.dot1x_service(ni) else { return };
        if !self.os(ni).is_some_and(|o| o.running(id)) {
            // the service was made just now: it starts here, not after the inbox
            if let Some(s) = self.os_mut(ni).and_then(|o| o.svcs.iter_mut().find(|s| s.id == id)) {
                s.running = true;
            }
            if let Some(o) = self.os_mut(ni) {
                o.inbox.retain(|j| !matches!(j, crate::os::Job::Start(i) if *i == id));
            }
        }
        self.svc_call(ni, id, None, |svc, os| {
            if let Some(a) = svc.as_any_mut().downcast_mut::<Dot1xAuth>() {
                a.begin_sta(os, assoc);
            }
        });
    }

    /// an EAPOL frame from a station that is not a key message
    pub(crate) fn dot1x_wlan_rx(&mut self, ni: usize, assoc: u32, src: Mac, pdu: &[u8], hop: HopId) {
        let Some(id) = self.os(ni).and_then(|o| o.svc_id(SVC)) else { return };
        self.svc_call(ni, id, Some(hop), |svc, os| {
            if let Some(a) = svc.as_any_mut().downcast_mut::<Dot1xAuth>() {
                a.rx(os, SesKey::Sta(assoc), src, pdu);
            }
        });
    }

    /// a WLAN association went
    pub(crate) fn dot1x_wlan_end(&mut self, ni: usize, assoc: u32) {
        let Some(id) = self.os(ni).and_then(|o| o.svc_id(SVC)) else { return };
        // (the end may come from inside the service: then it is out of its slot and has its
        // session removed by whoever called)
        self.svc_call(ni, id, None, |svc, os| {
            if let Some(a) = svc.as_any_mut().downcast_mut::<Dot1xAuth>() {
                a.end_sta(os, assoc);
            }
        });
    }

    /// "dot1x re-authenticate", "clear authentication sessions" (None: every port)
    pub(crate) fn dot1x_reauth(&mut self, ni: usize, port: Option<u16>) {
        let Some(id) = self.os(ni).and_then(|o| o.svc_id(SVC)) else { return };
        self.svc_call(ni, id, None, |svc, os| {
            if let Some(a) = svc.as_any_mut().downcast_mut::<Dot1xAuth>() {
                let keys: Vec<SesKey> = match port {
                    Some(p) => alloc::vec![SesKey::Port(p)],
                    None => a.ses.keys().copied().collect(),
                };
                for k in keys {
                    a.reauth(os, k);
                }
            }
        });
    }

    /// opens or closes a bridge port for everything but EAPOL; what it learned goes with it
    pub(crate) fn dot1x_set_closed(&mut self, ni: usize, port: u16, closed: bool) {
        let Some(Some(n)) = self.nodes.get_mut(ni) else { return };
        let Some(b) = n.bridge.as_mut() else { return };
        let Some(p) = b.ports.get_mut(port as usize) else { return };
        if p.closed != closed {
            p.closed = closed;
            if closed {
                b.flush_port(port);
            }
        }
    }

    /// the VLAN RADIUS assigned (None: the port's own again)
    pub(crate) fn dot1x_set_vlan(&mut self, ni: usize, port: u16, vlan: Option<u16>) {
        let Some(Some(n)) = self.nodes.get_mut(ni) else { return };
        if let Some(b) = n.bridge.as_mut() {
            b.assign_vlan(port, vlan);
        }
    }

    /// the VLAN exists on the bridge
    pub(crate) fn bridge_has_vlan(&self, ni: usize, vlan: u16) -> bool {
        self.nodes.get(ni).and_then(|n| n.as_ref()).and_then(|n| n.bridge.as_ref()).is_some_and(|b| b.vlans.contains_key(&vlan))
    }

    /// the port lets only EAPOL through
    pub fn dot1x_port_closed(&self, dev: u32, port: u16) -> bool {
        self.node(dev).and_then(|n| n.bridge.as_ref()).and_then(|b| b.ports.get(port as usize)).is_some_and(|p| p.closed)
    }

    /// the sessions of a device's authenticator
    pub fn dot1x_sessions(&self, dev: u32) -> Vec<SesView> {
        let Some(ni) = self.node_index(dev) else { return Vec::new() };
        match self.service::<Dot1xAuth>(dev, SVC) {
            Some(a) => a.views(self, ni),
            None => Vec::new(),
        }
    }
}
