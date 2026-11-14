// viewtest -- a canvas that acts like a web view (tests of ui's extensions for apps that show
// content drawn by an engine of their own, docs/GUIDE.md section 5 "Canvas"):
//
//   the page (ID_PAGE)   HB_KEYS while "Tastensenke" is on: every key is the page's, Ctrl + letter
//                        and Tab too (ui.key_take); F5 the page does not use: back to the command
//                        table (ui.accel_run); the arrow cursor (HB_CURSOR_SET), the hand over a link
//   the plain canvas     the defaults: crosshair, accelerators and Tab as everywhere
//   menus at a point     a right click on the page opens its menu one pass later, where the
//                        click was (ui.menu_popup_at), as when an engine answers; "Menü später"
//                        opens a flyout of the app's own at a point of the page (ui.popup_open_at)
//   the page's record    named "Webseite" (ui.canvas_label, E14)
//   the links            drawn by the page itself, published as K_LINK records without a hit box
//                        (ui.sem_rect): a script clicks one by its label, the page gets the click
//   "Dateien öffnen"     the file dialog with FD_MULTI in the folder --files DIR (E8): several
//                        files at once, ui.filedlg_paths() shown as "Dateien: N a b"
//   "Ordner wählen"      a folder dialog (FD_FOLDER) in --files DIR: filedlg_paths() is the folder
//   "Notiz"              a text box; the app window has one of the same id (each keeps its caret)
//   "Kette"              three commands, each queued by the frame that saw the one before (the
//                        three command rounds of a frame; the frame after them shows "Kette: 3")
//   "Rohe Tasten"        (E2) the page takes the keys as they go down and up (ui.key_event_take):
//                        "Roh:" lists them (+ down, - up, C- S- A- held modifiers, * repeat, :L :R :N
//                        location, #scancode)
//   "Eingabefeld"        (E4) the page has an editable field of its own: ui.ime_enable while it
//                        has the focus, ui.ime_pos at its caret, the composition from
//                        ui.ime_preedit drawn by the page ("IME:"), the committed text from
//                        text_take ("Feld:")
//   "Neues Fenster"      (E7) an independent app window (WF_APP) with a title bar of its own
//                        (titlebar_begin / titlebar_end: a tab in it, the caption buttons);
//                        --app-window opens it at the start
//   "Zeiger wechseln"    (E3) the page's cursor through ui.cursor_next: not-allowed, grab,
//                        zoom-in, the north-east corner, a custom cursor of the page's own;
//                        "Eigener größer" redraws it (ui.cursor_replace), "Eigener frei" frees it
//                        (ui.cursor_free); "Zeiger:" says ui.cursor_at under the pointer
//   "Hintergrund"        a thread of the app's own works 300 ms of real time, then plat::wake
//                        (an engine's thread): UIC_WAKE shows it ("Hintergrund: N"); meanwhile a
//                        timer polls every 100 ms ("Abfrage: läuft / fertig", an extension
//                        page's poll); tests/realwait.uis with `wait real`
//   "Hängt"              the sync hook says a job is running (`sync MS soft`)
//   "Kennwort"           a TB_PASSWORD box: the input method's purpose ("Zweck: N",
//                        plat::ime_purpose_of) is IP_PASSWORD while it has the keyboard, set by
//                        the box itself; command 6 takes the box away while it has it
//                        (tests/password.uis)
//   "Datum"              a flyout of the app's own at a point of the page (BrowserGUI's picker
//                        of an <input type=date>) with a date_picker in it, whose day closes its
//                        calendar only, and ui.calendar in place: days, ISO weeks or months
//                        between 2026-11-05 and 2027-02-10 (tests/calendar.uis)
//   Ctrl+N               a window of its own ("Teiler", as a browser's new window); its "Teilen"
//                        shows four splits inside each other at once, each taking its
//                        orientation and first position from the room it gets (as developer
//                        tools dock), the innermost pane a wrapping text over a box that fills the
//                        rest: new boxes inside new fill boxes get their size in their first pass
//                        and settle without "layout did not settle" (tests/nested.uis)
//   command 8            two app windows ("Fenster A", "Fenster B", as two browser windows), each
//                        with a title bar of its own whose tabs differ (one short tab, three
//                        long ones) and a sheet box of the same fixed id with a text box of the
//                        same fixed id inside, the second one with a long wrapping paragraph:
//                        every window keeps its own measurements of an id, so neither title bar
//                        nor sheet flips between the windows' sizes (tests/titlebars.uis); the
//                        paragraph is found by its whole text (tests/longtext.uis)
//
// tests/keys.uis, tests/menus.uis and tests/links.uis drive it; the texts above the page say what
// it saw.
#![no_std]
#![no_main]
// forbid, not deny: ui::main! needs no allow (the library's styles forbid unsafe code as well)
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use plat::a11y::Role;
use ui::*;

