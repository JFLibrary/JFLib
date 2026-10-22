// stp -- spanning tree on the IOS-like switches: what a bridge is set up to do (the protocol,
// the bridge priority, the timers, the settings of each port), the protocol that runs
// (stp98.rs: the spanning tree algorithm of IEEE 802.1D-1998 clause 8; rstp.rs: RSTP after IEEE
// 802.1D-2004 clause 17, from 802.1w), and how the bridge uses it: BPDUs in and out, the port
// states that let a port learn and forward, MAC entries flushed or aged fast after topology
// changes, BPDU guard
//
//   Stp { cfg, run }            Bridge::stp; run is None while spanning tree is off
//   StpCfg / PortCfg            the settings ("spanning-tree ..."), kept while it is off
//   ModeWord                    stp | rstp, and pvst | rapid-pvst taken for recognition: they run
//                               the IEEE protocol with its one tree (no per-VLAN instances, no
//                               vendor BPDUs)
//   PortRole / PortState        the roles and states both protocols show
//   StpSet                      one setting (Action::Stp)
//   TreeView / StpPortView      the tree of a bridge for "show spanning-tree", tests and the UI
//   Engine::stp_view(dev)       None while spanning tree is off (or the device is no bridge)
//   Engine::stp_apply(dev, set) starts, stops or restarts the protocol, or changes a setting
//   Stp::learns / forwards(p)   what the bridge may do with a port (everything while off)
//
// Off by default: a bridge runs spanning tree only after "spanning-tree mode ..." (typed, or in
// its startup configuration); until then it is the learning bridge it always was and keeps
// BPDUs to itself (Note::Consumed), so projects without it behave as before.  Hubs repeat BPDUs
// like any frame.  Both protocols count time in ticks of one second from the moment they start
// (the timer resolution of 802.1D-2004 17.22, used for the 802.1D-1998 timers too); BPDUs, links
// and settings act at once.  One tree per bridge: BPDUs go untagged out of every port that
// takes part (access ports and trunks), bridge IDs carry the system ID extension 0.
//
// Path costs follow 802.1D-2004 Table 17-3 (20 000 000 000 / kbit/s: 1G 20 000, 10G 2 000, 100G
// 200) for the speed the link runs at, unless "spanning-tree cost" sets one.  A port's link type
// is point-to-point unless its cable leads to a hub (shared) or "spanning-tree link-type" says
// otherwise.  "spanning-tree portfast" makes a port an edge port (it forwards at once and makes
// no topology changes) until a BPDU arrives on it; RSTP also finds edge ports by itself
// (AutoEdge: a designated port that hears no BPDU for 3 s).  BPDU guard works whenever it is
// set, also without spanning tree: a BPDU shuts the port off (err-disabled, Port::err_disabled)
// until "shutdown" and "no shutdown".

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use jfn_model::catalog::F_HUB;
use jfn_model::msg;
use netpkt::stp::{self as bpdu, Bpdu, BridgeId, PortId};

use crate::action::ActErr;
use crate::kernel::{Engine, Ns, SEC};
use crate::trace::{HopId, Note};

pub use crate::rstp::Rstp;
pub use crate::stp98::Stp98;

/// the node timer slot of the protocol's tick (the last one below the services')
pub const SLOT_STP: u32 = 15;

/// the defaults of 802.1D (Table 17-1 of 802.1D-2004, 8.10.2 of 802.1D-1998)
pub const PRIORITY: u16 = 32768;
pub const PORT_PRIORITY: u8 = 128;
pub const HELLO: u16 = 2;
pub const FWD_DELAY: u16 = 15;
pub const MAX_AGE: u16 = 20;

/// which protocol a bridge runs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Proto {
    /// IEEE 802.1D (the spanning tree algorithm of 802.1D-1998)
    Stp,
    /// IEEE 802.1w, now 802.1D-2004 clause 17
    Rstp,
}

/// the word of "spanning-tree mode"
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeWord {
    Stp,
    Rstp,
    /// taken for recognition: runs IEEE 802.1D with one tree
    Pvst,
    /// taken for recognition: runs IEEE 802.1w with one tree
    RapidPvst,
}

