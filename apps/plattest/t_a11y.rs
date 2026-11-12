// t_a11y -- the accessibility tree for screen readers (lib/ui/a11y.rs, lib/plat/a11y.rs)
//
//   plattest --test a11y       (headless) a demo window as a ui::Runtime: the tree of its
//                              records (roles, a heading and its level, names from labels and
//                              from the text before a slider / text box, states, values, tab
//                              strip and list containers, the focus), then actions as a screen
//                              reader asks them through plat::a11y::request: click, toggle,
//                              select, set a slider and a text, focus, a disabled button, the
//                              pointer click for a widget whose record id differs from its hit
//                              box and where that click must not go (a record out of view, cut
//                              off, under an open popup, a link's box of another id); the changes
//                              between two trees (plat::a11y::diff); the role names of the bridges
//                              in German and English
//   plattest --test a11y-bus   (Linux; tools/platshared/atspi_bus.py starts it on a fake session
//                              and accessibility bus) the demo in a real window with the AT-SPI2
//                              bridge; the fake screen reader reads the tree and acts, then
//                              presses "Beenden": the app state is checked at the end; with
//                              --lang CODE in that UI language (the localized role names)
//   plattest --test uia        (Windows, by hand: a window appears for a few seconds) the demo
//                              with the UI Automation providers; a thread of the test reads the
//                              tree through IUIAutomation (plat::uia_test_*: control types,
//                              patterns, the heading's HeadingLevel, the localized control
//                              types of a switch and a heading in German, then English and
//                              French after it changes the UI language), invokes, toggles,
//                              sets values, follows the caret, then presses "Beenden"
//   plattest --test caret      (Windows, by hand) only the caret as screen magnifiers see it:
//                              the hidden system caret (GetGUIThreadInfo) and TextPattern2's
//                              caret range of the focused text box agree and follow a
//                              selection the client makes; TextSelectionChanged arrives.  Linux:
//                              the caret steps of tools/platshared/atspi_bus.py (a11y-bus)
//   plattest --test a11y-show  (by hand) the demo in a real window until it is closed: probes with
//                              a screen reader; --lang CODE in that UI language
//   plattest --test a11y-ios   (iOS, under UIKit: tools/apple.py simapp) the demo with the
//                              UIAccessibility elements, read and driven as VoiceOver does from a
//                              thread of the test (plat::ios_ax_*): traits, values, containers,
//                              double taps and swipes with their result at once, the text field
//                              through UITextInput, the layout change of the focus
//   plattest --test a11y-mac   (macOS, a real window for a few seconds: tools/apple.py gui) the
//                              demo with the NSAccessibility elements; a thread of the test reads
//                              the tree through AppKit's own entry points on the main thread
//                              (accessibilityAttributeValue: and the actions, which serve VoiceOver's
//                              requests: plat::ax_test_*): roles, subroles, names, values, frames,
//                              the hit test and the focused element, then it presses, sets values,
//                              selects and focuses, reads the text box by character and line and
//                              moves its caret, checks the notifications, presses "Beenden"

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

    // the names the bridges give the roles (UIA LocalizedControlType, AT-SPI
    // GetLocalizedRoleName): ui hands plat its translation, they follow the UI language
    let names = || (ax::role_name(Role::Switch), ax::role_name(Role::Heading), ax::role_text(ax::ROLE_WINDOW));
    t.check(names() == ("Umschalter", "Überschrift", "Rahmen"), "a11y: role names in German (the default)");
    ui::i18n::set_language_code("en");
    t.check(names() == ("toggle switch", "heading", "frame"), "a11y: role names in English after the language changed");
    ui::i18n::set_language(ui::i18n::LANG_DE);
    t.check(names() == ("Umschalter", "Überschrift", "Rahmen"), "a11y: role names in German again");
    t_reach(t);
}

const ID_VIEW: u32 = 0x400;
const ID_LOW_HIT: u32 = 0x401;
const ID_LOW: u32 = 0x402;
const ID_AFTER_VIEW: u32 = 0x403;
const ID_CUT_HIT: u32 = 0x404;
const ID_CUT: u32 = 0x405;
const ID_AFTER_CUT: u32 = 0x406;
const ID_DOC: u32 = 0x407;
const ID_DROP: u32 = 0x408;
const ID_ITEM: u32 = 0x409;

/// records whose id is not their hit box, and what lies next to them: where their pointer click
/// goes (the counters: presses each widget got)
#[derive(Default)]
struct Reach {
    low: i32,
    after_view: i32,
    cut: i32,
    after_cut: i32,
    doc: i32,
}

