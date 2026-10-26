// wlan -- WLAN on layer 2: access points offer networks, stations scan, authenticate, associate
// and run the 4-way handshake; an association is a radio link between the station's port and
// the radio of the access point, made and dropped by the simulator (no cable of the project).
// The access point bridges its stations to its wired side and to each other; IP, DHCP and
// everything above work over it unchanged.  No radio physics: a station reaches an access point
// within the range of the network's band (jfn_model::wlan), the distance gives a signal, the
// signal a rate (see "Radio" below).
//
//   Engine::wlan_assocs() -> Vec<AssocView>     every association: station, AP, ports, SSID,
//                                               band, channel, signal, rate, state
//   wlan_station(dev) -> Option<StaView>        a station's state, network, failures, retry
//   wlan_survey(dev, ssid) -> Vec<Bss>          what a scan of the station finds now
//   wlan_ver()                                  changes with every association change
//   wlan_settings(dev) / wlan_pos(dev) / wlan_scale()   what the engine was told
//   Action::Move / Wlan / Scale (action.rs)     positions, settings, the scale of the canvas
//
// Hooks for the rest of the engine (crate): wlan_register (add_node), wlan_boot (from_project),
// wlan_added / wlan_removing / wlan_power / wlan_port_admin / wlan_fixing / wlan_unfixed
// (action.rs), wlan_timer (kernel.rs, node timer slots 12 and 13), wlan_link_up / wlan_carrier /
// wlan_rx / wlan_ap_end (link.rs), wlan_dyn_radio / wlan_radio_live / wlan_radio_up /
// wlan_ap_send (bridge.rs, l3.rs, view.rs, the console), wlan_end_vlan (problems.rs).
//
// Stations (a radio client port without a cable of the project) that have a network to join:
//   scan     every channel of their bands, 20 ms each (2.4 GHz 13, 5 GHz 19, 6 GHz the 6
//            preferred scanning channels): a laptop takes 760 ms; then the network of the SSID
//            with the best estimated rate wins (as wpa_supplicant's throughput estimate), then
//            the stronger signal, the higher band, the lower device id
//   auth     open system (or SAE for WPA3: its commit and confirm with 10 ms of computing):
//            two management frames of 250 µs each; a wrong SAE password: the AP never confirms,
//            the station gives up 1 s later
//   assoc    two frames, 500 µs; the AP gives an AID, announces the station with a Layer 2
//            Update frame (an 802.2 XID broadcast with the station's address, as hostapd does)
//   4-way    WPA2 / WPA3: real EAPOL-Key frames on the radio link (wlankey.rs); a MIC that does
//            not fit (another passphrase) is dropped, message 1 comes again after 100 ms, then
//            every second, four times in all, and then the AP ends it (reason 15)
//   failure  the station tries again after 10 s, 20 s, 30 s, and after the fourth failure waits
//            until something changes (its settings, an access point's networks of its SSID,
//            `iw ... connect`)
//   moves    a station or AP that moves (Action::Move: the window, "move" in scenarios) looks
//            again: out of range the station misses the beacons and counts the connection as
//            lost after 7 beacon intervals (716.8 ms), frames meanwhile get lost in the air; with
//            less than -70 dBm and an AP of its network at least 8 dB better it reassociates
//            there (roaming: a new handshake, the old AP lets it go; the station's port keeps
//            its carrier, so its addresses, neighbours and lease stay); a station without a
//            network looks again when anything moves or changes
//   off      a station switched off (or `iw ... disconnect`, new settings) leaves (reason 3);
//            an AP switched off falls silent (its stations notice it as above); an AP whose
//            network changes or whose radio is shut down ends its stations' associations
// Management frames (beacons, probes, authentication, association) are no frames here: they
// show as notes with their times.  Data travels as Ethernet II frames on the radio link (as the
// station's driver hands them to its stack), the handshake as EAPOL frames (ethertype 0x888E),
// so the recording, the packet tab and pcapng show both.
//
// Radio: the signal is -82 dBm at the edge of the range and 30 dB stronger per decade nearer
// (a path loss exponent of 3, indoors), measured from centre to centre of the boxes, in metres
// by the project's scale (dip_per_m, default 10 world units per metre).  The rate follows the
// minimum sensitivity of 802.11ax MCS 0 to 11 (-82 .. -52 dBm) as a share of the fastest rate
// the band, the AP's radio and the station allow; the link carries two thirds of it (preambles,
// acknowledgements, contention), each frame takes 300 µs (2.4 GHz), 200 µs (5 GHz) or 150 µs
// (6 GHz) to get the medium plus its flight time, and below -78 dBm up to 2 % of the frames get
// lost (at the very edge).  All of it is integer arithmetic, the same on every target.
//
// Access points: the networks of a band share the radio port of that band (its first
// network's channel counts); a radio with a cable of the project (older projects) is that
// link's alone, and so is a station's port with one: such radio links keep working as fixed
// associations, as before.  The AP's bridge sees all stations of a radio on its port: frames to
// a station go on that station's link (a broadcast to every station of the VLAN but its sender),
// a station's frame to another one is relayed unless client isolation or a guest network is on.
// With VLANs (WlanNet::vlan) the radio ports and, on a plain access point, wired ports at their
// default carry them as a trunk with VLAN 1 untagged.  A guest network's stations reach no
// other station; on a routing device (the home router) only the device itself.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use base::rng::Pcg32;
use jfn_model::catalog::{CliStyle, Media, F_ROUTER};
use jfn_model::project::Device;
use jfn_model::wlan::{self, Band, Role, Security, Wlan, WlanJoin, WlanNet};
use jfn_model::CableKind;
use netpkt::eapol::{EapolView, Key};
use netpkt::eth::{self, VlanTag, ET_EAPOL, L2};
use netpkt::Mac;

use crate::bridge::{PortMode, SwMode, VlanSet};
use crate::kernel::{Engine, Frame, Ns, MS, SEC, US};
use crate::link::{Dir, LinkRt};
use crate::trace::{HopId, Note};
use crate::wlankey::{self, Ptk};
use crate::wlantrace::WNote;

/// node timer slots: a station's steps and beacon loss, an access point's handshakes
pub const SLOT_WLAN_STA: u32 = 12;
pub const SLOT_WLAN_AP: u32 = 13;

/// association links have ids from here up (the project's ids stay far below)
pub const ASSOC_ID0: u32 = 0xF000_0000;

/// a link of the engine that is an association
pub fn is_assoc_id(id: u32) -> bool {
    id >= ASSOC_ID0
}

/// a scan listens this long on each channel
pub const DWELL: Ns = 20 * MS;
/// a management frame with its acknowledgement
pub const MGMT: Ns = 250 * US;
/// the computations of SAE (both sides)
pub const SAE_TIME: Ns = 10 * MS;
/// a station waits this long for an SAE confirm that does not come
pub const SAE_WAIT: Ns = SEC;
/// seven beacon intervals of 102.4 ms
pub const BEACON_LOSS: Ns = 716_800 * US;
/// the first repetition of a handshake message, the later ones, how many are sent in all
pub const M_FIRST: Ns = 100 * MS;
pub const M_NEXT: Ns = SEC;
pub const M_TRIES: u8 = 4;
/// the waits after the first, second and third failure; the fourth one ends the tries
pub const BACKOFF_S: [u32; 3] = [10, 20, 30];
pub const GIVE_UP: u8 = 4;
/// the signal at the edge of the range, the roaming trigger and the gain a roam needs (tenths
/// of dBm)
pub const EDGE10: i32 = -820;
pub const ROAM_BELOW10: i32 = -700;
pub const ROAM_GAIN10: i32 = 80;
/// frames get lost below this signal, up to LOSS_EDGE_PPM at the edge
const LOSS_FROM10: i32 = -780;
const LOSS_EDGE_PPM: u32 = 20_000;
/// the minimum sensitivity of 802.11ax MCS 0 .. 11 (tenths of dBm) and the share of the
/// fastest rate each gives (per mille)
const MCS_DBM10: [i32; 12] = [-820, -790, -770, -740, -700, -660, -650, -640, -590, -570, -540, -520];
const MCS_PM: [u32; 12] = [60, 120, 180, 240, 360, 480, 540, 600, 720, 800, 900, 1000];

/// where a frame for the radio of an access point comes from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Src {
    /// the device itself (its own interface)
    Own,
    /// a wired port of its bridge
    Wired,
    /// one of its stations (an association id)
    Sta(u32),
}

/// why a station waits without trying
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    /// no network of its SSID in range
    NoNetwork,
    /// the network is protected and the station has no passphrase
    NoPassphrase,
    /// it failed too often
    GaveUp,
}

/// a network a scan found
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bss {
    /// the access point (device id), its network (index in its settings) and radio port
    pub ap: u32,
    pub net: usize,
    pub port: u16,
    pub ssid: String,
    pub band: Band,
    pub channel: u16,
    pub security: Security,
    pub bssid: Mac,
    /// signal in tenths of dBm
    pub rssi10: i32,
    /// the PHY rate the station would get (kbit/s)
    pub kbps: u32,
}

impl Bss {
    /// the signal in whole dBm (rounded, halves up)
    pub fn dbm(&self) -> i32 {
        dbm(self.rssi10)
    }
}

/// a station's step
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// nothing to join, switched off, or its port has a cable
    Idle,
    Scan { until: Ns },
    /// authentication with an AP (`ok`: it will succeed); `old`: the association it roams from
    Auth { bss: Bss, until: Ns, ok: bool, old: Option<u32> },
    Assoc { bss: Bss, until: Ns, old: Option<u32>, rands: ([u8; 32], [u8; 32]) },
    /// associated (the handshake may still run)
    Joined { assoc: u32 },
    Backoff { until: Ns },
    Waiting(Wait),
}

/// a station
#[derive(Clone, Debug)]
pub struct Sta {
    /// its radio port
    pub port: u16,
    /// what it joins now (the project's setting, or what `iw ... connect` said)
    pub join: Option<WlanJoin>,
    pub phase: Phase,
    pub failures: u8,
}

/// the access point's side of a handshake
#[derive(Clone, Debug)]
pub(crate) struct ApHs {
    anonce: [u8; 32],
    replay: u64,
    /// the message it waits for: 2 or 4
    want: u8,
    /// messages 1 (or 3) sent
    tries: u8,
    due: Ns,
    ptk: Option<Ptk>,
}

/// the station's side of a handshake
#[derive(Clone, Debug)]
pub(crate) struct StaHs {
    anonce: [u8; 32],
    snonce: [u8; 32],
    ptk: Ptk,
}

