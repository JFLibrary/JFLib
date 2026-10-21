// pcap -- the recording as a pcapng file for Wireshark, stamped with the virtual clock
//
//   Engine::pcapng(&PcapOpts) -> Pcap { bytes, packets }
//
//   PcapScope::All         every frame once, one interface per cable ("PC1 eth0 – SW1 Gi1/0/1")
//   PcapScope::Link(id)    the frames of one cable
//   PcapScope::Device(id)  what a device sent (stamped t_tx) and received (stamped t_rx), one
//                          interface per port ("R1 Gi0/0/0"); a frame the cable lost or that is
//                          still under way was not received
//   PcapOpts               protocols and devices (Filter), a window of timestamps, a comment
//                          per packet with both ends and whether the cable lost it
//
// Ethernet cables are LINKTYPE_ETHERNET, serial lines LINKTYPE_C_HDLC: that is what the
// simulator sends on them.  Timestamps are nanoseconds of virtual time since the start
// (Wireshark shows 1970-01-01 plus the simulated time).  The bytes are the recorded ones, so
// checksums, tags and padding are exactly what the devices sent.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::ifname;
use netpkt::pcapng::{Writer, LINKTYPE_C_HDLC, LINKTYPE_ETHERNET};

use crate::kernel::{Engine, Ns};
use crate::trace::{Filter, Hop, HopDrop};

/// which frames a capture holds
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PcapScope {
    /// every recorded frame, one interface per cable
    All,
    /// one cable (project link id)
    Link(u32),
    /// what a device sent and received, one interface per port
    Device(u32),
}

/// what to capture
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PcapOpts {
    pub scope: PcapScope,
    /// protocols and devices
    pub filter: Filter,
    /// timestamps from..=until (virtual time)
    pub from: Ns,
    pub until: Ns,
    /// a comment per packet: "PC1 eth0 → SW1 Gi1/0/1", and whether the cable lost it
    pub comments: bool,
}

impl Default for PcapOpts {
    fn default() -> PcapOpts {
        PcapOpts { scope: PcapScope::All, filter: Filter::all(), from: 0, until: Ns::MAX, comments: true }
    }
}

/// a capture file
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pcap {
    pub bytes: Vec<u8>,
    /// packets in it
    pub packets: usize,
}

/// a packet to write: timestamp, frame, interface key (a cable or a port), received
struct Pk<'a> {
    t: Ns,
    hop: &'a Hop,
    key: u32,
    inbound: bool,
}

impl Engine {
    /// "PC1 eth0", "R1 Gi0/0/0" (a device that is gone: "Gerät 7 Port 0")
    pub fn end_name(&self, (dev, port): (u32, u16)) -> String {
        match self.node(dev) {
            Some(n) => format!("{} {}", n.name, ifname::short(n.port_name(port))),
            None => format!("Gerät {} Port {}", dev, port),
        }
    }

    /// the recording, or a part of it, as a pcapng file
    pub fn pcapng(&self, o: &PcapOpts) -> Pcap {
        let window = |t: Ns| o.from <= t && t <= o.until;
        let mut pks: Vec<Pk> = Vec::new();
        for h in self.trace.hops().filter(|h| o.filter.hop(h)) {
            match o.scope {
                PcapScope::All => pks.push(Pk { t: h.t_tx, hop: h, key: h.link, inbound: false }),
                PcapScope::Link(id) if h.link == id => pks.push(Pk { t: h.t_tx, hop: h, key: h.link, inbound: false }),
                PcapScope::Link(_) => {}
                PcapScope::Device(d) => {
                    if h.from.0 == d {
                        pks.push(Pk { t: h.t_tx, hop: h, key: h.from.1 as u32, inbound: false });
                    } else if h.to.0 == d && h.drop.is_none() && h.t_rx <= self.now {
                        pks.push(Pk { t: h.t_rx, hop: h, key: h.to.1 as u32, inbound: true });
                    }
                }
            }
        }
        pks.retain(|p| window(p.t));
        pks.sort_by_key(|p| (p.t, p.hop.id, p.inbound));
        let mut w = Writer::new("JFNetworker");
        let mut ifaces: BTreeMap<u32, u32> = BTreeMap::new();
        for p in &pks {
            let h = p.hop;
            let iface = match ifaces.get(&p.key) {
                Some(&i) => i,
                None => {
                    let name = match o.scope {
                        PcapScope::Device(d) => self.end_name((d, p.key as u16)),
                        _ => self.cable_name(h),
                    };
                    let i = w.add_interface(&name, if h.hdlc { LINKTYPE_C_HDLC } else { LINKTYPE_ETHERNET }, 0);
                    ifaces.insert(p.key, i);
                    i
                }
            };
            if o.comments {
                let mut c = format!("{} \u{2192} {}", self.end_name(h.from), self.end_name(h.to));
                match h.drop {
                    Some(HopDrop::Loss) => c.push_str(", auf der Leitung verloren"),
                    Some(HopDrop::LinkDown) => c.push_str(", verloren: die Leitung fiel aus"),
                    None => {}
                }
                w.packet_with_comment(iface, p.t, &h.bytes, &c);
            } else {
                w.packet(iface, p.t, &h.bytes);
            }
        }
        Pcap { bytes: w.finish(), packets: pks.len() }
    }

    /// "PC1 eth0 – SW1 Gi1/0/1": the cable of a frame, its ends in the project's order
    fn cable_name(&self, h: &Hop) -> String {
        let ends = match self.link_rt(h.link) {
            Some(l) => [(self.dev_of(l.ends[0].0), l.ends[0].1), (self.dev_of(l.ends[1].0), l.ends[1].1)],
            None => [h.from, h.to],
        };
        format!("{} \u{2013} {}", self.end_name(ends[0]), self.end_name(ends[1]))
    }
}
