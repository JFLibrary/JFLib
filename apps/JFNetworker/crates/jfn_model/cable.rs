// cable -- which cable joins two ports, and what a link of that cable defaults to
//
//   cable_kind(a, b) -> Result<CableKind, Why>     the correct cable ("Automatisch")
//   cable_fits(kind, a, b) -> Result<(), Why>      a cable the user chose: does it plug in?
//   needs_crossover(a, b)                          copper between two MDI or two MDI-X ports
//   link_kbps(a, b) / default_delay_ns(kind)       the speed both ends agree on, the delay
//   accepts(kind, media)                           does the plug of a kind go into a port?
//   CableKind::title / tile / max_len_m            German name, the palette's short name, the
//                                                  medium's length limit
//   CableKind::is_serial / dce_end                 a serial line, the end a chosen one has as DCE
//   CableGrade::name / fits(kind)                  the category of a cable (Cat5e .. OS2)
//   CableColor::title                              a colour the user gave a cable
//   Why::msg() / describe_msg(kind, kbps)           the reason and the tooltip text as a Msg
//                                                  (Display and describe(): their German text)
//   limits_msg(kind, grade)                        the kind's limits for a tooltip
//
// Copper: a straight cable joins MDI (host, router) and MDI-X (switch) ports, a crossover cable
// two of the same kind.  With auto-MDI-X (the default of every multi-gigabit port, and of the
// project unless "strict_mdix" is set) both cables work; the simulator decides that, this
// module only names the right one.  Optics: SFP, SFP+ and SFP28 cages take each other's
// optics at the lower speed; QSFP28 only joins QSFP28.  Radios: a client associates with an
// access point (no ad-hoc mode).  The optics in the cages (PortSpec::optic, P1) pick the fibre:
// long-reach (LR) at both ends single-mode, else multimode; the lengths each medium allows at a
// speed are phys::reach.
//
// H2 appends kinds, so old files keep what they said.  "serial" has no chosen end, as before;
// "serial-dce" and "serial-dte" name the first end (Link::a) of a serial line.  That is all they
// do: the engine has no clock on serial lines, an end only shows in the drawing and in "show
// interfaces".  A telephone cable (RJ11) and a coaxial cable join modems and the provider's WAN
// ports (Media::Wan is "coax / DSL / GPON" already).  DAC and AOC are cables with their
// transceivers built in: no optics count, SFP+ and SFP28 cages among each other, QSFP28 only with
// QSFP28, and no plain SFP.  A cable may also say its category (Link::grade, CableGrade::Unset:
// none, the limits as before; phys::reach_graded) and a colour for the eye (Link::color, the
// simulator never sees it; status stays on the LEDs).

use alloc::string::String;
use core::fmt;

use agent_core::wire_enum;

use crate::catalog::{Media, PortSpec};
use crate::msg::{phrase, Msg};
use crate::msg;

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
        SerialDce = "serial-dce",
        SerialDte = "serial-dte",
        Phone = "phone",
        Coax = "coax",
        Dac = "dac",
        Aoc = "aoc",
    }
}

wire_enum! {
    /// the category of a copper or fibre cable: what it carries how far (phys::reach_graded);
    /// Unset says nothing and the limits of its kind count as before
    pub enum CableGrade {
        Unset = "",
        Cat5e = "cat5e",
        Cat6 = "cat6",
        Cat6a = "cat6a",
        Cat7 = "cat7",
        Cat8 = "cat8",
        Om3 = "om3",
        Om4 = "om4",
        Os2 = "os2",
    }
}