/// an association
#[derive(Clone, Debug)]
pub struct Assoc {
    pub id: u32,
    pub(crate) li: usize,
    pub sta: u32,
    pub(crate) sta_ni: usize,
    pub sta_port: u16,
    pub sta_mac: Mac,
    pub ap: u32,
    pub(crate) ap_ni: usize,
    pub ap_port: u16,
    pub net: usize,
    pub ssid: String,
    pub band: Band,
    pub channel: u16,
    pub bssid: Mac,
    pub security: Security,
    /// the VLAN of the station on the AP's bridge
    pub vlan: u16,
    pub aid: u16,
    /// the station installed its keys / the AP did (open networks: both at once)
    pub sta_ok: bool,
    pub ap_ok: bool,
    pub rssi10: i32,
    pub kbps: u32,
    pub mcs: u8,
    pub since: Ns,
    /// it came from a roam: the station's port kept its carrier (no link change for its stack)
    pub roam: bool,
    /// out of range (or the AP fell silent) since
    pub lost: Option<Ns>,
    pub(crate) ap_hs: Option<ApHs>,
    pub(crate) sta_hs: Option<StaHs>,
    /// the PMK each side works with (another passphrase: another PMK)
    pmk_ap: [u8; 32],
    pmk_sta: [u8; 32],
}

/// what the engine knows of a device for WLAN
#[derive(Clone, Debug)]
pub struct WDev {
    pub x: i32,
    pub y: i32,
    pub cfg: Option<Wlan>,
    rng: Pcg32,
    /// the group keys (GTK, IGTK) of its networks by index
    gtk: BTreeMap<usize, ([u8; 16], [u8; 16])>,
    /// ports the WLAN layer made trunks (VLANs of its networks)
    auto: Vec<u16>,
}

/// the WLAN state of the engine
#[derive(Clone, Debug, Default)]
pub struct WlanState {
    /// world units per metre (0: jfn_model::wlan::DIP_PER_M)
    pub dip_per_m: u32,
    pub devs: BTreeMap<u32, WDev>,
    pub stas: BTreeMap<u32, Sta>,
    pub assocs: BTreeMap<u32, Assoc>,
    pub(crate) by_link: BTreeMap<usize, u32>,
    next: u32,
    ver: u32,
    /// PMKs of WPA2 by (passphrase, SSID): PBKDF2 costs, and the result never changes
    pmk: BTreeMap<(String, String), [u8; 32]>,
}

/// an association for the user interface
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssocView {
    /// its link id (the frames of the recording name it)
    pub id: u32,
    pub sta: u32,
    pub sta_port: u16,
    pub ap: u32,
    pub ap_port: u16,
    pub ssid: String,
    pub band: Band,
    pub channel: u16,
    pub security: Security,
    pub bssid: Mac,
    pub aid: u16,
    pub vlan: u16,
    /// dBm
    pub dbm: i32,
    /// the PHY rate (kbit/s)
    pub kbps: u32,
    /// the handshake is done on both sides
    pub authorized: bool,
    /// out of range or the AP fell silent: the station will notice
    pub lost: bool,
    pub since: Ns,
}

/// what a station does
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StaState {
    /// no network set, switched off or its radio port is down
    Idle,
    /// its radio port has a cable of the project (an older project's radio link)
    Fixed,
    Scanning,
    /// authentication and association
    Joining,
    /// associated, the 4-way handshake runs
    Handshake,
    Connected,
    /// roaming to another AP of its network
    Roaming,
    Backoff,
    NoNetwork,
    NoPassphrase,
    GaveUp,
}

/// a station for the user interface
#[derive(Clone, Debug, PartialEq)]
pub struct StaView {
    pub dev: u32,
    pub port: u16,
    pub state: StaState,
    /// what it joins now
    pub join: Option<WlanJoin>,
    /// its association
    pub assoc: Option<AssocView>,
    pub failures: u8,
    /// when it tries again (Backoff)
    pub retry_at: Option<Ns>,
}

/// whole dBm of tenths (rounded, halves up)
pub fn dbm(rssi10: i32) -> i32 {
    (rssi10 + 5).div_euclid(10)
}

/// the integer square root
fn isqrt(v: u64) -> u64 {
    if v < 2 {
        return v;
    }
    let mut x = v;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + v / x) / 2;
    }
    x
}

/// 1000 * log10(v), v >= 1 (integer arithmetic: log2 by squaring, 16 fractional bits)
fn log10_milli(v: u64) -> i64 {
    let v = v.max(1);
    let k = 63 - v.leading_zeros() as i64;
    // the mantissa in [1, 2) with 32 fractional bits
    let mut m: u128 = ((v as u128) << 32) >> k;
    let mut frac: i64 = 0;
    for i in 0..16 {
        m = (m * m) >> 32;
        if m >= 2u128 << 32 {
            m >>= 1;
            frac |= 1 << (15 - i);
        }
    }
    let log2_q16 = (k << 16) | frac;
    // 1000 * log10(2) = 301.03, rounded to the nearest thousandth of a decade
    (log2_q16 * 30103 + 3_276_800) / 6_553_600
}

/// the signal at `d_mm` from an AP whose band reaches `range_m` (tenths of dBm): -82 dBm at the
/// range, 30 dB per decade; nearer than 1 m counts as 1 m
pub fn signal10(range_m: u32, d_mm: u64) -> i32 {
    let lr = log10_milli(range_m.max(1) as u64 * 1000) - 3000;
    let ld = log10_milli(d_mm.max(1000)) - 3000;
    EDGE10 + ((lr - ld) * 300 / 1000) as i32
}

/// the MCS a signal allows (None: below MCS 0) and its share of the fastest rate (per mille)
pub fn mcs_of(rssi10: i32) -> Option<(u8, u32)> {
    (0..12).rev().find(|&i| rssi10 >= MCS_DBM10[i]).map(|i| (i as u8, MCS_PM[i]))
}

/// the time a frame needs to get the medium on a band
pub fn air_ns(band: Band) -> Ns {
    match band {
        Band::G24 => 300 * US,
        Band::G5 => 200 * US,
        Band::G6 => 150 * US,
    }
}

/// the frames lost at a signal (ppm): none above -78 dBm, up to 2 % at the edge
pub fn edge_loss_ppm(rssi10: i32) -> u32 {
    if rssi10 >= LOSS_FROM10 {
        return 0;
    }
    let d = (LOSS_FROM10 - rssi10.max(EDGE10)) as u32;
    LOSS_EDGE_PPM * d / (LOSS_FROM10 - EDGE10) as u32
}

/// the BSSID of the k-th network on a radio port: the port's MAC, the network's number in the
/// first byte (locally administered, unicast)
pub fn bssid(port_mac: Mac, k: usize) -> Mac {
    let mut m = port_mac;
    m.0[0] = 0x02 | ((k as u8 & 0x0F) << 4);
    m
}

impl WlanState {
    fn bump(&mut self) {
        self.ver = self.ver.wrapping_add(1);
    }
}

impl Engine {
    // ---------------------------------------------------------------- facts

    /// the scale in world units per metre
    pub fn wlan_scale(&self) -> u32 {
        if self.wlan.dip_per_m == 0 { wlan::DIP_PER_M } else { self.wlan.dip_per_m }
    }

    /// what the engine was told of a device's WLAN settings
    pub fn wlan_settings(&self, dev: u32) -> Option<&Wlan> {
        self.wlan.devs.get(&dev)?.cfg.as_ref()
    }

    /// a device's position (top left corner of its box)
    pub fn wlan_pos(&self, dev: u32) -> Option<(i32, i32)> {
        self.wlan.devs.get(&dev).map(|d| (d.x, d.y))
    }

    /// changes with every association that comes, goes or changes
    pub fn wlan_ver(&self) -> u32 {
        self.wlan.ver
    }

    /// the distance of two devices' centres in millimetres
    fn wlan_dist_mm(&self, a: u32, b: u32) -> u64 {
        let (Some(x), Some(y)) = (self.wlan.devs.get(&a), self.wlan.devs.get(&b)) else {
            return u64::MAX / 4;
        };
        let (dx, dy) = ((x.x as i64 - y.x as i64).unsigned_abs(), (x.y as i64 - y.y as i64).unsigned_abs());
        let s = (dx * dx + dy * dy).saturating_mul(1_000_000);
        isqrt(s) / self.wlan_scale().max(1) as u64
    }

    /// the nets an access point offers, None for anything else
    fn ap_nets(&self, dev: u32) -> Option<&[WlanNet]> {
        let n = self.node(dev)?;
        if wlan::role(n.model) != Role::Ap {
            return None;
        }
        Some(self.wlan.devs.get(&dev)?.cfg.as_ref()?.nets.as_slice())
    }

    /// the index of the radio port of a band on an AP
    fn radio_index(&self, dev: u32, band: Band) -> Option<u16> {
        let n = self.node(dev)?;
        let name = wlan::radio_port(n.model, band)?;
        n.port_by_name(&name)
    }

    /// the radio port of an AP is free for associations: no cable of the project on it
    pub(crate) fn wlan_dyn_radio(&self, ni: usize, pi: u16) -> bool {
        let Some(Some(n)) = self.nodes.get(ni) else { return false };
        n.ports.get(pi as usize).is_some_and(|p| p.spec.media == Media::RadioAp && p.link.is_none()) && wlan::role(n.model) == Role::Ap
    }

    /// the radio port beacons: powered, not shut down, free, and a valid network is on it
    pub(crate) fn wlan_radio_up(&self, ni: usize, pi: u16) -> bool {
        if !self.wlan_dyn_radio(ni, pi) {
            return false;
        }
        let Some(Some(n)) = self.nodes.get(ni) else { return false };
        if !n.power || !n.ports[pi as usize].admin_up {
            return false;
        }
        let dev = n.dev;
        let bands = wlan::ap_bands(n.model);
        self.ap_nets(dev).is_some_and(|nets| nets.iter().any(|x| bands & x.band.bit() != 0 && wlan::check_net(x).is_none() && self.radio_index(dev, x.band) == Some(pi)))
    }

    /// a station of the radio port has finished its handshake (the bridge floods there)
    pub(crate) fn wlan_radio_live(&self, ni: usize, pi: u16) -> bool {
        self.wlan.assocs.values().any(|a| a.ap_ni == ni && a.ap_port == pi && a.ap_ok)
    }

    /// the untagged VLAN of a bridge port (access VLAN, native VLAN of a trunk)
    fn native_vlan(&self, ni: usize, pi: u16) -> u16 {
        match self.nodes.get(ni).and_then(|n| n.as_ref()).and_then(|n| n.bridge.as_ref()).and_then(|b| b.ports.get(pi as usize)) {
            Some(bp) => match &bp.mode {
                PortMode::Access(v) => *v,
                PortMode::Trunk { native, .. } => *native,
            },
            None => 1,
        }
    }

