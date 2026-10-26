// pdu -- PDUs made by the user, and the recording as a pcapng file
//
//   Pdus                        the user's PDUs, the simple tool's source, the dialog's
//                               fields, the export dialog
//   pick(n, ui)                 a click with the tool P: the source, then the destination ->
//                               an ICMP echo (one probe) to the destination's address;
//                               pick_dev(n, ui, dev) the same for Enter on a focused device
//   open_dialog(n) / dialog(n, ui)   the complex PDU: source, destination (a device, an
//                               address or a name), protocol (ICMP echo, UDP, TCP, DNS query),
//                               ports, size, TTL, count, interval, again and again until
//                               stopped
//   panel(n, ui)                the PDUs panel: the tools, every PDU with its state; send
//                               again, stop, remove
//   export(n, ui) / export_done(n, ui) / file_dialog(n, ui)
//                               Datei › Exportieren › Aufzeichnung (pcapng): every recorded
//                               frame (Engine::pcapng), one interface per cable
//   engine_gone(keep)           a fresh engine: the runs end ("zurückgesetzt"); another
//                               project: the list empties
//
// The PDUs run in the engine: ICMP as a ping job (Engine::ping / ping_host: size, TTL, count,
// interval), a DNS query as a lookup of the device's resolver, UDP and TCP as a
// jfn_sim::PduSender service ("pdu-N").  Their state is read from there every frame.  A device
// as the destination means its address (one in the source's network first); a name is looked
// up by the source.  A new PDU wakes a play that found nothing more to do.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_sim::{IcmpKind, JobId, JobKind, Ns, PduProto, PduSender, PduSpec, PingEv, PingOpts, MS};
use netpkt::Ipv4;
use ui::*;

use super::cat_color;
use crate::app::constants::*;
use crate::app::tr::{t, tc, tf};
use crate::app::Net;
use crate::workspace::canvas::paint::{DEV_H, DEV_W};

/// the protocols of the dialog (German; shown through t())
// i18n
pub(crate) const PROTOS: [&str; 4] = ["ICMP-Echo (ping)", "UDP", "TCP", "DNS-Abfrage"];
/// the file type of the export dialog (German; shown through t())
const EXT: &str = "pcapng=Mitschnitt (pcapng)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Proto {
    Icmp,
    Udp,
    Tcp,
    Dns,
}

impl Proto {
    fn of(i: i32) -> Proto {
        match i {
            1 => Proto::Udp,
            2 => Proto::Tcp,
            3 => Proto::Dns,
            _ => Proto::Icmp,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Proto::Icmp => "ICMP",
            Proto::Udp => "UDP",
            Proto::Tcp => "TCP",
            Proto::Dns => "DNS",
        }
    }
    // its category (colour)
    fn cat(self) -> usize {
        match self {
            Proto::Icmp => 0,
            Proto::Dns => 2,
            Proto::Tcp => 3,
            Proto::Udp => 4,
        }
    }
}

/// what a PDU sends
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Spec {
    pub src: u32,
    /// as written: a device's name, an address or a name for the resolver
    pub dst: String,
    pub proto: Proto,
    pub sport: u16,
    pub dport: u16,
    /// data bytes
    pub size: u32,
    /// 0: the device's
    pub ttl: u8,
    /// 0: until stopped
    pub count: u32,
    pub interval_ms: u32,
}

/// how it runs in the engine
#[derive(Clone, Debug, PartialEq, Eq)]
enum Run {
    Ping(JobId),
    /// the PduSender service of that name
    Svc(String),
    /// a lookup of the source's resolver
    Lookup(u32),
    /// it did not start (German)
    Refused(String),
    /// the engine was replaced
    Gone,
}

pub(crate) struct UserPdu {
    pub id: u32,
    pub spec: Spec,
    run: Run,
    /// the virtual time it last started (K1: the run going on from an earlier moment drops it)
    at: Ns,
}

/// how a PDU stands: its words and whether it runs, worked or failed
struct State {
    text: String,
    running: bool,
    /// BADGE_* of the dot
    badge: u32,
}