impl ModeWord {
    pub fn proto(self) -> Proto {
        match self {
            ModeWord::Stp | ModeWord::Pvst => Proto::Stp,
            ModeWord::Rstp | ModeWord::RapidPvst => Proto::Rstp,
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            ModeWord::Stp => "stp",
            ModeWord::Rstp => "rstp",
            ModeWord::Pvst => "pvst",
            ModeWord::RapidPvst => "rapid-pvst",
        }
    }
    pub fn parse(s: &str) -> Option<ModeWord> {
        Some(match s {
            "stp" => ModeWord::Stp,
            "rstp" => ModeWord::Rstp,
            "pvst" => ModeWord::Pvst,
            "rapid-pvst" => ModeWord::RapidPvst,
            _ => return None,
        })
    }
}

/// the role of a port (802.1D-2004 17.7; for 802.1D-1998 derived from its state: a port that is
/// neither root nor designated is an alternate port, or a backup port when the designated port
/// of its LAN is one of this bridge's own)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortRole {
    Disabled,
    Root,
    Designated,
    Alternate,
    Backup,
}

impl PortRole {
    /// the German name (a term of the catalogue)
    pub fn msg(self) -> jfn_model::msg::Msg {
        jfn_model::msg::phrase(match self {
            // i18n
            PortRole::Disabled => "deaktiviert",
            PortRole::Root => "Root-Port",
            PortRole::Designated => "Designated-Port",
            PortRole::Alternate => "Alternate-Port",
            PortRole::Backup => "Backup-Port",
        })
    }
    /// the column "Role" of "show spanning-tree"
    pub fn short(self) -> &'static str {
        match self {
            PortRole::Disabled => "Disb",
            PortRole::Root => "Root",
            PortRole::Designated => "Desg",
            PortRole::Alternate => "Altn",
            PortRole::Backup => "Back",
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            PortRole::Disabled => "disabled",
            PortRole::Root => "root",
            PortRole::Designated => "designated",
            PortRole::Alternate => "alternate",
            PortRole::Backup => "backup",
        }
    }
}

/// the state of a port: 802.1D-1998 knows Blocking, Listening, Learning and Forwarding, RSTP
/// Discarding, Learning and Forwarding; Disabled while the port does not take part
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortState {
    Disabled,
    Blocking,
    Listening,
    Learning,
    Forwarding,
    Discarding,
}

impl PortState {
    /// the port learns source addresses
    pub fn learns(self) -> bool {
        matches!(self, PortState::Learning | PortState::Forwarding)
    }
    pub fn forwards(self) -> bool {
        self == PortState::Forwarding
    }
    /// the IEEE name (a term the catalogue keeps as it is)
    pub fn msg(self) -> jfn_model::msg::Msg {
        jfn_model::msg::phrase(match self {
            // i18n
            PortState::Disabled => "Disabled",
            PortState::Blocking => "Blocking",
            PortState::Listening => "Listening",
            PortState::Learning => "Learning",
            PortState::Forwarding => "Forwarding",
            PortState::Discarding => "Discarding",
        })
    }
    /// the column "Sts" of "show spanning-tree" (RSTP's discarding shows as BLK, as there)
    pub fn short(self) -> &'static str {
        match self {
            PortState::Disabled => "DIS",
            PortState::Blocking | PortState::Discarding => "BLK",
            PortState::Listening => "LIS",
            PortState::Learning => "LRN",
            PortState::Forwarding => "FWD",
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            PortState::Disabled => "disabled",
            PortState::Blocking => "blocking",
            PortState::Listening => "listening",
            PortState::Learning => "learning",
            PortState::Forwarding => "forwarding",
            PortState::Discarding => "discarding",
        }
    }
}

/// why a bridge drops a BPDU
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BpduDrop {
    /// an RST BPDU at a bridge that speaks 802.1D-1998 only (9.3.4: a type it does not know)
    Type,
    /// its Message Age is not below its Max Age
    Aged,
    /// this port's own BPDU, back over a loop
    Looped,
}

/// what a topology change note says
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TcWhat {
    /// this bridge saw the topology change (a port began to forward or stopped)
    Detected,
    /// STP: a TCN BPDU went to the root
    TcnSent,
    /// a TCN BPDU arrived on a designated port
    TcnReceived,
    /// STP: the designated bridge acknowledged the TCN
    Acked,
    /// a BPDU with the TC flag arrived (RSTP: on the root port or a designated port)
    Received,
    /// STP: the root sets / clears the TC flag of its BPDUs
    RootFlag(bool),
}