    /// what a scan of the station finds now: the networks of its SSID (None: every SSID) on
    /// its bands within range, best first
    pub fn wlan_survey(&self, dev: u32, ssid: Option<&str>) -> Vec<Bss> {
        let Some(sn) = self.node(dev) else { return Vec::new() };
        let Some(sp) = self.wlan.stas.get(&dev).map(|s| s.port) else { return Vec::new() };
        let sbands = wlan::client_bands(sn.model);
        let smax = sn.ports.get(sp as usize).map_or(0, |p| p.spec.kbps);
        let mut out = Vec::new();
        for (&ap, wd) in &self.wlan.devs {
            if ap == dev {
                continue;
            }
            let Some(ani) = self.node_index(ap) else { continue };
            let Some(Some(an)) = self.nodes.get(ani) else { continue };
            if !an.power || wlan::role(an.model) != Role::Ap {
                continue;
            }
            let Some(cfg) = wd.cfg.as_ref() else { continue };
            let abands = wlan::ap_bands(an.model);
            let d = self.wlan_dist_mm(dev, ap);
            // the networks on each radio port, numbered for their BSSIDs
            let mut k_of: BTreeMap<u16, usize> = BTreeMap::new();
            for (i, net) in cfg.nets.iter().enumerate() {
                let Some(port) = self.radio_index(ap, net.band) else { continue };
                let k = *k_of.entry(port).and_modify(|k| *k += 1).or_insert(0);
                if abands & net.band.bit() == 0 || wlan::check_net(net).is_some() || !self.wlan_radio_up(ani, port) {
                    continue;
                }
                if sbands & net.band.bit() == 0 || ssid.is_some_and(|s| s != net.ssid) {
                    continue;
                }
                let range = wlan::range_m(an.model, net.band);
                if d > range as u64 * 1000 {
                    continue;
                }
                let rssi10 = signal10(range, d);
                let amax = an.ports.get(port as usize).map_or(0, |p| p.spec.kbps);
                let max = wlan::band_max_kbps(net.band).min(amax).min(smax.max(1));
                let kbps = mcs_of(rssi10).map_or(0, |(_, pm)| (max as u64 * pm as u64 / 1000) as u32);
                out.push(Bss { ap, net: i, port, ssid: net.ssid.clone(), band: net.band, channel: wlan::channel_of(net), security: net.security, bssid: bssid(an.ports[port as usize].mac, k), rssi10, kbps });
            }
        }
        // the best first: rate, signal, band, then the lower ids
        out.sort_by(|a, b| (b.kbps, b.rssi10, b.band.index(), a.ap, a.net).cmp(&(a.kbps, a.rssi10, a.band.index(), b.ap, b.net)));
        out
    }

    /// the PMK of WPA2 for a passphrase and SSID (kept: PBKDF2 has 4096 rounds)
    fn pmk_for(&mut self, pass: &str, ssid: &str) -> [u8; 32] {
        let key = (String::from(pass), String::from(ssid));
        if let Some(p) = self.wlan.pmk.get(&key) {
            return *p;
        }
        let p = wlankey::pmk_psk(pass, ssid);
        self.wlan.pmk.insert(key, p);
        p
    }

    fn name(&self, dev: u32) -> String {
        self.node(dev).map(|n| n.name.clone()).unwrap_or_else(|| format!("{}", dev))
    }

    fn wnote(&mut self, dev: u32, hop: Option<HopId>, n: WNote) {
        self.note(dev, hop, Note::Wlan(n));
    }

    /// a line of a device's log: a Linux device's syslog (tag), an IOS-like one's console
    fn wlan_log(&mut self, ni: usize, tag: &str, ios: &str, text: &str) {
        let style = self.nodes.get(ni).and_then(|n| n.as_ref()).map(|n| n.model.cli);
        match style {
            Some(CliStyle::Ios) => self.ios_log(ni, &format!("{}: {}", ios, text)),
            Some(CliStyle::Linux) => self.os_log(ni, 6, tag, text, None),
            _ => {}
        }
    }

    // ---------------------------------------------------------------- registration

    /// a device starts running (add_node): its position and settings; a station gets its state
    pub(crate) fn wlan_register(&mut self, d: &Device, ni: usize) {
        let rng = Pcg32::derive(self.seed ^ 0x574C_414E, d.id as u64);
        self.wlan.devs.insert(d.id, WDev { x: d.x, y: d.y, cfg: d.wlan.clone(), rng, gtk: BTreeMap::new(), auto: Vec::new() });
        let Some(Some(n)) = self.nodes.get(ni) else { return };
        if wlan::role(n.model) == Role::Station
            && let Some(port) = wlan::station_port(n.model).and_then(|p| n.port_by_name(&p))
        {
            let join = d.wlan.as_ref().and_then(|w| w.join.clone()).filter(|j| !j.ssid.is_empty());
            self.wlan.stas.insert(d.id, Sta { port, join, phase: Phase::Idle, failures: 0 });
        }
        self.wlan_vlans(ni);
    }

    /// the project is loaded: every station with a network starts looking
    pub(crate) fn wlan_boot(&mut self) {
        let stas: Vec<u32> = self.wlan.stas.keys().copied().collect();
        for s in stas {
            self.sta_start(s);
        }
    }

    /// a device came (Action::AddDevice): a station starts, an AP wakes the waiting stations
    pub(crate) fn wlan_added(&mut self, dev: u32) {
        if self.wlan.stas.contains_key(&dev) {
            self.sta_start(dev);
        }
        if self.ap_nets(dev).is_some() {
            self.wlan_wake();
        }
    }

    /// a device goes (Action::RemoveDevice): its associations end
    pub(crate) fn wlan_removing(&mut self, dev: u32) {
        let ids: Vec<u32> = self.wlan.assocs.values().filter(|a| a.sta == dev || a.ap == dev).map(|a| a.id).collect();
        for id in ids {
            let leaving = self.wlan.assocs.get(&id).is_some_and(|a| a.sta == dev);
            self.assoc_drop(id, if leaving { Why::StaLeaving } else { Why::ApLeaving });
        }
        self.wlan.stas.remove(&dev);
        self.wlan.devs.remove(&dev);
        self.wlan.bump();
    }

    /// the power of a device changed (after the node itself was switched)
    pub(crate) fn wlan_power(&mut self, ni: usize, on: bool) {
        let dev = self.dev_of(ni);
        if self.wlan.stas.contains_key(&dev) {
            if on {
                self.sta_start(dev);
            } else {
                let ids: Vec<u32> = self.wlan.assocs.values().filter(|a| a.sta == dev).map(|a| a.id).collect();
                for id in ids {
                    self.assoc_drop(id, Why::StaLeaving);
                }
                if let Some(s) = self.wlan.stas.get_mut(&dev) {
                    s.phase = Phase::Idle;
                }
            }
        }
        if self.ap_nets(dev).is_some() {
            if on {
                self.wlan_wake();
            } else {
                // it falls silent: its stations notice the missing beacons
                let ids: Vec<u32> = self.wlan.assocs.values().filter(|a| a.ap == dev).map(|a| a.id).collect();
                for id in ids {
                    self.assoc_lost(id, true);
                }
            }
        }
        self.wlan.bump();
    }

    /// a port was shut down or switched on: an AP's radio ends its associations, a station's
    /// port leaves or starts looking
    pub(crate) fn wlan_port_admin(&mut self, ni: usize, pi: u16) {
        let dev = self.dev_of(ni);
        let up = self.nodes.get(ni).and_then(|n| n.as_ref()).is_some_and(|n| n.ports.get(pi as usize).is_some_and(|p| p.admin_up));
        if self.wlan.stas.get(&dev).is_some_and(|s| s.port == pi) {
            if up {
                self.sta_start(dev);
            } else {
                let ids: Vec<u32> = self.wlan.assocs.values().filter(|a| a.sta == dev).map(|a| a.id).collect();
                for id in ids {
                    self.assoc_drop(id, Why::StaLeaving);
                }
                if let Some(s) = self.wlan.stas.get_mut(&dev) {
                    s.phase = Phase::Idle;
                }
                self.sta_arm(dev);
            }
        }
        if self.wlan_dyn_radio(ni, pi) {
            if up {
                self.wlan_wake();
            } else {
                let ids: Vec<u32> = self.wlan.assocs.values().filter(|a| a.ap_ni == ni && a.ap_port == pi).map(|a| a.id).collect();
                for id in ids {
                    self.assoc_drop(id, Why::ApLeaving);
                }
            }
        }
        self.wlan.bump();
    }

    /// a cable of the project goes onto a port: what was associated there ends
    pub(crate) fn wlan_fixing(&mut self, ni: usize, pi: u16) {
        let radio = self.nodes.get(ni).and_then(|n| n.as_ref()).is_some_and(|n| n.ports.get(pi as usize).is_some_and(|p| p.spec.media.is_radio()));
        if !radio {
            return;
        }
        let ids: Vec<u32> = self.wlan.assocs.values().filter(|a| (a.sta_ni == ni && a.sta_port == pi) || (a.ap_ni == ni && a.ap_port == pi)).map(|a| a.id).collect();
        for id in ids {
            let sta = self.wlan.assocs.get(&id).is_some_and(|a| a.sta_ni == ni);
            self.assoc_drop(id, if sta { Why::StaLeaving } else { Why::ApLeaving });
        }
        let dev = self.dev_of(ni);
        if let Some(s) = self.wlan.stas.get_mut(&dev)
            && s.port == pi
        {
            s.phase = Phase::Idle;
        }
        self.sta_arm(dev);
    }

    /// a cable of the project left a port: a station may look again, an AP's radio is free
    pub(crate) fn wlan_unfixed(&mut self, ni: usize, pi: u16) {
        let dev = self.dev_of(ni);
        if self.wlan.stas.get(&dev).is_some_and(|s| s.port == pi) {
            self.sta_start(dev);
        }
        if self.wlan_dyn_radio(ni, pi) {
            self.wlan_wake();
        }
    }

    /// Action::Move: a device's new position; stations and APs look again
    pub(crate) fn wlan_move(&mut self, dev: u32, x: i32, y: i32) -> Result<(), crate::ActErr> {
        if self.node_index(dev).is_none() {
            return Err(crate::ActErr::UnknownDevice(dev));
        }
        match self.wlan.devs.get_mut(&dev) {
            Some(d) => {
                if (d.x, d.y) == (x, y) {
                    return Ok(());
                }
                d.x = x;
                d.y = y;
            }
            None => return Err(crate::ActErr::UnknownDevice(dev)),
        }
        self.wlan_reevaluate(Some(dev));
        Ok(())
    }

    /// Action::Scale: the scale of the canvas
    pub(crate) fn wlan_set_scale(&mut self, dip_per_m: u32) -> Result<(), crate::ActErr> {
        if self.wlan.dip_per_m == dip_per_m {
            return Ok(());
        }
        self.wlan.dip_per_m = dip_per_m;
        self.wlan_reevaluate(None);
        Ok(())
    }

