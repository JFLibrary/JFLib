// group -- the groups of the logical canvas (G1): named frames around devices, notes and other
// groups that collapse into one cluster symbol.  View data: the simulator never sees them, so
// the engine, its keyframes and every report are the same with and without groups.
//
//   Group, GroupColor            the record: id (the project's counter, shared with devices,
//                                links and notes), name, colour role, members (ids of devices,
//                                notes and groups, in their order), collapsed
//   Groups::of(&project)         the groups resolved: a member belongs to the first group that
//                                lists it, ids that are gone (a deleted device, which undo may
//                                bring back) and cycles are skipped, a group without a device or
//                                note below it does not count; has / get / parent / chain /
//                                members / leaves / devices / depth / order / hidden_by / rep
//   group(p, ids, name) -> id    a new group of the given things (their outermost ones: a member
//                                of a group that is given too stays in it) in the innermost
//                                group that holds them all, where the first of them was
//   ungroup(p, id)               the group goes, its members take its place in its parent (or
//                                go to the top)
//   rename / set_collapsed / set_color   one group; GroupErr::msg says why one was refused
//   unique_name(p, prefix)       the prefix and the lowest free number ("Gruppe 1": the app
//                                passes the word in its language)
//   normalize(p)                 what Groups resolves written back: no ids that are gone, no
//                                member twice, no cycles, no empty groups (Project::from_text)
//   issues(p)                    what does not fit (Project::check)
//
// The record in the .jfnet file ("groups": [] without any):
//
//   "groups": [{"id": 12, "name": "Serverraum", "color": "blue", "members": [3, 4, 5, 13],
//               "collapsed": true},
//              {"id": 13, "name": "Rack 2", "color": "green", "members": [6, 7]}]
//
// Group 13 lies in group 12; a member left out of every group is at the top.  A collapsed group
// shows as one cluster symbol at the middle of the boxes of the devices and notes below it (the
// app's canvas); they keep their positions, so expanding it shows them where they were.  Names:
// 1 to 64 characters, unique among the groups (letter case aside).  Members that are missing
// take their defaults ("color": "grey", "collapsed": false).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use agent_core::{record, wire_enum};

use crate::msg;
use crate::msg::Msg;
use crate::project::Project;

/// the longest name of a group (characters)
pub const NAME_MAX: usize = 64;

wire_enum! {
    /// the colour role of a group's frame: muted, never the only sign (the frame has its name,
    /// the cluster its name and count)
    pub enum GroupColor {
        Grey = "grey",
        Blue = "blue",
        Green = "green",
        Ochre = "ochre",
        Brick = "brick",
        Violet = "violet",
    }
}

impl GroupColor {
    /// the German name (the app translates it)
    pub fn title(self) -> &'static str {
        // i18n
        match self {
            GroupColor::Grey => "Grau",
            GroupColor::Blue => "Blau",
            GroupColor::Green => "Grün",
            GroupColor::Ochre => "Ocker",
            GroupColor::Brick => "Ziegelrot",
            GroupColor::Violet => "Violett",
        }
    }
    /// the role of the n-th group (blue first; neighbours differ)
    pub fn nth(n: usize) -> GroupColor {
        GroupColor::ALL[(n + 1) % GroupColor::ALL.len()]
    }
}

record! {
    /// a group of the logical canvas
    pub struct Group {
        pub id: u32,
        pub name: String,
        pub color: GroupColor,
        /// the devices, notes and groups in it (ids), in their order
        pub members: Vec<u32>,
        /// shown as one cluster symbol
        pub collapsed: bool,
    }
}

/// why an edit of the groups was refused
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupErr {
    /// no device, note or group given
    Nothing,
    UnknownGroup(u32),
    BadName(String),
    NameTaken(String),
}

