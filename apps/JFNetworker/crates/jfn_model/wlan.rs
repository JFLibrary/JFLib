// wlan -- the WLAN settings of a project and the radio facts behind them: the networks an access
// point offers, the network a station joins, the bands and ranges of the models, channels and
// their frequencies, the fastest rate of a band, the scale between the canvas and metres
//
//   Wlan { nets, join }                  Device::wlan (None: no WLAN settings, as in older files)
//   WlanNet { ssid, band, channel, security, passphrase, vlan, guest, isolate }
//   WlanJoin { ssid, passphrase }        what a station joins
//   Band (2.4 / 5 / 6 GHz), Security (open, WPA2-Personal, WPA3-Personal)
//   role(model) -> Role                  Ap (F_AP with a radio), Station (a radio client), None
//   ap_bands(model) / client_bands(model)   the bands of a model's radios (B_* bits)
//   range_m(model, band)                 how far an access point of the model reaches
//   radio_port(model, band)              the port of the access point that sends on a band
//   channels(band) / freq_mhz(band, ch) / default_channel(band) / channel_of(net)
//   scan_channels(band)                  the channels a station listens on when it scans
//   band_max_kbps(band)                  the fastest rate of a band (802.11ax, two streams)
//   DIP_PER_M / dip_per_m(project)       the scale: world units (DIP at 100 %) per metre
//   check_net(net) / check_join(join) -> Option<Msg>   what is wrong (German, a Msg)
//   new_net(model, nets)                 a network for the inspector: a free SSID, the best band
//   passphrase_ok(p)                     8 to 63 printable ASCII characters
//
// There is no radio physics: a station reaches a network when the access point is within the
// range of the network's band; the simulator turns the distance into a signal and a rate
// (wlan.rs of jfn_sim).  6 GHz takes WPA3-Personal only (as the standard asks); networks on one band
// share the radio of that band and its channel (the first network's channel counts).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::{record, wire_enum};

use crate::catalog::{Media, Model, F_AP};
use crate::msg::Msg;
use crate::msg;
use crate::project::Project;

wire_enum! {
    /// a WLAN band
    pub enum Band {
        G24 = "2.4",
        G5 = "5",
        G6 = "6",
    }
}

wire_enum! {
    /// how a network is protected
    pub enum Security {
        Open = "open",
        Wpa2 = "wpa2-personal",
        Wpa3 = "wpa3-personal",
    }
}

record! {
    /// a network an access point offers
    pub struct WlanNet {
        /// its name (1 to 32 bytes)
        pub ssid: String,
        pub band: Band,
        /// 0: the band's default channel
        pub channel: u16,
        pub security: Security,
        /// WPA2 / WPA3: 8 to 63 printable ASCII characters
        pub passphrase: String,
        /// the VLAN of its stations on the access point's bridge (0: none, the bridge's VLAN 1)
        pub vlan: u16,
        /// a guest network: its stations reach only the device itself (a routing device: the
        /// router, its DHCP and the way out), never the LAN or other stations
        pub guest: bool,
        /// client isolation: its stations do not reach each other (nor other stations)
        pub isolate: bool,
    }
}

record! {
    /// the network a station joins
    pub struct WlanJoin {
        pub ssid: String,
        /// for WPA2 / WPA3 networks (an open network ignores it)
        pub passphrase: String,
    }
}

record! {
    /// the WLAN settings of a device
    pub struct Wlan {
        /// what an access point (or the home router) offers
        pub nets: Vec<WlanNet>,
        /// what a station joins (None: none)
        pub join: Option<WlanJoin>,
    }
}

/// band bits
pub const B_24: u8 = 1;
pub const B_5: u8 = 2;
pub const B_6: u8 = 4;

/// the scale of a project without one: 10 world units (DIP at 100 %) per metre
pub const DIP_PER_M: u32 = 10;

/// the longest SSID (bytes)
pub const SSID_MAX: usize = 32;

