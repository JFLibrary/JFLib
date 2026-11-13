// page -- a small web page as a browser engine would publish it (apps/a11ytest; plattest's
// t_a11y_rich.rs takes the same file): drawn into a canvas, its accessibility tree published with
// ui.sem_rect and the record setters of section 19 of the guide -- landmarks (banner, navigation,
// complementary, main, form, content information), headings, links with addresses (one visited),
// a table with column and row headers (rows selectable, several at once), a tree with an item
// that opens, a text box with a name, a description and the states required / invalid, a spin
// button, radio buttons in a radio group, a tool bar with a separator, a tooltip, a status line
// and a log (live regions), a meter with a value text, a scroll bar, an alert, and below the
// view (AS_OFFSCREEN, scrolled away) an article, a figure and the footer.  Every record has an
// id64 and names its parent; a screen reader's requests come back through
// ui.a11y_request_take and change the page (Page::seen lists them as "node:action").
//
//   "Ansage"       ui.announce("Gespeichert", LIVE_POLITE)
//   "Status"       the status line says "Gespeichert" (a live region whose text changes)
//   "Nach unten"   scrolls the page to its end (the records of the top go out of view)
//   "Absenden"     (on the page) without a name: an alert appears, "Bitte einen Namen eingeben"

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

pub const ID_PAGE: u32 = 0x700;
pub const ID_ANNOUNCE: u32 = 0x701;
pub const ID_STATUS: u32 = 0x702;
pub const ID_DOWN: u32 = 0x703;

// the page's nodes (their id64)
pub const N_BANNER: u64 = 10;
pub const N_TITLE: u64 = 11;
pub const N_NAV: u64 = 20;
pub const N_START: u64 = 21;
pub const N_HELP: u64 = 22;
pub const N_ASIDE: u64 = 25;
pub const N_TIPTEXT: u64 = 26;
pub const N_MAIN: u64 = 30;
pub const N_TABLE: u64 = 40;
pub const N_ROW0: u64 = 41;
pub const N_ROW1: u64 = 42;
pub const N_ROW2: u64 = 43;
pub const N_H_ITEM: u64 = 44;
pub const N_H_PRICE: u64 = 45;
pub const N_R_APPLE: u64 = 46;
pub const N_C_APPLE: u64 = 47;
pub const N_R_PEAR: u64 = 48;
pub const N_C_PEAR: u64 = 49;
pub const N_TREE: u64 = 60;
pub const N_DOCS: u64 = 61;
pub const N_LETTERS: u64 = 62;
pub const N_PICS: u64 = 63;
pub const N_FORM: u64 = 70;
pub const N_NAME: u64 = 71;
pub const N_QTY: u64 = 72;
pub const N_SHIP: u64 = 73;
pub const N_POST: u64 = 74;
pub const N_PICKUP: u64 = 75;
pub const N_SEND: u64 = 76;
pub const N_TOOLS: u64 = 80;
pub const N_BOLD: u64 = 81;
pub const N_SEP: u64 = 82;
pub const N_ITALIC: u64 = 83;
pub const N_STATUS: u64 = 90;
pub const N_METER: u64 = 91;
pub const N_TIP: u64 = 92;
pub const N_LOG: u64 = 93;
pub const N_ALERT: u64 = 99;
pub const N_SCROLL: u64 = 100;
pub const N_ARTICLE: u64 = 110;
pub const N_NEWS: u64 = 111;
pub const N_PARA: u64 = 112; // .. 114: its paragraphs
pub const N_FIGURE: u64 = 120;
pub const N_CHART: u64 = 121;
pub const N_FOOTER: u64 = 130;
pub const N_COPY: u64 = 131;
pub const N_IMPRINT: u64 = 132;
// the log's lines: N_LINE + their number
pub const N_LINE: u64 = 1000;

// the view of the page and the page (DIP)
const VIEW_W: i32 = 520;
const VIEW_H: i32 = 420;
const PAGE_H: i32 = 880;

const PARAS: [&str; 3] = ["Die neue Version ist da.", "Sie liest Tabellen vor.", "Und Bäume."];

