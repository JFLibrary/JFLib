// t_a11y -- the accessibility tree for screen readers (lib/ui/a11y.rs, lib/plat/a11y.rs)
//
//   plattest --test a11y       (headless) a demo window as a ui::Runtime: the tree of its
//                              records (roles, a heading and its level, names from labels and
//                              from the text before a slider / text box, states, values, tab
//                              strip and list containers, the focus), then actions as a screen
//                              reader asks them through plat::a11y::request: click, toggle,
//                              select, set a slider and a text, focus, a disabled button, the
//                              pointer click for a widget whose record id differs from its hit
//                              box; the changes between two trees (plat::a11y::diff)
//   plattest --test a11y-bus   (Linux; tools/platshared/atspi_bus.py starts it on a fake session
//                              and accessibility bus) the demo in a real window with the AT-SPI2
//                              bridge; the fake screen reader reads the tree and acts, then
//                              presses "Beenden": the app state is checked at the end
//   plattest --test uia        (Windows, by hand: a window appears for a few seconds) the demo
//                              with the UI Automation providers; a thread of the test reads the
//                              tree through IUIAutomation (plat::uia_test_*: control types,
//                              patterns, the heading's HeadingLevel), invokes, toggles,
//                              sets values, follows the caret, then presses "Beenden"
//   plattest --test caret      (Windows, by hand) only the caret as screen magnifiers see it:
//                              the hidden system caret (GetGUIThreadInfo) and TextPattern2's
//                              caret range of the focused text box agree and follow a
//                              selection the client makes; TextSelectionChanged arrives.  Linux:
//                              the caret steps of tools/platshared/atspi_bus.py (a11y-bus)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI32, Ordering::SeqCst};

use plat::a11y::{self as ax, Act, Change, Node, Request, Role, Snapshot};
use ui::{App, AppDesc, Range, Ui};

use crate::T;

pub(crate) const DESC: AppDesc = AppDesc { title: "a11y-Test", w: 360, h: 700, ..AppDesc::NONE };

const ID_BTN: u32 = 0x100;
const ID_OFF: u32 = 0x101;
const ID_CHK: u32 = 0x102;
const ID_R1: u32 = 0x103;
const ID_R2: u32 = 0x104;
const ID_SW: u32 = 0x105;
const ID_SLD: u32 = 0x106;
const ID_TXT: u32 = 0x107;
const ID_TABS: u32 = 0x200;
const ID_LIST: u32 = 0x300;
const ID_PRG: u32 = 0x108;
const ID_OWN_HIT: u32 = 0x109;
const ID_OWN: u32 = 0x10A;
const ID_QUIT: u32 = 0x10B;
const FRUIT: [&str; 3] = ["Apfel", "Birne", "Kirsche"];

/// the demo's state, readable from other threads: clicks, check, radio, switch, slider, tab,
/// list row, clicks of the own widget, frames
pub(crate) static STATE: [AtomicI32; 9] = [const { AtomicI32::new(0) }; 9];
pub(crate) const CLICKS: usize = 0;
pub(crate) const CHECK: usize = 1;
pub(crate) const RADIO: usize = 2;
pub(crate) const SWITCH: usize = 3;
pub(crate) const SLIDER: usize = 4;
pub(crate) const TAB: usize = 5;
pub(crate) const LIST: usize = 6;
pub(crate) const OWN: usize = 7;
pub(crate) const FRAMES: usize = 8;

pub(crate) fn st(k: usize) -> i32 {
    STATE[k].load(SeqCst)
}

/// a window with one widget of every kind a screen reader meets
pub(crate) struct Demo {
    clicks: i32,
    check: i32,
    radio: i32,
    sw: i32,
    slider: i32,
    pub(crate) text: String,
    tab: i32,
    list: i32,
    own: i32,
    /// called from App::init with the running window (the UIA test starts its client there)
    pub(crate) on_init: Option<fn()>,
}

impl Demo {
    pub(crate) fn new() -> Demo {
        Demo { clicks: 0, check: 0, radio: 0, sw: 0, slider: 30, text: String::from("Anna"), tab: 0, list: 0, own: 0, on_init: None }
    }
}

