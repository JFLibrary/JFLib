// viewtest -- a canvas that acts like a web view (tests of ui's extensions for apps that show
// content drawn by an engine of their own, docs/GUIDE.md section 5 "Canvas"):
//
//   the page (ID_PAGE)   HB_KEYS while "Tastensenke" is on: every key is the page's, Ctrl + letter
//                        and Tab too (ui.key_take); F5 the page does not use: back to the command
//                        table (ui.accel_run); the arrow cursor (HB_CURSOR_SET), the hand over a link
//   the plain canvas     the defaults: crosshair, accelerators and Tab as everywhere
//
// tests/keys.uis drives it; the texts above the page say what it saw.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

const ID_SINK: u32 = 100;
const ID_PAGE: u32 = 101;
const ID_PLAIN: u32 = 102;

const CMD_NEW_TAB: u32 = 1;
const CMD_RELOAD: u32 = 2;

static CMDS: [Command; 2] = [
    Command::new(CMD_NEW_TAB, "Neuer Tab", 0, MOD_CTRL, VK_T, 0),
    Command::new(CMD_RELOAD, "Neu laden", 0, 0, VK_F5, 0),
];

const DESC: AppDesc = AppDesc {
    title: "Ansichtstest",
    flags: AF_RESIZABLE,
    w: 560,
    h: 480,
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
}

impl Default for ViewTest {
    fn default() -> Self {
        ViewTest { sink: 1, keys: Vec::new(), new_tab: 0, reload: 0, cursor: u32::MAX, hover: 0 }
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
        for (i, (l, _)) in LINKS.iter().enumerate() {
            ui.draw_color(if hover == i as u32 + 1 { 0xFF00_40C0 } else { 0xFF20_60E0 });
            ui.draw_rect(r[0] + ui.dp(l[0]), r[1] + ui.dp(l[1]), ui.dp(l[2]), ui.dp(l[3]));
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
        self.page(ui);
        self.plain(ui);
        ui.box_end();
    }

    fn command(&mut self, _ui: &mut Ui, id: u32, _arg: u32) -> bool {
        match id {
            CMD_NEW_TAB => self.new_tab += 1,
            CMD_RELOAD => self.reload += 1,
            _ => return false,
        }
        true
    }
}

ui::main!(ui::launch(&DESC, ViewTest::default(), &style::STYLE));