/// the page's state
pub struct Page {
    /// how far the page is scrolled (DIP)
    pub scroll: i32,
    /// the tree's "Dokumente" is open; the item selected
    pub open: bool,
    pub tree_sel: u64,
    /// the table's selected rows (1, 2)
    pub rows: [bool; 2],
    pub name: String,
    /// the text box's selection: anchor, caret (characters)
    pub sel: (u32, u32),
    pub qty: i32,
    /// 0 post, 1 pick-up
    pub ship: i32,
    pub bold: bool,
    pub italic: bool,
    pub status: &'static str,
    pub alert: bool,
    /// the node with the page's focus (0: none); the canvas has the keyboard
    pub focus: u64,
    has_focus: bool,
    /// the link clicked last
    pub clicked: &'static str,
    /// the log's lines
    pub lines: Vec<String>,
    /// the requests the page got: "node:action"
    pub seen: Vec<String>,
}

impl Default for Page {
    fn default() -> Self {
        Page {
            scroll: 0,
            open: false,
            tree_sel: 0,
            rows: [false; 2],
            name: String::new(),
            sel: (0, 0),
            qty: 1,
            ship: 0,
            bold: false,
            italic: false,
            status: "Bereit",
            alert: false,
            focus: 0,
            has_focus: false,
            clicked: "-",
            lines: Vec::new(),
            seen: Vec::new(),
        }
    }
}

// v rounded to a whole number (no_std has no f64::round)
fn round(v: f64) -> i32 {
    if v < 0.0 { (v - 0.5) as i32 } else { (v + 0.5) as i32 }
}

// the name of an action in Page::seen
fn act_name(a: &A11yAction) -> String {
    match a {
        A11yAction::Focus => String::from("focus"),
        A11yAction::Click => String::from("click"),
        A11yAction::ScrollIntoView => String::from("scroll"),
        A11yAction::Expand => String::from("expand"),
        A11yAction::Collapse => String::from("collapse"),
        A11yAction::Increment => String::from("increment"),
        A11yAction::Decrement => String::from("decrement"),
        A11yAction::SetValue(v) => format!("value={v}"),
        A11yAction::SetText(t) => format!("text={t}"),
        A11yAction::SelectText(a, c) => format!("selecttext={a},{c}"),
        A11yAction::Select => String::from("select"),
        A11yAction::AddToSelection => String::from("add"),
        A11yAction::RemoveFromSelection => String::from("remove"),
        _ => String::from("?"),
    }
}

impl Page {
    // where a node lies on the page (DIP; None: not there now)
    fn place(&self, id: u64) -> Option<[i32; 4]> {
        let tree_y = |k: i32| 52 + 24 * k;
        Some(match id {
            N_BANNER => [0, 0, 520, 40],
            N_TITLE => [12, 8, 300, 24],
            N_NAV => [0, 40, 140, 120],
            N_START => [12, 52, 100, 20],
            N_HELP => [12, 78, 100, 20],
            N_ASIDE => [0, 170, 140, 240],
            N_TIPTEXT => [8, 180, 124, 20],
            N_MAIN => [140, 40, 366, 780],
            N_TABLE => [150, 48, 240, 84],
            N_ROW0 => [150, 48, 240, 28],
            N_ROW1 => [150, 76, 240, 28],
            N_ROW2 => [150, 104, 240, 28],
            N_H_ITEM => [150, 48, 120, 28],
            N_H_PRICE => [270, 48, 120, 28],
            N_R_APPLE => [150, 76, 120, 28],
            N_C_APPLE => [270, 76, 120, 28],
            N_R_PEAR => [150, 104, 120, 28],
            N_C_PEAR => [270, 104, 120, 28],
            N_TREE => [400, 48, 104, 84],
            N_DOCS => [400, tree_y(0), 104, 22],
            N_LETTERS if self.open => [412, tree_y(1), 92, 22],
            N_PICS => [400, tree_y(if self.open { 2 } else { 1 }), 104, 22],
            N_FORM => [150, 140, 350, 110],
            N_NAME => [160, 150, 200, 24],
            N_QTY => [370, 150, 80, 24],
            N_SHIP => [160, 182, 232, 24],
            N_POST => [160, 182, 100, 24],
            N_PICKUP => [270, 182, 120, 24],
            N_SEND => [400, 182, 90, 24],
            N_TOOLS => [160, 214, 200, 28],
            N_BOLD => [162, 216, 60, 24],
            N_SEP => [226, 216, 2, 24],
            N_ITALIC => [232, 216, 60, 24],
            N_STATUS => [150, 258, 200, 20],
            N_METER => [360, 258, 140, 20],
            N_TIP => [160, 282, 220, 20],
            N_LOG => [150, 306, 350, 64],
            N_ALERT if self.alert => [150, 376, 350, 24],
            N_ARTICLE => [150, 440, 350, 200],
            N_NEWS => [160, 448, 300, 26],
            N_FIGURE => [150, 660, 350, 140],
            N_CHART => [160, 670, 330, 120],
            N_FOOTER => [0, 830, 520, 40],
            N_COPY => [12, 840, 200, 20],
            N_IMPRINT => [400, 840, 100, 20],
            p if (N_PARA..N_PARA + 3).contains(&p) => [160, 482 + 40 * (p - N_PARA) as i32, 330, 32],
            l if l >= N_LINE && ((l - N_LINE) as usize) < self.lines.len() => {
                let shown = self.lines.len().saturating_sub(3);
                let k = (l - N_LINE) as usize;
                if k < shown {
                    return None;
                }
                [154, 308 + 20 * (k - shown) as i32, 340, 18]
            }
            _ => return None,
        })
    }