impl Band {
    /// its bit (B_*)
    pub fn bit(self) -> u8 {
        match self {
            Band::G24 => B_24,
            Band::G5 => B_5,
            Band::G6 => B_6,
        }
    }
    /// 0, 1, 2
    pub fn index(self) -> usize {
        match self {
            Band::G24 => 0,
            Band::G5 => 1,
            Band::G6 => 2,
        }
    }
    /// "2,4 GHz", "5 GHz", "6 GHz" (the app translates it)
    pub fn title(self) -> &'static str {
        // i18n
        match self {
            Band::G24 => "2,4 GHz",
            Band::G5 => "5 GHz",
            Band::G6 => "6 GHz",
        }
    }
}

impl Security {
    /// "Offen", "WPA2-Personal", "WPA3-Personal" (the app translates it)
    pub fn title(self) -> &'static str {
        // i18n
        match self {
            Security::Open => "Offen",
            Security::Wpa2 => "WPA2-Personal",
            Security::Wpa3 => "WPA3-Personal",
        }
    }
    /// it needs a passphrase
    pub fn secured(self) -> bool {
        self != Security::Open
    }
}

/// what a model does with its radios
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// offers networks (access point, home router)
    Ap,
    /// joins one (laptop, phone, ...)
    Station,
    None,
}

/// the role of a model's radios
pub fn role(m: &Model) -> Role {
    let has = |media: Media| m.groups.iter().any(|g| g.media == media);
    if m.has(F_AP) && has(Media::RadioAp) {
        Role::Ap
    } else if has(Media::RadioClient) {
        Role::Station
    } else {
        Role::None
    }
}

/// the bands an access point of the model sends on (B_* bits)
pub fn ap_bands(m: &Model) -> u8 {
    match m.id {
        "jf-ap6e" => B_24 | B_5 | B_6,
        _ if role(m) == Role::Ap => B_24 | B_5,
        _ => 0,
    }
}

/// the bands a station of the model can use (B_* bits)
pub fn client_bands(m: &Model) -> u8 {
    match m.id {
        "laptop" | "smartphone" | "tablet" => B_24 | B_5 | B_6,
        "iot-sensor" => B_24,
        _ if role(m) == Role::Station => B_24 | B_5,
        _ => 0,
    }
}

/// how far an access point of the model reaches on a band (metres, indoors)
pub fn range_m(m: &Model, band: Band) -> u32 {
    let (a, b, c) = match m.id {
        "jf-ap6e" => (50, 35, 25),
        "home-router" => (40, 30, 20),
        _ => (40, 30, 20),
    };
    match band {
        Band::G24 => a,
        Band::G5 => b,
        Band::G6 => c,
    }
}

/// the radio port of an access point that sends on a band: its radios in model order, one per
/// band (2.4, 5, 6 GHz); a model with fewer radios sends the other bands on its last one
pub fn radio_port(m: &Model, band: Band) -> Option<String> {
    let radios: Vec<String> = m.ports().into_iter().filter(|p| p.media == Media::RadioAp).map(|p| p.name).collect();
    if radios.is_empty() {
        return None;
    }
    let i = band.index().min(radios.len() - 1);
    radios.into_iter().nth(i)
}

/// the radio port of a station (its first radio client port)
pub fn station_port(m: &Model) -> Option<String> {
    m.ports().into_iter().find(|p| p.media == Media::RadioClient).map(|p| p.name)
}

const CH5: [u16; 19] = [36, 40, 44, 48, 52, 56, 60, 64, 100, 104, 108, 112, 116, 120, 124, 128, 132, 136, 140];
/// the preferred scanning channels of 6 GHz below 6425 MHz
const PSC6: [u16; 6] = [5, 21, 37, 53, 69, 85];

/// the channels of a band (Europe: 2.4 GHz 1 to 13, 5 GHz 36 to 64 and 100 to 140, 6 GHz the
/// 20-MHz channels 1 to 93)
pub fn channels(band: Band) -> Vec<u16> {
    match band {
        Band::G24 => (1..=13).collect(),
        Band::G5 => CH5.to_vec(),
        Band::G6 => (0..24).map(|i| 1 + 4 * i).collect(),
    }
}

/// the channels a station listens on in a scan (6 GHz: the preferred scanning channels only)
pub fn scan_channels(band: Band) -> usize {
    match band {
        Band::G24 => 13,
        Band::G5 => CH5.len(),
        Band::G6 => PSC6.len(),
    }
}

