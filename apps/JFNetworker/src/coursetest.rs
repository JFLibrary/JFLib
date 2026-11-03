// coursetest -- the proof that the courses of the Akademie can be done (part of --selftest, group "courses",
// with --academy-dir): every step of every course that is no sample has a row in ROWS, which lives here and not
// in the course's folder because every file there ships to learners; a step without a row fails, and so does a
// row without a step, so a later course cannot skip its proof.
//
//   Row { course, step, proof }   Proof::
//   Lesson                        a lesson that shows no console: only the checks of every lesson
//   Shows(net, before)            a lesson with ```console blocks: `net` (a path in the course's folder) runs,
//                                 `before` happens unseen, then the blocks: a line that starts with a device's
//                                 prompt as it is at that moment is typed there, the lines after it are what
//                                 that console must print (a line "…" stands for any number of lines)
//   Solve(steps)                  a lab: no goal at the start, the steps as a learner does them, every goal, and
//                                 the checks still hold later
//   Hunt { repair, test, right, wrong }   a fault hunt: untouched below the pass mark, also with the right
//                                 diagnosis; repaired without a diagnosis or with the wrong one below it, the wrong
//                                 one costing the points; repaired with the right one 100 %; any device switched
//                                 off earns no more than the untouched network; the target network holds every
//                                 item but the diagnosis; the panels stay locked.  `test`: what a learner tries
//                                 after the repair (grading probes on a copy, so traffic before it counts: a
//                                 router that has yet to ask ARP drops the first packet)
//   Answers(texts)                a quiz or test: the right replies reach 100 (`texts`: one for each text
//                                 question, in order), wrong replies stay below the pass mark, every question
//                                 explains itself
//   Do::{Cli, Recable, Connect}   what a learner does; a line of Do::Cli is typed through the console the window
//                                 uses and waits for the prompt; a cable changes the project, as the canvas does,
//                                 and the simulator follows
//   Who::{Dev, Cable}             what a diagnosis names
//
// Every lesson, whatever its row: a code span that starts with a command word ("ping 10.0.0.2", "show ip
// route") is typed on the devices of tests/fixtures/course_commands.jfnet and must be accepted: on the host, else on
// the router or the switch in one of their modes.  So a span is a real example, no placeholder.  A fence is
// ```console (proved) or ```text (no commands).
//
// Every step: the name rule over every file (academytest.rs has it for lessons and labs), both languages in every
// text of every kind (course.json, labs, fault hunts, quizzes): not empty, not the same text unless it is a name or
// a number, German that reads as German and English as English (common words counted).  Every file of a
// course's folder is used by a step or a lesson.
//
// `--course-fill DIR` is the author's tool: what the consoles printed goes into a copy of each lesson below DIR
// (<course>/<lang>/<file>), so quoted outputs are written by running them; a block line "…" stays while it matches.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::academy::{lesson_refs, Course, Step, StepKind};
use jfn_model::lab::{Lab, Text};
use jfn_model::project::{PortRef, Project};
use jfn_model::quiz::{QKind, Question, Quiz, Reply};
use jfn_model::scenario;
use jfn_sim::assess::{Grader, Pick, Score};
use jfn_sim::goal::{judge, LabRun};
use jfn_sim::{Cmd, Report, MS, SEC};

use crate::doc::sim::Sim;
use crate::selftest::T;
use crate::workspace::academy::store;

/// a learner's step
#[derive(Clone, Copy)]
enum Do {
    /// a line typed at a device's console; it waits for the prompt
    Cli(&'static str, &'static str),
    /// the cable at this port deleted and connected again between the same ends: a new cable
    Recable(&'static str, &'static str),
    /// a new cable between two ports
    Connect(&'static str, &'static str, &'static str, &'static str),
}