    // the page scrolls so that node id lies in view
    fn reveal(&mut self, id: u64) {
        if let Some(r) = self.place(id) {
            if r[1] < self.scroll || r[1] + r[3] > self.scroll + VIEW_H {
                self.scroll = (r[1] - 20).clamp(0, PAGE_H - VIEW_H);
            }
        }
    }

    fn log(&mut self, line: String) {
        self.lines.push(line);
    }

    /// the requests of a screen reader for the page's records (taken at the start of a frame)
    pub fn requests(&mut self, ui: &mut Ui) {
        while let Some(r) = ui.a11y_request_take() {
            self.seen.push(format!("{}:{}", r.id, act_name(&r.action)));
            let id = r.id;
            match r.action {
                A11yAction::ScrollIntoView => self.reveal(id),
                A11yAction::Focus => {
                    self.focus = id;
                    ui.focus(ID_PAGE);
                    self.reveal(id);
                }
                A11yAction::Click => self.click(id),
                A11yAction::Select => match id {
                    N_ROW1 | N_ROW2 => self.rows = [id == N_ROW1, id == N_ROW2],
                    N_DOCS | N_LETTERS | N_PICS => self.tree_sel = id,
                    N_POST | N_PICKUP => self.ship = (id == N_PICKUP) as i32,
                    _ => {}
                },
                A11yAction::AddToSelection => self.pick(id, true),
                A11yAction::RemoveFromSelection => self.pick(id, false),
                A11yAction::Expand if id == N_DOCS => self.open = true,
                A11yAction::Collapse if id == N_DOCS => self.open = false,
                A11yAction::SetText(t) if id == N_NAME => {
                    let n = t.chars().count() as u32;
                    self.name = t;
                    self.sel = (n, n);
                }
                A11yAction::SelectText(a, c) if id == N_NAME => self.sel = (a, c),
                A11yAction::SetValue(v) if id == N_QTY => self.qty = round(v).clamp(1, 10),
                A11yAction::Increment if id == N_QTY => self.qty = (self.qty + 1).min(10),
                A11yAction::Decrement if id == N_QTY => self.qty = (self.qty - 1).max(1),
                A11yAction::SetValue(v) if id == N_SCROLL => self.scroll = round(v).clamp(0, PAGE_H - VIEW_H),
                A11yAction::Increment if id == N_SCROLL => self.scroll = (self.scroll + 40).min(PAGE_H - VIEW_H),
                A11yAction::Decrement if id == N_SCROLL => self.scroll = (self.scroll - 40).max(0),
                _ => {}
            }
        }
    }

    // a table row added to (on) or taken out of the selection
    fn pick(&mut self, id: u64, on: bool) {
        match id {
            N_ROW1 => self.rows[0] = on,
            N_ROW2 => self.rows[1] = on,
            _ => {}
        }
    }

    // a click on a node of the page
    fn click(&mut self, id: u64) {
        match id {
            N_START | N_HELP | N_IMPRINT => {
                self.clicked = match id {
                    N_START => "Start",
                    N_HELP => "Hilfe",
                    _ => "Impressum",
                };
                self.log(format!("Verweis {}", self.clicked));
            }
            N_BOLD => self.bold = !self.bold,
            N_ITALIC => self.italic = !self.italic,
            N_POST | N_PICKUP => self.ship = (id == N_PICKUP) as i32,
            // (a click on an item that opens: selected, and opened or closed, as a page's twisty does)
            N_DOCS => {
                self.tree_sel = id;
                self.open = !self.open;
            }
            N_LETTERS | N_PICS => self.tree_sel = id,
            N_ROW1 => self.rows[0] = !self.rows[0],
            N_ROW2 => self.rows[1] = !self.rows[1],
            N_SEND => {
                self.alert = self.name.is_empty();
                if !self.alert {
                    self.log(format!("Bestellt: {} x {}", self.qty, self.name));
                }
            }
            _ => {}
        }
    }