/// the default channel of a band
pub fn default_channel(band: Band) -> u16 {
    match band {
        Band::G24 => 1,
        Band::G5 => 36,
        Band::G6 => 37,
    }
}

/// the centre frequency of a channel (MHz)
pub fn freq_mhz(band: Band, ch: u16) -> u32 {
    match band {
        Band::G24 if ch == 14 => 2484,
        Band::G24 => 2407 + 5 * ch as u32,
        Band::G5 => 5000 + 5 * ch as u32,
        Band::G6 => 5950 + 5 * ch as u32,
    }
}

/// the channel a network uses (its own, else the band's default)
pub fn channel_of(n: &WlanNet) -> u16 {
    if n.channel == 0 { default_channel(n.band) } else { n.channel }
}

/// the fastest rate of a band (kbit/s): 802.11ax with two spatial streams, 40 MHz at 2.4 GHz,
/// 160 MHz at 5 and 6 GHz
pub fn band_max_kbps(band: Band) -> u32 {
    match band {
        Band::G24 => 573_500,
        Band::G5 | Band::G6 => 2_401_900,
    }
}

/// the scale of a project: world units per metre
pub fn dip_per_m(p: &Project) -> u32 {
    if p.dip_per_m == 0 { DIP_PER_M } else { p.dip_per_m }
}

/// a passphrase WPA2 and WPA3 take here: 8 to 63 printable ASCII characters
pub fn passphrase_ok(p: &str) -> bool {
    (8..=63).contains(&p.len()) && p.bytes().all(|b| (0x20..0x7F).contains(&b))
}

/// what is wrong with a network an access point offers (None: nothing)
pub fn check_net(n: &WlanNet) -> Option<Msg> {
    if n.ssid.is_empty() || n.ssid.len() > SSID_MAX {
        return Some(msg!("Eine SSID hat 1 bis 32 Bytes."));
    }
    if n.channel != 0 && !channels(n.band).contains(&n.channel) {
        return Some(msg!("Kanal {} gibt es im Band {} nicht.", n.channel, crate::msg::phrase(n.band.title())));
    }
    if n.band == Band::G6 && n.security != Security::Wpa3 {
        return Some(msg!("Im 6-GHz-Band gibt es nur WPA3-Personal."));
    }
    if n.security.secured() && !passphrase_ok(&n.passphrase) {
        return Some(msg!("Die Passphrase von \u{201e}{}\u{201c} braucht 8 bis 63 Zeichen (ASCII).", n.ssid));
    }
    if n.vlan > 4094 {
        return Some(msg!("VLAN {} gibt es nicht (1 bis 4094).", n.vlan));
    }
    None
}

/// what is wrong with what a station joins (None: nothing)
pub fn check_join(j: &WlanJoin) -> Option<Msg> {
    if j.ssid.is_empty() || j.ssid.len() > SSID_MAX {
        return Some(msg!("Eine SSID hat 1 bis 32 Bytes."));
    }
    if !j.passphrase.is_empty() && !passphrase_ok(&j.passphrase) {
        return Some(msg!("Die Passphrase von \u{201e}{}\u{201c} braucht 8 bis 63 Zeichen (ASCII).", j.ssid));
    }
    None
}

/// a new network for an access point that has `nets`: SSID "WLAN-N" with the lowest free N, the
/// fastest band the model has (5 GHz before 6 GHz, which needs WPA3), open
pub fn new_net(m: &Model, nets: &[WlanNet]) -> WlanNet {
    let mut k = 1;
    let ssid = loop {
        let s = format!("WLAN-{}", k);
        if !nets.iter().any(|n| n.ssid == s) {
            break s;
        }
        k += 1;
    };
    let bands = ap_bands(m);
    let band = if bands & B_5 != 0 { Band::G5 } else { Band::G24 };
    WlanNet { ssid, band, channel: 0, security: Security::Open, ..WlanNet::default() }
}

impl Wlan {
    /// nothing set (the project can drop it)
    pub fn is_empty(&self) -> bool {
        self.nets.is_empty() && self.join.is_none()
    }
}