const ID_SINK: u32 = 100;
const ID_PAGE: u32 = 101;
const ID_PLAIN: u32 = 102;
const ID_LATER: u32 = 103; // "Menü später"
const ID_FLYOUT: u32 = 104; // the flyout it opens at a point of the page
const ID_ROW: u32 = 105; // its row
const ID_TREE: u32 = 106; // "Baum prüfen": the links of the accessibility tree
const ID_FILES: u32 = 107; // "Dateien öffnen"
const ID_CURSOR: u32 = 108; // "Zeiger wechseln"
const ID_CBIG: u32 = 109; // "Eigener größer"
const ID_CFREE: u32 = 110; // "Eigener frei"
const ID_RAW: u32 = 111; // "Rohe Tasten"
const ID_IME: u32 = 112; // "Eingabefeld"
const ID_NEWWIN: u32 = 113; // "Neues Fenster"
const ID_APPWIN: u32 = 114; // the app window
const ID_APPTAB: u32 = 115; // its tab in the title bar
const ID_DIRS: u32 = 116; // "Ordner wählen"
const ID_NOTE: u32 = 117; // "Notiz": a text box of this id in the main window and in the app window
const ID_CHAIN: u32 = 118; // "Kette"
const ID_AGAIN: u32 = 119; // "Immer neu"
const ID_WORK: u32 = 120; // "Hintergrund"
const ID_HANG: u32 = 121; // "Hängt"
const ID_PW: u32 = 122; // "Kennwort" (TB_PASSWORD)
const ID_DATEBTN: u32 = 123; // "Datum": opens ID_DATEPOP at a point of the page
const ID_DATEPOP: u32 = 124; // that flyout
const ID_DATE: u32 = 125; // the date_picker in it (its calendar's days: #0x800F1040 + cell)
const ID_CALMODE: u32 = 126; // "Tag" / "Woche" / "Monat"
const ID_CAL: u32 = 127; // the calendar in place (cells: #0x800F10C0 + day, week row or month)
const ID_NESTWIN: u32 = 128; // "Teiler" (Ctrl+N)
const ID_NESTBTN: u32 = 129; // its "Teilen"
const ID_SPLIT: u32 = 136; // its four splits: 136 .. 139
const ID_NESTIN: u32 = 133; // the innermost pane's column: the wrapping text, then ID_NESTFILL
const ID_NESTFILL: u32 = 134; // the box that fills the rest of it
const ID_NESTAREA: u32 = 135; // the area of "Teiler" the splits fill
const ID_TBWIN: u32 = 140; // command 8: "Fenster A" (140) and "Fenster B" (141)
const ID_TBTAB: u32 = 142; // their tabs in the title bars: 142 .. 145
const ID_SHEET: u32 = 150; // the sheet box of both windows (one fixed id)
const ID_SHEET_TEXT: u32 = ID_SHEET + 3; // its text box (one fixed id in both windows)
const ID_FD: u32 = 0x10_0000; // the file dialog (ids .. + 0x3FFF; rows + 17 + i, name box + 12)
const ID_FD_DIR: u32 = 0x11_0000; // the folder dialog (FD_FOLDER)

const CMD_NEW_TAB: u32 = 1;
const CMD_RELOAD: u32 = 2;
const CMD_COPY: u32 = 3; // a row of the page's menu (Mi::Item)
const CMD_TREE: u32 = 4; // after the frame: the tree is complete
const CMD_CHAIN: u32 = 5; // "Kette": one link of the chain
const CMD_PW_AWAY: u32 = 6; // the password box goes away (and comes back), as with a closed window
const CMD_NESTED: u32 = 7; // Ctrl+N: the window "Teiler"
const CMD_TWO: u32 = 8; // "Fenster A" and "Fenster B"

const T_POLL: u32 = 1; // "Hintergrund": the timer that polls the thread's result

// "Hintergrund": the work the app's thread finished (it counts up, then plat::wake)
static WORK_DONE: AtomicU32 = AtomicU32::new(0);
// "Hängt": the sync hook reports a job that never ends
static HANG: AtomicBool = AtomicBool::new(false);

fn sync_pending() -> bool {
    HANG.load(Ordering::Relaxed)
}

// the page's context menu
static PAGE_MENU: [Mi; 3] = [Mi::Cmd(CMD_RELOAD), Mi::Sep, Mi::Item(CMD_COPY, "Link kopieren", 0)];

static CMDS: [Command; 3] = [
    Command::new(CMD_NEW_TAB, "Neuer Tab", 0, MOD_CTRL, VK_T, 0),
    Command::new(CMD_RELOAD, "Neu laden", 0, 0, VK_F5, 0),
    Command::new(CMD_NESTED, "Neues Fenster mit Teilern", 0, MOD_CTRL, VK_N, 0),
];

// the innermost pane's text of "Teiler": long enough to wrap there
const NEST_TEXT: &str = "Ein Text, der hier umbricht: seine Höhe hängt von der Breite ab, die vier neue Teiler ihm lassen.";

