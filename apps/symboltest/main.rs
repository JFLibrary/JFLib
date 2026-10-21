// symboltest -- the multi-colour vector symbols (ui::Symbol, lib/ui/symbol.rs) of JFNetworker's
// devices as a sheet: every symbol at 32, 64 and 128 DIP wide and without power (64 DIP at
// 45 %, LEDs and screens off), each in the palette of its category; for tests/sheet.uis
//
//   build/win/symboltest.exe --script apps/symboltest/tests/sheet.uis
//   build/win/symboltest.exe          (the sheet in a window; the button switches the theme)
//
// The art comes from apps/JFNetworker/icons/*.svg through tools/vsym.py; the symbols are
// parsed once and drawn from a SymbolCache (rasters), above 256 px as vectors.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use ui::*;

#[path = "../JFNetworker/src/workspace/icons/symbols_data.rs"]
mod symbols_data;
use symbols_data::{SYMBOLS, SYMBOL_GRID};

const DESC: AppDesc = AppDesc { title: "jfgui Symbole", flags: AF_RESIZABLE, w: 1430, h: 1000, ..AppDesc::NONE };

const ID_THEME: u32 = wid("theme");
const ID_PAGE: u32 = wid("page");
const ID_GRID: u32 = wid("grid");

// the category tints (their light-theme values)
const NETWORK: u32 = 0x4F_7396; // steel blue
const SECURITY: u32 = 0xA6_5A4A; // brick
const WIRELESS: u32 = 0x3E_8A86; // teal
const SERVER: u32 = 0x6A_6390; // slate violet
const DISTRIBUTED: u32 = 0x9E_9047; // olive amber
const END: u32 = 0x8A_929C; // neutral

/// the sheet's order: (symbol, category, tint); symbols not named here follow
const CATALOG: [(&str, &str, u32); 26] = [
    ("pc", "Endgerät", END),
    ("laptop", "Endgerät", END),
    ("smartphone", "Endgerät", END),
    ("tablet", "Endgerät", END),
    ("printer", "Endgerät", END),
    ("ip_phone", "Endgerät", END),
    ("iot_sensor", "Endgerät", END),
    ("iot_camera", "Endgerät", END),
    ("smart_tv", "Endgerät", END),
    ("switch_l2", "Netzwerk", NETWORK),
    ("switch_l3", "Netzwerk", NETWORK),
    ("router", "Netzwerk", NETWORK),
    ("home_router", "Netzwerk", NETWORK),
    ("firewall_ngfw", "Sicherheit", SECURITY),
    ("ap_wifi6e", "WLAN/WAN", WIRELESS),
    ("modem_ont", "WLAN/WAN", WIRELESS),
    ("cloud_internet", "WLAN/WAN", WIRELESS),
    ("server_1u", "Server", SERVER),
    ("server_tower", "Server", SERVER),
    ("load_balancer", "Verteilt", DISTRIBUTED),
    ("db_shard", "Verteilt", DISTRIBUTED),
    ("cache", "Verteilt", DISTRIBUTED),
    ("queue", "Verteilt", DISTRIBUTED),
    ("traffic_gen", "Verteilt", DISTRIBUTED),
    ("container_host", "Verteilt", DISTRIBUTED),
    ("note", "Notiz", END),
];

/// the widths of the sheet's columns (DIP), then the one without power
const SIZES: [i32; 3] = [32, 64, 128];
const OFF_W: i32 = 64;
const OFF_OPACITY: u32 = 115; // 45 %
const GAP: i32 = 12;
const ROW_W: i32 = 32 + 64 + 128 + 64 + 3 * GAP;
const ROW_H: i32 = 96;

/// the symbols, their palettes of this frame and the raster cache
struct Sheet {
    /// (name, category) per symbol
    names: Vec<(&'static str, &'static str)>,
    syms: Vec<Symbol>,
    tints: Vec<u32>,
    pals: RefCell<Vec<SymPalette>>,
    cache: RefCell<SymbolCache>,
    dpi: Cell<i32>,
}

/// one symbol's row of sizes (a canvas)
struct Row {
    sheet: Rc<Sheet>,
    i: usize,
}

