// phys -- the physical layer of the cables (P1): their lengths against the limits of their
// media, the optics at both ends of a fibre; a cable that is too long, or whose optics do not
// fit each other or the fibre, does not come up
//
//   Action::CableLen { link, len_m }        the cable's length in metres (action.rs; 0: unknown)
//   Action::Optic { dev, port, optic }      the transceiver in a port's cage
//   Engine::cable_phys(link) -> Option<CablePhys>   length, speed, reach and what keeps it down
//   PhysFault                               why a cable stays down (too long, the optics)
//   PhysNote                                the notes about it (Note::Phys, layer 1)
//
// The engine knows lengths only as numbers: from_project and Action::Connect take Link::len_m
// as the cable's length -- the project resolves automatic ones (jfn_model::phys::length: the
// app and load.rs put the result there) -- and Action::CableLen changes it; 0 means unknown and
// is never too long.  The limit is jfn_model::phys::reach for the cable's kind at the speed it
// runs (link_kbps) with the optic of its ends; WLAN associations, WAN lines and radio have none.
//
// Decision: a cable over its limit does not come up (no link with errors): IEEE 802.3 promises
// nothing beyond its reach, and a clear state teaches better.  It shows everywhere: both LEDs
// red, the ports down (IOS "down, line protocol is down" with a line in "show interfaces" that
// names the length and the limit, Linux NO-CARRIER in "ip link"), a note at both ends
// ("Kabel an Gi1/0/1 zu lang: 132 m, erlaubt 100 m (1000BASE-T); die Leitung bleibt aus."), a
// log line (IOS "%PHY-4-CABLE_LENGTH", the Linux kernel's), the problem in Engine::problems.
// Shortened again (or the optics fixed) it comes up as an unplugged cable does when it is
// plugged in.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::cable::CableKind;
use jfn_model::catalog::{CliStyle, Optic};
use jfn_model::msg;
use jfn_model::msg::{phrase, Msg};
use jfn_model::phys::{self as mphys, OpticFault, Reach};

use crate::action::ActErr;
use crate::kernel::Engine;
use crate::problems::{Problem, ProblemKind, Severity};
use crate::trace::Note;

/// why a cable stays down
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysFault {
    /// longer than its medium allows at its speed
    TooLong { len_m: u32, max_m: u32, std: &'static str },
    /// the optics of its ends do not fit each other or the fibre
    Optics(OpticFault),
}

/// the physical side of a cable (Engine::cable_phys)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CablePhys {
    /// its length in metres (0: unknown)
    pub len_m: u32,
    /// the speed it runs at
    pub kbps: u32,
    /// the limit of its medium (None: none)
    pub reach: Option<Reach>,
    /// what keeps it down (None: nothing physical)
    pub fault: Option<PhysFault>,
}

/// the lengths the engine was told and the cables that may not carry, by link slot
#[derive(Clone, Debug, Default)]
pub struct PhysState {
    pub(crate) len: BTreeMap<usize, u32>,
    pub(crate) bad: BTreeMap<usize, PhysFault>,
}

/// a decision about a cable (Note::Phys; ports are the noting device's)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhysNote {
    /// the cable on a port is longer than its medium allows: the link stays down
    TooLong { port: u16, len_m: u32, max_m: u32, std: &'static str },
    /// the optic of this end, the other end's, the fibre: they do not fit
    Optics { port: u16, own: Optic, other: Optic, fiber: CableKind },
}

impl PhysNote {
    /// the sentence as a message (`port` names a port index)
    pub fn message(&self, port: &dyn Fn(u16) -> String) -> Msg {
        match self {
            PhysNote::TooLong { port: p, len_m, max_m, std } => msg!("Kabel an {} zu lang: {} m, erlaubt {} m ({}); die Leitung bleibt aus.", port(*p), len_m, max_m, std),
            PhysNote::Optics { port: p, own, other, .. } if own != other => msg!("Optik {} an {} passt nicht zur Optik {} am anderen Ende; die Leitung bleibt aus.", own.short(), port(*p), other.short()),
            PhysNote::Optics { port: p, own, fiber, .. } => msg!("{} passt nicht zur Optik {} an {}; die Leitung bleibt aus.", phrase(fiber.title()), own.short(), port(*p)),
        }
    }
}