pub(crate) struct Pdus {
    pub list: Vec<UserPdu>,
    next: u32,
    /// the services' names count on (a PDU sent again gets a new one)
    runs: u32,
    /// the simple tool's source
    pub src: Option<u32>,
    /// the chosen line of the panel
    sel: i32,
    /// the dialog and its fields
    dlg: Shared<i32>,
    f_src: i32,
    f_dst: String,
    f_proto: i32,
    f_sport: i32,
    f_dport: i32,
    f_size: i32,
    f_ttl: i32,
    f_count: i32,
    f_interval: i32,
    f_periodic: i32,
    f_err: String,
    /// the export's save dialog
    fd: Shared<i32>,
    fd_path: String,
    fd_active: bool,
}

impl Pdus {
    pub(crate) fn new() -> Pdus {
        Pdus {
            list: Vec::new(),
            next: 1,
            runs: 1,
            src: None,
            sel: -1,
            dlg: Shared::new(0),
            f_src: 0,
            f_dst: String::new(),
            f_proto: 0,
            f_sport: 0,
            f_dport: 7,
            f_size: 56,
            f_ttl: 0,
            f_count: 1,
            f_interval: 1,
            f_periodic: 0,
            f_err: String::new(),
            fd: Shared::new(0),
            fd_path: String::new(),
            fd_active: false,
        }
    }

    /// the run goes on from moment `t` ("Von hier fortsetzen"): what started after it never ran
    pub(crate) fn resumed(&mut self, t: Ns) {
        for p in self.list.iter_mut().filter(|p| p.at > t) {
            p.run = Run::Gone;
        }
    }

    /// the engine went: the runs end; another project (keep false): the list empties
    pub(crate) fn engine_gone(&mut self, keep: bool) {
        self.src = None;
        if keep {
            for p in self.list.iter_mut() {
                p.run = Run::Gone;
            }
        } else {
            self.list.clear();
            self.next = 1;
            self.sel = -1;
        }
    }
}

// ---------------------------------------------------------------- devices and addresses

/// the device under a world point (the topmost)
fn device_at(n: &Net, at: [f32; 2]) -> Option<u32> {
    n.doc.proj.devices.iter().rev().find(|d| at[0] >= d.x as f32 && at[0] < d.x as f32 + DEV_W && at[1] >= d.y as f32 && at[1] < d.y as f32 + DEV_H).map(|d| d.id)
}

/// the device has an IPv4 address (besides 127.0.0.1)
fn has_ip(n: &Net, d: u32) -> bool {
    n.doc.sim.engine().ifaces(d).iter().any(|f| f.addrs.iter().any(|a| !a.addr.is_loopback()))
}

/// the address of device `dst` for `src` (never 127.0.0.1): one in a network of the source
/// first, then one of an interface that is up, then any
fn address_of(n: &Net, src: u32, dst: u32) -> Option<Ipv4> {
    let eng = n.doc.sim.engine();
    let mine: Vec<netpkt::Prefix4> = eng.ifaces(src).iter().flat_map(|f| f.addrs.iter().copied()).filter(|a| !a.addr.is_loopback()).collect();
    let theirs = eng.ifaces(dst);
    let all = || theirs.iter().flat_map(|f| f.addrs.iter().filter(|a| !a.addr.is_loopback()).map(move |a| (a.addr, f.up)));
    all().find(|(a, _)| mine.iter().any(|m| m.contains(*a)))
        .or_else(|| all().find(|(_, up)| *up))
        .or_else(|| all().next())
        .map(|(a, _)| a)
}

fn dev_name(n: &Net, d: u32) -> String {
    n.doc.proj.device(d).map_or_else(|| tf("Gerät {}", &[&d.to_string()]), |x| x.name.clone())
}

/// where a PDU goes: an address, or a name for the source's resolver (Err: why not)
fn target(n: &Net, s: &Spec) -> Result<String, String> {
    let dst = s.dst.trim();
    if dst.is_empty() {
        return Err(String::from(t("Das Ziel fehlt.")));
    }
    if s.proto == Proto::Dns {
        return Ok(String::from(dst));
    }
    if let Some(d) = n.doc.proj.devices.iter().find(|d| d.name.eq_ignore_ascii_case(dst)) {
        return match address_of(n, s.src, d.id) {
            Some(a) => Ok(a.to_string()),
            None => Err(tf("{} hat keine IP-Adresse.", &[&d.name])),
        };
    }
    Ok(String::from(dst))
}

