// msgtest -- checks of jfn_model::msg (part of --selftest): templates with "{}", "{1}" and
// braces, nested messages, spans of time and lists; the German texts of the model and the
// simulator as before (Note::describe, model summaries, cable reasons, refusals, problems,
// event rows, status lines) and their English through src/locales/engine.tsv

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::cable::{self, CableKind, Why};
use jfn_model::catalog::{self, Category, Media};
use jfn_model::msg::{dur, list, phrase, Msg};
use jfn_model::project::{PortRef, ProjErr, Project, ServiceDef};
use jfn_sim::http::Fail;
use jfn_sim::{ActErr, Action, DhcpReq, Engine, LookupErr, Note, ProblemKind, RelayDrop, RstWhy, Severity, TcpEnd, TcpState, MS, SEC, US};
use netpkt::{Ipv4, Mac, Prefix4};

use crate::selftest::T;

const BUDGET: usize = 2_000_000;

fn ip(s: &str) -> Ipv4 {
    Ipv4::parse(s).unwrap_or(Ipv4::UNSPECIFIED)
}

fn pfx(s: &str) -> Prefix4 {
    Prefix4::parse(s).unwrap_or(Prefix4 { addr: Ipv4::UNSPECIFIED, len: 0 })
}

fn s(x: &str) -> String {
    String::from(x)
}

// the rows of engine.tsv: German template -> English
fn rows() -> Vec<(&'static str, &'static str)> {
    include_str!("locales/engine.tsv").lines().skip(1).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect()
}

/// the English of engine.tsv as the app's lookup would give it (the source when missing)
fn english() -> impl Fn(&'static str) -> &'static str {
    let rows = rows();
    move |k: &'static str| rows.iter().find(|r| r.0 == k).map_or(k, |r| r.1)
}

pub fn run(t: &mut T) {
    templates(t);
    nested(t);
    times(t);
    notes(t);
    model(t);
    catalogue(t);
    engine(t);
}

// placeholders, braces, missing and surplus arguments
fn templates(t: &mut T) {
    let m = jfn_model::msg!("Port {} ist schon belegt.", 7);
    t.eq("text", m.text(), s("Port 7 ist schon belegt."));
    t.eq("display", m.to_string(), m.text());
    t.eq("parts", (m.tpl, m.args.clone()), ("Port {} ist schon belegt.", alloc::vec![s("7")]));
    let swap = |k: &'static str| if k == "{} an {}" { "{1} from {0}" } else { k };
    let m = jfn_model::msg!("{} an {}", "A", "B");
    t.eq("numbered", m.text_with(&swap), s("B from A"));
    t.eq("numbered german", m.text(), s("A an B"));
    t.eq("{} counts only {}", Msg::new("{} {0} {} {1}", alloc::vec![s("a"), s("b")]).text(), s("a a b b"));
    t.eq("braces", Msg::new("{{{}}} }} {{x}}", alloc::vec![s("v")]).text(), s("{v} } {x}"));
    t.eq("missing argument", Msg::new("<{}|{}|{5}>", alloc::vec![s("a")]).text(), s("<a||>"));
    t.eq("no placeholder of ours", Msg::new("{:03} {name} {", Vec::new()).text(), s("{:03} {name} {"));
    t.eq("plain", Msg::plain("Gi0/1 {}").text_with(&english()), s("Gi0/1 {}"));
    t.eq("new", Msg::new("x {}", alloc::vec![s("1")]), Msg { tpl: "x {}", args: alloc::vec![s("1")] });
    t.check("empty", Msg::default().is_empty() && Msg::default().text().is_empty());
}

// a Msg argument is translated with the outer one
fn nested(t: &mut T) {
    let m = jfn_model::msg!("RST gesendet: {}.", phrase("keine solche Verbindung"));
    t.eq("nested german", m.text(), s("RST gesendet: keine solche Verbindung."));
    let tr = |k: &'static str| match k {
        "RST gesendet: {}." => "RST sent: {}.",
        "keine solche Verbindung" => "no such connection",
        "[{}]" => "<{}>",
        x => x,
    };
    t.eq("nested translated", m.text_with(&tr), s("RST sent: no such connection."));
    let deep = jfn_model::msg!("[{}]", &m);
    t.eq("nested twice", (deep.text(), deep.text_with(&tr)), (s("[RST gesendet: keine solche Verbindung.]"), s("<RST sent: no such connection.>")));
    let odd = jfn_model::msg!("{} / {}", "a\u{1}b", jfn_model::msg!("<{}>", "{}{{x\u{2}"));
    t.eq("odd arguments", odd.text(), s("a\u{1}b / <{}{{x\u{2}>"));
    t.eq("identity", m.text_with(&|k| k), m.text());
    t.eq("list", list(alloc::vec![phrase("a"), phrase("b"), Msg::plain("c")]).text(), s("a, b, c"));
    t.eq("list of one", list(alloc::vec![phrase("a")]).text(), s("a"));
    t.check("empty list", list(Vec::new()).is_empty());
}

