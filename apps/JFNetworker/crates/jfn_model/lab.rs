// lab -- the task of a lab project: a title, an introduction, goals that checks in the .scn
// expectation syntax decide, hints, every text in German and English
//
//   Lab { title, summary, topic, intro, ordered, goals, hints }   Project::lab (None: a free
//                                                 project); the "lab" member of a .jfnet file
//   Goal { text, checks, hints }                  checks: the words of "expect ..." lines of a
//                                                 scenario (scenario.rs), all of them must hold
//                                                 at one moment
//   Text { de, en } / text.get(en)                a text in both languages (English falls back
//                                                 to the German)
//   lab.issues(&project) -> Vec<Msg>              what is wrong with it: no title or goals, a
//                                                 goal without checks, a check that does not
//                                                 parse, a device the project lacks
//   Goal::parse_checks() -> Result<Vec<Expect>, (index, why)>
//
// A text in a file is a string (German, shown in every language) or a pair ["Deutsch",
// "English"].  Which goals are reached, and when, is the simulator's business (jfn_sim's goal.rs
// evaluates the checks against the running engine); the file only describes the task.
//
// An assessment (Lab::assess, A1) grades the work with points: the project itself is the start
// network, the target ("answer") network travels in the file, and graded items say what of it
// counts, how much, and how it compares.  It is for learning, not for exams that must be
// tamper-proof: whoever opens the file can read the answer.
//
//   Assess { feedback, time_limit_s, items, target }   Feedback: Items (each item right or
//                                                 wrong), Score (the score only), End (nothing
//                                                 until it is handed in or the time is up)
//   Item { what, dev, id, port, svc, key, peer, peer_id, peer_port, value, cmp, checks, points,
//          text }                                 what: ItemKind::parse; cmp: Compare (norm,
//                                                 exact, any of several values)
//   Target { devices, links } / target.project(&base)   the answer as a project of its own (to
//                                                 run, to pick items from)
//   assess.total() / assess.issues(&project)      the points there are; what is wrong with it
//   item.kind() / item.values() / item.check_list() -> Result<Vec<Expect>, (index, why)>
//   canon(kind, text) -> Option<String>           a value in its canonical form ("10.0.1.1
//                                                 255.255.255.0" -> "10.0.1.1/24", "no shut" ->
//                                                 "up", "VLAN 20" -> "20"); None: no such value
//   RouteVal::parse(text) / Display                a static route: "10.0.2.0/24 via 10.0.1.2",
//                                                 "default via 10.0.0.1 dev Gi0/0 distance 5",
//                                                 IOS and Linux spellings read too
//
//   "assess": {
//     "feedback": "items",                         "items" | "score" | "end"
//     "time_limit_s": 1200,                        seconds from opening, 0: no limit
//     "items": [
//       {"what": "address", "dev": "R1", "id": 2, "port": "Gi0/0/0", "value": "10.0.1.1/24",
//        "points": 2, "text": ["R1 Gi0/0/0: Adresse 10.0.1.1/24", "R1 Gi0/0/0: address ..."]},
//       {"what": "config", "dev": "R1", "value": ["interface GigabitEthernet0/0/1",
//        " ip address 10.0.2.1 255.255.255.0", " no shutdown"]},
//       {"what": "cable", "dev": "PC2", "port": "eth0", "peer": "SW2", "peer_port": "Gi1/0/2"},
//       {"what": "check", "checks": ["PC1 reaches 10.0.2.10"], "points": 4}],
//     "target": {"devices": [...], "links": [...]}
//   }
//
// The items: "hostname" (value: the name), "address" of an interface ("10.0.1.1/24", an IPv6
// prefix too), "state" of an interface or port ("up": not shut down, "down"), "vlan" (the access
// VLAN of a switch port), "mode" ("access", "trunk", "dynamic", "routed"), "route" (a static
// route, RouteVal), "service" (`svc` and the path `key` of a setting, "pools.0.range": a JSON
// value), "cable" (between dev/port and peer/peer_port, a port left out: any; value: a
// CableKind or nothing), "config" (lines of the running configuration, a block's lines indented
// under its first line), "check" (expectations in the .scn syntax, all at one moment).  A device
// is found by `id` (the start network keeps the target's ids), else by `dev`.  "cmp": "norm" (the
// default: equivalent spellings count), "exact" (the same text), "any" (value is a list, one of
// which counts).  Members at their default are left out of the file; "points" defaults to 1.