// the paragraph of "Fenster B": about 600 bytes with umlauts and quotes, wrapped (a selector and
// `dump` take its whole text)
const LONG_TEXT: &str = "Ein langer Absatz, wie ihn eine Webseite zeigt: „Grüße aus dem Fenster B“ steht am Anfang, \
    dann folgen Sätze, die über viele Zeilen umbrechen, damit der Text deutlich länger wird als die 255 Bytes, \
    die ein Wähler früher fassen konnte, und länger als die 172 Bytes, die `dump` früher zeigte. Äpfel, Öfen und \
    Übungen sorgen für Zeichen aus zwei Bytes; ein Rückstrich \\ und \"Anführungszeichen\" stehen auch darin. \
    Am Ende steht ein Satz, an dem man sieht, dass alles angekommen ist: Ende des Absatzes.";

// the tabs of the two title bars: "Fenster A" one short one, "Fenster B" three long ones
const TABS_A: [&str; 1] = ["Start"];
const TABS_B: [&str; 3] = ["Eins", "Zwei mit einem langen Namen", "Drei, noch etwas länger"];

const DESC: AppDesc = AppDesc {
    title: "Ansichtstest",
    flags: AF_RESIZABLE,
    w: 560,
    h: 820,
    min_w: 400,
    min_h: 360,
    cmds: &CMDS,
    ..AppDesc::NONE
};

// the links the page draws: (x, y, w, h) in DIP inside the page, text
const LINKS: [([i32; 4], &str); 2] = [([16, 16, 140, 24], "Link Eins"), ([16, 56, 140, 24], "Link Zwei")];

struct ViewTest {
    sink: i32, // "Tastensenke": the page has HB_KEYS
    /// the keys the page took (key_text)
    keys: Vec<String>,
    new_tab: u32,
    reload: u32,
    /// the cursor the hit box under the pointer has (CUR_*)
    cursor: u32,
    /// the link under the pointer in the last pass (+1, 0 none)
    hover: u32,
    /// a right click on the page (px of the window): its menu opens in the next pass
    menu_at: Option<(i32, i32)>,
    /// "Menü später" was clicked: the flyout opens in the next pass
    later: bool,
    copy: u32,
    rows: u32,
    /// the link the page saw a click on
    clicked: &'static str,
    /// the names of the links in the accessibility tree ("Baum prüfen")
    tree: String,
    /// (E8) the file dialog: open, its path, the files it chose
    fd_open: Shared<i32>,
    fd_path: String,
    files: String,
    /// the folder dialog (FD_FOLDER): open, its path
    dir_open: Shared<i32>,
    dir_path: String,
    /// "Notiz" in the main window and in the app window (text boxes of the same id)
    note: String,
    app_note: String,
    /// "Kette": the links the commands counted, the link the frame last queued a command for
    chain: u32,
    chain_seen: u32,
    /// "Immer neu": every frame asks for another one (ui.invalidate in App::frame)
    again: i32,
    /// (E3) the page's cursor (an index into PAGE_CURSORS) and the custom one (handle, 0 none)
    page_cur: usize,
    custom: u32,
    /// (E2) "Rohe Tasten": the page takes KeyEvents; what it took
    raw: i32,
    raw_keys: Vec<String>,
    /// (E4) "Eingabefeld": the page's own field; whether the input method is on for it, its text
    ime: i32,
    ime_on: bool,
    field: String,
    /// (E7) the app window: open, clicks on its tab
    app_open: Shared<i32>,
    app_rect: Shared<WinRect>,
    app_tab: u32,
    /// "Hintergrund": the thread's work UIC_WAKE saw, the poll timer's state; "Hängt"
    work_seen: u32,
    polled: &'static str,
    hang: i32,
    /// "Kennwort": its text, taken away (CMD_PW_AWAY)
    pw: String,
    pw_away: bool,
    /// "Datum": the flyout opens in the next pass; the date picker's date, the calendar's mode,
    /// date and picks
    date_later: bool,
    date: (i32, u32, u32),
    cal_mode: i32,
    cal: (i32, u32, u32),
    cal_picks: u32,
    /// Ctrl+N: the window "Teiler", its splits shown ("Teilen"), their positions once moved
    nest_open: Shared<i32>,
    nest_rect: Shared<WinRect>,
    nested: bool,
    split: [Option<i32>; 4],
    /// command 8: "Fenster A" and "Fenster B" (open, placement), the tab each shows, the text of
    /// each one's sheet box
    two_open: [Shared<i32>; 2],
    two_rect: [Shared<WinRect>; 2],
    two_tab: [usize; 2],
    two_text: [String; 2],
}

// (E3) the cursors "Zeiger wechseln" walks through (0: the page's arrow / hand)
const PAGE_CURSORS: [u32; 6] = [0, CUR_NOT_ALLOWED, CUR_GRAB, CUR_ZOOM_IN, CUR_SIZE_NE, CUR_CUSTOM];

// a custom cursor of w x w DIP: a filled square with a frame
fn square(w: u32) -> CursorDraw {
    Rc::new(move |g: &mut ui::Gfx, dpi: i32| {
        let p = ((w as i32) * dpi + 95) / 96;
        g.dc.set_color(0xFF00_0000);
        g.dc.fill(g.sf, 0, 0, p, p);
        g.dc.set_color(0xFFFF_C000);
        g.dc.fill(g.sf, 1, 1, p - 2, p - 2);
    })
}