// msg::dur writes what fmt_ns writes, English takes the decimal point
fn times(t: &mut T) {
    for ns in [0i64, 12, 999, 1000, 450_000, 999_999, 1_000_000, 3_200_000, 999_999_999, 1_000_000_000, 1_250_000_000, 61_000_000_000, -12, -3_200_000, -1_250_000_000] {
        t.eq(&format!("dur {}", ns), dur(ns).text(), jfn_sim::fmt_ns(ns));
    }
    let en = english();
    t.eq("dur english", (dur(1_250_000_000).text_with(&en), dur(-3_200_000).text_with(&en), dur(450_000).text_with(&en)), (s("1.250 s"), s("-3.200 ms"), s("450 \u{b5}s")));
}

// Note::describe is the German text of Note::message, word for word as before
fn notes(t: &mut T) {
    let port = |p: u16| format!("Gi1/0/{}", p + 1);
    let iface = |i: u16| format!("eth{}", i);
    let mac = Mac([2, 0, 0, 0, 0, 1]);
    let cases: Vec<(Note, &str)> = alloc::vec![
        (Note::LinkUp { port: 0 }, "Leitung an Gi1/0/1 ist aktiv."),
        (Note::Learned { mac, port: 1, vlan: 10 }, "MAC 02:00:00:00:00:01 an Gi1/0/2 gelernt (VLAN 10)."),
        (Note::ArpLearned { ip: ip("10.0.0.2"), mac }, "ARP-Eintrag 10.0.0.2 \u{2192} 02:00:00:00:00:01."),
        (Note::IpFragNeeded { mtu: 1400 }, "Zu groß für die MTU 1400 und \u{201e}nicht fragmentieren\u{201c} gesetzt."),
        (Note::DhcpDiscover { iface: 0, xid: 0xabc, secs: 3 }, "DHCPDISCOVER über eth0 (xid 0x00000abc, seit 3 s)."),
        (Note::DhcpRequest { iface: 1, ip: ip("10.0.0.5"), kind: DhcpReq::Rebind }, "DHCPREQUEST über eth1: verlängert 10.0.0.5 bei jedem Server (Rebind)."),
        (Note::DhcpBound { iface: 0, addr: pfx("10.0.0.5/24"), router: Some(ip("10.0.0.1")), renew: 300 }, "eth0 hat 10.0.0.5/24 (Router 10.0.0.1); Verlängerung in 300 s."),
        (Note::DhcpRelayDrop { why: RelayDrop::NotMine }, "DHCP verworfen: giaddr gehört nicht zu diesem Gerät."),
        (Note::DnsAnswer { name: s("www.lab"), qtype: 1, n: 1 }, "DNS: www.lab A beantwortet (1 Eintrag)."),
        (Note::DnsAnswer { name: s("www.lab"), qtype: 1, n: 2 }, "DNS: www.lab A beantwortet (2 Einträge)."),
        (Note::ResolveAnswer { name: s("www.lab"), addr: Some(ip("10.0.0.2")), cached: true }, "www.lab ist 10.0.0.2 (aus dem Cache)."),
        (Note::ResolveAnswer { name: s("www.lab"), addr: None, cached: false }, "www.lab aufgelöst."),
        (Note::ResolveFailed { name: s("x.lab"), why: LookupErr::NotFound }, "x.lab nicht aufgelöst: unbekannter Name."),
        (Note::NtpSample { server: ip("10.0.0.53"), offset: -1_250_000, delay: 450_000 }, "NTP-Antwort von 10.0.0.53: Abweichung -1,250 ms, Laufzeit 450 \u{b5}s."),
        (Note::TcpStateChange { port: 7, peer: Ipv4::UNSPECIFIED, pport: 0, from: TcpState::Closed, to: TcpState::Listen }, "TCP Port 7: CLOSED \u{2192} LISTEN."),
        (Note::TcpRstSent { why: RstWhy::NoConn }, "RST gesendet: keine solche Verbindung."),
        (Note::TcpAcked { bytes: 1460, cwnd: 16060, rtt: Some(26 * US) }, "1460 Bytes bestätigt, RTT 26 \u{b5}s, cwnd 16060 Bytes."),
        (Note::TcpRto { seq: 7, rto: 400 * MS, n: 1 }, "Zeitüberschreitung: Neuübertragung ab Seq 7 (Versuch 1), nächstes RTO 400,000 ms."),
        (Note::TcpGiveUp { why: TcpEnd::Timeout }, "Verbindung aufgegeben: keine Antwort mehr (Zeitüberschreitung)."),
        (Note::TcpChallenge { syn: true }, "SYN im Fenster, aber nicht an der erwarteten Stelle: Challenge-ACK (RFC 5961)."),
        (Note::HttpIdleClose { header: false }, "Keep-Alive-Verbindung zu lange ungenutzt: geschlossen."),
    ];
    for (n, want) in &cases {
        t.eq("describe", n.describe(&port, &iface), s(want));
        t.eq("message is describe", n.message(&port, &iface).text(), n.describe(&port, &iface));
    }
    let en = english();
    let e = |n: Note| n.message(&port, &iface).text_with(&en);
    t.eq("note english", e(Note::TcpRto { seq: 7, rto: 400 * MS, n: 1 }), s("Timeout: retransmission from seq 7 (attempt 1), next RTO 400.000 ms."));
    t.eq("note english nested", e(Note::TcpGiveUp { why: TcpEnd::Timeout }), s("Connection given up: no more answers (timeout)."));
    t.eq("note english plural", e(Note::DnsAnswer { name: s("www.lab"), qtype: 1, n: 2 }), s("DNS: www.lab A answered (2 records)."));
    t.eq("note english quotes", e(Note::IpFragNeeded { mtu: 1400 }), s("Too big for the MTU 1400 and \u{201c}don't fragment\u{201d} is set."));
}