use alloc::string::String;
use alloc::vec::Vec;

use agent_core::jmap::{Field, Value};
use agent_core::{record, wire_enum};
use netpkt::{Ipv4, Prefix4, Prefix6};

use crate::msg;
use crate::msg::Msg;
use crate::project::{Device, Link, Project};
use crate::scenario::{self, Expect};

/// a text in German and English (the English may be missing: the German stands in)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Text {
    pub de: String,
    pub en: String,
}

impl Text {
    pub fn new(de: &str, en: &str) -> Text {
        Text { de: String::from(de), en: String::from(en) }
    }
    /// the text in English (`en`) or German
    pub fn get(&self, en: bool) -> &str {
        if en && !self.en.is_empty() { &self.en } else { &self.de }
    }
    pub fn is_empty(&self) -> bool {
        self.de.trim().is_empty()
    }
}

/// a string (German only) or a pair ["Deutsch", "English"]; {"de": .., "en": ..} reads too
impl Field for Text {
    fn put(&self) -> Value {
        if self.en.is_empty() {
            Value::string(self.de.as_str())
        } else {
            Value::Array(alloc::vec![Value::string(self.de.as_str()), Value::string(self.en.as_str())])
        }
    }
    fn take(v: Option<&Value>) -> Self {
        let s = |x: Option<&Value>| String::from(x.and_then(Value::as_str).unwrap_or(""));
        match v {
            Some(Value::String(t)) => Text { de: t.clone(), en: String::new() },
            Some(Value::Array(a)) => Text { de: s(a.first()), en: s(a.get(1)) },
            Some(o @ Value::Object(_)) => Text { de: s(o.get("de")), en: s(o.get("en")) },
            _ => Text::default(),
        }
    }
}

record! {
    /// a goal of a lab: what the user should reach, the checks that say it is reached, hints
    /// that open one by one
    pub struct Goal {
        pub text: Text,
        /// expectations as a scenario writes them after "expect" ("LB1 svc lb metric
        /// backends_up == 3", "WEB3 route 10.0.2.0/24 via connected"); a leading "expect" is
        /// allowed; all of them hold at one moment when the goal is reached
        pub checks: Vec<String>,
        pub hints: Vec<Text>,
    }
}

record! {
    /// the task of a lab project
    pub struct Lab {
        pub title: Text,
        /// one line for the list of labs
        pub summary: Text,
        /// the shelf of the list: "net" (networks) or "ds" (distributed systems)
        pub topic: String,
        /// the introduction, a text per paragraph
        pub intro: Vec<Text>,
        /// a goal counts only after the goals before it (a story: change, break, recover)
        pub ordered: bool,
        pub goals: Vec<Goal>,
        /// hints for the whole task, opened one by one
        pub hints: Vec<Text>,
        /// graded items with points, the target network, the feedback (None: goals only)
        pub assess: Option<Assess>,
    }
}

impl Goal {
    /// the checks as expectations; Err: the first that does not parse (its index, why)
    pub fn parse_checks(&self) -> Result<Vec<Expect>, (usize, String)> {
        self.checks.iter().enumerate().map(|(i, c)| scenario::parse_expectation(c).map_err(|e| (i, e))).collect()
    }
}

impl Lab {
    /// what is wrong with the lab in this project (German messages for the app's catalogue)
    pub fn issues(&self, p: &Project) -> Vec<Msg> {
        let mut v = Vec::new();
        if self.title.is_empty() {
            v.push(msg!("Das Labor hat keinen Titel."));
        }
        // an assessment may grade with its items alone
        if self.goals.is_empty() && self.assess.is_none() {
            v.push(msg!("Das Labor hat keine Ziele."));
        }
        for (i, g) in self.goals.iter().enumerate() {
            let n = i + 1;
            if g.text.is_empty() {
                v.push(msg!("Ziel {} hat keinen Text.", n));
            }
            if g.checks.is_empty() {
                v.push(msg!("Ziel {} hat keine Prüfung.", n));
            }
            for c in &g.checks {
                match scenario::parse_expectation(c) {
                    Err(e) => v.push(msg!("Ziel {}: \u{201e}{}\u{201c} ist keine Prüfung: {}", n, c, e)),
                    Ok(x) => {
                        if let Some(d) = x.dev()
                            && p.find(d).is_none()
                        {
                            v.push(msg!("Ziel {}: das Gerät {} gibt es im Projekt nicht.", n, d));
                        }
                    }
                }
            }
        }
        if let Some(a) = &self.assess {
            v.extend(a.issues(p));
        }
        v
    }
}