impl Default for ViewTest {
    fn default() -> Self {
        ViewTest {
            sink: 1,
            keys: Vec::new(),
            new_tab: 0,
            reload: 0,
            cursor: u32::MAX,
            hover: 0,
            menu_at: None,
            later: false,
            copy: 0,
            rows: 0,
            clicked: "-",
            tree: String::new(),
            fd_open: Shared::new(0),
            fd_path: String::new(),
            files: String::from("-"),
            dir_open: Shared::new(0),
            dir_path: String::new(),
            note: String::new(),
            app_note: String::new(),
            chain: 0,
            chain_seen: 0,
            again: 0,
            page_cur: 0,
            custom: 0,
            raw: 0,
            raw_keys: Vec::new(),
            ime: 0,
            ime_on: false,
            field: String::new(),
            app_open: Shared::new(0),
            app_rect: Shared::new(WinRect::new(0, 0, 480, 240, 0)),
            app_tab: 0,
            work_seen: 0,
            polled: "-",
            hang: 0,
            pw: String::new(),
            pw_away: false,
            date_later: false,
            date: (2026, 11, 13),
            cal_mode: 0,
            cal: (2026, 11, 13),
            cal_picks: 0,
            nest_open: Shared::new(0),
            nest_rect: Shared::new(WinRect::new(40, 40, 800, 600, 0)),
            nested: false,
            split: [None; 4],
            two_open: [Shared::new(0), Shared::new(0)],
            two_rect: [Shared::new(WinRect::new(60, 60, 520, 360, 0)), Shared::new(WinRect::new(120, 120, 640, 420, 0))],
            two_tab: [0; 2],
            two_text: [String::from("Blatt A"), String::from("Blatt B")],
        }
    }
}

// "2026-11-13"
fn ymd((y, m, d): (i32, u32, u32)) -> String {
    format!("{y}-{m:02}-{d:02}")
}

// the link of the page at (x, y) px relative to the page
fn link_at(ui: &Ui, x: i32, y: i32) -> Option<usize> {
    LINKS.iter().position(|(r, _)| {
        let (lx, ly, lw, lh) = (ui.dp(r[0]), ui.dp(r[1]), ui.dp(r[2]), ui.dp(r[3]));
        x >= lx && y >= ly && x < lx + lw && y < ly + lh
    })
}

// (E2) a key's name in "Roh:" (the modifiers by their own names)
fn raw_name(ui: &Ui, vk: u32) -> String {
    match vk {
        VK_SHIFT => String::from("Umschalt"),
        VK_CONTROL => String::from("Strg"),
        VK_MENU => String::from("Alt"),
        _ => ui.key_text(vk, 0),
    }
}

fn cursor_name(c: u32) -> &'static str {
    match c {
        CUR_ARROW => "Pfeil",
        CUR_HAND => "Hand",
        CUR_CROSS => "Kreuz",
        CUR_NOT_ALLOWED => "nicht erlaubt",
        CUR_GRAB => "Greifen",
        CUR_ZOOM_IN => "Vergrößern",
        CUR_SIZE_NE => "Größe NO",
        CUR_CUSTOM => "eigener",
        u32::MAX => "-",
        _ => "anderer",
    }
}

