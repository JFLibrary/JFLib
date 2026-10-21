// fault -- fault injection for the lab: something breaks now, perhaps only for a while, every
// fault becomes a mark of the timeline and can be healed; healing undoes exactly what the
// fault did, also when faults overlap
//
//   Action::Fault { fault, dur }   starts a fault (dur: it heals by itself after that long;
//                                  Engine::next_fault_id() tells the number it gets)
//   Action::Heal { id } / HealAll
//   Engine::faults()               every fault: number, what, start, planned end, healed, text
//   Engine::active_faults()
//
//   Fault::Unplug { link }         the cable is pulled
//   Fault::Impair { link, .. }     the cable gets worse: delay and jitter on top of its own,
//                                  loss combined with its own, bandwidth capped
//   Fault::Power { dev }           the device goes off (and on again when healed, if it was on)
//   Fault::Stop { dev, svc }       a service stops (and starts again if it ran)
//   Fault::SlowDisk { dev, .. }    the disk of the device takes longer (disk.rs)
//   Fault::Partition { a, b }      devices of one side drop the IPv4 packets of devices of the
//                                  other side (b empty: every device outside a), as a firewall
//                                  rule on each of them would
//
// The faults do not overwrite settings: a cable's own values stay as they are and the faults
// lie on top of them (link.rs asks link_over, l3.rs asks partition_drop), the disk keeps its
// settings and gets a slow-down from the faults of its device.  Power and service faults
// count: the device comes back on when the last power fault of it heals.  Each start and end
// is a note (FaultStart, FaultEnd) of the device it belongs to; the timer of a fault with a
// duration is a kernel event (Ev::Heal).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::ifname;
use netpkt::Ipv4;

use crate::action::{ActErr, Action};
use crate::kernel::{Engine, Ev, Ns, MS};
use crate::trace::{fmt_ns, Note};

/// marks kept at most (the oldest healed ones go first)
pub const MARKS_KEEP: usize = 2000;

/// what breaks
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// the cable (project link id) is pulled
    Unplug { link: u32 },
    /// the cable gets worse: `delay` and `jitter` are added to its own, `loss_ppm` combines
    /// with its own, `kbps` caps its speed
    Impair { link: u32, delay: Ns, jitter: Ns, loss_ppm: u32, kbps: Option<u32> },
    /// the device is switched off
    Power { dev: u32 },
    /// a service of the device stops
    Stop { dev: u32, svc: String },
    /// the disk of the device: `extra` latency per access, the transfer time in per mille
    /// (3000: three times as long)
    SlowDisk { dev: u32, extra: Ns, factor_pm: u32 },
    /// devices of `a` and of `b` no longer hear each other (b empty: every device not in a)
    Partition { a: Vec<u32>, b: Vec<u32> },
}

impl Fault {
    /// the kind as a word ("unplug", "impair", "power", "stop", "disk", "partition")
    pub fn kind(&self) -> &'static str {
        match self {
            Fault::Unplug { .. } => "unplug",
            Fault::Impair { .. } => "impair",
            Fault::Power { .. } => "power",
            Fault::Stop { .. } => "stop",
            Fault::SlowDisk { .. } => "disk",
            Fault::Partition { .. } => "partition",
        }
    }
}

/// a fault on the timeline
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaultMark {
    pub id: u32,
    pub fault: Fault,
    /// when it started
    pub at: Ns,
    /// when it heals by itself
    pub until: Option<Ns>,
    /// when it was healed
    pub healed: Option<Ns>,
    /// German, for the timeline and reports ("KV2 ausgeschaltet")
    pub text: String,
    /// the device whose notes tell its start and end
    pub dev: u32,
}

impl FaultMark {
    pub fn active(&self) -> bool {
        self.healed.is_none()
    }
}

/// what the faults do to a cable now
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkOver {
    /// pulled by this many faults
    pub cut: u32,
    pub delay: Ns,
    pub jitter: Ns,
    pub loss_ppm: u32,
    pub kbps: Option<u32>,
}