    /// Action::Wlan: a device's settings (an AP's networks, a station's network)
    pub(crate) fn wlan_set(&mut self, dev: u32, cfg: Option<Wlan>) -> Result<(), crate::ActErr> {
        let ni = self.node_index(dev).ok_or(crate::ActErr::UnknownDevice(dev))?;
        let cfg = cfg.filter(|w| !w.is_empty());
        let old = match self.wlan.devs.get_mut(&dev) {
            Some(d) => core::mem::replace(&mut d.cfg, cfg.clone()),
            None => return Err(crate::ActErr::UnknownDevice(dev)),
        };
        if old == cfg {
            return Ok(());
        }
        // an AP: associations whose network changed end (the flags of a network count at once)
        if self.ap_nets(dev).is_some() || old.as_ref().is_some_and(|w| !w.nets.is_empty()) {
            if let Some(d) = self.wlan.devs.get_mut(&dev) {
                d.gtk.clear();
            }
            let ids: Vec<u32> = self.wlan.assocs.values().filter(|a| a.ap == dev).map(|a| a.id).collect();
            for id in ids {
                let keep = {
                    let a = &self.wlan.assocs[&id];
                    cfg.as_ref().and_then(|w| w.nets.get(a.net)).is_some_and(|n| n.ssid == a.ssid && n.band == a.band && wlan::channel_of(n) == a.channel && n.security == a.security && old.as_ref().and_then(|o| o.nets.get(a.net)).is_some_and(|o| o.passphrase == n.passphrase && o.vlan == n.vlan) && wlan::check_net(n).is_none())
                };
                if !keep {
                    self.assoc_drop(id, Why::ApLeaving);
                }
            }
            self.wlan_vlans(ni);
            let ssids: Vec<String> = old.iter().chain(cfg.iter()).flat_map(|w| w.nets.iter().map(|n| n.ssid.clone())).collect();
            self.wlan_wake_ssids(&ssids);
        }
        // a station: another network to join starts over
        if self.wlan.stas.contains_key(&dev) {
            let join = cfg.as_ref().and_then(|w| w.join.clone()).filter(|j| !j.ssid.is_empty());
            self.wlan_join(ni, join);
        }
        // an IOS-like AP tells about its radios
        self.ios_snap(ni);
        self.wlan.bump();
        Ok(())
    }

    /// a station joins another network now (or none): it leaves its AP and starts again
    pub(crate) fn wlan_join(&mut self, ni: usize, join: Option<WlanJoin>) {
        let dev = self.dev_of(ni);
        let Some(s) = self.wlan.stas.get(&dev) else { return };
        if s.join == join && !matches!(s.phase, Phase::Waiting(_) | Phase::Idle | Phase::Backoff { .. }) {
            return;
        }
        let ids: Vec<u32> = self.wlan.assocs.values().filter(|a| a.sta == dev).map(|a| a.id).collect();
        for id in ids {
            self.assoc_drop(id, Why::StaLeaving);
        }
        if let Some(s) = self.wlan.stas.get_mut(&dev) {
            s.join = join;
            s.failures = 0;
            s.phase = Phase::Idle;
        }
        self.sta_start(dev);
    }

    /// the stations that wait for a network look again; connected ones see whether to roam
    fn wlan_wake(&mut self) {
        let stas: Vec<(u32, bool)> = self.wlan.stas.iter().map(|(&d, s)| (d, matches!(s.phase, Phase::Waiting(Wait::NoNetwork)))).collect();
        for (d, waiting) in stas {
            if waiting {
                self.sta_start(d);
            } else {
                self.sta_roam_check(d);
            }
        }
    }

    /// an access point's networks changed: the stations of their SSIDs that gave up or had no
    /// passphrase try again as well (with their failures forgotten)
    fn wlan_wake_ssids(&mut self, ssids: &[String]) {
        let stas: Vec<u32> = self
            .wlan
            .stas
            .iter()
            .filter(|(_, s)| matches!(s.phase, Phase::Waiting(Wait::GaveUp | Wait::NoPassphrase)) && s.join.as_ref().is_some_and(|j| ssids.contains(&j.ssid)))
            .map(|(&d, _)| d)
            .collect();
        for d in stas {
            if let Some(s) = self.wlan.stas.get_mut(&d) {
                s.failures = 0;
                s.phase = Phase::Idle;
            }
            self.sta_start(d);
        }
        self.wlan_wake();
    }

    /// something moved (one device, or all with the scale): signals, ranges, roaming, waiting
    /// stations
    fn wlan_reevaluate(&mut self, dev: Option<u32>) {
        let ids: Vec<u32> = self.wlan.assocs.values().filter(|a| dev.is_none_or(|d| a.sta == d || a.ap == d)).map(|a| a.id).collect();
        for id in ids {
            self.assoc_signal(id, false);
        }
        let stas: Vec<u32> = self.wlan.stas.keys().copied().collect();
        for s in stas {
            let waiting = self.wlan.stas.get(&s).is_some_and(|x| matches!(x.phase, Phase::Waiting(Wait::NoNetwork)));
            if waiting {
                if dev.is_none_or(|d| d == s || self.ap_nets(d).is_some()) {
                    self.sta_start(s);
                }
            } else if dev.is_none_or(|d| d == s || self.wlan.assocs.values().any(|a| a.sta == s && a.ap == d) || self.ap_nets(d).is_some()) {
                self.sta_roam_check(s);
            }
        }
    }

    /// the VLANs of a plain AP's networks on its bridge: its radio ports and its wired ports at
    /// their default work as trunks with VLAN 1 untagged (their settings stay as they are, as a
    /// trunk a neighbour negotiated would), and as before when no network has a VLAN
    fn wlan_vlans(&mut self, ni: usize) {
        let Some(Some(n)) = self.nodes.get(ni) else { return };
        if wlan::role(n.model) != Role::Ap || n.bridge.is_none() || n.model.has(F_ROUTER) {
            return;
        }
        let dev = n.dev;
        let vlans: Vec<u16> = self.wlan.devs.get(&dev).and_then(|d| d.cfg.as_ref()).map(|w| w.nets.iter().map(|x| x.vlan).filter(|&v| (2..=4094).contains(&v)).collect()).unwrap_or_default();
        let ports: Vec<u16> = n.ports.iter().enumerate().filter(|(_, p)| p.spec.media == Media::RadioAp && p.link.is_none() || p.spec.media == Media::Rj45 || p.spec.media.is_optical()).map(|(i, _)| i as u16).collect();
        let auto = self.wlan.devs.get(&dev).map(|d| d.auto.clone()).unwrap_or_default();
        let Some(Some(n)) = self.nodes.get_mut(ni) else { return };
        let Some(b) = n.bridge.as_mut() else { return };
        let trunk = PortMode::Trunk { native: 1, allowed: VlanSet::all() };
        let mut now = Vec::new();
        let mut flush = Vec::new();
        for v in &vlans {
            if !b.vlans.contains_key(v) {
                b.add_vlan(*v, None);
            }
        }
        for p in ports {
            let Some(bp) = b.ports.get_mut(p as usize) else { continue };
            let default = bp.member && bp.admin == SwMode::Dynamic && bp.access_vlan == 1;
            if !vlans.is_empty() && default {
                if bp.mode != trunk {
                    bp.mode = trunk.clone();
                    flush.push(p);
                }
                now.push(p);
            } else if auto.contains(&p) && bp.mode == trunk && bp.admin == SwMode::Dynamic {
                bp.mode = PortMode::Access(bp.access_vlan);
                flush.push(p);
            }
        }
        for p in flush {
            b.flush_port(p);
        }
        if let Some(d) = self.wlan.devs.get_mut(&dev) {
            d.auto = now;
        }
    }

    // ---------------------------------------------------------------- stations

    /// a station starts looking for its network (if it should)
    fn sta_start(&mut self, dev: u32) {
        let Some(ni) = self.node_index(dev) else { return };
        let Some(s) = self.wlan.stas.get(&dev) else { return };
        let port = s.port;
        let join = s.join.clone();
        let (power, admin, fixed, model) = match self.nodes.get(ni) {
            Some(Some(n)) => (n.power, n.ports[port as usize].admin_up, n.ports[port as usize].link.is_some_and(|li| !self.wlan.by_link.contains_key(&li)), n.model),
            _ => return,
        };
        let busy = self.wlan.assocs.values().any(|a| a.sta == dev);
        let phase = if !power || !admin || fixed || join.is_none() {
            Phase::Idle
        } else if busy {
            return;
        } else {
            let bands = wlan::client_bands(model);
            let chans: usize = Band::ALL.iter().filter(|b| bands & b.bit() != 0).map(|&b| wlan::scan_channels(b)).sum();
            let dur = DWELL * chans as Ns;
            let ssid = join.map(|j| j.ssid).unwrap_or_default();
            self.wnote(dev, None, WNote::ScanStart { ssid, bands, ms: (dur / MS) as u32 });
            Phase::Scan { until: self.now + dur }
        };
        if let Some(s) = self.wlan.stas.get_mut(&dev) {
            s.phase = phase;
        }
        self.sta_arm(dev);
        self.wlan.bump();
    }

    /// arms the station's timer for what is due next
    fn sta_arm(&mut self, dev: u32) {
        let Some(ni) = self.node_index(dev) else { return };
        let Some(s) = self.wlan.stas.get(&dev) else { return };
        let mut due = match &s.phase {
            Phase::Scan { until } | Phase::Auth { until, .. } | Phase::Assoc { until, .. } | Phase::Backoff { until } => Some(*until),
            _ => None,
        };
        for a in self.wlan.assocs.values().filter(|a| a.sta == dev) {
            if let Some(l) = a.lost {
                let t = l + BEACON_LOSS;
                due = Some(due.map_or(t, |d: Ns| d.min(t)));
            }
        }
        match due {
            Some(t) => self.set_timer(ni, SLOT_WLAN_STA, t),
            None => self.cancel_timer(ni, SLOT_WLAN_STA),
        }
    }

    /// a node's WLAN timer (kernel.rs)
    pub(crate) fn wlan_timer(&mut self, ni: usize, slot: u32) {
        let dev = self.dev_of(ni);
        if slot == SLOT_WLAN_AP {
            self.ap_timer(ni);
            return;
        }
        let now = self.now;
        // beacon loss first
        let lost: Vec<u32> = self.wlan.assocs.values().filter(|a| a.sta == dev && a.lost.is_some_and(|l| l + BEACON_LOSS <= now)).map(|a| a.id).collect();
        for id in lost {
            self.assoc_drop(id, Why::BeaconLoss);
        }
        let phase = self.wlan.stas.get(&dev).map(|s| s.phase.clone());
        match phase {
            Some(Phase::Scan { until }) if until <= now => self.sta_scan_done(dev),
            Some(Phase::Auth { until, .. }) if until <= now => self.sta_auth_done(dev),
            Some(Phase::Assoc { until, .. }) if until <= now => self.sta_assoc_done(dev),
            Some(Phase::Backoff { until }) if until <= now => {
                if let Some(s) = self.wlan.stas.get_mut(&dev) {
                    s.phase = Phase::Idle;
                }
                self.sta_start(dev);
            }
            _ => self.sta_arm(dev),
        }
    }

    /// the scan is over: the best network, or nothing
    fn sta_scan_done(&mut self, dev: u32) {
        let Some(join) = self.wlan.stas.get(&dev).and_then(|s| s.join.clone()) else { return };
        let found = self.wlan_survey(dev, Some(&join.ssid));
        let Some(best) = found.first().cloned() else {
            self.wnote(dev, None, WNote::ScanNone { ssid: join.ssid.clone() });
            self.sta_wait(dev, Wait::NoNetwork);
            return;
        };
        let ap = self.name(best.ap);
        self.wnote(dev, None, WNote::ScanFound { ssid: join.ssid.clone(), n: found.len() as u16, ap, band: best.band, ch: best.channel, dbm: best.dbm() });
        if best.security.secured() && join.passphrase.is_empty() {
            self.wnote(dev, None, WNote::NoPassphrase { ssid: join.ssid.clone() });
            self.sta_wait(dev, Wait::NoPassphrase);
            return;
        }
        self.sta_auth(dev, best, None);
    }

