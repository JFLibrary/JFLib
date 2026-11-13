// a11y_auto -- what the scripts see of the accessibility tree and how they act as a screen
// reader (auto.rs: the command `a11y`; section 12 and 19 of the guide)
//
//   a11y ACTION S [ARG]   a screen reader's request for the node of record S as a bridge sends
//                         it (plat::a11y::request / request_op), then the passes that carry it
//                         out: click, focus, scroll, expand, collapse, increment, decrement,
//                         select, add, remove, value N, text "s", selecttext ANCHOR CARET; a
//                         record with an id64 hands it to the app (ui.a11y_request_take)
//   a11y expect S role R | name "s" | desc "s" | url "s" | text "s" | level N | set POS SIZE |
//                 cell ROW COL [RSPAN CSPAN] | grid ROWS COLS | live polite|assertive|off |
//                 is STATE | not STATE | can ACTION | cannot ACTION | parent S2 | top |
//                 children N | missing
//                         R: the Sub's ARIA name (table, cell, banner, treeitem ...) or the role's
//                         (label, button, listitem ...); STATE: focused focusable disabled checked
//                         mixed selected expanded collapsed pressed multiline protected readonly
//                         required invalid busy haspopup visited modal multiselect offscreen
//                         selectable; ACTION: click focus value text expand step scroll select
//   a11y expect announced "s" | none   the last text ui.announce said since the last check
//   a11y dump [FILE]      the tree as text, a node per line indented by its depth: role[/sub],
//                         name, text (a slider's, meter's or progress bar's: valuetext), desc,
//                         url, value and range of those, the states, level, set, cell, live, key

use alloc::string::String;
use alloc::vec::Vec;

use plat::a11y::{self as ax, Act, Node, Op, OpRequest, Request, Snapshot, WinTree, NONE};

use super::a11y::{ID_MASK, RICH};
use super::auto::{next_word, parse_qstring, skip_ws, word_eq, Sel};
use super::runtime::parse_int;
use super::{App, Ui};

// the role names of Role in its declaration order
const ROLE_NAMES: [&str; 22] = [
    "group", "label", "button", "checkbox", "radio", "switch", "slider", "progress", "textbox", "link", "list", "listitem",
    "menubar", "menu", "menuitem", "tablist", "tab", "combo", "image", "canvas", "dialog", "heading",
];

const STATES: [(&str, u32); 21] = [
    ("focused", ax::S_FOCUSED),
    ("focusable", ax::S_FOCUSABLE),
    ("disabled", ax::S_DISABLED),
    ("checked", ax::S_CHECKED),
    ("mixed", ax::S_MIXED),
    ("selected", ax::S_SELECTED),
    ("expanded", ax::S_EXPANDED),
    ("pressed", ax::S_PRESSED),
    ("multiline", ax::S_MULTILINE),
    ("protected", ax::S_PROTECTED),
    ("readonly", ax::S_READONLY),
    ("required", ax::S_REQUIRED),
    ("invalid", ax::S_INVALID),
    ("busy", ax::S_BUSY),
    ("haspopup", ax::S_HASPOPUP),
    ("visited", ax::S_VISITED),
    ("modal", ax::S_MODAL),
    ("multiselect", ax::S_MULTISELECT),
    ("collapsed", ax::S_COLLAPSED),
    ("offscreen", ax::S_OFFSCREEN),
    ("selectable", ax::S_SELECTABLE),
];

const ACTIONS: [(&str, u32); 8] = [
    ("click", ax::A_CLICK),
    ("focus", ax::A_FOCUS),
    ("value", ax::A_VALUE),
    ("text", ax::A_TEXT),
    ("expand", ax::A_EXPAND),
    ("step", ax::A_STEP),
    ("scroll", ax::A_SCROLL),
    ("select", ax::A_SELECT),
];

// the role name of a node
fn role_name(n: &Node) -> &'static str {
    ROLE_NAMES.get(n.role as usize).copied().unwrap_or("?")
}