impl CableGrade {
    /// the name on the cable ("Cat6a"; Unset: none)
    pub fn name(self) -> &'static str {
        match self {
            CableGrade::Unset => "",
            CableGrade::Cat5e => "Cat5e",
            CableGrade::Cat6 => "Cat6",
            CableGrade::Cat6a => "Cat6a",
            CableGrade::Cat7 => "Cat7",
            CableGrade::Cat8 => "Cat8",
            CableGrade::Om3 => "OM3",
            CableGrade::Om4 => "OM4",
            CableGrade::Os2 => "OS2",
        }
    }
    /// a cable of this kind can have this category (one that cannot counts as Unset)
    pub fn fits(self, kind: CableKind) -> bool {
        match self {
            CableGrade::Unset => true,
            CableGrade::Cat5e | CableGrade::Cat6 | CableGrade::Cat6a | CableGrade::Cat7 | CableGrade::Cat8 => kind.is_copper(),
            CableGrade::Om3 | CableGrade::Om4 => kind == CableKind::FiberMm,
            CableGrade::Os2 => kind == CableKind::FiberSm,
        }
    }
}

wire_enum! {
    /// a colour the user gave a cable to tell it apart (Plain: its kind's own)
    pub enum CableColor {
        Plain = "",
        Blue = "blue",
        Teal = "teal",
        Violet = "violet",
        Pink = "pink",
        Brown = "brown",
        Slate = "slate",
    }
}

impl CableColor {
    /// the German name (the app translates it)
    pub fn title(self) -> &'static str {
        // i18n
        match self {
            CableColor::Plain => "Farbe der Kabelart",
            CableColor::Blue => "Blau",
            CableColor::Teal => "Türkis",
            CableColor::Violet => "Violett",
            CableColor::Pink => "Pink",
            CableColor::Brown => "Braun",
            CableColor::Slate => "Schiefer",
        }
    }
}