/// the link type of a port ("spanning-tree link-type")
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkType {
    /// point-to-point, unless the cable leads to a hub
    Auto,
    P2p,
    Shared,
}

/// the settings of a port
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortCfg {
    /// "spanning-tree cost" (None: by the speed of the link)
    pub cost: Option<u32>,
    /// "spanning-tree port-priority" (0 .. 240 in steps of 16)
    pub priority: u8,
    /// "spanning-tree portfast" (AdminEdge)
    pub edge: bool,
    /// "spanning-tree bpduguard enable"
    pub guard: bool,
    pub link: LinkType,
}

impl Default for PortCfg {
    fn default() -> PortCfg {
        PortCfg { cost: None, priority: PORT_PRIORITY, edge: false, guard: false, link: LinkType::Auto }
    }
}

/// the settings of the bridge
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StpCfg {
    /// None: spanning tree is off (the default)
    pub mode: Option<ModeWord>,
    /// the settable part of the bridge priority (0 .. 61440 in steps of 4096)
    pub priority: u16,
    pub hello: u16,
    pub fwd_delay: u16,
    pub max_age: u16,
    /// by bridge port index (filled as ports get settings)
    pub ports: Vec<PortCfg>,
}

impl Default for StpCfg {
    fn default() -> StpCfg {
        StpCfg { mode: None, priority: PRIORITY, hello: HELLO, fwd_delay: FWD_DELAY, max_age: MAX_AGE, ports: Vec::new() }
    }
}

impl StpCfg {
    pub fn port(&self, p: u16) -> PortCfg {
        self.ports.get(p as usize).cloned().unwrap_or_default()
    }
    fn port_mut(&mut self, p: u16) -> &mut PortCfg {
        if self.ports.len() <= p as usize {
            self.ports.resize(p as usize + 1, PortCfg::default());
        }
        &mut self.ports[p as usize]
    }
}

/// the protocol that runs
#[derive(Clone, Debug)]
pub enum Run {
    Stp(Stp98),
    Rstp(Rstp),
}

/// a bridge's spanning tree
#[derive(Clone, Debug, Default)]
pub struct Stp {
    pub cfg: StpCfg,
    pub run: Option<Run>,
    /// topology changes this bridge saw, and when the last one began
    pub tc_count: u32,
    pub tc_last: Option<Ns>,
}

impl Stp {
    /// the state of a port (Forwarding for every port while spanning tree is off)
    pub fn state(&self, p: u16) -> PortState {
        match &self.run {
            None => PortState::Forwarding,
            Some(Run::Stp(s)) => s.ports.get(p as usize).map_or(PortState::Disabled, |x| x.state),
            Some(Run::Rstp(r)) => r.state(p),
        }
    }
    /// the port may learn source addresses
    pub fn learns(&self, p: u16) -> bool {
        self.run.is_none() || self.state(p).learns()
    }
    /// the port may receive and send frames (BPDUs aside)
    pub fn forwards(&self, p: u16) -> bool {
        self.run.is_none() || self.state(p).forwards()
    }
    pub fn on(&self) -> bool {
        self.run.is_some()
    }
}

/// one setting of the spanning tree (Action::Stp)
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StpSet {
    /// "spanning-tree mode ..." (None: "no spanning-tree mode", spanning tree off)
    Mode(Option<ModeWord>),
    /// the bridge priority (None: the default 32768)
    Priority(Option<u16>),
    /// the timers in seconds (None: the default)
    Hello(Option<u16>),
    FwdDelay(Option<u16>),
    MaxAge(Option<u16>),
    Cost { port: String, cost: Option<u32> },
    PortPriority { port: String, priority: Option<u8> },
    Edge { port: String, on: bool },
    Guard { port: String, on: bool },
    Link { port: String, kind: LinkType },
    /// "clear spanning-tree detected-protocols": RSTP tries RSTP again (mcheck)
    Mcheck { port: Option<String> },
}

