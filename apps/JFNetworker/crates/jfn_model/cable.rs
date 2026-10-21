// cable -- which cable joins two ports, and what a link of that cable defaults to
//
//   cable_kind(a, b) -> Result<CableKind, Why>     the correct cable ("Automatisch")
//   cable_fits(kind, a, b) -> Result<(), Why>      a cable the user chose: does it plug in?
//   needs_crossover(a, b)                          copper between two MDI or two MDI-X ports
//   link_kbps(a, b) / default_delay_ns(kind)       the speed both ends agree on, the delay
//   CableKind::title / max_len_m                   German name, the medium's length limit
//
// Copper: a straight cable joins MDI (host, router) and MDI-X (switch) ports, a crossover cable
// two of the same kind.  With auto-MDI-X (the default of every multi-gigabit port, and of the
// project unless "strict_mdix" is set) both cables work; the simulator decides that, this
// module only names the right one.  Optics: SFP, SFP+ and SFP28 cages take each other's
// optics at the lower speed; QSFP28 only joins QSFP28.  Radios: a client associates with an
// access point (no ad-hoc mode).

use alloc::format;
use alloc::string::String;
use core::fmt;

use agent_core::wire_enum;

use crate::catalog::{Media, PortSpec};

wire_enum! {
    /// the kind of a link
    pub enum CableKind {
        CopperStraight = "copper",
        CopperCross = "copper-cross",
        FiberMm = "fiber-mm",
        FiberSm = "fiber-sm",
        Serial = "serial",
        Console = "console",
        Wireless = "wireless",
        Wan = "wan",
    }
}

impl CableKind {
    /// the German name (the app translates it)
    pub fn title(self) -> &'static str {
        match self {
            CableKind::CopperStraight => "Kupfer, gerade",
            CableKind::CopperCross => "Kupfer, gekreuzt",
            CableKind::FiberMm => "Glasfaser, Multimode",
            CableKind::FiberSm => "Glasfaser, Singlemode",
            CableKind::Serial => "Seriell (WAN)",
            CableKind::Console => "Konsolenkabel",
            CableKind::Wireless => "Funk (WLAN)",
            CableKind::Wan => "WAN-Leitung",
        }
    }
    pub fn is_copper(self) -> bool {
        matches!(self, CableKind::CopperStraight | CableKind::CopperCross)
    }
    pub fn is_fiber(self) -> bool {
        matches!(self, CableKind::FiberMm | CableKind::FiberSm)
    }
    /// carries Ethernet frames (not serial, not console)
    pub fn is_ethernet(self) -> bool {
        self.is_copper() || self.is_fiber() || self == CableKind::Wireless || self == CableKind::Wan
    }
    /// the longest cable of this kind at a speed (None: no limit modelled)
    pub fn max_len_m(self, kbps: u32) -> Option<u32> {
        match self {
            CableKind::CopperStraight | CableKind::CopperCross => Some(100),
            // OM3: 300 m at 10G, 100 m for 25G-SR and 100G-SR4
            CableKind::FiberMm => Some(if kbps >= 25_000_000 { 100 } else { 300 }),
            CableKind::FiberSm => Some(10_000),
            CableKind::Console => Some(15),
            CableKind::Serial | CableKind::Wireless | CableKind::Wan => None,
        }
    }
}

/// why two ports cannot be joined (German text through Display)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Why {
    /// both ends on the same device
    SameDevice,
    /// the port is connected already
    PortInUse(String),
    /// the two port kinds have no common cable
    MediaMismatch(Media, Media),
    /// two radios that are no client / access point pair
    NoAccessPoint,
    /// the chosen cable does not plug into these ports
    CableDoesNotFit(CableKind, Media),
}

impl fmt::Display for Why {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Why::SameDevice => f.write_str("Ein Kabel braucht zwei verschiedene Geräte."),
            Why::PortInUse(p) => write!(f, "Der Port {} ist schon belegt.", p),
            Why::MediaMismatch(a, b) => write!(f, "{} und {} passen nicht zusammen.", a.name(), b.name()),
            Why::NoAccessPoint => f.write_str("Funk verbindet ein WLAN-Gerät mit einem Access Point."),
            Why::CableDoesNotFit(c, m) => write!(f, "{} passt nicht in einen {}-Port.", c.title(), m.name()),
        }
    }
}

