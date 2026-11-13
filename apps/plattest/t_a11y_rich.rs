// t_a11y_rich -- a web page's accessibility tree (section 19 of the guide: the kinds K_TABLE ..=
// K_FIGURE, the record setters, id64s, named parents, records out of view, the requests that go
// to the app, live regions and announcements); the page is apps/a11ytest/src/page.rs
//
//   plattest --test a11y-rich      (headless) the page as a ui::Runtime: Sub and the fallback
//                                  roles, the further fields, parents that are named, not
//                                  rectangles, records out of view, requests of every kind
//                                  reaching the page (ui.a11y_request_take) and changing it,
//                                  live-region changes (plat::a11y::diff), the browser's rich
//                                  update format (a11y_wire VERSION2) with an announcement
//   plattest --test uia-rich       (Windows, by hand: a window for a few seconds) the page through
//                                  IUIAutomation: control types, landmarks, Grid / GridItem /
//                                  TableItem, Selection, ScrollItem, ExpandCollapse,
//                                  SelectionItem, Value of a link, the further properties; the
//                                  events LiveRegionChanged, SystemAlert and the notification of
//                                  an announcement
//   plattest --test a11y-rich-bus  (Linux; tools/platshared/atspi_bus.py --rich starts it on its
//                                  fake buses) the page with the AT-SPI2 bridge; the fake screen
//                                  reader reads and acts, then presses "Beenden"

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use plat::a11y::{self as ax, Act, Change, Node, Op, OpRequest, Request, Role, Snapshot, Sub, WinTree};
use ui::{App, AppDesc, Ui};

use crate::T;

#[path = "../a11ytest/src/page.rs"]
mod page;
use page::*;

pub(crate) const DESC: AppDesc = AppDesc { title: "a11y-Seite", w: 560, h: 640, ..AppDesc::NONE };
const ID_QUIT: u32 = 0x7F0;
const RICH: u64 = 1 << 62;

/// the page and a button that ends the run
#[derive(Default)]
pub(crate) struct RichDemo {
    pub(crate) page: Page,
    /// called from App::init with the running window (the UIA test starts its client there)
    pub(crate) on_init: Option<fn()>,
}

impl App for RichDemo {
    fn init(&mut self, _ui: &mut Ui) -> bool {
        if let Some(f) = self.on_init {
            f();
        }
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        self.page.frame(ui);
        if ui.button(ID_QUIT, "Beenden") & ui::EV_ACT != 0 {
            ui.quit();
        }
    }
}

/// the node of window 0 with this Sub and name
pub(crate) fn sub<'a>(s: &'a Snapshot, sub: Sub, name: &str) -> Option<&'a Node> {
    s.wins.first()?.nodes.iter().find(|n| n.sub == sub && n.name == name)
}

/// the node of window 0 with this role and name
fn role<'a>(s: &'a Snapshot, role: Role, name: &str) -> Option<&'a Node> {
    s.wins.first()?.nodes.iter().find(|n| n.role == role && n.name == name)
}

/// the label of window 0 that starts with this text
fn text<'a>(s: &'a Snapshot, start: &str) -> &'a str {
    s.wins.first().and_then(|w| w.nodes.iter().find(|n| n.role == Role::Label && n.name.starts_with(start))).map_or("", |n| n.name.as_str())
}

/// the parent of a node (in window 0)
fn parent<'a>(s: &'a Snapshot, n: Option<&Node>) -> Option<&'a Node> {
    let w = s.wins.first()?;
    let k = w.find(n?.key)?;
    w.nodes.get(w.nodes[k].parent as usize)
}

fn key(id: u64) -> u64 {
    RICH | id
}

struct Run {
    rt: ui::Runtime,
    now: u32,
}

impl Run {
    fn tree(&self) -> Snapshot {
        self.rt.ui().a11y_tree()
    }

    fn step(&mut self) {
        self.now += 50;
        self.rt.step(self.now, &[plat::Event { typ: plat::E_WAKE, ..Default::default() }]);
    }

    // a request as a bridge sends it, then the frame that carries it out
    fn act(&mut self, id: u64, act: Act) {
        ax::request(Request { win: 0, key: key(id), act });
        self.step();
    }

    fn op(&mut self, id: u64, op: Op) {
        ax::request_op(OpRequest { win: 0, key: key(id), op });
        self.step();
    }
}