    fn sta_wait(&mut self, dev: u32, w: Wait) {
        if let Some(s) = self.wlan.stas.get_mut(&dev) {
            s.phase = Phase::Waiting(w);
        }
        self.sta_arm(dev);
        self.wlan.bump();
    }

    /// authentication with the AP of a network (`old`: the association it roams from)
    fn sta_auth(&mut self, dev: u32, bss: Bss, old: Option<u32>) {
        let sae = bss.security == Security::Wpa3;
        let pass_sta = self.wlan.stas.get(&dev).and_then(|s| s.join.as_ref()).map(|j| j.passphrase.clone()).unwrap_or_default();
        let pass_ap = self.ap_nets(bss.ap).and_then(|n| n.get(bss.net)).map(|n| n.passphrase.clone()).unwrap_or_default();
        // SAE checks the password in the authentication already
        let ok = !sae || pass_sta == pass_ap;
        let dur = if sae { 4 * MGMT + SAE_TIME + if ok { 0 } else { SAE_WAIT } } else { 2 * MGMT };
        let ap = self.name(bss.ap);
        let sta = self.name(dev);
        self.wnote(dev, None, WNote::AuthStart { ap, sae, reassoc: old.is_some() });
        self.wnote(bss.ap, None, WNote::AuthBy { sta, sae });
        let until = self.now + dur;
        if let Some(s) = self.wlan.stas.get_mut(&dev) {
            s.phase = Phase::Auth { bss, until, ok, old };
        }
        self.sta_arm(dev);
        self.wlan.bump();
    }

    /// authentication done: on to the association (or a failure of SAE)
    fn sta_auth_done(&mut self, dev: u32) {
        let Some(Phase::Auth { bss, ok, old, .. }) = self.wlan.stas.get(&dev).map(|s| s.phase.clone()) else { return };
        if !ok {
            let sta = self.name(dev);
            let ap = self.name(bss.ap);
            self.wnote(bss.ap, None, WNote::SaeMismatch { sta });
            self.wnote(dev, None, WNote::AuthFailed { ap });
            if let Some(ni) = self.node_index(dev) {
                let text = format!("{}: no SAE confirm from {} ({}): the password may be wrong", self.port_name_of(dev), bss.bssid, bss.ssid);
                self.wlan_log(ni, "wpa_supplicant", "%DOT11-4-SAE", &text);
            }
            self.sta_failed(dev);
            return;
        }
        let mut rands = ([0u8; 32], [0u8; 32]);
        if bss.security == Security::Wpa3 {
            if let Some(d) = self.wlan.devs.get_mut(&bss.ap) {
                d.rng.fill(&mut rands.0);
            }
            if let Some(d) = self.wlan.devs.get_mut(&dev) {
                d.rng.fill(&mut rands.1);
            }
        }
        let until = self.now + 2 * MGMT;
        if let Some(s) = self.wlan.stas.get_mut(&dev) {
            s.phase = Phase::Assoc { bss, until, old, rands };
        }
        self.sta_arm(dev);
    }

    /// the association response: the station is on the AP (a roam leaves the old one first)
    fn sta_assoc_done(&mut self, dev: u32) {
        let Some(Phase::Assoc { bss, old, rands, .. }) = self.wlan.stas.get(&dev).map(|s| s.phase.clone()) else { return };
        // the AP must still offer the network on a free radio
        let still = self.ap_nets(bss.ap).and_then(|n| n.get(bss.net)).is_some_and(|n| n.ssid == bss.ssid && n.band == bss.band) && self.node_index(bss.ap).is_some_and(|ani| self.wlan_radio_up(ani, bss.port));
        if !still {
            if let Some(s) = self.wlan.stas.get_mut(&dev) {
                s.phase = Phase::Idle;
            }
            self.sta_start(dev);
            return;
        }
        if let Some(o) = old {
            let to = self.name(bss.ap);
            self.assoc_hand_over(o, to);
        }
        let id = self.assoc_make(dev, &bss, old.is_some(), rands);
        if let Some(s) = self.wlan.stas.get_mut(&dev) {
            s.phase = Phase::Joined { assoc: id };
        }
        self.sta_arm(dev);
    }

    /// a failure: wait and try again, or give up after GIVE_UP of them
    fn sta_failed(&mut self, dev: u32) {
        let Some(s) = self.wlan.stas.get_mut(&dev) else { return };
        s.failures = s.failures.saturating_add(1);
        let n = s.failures;
        if n >= GIVE_UP {
            s.phase = Phase::Waiting(Wait::GaveUp);
            self.wnote(dev, None, WNote::GiveUp { n });
        } else {
            let secs = BACKOFF_S[(n as usize - 1).min(BACKOFF_S.len() - 1)];
            s.phase = Phase::Backoff { until: self.now + secs as Ns * SEC };
            self.wnote(dev, None, WNote::Backoff { secs, n });
        }
        self.sta_arm(dev);
        self.wlan.bump();
    }

    /// a connected station looks for a better AP of its network: below -70 dBm, 8 dB better
    fn sta_roam_check(&mut self, dev: u32) {
        let Some(Phase::Joined { assoc }) = self.wlan.stas.get(&dev).map(|s| s.phase.clone()) else { return };
        let Some(a) = self.wlan.assocs.get(&assoc) else { return };
        if !a.sta_ok || a.lost.is_some() || a.rssi10 >= ROAM_BELOW10 {
            return;
        }
        let (cur, ssid, ap, port) = (a.rssi10, a.ssid.clone(), a.ap, a.ap_port);
        let best = self.wlan_survey(dev, Some(&ssid)).into_iter().filter(|b| !(b.ap == ap && b.port == port)).max_by_key(|b| (b.rssi10, b.band.index()));
        if let Some(b) = best
            && b.rssi10 >= cur + ROAM_GAIN10
        {
            let (from, to) = (self.name(ap), self.name(b.ap));
            self.wnote(dev, None, WNote::Roam { from, to, old: dbm(cur), new: b.dbm() });
            self.sta_auth(dev, b, Some(assoc));
        }
    }

    fn port_name_of(&self, dev: u32) -> String {
        match (self.node(dev), self.wlan.stas.get(&dev)) {
            (Some(n), Some(s)) => String::from(n.port_name(s.port)),
            _ => String::from("wlan0"),
        }
    }

    // ---------------------------------------------------------------- associations

    /// a new association: the radio link (not yet carrying data), the AP's notes and its
    /// Layer 2 Update; an open network is ready at once, else the AP starts the handshake
    fn assoc_make(&mut self, sta: u32, bss: &Bss, reassoc: bool, rands: ([u8; 32], [u8; 32])) -> u32 {
        let (Some(sni), Some(ani)) = (self.node_index(sta), self.node_index(bss.ap)) else { return 0 };
        let sport = self.wlan.stas.get(&sta).map(|s| s.port).unwrap_or(0);
        let sta_mac = self.nodes[sni].as_ref().map(|n| n.ports[sport as usize].mac).unwrap_or(Mac::ZERO);
        let id = ASSOC_ID0 + self.wlan.next;
        self.wlan.next += 1;
        let net = self.ap_nets(bss.ap).and_then(|n| n.get(bss.net)).cloned().unwrap_or_default();
        let native = self.native_vlan(ani, bss.port);
        // VLANs on plain APs; a routing device keeps all its stations in its LAN
        let routes = self.nodes[ani].as_ref().is_some_and(|n| n.model.has(F_ROUTER));
        let vlan = if (1..=4094).contains(&net.vlan) && !routes { net.vlan } else { native };
        // the lowest free AID of the radio
        let mut aid = 1u16;
        while self.wlan.assocs.values().any(|a| a.ap_ni == ani && a.ap_port == bss.port && a.aid == aid) {
            aid += 1;
        }
        let pass_sta = self.wlan.stas.get(&sta).and_then(|s| s.join.as_ref()).map(|j| j.passphrase.clone()).unwrap_or_default();
        let (pmk_ap, pmk_sta) = match net.security {
            Security::Open => ([0; 32], [0; 32]),
            Security::Wpa2 => (self.pmk_for(&net.passphrase, &net.ssid), self.pmk_for(&pass_sta, &net.ssid)),
            Security::Wpa3 => {
                let a = wlankey::pmk_sae(&net.passphrase, bss.bssid, sta_mac, &rands.0, &rands.1);
                let s = wlankey::pmk_sae(&pass_sta, bss.bssid, sta_mac, &rands.0, &rands.1);
                (a, s)
            }
        };
        let rt = LinkRt {
            id,
            ends: [(sni, sport), (ani, bss.port)],
            cable: CableKind::Wireless,
            kbps: 1,
            delay_ns: 0,
            jitter_ns: 0,
            loss_ppm: 0,
            // a roam: the station's port stays up (the AP drops its data until the handshake is
            // done, as an 802.1X port does)
            down: !reassoc,
            wrong_cable: false,
            epoch: 0,
            was_up: reassoc,
            dir: [Dir::default(), Dir::default()],
            rng: Pcg32::derive(self.seed ^ 0x5241_4449, id as u64),
        };
        self.links.push(Some(rt));
        let li = self.links.len() - 1;
        self.link_idx.insert(id, li);
        if let Some(Some(n)) = self.nodes.get_mut(sni) {
            n.ports[sport as usize].link = Some(li);
        }
        self.wlan.by_link.insert(li, id);
        let open = net.security == Security::Open;
        self.wlan.assocs.insert(
            id,
            Assoc {
                id,
                li,
                sta,
                sta_ni: sni,
                sta_port: sport,
                sta_mac,
                ap: bss.ap,
                ap_ni: ani,
                ap_port: bss.port,
                net: bss.net,
                ssid: bss.ssid.clone(),
                band: bss.band,
                channel: bss.channel,
                bssid: bss.bssid,
                security: net.security,
                vlan,
                aid,
                sta_ok: false,
                ap_ok: open,
                rssi10: bss.rssi10,
                kbps: bss.kbps,
                mcs: 0,
                since: self.now,
                roam: reassoc,
                lost: None,
                ap_hs: None,
                sta_hs: None,
                pmk_ap,
                pmk_sta,
            },
        );
        self.assoc_signal(id, true);
        let (sname, aname) = (self.name(sta), self.name(bss.ap));
        let (dbm_now, kbps) = self.wlan.assocs.get(&id).map(|a| (dbm(a.rssi10), a.kbps)).unwrap_or((0, 0));
        self.wnote(sta, None, WNote::Assoc { ap: aname.clone(), aid, band: bss.band, ch: bss.channel, dbm: dbm_now, kbps, reassoc });
        self.wnote(bss.ap, None, WNote::ApAssoc { sta: sname.clone(), aid, ssid: bss.ssid.clone(), port: bss.port, reassoc });
        let rport = self.nodes[ani].as_ref().map(|n| String::from(n.port_name(bss.port))).unwrap_or_default();
        let text = format!("{}: station {} ({}) {} {} (AID {})", rport, sta_mac.cisco(), sname, if reassoc { "reassociated with" } else { "associated with" }, bss.ssid, aid);
        self.wlan_log(ani, "hostapd", "%DOT11-6-ASSOC", &text);
        // the Layer 2 Update frame: the switches learn where the station is now
        self.wnote(bss.ap, None, WNote::L2Update { sta: sname });
        let mut upd = alloc::vec![0xFFu8; 6];
        upd.extend_from_slice(&sta_mac.0);
        upd.extend_from_slice(&[0x00, 0x06, 0x00, 0x01, 0xAF, 0x81, 0x01, 0x00]);
        eth::pad(&mut upd);
        let f: Frame = Arc::from(upd);
        self.bridge_output(ani, vlan, f, None);
        if open {
            self.assoc_authorize(id);
        } else {
            self.hs_start(id);
        }
        self.wlan.bump();
        id
    }