// ---------------------------------------------------------------- assessments (A1)

wire_enum! {
    /// what the learner sees of the grading while working: Items -- every item right or wrong
    /// (and what it found, after "Prüfen"); Score -- the score only; End -- nothing until the
    /// work is handed in or the time is up
    pub enum Feedback {
        Items = "items",
        Score = "score",
        End = "end",
    }
}

wire_enum! {
    /// how an item compares what it finds with what it expects: Norm -- equivalent spellings
    /// count (case and spaces, abbreviated commands, interface names, masks for prefix lengths,
    /// the order of a setting's members); Exact -- the same text (a setting: the same JSON);
    /// Any -- the value is a list, any one of them counts (each compared as Norm)
    pub enum Compare {
        Norm = "norm",
        Exact = "exact",
        Any = "any",
    }
}

wire_enum! {
    /// what an item grades
    pub enum ItemKind {
        Hostname = "hostname",
        Address = "address",
        State = "state",
        Vlan = "vlan",
        Mode = "mode",
        Route = "route",
        Service = "service",
        Cable = "cable",
        Config = "config",
        Check = "check",
    }
}

impl ItemKind {
    /// it looks at an interface or port of its device
    pub fn has_port(self) -> bool {
        matches!(self, ItemKind::Address | ItemKind::State | ItemKind::Vlan | ItemKind::Mode)
    }
    /// its value is a text with a canonical form (canon)
    pub fn is_text(self) -> bool {
        matches!(self, ItemKind::Hostname | ItemKind::Address | ItemKind::State | ItemKind::Vlan | ItemKind::Mode | ItemKind::Route)
    }
}

/// a graded item of an assessment (see the module comment for the kinds and their values)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    /// the kind ("address", ...: ItemKind::parse; an unknown one is an issue)
    pub what: String,
    /// the device: its name in the target, and its id (0: by name alone)
    pub dev: String,
    pub id: u32,
    /// the interface or port (address, state, vlan, mode; a cable's end at `dev`)
    pub port: String,
    /// a service's name and the path of its setting ("backends", "pools.0.range")
    pub svc: String,
    pub key: String,
    /// the far end of a cable: the device (name, id) and its port ("": any)
    pub peer: String,
    pub peer_id: u32,
    pub peer_port: String,
    /// what it expects (Null: nothing, e.g. a cable of any kind); with Compare::Any a list
    pub value: Value,
    pub cmp: Compare,
    /// the expectations of a check (the .scn syntax without "expect"), all at one moment
    pub checks: Vec<String>,
    pub points: u32,
    pub text: Text,
}

impl Field for Item {
    fn put(&self) -> Value {
        let mut o = agent_core::jmap::empty_object();
        o.insert("what", Value::string(self.what.as_str()));
        let text = |o: &mut Value, k: &str, s: &str| {
            if !s.is_empty() {
                o.insert(k, Value::string(s));
            }
        };
        text(&mut o, "dev", &self.dev);
        if self.id != 0 {
            o.insert("id", Value::u64(self.id as u64));
        }
        text(&mut o, "port", &self.port);
        text(&mut o, "svc", &self.svc);
        text(&mut o, "key", &self.key);
        text(&mut o, "peer", &self.peer);
        if self.peer_id != 0 {
            o.insert("peer_id", Value::u64(self.peer_id as u64));
        }
        text(&mut o, "peer_port", &self.peer_port);
        if self.value != Value::Null {
            o.insert("value", self.value.clone());
        }
        if self.cmp != Compare::Norm {
            o.insert("cmp", self.cmp.put());
        }
        if !self.checks.is_empty() {
            o.insert("checks", self.checks.put());
        }
        o.insert("points", Value::u64(self.points as u64));
        if !self.text.de.is_empty() || !self.text.en.is_empty() {
            o.insert("text", self.text.put());
        }
        o
    }
    fn take(v: Option<&Value>) -> Self {
        let Some(v) = v.filter(|v| v.as_object().is_some()) else {
            return Item::default();
        };
        let s = |k: &str| -> String { Field::take(v.get(k)) };
        Item {
            what: s("what"),
            dev: s("dev"),
            id: Field::take(v.get("id")),
            port: s("port"),
            svc: s("svc"),
            key: s("key"),
            peer: s("peer"),
            peer_id: Field::take(v.get("peer_id")),
            peer_port: s("peer_port"),
            value: Field::take(v.get("value")),
            cmp: Field::take(v.get("cmp")),
            checks: Field::take(v.get("checks")),
            // an item without "points" is worth one
            points: v.get("points").map_or(1, |p| Field::take(Some(p))),
            text: Field::take(v.get("text")),
        }
    }
}