/// what a diagnosis names
#[derive(Clone, Copy)]
enum Who {
    Dev(&'static str),
    Cable(&'static str, &'static str, &'static str, &'static str),
}

enum Proof {
    Lesson,
    Shows(&'static str, &'static [Do]),
    Solve(&'static [Do]),
    Hunt { repair: &'static [Do], test: &'static [Do], right: &'static [Who], wrong: &'static [Who] },
    Answers(&'static [&'static str]),
}

struct Row {
    course: &'static str,
    step: &'static str,
    proof: Proof,
}

const fn row(course: &'static str, step: &'static str, proof: Proof) -> Row {
    Row { course, step, proof }
}

const COMMANDS: &str = include_str!("../tests/fixtures/course_commands.jfnet");

/// the words that make a code span a command
const COMMAND_WORDS: [&str; 27] = ["terminal", "ip", "ping", "traceroute", "dig", "nslookup", "host", "dhclient", "cat", "curl", "ss", "echo", "hostname", "show", "enable", "configure", "interface", "no", "switchport", "vlan", "network", "default-router", "dns-server", "domain-name", "clear", "copy", "shutdown"];

/// what a console says about a line it did not understand
const REFUSED: [&str; 9] = ["command not found", "Usage:", "usage:", "Command \"", "% Invalid", "% Incomplete", "% Ambiguous", "% Unknown", "Translating"];

const R1_UP: &[Do] = &[Do::Cli("R1", "enable"), Do::Cli("R1", "configure terminal"), Do::Cli("R1", "interface GigabitEthernet0/0/1"), Do::Cli("R1", "ip address 10.2.0.1 255.255.255.0"), Do::Cli("R1", "no shutdown"), Do::Cli("R1", "end")];

const ROWS: &[Row] = &[
    // ---- grundlagen
    row("grundlagen", "netz", Proof::Shows("labs/erstes-netz.jfnet", &[])),
    row("grundlagen", "rahmen", Proof::Shows("labs/erstes-netz.jfnet", &[Do::Connect("PC2", "eth0", "SW1", "GigabitEthernet1/0/2"), Do::Cli("PC2", "ip addr add 192.168.1.20/24 dev eth0")])),
    row("grundlagen", "arp", Proof::Shows("labs/erstes-netz.jfnet", &[Do::Connect("PC2", "eth0", "SW1", "GigabitEthernet1/0/2"), Do::Cli("PC2", "ip addr add 192.168.1.20/24 dev eth0")])),
    row("grundlagen", "erstes-netz", Proof::Solve(&[Do::Connect("PC2", "eth0", "SW1", "GigabitEthernet1/0/2"), Do::Cli("PC2", "ip addr add 192.168.1.20/24 dev eth0"), Do::Cli("PC1", "ping -c 2 192.168.1.20"), Do::Cli("PC1", "ip neigh"), Do::Cli("SW1", "enable"), Do::Cli("SW1", "show mac address-table")])),
    row("grundlagen", "quiz-netz", Proof::Answers(&["show mac address-table"])),
    row("grundlagen", "ipv4", Proof::Lesson),
    row("grundlagen", "maske", Proof::Shows("labs/falsche-maske.jfnet", &[])),
    row("grundlagen", "selbes-netz", Proof::Shows("labs/falsche-maske.jfnet", &[])),
    row("grundlagen", "falsche-maske", Proof::Solve(&[Do::Cli("PC2", "ip addr del 192.168.10.130/25 dev eth0"), Do::Cli("PC2", "ip addr add 192.168.10.130/24 dev eth0")])),
    row("grundlagen", "quiz-adressen", Proof::Answers(&["255.255.255.128", "172.16.5.128/25"])),
    row("grundlagen", "router", Proof::Shows("labs/router-gateway.jfnet", &[])),
    row("grundlagen", "gateway", Proof::Shows("labs/router-gateway.jfnet", R1_UP)),
    row("grundlagen", "routing-tabelle", Proof::Shows("labs/router-gateway.jfnet", &[Do::Cli("R1", "enable"), Do::Cli("R1", "configure terminal"), Do::Cli("R1", "interface GigabitEthernet0/0/1"), Do::Cli("R1", "ip address 10.2.0.1 255.255.255.0"), Do::Cli("R1", "no shutdown"), Do::Cli("R1", "end"), Do::Cli("PC3", "ip route add default via 10.2.0.1")])),
    row("grundlagen", "router-gateway", Proof::Solve(&[Do::Cli("R1", "enable"), Do::Cli("R1", "configure terminal"), Do::Cli("R1", "interface GigabitEthernet0/0/1"), Do::Cli("R1", "ip address 10.2.0.1 255.255.255.0"), Do::Cli("R1", "no shutdown"), Do::Cli("R1", "end"), Do::Cli("PC3", "ip route add default via 10.2.0.1"), Do::Cli("PC1", "traceroute 10.2.0.10")])),
    row("grundlagen", "quiz-router", Proof::Answers(&["ip route add default via 10.2.0.1"])),
    row("grundlagen", "dhcp", Proof::Shows("labs/dhcp-pool.jfnet", &[])),
    row("grundlagen", "dhcp-pool", Proof::Solve(&[Do::Cli("R1", "enable"), Do::Cli("R1", "configure terminal"), Do::Cli("R1", "ip dhcp excluded-address 10.10.0.1 10.10.0.99"), Do::Cli("R1", "ip dhcp pool BUERO"), Do::Cli("R1", "network 10.10.0.0 255.255.255.0"), Do::Cli("R1", "default-router 10.10.0.1"), Do::Cli("R1", "end"), Do::Cli("PC1", "dhclient -v eth0"), Do::Cli("PC2", "dhclient eth0")])),
    row("grundlagen", "quiz-dhcp", Proof::Answers(&[])),
    row("grundlagen", "dns", Proof::Lesson),
    row("grundlagen", "dig", Proof::Shows("labs/namen.jfnet", &[])),
    row("grundlagen", "namen", Proof::Solve(&[Do::Cli("PC1", "echo \"nameserver 10.30.0.53\" > /etc/resolv.conf"), Do::Cli("PC1", "dig www.schule.lab"), Do::Cli("PC1", "dig start.schule.lab"), Do::Cli("PC1", "curl http://www.schule.lab/")])),
    row("grundlagen", "quiz-dns", Proof::Answers(&["/etc/resolv.conf"])),
    row("grundlagen", "alles-zusammen", Proof::Solve(&[
        Do::Cli("R1", "enable"), Do::Cli("R1", "configure terminal"), Do::Cli("R1", "interface GigabitEthernet0/0/1"), Do::Cli("R1", "ip address 10.30.0.1 255.255.255.0"), Do::Cli("R1", "no shutdown"), Do::Cli("R1", "exit"),
        Do::Cli("R1", "ip dhcp excluded-address 192.168.50.1 192.168.50.99"), Do::Cli("R1", "ip dhcp pool LAN"), Do::Cli("R1", "network 192.168.50.0 255.255.255.0"), Do::Cli("R1", "default-router 192.168.50.1"), Do::Cli("R1", "dns-server 10.30.0.53"), Do::Cli("R1", "end"),
        Do::Cli("NS1", "ip route add default via 10.30.0.1"),
        Do::Cli("WEB1", "ip addr del 10.30.0.80/27 dev eth0"), Do::Cli("WEB1", "ip addr add 10.30.0.80/24 dev eth0"), Do::Cli("WEB1", "ip route add default via 10.30.0.1"),
        Do::Cli("PC1", "dhclient eth0"), Do::Cli("PC2", "dhclient eth0"), Do::Cli("PC1", "curl http://www.schule.lab/"),
    ])),
    row("grundlagen", "abschluss", Proof::Answers(&["255.255.255.192", "10.4.8.255", "default-router 10.0.0.1"])),
    // ---- fehlersuche
    row("fehlersuche", "methode", Proof::Lesson),
    row("fehlersuche", "schichten", Proof::Lesson),
    row("fehlersuche", "eingrenzen", Proof::Shows("labs/rueckweg.jfnet", &[])),
    row("fehlersuche", "werkzeuge", Proof::Shows("labs/kabel.jfnet", &[])),
    row("fehlersuche", "notizen", Proof::Lesson),
    row("fehlersuche", "quiz-methode", Proof::Answers(&[])),
    row("fehlersuche", "kabel", Proof::Hunt { test: &[], repair: &[Do::Recable("SRV1", "eth0")], right: &[Who::Cable("SW1", "GigabitEthernet1/0/3", "SRV1", "eth0")], wrong: &[Who::Cable("PC1", "eth0", "SW1", "GigabitEthernet1/0/1")] }),
    row("fehlersuche", "port", Proof::Hunt { test: &[], repair: &[Do::Cli("SW1", "enable"), Do::Cli("SW1", "configure terminal"), Do::Cli("SW1", "interface GigabitEthernet1/0/2"), Do::Cli("SW1", "no shutdown"), Do::Cli("SW1", "end")], right: &[Who::Dev("SW1")], wrong: &[Who::Dev("PC2")] }),
    row("fehlersuche", "gateway", Proof::Hunt { test: &[], repair: &[Do::Cli("PC1", "ip route del default"), Do::Cli("PC1", "ip route add default via 10.0.1.1")], right: &[Who::Dev("PC1")], wrong: &[Who::Dev("R1")] }),
    row("fehlersuche", "halbes-netz", Proof::Hunt { test: &[], repair: &[Do::Cli("SRV1", "ip addr del 10.0.0.20/25 dev eth0"), Do::Cli("SRV1", "ip addr add 10.0.0.20/24 dev eth0")], right: &[Who::Dev("SRV1")], wrong: &[Who::Dev("PC2")] }),
    row("fehlersuche", "doppelt", Proof::Hunt { test: &[], repair: &[Do::Cli("LAP1", "ip addr del 10.0.0.10/24 dev eth0"), Do::Cli("LAP1", "ip addr add 10.0.0.77/24 dev eth0"), Do::Cli("PC1", "ip neigh flush all")], right: &[Who::Dev("LAP1")], wrong: &[Who::Dev("SRV1")] }),
    row("fehlersuche", "vlan-kurz", Proof::Shows("labs/vlan.jfnet", &[])),
    row("fehlersuche", "vlan", Proof::Hunt { test: &[], repair: &[Do::Cli("SW1", "enable"), Do::Cli("SW1", "configure terminal"), Do::Cli("SW1", "interface GigabitEthernet1/0/2"), Do::Cli("SW1", "switchport access vlan 10"), Do::Cli("SW1", "end")], right: &[Who::Dev("SW1")], wrong: &[Who::Dev("PC2")] }),
    row("fehlersuche", "rueckweg", Proof::Hunt { test: &[], repair: &[Do::Cli("R2", "enable"), Do::Cli("R2", "configure terminal"), Do::Cli("R2", "ip route 10.0.1.0 255.255.255.0 10.0.12.1"), Do::Cli("R2", "end")], right: &[Who::Dev("R2")], wrong: &[Who::Dev("R1")] }),
    row("fehlersuche", "name", Proof::Hunt { test: &[], repair: &[Do::Cli("PC1", "echo \"nameserver 10.0.2.53\" > /etc/resolv.conf")], right: &[Who::Dev("PC1")], wrong: &[Who::Dev("NS1")] }),
    row("fehlersuche", "relay-kurz", Proof::Shows("labs/relay.jfnet", &[])),
    row("fehlersuche", "relay", Proof::Hunt { test: &[], repair: &[Do::Cli("R1", "enable"), Do::Cli("R1", "configure terminal"), Do::Cli("R1", "interface GigabitEthernet0/0/1"), Do::Cli("R1", "ip helper-address 10.0.3.10"), Do::Cli("R1", "end"), Do::Cli("PC2", "dhclient eth0")], right: &[Who::Dev("R1")], wrong: &[Who::Dev("SRV1")] }),
    row("fehlersuche", "verlust", Proof::Hunt { test: &[], repair: &[Do::Recable("R1", "GigabitEthernet0/0/1")], right: &[Who::Cable("R1", "GigabitEthernet0/0/1", "R2", "GigabitEthernet0/0/0")], wrong: &[Who::Cable("PC1", "eth0", "R1", "GigabitEthernet0/0/0")] }),
    row("fehlersuche", "montagmorgen", Proof::Hunt { test: &[Do::Cli("PC1", "ping -c 3 10.0.2.53"), Do::Cli("PC2", "ping -c 3 10.0.2.80")], repair: &[
        Do::Recable("R2", "GigabitEthernet0/0/1"), Do::Cli("R2", "enable"), Do::Cli("R2", "configure terminal"), Do::Cli("R2", "ip route 10.0.1.0 255.255.255.0 10.0.12.1"), Do::Cli("R2", "end"),
        Do::Cli("PC1", "echo \"nameserver 10.0.2.53\" > /etc/resolv.conf"),
    ], right: &[Who::Cable("R2", "GigabitEthernet0/0/1", "SW2", "GigabitEthernet1/0/3"), Who::Dev("R2"), Who::Dev("PC1")], wrong: &[Who::Cable("R1", "GigabitEthernet0/0/1", "R2", "GigabitEthernet0/0/0"), Who::Dev("R1"), Who::Dev("PC2")] }),
    row("fehlersuche", "pruefung", Proof::Answers(&["ip helper-address 10.0.3.10"])),
];

fn s(x: &str) -> String {
    String::from(x)
}

pub(crate) fn run(t: &mut T) {
    let Some(root) = plat::arg_value("--academy-dir") else { return };
    let mut seen = BTreeSet::new();
    for e in store::list(&root) {
        let Some(c) = e.usable().cloned() else {
            t.check(&format!("{} is offered", e.id), false);
            continue;
        };
        // the sample has its own tests (academytest.rs, quiztest.rs)
        if c.sample {
            continue;
        }
        course_t(t, &e, &c);
        files_t(t, &e, &c);
        for step in c.steps() {
            let name = format!("{} {}", e.id, step.id);
            let Some(r) = ROWS.iter().find(|r| r.course == e.id && r.step == step.id) else {
                t.check(&format!("{}: a row in the table of coursetest.rs", name), false);
                continue;
            };
            seen.insert((r.course, r.step));
            match (step.kind(), &r.proof) {
                (Some(StepKind::Lesson), Proof::Lesson) => lesson_t(t, &name, &e, step, None),
                (Some(StepKind::Lesson), Proof::Shows(net, before)) => lesson_t(t, &name, &e, step, Some((net, before))),
                (Some(StepKind::Lab), Proof::Solve(steps)) => lab_t(t, &name, &e, step, steps),
                (Some(StepKind::Fault), Proof::Hunt { repair, test, right, wrong }) => hunt_t(t, &name, &e, step, repair, test, right, wrong),
                (Some(StepKind::Quiz | StepKind::Exam), Proof::Answers(texts)) => quiz_t(t, &name, &e, step, texts),
                _ => t.check(&format!("{}: the row fits the step's kind", name), false),
            }
        }
    }
    for r in ROWS.iter().filter(|r| !seen.contains(&(r.course, r.step))) {
        t.check(&format!("{} {}: a row without a step", r.course, r.step), false);
    }
}

// ---------------------------------------------------------------- a network as a learner has it

/// a project running as in the window: the console's text kept for the checks
struct Walk {
    p: Project,
    sim: Sim,
    out: BTreeMap<u32, String>,
}

impl Walk {
    fn new(p: &Project) -> Walk {
        Walk { p: p.clone(), sim: Sim::new(p), out: BTreeMap::new() }
    }
    fn id(&self, name: &str) -> u32 {
        self.p.find(name).map_or(0, |d| d.id)
    }
    fn reports(&mut self) -> String {
        let mut text = String::new();
        for r in self.sim.take_reports() {
            if let Report::CliOut { dev, bytes, .. } = r {
                let b = String::from_utf8_lossy(&bytes);
                self.out.entry(dev).or_default().push_str(&b);
                text.push_str(&b);
            }
        }
        text
    }
    fn secs(&mut self, secs: u64) {
        let _ = self.sim.send(Cmd::RunFor(secs * SEC));
        self.reports();
    }
    /// a line typed on a device's console, the prompt waited for: what it printed
    fn line(&mut self, dev: u32, line: &str) -> String {
        self.reports();
        let echo = format!("{}{}\n", self.sim.engine().cli_prompt(dev), line);
        self.out.entry(dev).or_default().push_str(&echo);
        let _ = self.sim.send(Cmd::Cli { dev, line: s(line) });
        let mut text = self.reports();
        let mut wait = 0;
        while self.sim.engine().cli_busy(dev) && wait < 1200 {
            let _ = self.sim.send(Cmd::RunFor(100 * MS));
            text.push_str(&self.reports());
            wait += 1;
        }
        text
    }
    fn act(&mut self, d: Do) {
        match d {
            Do::Cli(dev, line) => {
                let id = self.id(dev);
                self.line(id, line);
            }
            Do::Recable(dev, port) => {
                let id = self.id(dev);
                let Some(l) = self.p.link_at(id, port).cloned() else { return };
                let _ = self.p.disconnect(l.id);
                let _ = self.p.connect(&l.a, &l.b, Some(l.cable));
                self.sim.sync(&self.p);
            }
            Do::Connect(a, ap, b, bp) => {
                let (a, b) = (PortRef { dev: self.id(a), port: s(ap) }, PortRef { dev: self.id(b), port: s(bp) });
                let _ = self.p.connect(&a, &b, None);
                self.sim.sync(&self.p);
            }
        }
    }
    fn all(&mut self, steps: &[Do]) {
        for d in steps {
            self.act(*d);
        }
    }
    fn pick(&self, w: Who) -> Pick {
        match w {
            Who::Dev(d) => Pick::Dev(self.id(d)),
            Who::Cable(a, ap, b, bp) => Pick::Cable(PortRef { dev: self.id(a), port: s(ap) }, PortRef { dev: self.id(b), port: s(bp) }),
        }
    }
    /// the items of an assessment, handed in with these answers to its diagnoses, in order
    fn grade(&self, g: &Grader, who: &[Who]) -> Score {
        let mut named = who.iter();
        let picks: Vec<Option<Pick>> = g.items().iter().map(|i| if i.what == "diagnosis" { named.next().map(|w| self.pick(*w)) } else { None }).collect();
        let out = &self.out;
        g.grade_picks(&self.p, self.sim.engine(), &|d| out.get(&d).cloned().unwrap_or_default(), &picks, true)
    }
}

// ---------------------------------------------------------------- the texts

// words that are common in one language and rare in the other
const WORDS_DE: [&str; 34] = ["der", "die", "das", "und", "ist", "nicht", "ein", "eine", "einen", "mit", "auf", "für", "von", "den", "dem", "sich", "sie", "wird", "wenn", "auch", "zu", "im", "wie", "dann", "aber", "oder", "nur", "bei", "nach", "über", "hat", "kann", "er", "es"];
const WORDS_EN: [&str; 27] = ["the", "and", "is", "not", "of", "to", "that", "this", "it", "with", "for", "on", "are", "you", "if", "by", "from", "has", "which", "a", "be", "or", "but", "at", "as", "its", "can"];

// the words of a text outside code spans
fn plain_words(text: &str) -> Vec<String> {
    let mut code = false;
    let mut plain = String::new();
    for c in text.chars() {
        match c {
            '`' => code = !code,
            _ if !code => plain.push(c),
            _ => {}
        }
    }
    plain.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '\'').filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect()
}