impl App for Demo {
    fn init(&mut self, _ui: &mut Ui) -> bool {
        if let Some(f) = self.on_init {
            f();
        }
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        ui.heading(2, ui::TS_SUBTITLE, "Einstellungen");
        ui.text(ui::TS_BODY, &format!("Klicks: {}", self.clicks));
        if ui.button(ID_BTN, "Drücken") & ui::EV_ACT != 0 {
            self.clicks += 1;
        }
        ui.next_flags(ui::ST_DISABLED);
        ui.button(ID_OFF, "Gesperrt");
        ui.checkbox(ID_CHK, "Option", &mut self.check);
        ui.radio(ID_R1, "Eins", &mut self.radio, 0);
        ui.radio(ID_R2, "Zwei", &mut self.radio, 1);
        ui.switch(ID_SW, "Schalter", &mut self.sw);
        ui.box_begin(0, ui::BOX_H | ui::BOX_XCENTER, 8);
        ui.text(ui::TS_BODY, "Lautstärke");
        ui.next_size(160, 0);
        ui.slider(ID_SLD, &mut self.slider, &Range::new(0, 100, 1, 0));
        ui.box_end();
        ui.box_begin(0, ui::BOX_H | ui::BOX_XCENTER, 8);
        ui.text(ui::TS_BODY, "Name");
        ui.next_size(160, 0);
        ui.textbox(ID_TXT, None, &mut self.text, 64);
        ui.box_end();
        ui.tabs(ID_TABS, &mut self.tab, &["Erste", "Zweite"]);
        ui.next_size(-1, 100);
        let (a, b) = ui.list_begin(ID_LIST, &mut self.list, 3, 28);
        for i in a..b {
            ui.list_item(i, FRUIT[i as usize]);
        }
        ui.list_end(&mut self.list);
        ui.progress(ID_PRG, 40, 100);
        // a widget of its own whose record id differs from its hit box (the pointer click)
        let r = ui.place(ui.dp(120), ui.dp(28));
        if ui.hit_rect(ID_OWN_HIT, r[0], r[1], r[2], r[3], ui::HB_FOCUS) & ui::EV_ACT != 0 {
            self.own += 1;
        }
        ui.sem(ui::K_BUTTON, ID_OWN, Some("Eigen"), 0);
        if ui.button(ID_QUIT, "Beenden") & ui::EV_ACT != 0 {
            ui.quit();
        }
        for (k, v) in [self.clicks, self.check, self.radio, self.sw, self.slider, self.tab, self.list, self.own].into_iter().enumerate() {
            STATE[k].store(v, SeqCst);
        }
        STATE[FRAMES].fetch_add(1, SeqCst);
    }
}

/// the node of window 0 with this role and name
pub(crate) fn find<'a>(s: &'a Snapshot, role: Role, name: &str) -> Option<&'a Node> {
    s.wins.first()?.nodes.iter().find(|n| n.role == role && n.name == name)
}

struct Run {
    rt: ui::Runtime,
    now: u32,
}

impl Run {
    fn tree(&self) -> Snapshot {
        self.rt.ui().a11y_tree()
    }

    // one frame with the given events (time moves on by 50 ms)
    fn step(&mut self, events: &[plat::Event]) {
        self.now += 50;
        self.rt.step(self.now, events);
    }

    // a request as a bridge sends it, then the frame that carries it out
    fn act(&mut self, n: Option<&Node>, act: Act) {
        let Some(n) = n else { return };
        ax::request(Request { win: 0, key: n.key, act });
        self.step(&[plat::Event { typ: plat::E_WAKE, ..Default::default() }]);
    }
}