// the model's texts: German as before, English through the catalogue
fn model(t: &mut T) {
    let en = english();
    let m = |id: &str| catalog::model(id).map(|m| (m.summary(), m.summary_msg().text_with(&en))).unwrap_or_default();
    t.eq("summary switch", m("jf-s24p"), (s("24\u{d7} 1G PoE+, 4\u{d7} 10G SFP+, Konsole"), s("24\u{d7} 1G PoE+, 4\u{d7} 10G SFP+, Console")));
    t.eq("summary laptop", m("laptop"), (s("1\u{d7} Funk 2.4G, 1\u{d7} 2.5G, USB"), s("1\u{d7} radio 2.4G, 1\u{d7} 2.5G, USB")));
    t.eq("summary phone", m("ip-phone"), (s("1\u{d7} 1G (PoE-gespeist), 1\u{d7} 1G"), s("1\u{d7} 1G (PoE-powered), 1\u{d7} 1G")));
    t.eq("summary router", m("jf-r4").0, s("4\u{d7} 1G, 2\u{d7} 10G SFP+, 2\u{d7} Seriell, Konsole"));
    t.eq("speeds", [catalog::speed_str(2048), catalog::speed_str(2500), catalog::speed_str(115), catalog::speed_str(2_500_000), catalog::speed_str(100_000)], [s("2 Mbit/s"), s("2,5 Mbit/s"), s("115 kbit/s"), s("2.5G"), s("100M")]);
    t.eq("speed english", catalog::speed_msg(2500).text_with(&en), s("2.5 Mbit/s"));
    for k in [0u32, 115, 999, 1000, 2048, 2500, 99_900, 100_000, 999_999, 1_000_000, 2_400_000, 2_500_000, 100_000_000] {
        t.eq(&format!("speed_msg {}", k), catalog::speed_msg(k).text(), catalog::speed_str(k));
    }
    t.eq("cable tooltip", (cable::describe(CableKind::CopperStraight, 1_000_000), cable::describe_msg(CableKind::CopperCross, 1_000_000).text_with(&en)), (s("Kupfer, gerade \u{b7} 1G"), s("Copper, crossover \u{b7} 1G")));
    let why = [Why::MediaMismatch(Media::Serial, Media::Rj45), Why::CableDoesNotFit(CableKind::FiberMm, Media::Rj45), Why::PortInUse(s("SW1 Gi1/0/1")), Why::NoAccessPoint, Why::SameDevice];
    t.eq("why german", why.iter().map(|w| w.to_string()).collect::<Vec<_>>()[..2].to_vec(), alloc::vec![s("Seriell und RJ45 passen nicht zusammen."), s("Glasfaser, Multimode passt nicht in einen RJ45-Port.")]);
    t.check("why display is msg", why.iter().all(|w| w.to_string() == w.msg().text()));
    t.eq("why english", (why[0].msg().text_with(&en), why[1].msg().text_with(&en)), (s("Serial and RJ45 do not fit together."), s("Fiber, multimode does not fit the RJ45 port.")));
    let pe = ProjErr::Cable(Why::NoAccessPoint);
    t.eq("project error", (pe.to_string(), pe.msg().text_with(&en)), (s("Funk verbindet ein WLAN-Gerät mit einem Access Point."), s("A wireless link joins a WLAN client to an access point.")));
    t.eq("bad name", ProjErr::BadName(s("1x")).msg().text_with(&en), s("\u{201c}1x\u{201d} is not a valid device name: letters, digits and hyphens, a letter first, at most 63 characters."));
    let le = Project::from_text("{}").err();
    t.eq("load error", le.map(|e| (e.text.clone(), e.msg.text(), e.msg.text_with(&en))), Some((s("Das ist keine JFNetworker-Datei."), s("Das ist keine JFNetworker-Datei."), s("This is not a JFNetworker file."))));
    let mut p = Project::new("msg", 1);
    let a = p.add_device("pc", 0, 0).unwrap_or(0);
    if let Some(d) = p.device_mut(a) {
        d.name = s("1bad");
    }
    p.next_id = 1;
    let issues = p.check();
    t.check("issues", !issues.is_empty() && issues.iter().all(|i| i.text == i.msg.text()));
    t.check("issue english", issues.iter().any(|i| i.msg.text_with(&en) == "The next free number is too small."));
}

