// link -- cables at run time: who is connected, how fast, how far, how lossy; sending, the
// queue of each direction, arrival, and the link state
//
//   LinkRt                 both ends, cable, speed, delay, jitter, loss, unplugged, cable fit
//   Engine::send(n, p, f)  a node puts a frame on its port: serialise at once or queue
//   Engine::link_up(l)     powered ends, ports not shut down (nor err-disabled), plugged in, the
//                          cable works
//   Engine::refresh_link   after any change: notes LinkUp / LinkDown, drops what was on the
//                          cable, flushes the bridges' MAC entries of that port, tells spanning
//                          tree (stp.rs)
//
// Serialisation of Ethernet takes the frame (at least 60 bytes) + FCS + preamble + inter-frame
// gap: (max(len, 60) + 24) * 8 bits at the link speed.  The receiver gets the frame when its
// last bit arrives: start + serialisation + delay (+ jitter).  A queue holds up to 2 MiB per
// direction (tail drop beyond); a waiting frame (Queued) keeps the cause and batch of its
// sending for the trace.

use alloc::collections::VecDeque;

use base::rng::Pcg32;
use jfn_model::cable::{self, CableKind};
use jfn_model::catalog::{Media, F_BRIDGE, F_HUB};
use netpkt::decode;

use crate::kernel::{Engine, Ev, Frame, Ns};
use crate::trace::{DevEvent, Hop, HopId, Note};

/// bytes a direction may queue before it drops
pub const QUEUE_BYTES: usize = 2 << 20;

/// a frame waiting for its cable, with what its Hop will say about it
#[derive(Clone, Debug)]
pub struct Queued {
    pub frame: Frame,
    pub cause: Option<HopId>,
    pub batch: u64,
    pub at: Ns,
}

/// one direction of a cable
#[derive(Clone, Debug, Default)]
pub struct Dir {
    pub queue: VecDeque<Queued>,
    pub queued_bytes: usize,
    pub busy_until: Ns,
}

/// a cable at run time
#[derive(Clone, Debug)]
pub struct LinkRt {
    /// project link id
    pub id: u32,
    /// (node slot, port index) of both ends
    pub ends: [(usize, u16); 2],
    pub cable: CableKind,
    /// configured speed (the ports may be slower)
    pub kbps: u32,
    pub delay_ns: Ns,
    pub jitter_ns: Ns,
    pub loss_ppm: u32,
    /// unplugged on purpose
    pub down: bool,
    /// a copper cable of the wrong kind (only matters with strict MDI-X)
    pub wrong_cable: bool,
    /// changes whenever frames on the cable must be lost
    pub(crate) epoch: u32,
    pub(crate) was_up: bool,
    pub(crate) dir: [Dir; 2],
    pub(crate) rng: Pcg32,
}

impl LinkRt {
    /// the end at node slot `n`, port `p`: 0 or 1
    pub fn end_of(&self, n: usize, p: u16) -> Option<u8> {
        if self.ends[0] == (n, p) {
            Some(0)
        } else if self.ends[1] == (n, p) {
            Some(1)
        } else {
            None
        }
    }
    /// frames waiting in both directions
    pub fn queued(&self) -> usize {
        self.dir[0].queue.len() + self.dir[1].queue.len()
    }
}

/// time on the wire of a frame of `len` bytes at `kbps`
pub fn serialisation_ns(len: usize, kbps: u32, ethernet: bool) -> Ns {
    let wire = if ethernet { len.max(60) + 24 } else { len } as u128;
    let bps = (kbps.max(1) as u128) * 1000;
    ((wire * 8 * 1_000_000_000) / bps) as Ns
}

impl Engine {
    /// the speed the link runs at: the cable setting and both ports' speeds
    pub fn link_kbps(&self, li: usize) -> u32 {
        let Some(Some(l)) = self.links.get(li) else {
            return 0;
        };
        let mut k = l.kbps;
        for &(n, p) in &l.ends {
            if let Some(Some(node)) = self.nodes.get(n) {
                k = k.min(node.ports[p as usize].spec.kbps.max(1));
            }
        }
        k
    }

    /// the link carries traffic
    pub fn link_up(&self, li: usize) -> bool {
        let Some(Some(l)) = self.links.get(li) else {
            return false;
        };
        if l.down || (l.wrong_cable && self.strict_mdix) || self.faults.link(li).is_some_and(|o| o.cut > 0) {
            return false;
        }
        l.ends.iter().all(|&(n, p)| match self.nodes.get(n) {
            Some(Some(node)) => node.power && node.ports.get(p as usize).is_some_and(|port| port.admin_up && !port.err_disabled),
            _ => false,
        })
    }

