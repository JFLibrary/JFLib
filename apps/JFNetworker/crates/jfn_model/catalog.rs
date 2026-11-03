// catalog -- the devices JFNetworker offers: models with their ports, command line and role
//
//   catalog() -> &[Model]         every model in palette order
//   model(id) -> Option<&Model>   by its stable id ("jf-s24p"), stored in project files
//   default_model(kind)           the first model of a kind
//   Model::ports() / port(name)   the expanded ports ("GigabitEthernet1/0/1" ...); port() also
//                                 takes IOS abbreviations ("gi1/0/1")
//   Model::summary()              "24× 1G PoE+, 4× 10G SFP+, Konsole" (summary_msg: as a Msg)
//   speed_str(kbps)               "1G", "2.5G", "100M", "2 Mbit/s" (speed_msg: as a Msg)
//   Optic / form(m) / rack_units(m)   the transceiver of an optics cage (PortSpec::optic), how
//                                 a model stands in the physical view, its height in a rack (P1)
//   Prices (ASSUMED, of_hw, of_device)   what devices cost per hour, assumptions for an experiment
//   Hw / DiskClass / hw_of(id) / hw_presets()   the hardware of a device for the simulator's
//                                 res.rs (SA1): cores, speed, RAM, swap, disk; a table beside the
//                                 models
//
// The German names the app translates (its catalogue src/locales/engine.tsv): Category::title,
// Media::name, a model's name and title (the tables marked "i18n" for tools/i18n_check.py).
//
// The models are vendor neutral but modern: multi-gigabit copper, SFP+/SFP28/QSFP28 optics,
// PoE, WLAN 802.11ax and 802.11be with 6 GHz.  Network devices speak the IOS-like command line and use IOS interface names;
// hosts have a Linux-like shell and Linux names (eth0, wlan0).  `features` (F_*) tells the
// simulator which parts a device has (bridge, router, host OS, ...).  A firewall today is a router
// with access lists and NAT, its title says so; zones and stateful filtering come with milestone F2.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::wire_enum;

use crate::ifname;
use crate::msg::{list, phrase, Msg};
use crate::msg;

wire_enum! {
    /// what a device is (its model's kind)
    pub enum DeviceKind {
        Pc = "pc",
        Laptop = "laptop",
        Smartphone = "smartphone",
        Tablet = "tablet",
        Printer = "printer",
        IpPhone = "ip-phone",
        IotSensor = "iot-sensor",
        IotCamera = "iot-camera",
        SmartTv = "smart-tv",
        Server = "server",
        Switch = "switch",
        L3Switch = "l3-switch",
        Router = "router",
        HomeRouter = "home-router",
        Hub = "hub",
        Firewall = "firewall",
        AccessPoint = "access-point",
        Modem = "modem",
        Cloud = "cloud",
        LoadBalancer = "load-balancer",
        Database = "database",
        Cache = "cache",
        Queue = "queue",
        TrafficGen = "traffic-gen",
        ContainerHost = "container-host",
        Note = "note",
    }
}

/// the palette group of a model
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    EndDevices,
    Network,
    Security,
    WirelessWan,
    Servers,
    Distributed,
    Annotations,
}

impl Category {
    pub const ALL: [Category; 7] = [
        Category::EndDevices,
        Category::Network,
        Category::Security,
        Category::WirelessWan,
        Category::Servers,
        Category::Distributed,
        Category::Annotations,
    ];
    /// the German title (the app translates it)
    pub fn title(self) -> &'static str {
        // i18n
        match self {
            Category::EndDevices => "Endgeräte",
            Category::Network => "Netzwerk",
            Category::Security => "Sicherheit",
            Category::WirelessWan => "WLAN und WAN",
            Category::Servers => "Server",
            Category::Distributed => "Verteilte Systeme",
            Category::Annotations => "Notizen",
        }
    }
}

/// the command line a device offers
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CliStyle {
    /// IOS-like: user / privileged EXEC, configure terminal, show ...
    Ios,
    /// a Linux-like shell: ip, ping, dig, curl ...
    Linux,
    /// none (modem, cloud, note)
    None,
}

/// the physical kind of a port
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Media {
    /// twisted-pair copper (RJ45), 10M .. 10G
    Rj45,
    /// SFP cage, 1G optics
    Sfp,
    /// SFP+ cage, 10G optics (takes SFP too)
    SfpPlus,
    /// SFP28 cage, 25G optics (takes SFP+ and SFP)
    Sfp28,
    /// QSFP28 cage, 100G optics
    Qsfp28,
    /// synchronous serial WAN interface
    Serial,
    /// RJ45 console port of a network device
    Console,
    /// USB port of a host (for a USB console cable)
    Usb,
    /// the radio of an access point
    RadioAp,
    /// the radio of a WLAN client
    RadioClient,
    /// coax / DSL / GPON line of a modem or the provider cloud
    Wan,
    /// telephone line of a DSL modem
    Rj11,
    /// coaxial line of a cable modem or a TV
    Coax,
}

impl Media {
    /// the name of the port kind (German where it is a word: the app translates it)
    pub fn name(self) -> &'static str {
        // i18n
        match self {
            Media::Rj45 => "RJ45",
            Media::Sfp => "SFP",
            Media::SfpPlus => "SFP+",
            Media::Sfp28 => "SFP28",
            Media::Qsfp28 => "QSFP28",
            Media::Serial => "Seriell",
            Media::Console => "Konsole",
            Media::Usb => "USB",
            Media::RadioAp => "Funk (AP)",
            Media::RadioClient => "Funk",
            Media::Wan => "WAN",
            Media::Rj11 => "RJ11",
            Media::Coax => "Koax",
        }
    }
    /// an optics cage (SFP family or QSFP28)
    pub fn is_optical(self) -> bool {
        matches!(self, Media::Sfp | Media::SfpPlus | Media::Sfp28 | Media::Qsfp28)
    }
    /// carries Ethernet frames (copper, optics, radio)
    pub fn is_ethernet(self) -> bool {
        matches!(self, Media::Rj45 | Media::RadioAp | Media::RadioClient) || self.is_optical()
    }
    pub fn is_radio(self) -> bool {
        matches!(self, Media::RadioAp | Media::RadioClient)
    }
}

/// Power over Ethernet role of a port
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Poe {
    None,
    /// supplies power (a PoE+ switch port)
    Pse,
    /// supplies more power (a PoE++ switch port, IEEE 802.3bt)
    PseBt,
    /// is powered through this port (AP, phone, camera)
    Pd,
}