/// two loss rates one after the other (ppm)
pub fn combine_loss(a: u32, b: u32) -> u32 {
    let keep = (1_000_000 - a.min(1_000_000) as u64) * (1_000_000 - b.min(1_000_000) as u64) / 1_000_000;
    (1_000_000 - keep) as u32
}

/// the faults of an engine
#[derive(Clone, Debug, Default)]
pub struct Faults {
    marks: Vec<FaultMark>,
    /// the last number given
    last: u32,
    /// per link slot
    links: BTreeMap<usize, LinkOver>,
    /// per device: active power faults and whether it was on before the first
    power: BTreeMap<u32, (u32, bool)>,
    /// per device and service (lower case): active stop faults and whether it was enabled
    stops: BTreeMap<(u32, String), (u32, bool)>,
    /// active partitions: (number, side a, side b)
    parts: Vec<(u32, Vec<u32>, Vec<u32>)>,
}

impl Faults {
    /// what the faults do to a cable (link slot)
    pub fn link(&self, li: usize) -> Option<LinkOver> {
        self.links.get(&li).copied()
    }
    pub fn any_partition(&self) -> bool {
        !self.parts.is_empty()
    }
}

impl Engine {
    /// every fault (active and healed), oldest first
    pub fn faults(&self) -> &[FaultMark] {
        &self.faults.marks
    }
    /// the faults that are active now
    pub fn active_faults(&self) -> impl Iterator<Item = &FaultMark> {
        self.faults.marks.iter().filter(|m| m.active())
    }
    /// the number the next fault gets
    pub fn next_fault_id(&self) -> u32 {
        self.faults.last + 1
    }

    fn fault_dev_name(&self, dev: u32) -> String {
        self.node(dev).map(|n| n.name.clone()).unwrap_or_else(|| format!("{}", dev))
    }

    /// "R1 Gi0/0/1 – SW1 Gi1/0/1"
    fn link_text(&self, li: usize) -> String {
        let Some(Some(l)) = self.links.get(li) else {
            return String::from("?");
        };
        let end = |(n, p): (usize, u16)| match self.nodes.get(n) {
            Some(Some(node)) => format!("{} {}", node.name, ifname::short(node.port_name(p))),
            _ => String::from("?"),
        };
        format!("{} \u{2013} {}", end(l.ends[0]), end(l.ends[1]))
    }

    fn names(&self, devs: &[u32]) -> String {
        devs.iter().map(|&d| self.fault_dev_name(d)).collect::<Vec<_>>().join(", ")
    }

    /// the German line of a fault
    fn fault_text(&self, f: &Fault) -> String {
        let link = |id: u32| self.link_idx.get(&id).map(|&li| self.link_text(li)).unwrap_or_else(|| format!("Kabel {}", id));
        match f {
            Fault::Unplug { link: l } => format!("Kabel {} getrennt", link(*l)),
            Fault::Impair { link: l, delay, jitter, loss_ppm, kbps } => {
                let mut what = Vec::new();
                if *delay > 0 {
                    what.push(format!("+{} Laufzeit", fmt_ns(*delay as i64)));
                }
                if *jitter > 0 {
                    what.push(format!("+{} Jitter", fmt_ns(*jitter as i64)));
                }
                if *loss_ppm > 0 {
                    what.push(format!("{} Verlust", ppm_text(*loss_ppm)));
                }
                if let Some(k) = kbps {
                    what.push(format!("höchstens {}", jfn_model::catalog::speed_str(*k)));
                }
                if what.is_empty() {
                    what.push(String::from("unverändert"));
                }
                format!("Kabel {}: {}", link(*l), what.join(", "))
            }
            Fault::Power { dev } => format!("{} ausgeschaltet", self.fault_dev_name(*dev)),
            Fault::Stop { dev, svc } => format!("Dienst {} auf {} gestoppt", svc, self.fault_dev_name(*dev)),
            Fault::SlowDisk { dev, extra, factor_pm } => {
                let mut what = Vec::new();
                if *extra > 0 {
                    what.push(format!("+{} je Zugriff", fmt_ns(*extra as i64)));
                }
                if *factor_pm != 1000 {
                    what.push(format!("Übertragung \u{d7}{}", pm_text(*factor_pm)));
                }
                format!("Disk von {} langsam: {}", self.fault_dev_name(*dev), what.join(", "))
            }
            Fault::Partition { a, b } => {
                if b.is_empty() {
                    format!("Partition: {} | alle anderen", self.names(a))
                } else {
                    format!("Partition: {} | {}", self.names(a), self.names(b))
                }
            }
        }
    }