pub fn t_a11y_rich(t: &mut T) {
    let rt = match ui::Runtime::new(&DESC, RichDemo::default(), &style::STYLE) {
        Ok(rt) => rt,
        Err(_) => {
            t.check(false, "a11y-rich: the page starts headless");
            return;
        }
    };
    let mut r = Run { rt, now: 0 };
    let s = r.tree();
    t.check(Sub::Table as u8 == 1 && Sub::Figure as u8 == 27 && ax::SUB_COUNT == 28, "a11y-rich: the Sub numbers web/a11y.js reads (Table 1, Figure 27)");

    // kinds: a Sub each, the nearest basic role for bridges that do not know it
    let kinds: [(Sub, &str, Role); 22] = [
        (Sub::Banner, "Kopf", Role::Group),
        (Sub::Navigation, "Hauptmenü", Role::Group),
        (Sub::Complementary, "Randspalte", Role::Group),
        (Sub::Form, "Bestellung", Role::Group),
        (Sub::ContentInfo, "Fußzeile", Role::Group),
        (Sub::Table, "Preise", Role::Group),
        (Sub::Row, "Apfel 1,20 €", Role::Group),
        (Sub::ColumnHeader, "Preis", Role::Label),
        (Sub::RowHeader, "Apfel", Role::Label),
        (Sub::Cell, "1,20 €", Role::Label),
        (Sub::Tree, "Ordner", Role::List),
        (Sub::TreeItem, "Dokumente", Role::ListItem),
        (Sub::RadioGroup, "Versand", Role::Group),
        (Sub::Toolbar, "Werkzeuge", Role::Group),
        (Sub::Tooltip, "Pflichtfeld: Vor- und Nachname", Role::Label),
        (Sub::Status, "Bereit", Role::Label),
        (Sub::Log, "Verlauf", Role::Label),
        (Sub::Meter, "Speicher", Role::Progress),
        (Sub::SpinButton, "Menge", Role::Slider),
        (Sub::ScrollBar, "Seite", Role::Slider),
        (Sub::Article, "Neuigkeiten", Role::Group),
        (Sub::Figure, "Umsatz 2026", Role::Group),
    ];
    for (sb, name, rl) in kinds {
        t.check(sub(&s, sb, name).is_some_and(|n| n.role == rl), &format!("a11y-rich: {name:?} is {sb:?} (role {rl:?})"));
    }
    t.check(s.wins[0].nodes.iter().any(|n| n.sub == Sub::Main), "a11y-rich: the main landmark (no name)");
    t.check(s.wins[0].nodes.iter().any(|n| n.sub == Sub::Separator), "a11y-rich: a separator (no name)");
    t.check(sub(&s, Sub::TreeItem, "Briefe").is_none(), "a11y-rich: a closed tree item's items are not there");

    // the further fields
    let start = role(&s, Role::Link, "Start");
    t.check(start.is_some_and(|n| n.url == "https://example.org/" && n.state & ax::S_VISITED != 0 && n.set == [1, 2]), "a11y-rich: a link's address, visited, its place in the set");
    t.check(start.is_some_and(|n| n.key == key(N_START)), "a11y-rich: a record with an id64 is keyed by it");
    let name = role(&s, Role::TextBox, "Name");
    t.check(
        name.is_some_and(|n| n.text.is_empty() && n.desc == "Vor- und Nachname" && n.state & (ax::S_REQUIRED | ax::S_INVALID) == ax::S_REQUIRED | ax::S_INVALID),
        "a11y-rich: a text box with a name beside its text, a description, required, invalid",
    );
    t.check(sub(&s, Sub::Cell, "1,20 €").is_some_and(|n| n.cell == [1, 1, 1, 1]), "a11y-rich: a cell's place");
    t.check(sub(&s, Sub::ColumnHeader, "Preis").is_some_and(|n| n.cell == [0, 1, 1, 1]), "a11y-rich: a column header's place");
    let w = &s.wins[0];
    let table = w.find(key(N_TABLE));
    t.check(table.is_some_and(|k| w.grid_size(k) == (3, 2) && w.nodes[k].state & ax::S_MULTISELECT != 0), "a11y-rich: the table's size (Sem::grid), multi-select");
    t.check(
        table.and_then(|k| w.cell_at(k, 2, 1)).is_some_and(|k| w.nodes[k].name == "0,90 €"),
        "a11y-rich: the table's cell at row 2, column 1",
    );
    let cell = w.find(key(N_C_PEAR));
    t.check(cell.is_some_and(|k| w.headers_of(k, false).iter().map(|&h| w.nodes[h].name.as_str()).collect::<Vec<_>>() == ["Preis"]), "a11y-rich: a cell's column header");
    t.check(cell.is_some_and(|k| w.headers_of(k, true).iter().map(|&h| w.nodes[h].name.as_str()).collect::<Vec<_>>() == ["Birne"]), "a11y-rich: a cell's row header");
    let docs = sub(&s, Sub::TreeItem, "Dokumente");
    t.check(docs.is_some_and(|n| n.level() == 1 && n.set == [1, 2] && n.state & ax::S_COLLAPSED != 0 && n.actions & ax::A_EXPAND != 0), "a11y-rich: a tree item's level, place, collapsed, expandable");
    let meter = sub(&s, Sub::Meter, "Speicher");
    t.check(meter.is_some_and(|n| n.value == 0.4 && n.min == 0.0 && n.max == 1.0 && n.text == "40 %"), "a11y-rich: a meter's fractions and value text");
    let spin = sub(&s, Sub::SpinButton, "Menge");
    t.check(spin.is_some_and(|n| n.value == 1.0 && n.max == 10.0 && n.actions & (ax::A_VALUE | ax::A_STEP) == ax::A_VALUE | ax::A_STEP), "a11y-rich: a spin button's value, steps");
    t.check(sub(&s, Sub::Status, "Bereit").is_some_and(|n| n.live == ax::LIVE_POLITE), "a11y-rich: a status line is a polite live region");
    t.check(sub(&s, Sub::Log, "Verlauf").is_some_and(|n| n.live == ax::LIVE_POLITE), "a11y-rich: a log is a polite live region");
    t.check(role(&s, Role::Image, "Balkendiagramm").is_some_and(|n| n.desc == "Drei Balken, der letzte am höchsten"), "a11y-rich: an image's description");
    t.check(start.is_some_and(|n| n.actions & ax::A_SCROLL != 0 && n.actions & ax::A_CLICK != 0), "a11y-rich: a record with an id64 can be scrolled into view");

    // parents named, not rectangles: the tooltip lies outside the text box it hangs under
    t.check(parent(&s, sub(&s, Sub::Cell, "1,20 €")).is_some_and(|p| p.key == key(N_ROW1)), "a11y-rich: a cell hangs under its row");
    t.check(parent(&s, sub(&s, Sub::Row, "Apfel 1,20 €")).is_some_and(|p| p.key == key(N_TABLE)), "a11y-rich: a row under its table");
    t.check(parent(&s, sub(&s, Sub::Table, "Preise")).is_some_and(|p| p.sub == Sub::Main), "a11y-rich: the table under the main landmark");
    t.check(parent(&s, sub(&s, Sub::Tooltip, "Pflichtfeld: Vor- und Nachname")).is_some_and(|p| p.key == key(N_NAME)), "a11y-rich: a tooltip under the text box it explains");
    t.check(parent(&s, sub(&s, Sub::Banner, "Kopf")).is_some_and(|p| p.role == Role::Canvas && p.name == "Beispielseite"), "a11y-rich: the page's landmarks under its canvas (a widget id)");
    t.check(parent(&s, sub(&s, Sub::ContentInfo, "Fußzeile")).is_some_and(|p| p.role == Role::Canvas), "a11y-rich: the footer out of view under the canvas too");

    // out of view: in the tree, marked, not hit by a point
    let article = sub(&s, Sub::Article, "Neuigkeiten");
    t.check(article.is_some_and(|n| n.state & ax::S_OFFSCREEN != 0 && !w.on_screen(n)), "a11y-rich: the article below the view is S_OFFSCREEN");
    t.check(role(&s, Role::Heading, "Neuigkeiten").is_some_and(|n| n.state & ax::S_OFFSCREEN != 0 && n.level() == 2), "a11y-rich: its heading too, with its level");
    t.check(sub(&s, Sub::Banner, "Kopf").is_some_and(|n| n.state & ax::S_OFFSCREEN == 0), "a11y-rich: the banner in view is not");
    t.check(article.is_some_and(|n| w.at_point(n.rect[0] + 2, n.rect[1] + 2).is_none_or(|k| w.nodes[k].key != n.key)), "a11y-rich: a point does not hit a record out of view");

    // requests: every one goes to the page, which changes
    r.op(N_ARTICLE, Op::ScrollIntoView);
    let s2 = r.tree();
    t.check(sub(&s2, Sub::Article, "Neuigkeiten").is_some_and(|n| n.state & ax::S_OFFSCREEN == 0), "a11y-rich: ScrollIntoView: the article in view");
    t.check(sub(&s2, Sub::Banner, "Kopf").is_some_and(|n| n.state & ax::S_OFFSCREEN != 0), "a11y-rich: the banner went out of view");
    t.check(
        role(&s2, Role::Button, "Ansage").is_some() && parent(&s2, role(&s2, Role::Button, "Ansage")).is_none(),
        "a11y-rich: the app's own button is not held by the page's records scrolled over it",
    );
    t.check(text(&s2, "Anfrage:").starts_with(&format!("Anfrage: {N_ARTICLE}:scroll")), &format!("a11y-rich: the page saw the request ({})", text(&s2, "Anfrage:")));
    r.act(N_START, Act::Click);
    t.check(text(&r.tree(), "Geklickt:").starts_with("Geklickt: Start"), "a11y-rich: Click on a link");
    r.op(N_DOCS, Op::Expand);
    let s3 = r.tree();
    let letters = sub(&s3, Sub::TreeItem, "Briefe");
    t.check(letters.is_some_and(|n| n.level() == 2) && parent(&s3, letters).is_some_and(|p| p.key == key(N_DOCS)), "a11y-rich: Expand: the item's item appears, a level deeper, under it");
    t.check(sub(&s3, Sub::TreeItem, "Dokumente").is_some_and(|n| n.state & ax::S_EXPANDED != 0 && n.state & ax::S_COLLAPSED == 0), "a11y-rich: the item is expanded");
    r.op(N_DOCS, Op::Collapse);
    t.check(sub(&r.tree(), Sub::TreeItem, "Briefe").is_none(), "a11y-rich: Collapse");
    r.op(N_ROW2, Op::SelectItem);
    r.op(N_ROW1, Op::AddToSelection);
    t.check(text(&r.tree(), "Geklickt:").contains("Zeilen: 11"), "a11y-rich: SelectItem, AddToSelection on rows");
    t.check(sub(&r.tree(), Sub::Row, "Birne 0,90 €").is_some_and(|n| n.state & ax::S_SELECTED != 0), "a11y-rich: a selected row");
    r.op(N_ROW2, Op::RemoveFromSelection);
    t.check(text(&r.tree(), "Geklickt:").contains("Zeilen: 10"), "a11y-rich: RemoveFromSelection");
    r.act(N_NAME, Act::Text(String::from("Anna")));
    let s4 = r.tree();
    t.check(role(&s4, Role::TextBox, "Name").is_some_and(|n| n.text == "Anna" && n.state & ax::S_INVALID == 0), "a11y-rich: SetText on the text box (valid now)");
    r.op(N_QTY, Op::Increment);
    t.check(sub(&r.tree(), Sub::SpinButton, "Menge").is_some_and(|n| n.value == 2.0), "a11y-rich: Increment on the spin button");
    r.act(N_QTY, Act::Value(7.0));
    t.check(sub(&r.tree(), Sub::SpinButton, "Menge").is_some_and(|n| n.value == 7.0), "a11y-rich: SetValue on the spin button");
    r.op(N_QTY, Op::Decrement);
    t.check(text(&r.tree(), "Name:").contains("Menge: 6"), "a11y-rich: Decrement");
    r.act(N_HELP, Act::Focus);
    let s5 = r.tree();
    let f = &s5.wins[0];
    t.check(f.nodes.get(f.focus as usize).is_some_and(|n| n.key == key(N_HELP) && n.state & ax::S_FOCUSED != 0), "a11y-rich: Focus: the link has the focus of the tree");
    r.act(N_NAME, Act::Select(1, 3));
    t.check(text(&r.tree(), "Anfrage:").starts_with(&format!("Anfrage: {N_NAME}:selecttext=1,3")), "a11y-rich: a text selection reaches the page");
    r.act(N_BOLD, Act::Click);
    t.check(role(&r.tree(), Role::Button, "Fett").is_some_and(|n| n.state & ax::S_PRESSED != 0), "a11y-rich: Click on a tool bar button: pressed");

    // live regions: a change inside one is a Live change, a new alert too
    let before = Arc::new(r.tree());
    click_button(&mut r, "Status");
    let after = Arc::new(r.tree());
    let ch = ax::diff(&before, &after);
    t.check(ch.contains(&Change::Live(0, key(N_STATUS))), &format!("a11y-rich: the status line's new text is a Live change ({:?})", ch.iter().filter(|c| matches!(c, Change::Live(..))).collect::<Vec<_>>()));
    r.act(N_NAME, Act::Text(String::new()));
    let before = Arc::new(r.tree());
    r.act(N_SEND, Act::Click);
    let after = Arc::new(r.tree());
    let alert = sub(&after, Sub::Alert, "Bitte einen Namen eingeben");
    t.check(alert.is_some_and(|n| n.live == ax::LIVE_ASSERTIVE), "a11y-rich: an alert appears, assertive");
    t.check(ax::diff(&before, &after).contains(&Change::Live(0, key(N_ALERT))), "a11y-rich: a new alert is a Live change");

    t_wire_rich(t, &after);
    t_names(t);
}