pub fn t_a11y(t: &mut T) {
    let rt = match ui::Runtime::new(&DESC, Demo::new(), &style::STYLE) {
        Ok(rt) => rt,
        Err(_) => {
            t.check(false, "a11y: the demo starts headless");
            return;
        }
    };
    let mut r = Run { rt, now: 0 };
    let s = r.tree();
    t.expect_i(s.wins.len() as i64, 1, "a11y: one window");
    let Some(w) = s.wins.first() else { return };
    t.expect_s(Some(&w.title), Some("a11y-Test"), "a11y: the window title");
    t.check(w.active, "a11y: the window is active (headless)");
    let label = find(&s, Role::Label, "Klicks: 0");
    t.check(label.is_some(), "a11y: a text record is a label with its text");
    let head = find(&s, Role::Heading, "Einstellungen");
    t.check(
        head.is_some_and(|n| n.level() == 2 && n.value == 2.0 && n.actions == 0 && n.state & ax::S_FOCUSABLE == 0),
        "a11y: ui.heading is a heading of level 2, no control",
    );
    t.check(label.is_some_and(|n| n.level() == 0), "a11y: a label has no heading level");
    // a heading whose level changes: a value change (the bridges report the new level)
    if let Some(h) = head {
        let mut s2 = s.clone();
        if let Some(n) = s2.wins[0].nodes.iter_mut().find(|n| n.key == h.key) {
            n.value = 3.0;
        }
        t.check(ax::diff(&s, &s2) == alloc::vec![Change::Value(0, h.key)], "a11y: diff: a heading's new level is a value change");
    }
    let btn = find(&s, Role::Button, "Drücken");
    t.check(btn.is_some_and(|n| n.actions & ax::A_CLICK != 0 && n.state & ax::S_FOCUSABLE != 0), "a11y: a button can be clicked and focused");
    let off = find(&s, Role::Button, "Gesperrt");
    t.check(off.is_some_and(|n| n.state & ax::S_DISABLED != 0 && n.actions == 0), "a11y: a disabled button: no actions");
    let chk = find(&s, Role::CheckBox, "Option");
    t.check(chk.is_some_and(|n| n.state & ax::S_CHECKED == 0), "a11y: check box, not checked");
    let r1 = find(&s, Role::Radio, "Eins");
    let r2 = find(&s, Role::Radio, "Zwei");
    t.check(r1.is_some_and(|n| n.state & ax::S_CHECKED != 0) && r2.is_some_and(|n| n.state & ax::S_CHECKED == 0), "a11y: radio buttons: the first one is on");
    let sw = find(&s, Role::Switch, "Schalter");
    t.check(sw.is_some(), "a11y: switch");
    let sld = find(&s, Role::Slider, "Lautstärke");
    t.check(
        sld.is_some_and(|n| n.value == 30.0 && n.min == 0.0 && n.max == 100.0 && n.actions & ax::A_VALUE != 0),
        "a11y: slider: name from the text before it, value 30 in 0 ..= 100",
    );
    let txt = find(&s, Role::TextBox, "Name");
    t.check(txt.is_some_and(|n| n.text == "Anna" && n.actions & ax::A_TEXT != 0), "a11y: text box: name from the text before it, its text");
    let prg = find(&s, Role::Progress, "");
    t.check(prg.is_some_and(|n| n.value == 40.0 && n.max == 100.0), "a11y: progress bar 40 of 100");
    let tl = s.wins[0].nodes.iter().position(|n| n.role == Role::TabList);
    let tabs: Vec<&Node> = tl.map_or(Vec::new(), |k| s.wins[0].children(k as u32).map(|c| &s.wins[0].nodes[c]).collect());
    t.check(
        tabs.len() == 2 && tabs[0].role == Role::Tab && tabs[0].name == "Erste" && tabs[0].state & ax::S_SELECTED != 0,
        "a11y: the tab strip holds its two tabs, the first selected",
    );
    let list = s.wins[0].nodes.iter().position(|n| n.role == Role::List);
    let rows: Vec<&Node> = list.map_or(Vec::new(), |k| s.wins[0].children(k as u32).map(|c| &s.wins[0].nodes[c]).collect());
    t.check(
        rows.len() == 3 && rows.iter().all(|n| n.role == Role::ListItem) && rows[2].name == "Kirsche",
        "a11y: the list holds its three rows",
    );
    t.check(rows.first().is_some_and(|n| n.state & ax::S_SELECTED != 0), "a11y: the first row is selected");
    t.check(
        s.wins[0].nodes.iter().all(|n| n.parent == ax::NONE || (n.parent as usize) < s.wins[0].nodes.len()),
        "a11y: every parent exists",
    );
    t.check(find(&s, Role::Button, "Beenden").is_some_and(|n| s.wins[0].on_screen(n)), "a11y: the demo fits its window");

    // actions
    let before = r.tree();
    r.act(btn, Act::Click);
    t.expect_i(st(CLICKS) as i64, 1, "a11y: click on the button");
    let after = r.tree();
    t.check(find(&after, Role::Label, "Klicks: 1").is_some(), "a11y: the label shows the new text");
    let ch = ax::diff(&before, &after);
    t.check(label.is_some_and(|l| ch.contains(&Change::Name(0, l.key))), "a11y: diff: the label's name changed");

    let before = r.tree();
    r.act(chk, Act::Click);
    t.expect_i(st(CHECK) as i64, 1, "a11y: toggle the check box");
    let after = r.tree();
    t.check(find(&after, Role::CheckBox, "Option").is_some_and(|n| n.state & ax::S_CHECKED != 0), "a11y: the check box is checked now");
    let ch = ax::diff(&before, &after);
    t.check(chk.is_some_and(|c| ch.contains(&Change::State(0, c.key, ax::S_CHECKED, true))), "a11y: diff: checked went on");

    r.act(r2, Act::Click);
    t.expect_i(st(RADIO) as i64, 1, "a11y: select the second radio button");
    r.act(sw, Act::Click);
    t.expect_i(st(SWITCH) as i64, 1, "a11y: switch on");
    r.act(sld, Act::Value(75.0));
    t.expect_i(st(SLIDER) as i64, 75, "a11y: set the slider to 75");
    t.check(find(&r.tree(), Role::Slider, "Lautstärke").is_some_and(|n| n.value == 75.0), "a11y: the slider's node has the value");
    r.act(sld, Act::Value(250.0));
    t.expect_i(st(SLIDER) as i64, 100, "a11y: a value above the range is clamped");
    r.act(txt, Act::Text(String::from("Bert")));
    t.check(find(&r.tree(), Role::TextBox, "Name").is_some_and(|n| n.text == "Bert"), "a11y: set the text box's text");

    // the caret (screen magnifiers): the focused text box reports its rectangle, the focus
    // brings it along, a selection set by a screen reader moves it
    let before = r.tree();
    r.act(txt, Act::Focus);
    r.step(&[]);
    let after = r.tree();
    let tb = find(&after, Role::TextBox, "Name");
    // (the keyboard focus selects everything: the anchor at 0)
    t.check(
        tb.is_some_and(|n| n.state & ax::S_FOCUSED != 0 && n.caret == 4 && n.selection() == (n.anchor.min(4), 4)),
        "a11y: focused text box: the caret after 'Bert', the selection up to it",
    );
    let (cr, rect) = tb.map_or(([0; 4], [0; 4]), |n| (n.caret_rect, n.rect));
    t.check(
        cr[2] > 0 && cr[3] > 0 && cr[0] > rect[0] && cr[0] < rect[0] + rect[2] && cr[1] >= rect[1] && cr[1] + cr[3] <= rect[1] + rect[3],
        "a11y: the caret's rectangle lies inside the text box",
    );
    t.check(
        tb.is_some_and(|n| n.char_rect(4)[0] == cr[0] && n.char_rect(0)[0] < n.char_rect(2)[0] && n.char_rect(2)[0] < cr[0]),
        "a11y: character extents: exact at the caret, the characters before it to its left",
    );
    t.check(tb.is_some_and(|n| ax::diff(&before, &after).contains(&Change::Caret(0, n.key, 4))), "a11y: diff: the focus brings the caret along");
    r.act(tb, Act::Select(1, 1));
    r.step(&[]);
    let moved = r.tree();
    let tb2 = find(&moved, Role::TextBox, "Name");
    t.check(
        tb2.is_some_and(|n| n.caret == 1 && n.selection() == (1, 1) && n.caret_rect[0] < cr[0] && n.caret_rect[3] == cr[3]),
        "a11y: Select(1, 1) moves the caret and its rectangle to the left",
    );
    t.check(tb2.is_some_and(|n| ax::diff(&after, &moved).contains(&Change::Caret(0, n.key, 1))), "a11y: diff: the caret moved");
    t.check(
        tb2.is_some_and(|n| {
            let q = n.char_rect(2);
            n.offset_at(q[0] + 1, q[1] + 1) == 2 && n.offset_at(rect[0] - 5, q[1]) == -1
        }),
        "a11y: the character offset at a point",
    );
    r.act(tb2, Act::Select(0, 3));
    r.step(&[]);
    t.check(find(&r.tree(), Role::TextBox, "Name").is_some_and(|n| n.selection() == (0, 3) && n.caret == 3), "a11y: Select(0, 3) selects 'Ber'");
    r.act(tabs.get(1).copied(), Act::Click);
    t.expect_i(st(TAB) as i64, 1, "a11y: select the second tab");
    r.act(rows.get(2).copied(), Act::Click);
    t.expect_i(st(LIST) as i64, 2, "a11y: select the third list row");
    let clicks = st(CLICKS);
    r.act(off, Act::Click);
    t.check(st(CLICKS) == clicks, "a11y: a disabled button ignores the click");

    // focus: the check box, then the list (its focus goes to the selected row)
    let before = r.tree();
    r.act(chk, Act::Focus);
    let after = r.tree();
    let w = &after.wins[0];
    t.check(w.nodes.get(w.focus as usize).is_some_and(|n| n.role == Role::CheckBox), "a11y: focus on the check box");
    let ch = ax::diff(&before, &after);
    t.check(chk.is_some_and(|c| ch.contains(&Change::Focus(0, c.key))), "a11y: diff: the focus moved");
    r.act(list.map(|k| &s.wins[0].nodes[k]), Act::Focus);
    let after = r.tree();
    let w = &after.wins[0];
    t.check(
        w.nodes.get(w.focus as usize).is_some_and(|n| n.role == Role::ListItem && n.name == "Kirsche"),
        "a11y: a focused list passes the focus on to its selected row",
    );

    // a widget whose record id is not its hit box: a pointer click in its middle
    let own = find(&s, Role::Button, "Eigen");
    r.act(own, Act::Click);
    r.step(&[]);
    t.expect_i(st(OWN) as i64, 1, "a11y: a click no widget took becomes a pointer click");

    // requests for nodes that are gone or synthetic do nothing
    ax::request(Request { win: 0, key: 0xFFFF_0000_1234, act: Act::Click });
    ax::request(Request { win: 5, key: btn.map_or(0, |n| n.key), act: Act::Click });
    let clicks = st(CLICKS);
    r.step(&[plat::Event { typ: plat::E_WAKE, ..Default::default() }]);
    r.step(&[]);
    t.check(st(CLICKS) == clicks, "a11y: requests for unknown nodes are dropped");

    let keys: Vec<u64> = r.tree().wins[0].nodes.iter().map(|n| n.key).collect();
    let mut sorted = keys.clone();
    sorted.sort_unstable();
    sorted.dedup();
    t.check(sorted.len() == keys.len() && !keys.contains(&0), "a11y: keys are unique and never 0");

    // a new text and a selection in it in the same poll: the selection counts in the new text
    r.act(txt, Act::Focus);
    let key = txt.map_or(0, |n| n.key);
    ax::request(Request { win: 0, key, act: Act::Text(String::from("\u{1F600}\u{1F600}abc")) });
    ax::request(Request { win: 0, key, act: Act::Select(3, 4) });
    r.step(&[plat::Event { typ: plat::E_WAKE, ..Default::default() }]);
    r.step(&[]);
    t.check(
        find(&r.tree(), Role::TextBox, "Name").is_some_and(|n| n.text == "\u{1F600}\u{1F600}abc" && n.selection() == (3, 4)),
        "a11y: Text then Select in one poll: the selection in the new text",
    );
    t_wire(t, &mut r, chk);
    t.check(!ax::active(), "a11y: no bridge in a headless run");
}