    /// the device a fault's notes go to
    fn fault_dev(&self, f: &Fault) -> u32 {
        match f {
            Fault::Unplug { link } | Fault::Impair { link, .. } => self
                .link_idx
                .get(link)
                .and_then(|&li| self.links.get(li)?.as_ref())
                .map(|l| self.dev_of(l.ends[0].0))
                .unwrap_or(0),
            Fault::Power { dev } | Fault::Stop { dev, .. } | Fault::SlowDisk { dev, .. } => *dev,
            Fault::Partition { a, .. } => a.first().copied().unwrap_or(0),
        }
    }

    /// what the active faults do to a cable, again
    fn relink(&mut self, link: u32) {
        let Some(&li) = self.link_idx.get(&link) else {
            return;
        };
        let mut o = LinkOver::default();
        for m in self.faults.marks.iter().filter(|m| m.active()) {
            match &m.fault {
                Fault::Unplug { link: l } if *l == link => o.cut += 1,
                Fault::Impair { link: l, delay, jitter, loss_ppm, kbps } if *l == link => {
                    o.delay += *delay;
                    o.jitter += *jitter;
                    o.loss_ppm = combine_loss(o.loss_ppm, *loss_ppm);
                    if let Some(k) = kbps {
                        o.kbps = Some(o.kbps.map_or(*k, |x| x.min(*k)));
                    }
                }
                _ => {}
            }
        }
        if o == LinkOver::default() {
            self.faults.links.remove(&li);
        } else {
            self.faults.links.insert(li, o);
        }
        self.refresh_link(li);
    }

    /// the slow-down of a device's disk from its active faults
    fn redisk(&mut self, dev: u32) {
        let (mut extra, mut pm) = (0u64, 1000u64);
        for m in self.faults.marks.iter().filter(|m| m.active()) {
            if let Fault::SlowDisk { dev: d, extra: e, factor_pm } = &m.fault
                && *d == dev
            {
                extra += *e;
                pm = (pm * *factor_pm as u64 / 1000).clamp(1, 1_000_000_000);
            }
        }
        if let Some(ni) = self.node_index(dev)
            && let Some(os) = self.os_mut(ni)
        {
            os.disk.slow_extra = extra;
            os.disk.slow_pm = pm.min(u32::MAX as u64) as u32;
        }
    }

