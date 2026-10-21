// picker -- a remote file tree with check boxes: a host's file system (backup sources) or a
// restore point's content (restore items).  Folders load when they open (the server's browse
// routes); checking a folder takes everything in it; unchecking something inside a checked
// folder splits that folder into its other entries.
//
//   Picker::host(id) / Picker::point(id)
//   picker.frame(ui, net, wid, now) -> changed     the tree (fills next_size)
//   picker.selected() -> paths                     the checked paths (no path twice)

use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::ids;
use bc_proto::model::{EntryKind, Listing};
use ui::*;

use crate::net::Net;
use crate::tr::t;

#[derive(Clone, PartialEq, Eq)]
enum Source {
    Host(String),
    Point(String),
}

#[derive(Clone)]
pub struct Picker {
    src: Source,
    pub tree: TreeState,
    pub checked: BTreeSet<String>,
    /// show files too (restore: yes, sources: folders and files)
    pub files: bool,
}

struct Node {
    path: String,
    name: String,
    depth: u16,
    dir: bool,
    size: u64,
}

fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        return String::from(name);
    }
    let sep = if parent.contains('\\') { '\\' } else { '/' };
    if parent.ends_with(sep) { format!("{}{}", parent, name) } else { format!("{}{}{}", parent, sep, name) }
}

fn is_under(path: &str, ancestor: &str) -> bool {
    if path.len() <= ancestor.len() || !path.starts_with(ancestor) {
        return false;
    }
    ancestor.ends_with(['/', '\\']) || matches!(path.as_bytes()[ancestor.len()], b'/' | b'\\')
}

impl Picker {
    pub fn host(id: &str) -> Picker {
        Picker { src: Source::Host(String::from(id)), tree: TreeState::default(), checked: BTreeSet::new(), files: true }
    }

    pub fn point(id: &str) -> Picker {
        Picker { src: Source::Point(String::from(id)), tree: TreeState::default(), checked: BTreeSet::new(), files: true }
    }

    fn url(&self, path: &str) -> String {
        let p = ids::percent_encode(path, true);
        match &self.src {
            Source::Host(h) => format!("/api/hosts/{}/browse?path={}", h, p),
            Source::Point(pt) => format!("/api/points/{}/browse?path={}", pt, p),
        }
    }

    fn listing<'a>(&self, net: &'a Net, path: &str) -> Option<&'a Listing> {
        net.data.listings.get(&self.url(path))
    }

    /// 1 checked (itself or a folder above), 2 partly (something inside), 0 not
    fn state(&self, path: &str) -> u8 {
        if self.checked.contains(path) || self.checked.iter().any(|c| is_under(path, c)) {
            1
        } else if self.checked.iter().any(|c| is_under(c, path)) {
            2
        } else {
            0
        }
    }

    fn toggle(&mut self, net: &Net, path: &str) {
        match self.state(path) {
            1 => {
                if self.checked.remove(path) {
                    return;
                }
                // a folder above is checked: split it down to this path
                let Some(top) = self.checked.iter().find(|c| is_under(path, c)).cloned() else { return };
                self.checked.remove(&top);
                let mut dir = top;
                loop {
                    let Some(l) = self.listing(net, &dir) else { break };
                    let mut next = None;
                    for e in &l.entries {
                        let p = join(&dir, &e.name);
                        if p == path {
                            continue;
                        }
                        if is_under(path, &p) {
                            next = Some(p);
                        } else {
                            self.checked.insert(p);
                        }
                    }
                    match next {
                        Some(n) => dir = n,
                        None => break,
                    }
                }
            }
            _ => {
                self.checked.retain(|c| !is_under(c, path));
                self.checked.insert(String::from(path));
            }
        }
    }

    /// the checked paths
    pub fn selected(&self) -> Vec<String> {
        self.checked.iter().cloned().collect()
    }

    /// select paths (an existing job's sources)
    pub fn set_selected(&mut self, paths: &[String]) {
        self.checked = paths.iter().cloned().collect();
    }

    fn rows(&self, net: &Net, out: &mut Vec<Node>, path: &str, depth: u16) {
        let Some(l) = self.listing(net, path) else { return };
        for e in &l.entries {
            if !self.files && e.kind != EntryKind::Dir {
                continue;
            }
            let p = join(path, &e.name);
            let dir = e.kind == EntryKind::Dir;
            out.push(Node { path: p.clone(), name: e.name.clone(), depth, dir, size: e.size });
            if dir && self.tree.expanded.contains(&ids::key(&p)) {
                self.rows(net, out, &p, depth + 1);
            }
        }
    }

    /// the tree; true when the selection changed
    pub fn frame(&mut self, ui: &mut Ui, net: &mut Net, wid: u32, now: u32) -> bool {
        // load the root and every open folder
        let root = self.url("");
        net.want(&root, 120_000, now);
        let mut rows = Vec::new();
        self.rows(net, &mut rows, "", 0);
        for n in &rows {
            if n.dir && self.tree.expanded.contains(&ids::key(&n.path)) {
                let u = self.url(&n.path);
                net.want(&u, 120_000, now);
            }
        }
        if !net.loaded(&root) {
            if let Some(e) = net.data.errors.get(&root) {
                let e = e.clone();
                ui.empty_state(IC_WARNING, t("Ordner nicht verfügbar"), &e);
            } else {
                crate::widgets::loading(ui, wid + 0xFFF0);
            }
            return false;
        }
        let sizes: Vec<String> = rows.iter().map(|n| if n.dir { String::new() } else { crate::fmt::size(n.size) }).collect();
        let tr: Vec<TreeRow> = rows
            .iter()
            .zip(&sizes)
            .map(|(n, sz)| TreeRow {
                key: ids::key(&n.path),
                depth: n.depth,
                has_children: n.dir,
                check: Some(self.state(&n.path)),
                icon: if n.dir { IC_FOLDER } else { IC_DOCUMENT },
                label: &n.name,
                detail: if sz.is_empty() { None } else { Some(sz.as_str()) },
            })
            .collect();
        let ev = ui.tree(wid, &mut self.tree, &tr, 32);
        if let Some((k, _)) = ev.check {
            if let Some(n) = rows.iter().find(|n| ids::key(&n.path) == k) {
                let p = n.path.clone();
                self.toggle(net, &p);
                return true;
            }
        }
        false
    }
}