    /// the station may send: its link carries data now (the IP stack sees the link come up, a
    /// DHCP client of the port without a lease starts at once)
    fn assoc_authorize(&mut self, id: u32) {
        let Some(a) = self.wlan.assocs.get_mut(&id) else { return };
        a.sta_ok = true;
        let (li, sta, sni, sport, roam) = (a.li, a.sta, a.sta_ni, a.sta_port, a.roam);
        if let Some(Some(l)) = self.links.get_mut(li) {
            l.down = false;
        }
        self.refresh_link(li);
        if let Some(s) = self.wlan.stas.get_mut(&sta) {
            s.failures = 0;
        }
        let name = format!("dhcp-client@{}", self.nodes.get(sni).and_then(|n| n.as_ref()).map(|n| n.port_name(sport)).unwrap_or(""));
        if !roam && let Some(svc) = self.os(sni).and_then(|o| o.svc_id(&name)) {
            self.svc_ctl(sni, svc, crate::os::Ctl::Carrier);
        }
        self.wlan.bump();
    }

    /// signal, rate, delay and loss from the positions; out of range: lost (the station's timer
    /// waits for the missing beacons), back in range: found again (`quiet`: a new association,
    /// whose note says it all)
    fn assoc_signal(&mut self, id: u32, quiet: bool) {
        let Some(a) = self.wlan.assocs.get(&id) else { return };
        let (sta, ap, band, li, port, sport) = (a.sta, a.ap, a.band, a.li, a.ap_port, a.sta_port);
        let d = self.wlan_dist_mm(sta, ap);
        let Some(an) = self.node(ap) else { return };
        let range = wlan::range_m(an.model, band);
        let amax = an.ports.get(port as usize).map_or(0, |p| p.spec.kbps);
        let ap_on = an.power;
        let smax = self.node(sta).and_then(|n| n.ports.get(sport as usize)).map_or(0, |p| p.spec.kbps);
        let inr = d <= range as u64 * 1000 && ap_on;
        let rssi10 = signal10(range, d);
        let max = wlan::band_max_kbps(band).min(amax).min(smax.max(1));
        let (mcs, pm) = mcs_of(rssi10).unwrap_or((0, MCS_PM[0]));
        let kbps = (max as u64 * pm as u64 / 1000) as u32;
        let was_lost = a.lost.is_some();
        let old_mcs = a.mcs;
        let lost = if inr { None } else { a.lost.or(Some(self.now)) };
        let changed = (a.rssi10, a.kbps, a.lost.is_some()) != (rssi10, kbps, lost.is_some());
        if let Some(a) = self.wlan.assocs.get_mut(&id) {
            a.rssi10 = rssi10;
            a.kbps = kbps;
            a.mcs = mcs;
            a.lost = lost;
        }
        if changed {
            self.wlan.bump();
        }
        if let Some(Some(l)) = self.links.get_mut(li) {
            l.kbps = (kbps / 3 * 2).max(1);
            l.delay_ns = air_ns(band) + d / 300;
            l.loss_ppm = if lost.is_some() { 1_000_000 } else { edge_loss_ppm(rssi10) };
        }
        if quiet {
            return;
        }
        let name = self.name(ap);
        match (was_lost, lost.is_some()) {
            (false, true) => {
                self.wnote(sta, None, WNote::OutOfRange { ap: name });
                self.sta_arm(sta);
            }
            (true, false) => self.wnote(sta, None, WNote::BackInRange { ap: name, dbm: dbm(rssi10) }),
            (false, false) if mcs != old_mcs => self.wnote(sta, None, WNote::Rate { ap: name, dbm: dbm(rssi10), kbps }),
            _ => {}
        }
    }

    /// the AP fell silent (switched off): the station will miss its beacons
    fn assoc_lost(&mut self, id: u32, ap_forgets: bool) {
        let Some(a) = self.wlan.assocs.get_mut(&id) else { return };
        if a.lost.is_none() {
            a.lost = Some(self.now);
        }
        if ap_forgets {
            a.ap_ok = false;
            a.ap_hs = None;
        }
        let (li, sta) = (a.li, a.sta);
        if let Some(Some(l)) = self.links.get_mut(li) {
            l.loss_ppm = 1_000_000;
        }
        self.sta_arm(sta);
    }

    /// a roam: the old access point lets the station go (its link ends at the AP's side only:
    /// what was in the air is lost); the station's port keeps its carrier for the association
    /// that follows (assoc_make), so its stack keeps addresses, neighbours and leases
    fn assoc_hand_over(&mut self, id: u32, to: String) {
        let Some(a) = self.wlan.assocs.get(&id).cloned() else { return };
        if let Some(Some(n)) = self.nodes.get_mut(a.sta_ni) {
            n.ports[a.sta_port as usize].link = None;
        }
        if let Some(Some(l)) = self.links.get_mut(a.li) {
            l.ends[0] = l.ends[1];
            l.down = true;
        }
        self.refresh_link(a.li);
        self.wlan.assocs.remove(&id);
        self.wlan.by_link.remove(&a.li);
        let _ = self.remove_link(id);
        if self.node(a.ap).is_some_and(|n| n.power) {
            let sname = self.name(a.sta);
            self.wnote(a.ap, None, WNote::RoamedAway { sta: sname.clone(), to });
            let rport = self.nodes[a.ap_ni].as_ref().map(|n| String::from(n.port_name(a.ap_port))).unwrap_or_default();
            let text = format!("{}: station {} ({}) left {}", rport, a.sta_mac.cisco(), sname, a.ssid);
            self.wlan_log(a.ap_ni, "hostapd", "%DOT11-6-DISASSOC", &text);
        }
        self.ap_arm(a.ap_ni);
        self.wlan.bump();
    }

    /// an association ends: the link goes (the station's stack sees it go down), the notes of
    /// both sides, the station's next step
    fn assoc_drop(&mut self, id: u32, why: Why) {
        let Some(a) = self.wlan.assocs.get(&id).cloned() else { return };
        if let Some(Some(l)) = self.links.get_mut(a.li) {
            l.down = true;
        }
        self.refresh_link(a.li);
        self.wlan.assocs.remove(&id);
        self.wlan.by_link.remove(&a.li);
        let _ = self.remove_link(id);
        let (sname, aname) = (self.name(a.sta), self.name(a.ap));
        let ap_on = self.node(a.ap).is_some_and(|n| n.power);
        match &why {
            Why::StaLeaving => {
                self.wnote(a.sta, None, WNote::Left { ap: aname });
                if ap_on {
                    self.wnote(a.ap, None, WNote::StaLeft { sta: sname.clone(), reason: 3 });
                }
            }
            Why::ApLeaving => {
                self.wnote(a.sta, None, WNote::Deauthed { ap: aname, reason: 3 });
                self.wnote(a.ap, None, WNote::StaLeft { sta: sname.clone(), reason: 3 });
            }
            Why::HsTimeout => {
                self.wnote(a.ap, None, WNote::HsTimeout { sta: sname.clone() });
                self.wnote(a.sta, None, WNote::Deauthed { ap: aname, reason: 15 });
                let text = format!("{}: {} ended the 4-way handshake of {}: the passphrase may be wrong", self.port_name_of(a.sta), a.bssid, a.ssid);
                self.wlan_log(a.sta_ni, "wpa_supplicant", "%DOT11-4-AUTH", &text);
                let rport = self.nodes[a.ap_ni].as_ref().map(|n| String::from(n.port_name(a.ap_port))).unwrap_or_default();
                let text = format!("{}: station {} ({}) did not finish the 4-way handshake of {}, it is sent away", rport, a.sta_mac.cisco(), sname, a.ssid);
                self.wlan_log(a.ap_ni, "hostapd", "%DOT11-4-MAXRETRIES", &text);
            }
            Why::BeaconLoss => {
                self.wnote(a.sta, None, WNote::BeaconLoss { ap: aname });
                if ap_on {
                    self.wnote(a.ap, None, WNote::StaLeft { sta: sname.clone(), reason: 4 });
                }
            }
        }
        if !matches!(why, Why::HsTimeout) && ap_on {
            let rport = self.nodes[a.ap_ni].as_ref().map(|n| String::from(n.port_name(a.ap_port))).unwrap_or_default();
            let text = format!("{}: station {} ({}) left {}", rport, a.sta_mac.cisco(), sname, a.ssid);
            self.wlan_log(a.ap_ni, "hostapd", "%DOT11-6-DISASSOC", &text);
        }
        self.ap_arm(a.ap_ni);
        // the station's next step
        let joined = self.wlan.stas.get(&a.sta).is_some_and(|s| s.phase == Phase::Joined { assoc: id });
        if joined {
            match why {
                Why::HsTimeout => self.sta_failed(a.sta),
                Why::StaLeaving => {}
                Why::ApLeaving | Why::BeaconLoss => {
                    if let Some(s) = self.wlan.stas.get_mut(&a.sta) {
                        s.phase = Phase::Idle;
                    }
                    self.sta_start(a.sta);
                }
            }
        }
        self.sta_arm(a.sta);
        self.wlan.bump();
    }

    // ---------------------------------------------------------------- the 4-way handshake

    /// the AP sends message 1
    fn hs_start(&mut self, id: u32) {
        let Some(a) = self.wlan.assocs.get(&id) else { return };
        let ap = a.ap;
        let mut anonce = [0u8; 32];
        if let Some(d) = self.wlan.devs.get_mut(&ap) {
            d.rng.fill(&mut anonce);
        }
        let due = self.now + M_FIRST;
        if let Some(a) = self.wlan.assocs.get_mut(&id) {
            a.ap_hs = Some(ApHs { anonce, replay: 1, want: 2, tries: 1, due, ptk: None });
        }
        self.hs_send_m1(id);
        let ani = self.wlan.assocs[&id].ap_ni;
        self.ap_arm(ani);
    }

