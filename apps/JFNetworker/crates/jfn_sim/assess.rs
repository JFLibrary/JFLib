// assess -- the grading of an assessment (jfn_model::lab::Assess, A1): its items against a
// project and the engine that runs it -- points, a percentage, per item whether it holds and
// what it found; for authors the network captured as a target, the target's settings as
// candidate items, and a start network made from the target without what the items check
//
//   Grader::new(&assess)                  the items prepared once (their checks parsed)
//   grader.grade(&project, &engine, out) -> Score   out(dev): what the device printed (the
//                                         checks' "output contains"); a config item grades a
//                                         running configuration once (kept until it changes)
//   Score { points, total, items }        percent() (rounded down), held()
//   Graded { held, points, max, found, verdicts }   found: what the item found when it does
//                                         not hold (a Msg for the app's catalogue); verdicts:
//                                         a check item's expectations as goal.rs judged them
//   item_text(&item) -> Msg               the default text of an item (German template)
//   item_key(&item) -> u64                what an item looks at (not what it expects), stable
//   config_pairs(text) -> Vec<(block, line)>   a configuration's lines with their blocks
//   would_change(model, running, lines) -> Option<Change>   what typing the lines at a copy of a
//                                         device with that running configuration would add and
//                                         take away (and which lines it refused)
//   normalise(model, lines) -> Lines      what the lines make on a fresh device of the model
//   capture(&project, &engine) -> Target  the network as it runs: each device with its running
//                                         configuration, the cables
//   candidates(&target_project, &engine) -> Vec<Cand>   the target's settings as the rows of a
//                                         tree: devices with their hostname, interfaces (address,
//                                         state, VLAN, mode), static routes, services and their
//                                         settings, the lines of the running configuration that
//                                         a fresh device of the model does not have; then the
//                                         cables
//   start_network(&target_project, &engine, &items) -> Project   the target with what the items
//                                         check taken away: addresses, routes, VLANs, modes and
//                                         lines out of the configurations, an interface an item
//                                         wants up shut down (and the other way round), settings
//                                         of services left out, cables removed, a hostname a
//                                         neutral one
//
// What the learner has: the engine for hostnames, interfaces and switch ports (the
// administrative state: "no shutdown"), static routes and the running configuration; the
// project for services and cables.  A device is the project's with the item's id, else the one
// with its name (a learner who renames the device keeps the item); the checks name devices as
// the target does, and a device the learner renamed is found through its id.  Values compare in
// canonical form (jfn_model::lab::canon); with Compare::Exact a value must be the text the
// engine writes ("10.0.1.1/24", a line of the running configuration).
//
// Config items: a device has what the lines say when typing them at its console would change
// nothing.  So a copy of the learner's device (a fresh device of its model booted with its
// running configuration) gets the lines typed, as a learner types them ("int g0/0/1", "ip add
// ...", "no shut"), and its running configuration must stay as it was; what would come is
// missing ("es fehlt"), what would go is too much ("zu viel": "no shutdown" on a port that is
// shut down).  Lines the copy refuses must be there as written; console words copied along
// (enable, configure terminal, end) are dropped.  The start network takes away what the lines
// make on a fresh device (normalise).
//
// Grading only reads: a check that probes reachability pings on a copy of the engine
// (goal.rs).  The same project and engine grade the same, on every platform.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use core::cell::RefCell;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use agent_core::json;
use jfn_model::catalog::{self, CliStyle, Model};
use jfn_model::ifname;
use jfn_model::lab::{canon, config_lines, Assess, Compare, Item, ItemKind, RouteVal, Target};
use jfn_model::msg;
use jfn_model::msg::{phrase, Msg};
use jfn_model::project::{Device, PortRef, Project};
use jfn_model::scenario::{self, Expect};
use jfn_model::CableKind;
use netpkt::{Ipv4, Prefix4};

use crate::bridge::{BridgePort, SwMode};
use crate::goal::{self, check_text, judge, Verdict};
use crate::ip::IfKind;
use crate::ip6::Origin6;
use crate::kernel::Engine;
use crate::view::IfView;

// ---------------------------------------------------------------- results

/// how one item did
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Graded {
    pub held: bool,
    /// the points it got (its points when it holds, else 0) and the ones it is worth
    pub points: u32,
    pub max: u32,
    /// what it found when it does not hold (empty when it holds)
    pub found: Msg,
    /// a check item: its expectations as goal.rs judged them now
    pub verdicts: Vec<Verdict>,
}

/// the grade of the whole assessment
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Score {
    pub points: u32,
    pub total: u32,
    pub items: Vec<Graded>,
}

impl Score {
    /// the points in per cent, rounded down (100 only when everything holds)
    pub fn percent(&self) -> u32 {
        if self.total == 0 { 0 } else { (self.points as u64 * 100 / self.total as u64) as u32 }
    }
    /// how many items hold
    pub fn held(&self) -> usize {
        self.items.iter().filter(|g| g.held).count()
    }
}

// ---------------------------------------------------------------- configurations

/// runs of spaces as one, no space around
fn collapse(s: &str) -> String {
    let mut out = String::new();
    for w in s.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

/// a configuration's lines as (block, line): a line indented under a block's first line
/// belongs to that block, every other line is its own (block ""); "!" lines (they end a block),
/// "end", comments and blank lines are left out, runs of spaces collapse
pub fn config_pairs(text: &str) -> Vec<(String, String)> {
    let mut v = Vec::new();
    let mut head = String::new();
    for raw in text.lines() {
        let indented = raw.starts_with(' ') || raw.starts_with('\t');
        let line = collapse(raw);
        if line.is_empty() || line.starts_with('!') || line.starts_with('#') || line.eq_ignore_ascii_case("end") {
            if !indented && !line.is_empty() {
                head.clear();
            }
            continue;
        }
        if indented && !head.is_empty() {
            v.push((head.clone(), line));
        } else {
            head = line.clone();
            v.push((String::new(), line));
        }
    }
    v
}

/// a (block, line) pair as a reason shows it
fn pair_text(p: &(String, String)) -> String {
    if p.0.is_empty() { p.1.clone() } else { format!("{} \u{203a} {}", p.0, p.1) }
}

/// what lines of configuration make on a fresh device, each as (block, line): `have` what its
/// running configuration has then more than an untouched one's, `lack` what it lost (the start
/// network takes `have` away and puts `lack` back)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lines {
    pub have: Vec<(String, String)>,
    pub lack: Vec<(String, String)>,
}