// a built-in button of the window pressed through its record (a widget's click)
fn click_button(r: &mut Run, label: &str) {
    if let Some(n) = role(&r.tree(), Role::Button, label) {
        ax::request(Request { win: 0, key: n.key, act: Act::Click });
        r.step();
        r.step();
    }
}

// the browser's rich format: a full update with the further fields, an announcement
fn t_wire_rich(t: &mut T, s: &Snapshot) {
    use ax::wire::{self, Mirror};
    // (key, sub, desc, url, set, level, cell, grid, live) of every node sent, the announcements
    type Rich = (u64, u8, String, String, [u32; 2], u32, [u32; 4], [u32; 2], u8);
    fn read(b: &[u8]) -> Option<(u8, Vec<Rich>, Vec<(u8, i32, String)>)> {
        let mut p = 0usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = b.get(p..p + n)?;
            p += n;
            Some(s)
        };
        fn u32_(s: &[u8]) -> u32 {
            u32::from_le_bytes([s[0], s[1], s[2], s[3]])
        }
        let version = take(1)?[0];
        take(1)?;
        let mut nodes = Vec::new();
        for _ in 0..take(1)?[0] {
            take(2 + 16 + 8)?;
            let n = u32_(take(4)?) as usize;
            take(n)?;
            let order = u32_(take(4)?);
            if order != wire::UNCHANGED {
                take(order as usize * 12)?;
            }
            for _ in 0..u32_(take(4)?) {
                let key = u64::from_le_bytes(take(8)?.try_into().ok()?);
                take(1 + 4 + 4 + 16)?;
                for _ in 0..2 {
                    let n = u32_(take(4)?) as usize;
                    take(n)?;
                }
                take(8 + 16 + 32)?;
                if version == wire::VERSION2 {
                    let sub = take(1)?[0];
                    let mut strs = [String::new(), String::new()];
                    for s in strs.iter_mut() {
                        let n = u32_(take(4)?) as usize;
                        *s = String::from_utf8(take(n)?.to_vec()).ok()?;
                    }
                    let mut v = [0u32; 9];
                    for x in v.iter_mut() {
                        *x = u32_(take(4)?);
                    }
                    let live = take(1)?[0];
                    let [desc, url] = strs;
                    nodes.push((key, sub, desc, url, [v[0], v[1]], v[2], [v[3], v[4], v[5], v[6]], [v[7], v[8]], live));
                }
            }
        }
        let mut said = Vec::new();
        for _ in 0..u32_(take(4)?) {
            let kind = take(1)?[0];
            let win = take(1)?[0];
            take(8)?;
            let a = u32_(take(4)?) as i32;
            take(4)?;
            if kind == wire::E_TEXT || kind == wire::E_ANNOUNCE {
                let n = u32_(take(4)?) as usize;
                let s = String::from_utf8(take(n)?.to_vec()).ok()?;
                if kind == wire::E_ANNOUNCE {
                    said.push((win, a, s));
                }
            }
        }
        (p == b.len()).then_some((version, nodes, said))
    }
    let place = |w: &WinTree| [0, 0, w.size[0], w.size[1]];
    let snap = Arc::new(s.clone());
    let mut m = Mirror::new();
    m.set_version(wire::VERSION2);
    let said = [Change::Announce(0, String::from("Gespeichert"), ax::LIVE_POLITE)];
    let up = m.update(&snap, &said, &place).and_then(|b| read(&b));
    t.check(up.as_ref().is_some_and(|(v, ..)| *v == wire::VERSION2), "a11y-rich: wire: the rich format's first byte (VERSION2)");
    let cell = up.as_ref().and_then(|(_, n, _)| n.iter().find(|e| e.0 == key(N_C_PEAR)).cloned());
    t.check(cell.as_ref().is_some_and(|c| c.1 == Sub::Cell as u8 && c.6 == [2, 1, 1, 1]), "a11y-rich: wire: a cell's Sub and place");
    let link = up.as_ref().and_then(|(_, n, _)| n.iter().find(|e| e.0 == key(N_START)).cloned());
    t.check(link.as_ref().is_some_and(|l| l.3 == "https://example.org/" && l.4 == [1, 2]), "a11y-rich: wire: a link's address and place in its set");
    let table = up.as_ref().and_then(|(_, n, _)| n.iter().find(|e| e.0 == key(N_TABLE)).cloned());
    t.check(table.as_ref().is_some_and(|l| l.7 == [3, 2]), "a11y-rich: wire: the table's size");
    let status = up.as_ref().and_then(|(_, n, _)| n.iter().find(|e| e.0 == key(N_STATUS)).cloned());
    t.check(status.as_ref().is_some_and(|l| l.8 == ax::LIVE_POLITE), "a11y-rich: wire: a live region's politeness");
    t.check(up.as_ref().is_some_and(|(.., s)| s == &[(0, ax::LIVE_POLITE as i32, String::from("Gespeichert"))]), "a11y-rich: wire: the announcement (E_ANNOUNCE)");
    // the first format: no further fields, no announcement
    let mut m1 = Mirror::new();
    let up1 = m1.update(&snap, &said, &place).and_then(|b| read(&b));
    t.check(up1.is_some_and(|(v, n, s)| v == wire::VERSION && n.is_empty() && s.is_empty()), "a11y-rich: wire: the first format stays as it was (Android's host)");
}