// the browser's and Android's mirror (plat::a11y::wire): a full update, nothing when nothing
// changed, then only the changed node with its event
fn t_wire(t: &mut T, r: &mut Run, chk: Option<&Node>) {
    use alloc::sync::Arc;
    use ax::wire::{self, Mirror};
    // (window count, flags of the update, per window: order count or None, the keys of the
    // nodes sent), the events (kind, key, a, b) -- or None when the bytes do not parse
    type Read = (u8, Vec<(Option<u32>, Vec<u64>)>, Vec<(u8, u64, i32, i32)>);
    fn read(b: &[u8]) -> Option<Read> {
        let mut p = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = b.get(p..p + n)?;
            p += n;
            Some(s)
        };
        fn u32_(s: &[u8]) -> u32 {
            u32::from_le_bytes([s[0], s[1], s[2], s[3]])
        }
        fn u64_(s: &[u8]) -> u64 {
            u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])
        }
        if take(1)?[0] != wire::VERSION {
            return None;
        }
        let flags = take(1)?[0];
        let mut wins = Vec::new();
        for _ in 0..take(1)?[0] {
            take(2 + 16 + 8)?;
            let n = u32_(take(4)?) as usize;
            take(n)?;
            let order = u32_(take(4)?);
            if order != wire::UNCHANGED {
                take(order as usize * 12)?;
            }
            let mut keys = Vec::new();
            for _ in 0..u32_(take(4)?) {
                keys.push(u64_(take(8)?));
                take(1 + 4 + 4 + 16)?;
                for _ in 0..2 {
                    let n = u32_(take(4)?) as usize;
                    take(n)?;
                }
                take(8 + 16 + 32)?;
            }
            wins.push(((order != wire::UNCHANGED).then_some(order), keys));
        }
        let mut events = Vec::new();
        for _ in 0..u32_(take(4)?) {
            let kind = take(1)?[0];
            take(1)?;
            let key = u64_(take(8)?);
            let a = u32_(take(4)?) as i32;
            let c = u32_(take(4)?) as i32;
            if kind == wire::E_TEXT {
                let n = u32_(take(4)?) as usize;
                take(n)?;
            }
            events.push((kind, key, a, c));
        }
        (p == b.len()).then_some((flags, wins, events))
    }
    t.check(
        Role::Dialog as u8 == 20 && Role::Heading as u8 == 21,
        "a11y: wire: the role numbers web/a11y.js and GalleryAccessibility.java read (Dialog 20, Heading 21)",
    );
    let place = |w: &ax::WinTree| [0, 0, w.size[0], w.size[1]];
    let mut m = Mirror::new();
    let s1 = Arc::new(r.tree());
    let n1 = s1.wins[0].nodes.len();
    let full = m.update(&s1, &[], &place).and_then(|b| read(&b));
    t.check(
        full.as_ref().is_some_and(|(f, w, _)| *f == wire::F_FULL && w.len() == 1 && w[0].0 == Some(n1 as u32) && w[0].1.len() == n1),
        "a11y: wire: the first update is full -- every node in order and with its data",
    );
    t.check(m.update(&Arc::new(r.tree()), &[], &place).is_none(), "a11y: wire: nothing changed, nothing to send");
    r.act(chk, Act::Click);
    let s2 = Arc::new(r.tree());
    let ch = ax::diff(&s1, &s2);
    let up = m.update(&s2, &ch, &place).and_then(|b| read(&b));
    let key = chk.map_or(0, |n| n.key);
    t.check(
        up.as_ref().is_some_and(|(f, w, e)| {
            *f == 0 && w[0].0.is_none() && w[0].1.contains(&key) && w[0].1.len() < n1 / 2
                && e.contains(&(wire::E_STATE, key, ax::S_CHECKED as i32, 0))
        }),
        "a11y: wire: a toggle sends the same order, the changed nodes and the state event",
    );
    m.reset();
    t.check(m.update(&s2, &[], &place).and_then(|b| read(&b)).is_some_and(|(f, ..)| f == wire::F_FULL), "a11y: wire: after reset a full update");
}