// every name of the catalogue has a row; German and English differ where German has words
fn catalogue(t: &mut T) {
    let rows = rows();
    let has = |k: &str| rows.iter().any(|r| r.0 == k && !r.1.is_empty());
    for m in catalog::catalog() {
        t.check(&format!("row for {}", m.name), has(m.name) && has(m.title));
    }
    t.check("categories", Category::ALL.iter().all(|c| has(c.title())));
    t.check("cables", CableKind::ALL.iter().all(|k| has(k.title())));
    let media = [Media::Rj45, Media::Sfp, Media::SfpPlus, Media::Sfp28, Media::Qsfp28, Media::Serial, Media::Console, Media::Usb, Media::RadioAp, Media::RadioClient, Media::Wan];
    t.check("media", media.iter().all(|m| has(m.name())));
    t.check("severities", has(Severity::Error.title()) && has(Severity::Warning.title()));
    let ends = [TcpEnd::Closed, TcpEnd::Refused, TcpEnd::Reset, TcpEnd::Timeout, TcpEnd::Unreachable, TcpEnd::Aborted];
    t.check("tcp ends", ends.iter().all(|e| has(e.text())));
    let lookups = [LookupErr::NotFound, LookupErr::NoData, LookupErr::Timeout, LookupErr::ServFail, LookupErr::Refused, LookupErr::NoServer, LookupErr::BadName];
    t.check("lookup errors", lookups.iter().all(|e| has(e.text())));
    let en = english();
    t.eq("fail english", Fail::Connect(TcpEnd::Refused).msg().text_with(&en), s("no connection: refused (RST to the SYN)"));
    t.eq("fail german", Fail::Reset(TcpEnd::Reset).text(), s("Verbindung abgebrochen: von der Gegenseite zurückgesetzt"));
    t.check("brand words", rows.iter().all(|r| !r.1.contains("Wi-Fi") && !r.1.contains("successor")));
}