    fn check_fault(&self, f: &Fault) -> Result<(), ActErr> {
        let dev_ok = |d: u32| if self.node_index(d).is_some() { Ok(()) } else { Err(ActErr::UnknownDevice(d)) };
        match f {
            Fault::Unplug { link } | Fault::Impair { link, .. } => {
                if !self.link_idx.contains_key(link) {
                    return Err(ActErr::UnknownLink(*link));
                }
            }
            Fault::Power { dev } => dev_ok(*dev)?,
            Fault::Stop { dev, svc } => {
                let ni = self.node_index(*dev).ok_or(ActErr::UnknownDevice(*dev))?;
                let os = self.os(ni).ok_or(ActErr::NoIp(*dev))?;
                if os.svc(svc).is_none_or(|s| s.console) {
                    return Err(ActErr::Refused(format!("Den Dienst \u{201e}{}\u{201c} gibt es auf {} nicht.", svc, self.fault_dev_name(*dev))));
                }
            }
            Fault::SlowDisk { dev, factor_pm, .. } => {
                let ni = self.node_index(*dev).ok_or(ActErr::UnknownDevice(*dev))?;
                self.os(ni).ok_or(ActErr::NoIp(*dev))?;
                if *factor_pm == 0 {
                    return Err(ActErr::Refused(String::from("Der Faktor einer langsamen Disk muss größer als 0 sein.")));
                }
            }
            Fault::Partition { a, b } => {
                if a.is_empty() {
                    return Err(ActErr::Refused(String::from("Eine Partition braucht auf einer Seite mindestens ein Gerät.")));
                }
                for &d in a.iter().chain(b.iter()) {
                    dev_ok(d)?;
                }
                if let Some(d) = a.iter().find(|d| b.contains(d)) {
                    return Err(ActErr::Refused(format!("{} steht auf beiden Seiten der Partition.", self.fault_dev_name(*d))));
                }
            }
        }
        Ok(())
    }

    /// starts a fault (Action::Fault): its number
    pub(crate) fn fault_start(&mut self, f: Fault, dur: Option<Ns>) -> Result<u32, ActErr> {
        let f = match f {
            Fault::Partition { mut a, mut b } => {
                a.sort();
                a.dedup();
                b.sort();
                b.dedup();
                Fault::Partition { a, b }
            }
            Fault::Impair { link, delay, jitter, loss_ppm, kbps } => Fault::Impair { link, delay, jitter, loss_ppm: loss_ppm.min(1_000_000), kbps: kbps.map(|k| k.max(1)) },
            f => f,
        };
        self.check_fault(&f)?;
        let now = self.now;
        self.faults.last += 1;
        let id = self.faults.last;
        let text = self.fault_text(&f);
        let dev = self.fault_dev(&f);
        let until = dur.map(|d| now + d.clamp(1, MAX_DUR));
        // the mark first: the overlays are computed from the active marks
        self.faults.marks.push(FaultMark { id, fault: f.clone(), at: now, until, healed: None, text: text.clone(), dev });
        if self.faults.marks.len() > MARKS_KEEP
            && let Some(i) = self.faults.marks.iter().position(|m| !m.active())
        {
            self.faults.marks.remove(i);
        }
        self.note(dev, None, Note::FaultStart { id, what: text });
        match &f {
            Fault::Unplug { link } | Fault::Impair { link, .. } => self.relink(*link),
            Fault::Power { dev } => {
                let on = self.node(*dev).is_some_and(|n| n.power);
                let e = self.faults.power.entry(*dev).or_insert((0, on));
                e.0 += 1;
                if on {
                    self.apply(Action::Power { dev: *dev, on: false })?;
                }
            }
            Fault::Stop { dev, svc } => {
                let ni = self.node_index(*dev).ok_or(ActErr::UnknownDevice(*dev))?;
                let enabled = self.os(ni).and_then(|o| o.svc(svc)).is_some_and(|s| s.enabled);
                let e = self.faults.stops.entry((*dev, svc.to_ascii_lowercase())).or_insert((0, enabled));
                e.0 += 1;
                if enabled {
                    self.set_service_running(*dev, svc, false)?;
                }
            }
            Fault::SlowDisk { dev, .. } => self.redisk(*dev),
            Fault::Partition { a, b } => self.faults.parts.push((id, a.clone(), b.clone())),
        }
        if let Some(t) = until {
            self.schedule(t, Ev::Heal { id });
        }
        Ok(id)
    }