// German that reads as German, English as English: never more of the other language's common words, and a longer
// text has some of its own
fn reads_as(t: &mut T, name: &str, text: &str, de: bool) {
    let words = plain_words(text);
    let count = |list: &[&str]| words.iter().filter(|w| list.contains(&w.as_str())).count();
    let (g, e) = (count(&WORDS_DE), count(&WORDS_EN));
    let (own, other) = if de { (g, e) } else { (e, g) };
    let good = other <= own && (words.len() < 12 || own > 0);
    t.check(&format!("{}: reads as {} ({} German, {} English words): {}", name, if de { "German" } else { "English" }, g, e, text), good);
}

// a text of a course: in both languages, two texts unless it is a name or a number
fn both(t: &mut T, name: &str, x: &Text) {
    t.check(&format!("{}: German", name), !x.de.trim().is_empty());
    t.check(&format!("{}: English: {}", name, x.de), !x.en.trim().is_empty());
    if x.de == x.en {
        t.check(&format!("{}: the same text only for a name or a number: {}", name, x.de), plain_words(&x.de).iter().filter(|w| w.chars().any(char::is_alphabetic)).count() <= 2);
        return;
    }
    reads_as(t, name, &x.de, true);
    reads_as(t, name, &x.en, false);
}

// the naming rule of the README: our own words, no one else's names
fn name_rule(t: &mut T, name: &str, text: &str) {
    let low = text.to_lowercase();
    for word in ["networking academy", "netacad", "cisco", "packet tracer"] {
        t.check(&format!("{}: no {}", name, word), !low.contains(word));
    }
}