// texts of a running engine: refusals, problems, the event list, status lines
fn engine(t: &mut T) {
    let en = english();
    let mut p = Project::new("msg", 3);
    let pr = |dev: u32, port: &str| PortRef { dev, port: s(port) };
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let pc2 = p.add_device("pc", 200, 0).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 300, 100).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 100, 100).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    let _ = p.connect(&pr(pc2, "eth0"), &pr(sw, "gi1/0/2"), None);
    let mut e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    let addr = |dev: u32, iface: &str, a: &str| Action::IfAddr { dev, iface: s(iface), prefix: pfx(a), add: true };
    t.check("same address twice", e.apply(addr(pc1, "eth0", "10.0.0.1/24")).is_ok() && e.apply(addr(pc2, "eth0", "10.0.0.1/24")).is_ok());
    // refusals: the German text as before, the message for the catalogue
    let ue = ActErr::UnknownIface { dev: 3, iface: s("Gi0/9") };
    t.eq("refusal", (ue.to_string(), ue.msg().text_with(&en)), (s("Gerät 3 hat kein Interface \u{201e}Gi0/9\u{201c}."), s("Device 3 has no interface \u{201c}Gi0/9\u{201d}.")));
    match e.apply(addr(pc1, "eth0", "224.0.0.1/24")) {
        Err(ActErr::Refused(m)) => t.eq("bad address", (m.tpl, m.text(), m.text_with(&en)), ("{} ist keine gültige Interface-Adresse.", s("224.0.0.1/24 ist keine gültige Interface-Adresse."), s("224.0.0.1/24 is not a valid interface address."))),
        other => t.check(&format!("bad address refused: {:?}", other), false),
    }
    let r = e.apply(addr(r1, "gi0/0/0", "10.1.0.1/24"));
    match (r, e.apply(addr(r1, "gi0/0/1", "10.1.0.2/24"))) {
        (Ok(()), Err(err @ ActErr::Refused(_))) => {
            let m = err.msg();
            t.eq("overlap", (m.tpl, m.text() == err.to_string(), m.text_with(&en)), ("{} überschneidet sich mit {}.", true, s("10.1.0.0/24 overlaps with GigabitEthernet0/0/0.")));
        }
        other => t.check(&format!("overlap refused: {:?}", other), false),
    }
    match e.apply(Action::RemoveService { dev: pc1, name: s("nope") }) {
        Err(err) => t.eq("no such service", (err.to_string(), err.msg().text_with(&en)), (s("Den Dienst \u{201e}nope\u{201c} gibt es nicht."), s("The service \u{201c}nope\u{201d} does not exist."))),
        Ok(()) => t.check("no such service refused", false),
    }
    let bad = ServiceDef { kind: s("tcp-client"), name: s("c"), disabled: false, config: agent_core::json::parse(b"{\"server\": \"x\"}").unwrap_or(Value::Null) };
    match e.apply(Action::Service { dev: pc1, def: bad }) {
        Err(err) => t.eq("bad service", (err.to_string(), err.msg().text_with(&en)), (s("Dienst tcp-client: \u{201e}server\u{201c} fehlt oder ist keine IPv4-Adresse."), s("Service tcp-client: \u{201c}server\u{201d} is missing or not an IPv4 address."))),
        Ok(()) => t.check("bad service refused", false),
    }
    // problems: the German text is the message's
    let probs = e.problems();
    let dup: Vec<_> = probs.iter().filter(|p| p.kind == ProblemKind::DuplicateIp).collect();
    t.eq("duplicate texts", dup.iter().map(|p| p.text.clone()).collect::<Vec<_>>(), alloc::vec![s("10.0.0.1 an eth0 ist im selben Netz doppelt vergeben: auch PC2 (eth0)."), s("10.0.0.1 an eth0 ist im selben Netz doppelt vergeben: auch PC1 (eth0).")]);
    t.check("problem text is msg", !probs.is_empty() && probs.iter().all(|p| p.text == p.msg.text()));
    t.eq("problem english", dup.first().map(|p| p.msg.text_with(&en)), Some(s("10.0.0.1 on eth0 is used twice in the same network: also PC2 (eth0).")));
    // the event list and status lines
    let echo = ServiceDef { kind: s("tcp-echo"), name: s("echo"), disabled: false, config: agent_core::json::parse(b"{\"port\": 7}").unwrap_or(Value::Null) };
    t.check("echo", e.apply(Action::Service { dev: pc2, def: echo }).is_ok());
    let _ = e.run_until(2 * SEC, BUDGET);
    let rows = e.event_rows(&jfn_sim::Filter::all(), 0, 100_000);
    t.check("event rows", !rows.is_empty() && rows.iter().all(|r| r.text == r.msg.text() && e.trace.event(r.seq).is_some_and(|ev| e.describe(ev) == r.text)));
    t.check("event rows translate", rows.iter().all(|r| r.msg.text_with(&en) != r.text));
    let svcs = e.services(pc2);
    t.eq("status", svcs.iter().map(|v| (v.status.clone(), v.status_msg.text(), v.status_msg.text_with(&en))).collect::<Vec<_>>(), alloc::vec![(s("Port 7: 0 Verbindungen, 0 Bytes zurückgeschickt"), s("Port 7: 0 Verbindungen, 0 Bytes zurückgeschickt"), s("Port 7: 0 connections, 0 bytes sent back"))]);
}
