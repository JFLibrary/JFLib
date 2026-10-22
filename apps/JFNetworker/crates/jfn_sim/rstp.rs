// rstp -- the Rapid Spanning Tree Protocol after IEEE 802.1D-2004 clause 17 (802.1w): the
// state machines of 17.22 .. 17.29 over the variables of 17.17 .. 17.20, for one bridge
//
//   Rstp::new / begin(ports)           the bridge and its ports; BEGIN puts every machine in its
//                                      first state
//   rx(port, bpdu)                     a BPDU arrived (Port Receive and what follows)
//   tick()                             one second (Port Timers, 17.22)
//   port(p, up, cost, p2p)             portEnabled, PortPathCost and operPointToPointMAC
//   set_bridge_id / set_times / set_port_id / set_edge / mcheck   management (17.13)
//   state(p), view(mode)
//
// Machines: Port Information (PIM, 17.27), Port Role Selection (PRS, 17.28), Port Role
// Transitions (PRT, 17.29), Port State Transition (PST, 17.30), Topology Change (TCM, 17.31),
// Port Protocol Migration (PPM, 17.24), Port Transmit (PTX, 17.26), Port Receive (PRX, 17.23),
// Bridge Detection (BDM, 17.25).  After every event they run until none of them moves: first
// all but Port Transmit, then Port Transmit, so that a BPDU tells the settled state.  A global
// transition (BEGIN, a port that is not enabled) holds its machine in that state.
//
// What the standard leaves to an implementation, done here: rstpVersion is TRUE (Force Protocol
// Version 2; "spanning-tree mode stp" runs stp98.rs instead); the filtering database forgets a
// port's entries at once when fdbFlush is set; AutoEdge is TRUE (a designated port that hears
// no BPDU for EdgeDelay becomes an edge port), AdminEdge is "spanning-tree portfast"; TxHoldCount
// is 6; a TCN BPDU sets rcvdTcn in Port Information (it carries no priority vector); a BPDU
// with this port's own bridge and port ID came back over a loop and is dropped.  Times count in
// whole seconds; Message Age grows by one second per bridge.

use alloc::vec::Vec;

use netpkt::stp::{Bpdu, BridgeId, Kind, PortId, Role as BRole, Times, F_AGREEMENT, F_FORWARDING, F_LEARNING, F_PROPOSAL, F_TC, F_TCA};

use crate::stp::{BpduDrop, ModeWord, Out, PortRole, PortState, Proto, StpPortView, TcWhat, TreeView};
use crate::trace::Note;

/// Migrate Time (17.13.9)
const MIGRATE: u16 = 3;
/// Transmit Hold Count (17.13.12)
const TX_HOLD: u16 = 6;