fn course_t(t: &mut T, e: &store::Entry, c: &Course) {
    both(t, &format!("{}: title", e.id), &c.title);
    both(t, &format!("{}: summary", e.id), &c.summary);
    for u in &c.units {
        both(t, &format!("{}: unit", e.id), &u.title);
    }
    for step in c.steps() {
        both(t, &format!("{} {}: title", e.id, step.id), &step.title);
    }
}

// every file of the folder: the name rule, and used by something (what ships must be for the learner)
fn files_t(t: &mut T, e: &store::Entry, c: &Course) {
    let mut used: BTreeSet<String> = BTreeSet::new();
    used.insert(s("course.json"));
    for step in c.steps() {
        if step.kind() == Some(StepKind::Lesson) {
            for lang in ["de", "en"] {
                let rel = format!("{}/{}", lang, step.file);
                if let Some((md, _)) = store::lesson(&e.dir, &step.file, lang) {
                    used.extend(lesson_refs(&md).into_iter().filter(|r| r.image).map(|r| r.target));
                }
                used.insert(rel);
            }
        } else {
            used.insert(step.file.clone());
        }
    }
    let mut stack = alloc::vec![String::new()];
    while let Some(rel) = stack.pop() {
        let dir = if rel.is_empty() { e.dir.clone() } else { format!("{}/{}", e.dir, rel) };
        for f in plat::fs::read_dir(&dir).unwrap_or_default() {
            let path = if rel.is_empty() { f.name.clone() } else { format!("{}/{}", rel, f.name) };
            if f.kind == plat::fs::FileKind::Dir {
                stack.push(path);
                continue;
            }
            let data = plat::read_file(&format!("{}/{}", e.dir, path)).unwrap_or_default();
            name_rule(t, &format!("{} {}", e.id, path), &String::from_utf8_lossy(&data));
            t.check(&format!("{} {}: used by a step or a lesson", e.id, path), used.contains(&path));
        }
    }
}