// ---------------------------------------------------------------- sending

// the PDU starts in the engine
fn start(n: &mut Net, id: u32, s: &Spec) -> Run {
    if n.doc.proj.device(s.src).is_none() {
        return Run::Refused(String::from(t("Die Quelle gibt es nicht mehr.")));
    }
    if !has_ip(n, s.src) {
        return Run::Refused(tf("{} hat keine IP-Adresse.", &[&dev_name(n, s.src)]));
    }
    let to = match target(n, s) {
        Ok(t) => t,
        Err(e) => return Run::Refused(e),
    };
    let interval = s.interval_ms.max(1) as Ns * MS;
    super::kick(n);
    let src = s.src;
    match s.proto {
        Proto::Icmp => {
            let o = PingOpts { count: s.count, size: s.size.min(65_000) as u16, ttl: (s.ttl > 0).then_some(s.ttl), interval, ..PingOpts::default() };
            match n.doc.sim.with_engine("pdu: ping", move |e| e.ping_host(src, &to, o.clone())) {
                Ok(job) => Run::Ping(job),
                Err(e) => Run::Refused(e.to_string()),
            }
        }
        Proto::Dns => match n.doc.sim.with_engine("pdu: lookup", move |e| e.lookup(src, &to, netpkt::dns::TYPE_A)) {
            Ok(l) => Run::Lookup(l),
            Err(e) => Run::Refused(e.to_string()),
        },
        Proto::Udp | Proto::Tcp => {
            let spec = PduSpec {
                proto: if s.proto == Proto::Udp { PduProto::Udp } else { PduProto::Tcp },
                dst: to,
                sport: s.sport,
                dport: s.dport,
                size: s.size,
                ttl: (s.ttl > 0).then_some(s.ttl),
                count: s.count,
                interval,
            };
            // a name of its own each time it starts
            let name = format!("pdu-{}-{}", id, n.sim.pdu.runs);
            n.sim.pdu.runs += 1;
            let svc = name.clone();
            match n.doc.sim.with_engine("pdu: service", move |e| e.add_service(src, &svc, Box::new(PduSender::new(spec.clone())), true)) {
                Ok(()) => Run::Svc(name),
                Err(e) => Run::Refused(e.to_string()),
            }
        }
    }
}

/// a new PDU: it starts at once and joins the list (the PDUs panel shows)
pub(crate) fn send(n: &mut Net, s: Spec) {
    let id = n.sim.pdu.next;
    n.sim.pdu.next += 1;
    let run = start(n, id, &s);
    let at = n.doc.sim.engine().now();
    n.sim.pdu.list.push(UserPdu { id, spec: s, run, at });
    n.shell.dock.show(P_PDUS);
}

// stop what runs of PDU i
fn stop(n: &mut Net, i: usize) {
    let Some(p) = n.sim.pdu.list.get(i) else { return };
    let src = p.spec.src;
    match p.run.clone() {
        Run::Ping(job) => {
            n.doc.sim.with_engine("pdu: stop", move |e| e.cancel_job(job));
        }
        Run::Svc(name) => {
            let _ = n.doc.sim.with_engine("pdu: stop", move |e| e.set_service_running(src, &name, false));
        }
        _ => {}
    }
}

// remove PDU i (its service goes too)
fn remove(n: &mut Net, i: usize) {
    stop(n, i);
    if let Some(Run::Svc(name)) = n.sim.pdu.list.get(i).map(|p| p.run.clone()) {
        let src = n.sim.pdu.list[i].spec.src;
        let _ = n.doc.sim.with_engine("pdu: remove", move |e| e.remove_service(src, &name));
    }
    n.sim.pdu.list.remove(i);
}

// send PDU i again
fn again(n: &mut Net, i: usize) {
    stop(n, i);
    if let Some(Run::Svc(name)) = n.sim.pdu.list.get(i).map(|p| p.run.clone()) {
        let src = n.sim.pdu.list[i].spec.src;
        let _ = n.doc.sim.with_engine("pdu: remove", move |e| e.remove_service(src, &name));
    }
    let Some(p) = n.sim.pdu.list.get(i) else { return };
    let (id, s) = (p.id, p.spec.clone());
    let run = start(n, id, &s);
    n.sim.pdu.list[i].run = run;
    n.sim.pdu.list[i].at = n.doc.sim.engine().now();
}

