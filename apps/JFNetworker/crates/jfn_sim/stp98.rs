// stp98 -- the spanning tree algorithm of IEEE 802.1D-1998 clause 8 (STP): root and designated
// port selection, the port states Blocking, Listening, Learning and Forwarding, configuration
// BPDUs relayed from the root, topology change notification (TCN up the root ports until the
// acknowledgment, the TC flag of the root for Max Age + Forward Delay, MAC entries aging after
// Forward Delay meanwhile)
//
//   Stp98::new / begin(ports)          the bridge and its ports (initialisation, 8.8.1)
//   rx(port, bpdu)                     a Configuration or TCN BPDU arrived (8.7.1, 8.7.2)
//   tick()                             one second: hello, TCN, topology change, message age,
//                                      forward delay and hold timers (8.7.3 .. 8.7.8)
//   port(p, up, cost)                  enable / disable a port (8.8.1, 8.8.2), its path cost
//   set_bridge_id / set_times / set_port_id / set_edge   management (8.8.4 .. 8.8.6)
//   view(mode)                         the tree for show commands
//
// The procedures keep the names and the order of the standard's.  RST BPDUs are not for this
// protocol: a bridge of 1998 does not know their type and drops them (9.3.4), which is why RSTP
// bridges fall back to Configuration BPDUs on such a port.  Timers count whole seconds; the
// message age of a relayed BPDU is the age of the root port's information plus one second.  A
// BPDU with this port's own bridge and port ID came back over a loop and is dropped (9.3.4).
//
// Edge ports ("spanning-tree portfast", not part of 802.1D-1998): a designated edge port goes to
// Forwarding at once and makes no topology change; the first BPDU it hears makes it a normal
// port again until its link goes down.  Roles for show commands: the root port, designated
// ports, and the others as alternate ports (backup ports when the designated port of their LAN
// is one of this bridge's).

use alloc::vec::Vec;

use netpkt::stp::{Bpdu, BridgeId, Kind, PortId, Times, F_TC, F_TCA};

use crate::stp::{BpduDrop, ModeWord, Out, PortRole, PortState, Proto, StpPortView, TcWhat, TreeView};
use crate::trace::Note;

/// Hold Time (8.10.2): at most one Configuration BPDU per port in this many seconds
const HOLD: u16 = 1;
/// the overestimate of the message age a relay adds (8.10.2)
const AGE_INCR: u16 = 1;

/// a port of the bridge (8.5.5)
#[derive(Clone, Debug)]
pub struct Port98 {
    /// takes part in bridging (an Ethernet port that is not routed)
    pub member: bool,
    pub id: PortId,
    pub state: PortState,
    pub cost: u32,
    pub d_root: BridgeId,
    pub d_cost: u32,
    pub d_bridge: BridgeId,
    pub d_port: PortId,
    pub tca: bool,
    pub config_pending: bool,
    /// the message age timer: the age of the stored information in seconds (None: stopped),
    /// and the Max Age it came with
    pub age: Option<u16>,
    pub age_max: u16,
    pub fwd_timer: Option<u16>,
    pub hold_timer: Option<u16>,
    pub admin_edge: bool,
    pub oper_edge: bool,
    /// the link type (shown only: 802.1D-1998 does not use it)
    pub p2p: bool,
    /// the role last shown
    pub role: PortRole,
    pub tx: u64,
    pub rx: u64,
    pub to_fwd: u32,
}

/// the bridge (8.5.3)
#[derive(Clone, Debug)]
pub struct Stp98 {
    pub id: BridgeId,
    pub root: BridgeId,
    pub root_cost: u32,
    pub root_port: Option<u16>,
    /// the timers in use (learned from the root) and the bridge's own
    pub max_age: u16,
    pub hello: u16,
    pub fwd_delay: u16,
    pub b_max_age: u16,
    pub b_hello: u16,
    pub b_fwd_delay: u16,
    pub tc_detected: bool,
    pub tc: bool,
    pub hello_timer: Option<u16>,
    pub tcn_timer: Option<u16>,
    pub tc_timer: Option<u16>,
    pub ports: Vec<Port98>,
    /// the root and root port last shown
    shown: Option<(BridgeId, Option<u16>)>,
}