// what the bridges call the Subs: German, then English
fn t_names(t: &mut T) {
    t.check(ax::sub_name(Sub::Table) == "Tabelle" && ax::sub_name(Sub::Main) == "Hauptinhalt", "a11y-rich: the Subs' names in German (the default)");
    ui::i18n::set_language_code("en");
    t.check(ax::sub_name(Sub::Table) == "table" && ax::sub_name(Sub::TreeItem) == "tree item", &format!("a11y-rich: the Subs' names in English ({})", ax::sub_name(Sub::Table)));
    ui::i18n::set_language(ui::i18n::LANG_DE);
}

/// (Linux) the page in a real window for tools/platshared/atspi_bus.py --rich: the fake screen
/// reader opens "Dokumente", selects the second row, sets the name to "Bert", says an announcement
/// ("Ansage") and changes the status ("Status"), then presses "Beenden"
pub fn t_a11y_rich_bus(t: &mut T) {
    let mut app = RichDemo::default();
    let code = ui::run(&DESC, &mut app, &style::STYLE);
    t.expect_i(code as i64, 0, "a11y-rich-bus: the window ran");
    t.check(app.page.open, "a11y-rich-bus: the tree item was opened (Action expand)");
    t.check(app.page.rows[1], "a11y-rich-bus: the second row was selected (Selection.SelectChild)");
    t.expect_s(Some(&app.page.name), Some("Bert"), "a11y-rich-bus: the name was set (EditableText)");
    t.expect_s(Some(app.page.status), Some("Gespeichert"), "a11y-rich-bus: the status changed");
    t.check(app.page.seen.iter().any(|s| s == &format!("{N_ARTICLE}:scroll")), "a11y-rich-bus: the article was scrolled into view (Component.ScrollTo)");
}