fn lab_texts(t: &mut T, name: &str, lab: &Lab) {
    both(t, &format!("{}: title", name), &lab.title);
    both(t, &format!("{}: summary", name), &lab.summary);
    for x in &lab.intro {
        both(t, &format!("{}: introduction", name), x);
    }
    for x in &lab.hints {
        both(t, &format!("{}: hint", name), x);
    }
    for g in &lab.goals {
        both(t, &format!("{}: goal", name), &g.text);
        for x in &g.hints {
            both(t, &format!("{}: a goal's hint", name), x);
        }
    }
    if let Some(a) = &lab.assess {
        for i in &a.items {
            both(t, &format!("{}: item", name), &i.text);
            if i.what == "diagnosis" {
                both(t, &format!("{}: a diagnosis explains", name), &i.explain);
            }
        }
    }
}

// ---------------------------------------------------------------- a lesson

// the fenced blocks of a Markdown text: language, lines, the lines they span
struct Fence {
    lang: String,
    lines: Vec<String>,
    from: usize,
    to: usize,
}

fn fences(md: &str) -> Vec<Fence> {
    let mut v = Vec::new();
    let mut open: Option<Fence> = None;
    for (i, line) in md.lines().enumerate() {
        match (open.as_mut(), line.trim_start().strip_prefix("```")) {
            (None, Some(lang)) => open = Some(Fence { lang: s(lang.trim()), lines: Vec::new(), from: i, to: i }),
            (Some(f), Some(_)) => {
                f.to = i;
                v.extend(open.take());
            }
            (Some(f), None) => f.lines.push(s(line)),
            (None, None) => {}
        }
    }
    v
}

