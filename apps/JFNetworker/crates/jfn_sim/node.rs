// node -- a device at run time: its ports, the parts its model has, its random numbers
//
//   Node::new(dev, name, model, power, seed)  ports from the catalog model; a Bridge for
//                                             switching models; the IPv4 part; admin state
//                                             as IOS boots it
//   port_mac(dev, index) / base_mac(dev)      02:4a:dd:dd:dd:pp (locally administered)
//   Node::port_name(i) / port_by_name(name)
//   Node::os                                  sockets, services, resolver, log, clock (os.rs)
//
// Router and firewall interfaces start shut down (as on IOS: "no shutdown" brings them up);
// switch ports, hosts and access points start up.

use alloc::string::String;
use alloc::vec::Vec;

use base::rng::Pcg32;
use jfn_model::catalog::{CliStyle, Model, PortSpec, F_BRIDGE, F_HOST, F_ROUTER};
use netpkt::Mac;

use crate::bridge::Bridge;
use crate::ip::IpStack;
use crate::os::Os;

/// traffic counters of a port
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PortCounters {
    pub rx_frames: u64,
    pub rx_bytes: u64,
    pub tx_frames: u64,
    pub tx_bytes: u64,
    /// frames dropped at this port (no link, queue full, refused)
    pub drops: u64,
}

/// a port at run time
#[derive(Clone, Debug)]
pub struct Port {
    pub spec: PortSpec,
    pub mac: Mac,
    /// "no shutdown"
    pub admin_up: bool,
    /// shut off by BPDU guard until "shutdown" (stp.rs); the link stays down meanwhile
    pub err_disabled: bool,
    /// the engine slot of its cable
    pub link: Option<usize>,
    pub counters: PortCounters,
}

/// a device at run time
#[derive(Clone, Debug)]
pub struct Node {
    pub dev: u32,
    pub name: String,
    pub model: &'static Model,
    pub power: bool,
    pub ports: Vec<Port>,
    pub bridge: Option<Bridge>,
    /// the IPv4 part (None for hubs, modems, the cloud)
    pub ip: Option<IpStack>,
    pub base_mac: Mac,
    /// the device's own random numbers (sequence numbers, ports, transaction ids); derived
    /// from the project seed and the device id, so adding a device does not change the others
    pub rng: Pcg32,
    pub(crate) timers: Vec<u32>,
    /// the operating system of a device with IP
    pub os: Option<Os>,
    /// the command line: mode, running command, settings of the console and the configuration
    pub cli: crate::cli::CliDev,
}

/// the MAC of a port: 02:4a + 24 bits of the device id + the port index + 1
pub fn port_mac(dev: u32, index: u16) -> Mac {
    let d = dev.to_be_bytes();
    Mac([0x02, 0x4A, d[1], d[2], d[3], (index as u8).wrapping_add(1)])
}

/// the device's own MAC (bridge id, management): the port byte 0
pub fn base_mac(dev: u32) -> Mac {
    let d = dev.to_be_bytes();
    Mac([0x02, 0x4A, d[1], d[2], d[3], 0])
}

/// IOS routers and firewalls boot with their interfaces shut down
fn boots_shut(m: &Model) -> bool {
    m.cli == CliStyle::Ios && m.has(F_ROUTER) && !m.has(F_BRIDGE)
}

impl Node {
    pub fn new(dev: u32, name: &str, model: &'static Model, power: bool, seed: u64) -> Node {
        let up = !boots_shut(model);
        let ports: Vec<Port> = model
            .ports()
            .into_iter()
            .map(|spec| Port { mac: port_mac(dev, spec.index), admin_up: up, err_disabled: false, link: None, counters: PortCounters::default(), spec })
            .collect();
        let mut bridge = if model.has(F_BRIDGE) { Some(Bridge::new(&ports.iter().map(|p| p.spec.clone()).collect::<Vec<_>>())) } else { None };
        // a home router routes its WAN port and bridges the rest
        if model.has(F_ROUTER | F_HOST)
            && let Some(b) = bridge.as_mut()
        {
            for (i, p) in ports.iter().enumerate() {
                if p.spec.name == "wan" {
                    b.ports[i].member = false;
                }
            }
        }
        let member = |p: u16| bridge.as_ref().is_some_and(|b| b.ports.get(p as usize).is_some_and(|bp| bp.member));
        let ip = IpStack::for_model(model, &ports, &member, base_mac(dev));
        let os = ip.as_ref().map(|_| Os::new(model, dev, seed));
        Node {
            dev,
            name: String::from(name),
            model,
            power,
            ports,
            bridge,
            ip,
            base_mac: base_mac(dev),
            rng: Pcg32::derive(seed, dev as u64),
            timers: Vec::new(),
            os,
            cli: crate::cli::CliDev::new(model),
        }
    }
    pub fn port_name(&self, i: u16) -> &str {
        self.ports.get(i as usize).map(|p| p.spec.name.as_str()).unwrap_or("?")
    }
    /// a port index by any spelling ("gi1/0/1", "eth0")
    pub fn port_by_name(&self, name: &str) -> Option<u16> {
        self.model.port(name).map(|p| p.index)
    }
    /// the port that owns a MAC address
    pub fn port_of_mac(&self, mac: Mac) -> Option<u16> {
        self.ports.iter().position(|p| p.mac == mac).map(|i| i as u16)
    }
}