    fn hs_send_m1(&mut self, id: u32) {
        let Some(a) = self.wlan.assocs.get(&id) else { return };
        let Some(hs) = a.ap_hs.as_ref() else { return };
        let sae = a.security == Security::Wpa3;
        let pdu = wlankey::seal(&wlankey::m1(sae, hs.replay, &hs.anonce), None, sae, 2);
        let f = wlankey::frame(a.sta_mac, a.bssid, &pdu);
        let (li, ap, sta) = (a.li, a.ap, self.name(a.sta));
        self.wnote(ap, None, WNote::Eapol { msg: 1, out: true, peer: sta });
        self.send_on(li, 1, Arc::from(f), None);
    }

    fn hs_send_m3(&mut self, id: u32, cause: Option<HopId>) {
        let Some(a) = self.wlan.assocs.get(&id) else { return };
        let Some(hs) = a.ap_hs.as_ref() else { return };
        let Some(ptk) = hs.ptk.clone() else { return };
        let sae = a.security == Security::Wpa3;
        let (ap, net, replay, anonce) = (a.ap, a.net, hs.replay, hs.anonce);
        let (gtk, igtk) = self.group_keys(ap, net);
        let Some(a) = self.wlan.assocs.get(&id) else { return };
        let wrapped = wlankey::wrap(&ptk.kek, &wlankey::key_data(sae, &gtk, &igtk));
        let pdu = wlankey::seal(&wlankey::m3(sae, replay, &anonce, wrapped), Some(&ptk), sae, 2);
        let f = wlankey::frame(a.sta_mac, a.bssid, &pdu);
        let (li, sta) = (a.li, self.name(a.sta));
        self.wnote(ap, cause, WNote::Eapol { msg: 3, out: true, peer: sta });
        self.send_on(li, 1, Arc::from(f), cause);
    }

    /// the group keys of an AP's network (made at its first station)
    fn group_keys(&mut self, ap: u32, net: usize) -> ([u8; 16], [u8; 16]) {
        let Some(d) = self.wlan.devs.get_mut(&ap) else { return ([0; 16], [0; 16]) };
        if let Some(k) = d.gtk.get(&net) {
            return *k;
        }
        let (mut g, mut i) = ([0u8; 16], [0u8; 16]);
        d.rng.fill(&mut g);
        d.rng.fill(&mut i);
        d.gtk.insert(net, (g, i));
        (g, i)
    }

    /// arms an AP's timer for its next handshake message
    fn ap_arm(&mut self, ni: usize) {
        let due = self.wlan.assocs.values().filter(|a| a.ap_ni == ni).filter_map(|a| a.ap_hs.as_ref().map(|h| h.due)).min();
        match due {
            Some(t) => self.set_timer(ni, SLOT_WLAN_AP, t),
            None => self.cancel_timer(ni, SLOT_WLAN_AP),
        }
    }

    /// an AP's timer: messages that got no answer come again, or the handshake ends
    fn ap_timer(&mut self, ni: usize) {
        let now = self.now;
        let due: Vec<u32> = self.wlan.assocs.values().filter(|a| a.ap_ni == ni && a.ap_hs.as_ref().is_some_and(|h| h.due <= now)).map(|a| a.id).collect();
        for id in due {
            let Some(hs) = self.wlan.assocs.get(&id).and_then(|a| a.ap_hs.clone()) else { continue };
            if hs.tries >= M_TRIES {
                self.assoc_drop(id, Why::HsTimeout);
                continue;
            }
            let n = hs.tries + 1;
            if let Some(h) = self.wlan.assocs.get_mut(&id).and_then(|a| a.ap_hs.as_mut()) {
                h.tries = n;
                h.replay += 1;
                h.due = now + M_NEXT;
            }
            let (ap, sta) = (self.wlan.assocs[&id].ap, self.name(self.wlan.assocs[&id].sta));
            let m = if hs.want == 2 { 1 } else { 3 };
            self.wnote(ap, None, WNote::HsRetry { msg: m, sta, n });
            if m == 1 {
                self.hs_send_m1(id);
            } else {
                self.hs_send_m3(id, None);
            }
        }
        self.ap_arm(ni);
    }

    /// an EAPOL frame reached the AP
    fn eapol_at_ap(&mut self, id: u32, frame: &[u8], hop: HopId) {
        let Some(a) = self.wlan.assocs.get(&id) else { return };
        let Some(pdu) = frame.get(14..).and_then(EapolView::parse).map(|v| v.pdu().to_vec()) else { return };
        let Some(k) = EapolView::parse(&pdu).and_then(|v| Key::parse(v.body())) else { return };
        let (ap, sta_name, sae) = (a.ap, self.name(a.sta), a.security == Security::Wpa3);
        let msg = k.message().unwrap_or(0);
        self.wnote(ap, Some(hop), WNote::Eapol { msg, out: false, peer: sta_name.clone() });
        let Some(a) = self.wlan.assocs.get(&id) else { return };
        let Some(hs) = a.ap_hs.clone() else { return };
        match (hs.want, msg) {
            (2, 2) if k.replay == hs.replay => {
                let ptk = Ptk::derive(sae, &a.pmk_ap, a.bssid, a.sta_mac, &hs.anonce, &k.nonce);
                if !wlankey::mic_ok(&pdu, &ptk, sae) {
                    self.wnote(ap, Some(hop), WNote::MicBad { msg: 2, sta: sta_name });
                    return;
                }
                if let Some(h) = self.wlan.assocs.get_mut(&id).and_then(|a| a.ap_hs.as_mut()) {
                    h.ptk = Some(ptk);
                    h.replay += 1;
                    h.want = 4;
                    h.tries = 1;
                    h.due = self.now + M_NEXT;
                }
                self.hs_send_m3(id, Some(hop));
                let ani = self.wlan.assocs[&id].ap_ni;
                self.ap_arm(ani);
            }
            (4, 4) if k.replay == hs.replay => {
                let ok = hs.ptk.as_ref().is_some_and(|p| wlankey::mic_ok(&pdu, p, sae));
                if !ok {
                    self.wnote(ap, Some(hop), WNote::MicBad { msg: 4, sta: sta_name });
                    return;
                }
                if let Some(a) = self.wlan.assocs.get_mut(&id) {
                    a.ap_hs = None;
                    a.ap_ok = true;
                }
                self.wnote(ap, Some(hop), WNote::Authorized { peer: sta_name });
                let ani = self.wlan.assocs[&id].ap_ni;
                self.ap_arm(ani);
                self.wlan.bump();
            }
            _ => {}
        }
    }