/// a click with the tool P: the source, then the destination (an ICMP echo)
pub(crate) fn pick(n: &mut Net, ui: &mut Ui) {
    let (mx, my) = ui.mouse();
    let at = n.canvas.st.vp.to_world(mx as f32 + 0.5, my as f32 + 0.5);
    let Some(d) = device_at(n, at) else { return };
    pick_dev(n, ui, d);
}

/// the tool chose device d (a click, or Enter on the focused device): the source, then the
/// destination (the source again: choose anew)
pub(crate) fn pick_dev(n: &mut Net, ui: &mut Ui, d: u32) {
    match n.sim.pdu.src {
        None => {
            if has_ip(n, d) {
                n.sim.pdu.src = Some(d);
                n.file.msg = None;
            } else {
                let name = dev_name(n, d);
                n.file.msg = Some((IB_WARNING, tf("{} kann keine PDU senden", &[&name]), String::from(t("Das Gerät hat keine IP-Adresse; eine Quelle braucht eine."))));
            }
        }
        // the source again: choose anew
        Some(s) if s == d => n.sim.pdu.src = None,
        Some(s) => {
            n.sim.pdu.src = None;
            let dst = dev_name(n, d);
            if address_of(n, s, d).is_none() {
                n.file.msg = Some((IB_WARNING, tf("{} hat keine IP-Adresse", &[&dst]), String::from(t("Ein Ziel braucht eine IPv4-Adresse für das ICMP-Echo."))));
                return;
            }
            n.file.msg = None;
            send(n, Spec { src: s, dst, proto: Proto::Icmp, sport: 0, dport: 0, size: 56, ttl: 0, count: 1, interval_ms: 1000 });
        }
    }
    ui.repass();
}

// ---------------------------------------------------------------- state

fn icmp_text(k: IcmpKind) -> String {
    match k {
        IcmpKind::NetUnreachable => String::from(t("Netz nicht erreichbar")),
        IcmpKind::HostUnreachable => String::from(t("Host nicht erreichbar")),
        IcmpKind::ProtoUnreachable => String::from(t("Protokoll nicht erreichbar")),
        IcmpKind::PortUnreachable => String::from(t("Port nicht erreichbar")),
        IcmpKind::FragNeeded(m) => tf("Fragmentierung nötig (MTU {})", &[&m.to_string()]),
        IcmpKind::Prohibited => String::from(t("verboten")),
        IcmpKind::TtlExceeded => String::from(t("TTL abgelaufen")),
        IcmpKind::ReasmExceeded => String::from(t("Zusammensetzen abgelaufen")),
        IcmpKind::NoRoute => String::from(t("keine Route")),
        IcmpKind::Other(t, c) => format!("ICMP {}/{}", t, c),
    }
}

fn of_count(sent: u32, count: u32) -> String {
    if count == 0 { format!("{}", sent) } else { tf("{} von {}", &[&sent.to_string(), &count.to_string()]) }
}