/// what typing lines at a copy of a device would change in its running configuration, each as
/// (block, line): what would come, what would go, the lines the copy refused (as written)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Change {
    pub add: Vec<(String, String)>,
    pub gone: Vec<(String, String)>,
    pub refused: Vec<(String, String)>,
}

impl Change {
    /// nothing would change: the device has what the lines say
    pub fn none(&self) -> bool {
        self.add.is_empty() && self.gone.is_empty()
    }
}

/// the name of the scratch devices that normalise lines
const NORM_NAME: &str = "JFN-NORM";

/// an engine with one device of the model that boots with `text`
fn scratch(model: &Model, text: &str) -> Option<Engine> {
    let mut p = Project::new("", 1);
    p.devices.push(Device { id: 1, name: String::from(NORM_NAME), model: String::from(model.id), startup_config: String::from(text), ..Device::default() });
    p.next_id = 2;
    Engine::from_project(&p).ok()
}

/// a word that moves between the modes of a console (a snippet copied from one may have them)
fn mode_word(line: &str) -> bool {
    let w = line.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    matches!(w.as_str(), "enable" | "disable" | "configure" | "conf" | "end" | "exit" | "write" | "copy")
}

/// the lines as they are written, with their blocks (console words left out)
fn literal(lines: &[String]) -> Vec<(String, String)> {
    let kept: Vec<&str> = lines.iter().map(String::as_str).filter(|l| !mode_word(l)).collect();
    config_pairs(&kept.join("\n"))
}

/// the (block, line) of line k of a snippet (its block: the last line before it that is not
/// indented)
fn pair_at(lines: &[String], k: usize) -> Option<(String, String)> {
    let raw = lines.get(k)?;
    let line = collapse(raw);
    if line.is_empty() || mode_word(&line) {
        return None;
    }
    if raw.starts_with(' ') || raw.starts_with('\t') {
        let head = lines[..k].iter().rev().find(|l| !l.starts_with(' ') && !l.starts_with('\t') && !collapse(l).is_empty())?;
        Some((collapse(head), line))
    } else {
        Some((String::new(), line))
    }
}

/// the running configuration of a fresh device of the model, as pairs (kept per model)
fn base_pairs(model: &'static Model, cache: &mut BTreeMap<&'static str, BTreeSet<(String, String)>>) -> BTreeSet<(String, String)> {
    cache.entry(model.id).or_insert_with(|| scratch(model, "").map(|e| config_pairs(&e.running_config(1)).into_iter().collect()).unwrap_or_default()).clone()
}

/// what lines of configuration want, normalised on a scratch device of the model: what its
/// running configuration has more than a fresh one's must be there, what it lost must be gone
/// (a line the device refuses stays as written)
pub fn normalise(model: &'static Model, lines: &[String]) -> Lines {
    normalise_with(model, lines, &mut BTreeMap::new())
}

/// a device of the model booted with `config`, with the lines typed at its console, as a
/// learner types them (IOS: in global configuration mode, a submode falls back to it); the
/// lines it refused
fn typed(model: &Model, config: &str, lines: &[String]) -> Option<(Engine, Vec<usize>)> {
    let mut e = scratch(model, config)?;
    let ni = e.node_index(1)?;
    if model.cli == CliStyle::Ios {
        let _ = e.cli_run(ni, "enable");
        let _ = e.cli_run(ni, "configure terminal");
    }
    let mut refused = Vec::new();
    for (k, l) in lines.iter().enumerate() {
        let t = l.trim();
        if t.is_empty() || t.starts_with('!') || t.starts_with('#') || mode_word(t) {
            continue;
        }
        if e.cli_run(ni, t).is_err() {
            refused.push(k);
        }
    }
    Some((e, refused))
}

fn normalise_with(model: &'static Model, lines: &[String], cache: &mut BTreeMap<&'static str, BTreeSet<(String, String)>>) -> Lines {
    let base = base_pairs(model, cache);
    let Some((e, refused)) = typed(model, "", lines) else {
        return Lines { have: literal(lines), ..Lines::default() };
    };
    let now: BTreeSet<(String, String)> = config_pairs(&e.running_config(1)).into_iter().collect();
    let mut have: Vec<(String, String)> = now.difference(&base).cloned().collect();
    // the scratch device's own name is no part of the item
    let lack: Vec<(String, String)> = base.difference(&now).filter(|p| !(p.0.is_empty() && p.1.starts_with("hostname "))).cloned().collect();
    for k in refused {
        if let Some(p) = pair_at(lines, k)
            && !have.contains(&p)
        {
            have.push(p);
        }
    }
    Lines { have, lack }
}

/// what typing the lines at a copy of a device with this running configuration would change
/// (None: no copy of the model boots)
pub fn would_change(model: &Model, running: &str, lines: &[String]) -> Option<Change> {
    let before: BTreeSet<(String, String)> = scratch(model, running).map(|e| config_pairs(&e.running_config(1)).into_iter().collect())?;
    let (e, refused) = typed(model, running, lines)?;
    let after: BTreeSet<(String, String)> = config_pairs(&e.running_config(1)).into_iter().collect();
    Some(Change {
        add: after.difference(&before).cloned().collect(),
        gone: before.difference(&after).cloned().collect(),
        refused: refused.into_iter().filter_map(|k| pair_at(lines, k)).collect(),
    })
}

// ---------------------------------------------------------------- the grader

/// an assessment's items, ready to grade a project and its engine
#[derive(Clone, Debug)]
pub struct Grader {
    items: Vec<Item>,
    /// per item (check items): its expectations; Err: the first that does not parse
    checks: Vec<Result<Vec<Expect>, (usize, String)>>,
    /// the target's devices: lower-case name -> id (the checks name them so)
    names: BTreeMap<String, u32>,
    /// config items: (item, a hash of the device's running configuration) -> held, found (a
    /// configuration that did not change grades the same)
    seen: RefCell<BTreeMap<(usize, u64), (bool, Msg)>>,
}

/// config grades a Grader keeps at most (then it starts again)
const SEEN_MAX: usize = 512;

/// the project's device an item names: by id when the project has it, else by name
fn find_dev(p: &Project, e: &Engine, id: u32, name: &str) -> Option<u32> {
    if id != 0 && p.device(id).is_some() {
        return Some(id);
    }
    if name.trim().is_empty() {
        return None;
    }
    goal::resolve(p, e, name.trim())
}

impl Grader {
    pub fn new(a: &Assess) -> Grader {
        let names = a.target.devices.iter().map(|d| (d.name.to_ascii_lowercase(), d.id)).collect();
        let checks = a.items.iter().map(|it| if it.kind() == Some(ItemKind::Check) { it.check_list() } else { Ok(Vec::new()) }).collect();
        Grader { items: a.items.clone(), checks, names, seen: RefCell::new(BTreeMap::new()) }
    }