impl CableKind {
    /// the German name (the app translates it)
    pub fn title(self) -> &'static str {
        // i18n
        match self {
            CableKind::CopperStraight => "Kupfer, gerade",
            CableKind::CopperCross => "Kupfer, gekreuzt",
            CableKind::FiberMm => "Glasfaser, Multimode",
            CableKind::FiberSm => "Glasfaser, Singlemode",
            CableKind::Serial => "Seriell (WAN)",
            CableKind::Console => "Konsolenkabel",
            CableKind::Wireless => "Funk (WLAN)",
            CableKind::Wan => "WAN-Leitung",
            CableKind::SerialDce => "Seriell, erstes Ende DCE",
            CableKind::SerialDte => "Seriell, erstes Ende DTE",
            CableKind::Phone => "Telefonkabel (RJ11)",
            CableKind::Coax => "Koaxialkabel",
            CableKind::Dac => "Direktkabel (DAC)",
            CableKind::Aoc => "Aktives Glasfaserkabel (AOC)",
        }
    }
    /// the short name on a palette tile (the app translates it)
    pub fn tile(self) -> &'static str {
        // i18n
        match self {
            CableKind::CopperStraight => "Kupfer, gerade",
            CableKind::CopperCross => "Kupfer, gekreuzt",
            CableKind::FiberMm => "Multimode",
            CableKind::FiberSm => "Singlemode",
            CableKind::Serial => "Seriell",
            CableKind::Console => "Konsole",
            CableKind::Wireless => "Funk",
            CableKind::Wan => "WAN",
            CableKind::SerialDce => "Seriell DCE",
            CableKind::SerialDte => "Seriell DTE",
            CableKind::Phone => "Telefon",
            CableKind::Coax => "Koax",
            CableKind::Dac => "DAC",
            CableKind::Aoc => "AOC",
        }
    }
    pub fn is_copper(self) -> bool {
        matches!(self, CableKind::CopperStraight | CableKind::CopperCross)
    }
    pub fn is_fiber(self) -> bool {
        matches!(self, CableKind::FiberMm | CableKind::FiberSm)
    }
    /// a serial line, with or without a chosen end
    pub fn is_serial(self) -> bool {
        matches!(self, CableKind::Serial | CableKind::SerialDce | CableKind::SerialDte)
    }
    /// the end (0: Link::a, 1: Link::b) a serial line of a chosen end has as DCE
    pub fn dce_end(self) -> Option<usize> {
        match self {
            CableKind::SerialDce => Some(0),
            CableKind::SerialDte => Some(1),
            _ => None,
        }
    }
    /// its delay follows its length (phys::delay_ns)
    pub fn delay_follows_length(self) -> bool {
        self.is_copper() || self.is_fiber() || matches!(self, CableKind::Dac | CableKind::Aoc)
    }
    /// carries Ethernet frames (not serial, not console)
    pub fn is_ethernet(self) -> bool {
        !self.is_serial() && self != CableKind::Console
    }
    /// the longest cable of this kind at a speed with the optics that belong to it (None: no
    /// limit modelled; the table is phys::reach)
    pub fn max_len_m(self, kbps: u32) -> Option<u32> {
        let optic = match self {
            CableKind::FiberSm => Some(crate::catalog::Optic::Lr),
            CableKind::FiberMm => Some(crate::catalog::Optic::Sr),
            _ => None,
        };
        crate::phys::reach(self, kbps, optic).map(|r| r.max_m)
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

impl Why {
    /// the reason as a message (Display: its German text)
    pub fn msg(&self) -> Msg {
        match self {
            Why::SameDevice => msg!("Ein Kabel braucht zwei verschiedene Geräte."),
            Why::PortInUse(p) => msg!("Der Port {} ist schon belegt.", p),
            Why::MediaMismatch(a, b) => msg!("{} und {} passen nicht zusammen.", phrase(a.name()), phrase(b.name())),
            Why::NoAccessPoint => msg!("Funk verbindet ein WLAN-Gerät mit einem Access Point."),
            Why::CableDoesNotFit(c, m) => msg!("{} passt nicht in einen {}-Port.", phrase(c.title()), phrase(m.name())),
        }
    }
}

impl fmt::Display for Why {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.msg().text())
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
                // long-reach optics at both ends take single-mode fibre (P1); optics that do
                // not match still plug in, the link stays down (phys.rs)
                let lr = |p: &PortSpec| p.optic == Some(crate::catalog::Optic::Lr);
                Ok(if lr(a) && lr(b) { CableKind::FiberSm } else { CableKind::FiberMm })
            } else {
                Err(Why::MediaMismatch(x, y))
            }
        }
        (Serial, Serial) => Ok(CableKind::Serial),
        (Console, Usb) | (Usb, Console) => Ok(CableKind::Console),
        (RadioAp, RadioClient) | (RadioClient, RadioAp) => Ok(CableKind::Wireless),
        (RadioAp, RadioAp) | (RadioClient, RadioClient) => Err(Why::NoAccessPoint),
        (Wan, Wan) => Ok(CableKind::Wan),
        (Rj11, Rj11) | (Rj11, Wan) | (Wan, Rj11) => Ok(CableKind::Phone),
        (Coax, Coax) | (Coax, Wan) | (Wan, Coax) => Ok(CableKind::Coax),
        (x, y) => Err(Why::MediaMismatch(x, y)),
    }
}

/// does a chosen cable plug into both ports? (copper: either straight or crossover)
pub fn cable_fits(kind: CableKind, a: &PortSpec, b: &PortSpec) -> Result<(), Why> {
    let right = cable_kind(a, b)?;
    let ok = match kind {
        CableKind::CopperStraight | CableKind::CopperCross => right.is_copper(),
        CableKind::FiberMm | CableKind::FiberSm => right.is_fiber(),
        // no optics: both cages must take the plug, and cages that join at all
        CableKind::Dac | CableKind::Aoc => right.is_fiber() && accepts(kind, a.media) && accepts(kind, b.media),
        CableKind::Serial | CableKind::SerialDce | CableKind::SerialDte => right.is_serial(),
        // a line between two WAN ports may be either
        CableKind::Phone | CableKind::Coax => kind == right || right == CableKind::Wan,
        k => k == right,
    };
    if ok {
        Ok(())
    } else {
        let m = if accepts(kind, a.media) { b.media } else { a.media };
        Err(Why::CableDoesNotFit(kind, m))
    }
}