// the code spans of a Markdown text, outside fences
fn spans(md: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut fence = false;
    for line in md.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
            continue;
        }
        if !fence {
            v.extend(line.split('`').skip(1).step_by(2).map(String::from));
        }
    }
    v
}

// the quoted lines against what was printed; "…" stands for any number of lines
fn same_output(quoted: &[String], got: &str) -> bool {
    let got: Vec<&str> = got.lines().map(str::trim_end).collect();
    fn go(q: &[String], g: &[&str]) -> bool {
        match q.first() {
            None => g.is_empty(),
            Some(x) if x.trim() == "…" => (0..=g.len()).any(|k| go(&q[1..], &g[k..])),
            Some(x) => g.first().is_some_and(|y| x.trim_end() == *y) && go(&q[1..], &g[1..]),
        }
    }
    go(quoted, &got)
}

fn lesson_t(t: &mut T, name: &str, e: &store::Entry, step: &Step, net: Option<(&&'static str, &&'static [Do])>) {
    let fill = plat::arg_value("--course-fill");
    for lang in ["de", "en"] {
        let n = format!("{} {}", name, lang);
        let rel = format!("{}/{}", lang, step.file);
        let Some(md) = plat::read_file(&format!("{}/{}", e.dir, rel)).map(|b| String::from_utf8_lossy(&b).into_owned()) else {
            t.check(&format!("{}: the file", n), false);
            continue;
        };
        reads_as(t, &n, &md, lang == "de");
        let words = plain_words(&md).len();
        t.check(&format!("{}: a lesson of some length ({} words)", n, words), words >= 150);
        for span in spans(&md) {
            span_t(t, &n, &span);
        }
        let blocks = fences(&md);
        for f in &blocks {
            t.check(&format!("{}: a fence is console or text, not \"{}\"", n, f.lang), f.lang == "console" || f.lang == "text");
        }
        let console: Vec<&Fence> = blocks.iter().filter(|f| f.lang == "console").collect();
        t.check(&format!("{}: its console blocks and its row agree", n), console.is_empty() || net.is_some());
        let Some((path, before)) = net else { continue };
        t.check(&format!("{}: the row names a net for blocks", n), !console.is_empty());
        let Ok(p) = crate::doc::file::read(&format!("{}/{}", e.dir, path)) else {
            t.check(&format!("{}: {} loads", n, path), false);
            continue;
        };
        let mut w = Walk::new(&p);
        w.secs(3);
        w.all(before);
        let mut lines: Vec<String> = md.lines().map(String::from).collect();
        let mut shift = 0isize;
        for (k, f) in console.iter().enumerate() {
            let filled = block_t(t, &format!("{} block {}", n, k + 1), &mut w, f);
            let from = (f.from as isize + shift) as usize + 1;
            let to = (f.to as isize + shift) as usize;
            shift += filled.len() as isize - (to - from) as isize;
            lines.splice(from..to, filled);
        }
        if let Some(dir) = &fill {
            let text = lines.join("\n") + "\n";
            let path = format!("{}/{}/{}", dir, e.id, rel);
            plat::mkdir(dir);
            plat::mkdir(&format!("{}/{}", dir, e.id));
            plat::mkdir(&format!("{}/{}/{}", dir, e.id, lang));
            plat::write_file(&path, text.as_bytes());
        }
    }
}

// a console block run: what the block should say
fn block_t(t: &mut T, name: &str, w: &mut Walk, f: &Fence) -> Vec<String> {
    let mut filled = Vec::new();
    let mut k = 0;
    while k < f.lines.len() {
        let line = &f.lines[k];
        let ids: Vec<u32> = w.p.devices.iter().map(|d| d.id).collect();
        let Some((dev, cmd)) = ids.iter().find_map(|&id| {
            let prompt = w.sim.engine().cli_prompt(id);
            line.strip_prefix(prompt.as_str()).filter(|_| !prompt.is_empty()).map(|c| (id, s(c)))
        }) else {
            t.check(&format!("{}: \"{}\" is no prompt of a device now", name, line), false);
            filled.push(line.clone());
            k += 1;
            continue;
        };
        // a person takes a moment to type
        w.secs(1);
        let got = w.line(dev, &cmd);
        // what follows up to a line with a prompt as it is now (a command may have changed its own)
        let mut end = k + 1;
        while end < f.lines.len() && !ids.iter().any(|&id| f.lines[end].starts_with(w.sim.engine().cli_prompt(id).as_str())) {
            end += 1;
        }
        let quoted = &f.lines[k + 1..end];
        let same = same_output(quoted, &got);
        t.check(&format!("{}: {}\n  quoted: {:?}\n  printed: {:?}", name, line, quoted, got), same);
        filled.push(line.clone());
        if same {
            filled.extend(quoted.iter().cloned());
        } else {
            filled.extend(got.lines().map(|l| s(l.trim_end())));
        }
        k = end;
    }
    filled
}