#[cfg(not(windows))]
pub fn t_uia_rich(t: &mut T) {
    t.info("  (uia-rich: UI Automation is the Windows bridge; Linux has a11y-rich-bus)\n");
}

#[cfg(windows)]
pub use uia::t_uia_rich;

/// (Windows) the page with the UI Automation providers, read and driven through IUIAutomation
/// from a thread of the test (plat::uia_test_*)
#[cfg(windows)]
mod uia {
    use alloc::boxed::Box;
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::sync::atomic::{AtomicIsize, Ordering::SeqCst};

    use plat::sync::Mutex;
    use plat::{UiaTestAct as A, UiaTestElem, uia_test_act, uia_test_tree};

    use super::*;

    const CT_BUTTON: i32 = 50000;
    const CT_EDIT: i32 = 50004;
    const CT_LINK: i32 = 50005;
    const CT_PROGRESS: i32 = 50012;
    const CT_SCROLLBAR: i32 = 50014;
    const CT_SPINNER: i32 = 50016;
    const CT_STATUSBAR: i32 = 50017;
    const CT_TEXT: i32 = 50020;
    const CT_TOOLBAR: i32 = 50021;
    const CT_TOOLTIP: i32 = 50022;
    const CT_TREE: i32 = 50023;
    const CT_TREEITEM: i32 = 50024;
    const CT_GROUP: i32 = 50026;
    const CT_DATAITEM: i32 = 50029;
    const CT_HEADERITEM: i32 = 50035;
    const CT_TABLE: i32 = 50036;
    const CT_SEPARATOR: i32 = 50038;