impl GroupErr {
    /// the reason as a message
    pub fn msg(&self) -> Msg {
        match self {
            GroupErr::Nothing => msg!("Nichts zum Gruppieren: Geräte, Notizen oder Gruppen auswählen."),
            GroupErr::UnknownGroup(id) => msg!("Die Gruppe {} gibt es nicht.", id),
            GroupErr::BadName(n) => msg!("\u{201e}{}\u{201c} taugt nicht als Name einer Gruppe: 1 bis 64 Zeichen.", n),
            GroupErr::NameTaken(n) => msg!("Eine Gruppe \u{201e}{}\u{201c} gibt es schon.", n),
        }
    }
}

/// the groups of a project resolved (see the file header)
#[derive(Clone, Debug, Default)]
pub struct Groups {
    /// device, note or group id -> the group it is in
    parent: BTreeMap<u32, u32>,
    /// group id -> the members that count, in order
    kids: BTreeMap<u32, Vec<u32>>,
    /// group id -> its index in Project::groups
    idx: BTreeMap<u32, usize>,
    /// the groups that count, outer ones first (depth, then the file's order)
    order: Vec<u32>,
    /// group id -> its depth (0: at the top)
    depth: BTreeMap<u32, usize>,
}

impl Groups {
    /// the project's groups resolved
    pub fn of(p: &Project) -> Groups {
        let mut g = Groups::default();
        if p.groups.is_empty() {
            return g;
        }
        let leaves: BTreeSet<u32> = p.devices.iter().map(|d| d.id).chain(p.notes.iter().map(|n| n.id)).collect();
        // the first group with an id counts; ids of devices, links and notes are theirs
        let links: BTreeSet<u32> = p.links.iter().map(|l| l.id).collect();
        for (i, x) in p.groups.iter().enumerate() {
            if x.id != 0 && !leaves.contains(&x.id) && !links.contains(&x.id) {
                g.idx.entry(x.id).or_insert(i);
            }
        }
        let known = |id: u32| leaves.contains(&id) || g.idx.contains_key(&id);
        let mut parent: BTreeMap<u32, u32> = BTreeMap::new();
        // in the file's order: a member goes to the first group that lists it
        let mut groups: Vec<(u32, usize)> = g.idx.iter().map(|(&id, &i)| (id, i)).collect();
        groups.sort_by_key(|x| x.1);
        for &(gid, i) in &groups {
            for &m in &p.groups[i].members {
                if m != gid && known(m) && !parent.contains_key(&m) {
                    parent.insert(m, gid);
                }
            }
        }
        // a cycle loses the link that closes it (the group of the file met first keeps its own)
        for &(gid, _) in &groups {
            let mut at = gid;
            let mut steps = 0;
            while let Some(&up) = parent.get(&at) {
                if up == gid {
                    parent.remove(&at);
                    break;
                }
                at = up;
                steps += 1;
                if steps > groups.len() {
                    break;
                }
            }
        }
        // a group counts with a device or note below it
        let mut kids: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for &(gid, i) in &groups {
            let v: Vec<u32> = p.groups[i].members.iter().copied().filter(|m| parent.get(m) == Some(&gid)).collect();
            let mut seen = BTreeSet::new();
            kids.insert(gid, v.into_iter().filter(|m| seen.insert(*m)).collect());
        }
        let mut full: BTreeMap<u32, bool> = BTreeMap::new();
        fn has_leaf(gid: u32, kids: &BTreeMap<u32, Vec<u32>>, leaves: &BTreeSet<u32>, memo: &mut BTreeMap<u32, bool>, depth: usize) -> bool {
            if let Some(&v) = memo.get(&gid) {
                return v;
            }
            let v = depth < 256 && kids.get(&gid).is_some_and(|ks| ks.iter().any(|&k| leaves.contains(&k) || has_leaf(k, kids, leaves, memo, depth + 1)));
            memo.insert(gid, v);
            v
        }
        for &(gid, _) in &groups {
            has_leaf(gid, &kids, &leaves, &mut full, 0);
        }
        let counts = |id: u32| full.get(&id).copied().unwrap_or(false);
        g.idx.retain(|id, _| counts(*id));
        parent.retain(|m, up| counts(*up) && (leaves.contains(m) || counts(*m)));
        for (gid, ks) in kids {
            if counts(gid) {
                let ks: Vec<u32> = ks.into_iter().filter(|k| leaves.contains(k) || counts(*k)).collect();
                g.kids.insert(gid, ks);
            }
        }
        g.parent = parent;
        for &(gid, _) in &groups {
            if counts(gid) {
                let d = g.chain(gid).len();
                g.depth.insert(gid, d);
            }
        }
        let mut order: Vec<(usize, usize, u32)> = groups.iter().filter(|x| counts(x.0)).map(|&(id, i)| (g.depth.get(&id).copied().unwrap_or(0), i, id)).collect();
        order.sort();
        g.order = order.into_iter().map(|x| x.2).collect();
        g
    }