// a quoted string of a script line
fn qstring(p: &[u8]) -> Option<String> {
    parse_qstring(skip_ws(p), 4096).map(|(s, _)| String::from_utf8_lossy(&s).into_owned())
}

// one line of the dump: role[/sub] "name" [text] [desc] [url] [value] states, the rich fields, key
fn dump_node(out: &mut String, t: &WinTree, k: usize) {
    let n = &t.nodes[k];
    let mut depth = 0;
    let mut p = n.parent;
    while p != NONE && depth < 64 {
        depth += 1;
        p = t.nodes[p as usize].parent;
    }
    for _ in 0..depth + 1 {
        out.push_str("  ");
    }
    out.push_str(role_name(n));
    if n.sub != ax::Sub::None {
        out.push('/');
        out.push_str(n.sub.aria());
    }
    out.push_str(&alloc::format!(" {:?}", n.name));
    // a slider's, meter's or progress bar's value (its text: the record's value_text)
    let ranged = matches!(n.role, ax::Role::Slider | ax::Role::Progress);
    if !n.text.is_empty() {
        out.push_str(&alloc::format!(" {}={:?}", if ranged { "valuetext" } else { "text" }, n.text));
    }
    if !n.desc.is_empty() {
        out.push_str(&alloc::format!(" desc={:?}", n.desc));
    }
    if !n.url.is_empty() {
        out.push_str(&alloc::format!(" url={:?}", n.url));
    }
    if ranged {
        out.push_str(&alloc::format!(" value={}", n.value));
        if n.max > n.min {
            out.push_str(&alloc::format!(" range={}..{}", n.min, n.max));
        }
    }
    for (name, bit) in STATES {
        if n.state & bit != 0 {
            out.push(' ');
            out.push_str(name);
        }
    }
    if n.level() > 0 {
        out.push_str(&alloc::format!(" level={}", n.level()));
    }
    if n.set != [0, 0] {
        out.push_str(&alloc::format!(" set={}/{}", n.set[0], n.set[1]));
    }
    if let Some(c) = n.cell_place() {
        out.push_str(&alloc::format!(" cell={},{}", c[0], c[1]));
    }
    if n.live != 0 {
        out.push_str(if n.live == ax::LIVE_ASSERTIVE { " live=assertive" } else { " live=polite" });
    }
    out.push_str(&alloc::format!(" #{:x}\n", n.key));
}

/// the tree as text (a11y dump)
pub(crate) fn dump_tree(s: &Snapshot) -> String {
    let mut out = String::new();
    for t in &s.wins {
        out.push_str(&alloc::format!("window {} {:?}\n", t.win, t.title));
        for k in 0..t.nodes.len() {
            dump_node(&mut out, t, k);
        }
    }
    out
}

impl Ui {
    /// (scripts, tests) the key of the node of record k (self.sems[k]) in the accessibility tree:
    /// RICH | its id64 (the first record of that id64 in its window), else its occurrence among
    /// the records of its widget id in its window << 32 | the id
    pub(crate) fn a11y_key_of(&self, k: usize) -> u64 {
        let Some(r) = self.sems.get(k) else { return 0 };
        let rid = r.ax.as_ref().map_or(0, |a| a.id & ID_MASK);
        let same_win = self.sems[..k].iter().filter(|s| s.win == r.win);
        if rid != 0 && !same_win.clone().any(|s| s.ax.as_ref().is_some_and(|a| a.id & ID_MASK == rid)) {
            return RICH | rid;
        }
        let n = same_win.filter(|s| s.id == r.id).count() as u64 + 1;
        n << 32 | r.id as u64
    }

