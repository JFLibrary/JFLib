// inputtest -- (G7a) the pointer lock, the raw relative motion and where keys sit: what a game
// needs of the input, to try by hand (docs/GAME.md, the check list of the stage)
//
//   "Zeiger fangen"      ui.pointer_lock(true): the pointer is hidden and stays put, its raw
//                        motion adds up in "Summe" (ui.pointer_motion of every pass) and moves the
//                        dot in the field; "Freigeben" (or Esc, or leaving the window) lets it go.
//                        A press in the field asks for the lock too (the browser grants it only
//                        from a press: "wartet" until the next one)
//   "Zeiger: ..."        the state (aus, wartet, gefangen, nicht möglich, abgelehnt)
//   the field            takes the keys while it has the focus (ui.key_event_take): "Tasten" lists
//                        them by their place (KeyEvent::code as the browser's code name, + down,
//                        - up), "Letzte Taste" the details (code, the platform's scan code, the
//                        layout's character, what the layout prints on the key: plat::key_label)
//   "WASD"               the keys at the places of W, A, S and D whatever the layout prints on
//                        them (Z, Q, S, D on a French keyboard), held or not (ui.key_code_down)
//
//   python build_rs.py run inputtest                    (Linux: --target linux; the browser:
//   python build_rs.py test inputtest                    build with --target web)
//   build/win/inputtest.exe --script apps/inputtest/tests/lock.uis
//
// A headless run (scripts) plays the lock: Locked at once, Esc lets it go, `motion DX DY` moves
// the pointer, `keydown w code 26` presses the key at W's place.
//
//   inputtest --lock [--auto-close MS]   asks for the lock in the first frame and writes every
//                                        state of it to stdout ("lock: gefangen"), with the motion
//                                        summed when it ends: whether a compositor grants it
//                                        (a nested KWin without a screen; no input needed)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use plat::{KC_A, KC_D, KC_S, KC_W, key_code_name, key_label};
use ui::*;

/// the app's own version: version= of apps/inputtest/PACKAGE (the library's is ui::VERSION)
const APP_VERSION: &str = rt::app_version!();

const ID_LOCK: u32 = 100;
const ID_FREE: u32 = 101;
const ID_FIELD: u32 = 102;
const ID_CLEAR: u32 = 103;

const CMD_QUIT: u32 = 1;

static CMDS: [Command; 1] = [Command::new(CMD_QUIT, "&Beenden", 0, MOD_CTRL, b'Q' as u32, 0)];
static M_FILE: [Mi; 1] = [Mi::Cmd(CMD_QUIT)];
static MENUBAR: [Mi; 1] = [Mi::Menu("&Datei", &M_FILE)];

const DESC: AppDesc = AppDesc {
    title: "Eingabetest",
    flags: AF_RESIZABLE,
    w: 720,
    h: 560,
    min_w: 420,
    min_h: 380,
    cmds: &CMDS,
    menubar: &MENUBAR,
    ..AppDesc::NONE
};

/// keys the list keeps
const KEYS_MAX: usize = 12;

#[derive(Default)]
struct Inputtest {
    /// the raw motion since the lock began, and the passes that had some
    sum: (f64, f64),
    moves: u32,
    last: (f64, f64),
    /// the state of the last pass (a change starts a new sum)
    was: PointerLock,
    keys: Vec<String>,
    detail: String,
    /// --lock: asked in the first frame, the states written to stdout
    report: Option<bool>,
}

fn state_name(s: PointerLock) -> &'static str {
    match s {
        PointerLock::Off => "aus",
        PointerLock::Pending => "wartet",
        PointerLock::Locked => "gefangen",
        PointerLock::Unsupported => "nicht möglich",
        PointerLock::Refused => "abgelehnt",
        _ => "?",
    }
}

fn code_name(code: u32) -> String {
    match key_code_name(code) {
        "" if code == 0 => String::from("?"),
        "" => format!("0x{code:02X}"),
        n => String::from(n),
    }
}