    static HWND: AtomicIsize = AtomicIsize::new(0);
    static RESULTS: Mutex<Vec<(bool, String)>> = Mutex::new(Vec::new());

    fn record(ok: bool, name: &str) {
        RESULTS.lock().push((ok, String::from(name)));
    }

    fn start() {
        HWND.store(plat::uia_test_hwnd(0), SeqCst);
        if plat::thread_spawn(0, Box::new(client)).is_none() {
            record(false, "uia-rich: the client thread starts");
        }
    }

    fn find<'a>(t: &'a [UiaTestElem], name: &str, ct: i32) -> Option<&'a UiaTestElem> {
        t.iter().find(|e| e.name == name && e.control_type == ct)
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

    // the page's label that starts with this text
    fn label(t: &[UiaTestElem], start: &str) -> String {
        t.iter().find(|e| e.control_type == CT_TEXT && e.name.starts_with(start)).map_or(String::new(), |e| e.name.clone())
    }

    fn client() {
        let hwnd = HWND.load(SeqCst);
        steps(hwnd);
        plat::evq_push(plat::Event { typ: plat::E_CLOSE, ..Default::default() });
        plat::wake();
    }

    fn steps(hwnd: isize) {
        let mut tree = Vec::new();
        for _ in 0..100 {
            if let Some(t) = uia_test_tree(hwnd) {
                if t.iter().any(|e| e.name == "Preise") {
                    tree = t;
                    break;
                }
            }
            plat::sleep_ms(100);
        }
        record(!tree.is_empty(), "uia-rich: IUIAutomation reads the page");
        if tree.is_empty() {
            return;
        }
        let t = &tree;
        // control types, landmarks, the ARIA role
        let table = find(t, "Preise", CT_TABLE);
        record(table.is_some_and(|e| e.grid == Some((3, 2)) && e.multiple == Some(true)), "uia-rich: a Table with Grid (3 x 2), Selection with several items");
        let cell = find(t, "0,90 €", CT_DATAITEM);
        record(cell.is_some_and(|e| e.cell == Some([2, 1, 1, 1])), "uia-rich: a cell is a DataItem with GridItem (row 2, column 1)");
        record(cell.is_some_and(|e| e.col_headers == ["Preis"]), &alloc::format!("uia-rich: TableItem: its column header ({:?})", cell.map(|e| &e.col_headers)));
        record(find(t, "Preis", CT_HEADERITEM).is_some_and(|e| e.aria_role == "columnheader"), "uia-rich: a column header is a HeaderItem");
        record(find(t, "Birne 0,90 €", CT_DATAITEM).is_some_and(|e| e.selected == Some(false) && e.pos_in_set == 2 && e.size_of_set == 2), "uia-rich: a row: SelectionItem, PositionInSet 2 of 2");
        record(find(t, "Hauptmenü", CT_GROUP).is_some_and(|e| e.landmark == 80003), "uia-rich: navigation: a Group with LandmarkType Navigation");
        record(find(t, "Kopf", CT_GROUP).is_some_and(|e| e.landmark == 80000 && e.landmark_name == "Banner"), "uia-rich: banner: a custom landmark named Banner");
        record(find(t, "Bestellung", CT_GROUP).is_some_and(|e| e.landmark == 80001), "uia-rich: form: LandmarkType Form");
        record(t.iter().any(|e| e.landmark == 80002), "uia-rich: main: LandmarkType Main");
        record(find(t, "Ordner", CT_TREE).is_some(), "uia-rich: a Tree");
        let docs = find(t, "Dokumente", CT_TREEITEM);
        record(docs.is_some_and(|e| e.expand == 0 && e.level == 1 && e.pos_in_set == 1 && e.size_of_set == 2), &alloc::format!("uia-rich: a TreeItem: collapsed, Level 1, 1 of 2 ({:?})", docs.map(|e| (e.expand, e.level, e.pos_in_set, e.size_of_set, e.selected, e.scroll_item, e.aria_properties.clone(), e.enabled))));
        record(find(t, "Menge", CT_SPINNER).is_some_and(|e| e.range == Some((1.0, 1.0, 10.0))), "uia-rich: a Spinner with RangeValue 1 in 1 ..= 10");
        record(find(t, "Seite", CT_SCROLLBAR).is_some(), "uia-rich: a ScrollBar");
        record(find(t, "Werkzeuge", CT_TOOLBAR).is_some() && t.iter().any(|e| e.control_type == CT_SEPARATOR), "uia-rich: a ToolBar with a Separator");
        record(find(t, "Pflichtfeld: Vor- und Nachname", CT_TOOLTIP).is_some(), "uia-rich: a ToolTip");
        record(find(t, "Bereit", CT_STATUSBAR).is_some_and(|e| e.live == 1), "uia-rich: a status line: StatusBar, LiveSetting Polite");
        record(find(t, "Speicher", CT_PROGRESS).is_some_and(|e| e.aria_role == "meter" && e.range == Some((0.4, 0.0, 1.0))), "uia-rich: a meter: ProgressBar with AriaRole meter");
        record(find(t, "Start", CT_LINK).is_some_and(|e| e.value.as_deref() == Some("https://example.org/") && e.pos_in_set == 1), "uia-rich: a link's address as its Value");
        let name = find(t, "Name", CT_EDIT);
        record(name.is_some_and(|e| e.required && !e.valid && e.desc == "Vor- und Nachname"), "uia-rich: a text box: IsRequiredForForm, not IsDataValidForForm, FullDescription");
        record(name.is_some_and(|e| e.aria_properties.contains("required=true")), &alloc::format!("uia-rich: AriaProperties ({:?})", name.map(|e| &e.aria_properties)));
        let article = find(t, "Neuigkeiten", CT_GROUP);
        record(article.is_some_and(|e| e.offscreen && e.aria_role == "article" && e.scroll_item), "uia-rich: the article below the view: IsOffscreen, ScrollItem");

        // actions through the patterns, the page's answers
        record(plat::uia_test_listen(hwnd), "uia-rich: event handlers registered");
        record(uia_test_act(hwnd, "Dokumente", CT_TREEITEM, A::Expand) && wait_tree(hwnd, |t| find(t, "Briefe", CT_TREEITEM).is_some_and(|e| e.level == 2)), "uia-rich: ExpandCollapse.Expand: the item's item appears");
        record(uia_test_act(hwnd, "Dokumente", CT_TREEITEM, A::Collapse) && wait_tree(hwnd, |t| find(t, "Briefe", CT_TREEITEM).is_none()), "uia-rich: ExpandCollapse.Collapse");
        record(uia_test_act(hwnd, "Neuigkeiten", CT_GROUP, A::ScrollIntoView) && wait_tree(hwnd, |t| find(t, "Neuigkeiten", CT_GROUP).is_some_and(|e| !e.offscreen)), "uia-rich: ScrollItem.ScrollIntoView: the article in view");
        record(uia_test_act(hwnd, "Birne 0,90 €", CT_DATAITEM, A::Select) && wait_tree(hwnd, |t| label(t, "Geklickt:").contains("Zeilen: 01")), "uia-rich: SelectionItem.Select on a row");
        record(uia_test_act(hwnd, "Apfel 1,20 €", CT_DATAITEM, A::AddToSelection) && wait_tree(hwnd, |t| label(t, "Geklickt:").contains("Zeilen: 11")), "uia-rich: AddToSelection");
        record(uia_test_act(hwnd, "Birne 0,90 €", CT_DATAITEM, A::RemoveFromSelection) && wait_tree(hwnd, |t| label(t, "Geklickt:").contains("Zeilen: 10")), "uia-rich: RemoveFromSelection");
        record(uia_test_act(hwnd, "Start", CT_LINK, A::Invoke) && wait_tree(hwnd, |t| label(t, "Geklickt:").starts_with("Geklickt: Start")), "uia-rich: Invoke on a link");
        record(uia_test_act(hwnd, "Name", CT_EDIT, A::SetValue(String::from("Bert"))) && wait_tree(hwnd, |t| find(t, "Name", CT_EDIT).is_some_and(|e| e.value.as_deref() == Some("Bert") && e.valid)), "uia-rich: Value.SetValue on the text box");
        record(uia_test_act(hwnd, "Menge", CT_SPINNER, A::SetRange(5.0)) && wait_tree(hwnd, |t| label(t, "Name:").contains("Menge: 5")), "uia-rich: RangeValue.SetValue on the spin button");
        record(uia_test_act(hwnd, "Fett", CT_BUTTON, A::Invoke) && wait_tree(hwnd, |t| label(t, "Name:").contains("Fett: 1")), "uia-rich: Invoke on a tool bar button");
        // the events: a live region, an announcement, an alert
        record(uia_test_act(hwnd, "Status", CT_BUTTON, A::Invoke), "uia-rich: the status changes");
        record(uia_test_act(hwnd, "Ansage", CT_BUTTON, A::Invoke), "uia-rich: an announcement");
        record(uia_test_act(hwnd, "Name", CT_EDIT, A::SetValue(String::new())), "uia-rich: the name emptied");
        plat::sleep_ms(200);
        record(uia_test_act(hwnd, "Absenden", CT_BUTTON, A::Invoke), "uia-rich: sending without a name");
        let want = ["event:20024:Gespeichert", "notify:Gespeichert:2", "event:20023:Bitte einen Namen eingeben", "event:20024:Bitte einen Namen eingeben"];
        let mut ev: Vec<String> = Vec::new();
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
            record(ok, &alloc::format!("uia-rich: event {w}"));
            if !ok {
                plat::write_stdout(alloc::format!("  events: {ev:?}\n").as_bytes());
            }
        }
    }

    pub fn t_uia_rich(t: &mut T) {
        let mut app = RichDemo { on_init: Some(start), ..RichDemo::default() };
        let code = ui::run(&DESC, &mut app, &style::STYLE);
        let results = core::mem::take(&mut *RESULTS.lock());
        t.check(!results.is_empty(), "uia-rich: the client ran");
        for (ok, name) in results {
            t.check(ok, &name);
        }
        t.expect_i(code as i64, 0, "uia-rich: the window ran");
    }
}