impl ViewTest {
    // the page: a canvas with HB_KEYS (while the switch is on) and the cursor it asks for
    fn page(&mut self, ui: &mut Ui) {
        ui.next_size(360, 120);
        let hover = self.hover;
        let mut cur = if hover != 0 { CUR_HAND } else { CUR_ARROW };
        ui.next_flags(HB_CURSOR_SET | cur << HB_CURSOR_SHIFT | if self.sink != 0 { HB_KEYS } else { 0 });
        // (E3) a cursor of its own: any shape through cursor_next, the custom one through CUR_CUSTOM
        let own = PAGE_CURSORS[self.page_cur];
        if own != 0 {
            if own == CUR_CUSTOM {
                ui.cursor_custom(self.custom);
            }
            ui.cursor_next(own);
            cur = own;
        }
        ui.canvas_label("Webseite"); // (E14) the name of the page's own record
        let inp = ui.canvas(ID_PAGE, None, 0, 0).clone();
        let r = inp.rect;
        ui.draw_color(0xFFFF_FFFF);
        ui.draw_rect(r[0], r[1], r[2], r[3]);
        for (i, (l, text)) in LINKS.iter().enumerate() {
            let lr = [r[0] + ui.dp(l[0]), r[1] + ui.dp(l[1]), ui.dp(l[2]), ui.dp(l[3])];
            ui.draw_color(if hover == i as u32 + 1 { 0xFF00_40C0 } else { 0xFF20_60E0 });
            ui.draw_rect(lr[0], lr[1], lr[2], lr[3]);
            // what an engine's tree would say: a link with its text, no hit box of its own
            ui.sem_rect(K_LINK, 0, lr, Some(text), 0);
        }
        // a click on the page: on a link?
        if inp.ev & EV_ACT != 0 {
            if let Some(i) = link_at(ui, inp.press_x, inp.press_y) {
                self.clicked = LINKS[i].1;
                ui.repass();
            }
        }
        let now = if inp.ev & EV_HOVER != 0 { link_at(ui, inp.x, inp.y).map_or(0, |i| i as u32 + 1) } else { 0 };
        if now != hover {
            self.hover = now;
            ui.repass();
        }
        // (E4) the page's own field: the input method while it has the focus, at its caret
        let want = self.ime != 0 && inp.ev & EV_FOCUS != 0;
        if want != self.ime_on {
            self.ime_on = want;
            ui.ime_enable(want, IP_TEXT);
        }
        if want {
            let caret = [r[0] + ui.dp(16), r[1] + ui.dp(96), ui.dp(18)];
            ui.ime_pos(caret[0], caret[1], caret[2]);
            for c in ui.text_take(256) {
                if let Some(ch) = char::from_u32(c) {
                    self.field.push(ch);
                    ui.repass();
                }
            }
        }
        // (E2) the keys as they went down and up
        if self.sink != 0 && self.raw != 0 && inp.ev & EV_FOCUS != 0 {
            while let Some(k) = ui.key_event_take() {
                let loc = match k.location {
                    KL_LEFT => ":L",
                    KL_RIGHT => ":R",
                    KL_NUMPAD => ":N",
                    _ => "",
                };
                let scan = if k.scancode != 0 { format!("#{}", k.scancode) } else { String::new() };
                // the modifiers held with another key: C- Ctrl, S- Shift, A- Alt
                let mut mods = String::new();
                for (m, vk, t) in [(MOD_CTRL, VK_CONTROL, "C-"), (MOD_SHIFT, VK_SHIFT, "S-"), (MOD_ALT, VK_MENU, "A-")] {
                    if k.mods & m != 0 && k.vk != vk {
                        mods.push_str(t);
                    }
                }
                self.raw_keys.push(format!("{}{}{}{}{}{}", if k.down { "+" } else { "-" }, mods, raw_name(ui, k.vk), if k.repeat { "*" } else { "" }, loc, scan));
                ui.repass();
            }
        }
        // the keys: the page takes them all while it has the focus; F5 it hands back
        if self.sink != 0 && self.raw == 0 && inp.ev & EV_FOCUS != 0 {
            loop {
                let k = ui.key_take();
                if k == 0 {
                    break;
                }
                let (vk, m) = (k & 0xFFFF, k >> 16);
                if matches!(vk, VK_SHIFT | VK_CONTROL | VK_MENU) {
                    continue;
                }
                if vk == VK_F5 && m == 0 {
                    ui.accel_run(k);
                    continue;
                }
                self.keys.push(ui.key_text(vk, m));
                ui.repass();
            }
        }
        // a right click: the menu comes a pass later, where the click was (an engine answers)
        if inp.ev & EV_RCLICK != 0 {
            self.menu_at = Some((r[0] + inp.x, r[1] + inp.y));
            ui.repass();
        }
        // "Menü später": the flyout hangs at a point of the page
        if self.later {
            self.later = false;
            ui.popup_open_at(ID_FLYOUT, r[0] + ui.dp(200), r[1] + ui.dp(60));
        }
        // "Datum": the picker's flyout below a field of the page (an <input type=date>)
        if self.date_later {
            self.date_later = false;
            ui.popup_open_at(ID_DATEPOP, r[0] + ui.dp(16), r[1] + ui.dp(24));
        }
        // the cursor of the hit box under the pointer: the last pass's, once more when it is not
        // yet the one this pass asked for
        if inp.ev & EV_HOVER != 0 {
            let c = ui.cursor_at(r[0] + inp.x, r[1] + inp.y);
            if c != self.cursor || c != cur {
                self.cursor = c;
                ui.repass();
            }
        }
    }

    // "Datum": a flyout of the app's own (a web page's date input) with a calendar in place --
    // days, ISO weeks or months between 2026-11-05 and 2027-02-10 -- and a date picker, whose day
    // closes only the picker's calendar.  The picker comes last: a popup opened inside a flyout
    // is drawn where its flyout_begin is, so the flyout's later widgets would lie over it
    fn date_flyout(&mut self, ui: &mut Ui) {
        if ui.flyout_begin(ID_DATEPOP, 0) {
            ui.segmented(ID_CALMODE, &mut self.cal_mode, &["Tag", "Woche", "Monat"]);
            let mode = [CAL_DAY, CAL_WEEK, CAL_MONTH][self.cal_mode.clamp(0, 2) as usize];
            let range = DateRange::new(mode, (2026, 11, 5), (2027, 2, 10));
            let (mut y, mut m, mut d) = self.cal;
            if ui.calendar(ID_CAL, &mut y, &mut m, &mut d, &range) & EV_ACT != 0 {
                self.cal = (y, m, d);
                self.cal_picks += 1;
            }
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.text(TS_BODY, "Termin");
            let (mut y, mut m, mut d) = self.date;
            if ui.date_picker(ID_DATE, &mut y, &mut m, &mut d) & EV_ACT != 0 {
                self.date = (y, m, d);
            }
            ui.box_end();
        }
        ui.flyout_end();
    }