/// runs an operating system with sockets, services and a shell
pub const F_HOST: u32 = 1;
/// a learning bridge between its Ethernet ports (switch)
pub const F_BRIDGE: u32 = 2;
/// forwards IP between interfaces (router, L3 switch)
pub const F_ROUTER: u32 = 4;
/// a firewall model; nothing reads it yet: it routes with access lists and NAT, zones and stateful
/// filtering come with milestone F2
pub const F_FIREWALL: u32 = 8;
/// bridges its radios with its wired port (access point)
pub const F_AP: u32 = 16;
/// repeats every frame to every other port
pub const F_HUB: u32 = 32;
/// the provider network / Internet
pub const F_CLOUD: u32 = 64;
/// a model made for address translation (home router, firewall); nothing reads it: NAT comes from
/// the device's configuration
pub const F_NAT: u32 = 128;
/// not a device (a note on the canvas)
pub const F_ANNOTATION: u32 = 256;

/// ports of one kind with consecutive numbers: names are `prefix` + number, or `prefix`
/// alone for a single unnumbered port
#[derive(Clone, Copy, Debug)]
pub struct PortGroup {
    pub prefix: &'static str,
    pub first: u16,
    pub count: u16,
    pub media: Media,
    pub kbps: u32,
    /// MDI-X: the port crosses the pairs (switch ports); hosts and routers are MDI
    pub mdix: bool,
    pub poe: Poe,
}

const NONUM: u16 = u16::MAX;

const fn grp(prefix: &'static str, first: u16, count: u16, media: Media, kbps: u32, mdix: bool, poe: Poe) -> PortGroup {
    PortGroup { prefix, first, count, media, kbps, mdix, poe }
}

const fn one(name: &'static str, media: Media, kbps: u32, mdix: bool, poe: Poe) -> PortGroup {
    PortGroup { prefix: name, first: NONUM, count: 1, media, kbps, mdix, poe }
}

const fn console() -> PortGroup {
    one("Console", Media::Console, 115, false, Poe::None)
}

pub const K1G: u32 = 1_000_000;
pub const K2G5: u32 = 2_500_000;
pub const K5G: u32 = 5_000_000;
pub const K10G: u32 = 10_000_000;
pub const K25G: u32 = 25_000_000;
pub const K100G: u32 = 100_000_000;

/// one port of a device
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortSpec {
    pub name: String,
    /// position in the model's port list
    pub index: u16,
    pub media: Media,
    pub kbps: u32,
    pub mdix: bool,
    pub poe: Poe,
    /// the transceiver in an optics cage (None: no cage); a device may plug in another one
    /// (Device::optics, P1)
    pub optic: Option<Optic>,
}

/// a device model
#[derive(Clone, Copy, Debug)]
pub struct Model {
    /// stable id in project files ("jf-s24p")
    pub id: &'static str,
    /// shown name ("JF-S24P")
    pub name: &'static str,
    pub kind: DeviceKind,
    pub category: Category,
    /// German short description (the app translates it)
    pub title: &'static str,
    /// the symbol of the device on the canvas (apps/JFNetworker/icons/NAME.svg)
    pub symbol: &'static str,
    pub cli: CliStyle,
    /// F_* bits
    pub features: u32,
    /// the start of new device names ("SW" -> SW1, SW2, ...)
    pub name_prefix: &'static str,
    pub groups: &'static [PortGroup],
}

impl Model {
    pub fn has(&self, f: u32) -> bool {
        self.features & f == f
    }
    /// every port, expanded from the groups
    pub fn ports(&self) -> Vec<PortSpec> {
        let mut v = Vec::new();
        for g in self.groups {
            for i in 0..g.count {
                let name = if g.first == NONUM { String::from(g.prefix) } else { format!("{}{}", g.prefix, g.first + i) };
                let optic = if g.media.is_optical() { Some(Optic::Sr) } else { None };
                v.push(PortSpec { name, index: v.len() as u16, media: g.media, kbps: g.kbps, mdix: g.mdix, poe: g.poe, optic });
            }
        }
        v
    }
    pub fn port_count(&self) -> usize {
        self.groups.iter().map(|g| g.count as usize).sum()
    }
    /// a port by its name, case-insensitive, also as an IOS abbreviation ("gi1/0/1")
    pub fn port(&self, name: &str) -> Option<PortSpec> {
        let ports = self.ports();
        let n = name.trim();
        if let Some(p) = ports.iter().find(|p| p.name.eq_ignore_ascii_case(n)) {
            return Some(p.clone());
        }
        let full = ifname::expand(n)?;
        ports.into_iter().find(|p| p.name.eq_ignore_ascii_case(&full))
    }
    /// "24× 1G PoE+, 4× 10G SFP+, Konsole"
    pub fn summary(&self) -> String {
        self.summary_msg().text()
    }
    /// the ports as a message (summary() is its German text)
    pub fn summary_msg(&self) -> Msg {
        let parts = self.groups.iter().map(|g| match g.media {
            Media::Console => phrase("Konsole"),
            Media::Usb => phrase("USB"),
            Media::RadioAp | Media::RadioClient => msg!("{}× Funk {}", g.count, speed_str(g.kbps)),
            Media::Serial => msg!("{}× Seriell", g.count),
            Media::Wan => msg!("{}× WAN", g.count),
            Media::Rj11 => msg!("{}× RJ11", g.count),
            Media::Coax => msg!("{}× Koax", g.count),
            m => {
                let base = if m.is_optical() { msg!("{}× {} {}", g.count, speed_str(g.kbps), m.name()) } else { msg!("{}× {}", g.count, speed_str(g.kbps)) };
                match g.poe {
                    Poe::Pse => msg!("{} PoE+", base),
                    Poe::PseBt => msg!("{} PoE++", base),
                    Poe::Pd => msg!("{} (PoE-gespeist)", base),
                    Poe::None => base,
                }
            }
        });
        list(parts.collect())
    }
}

/// a link speed: "100M", "1G", "2.5G", "10G", "2 Mbit/s" (below 100 Mbit/s), "115 kbit/s"
pub fn speed_str(kbps: u32) -> String {
    if kbps >= K1G {
        let g = kbps / 100_000; // tenths of a gigabit
        if g % 10 == 0 { format!("{}G", g / 10) } else { format!("{}.{}G", g / 10, g % 10) }
    } else if kbps >= 100_000 {
        format!("{}M", kbps / 1000)
    } else if kbps >= 1000 {
        let m = kbps / 100;
        if m % 10 == 0 { format!("{} Mbit/s", m / 10) } else { format!("{},{} Mbit/s", m / 10, m % 10) }
    } else {
        format!("{} kbit/s", kbps)
    }
}