/// optics that can join: SFP / SFP+ / SFP28 among each other, QSFP28 only with QSFP28
fn optics_join(a: Media, b: Media) -> bool {
    let sfp = |m: Media| matches!(m, Media::Sfp | Media::SfpPlus | Media::Sfp28);
    (sfp(a) && sfp(b)) || (a == Media::Qsfp28 && b == Media::Qsfp28)
}

/// copper between two MDI or two MDI-X ports needs a crossover cable (without auto-MDI-X)
pub fn needs_crossover(a: &PortSpec, b: &PortSpec) -> bool {
    a.mdix == b.mdix
}

/// the correct cable for two ports
pub fn cable_kind(a: &PortSpec, b: &PortSpec) -> Result<CableKind, Why> {
    use Media::*;
    match (a.media, b.media) {
        (Rj45, Rj45) => Ok(if needs_crossover(a, b) { CableKind::CopperCross } else { CableKind::CopperStraight }),
        (x, y) if x.is_optical() && y.is_optical() => {
            if optics_join(x, y) {
                Ok(CableKind::FiberMm)
            } else {
                Err(Why::MediaMismatch(x, y))
            }
        }
        (Serial, Serial) => Ok(CableKind::Serial),
        (Console, Usb) | (Usb, Console) => Ok(CableKind::Console),
        (RadioAp, RadioClient) | (RadioClient, RadioAp) => Ok(CableKind::Wireless),
        (RadioAp, RadioAp) | (RadioClient, RadioClient) => Err(Why::NoAccessPoint),
        (Wan, Wan) => Ok(CableKind::Wan),
        (x, y) => Err(Why::MediaMismatch(x, y)),
    }
}

/// does a chosen cable plug into both ports? (copper: either straight or crossover)
pub fn cable_fits(kind: CableKind, a: &PortSpec, b: &PortSpec) -> Result<(), Why> {
    let right = cable_kind(a, b)?;
    let ok = match kind {
        CableKind::CopperStraight | CableKind::CopperCross => right.is_copper(),
        CableKind::FiberMm | CableKind::FiberSm => right.is_fiber(),
        k => k == right,
    };
    if ok {
        Ok(())
    } else {
        let m = if cable_media_ok(kind, a.media) { b.media } else { a.media };
        Err(Why::CableDoesNotFit(kind, m))
    }
}

fn cable_media_ok(kind: CableKind, m: Media) -> bool {
    match kind {
        CableKind::CopperStraight | CableKind::CopperCross => m == Media::Rj45,
        CableKind::FiberMm | CableKind::FiberSm => m.is_optical(),
        CableKind::Serial => m == Media::Serial,
        CableKind::Console => matches!(m, Media::Console | Media::Usb),
        CableKind::Wireless => m.is_radio(),
        CableKind::Wan => m == Media::Wan,
    }
}

/// the speed both ends agree on (auto-negotiation: the lower one)
pub fn link_kbps(a: &PortSpec, b: &PortSpec) -> u32 {
    a.kbps.min(b.kbps)
}

/// the propagation delay of a new link: 10 m of copper, 100 m of fiber, a regional serial or
/// WAN line, nothing for console and radio (the radio's airtime comes from its rate)
pub fn default_delay_ns(kind: CableKind) -> u64 {
    match kind {
        CableKind::CopperStraight | CableKind::CopperCross => 50,
        CableKind::FiberMm | CableKind::FiberSm => 500,
        CableKind::Serial => 2_000_000,
        CableKind::Wan => 5_000_000,
        CableKind::Console | CableKind::Wireless => 0,
    }
}

/// "Kupfer, gerade · 1G" for tooltips
pub fn describe(kind: CableKind, kbps: u32) -> String {
    format!("{} · {}", kind.title(), crate::catalog::speed_str(kbps))
}