    /// the items it grades
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// the points there are
    pub fn total(&self) -> u32 {
        self.items.iter().map(|i| i.points).sum()
    }

    /// every item against the project and its engine now
    pub fn grade(&self, p: &Project, e: &Engine, out: &dyn Fn(u32) -> String) -> Score {
        let mut s = Score { points: 0, total: self.total(), items: Vec::with_capacity(self.items.len()) };
        for k in 0..self.items.len() {
            let mut g = self.grade_item(k, p, e, out);
            g.max = self.items[k].points;
            g.points = if g.held { g.max } else { 0 };
            if g.held {
                g.found = Msg::default();
            }
            s.points += g.points;
            s.items.push(g);
        }
        s
    }

    // an expectation of a check with its device as the project names it now: a device the
    // target knows by that name and the learner renamed is found through its id
    fn check_now(&self, p: &Project, e: &Engine, x: &Expect) -> Expect {
        let Some(d) = x.dev() else { return x.clone() };
        if goal::resolve(p, e, d).is_some() {
            return x.clone();
        }
        let Some(now) = self.names.get(&d.to_ascii_lowercase()).and_then(|&id| p.device(id)) else {
            return x.clone();
        };
        let t = check_text(x);
        let rest = t.split_once(' ').map_or("", |r| r.1);
        scenario::parse_expectation(&format!("{} {}", now.name, rest)).unwrap_or_else(|_| x.clone())
    }

    fn grade_item(&self, k: usize, p: &Project, e: &Engine, out: &dyn Fn(u32) -> String) -> Graded {
        let it = &self.items[k];
        let no = |found: Msg| Graded { held: false, found, ..Graded::default() };
        let Some(kind) = it.kind() else {
            return no(msg!("unbekannte Art {}", it.what));
        };
        if kind == ItemKind::Check {
            return self.grade_check(k, p, e, out);
        }
        let Some(dev) = find_dev(p, e, it.id, &it.dev) else {
            return no(msg!("das Gerät {} gibt es nicht", it.dev_name()));
        };
        let held = |ok: bool, found: Msg| Graded { held: ok, found, ..Graded::default() };
        let texts: Vec<&str> = it.values().into_iter().filter_map(Value::as_str).collect();
        // a text value: the same canonical form (Exact: the same text)
        let text_ok = |have: &str| {
            texts.iter().any(|w| match it.cmp {
                Compare::Exact => w.trim() == have,
                _ => canon(kind, w).is_some_and(|c| if kind == ItemKind::Hostname { c.eq_ignore_ascii_case(have) } else { c == have }),
            })
        };
        match kind {
            ItemKind::Hostname => {
                let name = e.node(dev).map(|n| n.name.clone()).or_else(|| p.device(dev).map(|d| d.name.clone())).unwrap_or_default();
                held(text_ok(&name), msg!("heißt {}", name))
            }
            ItemKind::Address => {
                let ifs = e.ifaces(dev);
                let Some(f) = ifs.iter().find(|f| ifname::same(&f.name, &it.port)) else {
                    return no(msg!("kein Interface {}", it.port));
                };
                let mut have: Vec<String> = f.addrs.iter().map(|a| format!("{}", a)).collect();
                have.extend(f.addrs6.iter().filter(|a| a.origin != Origin6::LinkLocal).map(|a| format!("{}/{}", a.addr, a.len)));
                let ok = have.iter().any(|h| text_ok(h));
                match have.first() {
                    None => held(false, msg!("hat keine Adresse")),
                    Some(first) => held(ok, msg!("hat {}", first)),
                }
            }
            ItemKind::State => match admin_state(e, dev, &it.port) {
                None => no(msg!("kein Interface {}", it.port)),
                Some(up) => held(text_ok(if up { "up" } else { "down" }), if up { msg!("ist eingeschaltet") } else { msg!("ist ausgeschaltet (shutdown)") }),
            },
            ItemKind::Vlan => match switchport(e, dev, &it.port) {
                None => no(msg!("{} ist kein Switchport", ifname::short(&it.port))),
                Some(b) => held(text_ok(&format!("{}", b.access_vlan)), msg!("im VLAN {}", b.access_vlan)),
            },
            ItemKind::Mode => match port_mode(e, dev, &it.port) {
                None => no(msg!("kein Interface {}", it.port)),
                Some(m) => held(text_ok(m), msg!("im Modus {}", m)),
            },
            ItemKind::Route => {
                let mine = statics(e, dev);
                let ok = it.values().into_iter().filter_map(Value::as_str).any(|w| match it.cmp {
                    Compare::Exact => mine.iter().any(|(r, _)| format!("{}", r) == w.trim()),
                    _ => RouteVal::parse(w).is_some_and(|x| mine.iter().any(|(r, ad)| route_like(&x, r, *ad))),
                });
                let want = texts.first().and_then(|w| RouteVal::parse(w)).map(|x| x.prefix);
                let same: Vec<String> = mine.iter().filter(|(r, _)| Some(r.prefix) == want).map(|(r, _)| format!("{}", r)).collect();
                let found = match (&want, same.is_empty()) {
                    (Some(px), true) => msg!("keine statische Route nach {}", prefix_text(*px)),
                    _ => msg!("statt dessen: {}", same.join(", ")),
                };
                held(ok, found)
            }
            ItemKind::Service => {
                let Some(def) = p.device(dev).and_then(|d| d.services.iter().find(|s| s.name.eq_ignore_ascii_case(it.svc.trim())).or_else(|| d.services.iter().find(|s| s.kind.eq_ignore_ascii_case(it.svc.trim())))) else {
                    return no(msg!("kein Dienst {}", it.svc));
                };
                let Some(have) = at_path(&def.config, &it.key) else {
                    return no(msg!("keine Einstellung {}", it.key));
                };
                let ok = it.values().into_iter().any(|w| match it.cmp {
                    Compare::Exact => json::stringify(w).ok() == json::stringify(have).ok(),
                    _ => json_like(w, have),
                });
                held(ok, msg!("ist {}", short_json(have)))
            }
            ItemKind::Cable => self.grade_cable(it, dev, p, e),
            ItemKind::Config => {
                let run = e.running_config(dev);
                let key = (k, fnv(&[&run]));
                if let Some((ok, found)) = self.seen.borrow().get(&key) {
                    return held(*ok, found.clone());
                }
                let (ok, found) = config_holds(it, e.node(dev).map(|n| n.model), &run);
                let mut seen = self.seen.borrow_mut();
                if seen.len() >= SEEN_MAX {
                    seen.clear();
                }
                seen.insert(key, (ok, found.clone()));
                held(ok, found)
            }
            ItemKind::Check => held(false, Msg::default()),
        }
    }