impl Item {
    /// its kind (None: unknown)
    pub fn kind(&self) -> Option<ItemKind> {
        ItemKind::parse(self.what.trim())
    }
    /// the values it accepts: the value, or with Compare::Any each of the list (a text, a JSON
    /// value, a config item's lines)
    pub fn values(&self) -> Vec<&Value> {
        match (&self.cmp, &self.value) {
            (Compare::Any, Value::Array(a)) => a.iter().collect(),
            (_, Value::Null) => Vec::new(),
            (_, v) => alloc::vec![v],
        }
    }
    /// the checks of a check item as expectations; Err: the first that does not parse
    pub fn check_list(&self) -> Result<Vec<Expect>, (usize, String)> {
        self.checks.iter().enumerate().map(|(i, c)| scenario::parse_expectation(c).map_err(|e| (i, e))).collect()
    }
    /// the device it names, for messages ("R1", else "#3")
    pub fn dev_name(&self) -> String {
        if self.dev.is_empty() && self.id != 0 { alloc::format!("#{}", self.id) } else { self.dev.clone() }
    }
}

/// the lines of a config item's value: a list of strings, or one text with line ends
pub fn config_lines(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => s.lines().map(String::from).collect(),
        Value::Array(a) => a.iter().filter_map(Value::as_str).flat_map(|s| s.lines().map(String::from).collect::<Vec<_>>()).collect(),
        _ => Vec::new(),
    }
}

record! {
    /// the target network of an assessment: the devices with the configuration they have when
    /// the task is solved, and the cables
    pub struct Target {
        pub devices: Vec<Device>,
        pub links: Vec<Link>,
    }
}

impl Target {
    /// the target as a project of its own: the base project's seed, cable rules and title
    pub fn project(&self, base: &Project) -> Project {
        let mut p = Project::new(&base.title, base.seed);
        p.strict_mdix = base.strict_mdix;
        p.devices = self.devices.clone();
        p.links = self.links.clone();
        p.next_id = p.max_id() + 1;
        p
    }
    /// a device of the target by id, else by name
    pub fn device(&self, id: u32, name: &str) -> Option<&Device> {
        self.devices.iter().find(|d| id != 0 && d.id == id).or_else(|| self.devices.iter().find(|d| !name.is_empty() && d.name.eq_ignore_ascii_case(name.trim())))
    }
}

record! {
    /// an assessment: graded items, the target network and what the learner sees
    pub struct Assess {
        pub feedback: Feedback,
        /// seconds from opening it to the end (0: no limit)
        pub time_limit_s: u32,
        pub items: Vec<Item>,
        /// the answer (the project itself is the start network)
        pub target: Target,
    }
}

impl Assess {
    /// the points there are
    pub fn total(&self) -> u32 {
        self.items.iter().map(|i| i.points).sum()
    }