    // the window and the node key of record S of a script line, the rest of the line
    fn a11y_sel<'a>(&mut self, p: &'a [u8]) -> (Option<(u8, u64)>, &'a [u8]) {
        match self.resolve(p) {
            (Some(Sel::Rec(k)), rest) => (Some((self.sems[k].win, self.a11y_key_of(k))), rest),
            (_, rest) => (None, rest),
        }
    }

    /// (auto.rs) the script command `a11y`
    pub(crate) fn sc_a11y(&mut self, app: &mut dyn App, a: &[u8]) {
        if let Some(r) = word_eq(a, "expect") {
            return self.sc_a11y_expect(skip_ws(r));
        }
        if let Some(r) = word_eq(a, "dump") {
            let text = dump_tree(&self.a11y_tree());
            let r = skip_ws(r);
            if r.is_empty() {
                plat::write_stdout(text.as_bytes());
            } else {
                let path = String::from_utf8_lossy(r).into_owned();
                super::perm::trusted(|| plat::write_file(path.trim(), text.as_bytes()));
            }
            return;
        }
        let verb = a;
        let (sel, rest) = self.a11y_sel(next_word(a));
        let Some((win, key)) = sel else { return self.sel_fail() };
        let rest = skip_ws(rest);
        let req: Result<Act, Op> = if word_eq(verb, "click").is_some() {
            Ok(Act::Click)
        } else if word_eq(verb, "focus").is_some() {
            Ok(Act::Focus)
        } else if word_eq(verb, "value").is_some() {
            Ok(Act::Value(parse_int(rest).0 as f64))
        } else if word_eq(verb, "text").is_some() {
            Ok(Act::Text(qstring(rest).unwrap_or_default()))
        } else if word_eq(verb, "selecttext").is_some() {
            let (anchor, r) = parse_int(rest);
            Ok(Act::Select(anchor, parse_int(r).0))
        } else if word_eq(verb, "scroll").is_some() {
            Err(Op::ScrollIntoView)
        } else if word_eq(verb, "expand").is_some() {
            Err(Op::Expand)
        } else if word_eq(verb, "collapse").is_some() {
            Err(Op::Collapse)
        } else if word_eq(verb, "increment").is_some() {
            Err(Op::Increment)
        } else if word_eq(verb, "decrement").is_some() {
            Err(Op::Decrement)
        } else if word_eq(verb, "select").is_some() {
            Err(Op::SelectItem)
        } else if word_eq(verb, "add").is_some() {
            Err(Op::AddToSelection)
        } else if word_eq(verb, "remove").is_some() {
            Err(Op::RemoveFromSelection)
        } else {
            return self.fail(b"unknown a11y action");
        };
        match req {
            Ok(act) => ax::request(Request { win, key, act }),
            Err(op) => ax::request_op(OpRequest { win, key, op }),
        }
        self.a11y_poll();
        self.run_passes(app);
    }

    fn sc_a11y_expect(&mut self, a: &[u8]) {
        if let Some(r) = word_eq(a, "announced") {
            let said = self.a11y_said_take();
            let last = said.last().map(|s| s.as_str());
            if word_eq(skip_ws(r), "none").is_some() {
                if let Some(l) = last {
                    self.fail(alloc::format!("announced {l:?}").as_bytes());
                }
            } else if last != qstring(r).as_deref() {
                self.fail(alloc::format!("announced {last:?}").as_bytes());
            }
            return;
        }
        let (sel, cond) = self.a11y_sel(a);
        let cond = skip_ws(cond);
        let Some((win, key)) = sel else { return self.sel_fail() };
        let tree = self.a11y_tree();
        let Some(t) = tree.win(win) else { return self.fail(b"no such window") };
        let Some(k) = t.find(key) else {
            if word_eq(cond, "missing").is_none() {
                self.fail(b"the record is no node of the tree");
            }
            return;
        };
        let n = &t.nodes[k];
        let num = |p: &[u8]| -> Vec<u32> {
            let mut v = Vec::new();
            let mut p = skip_ws(p);
            while !p.is_empty() {
                let (x, r) = parse_int(p);
                v.push(x.max(0) as u32);
                p = skip_ws(r);
            }
            v
        };
        let bad: Option<String> = if word_eq(cond, "missing").is_some() {
            Some(String::from("the node exists"))
        } else if let Some(r) = word_eq(cond, "role") {
            let want = String::from_utf8_lossy(skip_ws(r)).into_owned();
            let want = want.trim();
            (want != n.sub.aria() && want != role_name(n)).then(|| alloc::format!("role is {}/{}", role_name(n), n.sub.aria()))
        } else if let Some(r) = word_eq(cond, "name") {
            (qstring(r).as_deref() != Some(n.name.as_str())).then(|| alloc::format!("name is {:?}", n.name))
        } else if let Some(r) = word_eq(cond, "desc") {
            (qstring(r).as_deref() != Some(n.desc.as_str())).then(|| alloc::format!("desc is {:?}", n.desc))
        } else if let Some(r) = word_eq(cond, "url") {
            (qstring(r).as_deref() != Some(n.url.as_str())).then(|| alloc::format!("url is {:?}", n.url))
        } else if let Some(r) = word_eq(cond, "text") {
            (qstring(r).as_deref() != Some(n.text.as_str())).then(|| alloc::format!("text is {:?}", n.text))
        } else if let Some(r) = word_eq(cond, "level") {
            (num(r).first() != Some(&n.level())).then(|| alloc::format!("level is {}", n.level()))
        } else if let Some(r) = word_eq(cond, "set") {
            (num(r) != n.set).then(|| alloc::format!("set is {:?}", n.set))
        } else if let Some(r) = word_eq(cond, "cell") {
            let v = num(r);
            let have = n.cell_place().unwrap_or([0; 4]);
            (v.len() < 2 || v[..] != have[..v.len().min(4)]).then(|| alloc::format!("cell is {have:?}"))
        } else if let Some(r) = word_eq(cond, "grid") {
            let size = t.grid_size(k);
            (num(r) != [size.0, size.1]).then(|| alloc::format!("grid is {size:?}"))
        } else if let Some(r) = word_eq(cond, "live") {
            let have = match n.live {
                ax::LIVE_POLITE => "polite",
                ax::LIVE_ASSERTIVE => "assertive",
                _ => "off",
            };
            (word_eq(skip_ws(r), have).is_none()).then(|| alloc::format!("live is {have}"))
        } else if let Some((r, want)) = word_eq(cond, "is").map(|r| (r, true)).or_else(|| word_eq(cond, "not").map(|r| (r, false))) {
            let r = skip_ws(r);
            match STATES.iter().find(|(name, _)| word_eq(r, name).is_some()) {
                Some(&(name, bit)) => ((n.state & bit != 0) != want).then(|| alloc::format!("{name} is {}", if want { "off" } else { "on" })),
                None => Some(String::from("unknown state")),
            }
        } else if let Some((r, want)) = word_eq(cond, "can").map(|r| (r, true)).or_else(|| word_eq(cond, "cannot").map(|r| (r, false))) {
            let r = skip_ws(r);
            match ACTIONS.iter().find(|(name, _)| word_eq(r, name).is_some()) {
                Some(&(name, bit)) => ((n.actions & bit != 0) != want).then(|| alloc::format!("{} {name}", if want { "cannot" } else { "can" })),
                None => Some(String::from("unknown action")),
            }
        } else if word_eq(cond, "top").is_some() {
            (n.parent != NONE).then(|| String::from("it has a parent"))
        } else if let Some(r) = word_eq(cond, "children") {
            (num(r).first() != Some(&n.count)).then(|| alloc::format!("{} children", n.count))
        } else if let Some(r) = word_eq(cond, "parent") {
            let parent = t.nodes.get(n.parent as usize).map(|p| p.key);
            let n_name = n.name.clone();
            match self.a11y_sel(skip_ws(r)).0 {
                Some((_, want)) if Some(want) == parent => None,
                Some(_) => Some(alloc::format!("the parent of {n_name:?} is {:?}", parent.and_then(|p| t.find(p)).map(|j| t.nodes[j].name.clone()))),
                None => Some(String::from("no such parent record")),
            }
        } else {
            Some(String::from("unknown condition"))
        };
        if let Some(b) = bad {
            self.fail(b.as_bytes());
        }
    }
}