impl Stp98 {
    pub fn new(id: BridgeId, hello: u16, fwd_delay: u16, max_age: u16, members: &[bool], ids: &[PortId], edges: &[bool]) -> Stp98 {
        let ports = members
            .iter()
            .enumerate()
            .map(|(i, &member)| Port98 {
                member,
                id: ids[i],
                state: PortState::Disabled,
                cost: 1,
                d_root: id,
                d_cost: 0,
                d_bridge: id,
                d_port: ids[i],
                tca: false,
                config_pending: false,
                age: None,
                age_max: max_age,
                fwd_timer: None,
                hold_timer: None,
                admin_edge: edges[i],
                oper_edge: false,
                p2p: true,
                role: PortRole::Disabled,
                tx: 0,
                rx: 0,
                to_fwd: 0,
            })
            .collect();
        Stp98 {
            id,
            root: id,
            root_cost: 0,
            root_port: None,
            max_age,
            hello,
            fwd_delay,
            b_max_age: max_age,
            b_hello: hello,
            b_fwd_delay: fwd_delay,
            tc_detected: false,
            tc: false,
            hello_timer: None,
            tcn_timer: None,
            tc_timer: None,
            ports,
            shown: None,
        }
    }

    /// initialisation (8.8.1): every port that is up starts as a designated port, the bridge
    /// as the root; the first Configuration BPDUs go out now
    pub(crate) fn begin(&mut self, ports: &[(bool, u32, bool)], out: &mut Vec<Out>) {
        self.shown = Some((self.id, None));
        for (i, &(up, cost, p2p)) in ports.iter().enumerate() {
            let p = &mut self.ports[i];
            p.cost = cost;
            p.p2p = p2p;
            if up && p.member {
                self.initialize_port(i);
            }
        }
        self.port_state_selection(out);
        self.config_bpdu_generation(out);
        self.hello_timer = Some(self.hello);
        self.update_roles(out);
    }

    fn is_root(&self) -> bool {
        self.root == self.id
    }
    fn designated(&self, i: usize) -> bool {
        let p = &self.ports[i];
        p.d_bridge == self.id && p.d_port == p.id
    }
    fn up(&self, i: usize) -> bool {
        self.ports[i].state != PortState::Disabled
    }

    fn initialize_port(&mut self, i: usize) {
        self.become_designated_port(i);
        let p = &mut self.ports[i];
        p.state = PortState::Blocking;
        p.tca = false;
        p.config_pending = false;
        p.age = None;
        p.fwd_timer = None;
        p.hold_timer = None;
        p.oper_edge = p.admin_edge;
    }

    fn become_designated_port(&mut self, i: usize) {
        let (root, cost, id) = (self.root, self.root_cost, self.id);
        let p = &mut self.ports[i];
        p.d_root = root;
        p.d_cost = cost;
        p.d_bridge = id;
        p.d_port = p.id;
    }

    // ---------------------------------------------------------------- selection (8.6.8, 8.6.9)

    fn better_root_port(&self, i: usize, rp: Option<usize>) -> bool {
        let p = &self.ports[i];
        if !self.up(i) || self.designated(i) || p.d_root >= self.id {
            return false;
        }
        let Some(r) = rp else {
            return true;
        };
        let r = &self.ports[r];
        (p.d_root, p.d_cost + p.cost, p.d_bridge, p.d_port, p.id) < (r.d_root, r.d_cost + r.cost, r.d_bridge, r.d_port, r.id)
    }

    fn root_selection(&mut self) {
        let mut rp: Option<usize> = None;
        for i in 0..self.ports.len() {
            if self.better_root_port(i, rp) {
                rp = Some(i);
            }
        }
        self.root_port = rp.map(|r| r as u16);
        match rp {
            None => {
                self.root = self.id;
                self.root_cost = 0;
            }
            Some(r) => {
                self.root = self.ports[r].d_root;
                self.root_cost = self.ports[r].d_cost + self.ports[r].cost;
            }
        }
    }

    fn should_become_designated(&self, i: usize) -> bool {
        let p = &self.ports[i];
        if self.designated(i) || p.d_root != self.root {
            return true;
        }
        (self.root_cost, self.id, p.id) < (p.d_cost, p.d_bridge, p.d_port)
    }

    fn designated_port_selection(&mut self) {
        for i in 0..self.ports.len() {
            if self.up(i) && self.should_become_designated(i) {
                self.become_designated_port(i);
            }
        }
    }

    fn configuration_update(&mut self, out: &mut Vec<Out>) {
        self.root_selection();
        self.designated_port_selection();
        let now = (self.root, self.root_port);
        if self.shown != Some(now) {
            self.shown = Some(now);
            let port = self.root_port;
            out.push(Out::Note(Note::StpRoot { root: self.root, port, cost: self.root_cost }));
        }
    }