    /// there are groups that count
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
    /// id is a group that counts
    pub fn has(&self, id: u32) -> bool {
        self.idx.contains_key(&id)
    }
    /// the record of a group that counts
    pub fn get<'a>(&self, p: &'a Project, id: u32) -> Option<&'a Group> {
        p.groups.get(*self.idx.get(&id)?)
    }
    /// the group a device, note or group is in
    pub fn parent(&self, id: u32) -> Option<u32> {
        self.parent.get(&id).copied()
    }
    /// the groups around a device, note or group, the innermost first
    pub fn chain(&self, id: u32) -> Vec<u32> {
        let mut v = Vec::new();
        let mut at = id;
        while let Some(&up) = self.parent.get(&at) {
            if v.contains(&up) || v.len() > 256 {
                break;
            }
            v.push(up);
            at = up;
        }
        v
    }
    /// the members of a group that count, in order
    pub fn members(&self, id: u32) -> &[u32] {
        self.kids.get(&id).map_or(&[], |v| v.as_slice())
    }
    /// the devices and notes below a group (all levels), in order
    pub fn leaves(&self, id: u32) -> Vec<u32> {
        let mut v = Vec::new();
        self.collect(id, &mut v, 0);
        v
    }
    fn collect(&self, id: u32, out: &mut Vec<u32>, depth: usize) {
        for &k in self.members(id) {
            if self.has(k) {
                if depth < 256 {
                    self.collect(k, out, depth + 1);
                }
            } else {
                out.push(k);
            }
        }
    }
    /// the devices below a group (all levels)
    pub fn devices(&self, p: &Project, id: u32) -> Vec<u32> {
        self.leaves(id).into_iter().filter(|&d| p.device(d).is_some()).collect()
    }
    /// the groups below a group (all levels), outer ones first
    pub fn subgroups(&self, id: u32) -> Vec<u32> {
        self.order.iter().copied().filter(|&g| g != id && self.chain(g).contains(&id)).collect()
    }
    /// how deep a group lies (0: at the top)
    pub fn depth(&self, id: u32) -> usize {
        self.depth.get(&id).copied().unwrap_or(0)
    }
    /// the groups that count, outer ones first
    pub fn order(&self) -> &[u32] {
        &self.order
    }
    /// the outermost collapsed group around a device, note or group (None: it shows)
    pub fn hidden_by(&self, p: &Project, id: u32) -> Option<u32> {
        self.chain(id).into_iter().rev().find(|&g| self.get(p, g).is_some_and(|x| x.collapsed))
    }
    /// what shows for a device, note or group: itself, or the cluster that hides it
    pub fn rep(&self, p: &Project, id: u32) -> u32 {
        self.hidden_by(p, id).unwrap_or(id)
    }
}

/// the prefix and the lowest free number among the groups' names ("Gruppe 1", "Gruppe 2")
pub fn unique_name(p: &Project, prefix: &str) -> String {
    let mut n = 1u32;
    loop {
        let c = format!("{} {}", prefix, n);
        if !taken(p, &c, 0) {
            return c;
        }
        n += 1;
    }
}