    // Ctrl+N: "Teiler", a window of its own.  "Teilen" (below the area, so the splits come a pass
    // after the click) shows four splits inside each other, each one new in the filling pane of
    // the one before; as a browser's developer tools dock by the room they have, each split takes
    // its orientation and first position from the size of the box it goes into (wider than twice
    // its height: side by side, else one above the other; half of it).  The innermost pane is a
    // column of a wrapping text and a box that fills the rest
    fn nested_window(&mut self, ui: &mut Ui) {
        let (open, rect) = (self.nest_open.clone(), self.nest_rect.clone());
        if ui.window_begin(ID_NESTWIN, "Teiler", Some(&open), WF_APP | WF_RESIZABLE, Some(&rect)) {
            ui.next_size(-1, -1);
            ui.box_begin(0, BOX_V | pad(8), 8);
            ui.next_size(-1, -1);
            ui.box_begin(ID_NESTAREA, BOX_V, 0);
            if self.nested {
                self.splits(ui);
            }
            ui.box_end();
            if ui.button(ID_NESTBTN, "Teilen") & EV_ACT != 0 {
                self.nested = !self.nested;
                ui.repass();
            }
            ui.box_end();
        }
        ui.window_end();
    }

    // command 8: "Fenster A" (k = 0) and "Fenster B" (k = 1), two app windows built from the same
    // code with ids of their own fixed values: the title bar (TB_ID inside the library) with its
    // tabs, the sheet box ID_SHEET and its text box ID_SHEET_TEXT
    fn two_windows(&mut self, ui: &mut Ui) {
        for k in 0..2 {
            let (open, rect) = (self.two_open[k].clone(), self.two_rect[k].clone());
            let title = if k == 0 { "Fenster A" } else { "Fenster B" };
            if ui.window_begin(ID_TBWIN + k as u32, title, Some(&open), WF_APP | WF_RESIZABLE, Some(&rect)) {
                ui.titlebar_begin(title, 0);
                let tabs: &[&str] = if k == 0 { &TABS_A } else { &TABS_B };
                for (i, t) in tabs.iter().enumerate() {
                    if ui.button(ID_TBTAB + i as u32, t) & EV_ACT != 0 {
                        self.two_tab[k] = i;
                    }
                }
                ui.titlebar_end();
                ui.next_size(-1, 0);
                ui.box_begin(ID_SHEET, BOX_V | BOX_CARD | pad(12), 8);
                ui.text(TS_BODY, &format!("Reiter: {}", tabs[self.two_tab[k].min(tabs.len() - 1)]));
                if k == 1 {
                    ui.text(TS_BODY | TX_WRAP, LONG_TEXT);
                }
                ui.next_size(if k == 0 { 200 } else { 320 }, 0);
                ui.textbox(ID_SHEET_TEXT, Some("Blatt"), &mut self.two_text[k], 256);
                ui.box_end();
            }
            ui.window_end();
        }
    }

    // the four splits of "Teiler" and the innermost column
    fn splits(&mut self, ui: &mut Ui) {
        let names = ["Links", "Oben", "Mitte", "Innen"];
        for (i, name) in names.iter().enumerate() {
            let r = ui.box_rect();
            let (w, h) = (ui.px2dip(r[2]), ui.px2dip(r[3]));
            let vertical = w < 2 * h;
            let pos = self.split[i].unwrap_or(if vertical { h } else { w } / 2);
            ui.next_size(-1, -1);
            ui.split_begin(ID_SPLIT + i as u32, pos, vertical);
            ui.text(TS_BODY, name);
            ui.split_next();
        }
        ui.next_size(-1, -1);
        ui.box_begin(ID_NESTIN, BOX_V | pad(4), 4);
        ui.text(TS_BODY | TX_WRAP, NEST_TEXT);
        ui.next_size(-1, -1);
        ui.box_begin(ID_NESTFILL, BOX_V | BOX_BG, 0);
        ui.text(TS_BODY, "Rest");
        ui.box_end();
        ui.box_end();
        for i in (0..names.len()).rev() {
            let mut p = self.split[i].unwrap_or(0);
            if ui.split_end(&mut p) {
                self.split[i] = Some(p);
            }
        }
    }

    // a canvas with the defaults
    fn plain(&mut self, ui: &mut Ui) {
        ui.next_size(360, 60);
        let inp = ui.canvas(ID_PLAIN, None, 0, 0).clone();
        let r = inp.rect;
        ui.draw_color(0xFFE0_E0E0);
        ui.draw_rect(r[0], r[1], r[2], r[3]);
        if inp.ev & EV_HOVER != 0 {
            let c = ui.cursor_at(r[0] + inp.x, r[1] + inp.y);
            if c != self.cursor {
                self.cursor = c;
                ui.repass();
            }
        }
    }
}