/// what the protocols ask the bridge to do (the glue below carries it out)
#[derive(Clone, Debug)]
pub(crate) enum Out {
    /// a BPDU goes out of a port
    Tx { port: u16, bpdu: Bpdu },
    Note(Note),
    /// RSTP: forget what was learned on a port, now
    Flush { port: u16 },
    /// 802.1D: MAC entries age out after Forward Delay (seconds) while the topology changes;
    /// None: normal aging again
    Aging(Option<u16>),
}

/// a port of the tree as "show spanning-tree" shows it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StpPortView {
    pub port: u16,
    pub id: PortId,
    pub role: PortRole,
    pub state: PortState,
    pub cost: u32,
    /// an edge port now
    pub edge: bool,
    pub p2p: bool,
    /// RSTP: the port speaks 802.1D because its neighbour does
    pub stp_peer: bool,
    /// the designated root, cost, bridge and port of its LAN
    pub d_root: BridgeId,
    pub d_cost: u32,
    pub d_bridge: BridgeId,
    pub d_port: PortId,
    /// seconds left: STP the forward delay timer, RSTP fdWhile; the age of the stored
    /// information (STP) or what is left of it (RSTP rcvdInfoWhile)
    pub fwd_timer: u16,
    pub info_age: u16,
    pub tx: u64,
    pub rx: u64,
    pub to_fwd: u32,
}

/// the tree of a bridge
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeView {
    pub proto: Proto,
    pub mode: ModeWord,
    pub bridge: BridgeId,
    pub root: BridgeId,
    pub root_cost: u32,
    pub root_port: Option<u16>,
    /// the timers in use (the root's) and the bridge's own
    pub hello: u16,
    pub max_age: u16,
    pub fwd_delay: u16,
    pub b_hello: u16,
    pub b_max_age: u16,
    pub b_fwd_delay: u16,
    /// a topology change is going on (STP: the TC flag; RSTP: a port's tcWhile runs)
    pub tc: bool,
    pub tc_count: u32,
    pub tc_last: Option<Ns>,
    /// the ports that take part and are up, by index
    pub ports: Vec<StpPortView>,
}

impl TreeView {
    pub fn is_root(&self) -> bool {
        self.root == self.bridge
    }
    pub fn port(&self, p: u16) -> Option<&StpPortView> {
        self.ports.iter().find(|x| x.port == p)
    }
}

/// 802.1D's rule for the timers: 2 x (Forward Delay - 1) >= Max Age >= 2 x (Hello Time + 1)
pub fn timers_ok(hello: u16, fwd_delay: u16, max_age: u16) -> bool {
    2 * (fwd_delay.saturating_sub(1)) >= max_age && max_age >= 2 * (hello + 1)
}

impl Engine {
    fn stp_of(&self, ni: usize) -> Option<&Stp> {
        Some(&self.nodes.get(ni)?.as_ref()?.bridge.as_ref()?.stp)
    }
    fn stp_mut(&mut self, ni: usize) -> Option<&mut Stp> {
        Some(&mut self.nodes.get_mut(ni)?.as_mut()?.bridge.as_mut()?.stp)
    }

    /// the spanning tree of a bridge (None while it is off)
    pub fn stp_view(&self, dev: u32) -> Option<TreeView> {
        let ni = self.node_index(dev)?;
        let st = self.stp_of(ni)?;
        let mode = st.cfg.mode?;
        let mut v = match st.run.as_ref()? {
            Run::Stp(s) => s.view(mode),
            Run::Rstp(r) => r.view(mode),
        };
        v.tc_count = st.tc_count;
        v.tc_last = st.tc_last;
        Some(v)
    }

    /// the spanning tree runs on node `ni`
    pub(crate) fn stp_runs(&self, ni: usize) -> bool {
        self.stp_of(ni).is_some_and(|s| s.on())
    }