    pub(crate) fn note(&mut self, dev: u32, hop: Option<HopId>, note: Note) {
        if crate::cli::wants(&self.cli, &note) {
            self.cli_note(dev, hop, &note);
        }
        let t = self.now;
        let hop = hop.filter(|&h| h != 0);
        self.trace.push_event(DevEvent { t, dev, hop, note, seq: 0, batch: 0 });
    }

    /// re-evaluates a link after a change of power, port state, cable or plug
    pub(crate) fn refresh_link(&mut self, li: usize) {
        let up = self.link_up(li);
        let Some(Some(l)) = self.links.get_mut(li) else {
            return;
        };
        if up == l.was_up {
            return;
        }
        l.was_up = up;
        let ends = l.ends;
        if !up {
            // whatever is on the cable or waiting for it is lost
            l.epoch = l.epoch.wrapping_add(1);
            for d in l.dir.iter_mut() {
                d.queue.clear();
                d.queued_bytes = 0;
                d.busy_until = 0;
            }
        }
        for (n, p) in ends {
            let Some(Some(node)) = self.nodes.get_mut(n) else {
                continue;
            };
            if !up && let Some(b) = node.bridge.as_mut() {
                b.flush_port(p);
            }
            let dev = node.dev;
            if node.power {
                self.note(dev, None, if up { Note::LinkUp { port: p } } else { Note::LinkDown { port: p } });
            }
        }
        if !up {
            for (n, p) in ends {
                self.l3_port_down(n, p);
            }
        }
        for (n, p) in ends {
            self.os_link(n, p, up);
        }
        for (n, p) in ends {
            self.stp_link(n, p);
        }
    }

    /// re-evaluates every link of a node
    pub(crate) fn refresh_node_links(&mut self, ni: usize) {
        let links: alloc::vec::Vec<usize> = match self.nodes.get(ni) {
            Some(Some(n)) => n.ports.iter().filter_map(|p| p.link).collect(),
            _ => return,
        };
        for li in links {
            self.refresh_link(li);
        }
    }

    /// node `ni` sends `frame` out of port `pi`
    pub(crate) fn send(&mut self, ni: usize, pi: u16, frame: Frame, cause: Option<HopId>) {
        let Some(Some(node)) = self.nodes.get_mut(ni) else {
            return;
        };
        let dev = node.dev;
        let Some(port) = node.ports.get_mut(pi as usize) else {
            return;
        };
        if matches!(port.spec.media, Media::Console | Media::Usb) {
            return;
        }
        let Some(li) = port.link else {
            port.counters.drops += 1;
            self.note(dev, cause, Note::DropNoLink { port: pi });
            return;
        };
        if !self.link_up(li) {
            if let Some(Some(n)) = self.nodes.get_mut(ni) {
                n.ports[pi as usize].counters.drops += 1;
            }
            self.note(dev, cause, Note::DropNoLink { port: pi });
            return;
        }
        let now = self.now;
        let batch = self.trace.stamp(now);
        let Some(Some(l)) = self.links.get_mut(li) else {
            return;
        };
        let Some(d) = l.end_of(ni, pi) else {
            return;
        };
        let dir = &mut l.dir[d as usize];
        if dir.busy_until > now || !dir.queue.is_empty() {
            if dir.queued_bytes + frame.len() > QUEUE_BYTES {
                if let Some(Some(n)) = self.nodes.get_mut(ni) {
                    n.ports[pi as usize].counters.drops += 1;
                }
                self.note(dev, cause, Note::DropQueueFull { port: pi });
                return;
            }
            dir.queued_bytes += frame.len();
            dir.queue.push_back(Queued { frame, cause, batch, at: now });
            return;
        }
        self.start_tx(li, d, Queued { frame, cause, batch, at: now });
    }