/// speed_str as a message (its German text is speed_str): "2,5 Mbit/s" takes the language's
/// decimal mark
pub fn speed_msg(kbps: u32) -> Msg {
    if kbps >= K1G {
        let g = kbps / 100_000; // tenths of a gigabit
        Msg::plain(if g % 10 == 0 { format!("{}G", g / 10) } else { format!("{}.{}G", g / 10, g % 10) })
    } else if kbps >= 100_000 {
        Msg::plain(format!("{}M", kbps / 1000))
    } else if kbps >= 1000 {
        let m = kbps / 100;
        if m % 10 == 0 { msg!("{} Mbit/s", m / 10) } else { msg!("{},{} Mbit/s", m / 10, m % 10) }
    } else {
        msg!("{} kbit/s", kbps)
    }
}

use Media::*;
use Poe::{None as NoPoe, Pd, Pse, PseBt};

// i18n: name title
static CATALOG: &[Model] = &[
    // ---- end devices
    Model {
        id: "pc",
        name: "PC",
        kind: DeviceKind::Pc,
        category: Category::EndDevices,
        title: "Desktop-PC (2.5G Ethernet, USB-Konsole)",
        symbol: "pc",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "PC",
        groups: &[grp("eth", 0, 1, Rj45, K2G5, false, NoPoe), one("usb0", Usb, 0, false, NoPoe)],
    },
    Model {
        id: "laptop",
        name: "Laptop",
        kind: DeviceKind::Laptop,
        category: Category::EndDevices,
        title: "Laptop (WLAN 802.11ax, 6 GHz, USB-C-Ethernet 2.5G)",
        symbol: "laptop",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "Laptop",
        groups: &[
            one("wlan0", RadioClient, 2_400_000, false, NoPoe),
            grp("eth", 0, 1, Rj45, K2G5, false, NoPoe),
            one("usb0", Usb, 0, false, NoPoe),
        ],
    },
    Model {
        id: "smartphone",
        name: "Smartphone",
        kind: DeviceKind::Smartphone,
        category: Category::EndDevices,
        title: "Smartphone (WLAN 802.11ax, 6 GHz)",
        symbol: "smartphone",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "Phone",
        groups: &[one("wlan0", RadioClient, 1_200_000, false, NoPoe)],
    },
    Model {
        id: "tablet",
        name: "Tablet",
        kind: DeviceKind::Tablet,
        category: Category::EndDevices,
        title: "Tablet (WLAN 802.11ax, 6 GHz)",
        symbol: "tablet",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "Tablet",
        groups: &[one("wlan0", RadioClient, 1_200_000, false, NoPoe)],
    },
    Model {
        id: "printer",
        name: "Drucker",
        kind: DeviceKind::Printer,
        category: Category::EndDevices,
        title: "Netzwerkdrucker (1G, WLAN)",
        symbol: "printer",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "Printer",
        groups: &[grp("eth", 0, 1, Rj45, K1G, false, NoPoe), one("wlan0", RadioClient, 600_000, false, NoPoe)],
    },
    Model {
        id: "ip-phone",
        name: "IP-Telefon",
        kind: DeviceKind::IpPhone,
        category: Category::EndDevices,
        title: "IP-Telefon (PoE, Port für einen PC)",
        symbol: "ip_phone",
        cli: CliStyle::Linux,
        features: F_HOST | F_BRIDGE,
        name_prefix: "IPPhone",
        groups: &[grp("eth", 0, 1, Rj45, K1G, false, Pd), one("pc0", Rj45, K1G, true, NoPoe)],
    },
    Model {
        id: "iot-sensor",
        name: "IoT-Sensor",
        kind: DeviceKind::IotSensor,
        category: Category::EndDevices,
        title: "IoT-Sensor (WLAN 2,4 GHz)",
        symbol: "iot_sensor",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "Sensor",
        groups: &[one("wlan0", RadioClient, 150_000, false, NoPoe)],
    },
    Model {
        id: "iot-camera",
        name: "IP-Kamera",
        kind: DeviceKind::IotCamera,
        category: Category::EndDevices,
        title: "IP-Kamera (1G, PoE)",
        symbol: "iot_camera",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "Cam",
        groups: &[grp("eth", 0, 1, Rj45, K1G, false, Pd)],
    },
    Model {
        id: "smart-tv",
        name: "Smart-TV",
        kind: DeviceKind::SmartTv,
        category: Category::EndDevices,
        title: "Smart-TV (1G, WLAN, Koax)",
        symbol: "smart_tv",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "TV",
        groups: &[grp("eth", 0, 1, Rj45, K1G, false, NoPoe), one("wlan0", RadioClient, 1_200_000, false, NoPoe), one("coax0", Coax, K1G, false, NoPoe)],
    },
    // ---- network
    Model {
        id: "jf-s24p",
        name: "JF-S24P",
        kind: DeviceKind::Switch,
        category: Category::Network,
        title: "Access-Switch 24× 1G PoE+, 4× 10G SFP+",
        symbol: "switch_l2",
        cli: CliStyle::Ios,
        features: F_BRIDGE,
        name_prefix: "SW",
        groups: &[
            grp("GigabitEthernet1/0/", 1, 24, Rj45, K1G, true, Pse),
            grp("TenGigabitEthernet1/1/", 1, 4, SfpPlus, K10G, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-s8",
        name: "JF-S8",
        kind: DeviceKind::Switch,
        category: Category::Network,
        title: "Tisch-Switch 8× 1G, unmanaged (ohne Befehlszeile)",
        symbol: "switch_desk",
        cli: CliStyle::None,
        features: F_BRIDGE,
        name_prefix: "SW",
        groups: &[grp("port", 0, 8, Rj45, K1G, true, NoPoe)],
    },
    Model {
        id: "jf-s8p",
        name: "JF-S8P",
        kind: DeviceKind::Switch,
        category: Category::Network,
        title: "Kompakt-Switch 8× 1G PoE+, 2× 10G SFP+",
        symbol: "switch_desk",
        cli: CliStyle::Ios,
        features: F_BRIDGE,
        name_prefix: "SW",
        groups: &[
            grp("GigabitEthernet1/0/", 1, 8, Rj45, K1G, true, Pse),
            grp("TenGigabitEthernet1/1/", 1, 2, SfpPlus, K10G, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-s24",
        name: "JF-S24",
        kind: DeviceKind::Switch,
        category: Category::Network,
        title: "Access-Switch 24× 1G, 4× 10G SFP+",
        symbol: "switch_l2",
        cli: CliStyle::Ios,
        features: F_BRIDGE,
        name_prefix: "SW",
        groups: &[
            grp("GigabitEthernet1/0/", 1, 24, Rj45, K1G, true, NoPoe),
            grp("TenGigabitEthernet1/1/", 1, 4, SfpPlus, K10G, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-s48p",
        name: "JF-S48P",
        kind: DeviceKind::Switch,
        category: Category::Network,
        title: "Access-Switch 48× 1G PoE+, 4× 10G SFP+",
        symbol: "switch_l2",
        cli: CliStyle::Ios,
        features: F_BRIDGE,
        name_prefix: "SW",
        groups: &[
            grp("GigabitEthernet1/0/", 1, 48, Rj45, K1G, true, Pse),
            grp("TenGigabitEthernet1/1/", 1, 4, SfpPlus, K10G, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-s24m",
        name: "JF-S24M",
        kind: DeviceKind::Switch,
        category: Category::Network,
        title: "Multi-Gigabit-Switch 24× 2.5G PoE++, 4× 25G SFP28",
        symbol: "switch_l2",
        cli: CliStyle::Ios,
        features: F_BRIDGE,
        name_prefix: "SW",
        groups: &[
            grp("TwoGigabitEthernet1/0/", 1, 24, Rj45, K2G5, true, PseBt),
            grp("TwentyFiveGigE1/1/", 1, 4, Sfp28, K25G, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-r4",
        name: "JF-R4",
        kind: DeviceKind::Router,
        category: Category::Network,
        title: "Edge-Router 4× 1G, 2× 10G SFP+, 2× Seriell",
        symbol: "router",
        cli: CliStyle::Ios,
        features: F_ROUTER,
        name_prefix: "R",
        groups: &[
            grp("GigabitEthernet0/0/", 0, 4, Rj45, K1G, false, NoPoe),
            grp("TenGigabitEthernet0/1/", 0, 2, SfpPlus, K10G, false, NoPoe),
            grp("Serial0/2/", 0, 2, Serial, 2048, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-r2",
        name: "JF-R2",
        kind: DeviceKind::Router,
        category: Category::Network,
        title: "Filial-Router 2× 1G, 2× Seriell",
        symbol: "router",
        cli: CliStyle::Ios,
        features: F_ROUTER,
        name_prefix: "R",
        groups: &[
            grp("GigabitEthernet0/0/", 0, 2, Rj45, K1G, false, NoPoe),
            grp("Serial0/1/", 0, 2, Serial, 2048, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-r8",
        name: "JF-R8",
        kind: DeviceKind::Router,
        category: Category::Network,
        title: "Core-Router 8× 10G SFP+, 2× 100G QSFP28",
        symbol: "router",
        cli: CliStyle::Ios,
        features: F_ROUTER,
        name_prefix: "CR",
        groups: &[
            one("GigabitEthernet0", Rj45, K1G, false, NoPoe),
            grp("TenGigabitEthernet0/0/", 0, 8, SfpPlus, K10G, false, NoPoe),
            grp("HundredGigE0/1/", 0, 2, Qsfp28, K100G, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-s24l3",
        name: "JF-S24L3",
        kind: DeviceKind::L3Switch,
        category: Category::Network,
        title: "L3-Switch 24× 1G, 4× 10G SFP+",
        symbol: "switch_l3",
        cli: CliStyle::Ios,
        features: F_BRIDGE | F_ROUTER,
        name_prefix: "MLS",
        groups: &[
            grp("GigabitEthernet1/0/", 1, 24, Rj45, K1G, true, NoPoe),
            grp("TenGigabitEthernet1/1/", 1, 4, SfpPlus, K10G, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-s48x",
        name: "JF-S48X",
        kind: DeviceKind::L3Switch,
        category: Category::Network,
        title: "Rechenzentrums-L3-Switch 48× 10G (10GBASE-T), 6× 100G QSFP28",
        symbol: "switch_l3",
        cli: CliStyle::Ios,
        features: F_BRIDGE | F_ROUTER,
        name_prefix: "DCSW",
        groups: &[
            grp("TenGigabitEthernet1/0/", 1, 48, Rj45, K10G, true, NoPoe),
            grp("HundredGigE1/0/", 49, 6, Qsfp28, K100G, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-s48y",
        name: "JF-S48Y",
        kind: DeviceKind::L3Switch,
        category: Category::Network,
        title: "Leaf-Switch (L3) 48× 25G SFP28, 8× 100G QSFP28",
        symbol: "switch_l3",
        cli: CliStyle::Ios,
        features: F_BRIDGE | F_ROUTER,
        name_prefix: "Leaf",
        groups: &[
            grp("TwentyFiveGigE1/0/", 1, 48, Sfp28, K25G, false, NoPoe),
            grp("HundredGigE1/0/", 49, 8, Qsfp28, K100G, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-s32c",
        name: "JF-S32C",
        kind: DeviceKind::L3Switch,
        category: Category::Network,
        title: "Spine-Switch (L3) 32× 100G QSFP28",
        symbol: "switch_l3",
        cli: CliStyle::Ios,
        features: F_BRIDGE | F_ROUTER,
        name_prefix: "Spine",
        groups: &[
            grp("HundredGigE1/0/", 1, 32, Qsfp28, K100G, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "home-router",
        name: "Heimrouter",
        kind: DeviceKind::HomeRouter,
        category: Category::Network,
        title: "Heimrouter mit WLAN 802.11ax (1G WAN, 4× 1G LAN)",
        symbol: "home_router",
        cli: CliStyle::Linux,
        features: F_HOST | F_ROUTER | F_BRIDGE | F_AP | F_NAT,
        name_prefix: "Gateway",
        groups: &[
            one("wan", Rj45, K1G, false, NoPoe),
            grp("lan", 1, 4, Rj45, K1G, true, NoPoe),
            one("wlan0", RadioAp, 2_400_000, false, NoPoe),
        ],
    },
    Model {
        id: "hub",
        name: "Hub",
        kind: DeviceKind::Hub,
        category: Category::Network,
        title: "Hub 8× 100M (Lehrgerät: jedes Paket an alle)",
        symbol: "switch_l2",
        cli: CliStyle::None,
        features: F_HUB,
        name_prefix: "Hub",
        groups: &[grp("port", 0, 8, Rj45, 100_000, true, NoPoe)],
    },
    Model {
        id: "repeater",
        name: "Repeater",
        kind: DeviceKind::Hub,
        category: Category::Network,
        title: "Repeater 2× 1G (Schicht 1: verlängert eine Kupferstrecke)",
        symbol: "repeater",
        cli: CliStyle::None,
        features: F_HUB,
        name_prefix: "Rep",
        groups: &[grp("port", 0, 2, Rj45, K1G, true, NoPoe)],
    },
    Model {
        id: "bridge",
        name: "Bridge",
        kind: DeviceKind::Switch,
        category: Category::Network,
        title: "Bridge 2× 1G (Schicht 2: lernt Adressen, ohne Befehlszeile)",
        symbol: "bridge",
        cli: CliStyle::None,
        features: F_BRIDGE,
        name_prefix: "Br",
        groups: &[grp("port", 0, 2, Rj45, K1G, true, NoPoe)],
    },
    // ---- security
    Model {
        id: "jf-fw",
        name: "JF-FW",
        kind: DeviceKind::Firewall,
        category: Category::Security,
        title: "Firewall (Router mit ACLs und NAT, Zonen mit Sitzungen) 8× 1G, 2× 10G SFP+",
        symbol: "firewall_ngfw",
        cli: CliStyle::Ios,
        features: F_ROUTER | F_FIREWALL | F_NAT,
        name_prefix: "FW",
        groups: &[
            grp("GigabitEthernet0/", 0, 8, Rj45, K1G, false, NoPoe),
            grp("TenGigabitEthernet0/", 8, 2, SfpPlus, K10G, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-fw2",
        name: "JF-FW2",
        kind: DeviceKind::Firewall,
        category: Category::Security,
        title: "Kleine Firewall (Router mit ACLs und NAT, Zonen mit Sitzungen) 1G WAN, 4× 1G LAN",
        symbol: "firewall_desk",
        cli: CliStyle::Ios,
        features: F_ROUTER | F_FIREWALL | F_NAT,
        name_prefix: "FW",
        groups: &[
            grp("GigabitEthernet0/", 0, 1, Rj45, K1G, false, NoPoe),
            grp("GigabitEthernet0/", 1, 4, Rj45, K1G, true, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-fw8",
        name: "JF-FW8",
        kind: DeviceKind::Firewall,
        category: Category::Security,
        title: "Rechenzentrums-Firewall (Router mit ACLs und NAT, Zonen mit Sitzungen) 8× 10G SFP+, 4× 100G QSFP28",
        symbol: "firewall_ngfw",
        cli: CliStyle::Ios,
        features: F_ROUTER | F_FIREWALL | F_NAT,
        name_prefix: "FW",
        groups: &[
            grp("TenGigabitEthernet0/0/", 0, 8, SfpPlus, K10G, false, NoPoe),
            grp("HundredGigE0/1/", 0, 4, Qsfp28, K100G, false, NoPoe),
            console(),
        ],
    },
    // ---- wireless and WAN
    Model {
        id: "jf-ap6e",
        name: "JF-AP6E",
        kind: DeviceKind::AccessPoint,
        category: Category::WirelessWan,
        title: "WLAN-Access-Point 802.11ax (6 GHz, 2.5G, PoE-gespeist)",
        symbol: "ap_wifi6e",
        cli: CliStyle::Ios,
        features: F_AP | F_BRIDGE,
        name_prefix: "AP",
        groups: &[
            one("TwoGigabitEthernet0", Rj45, K2G5, false, Pd),
            one("Dot11Radio0", RadioAp, 574_000, false, NoPoe),
            one("Dot11Radio1", RadioAp, 2_400_000, false, NoPoe),
            one("Dot11Radio2", RadioAp, 2_400_000, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "jf-ap7",
        name: "JF-AP7",
        kind: DeviceKind::AccessPoint,
        category: Category::WirelessWan,
        title: "WLAN-Access-Point 802.11be (6 GHz, 5G, PoE-gespeist)",
        symbol: "ap_wifi6e",
        cli: CliStyle::Ios,
        features: F_AP | F_BRIDGE,
        name_prefix: "AP",
        groups: &[
            one("FiveGigabitEthernet0", Rj45, K5G, false, Pd),
            one("Dot11Radio0", RadioAp, 1_376_000, false, NoPoe),
            one("Dot11Radio1", RadioAp, 5_765_000, false, NoPoe),
            one("Dot11Radio2", RadioAp, 11_530_000, false, NoPoe),
            console(),
        ],
    },
    Model {
        id: "modem-ont",
        name: "Modem/ONT",
        kind: DeviceKind::Modem,
        category: Category::WirelessWan,
        title: "Glasfaser-ONT / Kabelmodem (WAN, 1G LAN)",
        symbol: "modem_ont",
        cli: CliStyle::None,
        features: F_BRIDGE,
        name_prefix: "Modem",
        groups: &[one("wan0", Wan, K1G, false, NoPoe), one("lan0", Rj45, K1G, true, NoPoe)],
    },
    Model {
        id: "cloud",
        name: "Internet",
        kind: DeviceKind::Cloud,
        category: Category::WirelessWan,
        title: "Internet / Netz des Providers",
        symbol: "cloud_internet",
        cli: CliStyle::None,
        features: F_CLOUD,
        name_prefix: "Internet",
        groups: &[grp("eth", 0, 8, Rj45, K10G, true, NoPoe), grp("wan", 0, 4, Wan, K1G, false, NoPoe)],
    },
    // ---- servers
    Model {
        id: "server-1u",
        name: "Rack-Server 1U",
        kind: DeviceKind::Server,
        category: Category::Servers,
        title: "Rack-Server 1U (2× 10GBASE-T, 1G Management)",
        symbol: "server_1u",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "SRV",
        groups: &[grp("eth", 0, 1, Rj45, K1G, false, NoPoe), grp("eth", 1, 2, Rj45, K10G, false, NoPoe)],
    },
    Model {
        id: "server-1u-sfp",
        name: "Rack-Server 1U SFP28",
        kind: DeviceKind::Server,
        category: Category::Servers,
        title: "Rack-Server 1U (2× 25G SFP28, 1G Management)",
        symbol: "server_1u",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "SRV",
        groups: &[grp("eth", 0, 1, Rj45, K1G, false, NoPoe), grp("eth", 1, 2, Sfp28, K25G, false, NoPoe)],
    },
    Model {
        id: "server-tower",
        name: "Tower-Server",
        kind: DeviceKind::Server,
        category: Category::Servers,
        title: "Tower-Server (2× 1G)",
        symbol: "server_tower",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "SRV",
        groups: &[grp("eth", 0, 2, Rj45, K1G, false, NoPoe)],
    },
    // ---- distributed systems
    Model {
        id: "load-balancer",
        name: "Load Balancer",
        kind: DeviceKind::LoadBalancer,
        category: Category::Distributed,
        title: "Load Balancer L4/L7 (4× 10GBASE-T)",
        symbol: "load_balancer",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "LB",
        groups: &[grp("eth", 0, 4, Rj45, K10G, false, NoPoe)],
    },
    Model {
        id: "database",
        name: "DB-Shard",
        kind: DeviceKind::Database,
        category: Category::Distributed,
        title: "Datenbank-Knoten / Shard (2× 10GBASE-T)",
        symbol: "db_shard",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "DB",
        groups: &[grp("eth", 0, 2, Rj45, K10G, false, NoPoe)],
    },
    Model {
        id: "cache",
        name: "Cache",
        kind: DeviceKind::Cache,
        category: Category::Distributed,
        title: "Cache-Knoten LRU/TTL (2× 10GBASE-T)",
        symbol: "cache",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "Cache",
        groups: &[grp("eth", 0, 2, Rj45, K10G, false, NoPoe)],
    },
    Model {
        id: "queue",
        name: "Message Queue",
        kind: DeviceKind::Queue,
        category: Category::Distributed,
        title: "Message-Queue-Knoten (2× 10GBASE-T)",
        symbol: "queue",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "MQ",
        groups: &[grp("eth", 0, 2, Rj45, K10G, false, NoPoe)],
    },
    Model {
        id: "traffic-gen",
        name: "Traffic-Generator",
        kind: DeviceKind::TrafficGen,
        category: Category::Distributed,
        title: "Lastgenerator für offene und geschlossene Last (4× 10GBASE-T)",
        symbol: "traffic_gen",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "Gen",
        groups: &[grp("eth", 0, 4, Rj45, K10G, false, NoPoe)],
    },
    Model {
        id: "container-host",
        name: "Container-Host",
        kind: DeviceKind::ContainerHost,
        category: Category::Distributed,
        title: "Container-Host für mehrere Dienste und Skripte (2× 10GBASE-T)",
        symbol: "container_host",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "Host",
        groups: &[grp("eth", 0, 2, Rj45, K10G, false, NoPoe)],
    },
    // ---- annotations
    Model {
        id: "note",
        name: "Notiz",
        kind: DeviceKind::Note,
        category: Category::Annotations,
        title: "Notiz auf der Zeichenfläche",
        symbol: "note",
        cli: CliStyle::None,
        features: F_ANNOTATION,
        name_prefix: "Notiz",
        groups: &[],
    },
    // ---- the modems of a coaxial and a telephone line (H2)
    Model {
        id: "cable-modem",
        name: "Kabelmodem",
        kind: DeviceKind::Modem,
        category: Category::WirelessWan,
        title: "Kabelmodem (Koax, 1G LAN)",
        symbol: "modem_ont",
        cli: CliStyle::None,
        features: F_BRIDGE,
        name_prefix: "Modem",
        groups: &[one("coax0", Coax, K1G, false, NoPoe), one("lan0", Rj45, K1G, true, NoPoe)],
    },
    Model {
        id: "dsl-modem",
        name: "DSL-Modem",
        kind: DeviceKind::Modem,
        category: Category::WirelessWan,
        title: "DSL-Modem (RJ11, 1G LAN)",
        symbol: "modem_ont",
        cli: CliStyle::None,
        features: F_BRIDGE,
        name_prefix: "Modem",
        groups: &[one("dsl0", Rj11, 100_000, false, NoPoe), one("lan0", Rj45, K1G, true, NoPoe)],
    },
    // ---- the servers by size (SA1; their figures: hw_of)
    Model {
        id: "server-s",
        name: "Rack-Server S",
        kind: DeviceKind::Server,
        category: Category::Servers,
        title: "Rack-Server S (2× 10GBASE-T, 1G Management)",
        symbol: "server_1u",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "SRV",
        groups: &[grp("eth", 0, 1, Rj45, K1G, false, NoPoe), grp("eth", 1, 2, Rj45, K10G, false, NoPoe)],
    },
    Model {
        id: "server-m",
        name: "Rack-Server M",
        kind: DeviceKind::Server,
        category: Category::Servers,
        title: "Rack-Server M (2× 25G SFP28, 1G Management)",
        symbol: "server_1u",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "SRV",
        groups: &[grp("eth", 0, 1, Rj45, K1G, false, NoPoe), grp("eth", 1, 2, Sfp28, K25G, false, NoPoe)],
    },
    Model {
        id: "server-l",
        name: "Rack-Server L",
        kind: DeviceKind::Server,
        category: Category::Servers,
        title: "Rack-Server L (2× 25G SFP28, 1G Management)",
        symbol: "server_1u",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "SRV",
        groups: &[grp("eth", 0, 1, Rj45, K1G, false, NoPoe), grp("eth", 1, 2, Sfp28, K25G, false, NoPoe)],
    },
    Model {
        id: "server-xl",
        name: "Rack-Server XL",
        kind: DeviceKind::Server,
        category: Category::Servers,
        title: "Rack-Server XL (2× 100G QSFP28, 1G Management)",
        symbol: "server_1u",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "SRV",
        groups: &[grp("eth", 0, 1, Rj45, K1G, false, NoPoe), grp("eth", 1, 2, Qsfp28, K100G, false, NoPoe)],
    },
    Model {
        id: "nas",
        name: "NAS",
        kind: DeviceKind::Server,
        category: Category::Servers,
        title: "NAS mit Festplatten (2× 10GBASE-T)",
        symbol: "server_tower",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "NAS",
        groups: &[grp("eth", 0, 2, Rj45, K10G, false, NoPoe)],
    },
];

/// every model in palette order
pub fn catalog() -> &'static [Model] {
    CATALOG
}

/// a model by its id (case-insensitive)
pub fn model(id: &str) -> Option<&'static Model> {
    CATALOG.iter().find(|m| m.id.eq_ignore_ascii_case(id))
}

/// the first model of a kind
pub fn default_model(kind: DeviceKind) -> Option<&'static Model> {
    CATALOG.iter().find(|m| m.kind == kind)
}

/// the models of a palette group
pub fn in_category(c: Category) -> impl Iterator<Item = &'static Model> {
    CATALOG.iter().filter(move |m| m.category == c)
}

// ---------------------------------------------------------------- the physical side (P1)
//
// Optic: the transceiver in an SFP / SFP+ / SFP28 / QSFP28 cage.  Every cage comes with a
// short-reach multimode one (SR: 850 nm, SX at 1G, SR4 at 100G); a device may take a long-reach
// single-mode one instead (LR: 1310 nm, LX at 1G, LR4 at 100G; Device::optics).  The optics of
// both ends and the fibre between them must match (phys.rs: reach, the simulator's link state).
// Form and rack_units: how a model stands in the physical view (phys.rs) -- in a rack with its
// height in rack units, on a desk, carried around, on the ceiling or a wall, on the floor beside
// a desk (a tower), or nowhere at all (the provider's cloud).

wire_enum! {
    /// the transceiver of an optics cage
    pub enum Optic {
        Sr = "sr",
        Lr = "lr",
    }
}

impl Optic {
    /// "SR", "LR"
    pub fn short(self) -> &'static str {
        match self {
            Optic::Sr => "SR",
            Optic::Lr => "LR",
        }
    }
    /// the German name (the app translates it)
    pub fn title(self) -> &'static str {
        // i18n
        match self {
            Optic::Sr => "SR, Multimode (kurze Strecken)",
            Optic::Lr => "LR, Singlemode (bis 10 km)",
        }
    }
}

/// how a device stands in the physical view
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Form {
    /// mounted in a rack (Model's rack_units high); it may stand on a desk or the floor too
    Rack,
    /// stands on a desk or table
    Desktop,
    /// carried around (laptop, smartphone, tablet): on a desk or anywhere in a room
    Handheld,
    /// on the ceiling or a wall (access point, camera, sensor)
    Ceiling,
    /// stands on the floor (a tower server)
    Tower,
    /// no physical place (the provider's network)
    Virtual,
}

impl Form {
    /// the German name (the app translates it)
    pub fn title(self) -> &'static str {
        // i18n
        match self {
            Form::Rack => "Rack-Gerät",
            Form::Desktop => "Tischgerät",
            Form::Handheld => "Mobilgerät",
            Form::Ceiling => "Decken- oder Wandgerät",
            Form::Tower => "Standgerät",
            Form::Virtual => "ohne Ort",
        }
    }
}

/// the form of a model
pub fn form(m: &Model) -> Form {
    match m.kind {
        DeviceKind::Pc | DeviceKind::Printer | DeviceKind::IpPhone | DeviceKind::SmartTv | DeviceKind::HomeRouter | DeviceKind::Modem | DeviceKind::Hub => Form::Desktop,
        // the small boxes among the switches, routers and firewalls
        _ if matches!(m.id, "jf-s8" | "jf-s8p" | "bridge" | "jf-r2" | "jf-fw2") => Form::Desktop,
        DeviceKind::Laptop | DeviceKind::Smartphone | DeviceKind::Tablet => Form::Handheld,
        DeviceKind::AccessPoint | DeviceKind::IotCamera | DeviceKind::IotSensor => Form::Ceiling,
        DeviceKind::Server if m.id == "server-tower" => Form::Tower,
        DeviceKind::Cloud | DeviceKind::Note => Form::Virtual,
        _ => Form::Rack,
    }
}

/// the height of a rack device in rack units (1 U = 44.45 mm); 0 for other forms
pub fn rack_units(m: &Model) -> u8 {
    if form(m) != Form::Rack {
        return 0;
    }
    match m.id {
        "jf-r8" | "jf-fw8" | "database" | "container-host" | "server-l" | "server-xl" | "nas" => 2,
        _ => 1,
    }
}

// ---------------------------------------------------------------- the hardware of a device (SA1)
//
// Hw: what the simulator's res.rs simulates of a device: cores, how fast a core is against the
// reference core, RAM, swap, the disk.  A device has its own (Device::hw), or in a project with "Hardware
// simulieren" (Project::metered) the row of its model below; without either it has none and costs
// nothing (jfn_sim stays as it always was).  The figures of hw_of are round assumptions of ours,
// typical of the size and not measured on a product: the window says so.  More cores come with a
// lower clock, as they do in real servers, so a bigger machine is not simply more of everything.

wire_enum! {
    /// the kind of a device's disk (its access times: DiskClass::spec)
    pub enum DiskClass {
        Sata = "sata",
        Nvme = "nvme",
        NvmeFast = "nvme-fast",
        Hdd = "hdd",
    }
}

impl DiskClass {
    /// latency (ns), bytes per second, accesses per second
    pub fn spec(self) -> (u64, u64, u64) {
        match self {
            // the numbers of a device's disk without hardware (jfn_sim's Disk::default)
            DiskClass::Sata => (100_000, 500_000_000, 50_000),
            DiskClass::Nvme => (20_000, 3_000_000_000, 500_000),
            DiskClass::NvmeFast => (10_000, 7_000_000_000, 1_000_000),
            DiskClass::Hdd => (6_000_000, 200_000_000, 200),
        }
    }
    /// its name (the app translates it)
    pub fn title(self) -> &'static str {
        // i18n
        match self {
            DiskClass::Sata => "SATA-SSD",
            DiskClass::Nvme => "NVMe-SSD",
            DiskClass::NvmeFast => "schnelle NVMe-SSD",
            DiskClass::Hdd => "Festplatte",
        }
    }
}

agent_core::record! {
    /// the hardware of a device (the simulator's res.rs)
    pub struct Hw {
        /// cores (at least 1)
        pub cores: u32,
        /// how fast a core is in per mille of the reference core (1000: a script's instruction
        /// takes its 5 ns, a cost of 1 ms takes 1 ms)
        pub speed_pm: u32,
        pub ram_mb: u64,
        /// what may lie on the disk beyond the RAM before the largest service is killed
        pub swap_mb: u64,
        pub disk: DiskClass,
        pub disk_gb: u64,
    }
}

impl Eq for Hw {}

impl Hw {
    const fn of(cores: u32, speed_pm: u32, ram_gb: u64, swap_gb: u64, disk: DiskClass, disk_gb: u64) -> Hw {
        Hw { cores, speed_pm, ram_mb: ram_gb << 10, swap_mb: swap_gb << 10, disk, disk_gb }
    }
    /// values that cannot run brought into range: at least a core at 1 ‰ of the reference, 1 MiB
    pub fn sane(&self) -> Hw {
        Hw { cores: self.cores.clamp(1, 1024), speed_pm: self.speed_pm.clamp(1, 100_000), ram_mb: self.ram_mb.max(1), swap_mb: self.swap_mb, disk: self.disk, disk_gb: self.disk_gb }
    }
    /// "8 Kerne, 32 GiB RAM, NVMe-SSD 960 GiB" (the app translates it)
    pub fn summary_msg(&self) -> Msg {
        let ram = if self.ram_mb >= 1024 { msg!("{} GiB", self.ram_mb >> 10) } else { msg!("{} MiB", self.ram_mb) };
        let cores = if self.cores == 1 { phrase("1 Kern") } else { msg!("{} Kerne", self.cores) };
        if self.speed_pm == 1000 {
            msg!("{}, {} RAM, {} {} GiB", cores, ram, phrase(self.disk.title()), self.disk_gb)
        } else {
            msg!("{} mit {} % Tempo, {} RAM, {} {} GiB", cores, self.speed_pm / 10, ram, phrase(self.disk.title()), self.disk_gb)
        }
    }
}

/// the figures of a model in a project with "Hardware simulieren" (assumptions, see above): None
/// for devices that stay without (network devices, phones, load generators: they never pay)
pub fn hw_of(id: &str) -> Option<Hw> {
    use DiskClass::{Hdd, Nvme, NvmeFast, Sata};
    Some(match id {
        "pc" => Hw::of(4, 1000, 16, 2, Sata, 480),
        "laptop" => Hw::of(4, 800, 16, 2, Nvme, 480),
        "server-tower" => Hw::of(4, 1000, 16, 2, Sata, 960),
        "server-1u" | "server-1u-sfp" => Hw::of(8, 1000, 32, 4, Sata, 960),
        "server-s" => Hw::of(4, 1100, 16, 2, Sata, 480),
        "server-m" => Hw::of(8, 1000, 32, 4, Nvme, 960),
        "server-l" => Hw::of(16, 950, 64, 8, Nvme, 1920),
        "server-xl" => Hw::of(32, 900, 256, 16, NvmeFast, 3840),
        "nas" => Hw::of(2, 700, 8, 2, Hdd, 8000),
        "load-balancer" => Hw::of(8, 1000, 16, 0, Sata, 240),
        "database" => Hw::of(16, 1000, 64, 8, Nvme, 1920),
        "cache" => Hw::of(4, 1000, 64, 0, Sata, 240),
        "queue" => Hw::of(4, 1000, 16, 2, Nvme, 480),
        "container-host" => Hw::of(16, 1000, 64, 8, Nvme, 960),
        _ => return None,
    })
}

/// the models with figures, smallest server first: the choices of the window's hardware page
pub fn hw_presets() -> impl Iterator<Item = &'static Model> {
    ["server-s", "server-tower", "server-1u", "server-m", "server-l", "server-xl", "nas", "pc", "laptop", "load-balancer", "database", "cache", "queue", "container-host"].into_iter().filter_map(model)
}

// ---------------------------------------------------------------- what a device costs (SA2)
//
// Prices: euros per hour, as round figures of our own, near what rented servers cost in 2026 and not
// the offer of any provider: an experiment's results say so wherever they show them.  A core costs
// the same up to 16 cores, more beyond that and more again beyond 32 (machines of that size are rarer
// and dearer per core); a host costs a fee of its own whatever its size (housing, power, operating
// it), which is what makes many small machines dearer than one of the same sum.  An experiment may
// name other figures (Experiment::prices).

/// euros per hour in millionths (assumptions, see above)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prices {
    /// a core at the reference speed; its price goes with its speed
    pub core: u64,
    /// the factor on cores beyond 16 and beyond 32, in millionths
    pub big: u64,
    pub huge: u64,
    /// a GiB of RAM
    pub ram_gb: u64,
    /// a GiB of disk by its kind
    pub hdd_gb: u64,
    pub sata_gb: u64,
    pub nvme_gb: u64,
    pub nvme_fast_gb: u64,
    /// the fee of a host whatever its size
    pub host: u64,
    /// a switch, a router or firewall
    pub switch: u64,
    pub router: u64,
}

impl Prices {
    /// the figures of an experiment that names none
    pub const ASSUMED: Prices = Prices { core: 30_000, big: 1_500_000, huge: 2_000_000, ram_gb: 4_000, hdd_gb: 40, sata_gb: 100, nvme_gb: 200, nvme_fast_gb: 400, host: 20_000, switch: 10_000, router: 30_000 };
    /// the names an experiment may give figures for
    pub const KEYS: &'static [&'static str] = &["core", "big", "huge", "ram_gb", "hdd_gb", "sata_gb", "nvme_gb", "nvme_fast_gb", "host", "switch", "router"];
    /// one figure by its name, read
    pub fn get(&self, key: &str) -> u64 {
        match key {
            "core" => self.core,
            "big" => self.big,
            "huge" => self.huge,
            "ram_gb" => self.ram_gb,
            "hdd_gb" => self.hdd_gb,
            "sata_gb" => self.sata_gb,
            "nvme_gb" => self.nvme_gb,
            "nvme_fast_gb" => self.nvme_fast_gb,
            "host" => self.host,
            "switch" => self.switch,
            "router" => self.router,
            _ => 0,
        }
    }
    /// one figure by its name
    pub fn set(&mut self, key: &str, v: u64) {
        match key {
            "core" => self.core = v,
            "big" => self.big = v,
            "huge" => self.huge = v,
            "ram_gb" => self.ram_gb = v,
            "hdd_gb" => self.hdd_gb = v,
            "sata_gb" => self.sata_gb = v,
            "nvme_gb" => self.nvme_gb = v,
            "nvme_fast_gb" => self.nvme_fast_gb = v,
            "host" => self.host = v,
            "switch" => self.switch = v,
            "router" => self.router = v,
            _ => {}
        }
    }
    /// what hardware of these figures costs per hour (millionths of a euro)
    pub fn of_hw(&self, hw: &Hw) -> u64 {
        let c = hw.cores as u64;
        // the cores in thousandths of a core at the reference price
        let cores = c.min(16) * 1000 + c.clamp(16, 32).saturating_sub(16) * self.big / 1000 + c.saturating_sub(32) * self.huge / 1000;
        let per_gb = match hw.disk {
            DiskClass::Hdd => self.hdd_gb,
            DiskClass::Sata => self.sata_gb,
            DiskClass::Nvme => self.nvme_gb,
            DiskClass::NvmeFast => self.nvme_fast_gb,
        };
        self.core * cores / 1000 * hw.speed_pm as u64 / 1000 + self.ram_gb * hw.ram_mb / 1024 + per_gb * hw.disk_gb + self.host
    }
    /// what a device of this model costs per hour, with its own figures or else its model's; a
    /// load generator, a client or a cloud has no price
    pub fn of_device(&self, model_id: &str, hw: Option<&Hw>) -> u64 {
        match hw.cloned().or_else(|| hw_of(model_id)) {
            Some(h) => self.of_hw(&h),
            None => match model(model_id).map(|m| m.kind) {
                Some(DeviceKind::Switch | DeviceKind::L3Switch) => self.switch,
                Some(DeviceKind::Router | DeviceKind::Firewall) => self.router,
                _ => 0,
            },
        }
    }
}

/// the service a tile of the palette brings when it is placed in the window (SA3): (kind, name).  A
/// file that is loaded gets nothing from here
pub fn preset_service(model: &str) -> Option<(&'static str, &'static str)> {
    Some(match model {
        "load-balancer" => ("lb", "lb"),
        "database" => ("db", "db"),
        "cache" => ("cache", "cache"),
        "queue" => ("mq", "mq"),
        "traffic-gen" => ("http-load", "load"),
        _ => return None,
    })
}

/// the config of a tile's service when it is more than empty: JSON text
pub fn preset_config(model: &str) -> Option<&'static str> {
    match model {
        // a small example table, so that the database runs as soon as it stands there
        "database" => Some(r#"{"tables": [{"name": "users", "rows": 100000, "row_bytes": 200, "key": "id", "columns": ["email:text", "balance:int:1000"], "indexes": ["email"]}]}"#),
        _ => None,
    }
}