    /// the buttons around the page, the texts that say what happened, the page
    pub fn frame(&mut self, ui: &mut Ui) {
        self.requests(ui);
        ui.box_begin(0, BOX_V | pad(12), 8);
        ui.heading(1, TS_SUBTITLE, "Barrierefreiheit");
        ui.box_begin(0, BOX_H, 8);
        if ui.button(ID_ANNOUNCE, "Ansage") & EV_ACT != 0 {
            ui.announce("Gespeichert", LIVE_POLITE);
        }
        if ui.button(ID_STATUS, "Status") & EV_ACT != 0 {
            self.status = "Gespeichert";
        }
        if ui.button(ID_DOWN, "Nach unten") & EV_ACT != 0 {
            self.scroll = PAGE_H - VIEW_H;
        }
        ui.box_end();
        let last = self.seen.last().map_or("-", |s| s.as_str());
        ui.text(TS_BODY, &format!("Anfrage: {last} ({})", self.seen.len()));
        ui.text(TS_BODY, &format!("Geklickt: {} Zeilen: {}{} Baum: {}", self.clicked, self.rows[0] as u8, self.rows[1] as u8, self.open as u8));
        ui.text(TS_BODY, &format!("Name: {} Menge: {} Versand: {} Fett: {}", self.name, self.qty, self.ship, self.bold as u8));
        self.page(ui);
        ui.box_end();
    }