// how PDU p stands, from the engine
fn state(n: &Net, p: &UserPdu) -> State {
    let eng = n.doc.sim.engine();
    let src = p.spec.src;
    let s = |text: String, running: bool, badge: u32| State { text, running, badge };
    match &p.run {
        Run::Refused(why) => s(why.clone(), false, BADGE_CRIT),
        Run::Gone => s(String::from(t("zurückgesetzt")), false, BADGE_NEUTRAL),
        Run::Ping(job) => {
            let Some(j) = eng.job(*job) else { return s(String::from(t("beendet")), false, BADGE_NEUTRAL) };
            let JobKind::Ping(pj) = &j.kind else { return s(String::from("–"), false, BADGE_NEUTRAL) };
            if pj.sent == 0 {
                if !j.done {
                    return s(String::from(t("fragt nach dem Namen …")), true, BADGE_INFO);
                }
                let why = pj.lookup.and_then(|l| eng.lookup_state(src, l)).and_then(|l| l.result.clone()).and_then(|r| r.err()).map_or(t("Name unbekannt"), |e| t(e.text()));
                return s(format!("„{}“: {}", p.spec.dst.trim(), why), false, BADGE_CRIT);
            }
            let last_err = pj.events.iter().rev().find_map(|e| match e {
                PingEv::Error { kind, .. } => Some(icmp_text(*kind)),
                PingEv::Timeout { .. } => Some(String::from(t("keine Antwort"))),
                _ => None,
            });
            let sent = of_count(pj.sent, pj.opts.count);
            if !j.done {
                return s(tf("läuft: {} gesendet, {} Antworten", &[&sent, &pj.received.to_string()]), true, BADGE_INFO);
            }
            if pj.opts.count == 0 {
                // again and again: only a stop ends it
                return s(tf("gestoppt: {} gesendet, {} Antworten", &[&pj.sent.to_string(), &pj.received.to_string()]), false, if pj.received > 0 { BADGE_OK } else { BADGE_WARN });
            }
            match last_err {
                None => s(tf("fertig: {} gesendet, alle beantwortet", &[&pj.sent.to_string()]), false, BADGE_OK),
                Some(e) if pj.received > 0 => s(tf("fertig: {} von {} beantwortet; {}", &[&pj.received.to_string(), &pj.sent.to_string(), &e]), false, BADGE_WARN),
                Some(e) => s(tf("gescheitert: {}", &[&e]), false, BADGE_CRIT),
            }
        }
        Run::Svc(name) => {
            let Some(x) = eng.service::<PduSender>(src, name) else { return s(String::from(t("beendet")), false, BADGE_NEUTRAL) };
            let sent = of_count(x.sent, x.spec.count);
            let what = t(if x.spec.proto == PduProto::Tcp { "Verbindungen" } else { "gesendet" });
            if x.dst.is_none() {
                return match &x.error_msg {
                    Some(e) => s(crate::app::tr::msg(e), false, BADGE_CRIT),
                    None => s(String::from(t("fragt nach dem Namen …")), true, BADGE_INFO),
                };
            }
            if x.sent == 0 && x.error_msg.is_some() {
                return s(x.error_msg.as_ref().map(crate::app::tr::msg).unwrap_or_default(), false, BADGE_CRIT);
            }
            let tail = match (&x.error_msg, x.spec.proto) {
                (Some(e), _) if x.failed > 0 => tf(", {} gescheitert ({})", &[&x.failed.to_string(), &crate::app::tr::msg(e)]),
                _ => String::new(),
            };
            let answers = if x.spec.proto == PduProto::Tcp { tf("{} beendet", &[&x.replies.to_string()]) } else { tf("{} Antworten", &[&x.replies.to_string()]) };
            if !x.done() {
                return s(tf("läuft: {} {}, {}{}", &[&sent, what, &answers, &tail]), true, BADGE_INFO);
            }
            let stopped = t(if x.stopped { "gestoppt" } else { "fertig" });
            let badge = if x.failed > 0 { if x.replies > 0 { BADGE_WARN } else { BADGE_CRIT } } else { BADGE_OK };
            s(format!("{}: {} {}, {}{}", stopped, sent, what, answers, tail), false, badge)
        }
        Run::Lookup(id) => match eng.lookup_state(src, *id) {
            None => s(String::from(t("beendet")), false, BADGE_NEUTRAL),
            Some(l) => match &l.result {
                None => s(String::from(t("fragt …")), true, BADGE_INFO),
                Some(Ok(a)) => {
                    let addrs: Vec<String> = a.addrs.iter().map(|x| x.to_string()).collect();
                    let text = if addrs.is_empty() { String::from(t("Antwort ohne Adresse")) } else { addrs.join(", ") };
                    s(tf("fertig: {}", &[&text]), false, BADGE_OK)
                }
                Some(Err(e)) => s(tf("gescheitert: {}", &[t(e.text())]), false, BADGE_CRIT),
            },
        },
    }
}

// ---------------------------------------------------------------- the panel

