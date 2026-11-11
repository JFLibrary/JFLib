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
//   the links            drawn by the page itself, published as K_LINK records without a hit box
//                        (ui.sem_rect): a script clicks one by its label, the page gets the click
//   "Dateien öffnen"     the file dialog with FD_MULTI in the folder --files DIR (E8): several
//                        files at once, ui.filedlg_paths() shown as "Dateien: N a b"
//
// tests/keys.uis, tests/menus.uis and tests/links.uis drive it; the texts above the page say what
// it saw.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
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
const ID_FD: u32 = 0x10_0000; // the file dialog (ids .. + 0x3FFF; rows + 17 + i, name box + 12)

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
    h: 600,
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

fn cursor_name(c: u32) -> &'static str {
    match c {
        CUR_ARROW => "Pfeil",
        CUR_HAND => "Hand",
        CUR_CROSS => "Kreuz",
        u32::MAX => "-",
        _ => "anderer",
    }
}

impl ViewTest {
    // the page: a canvas with HB_KEYS (while the switch is on) and the cursor it asks for
    fn page(&mut self, ui: &mut Ui) {
        ui.next_size(360, 120);
        let hover = self.hover;
        let cur = if hover != 0 { CUR_HAND } else { CUR_ARROW };
        ui.next_flags(HB_CURSOR_SET | cur << HB_CURSOR_SHIFT | if self.sink != 0 { HB_KEYS } else { 0 });
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
        // the keys: the page takes them all while it has the focus; F5 it hands back
        if self.sink != 0 && inp.ev & EV_FOCUS != 0 {
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
            let c = (ui.hit_flags_at(r[0] + inp.x, r[1] + inp.y) >> HB_CURSOR_SHIFT) & 15;
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
            let c = (ui.hit_flags_at(r[0] + inp.x, r[1] + inp.y) >> HB_CURSOR_SHIFT) & 15;
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
        ui.checkbox(ID_SINK, "Tastensenke", &mut self.sink);
        ui.text(TS_BODY, &format!("Tasten: {}", self.keys.join(" ")));
        ui.text(TS_BODY, &format!("Befehle: Tab {} Laden {}", self.new_tab, self.reload));
        ui.text(TS_BODY, &format!("Zeiger: {}", cursor_name(self.cursor)));
        if let Some((x, y)) = self.menu_at.take() {
            ui.menu_popup_at(&PAGE_MENU, x, y);
        }
        self.page(ui);
        self.plain(ui);
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
        if ui.button(ID_FILES, "Dateien öffnen") & EV_ACT != 0 {
            // the folder of --files, so the dialog starts there
            let dir = plat::arg_value("--files").unwrap_or_default();
            self.fd_path = format!("{}/", dir.trim_end_matches('/'));
            self.fd_open.set(1);
        }
        ui.box_end();
        ui.text(TS_BODY, &format!("Dateien: {}", self.files));
        ui.box_end();
        // (E8) several files: the names the dialog chose, in its order
        if ui.filedlg(ID_FD, FD_OPEN | FD_MULTI, &self.fd_open, &mut self.fd_path, 4096, &["txt=Textdatei"]) & EV_ACT != 0 {
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