impl Engine {
    // what keeps link slot `li` down physically, from its length, speed, kind and optics
    fn phys_eval(&self, li: usize) -> Option<PhysFault> {
        let l = self.links.get(li)?.as_ref()?;
        if crate::wlan::is_assoc_id(l.id) {
            return None;
        }
        let spec = |(n, p): (usize, u16)| self.nodes.get(n)?.as_ref().and_then(|x| x.ports.get(p as usize)).map(|x| x.spec.optic);
        let (oa, ob) = (spec(l.ends[0]).flatten(), spec(l.ends[1]).flatten());
        if let Some(f) = mphys::optic_fault(l.cable, oa, ob) {
            return Some(PhysFault::Optics(f));
        }
        let len = self.phys.len.get(&li).copied().unwrap_or(0);
        let r = mphys::reach(l.cable, self.link_kbps(li), oa)?;
        if len > r.max_m { Some(PhysFault::TooLong { len_m: len, max_m: r.max_m, std: r.std }) } else { None }
    }

    /// link slot `li` may not carry for a physical reason (link_up)
    pub(crate) fn phys_bad(&self, li: usize) -> bool {
        !self.phys.bad.is_empty() && self.phys.bad.contains_key(&li)
    }

    /// a new cable: its length (0: unknown) and what follows from it, before it is refreshed
    pub(crate) fn phys_init(&mut self, li: usize, len_m: u32) {
        if len_m > 0 {
            self.phys.len.insert(li, len_m);
        }
        match self.phys_eval(li) {
            Some(f) => {
                self.phys.bad.insert(li, f);
            }
            None => {
                self.phys.bad.remove(&li);
            }
        }
    }

    /// after a change of a cable's length, speed, kind or optics: the fault again, the link
    /// state after it (a cable that stays down for another reason gets no notes)
    pub(crate) fn phys_update(&mut self, li: usize) {
        match self.phys_eval(li) {
            Some(f) => {
                self.phys.bad.insert(li, f);
            }
            None => {
                self.phys.bad.remove(&li);
            }
        }
        self.refresh_link(li);
    }

    /// a cable went away
    pub(crate) fn phys_forget(&mut self, li: usize) {
        self.phys.len.remove(&li);
        self.phys.bad.remove(&li);
    }

    /// the notes and log lines of a cable that is down for a physical reason (refresh_link
    /// when it goes down, add_link for one that never came up): at both ends
    pub(crate) fn phys_down(&mut self, li: usize) {
        let Some(&f) = self.phys.bad.get(&li) else { return };
        let Some(Some(l)) = self.links.get(li) else { return };
        let (ends, fiber) = (l.ends, l.cable);
        let optic = |e: &Engine, (n, p): (usize, u16)| e.nodes.get(n).and_then(|x| x.as_ref()).and_then(|x| x.ports.get(p as usize)).and_then(|x| x.spec.optic).unwrap_or(Optic::Sr);
        for k in 0..2 {
            let (ni, pi) = ends[k];
            let (dev, name, style, power) = match self.nodes.get(ni) {
                Some(Some(n)) => (n.dev, String::from(n.port_name(pi)), n.cli.style, n.power),
                _ => continue,
            };
            if !power {
                continue;
            }
            let (note, log) = match f {
                PhysFault::TooLong { len_m, max_m, std } => (
                    PhysNote::TooLong { port: pi, len_m, max_m, std },
                    if style == CliStyle::Ios {
                        format!("%PHY-4-CABLE_LENGTH: {}: the cable is {} m long, {} allows {} m; the link stays down", name, len_m, std, max_m)
                    } else {
                        format!("{}: cable of {} m longer than the {} m {} allows, no carrier", name, len_m, max_m, std)
                    },
                ),
                PhysFault::Optics(_) => {
                    let (own, other) = (optic(self, ends[k]), optic(self, ends[1 - k]));
                    let log = if style == CliStyle::Ios {
                        if own != other {
                            format!("%PHY-4-OPTICS: {}: {} optics here, {} at the far end; the link stays down", name, own.short(), other.short())
                        } else {
                            format!("%PHY-4-OPTICS: {}: {} optics on the wrong fibre; the link stays down", name, own.short())
                        }
                    } else if own != other {
                        format!("{}: {} transceiver, {} at the far end, no carrier", name, own.short(), other.short())
                    } else {
                        format!("{}: {} transceiver on the wrong fibre, no carrier", name, own.short())
                    };
                    (PhysNote::Optics { port: pi, own, other, fiber }, log)
                }
            };
            self.note(dev, None, Note::Phys(note));
            if style == CliStyle::Ios {
                self.ios_log(ni, &log);
            } else if self.os(ni).is_some() {
                self.os_log(ni, 4, "kernel", &log, None);
            }
        }
    }