/// the PDUs panel: the tools, then a line pair per PDU (what, from where to where; its state)
/// with send again, stop and remove
pub(crate) fn panel(n: &mut Net, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER | pad(6), 6);
    let mut tool = (n.canvas.tool == crate::workspace::canvas::tools::Tool::Pdu) as i32;
    ui.next_flags(ST_COMPACT);
    ui.next_icon(IC_MAIL);
    if ui.toggle_button(ID_PDU_SIMPLE, t("Einfache PDU"), &mut tool) & EV_ACT != 0 {
        ui.cmd_run(CMD_TOOL_PDU, 0);
    }
    ui.tooltip(t("Werkzeug P: die Quelle anklicken, dann das Ziel (ein ICMP-Echo)"));
    ui.next_flags(ST_COMPACT);
    ui.next_icon(IC_ADD);
    if ui.button(ID_PDU_NEW, t("Komplexe PDU …")) & EV_ACT != 0 {
        ui.cmd_run(CMD_PDU_COMPLEX, 0);
    }
    ui.box_end();
    if n.sim.pdu.list.is_empty() {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(12), 0);
        ui.empty_state(IC_MAIL, t("Noch keine PDUs"), t("Mit dem Werkzeug P erst die Quelle, dann das Ziel anklicken: ein ICMP-Echo. „Komplexe PDU …“ wählt Protokoll, Ports, Größe, TTL und Anzahl."));
        ui.box_end();
        ui.box_end();
        return;
    }
    let states: Vec<State> = n.sim.pdu.list.iter().map(|p| state(n, p)).collect();
    let dark = ui.theme_dark();
    let mut sel = n.sim.pdu.sel;
    let count = n.sim.pdu.list.len() as i32;
    ui.next_size(-1, -1);
    let (first, end) = ui.list_begin(ID_PDU_LIST, &mut sel, count, 46);
    let mut act: Option<(usize, u32)> = None;
    for i in first..end {
        let p = &n.sim.pdu.list[i as usize];
        let s = &states[i as usize];
        ui.list_item_begin(i);
        let d = ui.dp(8);
        let q = ui.place(d, d);
        ui.cmd_rrect(q[0], q[1], d, d, ui.dp(2), 0, cat_color(p.spec.proto.cat(), dark), 0, 0);
        let to = match p.spec.proto {
            Proto::Udp | Proto::Tcp => format!("{}:{}", p.spec.dst.trim(), p.spec.dport),
            _ => String::from(p.spec.dst.trim()),
        };
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V, 2);
        ui.next_size(-1, 0);
        ui.text(TS_BODY_STRONG | TX_ELLIPSIS, &format!("#{} {} · {} → {}", p.id, p.spec.proto.name(), dev_name(n, p.spec.src), to));
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.status_dot(s.badge);
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &s.text);
        ui.tooltip(&s.text);
        ui.box_end();
        ui.box_end();
        let base = ID_PDU_ACT0 + 4 * (i as u32 & 0xFF);
        ui.next_flags(ST_ICON_ONLY | ST_SUBTLE | ST_COMPACT);
        if ui.icon_button(base, IC_REFRESH, t("Erneut senden")) & EV_ACT != 0 {
            act = Some((i as usize, 0));
        }
        ui.next_flags(ST_ICON_ONLY | ST_SUBTLE | ST_COMPACT | if s.running { 0 } else { ST_DISABLED });
        if ui.icon_button(base + 1, IC_STOP, t("Stoppen")) & EV_ACT != 0 {
            act = Some((i as usize, 1));
        }
        ui.next_flags(ST_ICON_ONLY | ST_SUBTLE | ST_COMPACT);
        if ui.icon_button(base + 2, IC_DELETE, t("Entfernen")) & EV_ACT != 0 {
            act = Some((i as usize, 2));
        }
        ui.list_item_end();
    }
    let (_, s) = ui.list_end(&mut sel);
    n.sim.pdu.sel = s;
    ui.box_end();
    match act {
        Some((i, 0)) => again(n, i),
        Some((i, 1)) => stop(n, i),
        Some((i, _)) => remove(n, i),
        None => return,
    }
    ui.repass();
}

// ---------------------------------------------------------------- the dialog

// the devices a PDU can leave from (with an address), by name
fn sources(n: &Net) -> Vec<(u32, String)> {
    let mut v: Vec<(u32, String)> = n.doc.proj.devices.iter().filter(|d| has_ip(n, d.id)).map(|d| (d.id, d.name.clone())).collect();
    v.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    v
}