    /// puts a frame on the wire now
    fn start_tx(&mut self, li: usize, d: u8, q: Queued) {
        let Queued { frame, cause, batch, at } = q;
        let mut kbps = self.link_kbps(li);
        // what injected faults add (fault.rs)
        let over = self.faults.link(li).unwrap_or_default();
        if let Some(k) = over.kbps {
            kbps = kbps.min(k);
        }
        let now = self.now;
        let Some(Some(l)) = self.links.get_mut(li) else {
            return;
        };
        let ser = serialisation_ns(frame.len(), kbps, l.cable.is_ethernet());
        let end = now + ser;
        l.dir[d as usize].busy_until = end;
        let epoch = l.epoch;
        let loss = crate::fault::combine_loss(l.loss_ppm, over.loss_ppm);
        let lost = loss > 0 && l.rng.chance_ppm(loss);
        let jitter_max = l.jitter_ns + over.jitter;
        let jitter = if jitter_max > 0 { l.rng.range(0, jitter_max as i64) as Ns } else { 0 };
        let t_rx = end + l.delay_ns + over.delay + jitter;
        let (from, to) = (l.ends[d as usize], l.ends[1 - d as usize]);
        let id = l.id;
        let serial = l.cable == CableKind::Serial;
        let dev_of = |e: &Engine, n: usize| match e.nodes.get(n) {
            Some(Some(node)) => node.dev,
            _ => 0,
        };
        let (fd, td) = (dev_of(self, from.0), dev_of(self, to.0));
        if let Some(Some(n)) = self.nodes.get_mut(from.0) {
            let c = &mut n.ports[from.1 as usize].counters;
            c.tx_frames += 1;
            c.tx_bytes += frame.len() as u64;
        }
        let hop = self.trace.push_hop(Hop {
            id: 0,
            t_tx: now,
            t_rx,
            link: id,
            from: (fd, from.1),
            to: (td, to.1),
            mask: if serial && frame.len() > 4 { decode::ip_mask(&frame[4..]) } else { decode::proto_mask(&frame) },
            drop: if lost { Some(crate::trace::HopDrop::Loss) } else { None },
            bytes: frame.clone(),
            cause,
            batch,
            t_queued: at,
            hdlc: serial,
        });
        self.schedule(end, Ev::TxDone { link: li, epoch, dir: d });
        if !lost {
            self.schedule(t_rx, Ev::Arrive { link: li, epoch, dir: d, frame, hop });
        }
    }

    pub(crate) fn tx_done(&mut self, li: usize, epoch: u32, d: u8) {
        let Some(Some(l)) = self.links.get_mut(li) else {
            return;
        };
        if l.epoch != epoch {
            return;
        }
        let dir = &mut l.dir[d as usize];
        // a frame that went out at this same moment (its sender came first) has the wire now;
        // its own TxDone starts the next one
        if dir.busy_until > self.now {
            return;
        }
        let Some(q) = dir.queue.pop_front() else {
            return;
        };
        dir.queued_bytes -= q.frame.len();
        self.start_tx(li, d, q);
    }

    pub(crate) fn arrive(&mut self, li: usize, epoch: u32, d: u8, frame: Frame, hop: HopId) {
        let up = self.link_up(li);
        let Some(Some(l)) = self.links.get(li) else {
            self.trace.mark_lost(hop);
            return;
        };
        let (ni, pi) = l.ends[1 - d as usize];
        if l.epoch != epoch || !up {
            self.trace.mark_lost(hop);
            if let Some(Some(n)) = self.nodes.get(ni) {
                let dev = n.dev;
                self.note(dev, Some(hop), Note::LostInFlight { port: pi });
            }
            return;
        }
        let Some(Some(node)) = self.nodes.get_mut(ni) else {
            return;
        };
        let c = &mut node.ports[pi as usize].counters;
        c.rx_frames += 1;
        c.rx_bytes += frame.len() as u64;
        self.receive(ni, pi, frame, hop);
    }

    /// a node got a frame on a port: hub, bridge member, routed port with IP, or a plain host
    fn receive(&mut self, ni: usize, pi: u16, frame: Frame, hop: HopId) {
        let Some(Some(node)) = self.nodes.get(ni) else {
            return;
        };
        let dev = node.dev;
        if node.model.has(F_HUB) {
            let out: alloc::vec::Vec<u16> = (0..node.ports.len() as u16).filter(|&p| p != pi && node.ports[p as usize].link.is_some_and(|li| self.link_up(li))).collect();
            self.note(dev, Some(hop), Note::Repeated { ports: out.len() as u16 });
            for p in out {
                self.send(ni, p, frame.clone(), Some(hop));
            }
            return;
        }
        let member = node.model.has(F_BRIDGE) && node.bridge.as_ref().is_some_and(|b| b.ports.get(pi as usize).is_some_and(|bp| bp.member));
        if member {
            self.bridge_input(ni, pi, frame, hop);
        } else if node.ip.is_some() {
            self.l3_from_port(ni, pi, frame, hop);
        } else {
            self.host_input(ni, pi, frame, hop);
        }
    }

    /// a host keeps frames for its port's MAC, broadcasts and group addresses
    fn host_input(&mut self, ni: usize, pi: u16, frame: Frame, hop: HopId) {
        let Some(Some(node)) = self.nodes.get(ni) else {
            return;
        };
        let dev = node.dev;
        let mine = match netpkt::eth::EthView::parse(&frame) {
            Some(e) => e.dst() == node.ports[pi as usize].mac || e.dst().is_multicast(),
            None => false,
        };
        self.note(dev, Some(hop), if mine { Note::Delivered { port: pi } } else { Note::NotForMe { port: pi } });
    }
}

/// does a copper cable of this kind fit the ports' MDI / MDI-X roles?
pub fn right_cable(kind: CableKind, a: &jfn_model::PortSpec, b: &jfn_model::PortSpec) -> bool {
    !kind.is_copper() || cable::cable_kind(a, b).is_ok_and(|k| k == kind)
}