/// (Linux) the demo in a real window for tools/platshared/atspi_bus.py: the fake screen reader
/// clicks "Drücken", toggles "Option", sets the slider to 75 and the text to "Bert", then
/// presses "Beenden"
pub fn t_a11y_bus(t: &mut T) {
    let mut app = Demo::new();
    let code = ui::run(&DESC, &mut app, &style::STYLE);
    t.expect_i(code as i64, 0, "a11y-bus: the window ran");
    t.expect_i(st(CLICKS) as i64, 1, "a11y-bus: the button was clicked once");
    t.expect_i(st(CHECK) as i64, 1, "a11y-bus: the check box was toggled");
    t.expect_i(st(SLIDER) as i64, 75, "a11y-bus: the slider was set to 75");
    t.expect_s(Some(&app.text), Some("Bert"), "a11y-bus: the text was set");
    t.expect_i(st(TAB) as i64, 1, "a11y-bus: the second tab was selected");
}

#[cfg(not(windows))]
pub fn t_uia(t: &mut T) {
    t.info("  (uia: UI Automation is the Windows bridge; Linux has a11y-bus)\n");
}

#[cfg(not(windows))]
pub fn t_caret(t: &mut T) {
    t.info("  (caret: Linux checks the caret on AT-SPI2 with tools/platshared/atspi_bus.py)\n");
}