    fn grade_cable(&self, it: &Item, dev: u32, p: &Project, e: &Engine) -> Graded {
        let no = |found: Msg| Graded { held: false, found, ..Graded::default() };
        let Some(peer) = find_dev(p, e, it.peer_id, &it.peer) else {
            return no(msg!("das Gerät {} gibt es nicht", it.peer));
        };
        let end = |r: &PortRef, d: u32, port: &str| r.dev == d && (port.trim().is_empty() || ifname::same(&r.port, port));
        let link = p.links.iter().find(|l| (end(&l.a, dev, &it.port) && end(&l.b, peer, &it.peer_port)) || (end(&l.b, dev, &it.port) && end(&l.a, peer, &it.peer_port)));
        let name = |d: u32| p.device(d).map_or_else(String::new, |x| x.name.clone());
        let Some(l) = link else {
            // what that port has instead
            let other = if it.port.trim().is_empty() { None } else { p.link_at(dev, &it.port) };
            return match other {
                Some(o) => {
                    let far = if o.a.dev == dev && ifname::same(&o.a.port, &it.port) { &o.b } else { &o.a };
                    no(msg!("{} {} hängt an {} {}", name(dev), ifname::short(&it.port), name(far.dev), ifname::short(&far.port)))
                }
                None => no(msg!("nicht verbunden")),
            };
        };
        let kinds: Vec<&str> = it.values().into_iter().filter_map(Value::as_str).collect();
        let ok = kinds.is_empty()
            || kinds.iter().any(|k| match it.cmp {
                Compare::Exact => k.trim() == l.cable.as_str(),
                _ => CableKind::parse(k.trim()) == Some(l.cable),
            });
        Graded { held: ok, found: msg!("falsches Kabel: {}", phrase(l.cable.title())), ..Graded::default() }
    }

    fn grade_check(&self, k: usize, p: &Project, e: &Engine, out: &dyn Fn(u32) -> String) -> Graded {
        let xs = match &self.checks[k] {
            Ok(xs) => xs,
            Err((i, why)) => return Graded { held: false, found: msg!("Prüfung {} ist fehlerhaft: {}", i + 1, why), ..Graded::default() },
        };
        let verdicts: Vec<Verdict> = xs.iter().map(|x| judge(p, e, &self.check_now(p, e, x), out)).collect();
        let held = !verdicts.is_empty() && verdicts.iter().all(|v| v.held);
        let found = match verdicts.iter().find(|v| !v.held) {
            None if verdicts.is_empty() => msg!("keine Prüfung"),
            None => Msg::default(),
            Some(v) => {
                let r = v.reason();
                if r.is_empty() { Msg::plain(check_text(&v.x)) } else { msg!("{}: {}", check_text(&v.x), r) }
            }
        };
        Graded { held, found, verdicts, ..Graded::default() }
    }
}

/// whether a device with this running configuration has what a config item's lines say: typed
/// at a copy of it they change nothing (Exact: the lines are there as written); what is missing
/// or too much
fn config_holds(it: &Item, model: Option<&'static Model>, run: &str) -> (bool, Msg) {
    let mine: BTreeSet<(String, String)> = config_pairs(run).into_iter().collect();
    let mut found = Msg::default();
    for v in it.values() {
        let lines = config_lines(v);
        let (missing, extra) = match (it.cmp, model.and_then(|m| would_change(m, run, &lines))) {
            (Compare::Exact, _) | (_, None) => (literal(&lines).into_iter().find(|x| !mine.contains(x)), None),
            (_, Some(c)) => (c.add.first().cloned().or_else(|| c.refused.into_iter().find(|x| !mine.contains(x))), c.gone.first().cloned()),
        };
        match (missing, extra) {
            (None, None) => return (true, Msg::default()),
            (Some(m), _) if found.is_empty() => found = msg!("es fehlt: {}", pair_text(&m)),
            (None, Some(x)) if found.is_empty() => found = msg!("zu viel: {}", pair_text(&x)),
            _ => {}
        }
    }
    (false, found)
}

// ---------------------------------------------------------------- what the learner has

/// "not shut down" of an interface or port (None: there is none of that name)
fn admin_state(e: &Engine, dev: u32, port: &str) -> Option<bool> {
    if let Some(f) = e.ifaces(dev).into_iter().find(|f| ifname::same(&f.name, port)) {
        return Some(f.admin_up);
    }
    let n = e.node(dev)?;
    let i = n.model.port(port)?.index;
    n.ports.get(i as usize).map(|p| p.admin_up)
}

/// the bridge port of a switch port (None: no bridge, or a port outside it)
fn switchport<'a>(e: &'a Engine, dev: u32, port: &str) -> Option<&'a BridgePort> {
    let i = e.port_index(dev, port)?;
    e.bridge(dev)?.ports.get(i as usize).filter(|b| b.member)
}

/// "access", "trunk", "dynamic" for a switch port, "routed" for a port with an IP interface
fn port_mode(e: &Engine, dev: u32, port: &str) -> Option<&'static str> {
    if let Some(b) = switchport(e, dev, port) {
        return Some(match b.admin {
            SwMode::Access => "access",
            SwMode::Trunk => "trunk",
            SwMode::Dynamic => "dynamic",
        });
    }
    let routed = e.ifaces(dev).iter().any(|f| matches!(f.kind, IfKind::Port(_)) && ifname::same(&f.name, port));
    if routed { Some("routed") } else { None }
}

/// the static routes of a device (the distance only when it is not 1) with their distance
fn statics(e: &Engine, dev: u32) -> Vec<(RouteVal, u8)> {
    let Some(st) = e.node(dev).and_then(|n| n.ip.as_ref()) else {
        return Vec::new();
    };
    st.statics
        .iter()
        .map(|r| {
            let d = r.iface.and_then(|i| st.ifaces.get(i as usize)).map(|f| f.name.clone());
            (RouteVal { prefix: r.prefix.trunc(), via: r.via, dev: d, ad: if r.ad != 1 { Some(r.ad) } else { None } }, r.ad)
        })
        .collect()
}

/// a route an item asks for matches one the device has (what the item leaves out may be
/// anything)
fn route_like(want: &RouteVal, have: &RouteVal, ad: u8) -> bool {
    want.prefix == have.prefix
        && want.via.is_none_or(|v| have.via == Some(v))
        && want.dev.as_ref().is_none_or(|d| have.dev.as_ref().is_some_and(|h| ifname::same(h, d)))
        && want.ad.is_none_or(|a| a == ad)
}