/// "Komplexe PDU …": the dialog (the selected device is the source)
pub(crate) fn open_dialog(n: &mut Net) {
    let srcs = sources(n);
    let sel = crate::workspace::canvas::cables::sel_devices(n);
    if let Some(i) = sel.first().and_then(|d| srcs.iter().position(|s| s.0 == *d)) {
        n.sim.pdu.f_src = i as i32;
    }
    n.sim.pdu.f_src = n.sim.pdu.f_src.clamp(0, (srcs.len() as i32 - 1).max(0));
    n.sim.pdu.f_err.clear();
    n.sim.pdu.dlg.set(1);
}

// a labelled field of the dialog: the label column, then the control
fn field(ui: &mut Ui, label: &'static str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(110, 0);
    ui.text(TS_BODY | TX_SECONDARY, if label.is_empty() { label } else { tc(label) });
}

/// the dialog of a complex PDU (once per frame)
pub(crate) fn dialog(n: &mut Net, ui: &mut Ui) {
    let open = n.sim.pdu.dlg.clone();
    let srcs = sources(n);
    if ui.dialog_begin(ID_PDU_DLG, t("Komplexe PDU"), &open) {
        let f = &mut n.sim.pdu;
        ui.box_begin(0, BOX_V, 8);
        ui.next_size(420, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Die PDU läuft im Simulator wie ein Befehl auf der Quelle: ARP, Routen und Antworten wie echt."));
        let names: Vec<&str> = srcs.iter().map(|s| s.1.as_str()).collect();
        field(ui, "Quelle");
        ui.next_size(240, 0);
        if names.is_empty() {
            ui.text(TS_BODY | TX_SECONDARY, t("kein Gerät mit IP-Adresse"));
        } else {
            ui.combobox(ID_PD_SRC, &mut f.f_src, &names);
        }
        ui.box_end();
        field(ui, "Ziel");
        ui.next_size(240, 0);
        if ui.textbox(ID_PD_DST, Some(t("Gerät, IP-Adresse oder Name")), &mut f.f_dst, 128) & EV_ACT != 0 {
            f.f_err.clear();
        }
        ui.box_end();
        field(ui, "Protokoll|PDU");
        ui.next_size(240, 0);
        let before = f.f_proto;
        let protos: Vec<&str> = PROTOS.iter().map(|&p| t(p)).collect();
        if ui.combobox(ID_PD_PROTO, &mut f.f_proto, &protos) & EV_ACT != 0 && f.f_proto != before {
            // sizes and ports that fit the protocol
            match Proto::of(f.f_proto) {
                Proto::Icmp => f.f_size = 56,
                Proto::Udp => {
                    f.f_size = 32;
                    f.f_dport = 7;
                }
                Proto::Tcp => {
                    f.f_size = 1000;
                    f.f_dport = 7;
                }
                Proto::Dns => {}
            }
        }
        ui.box_end();
        let proto = Proto::of(f.f_proto);
        let ports = matches!(proto, Proto::Udp | Proto::Tcp);
        let dns = proto == Proto::Dns;
        let num = |ui: &mut Ui, id: u32, v: &mut i32, min: i32, max: i32, on: bool| {
            ui.next_size(120, 0);
            if !on {
                ui.next_flags(ST_DISABLED);
            }
            ui.number_box(id, v, &Range::new(min, max, 1, 0));
        };
        field(ui, "Quellport");
        num(ui, ID_PD_SPORT, &mut f.f_sport, 0, 65535, ports);
        ui.text(TS_CAPTION | TX_TERTIARY, t("0: frei gewählt"));
        ui.box_end();
        field(ui, "Zielport");
        num(ui, ID_PD_DPORT, &mut f.f_dport, 1, 65535, ports);
        ui.box_end();
        field(ui, "Größe (Bytes)");
        num(ui, ID_PD_SIZE, &mut f.f_size, 0, 60000, !dns);
        ui.box_end();
        field(ui, "TTL");
        num(ui, ID_PD_TTL, &mut f.f_ttl, 0, 255, matches!(proto, Proto::Icmp | Proto::Udp));
        ui.text(TS_CAPTION | TX_TERTIARY, t("0: die des Geräts"));
        ui.box_end();
        field(ui, "Anzahl");
        num(ui, ID_PD_COUNT, &mut f.f_count, 1, 10000, !dns && f.f_periodic == 0);
        ui.box_end();
        field(ui, "Abstand (s)");
        num(ui, ID_PD_INTERVAL, &mut f.f_interval, 1, 3600, !dns);
        ui.box_end();
        field(ui, "");
        if dns {
            ui.next_flags(ST_DISABLED);
        }
        ui.checkbox(ID_PD_PERIODIC, t("Periodisch, bis „Stoppen“"), &mut f.f_periodic);
        ui.box_end();
        if !f.f_err.is_empty() {
            let e = f.f_err.clone();
            crate::workspace::error_text(ui, &e, 420);
        }
        ui.box_end();
        if ui.dialog_buttons(Some(t("Senden")), None, Some(t("Abbrechen"))) == DLG_PRIMARY {
            submit(n, &srcs, &open);
        }
    }
    ui.dialog_end();
}

// "Senden": a checked PDU starts (else the dialog stays open with the reason)
fn submit(n: &mut Net, srcs: &[(u32, String)], open: &Shared<i32>) {
    let f = &n.sim.pdu;
    let Some(&(src, _)) = srcs.get(f.f_src.max(0) as usize) else {
        n.sim.pdu.f_err = String::from(t("Kein Gerät mit IP-Adresse kann senden."));
        open.set(1);
        return;
    };
    let proto = Proto::of(f.f_proto);
    let periodic = f.f_periodic != 0 && proto != Proto::Dns;
    let s = Spec {
        src,
        dst: String::from(f.f_dst.trim()),
        proto,
        sport: f.f_sport.clamp(0, 65535) as u16,
        dport: f.f_dport.clamp(1, 65535) as u16,
        size: f.f_size.max(0) as u32,
        ttl: f.f_ttl.clamp(0, 255) as u8,
        count: if periodic { 0 } else if proto == Proto::Dns { 1 } else { f.f_count.max(1) as u32 },
        interval_ms: f.f_interval.max(1) as u32 * 1000,
    };
    if let Err(e) = target(n, &s) {
        n.sim.pdu.f_err = e;
        open.set(1);
        return;
    }
    n.sim.pdu.f_err.clear();
    send(n, s);
}

// ---------------------------------------------------------------- export

/// Datei › Exportieren › Aufzeichnung (pcapng): the save dialog
pub(crate) fn export(n: &mut Net, ui: &mut Ui) {
    let name = n.doc.file_name();
    let stem = name.strip_suffix(".jfnet").unwrap_or(&name);
    let dir = match &n.doc.path {
        Some(p) if !ui.headless() => String::from(&p[..p.rfind(['/', '\\']).unwrap_or(0)]),
        _ => plat::cwd(),
    };
    n.sim.pdu.fd_path = format!("{}/{}.pcapng", dir.replace('\\', "/").trim_end_matches('/'), stem);
    n.sim.pdu.fd_active = true;
    n.sim.pdu.fd.set(1);
}

/// the dialog chose its file: every recorded frame into it
pub(crate) fn export_done(n: &mut Net, ui: &mut Ui) {
    let path = n.sim.pdu.fd_path.clone();
    let pcap = n.doc.sim.engine().pcapng(&jfn_sim::PcapOpts::default());
    let file = crate::doc::file::base_name(&path).to_string();
    n.file.msg = Some(if plat::write_file(&path, &pcap.bytes) {
        (IB_SUCCESS, tf("Aufzeichnung als „{}“ exportiert", &[&file]), tf("{} Rahmen im pcapng-Format, zum Beispiel für Wireshark.", &[&pcap.packets.to_string()]))
    } else {
        (IB_ERROR, tf("„{}“ wurde nicht geschrieben", &[&file]), String::from(t("Die Datei ließ sich nicht schreiben.")))
    });
    ui.repass();
}

/// the export's save dialog (once per frame)
pub(crate) fn file_dialog(n: &mut Net, ui: &mut Ui) {
    let open = n.sim.pdu.fd.clone();
    let ev = ui.filedlg(ID_PCAP_DLG, FD_SAVE, &open, &mut n.sim.pdu.fd_path, 1040, &[t(EXT)]);
    if ev & EV_ACT != 0 {
        n.sim.pdu.fd_active = false;
        ui.cmd_run(CMD_PCAP_DONE, 0);
    } else if n.sim.pdu.fd_active && open.get() == 0 {
        n.sim.pdu.fd_active = false;
    }
}