// another group than `except` has the name (letter case aside)
fn taken(p: &Project, name: &str, except: u32) -> bool {
    p.groups.iter().any(|g| g.id != except && g.name.to_lowercase() == name.to_lowercase())
}

fn check_name(p: &Project, name: &str, except: u32) -> Result<String, GroupErr> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > NAME_MAX || n.chars().any(|c| c.is_control()) {
        return Err(GroupErr::BadName(String::from(n)));
    }
    if taken(p, n, except) {
        return Err(GroupErr::NameTaken(String::from(n)));
    }
    Ok(String::from(n))
}

fn index(p: &Project, id: u32) -> Option<usize> {
    p.groups.iter().position(|g| g.id == id)
}

/// a new group of the devices, notes and groups `ids` (their outermost ones) in the innermost
/// group that holds them all, where the first of them was; `name` empty: unique_name(prefix)
/// -> its id
pub fn group(p: &mut Project, ids: &[u32], name: &str, prefix: &str) -> Result<u32, GroupErr> {
    let gs = Groups::of(p);
    let mut given: Vec<u32> = Vec::new();
    for &id in ids {
        let ok = p.device(id).is_some() || p.note(id).is_some() || gs.has(id);
        if ok && !given.contains(&id) {
            given.push(id);
        }
    }
    // the outermost ones: a member of a given group stays in it
    let tops: Vec<u32> = given.iter().copied().filter(|&id| !gs.chain(id).iter().any(|g| given.contains(g))).collect();
    if tops.is_empty() {
        return Err(GroupErr::Nothing);
    }
    let name = if name.trim().is_empty() { unique_name(p, prefix) } else { check_name(p, name, 0)? };
    // the innermost group holding them all: the common start of their chains (outermost first)
    let chains: Vec<Vec<u32>> = tops.iter().map(|&id| gs.chain(id).into_iter().rev().collect()).collect();
    let mut lca: Option<u32> = None;
    for k in 0.. {
        let Some(&g) = chains[0].get(k) else { break };
        if chains.iter().all(|c| c.get(k) == Some(&g)) {
            lca = Some(g);
        } else {
            break;
        }
    }
    // where the first of them was in the group that holds them (its own member, or the member
    // of that group it lies in): the members before it that stay
    let at = lca.and_then(|l| {
        let first = tops[0];
        let inner = core::iter::once(first).chain(gs.chain(first)).find(|&x| gs.parent(x) == Some(l))?;
        let m = &p.groups.get(index(p, l)?)?.members;
        let k = m.iter().position(|&x| x == inner)?;
        Some(m[..k].iter().filter(|x| !tops.contains(x)).count())
    });
    let color = GroupColor::nth(p.groups.len());
    let id = p.alloc_id();
    // the given things leave their groups, the new one goes into the group that held them;
    // groups left empty go
    for g in p.groups.iter_mut() {
        g.members.retain(|m| !tops.contains(m));
    }
    if let Some(i) = lca.and_then(|l| index(p, l)) {
        let m = &mut p.groups[i].members;
        let k = at.unwrap_or(m.len()).min(m.len());
        m.insert(k, id);
    }
    p.groups.push(Group { id, name, color, members: tops, collapsed: false });
    drop_empty(p);
    Ok(id)
}

// groups whose records have no members go (also out of the groups that list them)
fn drop_empty(p: &mut Project) {
    loop {
        let gone: Vec<u32> = p.groups.iter().filter(|g| g.members.is_empty()).map(|g| g.id).collect();
        if gone.is_empty() {
            return;
        }
        p.groups.retain(|g| !gone.contains(&g.id));
        for g in p.groups.iter_mut() {
            g.members.retain(|m| !gone.contains(m));
        }
    }
}