    // ---------------------------------------------------------------- port states (8.6.10 .. 8.6.13)

    fn set_state(&mut self, i: usize, s: PortState, out: &mut Vec<Out>) {
        let p = &mut self.ports[i];
        if p.state == s {
            return;
        }
        let from = p.state;
        p.state = s;
        if s == PortState::Forwarding {
            p.to_fwd += 1;
        }
        out.push(Out::Note(Note::StpState { port: i as u16, from, to: s }));
    }

    fn make_forwarding(&mut self, i: usize, out: &mut Vec<Out>) {
        if self.ports[i].state != PortState::Blocking {
            return;
        }
        if self.ports[i].oper_edge {
            // an edge port has no loop to fear
            self.set_state(i, PortState::Forwarding, out);
            return;
        }
        self.set_state(i, PortState::Listening, out);
        self.ports[i].fwd_timer = Some(self.fwd_delay);
    }

    fn make_blocking(&mut self, i: usize, out: &mut Vec<Out>) {
        let s = self.ports[i].state;
        if s != PortState::Disabled && s != PortState::Blocking {
            if (s == PortState::Forwarding || s == PortState::Learning) && !self.ports[i].oper_edge {
                self.topology_change_detection(Some(i), out);
            }
            self.set_state(i, PortState::Blocking, out);
            self.ports[i].fwd_timer = None;
        }
    }

    fn port_state_selection(&mut self, out: &mut Vec<Out>) {
        for i in 0..self.ports.len() {
            if !self.up(i) {
                continue;
            }
            if Some(i as u16) == self.root_port {
                self.ports[i].config_pending = false;
                self.ports[i].tca = false;
                self.make_forwarding(i, out);
            } else if self.designated(i) {
                self.ports[i].age = None;
                self.make_forwarding(i, out);
            } else {
                self.ports[i].config_pending = false;
                self.ports[i].tca = false;
                self.make_blocking(i, out);
            }
        }
    }

    /// the roles as show commands name them; a note for every change
    fn update_roles(&mut self, out: &mut Vec<Out>) {
        for i in 0..self.ports.len() {
            let r = if !self.up(i) {
                PortRole::Disabled
            } else if Some(i as u16) == self.root_port {
                PortRole::Root
            } else if self.designated(i) {
                PortRole::Designated
            } else if self.ports[i].d_bridge == self.id {
                PortRole::Backup
            } else {
                PortRole::Alternate
            };
            let p = &mut self.ports[i];
            if p.role != r {
                let from = p.role;
                p.role = r;
                if self.ports[i].member && !(from == PortRole::Disabled && r == PortRole::Disabled) {
                    out.push(Out::Note(Note::StpRole { port: i as u16, from, to: r }));
                }
            }
        }
    }

    // ---------------------------------------------------------------- BPDUs out (8.6.1, 8.6.6)

    fn transmit_config(&mut self, i: usize, out: &mut Vec<Out>) {
        if self.ports[i].hold_timer.is_some() {
            self.ports[i].config_pending = true;
            return;
        }
        let msg_age = match self.root_port {
            Some(r) if !self.is_root() => self.ports[r as usize].age.unwrap_or(0) + AGE_INCR,
            _ => 0,
        };
        if msg_age >= self.max_age {
            return;
        }
        let p = &self.ports[i];
        let mut flags = 0;
        if self.tc {
            flags |= F_TC;
        }
        if p.tca {
            flags |= F_TCA;
        }
        let t = Times { msg_age, max_age: self.max_age, hello: self.hello, fwd_delay: self.fwd_delay };
        let b = Bpdu::config(flags, self.root, self.root_cost, self.id, p.id, t);
        out.push(Out::Tx { port: i as u16, bpdu: b });
        let p = &mut self.ports[i];
        p.tx += 1;
        p.tca = false;
        p.config_pending = false;
        p.hold_timer = Some(HOLD);
    }

    fn config_bpdu_generation(&mut self, out: &mut Vec<Out>) {
        for i in 0..self.ports.len() {
            if self.up(i) && self.designated(i) {
                self.transmit_config(i, out);
            }
        }
    }

    fn transmit_tcn(&mut self, out: &mut Vec<Out>) {
        if let Some(r) = self.root_port {
            out.push(Out::Tx { port: r, bpdu: Bpdu::tcn() });
            self.ports[r as usize].tx += 1;
            out.push(Out::Note(Note::StpTopology { port: Some(r), what: TcWhat::TcnSent }));
        }
    }