    /// what is wrong with the assessment in this project (German messages for the app's
    /// catalogue)
    pub fn issues(&self, p: &Project) -> Vec<Msg> {
        let mut v = Vec::new();
        if self.items.is_empty() {
            v.push(msg!("Die Aufgabe hat keine Kriterien."));
        }
        if self.target.devices.is_empty() {
            v.push(msg!("Die Aufgabe hat kein Zielnetz."));
        }
        let found = |id: u32, name: &str| (id != 0 && p.device(id).is_some()) || (!name.is_empty() && p.find(name).is_some());
        for (i, it) in self.items.iter().enumerate() {
            let n = i + 1;
            let Some(kind) = it.kind() else {
                v.push(msg!("Kriterium {}: unbekannte Art \u{201e}{}\u{201c}.", n, it.what));
                continue;
            };
            if kind == ItemKind::Check {
                if it.checks.is_empty() {
                    v.push(msg!("Kriterium {} hat keine Prüfung.", n));
                }
                for c in &it.checks {
                    match scenario::parse_expectation(c) {
                        Err(e) => v.push(msg!("Kriterium {}: \u{201e}{}\u{201c} ist keine Prüfung: {}", n, c, e)),
                        Ok(x) => {
                            if let Some(d) = x.dev()
                                && p.find(d).is_none()
                                && self.target.device(0, d).is_none_or(|t| p.device(t.id).is_none())
                            {
                                v.push(msg!("Kriterium {}: das Gerät {} gibt es im Projekt nicht.", n, d));
                            }
                        }
                    }
                }
                continue;
            }
            if it.dev.is_empty() && it.id == 0 {
                v.push(msg!("Kriterium {} nennt kein Gerät.", n));
            } else if !found(it.id, &it.dev) {
                v.push(msg!("Kriterium {}: das Gerät {} gibt es im Projekt nicht.", n, it.dev_name()));
            }
            if kind.has_port() && it.port.trim().is_empty() {
                v.push(msg!("Kriterium {} nennt keine Schnittstelle.", n));
            }
            if kind == ItemKind::Service && it.svc.trim().is_empty() {
                v.push(msg!("Kriterium {} nennt keinen Dienst.", n));
            }
            if kind == ItemKind::Cable && it.peer.trim().is_empty() && it.peer_id == 0 {
                v.push(msg!("Kriterium {} nennt kein zweites Gerät.", n));
            } else if kind == ItemKind::Cable && !found(it.peer_id, &it.peer) {
                v.push(msg!("Kriterium {}: das Gerät {} gibt es im Projekt nicht.", n, it.peer));
            }
            if it.cmp == Compare::Any && it.value.as_array().is_none_or(|a| a.is_empty()) {
                v.push(msg!("Kriterium {}: \u{201e}any\u{201c} braucht eine Liste von Werten.", n));
                continue;
            }
            let values = it.values();
            if values.is_empty() && kind != ItemKind::Cable {
                v.push(msg!("Kriterium {} erwartet keinen Wert.", n));
            }
            for x in values {
                let bad = match kind {
                    _ if kind.is_text() => x.as_str().is_none_or(|s| canon(kind, s).is_none()),
                    ItemKind::Cable => x.as_str().is_none_or(|s| crate::cable::CableKind::parse(s.trim()).is_none()),
                    ItemKind::Config => config_lines(x).iter().all(|l| l.trim().is_empty()),
                    _ => false,
                };
                if bad {
                    let shown = match x {
                        Value::String(s) => s.clone(),
                        other => agent_core::json::stringify(other).unwrap_or_default(),
                    };
                    v.push(msg!("Kriterium {}: \u{201e}{}\u{201c} ist kein gültiger Wert.", n, shown));
                }
            }
        }
        v
    }
}

/// a static route as an item writes it: the prefix, the next hop and / or the interface, the
/// distance (None: whatever the route has)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteVal {
    pub prefix: Prefix4,
    pub via: Option<Ipv4>,
    pub dev: Option<String>,
    pub ad: Option<u8>,
}

impl RouteVal {
    /// "10.0.2.0/24 via 10.0.1.2", "default via 10.0.0.1", "10.0.2.0/24 dev Gi0/1 distance 5",
    /// IOS "ip route 10.0.2.0 255.255.255.0 [Gi0/1] [10.0.1.2] [5]" and Linux "ip route add
    /// 10.0.2.0/24 via 10.0.1.2 [dev eth0]"
    pub fn parse(s: &str) -> Option<RouteVal> {
        let w: Vec<&str> = s.split_whitespace().collect();
        let w: &[&str] = match w.as_slice() {
            ["ip", "route", "add", rest @ ..] | ["ip", "route", rest @ ..] | ["route", "add", rest @ ..] => rest,
            all => all,
        };
        let (prefix, rest) = match w {
            ["default" | "0.0.0.0/0", rest @ ..] => (Prefix4::default(), rest),
            [net, mask, rest @ ..] if !net.contains('/') && Ipv4::parse(mask).is_some_and(|m| m.mask_len().is_some()) && Ipv4::parse(net).is_some() => {
                (Prefix4::from_mask(Ipv4::parse(net)?, Ipv4::parse(mask)?)?, rest)
            }
            [p, rest @ ..] if p.contains('/') => (Prefix4::parse(p)?, rest),
            _ => return None,
        };
        let mut r = RouteVal { prefix: prefix.trunc(), via: None, dev: None, ad: None };
        let mut k = 0;
        while k < rest.len() {
            match rest[k] {
                "via" => {
                    r.via = Some(Ipv4::parse(rest.get(k + 1)?)?);
                    k += 2;
                }
                "dev" => {
                    r.dev = Some(String::from(*rest.get(k + 1)?));
                    k += 2;
                }
                "distance" | "metric" => {
                    r.ad = Some(rest.get(k + 1)?.parse().ok()?);
                    k += 2;
                }
                x => {
                    // the IOS order: an interface, an address, a distance
                    if let Some(a) = Ipv4::parse(x) {
                        r.via = Some(a);
                    } else if let Ok(d) = x.parse::<u8>() {
                        r.ad = Some(d);
                    } else if x.as_bytes().first().is_some_and(|c| c.is_ascii_alphabetic()) {
                        r.dev = Some(String::from(x));
                    } else {
                        return None;
                    }
                    k += 1;
                }
            }
        }
        if r.via.is_none() && r.dev.is_none() {
            return None;
        }
        Some(r)
    }
}