#[cfg(windows)]
pub use uia::{t_caret, t_uia};

/// (Windows) the demo with the UI Automation providers, read and driven through IUIAutomation
/// from a thread of the test (plat::uia_test_*)
#[cfg(windows)]
mod uia {
    use alloc::boxed::Box;
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::sync::atomic::{AtomicBool, AtomicIsize, Ordering::SeqCst};

    use plat::sync::Mutex;
    use plat::{UiaTestAct as A, UiaTestElem, uia_test_act, uia_test_caret, uia_test_caret_move, uia_test_system_caret, uia_test_tree};

    use super::*;

    // UIA control types
    const CT_BUTTON: i32 = 50000;
    const CT_CHECKBOX: i32 = 50002;
    const CT_EDIT: i32 = 50004;
    const CT_LISTITEM: i32 = 50007;
    const CT_LIST: i32 = 50008;
    const CT_PROGRESS: i32 = 50012;
    const CT_RADIO: i32 = 50013;
    const CT_SLIDER: i32 = 50015;
    const CT_TAB: i32 = 50018;
    const CT_TABITEM: i32 = 50019;
    const CT_TEXT: i32 = 50020;

    static HWND: AtomicIsize = AtomicIsize::new(0);
    // --test caret: only the caret steps
    static CARET_ONLY: AtomicBool = AtomicBool::new(false);
    static RESULTS: Mutex<Vec<(bool, String)>> = Mutex::new(Vec::new());

    fn record(ok: bool, name: &str) {
        RESULTS.lock().push((ok, String::from(name)));
    }

    // (App::init, UI thread) the window exists: the client starts
    fn start() {
        HWND.store(plat::uia_test_hwnd(0), SeqCst);
        if plat::thread_spawn(0, Box::new(client)).is_none() {
            record(false, "uia: the client thread starts");
        }
    }

    // wait up to 3 s until state k is v
    fn wait_st(k: usize, v: i32) -> bool {
        for _ in 0..60 {
            if st(k) == v {
                return true;
            }
            plat::sleep_ms(50);
        }
        false
    }

    // wait up to 3 s until the tree shows what ok wants
    fn wait_tree(hwnd: isize, ok: impl Fn(&[UiaTestElem]) -> bool) -> bool {
        for _ in 0..60 {
            if uia_test_tree(hwnd).is_some_and(|t| ok(&t)) {
                return true;
            }
            plat::sleep_ms(50);
        }
        false
    }