    // a record of the page: kind, id, its label and value at its place (scrolled with the page
    // unless fixed), out of view marked AS_OFFSCREEN, drawn when it lies in view
    #[allow(clippy::too_many_arguments)]
    fn rec<'a>(&self, ui: &'a mut Ui, view: [i32; 4], kind: u16, id: u64, parent: u64, label: &str, value: i32, fixed: bool) -> &'a mut Sem {
        let p = self.place(id).unwrap_or([0; 4]);
        let y = if fixed { p[1] } else { p[1] - self.scroll };
        let r = [view[0] + ui.dp(p[0]), view[1] + ui.dp(y), ui.dp(p[2]), ui.dp(p[3])];
        let inside = y + p[3] > 0 && y < VIEW_H;
        if inside {
            ui.draw_color(match kind {
                K_LINK => 0xFF20_60E0,
                K_ALERT => 0xFFC0_2020,
                _ => 0xFF30_3030,
            });
            if matches!(kind, K_TABLE | K_ROW | K_CELL | K_COLHEADER | K_ROWHEADER | K_TEXTBOX | K_SPIN | K_BUTTON | K_TREE | K_LOG | K_METER) {
                ui.draw_frame(r[0], r[1], r[2], r[3], 0, 1);
            }
            if !label.is_empty() && !matches!(kind, K_LANDMARK | K_TABLE | K_ROW | K_TREE | K_RADIOGROUP | K_TOOLBAR | K_LOG | K_ARTICLE | K_FIGURE) {
                ui.emit_text(r[0] + ui.dp(4), r[1] + r[3] * 3 / 4, label, ui.draw_color_get());
            }
        }
        let focused = self.has_focus && self.focus == id;
        let rec = ui.sem_rect(kind, 0, r, Some(label), value).id64(id).parent(parent);
        if !inside {
            rec.states(AS_OFFSCREEN);
        }
        if focused {
            rec.state |= SS_FOCUSED;
        }
        rec
    }

    // the page: a canvas and the records of what it shows
    fn page(&mut self, ui: &mut Ui) {
        ui.next_size(VIEW_W, VIEW_H);
        ui.canvas_label("Beispielseite");
        let inp = ui.canvas(ID_PAGE, None, 0, 0).clone();
        let v = inp.rect;
        self.has_focus = inp.ev & EV_FOCUS != 0;
        ui.draw_color(0xFFFF_FFFF);
        ui.draw_rect(v[0], v[1], v[2], v[3]);
        ui.clip_push(v[0], v[1], v[2], v[3]);
        let page = ID_PAGE as u64;
        // the banner and its heading, the navigation with two links, the complementary column
        self.rec(ui, v, K_LANDMARK, N_BANNER, page, "Kopf", LM_BANNER, false);
        self.rec(ui, v, K_HEADING, N_TITLE, N_BANNER, "Beispielseite", 1, false);
        self.rec(ui, v, K_LANDMARK, N_NAV, page, "Hauptmenü", LM_NAVIGATION, false);
        self.rec(ui, v, K_LINK, N_START, N_NAV, "Start", 0, false).url("https://example.org/").states(AS_VISITED).set_pos(1, 2);
        self.rec(ui, v, K_LINK, N_HELP, N_NAV, "Hilfe", 0, false).url("https://example.org/hilfe").set_pos(2, 2);
        self.rec(ui, v, K_LANDMARK, N_ASIDE, page, "Randspalte", LM_COMPLEMENTARY, false);
        self.rec(ui, v, K_TEXT, N_TIPTEXT, N_ASIDE, "Tipp des Tages", 0, false);
        // the main content
        self.rec(ui, v, K_LANDMARK, N_MAIN, page, "", LM_MAIN, false);
        // a table: a header row, two rows with a row header each (rows selectable, several at once)
        self.rec(ui, v, K_TABLE, N_TABLE, N_MAIN, "Preise", 0, false).grid(3, 2).states(AS_MULTISELECT);
        self.rec(ui, v, K_ROW, N_ROW0, N_TABLE, "Artikel Preis", 0, false);
        self.rec(ui, v, K_COLHEADER, N_H_ITEM, N_ROW0, "Artikel", 0, false).cell(0, 0);
        self.rec(ui, v, K_COLHEADER, N_H_PRICE, N_ROW0, "Preis", 0, false).cell(0, 1);
        for (k, (row, head, cell, h, c)) in [(N_ROW1, N_R_APPLE, N_C_APPLE, "Apfel", "1,20 €"), (N_ROW2, N_R_PEAR, N_C_PEAR, "Birne", "0,90 €")].into_iter().enumerate() {
            // (a row is named by its cells, as a page's rows are)
            let rl = format!("{h} {c}");
            let rec = self.rec(ui, v, K_ROW, row, N_TABLE, &rl, 0, false).states(AS_SELECTABLE).set_pos(k as u32 + 1, 2);
            if self.rows[k] {
                rec.state |= SS_SELECTED;
            }
            self.rec(ui, v, K_ROWHEADER, head, row, h, 0, false).cell(k as u32 + 1, 0);
            self.rec(ui, v, K_CELL, cell, row, c, 0, false).cell(k as u32 + 1, 1);
        }
        // a tree: "Dokumente" opens to "Briefe" (one level deeper), "Bilder" is closed
        self.rec(ui, v, K_TREE, N_TREE, N_MAIN, "Ordner", 0, false);
        let open = self.open;
        let docs = self.rec(ui, v, K_TREEITEM, N_DOCS, N_TREE, "Dokumente", 0, false).level(1).set_pos(1, 2);
        if open {
            docs.state |= SS_OPEN;
        } else {
            docs.states(AS_COLLAPSED);
        }
        if self.tree_sel == N_DOCS {
            docs.state |= SS_SELECTED;
        }
        if open {
            let letters = self.rec(ui, v, K_TREEITEM, N_LETTERS, N_DOCS, "Briefe", 0, false).level(2).set_pos(1, 1);
            if self.tree_sel == N_LETTERS {
                letters.state |= SS_SELECTED;
            }
        }
        let pics = self.rec(ui, v, K_TREEITEM, N_PICS, N_TREE, "Bilder", 0, false).level(1).set_pos(2, 2).states(AS_COLLAPSED);
        if self.tree_sel == N_PICS {
            pics.state |= SS_SELECTED;
        }
        // a form: a text box with a name and a description, a spin button, radio buttons, a tool bar
        self.rec(ui, v, K_LANDMARK, N_FORM, N_MAIN, "Bestellung", LM_FORM, false);
        let (name, sel) = (self.name.clone(), self.sel);
        let tb = self.rec(ui, v, K_TEXTBOX, N_NAME, N_FORM, &name, 0, false).name("Name").desc("Vor- und Nachname").states(AS_REQUIRED);
        if name.is_empty() {
            tb.states(AS_INVALID);
        }
        if tb.state & SS_FOCUSED != 0 {
            tb.text_sel(sel.0, sel.1);
        }
        let qty = self.qty;
        self.rec(ui, v, K_SPIN, N_QTY, N_FORM, "Menge", qty, false).range(qty as f64, 1.0, 10.0, 1.0);
        self.rec(ui, v, K_RADIOGROUP, N_SHIP, N_FORM, "Versand", 0, false);
        for (k, (id, label)) in [(N_POST, "Post"), (N_PICKUP, "Abholung")].into_iter().enumerate() {
            let on = self.ship == k as i32;
            let r = self.rec(ui, v, K_RADIO, id, N_SHIP, label, 0, false).set_pos(k as u32 + 1, 2);
            if on {
                r.state |= SS_CHECKED;
            }
        }
        self.rec(ui, v, K_BUTTON, N_SEND, N_FORM, "Absenden", 0, false);
        self.rec(ui, v, K_TOOLBAR, N_TOOLS, N_FORM, "Werkzeuge", 0, false);
        let (bold, italic) = (self.bold, self.italic);
        let b = self.rec(ui, v, K_BUTTON, N_BOLD, N_TOOLS, "Fett", 0, false);
        if bold {
            b.state |= SS_CHECKED;
        }
        self.rec(ui, v, K_SEPARATOR, N_SEP, N_TOOLS, "", 0, false);
        let i = self.rec(ui, v, K_BUTTON, N_ITALIC, N_TOOLS, "Kursiv", 0, false);
        if italic {
            i.state |= SS_CHECKED;
        }
        // a status line and a log (live regions), a meter, a tooltip, an alert
        self.rec(ui, v, K_STATUS, N_STATUS, N_MAIN, self.status, 0, false);
        self.rec(ui, v, K_METER, N_METER, N_MAIN, "Speicher", 40, false).range(0.4, 0.0, 1.0, 0.0).value_text("40 %");
        self.rec(ui, v, K_TOOLTIP, N_TIP, N_NAME, "Pflichtfeld: Vor- und Nachname", 0, false);
        self.rec(ui, v, K_LOG, N_LOG, N_MAIN, "Verlauf", 0, false);
        let shown = self.lines.len().saturating_sub(3);
        for k in shown..self.lines.len() {
            let line = self.lines[k].clone();
            self.rec(ui, v, K_TEXT, N_LINE + k as u64, N_LOG, &line, 0, false);
        }
        if self.alert {
            self.rec(ui, v, K_ALERT, N_ALERT, N_MAIN, "Bitte einen Namen eingeben", 0, false);
        }
        // below the view: an article with a heading and paragraphs, a figure, the footer
        self.rec(ui, v, K_ARTICLE, N_ARTICLE, N_MAIN, "Neuigkeiten", 0, false);
        self.rec(ui, v, K_HEADING, N_NEWS, N_ARTICLE, "Neuigkeiten", 2, false);
        for (k, p) in PARAS.iter().enumerate() {
            self.rec(ui, v, K_TEXT, N_PARA + k as u64, N_ARTICLE, p, 0, false);
        }
        self.rec(ui, v, K_FIGURE, N_FIGURE, N_MAIN, "Umsatz 2026", 0, false);
        self.rec(ui, v, K_IMAGE, N_CHART, N_FIGURE, "Balkendiagramm", 0, false).desc("Drei Balken, der letzte am höchsten");
        self.rec(ui, v, K_LANDMARK, N_FOOTER, page, "Fußzeile", LM_CONTENTINFO, false);
        self.rec(ui, v, K_TEXT, N_COPY, N_FOOTER, "© 2026 Beispiel", 0, false);
        self.rec(ui, v, K_LINK, N_IMPRINT, N_FOOTER, "Impressum", 0, false).url("https://example.org/impressum");
        ui.clip_pop();
        // the page's scroll bar (fixed at the right edge of the view, not scrolled)
        let sb = [v[0] + v[2] - ui.dp(14), v[1], ui.dp(14), v[3]];
        ui.draw_color(0xFFD0_D0D0);
        ui.draw_rect(sb[0], sb[1], sb[2], sb[3]);
        let max = PAGE_H - VIEW_H;
        ui.sem_rect(K_SCROLLBAR, 0, sb, Some("Seite"), self.scroll).id64(N_SCROLL).parent(page).range(self.scroll as f64, 0.0, max as f64, 40.0);
    }
}