impl App for Inputtest {
    fn frame(&mut self, ui: &mut Ui) {
        // the lock and its motion of this pass
        if self.report.is_none() {
            let on = plat::args().iter().any(|a| a == "--lock");
            self.report = Some(on);
            if on {
                let s = ui.pointer_lock(true);
                plat::write_stdout(format!("lock: asked -> {}\n", state_name(s)).as_bytes());
            }
        }
        let st = ui.pointer_lock_state();
        if st != self.was && self.report == Some(true) {
            plat::write_stdout(format!("lock: {} (motion so far {:.1} {:.1})\n", state_name(st), self.sum.0, self.sum.1).as_bytes());
        }
        if st != self.was {
            if st == PointerLock::Locked {
                self.sum = (0.0, 0.0);
                self.moves = 0;
            }
            self.was = st;
        }
        let (dx, dy) = ui.pointer_motion();
        if dx != 0.0 || dy != 0.0 {
            self.sum.0 += dx;
            self.sum.1 += dy;
            self.last = (dx, dy);
            self.moves += 1;
        }

        ui.menubar(None);
        ui.next_size(0, -1);
        ui.box_begin(0, BOX_V | pad(16), 8);
        ui.text(TS_SUBTITLE, "Zeiger und Tasten für Spiele");
        ui.box_begin(0, BOX_H, 8);
        if ui.accent_button(ID_LOCK, "Zeiger fangen") & EV_ACT != 0 {
            ui.pointer_lock(true);
        }
        if ui.button(ID_FREE, "Freigeben") & EV_ACT != 0 {
            ui.pointer_lock(false);
        }
        if ui.button(ID_CLEAR, "Liste leeren") & EV_ACT != 0 {
            self.keys.clear();
            self.detail.clear();
        }
        ui.box_end();
        ui.text(TS_BODY, &format!("Zeiger: {}", state_name(st)));
        ui.text(TS_BODY, &format!("Summe: {:.0} {:.0}", self.sum.0, self.sum.1));
        ui.text(TS_BODY | TX_SECONDARY, &format!("Letzte Bewegung: {:.1} {:.1} · Bewegungen: {}", self.last.0, self.last.1, self.moves));

        // the field: a press asks for the lock, it takes the keys while focused
        ui.next_size(0, 160);
        ui.next_flags(HB_KEYS);
        ui.canvas_label("Feld");
        let inp = ui.canvas(ID_FIELD, None, 0, 0).clone();
        if inp.pressed & (1 << BTN_LEFT) != 0 && st != PointerLock::Locked {
            ui.pointer_lock(true);
        }
        let [x, y, w, h] = inp.rect;
        let card = ui.color(T_CARD);
        let stroke = ui.color(T_CARD_STROKE);
        ui.cmd_rrect(x, y, w, h, 6, 1, card, stroke, stroke);
        // the dot: the middle plus the motion, kept inside
        let r = (6 * inp.dpi / 96).max(3);
        let wrap = |v: f64, n: i32| -> i32 { (v as i64).rem_euclid((n - 2 * r).max(1) as i64) as i32 };
        let cx = x + r + wrap(self.sum.0 + (w / 2 - r) as f64, w);
        let cy = y + r + wrap(self.sum.1 + (h / 2 - r) as f64, h);
        ui.draw_color(if st == PointerLock::Locked { ui.color(T_ACCENT) } else { ui.color(T_TEXT2) });
        ui.draw_circle(cx << 8, cy << 8, r << 8, 0);
        let hint = if st == PointerLock::Locked { "Esc gibt den Zeiger frei" } else { "Ins Feld klicken: Zeiger fangen, Tasten zeigen" };
        ui.text(TS_CAPTION | TX_SECONDARY, hint);

        if inp.ev & EV_FOCUS != 0 {
            while let Some(k) = ui.key_event_take() {
                let name = code_name(k.code);
                self.keys.push(format!("{}{}", if k.down { "+" } else { "-" }, name));
                if self.keys.len() > KEYS_MAX {
                    self.keys.remove(0);
                }
                let ch = char::from_u32(k.ch).filter(|_| k.ch != 0).map_or(String::from("–"), |c| format!("{c}"));
                let label = key_label(k.code).unwrap_or_else(|| String::from("–"));
                self.detail = format!("Letzte Taste: {name} · Code 0x{:02X} · Scancode {} · Zeichen {ch} · Beschriftung {label}", k.code, k.scancode);
                ui.repass();
            }
        }
        ui.text(TS_BODY, &format!("Tasten: {}", self.keys.join(" ")));
        ui.text(TS_BODY | TX_SECONDARY, if self.detail.is_empty() { "Letzte Taste: –" } else { &self.detail });
        let mut held = String::new();
        let mut labels = String::new();
        for (code, n) in [(KC_W, "W"), (KC_A, "A"), (KC_S, "S"), (KC_D, "D")] {
            if ui.key_code_down(code) {
                held.push_str(n);
            }
            labels.push_str(&format!(" {n}={}", key_label(code).unwrap_or_else(|| String::from("?"))));
        }
        ui.text(TS_BODY, &format!("WASD gehalten: {}", if held.is_empty() { "–" } else { &held }));
        ui.text(TS_BODY | TX_SECONDARY, &format!("Beschriftung der Lagen:{labels}"));
        ui.box_end();
        ui.statusbar_begin();
        ui.text(TS_CAPTION, &format!("Eingabetest {} · JFLib {}", APP_VERSION, ui::VERSION));
        ui.statusbar_end();
    }

    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
        match id {
            CMD_QUIT => ui.quit(),
            _ => return false,
        }
        true
    }
}

ui::main!(ui::launch(&DESC, Inputtest::default(), &style::STYLE));