// a code span that starts with a command word is typed: the host, else the router and the switch in their modes
fn span_t(t: &mut T, name: &str, span: &str) {
    let w: Vec<&str> = span.split_whitespace().collect();
    if w.len() < 2 && span != "enable" || !w.first().is_some_and(|x| COMMAND_WORDS.contains(x)) {
        return;
    }
    let Ok(p) = Project::from_text(COMMANDS) else {
        t.check("the commands' net loads", false);
        return;
    };
    let accepted = |text: &str| !REFUSED.iter().any(|r| text.contains(r));
    let mut walk = Walk::new(&p);
    let pc = walk.id("PC1");
    if accepted(&walk.line(pc, span)) {
        t.check("", true);
        return;
    }
    const MODES: [&[&str]; 5] = [&[], &["configure terminal"], &["configure terminal", "interface GigabitEthernet0/0/0"], &["configure terminal", "ip dhcp pool P"], &["configure terminal", "vlan 10"]];
    const MODES_SW: [&[&str]; 2] = [&["configure terminal", "interface GigabitEthernet1/0/1"], &["configure terminal", "vlan 10"]];
    let tries: Vec<(&str, &[&str])> = MODES.iter().map(|m| ("R1", *m)).chain(MODES.iter().map(|m| ("SW1", *m))).chain(MODES_SW.iter().map(|m| ("SW1", *m))).collect();
    for (dev, mode) in tries {
        let mut walk = Walk::new(&p);
        let id = walk.id(dev);
        walk.line(id, "enable");
        for m in mode {
            walk.line(id, m);
        }
        if accepted(&walk.line(id, span)) {
            t.check("", true);
            return;
        }
    }
    t.check(&format!("{}: the command `{}` is accepted", name, span), false);
}

// ---------------------------------------------------------------- a lab

fn lab_t(t: &mut T, name: &str, e: &store::Entry, step: &Step, solve: &[Do]) {
    let Ok(p) = crate::doc::file::read(&format!("{}/{}", e.dir, step.file)) else {
        t.check(&format!("{} loads", name), false);
        return;
    };
    let Some(lab) = p.lab.clone() else {
        t.check(&format!("{} has a task", name), false);
        return;
    };
    lab_texts(t, name, &lab);
    t.check(&format!("{}: a goal has hints", name), lab.goals.iter().all(|g| !g.hints.is_empty()));
    let mut w = Walk::new(&p);
    let mut run = LabRun::new(&lab);
    let eval = |w: &Walk, run: &mut LabRun| {
        let out = &w.out;
        run.eval(&w.p, w.sim.engine(), &|d| out.get(&d).cloned().unwrap_or_default());
    };
    for _ in 0..4 {
        w.secs(1);
        eval(&w, &mut run);
    }
    t.eq(&format!("{}: no goal at the start", name), run.reached(), 0);
    for d in solve {
        w.act(*d);
        eval(&w, &mut run);
    }
    for _ in 0..4 {
        w.secs(1);
        eval(&w, &mut run);
    }
    let why: Vec<String> = run.goals.iter().filter(|g| g.reached.is_none()).flat_map(|g| g.last.iter().filter(|x| !x.held).map(|x| x.report())).collect();
    t.check(&format!("{}: solved ({})", name, why.join("; ")), run.done());
    // and they stay: the checks still hold later (a goal once reached would not say so)
    w.secs(20);
    let out = &w.out;
    for g in &lab.goals {
        for c in &g.checks {
            let held = scenario::parse_expectation(c).is_ok_and(|x| judge(&w.p, w.sim.engine(), &x, &|d| out.get(&d).cloned().unwrap_or_default()).held);
            t.check(&format!("{}: still {}", name, c), held);
        }
    }
}

// ---------------------------------------------------------------- a fault hunt

#[allow(clippy::too_many_arguments)]
fn hunt_t(t: &mut T, name: &str, e: &store::Entry, step: &Step, repair: &[Do], test: &[Do], right: &[Who], wrong: &[Who]) {
    let Ok(p) = crate::doc::file::read(&format!("{}/{}", e.dir, step.file)) else {
        t.check(&format!("{} loads", name), false);
        return;
    };
    let Some(lab) = p.lab.clone() else {
        t.check(&format!("{} has a task", name), false);
        return;
    };
    let Some(a) = lab.assess.clone() else {
        t.check(&format!("{} grades", name), false);
        return;
    };
    lab_texts(t, name, &lab);
    t.check(&format!("{}: nothing shows the answer", name), lab.lock.problems && lab.lock.faults && lab.lock.answer);
    t.check(&format!("{}: a pass mark of its own", name), lab.pass > 0 && lab.pass < 100);
    t.check(&format!("{}: hints", name), !lab.hints.is_empty());
    t.check(&format!("{}: a diagnosis", name), a.items.iter().any(|i| i.what == "diagnosis"));
    let g = Grader::new(&a);
    let pass = lab.pass_percent();
    // long enough for a DHCP client's first tries and for a measurement's span
    let wait = 20;
    // the target network holds every item but the diagnosis
    let goal = a.target.project(&p);
    let mut w = Walk::new(&goal);
    w.secs(wait);
    w.all(test);
    let sc = w.grade(&g, &[]);
    let diag: u32 = a.items.iter().filter(|i| i.what == "diagnosis").map(|i| i.points).sum();
    t.eq(&format!("{}: the target holds the items", name), sc.points, a.total() - diag);
    // untouched: below the mark, with and without the diagnosis
    let mut w = Walk::new(&p);
    w.secs(wait);
    let broken = w.grade(&g, &[]);
    t.check(&format!("{}: untouched below the mark ({} %)", name, broken.percent()), broken.percent() < pass);
    let named = w.grade(&g, right);
    t.check(&format!("{}: untouched with the diagnosis below the mark ({} %)", name, named.percent()), named.percent() < pass);
    // any device off earns no more than the untouched network
    for d in &p.devices {
        let mut w = Walk::new(&p);
        let mut q = w.p.clone();
        if let Some(x) = q.device_mut(d.id) {
            x.off = true;
        }
        w.p = q;
        w.sim.sync(&w.p);
        w.secs(wait);
        let sc = w.grade(&g, &[]);
        t.check(&format!("{}: {} off earns nothing ({} against {})", name, d.name, sc.points, broken.points), sc.points <= broken.points);
    }
    // repaired
    let mut w = Walk::new(&p);
    w.secs(2);
    w.all(repair);
    w.secs(wait);
    w.all(test);
    let full = w.grade(&g, right);
    t.eq(&format!("{}: repaired with the diagnosis: 100 % ({})", name, why(&a, &full)), full.percent(), 100);
    let none = w.grade(&g, &[]);
    t.check(&format!("{}: repaired without a diagnosis below the mark ({} %)", name, none.percent()), none.percent() < pass);
    let bad = w.grade(&g, wrong);
    t.check(&format!("{}: repaired with a wrong one below the mark ({} %)", name, bad.percent()), bad.percent() < pass);
    t.eq(&format!("{}: a wrong one costs its points", name), bad.points, a.total() - diag);
}