/// a setting of a service by its path: members by name, list entries by number ("pools.0.range")
fn at_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.').map(str::trim).filter(|s| !s.is_empty()) {
        cur = match cur {
            Value::Object(_) => cur.get(seg)?,
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// a JSON value the learner's equals the one an item wants: an object's members as wanted (more
/// members are fine), a list as long and in order (a list of objects in any order), numbers by
/// value, strings without the spaces around
fn json_like(want: &Value, have: &Value) -> bool {
    match (want, have) {
        (Value::Object(w), Value::Object(_)) => w.iter().all(|(k, x)| have.get(k).is_some_and(|y| json_like(x, y))),
        (Value::Array(w), Value::Array(h)) => {
            if w.len() != h.len() {
                return false;
            }
            if !w.is_empty() && w.iter().all(|x| matches!(x, Value::Object(_))) {
                let mut used = vec![false; h.len()];
                w.iter().all(|x| match (0..h.len()).find(|&k| !used[k] && json_like(x, &h[k])) {
                    Some(k) => {
                        used[k] = true;
                        true
                    }
                    None => false,
                })
            } else {
                w.iter().zip(h.iter()).all(|(x, y)| json_like(x, y))
            }
        }
        (Value::Number(a), Value::Number(b)) => a == b || (a.as_i64().is_some() && a.as_i64() == b.as_i64()),
        (Value::String(a), Value::String(b)) => a.trim() == b.trim(),
        (Value::Number(n), Value::String(s)) | (Value::String(s), Value::Number(n)) => n.as_str() == s.trim(),
        (a, b) => a == b,
    }
}

/// a JSON value in one line, cut after 60 characters
fn short_json(v: &Value) -> String {
    let s = json::stringify(v).unwrap_or_default();
    if s.chars().count() <= 60 {
        return s;
    }
    let mut t: String = s.chars().take(59).collect();
    t.push('\u{2026}');
    t
}

// ---------------------------------------------------------------- texts and keys

/// a value for a text (the canonical form when it has one)
fn value_text(kind: ItemKind, v: &Value) -> String {
    match v {
        Value::String(s) => canon(kind, s).unwrap_or_else(|| String::from(s.trim())),
        Value::Null => String::new(),
        other => short_json(other),
    }
}

/// the default text of an item (German; the app's catalogue has the English)
pub fn item_text(it: &Item) -> Msg {
    let Some(kind) = it.kind() else {
        return Msg::plain(it.what.clone());
    };
    let dev = it.dev_name();
    let port = ifname::short(&it.port);
    let val = it.values().first().map(|v| value_text(kind, v)).unwrap_or_default();
    match kind {
        ItemKind::Hostname => msg!("{} heißt {}", dev, val),
        ItemKind::Address => msg!("{} {}: Adresse {}", dev, port, val),
        ItemKind::State if val == "down" => msg!("{} {} ist ausgeschaltet", dev, port),
        ItemKind::State => msg!("{} {} ist eingeschaltet", dev, port),
        ItemKind::Vlan => msg!("{} {} im VLAN {}", dev, port, val),
        ItemKind::Mode => msg!("{} {} im Modus {}", dev, port, val),
        ItemKind::Route => msg!("{}: statische Route {}", dev, val),
        ItemKind::Service => msg!("{}: {} {} = {}", dev, it.svc, it.key, val),
        ItemKind::Cable if it.port.is_empty() || it.peer_port.is_empty() => msg!("Kabel zwischen {} und {}", dev, it.peer),
        ItemKind::Cable => msg!("Kabel {} {} \u{2013} {} {}", dev, port, it.peer, ifname::short(&it.peer_port)),
        ItemKind::Config => {
            let lines: Vec<String> = it.values().first().map(|v| config_lines(v)).unwrap_or_default().iter().map(|l| collapse(l)).filter(|l| !l.is_empty()).collect();
            let mut s = lines.join(" \u{b7} ");
            if s.chars().count() > 90 {
                s = s.chars().take(89).collect();
                s.push('\u{2026}');
            }
            msg!("{}: {}", dev, s)
        }
        ItemKind::Check => match it.check_list().ok().as_deref() {
            Some([Expect::Reach { dev, host, negate: false }]) => msg!("{} erreicht {}", dev, host),
            Some([Expect::Reach { dev, host, negate: true }]) => msg!("{} erreicht {} nicht", dev, host),
            _ => Msg::plain(it.checks.join("; ")),
        },
    }
}

/// FNV-1a over the parts of an identity
fn fnv(parts: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        for b in p.bytes().chain(core::iter::once(0)) {
            h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

/// a port name in its long form (for keys: "gi0/0/0" and "GigabitEthernet0/0/0" are one)
fn long(port: &str) -> String {
    let p = port.trim();
    ifname::expand(p).unwrap_or_else(|| String::from(p)).to_ascii_lowercase()
}

/// what an item looks at, as a number (two items with the same key grade the same thing; the
/// value and the points do not count)
pub fn item_key(it: &Item) -> u64 {
    let id = format!("{}", it.id);
    let dev = if it.id != 0 { id } else { it.dev.to_ascii_lowercase() };
    let kind = it.what.trim();
    match it.kind() {
        Some(ItemKind::Route) => fnv(&[kind, &dev, &it.values().first().map(|v| value_text(ItemKind::Route, v)).unwrap_or_default()]),
        Some(ItemKind::Service) => fnv(&[kind, &dev, &it.svc.to_ascii_lowercase(), it.key.trim()]),
        Some(ItemKind::Cable) => {
            let peer = if it.peer_id != 0 { format!("{}", it.peer_id) } else { it.peer.to_ascii_lowercase() };
            let (a, b) = ((dev, long(&it.port)), (peer, long(&it.peer_port)));
            let (x, y) = if a <= b { (a, b) } else { (b, a) };
            fnv(&[kind, &x.0, &x.1, &y.0, &y.1])
        }
        Some(ItemKind::Config) => {
            let lines: Vec<String> = it.values().first().map(|v| config_lines(v)).unwrap_or_default().iter().map(|l| collapse(l)).collect();
            fnv(&[kind, &dev, &lines.join("\n")])
        }
        Some(ItemKind::Check) => fnv(&[kind, &it.checks.join("\n")]),
        _ => fnv(&[kind, &dev, &long(&it.port)]),
    }
}

// ---------------------------------------------------------------- for authors

/// the network as it runs: each device with its running configuration (as startup
/// configuration; a device without a command line keeps its own), the cables
pub fn capture(p: &Project, e: &Engine) -> Target {
    let devices = p
        .devices
        .iter()
        .map(|d| {
            let run = e.running_config(d.id);
            Device { startup_config: if run.trim().is_empty() { d.startup_config.clone() } else { run }, ..d.clone() }
        })
        .collect();
    Target { devices, links: p.links.clone() }
}

/// a row of the tree an author picks items from: a group (a device, an interface, "Kabel" ...)
/// or a candidate item
#[derive(Clone, Debug, PartialEq)]
pub struct Cand {
    /// item_key of its item, or a key of its own for a group
    pub key: u64,
    pub depth: u16,
    /// the label (German; the app translates it)
    pub label: Msg,
    /// None: a group
    pub item: Option<Item>,
}

/// an item of the target's device d with the target's name and id
fn cand_item(d: &Device, kind: ItemKind, port: &str, value: Value) -> Item {
    Item { what: String::from(kind.as_str()), dev: d.name.clone(), id: d.id, port: String::from(port), value, points: 1, ..Item::default() }
}

fn group(key: &[&str], depth: u16, label: Msg) -> Cand {
    let mut k = Vec::with_capacity(key.len() + 1);
    k.push("group");
    k.extend_from_slice(key);
    Cand { key: fnv(&k), depth, label, item: None }
}

fn row(depth: u16, label: Msg, it: Item) -> Cand {
    Cand { key: item_key(&it), depth, label, item: Some(it) }
}

/// the interfaces of a device worth a row: ports with a cable or a setting, every interface
/// with an address, loopbacks, VLAN interfaces and subinterfaces (in that order: the ports by
/// number, then the others)
fn listed_ifaces(p: &Project, e: &Engine, d: &Device) -> Vec<(String, Option<IfView>, Option<BridgePort>, bool)> {
    let Some(n) = e.node(d.id) else {
        return Vec::new();
    };
    let ifs = e.ifaces(d.id);
    let mut v = Vec::new();
    for (i, port) in n.ports.iter().enumerate() {
        let name = &port.spec.name;
        let f = ifs.iter().find(|f| f.kind == IfKind::Port(i as u16)).cloned();
        let b = n.bridge.as_ref().and_then(|b| b.ports.get(i)).filter(|b| b.member).cloned();
        let linked = p.link_at(d.id, name).is_some();
        let set = f.as_ref().is_some_and(|f| !f.addrs.is_empty())
            || b.as_ref().is_some_and(|b| b.access_vlan != 1 || b.admin != SwMode::Dynamic)
            || (!port.admin_up && b.is_some());
        if (linked || set) && (f.is_some() || b.is_some()) {
            v.push((name.clone(), f, b, port.admin_up));
        }
    }
    for f in ifs.iter().filter(|f| !matches!(f.kind, IfKind::Port(_))) {
        if f.name == "lo" && f.addrs.iter().all(|a| a.addr == Ipv4::LOCALHOST) {
            continue;
        }
        v.push((f.name.clone(), Some(f.clone()), None, f.admin_up));
    }
    v
}

/// the target's settings as the rows of a tree, each row a candidate item or a group (see the
/// module comment); depth 0 the devices and "Kabel"
pub fn candidates(p: &Project, e: &Engine) -> Vec<Cand> {
    let mut v = Vec::new();
    for d in &p.devices {
        let ids = format!("{}", d.id);
        let m = catalog::model(&d.model);
        v.push(group(&["dev", &ids], 0, Msg::plain(d.name.clone())));
        v.push(row(1, msg!("Hostname {}", d.name), cand_item(d, ItemKind::Hostname, "", Value::string(d.name.as_str()))));
        // interfaces
        for (name, f, b, up) in listed_ifaces(p, e, d) {
            v.push(group(&["if", &ids, &long(&name)], 1, Msg::plain(ifname::short(&name))));
            if let Some(f) = &f {
                for a in &f.addrs {
                    let t = format!("{}", a);
                    v.push(row(2, msg!("Adresse {}", t), cand_item(d, ItemKind::Address, &name, Value::string(t.as_str()))));
                }
                for a in f.addrs6.iter().filter(|a| a.origin != Origin6::LinkLocal) {
                    let t = format!("{}/{}", a.addr, a.len);
                    v.push(row(2, msg!("Adresse {}", t), cand_item(d, ItemKind::Address, &name, Value::string(t.as_str()))));
                }
            }
            let (label, val) = if up { (msg!("eingeschaltet"), "up") } else { (msg!("ausgeschaltet (shutdown)"), "down") };
            v.push(row(2, label, cand_item(d, ItemKind::State, &name, Value::string(val))));
            if let Some(b) = &b {
                if b.admin != SwMode::Trunk {
                    let t = format!("{}", b.access_vlan);
                    v.push(row(2, msg!("VLAN {}", t), cand_item(d, ItemKind::Vlan, &name, Value::string(t.as_str()))));
                }
                let mode = match b.admin {
                    SwMode::Access => "access",
                    SwMode::Trunk => "trunk",
                    SwMode::Dynamic => "dynamic",
                };
                v.push(row(2, msg!("Modus {}", mode), cand_item(d, ItemKind::Mode, &name, Value::string(mode))));
            } else if f.as_ref().is_some_and(|f| matches!(f.kind, IfKind::Port(_))) && m.is_some_and(|m| m.has(catalog::F_BRIDGE)) {
                v.push(row(2, msg!("Modus {}", "routed"), cand_item(d, ItemKind::Mode, &name, Value::string("routed"))));
            }
        }
        // static routes
        let routes = statics(e, d.id);
        if !routes.is_empty() {
            v.push(group(&["routes", &ids], 1, msg!("Statische Routen")));
            for (r, _) in routes {
                let t = format!("{}", r);
                v.push(row(2, Msg::plain(t.clone()), cand_item(d, ItemKind::Route, "", Value::string(t.as_str()))));
            }
        }
        // services and their settings
        if !d.services.is_empty() {
            v.push(group(&["svcs", &ids], 1, msg!("Dienste")));
            for s in &d.services {
                v.push(group(&["svc", &ids, &s.name], 2, Msg::plain(format!("{} ({})", s.name, s.kind))));
                for (k, x) in s.config.as_object().unwrap_or(&[]) {
                    if s.kind == "script" && k == "source" {
                        continue;
                    }
                    let it = Item { what: String::from(ItemKind::Service.as_str()), dev: d.name.clone(), id: d.id, svc: s.name.clone(), key: k.clone(), value: x.clone(), points: 1, ..Item::default() };
                    v.push(row(3, Msg::plain(format!("{} = {}", k, short_json(x))), it));
                }
            }
        }
        // the running configuration: what a fresh device of the model does not have, blocks
        // with their lines and single lines
        let run = e.running_config(d.id);
        let pairs = config_pairs(&run);
        let base: BTreeSet<(String, String)> = m.and_then(|m| scratch(m, "")).map(|x| config_pairs(&x.running_config(1)).into_iter().collect()).unwrap_or_default();
        let mut rows = Vec::new();
        let mut k = 0;
        while k < pairs.len() {
            let head = &pairs[k].1;
            let all: Vec<&String> = pairs[k + 1..].iter().take_while(|p| &p.0 == head).map(|p| &p.1).collect();
            k += 1 + all.len();
            if head.starts_with("version ") || head.starts_with("hostname ") {
                continue;
            }
            let body: Vec<&String> = all.into_iter().filter(|l| !base.contains(&(head.clone(), (*l).clone()))).collect();
            let config = |lines: Vec<Value>| Item { what: String::from(ItemKind::Config.as_str()), dev: d.name.clone(), id: d.id, value: Value::Array(lines), points: 1, ..Item::default() };
            if !body.is_empty() {
                let mut lines = vec![Value::string(head.as_str())];
                lines.extend(body.iter().map(|l| Value::string(format!(" {}", l))));
                rows.push(row(2, Msg::plain(head.clone()), config(lines)));
                // one line: the block's row is its row
                if body.len() > 1 {
                    for l in &body {
                        rows.push(row(3, Msg::plain((*l).clone()), config(vec![Value::string(head.as_str()), Value::string(format!(" {}", l))])));
                    }
                }
            } else if !base.contains(&(String::new(), head.clone())) {
                rows.push(row(2, Msg::plain(head.clone()), config(vec![Value::string(head.as_str())])));
            }
        }
        if !rows.is_empty() {
            let ios = m.is_some_and(|m| m.cli == CliStyle::Ios);
            v.push(group(&["cfg", &ids], 1, if ios { msg!("Konfiguration (IOS-artig)") } else { msg!("Konfiguration") }));
            v.extend(rows);
        }
    }
    if !p.links.is_empty() {
        v.push(group(&["cables"], 0, msg!("Kabel")));
        for l in &p.links {
            let (Some(a), Some(b)) = (p.device(l.a.dev), p.device(l.b.dev)) else { continue };
            let it = Item {
                what: String::from(ItemKind::Cable.as_str()),
                dev: a.name.clone(),
                id: a.id,
                port: l.a.port.clone(),
                peer: b.name.clone(),
                peer_id: b.id,
                peer_port: l.b.port.clone(),
                points: 1,
                ..Item::default()
            };
            let label = format!("{} {} \u{2013} {} {}", a.name, ifname::short(&l.a.port), b.name, ifname::short(&l.b.port));
            v.push(row(1, Msg::plain(label), it));
        }
    }
    v
}

// ---------------------------------------------------------------- the start network

/// a configuration as lines, to take things out and put them in (IOS blocks: a line that is
/// not indented and the indented lines after it)
struct CfgText {
    lines: Vec<String>,
}

impl CfgText {
    fn parse(t: &str) -> CfgText {
        CfgText { lines: t.lines().map(String::from).collect() }
    }
    fn text(&self) -> String {
        let mut s = String::new();
        for l in &self.lines {
            s.push_str(l);
            s.push('\n');
        }
        s
    }
    /// the index of the block whose first line `head` accepts, and the end of its lines
    fn block(&self, head: &dyn Fn(&str) -> bool) -> Option<(usize, usize)> {
        let i = self.lines.iter().position(|l| !l.starts_with(' ') && head(&collapse(l)))?;
        let mut j = i + 1;
        while j < self.lines.len() && self.lines[j].starts_with(' ') {
            j += 1;
        }
        Some((i, j))
    }
    /// takes the lines of a block that `pred` accepts out
    fn remove_in(&mut self, head: &dyn Fn(&str) -> bool, pred: &dyn Fn(&str) -> bool) {
        let Some((i, j)) = self.block(head) else { return };
        let mut k = j;
        while k > i + 1 {
            k -= 1;
            if pred(&collapse(&self.lines[k])) {
                self.lines.remove(k);
            }
        }
    }
    /// puts a line at the end of a block
    fn add_in(&mut self, head: &dyn Fn(&str) -> bool, line: &str) {
        if let Some((_, j)) = self.block(head) {
            self.lines.insert(j, format!(" {}", line.trim()));
        }
    }
    /// puts a line that is not indented before the "end" of the configuration (at its end
    /// without one)
    fn add_top(&mut self, line: &str) {
        let at = self.lines.iter().rposition(|l| l.trim().eq_ignore_ascii_case("end")).unwrap_or(self.lines.len());
        self.lines.insert(at, String::from(line.trim()));
    }
    /// takes the lines that are not indented and that `pred` accepts out, a block with its lines
    fn remove_top(&mut self, pred: &dyn Fn(&str) -> bool) {
        let mut k = 0;
        while k < self.lines.len() {
            if !self.lines[k].starts_with(' ') && pred(&collapse(&self.lines[k])) {
                self.lines.remove(k);
                while k < self.lines.len() && self.lines[k].starts_with(' ') {
                    self.lines.remove(k);
                }
            } else {
                k += 1;
            }
        }
    }
}

/// the block of an interface ("interface GigabitEthernet0/0/0" for "gi0/0/0")
fn if_head(port: &str) -> impl Fn(&str) -> bool + '_ {
    move |h: &str| h.strip_prefix("interface ").is_some_and(|n| ifname::same(n, port))
}

/// a name for a device that is not `avoid`: its model's name and the lowest free number
fn neutral_name(p: &Project, model: Option<&'static Model>, avoid: &str) -> String {
    // the model's name as a factory would set it ("JF-R4-1")
    let name: String = model.map_or("Device", |m| m.name).chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    let prefix = if name.as_bytes().first().is_some_and(|c| c.is_ascii_alphabetic()) { name } else { String::from("Device") };
    let mut k = 1;
    loop {
        let n = format!("{}-{}", prefix, k);
        if p.find(&n).is_none() && !n.eq_ignore_ascii_case(avoid) {
            return n;
        }
        k += 1;
    }
}

/// takes what an item checks out of the start network (one device's configuration, a cable, a
/// setting, a name)
fn strip(p: &mut Project, it: &Item) {
    let Some(kind) = it.kind() else { return };
    if kind == ItemKind::Check {
        return;
    }
    if kind == ItemKind::Cable {
        let end = |r: &PortRef, d: u32, port: &str| r.dev == d && (port.trim().is_empty() || ifname::same(&r.port, port));
        let (a, b) = (it.id, it.peer_id);
        p.links.retain(|l| !((end(&l.a, a, &it.port) && end(&l.b, b, &it.peer_port)) || (end(&l.b, a, &it.port) && end(&l.a, b, &it.peer_port))));
        return;
    }
    let Some(ix) = p.devices.iter().position(|d| (it.id != 0 && d.id == it.id) || (it.id == 0 && d.name.eq_ignore_ascii_case(it.dev.trim()))) else { return };
    let model = catalog::model(&p.devices[ix].model);
    let ios = model.is_some_and(|m| m.cli == CliStyle::Ios);
    if kind == ItemKind::Hostname {
        let name = neutral_name(p, model, &it.dev);
        let d = &mut p.devices[ix];
        let mut c = CfgText::parse(&d.startup_config);
        for l in c.lines.iter_mut() {
            if !l.starts_with(' ') && l.trim_start().starts_with("hostname ") {
                *l = format!("hostname {}", name);
            }
        }
        d.startup_config = c.text();
        d.name = name;
        return;
    }
    if kind == ItemKind::Service {
        let d = &mut p.devices[ix];
        if let Some(s) = d.services.iter_mut().find(|s| s.name.eq_ignore_ascii_case(it.svc.trim()) || s.kind.eq_ignore_ascii_case(it.svc.trim())) {
            remove_path(&mut s.config, &it.key);
        }
        return;
    }
    let d = &mut p.devices[ix];
    let mut c = CfgText::parse(&d.startup_config);
    let first = it.values().first().and_then(|v| v.as_str()).and_then(|s| canon(kind, s));
    let port = it.port.trim();
    match kind {
        ItemKind::Address if ios => {
            let want = first.unwrap_or_default();
            c.remove_in(&if_head(port), &|l: &str| {
                let a = l.strip_prefix("ip address ").or_else(|| l.strip_prefix("ipv6 address "));
                a.is_some_and(|a| canon(ItemKind::Address, a).is_some_and(|x| x == want))
            });
        }
        ItemKind::Address => {
            let want = first.unwrap_or_default();
            c.remove_top(&|l: &str| l.starts_with("ip ") && l.contains(&format!(" dev {}", port)) && canon(ItemKind::Address, l).is_some_and(|x| x == want));
        }
        ItemKind::State if ios => {
            if first.as_deref() == Some("down") {
                c.remove_in(&if_head(port), &|l: &str| l == "shutdown");
            } else {
                c.remove_in(&if_head(port), &|l: &str| l == "shutdown");
                c.add_in(&if_head(port), "shutdown");
            }
        }
        ItemKind::State => {
            let down = format!("ip link set dev {} down", port);
            c.remove_top(&|l: &str| l == down);
            if first.as_deref() != Some("down") {
                c.lines.push(down);
            }
        }
        ItemKind::Vlan => c.remove_in(&if_head(port), &|l: &str| l.starts_with("switchport access vlan ")),
        ItemKind::Mode => {
            if first.as_deref() == Some("routed") {
                // a switch port again: without its address (a switch port takes none)
                c.remove_in(&if_head(port), &|l: &str| l == "no switchport" || l.starts_with("ip address ") || l == "no ip address");
            } else {
                c.remove_in(&if_head(port), &|l: &str| l.starts_with("switchport mode "));
            }
        }
        ItemKind::Route => {
            let want = first.as_deref().and_then(RouteVal::parse);
            if let Some(w) = want {
                c.remove_top(&|l: &str| (l.starts_with("ip route ")) && RouteVal::parse(l).is_some_and(|r| route_like(&w, &r, r.ad.unwrap_or(1))));
            }
        }
        ItemKind::Config => {
            if let Some(m) = model {
                let ls = config_lines(it.values().first().copied().unwrap_or(&Value::Null));
                let exact = it.cmp == Compare::Exact;
                let want = if exact { Lines { have: literal(&ls), ..Lines::default() } } else { normalise(m, &ls) };
                // the first line of a block whose lines the item names as written is only
                // their place (a normalised item names a block's first line when it makes the
                // block: "router ospf 1" goes with its lines)
                let heads: BTreeSet<String> = want.have.iter().filter(|p| !p.0.is_empty()).map(|p| p.0.clone()).collect();
                for (head, line) in &want.have {
                    let (h, l) = (head.clone(), line.clone());
                    if !h.is_empty() {
                        c.remove_in(&|x: &str| x == h, &|x: &str| x == l);
                    } else if !(exact && heads.contains(&l)) {
                        c.remove_top(&|x: &str| x == l);
                    }
                }
                for (head, line) in &want.lack {
                    let h = head.clone();
                    if h.is_empty() {
                        c.add_top(line);
                    } else {
                        c.add_in(&|x: &str| x == h, line);
                    }
                }
            }
        }
        _ => {}
    }
    d.startup_config = c.text();
}

/// takes a setting out of a service's configuration (its path; nothing when it is not there)
fn remove_path(v: &mut Value, path: &str) {
    let segs: Vec<&str> = path.split('.').map(str::trim).filter(|s| !s.is_empty()).collect();
    let Some((last, head)) = segs.split_last() else { return };
    let mut cur = v;
    for s in head {
        cur = match cur {
            Value::Object(m) => match m.iter_mut().find(|(k, _)| k.as_str() == *s) {
                Some((_, x)) => x,
                None => return,
            },
            Value::Array(a) => match s.parse::<usize>().ok().and_then(|i| a.get_mut(i)) {
                Some(x) => x,
                None => return,
            },
            _ => return,
        };
    }
    match cur {
        Value::Object(m) => m.retain(|(k, _)| k.as_str() != *last),
        Value::Array(a) => {
            if let Ok(i) = last.parse::<usize>()
                && i < a.len()
            {
                a.remove(i);
            }
        }
        _ => {}
    }
}

/// the start network: the target (each device with its running configuration from `e`, an
/// engine of the target) without what the items check; no lab
pub fn start_network(target: &Project, e: &Engine, items: &[Item]) -> Project {
    let mut p = target.clone();
    p.lab = None;
    for d in p.devices.iter_mut() {
        let run = e.running_config(d.id);
        if !run.trim().is_empty() {
            d.startup_config = run;
        }
    }
    for it in items {
        strip(&mut p, it);
    }
    p
}

/// a prefix of a route as an item writes it ("10.0.2.0/24", "default")
pub fn prefix_text(p: Prefix4) -> String {
    if p.len == 0 { String::from("default") } else { format!("{}", p) }
}