impl Draw for Row {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let s = &self.sheet;
        let dpi = s.dpi.get();
        let dp = |v: i32| (v * dpi + 48) / 96;
        let pal = s.pals.borrow()[self.i];
        let sym = &s.syms[self.i];
        let mut cache = s.cache.borrow_mut();
        let mut x = r[0];
        for k in 0..SIZES.len() + 1 {
            let w = dp(if k < SIZES.len() { SIZES[k] } else { OFF_W });
            let y = r[1] + (r[3] - sym.height(w)) / 2;
            if k < SIZES.len() {
                cache.draw(g, self.i as u32, sym, x, y, w, &pal, 255);
            } else {
                cache.draw(g, self.i as u32, sym, x, y, w, &pal.powered_off(), OFF_OPACITY);
            }
            x += w + dp(GAP);
        }
    }
}

struct App1 {
    sheet: Rc<Sheet>,
    rows: Vec<DrawRef>,
    ver: u32,
    last: u32,
}

impl App1 {
    fn new() -> App1 {
        let mut order: Vec<(&'static str, &'static str, u32, &'static str)> = Vec::new();
        for (name, cat, tint) in CATALOG {
            if let Some((_, art)) = SYMBOLS.iter().find(|(n, _)| *n == name) {
                order.push((name, cat, tint, art));
            }
        }
        for &(name, art) in SYMBOLS {
            if !CATALOG.iter().any(|&(n, _, _)| n == name) {
                order.push((name, "?", END, art));
            }
        }
        let sheet = Rc::new(Sheet {
            names: order.iter().map(|o| (o.0, o.1)).collect(),
            syms: order.iter().map(|o| Symbol::parse(o.3, SYMBOL_GRID)).collect(),
            tints: order.iter().map(|o| o.2).collect(),
            pals: RefCell::new(Vec::new()),
            cache: RefCell::new(SymbolCache::new()),
            dpi: Cell::new(96),
        });
        let rows = (0..order.len()).map(|i| -> DrawRef { Rc::new(Row { sheet: sheet.clone(), i }) }).collect();
        App1 { sheet, rows, ver: 1, last: 0 }
    }
}

impl App for App1 {
    fn frame(&mut self, ui: &mut Ui) {
        // the palettes of the theme; a change redraws the canvases
        let pals: Vec<SymPalette> = self.sheet.tints.iter().map(|&t| SymPalette::from_theme(ui, t)).collect();
        let key = pals.iter().fold(ui.dpi() as u32, |h, p| h.rotate_left(5) ^ p.hash());
        if key != self.last {
            self.last = key;
            self.ver = self.ver.wrapping_add(1);
        }
        *self.sheet.pals.borrow_mut() = pals;
        self.sheet.dpi.set(ui.dpi());

        ui.box_begin(0, BOX_V | pad(16), 4);
        ui.box_begin(0, BOX_H, 12);
        ui.next_size(-1, 0);
        ui.text(TS_SUBTITLE, &format!("Gerätesymbole ({})", self.rows.len()));
        let dark = ui.theme_dark();
        if ui.button(ID_THEME, if dark { "Hell" } else { "Dunkel" }) & EV_ACT != 0 {
            ui.theme_set(if dark { 0 } else { 1 });
        }
        ui.box_end();
        ui.text(TS_CAPTION | TX_SECONDARY, "32, 64 und 128 DIP breit; ohne Strom (64 DIP, 45 %)");
        ui.box_end();
        ui.next_size(0, -1);
        ui.scroll_begin(ID_PAGE, SC_V | pad(16));
        ui.gridbox_begin(ID_GRID, &[0, 0, 0, 0], 0, 10);
        for i in 0..self.rows.len() {
            let (name, cat) = self.sheet.names[i];
            ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 4);
            ui.box_begin(0, BOX_H, 8);
            ui.text(TS_BODY_STRONG, name);
            ui.text(TS_CAPTION | TX_SECONDARY, cat);
            ui.box_end();
            ui.next_size(ROW_W, ROW_H);
            ui.canvas(ui.id(name), Some(self.rows[i].clone()), 0, self.ver);
            ui.box_end();
        }
        ui.gridbox_end();
        ui.scroll_end();
    }
}

ui::main!(ui::launch(&DESC, App1::new(), &style::STYLE));