    // ---------------------------------------------------------------- topology change (8.6.14 ..)

    fn set_tc(&mut self, on: bool, out: &mut Vec<Out>) {
        if self.tc != on {
            self.tc = on;
            out.push(Out::Aging(if on { Some(self.fwd_delay) } else { None }));
        }
    }

    fn topology_change_detection(&mut self, port: Option<usize>, out: &mut Vec<Out>) {
        out.push(Out::Note(Note::StpTopology { port: port.map(|p| p as u16), what: TcWhat::Detected }));
        if self.is_root() {
            if !self.tc {
                out.push(Out::Note(Note::StpTopology { port: None, what: TcWhat::RootFlag(true) }));
            }
            self.set_tc(true, out);
            self.tc_timer = Some(self.b_max_age + self.b_fwd_delay);
        } else if !self.tc_detected {
            self.transmit_tcn(out);
            self.tcn_timer = Some(self.b_hello);
        }
        self.tc_detected = true;
    }

    fn topology_change_acknowledged(&mut self, out: &mut Vec<Out>) {
        if self.tc_detected {
            out.push(Out::Note(Note::StpTopology { port: self.root_port, what: TcWhat::Acked }));
        }
        self.tc_detected = false;
        self.tcn_timer = None;
    }

    fn become_root_bridge(&mut self, out: &mut Vec<Out>) {
        self.max_age = self.b_max_age;
        self.hello = self.b_hello;
        self.fwd_delay = self.b_fwd_delay;
        self.topology_change_detection(None, out);
        self.tcn_timer = None;
        self.config_bpdu_generation(out);
        self.hello_timer = Some(self.hello);
    }

    // ---------------------------------------------------------------- BPDUs in (8.7.1, 8.7.2)

    fn supersedes(&self, i: usize, b: &Bpdu) -> bool {
        let p = &self.ports[i];
        if b.root != p.d_root {
            return b.root < p.d_root;
        }
        if b.cost != p.d_cost {
            return b.cost < p.d_cost;
        }
        if b.bridge != p.d_bridge {
            return b.bridge < p.d_bridge;
        }
        b.bridge != self.id || b.port <= p.d_port
    }

    /// a BPDU arrived on port `i`
    pub(crate) fn rx(&mut self, i: u16, b: &Bpdu, out: &mut Vec<Out>) {
        let i = i as usize;
        if i >= self.ports.len() || !self.up(i) {
            return;
        }
        // 9.3.4: RST BPDUs are of a type this protocol does not know; a Message Age at Max Age
        // or one's own BPDU back on the same port is dropped
        let why = match b.kind {
            Kind::Rst => Some(BpduDrop::Type),
            Kind::Config if b.msg_age >= b.max_age => Some(BpduDrop::Aged),
            Kind::Config if b.bridge == self.id && b.port == self.ports[i].id => Some(BpduDrop::Looped),
            _ => None,
        };
        if let Some(why) = why {
            out.push(Out::Note(Note::StpDropped { port: i as u16, why }));
            return;
        }
        self.ports[i].rx += 1;
        if self.ports[i].oper_edge {
            self.ports[i].oper_edge = false;
            out.push(Out::Note(Note::StpEdge { port: i as u16, edge: false }));
        }
        match b.kind {
            Kind::Config => self.received_config_bpdu(i, b, out),
            Kind::Tcn => self.received_tcn_bpdu(i, out),
            Kind::Rst => {}
        }
        self.update_roles(out);
    }

    fn received_config_bpdu(&mut self, i: usize, b: &Bpdu, out: &mut Vec<Out>) {
        let was_root = self.is_root();
        if self.supersedes(i, b) {
            let t = b.times();
            let p = &mut self.ports[i];
            p.d_root = b.root;
            p.d_cost = b.cost;
            p.d_bridge = b.bridge;
            p.d_port = b.port;
            p.age = Some(t.msg_age);
            p.age_max = t.max_age;
            self.configuration_update(out);
            self.port_state_selection(out);
            if !self.is_root() && was_root {
                self.hello_timer = None;
                if self.tc_detected {
                    self.tc_timer = None;
                    self.transmit_tcn(out);
                    self.tcn_timer = Some(self.b_hello);
                }
            }
            if Some(i as u16) == self.root_port {
                self.max_age = t.max_age;
                self.hello = t.hello.max(1);
                self.fwd_delay = t.fwd_delay;
                self.set_tc(b.tc(), out);
                self.config_bpdu_generation(out);
                if b.tca() {
                    self.topology_change_acknowledged(out);
                }
            }
        } else if self.designated(i) {
            // an inferior BPDU on our LAN: our own information answers it
            self.transmit_config(i, out);
        }
    }