/// a spanning tree priority vector (17.5, 17.6): root bridge, root path cost, designated bridge,
/// designated port, and the port it was received on; lower is better
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Vector {
    pub root: BridgeId,
    pub cost: u32,
    pub bridge: BridgeId,
    pub port: PortId,
    pub rx: PortId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InfoIs {
    Received,
    Mine,
    Aged,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RcvdInfo {
    SuperiorDesignated,
    RepeatedDesignated,
    InferiorDesignated,
    InferiorRootAlternate,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pim {
    Disabled,
    Aged,
    Update,
    Current,
    Receive,
    SuperiorDesignated,
    RepeatedDesignated,
    InferiorDesignated,
    NotDesignated,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prt {
    InitPort,
    DisablePort,
    DisabledPort,
    RootPort,
    RootProposed,
    RootAgreed,
    Reroot,
    RootForward,
    RootLearn,
    Rerooted,
    DesignatedPort,
    DesignatedPropose,
    DesignatedSynced,
    DesignatedRetired,
    DesignatedDiscard,
    DesignatedLearn,
    DesignatedForward,
    BlockPort,
    AlternatePort,
    AlternateProposed,
    AlternateAgreed,
    BackupPort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pst {
    Discarding,
    Learning,
    Forwarding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tcm {
    Inactive,
    Learning,
    Detected,
    Active,
    NotifiedTcn,
    NotifiedTc,
    Propagating,
    Acknowledged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ppm {
    CheckingRstp,
    SelectingStp,
    Sensing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ptx {
    TransmitInit,
    Idle,
    TransmitPeriodic,
    TransmitConfig,
    TransmitTcn,
    TransmitRstp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prx {
    Discard,
    Receive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bdm {
    Edge,
    NotEdge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prs {
    InitBridge,
    RoleSelection,
}

/// a port with the variables of 17.19 and its timers (17.17)
#[derive(Clone, Debug)]
pub struct RPort {
    pub member: bool,
    pub id: PortId,
    pub cost: u32,
    pub admin_edge: bool,
    pub auto_edge: bool,
    /// operPointToPointMAC
    pub p2p: bool,
    /// portEnabled
    pub enabled: bool,
    agree: bool,
    agreed: bool,
    designated_priority: Vector,
    designated_times: Times,
    disputed: bool,
    fdb_flush: bool,
    forward: bool,
    forwarding: bool,
    info_is: InfoIs,
    learn: bool,
    learning: bool,
    mcheck: bool,
    msg_priority: Vector,
    msg_times: Times,
    new_info: bool,
    pub oper_edge: bool,
    port_priority: Vector,
    port_times: Times,
    proposed: bool,
    proposing: bool,
    rcvd_bpdu: bool,
    rcvd_info: RcvdInfo,
    rcvd_msg: bool,
    rcvd_rstp: bool,
    rcvd_stp: bool,
    rcvd_tc: bool,
    rcvd_tc_ack: bool,
    rcvd_tcn: bool,
    re_root: bool,
    reselect: bool,
    pub role: PortRole,
    selected: bool,
    selected_role: PortRole,
    pub send_rstp: bool,
    sync: bool,
    synced: bool,
    tc_ack: bool,
    tc_prop: bool,
    tx_count: u16,
    updt_info: bool,
    // timers
    edge_delay_while: u16,
    fd_while: u16,
    hello_when: u16,
    mdelay_while: u16,
    rb_while: u16,
    rcvd_info_while: u16,
    rr_while: u16,
    tc_while: u16,
    /// the BPDU of rcvdBpdu
    bpdu: Option<Bpdu>,
    pim: Pim,
    prt: Prt,
    pst: Pst,
    tcm: Tcm,
    ppm: Ppm,
    ptx: Ptx,
    prx: Prx,
    bdm: Bdm,
    pub tx: u64,
    pub rx: u64,
    pub to_fwd: u32,
}

/// the bridge with the variables of 17.18
#[derive(Clone, Debug)]
pub struct Rstp {
    pub id: BridgeId,
    /// BridgeTimes (17.18.4)
    pub times: Times,
    pub root_priority: Vector,
    pub root_port_id: PortId,
    pub root_times: Times,
    pub ports: Vec<RPort>,
    prs: Prs,
    /// the root and root port last shown
    shown: Option<(BridgeId, Option<u16>)>,
}

fn role_bits(r: PortRole) -> BRole {
    match r {
        PortRole::Root => BRole::Root,
        PortRole::Designated => BRole::Designated,
        PortRole::Alternate | PortRole::Backup => BRole::AltBackup,
        PortRole::Disabled => BRole::Unknown,
    }
}

impl Rstp {
    pub fn new(id: BridgeId, hello: u16, fwd_delay: u16, max_age: u16, members: &[bool], ids: &[PortId], edges: &[bool]) -> Rstp {
        let times = Times { msg_age: 0, max_age, hello, fwd_delay };
        let ports = members
            .iter()
            .enumerate()
            .map(|(i, &member)| RPort {
                member,
                id: ids[i],
                cost: 1,
                admin_edge: edges[i],
                auto_edge: true,
                p2p: true,
                enabled: false,
                agree: false,
                agreed: false,
                designated_priority: Vector::default(),
                designated_times: times,
                disputed: false,
                fdb_flush: false,
                forward: false,
                forwarding: false,
                info_is: InfoIs::Disabled,
                learn: false,
                learning: false,
                mcheck: false,
                msg_priority: Vector::default(),
                msg_times: times,
                new_info: false,
                oper_edge: false,
                port_priority: Vector::default(),
                port_times: times,
                proposed: false,
                proposing: false,
                rcvd_bpdu: false,
                rcvd_info: RcvdInfo::Other,
                rcvd_msg: false,
                rcvd_rstp: false,
                rcvd_stp: false,
                rcvd_tc: false,
                rcvd_tc_ack: false,
                rcvd_tcn: false,
                re_root: false,
                reselect: false,
                role: PortRole::Disabled,
                selected: false,
                selected_role: PortRole::Disabled,
                send_rstp: true,
                sync: false,
                synced: false,
                tc_ack: false,
                tc_prop: false,
                tx_count: 0,
                updt_info: false,
                edge_delay_while: 0,
                fd_while: 0,
                hello_when: 0,
                mdelay_while: 0,
                rb_while: 0,
                rcvd_info_while: 0,
                rr_while: 0,
                tc_while: 0,
                bpdu: None,
                pim: Pim::Disabled,
                prt: Prt::InitPort,
                pst: Pst::Discarding,
                tcm: Tcm::Inactive,
                ppm: Ppm::CheckingRstp,
                ptx: Ptx::TransmitInit,
                prx: Prx::Discard,
                bdm: Bdm::NotEdge,
                tx: 0,
                rx: 0,
                to_fwd: 0,
            })
            .collect();
        let bp = Vector { root: id, cost: 0, bridge: id, port: PortId(0), rx: PortId(0) };
        Rstp { id, times, root_priority: bp, root_port_id: PortId(0), root_times: times, ports, prs: Prs::InitBridge, shown: None }
    }

    /// BEGIN: every machine in its first state (with its actions), then the machines run
    pub(crate) fn begin(&mut self, ports: &[(bool, u32, bool)], out: &mut Vec<Out>) {
        self.shown = Some((self.id, None));
        for (i, &(up, cost, p2p)) in ports.iter().enumerate() {
            let p = &mut self.ports[i];
            p.enabled = up && p.member;
            p.cost = cost;
            p.p2p = p2p;
        }
        for i in 0..self.ports.len() {
            self.pim_to(i, Pim::Disabled, out);
        }
        self.prs_to(Prs::InitBridge, out);
        for i in 0..self.ports.len() {
            // INIT_PORT goes on to DISABLE_PORT while every selectedRole is still DisabledPort
            self.prt_to(i, Prt::InitPort, out);
            self.prt_to(i, Prt::DisablePort, out);
            self.pst_to(i, Pst::Discarding, out);
            self.tcm_to(i, Tcm::Inactive, out);
            self.ppm_to(i, Ppm::CheckingRstp, out);
            self.ptx_to(i, Ptx::TransmitInit, out);
            self.prx_to(i, Prx::Discard, out);
            let e = if self.ports[i].admin_edge { Bdm::Edge } else { Bdm::NotEdge };
            self.bdm_to(i, e, out);
        }
        self.run(out);
    }

    /// the state of a port as the bridge uses it
    pub fn state(&self, p: u16) -> PortState {
        match self.ports.get(p as usize) {
            Some(x) if x.enabled && x.forwarding => PortState::Forwarding,
            Some(x) if x.enabled && x.learning => PortState::Learning,
            Some(x) if x.enabled => PortState::Discarding,
            _ => PortState::Disabled,
        }
    }

    // ---------------------------------------------------------------- conditions (17.20)

    fn fwd_delay(&self, i: usize) -> u16 {
        self.ports[i].designated_times.fwd_delay
    }
    fn hello_time(&self, i: usize) -> u16 {
        self.ports[i].designated_times.hello
    }
    fn max_age(&self, i: usize) -> u16 {
        self.ports[i].designated_times.max_age
    }
    /// forwardDelay (17.20.5): HelloTime while the port speaks RSTP, FwdDelay otherwise
    fn forward_delay(&self, i: usize) -> u16 {
        if self.ports[i].send_rstp { self.hello_time(i) } else { self.fwd_delay(i) }
    }
    fn edge_delay(&self, i: usize) -> u16 {
        if self.ports[i].p2p { MIGRATE } else { self.max_age(i) }
    }
    fn all_synced(&self) -> bool {
        self.ports.iter().filter(|p| p.member).all(|p| p.selected && p.role == p.selected_role && !p.updt_info && (p.synced || p.role == PortRole::Root))
    }
    fn re_rooted(&self, i: usize) -> bool {
        self.ports.iter().enumerate().all(|(j, p)| j == i || p.rr_while == 0)
    }

    // ---------------------------------------------------------------- procedures (17.21)

    fn betterorsame_info(&self, i: usize, new: InfoIs) -> bool {
        let p = &self.ports[i];
        (new == InfoIs::Received && p.info_is == InfoIs::Received && p.msg_priority <= p.port_priority) || (new == InfoIs::Mine && p.info_is == InfoIs::Mine && p.designated_priority <= p.port_priority)
    }

    /// rcvInfo (17.21.8)
    fn rcv_info(&mut self, i: usize) -> RcvdInfo {
        let p = &mut self.ports[i];
        let Some(b) = p.bpdu else {
            return RcvdInfo::Other;
        };
        if b.kind == Kind::Tcn {
            return RcvdInfo::Other;
        }
        let t = b.times();
        p.msg_priority = Vector { root: b.root, cost: b.cost, bridge: b.bridge, port: b.port, rx: p.id };
        p.msg_times = Times { hello: t.hello.max(1), ..t };
        let (m, pp) = (p.msg_priority, p.port_priority);
        let designated = b.kind == Kind::Config || b.role() == BRole::Designated;
        if designated {
            // superior: better, or a new word from the same designated port (17.6)
            let same = m.bridge.mac() == pp.bridge.mac() && m.port.number() == pp.port.number();
            if m < pp || (same && m != pp) || (m == pp && p.msg_times != p.port_times) {
                return RcvdInfo::SuperiorDesignated;
            }
            if m == pp {
                return RcvdInfo::RepeatedDesignated;
            }
            return RcvdInfo::InferiorDesignated;
        }
        if matches!(b.role(), BRole::Root | BRole::AltBackup) && m >= pp {
            return RcvdInfo::InferiorRootAlternate;
        }
        RcvdInfo::Other
    }

    fn record_proposal(&mut self, i: usize) {
        let p = &mut self.ports[i];
        if let Some(b) = p.bpdu
            && b.role() == BRole::Designated
            && b.proposal()
        {
            p.proposed = true;
        }
    }

    fn record_agreement(&mut self, i: usize) {
        let p = &mut self.ports[i];
        match p.bpdu {
            Some(b) if p.p2p && b.agreement() => {
                p.agreed = true;
                p.proposing = false;
            }
            _ => p.agreed = false,
        }
    }

    fn record_dispute(&mut self, i: usize) {
        let p = &mut self.ports[i];
        if p.bpdu.is_some_and(|b| b.learning()) {
            p.disputed = true;
            p.agreed = false;
        }
    }

    fn set_tc_flags(&mut self, i: usize) {
        let p = &mut self.ports[i];
        if let Some(b) = p.bpdu {
            match b.kind {
                Kind::Tcn => p.rcvd_tcn = true,
                _ => {
                    if b.tc() {
                        p.rcvd_tc = true;
                    }
                    if b.tca() {
                        p.rcvd_tc_ack = true;
                    }
                }
            }
        }
    }

    fn updt_rcvd_info_while(&mut self, i: usize) {
        let p = &mut self.ports[i];
        let t = p.port_times;
        p.rcvd_info_while = if t.msg_age + 1 <= t.max_age { 3 * t.hello } else { 0 };
    }

    /// newTcWhile (17.21.7)
    fn new_tc_while(&mut self, i: usize) {
        let (rt, hello) = (self.root_times, self.hello_time(i));
        let p = &mut self.ports[i];
        if p.tc_while == 0 {
            if p.send_rstp {
                p.tc_while = hello + 1;
                p.new_info = true;
            } else {
                p.tc_while = rt.max_age + rt.fwd_delay;
            }
        }
    }

    fn set_sync_tree(&mut self) {
        for p in self.ports.iter_mut() {
            p.sync = true;
        }
    }
    fn set_reroot_tree(&mut self) {
        for p in self.ports.iter_mut() {
            p.re_root = true;
        }
    }
    fn set_tc_prop_tree(&mut self, except: usize) {
        for (j, p) in self.ports.iter_mut().enumerate() {
            if j != except {
                p.tc_prop = true;
            }
        }
    }

    /// updtRolesTree (17.21.25): the root priority vector, the designated priority vectors and
    /// times, and every port's role
    fn updt_roles_tree(&mut self, out: &mut Vec<Out>) {
        let me = self.id.mac();
        let mut best = Vector { root: self.id, cost: 0, bridge: self.id, port: PortId(0), rx: PortId(0) };
        let mut root: Option<usize> = None;
        for (i, p) in self.ports.iter().enumerate() {
            if !p.member || p.info_is != InfoIs::Received || p.port_priority.bridge.mac() == me {
                continue;
            }
            let pp = p.port_priority;
            let v = Vector { root: pp.root, cost: pp.cost.saturating_add(p.cost), bridge: pp.bridge, port: pp.port, rx: p.id };
            if v < best {
                best = v;
                root = Some(i);
            }
        }
        self.root_priority = best;
        self.root_port_id = best.rx;
        self.root_times = match root {
            None => self.times,
            Some(r) => {
                let t = self.ports[r].port_times;
                Times { msg_age: t.msg_age + 1, ..t }
            }
        };
        let hello = self.times.hello;
        let rt = self.root_times;
        for p in self.ports.iter_mut() {
            p.designated_priority = Vector { root: best.root, cost: best.cost, bridge: self.id, port: p.id, rx: p.id };
            p.designated_times = Times { hello, ..rt };
        }
        for (i, p) in self.ports.iter_mut().enumerate() {
            match p.info_is {
                InfoIs::Disabled => p.selected_role = PortRole::Disabled,
                InfoIs::Aged => {
                    p.updt_info = true;
                    p.selected_role = PortRole::Designated;
                }
                InfoIs::Mine => {
                    p.selected_role = PortRole::Designated;
                    if p.port_priority != p.designated_priority || p.port_times != p.designated_times {
                        p.updt_info = true;
                    }
                }
                InfoIs::Received => {
                    if root == Some(i) {
                        p.selected_role = PortRole::Root;
                        p.updt_info = false;
                    } else if p.designated_priority >= p.port_priority {
                        // the designated priority is not better: someone else is designated
                        p.selected_role = if p.port_priority.bridge.mac() == me { PortRole::Backup } else { PortRole::Alternate };
                        p.updt_info = false;
                    } else {
                        p.selected_role = PortRole::Designated;
                        p.updt_info = true;
                    }
                }
            }
        }
        let now = (best.root, root.map(|r| r as u16));
        if self.shown != Some(now) {
            self.shown = Some(now);
            out.push(Out::Note(Note::StpRoot { root: best.root, port: now.1, cost: best.cost }));
        }
    }

    fn tx_rstp(&mut self, i: usize, out: &mut Vec<Out>) {
        let p = &self.ports[i];
        let d = p.designated_priority;
        let mut flags = role_bits(p.role).bits();
        if p.proposing {
            flags |= F_PROPOSAL;
        }
        if p.agree {
            flags |= F_AGREEMENT;
        }
        if p.learning {
            flags |= F_LEARNING;
        }
        if p.forwarding {
            flags |= F_FORWARDING;
        }
        if p.tc_while != 0 {
            flags |= F_TC;
        }
        let b = Bpdu::rst(flags, d.root, d.cost, d.bridge, d.port, p.designated_times);
        out.push(Out::Tx { port: i as u16, bpdu: b });
        self.ports[i].tx += 1;
    }

    fn tx_config(&mut self, i: usize, out: &mut Vec<Out>) {
        let p = &self.ports[i];
        let d = p.designated_priority;
        let mut flags = 0;
        if p.tc_while != 0 {
            flags |= F_TC;
        }
        if p.tc_ack {
            flags |= F_TCA;
        }
        let b = Bpdu::config(flags, d.root, d.cost, d.bridge, d.port, p.designated_times);
        out.push(Out::Tx { port: i as u16, bpdu: b });
        self.ports[i].tx += 1;
    }

    fn tx_tcn(&mut self, i: usize, out: &mut Vec<Out>) {
        out.push(Out::Tx { port: i as u16, bpdu: Bpdu::tcn() });
        self.ports[i].tx += 1;
        out.push(Out::Note(Note::StpTopology { port: Some(i as u16), what: TcWhat::TcnSent }));
    }

    // ---------------------------------------------------------------- the machines

    /// runs the machines until none moves (Port Transmit last)
    fn run(&mut self, out: &mut Vec<Out>) {
        let n = self.ports.len();
        for _ in 0..10_000 {
            let mut moved = false;
            for i in 0..n {
                moved |= self.prx(i, out);
                moved |= self.ppm(i, out);
                moved |= self.bdm(i, out);
                moved |= self.pim(i, out);
            }
            moved |= self.prs(out);
            for i in 0..n {
                moved |= self.prt(i, out);
                moved |= self.pst(i, out);
                moved |= self.tcm(i, out);
            }
            if moved {
                continue;
            }
            for i in 0..n {
                moved |= self.ptx(i, out);
            }
            if !moved {
                return;
            }
        }
    }

    // ---- Port Receive (17.23)

    fn prx_to(&mut self, i: usize, s: Prx, _out: &mut Vec<Out>) {
        let p = &mut self.ports[i];
        p.prx = s;
        match s {
            Prx::Discard => {
                p.rcvd_bpdu = false;
                p.rcvd_rstp = false;
                p.rcvd_stp = false;
                p.rcvd_msg = false;
                p.edge_delay_while = MIGRATE;
            }
            Prx::Receive => {
                // updtBPDUVersion
                match p.bpdu.map(|b| b.kind) {
                    Some(Kind::Rst) => p.rcvd_rstp = true,
                    Some(_) => p.rcvd_stp = true,
                    None => {}
                }
                p.oper_edge = false;
                p.rcvd_bpdu = false;
                p.rcvd_msg = true;
                p.edge_delay_while = MIGRATE;
            }
        }
    }

    fn prx(&mut self, i: usize, out: &mut Vec<Out>) -> bool {
        let p = &self.ports[i];
        if (p.rcvd_bpdu || p.edge_delay_while != MIGRATE) && !p.enabled {
            self.prx_to(i, Prx::Discard, out);
            return true;
        }
        let go = match p.prx {
            Prx::Discard => p.rcvd_bpdu && p.enabled,
            Prx::Receive => p.rcvd_bpdu && p.enabled && !p.rcvd_msg,
        };
        if go {
            let edge = p.oper_edge;
            self.prx_to(i, Prx::Receive, out);
            if edge {
                out.push(Out::Note(Note::StpEdge { port: i as u16, edge: false }));
            }
        }
        go
    }

    // ---- Port Protocol Migration (17.24)

    fn ppm_to(&mut self, i: usize, s: Ppm, out: &mut Vec<Out>) {
        let p = &mut self.ports[i];
        let was = p.send_rstp;
        p.ppm = s;
        match s {
            Ppm::CheckingRstp => {
                p.mcheck = false;
                p.send_rstp = true;
                p.mdelay_while = MIGRATE;
            }
            Ppm::SelectingStp => {
                p.send_rstp = false;
                p.mdelay_while = MIGRATE;
            }
            Ppm::Sensing => {
                p.rcvd_rstp = false;
                p.rcvd_stp = false;
            }
        }
        if was != p.send_rstp && p.enabled {
            out.push(Out::Note(Note::StpMigrate { port: i as u16, rstp: p.send_rstp }));
        }
    }

    fn ppm(&mut self, i: usize, out: &mut Vec<Out>) -> bool {
        let p = &self.ports[i];
        let next = match p.ppm {
            Ppm::CheckingRstp if p.mdelay_while != MIGRATE && !p.enabled => Some(Ppm::CheckingRstp),
            Ppm::CheckingRstp if p.mdelay_while == 0 => Some(Ppm::Sensing),
            Ppm::SelectingStp if p.mdelay_while == 0 || !p.enabled || p.mcheck => Some(Ppm::Sensing),
            Ppm::Sensing if !p.enabled || p.mcheck || (!p.send_rstp && p.rcvd_rstp) => Some(Ppm::CheckingRstp),
            Ppm::Sensing if p.send_rstp && p.rcvd_stp => Some(Ppm::SelectingStp),
            _ => None,
        };
        match next {
            Some(s) => {
                self.ppm_to(i, s, out);
                true
            }
            None => false,
        }
    }

    // ---- Bridge Detection (17.25)

    fn bdm_to(&mut self, i: usize, s: Bdm, out: &mut Vec<Out>) {
        let p = &mut self.ports[i];
        let was = p.oper_edge;
        p.bdm = s;
        p.oper_edge = s == Bdm::Edge;
        if was != p.oper_edge && p.enabled {
            out.push(Out::Note(Note::StpEdge { port: i as u16, edge: p.oper_edge }));
        }
    }

    fn bdm(&mut self, i: usize, out: &mut Vec<Out>) -> bool {
        let p = &self.ports[i];
        let next = match p.bdm {
            Bdm::Edge if (!p.enabled && !p.admin_edge) || !p.oper_edge => Some(Bdm::NotEdge),
            Bdm::NotEdge if (!p.enabled && p.admin_edge) || (p.edge_delay_while == 0 && p.auto_edge && p.send_rstp && p.proposing) => Some(Bdm::Edge),
            _ => None,
        };
        match next {
            Some(s) => {
                self.bdm_to(i, s, out);
                true
            }
            None => false,
        }
    }

    // ---- Port Information (17.27)

    fn pim_to(&mut self, i: usize, s: Pim, out: &mut Vec<Out>) {
        self.ports[i].pim = s;
        match s {
            Pim::Disabled => {
                let p = &mut self.ports[i];
                p.rcvd_msg = false;
                p.proposing = false;
                p.proposed = false;
                p.agree = false;
                p.agreed = false;
                p.rcvd_info_while = 0;
                p.info_is = InfoIs::Disabled;
                p.reselect = true;
                p.selected = false;
            }
            Pim::Aged => {
                let p = &mut self.ports[i];
                p.info_is = InfoIs::Aged;
                p.reselect = true;
                p.selected = false;
            }
            Pim::Update => {
                let mine = self.betterorsame_info(i, InfoIs::Mine);
                let p = &mut self.ports[i];
                p.proposing = false;
                p.proposed = false;
                p.agreed = p.agreed && mine;
                p.synced = p.synced && p.agreed;
                p.port_priority = p.designated_priority;
                p.port_times = p.designated_times;
                p.updt_info = false;
                p.info_is = InfoIs::Mine;
                p.new_info = true;
            }
            Pim::Current => {}
            Pim::Receive => {
                let r = self.rcv_info(i);
                self.ports[i].rcvd_info = r;
            }
            Pim::SuperiorDesignated => {
                self.ports[i].agreed = false;
                self.ports[i].proposing = false;
                self.record_proposal(i);
                self.set_tc_flags(i);
                let same = self.betterorsame_info(i, InfoIs::Received);
                let p = &mut self.ports[i];
                p.agree = p.agree && same;
                p.port_priority = p.msg_priority;
                p.port_times = p.msg_times;
                self.updt_rcvd_info_while(i);
                let p = &mut self.ports[i];
                p.info_is = InfoIs::Received;
                p.reselect = true;
                p.selected = false;
                p.rcvd_msg = false;
            }
            Pim::RepeatedDesignated => {
                self.record_proposal(i);
                self.set_tc_flags(i);
                self.updt_rcvd_info_while(i);
                self.ports[i].rcvd_msg = false;
            }
            Pim::InferiorDesignated => {
                self.record_dispute(i);
                self.ports[i].rcvd_msg = false;
            }
            Pim::NotDesignated => {
                self.record_agreement(i);
                self.set_tc_flags(i);
                self.ports[i].rcvd_msg = false;
            }
            Pim::Other => {
                // a TCN BPDU: no priority vector, but a topology change
                if self.ports[i].bpdu.is_some_and(|b| b.kind == Kind::Tcn) {
                    self.set_tc_flags(i);
                }
                self.ports[i].rcvd_msg = false;
            }
        }
        let _ = out;
    }

    fn pim(&mut self, i: usize, out: &mut Vec<Out>) -> bool {
        let p = &self.ports[i];
        if !p.enabled && p.info_is != InfoIs::Disabled {
            self.pim_to(i, Pim::Disabled, out);
            return true;
        }
        let next = match p.pim {
            Pim::Disabled if p.rcvd_msg => Some(Pim::Disabled),
            Pim::Disabled if p.enabled => Some(Pim::Aged),
            Pim::Aged if p.selected && p.updt_info => Some(Pim::Update),
            Pim::Update => Some(Pim::Current),
            Pim::Current if p.selected && p.updt_info => Some(Pim::Update),
            Pim::Current if p.info_is == InfoIs::Received && p.rcvd_info_while == 0 && !p.updt_info && !p.rcvd_msg => {
                out.push(Out::Note(Note::StpAged { port: i as u16 }));
                Some(Pim::Aged)
            }
            Pim::Current if p.rcvd_msg && !p.updt_info => Some(Pim::Receive),
            Pim::Receive => Some(match p.rcvd_info {
                RcvdInfo::SuperiorDesignated => Pim::SuperiorDesignated,
                RcvdInfo::RepeatedDesignated => Pim::RepeatedDesignated,
                RcvdInfo::InferiorDesignated => Pim::InferiorDesignated,
                RcvdInfo::InferiorRootAlternate => Pim::NotDesignated,
                RcvdInfo::Other => Pim::Other,
            }),
            Pim::SuperiorDesignated | Pim::RepeatedDesignated | Pim::InferiorDesignated | Pim::NotDesignated | Pim::Other => Some(Pim::Current),
            _ => None,
        };
        match next {
            Some(s) => {
                self.pim_to(i, s, out);
                true
            }
            None => false,
        }
    }

    // ---- Port Role Selection (17.28)

    fn prs_to(&mut self, s: Prs, out: &mut Vec<Out>) {
        self.prs = s;
        match s {
            Prs::InitBridge => {
                for p in self.ports.iter_mut() {
                    p.selected_role = PortRole::Disabled;
                }
            }
            Prs::RoleSelection => {
                for p in self.ports.iter_mut() {
                    p.reselect = false;
                }
                self.updt_roles_tree(out);
                // setSelectedTree
                if self.ports.iter().all(|p| !p.reselect) {
                    for p in self.ports.iter_mut() {
                        p.selected = true;
                    }
                }
            }
        }
    }

    fn prs(&mut self, out: &mut Vec<Out>) -> bool {
        let go = match self.prs {
            Prs::InitBridge => true,
            Prs::RoleSelection => self.ports.iter().any(|p| p.reselect),
        };
        if go {
            self.prs_to(Prs::RoleSelection, out);
        }
        go
    }

    // ---- Port Role Transitions (17.29)

    fn set_role(&mut self, i: usize, r: PortRole, out: &mut Vec<Out>) {
        let p = &mut self.ports[i];
        if p.role != r {
            let from = p.role;
            p.role = r;
            if p.member && !(from == PortRole::Disabled && r == PortRole::Disabled) {
                out.push(Out::Note(Note::StpRole { port: i as u16, from, to: r }));
            }
        }
    }

    fn prt_to(&mut self, i: usize, s: Prt, out: &mut Vec<Out>) {
        self.ports[i].prt = s;
        let (fd, ma, hello, fwdd) = (self.fwd_delay(i), self.max_age(i), self.hello_time(i), self.forward_delay(i));
        match s {
            Prt::InitPort => {
                self.set_role(i, PortRole::Disabled, out);
                let p = &mut self.ports[i];
                p.learn = false;
                p.forward = false;
                p.synced = false;
                p.sync = true;
                p.re_root = true;
                p.rr_while = fd;
                p.fd_while = ma;
                p.rb_while = 0;
            }
            Prt::DisablePort => {
                let r = self.ports[i].selected_role;
                self.set_role(i, r, out);
                let p = &mut self.ports[i];
                p.learn = false;
                p.forward = false;
            }
            Prt::DisabledPort => {
                let p = &mut self.ports[i];
                p.fd_while = ma;
                p.synced = true;
                p.rr_while = 0;
                p.sync = false;
                p.re_root = false;
            }
            Prt::RootPort => {
                self.set_role(i, PortRole::Root, out);
                self.ports[i].rr_while = fd;
            }
            Prt::RootProposed => {
                self.set_sync_tree();
                self.ports[i].proposed = false;
            }
            Prt::RootAgreed => {
                let p = &mut self.ports[i];
                p.proposed = false;
                p.sync = false;
                p.agree = true;
                p.new_info = true;
            }
            Prt::Reroot => self.set_reroot_tree(),
            Prt::RootForward => {
                let p = &mut self.ports[i];
                p.fd_while = 0;
                p.forward = true;
            }
            Prt::RootLearn => {
                let p = &mut self.ports[i];
                p.fd_while = fwdd;
                p.learn = true;
            }
            Prt::Rerooted => self.ports[i].re_root = false,
            Prt::DesignatedPort => self.set_role(i, PortRole::Designated, out),
            Prt::DesignatedPropose => {
                let ed = self.edge_delay(i);
                let p = &mut self.ports[i];
                p.proposing = true;
                p.edge_delay_while = ed;
                p.new_info = true;
            }
            Prt::DesignatedSynced => {
                let p = &mut self.ports[i];
                p.rr_while = 0;
                p.synced = true;
                p.sync = false;
            }
            Prt::DesignatedRetired => self.ports[i].re_root = false,
            Prt::DesignatedDiscard => {
                let p = &mut self.ports[i];
                p.learn = false;
                p.forward = false;
                p.disputed = false;
                p.fd_while = fwdd;
            }
            Prt::DesignatedLearn => {
                let p = &mut self.ports[i];
                p.learn = true;
                p.fd_while = fwdd;
            }
            Prt::DesignatedForward => {
                let p = &mut self.ports[i];
                p.forward = true;
                p.fd_while = 0;
                p.agreed = p.send_rstp;
            }
            Prt::BlockPort => {
                let r = self.ports[i].selected_role;
                self.set_role(i, r, out);
                let p = &mut self.ports[i];
                p.learn = false;
                p.forward = false;
            }
            Prt::AlternatePort => {
                let p = &mut self.ports[i];
                p.fd_while = fwdd;
                p.synced = true;
                p.rr_while = 0;
                p.sync = false;
                p.re_root = false;
            }
            Prt::AlternateProposed => {
                self.set_sync_tree();
                self.ports[i].proposed = false;
            }
            Prt::AlternateAgreed => {
                let p = &mut self.ports[i];
                p.proposed = false;
                p.agree = true;
                p.new_info = true;
            }
            Prt::BackupPort => self.ports[i].rb_while = 2 * hello,
        }
    }

    fn prt(&mut self, i: usize, out: &mut Vec<Out>) -> bool {
        let next = self.prt_next(i);
        match next {
            Some(s) => {
                self.prt_to(i, s, out);
                true
            }
            None => false,
        }
    }

    fn prt_next(&self, i: usize) -> Option<Prt> {
        let p = &self.ports[i];
        let q = p.selected && !p.updt_info;
        // the role changes (all qualified by selected && !updtInfo)
        if q && p.role != p.selected_role {
            return Some(match p.selected_role {
                PortRole::Disabled => Prt::DisablePort,
                PortRole::Root => Prt::RootPort,
                PortRole::Designated => Prt::DesignatedPort,
                PortRole::Alternate | PortRole::Backup => Prt::BlockPort,
            });
        }
        let (fd, ma, hello, fwdd) = (self.fwd_delay(i), self.max_age(i), self.hello_time(i), self.forward_delay(i));
        match p.prt {
            Prt::InitPort => Some(Prt::DisablePort),
            Prt::DisablePort if q && !p.learning && !p.forwarding => Some(Prt::DisabledPort),
            Prt::DisabledPort if q && (p.fd_while != ma || p.sync || p.re_root || !p.synced) => Some(Prt::DisabledPort),
            Prt::RootProposed | Prt::RootAgreed | Prt::Reroot | Prt::RootForward | Prt::RootLearn | Prt::Rerooted => Some(Prt::RootPort),
            Prt::RootPort if q => {
                let fast = p.fd_while == 0 || (self.re_rooted(i) && p.rb_while == 0);
                if p.proposed && !p.agree {
                    Some(Prt::RootProposed)
                } else if (self.all_synced() && !p.agree) || (p.proposed && p.agree) {
                    Some(Prt::RootAgreed)
                } else if !p.forward && !p.re_root {
                    Some(Prt::Reroot)
                } else if p.rr_while != fd {
                    Some(Prt::RootPort)
                } else if p.re_root && p.forward {
                    Some(Prt::Rerooted)
                } else if fast && !p.learn {
                    Some(Prt::RootLearn)
                } else if fast && p.learn && !p.forward {
                    Some(Prt::RootForward)
                } else {
                    None
                }
            }
            Prt::DesignatedPropose | Prt::DesignatedSynced | Prt::DesignatedRetired | Prt::DesignatedDiscard | Prt::DesignatedLearn | Prt::DesignatedForward => Some(Prt::DesignatedPort),
            Prt::DesignatedPort if q => {
                let may = (p.fd_while == 0 || p.agreed || p.oper_edge) && (p.rr_while == 0 || !p.re_root) && !p.sync;
                if !p.forward && !p.agreed && !p.proposing && !p.oper_edge {
                    Some(Prt::DesignatedPropose)
                } else if (!p.learning && !p.forwarding && !p.synced) || (p.agreed && !p.synced) || (p.oper_edge && !p.synced) || (p.sync && p.synced) {
                    Some(Prt::DesignatedSynced)
                } else if p.rr_while == 0 && p.re_root {
                    Some(Prt::DesignatedRetired)
                } else if ((p.sync && !p.synced) || (p.re_root && p.rr_while != 0) || p.disputed) && !p.oper_edge && (p.learn || p.forward) {
                    Some(Prt::DesignatedDiscard)
                } else if may && !p.learn {
                    Some(Prt::DesignatedLearn)
                } else if may && p.learn && !p.forward {
                    Some(Prt::DesignatedForward)
                } else {
                    None
                }
            }
            Prt::BlockPort if q && !p.learning && !p.forwarding => Some(Prt::AlternatePort),
            Prt::AlternateProposed | Prt::AlternateAgreed | Prt::BackupPort => Some(Prt::AlternatePort),
            Prt::AlternatePort if q => {
                if p.proposed && !p.agree {
                    Some(Prt::AlternateProposed)
                } else if (self.all_synced() && !p.agree) || (p.proposed && p.agree) {
                    Some(Prt::AlternateAgreed)
                } else if p.fd_while != fwdd || p.sync || p.re_root || !p.synced {
                    Some(Prt::AlternatePort)
                } else if p.rb_while != 2 * hello && p.role == PortRole::Backup {
                    Some(Prt::BackupPort)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    // ---- Port State Transition (17.30)

    fn pst_to(&mut self, i: usize, s: Pst, out: &mut Vec<Out>) {
        let from = self.state(i as u16);
        let p = &mut self.ports[i];
        p.pst = s;
        match s {
            Pst::Discarding => {
                p.learning = false;
                p.forwarding = false;
            }
            Pst::Learning => p.learning = true,
            Pst::Forwarding => {
                p.forwarding = true;
                p.to_fwd += 1;
            }
        }
        let to = self.state(i as u16);
        if from != to && self.ports[i].enabled {
            out.push(Out::Note(Note::StpState { port: i as u16, from, to }));
        }
    }

    fn pst(&mut self, i: usize, out: &mut Vec<Out>) -> bool {
        let p = &self.ports[i];
        let next = match p.pst {
            Pst::Discarding if p.learn => Some(Pst::Learning),
            Pst::Learning if !p.learn => Some(Pst::Discarding),
            Pst::Learning if p.forward => Some(Pst::Forwarding),
            Pst::Forwarding if !p.forward => Some(Pst::Discarding),
            _ => None,
        };
        match next {
            Some(s) => {
                self.pst_to(i, s, out);
                true
            }
            None => false,
        }
    }

    // ---- Topology Change (17.31)

    fn tcm_to(&mut self, i: usize, s: Tcm, out: &mut Vec<Out>) {
        self.ports[i].tcm = s;
        match s {
            Tcm::Inactive => {
                let p = &mut self.ports[i];
                p.fdb_flush = true;
                p.tc_while = 0;
                if p.role != PortRole::Designated {
                    p.tc_ack = false;
                }
            }
            Tcm::Learning => {
                let p = &mut self.ports[i];
                p.rcvd_tc = false;
                p.rcvd_tcn = false;
                p.rcvd_tc_ack = false;
                p.tc_prop = false;
            }
            Tcm::Detected => {
                self.new_tc_while(i);
                self.set_tc_prop_tree(i);
                self.ports[i].new_info = true;
                out.push(Out::Note(Note::StpTopology { port: Some(i as u16), what: TcWhat::Detected }));
            }
            Tcm::Active => {}
            Tcm::NotifiedTcn => {
                self.new_tc_while(i);
                out.push(Out::Note(Note::StpTopology { port: Some(i as u16), what: TcWhat::TcnReceived }));
            }
            Tcm::NotifiedTc => {
                let tcn = self.ports[i].rcvd_tcn;
                let p = &mut self.ports[i];
                p.rcvd_tcn = false;
                p.rcvd_tc = false;
                if p.role == PortRole::Designated {
                    p.tc_ack = true;
                }
                self.set_tc_prop_tree(i);
                if !tcn {
                    out.push(Out::Note(Note::StpTopology { port: Some(i as u16), what: TcWhat::Received }));
                }
            }
            Tcm::Propagating => {
                self.new_tc_while(i);
                let p = &mut self.ports[i];
                p.fdb_flush = true;
                p.tc_prop = false;
            }
            Tcm::Acknowledged => {
                let p = &mut self.ports[i];
                p.tc_while = 0;
                p.rcvd_tc_ack = false;
                out.push(Out::Note(Note::StpTopology { port: Some(i as u16), what: TcWhat::Acked }));
            }
        }
        // the filtering database forgets at once (rstpVersion)
        if self.ports[i].fdb_flush {
            self.ports[i].fdb_flush = false;
            out.push(Out::Flush { port: i as u16 });
        }
    }

    fn tcm(&mut self, i: usize, out: &mut Vec<Out>) -> bool {
        let p = &self.ports[i];
        let active = p.role == PortRole::Root || p.role == PortRole::Designated;
        let any = p.rcvd_tc || p.rcvd_tcn || p.rcvd_tc_ack || p.tc_prop;
        // (the flags are cleared in LEARNING whatever the role, as 802.1Q has it: a port that
        // left the active roles with tcProp set would wait there for ever)
        let next = match p.tcm {
            Tcm::Inactive if p.learn && !p.fdb_flush => Some(Tcm::Learning),
            Tcm::Learning if active && p.forward && !p.oper_edge => Some(Tcm::Detected),
            Tcm::Learning if any => Some(Tcm::Learning),
            Tcm::Learning if !active && !(p.learn || p.learning) => Some(Tcm::Inactive),
            Tcm::Detected | Tcm::NotifiedTc | Tcm::Propagating | Tcm::Acknowledged => Some(Tcm::Active),
            Tcm::NotifiedTcn => Some(Tcm::NotifiedTc),
            Tcm::Active if !active || p.oper_edge => Some(Tcm::Learning),
            Tcm::Active if p.rcvd_tcn => Some(Tcm::NotifiedTcn),
            Tcm::Active if p.rcvd_tc => Some(Tcm::NotifiedTc),
            Tcm::Active if p.tc_prop && !p.oper_edge => Some(Tcm::Propagating),
            Tcm::Active if p.rcvd_tc_ack => Some(Tcm::Acknowledged),
            _ => None,
        };
        match next {
            Some(s) => {
                self.tcm_to(i, s, out);
                true
            }
            None => false,
        }
    }

    // ---- Port Transmit (17.26)

    fn ptx_to(&mut self, i: usize, s: Ptx, out: &mut Vec<Out>) {
        self.ports[i].ptx = s;
        match s {
            Ptx::TransmitInit => {
                let p = &mut self.ports[i];
                p.new_info = true;
                p.tx_count = 0;
            }
            Ptx::Idle => {
                let h = self.hello_time(i);
                self.ports[i].hello_when = h;
            }
            Ptx::TransmitPeriodic => {
                let p = &mut self.ports[i];
                p.new_info = p.new_info || p.role == PortRole::Designated || (p.role == PortRole::Root && p.tc_while != 0);
            }
            Ptx::TransmitConfig => {
                self.ports[i].new_info = false;
                self.tx_config(i, out);
                let p = &mut self.ports[i];
                p.tx_count += 1;
                p.tc_ack = false;
            }
            Ptx::TransmitTcn => {
                self.ports[i].new_info = false;
                self.tx_tcn(i, out);
                self.ports[i].tx_count += 1;
            }
            Ptx::TransmitRstp => {
                self.ports[i].new_info = false;
                self.tx_rstp(i, out);
                let p = &mut self.ports[i];
                p.tx_count += 1;
                p.tc_ack = false;
            }
        }
    }

    fn ptx(&mut self, i: usize, out: &mut Vec<Out>) -> bool {
        let p = &self.ports[i];
        if !p.enabled {
            if p.ptx != Ptx::TransmitInit {
                self.ptx_to(i, Ptx::TransmitInit, out);
                return true;
            }
            return false;
        }
        let q = p.selected && !p.updt_info;
        let next = match p.ptx {
            Ptx::TransmitInit | Ptx::TransmitPeriodic | Ptx::TransmitConfig | Ptx::TransmitTcn | Ptx::TransmitRstp => Some(Ptx::Idle),
            Ptx::Idle if q => {
                let room = p.tx_count < TX_HOLD && p.hello_when != 0;
                if p.hello_when == 0 {
                    Some(Ptx::TransmitPeriodic)
                } else if p.send_rstp && p.new_info && room {
                    Some(Ptx::TransmitRstp)
                } else if !p.send_rstp && p.new_info && p.role == PortRole::Root && room {
                    Some(Ptx::TransmitTcn)
                } else if !p.send_rstp && p.new_info && p.role == PortRole::Designated && room {
                    Some(Ptx::TransmitConfig)
                } else {
                    None
                }
            }
            _ => None,
        };
        match next {
            Some(s) => {
                self.ptx_to(i, s, out);
                true
            }
            None => false,
        }
    }

    // ---------------------------------------------------------------- events

    /// a BPDU arrived on port `i`
    pub(crate) fn rx(&mut self, i: u16, b: &Bpdu, out: &mut Vec<Out>) {
        let i = i as usize;
        let Some(p) = self.ports.get_mut(i) else {
            return;
        };
        if !p.enabled {
            return;
        }
        // 9.3.4: a Message Age at Max Age, or one's own BPDU back on the same port
        if b.kind != Kind::Tcn && (b.msg_age >= b.max_age || (b.bridge == self.id && b.port == p.id)) {
            let why = if b.msg_age >= b.max_age { BpduDrop::Aged } else { BpduDrop::Looped };
            out.push(Out::Note(Note::StpDropped { port: i as u16, why }));
            return;
        }
        p.rx += 1;
        p.bpdu = Some(*b);
        p.rcvd_bpdu = true;
        self.run(out);
    }

    /// one second: the Port Timers machine decrements every timer (17.22)
    pub(crate) fn tick(&mut self, out: &mut Vec<Out>) {
        for p in self.ports.iter_mut() {
            for t in [&mut p.hello_when, &mut p.tc_while, &mut p.fd_while, &mut p.rcvd_info_while, &mut p.rr_while, &mut p.rb_while, &mut p.mdelay_while, &mut p.edge_delay_while, &mut p.tx_count] {
                *t = t.saturating_sub(1);
            }
        }
        self.run(out);
    }

    /// portEnabled, PortPathCost and operPointToPointMAC of a port
    pub(crate) fn port(&mut self, i: u16, up: bool, cost: u32, p2p: bool, out: &mut Vec<Out>) {
        let i = i as usize;
        let Some(p) = self.ports.get_mut(i) else {
            return;
        };
        if up {
            p.member = true;
        }
        let changed = p.enabled != up || p.cost != cost;
        p.enabled = up;
        p.p2p = p2p;
        if p.cost != cost {
            p.cost = cost;
            // a new path cost: the roles are chosen again (17.13.11)
            p.reselect = true;
            p.selected = false;
        }
        if changed || up {
            self.run(out);
        }
    }

    /// a new bridge priority (17.13.6): the roles are chosen again
    pub(crate) fn set_bridge_id(&mut self, id: BridgeId, out: &mut Vec<Out>) {
        self.id = id;
        for p in self.ports.iter_mut() {
            p.reselect = true;
            p.selected = false;
        }
        self.run(out);
    }

    /// new bridge timers (17.13.4, .5, .8)
    pub(crate) fn set_times(&mut self, hello: u16, fwd_delay: u16, max_age: u16, out: &mut Vec<Out>) {
        self.times = Times { msg_age: 0, max_age, hello, fwd_delay };
        for p in self.ports.iter_mut() {
            p.reselect = true;
            p.selected = false;
        }
        self.run(out);
    }

    /// a new port priority (17.13.10)
    pub(crate) fn set_port_id(&mut self, i: u16, id: PortId, out: &mut Vec<Out>) {
        if let Some(p) = self.ports.get_mut(i as usize) {
            p.id = id;
            p.reselect = true;
            p.selected = false;
            self.run(out);
        }
    }

    /// AdminEdge ("spanning-tree portfast"); it takes effect at once
    pub(crate) fn set_edge(&mut self, i: u16, on: bool, out: &mut Vec<Out>) {
        let i = i as usize;
        if i >= self.ports.len() {
            return;
        }
        self.ports[i].admin_edge = on;
        if self.ports[i].enabled && self.ports[i].oper_edge != on {
            self.bdm_to(i, if on { Bdm::Edge } else { Bdm::NotEdge }, out);
        }
        self.run(out);
    }

    /// mcheck (17.19.13): try RSTP again on one port or on all
    pub(crate) fn mcheck(&mut self, only: Option<u16>, out: &mut Vec<Out>) {
        for (i, p) in self.ports.iter_mut().enumerate() {
            if only.is_none_or(|o| o as usize == i) && p.enabled {
                p.mcheck = true;
            }
        }
        self.run(out);
    }

    pub fn view(&self, mode: ModeWord) -> TreeView {
        let rp = self.ports.iter().position(|p| p.enabled && p.role == PortRole::Root).map(|r| r as u16);
        let ports = (0..self.ports.len())
            .filter(|&i| self.ports[i].member && self.ports[i].enabled)
            .map(|i| {
                let p = &self.ports[i];
                // the designated vector of its LAN: received, or its own
                let d = if p.info_is == InfoIs::Received { p.port_priority } else { p.designated_priority };
                StpPortView {
                    port: i as u16,
                    id: p.id,
                    role: p.role,
                    state: self.state(i as u16),
                    cost: p.cost,
                    edge: p.oper_edge,
                    p2p: p.p2p,
                    stp_peer: !p.send_rstp,
                    d_root: d.root,
                    d_cost: d.cost,
                    d_bridge: d.bridge,
                    d_port: d.port,
                    fwd_timer: p.fd_while,
                    info_age: p.rcvd_info_while,
                    tx: p.tx,
                    rx: p.rx,
                    to_fwd: p.to_fwd,
                }
            })
            .collect();
        TreeView {
            proto: Proto::Rstp,
            mode,
            bridge: self.id,
            root: self.root_priority.root,
            root_cost: self.root_priority.cost,
            root_port: rp,
            hello: self.root_times.hello,
            max_age: self.root_times.max_age,
            fwd_delay: self.root_times.fwd_delay,
            b_hello: self.times.hello,
            b_max_age: self.times.max_age,
            b_fwd_delay: self.times.fwd_delay,
            tc: self.ports.iter().any(|p| p.enabled && p.tc_while != 0),
            tc_count: 0,
            tc_last: None,
            ports,
        }
    }
}
