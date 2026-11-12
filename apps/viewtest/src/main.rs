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
//   "Rohe Tasten"        (E2) the page takes the keys as they go down and up (ui.key_event_take):
//                        "Roh:" lists them (+ down, - up, C- S- A- held modifiers, * repeat, :L :R :N
//                        location, #scancode)
//   "Eingabefeld"        (E4) the page has an editable field of its own: ui.ime_enable while it
//                        has the focus, ui.ime_pos at its caret, the composition from
//                        ui.ime_preedit drawn by the page ("IME:"), the committed text from
//                        text_take ("Feld:")
//   "Neues Fenster"      (E7) an independent app window (WF_APP) with a title bar of its own
//                        (titlebar_begin / titlebar_end: a tab in it, the caption buttons)
//   "Zeiger wechseln"    (E3) the page's cursor through ui.cursor_next: not-allowed, grab,
//                        zoom-in, the north-east corner, a custom cursor of the page's own;
//                        "Eigener größer" redraws it (ui.cursor_replace), "Eigener frei" frees it
//                        (ui.cursor_free); "Zeiger:" says ui.cursor_at under the pointer
//
// tests/keys.uis, tests/menus.uis and tests/links.uis drive it; the texts above the page say what
// it saw.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

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
const ID_FD: u32 = 0x10_0000; // the file dialog (ids .. + 0x3FFF; rows + 17 + i, name box + 12)
const ID_FD_DIR: u32 = 0x11_0000; // the folder dialog (FD_FOLDER)

const CMD_NEW_TAB: u32 = 1;
const CMD_RELOAD: u32 = 2;
const CMD_COPY: u32 = 3; // a row of the page's menu (Mi::Item)
const CMD_TREE: u32 = 4; // after the frame: the tree is complete

// the page's context menu
static PAGE_MENU: [Mi; 3] = [Mi::Cmd(CMD_RELOAD), Mi::Sep, Mi::Item(CMD_COPY, "Link kopieren", 0)];

static CMDS: [Command; 2] = [
    Command::new(CMD_NEW_TAB, "Neuer Tab", 0, MOD_CTRL, VK_T, 0),
    Command::new(CMD_RELOAD, "Neu laden", 0, 0, VK_F5, 0),
];

const DESC: AppDesc = AppDesc {
    title: "Ansichtstest",
    flags: AF_RESIZABLE,
    w: 560,
    h: 760,
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
        }
    }
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
            ui.box_end();
        }
        ui.window_end();
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
    }

    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
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
            _ => return false,
        }
        true
    }
}

ui::main!(ui::launch(&DESC, ViewTest::default(), &style::STYLE));