impl App for Reach {
    fn frame(&mut self, ui: &mut Ui) {
        // below the fold of a small view: the record's middle on the button after the view
        ui.next_size(-1, 40);
        ui.scroll_begin(ID_VIEW, ui::SC_V);
        ui.place(ui.dp(120), ui.dp(44));
        let r = ui.place(ui.dp(120), ui.dp(28));
        self.low += (ui.hit_rect(ID_LOW_HIT, r[0], r[1], r[2], r[3], ui::HB_FOCUS) & ui::EV_ACT != 0) as i32;
        ui.sem(ui::K_BUTTON, ID_LOW, Some("Unten"), 0);
        ui.scroll_end();
        ui.next_size(-1, 0);
        self.after_view += (ui.button(ID_AFTER_VIEW, "Nach der Ansicht") & ui::EV_ACT != 0) as i32;
        // cut off by a box that cannot scroll: a record with an id, a link without one
        ui.next_size(-1, 28);
        ui.box_begin(0, ui::BOX_V | ui::BOX_CLIP, 0);
        ui.place(ui.dp(120), ui.dp(32));
        let r = ui.place(ui.dp(120), ui.dp(28));
        self.cut += (ui.hit_rect(ID_CUT_HIT, r[0], r[1], r[2], r[3], ui::HB_FOCUS) & ui::EV_ACT != 0) as i32;
        ui.sem(ui::K_BUTTON, ID_CUT, Some("Abgeschnitten"), 0);
        ui.sem(ui::K_LINK, 0, Some("Abgeschnittener Verweis"), 0);
        ui.box_end();
        ui.next_size(-1, 0);
        self.after_cut += (ui.button(ID_AFTER_CUT, "Nach dem Ausschnitt") & ui::EV_ACT != 0) as i32;
        // a link of a document: no id, its box has the document's
        let r = ui.place(ui.dp(120), ui.dp(24));
        self.doc += (ui.hit_rect(ID_DOC, r[0], r[1], r[2], r[3], ui::HB_FOCUS) & ui::EV_ACT != 0) as i32;
        ui.sem(ui::K_LINK, 0, Some("Verweis"), 0);
        // an open popup takes every press
        ui.dropdown_button(ID_DROP, "Mehr");
        if ui.flyout_begin(ID_DROP, ui::FL_BELOW) {
            ui.menu_item(ID_ITEM, "Eintrag");
        }
        ui.flyout_end();
        for (k, v) in [self.low, self.after_view, self.cut, self.after_cut, self.doc].into_iter().enumerate() {
            REACH[k].store(v, SeqCst);
        }
    }
}

static REACH: [AtomicI32; 5] = [const { AtomicI32::new(0) }; 5];