/// does the plug of a cable of this kind go into a port of this medium?
pub fn accepts(kind: CableKind, m: Media) -> bool {
    match kind {
        CableKind::CopperStraight | CableKind::CopperCross => m == Media::Rj45,
        CableKind::FiberMm | CableKind::FiberSm => m.is_optical(),
        CableKind::Serial | CableKind::SerialDce | CableKind::SerialDte => m == Media::Serial,
        CableKind::Console => matches!(m, Media::Console | Media::Usb),
        CableKind::Wireless => m.is_radio(),
        CableKind::Wan => m == Media::Wan,
        CableKind::Phone => matches!(m, Media::Rj11 | Media::Wan),
        CableKind::Coax => matches!(m, Media::Coax | Media::Wan),
        CableKind::Dac | CableKind::Aoc => matches!(m, Media::SfpPlus | Media::Sfp28 | Media::Qsfp28),
    }
}

/// the speed both ends agree on (auto-negotiation: the lower one)
pub fn link_kbps(a: &PortSpec, b: &PortSpec) -> u32 {
    a.kbps.min(b.kbps)
}

/// the propagation delay of a new link: 10 m of copper, 100 m of fiber, a regional serial or
/// WAN line, nothing for console and radio (the radio's airtime comes from its rate), 3 m of DAC,
/// 10 m of AOC, a provider's line for telephone and coax
pub fn default_delay_ns(kind: CableKind) -> u64 {
    match kind {
        CableKind::CopperStraight | CableKind::CopperCross => 50,
        CableKind::FiberMm | CableKind::FiberSm => 500,
        CableKind::Serial | CableKind::SerialDce | CableKind::SerialDte => 2_000_000,
        CableKind::Wan | CableKind::Phone | CableKind::Coax => 5_000_000,
        CableKind::Console | CableKind::Wireless => 0,
        CableKind::Dac => 15,
        CableKind::Aoc => 50,
    }
}

/// "Kupfer, gerade · 1G" for tooltips
pub fn describe(kind: CableKind, kbps: u32) -> String {
    describe_msg(kind, kbps).text()
}

/// describe() as a message
pub fn describe_msg(kind: CableKind, kbps: u32) -> Msg {
    msg!("{} · {}", phrase(kind.title()), crate::catalog::speed_msg(kbps))
}

/// what a kind of a category allows, from phys::reach: "100 m bei 1G, 55 m bei 10G, nicht bei
/// 25G" (radio and WAN lines: no limit)
pub fn limits_msg(kind: CableKind, grade: CableGrade) -> Msg {
    use crate::catalog::{speed_msg, Optic, K100G, K10G, K1G, K25G};
    let speeds: &[u32] = match kind {
        CableKind::CopperStraight | CableKind::CopperCross => &[K1G, K10G, K25G],
        CableKind::FiberMm => &[K1G, K10G, K25G, K100G],
        CableKind::FiberSm => &[K1G, K10G],
        CableKind::Serial | CableKind::SerialDce | CableKind::SerialDte => &[64, 2048],
        CableKind::Console => &[115],
        CableKind::Phone => &[100_000],
        CableKind::Coax => &[K1G],
        CableKind::Dac | CableKind::Aoc => &[K10G, K100G],
        CableKind::Wireless | CableKind::Wan => &[],
    };
    if speeds.is_empty() {
        return phrase("keine Längengrenze");
    }
    let optic = if kind == CableKind::FiberSm { Some(Optic::Lr) } else { None };
    let part = |k: &u32| match crate::phys::reach_graded(kind, grade, *k, optic) {
        Some(r) if r.max_m > 0 => msg!("{} m bei {}", r.max_m, speed_msg(*k)),
        _ => msg!("nicht bei {}", speed_msg(*k)),
    };
    crate::msg::list(speeds.iter().map(part).collect())
}
