// cardtest -- ui.action_card (lib/ui/action_card.rs): cards with a picture area, with an icon beside
// the title and a badge, a selected one, a disabled one, list rows with a text cut with "…" and a
// caption at the far end; how often each was chosen, for tests/cards.uis
//
//   build/win/cardtest.exe --script apps/cardtest/tests/cards.uis
//   build/win/cardtest.exe        (a window)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::rc::Rc;

use ui::*;

const DESC: AppDesc = AppDesc { title: "jfgui Karten", flags: AF_RESIZABLE, w: 760, h: 620, ..AppDesc::NONE };

const CARD_A: u32 = 100;
const CARD_B: u32 = 101;
const CARD_C: u32 = 102;
const CARD_D: u32 = 103;
const CARD_OFF: u32 = 104;
const ROW_A: u32 = 110;
const ROW_B: u32 = 111;

/// a drawing that fills its area (the picture of a card)
struct Fill;

impl Draw for Fill {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        g.dc.set_color(0xFF4C8BF5);
        let mut sh = g.shapes();
        sh.line_fx((r[0] + 4) << 8, (r[1] + 4) << 8, (r[0] + r[2] - 4) << 8, (r[1] + r[3] - 4) << 8, 3 << 8);
    }
}

struct Cards {
    chosen: [u32; 7],
}

impl App for Cards {
    fn frame(&mut self, ui: &mut Ui) {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | pad(16), 12);
        ui.text(TS_BODY, &format!("Gewählt: {:?}", self.chosen));
        // a row of cards: the same height whatever their texts
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H, 12);
        ui.next_size(160, 0);
        let draw: DrawRef = Rc::new(Fill);
        let a = ui.action_card(CARD_A, "Karte mit Bild", &CardDesc { title: "Mit Bild", text: "Die Zeichnung der App steht in der Fläche über den Texten.", picture_h: 60, draw: Some(draw), draw_ver: 1, ..CardDesc::default() });
        ui.next_size(160, 0);
        let b = ui.action_card(CARD_B, "Karte mit Symbol", &CardDesc { title: "Ohne Bild", icon: IC_ADD, text: "Kurz.", picture_h: 60, ..CardDesc::default() });
        ui.next_size(160, 0);
        ui.next_flags(ST_CHECKED);
        let c = ui.action_card(CARD_C, "Gewählte Karte", &CardDesc { title: "Gewählt", icon: IC_FLAG, text: "Das Symbol steht neben dem Titel.", ..CardDesc::default() });
        ui.next_size(160, 0);
        ui.next_flags(ST_DISABLED);
        let off = ui.action_card(CARD_OFF, "Abgeschaltete Karte", &CardDesc { title: "Aus", icon: IC_FLAG, text: "Sie nimmt nichts an.", ..CardDesc::default() });
        ui.box_end();
        // natural width
        let d = ui.action_card(CARD_D, "Karte mit Plakette", &CardDesc { title: "Plakette", badge: Some((BADGE_OK, "neu")), text: "200 DIP breit, wenn niemand etwas sagt.", ..CardDesc::default() });
        // a list: rows with one line each, a caption at the far end
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | BOX_CARD | pad(4), 2);
        ui.next_size(-1, 0);
        ui.next_flags(ST_SUBTLE);
        let r0 = ui.action_card(ROW_A, "kurz.txt", &CardDesc { title: "kurz.txt", icon: IC_DOCUMENT, text: "C:/Ordner", one_line: true, aside: "Heute 14:05", ..CardDesc::default() });
        ui.next_size(-1, 0);
        ui.next_flags(ST_SUBTLE);
        let r1 = ui.action_card(ROW_B, "lang.txt", &CardDesc { title: "lang.txt", icon: IC_DOCUMENT, text: "C:/ein/sehr/langer/Ordner/den/die/Zeile/nicht/ganz/zeigen/kann/weil/er/so/lang/ist/dass/die/Spalte/nicht/reicht/und/noch/ein/Stueck/darueber/hinaus/geht", one_line: true, aside: "01.10.2026", badge: Some((BADGE_WARN, "fehlt")), ..CardDesc::default() });
        ui.box_end();
        ui.box_end();
        for (k, ev) in [a, b, c, d, off, r0, r1].into_iter().enumerate() {
            if ev & EV_ACT != 0 {
                self.chosen[k] += 1;
                ui.repass();
            }
        }
    }
}

ui::main!(ui::launch(&DESC, Cards { chosen: [0; 7] }, &style::STYLE));