    fn received_tcn_bpdu(&mut self, i: usize, out: &mut Vec<Out>) {
        if self.designated(i) {
            out.push(Out::Note(Note::StpTopology { port: Some(i as u16), what: TcWhat::TcnReceived }));
            self.topology_change_detection(Some(i), out);
            self.ports[i].tca = true;
            self.transmit_config(i, out);
        }
    }

    // ---------------------------------------------------------------- timers (8.7.3 .. 8.7.8)

    /// one second
    pub(crate) fn tick(&mut self, out: &mut Vec<Out>) {
        let dec = |t: &mut Option<u16>| -> bool {
            match t {
                Some(v) if *v <= 1 => {
                    *t = None;
                    true
                }
                Some(v) => {
                    *v -= 1;
                    false
                }
                None => false,
            }
        };
        let n = self.ports.len();
        let mut hold = Vec::new();
        let mut aged = Vec::new();
        let mut fwd = Vec::new();
        for i in 0..n {
            let p = &mut self.ports[i];
            if dec(&mut p.hold_timer) {
                hold.push(i);
            }
            if let Some(a) = p.age.as_mut() {
                *a = a.saturating_add(1);
                if *a >= p.age_max {
                    p.age = None;
                    aged.push(i);
                }
            }
            if dec(&mut p.fwd_timer) {
                fwd.push(i);
            }
        }
        let tcn = dec(&mut self.tcn_timer);
        let tc = dec(&mut self.tc_timer);
        let hello = dec(&mut self.hello_timer);
        for i in hold {
            if self.ports[i].config_pending {
                self.transmit_config(i, out);
            }
        }
        for i in aged {
            self.message_age_expiry(i, out);
        }
        for i in fwd {
            self.forward_delay_expiry(i, out);
        }
        if tcn && !self.is_root() {
            self.transmit_tcn(out);
            self.tcn_timer = Some(self.b_hello);
        }
        if tc {
            self.tc_detected = false;
            if self.tc {
                out.push(Out::Note(Note::StpTopology { port: None, what: TcWhat::RootFlag(false) }));
            }
            self.set_tc(false, out);
        }
        if hello {
            self.config_bpdu_generation(out);
            self.hello_timer = Some(self.hello);
        }
        self.update_roles(out);
    }

    fn message_age_expiry(&mut self, i: usize, out: &mut Vec<Out>) {
        if !self.up(i) {
            return;
        }
        out.push(Out::Note(Note::StpAged { port: i as u16 }));
        let was_root = self.is_root();
        self.become_designated_port(i);
        self.configuration_update(out);
        self.port_state_selection(out);
        if self.is_root() && !was_root {
            self.become_root_bridge(out);
        }
    }