impl App for ViewTest {
    fn init(&mut self, ui: &mut Ui) -> bool {
        // --app-window: the app window is open from the start (real-window checks without input,
        // tools/x11check.py appwin)
        if plat::args().iter().any(|a| a == "--app-window") {
            self.app_open.set(1);
        }
        ui.sync_hook_set(Some(sync_pending));
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_V | pad(12), 8);
        ui.box_begin(0, BOX_H, 16);
        ui.checkbox(ID_SINK, "Tastensenke", &mut self.sink);
        ui.checkbox(ID_RAW, "Rohe Tasten", &mut self.raw);
        ui.checkbox(ID_IME, "Eingabefeld", &mut self.ime);
        ui.box_end();
        ui.text(TS_BODY, &format!("Tasten: {}", self.keys.join(" ")));
        ui.text(TS_BODY, &format!("Befehle: Tab {} Laden {}", self.new_tab, self.reload));
        ui.text(TS_BODY, &format!("Zeiger: {}", cursor_name(self.cursor)));
        if let Some((x, y)) = self.menu_at.take() {
            ui.menu_popup_at(&PAGE_MENU, x, y);
        }
        self.page(ui);
        self.plain(ui);
        ui.text(TS_BODY, &format!("Roh: {}", self.raw_keys.join(" ")));
        // (E4) what the page's field shows: its text and the composition at its caret
        let pe = ui.ime_preedit();
        let pre = if pe.text.is_empty() { String::from("-") } else { format!("{} {}", pe.text, pe.cursor) };
        ui.text(TS_BODY, &format!("Feld: {} IME: {}", self.field, pre));
        ui.text(TS_BODY, &format!("Menü: Kopieren {} Eintrag {}", self.copy, self.rows));
        ui.text(TS_BODY, &format!("Geklickt: {}", self.clicked));
        if ui.button(ID_TREE, "Baum prüfen") & EV_ACT != 0 {
            ui.cmd_run(CMD_TREE, 0);
        }
        ui.text(TS_BODY, &format!("Baum: {}", self.tree));
        ui.box_begin(0, BOX_H, 8);
        if ui.button(ID_LATER, "Menü später") & EV_ACT != 0 {
            self.later = true;
        }
        if ui.button(ID_NEWWIN, "Neues Fenster") & EV_ACT != 0 {
            self.app_open.set(1);
        }
        if ui.button(ID_FILES, "Dateien öffnen") & EV_ACT != 0 {
            // the folder of --files, so the dialog starts there
            let dir = plat::arg_value("--files").unwrap_or_default();
            self.fd_path = format!("{}/", dir.trim_end_matches('/'));
            self.fd_open.set(1);
        }
        if ui.button(ID_DIRS, "Ordner wählen") & EV_ACT != 0 {
            // a folder dialog after a file dialog: filedlg_paths is the folder alone
            let dir = plat::arg_value("--files").unwrap_or_default();
            self.dir_path = String::from(dir.trim_end_matches('/'));
            self.dir_open.set(1);
        }
        ui.box_end();
        ui.text(TS_BODY, &format!("Dateien: {}", self.files));
        // (E3) the page's cursor and the custom one
        ui.box_begin(0, BOX_H, 8);
        if ui.button(ID_CURSOR, "Zeiger wechseln") & EV_ACT != 0 {
            self.page_cur = (self.page_cur + 1) % PAGE_CURSORS.len();
            if PAGE_CURSORS[self.page_cur] == CUR_CUSTOM && self.custom == 0 {
                self.custom = ui.cursor_new(square(16), 16, 16, 8 | 8 << 16);
            }
        }
        if ui.button(ID_CBIG, "Eigener größer") & EV_ACT != 0 && self.custom != 0 {
            ui.cursor_replace(self.custom, square(24), 24, 24, 12 | 12 << 16);
        }
        if ui.button(ID_CFREE, "Eigener frei") & EV_ACT != 0 && self.custom != 0 {
            ui.cursor_free(self.custom);
            self.custom = 0;
        }
        ui.box_end();
        let own = match ui.cursor_current() {
            0 => String::from("keiner"),
            h => match ui.cursor_render(h, 96) {
                Some(sf) => format!("{} {}x{}", h, sf.w, sf.h),
                None => format!("{h} frei"),
            },
        };
        ui.text(TS_BODY, &format!("Eigener: {own}"));
        ui.box_begin(0, BOX_H, 8);
        ui.next_size(240, 0);
        ui.textbox(ID_NOTE, Some("Notiz"), &mut self.note, 256);
        // a password box sets the input method's purpose itself (shown after it: this pass's)
        if !self.pw_away {
            ui.next_size(160, 0);
            ui.textbox(ID_PW, Some("Kennwort"), &mut self.pw, 64 | TB_PASSWORD);
        }
        ui.text(TS_BODY, &format!("Zweck: {}", plat::ime_purpose_of()));
        ui.box_end();
        // a chain of commands, each queued by the frame that saw the one before: the click's
        // frame and the next two use the three command rounds of a frame, the third link's
        // change needs the frame after them
        ui.box_begin(0, BOX_H, 8);
        if ui.button(ID_CHAIN, "Kette") & EV_ACT != 0 {
            self.chain_seen = 0;
            self.chain = 0;
            ui.cmd_run(CMD_CHAIN, 0);
        } else if self.chain > 0 && self.chain < 3 && self.chain_seen != self.chain {
            self.chain_seen = self.chain;
            ui.cmd_run(CMD_CHAIN, 0);
        }
        ui.text(TS_BODY, &format!("Kette: {}", self.chain));
        // an app that asks for a frame in every frame: the library stops after 8 (a UIWARN)
        ui.checkbox(ID_AGAIN, "Immer neu", &mut self.again);
        if self.again != 0 {
            ui.invalidate();
        }
        ui.box_end();
        // a thread of the app's own (an engine's): real time, then plat::wake; a timer polls
        ui.box_begin(0, BOX_H, 8);
        if ui.button(ID_WORK, "Hintergrund") & EV_ACT != 0 {
            plat::thread_spawn(0, Box::new(|| {
                plat::sleep_ms(300);
                WORK_DONE.fetch_add(1, Ordering::Relaxed);
                plat::wake();
            }));
            self.polled = "läuft";
            ui.timer(T_POLL, 100);
        }
        if ui.checkbox(ID_HANG, "Hängt", &mut self.hang) & EV_ACT != 0 {
            HANG.store(self.hang != 0, Ordering::Relaxed);
        }
        ui.text(TS_BODY, &format!("Hintergrund: {} Abfrage: {}", self.work_seen, self.polled));
        ui.box_end();
        // "Datum": the flyout below (date_flyout); what its date picker and calendar hold
        ui.box_begin(0, BOX_H, 8);
        if ui.button(ID_DATEBTN, "Datum") & EV_ACT != 0 {
            self.date_later = true;
        }
        let (y, m, d) = self.cal;
        let (wy, w) = datetime::iso_week(y, m as u8, d as u8);
        ui.text(TS_BODY, &format!("Datum: {}", ymd(self.date)));
        ui.text(TS_BODY, &format!("Kalender: {} {wy}-W{w:02} Wahl: {}", ymd(self.cal), self.cal_picks));
        ui.box_end();
        ui.box_end();
        // (E7) an independent app window: its own title bar with a tab and the caption buttons
        let (open, rect) = (self.app_open.clone(), self.app_rect.clone());
        if ui.window_begin(ID_APPWIN, "Privates Fenster", Some(&open), WF_APP | WF_RESIZABLE, Some(&rect)) {
            ui.titlebar_begin("Privates Fenster", 0);
            if ui.button(ID_APPTAB, "Tab Eins") & EV_ACT != 0 {
                self.app_tab += 1;
            }
            ui.titlebar_end();
            ui.box_begin(0, BOX_V | pad(12), 8);
            ui.text(TS_BODY, &format!("Tab geklickt: {}", self.app_tab));
            // the same id as the main window's "Notiz": each box keeps its own caret
            ui.next_size(240, 0);
            ui.textbox(ID_NOTE, Some("Notiz"), &mut self.app_note, 256);
            ui.box_end();
        }
        ui.window_end();
        self.nested_window(ui);
        self.two_windows(ui);
        // (E8) several files: the names the dialog chose, in its order
        if ui.filedlg(ID_FD, FD_OPEN | FD_MULTI, &self.fd_open, &mut self.fd_path, 4096, &["txt=Textdatei"]) & EV_ACT != 0 {
            let all = ui.filedlg_paths();
            let names: Vec<&str> = all.iter().map(|p| p.rsplit('/').next().unwrap_or(p)).collect();
            self.files = format!("{} {}", all.len(), names.join(" "));
        }
        if ui.filedlg(ID_FD_DIR, FD_FOLDER, &self.dir_open, &mut self.dir_path, 4096, &[]) & EV_ACT != 0 {
            let all = ui.filedlg_paths();
            let names: Vec<&str> = all.iter().map(|p| p.rsplit('/').next().unwrap_or(p)).collect();
            self.files = format!("{} {}", all.len(), names.join(" "));
        }
        if ui.flyout_begin(ID_FLYOUT, 0) && ui.menu_item(ID_ROW, "Eintrag A") & EV_ACT != 0 {
            self.rows += 1;
        }
        ui.flyout_end();
        self.date_flyout(ui);
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            CMD_TREE => {
                // what a screen reader gets: the page's links as links with their names
                let tree = ui.a11y_tree();
                let links = tree.wins.first().map(|w| w.nodes.iter().filter(|n| n.role == Role::Link).map(|n| n.name.as_str()).collect::<Vec<_>>());
                self.tree = links.unwrap_or_default().join(", ");
            }
            CMD_NEW_TAB => self.new_tab += 1,
            CMD_RELOAD => self.reload += 1,
            CMD_COPY => self.copy += 1,
            CMD_CHAIN => self.chain += 1,
            CMD_PW_AWAY => self.pw_away = !self.pw_away,
            CMD_NESTED => self.nest_open.set(1),
            CMD_TWO => {
                self.two_open[0].set(1);
                self.two_open[1].set(1);
            }
            // "Hintergrund": the thread's wake-up, the poll timer
            UIC_WAKE => self.work_seen = WORK_DONE.load(Ordering::Relaxed),
            UIC_TIMER if arg == T_POLL => {
                if WORK_DONE.load(Ordering::Relaxed) > 0 {
                    self.polled = "fertig";
                } else {
                    ui.timer(T_POLL, 100);
                }
            }
            _ => return false,
        }
        true
    }
}

ui::main!(ui::launch(&DESC, ViewTest::default(), &style::STYLE));