    fn find<'a>(t: &'a [UiaTestElem], name: &str, ct: i32) -> Option<&'a UiaTestElem> {
        t.iter().find(|e| e.name == name && e.control_type == ct)
    }

    // wait up to 3 s until ok()
    fn wait(mut ok: impl FnMut() -> bool) -> bool {
        for _ in 0..60 {
            if ok() {
                return true;
            }
            plat::sleep_ms(50);
        }
        false
    }

    // (§11) the caret of the text box "Name" (holding text) as screen magnifiers see it: the
    // hidden system caret (GetGUIThreadInfo) and TextPattern2's caret range agree; a selection
    // made by the client moves both
    fn caret(hwnd: isize, text: &str) {
        record(uia_test_act(hwnd, "Name", CT_EDIT, A::Focus), "uia: SetFocus on the text box");
        record(wait(|| uia_test_caret(hwnd, "Name").is_some_and(|c| c.active)), "uia: TextPattern2: the caret is active in the focused box");
        let sys = || uia_test_system_caret(hwnd);
        let range = || uia_test_caret(hwnd, "Name").and_then(|c| c.rects.first().copied());
        record(wait(|| sys().is_some_and(|(h, _)| h == hwnd)), "uia: the hidden system caret is in the window (GetGUIThreadInfo)");
        let edit = uia_test_tree(hwnd).and_then(|t| find(&t, "Name", CT_EDIT).map(|e| e.rect));
        let (Some((_, s0)), Some(r0), Some(e)) = (sys(), range(), edit) else {
            record(false, "uia: the caret's rectangles");
            return;
        };
        record(
            s0[0] > e[0] && s0[2] < e[0] + e[2] && s0[1] >= e[1] && s0[3] <= e[1] + e[3],
            &alloc::format!("uia: the system caret lies inside the text box ({s0:?} in {e:?})"),
        );
        let near = |s: [i32; 4], r: [f64; 4]| {
            (s[0] as f64 - r[0]).abs() <= 2.0 && (s[1] as f64 - r[1]).abs() <= 2.0 && ((s[3] - s[1]) as f64 - r[3]).abs() <= 2.0
        };
        record(near(s0, r0), &alloc::format!("uia: the caret range's rectangle is the system caret's ({r0:?} / {s0:?})"));
        record(uia_test_caret(hwnd, "Name").is_some_and(|c| c.text == text), "uia: TextPattern: the document range's text");
        // three characters back: both move to the left
        record(uia_test_caret_move(hwnd, "Name", -3, 0), "uia: caret range Move(Character, -3), Select");
        record(wait(|| range().is_some_and(|r| r[0] < r0[0] - 2.0)), "uia: the caret range moved to the left");
        record(wait(|| sys().is_some_and(|(_, s)| s[0] < s0[0] - 2 && range().is_some_and(|r| near(s, r)))), "uia: the system caret followed it");
        // two characters selected from there
        let two: String = text.chars().skip(1).take(2).collect();
        record(uia_test_caret_move(hwnd, "Name", 0, 2), "uia: caret range end moved by 2, Select");
        record(wait(|| uia_test_caret(hwnd, "Name").is_some_and(|c| c.selected == two)), &alloc::format!("uia: GetSelection: '{two}'"));
    }

    fn client() {
        let hwnd = HWND.load(SeqCst);
        if CARET_ONLY.load(SeqCst) {
            caret_steps(hwnd);
        } else {
            steps(hwnd);
        }
        // end the demo (a close request if the button is not there)
        if !uia_test_act(hwnd, "Beenden", CT_BUTTON, A::Invoke) {
            plat::evq_push(plat::Event { typ: plat::E_CLOSE, ..Default::default() });
            plat::wake();
        }
    }

    fn steps(hwnd: isize) {
        // the first WM_GETOBJECT turns the bridge on; the tree comes with the next frame
        let mut tree = Vec::new();
        for _ in 0..100 {
            if let Some(t) = uia_test_tree(hwnd) {
                if t.iter().any(|e| e.name == "Drücken") {
                    tree = t;
                    break;
                }
            }
            plat::sleep_ms(100);
        }
        record(!tree.is_empty(), "uia: IUIAutomation reads the window's elements");
        if tree.is_empty() {
            return;
        }
        let t = &tree;
        record(find(t, "Klicks: 0", CT_TEXT).is_some_and(|e| e.heading == 0), "uia: a label is a Text element, no heading");
        record(find(t, "Einstellungen", CT_TEXT).is_some_and(|e| e.heading == 2), "uia: a heading is a Text element with HeadingLevel2");
        record(find(t, "Drücken", CT_BUTTON).is_some_and(|e| e.enabled), "uia: a button, enabled");
        record(find(t, "Gesperrt", CT_BUTTON).is_some_and(|e| !e.enabled), "uia: a disabled button");
        record(find(t, "Option", CT_CHECKBOX).is_some_and(|e| e.toggle == 0), "uia: check box, Toggle off");
        record(find(t, "Schalter", CT_BUTTON).is_some_and(|e| e.toggle == 0), "uia: switch, Toggle off");
        record(
            find(t, "Eins", CT_RADIO).is_some_and(|e| e.selected == Some(true)) && find(t, "Zwei", CT_RADIO).is_some_and(|e| e.selected == Some(false)),
            "uia: radio buttons, SelectionItem",
        );
        record(find(t, "Lautstärke", CT_SLIDER).is_some_and(|e| e.range == Some((30.0, 0.0, 100.0))), "uia: slider, RangeValue 30 in 0 ..= 100");
        record(find(t, "Name", CT_EDIT).is_some_and(|e| e.value.as_deref() == Some("Anna")), "uia: text box, Value 'Anna'");
        record(t.iter().any(|e| e.control_type == CT_PROGRESS && e.range.is_some_and(|r| r.0 == 40.0)), "uia: progress bar, RangeValue 40");
        let tab = t.iter().position(|e| e.control_type == CT_TAB);
        record(
            tab.is_some_and(|k| {
                let d = t[k].depth;
                find(t, "Erste", CT_TABITEM).is_some_and(|e| e.depth == d + 1 && e.selected == Some(true))
                    && find(t, "Zweite", CT_TABITEM).is_some_and(|e| e.depth == d + 1)
            }),
            "uia: the tab strip holds its tabs, the first selected",
        );
        let list = t.iter().position(|e| e.control_type == CT_LIST);
        record(
            list.is_some_and(|k| ["Apfel", "Birne", "Kirsche"].iter().all(|n| find(t, n, CT_LISTITEM).is_some_and(|e| e.depth == t[k].depth + 1))),
            "uia: the list holds its rows",
        );

        // actions through the patterns; the events they cause
        record(plat::uia_test_listen(hwnd), "uia: event handlers registered");
        record(uia_test_act(hwnd, "Drücken", CT_BUTTON, A::Invoke) && wait_st(CLICKS, 1), "uia: Invoke on the button");
        record(wait_tree(hwnd, |t| find(t, "Klicks: 1", CT_TEXT).is_some()), "uia: the label's new name");
        record(uia_test_act(hwnd, "Option", CT_CHECKBOX, A::Toggle) && wait_st(CHECK, 1), "uia: Toggle on the check box");
        record(wait_tree(hwnd, |t| find(t, "Option", CT_CHECKBOX).is_some_and(|e| e.toggle == 1)), "uia: the check box's toggle state");
        record(uia_test_act(hwnd, "Schalter", CT_BUTTON, A::Toggle) && wait_st(SWITCH, 1), "uia: Toggle on the switch");
        record(uia_test_act(hwnd, "Zwei", CT_RADIO, A::Select) && wait_st(RADIO, 1), "uia: Select on a radio button");
        record(uia_test_act(hwnd, "Lautstärke", CT_SLIDER, A::SetRange(75.0)) && wait_st(SLIDER, 75), "uia: RangeValue.SetValue(75)");
        record(
            uia_test_act(hwnd, "Name", CT_EDIT, A::SetValue(String::from("Bert")))
                && wait_tree(hwnd, |t| find(t, "Name", CT_EDIT).is_some_and(|e| e.value.as_deref() == Some("Bert"))),
            "uia: Value.SetValue('Bert')",
        );
        caret(hwnd, "Bert");
        record(uia_test_act(hwnd, "Zweite", CT_TABITEM, A::Select) && wait_st(TAB, 1), "uia: Select on a tab");
        record(uia_test_act(hwnd, "Kirsche", CT_LISTITEM, A::Select) && wait_st(LIST, 2), "uia: Select on a list row");
        record(
            uia_test_act(hwnd, "Option", CT_CHECKBOX, A::Focus) && wait_tree(hwnd, |t| find(t, "Option", CT_CHECKBOX).is_some_and(|e| e.focused)),
            "uia: SetFocus on the check box",
        );
        record(wait(|| plat::uia_test_system_caret(hwnd).is_none()), "uia: no system caret without a focused text box");
        let clicks = st(CLICKS);
        record(!uia_test_act(hwnd, "Gesperrt", CT_BUTTON, A::Invoke) && st(CLICKS) == clicks, "uia: Invoke on a disabled button fails");
        record(uia_test_act(hwnd, "Eigen", CT_BUTTON, A::Invoke) && wait_st(OWN, 1), "uia: Invoke on a widget of its own (pointer click)");
        let mut ev: Vec<String> = Vec::new();
        let want = [
            "prop:30005:Klicks: 1=Klicks: 1",
            "prop:30086:Option=1",
            "prop:30047:Lautstärke=75",
            "prop:30045:Name=Bert",
            "prop:30079:Zweite=1",
            "focus:Option",
            "event:20015:Name", // TextChanged
            "event:20014:Name", // TextSelectionChanged: the caret moved
        ];
        for _ in 0..60 {
            ev.extend(plat::uia_test_events());
            if want.iter().all(|w| ev.iter().any(|e| e == w)) {
                break;
            }
            plat::sleep_ms(50);
        }
        plat::uia_test_unlisten();
        for w in want {
            let ok = ev.iter().any(|e| e == w);
            record(ok, &alloc::format!("uia: event {w}"));
            if !ok {
                plat::write_stdout(alloc::format!("  events: {ev:?}\n").as_bytes());
            }
        }
    }

    // --test caret: the tree, then the caret steps and their events
    fn caret_steps(hwnd: isize) {
        record(wait(|| uia_test_tree(hwnd).is_some_and(|t| find(&t, "Name", CT_EDIT).is_some())), "uia: IUIAutomation reads the text box");
        record(plat::uia_test_listen(hwnd), "uia: event handlers registered");
        caret(hwnd, "Anna");
        let mut ev: Vec<String> = Vec::new();
        record(
            wait(|| {
                ev.extend(plat::uia_test_events());
                ev.iter().any(|e| e == "event:20014:Name")
            }),
            "uia: event TextSelectionChanged of the text box",
        );
        plat::uia_test_unlisten();
    }

    pub fn t_caret(t: &mut T) {
        CARET_ONLY.store(true, SeqCst);
        let mut app = Demo::new();
        app.on_init = Some(start);
        let code = ui::run(&DESC, &mut app, &style::STYLE);
        let results = core::mem::take(&mut *RESULTS.lock());
        t.check(!results.is_empty(), "caret: the client ran");
        for (ok, name) in results {
            t.check(ok, &name);
        }
        t.expect_i(code as i64, 0, "caret: the window ran");
    }

    pub fn t_uia(t: &mut T) {
        let mut app = Demo::new();
        app.on_init = Some(start);
        let code = ui::run(&DESC, &mut app, &style::STYLE);
        let results = core::mem::take(&mut *RESULTS.lock());
        t.check(!results.is_empty(), "uia: the client ran");
        for (ok, name) in results {
            t.check(ok, &name);
        }
        t.expect_i(code as i64, 0, "uia: the window ran");
        t.expect_s(Some(&app.text), Some("Bert"), "uia: the text box's text in the app");
    }
}
