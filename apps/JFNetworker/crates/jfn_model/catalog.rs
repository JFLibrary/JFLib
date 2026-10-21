// catalog -- the devices JFNetworker offers: models with their ports, command line and role
//
//   catalog() -> &[Model]         every model in palette order
//   model(id) -> Option<&Model>   by its stable id ("jf-s24p"), stored in project files
//   default_model(kind)           the first model of a kind
//   Model::ports() / port(name)   the expanded ports ("GigabitEthernet1/0/1" ...); port() also
//                                 takes IOS abbreviations ("gi1/0/1")
//   Model::summary()              "24× 1G PoE+, 4× 10G SFP+, Konsole"
//   speed_str(kbps)               "1G", "2.5G", "100M", "2 Mbit/s"
//
// The models are vendor neutral but modern: multi-gigabit copper, SFP+/SFP28/QSFP28 optics,
// PoE, WLAN 802.11ax with 6 GHz.  Network devices speak the IOS-like command line and use IOS interface names;
// hosts have a Linux-like shell and Linux names (eth0, wlan0).  `features` (F_*) tells the
// simulator which parts a device has (bridge, router, host OS, ...).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::wire_enum;

use crate::ifname;

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
}

impl Media {
    pub fn name(self) -> &'static str {
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
    /// is powered through this port (AP, phone, camera)
    Pd,
}

/// runs an operating system with sockets, services and a shell
pub const F_HOST: u32 = 1;
/// a learning bridge between its Ethernet ports (switch)
pub const F_BRIDGE: u32 = 2;
/// forwards IP between interfaces (router, L3 switch)
pub const F_ROUTER: u32 = 4;
/// stateful filtering and zones
pub const F_FIREWALL: u32 = 8;
/// bridges its radios with its wired port (access point)
pub const F_AP: u32 = 16;
/// repeats every frame to every other port
pub const F_HUB: u32 = 32;
/// the provider network / Internet
pub const F_CLOUD: u32 = 64;
/// network address translation for a private LAN (home router, firewall)
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
                v.push(PortSpec { name, index: v.len() as u16, media: g.media, kbps: g.kbps, mdix: g.mdix, poe: g.poe });
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
        let mut parts: Vec<String> = Vec::new();
        for g in self.groups {
            let s = match g.media {
                Media::Console => String::from("Konsole"),
                Media::Usb => String::from("USB"),
                Media::RadioAp | Media::RadioClient => format!("{}× Funk {}", g.count, speed_str(g.kbps)),
                Media::Serial => format!("{}× Seriell", g.count),
                Media::Wan => format!("{}× WAN", g.count),
                m => {
                    let mut s = format!("{}× {}", g.count, speed_str(g.kbps));
                    if m.is_optical() {
                        s.push(' ');
                        s.push_str(m.name());
                    }
                    match g.poe {
                        Poe::Pse => s.push_str(" PoE+"),
                        Poe::Pd => s.push_str(" (PoE-gespeist)"),
                        Poe::None => {}
                    }
                    s
                }
            };
            parts.push(s);
        }
        parts.join(", ")
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

use Media::*;
use Poe::{None as NoPoe, Pd, Pse};

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
        title: "Smart-TV (1G, WLAN)",
        symbol: "smart_tv",
        cli: CliStyle::Linux,
        features: F_HOST,
        name_prefix: "TV",
        groups: &[grp("eth", 0, 1, Rj45, K1G, false, NoPoe), one("wlan0", RadioClient, 1_200_000, false, NoPoe)],
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
    // ---- security
    Model {
        id: "jf-fw",
        name: "JF-FW",
        kind: DeviceKind::Firewall,
        category: Category::Security,
        title: "Next-Generation-Firewall 8× 1G, 2× 10G SFP+",
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