    fn forward_delay_expiry(&mut self, i: usize, out: &mut Vec<Out>) {
        match self.ports[i].state {
            PortState::Listening => {
                self.set_state(i, PortState::Learning, out);
                self.ports[i].fwd_timer = Some(self.fwd_delay);
            }
            PortState::Learning => {
                self.set_state(i, PortState::Forwarding, out);
                let some = (0..self.ports.len()).any(|j| self.up(j) && self.designated(j));
                if some {
                    self.topology_change_detection(Some(i), out);
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- management (8.8)

    /// a port's link went up or down (enable_port / disable_port), or its cost changed
    pub(crate) fn port(&mut self, i: u16, up: bool, cost: u32, p2p: bool, out: &mut Vec<Out>) {
        let i = i as usize;
        if i >= self.ports.len() {
            return;
        }
        self.ports[i].p2p = p2p;
        if up {
            self.ports[i].member = true;
        }
        let was = self.up(i);
        let cost_changed = self.ports[i].cost != cost;
        self.ports[i].cost = cost;
        match (was, up) {
            (false, true) => {
                self.initialize_port(i);
                self.port_state_selection(out);
            }
            (true, false) => self.disable_port(i, out),
            (true, true) if cost_changed => {
                // 8.8.6: a new path cost takes effect at once
                self.configuration_update(out);
                self.port_state_selection(out);
            }
            _ => {}
        }
        self.update_roles(out);
    }

    fn disable_port(&mut self, i: usize, out: &mut Vec<Out>) {
        let was_root = self.is_root();
        self.become_designated_port(i);
        self.set_state(i, PortState::Disabled, out);
        let p = &mut self.ports[i];
        p.tca = false;
        p.config_pending = false;
        p.age = None;
        p.fwd_timer = None;
        p.hold_timer = None;
        p.oper_edge = false;
        self.configuration_update(out);
        self.port_state_selection(out);
        if self.is_root() && !was_root {
            self.become_root_bridge(out);
        }
    }

    /// 8.8.4: a new bridge priority
    pub(crate) fn set_bridge_id(&mut self, id: BridgeId, out: &mut Vec<Out>) {
        let was_root = self.is_root();
        for i in 0..self.ports.len() {
            if self.up(i) && self.designated(i) {
                self.ports[i].d_bridge = id;
            }
        }
        self.id = id;
        self.configuration_update(out);
        self.port_state_selection(out);
        if self.is_root() && !was_root {
            self.become_root_bridge(out);
        } else if !self.is_root() && was_root {
            self.hello_timer = None;
        }
        self.update_roles(out);
    }

    /// new bridge timers: the root uses them at once, the others keep the root's
    pub(crate) fn set_times(&mut self, hello: u16, fwd_delay: u16, max_age: u16, _out: &mut Vec<Out>) {
        self.b_hello = hello;
        self.b_fwd_delay = fwd_delay;
        self.b_max_age = max_age;
        if self.is_root() {
            self.hello = hello;
            self.fwd_delay = fwd_delay;
            self.max_age = max_age;
        }
    }

    /// 8.8.5: a new port priority
    pub(crate) fn set_port_id(&mut self, i: u16, id: PortId, out: &mut Vec<Out>) {
        let i = i as usize;
        if i >= self.ports.len() {
            return;
        }
        if self.up(i) && self.designated(i) {
            self.ports[i].d_port = id;
        }
        self.ports[i].id = id;
        if self.up(i) && self.id == self.ports[i].d_bridge && id < self.ports[i].d_port {
            self.become_designated_port(i);
        }
        // the port ID breaks ties of the root port too: choose again at once
        if self.up(i) {
            self.configuration_update(out);
            self.port_state_selection(out);
        }
        self.update_roles(out);
    }

    /// "spanning-tree portfast" on or off
    pub(crate) fn set_edge(&mut self, i: u16, on: bool, out: &mut Vec<Out>) {
        let i = i as usize;
        if i >= self.ports.len() {
            return;
        }
        self.ports[i].admin_edge = on;
        if self.ports[i].oper_edge != on && self.up(i) {
            self.ports[i].oper_edge = on;
            out.push(Out::Note(Note::StpEdge { port: i as u16, edge: on }));
            // a designated port that waits goes on at once
            if on && matches!(self.ports[i].state, PortState::Listening | PortState::Learning) && (self.designated(i) || Some(i as u16) == self.root_port) {
                self.ports[i].fwd_timer = None;
                self.set_state(i, PortState::Forwarding, out);
            }
        }
    }

    pub fn view(&self, mode: ModeWord) -> TreeView {
        let ports = (0..self.ports.len())
            .filter(|&i| self.ports[i].member && self.up(i))
            .map(|i| {
                let p = &self.ports[i];
                StpPortView {
                    port: i as u16,
                    id: p.id,
                    role: p.role,
                    state: p.state,
                    cost: p.cost,
                    edge: p.oper_edge,
                    p2p: p.p2p,
                    stp_peer: false,
                    d_root: p.d_root,
                    d_cost: p.d_cost,
                    d_bridge: p.d_bridge,
                    d_port: p.d_port,
                    fwd_timer: p.fwd_timer.unwrap_or(0),
                    info_age: p.age.unwrap_or(0),
                    tx: p.tx,
                    rx: p.rx,
                    to_fwd: p.to_fwd,
                }
            })
            .collect();
        TreeView {
            proto: Proto::Stp,
            mode,
            bridge: self.id,
            root: self.root,
            root_cost: self.root_cost,
            root_port: self.root_port,
            hello: self.hello,
            max_age: self.max_age,
            fwd_delay: self.fwd_delay,
            b_hello: self.b_hello,
            b_max_age: self.b_max_age,
            b_fwd_delay: self.b_fwd_delay,
            tc: self.tc,
            tc_count: 0,
            tc_last: None,
            ports,
        }
    }
}