impl core::fmt::Display for RouteVal {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        if self.prefix.len == 0 {
            f.write_str("default")?;
        } else {
            write!(f, "{}", self.prefix)?;
        }
        if let Some(v) = self.via {
            write!(f, " via {}", v)?;
        }
        if let Some(d) = &self.dev {
            write!(f, " dev {}", crate::ifname::short(d))?;
        }
        if let Some(a) = self.ad {
            write!(f, " distance {}", a)?;
        }
        Ok(())
    }
}

/// the words of a value without a command in front ("ip address 10.0.1.1 255.255.255.0" ->
/// "10.0.1.1 255.255.255.0", "switchport mode trunk" -> "trunk")
fn strip_words<'a>(s: &'a str, prefixes: &[&str]) -> &'a str {
    let t = s.trim();
    for p in prefixes {
        if t.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p)) && t.as_bytes().get(p.len()) == Some(&b' ') {
            return t[p.len()..].trim();
        }
    }
    t
}

/// a value of an item in its canonical form: a hostname as it is, an address "10.0.1.1/24" (an
/// IPv6 one compressed), a state "up" / "down", a VLAN "20", a mode "access" / "trunk" /
/// "dynamic" / "routed", a route as RouteVal writes it; None: no such value
pub fn canon(kind: ItemKind, s: &str) -> Option<String> {
    let t = s.trim();
    Some(match kind {
        ItemKind::Hostname => {
            let h = strip_words(t, &["hostname"]);
            if !crate::project::valid_name(h) {
                return None;
            }
            String::from(h)
        }
        ItemKind::Address => {
            let a = strip_words(t, &["ip address", "ip addr add", "ipv6 address", "address"]);
            let a = a.split(" dev ").next().unwrap_or(a).trim();
            let w: Vec<&str> = a.split_whitespace().collect();
            match w.as_slice() {
                [x] if x.contains(':') => alloc::format!("{}", Prefix6::parse(x)?),
                [x] => alloc::format!("{}", Prefix4::parse(x).filter(|_| x.contains('/'))?),
                [x, m] => alloc::format!("{}", Prefix4::from_mask(Ipv4::parse(x)?, Ipv4::parse(m)?)?),
                _ => return None,
            }
        }
        ItemKind::State => match t.to_ascii_lowercase().as_str() {
            "up" | "on" | "no shutdown" | "no shut" | "enabled" | "an" | "ein" | "eingeschaltet" => String::from("up"),
            "down" | "off" | "shutdown" | "shut" | "disabled" | "aus" | "ausgeschaltet" => String::from("down"),
            _ => return None,
        },
        ItemKind::Vlan => {
            let v = strip_words(t, &["switchport access vlan"]);
            // "VLAN 20", "vlan20", "VLAN0020"
            let v = match v.get(..4) {
                Some(h) if h.eq_ignore_ascii_case("vlan") => v[4..].trim(),
                _ => v,
            };
            let n: u16 = v.parse().ok()?;
            if !(1..=4094).contains(&n) {
                return None;
            }
            alloc::format!("{}", n)
        }
        ItemKind::Mode => match strip_words(t, &["switchport mode"]).to_ascii_lowercase().as_str() {
            "access" => String::from("access"),
            "trunk" => String::from("trunk"),
            "dynamic" | "dynamic auto" | "auto" | "dynamic desirable" => String::from("dynamic"),
            "routed" | "no switchport" | "layer3" | "l3" => String::from("routed"),
            _ => return None,
        },
        ItemKind::Route => alloc::format!("{}", RouteVal::parse(t)?),
        _ => return None,
    })
}