    /// heals a fault (Action::Heal)
    pub(crate) fn fault_heal(&mut self, id: u32) -> Result<(), ActErr> {
        let now = self.now;
        let Some(m) = self.faults.marks.iter_mut().find(|m| m.id == id) else {
            return Err(ActErr::Refused(format!("Einen Fehler {} gibt es nicht.", id)));
        };
        if !m.active() {
            return Err(ActErr::Refused(format!("Fehler {} ist schon behoben.", id)));
        }
        m.healed = Some(now);
        let (f, dev, text) = (m.fault.clone(), m.dev, m.text.clone());
        self.note(dev, None, Note::FaultEnd { id, what: text });
        match f {
            Fault::Unplug { link } | Fault::Impair { link, .. } => self.relink(link),
            Fault::Power { dev } => {
                let left = match self.faults.power.get_mut(&dev) {
                    Some(e) => {
                        e.0 = e.0.saturating_sub(1);
                        (e.0, e.1)
                    }
                    None => (0, false),
                };
                if left.0 == 0 {
                    self.faults.power.remove(&dev);
                    if left.1 && self.node_index(dev).is_some() {
                        self.apply(Action::Power { dev, on: true })?;
                    }
                }
            }
            Fault::Stop { dev, svc } => {
                let key = (dev, svc.to_ascii_lowercase());
                let left = match self.faults.stops.get_mut(&key) {
                    Some(e) => {
                        e.0 = e.0.saturating_sub(1);
                        (e.0, e.1)
                    }
                    None => (0, false),
                };
                if left.0 == 0 {
                    self.faults.stops.remove(&key);
                    if left.1 && self.node_index(dev).and_then(|ni| self.os(ni)).is_some_and(|o| o.svc(&svc).is_some()) {
                        self.set_service_running(dev, &svc, true)?;
                    }
                }
            }
            Fault::SlowDisk { dev, .. } => self.redisk(dev),
            Fault::Partition { .. } => self.faults.parts.retain(|p| p.0 != id),
        }
        Ok(())
    }

    /// heals every active fault, the newest first
    pub(crate) fn fault_heal_all(&mut self) -> Result<(), ActErr> {
        let ids: Vec<u32> = self.faults.marks.iter().rev().filter(|m| m.active()).map(|m| m.id).collect();
        for id in ids {
            self.fault_heal(id)?;
        }
        Ok(())
    }

    /// the end of a fault's duration (Ev::Heal)
    pub(crate) fn fault_due(&mut self, id: u32) {
        let due = self.faults.marks.iter().any(|m| m.id == id && m.active() && m.until.is_some_and(|u| u <= self.now));
        if due {
            self.trace.new_batch();
            let _ = self.fault_heal(id);
        }
    }

    /// whether node `ni` drops an IPv4 packet from `src` because of a partition
    pub(crate) fn partition_drop(&self, ni: usize, src: Ipv4) -> bool {
        if self.faults.parts.is_empty() {
            return false;
        }
        let me = self.dev_of(ni);
        let has = |devs: &[u32]| devs.iter().any(|&d| self.node_index(d).and_then(|n| self.stack(n)).is_some_and(|s| s.is_mine(src)));
        self.faults.parts.iter().any(|(_, a, b)| {
            let in_a = a.contains(&me);
            let in_b = if b.is_empty() { !in_a } else { b.contains(&me) };
            let from_a = has(a);
            let from_b = if b.is_empty() { !from_a } else { has(b) };
            (in_a && from_b) || (in_b && from_a)
        })
    }
}

/// "5 %", "0,5 %", "250 ppm"
pub fn ppm_text(ppm: u32) -> String {
    if ppm % 10_000 == 0 {
        format!("{} %", ppm / 10_000)
    } else if ppm % 1000 == 0 {
        format!("{},{} %", ppm / 10_000, ppm % 10_000 / 1000)
    } else {
        format!("{} ppm", ppm)
    }
}

/// "3", "1,5" from per mille
fn pm_text(pm: u32) -> String {
    if pm % 1000 == 0 { format!("{}", pm / 1000) } else { format!("{},{}", pm / 1000, (pm % 1000) / 100) }
}

/// the longest a fault may last (a day of virtual time)
pub const MAX_DUR: Ns = 86_400_000 * MS;