    /// an EAPOL frame reached the station
    fn eapol_at_sta(&mut self, id: u32, frame: &[u8], hop: HopId) {
        let Some(a) = self.wlan.assocs.get(&id) else { return };
        let Some(pdu) = frame.get(14..).and_then(EapolView::parse).map(|v| v.pdu().to_vec()) else { return };
        let Some(k) = EapolView::parse(&pdu).and_then(|v| Key::parse(v.body())) else { return };
        let (sta, ap_name, sae) = (a.sta, self.name(a.ap), a.security == Security::Wpa3);
        let had = a.sta_hs.clone();
        let msg = k.message().unwrap_or(0);
        self.wnote(sta, Some(hop), WNote::Eapol { msg, out: false, peer: ap_name.clone() });
        match msg {
            1 => {
                // a repeated message 1 with the same ANonce gets the same SNonce
                let reuse = had.as_ref().filter(|h| h.anonce == k.nonce).map(|h| h.snonce);
                let snonce = match reuse {
                    Some(s) => s,
                    None => {
                        let mut s = [0u8; 32];
                        if let Some(d) = self.wlan.devs.get_mut(&sta) {
                            d.rng.fill(&mut s);
                        }
                        s
                    }
                };
                let Some(a) = self.wlan.assocs.get(&id) else { return };
                let ptk = Ptk::derive(sae, &a.pmk_sta, a.bssid, a.sta_mac, &k.nonce, &snonce);
                let pdu = wlankey::seal(&wlankey::m2(sae, k.replay, &snonce), Some(&ptk), sae, 1);
                let f = wlankey::frame(a.bssid, a.sta_mac, &pdu);
                let li = a.li;
                if let Some(a) = self.wlan.assocs.get_mut(&id) {
                    a.sta_hs = Some(StaHs { anonce: k.nonce, snonce, ptk });
                }
                self.wnote(sta, Some(hop), WNote::Eapol { msg: 2, out: true, peer: ap_name });
                self.send_on(li, 0, Arc::from(f), Some(hop));
            }
            3 => {
                let Some(hs) = had else { return };
                if hs.anonce != k.nonce || !wlankey::mic_ok(&pdu, &hs.ptk, sae) || wlankey::unwrap_gtk(&hs.ptk.kek, &k.data).is_none() {
                    return;
                }
                let Some(a) = self.wlan.assocs.get(&id) else { return };
                let pdu = wlankey::seal(&wlankey::m4(sae, k.replay), Some(&hs.ptk), sae, 1);
                let f = wlankey::frame(a.bssid, a.sta_mac, &pdu);
                let (li, sni, bssid, ssid) = (a.li, a.sta_ni, a.bssid, a.ssid.clone());
                self.wnote(sta, Some(hop), WNote::Eapol { msg: 4, out: true, peer: ap_name.clone() });
                self.send_on(li, 0, Arc::from(f), Some(hop));
                self.wnote(sta, Some(hop), WNote::Authorized { peer: ap_name });
                let text = format!("{}: connected to {} ({}), keys in place", self.port_name_of(sta), bssid, ssid);
                self.wlan_log(sni, "wpa_supplicant", "%DOT11-6-KEYS", &text);
                self.assoc_authorize(id);
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- the links (link.rs)

    /// an association link carries data: the station installed its keys, is powered and its
    /// port is up -- the AP's side does not count (a station hears of a silent AP late)
    pub(crate) fn wlan_link_up(&self, li: usize, l: &LinkRt) -> bool {
        if l.down || !self.wlan.by_link.contains_key(&li) {
            return false;
        }
        let (n, p) = l.ends[0];
        match self.nodes.get(n) {
            Some(Some(node)) => node.power && node.ports.get(p as usize).is_some_and(|x| x.admin_up),
            _ => false,
        }
    }

    /// an association link exists and the station is powered (EAPOL frames travel before the
    /// keys are in place)
    pub(crate) fn wlan_carrier(&self, li: usize) -> bool {
        let Some(&id) = self.wlan.by_link.get(&li) else { return false };
        self.wlan.assocs.get(&id).is_some_and(|a| self.nodes.get(a.sta_ni).and_then(|n| n.as_ref()).is_some_and(|n| n.power))
    }

    /// the AP's end of an association link: its node slot and the station's MAC
    pub(crate) fn wlan_ap_end(&self, li: usize) -> Option<(usize, Mac)> {
        let id = self.wlan.by_link.get(&li)?;
        self.wlan.assocs.get(id).map(|a| (a.ap_ni, a.sta_mac))
    }

    /// a frame arrived on an association link (link.rs, after the counters): EAPOL goes to the
    /// handshake, the AP's side into its bridge; false for the station's data (the normal way)
    pub(crate) fn wlan_rx(&mut self, li: usize, ni: usize, pi: u16, frame: &Frame, hop: HopId) -> bool {
        let Some(&id) = self.wlan.by_link.get(&li) else { return false };
        let Some(a) = self.wlan.assocs.get(&id) else { return false };
        let at_ap = ni == a.ap_ni;
        let powered = self.nodes.get(ni).and_then(|n| n.as_ref()).is_some_and(|n| n.power);
        if !powered {
            self.trace.mark_lost(hop);
            return true;
        }
        let Some(l2) = L2::parse(frame) else { return false };
        if l2.ethertype == ET_EAPOL {
            if at_ap {
                self.eapol_at_ap(id, frame, hop);
            } else {
                self.eapol_at_sta(id, frame, hop);
            }
            return true;
        }
        if !at_ap {
            return false;
        }
        let (ap, ap_ok, vlan) = (a.ap, a.ap_ok, a.vlan);
        if !ap_ok {
            if let Some(Some(n)) = self.nodes.get_mut(ni) {
                n.ports[pi as usize].counters.drops += 1;
            }
            self.wnote(ap, Some(hop), WNote::NotAuthorized { mac: l2.src });
            return true;
        }
        let native = self.native_vlan(ni, pi);
        let f: Frame = if vlan != native && l2.tag.is_none() { Arc::from(eth::tagged(frame, VlanTag::new(vlan))) } else { frame.clone() };
        self.bridge_input_from(ni, pi, f, hop, Some(id));
        true
    }

    /// a frame of the AP's bridge for one of its radio ports: to the station it is for, a
    /// broadcast to every station of its VLAN but its sender; client isolation and guest
    /// networks keep stations apart
    pub(crate) fn wlan_ap_send(&mut self, ni: usize, pi: u16, frame: Frame, cause: Option<HopId>, from: Src) {
        let Some(l2) = L2::parse(&frame) else { return };
        let native = self.native_vlan(ni, pi);
        let vlan = match l2.tag {
            Some(t) if t.vid != 0 => t.vid,
            _ => native,
        };
        let plain: Frame = if l2.tag.is_some() { Arc::from(eth::untagged(&frame)) } else { frame };
        let Some(Some(node)) = self.nodes.get(ni) else { return };
        let dev = node.dev;
        let routes = node.model.has(F_ROUTER);
        let nets: Vec<WlanNet> = self.wlan.devs.get(&dev).and_then(|d| d.cfg.as_ref()).map(|w| w.nets.clone()).unwrap_or_default();
        let flags = |net: usize| nets.get(net).map_or((false, false), |n| (n.isolate, n.guest));
        let src_flags = match from {
            Src::Sta(s) => self.wlan.assocs.get(&s).map(|a| flags(a.net)),
            _ => None,
        };
        let unicast = l2.dst.is_unicast();
        let targets: Vec<(u32, usize, usize)> = self
            .wlan
            .assocs
            .values()
            .filter(|a| a.ap_ni == ni && a.ap_port == pi && a.vlan == vlan && a.ap_ok && a.sta_mac != l2.src && (!unicast || a.sta_mac == l2.dst))
            .map(|a| (a.id, a.li, a.net))
            .collect();
        if unicast && targets.is_empty() {
            if let Some(Some(n)) = self.nodes.get_mut(ni) {
                n.ports[pi as usize].counters.drops += 1;
            }
            self.wnote(dev, cause, WNote::NoStation { mac: l2.dst });
            return;
        }
        for (_, li, net) in targets {
            let (iso, guest) = flags(net);
            let ok = match (from, src_flags) {
                (Src::Own, _) => true,
                (Src::Wired, _) => !(guest && routes),
                (Src::Sta(_), Some((si, sg))) => !(si || sg || iso || guest),
                (Src::Sta(_), None) => true,
            };
            if !ok {
                if unicast {
                    self.wnote(dev, cause, WNote::Isolated { mac: l2.dst, guest: guest || src_flags.is_some_and(|f| f.1) });
                }
                continue;
            }
            self.send_on(li, 1, plain.clone(), cause);
        }
    }

    /// a station of a guest network on a routing device: its frames reach only the device itself
    pub(crate) fn wlan_guest_router(&self, assoc: u32) -> bool {
        let Some(a) = self.wlan.assocs.get(&assoc) else { return false };
        let routes = self.nodes.get(a.ap_ni).and_then(|n| n.as_ref()).is_some_and(|n| n.model.has(F_ROUTER));
        routes && self.ap_nets(a.ap).and_then(|n| n.get(a.net)).is_some_and(|n| n.guest)
    }

    /// problems.rs: the VLAN the AP's end of an association link joins
    pub(crate) fn wlan_end_vlan(&self, li: usize, ni: usize) -> Option<u16> {
        let id = self.wlan.by_link.get(&li)?;
        self.wlan.assocs.get(id).filter(|a| a.ap_ni == ni).map(|a| a.vlan)
    }

    // ---------------------------------------------------------------- views

    fn assoc_view(&self, a: &Assoc) -> AssocView {
        AssocView {
            id: a.id,
            sta: a.sta,
            sta_port: a.sta_port,
            ap: a.ap,
            ap_port: a.ap_port,
            ssid: a.ssid.clone(),
            band: a.band,
            channel: a.channel,
            security: a.security,
            bssid: a.bssid,
            aid: a.aid,
            vlan: a.vlan,
            dbm: dbm(a.rssi10),
            kbps: a.kbps,
            authorized: a.sta_ok && a.ap_ok,
            lost: a.lost.is_some(),
            since: a.since,
        }
    }

    /// every association, by id
    pub fn wlan_assocs(&self) -> Vec<AssocView> {
        self.wlan.assocs.values().map(|a| self.assoc_view(a)).collect()
    }

    /// a station's state (None: no station)
    pub fn wlan_station(&self, dev: u32) -> Option<StaView> {
        let s = self.wlan.stas.get(&dev)?;
        let n = self.node(dev)?;
        let mine = self.wlan.assocs.values().find(|a| a.sta == dev);
        let fixed = n.ports.get(s.port as usize).is_some_and(|p| p.link.is_some_and(|li| !self.wlan.by_link.contains_key(&li)));
        let state = match &s.phase {
            _ if fixed => StaState::Fixed,
            Phase::Idle => StaState::Idle,
            Phase::Scan { .. } => StaState::Scanning,
            Phase::Auth { old: Some(_), .. } | Phase::Assoc { old: Some(_), .. } => StaState::Roaming,
            Phase::Auth { .. } | Phase::Assoc { .. } => StaState::Joining,
            Phase::Joined { .. } => match mine {
                Some(a) if a.sta_ok && a.ap_ok => StaState::Connected,
                _ => StaState::Handshake,
            },
            Phase::Backoff { .. } => StaState::Backoff,
            Phase::Waiting(Wait::NoNetwork) => StaState::NoNetwork,
            Phase::Waiting(Wait::NoPassphrase) => StaState::NoPassphrase,
            Phase::Waiting(Wait::GaveUp) => StaState::GaveUp,
        };
        let retry_at = match &s.phase {
            Phase::Backoff { until } => Some(*until),
            _ => None,
        };
        Some(StaView { dev, port: s.port, state, join: s.join.clone(), assoc: mine.map(|a| self.assoc_view(a)), failures: s.failures, retry_at })
    }

    /// problems.rs: networks an AP cannot offer, stations that find no network of their SSID
    /// or whose passphrase does not fit it
    pub(crate) fn wlan_problems(&self, out: &mut Vec<crate::Problem>) {
        use crate::problems::{Problem, ProblemKind, Severity};
        let mut push = |kind, severity, dev, port, others: Vec<u32>, m: jfn_model::Msg| out.push(Problem { kind, severity, dev, port, link: None, others, text: m.text(), msg: m });
        // every network that is offered: (AP, net)
        let mut offers: Vec<(u32, &WlanNet)> = Vec::new();
        for &ap in self.wlan.devs.keys() {
            let Some(nets) = self.ap_nets(ap) else { continue };
            for n in nets {
                match wlan::check_net(n) {
                    Some(m) => push(ProblemKind::WlanBadNet, Severity::Error, ap, self.radio_index(ap, n.band), Vec::new(), m),
                    None => offers.push((ap, n)),
                }
            }
        }
        for (&dev, s) in &self.wlan.stas {
            let Some(j) = s.join.as_ref() else { continue };
            let Some(n) = self.node(dev) else { continue };
            if n.ports.get(s.port as usize).is_some_and(|p| p.link.is_some_and(|li| !self.wlan.by_link.contains_key(&li))) {
                continue;
            }
            let name = n.name.clone();
            let mine: Vec<&(u32, &WlanNet)> = offers.iter().filter(|o| o.1.ssid == j.ssid).collect();
            let aps: Vec<u32> = {
                let mut v: Vec<u32> = mine.iter().map(|o| o.0).collect();
                v.dedup();
                v
            };
            if mine.is_empty() {
                push(ProblemKind::WlanNoNetwork, Severity::Error, dev, Some(s.port), Vec::new(), jfn_model::msg!("Kein Access Point bietet das WLAN \u{201e}{}\u{201c} an, das {} sucht.", j.ssid, name));
                continue;
            }
            if self.wlan_survey(dev, Some(&j.ssid)).is_empty() {
                push(ProblemKind::WlanNoNetwork, Severity::Warning, dev, Some(s.port), aps.clone(), jfn_model::msg!("Das WLAN \u{201e}{}\u{201c} ist für {} außer Reichweite.", j.ssid, name));
            }
            let secured: Vec<&&(u32, &WlanNet)> = mine.iter().filter(|o| o.1.security.secured()).collect();
            if secured.is_empty() {
                continue;
            }
            if j.passphrase.is_empty() {
                push(ProblemKind::WlanPassphrase, Severity::Error, dev, Some(s.port), aps, jfn_model::msg!("{} hat keine Passphrase für das WLAN \u{201e}{}\u{201c}.", name, j.ssid));
            } else if !secured.iter().any(|o| o.1.passphrase == j.passphrase) {
                push(ProblemKind::WlanPassphrase, Severity::Error, dev, Some(s.port), aps, jfn_model::msg!("Die Passphrase von {} passt nicht zum WLAN \u{201e}{}\u{201c}.", name, j.ssid));
            }
        }
    }

    /// the stations of an AP (by AID)
    pub fn wlan_stations_of(&self, ap: u32) -> Vec<AssocView> {
        let mut v: Vec<AssocView> = self.wlan.assocs.values().filter(|a| a.ap == ap).map(|a| self.assoc_view(a)).collect();
        v.sort_by_key(|a| (a.ap_port, a.aid));
        v
    }
}

/// why an association ends
#[derive(Clone, Debug, PartialEq, Eq)]
enum Why {
    /// the station leaves (switched off, other settings, disconnect, removed)
    StaLeaving,
    /// the AP ends it (its network changed, its radio is down, it is removed)
    ApLeaving,
    /// the handshake did not come about
    HsTimeout,
    /// the station missed the beacons
    BeaconLoss,
}
