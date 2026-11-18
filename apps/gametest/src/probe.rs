// probe -- the window the scripts drive (tests/input.uis, tests/tick.uis): ui's real input path into a
// game::Input, as a game feeds it: the keys a canvas with HB_KEYS takes (KeyEvent::code), its buttons
// (UiInput::pressed / released), ui.pointer_motion, ui.pointer_lock_state; a press that asks for the
// lock calls ui.pointer_lock(true).  A FixedTick on plat::now_us (the script's virtual clock in a
// headless run) takes the input per tick.  What it saw is text for the scripts' `expect`.

use alloc::format;
use alloc::string::String;

use game::input::{MOUSE_LEFT, MOUSE_RIGHT};
use game::{ActionId, ActionKind, ActionMap, Bind, FixedTick, Input, Source};
use plat::{KC_A, KC_D, KC_S, KC_SPACE, KC_W, PointerLock};
use ui::*;

const ID_GAME: u32 = 100;
const TIMER_FRAME: u32 = 1;

const DESC: AppDesc = AppDesc { title: "Spielkern-Test", flags: AF_RESIZABLE, w: 640, h: 520, min_w: 320, min_h: 300, ..AppDesc::NONE };

struct Probe {
    input: Input,
    fwd: ActionId,
    side: ActionId,
    fire: ActionId,
    jump: ActionId,
    look_x: ActionId,
    look_y: ActionId,
    clock: FixedTick,
    /// the look summed per frame (radians)
    look: (f64, f64),
    /// presses of fire seen per frame, asks for the lock, pauses seen
    clicks: u32,
    asks: u32,
    pauses: u32,
    /// ticks run, ticks with forward down, fire and jump pressed per tick
    ticks: u64,
    fwd_ticks: u64,
    fire_ticks: u32,
    jump_ticks: u32,
}

fn lock_name(s: PointerLock) -> &'static str {
    match s {
        PointerLock::Off => "aus",
        PointerLock::Pending => "wartet",
        PointerLock::Locked => "gefangen",
        PointerLock::Unsupported => "nicht möglich",
        PointerLock::Refused => "abgelehnt",
        _ => "?",
    }
}

impl Probe {
    fn new() -> Probe {
        let mut m = ActionMap::new();
        let fwd = m.add("forward", ActionKind::Axis, &[Bind::key(KC_W), Bind::scaled(Source::Key(KC_S), -1.0)]);
        let side = m.add("strafe", ActionKind::Axis, &[Bind::key(KC_D), Bind::scaled(Source::Key(KC_A), -1.0)]);
        let fire = m.add("fire", ActionKind::Button, &[Bind::new(Source::Mouse(MOUSE_LEFT))]);
        let jump = m.add("jump", ActionKind::Button, &[Bind::key(KC_SPACE), Bind::new(Source::Mouse(MOUSE_RIGHT))]);
        let look_x = m.add("look_x", ActionKind::Motion, &[Bind::new(Source::MouseX)]);
        let look_y = m.add("look_y", ActionKind::Motion, &[Bind::new(Source::MouseY)]);
        let mut input = Input::new(m);
        // the scripts compare angles: the factor of a 1000-dpi mouse on every platform
        input.set_mouse(game::MouseSettings { factor: Some(1.0), ..game::MouseSettings::DEFAULT });
        Probe { input, fwd, side, fire, jump, look_x, look_y, clock: FixedTick::default(), look: (0.0, 0.0), clicks: 0, asks: 0, pauses: 0, ticks: 0,
                fwd_ticks: 0, fire_ticks: 0, jump_ticks: 0 }
    }
}

impl App for Probe {
    fn frame(&mut self, ui: &mut Ui) {
        // the lock as the window has it, then the canvas' input of this pass (the edges come in the first
        // pass of a batch only)
        self.input.set_lock(ui.pointer_lock_state());
        ui.next_size(0, -1);
        ui.box_begin(0, BOX_V | pad(12), 6);
        ui.text(TS_SUBTITLE, "Eingabe eines Spiels");
        ui.next_size(0, 160);
        ui.next_flags(HB_KEYS);
        ui.canvas_label("Spiel");
        let inp = ui.canvas(ID_GAME, None, 0, 0).clone();
        if inp.ev & EV_FOCUS != 0 {
            while let Some(k) = ui.key_event_take() {
                self.input.key(k.code, k.down);
            }
        }
        self.input.buttons(inp.pressed, inp.released);
        let (dx, dy) = ui.pointer_motion();
        self.input.motion(dx, dy);
        if self.input.take_lock_request() {
            self.asks += 1;
            ui.pointer_lock(true);
        }
        if self.input.take_pause() {
            self.pauses += 1;
        }
        if self.input.pressed(self.fire) {
            self.clicks += 1;
        }
        self.look.0 += self.input.motion_of(self.look_x);
        self.look.1 += self.input.motion_of(self.look_y);
        // the ticks due by the clock: each takes the input
        let n = self.clock.advance_to(plat::task::now_us().saturating_mul(1000));
        for _ in 0..n {
            let t = self.input.tick();
            self.ticks += 1;
            self.fwd_ticks += t.down(self.fwd) as u64;
            self.fire_ticks += t.pressed(self.fire) as u32;
            self.jump_ticks += t.pressed(self.jump) as u32;
        }
        self.input.end_frame();
        if self.input.playing() {
            // the next frame through a timer (an animation's way: ui.invalidate in every frame stops after 8)
            ui.timer(TIMER_FRAME, 1);
        }

        let [x, y, w, h] = inp.rect;
        let card = ui.color(T_CARD);
        let stroke = ui.color(T_CARD_STROKE);
        ui.cmd_rrect(x, y, w, h, 6, 1, card, stroke, stroke);
        let deg = 180.0 / core::f64::consts::PI;
        ui.text(TS_BODY, &format!("Zustand: {}", if self.input.playing() { "spielt" } else { "Pause" }));
        ui.text(TS_BODY, &format!("Sperre: {}", lock_name(self.input.lock())));
        ui.text(TS_BODY, &format!("Vor: {} · Seite: {} · Springen: {}", self.input.axis(self.fwd), self.input.axis(self.side), self.input.down(self.jump) as u8));
        ui.text(TS_BODY, &format!("Klicks: {} · Anfragen: {} · Pausen: {}", self.clicks, self.asks, self.pauses));
        ui.text(TS_BODY, &format!("Blick: {:.3} {:.3}", self.look.0 * deg, self.look.1 * deg));
        ui.text(TS_BODY, &format!("Takte: {} · vor: {} · Feuer: {} · Sprünge: {}", self.ticks, self.fwd_ticks, self.fire_ticks, self.jump_ticks));
        let mut labels = String::new();
        for a in [self.fwd, self.side, self.fire, self.jump] {
            let binds: alloc::vec::Vec<String> = self.input.map().binds(a).iter().map(|b| b.source.label()).collect();
            labels.push_str(&format!(" {}={}", self.input.map().name(a), binds.join("/")));
        }
        ui.text(TS_CAPTION | TX_SECONDARY, &format!("Belegung:{labels}"));
        ui.box_end();
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        if id == UIC_TIMER && arg == TIMER_FRAME {
            ui.invalidate();
            return true;
        }
        false
    }
}

pub fn launch() -> i32 {
    ui::launch(&DESC, Probe::new(), &style::STYLE)
}