    /// a setting of the spanning tree
    pub(crate) fn stp_apply(&mut self, dev: u32, set: StpSet) -> Result<(), ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        if self.stp_of(ni).is_none() {
            return Err(ActErr::NoBridge(dev));
        }
        let port_of = |e: &Engine, name: &str| -> Result<u16, ActErr> {
            let (_, pi) = e.resolve(dev, name)?;
            let member = e.nodes.get(ni).and_then(|n| n.as_ref()).and_then(|n| n.bridge.as_ref()).is_some_and(|b| b.ports.get(pi as usize).is_some_and(|bp| bp.member));
            if member { Ok(pi) } else { Err(ActErr::Refused(msg!("{} ist kein Switch-Port.", name))) }
        };
        match set {
            StpSet::Mode(m) => {
                let before = self.stp_of(ni).and_then(|s| s.cfg.mode);
                if let Some(s) = self.stp_mut(ni) {
                    s.cfg.mode = m;
                }
                // a startup configuration starts the protocol when it is through (stp_booted),
                // so that the first BPDUs carry its priority and timers
                if self.cdev(ni).is_some_and(|c| c.booting) {
                    return Ok(());
                }
                match (before.map(|b| b.proto()), m.map(|x| x.proto())) {
                    (Some(a), Some(b)) if a == b => {}
                    (_, Some(_)) => self.stp_start(ni),
                    (_, None) => self.stp_stop(ni),
                }
            }
            StpSet::Priority(p) => {
                let p = p.unwrap_or(PRIORITY);
                if p > 61440 || p % 4096 != 0 {
                    return Err(ActErr::Refused(msg!("Die Bridge-Priorität {} geht nicht: 0 bis 61440 in Schritten von 4096.", p)));
                }
                let mac = self.nodes.get(ni).and_then(|n| n.as_ref()).map(|n| n.base_mac).unwrap_or_default();
                let mut out = Vec::new();
                if let Some(s) = self.stp_mut(ni) {
                    s.cfg.priority = p;
                    let id = BridgeId::new(p, mac);
                    match s.run.as_mut() {
                        Some(Run::Stp(x)) => x.set_bridge_id(id, &mut out),
                        Some(Run::Rstp(x)) => x.set_bridge_id(id, &mut out),
                        None => {}
                    }
                }
                self.stp_out(ni, None, out);
            }
            StpSet::Hello(v) | StpSet::FwdDelay(v) | StpSet::MaxAge(v) if v.is_some_and(|x| x == 0) => {
                return Err(ActErr::Refused(msg!("Ein Spanning-Tree-Timer braucht mindestens eine Sekunde.")));
            }
            StpSet::Hello(v) | StpSet::FwdDelay(v) | StpSet::MaxAge(v) => {
                let Some(cfg) = self.stp_of(ni).map(|s| s.cfg.clone()) else {
                    return Ok(());
                };
                let (mut h, mut f, mut m) = (cfg.hello, cfg.fwd_delay, cfg.max_age);
                match set {
                    StpSet::Hello(_) => h = v.unwrap_or(HELLO),
                    StpSet::FwdDelay(_) => f = v.unwrap_or(FWD_DELAY),
                    _ => m = v.unwrap_or(MAX_AGE),
                }
                if !timers_ok(h, f, m) {
                    return Err(ActErr::Refused(msg!("Die Timer passen nicht zusammen: 2 × (Forward Delay − 1) ≥ Max Age ≥ 2 × (Hello Time + 1) (jetzt Hello {}, Forward Delay {}, Max Age {}).", h, f, m)));
                }
                let mut out = Vec::new();
                if let Some(s) = self.stp_mut(ni) {
                    s.cfg.hello = h;
                    s.cfg.fwd_delay = f;
                    s.cfg.max_age = m;
                    match s.run.as_mut() {
                        Some(Run::Stp(x)) => x.set_times(h, f, m, &mut out),
                        Some(Run::Rstp(x)) => x.set_times(h, f, m, &mut out),
                        None => {}
                    }
                }
                self.stp_out(ni, None, out);
            }
            StpSet::Cost { port, cost } => {
                let pi = port_of(self, &port)?;
                if cost.is_some_and(|c| c == 0 || c > 200_000_000) {
                    return Err(ActErr::Refused(msg!("Die Pfadkosten gehen von 1 bis 200000000.")));
                }
                if let Some(s) = self.stp_mut(ni) {
                    s.cfg.port_mut(pi).cost = cost;
                }
                self.stp_link(ni, pi);
            }
            StpSet::PortPriority { port, priority } => {
                let pi = port_of(self, &port)?;
                let p = priority.unwrap_or(PORT_PRIORITY);
                if p > 240 || p % 16 != 0 {
                    return Err(ActErr::Refused(msg!("Die Port-Priorität {} geht nicht: 0 bis 240 in Schritten von 16.", p)));
                }
                let mut out = Vec::new();
                if let Some(s) = self.stp_mut(ni) {
                    s.cfg.port_mut(pi).priority = p;
                    let id = PortId::new(p, pi + 1);
                    match s.run.as_mut() {
                        Some(Run::Stp(x)) => x.set_port_id(pi, id, &mut out),
                        Some(Run::Rstp(x)) => x.set_port_id(pi, id, &mut out),
                        None => {}
                    }
                }
                self.stp_out(ni, None, out);
            }
            StpSet::Edge { port, on } => {
                let pi = port_of(self, &port)?;
                let mut out = Vec::new();
                if let Some(s) = self.stp_mut(ni) {
                    s.cfg.port_mut(pi).edge = on;
                    match s.run.as_mut() {
                        Some(Run::Stp(x)) => x.set_edge(pi, on, &mut out),
                        Some(Run::Rstp(x)) => x.set_edge(pi, on, &mut out),
                        None => {}
                    }
                }
                self.stp_out(ni, None, out);
            }
            StpSet::Guard { port, on } => {
                let pi = port_of(self, &port)?;
                if let Some(s) = self.stp_mut(ni) {
                    s.cfg.port_mut(pi).guard = on;
                }
            }
            StpSet::Link { port, kind } => {
                let pi = port_of(self, &port)?;
                if let Some(s) = self.stp_mut(ni) {
                    s.cfg.port_mut(pi).link = kind;
                }
                self.stp_link(ni, pi);
            }
            StpSet::Mcheck { port } => {
                let only = match port {
                    Some(name) => Some(port_of(self, &name)?),
                    None => None,
                };
                let mut out = Vec::new();
                if let Some(Some(Run::Rstp(r))) = self.stp_mut(ni).map(|s| s.run.as_mut()) {
                    r.mcheck(only, &mut out);
                }
                self.stp_out(ni, None, out);
            }
        }
        Ok(())
    }

    /// what a port of node `ni` gives the protocol: (takes part now, path cost, point-to-point)
    fn stp_port_now(&self, ni: usize, pi: u16) -> (bool, u32, bool) {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return (false, 1, true);
        };
        let Some(b) = n.bridge.as_ref() else {
            return (false, 1, true);
        };
        let cfg = b.stp.cfg.port(pi);
        let member = b.ports.get(pi as usize).is_some_and(|bp| bp.member);
        let port = &n.ports[pi as usize];
        let li = port.link;
        let up = n.power && member && li.is_some_and(|l| self.link_up(l));
        let kbps = li.map(|l| self.link_kbps(l)).unwrap_or(port.spec.kbps);
        let cost = cfg.cost.unwrap_or_else(|| bpdu::path_cost(kbps));
        // a hub at the far end makes the LAN shared (half duplex)
        let hub = li.and_then(|l| self.links.get(l)?.as_ref()).and_then(|l| {
            let far = if l.ends[0] == (ni, pi) { l.ends[1] } else { l.ends[0] };
            self.nodes.get(far.0)?.as_ref().map(|x| x.model.has(F_HUB))
        });
        let p2p = match cfg.link {
            LinkType::Auto => hub != Some(true),
            LinkType::P2p => true,
            LinkType::Shared => false,
        };
        (up, cost, p2p)
    }

    /// starts (or restarts) the protocol of node `ni` from its settings (802.1D BEGIN)
    pub(crate) fn stp_start(&mut self, ni: usize) {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return;
        };
        if !n.power {
            return;
        }
        let Some(b) = n.bridge.as_ref() else {
            return;
        };
        let cfg = b.stp.cfg.clone();
        let Some(mode) = cfg.mode else {
            return;
        };
        let nports = b.ports.len();
        let members: Vec<bool> = b.ports.iter().map(|p| p.member).collect();
        let id = BridgeId::new(cfg.priority, n.base_mac);
        let ports: Vec<(bool, u32, bool)> = (0..nports as u16).map(|p| self.stp_port_now(ni, p)).collect();
        let ids: Vec<PortId> = (0..nports as u16).map(|p| PortId::new(cfg.port(p).priority, p + 1)).collect();
        let edges: Vec<bool> = (0..nports as u16).map(|p| cfg.port(p).edge).collect();
        let mut out = Vec::new();
        let run = match mode.proto() {
            Proto::Stp => Run::Stp(Stp98::new(id, cfg.hello, cfg.fwd_delay, cfg.max_age, &members, &ids, &edges)),
            Proto::Rstp => Run::Rstp(Rstp::new(id, cfg.hello, cfg.fwd_delay, cfg.max_age, &members, &ids, &edges)),
        };
        if let Some(Some(n)) = self.nodes.get_mut(ni)
            && let Some(b) = n.bridge.as_mut()
        {
            // a restart (another protocol) begins with normal aging
            b.fast_aging = None;
            b.stp.run = Some(run);
            match b.stp.run.as_mut() {
                Some(Run::Stp(x)) => x.begin(&ports, &mut out),
                Some(Run::Rstp(x)) => x.begin(&ports, &mut out),
                None => {}
            }
        }
        let now = self.now;
        self.set_timer(ni, SLOT_STP, now + SEC);
        self.stp_out(ni, None, out);
    }

    /// the startup configuration is through: the protocol it asks for starts
    pub(crate) fn stp_booted(&mut self, ni: usize) {
        if !self.stp_runs(ni) && self.stp_of(ni).is_some_and(|s| s.cfg.mode.is_some()) {
            self.stp_start(ni);
        }
    }

    /// spanning tree off: every port forwards again, MAC entries age as usual
    pub(crate) fn stp_stop(&mut self, ni: usize) {
        if let Some(Some(n)) = self.nodes.get_mut(ni)
            && let Some(b) = n.bridge.as_mut()
        {
            b.stp.run = None;
            b.fast_aging = None;
        }
        self.cancel_timer(ni, SLOT_STP);
        self.ios_snap_if_ios(ni);
    }

    /// node timer SLOT_STP: one tick of the protocol
    pub(crate) fn stp_tick(&mut self, ni: usize) {
        let mut out = Vec::new();
        match self.stp_mut(ni).and_then(|s| s.run.as_mut()) {
            Some(Run::Stp(x)) => x.tick(&mut out),
            Some(Run::Rstp(x)) => x.tick(&mut out),
            None => return,
        }
        let now = self.now;
        self.set_timer(ni, SLOT_STP, now + SEC);
        self.stp_out(ni, None, out);
    }

    /// a port's link, membership or settings changed: the protocol hears it
    pub(crate) fn stp_link(&mut self, ni: usize, pi: u16) {
        if !self.stp_runs(ni) {
            return;
        }
        let (up, cost, p2p) = self.stp_port_now(ni, pi);
        let mut out = Vec::new();
        match self.stp_mut(ni).and_then(|s| s.run.as_mut()) {
            Some(Run::Stp(x)) => x.port(pi, up, cost, p2p, &mut out),
            Some(Run::Rstp(x)) => x.port(pi, up, cost, p2p, &mut out),
            None => return,
        }
        self.stp_out(ni, None, out);
    }

    /// power: off forgets the protocol's state (the settings stay), on starts it again
    pub(crate) fn stp_power(&mut self, ni: usize, on: bool) {
        if on {
            if !self.stp_runs(ni) {
                self.stp_start(ni);
            }
        } else if let Some(s) = self.stp_mut(ni) {
            s.run = None;
            if let Some(Some(n)) = self.nodes.get_mut(ni)
                && let Some(b) = n.bridge.as_mut()
            {
                b.fast_aging = None;
            }
        }
    }

    /// a link-local frame arrived at a bridge port (bridge_input): true when spanning tree or
    /// BPDU guard took it
    pub(crate) fn stp_bpdu(&mut self, ni: usize, pi: u16, frame: &[u8], hop: HopId) -> bool {
        let Some(b) = bpdu::of_frame(frame) else {
            return false;
        };
        let (guard, on) = match self.stp_of(ni) {
            Some(s) => (s.cfg.port(pi).guard, s.on()),
            None => return false,
        };
        let dev = self.dev_of(ni);
        if guard {
            self.stp_err_disable(ni, pi, hop);
            return true;
        }
        if !on {
            return false;
        }
        self.note(dev, Some(hop), Note::StpRx { port: pi, bpdu: b });
        let mut out = Vec::new();
        match self.stp_mut(ni).and_then(|s| s.run.as_mut()) {
            Some(Run::Stp(x)) => x.rx(pi, &b, &mut out),
            Some(Run::Rstp(x)) => x.rx(pi, &b, &mut out),
            None => {}
        }
        self.stp_out(ni, Some(hop), out);
        true
    }

    /// BPDU guard: the port is shut off until "shutdown" and "no shutdown"
    fn stp_err_disable(&mut self, ni: usize, pi: u16, hop: HopId) {
        let dev = self.dev_of(ni);
        let (name, li) = match self.nodes.get_mut(ni) {
            Some(Some(n)) => {
                n.ports[pi as usize].err_disabled = true;
                (n.ports[pi as usize].spec.name.clone(), n.ports[pi as usize].link)
            }
            _ => return,
        };
        self.note(dev, Some(hop), Note::StpErrDisable { port: pi });
        if self.cdev(ni).is_some_and(|c| c.style == jfn_model::catalog::CliStyle::Ios) {
            self.ios_log(ni, &format!("%SPANTREE-2-BLOCK_BPDUGUARD: a BPDU arrived on {}, where BPDU guard is on: the port is shut off", name));
            self.ios_log(ni, &format!("%PM-4-ERR_DISABLE: {} is err-disabled (bpduguard); shutdown and no shutdown bring it back", name));
        }
        if let Some(li) = li {
            self.refresh_link(li);
        }
    }

    /// logs interface changes of an IOS device (VLAN interfaces follow the forwarding ports)
    fn ios_snap_if_ios(&mut self, ni: usize) {
        if self.cdev(ni).is_some_and(|c| c.style == jfn_model::catalog::CliStyle::Ios) {
            self.ios_snap(ni);
        }
    }

    /// carries out what the protocol asked for
    fn stp_out(&mut self, ni: usize, cause: Option<HopId>, outs: Vec<Out>) {
        if outs.is_empty() {
            return;
        }
        let dev = self.dev_of(ni);
        let mut states = false;
        for o in outs {
            match o {
                Out::Tx { port, bpdu } => {
                    let mac = match self.nodes.get(ni) {
                        Some(Some(n)) => n.ports[port as usize].mac,
                        _ => continue,
                    };
                    self.note(dev, cause, Note::StpTx { port, bpdu });
                    self.send(ni, port, Arc::from(bpdu.frame(mac)), cause);
                }
                Out::Note(n) => {
                    if let Note::StpState { .. } = n {
                        states = true;
                    }
                    if let Note::StpTopology { what: TcWhat::Detected, .. } = n {
                        let now = self.now;
                        if let Some(s) = self.stp_mut(ni) {
                            s.tc_count += 1;
                            s.tc_last = Some(now);
                        }
                    }
                    self.note(dev, cause, n);
                }
                Out::Flush { port } => {
                    let n = match self.nodes.get_mut(ni) {
                        Some(Some(n)) => match n.bridge.as_mut() {
                            Some(b) => {
                                let before = b.fdb.len();
                                b.flush_port(port);
                                (before - b.fdb.len()) as u32
                            }
                            None => 0,
                        },
                        _ => 0,
                    };
                    if n > 0 {
                        self.note(dev, cause, Note::StpFlush { port, entries: n });
                    }
                }
                Out::Aging(a) => {
                    let now = self.now;
                    let normal = match self.nodes.get_mut(ni) {
                        Some(Some(n)) => match n.bridge.as_mut() {
                            Some(b) => {
                                let was = b.fast_aging;
                                b.fast_aging = a.map(|s| s as Ns * SEC);
                                // entries that aged out fast stay out
                                if a.is_none()
                                    && let Some(f) = was
                                {
                                    b.fdb.retain(|_, e| e.fixed || now.saturating_sub(e.last) <= f);
                                }
                                b.aging / SEC
                            }
                            None => continue,
                        },
                        _ => continue,
                    };
                    self.note(dev, cause, Note::StpAging { secs: a.unwrap_or(normal as u16), fast: a.is_some() });
                }
            }
        }
        if states {
            // VLAN interfaces are up while a port of their VLAN forwards (log.rs, l3.rs)
            self.ios_snap_if_ios(ni);
        }
    }
}