    /// Action::CableLen
    pub(crate) fn set_cable_len(&mut self, link: u32, len_m: u32) -> Result<(), ActErr> {
        let li = *self.link_idx.get(&link).ok_or(ActErr::UnknownLink(link))?;
        if self.phys.len.get(&li).copied().unwrap_or(0) == len_m {
            return Ok(());
        }
        if len_m == 0 {
            self.phys.len.remove(&li);
        } else {
            self.phys.len.insert(li, len_m);
        }
        self.phys_update(li);
        Ok(())
    }

    /// Action::Optic
    pub(crate) fn set_optic(&mut self, dev: u32, port: &str, optic: Optic) -> Result<(), ActErr> {
        let (ni, pi) = self.resolve(dev, port)?;
        let li = match self.nodes.get_mut(ni) {
            Some(Some(n)) => {
                let p = &mut n.ports[pi as usize];
                if p.spec.optic.is_none() {
                    return Err(ActErr::Refused(msg!("{} hat keinen Schacht für eine Optik.", p.spec.name)));
                }
                p.spec.optic = Some(optic);
                p.link
            }
            _ => return Err(ActErr::UnknownDevice(dev)),
        };
        if let Some(li) = li {
            self.phys_update(li);
        }
        Ok(())
    }

    /// the physical side of a cable of the project
    pub fn cable_phys(&self, link: u32) -> Option<CablePhys> {
        let &li = self.link_idx.get(&link)?;
        let l = self.links.get(li)?.as_ref()?;
        let kbps = self.link_kbps(li);
        let optic = self.nodes.get(l.ends[0].0).and_then(|x| x.as_ref()).and_then(|x| x.ports.get(l.ends[0].1 as usize)).and_then(|x| x.spec.optic);
        Some(CablePhys {
            len_m: self.phys.len.get(&li).copied().unwrap_or(0),
            kbps,
            reach: if crate::wlan::is_assoc_id(l.id) { None } else { mphys::reach(l.cable, kbps, optic) },
            fault: self.phys.bad.get(&li).copied(),
        })
    }

    /// the problems: cables too long, optics that do not fit (problems.rs)
    pub(crate) fn phys_problems(&self, out: &mut Vec<Problem>) {
        for (&li, f) in &self.phys.bad {
            let Some(Some(l)) = self.links.get(li) else { continue };
            let dev = |k: usize| self.dev_of(l.ends[k].0);
            let name = |k: usize| self.end_name((dev(k), l.ends[k].1));
            let optic = |k: usize| self.nodes.get(l.ends[k].0).and_then(|x| x.as_ref()).and_then(|x| x.ports.get(l.ends[k].1 as usize)).and_then(|x| x.spec.optic).unwrap_or(Optic::Sr);
            let (kind, m) = match *f {
                PhysFault::TooLong { len_m, max_m, std } => (ProblemKind::CableTooLong, msg!("Kabel zu lang: {} m, erlaubt {} m ({} zwischen {} und {}); die Leitung bleibt aus.", len_m, max_m, std, name(0), name(1))),
                PhysFault::Optics(OpticFault::Mismatch(..)) => {
                    (ProblemKind::OpticMismatch, msg!("Optiken passen nicht: {} an {}, {} an {}; die Leitung bleibt aus.", optic(0).short(), name(0), optic(1).short(), name(1)))
                }
                PhysFault::Optics(OpticFault::WrongFiber(o)) => (
                    ProblemKind::OpticMismatch,
                    msg!("{} zwischen {} und {} passt nicht zur Optik {} (SR braucht Multimode, LR Singlemode); die Leitung bleibt aus.", phrase(l.cable.title()), name(0), name(1), o.short()),
                ),
            };
            out.push(Problem { kind, severity: Severity::Error, dev: dev(0), port: Some(l.ends[0].1), link: Some(l.id), others: alloc::vec![dev(1)], text: m.text(), msg: m });
        }
    }
}