// a click no widget takes goes where it reaches the record, or nowhere
fn t_reach(t: &mut T) {
    let presses = || REACH.iter().map(|v| v.load(SeqCst)).collect::<Vec<i32>>();
    let rt = match ui::Runtime::new(&DESC, Reach::default(), &style::STYLE) {
        Ok(rt) => rt,
        Err(_) => {
            t.check(false, "a11y reach: the window starts headless");
            return;
        }
    };
    let mut r = Run { rt, now: 0 };
    let s = r.tree();
    let middle = |n: &Node| (n.rect[0] + n.rect[2] / 2, n.rect[1] + n.rect[3] / 2);
    let holds = |n: &Node, p: (i32, i32)| p.0 >= n.rect[0] && p.0 < n.rect[0] + n.rect[2] && p.1 >= n.rect[1] && p.1 < n.rect[1] + n.rect[3];
    let (low, after_view) = (find(&s, Role::Button, "Unten"), find(&s, Role::Button, "Nach der Ansicht"));
    t.check(low.zip(after_view).is_some_and(|(l, a)| holds(a, middle(l))), "a11y reach: the middle of the record out of view lies on the next button");
    let (cut, after_cut) = (find(&s, Role::Button, "Abgeschnitten"), find(&s, Role::Button, "Nach dem Ausschnitt"));
    t.check(cut.zip(after_cut).is_some_and(|(c, a)| holds(a, middle(c))), "a11y reach: the middle of the cut off record lies on the next button");

    // out of view: the view brings it in, the press is its own
    r.act(low, Act::Click);
    r.step(&[]);
    t.check(presses() == [1, 0, 0, 0, 0], &format!("a11y reach: a record out of view is scrolled in and pressed ({:?})", presses()));
    t.check(find(&r.tree(), Role::Button, "Unten").zip(find(&s, Role::Button, "Unten")).is_some_and(|(now, was)| now.rect[1] < was.rect[1]), "a11y reach: the view scrolled");
    // cut off, with an id and without: the press would be the next button's, so there is none
    r.act(cut, Act::Click);
    r.step(&[]);
    t.check(presses() == [1, 0, 0, 0, 0], &format!("a11y reach: no press for a record that is cut off ({:?})", presses()));
    r.act(find(&s, Role::Link, "Abgeschnittener Verweis"), Act::Click);
    r.step(&[]);
    t.check(presses() == [1, 0, 0, 0, 0], &format!("a11y reach: no press for a link that is cut off ({:?})", presses()));
    // a link without an id: the box of the document that draws it
    r.act(find(&s, Role::Link, "Verweis"), Act::Click);
    r.step(&[]);
    t.check(presses() == [1, 0, 0, 0, 1], &format!("a11y reach: a link is pressed through the box of its document ({:?})", presses()));
    // an open popup: no press (it would only close it), nothing scrolls
    let drop = find(&s, Role::Combo, "Mehr").or_else(|| find(&s, Role::Button, "Mehr"));
    r.act(drop, Act::Click);
    r.step(&[]);
    t.expect_i(r.rt.ui().popup_count() as i64, 1, "a11y reach: the popup is open");
    r.act(find(&r.tree(), Role::Link, "Verweis"), Act::Click);
    r.step(&[]);
    t.expect_i(r.rt.ui().popup_count() as i64, 1, "a11y reach: a click under it does not close it");
    t.check(presses() == [1, 0, 0, 0, 1], &format!("a11y reach: no press under an open popup ({:?})", presses()));
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

// --lang CODE: the UI language (the library's texts follow it; the demo has no catalogue of its
// own, so the user's languages alone do not change it)
fn lang_arg() {
    if let Some(l) = plat::arg_value("--lang") {
        ui::i18n::set_language_code(&l);
    }
}

/// (Linux) the demo in a real window for tools/platshared/atspi_bus.py: the fake screen reader
/// clicks "Drücken", toggles "Option", sets the slider to 75 and the text to "Bert", then
/// presses "Beenden"; --lang en: the role names in English
pub fn t_a11y_bus(t: &mut T) {
    lang_arg();
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

/// (by hand) the demo in a real window until it is closed or "Beenden" is pressed: for probes
/// with a screen reader (VoiceOver, Narrator, Orca); --lang CODE: the UI language
pub fn t_a11y_show(t: &mut T) {
    lang_arg();
    let mut app = Demo::new();
    let code = ui::run(&DESC, &mut app, &style::STYLE);
    t.expect_i(code as i64, 0, "a11y-show: the window ran");
}

#[cfg(not(target_os = "macos"))]
pub fn t_a11y_mac(t: &mut T) {
    t.info("  (a11y-mac: NSAccessibility is the macOS bridge)\n");
}

#[cfg(target_os = "macos")]
pub use mac::t_a11y_mac;

#[cfg(not(target_os = "ios"))]
pub fn t_a11y_ios(t: &mut T) {
    t.info("  (a11y-ios: UIAccessibility is the iOS bridge; tools/apple.py simapp runs it)\n");
}

#[cfg(target_os = "ios")]
pub use ios::t_a11y_ios;

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
        // the control types the bridge names itself: in the UI language, following a change
        let types = |t: &[UiaTestElem]| {
            let ty = |name: &str, ct: i32| find(t, name, ct).map_or(String::new(), |e| e.localized_type.clone());
            (ty("Schalter", CT_BUTTON), ty("Einstellungen", CT_TEXT))
        };
        let de = types(t);
        record(de.0 == "Umschalter" && de.1 == "Überschrift", &alloc::format!("uia: LocalizedControlType of a switch and a heading in German ({de:?})"));
        record(find(t, "Drücken", CT_BUTTON).is_some_and(|e| !e.localized_type.is_empty()), "uia: a button keeps UIA's own LocalizedControlType");
        // (a catalogue with French: without one the library has German and English)
        ui::i18n::set_catalog(ui::i18n::Catalog::from_tsv("de\ten\tfr\n"));
        for (lang, want) in [("en", ("toggle switch", "heading")), ("fr", ("interrupteur", "titre"))] {
            ui::i18n::set_language_code(lang);
            let got = uia_test_tree(hwnd).map(|t| types(&t)).unwrap_or_default();
            record(got.0 == want.0 && got.1 == want.1, &alloc::format!("uia: LocalizedControlType in {lang} after the language changed ({got:?})"));
        }
        ui::i18n::set_language(ui::i18n::LANG_DE);
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

/// (macOS) the demo with the NSAccessibility elements, read and driven through AppKit's entry
/// points for assistive technology from a thread of the test (plat::ax_test_*)
#[cfg(target_os = "macos")]
mod mac {
    use alloc::boxed::Box;
    use alloc::string::String;
    use alloc::vec::Vec;

    use plat::sync::Mutex;
    use plat::{AxTestAct as A, AxTestElem, AxTestText as Q, ax_test_act, ax_test_events, ax_test_focused, ax_test_hit, ax_test_listen, ax_test_text, ax_test_tree};

    use super::*;

    static RESULTS: Mutex<Vec<(bool, String)>> = Mutex::new(Vec::new());
    // the demo's platform window (the main window)
    const PW: i32 = 0;

    fn record(ok: bool, name: &str) {
        RESULTS.lock().push((ok, String::from(name)));
    }

    // (App::init, UI thread) the window exists: the client starts
    fn start() {
        if plat::thread_spawn(0, Box::new(client)).is_none() {
            record(false, "a11y-mac: the client thread starts");
        }
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

    fn wait_st(k: usize, v: i32) -> bool {
        wait(|| st(k) == v)
    }

    fn find<'a>(t: &'a [AxTestElem], role: &str, name: &str) -> Option<&'a AxTestElem> {
        t.iter().find(|e| e.role == role && e.name() == name)
    }

    fn wait_tree(ok: impl Fn(&[AxTestElem]) -> bool) -> bool {
        wait(|| ax_test_tree(PW).is_some_and(|t| ok(&t)))
    }

    fn client() {
        steps();
        if !ax_test_act(PW, "AXButton", "Beenden", A::Press) {
            plat::evq_push(plat::Event { typ: plat::E_CLOSE, ..Default::default() });
            plat::wake();
        }
    }

    fn steps() {
        // the first question turns the bridge on; the tree comes with the next frame
        let mut tree = Vec::new();
        let mut last = None;
        for _ in 0..100 {
            last = ax_test_tree(PW);
            if let Some(t) = &last {
                if t.iter().any(|e| e.title == "Drücken") {
                    tree = t.clone();
                    break;
                }
            }
            plat::sleep_ms(100);
        }
        record(!tree.is_empty(), "a11y-mac: AppKit serves the window's elements");
        if tree.is_empty() {
            plat::write_stdout(alloc::format!("  the window's elements: {last:?}\n").as_bytes());
            return;
        }
        let t = &tree;
        record(plat::a11y::active(), "a11y-mac: the question turned the bridge on");
        record(find(t, "AXStaticText", "Klicks: 0").is_some_and(|e| e.title.is_empty()), "a11y-mac: a label is static text, its text the value");
        let head = find(t, "AXHeading", "Einstellungen");
        record(head.is_some_and(|e| e.value == "2"), "a11y-mac: a heading is AXHeading with its level as value");
        record(head.is_some_and(|e| e.role_desc == "Überschrift"), "a11y-mac: the heading's role description in the UI language");
        let btn = find(t, "AXButton", "Drücken");
        record(btn.is_some_and(|e| e.enabled && e.actions.iter().any(|a| a == "AXPress")), "a11y-mac: a button, enabled, AXPress");
        record(btn.is_some_and(|e| !e.attrs.iter().any(|a| a == "AXValue")), "a11y-mac: a button has no value");
        record(find(t, "AXButton", "Gesperrt").is_some_and(|e| !e.enabled), "a11y-mac: a disabled button");
        record(find(t, "AXCheckBox", "Option").is_some_and(|e| e.value == "0" && e.subrole.is_empty()), "a11y-mac: check box, value 0");
        record(
            find(t, "AXRadioButton", "Eins").is_some_and(|e| e.value == "1") && find(t, "AXRadioButton", "Zwei").is_some_and(|e| e.value == "0"),
            "a11y-mac: radio buttons, the first one on",
        );
        record(find(t, "AXCheckBox", "Schalter").is_some_and(|e| e.subrole == "AXSwitch" && e.value == "0"), "a11y-mac: a switch: check box / AXSwitch");
        let sld = find(t, "AXSlider", "Lautstärke");
        record(
            sld.is_some_and(|e| e.value == "30" && e.value_settable && e.actions.iter().any(|a| a == "AXIncrement")),
            "a11y-mac: slider, value 30, settable, AXIncrement",
        );
        record(find(t, "AXTextField", "Name").is_some_and(|e| e.value == "Anna" && e.value_settable), "a11y-mac: text field, its text, settable");
        record(t.iter().any(|e| e.role == "AXProgressIndicator" && e.value == "40"), "a11y-mac: progress indicator, value 40");
        let tab = t.iter().position(|e| e.role == "AXTabGroup");
        record(
            tab.is_some_and(|k| {
                let d = t[k].depth;
                find(t, "AXRadioButton", "Erste").is_some_and(|e| e.depth == d + 1 && e.subrole == "AXTabButton" && e.value == "1")
                    && find(t, "AXRadioButton", "Zweite").is_some_and(|e| e.depth == d + 1 && e.value == "0")
            }),
            "a11y-mac: the tab group holds its tabs (AXTabButton), the first selected",
        );
        let list = t.iter().position(|e| e.role == "AXList");
        record(
            list.is_some_and(|k| FRUIT.iter().all(|n| find(t, "AXStaticText", n).is_some_and(|e| e.depth == t[k].depth + 1))),
            "a11y-mac: the list holds its rows",
        );
        record(find(t, "AXStaticText", "Apfel").is_some_and(|e| e.selected), "a11y-mac: the first row is selected");
        // the frame: inside the window's client area (points from the top left of the screen)
        let client = plat::win_get_rect(PW).unwrap_or([0; 4]);
        let scale = plat::win_dpi(PW) as f64 / 96.0;
        let c = [client[0] as f64 / scale, client[1] as f64 / scale, client[2] as f64 / scale, client[3] as f64 / scale];
        let f = btn.map_or([0.0; 4], |e| e.frame);
        record(
            f[2] > 10.0 && f[3] > 10.0 && f[0] >= c[0] && f[1] >= c[1] && f[0] + f[2] <= c[0] + c[2] + 0.5 && f[1] + f[3] <= c[1] + c[3] + 0.5,
            &alloc::format!("a11y-mac: the button's frame lies in the window ({f:?} in {c:?})"),
        );
        let hit = ax_test_hit(f[0] + f[2] / 2.0, f[1] + f[3] / 2.0);
        record(hit.as_ref().is_some_and(|h| h.0 == "AXButton" && h.1 == "Drücken"), &alloc::format!("a11y-mac: the hit test finds the button ({hit:?})"));

        // actions as VoiceOver sends them; the notifications they cause
        ax_test_listen(true);
        record(ax_test_act(PW, "AXButton", "Drücken", A::Press) && wait_st(CLICKS, 1), "a11y-mac: AXPress on the button");
        record(wait_tree(|t| find(t, "AXStaticText", "Klicks: 1").is_some()), "a11y-mac: the label's new text");
        record(ax_test_act(PW, "AXCheckBox", "Option", A::Press) && wait_st(CHECK, 1), "a11y-mac: AXPress on the check box");
        record(wait_tree(|t| find(t, "AXCheckBox", "Option").is_some_and(|e| e.value == "1")), "a11y-mac: the check box's value 1");
        record(ax_test_act(PW, "AXCheckBox", "Schalter", A::Press) && wait_st(SWITCH, 1), "a11y-mac: AXPress on the switch");
        record(ax_test_act(PW, "AXRadioButton", "Zwei", A::Press) && wait_st(RADIO, 1), "a11y-mac: AXPress on a radio button");
        record(ax_test_act(PW, "AXSlider", "Lautstärke", A::Number(75.0)) && wait_st(SLIDER, 75), "a11y-mac: AXValue of the slider := 75");
        record(wait_tree(|t| find(t, "AXSlider", "Lautstärke").is_some_and(|e| e.value == "75")), "a11y-mac: the slider's new value");
        record(ax_test_act(PW, "AXSlider", "Lautstärke", A::Increment) && wait_st(SLIDER, 80), "a11y-mac: AXIncrement: 5 % up");
        record(wait_tree(|t| find(t, "AXSlider", "Lautstärke").is_some_and(|e| e.value == "80")), "a11y-mac: the slider at 80");
        record(ax_test_act(PW, "AXSlider", "Lautstärke", A::Decrement) && wait_st(SLIDER, 75), "a11y-mac: AXDecrement: 5 % down");
        record(
            ax_test_act(PW, "AXTextField", "Name", A::Text(String::from("Bert")))
                && wait_tree(|t| find(t, "AXTextField", "Name").is_some_and(|e| e.value == "Bert")),
            "a11y-mac: AXValue of the text field := 'Bert'",
        );
        text_steps();
        record(ax_test_act(PW, "AXRadioButton", "Zweite", A::Select) && wait_st(TAB, 1), "a11y-mac: AXSelected of a tab");
        record(ax_test_act(PW, "AXStaticText", "Kirsche", A::Select) && wait_st(LIST, 2), "a11y-mac: AXSelected of a list row");
        record(
            ax_test_act(PW, "AXCheckBox", "Option", A::Focus) && wait_tree(|t| find(t, "AXCheckBox", "Option").is_some_and(|e| e.focused)),
            "a11y-mac: AXFocused of the check box",
        );
        let focused = ax_test_focused();
        record(
            focused.as_ref().is_some_and(|f| f.0 == "AXCheckBox" && f.1 == "Option"),
            &alloc::format!("a11y-mac: the application's focused element ({focused:?})"),
        );
        let clicks = st(CLICKS);
        record(!ax_test_act(PW, "AXButton", "Gesperrt", A::Press) && st(CLICKS) == clicks, "a11y-mac: no AXPress on a disabled button");
        record(ax_test_act(PW, "AXButton", "Eigen", A::Press) && wait_st(OWN, 1), "a11y-mac: AXPress on a widget of its own (pointer click)");
        let mut ev: Vec<String> = Vec::new();
        let want = [
            "AXValueChanged:Klicks: 1",
            "AXValueChanged:Option",
            "AXValueChanged:Lautstärke",
            "AXValueChanged:Name",
            "AXValueChanged:Zweite",
            "AXFocusedUIElementChanged:Option",
            "AXSelectedTextChanged:Name",
        ];
        wait(|| {
            ev.extend(ax_test_events());
            want.iter().all(|w| ev.iter().any(|e| e == w))
        });
        ax_test_listen(false);
        for w in want {
            let ok = ev.iter().any(|e| e == w);
            record(ok, &alloc::format!("a11y-mac: notification {w}"));
            if !ok {
                plat::write_stdout(alloc::format!("  notifications: {ev:?}\n").as_bytes());
            }
        }
    }

    // "location,length" -> (location, length) (-1, -1: none)
    fn range(s: &str) -> (i64, i64) {
        let mut it = s.split(',').map(|x| x.trim().parse::<i64>().unwrap_or(-1));
        (it.next().unwrap_or(-1), it.next().unwrap_or(-1))
    }

    // "x,y,w,h" -> the rectangle
    fn rect(s: &str) -> [f64; 4] {
        let mut r = [0.0; 4];
        for (k, x) in s.split(',').take(4).enumerate() {
            r[k] = x.trim().parse::<f64>().unwrap_or(0.0);
        }
        r
    }

    // the text field as VoiceOver reads it: characters, lines, the caret and the selection
    // (UTF-16 offsets), the rectangles of characters, a point back to its character
    fn text_steps() {
        let q = |x: Q| ax_test_text(PW, "AXTextField", "Name", x).unwrap_or_default();
        let sel = || range(&q(Q::SelRange));
        record(
            ax_test_act(PW, "AXTextField", "Name", A::Focus) && wait_tree(|t| find(t, "AXTextField", "Name").is_some_and(|e| e.focused)),
            "a11y-mac: AXFocused of the text field",
        );
        record(q(Q::Count) == "4", "a11y-mac: text: AXNumberOfCharacters 4");
        record(
            wait(|| {
                let (a, n) = sel();
                a >= 0 && a + n == 4
            }),
            &alloc::format!("a11y-mac: text: the selection ends at the caret after 'Bert' ({:?})", sel()),
        );
        record(q(Q::Line) == "0", "a11y-mac: text: AXInsertionPointLineNumber 0");
        record(q(Q::StringFor(1, 2)) == "er" && q(Q::AttrFor(0, 4)) == "Bert", "a11y-mac: text: AXStringForRange, AXAttributedStringForRange");
        record(q(Q::LineFor(3)) == "0" && q(Q::RangeForLine(0)) == "0,4", "a11y-mac: text: one line of four characters");
        record(q(Q::RangeFor(2)) == "2,1", "a11y-mac: text: AXRangeForIndex");
        record(
            ax_test_act(PW, "AXTextField", "Name", A::Range(1, 0)) && wait(|| sel() == (1, 0)),
            "a11y-mac: text: AXSelectedTextRange := 1,0 moves the caret",
        );
        // the rectangles of characters: in the field, from left to right; a point finds its
        // character again
        let field = ax_test_tree(PW).and_then(|t| find(&t, "AXTextField", "Name").map(|e| e.frame)).unwrap_or([0.0; 4]);
        let (r1, r2) = (rect(&q(Q::FrameFor(1, 1))), rect(&q(Q::FrameFor(2, 1))));
        let inside = |r: [f64; 4]| {
            r[2] > 0.0 && r[3] > 0.0 && r[0] >= field[0] - 0.5 && r[0] + r[2] <= field[0] + field[2] + 0.5 && r[1] >= field[1] - 0.5
                && r[1] + r[3] <= field[1] + field[3] + 0.5
        };
        record(
            inside(r1) && inside(r2) && r1[0] < r2[0],
            &alloc::format!("a11y-mac: text: AXBoundsForRange in the field, left to right ({r1:?}, {r2:?} in {field:?})"),
        );
        let at = q(Q::RangeAt(r2[0] + r2[2] / 4.0, r2[1] + r2[3] / 2.0));
        record(at == "2,1", &alloc::format!("a11y-mac: text: AXRangeForPosition finds the character again ({at})"));
        record(
            ax_test_act(PW, "AXTextField", "Name", A::Range(0, 3)) && wait(|| q(Q::Selected) == "Ber"),
            "a11y-mac: text: AXSelectedTextRange := 0,3 selects 'Ber'",
        );
        // characters outside the basic plane: two UTF-16 units each
        record(
            ax_test_act(PW, "AXTextField", "Name", A::Text(String::from("\u{1F600}\u{1F600}abc"))) && wait(|| q(Q::Count) == "7"),
            "a11y-mac: text: two emoji and three letters are 7 UTF-16 units",
        );
        record(
            q(Q::StringFor(4, 1)) == "a" && q(Q::RangeFor(1)) == "0,2" && q(Q::StringFor(2, 2)) == "\u{1F600}",
            "a11y-mac: text: a character outside the basic plane is one range of two units",
        );
        record(
            ax_test_act(PW, "AXTextField", "Name", A::Range(5, 0)) && wait(|| sel() == (5, 0)),
            &alloc::format!("a11y-mac: text: the caret after 'a' at unit 5 ({:?})", sel()),
        );
        record(
            ax_test_act(PW, "AXTextField", "Name", A::Text(String::from("Bert"))) && wait(|| q(Q::Count) == "4"),
            "a11y-mac: text: back to 'Bert'",
        );
    }

    pub fn t_a11y_mac(t: &mut T) {
        let mut app = Demo::new();
        app.on_init = Some(start);
        let code = ui::run(&DESC, &mut app, &style::STYLE);
        let results = core::mem::take(&mut *RESULTS.lock());
        t.check(!results.is_empty(), "a11y-mac: the client ran");
        for (ok, name) in results {
            t.check(ok, &name);
        }
        t.expect_i(code as i64, 0, "a11y-mac: the window ran");
        t.expect_s(Some(&app.text), Some("Bert"), "a11y-mac: the text field's text in the app");
    }
}

/// (iOS) the demo under UIKit's host with the UIAccessibility elements, read and driven as
/// VoiceOver does from a thread of the test (plat::ios_ax_*); the host runs the app after main
/// returned, so the client prints its checks itself and ends with "Beenden"
#[cfg(target_os = "ios")]
mod ios {
    use alloc::boxed::Box;
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::sync::atomic::{AtomicI32, Ordering::SeqCst};

    use plat::{IosAxAct as A, IosAxElem, IosAxText as Q, ios_ax_act, ios_ax_events, ios_ax_listen, ios_ax_select, ios_ax_text, ios_ax_tree, ios_ax_type};

    use super::*;

    static CHECKS: AtomicI32 = AtomicI32::new(0);
    static FAILS: AtomicI32 = AtomicI32::new(0);

    fn record(ok: bool, name: &str) {
        CHECKS.fetch_add(1, SeqCst);
        if !ok {
            FAILS.fetch_add(1, SeqCst);
        }
        plat::write_stdout(alloc::format!("{} {name}\n", if ok { "PASS" } else { "FAIL" }).as_bytes());
    }

    fn start() {
        if plat::thread_spawn(0, Box::new(client)).is_none() {
            record(false, "a11y-ios: the client thread starts");
        }
    }

    fn wait(mut ok: impl FnMut() -> bool) -> bool {
        for _ in 0..60 {
            if ok() {
                return true;
            }
            plat::sleep_ms(50);
        }
        false
    }

    fn find<'a>(t: &'a [IosAxElem], label: &str) -> Option<&'a IosAxElem> {
        t.iter().find(|e| e.element && e.label == label)
    }

    // the element with this label and trait (a slider after the text that names it)
    fn find_tr<'a>(t: &'a [IosAxElem], label: &str, tr: &str) -> Option<&'a IosAxElem> {
        t.iter().find(|e| e.element && e.label == label && e.traits.contains(&tr))
    }

    fn field(t: &[IosAxElem]) -> Option<&IosAxElem> {
        t.iter().find(|e| e.element && e.label == "Name" && e.text_input)
    }

    fn has(e: &IosAxElem, tr: &str) -> bool {
        e.traits.contains(&tr)
    }

    fn tree() -> Vec<IosAxElem> {
        ios_ax_tree().unwrap_or_default()
    }

    fn client() {
        steps();
        let (n, f) = (CHECKS.load(SeqCst), FAILS.load(SeqCst));
        plat::write_stdout(alloc::format!("a11y-ios: {n} checks, {f} failed{}\n", if f != 0 { " (check(s) failed)" } else { "" }).as_bytes());
        if !ios_ax_act("Beenden", "", A::Activate) {
            plat::evq_push(plat::Event { typ: plat::E_CLOSE, ..Default::default() });
            plat::wake();
        }
    }

    fn steps() {
        // the first question turns the bridge on; the tree comes with the next frame
        let mut t = Vec::new();
        for _ in 0..100 {
            t = tree();
            if find(&t, "Drücken").is_some() {
                break;
            }
            plat::sleep_ms(100);
        }
        record(find(&t, "Drücken").is_some(), "a11y-ios: the view's accessibilityElements hold the demo");
        if find(&t, "Drücken").is_none() {
            plat::write_stdout(alloc::format!("  elements: {t:?}\n").as_bytes());
            return;
        }
        let t = &t;
        record(plat::a11y::active(), "a11y-ios: the question turned the bridge on");
        record(find(t, "Einstellungen").is_some_and(|e| has(e, "header")), "a11y-ios: a heading is a header");
        record(find(t, "Klicks: 0").is_some_and(|e| has(e, "static")), "a11y-ios: a label is static text");
        let btn = find(t, "Drücken");
        record(btn.is_some_and(|e| has(e, "button") && !has(e, "disabled")), "a11y-ios: a button");
        record(find(t, "Gesperrt").is_some_and(|e| has(e, "button") && has(e, "disabled")), "a11y-ios: a disabled button: not enabled");
        record(find(t, "Option").is_some_and(|e| has(e, "button") && !has(e, "selected")), "a11y-ios: a check box: a button, not selected");
        record(
            find(t, "Eins").is_some_and(|e| has(e, "selected")) && find(t, "Zwei").is_some_and(|e| !has(e, "selected")),
            "a11y-ios: radio buttons: the first one selected",
        );
        record(find(t, "Schalter").is_some_and(|e| has(e, "toggle") && e.value == "0"), "a11y-ios: a switch: a toggle button, value 0");
        record(find_tr(t, "Lautstärke", "adjustable").is_some_and(|e| e.value == "30"), "a11y-ios: a slider: adjustable, value 30");
        record(field(t).is_some_and(|e| e.value == "Anna" && has(e, "keyboard")), "a11y-ios: a text field: UITextInput, its text");
        record(t.iter().any(|e| e.element && e.value == "40 %" && has(e, "frequent")), "a11y-ios: a progress bar: 40 %, updates frequently");
        let tab = t.iter().position(|e| !e.element && e.traits.contains(&"tabbar"));
        record(
            tab.is_some_and(|k| {
                let d = t[k].depth;
                find(t, "Erste").is_some_and(|e| e.depth == d + 1 && has(e, "button") && has(e, "selected"))
                    && find(t, "Zweite").is_some_and(|e| e.depth == d + 1 && !has(e, "selected"))
            }),
            "a11y-ios: the tab strip is a tab bar holding its tabs, the first selected",
        );
        let list = t.iter().position(|e| !e.element && e.container == 2);
        record(
            list.is_some_and(|k| FRUIT.iter().all(|n| find(t, n).is_some_and(|e| e.depth == t[k].depth + 1))),
            "a11y-ios: the list is a list container holding its rows",
        );
        record(find(t, "Apfel").is_some_and(|e| has(e, "selected")), "a11y-ios: the first row is selected");
        let f = btn.map_or([0.0; 4], |e| e.frame);
        record(f[2] > 10.0 && f[3] > 10.0 && f[0] >= 0.0 && f[1] >= 0.0, &alloc::format!("a11y-ios: the button's frame on the screen ({f:?})"));

        // actions as VoiceOver takes them: the new state is there right after the action
        ios_ax_listen(true);
        let now = |label: &str, ok: &dyn Fn(&IosAxElem) -> bool| find(&tree(), label).is_some_and(ok);
        let slider = |v: &str| find_tr(&tree(), "Lautstärke", "adjustable").is_some_and(|e| e.value == v);
        record(ios_ax_act("Drücken", "", A::Activate) && st(CLICKS) == 1, "a11y-ios: a double tap presses the button at once");
        record(now("Klicks: 1", &|_| true), "a11y-ios: the label's new text at once");
        record(ios_ax_act("Option", "", A::Activate) && now("Option", &|e| has(e, "selected")), "a11y-ios: the check box is selected right after its double tap");
        record(ios_ax_act("Schalter", "", A::Activate) && now("Schalter", &|e| e.value == "1"), "a11y-ios: the switch's value 1 right after");
        record(ios_ax_act("Zwei", "", A::Activate) && st(RADIO) == 1, "a11y-ios: the second radio button");
        record(ios_ax_act("Lautstärke", "adjustable", A::Increment) && slider("35"), "a11y-ios: swipe up: the slider at 35 right after");
        record(ios_ax_act("Lautstärke", "adjustable", A::Decrement) && slider("30"), "a11y-ios: swipe down: back to 30");
        record(!ios_ax_act("Klicks: 1", "", A::Increment), "a11y-ios: a label is not adjustable");
        record(ios_ax_act("Zweite", "", A::Activate) && st(TAB) == 1, "a11y-ios: the second tab");
        record(ios_ax_act("Kirsche", "", A::Activate) && st(LIST) == 2, "a11y-ios: the third list row");
        let clicks = st(CLICKS);
        record(!ios_ax_act("Gesperrt", "", A::Activate) && st(CLICKS) == clicks, "a11y-ios: a disabled button refuses the double tap");
        record(ios_ax_act("Eigen", "", A::Activate) && wait(|| st(OWN) == 1), "a11y-ios: a widget of its own (a pointer click)");

        // the text field: a double tap focuses it (the keyboard follows), UITextInput reads and moves
        let q = |x: Q| ios_ax_text("Name", x).unwrap_or_default();
        let range = |s: String| -> (i64, i64) {
            let mut it = s.split(',').map(|x| x.parse::<i64>().unwrap_or(-1));
            (it.next().unwrap_or(-1), it.next().unwrap_or(-1))
        };
        record(ios_ax_act("Name", "keyboard", A::Activate), "a11y-ios: a double tap on the text field");
        record(
            wait(|| {
                let (a, n) = range(q(Q::Selected));
                a >= 0 && a + n == 4
            }),
            &alloc::format!("a11y-ios: the focused field: the selection ends after 'Anna' ({})", q(Q::Selected)),
        );
        record(q(Q::Text(1, 2)) == "nn", "a11y-ios: textInRange: of the field");
        record(ios_ax_type("Bert") && wait(|| field(&tree()).is_some_and(|e| e.value == "Bert")), "a11y-ios: the keyboard types 'Bert' over the selection");
        record(ios_ax_select("Name", 1, 0) && wait(|| q(Q::Selected) == "1,0"), "a11y-ios: setSelectedTextRange: moves the caret");
        let frame = field(&tree()).map_or([0.0; 4], |e| e.frame);
        let c = {
            let mut r = [0.0; 4];
            for (k, x) in q(Q::Caret(1)).split(',').take(4).enumerate() {
                r[k] = x.parse::<f64>().unwrap_or(0.0);
            }
            r
        };
        record(
            c[3] > 0.0 && c[0] >= frame[0] - 0.5 && c[0] <= frame[0] + frame[2] && c[1] >= frame[1] - 0.5 && c[1] + c[3] <= frame[1] + frame[3] + 0.5,
            &alloc::format!("a11y-ios: the caret's rectangle lies in the field ({c:?} in {frame:?})"),
        );
        record(q(Q::Text(0, 4)) == "Bert", "a11y-ios: the field's text through UITextInput");
        let mut ev: Vec<String> = Vec::new();
        record(
            wait(|| {
                ev.extend(ios_ax_events());
                ev.iter().any(|e| e == "LayoutChanged:Name")
            }),
            &alloc::format!("a11y-ios: the focus on the text field: a layout change pointing at it ({ev:?})"),
        );
        ios_ax_listen(false);
    }

    pub fn t_a11y_ios(t: &mut T) {
        let mut app = Demo::new();
        app.on_init = Some(start);
        let code = ui::launch(&DESC, app, &style::STYLE);
        t.expect_i(code as i64, 0, "a11y-ios: UIKit's host takes the demo");
    }
}