/// the group goes; its members take its place in the group it is in (or go to the top)
pub fn ungroup(p: &mut Project, id: u32) -> Result<(), GroupErr> {
    let i = index(p, id).ok_or(GroupErr::UnknownGroup(id))?;
    let up = Groups::of(p).parent(id);
    let g = p.groups.remove(i);
    for x in p.groups.iter_mut() {
        if Some(x.id) == up {
            if let Some(k) = x.members.iter().position(|&m| m == id) {
                let add: Vec<u32> = g.members.iter().copied().filter(|m| !x.members.contains(m)).collect();
                x.members.splice(k..k + 1, add);
            }
        }
        x.members.retain(|&m| m != id);
    }
    drop_empty(p);
    Ok(())
}

/// a group's new name (1 to 64 characters, unique)
pub fn rename(p: &mut Project, id: u32, name: &str) -> Result<(), GroupErr> {
    let i = index(p, id).ok_or(GroupErr::UnknownGroup(id))?;
    let n = check_name(p, name, id)?;
    p.groups[i].name = n;
    Ok(())
}

/// a group shown as one cluster symbol, or with its members
pub fn set_collapsed(p: &mut Project, id: u32, on: bool) -> Result<(), GroupErr> {
    let i = index(p, id).ok_or(GroupErr::UnknownGroup(id))?;
    p.groups[i].collapsed = on;
    Ok(())
}

/// a group's colour role
pub fn set_color(p: &mut Project, id: u32, c: GroupColor) -> Result<(), GroupErr> {
    let i = index(p, id).ok_or(GroupErr::UnknownGroup(id))?;
    p.groups[i].color = c;
    Ok(())
}

/// the groups as Groups::of resolves them, written back (a file read in)
pub fn normalize(p: &mut Project) {
    if p.groups.is_empty() {
        return;
    }
    let gs = Groups::of(p);
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for g in &p.groups {
        if gs.has(g.id) && seen.insert(g.id) {
            out.push(Group { members: gs.members(g.id).to_vec(), ..g.clone() });
        }
    }
    p.groups = out;
}

/// what does not fit: (error, the message)
pub fn issues(p: &Project) -> Vec<(bool, Msg)> {
    let mut v = Vec::new();
    if p.groups.is_empty() {
        return v;
    }
    let mut ids: BTreeSet<u32> = p.devices.iter().map(|d| d.id).chain(p.links.iter().map(|l| l.id)).chain(p.notes.iter().map(|n| n.id)).collect();
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut member: BTreeSet<u32> = BTreeSet::new();
    let mut twice: BTreeSet<u32> = BTreeSet::new();
    for g in &p.groups {
        if g.id == 0 || !ids.insert(g.id) {
            v.push((true, msg!("Die Nummer {} der Gruppe \u{201e}{}\u{201c} ist schon vergeben.", g.id, g.name)));
        }
        if !names.insert(g.name.trim().to_lowercase()) || g.name.trim().is_empty() {
            v.push((false, msg!("Der Name der Gruppe \u{201e}{}\u{201c} ist leer oder doppelt.", g.name)));
        }
        for &m in &g.members {
            if !member.insert(m) {
                twice.insert(m);
            }
        }
    }
    let gs = Groups::of(p);
    for m in twice {
        let name = p.device(m).map(|d| d.name.clone()).or_else(|| p.groups.iter().find(|g| g.id == m).map(|g| g.name.clone()));
        if let Some(n) = name {
            v.push((false, msg!("\u{201e}{}\u{201c} gehört zu mehr als einer Gruppe; es zählt die erste.", n)));
        }
    }
    for g in &p.groups {
        // a group that lists a group around it (a cycle)
        if g.members.iter().any(|&m| m == g.id || gs.chain(g.id).contains(&m)) {
            v.push((false, msg!("Die Gruppe \u{201e}{}\u{201c} liegt in sich selbst.", g.name)));
        }
    }
    v
}