// what the items that do not hold found
fn why(a: &jfn_model::lab::Assess, sc: &Score) -> String {
    sc.items.iter().zip(&a.items).filter(|(g, _)| !g.held).map(|(g, i)| format!("{}: {}", i.text.de, g.found.text())).collect::<Vec<_>>().join("; ")
}

// ---------------------------------------------------------------- a quiz or test

fn quiz_t(t: &mut T, name: &str, e: &store::Entry, step: &Step, texts: &[&str]) {
    let Some(text) = store::read_quiz(&e.dir, &step.file) else {
        t.check(&format!("{} reads", name), false);
        return;
    };
    let q = match Quiz::from_text(&text) {
        Ok(q) => q,
        Err(_) => {
            t.check(&format!("{} is a quiz", name), false);
            return;
        }
    };
    t.check(&format!("{}: a pass mark of its own", name), q.pass > 0 && q.pass < 100);
    let mut typed = texts.iter();
    let mut right = Vec::new();
    let mut wrong = Vec::new();
    for (i, x) in q.questions.iter().enumerate() {
        let qn = format!("{} question {}", name, i + 1);
        both(t, &format!("{}: stem", qn), &x.stem);
        both(t, &format!("{}: explanation", qn), &x.explain);
        for a in &x.answers {
            both(t, &format!("{}: answer", qn), &a.text);
            if !a.explain.is_empty() {
                both(t, &format!("{}: an answer's explanation", qn), &a.explain);
            }
        }
        for o in &x.order {
            both(t, &format!("{}: an entry", qn), o);
        }
        let want = if x.kind_of() == Some(QKind::Text) { typed.next().copied() } else { Some("") };
        t.check(&format!("{}: the table has its text", qn), want.is_some());
        let r = right_reply(x, want.unwrap_or(""));
        t.eq(&format!("{}: the right reply", qn), x.grade(&r).got, x.grade(&r).max);
        right.push(r);
        let r = wrong_reply(x);
        t.eq(&format!("{}: a wrong reply", qn), x.grade(&r).got, 0);
        wrong.push(r);
    }
    t.check(&format!("{}: the table has no text too many", name), typed.next().is_none());
    t.eq(&format!("{}: all right", name), q.grade(&right).percent(), 100);
    t.check(&format!("{}: all wrong below the mark", name), q.grade(&wrong).percent() < q.pass_percent());
    t.check(&format!("{}: the title in both languages when it has one", name), q.title.is_empty() || !q.title.en.is_empty());
    if step.kind() == Some(StepKind::Exam) {
        t.check(&format!("{}: a test says what it asks of the learner", name), q.minutes > 0);
    }
}

// the reply that gets all the points of a question
fn right_reply(x: &Question, text: &str) -> Reply {
    let mut r = x.blank(&[]);
    match x.kind_of() {
        Some(QKind::Single) => r.choice = x.answers.iter().position(|a| a.right).map_or(-1, |i| i as i32),
        Some(QKind::Multi) => r.marks = x.answers.iter().map(|a| a.right as i32).collect(),
        Some(QKind::Number) => r.text = x.value_text(),
        Some(QKind::Order) => r.order = (0..x.order.len()).collect(),
        Some(QKind::Text) => r.text = s(text),
        None => {}
    }
    r
}

// a reply that gets none: the wrong answers marked, a text that is no answer, the order turned round
fn wrong_reply(x: &Question) -> Reply {
    let mut r = x.blank(&[]);
    match x.kind_of() {
        Some(QKind::Single) => r.choice = x.answers.iter().position(|a| !a.right).map_or(-1, |i| i as i32),
        Some(QKind::Multi) => r.marks = x.answers.iter().map(|a| !a.right as i32).collect(),
        Some(QKind::Number) => r.text = s("999999"),
        Some(QKind::Order) => r.order = (0..x.order.len()).rev().collect(),
        Some(QKind::Text) => r.text = s("keine Antwort"),
        None => {}
    }
    r
}
